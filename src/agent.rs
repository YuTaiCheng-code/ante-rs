use serde_json::Value;
use std::sync::Arc;

use crate::llm::{Block, ChatRequest, ChatResponse, Message, Role, StreamEvent, Usage};
use crate::runtime::Runtime;
use crate::tools::ToolResult;

#[derive(Debug, Clone)]
pub enum AgentEvent {
    TextDelta(String),
    AssistantMessage(String),
    ThinkingDelta(String),
    ToolStart { id: String, name: String, args: Value },
    ToolEnd { id: String, name: String, result: String, is_error: bool },
    Usage(Usage),
    TurnEnd { status: String, steps: u32 },
}

#[derive(Debug, Clone)]
pub struct TurnOutcome {
    pub final_text: String,
    pub thinking: String,
    pub usage: Usage,
    pub steps: u32,
    pub status: String, // Completed | Interrupted | Error
    pub error: Option<String>,
}

pub fn tool_result_text(r: &ToolResult) -> String {
    if r.is_error {
        format!("error: {}", r.content)
    } else {
        r.content.clone()
    }
}

/// One assistant turn: stream, execute tool calls, repeat until the model stops.
pub async fn run_turn<F>(
    runtime: &Arc<Runtime>,
    messages: &mut Vec<Message>,
    ctx: &crate::tools::ToolCtx,
    max_steps: u32,
    mut on_event: F,
) -> TurnOutcome
where
    F: FnMut(AgentEvent) + Send,
{
    let mut steps = 0;
    let mut final_text = String::new();
    let mut all_thinking = String::new();
    let mut usage = Usage::default();

    let client = match runtime.client() {
        Ok(c) => c,
        Err(e) => {
            return TurnOutcome {
                final_text: String::new(),
                thinking: String::new(),
                usage,
                steps,
                status: "Error".into(),
                error: Some(e.to_string()),
            }
        }
    };

    if !client.has_credential() {
        let details = format!(
            "not signed in to {}\nRun /providers to sign in, or set the provider's API key.",
            runtime.provider_id
        );
        return TurnOutcome {
            final_text: String::new(),
            thinking: String::new(),
            usage,
            steps,
            status: "Error".into(),
            error: Some(format!(
                "oauth|OAuth sign-in required|{details}"
            )),
        };
    }

    loop {
        if steps >= max_steps {
            return TurnOutcome {
                final_text,
                thinking: all_thinking,
                usage,
                steps,
                status: "Error".into(),
                error: Some("step budget exhausted".into()),
            };
        }
        steps += 1;

        let req = ChatRequest {
            model: runtime.model.id.clone(),
            system: runtime.system_sections(),
            messages: messages.clone(),
            tools: runtime.tool_schemas(),
            max_tokens: runtime
                .model
                .max_tokens
                .unwrap_or(8192)
                .min(32768),
            effort: runtime.effort.clone(),
            include_images: runtime.model.support_vision,
        };
        let mut text = String::new();
        let mut thinking = String::new();
        let resp: Result<ChatResponse, anyhow::Error> = client
            .complete(req, |ev| match ev {
                StreamEvent::TextDelta(t) => {
                    text.push_str(t);
                    on_event(AgentEvent::TextDelta(t.clone()));
                }
                StreamEvent::ThinkingDelta(t) => {
                    thinking.push_str(t);
                    on_event(AgentEvent::ThinkingDelta(t.clone()));
                }
                StreamEvent::Usage(u) => {
                    usage.input_tokens = u.input_tokens;
                    usage.output_tokens = u.output_tokens;
                    on_event(AgentEvent::Usage(*u));
                }
                _ => {}
            })
            .await;
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                return TurnOutcome {
                    final_text,
                    thinking: all_thinking,
                    usage,
                    steps,
                    status: "Error".into(),
                    error: Some(e.to_string()),
                };
            }
        };

        usage = Usage {
            input_tokens: usage.input_tokens.max(resp.usage.input_tokens),
            output_tokens: usage.output_tokens + resp.usage.output_tokens,
        };
        all_thinking.push_str(&thinking);
        if !resp.text.is_empty() {
            final_text = resp.text.clone();
            on_event(AgentEvent::AssistantMessage(resp.text.clone()));
        }

        let mut assistant_blocks: Vec<Block> = Vec::new();
        if !resp.text.is_empty() {
            assistant_blocks.push(Block::Text(resp.text.clone()));
        }
        for call in &resp.tool_calls {
            assistant_blocks.push(Block::ToolUse {
                id: call.id.clone(),
                name: call.name.clone(),
                input: call.input.clone(),
            });
        }
        if !assistant_blocks.is_empty() {
            messages.push(Message {
                role: Role::Assistant,
                blocks: assistant_blocks,
            });
        }

        if resp.tool_calls.is_empty() {
            return TurnOutcome {
                final_text: if resp.text.is_empty() { final_text } else { resp.text },
                thinking: all_thinking,
                usage,
                steps,
                status: "Completed".into(),
                error: None,
            };
        }

        let mut result_blocks: Vec<Block> = Vec::new();
        for call in &resp.tool_calls {
            on_event(AgentEvent::ToolStart {
                id: call.id.clone(),
                name: call.name.clone(),
                args: call.input.clone(),
            });
            let mut denied_reason: Option<String> = None;

            let decision = ctx.runtime.permission_mode.to_owned();
            let _ = decision;
            let mut permission = crate::tools::permission::PermissionManager::new(
                runtime.permission_mode,
                &runtime.permission_rules,
            );
            let _ = &mut permission;
            let perm = permission.check(&call.name, &call.input);
            if std::env::var("ANTE_DEBUG_PERM").is_ok() {
                eprintln!("[perm] tool={} mode={} interactive={} decision={:?}", call.name, runtime.permission_mode.as_str(), runtime.interactive, perm);
            }
            let allowed = match perm {
                crate::tools::permission::Decision::Allow => true,
                crate::tools::permission::Decision::Deny => false,
                crate::tools::permission::Decision::Ask => {
                    if runtime.interactive {
                        let arg = crate::tools::permission::primary_arg(&call.name, &call.input);
                        match ask_permission(&call.name, &arg).await {
                            Approval::Once => true,
                            Approval::Session => {
                                let m = permission.grant_session(&call.name, &arg);
                                eprintln!("Allowed for this session: {m}");
                                true
                            }
                            Approval::Always => {
                                let m = permission.grant_always(&call.name, &arg, &mut runtime.settings.clone());
                                let _ = crate::settings::append_allow_rule(&m);
                                eprintln!("Saved to settings: {m}");
                                true
                            }
                            Approval::No => false,
                            Approval::Feedback(msg) => {
                                denied_reason = Some(format!("Tool call denied by user: {msg}"));
                                false
                            }
                        }
                    } else {
                        false
                    }
                }
            };

            let result = if !allowed {
                ToolResult::err(denied_reason.take().unwrap_or_else(|| {
                    "Tool call denied by user and was not executed. The tool call was denied by the harness. Do not retry it unchanged; use a permitted alternative or report the blocker.".into()
                }))
            } else if call.input.is_null() && call.name != "TodoWrite" {
                ToolResult::err("Tool arguments were not valid JSON; re-issue the call with valid JSON arguments.")
            } else {
                crate::tools::execute(ctx, &call.name, &call.input).await
            };

            let text = tool_result_text(&result);
            on_event(AgentEvent::ToolEnd {
                id: call.id.clone(),
                name: call.name.clone(),
                result: text.clone(),
                is_error: result.is_error,
            });
            let mut blocks = vec![Block::ToolResult {
                id: call.id.clone(),
                name: call.name.clone(),
                content: text,
                is_error: result.is_error,
            }];
            for (media, data) in result.images {
                blocks.push(Block::Image { media_type: media, data });
            }
            result_blocks.extend(blocks);
        }

        messages.push(Message {
            role: Role::User,
            blocks: result_blocks,
        });

        if runtime.auto_compact {
            maybe_compact(messages, &runtime, usage);
        }
    }
}

fn maybe_compact(messages: &mut Vec<Message>, runtime: &Runtime, usage: Usage) {
    let limit = runtime.model.context_limit.unwrap_or(200_000);
    let used = usage.input_tokens + usage.output_tokens;
    if used as f64 <= 0.85 * limit as f64 || messages.len() < 8 {
        return;
    }
    // Keep the first user message plus the most recent messages, and say so.
    let first = messages.first().cloned();
    let tail: Vec<Message> = messages[messages.len().saturating_sub(6)..].to_vec();
    *messages = Vec::new();
    if let Some(f) = first {
        messages.push(f);
    }
    messages.push(Message::user_text(
        "<system-reminder>\nEarlier conversation was compacted to fit the context window; continue from the recent messages below.\n</system-reminder>",
    ));
    messages.extend(tail);
}

enum Approval {
    Once,
    Session,
    Always,
    No,
    Feedback(String),
}

async fn ask_permission(tool: &str, arg: &str) -> Approval {
    use std::io::{BufRead, Write};
    let matcher = crate::tools::permission::generalize_matcher(tool, arg);
    let detail = crate::util::truncate(arg, 300, "…");
    eprintln!(
        "\nDo you want to proceed?\n  {tool}: {detail}\n\n  1 Yes\n  2 Yes, allow `{matcher}` for this session\n  3 Yes, always allow this (save to settings)\n  4 No\n  5 No, and tell Ante what to do differently\n> "
    );
    let _ = std::io::stderr().flush();
    let stdin = std::io::stdin();
    let mut line = String::new();
    if stdin.lock().read_line(&mut line).is_err() {
        return Approval::No;
    }
    match line.trim() {
        "1" | "y" | "yes" | "" => Approval::Once,
        "2" | "s" | "session" => Approval::Session,
        "3" | "a" | "always" => Approval::Always,
        "5" | "f" | "feedback" => {
            eprint!("What should Ante do differently? > ");
            let _ = std::io::stderr().flush();
            let mut reason = String::new();
            if stdin.lock().read_line(&mut reason).is_err() {
                return Approval::No;
            }
            let reason = reason.trim().chars().take(2000).collect::<String>();
            if reason.is_empty() {
                Approval::No
            } else {
                Approval::Feedback(reason)
            }
        }
        _ => Approval::No,
    }
}

pub async fn agent_tool(ctx: &crate::tools::ToolCtx, input: &Value) -> ToolResult {
    if ctx.depth >= 3 {
        return ToolResult::err("subagents cannot be nested deeper than 3 levels");
    }
    let subagent_type = input
        .get("subagent_type")
        .and_then(|s| s.as_str())
        .unwrap_or("general");
    let prompt = match input.get("prompt").and_then(|p| p.as_str()) {
        Some(p) => p,
        None => return ToolResult::err("`prompt` is required"),
    };

    let mut tools = if subagent_type == "explore" {
        vec!["Glob", "Grep", "Read", "Bash"]
            .into_iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    } else {
        ctx.runtime
            .tool_names
            .iter()
            .filter(|t| t.as_str() != "Agent")
            .cloned()
            .collect()
    };
    if subagent_type == "explore" {
        tools.retain(|t| t != "Write" && t != "Edit");
    }

    let mut child = (*ctx.runtime).clone();
    child.tool_names = tools;
    child.interactive = false;
    child.system_override = Some(if subagent_type == "explore" {
        "You are Ante's codebase exploration specialist. Answer the assigned question without changing state.\n\nYou have been invoked in the following environment:".to_string()
    } else {
        "You are a general-purpose subagent. Complete the assigned task and return a concise final report.".to_string()
    });
    let rt = Arc::new(child);

    let mut messages = vec![Message::user_text(prompt.to_string())];
    let mut child_ctx = crate::tools::ToolCtx::new(rt.clone(), ctx.cwd.clone());
    child_ctx.depth = ctx.depth + 1;
    let outcome = run_turn(&rt, &mut messages, &child_ctx, 12, |_| {}).await;
    let mut text = outcome.final_text;
    if let Some(err) = outcome.error {
        text.push_str(&format!("\n\n(subagent error: {err})"));
        return ToolResult {
            content: text,
            is_error: true,
            images: Vec::new(),
        };
    }
    ToolResult::ok(text)
}
