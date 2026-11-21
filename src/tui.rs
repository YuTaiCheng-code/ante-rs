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

use crate::runtime::Runtime;

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
    ("init", "Initialize this project with AGENTS.md"),
    ("mcp", "Manage MCP servers"),
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

pub struct Tui {
    rt: Arc<Runtime>,
    input: String,
    messages: Vec<String>,
    banner: bool,
    overlay: Option<Overlay>,
}

impl Tui {
    pub fn new(rt: Arc<Runtime>) -> Tui {
        Tui { rt, input: String::new(), messages: Vec::new(), banner: true, overlay: None }
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
                queue!(out, cursor::MoveTo(*x, 1 + i as u16), Print(text))?;
            }
            queue!(
                out,
                cursor::MoveTo(24, 3),
                Print(format!("Ante v{}", crate::VERSION)),
                cursor::MoveTo(24, 4),
                Print(format!("{} · {}", self.rt.model.id, self.rt.provider_id)),
                cursor::MoveTo(24, 5),
                Print(tail_path(&self.rt.cwd.display().to_string(), cols.saturating_sub(26) as usize)),
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
            queue!(
                out,
                cursor::MoveTo(0, top + i as u16),
                Print(" ".repeat(self.cols() as usize)),
                cursor::MoveTo(0, top + i as u16),
                Print(format!(
                    "{}/{name} - {}",
                    if i == 0 { "  → " } else { "    " },
                    ellipsis(desc, self.cols().saturating_sub(name.len() as u16 + 10) as usize)
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

    fn handle_command(&mut self, line: &str) {
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
            "usage" => self.messages.push("usage: 0 input / 0 output tokens".into()),
            "context" => self.messages.push(format!(
                "context: 0 of {} tokens",
                self.rt.model.context_limit.unwrap_or(0)
            )),
            "theme" => self.overlay = Some(Overlay::Theme(0)),
            "term" => {
                self.messages.push(crate::tools::term::command(arg, &self.rt.cwd));
            }
            "statusline" | "update" | "offline-mode" | "mcp" | "goal" | "diff"
            | "init" | "compact" | "rename" | "resume" | "rewind" | "fork" | "import-claude"
            | "add-provider" => {
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
                        tui.handle_command(&line);
                    } else {
                        tui.messages.push(format!("❯ {line}"));
                        tui.messages.push("(streaming responses are shown in the full build)".into());
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
