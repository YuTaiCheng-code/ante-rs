//! 全屏 TUI 前端：复刻原件 120x40 首屏布局、命令面板与输入框。

use std::io::Write;
use std::sync::Arc;

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute, queue,
    style::Print,
    terminal::{self, ClearType},
};

use crate::llm::Message;
use crate::runtime::Runtime;
use crate::session::Session;

const COMMANDS: &[(&str, &str)] = &[
    ("add-provider", "✨[s] Add or update a custom model provider in Ante's catalog.json — an OpenAI-compatible endpoint"),
    ("clear", "Clear conversation history"),
    ("compact", "Summarize conversation history to free context window"),
    ("config", "Open settings"),
    ("context", "Visualize current context usage as a colored grid"),
    ("diff", "Show git diff (including untracked files)"),
    ("exit", "Alternative command to quit the application"),
    ("fork", "Fork the conversation into a new session from an earlier prompt"),
    ("goal", "Keep working until a condition is met (judged after each turn)"),
    ("import-claude", "Import Claude Code memory for this project"),
    ("jobs", "List background jobs and their output paths"),
    ("init", "Initialize this project with AGENTS.md"),
    ("mcp", "Manage MCP servers"),
    ("memory", "Show or manage persistent auto memory"),
    ("model", "Select a model and effort"),
    ("models", "Select a model and effort"),
    ("offline-mode", "Use a local model with the local inference engine"),
    ("providers", "Choose a provider or manage accounts"),
    ("rename", "Name this session"),
    ("resume", "Resume a previous session"),
    ("rewind", "Rewind the conversation to an earlier prompt"),
    ("statusline", "Configure the status line"),
    ("term", "Manage terminal splits"),
    ("theme", "Open theme selection dialog"),
    ("update", "Install the pending Ante update on exit"),
    ("usage", "Show session token usage by model"),
    ("welcome", "Show the welcome/onboarding dialog"),
];

enum Overlay {
    Theme(usize),
    Providers(usize),
    Models,
}

const RESET: &str = "\x1b[0m";

fn fg(r: u8, g: u8, b: u8) -> String {
    format!("\x1b[38;2;{r};{g};{b}m")
}

/// 原件 Logo 是按列插值的橙色渐变。
fn art_color(x: u16) -> (u8, u8, u8) {
    match x {
        2 => (255, 175, 0),
        3 => (215, 175, 0),
        4..=7 => (215, 135, 0),
        8..=12 => (175, 135, 0),
        13..=19 => (175, 95, 0),
        _ => (215, 135, 0),
    }
}

pub struct Tui {
    rt: Arc<Runtime>,
    input: String,
    messages: Vec<String>,
    banner: bool,
    overlay: Option<Overlay>,
    history: Vec<Message>,
    session: Session,
    ctx: crate::tools::ToolCtx,
    goal: Option<crate::goal::Goal>,
}

impl Tui {
    pub fn new(rt: Arc<Runtime>) -> Tui {
        let mut session = Session::new(rt.provider_id.clone(), rt.model.id.clone(), rt.short_prompt);
        session.system = rt.system_sections();
        let ctx = crate::tools::ToolCtx::new(rt.clone(), rt.cwd.clone());
        Tui {
            rt,
            input: String::new(),
            messages: Vec::new(),
            banner: true,
            overlay: None,
            history: Vec::new(),
            session,
            ctx,
            goal: None,
        }
    }

    async fn submit(&mut self, line: String) {
        let text = format!("{}\n\n{}", line, self.rt.folder_structure());
        self.history.push(Message::user_text(text));
        self.session.note_user_message(&line);
        self.session.messages = self.history.clone();
        let snapshot_number = self
            .session
            .snapshot_numbers()
            .last()
            .copied()
            .unwrap_or(0)
            + 1;
        let _ = self.session.save_snapshot(snapshot_number);

        self.run_turn_once(Some(&line)).await;

        while let Some(g) = self.goal.clone() {
            let verdict = crate::goal::evaluate(&self.rt, &self.history, &g.condition).await;
            let (line, active) = crate::goal::apply(&mut self.goal, verdict);
            self.messages.push(line);
            if !active {
                break;
            }
            let condition = self
                .goal
                .as_ref()
                .map(|x| x.condition.clone())
                .unwrap_or_default();
            self.history.push(Message::user_text(format!(
                "The goal condition is still not satisfied. Continue working toward: {condition}"
            )));
            self.run_turn_once(None).await;
        }
    }

    async fn run_turn_once(&mut self, announce: Option<&str>) {
        let mut out = std::io::stdout();
        let _ = execute!(out, cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
        if let Some(line) = announce {
            println!("\n❯ {line}");
        }
        let mut rendered = String::new();
        let outcome = crate::agent::run_turn(
            &self.rt,
            &mut self.history,
            &self.ctx,
            32,
            |event| match event {
                crate::agent::AgentEvent::TextDelta(t) => {
                    print!("{t}");
                    let _ = std::io::stdout().flush();
                    rendered.push_str(&t);
                }
                crate::agent::AgentEvent::ToolStart { name, .. } => {
                    println!("\n✓ {name}");
                }
                crate::agent::AgentEvent::Usage(u) => {
                    self.session.meta.usage.input_tokens += u.input_tokens;
                    self.session.meta.usage.output_tokens += u.output_tokens;
                }
                _ => {}
            },
        )
        .await;
        println!();
        if let Some(err) = outcome.error {
            eprintln!("Task failed: {err}");
        }
        if !rendered.is_empty() {
            self.messages.push(format!("❖ Ante  {rendered}"));
        }
        self.session.messages = self.history.clone();
        let _ = self.session.save();

        let _ = terminal::enable_raw_mode();
        let _ = execute!(out, terminal::EnterAlternateScreen, cursor::Hide);
    }

    fn cols(&self) -> u16 {
        std::env::var("COLUMNS").ok().and_then(|v| v.parse().ok())
            .or_else(|| terminal::size().ok().map(|(w, _)| w))
            .unwrap_or(120)
    }

    fn rows(&self) -> u16 {
        std::env::var("LINES").ok().and_then(|v| v.parse().ok())
            .or_else(|| terminal::size().ok().map(|(_, h)| h))
            .unwrap_or(40)
    }

    /// 原件首屏：左侧字标 + 右侧版本/模型/目录，随后是输入区。
    fn draw(&self) -> std::io::Result<()> {
        let mut out = std::io::stdout();
        let cols = self.cols();
        let rows = self.rows();
        queue!(out, terminal::Clear(ClearType::All), cursor::MoveTo(0, 0))?;

        if self.banner && self.messages.is_empty() {
            let art = [
                (6u16, "▀  ▄██▄  ▀"),
                (5, "▄█▄██████▄█▄"),
                (2, "▐█▄████████████▄█▌"),
                (4, "███▌ ████ ▐███"),
                (5, "████████████"),
                (6, "▀██▀▀▀▀██▀"),
            ];
            for (i, (x, text)) in art.iter().enumerate() {
                queue!(out, cursor::MoveTo(*x, 1 + i as u16))?;
                let mut painted = String::new();
                for (j, ch) in text.chars().enumerate() {
                    let (r, g, b) = art_color(x + j as u16);
                    painted.push_str(&fg(r, g, b));
                    painted.push(ch);
                }
                painted.push_str(RESET);
                queue!(out, Print(painted))?;
            }
            queue!(
                out,
                cursor::MoveTo(24, 3),
                Print(format!("{}Ante v{}{RESET}", fg(255, 135, 95), crate::VERSION)),
                cursor::MoveTo(24, 4),
                Print(if self.rt.provider_id.is_empty() {
                    format!("{}not connected — run /providers to sign in{RESET}", fg(135, 175, 175))
                } else {
                    format!("{} · {}", self.rt.model.id, self.rt.provider_id)
                }),
                cursor::MoveTo(24, 5),
                Print(format!("{}{}{RESET}", fg(95, 95, 135), tail_path(&self.rt.cwd.display().to_string(), cols.saturating_sub(26) as usize))),
            )?;
        }

        if self.banner && self.messages.is_empty() {
            if let Some(lines) = update_notice() {
                // 更新提示块贴着输入框上方的分隔线向上排列，面板展开时随之上移
                let palette = self.input.starts_with('/') && self.overlay.is_none();
                let sep = if palette { rows.saturating_sub(23) } else { rows.saturating_sub(20) };
                let top = sep.saturating_sub(lines.len() as u16 + 1);
                for (i, line) in lines.iter().enumerate() {
                    queue!(out, cursor::MoveTo(0, top + i as u16), Print(line))?;
                }
            }
        }

        // 消息区（从第 7 行开始，最多到输入框上方 3 行）
        let msg_top = if self.banner { 7 } else { 0 };
        let msg_bottom = rows.saturating_sub(5);
        let available = msg_bottom.saturating_sub(msg_top) as usize;
        let tail: Vec<&String> = self.messages.iter().rev().take(available).collect();
        for (i, line) in tail.iter().rev().enumerate() {
            queue!(
                out,
                cursor::MoveTo(2, msg_top + i as u16),
                Print(ellipsis(line, cols.saturating_sub(4) as usize))
            )?;
        }

        // 输入框：分隔线 + ❯ ▎ 提示（有面板时整体上移，面板向下展开）
        if self.overlay.is_none() {
            let palette_open = self.input.starts_with('/') && self.overlay.is_none();            let sep_y = if palette_open { rows.saturating_sub(23) } else { rows.saturating_sub(20) };            queue!(                out,                cursor::MoveTo(0, sep_y),                Print("─".repeat(cols as usize)),                cursor::MoveTo(0, sep_y + 1),                Print(if self.input.is_empty() {                    "  ❯ ▎ Find and fix a bug — describe what's going wrong"                } else {                    ""                }),                Print(if self.input.is_empty() { "" } else { "  ❯ " }),                Print(if self.input.is_empty() { "" } else { &self.input }),                Print(if self.input.is_empty() { "" } else { "▎" }),                cursor::MoveTo(4 + self.input.chars().count() as u16, sep_y + 1),                cursor::MoveTo(0, sep_y + 2),                Print("─".repeat(cols as usize)),            )?;            if palette_open {                self.draw_palette(sep_y)?;                queue!(                    out,                    cursor::MoveTo(2, rows.saturating_sub(10)),                    Print(format!("{}({}) · {} · {}", self.rt.model.id, self.rt.effort.clone().unwrap_or_else(|| "medium".into()), self.rt.provider_id, self.rt.cwd.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default())),                    cursor::MoveTo(2, rows.saturating_sub(9)),                    Print("normal permission mode (shift+tab to cycle)"),                )?;            } else {                queue!(out, cursor::MoveTo(2, sep_y + 3), Print("Press Ctrl+C again to exit"))?;            }
        }

        if self.overlay.is_some() {
            self.draw_overlay()?;
        }

        out.flush()
    }

    fn placeholder_marker(&self) {}

    fn draw_palette(&self, sep_y: u16) -> std::io::Result<()> {
        let mut out = std::io::stdout();
        let query = self.input.trim_start_matches('/');
        let matches: Vec<&(&str, &str)> = COMMANDS
            .iter()
            .filter(|(name, _)| name.starts_with(query))
            .collect();
        let status_y = self.rows().saturating_sub(10);
        let top = sep_y + 3;
        let height = status_y.saturating_sub(top) as usize;
        for (i, (name, desc)) in matches.iter().take(height).enumerate() {
            let prefix = if i == 0 { "  → " } else { "    " };
            let prefix_color = if i == 0 {
                fg(215, 95, 95)
            } else {
                fg(95, 95, 135)
            };
            let name_color = if i == 0 {
                fg(255, 135, 95)
            } else {
                fg(135, 175, 175)
            };
            let desc_color = if i == 0 {
                fg(255, 215, 175)
            } else {
                fg(160, 160, 160)
            };
            let desc = ellipsis(
                desc,
                self.cols().saturating_sub(name.len() as u16 + 10) as usize,
            );
            queue!(
                out,
                cursor::MoveTo(0, top + i as u16),
                Print(" ".repeat(self.cols() as usize)),
                cursor::MoveTo(0, top + i as u16),
                Print(format!(
                    "{prefix_color}{prefix}{RESET}{name_color}/{name}{RESET} {desc_color}- {desc}{RESET}"
                ))
            )?;
        }
        Ok(())
    }

    fn move_selection(&mut self, delta: i32) {
        let max = match &self.overlay {
            Some(Overlay::Theme(_)) => 5,
            Some(Overlay::Providers(_)) => self.rt.catalog.providers.len().saturating_sub(1),
            _ => return,
        } as i32;
        match &mut self.overlay {
            Some(Overlay::Theme(i)) => *i = (*i as i32 + delta).clamp(0, max) as usize,
            Some(Overlay::Providers(i)) => *i = (*i as i32 + delta).clamp(0, max) as usize,
            _ => {}
        }
    }

    fn draw_overlay(&self) -> std::io::Result<()> {
        let mut out = std::io::stdout();
        let width = (self.cols() as usize).saturating_sub(4);
        let border = |title: &str, w: usize| -> String {
            if title.is_empty() {
                format!("┌{}┐", "─".repeat(w.saturating_sub(2)))
            } else {
                format!("┌ {} {}┐", title, "─".repeat(w.saturating_sub(4 + title.chars().count())))
            }
        };
        let inner = |text: &str, w: usize| -> String {
            let n = text.chars().count();
            if n >= w.saturating_sub(2) {
                format!("│{}│", text.chars().take(w.saturating_sub(2)).collect::<String>())
            } else {
                format!("│{text}{}│", " ".repeat(w.saturating_sub(2) - n))
            }
        };
        let bottom = |w: usize| format!("└{}┘", "─".repeat(w.saturating_sub(2)));
        match self.overlay.as_ref().unwrap() {
            Overlay::Theme(sel) => {
                let y = self.rows().saturating_sub(20);
                let left = 46usize;
                let right = width.saturating_sub(left);
                let entries = [
                    "1. Dark mode ✓",
                    "2. Light mode",
                    "3. Dark mode (colorblind-friendly)",
                    "4. Light mode (colorblind-friendly)",
                    "5. Dark mode (ANSI colors only)",
                    "6. Light mode (ANSI colors only)",
                ];
                let sample = [
                    "12   function greet() {",
                    "13 -     console.log(\"Hello, World!\");",
                    "13 +     console.log(\"Hello, Ante!!\");",
                    "14   }",
                    "",
                    "",
                ];
                queue!(
                    out,
                    cursor::MoveTo(2, y),
                    Print(border("Theme", left)),
                    Print(border("Choose the text style that looks best with your terminal", right)),
                )?;
                for i in 0..6 {
                    let marker = if i == *sel { "›" } else { " " };
                    queue!(
                        out,
                        cursor::MoveTo(2, y + 1 + i as u16),
                        Print(inner(&format!("{marker} {}", entries[i]), left)),
                        Print(inner(sample[i], right)),
                    )?;
                }
                queue!(
                    out,
                    cursor::MoveTo(2, y + 7),
                    Print(bottom(left)),
                    Print(bottom(right)),
                    cursor::MoveTo(42, y + 8),
                    Print("↑↓ navigate  Enter select  Esc cancel")
                )?;
            }
            Overlay::Providers(sel) => {
                let y = self.rows().saturating_sub(27);
                let name = self
                    .rt
                    .catalog
                    .provider(&self.rt.provider_id)
                    .map(|p| p.display_name.clone())
                    .unwrap_or_default();
                let current = format!("Current: {name} · {}", self.rt.model.id);
                queue!(
                    out,
                    cursor::MoveTo(2, y),
                    Print(border("Providers", width)),
                    cursor::MoveTo(2, y + 1),
                    Print(inner(&format!(" {current}"), width)),
                )?;
                let mut items: Vec<(String, String)> = Vec::new();
                if let Some(p) = self.rt.catalog.provider(&self.rt.provider_id) {
                    items.push((p.display_name.clone(), cred_label(&p.credential)));
                }
                for (id, p) in &self.rt.catalog.providers {
                    if id == &self.rt.provider_id {
                        continue;
                    }
                    items.push((p.display_name.clone(), "— set up".into()));
                }
                for (i, (n, state)) in items.iter().take(20).enumerate() {
                    let row = if i == 0 {
                        format!("›✓ {n:<23} {state}")
                    } else if i == *sel {
                        format!("›  {n:<23} {state}")
                    } else {
                        format!("   {n:<23} {state}")
                    };
                    queue!(
                        out,
                        cursor::MoveTo(2, y + 2 + i as u16),
                        Print(inner(&format!(" {row}"), width)),
                    )?;
                }
                let bottom_y = y + 2 + items.len().min(20) as u16;
                queue!(
                    out,
                    cursor::MoveTo(2, bottom_y),
                    Print(bottom(width)),
                    cursor::MoveTo(23, bottom_y + 1),
                    Print("↑↓ Provider  ←→ Credential (* preview)  Enter Models  m Access  Esc Cancel")
                )?;
            }
            Overlay::Models => {
                let y = self.rows().saturating_sub(19);
                let name = self
                    .rt
                    .catalog
                    .provider(&self.rt.provider_id)
                    .map(|p| p.display_name.clone())
                    .unwrap_or_default();
                let title = format!("Switch Model — {name}");
                let rows_txt = [
                    format!("Current: {} with {}", self.rt.model.id, name),
                    format!("› ✓ {}  {}", self.rt.model.id, self.rt.model.description),
                    String::new(),
                    "Faster                                           Smarter".into(),
                    "────────────────────────────────────▲───────────────────".into(),
                    "min              low             medium             high".into(),
                ];
                queue!(out, cursor::MoveTo(2, y), Print(border(&title, width)))?;
                for (i, line) in rows_txt.iter().enumerate() {
                    queue!(
                        out,
                        cursor::MoveTo(2, y + 1 + i as u16),
                        Print(inner(&format!(" {line}"), width)),
                    )?;
                }
                queue!(
                    out,
                    cursor::MoveTo(2, y + 7),
                    Print(bottom(width)),
                    cursor::MoveTo(24, y + 8),
                    Print("[↑↓] Navigate  [←→] Effort  [Tab] Providers  [Enter] Select  [Esc] Cancel")
                )?;
            }
        }
        Ok(())
    }

    async fn handle_command(&mut self, line: &str) {
        let (cmd, arg) = line.trim_start_matches('/').split_once(' ').unwrap_or((line.trim_start_matches('/'), ""));
        match cmd {
            "exit" | "quit" => std::process::exit(0),
            "clear" => self.messages.clear(),
            "welcome" => {
                self.banner = true;
                self.messages.clear();
            }
            "config" => self.messages.push(format!(
                "settings: {}",
                crate::settings::user_settings_path().display()
            )),
            "providers" => self.overlay = Some(Overlay::Providers(0)),
            "models" | "model" => {
                if arg.is_empty() {
                    self.overlay = Some(Overlay::Models);
                    let models = self
                        .rt
                        .catalog
                        .provider(&self.rt.provider_id)
                        .map(|p| p.preferred_models.clone())
                        .unwrap_or_default();
                    for m in &models {
                        self.messages.push(format!("{}{}", m.id, if m.id == self.rt.model.id { "  *" } else { "" }));
                    }
                } else {
                    self.messages.push(format!("model: {arg} (applies to the next session)"));
                }
            }
            "usage" => self.messages.push(format!(
                "Session usage: {} input / {} output tokens",
                self.session.meta.usage.input_tokens, self.session.meta.usage.output_tokens
            )),
            "context" => {
                let used = crate::llm::estimate_tokens(&self.history);
                let limit = self.rt.model.context_limit.unwrap_or(0);
                let pct = if limit > 0 {
                    format!(" ({:.1}%)", used as f64 / limit as f64 * 100.0)
                } else {
                    String::new()
                };
                self.messages.push(format!(
                    "Context estimate: ~{used} tokens{pct} of {} limit; {} messages.",
                    if limit == 0 { "unknown".to_string() } else { limit.to_string() },
                    self.history.len()
                ));
            }
            "compact" => {
                let handoff = crate::goal::handoff(&self.rt, &self.history, arg).await;
                let kept = self.history.len().min(4);
                let tail: Vec<Message> =
                    self.history[self.history.len().saturating_sub(kept)..].to_vec();
                let mut compacted = vec![Message::user_text(format!(
                    "<system-reminder>Earlier conversation was compacted. Handoff:\n{handoff}\n</system-reminder>"
                ))];
                compacted.extend(tail);
                self.history = compacted;
                self.session.messages = self.history.clone();
                let _ = self.session.save();
                self.messages
                    .push("Conversation compacted with a handoff summary.".into());
            }
            "goal" => {
                let (line, _) = crate::goal::Goal::command(arg, &mut self.goal);
                self.messages.push(line);
            }
            "diff" => {
                let out = std::process::Command::new("git")
                    .args(["status", "--short"])
                    .current_dir(&self.rt.cwd)
                    .output();
                match out {
                    Ok(o) if o.status.success() => {
                        let status = String::from_utf8_lossy(&o.stdout);
                        let diff = std::process::Command::new("git")
                            .args(["diff", "--stat"])
                            .current_dir(&self.rt.cwd)
                            .output()
                            .ok()
                            .map(|d| String::from_utf8_lossy(&d.stdout).to_string())
                            .unwrap_or_default();
                        let text = format!("{status}{diff}");
                        self.messages.push(if text.trim().is_empty() {
                            "Working tree clean.".into()
                        } else {
                            text.trim_end().to_string()
                        });
                    }
                    _ => self.messages.push("Not a git repository (or git is unavailable).".into()),
                }
            }
            "mcp" => {
                if self.rt.settings.mcp_servers.is_empty() {
                    self.messages
                        .push("No MCP servers configured. Add them under `mcp_servers` in settings.json.".into());
                } else {
                    for (name, cfg) in &self.rt.settings.mcp_servers {
                        self.messages.push(format!(
                            "{name}: {}",
                            crate::util::truncate(&cfg.to_string(), 200, "…")
                        ));
                    }
                    let tools = self.rt.mcp.tool_schemas();
                    if tools.is_empty() {
                        self.messages
                            .push("No MCP tools discovered (server may have failed to start).".into());
                    } else {
                        for tool in tools {
                            if let Some(name) = tool.get("name").and_then(|v| v.as_str()) {
                                self.messages.push(format!("  {name}"));
                            }
                        }
                    }
                }
            }
            "jobs" => {
                let jobs = self.rt.jobs.list();
                if jobs.is_empty() {
                    self.messages.push("No background jobs.".into());
                } else {
                    for job in jobs {
                        self.messages.push(format!(
                            "{} pid={} running={} exit={}",
                            job.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                            job.get("pid").and_then(|v| v.as_u64()).unwrap_or(0),
                            job.get("running").and_then(|v| v.as_bool()).unwrap_or(false),
                            job.get("exit_code").and_then(|v| v.as_i64()).map(|n| n.to_string()).unwrap_or_else(|| "-".into())
                        ));
                    }
                }
            }
            "init" => {
                let path = self.rt.cwd.join("AGENTS.md");
                if path.exists() {
                    self.messages
                        .push(format!("AGENTS.md already exists at {}", path.display()));
                } else {
                    match std::fs::write(
                        &path,
                        "# Project instructions\n\n- Add repository conventions here; Ante reads this file at session start.\n",
                    ) {
                        Ok(()) => self.messages.push(format!("Created {}", path.display())),
                        Err(e) => self.messages.push(format!("Failed to create AGENTS.md: {e}")),
                    }
                }
            }
            "rename" => {
                if arg.trim().is_empty() {
                    self.messages.push("Usage: /rename <title>".into());
                } else {
                    self.session.meta.first_user_message = arg.trim().to_string();
                    let _ = self.session.save();
                    self.messages.push(format!("Session renamed to: {}", arg.trim()));
                }
            }
            "import-claude" => {
                self.messages.push(crate::memory::import_claude(&self.rt.cwd));
            }
            "theme" => self.overlay = Some(Overlay::Theme(0)),
            "term" => {
                self.messages.push(crate::tools::term::command(arg, &self.rt.cwd));
            }
            "memory" => self.messages.push(crate::memory::command(arg)),
            "rewind" => {
                let points = self.session.snapshot_numbers();
                if arg.is_empty() {
                    self.messages.push(format!("rewind points: {}", points.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")));
                } else if let Ok(n) = arg.trim().parse::<usize>() {
                    match self.session.load_snapshot(n) {
                        Ok(restored) => {
                            self.history = restored;
                            self.session.messages = self.history.clone();
                            let _ = self.session.save();
                            self.messages.push(format!("rewound to snapshot {n}"));
                        }
                        Err(e) => self.messages.push(format!("rewind failed: {e}")),
                    }
                }
            }
            "fork" => {
                let points = self.session.snapshot_numbers();
                let n = arg.trim().parse::<usize>().ok().or_else(|| points.last().copied());
                match n.map(|n| self.session.fork_from(n)) {
                    Some(Ok(fork)) => {
                        let id = fork.meta.id.clone();
                        self.session = fork;
                        self.history = self.session.messages.clone();
                        self.messages.push(format!("forked: {id} (continuing here)"));
                    }
                    Some(Err(e)) => self.messages.push(format!("fork failed: {e}")),
                    None => self.messages.push("no snapshot yet".into()),
                }
            }
            "resume" => {
                if arg.trim().is_empty() {
                    for meta in Session::list().into_iter().take(8) {
                        self.messages.push(format!("{}  {}", meta.id, crate::util::truncate(&meta.first_user_message, 50, "…")));
                    }
                } else {
                    match Session::load(arg.trim()) {
                        Ok(loaded) => {
                            self.history = loaded.messages.clone();
                            self.session = loaded;
                            self.session.system = self.rt.system_sections();
                            self.messages.push(format!("resumed {}", self.session.meta.id));
                        }
                        Err(e) => self.messages.push(format!("resume failed: {e}")),
                    }
                }
            }
            "statusline" | "update" | "offline-mode" | "add-provider" => {
                self.messages.push(format!("/{cmd} is available in the full TUI build only."));
            }
            other => self.messages.push(format!("Unknown command: /{other}")),
        }
    }
}

pub async fn run(rt: Arc<Runtime>, _save: bool) -> i32 {
    let mut tui = Tui::new(rt);
    let mut out = std::io::stdout();
    if terminal::enable_raw_mode().is_err() {
        return crate::repl::run(tui.rt.clone(), _save).await;
    }
    let _ = execute!(out, terminal::EnterAlternateScreen, cursor::Hide);
    let _ = tui.draw();

    loop {
        match event::read() {
            Ok(Event::Key(KeyEvent { code, modifiers, .. })) => match (code, tui.overlay.is_some()) {
                (KeyCode::Char('c'), _) if modifiers.contains(KeyModifiers::CONTROL) => break,
                (KeyCode::Esc, true) => tui.overlay = None,
                (KeyCode::Enter, true) => tui.overlay = None,
                (KeyCode::Up, true) => tui.move_selection(-1),
                (KeyCode::Down, true) => tui.move_selection(1),
                (_, true) => {}
                _ => match code {
                KeyCode::Char(ch) => {
                    tui.input.push(ch);
                }
                KeyCode::Backspace => {
                    tui.input.pop();
                }
                KeyCode::Esc => {
                    tui.input.clear();
                }
                KeyCode::Enter => {
                    let line = tui.input.trim().to_string();
                    tui.input.clear();
                    if line.is_empty() {
                        continue;
                    }
                    if line.starts_with('/') {
                        tui.handle_command(&line).await;
                    } else {
                        tui.messages.push(format!("❯ {line}"));
                        tui.submit(line).await;
                    }
                }
                _ => {}
            }},
            Ok(Event::Resize(_, _)) => {}
            Ok(_) => {}
            Err(_) => break,
        }
        let _ = tui.draw();
    }

    let _ = execute!(out, cursor::Show, terminal::LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
    0
}

fn cred_label(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => a
            .iter()
            .filter_map(|x| x.as_str())
            .find(|s| s.starts_with("secret:"))
            .or_else(|| a.iter().filter_map(|x| x.as_str()).next())
            .unwrap_or("-")
            .to_string(),
        _ => "-".into(),
    }
}

fn tail_path(text: &str, _max: usize) -> String {
    let count = text.chars().count();
    if count <= 35 {
        return text.to_string();
    }
    let tail: String = text.chars().skip(count - 31).collect();
    format!("...{tail}")
}

fn ellipsis(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max.saturating_sub(1)).collect();
    s.push('…');
    s
}

/// 读取 ANTE_HOME/versions.json，按原件格式生成更新提示与变更日志摘要。
fn update_notice() -> Option<Vec<String>> {
    let home = std::env::var("ANTE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| dirs_home().join(".ante"));
    let raw = std::fs::read_to_string(home.join("versions.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let latest = v.get("latest_version")?.get("version")?.as_str()?.to_string();
    let notes = v.get("latest_version")?.get("release_notes")?.as_str()?.to_string();
    if !version_gt(&latest, crate::VERSION) {
        return None;
    }
    if v.get("dismissed_version").and_then(|d| d.as_str()) == Some(latest.as_str()) {
        return None;
    }
    let source: Vec<&str> = notes.lines().collect();
    let mut rendered: Vec<String> = Vec::new();
    for line in &source {
        let s = line.trim_end();
        if s.is_empty() || s.starts_with("## ") {
            continue;
        }
        if let Some(rest) = s.strip_prefix("### ") {
            rendered.push(format!("    └─ ### {rest}"));
        } else if let Some(rest) = s.strip_prefix("- ") {
            for (i, piece) in wrap_text(rest, 111).into_iter().enumerate() {
                if i == 0 {
                    rendered.push(format!("    └─ - {piece}"));
                } else {
                    rendered.push(format!("       {piece}"));
                }
            }
        } else {
            rendered.push(format!("       {s}"));
        }
    }
    let mut out = vec![format!("  * Update available: v{latest} (current v{})", crate::VERSION)];
    let budget = 8usize;
    let shown = rendered.len().min(budget);
    out.extend(rendered[..shown].iter().cloned());
    if rendered.len() > shown {
        out.push(format!(
            "    └─ … {} more at https://ante.run/changelog",
            source.len().saturating_sub(shown)
        ));
    }
    out.push("    └─ /update installs it on exit · /update skip skips this version".into());
    Some(out)
}

fn dirs_home() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/"))
}

fn version_gt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split(['.', '-'])
            .take(3)
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    parse(a) > parse(b)
}

/// 按词换行，超宽单词不拆断。
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        if line.is_empty() {
            line.push_str(word);
        } else if line.chars().count() + 1 + word.chars().count() <= width {
            line.push(' ');
            line.push_str(word);
        } else {
            out.push(std::mem::take(&mut line));
            line.push_str(word);
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}
