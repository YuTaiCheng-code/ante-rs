use std::io::{BufRead, Write};
use std::sync::Arc;

use crate::agent::{self, AgentEvent};
use crate::llm::Message;
use crate::runtime::Runtime;
use crate::session::Session;

pub async fn run(rt: Arc<Runtime>, save: bool) -> i32 {
    let mut session = Session::new(rt.provider_id.clone(), rt.model.id.clone(), rt.short_prompt);
    session.system = rt.system_sections();
    let mut messages: Vec<Message> = session.messages.clone();
    let ctx = crate::tools::ToolCtx::new(rt.clone(), rt.cwd.clone());
    let mut goal: Option<crate::goal::Goal> = None;

    welcome(&rt);
    let skills = crate::skills::discover(&rt.cwd);

    loop {
        print!("\n❯ ");
        let _ = std::io::stdout().flush();
        let mut raw = String::new();
        // 每次读取都重新加锁，避免长期占用 stdin 导致审批提示读不到下一行
        if std::io::stdin().read_line(&mut raw).is_err() || raw.is_empty() {
            break;
        }
        let input = raw.trim().to_string();
        if input.is_empty() {
            continue;
        }
        if let Some(cmdline) = input.strip_prefix('/') {
            let (cmd, arg) = cmdline.split_once(' ').unwrap_or((cmdline, ""));
            match cmd {
                "exit" | "quit" => break,
                "clear" => {
                    messages.clear();
                    println!("Conversation cleared.");
                }
                "compact" => {
                    let kept = messages.len().min(4);
                    let tail: Vec<Message> = messages[messages.len() - kept..].to_vec();
                    messages = vec![Message::user_text(
                        "<system-reminder>Earlier conversation was compacted to save context.</system-reminder>",
                    )];
                    messages.extend(tail);
                    println!("Conversation compacted.");
                }
                "context" => {
                    let used: usize = messages.iter().map(|m| m.text().len()).sum::<usize>() / 4;
                    println!(
                        "Context: ~{used} tokens used of {} limit; {} messages.",
                        rt.model.context_limit.unwrap_or(0),
                        messages.len()
                    );
                }
                "usage" => {
                    println!(
                        "Session usage: {} input / {} output tokens",
                        session.meta.usage.input_tokens, session.meta.usage.output_tokens
                    );
                }
                "models" => {
                    if arg.is_empty() {
                        for (id, p) in &rt.catalog.providers {
                            println!("{id} ({})", p.display_name);
                            for m in &p.preferred_models {
                                println!("  {}{}", m.id, if m.id == rt.model.id { "  *" } else { "" });
                            }
                        }
                    }
                }
                "model" => {
                    if arg.is_empty() {
                        println!("{}", rt.model.id);
                    } else {
                        println!("Model switches apply to the next session: {}", arg);
                    }
                }
                "providers" => {
                    for (id, p) in &rt.catalog.providers {
                        println!("{:<22} {:<10} {}", id, p.wire_style, p.base_url);
                    }
                }
                "config" => {
                    println!("{}", crate::settings::user_settings_path().display());
                }
                "init" => {
                    let path = rt.cwd.join("AGENTS.md");
                    if path.exists() {
                        println!("AGENTS.md already exists at {}", path.display());
                    } else {
                        let _ = std::fs::write(&path, "# Project instructions\n\n- Add repository conventions here; Ante reads this file at session start.\n");
                        println!("Created {}", path.display());
                    }
                }
                "diff" => {
                    let out = std::process::Command::new("git")
                        .args(["diff", "--stat"])
                        .current_dir(&rt.cwd)
                        .output();
                    match out {
                        Ok(o) if o.status.success() => print!("{}", String::from_utf8_lossy(&o.stdout)),
                        _ => println!("Not a git repository (or git is unavailable)."),
                    }
                }
                "rollover" => {
                    let handoff = crate::goal::handoff(&rt, &messages, arg).await;
                    session.messages = messages.clone();
                    let _ = session.save();
                    let old_id = session.meta.id.clone();
                    let mut next = Session::new(rt.provider_id.clone(), rt.model.id.clone(), rt.short_prompt);
                    next.meta.rolled_from = Some(old_id.clone());
                    next.system = rt.system_sections();
                    let intro = format!(
                        "This session is continued from {old_id} via a handoff.\nHandoff instructions: {}\n\n{handoff}", if arg.is_empty() { "(none)" } else { arg }
                    );
                    next.messages = vec![Message::user_text(intro.clone())];
                    messages.clear();
                    messages.push(Message::user_text(intro));
                    let new_id = next.meta.id.clone();
                    let _ = next.save();
                    session = next;
                    println!("Rollover: {new_id} (from {old_id})");
                }
                "rename" => {
                    if arg.is_empty() {
                        println!("Usage: /rename <title>");
                    } else {
                        session.meta.first_user_message = arg.to_string();
                        println!("Session renamed to: {arg}");
                    }
                }
                "mcp" => {
                    if rt.settings.mcp_servers.is_empty() {
                        println!("No MCP servers configured. Add them under `mcp_servers` in settings.json.");
                    } else {
                        for (name, cfg) in &rt.settings.mcp_servers {
                            println!("{name}: {}", crate::util::truncate(&cfg.to_string(), 200, "…"));
                        }
                    }
                }
                "goal" => {
                    let (line, _) = crate::goal::Goal::command(arg, &mut goal);
                    println!("{line}");
                }
                "welcome" => welcome(&rt),
                "fork" => {
                    let id = format!("ses_{}", crate::util::ulid());
                    println!("Forked session: {id}");
                    println!("The fork starts from the current conversation; resume it with `ante --resume {id}`.");
                }
                "rewind" => {
                    let prompts = messages
                        .iter()
                        .filter(|m| m.role == crate::llm::Role::User)
                        .count();
                    println!("Rewind points: {prompts} user prompt(s). Use `ante --resume` to reopen an earlier session.");
                }
                "offline-mode" => {
                    println!(
                        "Offline config: {}",
                        crate::util::ante_home().join("offline-config.json").display()
                    );
                    println!("Install the local engine with `ante offline install`.");
                }
                "term" => {
                    println!("{}", crate::tools::term::command(arg, &rt.cwd));
                }
                "theme" | "statusline" | "update" => {
                    println!("/{cmd} is not available in this build.");
                }
                "resume" | "import-claude" => {
                    println!("Session resume is available from the CLI: `ante --resume <session-id>`.");
                }
                _ => {
                    if let Some(skill) = skills.iter().find(|s| s.name == cmd) {
                        let body = crate::skills::body(skill).unwrap_or_default();
                        if body.is_empty() {
                            println!("Skill '{cmd}' has no readable SKILL.md body.");
                        } else {
                            let msg = format!(
                                "Follow this skill's instructions. Arguments: {arg}\n\n{body}"
                            );
                            messages.push(Message::user_text(msg));
                            turn(&rt, &mut messages, &ctx, &mut session).await;

    // 目标闭环：每轮结束后由判定器决定是否继续。
    while let Some(g) = goal.clone() {
        let verdict = crate::goal::evaluate(&rt, &messages, &g.condition).await;
        let (line, active) = crate::goal::apply(&mut goal, verdict);
        println!("{line}");
        if !active {
            break;
        }
        let condition = goal.as_ref().map(|x| x.condition.clone()).unwrap_or_default();
        messages.push(Message::user_text(format!(
            "The goal condition is still not satisfied. Continue working toward: {condition}"
        )));
        turn(&rt, &mut messages, &ctx, &mut session).await;
    }
                        }
                    } else {
                        println!("Unknown command: /{cmd}");
                    }
                }
            }
            continue;
        }

        let text = format!("{}\n\n{}", input, rt.folder_structure());
        session.note_user_message(&input);
        messages.push(Message::user_text(text));
        turn(&rt, &mut messages, &ctx, &mut session).await;

    // 目标闭环：每轮结束后由判定器决定是否继续。
    while let Some(g) = goal.clone() {
        let verdict = crate::goal::evaluate(&rt, &messages, &g.condition).await;
        let (line, active) = crate::goal::apply(&mut goal, verdict);
        println!("{line}");
        if !active {
            break;
        }
        let condition = goal.as_ref().map(|x| x.condition.clone()).unwrap_or_default();
        messages.push(Message::user_text(format!(
            "The goal condition is still not satisfied. Continue working toward: {condition}"
        )));
        turn(&rt, &mut messages, &ctx, &mut session).await;
    }
        if save {
            session.messages = messages.clone();
            let _ = session.save();
        }
    }
    if save {
        session.messages = messages.clone();
        let _ = session.save();
        println!("Session saved: {}", session.meta.id);
    }
    0
}

fn welcome(rt: &Arc<Runtime>) {
    let art = [
        "  ▄   ▀  ▄██▄  ▀   ▄  ",
        "  █  ▄█▄██████▄█▄  █  ",
        "  ▐█▄████████████▄█▌  ",
        "  ███▌ ████ ▐███       ",
        "  ████████████         ",
        "  ▀██▀▀▀▀██▀            ",
    ];
    let meta = format!("Ante v{}", crate::VERSION);
    let who = format!("{} · {}", rt.model.id, rt.provider_id);
    let cwd = rt.cwd.display().to_string();
    println!();
    for (i, line) in art.iter().enumerate() {
        let right = match i {
            2 => meta.clone(),
            3 => who.clone(),
            4 => cwd.clone(),
            _ => String::new(),
        };
        println!("{line}   {right}");
    }
    println!();
    println!("  not connected — run /providers to sign in");
    println!("  type /exit to quit · Ctrl-D ends the session");
}

async fn turn(
    rt: &Arc<Runtime>,
    messages: &mut Vec<Message>,
    ctx: &crate::tools::ToolCtx,
    session: &mut Session,
) {
    let mut printed = String::new();
    let outcome = agent::run_turn(rt, messages, ctx, 32, |ev| match ev {
        AgentEvent::TextDelta(t) => {
            if printed.is_empty() {
                println!("\n❖ Ante");
                print!("  ");
            }
            print!("{}", t.replace('\n', "\n  "));
            let _ = std::io::stdout().flush();
            printed.push_str(&t);
        }
        AgentEvent::ToolStart { name, .. } => {
            println!("\n✓ {name}");
        }
        AgentEvent::Usage(u) => {
            session.meta.usage.input_tokens += u.input_tokens;
            session.meta.usage.output_tokens += u.output_tokens;
        }
        _ => {}
    })
    .await;
    println!();
    if let Some(err) = outcome.error {
        eprintln!("Task failed: {err}");
    }
}
