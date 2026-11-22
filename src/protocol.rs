use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::agent::{self, AgentEvent};
use crate::llm::Message;
use crate::runtime::{Runtime, RuntimeOptions};
use crate::session::Session;
use crate::settings::Settings;
use crate::util::now_rfc3339;

#[derive(Debug, Deserialize)]
struct OpMsg {
    #[allow(dead_code)]
    id: String,
    op: Value,
}

#[derive(Debug, Serialize, Default, Deserialize)]
#[serde(default)]
pub struct StartSessionArgs {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub permission_mode: Option<String>,
    pub effort: Option<String>,
    pub cwd: Option<String>,
}

pub fn emit(parent: &str, event: Value) {
    println!(
        "{}",
        json!({
            "timestamp": now_rfc3339(),
            "id": format!("evt_{}", crate::util::ulid()),
            "event": event,
            "parent": parent,
        })
    );
}

pub struct Server {
    pub settings: Settings,
    pub runtime: Option<Arc<Runtime>>,
    pub session: Option<Session>,
    pub messages: Vec<Message>,
    pub ctx: Option<crate::tools::ToolCtx>,
}

impl Server {
    pub fn new(settings: Settings) -> Server {
        Server {
            settings,
            runtime: None,
            session: None,
            messages: Vec::new(),
            ctx: None,
        }
    }

    /// Handle one JSONL line. Returns false when the server should shut down.
    pub async fn handle_line(&mut self, line: &str) -> bool {
        let line = line.trim();
        if line.is_empty() {
            return true;
        }
        let msg: OpMsg = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(e) => {
                emit("op_unknown", json!({"Error": format!("Invalid OpMsg JSON: {e}")}));
                return true;
            }
        };
        if !is_ulid(msg.id.strip_prefix("op_").unwrap_or(&msg.id)) {
            emit(
                &msg.id,
                json!({"Error": "Invalid OpMsg JSON: invalid ULID"}),
            );
            return true;
        }
        let op_id = msg.id.clone();
        let Some((variant, payload)) = msg.op.as_object().and_then(|o| o.iter().next()).map(|(k, v)| (k.clone(), v.clone())) else {
            emit(&op_id, json!({"Error": "Invalid OpMsg JSON: missing field `op`"}));
            return true;
        };

        match variant.as_str() {
            "StartSession" => {
                let args: StartSessionArgs = serde_json::from_value(payload).unwrap_or_default();
                let settings = self.settings.clone();
                let rt = Runtime::build(RuntimeOptions {
                    settings,
                    provider: args.provider,
                    model: args.model,
                    effort: args.effort,
                    permission_mode: args.permission_mode.or(Some("strict".into())),
                    tools: None,
                    include_tools: vec![],
                    exclude_tools: vec![],
                    skills: None,
                    short_prompt: None,
                    system_prompt: None,
                    append_system_prompt: None,
                    session_id: None,
                    interactive: false,
                });
                if !rt.mcp.configured().is_empty() {
                    if let Err(e) = rt.mcp.start().await {
                        eprintln!("warning: MCP initialization failed: {e}");
                    }
                }
                let rt = Arc::new(rt);
                let mut session = Session::new(rt.provider_id.clone(), rt.model.id.clone(), rt.short_prompt);
                session.system = rt.system_sections();
                let skills = crate::skills::discover(&rt.cwd);
                let agents: Vec<Value> = serde_json::from_str(crate::runtime::BUILTIN_AGENTS).unwrap_or_default();
                emit(
                    &op_id,
                    json!({"SessionStart": {
                        "model": rt.describe(),
                        "provider": rt.provider_json(),
                        "session_id": session.meta.id,
                        "cwd": rt.cwd.display().to_string(),
                        "permission_mode": rt.permission_mode.as_str(),
                        "skills": skills.iter().map(|s| json!({"name": s.name, "description": s.description, "scope": s.scope, "argument_hint": s.argument_hint})).collect::<Vec<_>>(),
                        "subagents": agents,
                    }}),
                );
                emit(&op_id, json!({"ExtensionRefreshed": {
                    "session_id": session.meta.id,
                    "skills": skills.iter().map(|s| json!({"name": s.name, "description": s.description, "scope": s.scope, "argument_hint": s.argument_hint})).collect::<Vec<_>>(),
                    "subagents": agents,
                    "mcp_servers": [],
                }}));
                self.messages.clear();
                self.ctx = Some(crate::tools::ToolCtx::new(rt.clone(), rt.cwd.clone()));
                self.runtime = Some(rt);
                self.session = Some(session);
            }
            "UserInput" => {
                let text = payload.as_str().unwrap_or_default().to_string();
                if self.runtime.is_none() {
                    emit(&op_id, json!({"Error": "session not initialized"}));
                    return true;
                }
                let rt = self.runtime.clone().unwrap();
                let ctx = self.ctx.clone().unwrap();
                if let Some(s) = self.session.as_mut() {
                    s.note_user_message(&text);
                }
                emit(&op_id, json!({"UserInput": text}));
                emit(&op_id, json!({"TurnStart": {"turn_id": op_id}}));
                let mut messages = self.messages.clone();
                messages.push(Message::user_text(format!("{text}\n\n{}", rt.folder_structure())));
                let mut text_out = String::new();
                let outcome = agent::run_turn(&rt, &mut messages, &ctx, 32, |ev| match ev {
                    AgentEvent::TextDelta(t) => {
                        text_out.push_str(&t);
                        emit(&op_id, json!({"AgentMessageDelta": t}));
                    }
                    AgentEvent::AssistantMessage(t) => {
                        text_out.clear();
                        emit(&op_id, json!({"AgentMessage": t}));
                    }
                    AgentEvent::ToolStart { id, name, args } => {
                        emit(&op_id, json!({"ToolStart": {"id": id, "name": name, "args": args}}));
                    }
                    AgentEvent::ToolEnd { id, name, result, is_error } => {
                        emit(&op_id, json!({"ToolEnd": {
                            "tool_use_id": id, "tool_name": name,
                            "status": if is_error { "Error" } else { "Completed" },
                            "result_json": if is_error { json!({"Error": result}) } else { json!({"Completed": result}) }
                        }}));
                    }
                    AgentEvent::Usage(u) => {
                        emit(&op_id, json!({"UsageUpdate": {
                            "usage": {"input_tokens": u.input_tokens, "output_tokens": u.output_tokens},
                            "context": {"used_tokens": u.input_tokens + u.output_tokens, "limit_tokens": rt.model.context_limit.unwrap_or(0)}
                        }}));
                    }
                    _ => {}
                })
                .await;
                self.messages = messages;
                if let Some(err) = outcome.error {
                    emit(&op_id, json!({"TurnEnd": {"turn_id": op_id, "status": {"Error": {"kind": "error", "headline": "llm error", "details": [err]}}, "steps": outcome.steps}}));
                } else {
                    emit(&op_id, json!({"TurnEnd": {"turn_id": op_id, "status": "Completed", "steps": outcome.steps}}));
                }
                if let Some(s) = self.session.as_mut() {
                    s.messages = self.messages.clone();
                    let _ = s.save();
                }
            }
            "Shutdown" => {
                if let Some(s) = &self.session {
                    emit(&op_id, json!({"SessionEnd": {
                        "session_id": s.meta.id,
                        "reason": "Shutdown",
                        "usage": {"input_tokens": s.meta.usage.input_tokens, "output_tokens": s.meta.usage.output_tokens},
                    }}));
                }
                emit(&op_id, json!("Goodbye"));
                return false;
            }
            "Interrupt" => {
                emit(&op_id, json!({"TurnEnd": {"turn_id": op_id, "status": "Interrupted", "steps": 0}}));
            }
            "SlashCommand" => {
                let cmd = payload
                    .get("command")
                    .and_then(|c| c.as_str())
                    .or_else(|| payload.as_str())
                    .unwrap_or("");
                match cmd {
                    "/clear" => {
                        self.messages.clear();
                        emit(&op_id, json!({"AgentMessage": "Conversation cleared."}));
                    }
                    "/context" => {
                        let rt = self.runtime.clone();
                        let used = self.messages.iter().map(|m| m.text().len()).sum::<usize>() / 4;
                        if let Some(rt) = rt {
                            emit(&op_id, json!({"AgentMessage": format!("Context: ~{used} tokens used of {} limit ({} messages).", rt.model.context_limit.unwrap_or(0), self.messages.len())}));
                        }
                    }
                    "/compact" => {
                        let kept = self.messages.len().min(4);
                        let tail: Vec<Message> = self.messages[self.messages.len() - kept..].to_vec();
                        self.messages = vec![Message::user_text("<system-reminder>Earlier conversation was compacted.</system-reminder>")];
                        self.messages.extend(tail);
                        emit(&op_id, json!({"CompactEnd": {"summary": "conversation compacted"}}));
                    }
                    "/exit" => {
                        emit(&op_id, json!("Goodbye"));
                        return false;
                    }
                    other => {
                        emit(&op_id, json!({"AgentMessage": format!("Unknown command: {other}")}));
                    }
                }
            }
            "ContextReport" => {
                emit(&op_id, json!({"InfoBlockStart": {"title": "Context"}}));
                emit(&op_id, json!({"InfoBlockAppend": {"text": format!("{} messages in context", self.messages.len())}}));
            }
            "Compact" => {
                emit(&op_id, json!({"CompactEnd": {"summary": null}}));
            }
            "ApprovalResponse" | "QuestionResponse" | "Steer" | "UpdateSession" | "ResumeSession"
            | "RegisterLocalProvider" | "RestoreLocalProvider" | "RewindSession" | "ForkSession"
            | "Goal" | "AmbientPhrase" | "AmbientSuggestion" | "ShellInput" => {
                emit(&op_id, json!({"Error": format!("op '{variant}' is not supported by this build")}));
            }
            other => {
                emit(&op_id, json!({"Error": format!("unknown op '{other}'")}));
            }
        }
        true
    }
}

fn is_ulid(s: &str) -> bool {
    s.len() == 26
        && s.bytes().all(|b| {
            b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b.to_ascii_uppercase())
        })
}
