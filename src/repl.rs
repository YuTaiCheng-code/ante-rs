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
                    let handoff = crate::goal::handoff(&rt, &messages, arg).await;
                    let kept = messages.len().min(4);
                    let tail: Vec<Message> = messages[messages.len() - kept..].to_vec();
                    let mut compacted = vec![Message::user_text(format!(
                        "<system-reminder>Earlier conversation was compacted. Handoff:\n{handoff}\n</system-reminder>"
                    ))];
                    compacted.extend(tail);
                    messages = compacted;
                    session.messages = messages.clone();
                    let _ = session.save();
                    println!("Conversation compacted with a handoff summary.");
                }
                "context" => {
                    let used = crate::llm::estimate_tokens(&messages);
                    let limit = rt.model.context_limit.unwrap_or(0);
                    let pct = if limit > 0 {
                        format!(" ({:.1}%)", used as f64 / limit as f64 * 100.0)
                    } else {
                        String::new()
                    };
                    println!(
                        "Context estimate: ~{used} tokens{pct} of {} limit; {} messages.",
                        if limit == 0 { "unknown".to_string() } else { limit.to_string() },
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
                        let _ = session.save();
                        println!("Session renamed to: {arg}");
                    }
                }
                "memory" => {
                    println!("{}", crate::memory::command(arg));
                }
                "jobs" => {
                    let jobs = rt.jobs.list();
                    if jobs.is_empty() {
                        println!("No background jobs.");
                    } else {
                        for job in jobs {
                            println!(
                                "{} pid={} running={} exit={}",
                                job.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                                job.get("pid").and_then(|v| v.as_u64()).unwrap_or(0),
                                job.get("running").and_then(|v| v.as_bool()).unwrap_or(false),
                                job.get("exit_code").and_then(|v| v.as_i64()).map(|n| n.to_string()).unwrap_or_else(|| "-".into())
                            );
                        }
                    }
                }
                "mcp" => {
                    if rt.settings.mcp_servers.is_empty() {
                        println!("No MCP servers configured. Add them under `mcp_servers` in settings.json.");
                    } else {
                        for (name, cfg) in &rt.settings.mcp_servers {
                            println!("{name}: {}", crate::util::truncate(&cfg.to_string(), 200, "…"));
                        }
                        let tools = rt.mcp.tool_schemas();
                        if tools.is_empty() {
                            println!("No MCP tools discovered (server may have failed to start).");
                        } else {
                            println!("Connected tools:");
                            for tool in tools {
                                if let Some(name) = tool.get("name").and_then(|v| v.as_str()) {
                                    println!("  {name}");
                                }
                            }
                        }
                    }
                }
                "goal" => {
                    let (line, _) = crate::goal::Goal::command(arg, &mut goal);
                    println!("{line}");
                }
                "welcome" => welcome(&rt),
                "fork" => {
                    let points = session.snapshot_numbers();
                    let number = arg.trim().parse::<usize>().ok().or_else(|| points.last().copied());
                    match number.map(|n| session.fork_from(n)) {
                        Some(Ok(fork)) => {
                            let id = fork.meta.id.clone();
                            session = fork;
                            messages = session.messages.clone();
                            println!("Forked session: {id} (continuing in the fork)");
                        }
                        Some(Err(e)) => println!("Fork failed: {e}"),
                        None => println!("No snapshot yet. Send a prompt first."),
                    }
                }
                "rewind" => {
                    let points = session.snapshot_numbers();
                    if arg.is_empty() {
                        println!("Rewind points: {}", points.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", "));
                        if let Some(last) = points.last() {
                            println!("Use `/rewind {last}` to restore the latest prompt boundary.");
                        }
                    } else {
                        match arg.trim().parse::<usize>() {
                            Ok(n) => match session.load_snapshot(n) {
                                Ok(restored) => {
                                    messages = restored;
                                    session.messages = messages.clone();
                                    let _ = session.save();
                                    println!("Rewound to snapshot {n} ({} messages).", messages.len());
                                }
                                Err(e) => println!("Rewind failed: {e}"),
                            },
                            Err(_) => println!("Usage: /rewind [snapshot-number]"),
                        }
                    }
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
                "resume" => {
                    if arg.is_empty() {
                        let sessions = Session::list();
                        if sessions.is_empty() {
                            println!("No saved sessions.");
                        } else {
                            for meta in sessions.iter().take(12) {
                                println!("{}  {}  {}", meta.id, meta.last_active_time, crate::util::truncate(&meta.first_user_message, 60, "…"));
                            }
                            println!("Use `/resume <session-id>`.");
                        }
                    } else {
                        match Session::load(arg.trim()) {
                            Ok(mut loaded) => {
                                messages = loaded.messages.clone();
                                loaded.system = rt.system_sections();
                                session = loaded;
                                println!("Resumed {}.", session.meta.id);
                            }
                            Err(e) => println!("Resume failed: {e}"),
                        }
                    }
                }
                "import-claude" => {
                    println!("{}", crate::memory::import_claude(&rt.cwd));
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
        session.messages = messages.clone();
        let snapshot_number = session
            .snapshot_numbers()
            .last()
            .copied()
            .unwrap_or(0)
            + 1;
        if let Err(e) = session.save_snapshot(snapshot_number) {
            eprintln!("warning: failed to save rewind snapshot: {e}");
        }
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
