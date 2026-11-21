use serde_json::{json, Value};
use std::io::Write;
use std::sync::Arc;

use crate::agent::{self, AgentEvent};
use crate::llm::Message;
use crate::runtime::Runtime;
use crate::session::Session;
use crate::util::now_rfc3339;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Human,
    Minimal,
}

impl OutputFormat {
    pub fn parse(s: &str) -> OutputFormat {
        match s {
            "json" => OutputFormat::Json,
            "human" => OutputFormat::Human,
            _ => OutputFormat::Minimal,
        }
    }
}

pub struct HeadlessOptions {
    pub prompt: String,
    pub format: OutputFormat,
    pub verify: bool,
    pub session_save: bool,
}

pub fn emit_event(parent: &str, event: Value) {
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

pub async fn run(rt: Arc<Runtime>, opts: HeadlessOptions) -> i32 {
    let op_id = format!("op_{}", crate::util::ulid());
    let format = opts.format;
    let save = opts.session_save && rt.settings.session_save();
    let mut session = Session::new(rt.provider_id.clone(), rt.model.id.clone(), rt.short_prompt);
    session.system = rt.system_sections();

    let skills = crate::skills::discover(&rt.cwd);
    let agents: Vec<Value> = serde_json::from_str(crate::runtime::BUILTIN_AGENTS).unwrap_or_default();

    if format == OutputFormat::Json {
        emit_event(
            &op_id,
            json!({
                "SessionStart": {
                    "model": rt.describe(),
                    "provider": rt.provider_json(),
                    "session_id": session.meta.id,
                    "cwd": rt.cwd.display().to_string(),
                    "permission_mode": rt.permission_mode.as_str(),
                    "skills": skills.iter().map(|s| json!({
                        "name": s.name, "description": s.description, "scope": s.scope, "argument_hint": s.argument_hint
                    })).collect::<Vec<_>>(),
                    "subagents": agents,
                }
            }),
        );
    }

    let prompt_with_context = if rt.interactive {
        opts.prompt.clone()
    } else {
        format!("{}\n\n{}", opts.prompt, rt.folder_structure())
    };
    session.note_user_message(&opts.prompt);

    if format == OutputFormat::Json {
        emit_event(&op_id, json!({"UserInput": prompt_with_context}));
        emit_event(&op_id, json!({"TurnStart": {"turn_id": op_id}}));
    }

    let mut messages: Vec<Message> = session.messages.clone();
    messages.push(Message::user_text(prompt_with_context));
    let ctx = crate::tools::ToolCtx::new(rt.clone(), rt.cwd.clone());

    let mut printed_text = String::new();
    let outcome = agent::run_turn(&rt, &mut messages, &ctx, 32, |ev| match ev {
        AgentEvent::TextDelta(t) => match format {
            OutputFormat::Json => {}
            OutputFormat::Human | OutputFormat::Minimal => {
                if printed_text.is_empty() {
                    println!();
                    println!("❖ Ante");
                    print!("  ");
                }
                print!("{}", t.replace('\n', "\n  "));
                let _ = std::io::stdout().flush();
                printed_text.push_str(&t);
            }
        },
        AgentEvent::AssistantMessage(text) => {
            if format == OutputFormat::Json {
                emit_event(&op_id, json!({"AgentMessage": text}));
            } else if format == OutputFormat::Human {
                println!();
                println!();
                println!("✓ Done");
            }
            printed_text.clear();
        }
        AgentEvent::ToolStart { id, name, args } => {
            if format == OutputFormat::Json {
                emit_event(&op_id, json!({"ToolStart": {"id": id, "name": name, "args": args}}));
            } else if format == OutputFormat::Human {
                println!("");
                println!("✓ {}", name);
            }
        }
        AgentEvent::ToolEnd { id, name, result, is_error } => {
            if format == OutputFormat::Json {
                emit_event(
                    &op_id,
                    json!({"ToolEnd": {
                        "tool_use_id": id,
                        "tool_name": name,
                        "status": if is_error { "Error" } else { "Completed" },
                        "result_json": if is_error { json!({"Error": result}) } else { json!({"Completed": result}) }
                    }}),
                );
            }
        }
        AgentEvent::Usage(u) => {
            if format == OutputFormat::Json {
                emit_event(
                    &op_id,
                    json!({"UsageUpdate": {
                        "usage": {"input_tokens": u.input_tokens, "output_tokens": u.output_tokens},
                        "context": {
                            "used_tokens": u.input_tokens + u.output_tokens,
                            "limit_tokens": rt.model.context_limit.unwrap_or(0)
                        }
                    }}),
                );
            }
            session.meta.usage.input_tokens += u.input_tokens;
            session.meta.usage.output_tokens += u.output_tokens;
        }
        _ => {}
    })
    .await;

    session.messages = messages.clone();
    if let Some(err) = &outcome.error {
        let parts: Vec<&str> = err.split('|').collect();
        let (kind, headline, details) = if parts.len() >= 3 {
            (
                parts[0].to_string(),
                parts[1].to_string(),
                vec![parts[2].to_string()],
            )
        } else {
            ("error".to_string(), "llm error".to_string(), vec![err.clone()])
        };
        if format == OutputFormat::Json {
            emit_event(
                &op_id,
                json!({"TurnEnd": {
                    "turn_id": op_id,
                    "status": {"Error": {"kind": kind, "headline": headline, "details": details}},
                    "steps": outcome.steps
                }}),
            );
        } else {
            eprintln!("\nTask failed: {}", headline);
            for d in details {
                eprintln!("  {d}");
            }
        }
    } else {
        if format == OutputFormat::Json {
            emit_event(
                &op_id,
                json!({"TurnEnd": {"turn_id": op_id, "status": "Completed", "steps": outcome.steps}}),
            );
        } else if format == OutputFormat::Human {
            // final text was already streamed; nothing else to print
        } else if !outcome.final_text.is_empty() && printed_text.is_empty() {
            println!();
            println!("❖ Ante");
            for line in outcome.final_text.lines() {
                println!("  {line}");
            }
        }
        if format != OutputFormat::Json {
            println!();
        }
    }

    if save {
        let _ = session.save();
    }

    if opts.verify {
        let verify_prompt = "Review the work you just completed. Verify it with the narrowest check that directly exercises it (a test, the command, a schema check), and report what passed and what remains unverified.";
        let mut verify_messages = messages.clone();
        verify_messages.push(Message::user_text(verify_prompt));
        let outcome = agent::run_turn(&rt, &mut verify_messages, &ctx, 12, |_| {}).await;
        if format == OutputFormat::Json {
            emit_event(&op_id, json!({"AgentMessage": outcome.final_text}));
        } else {
            println!("{}", outcome.final_text);
        }
    }

    if outcome.error.is_some() {
        1
    } else {
        0
    }
}
