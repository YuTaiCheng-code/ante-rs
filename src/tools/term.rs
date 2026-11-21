//! Agent-drivable terminal sessions backing `/term`. Each session is a `sh -c`
//! child with piped stdin/stdout; a reader thread appends output. This is the
//! non-GUI equivalent of the original's terminal splits: the agent can create,
//! write to, read from, list and kill them.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;

use super::{ToolCtx, ToolResult};

struct Term {
    child: Child,
    output: Arc<Mutex<String>>,
    command: String,
}

fn sessions() -> &'static Mutex<HashMap<String, Term>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, Term>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn spawn(name: &str, command: &str, cwd: &std::path::Path) -> Result<(), String> {
    if sessions().lock().map(|s| s.contains_key(name)).unwrap_or(false) {
        return Err(format!("terminal `{name}` already exists"));
    }
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to launch `{name}`: {e}"))?;
    let output = Arc::new(Mutex::new(String::new()));
    for pipe in [
        child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
        child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let sink = output.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                if let Ok(mut buf) = sink.lock() {
                    buf.push_str(&line);
                    buf.push('\n');
                }
            }
        });
    }
    sessions().lock().map_err(|_| "terminal registry poisoned".to_string())?.insert(
        name.to_string(),
        Term { child, output, command: command.to_string() },
    );
    Ok(())
}

pub fn run(ctx: &ToolCtx, input: &Value) -> ToolResult {
    let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("list");
    let name = input.get("name").and_then(|v| v.as_str()).unwrap_or("");
    match action {
        "create" => {
            let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
            if name.is_empty() || command.is_empty() {
                return ToolResult::err("`name` and `command` are required");
            }
            match spawn(name, command, &ctx.cwd) {
                Ok(()) => ToolResult::ok(format!("terminal `{name}` started: {command}")),
                Err(e) => ToolResult::err(e),
            }
        }
        "write" => {
            let data = input.get("input").and_then(|v| v.as_str()).unwrap_or("");
            let mut guard = match sessions().lock() {
                Ok(g) => g,
                Err(_) => return ToolResult::err("terminal registry poisoned"),
            };
            let Some(t) = guard.get_mut(name) else {
                return ToolResult::err(format!("no terminal named `{name}`"));
            };
            match t.child.stdin.as_mut() {
                Some(stdin) => match writeln!(stdin, "{data}") {
                    Ok(_) => ToolResult::ok(format!("sent to `{name}`")),
                    Err(e) => ToolResult::err(format!("write failed: {e}")),
                },
                None => ToolResult::err("terminal stdin is closed"),
            }
        }
        "read" => {
            let mut guard = match sessions().lock() {
                Ok(g) => g,
                Err(_) => return ToolResult::err("terminal registry poisoned"),
            };
            let Some(t) = guard.get_mut(name) else {
                return ToolResult::err(format!("no terminal named `{name}`"));
            };
            let mut text = t.output.lock().map(|b| b.clone()).unwrap_or_default();
            for _ in 0..4 {
                if !text.trim().is_empty() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
                text = t.output.lock().map(|b| b.clone()).unwrap_or_default();
            }
            let tail: String = text.lines().rev().take(200).collect::<Vec<_>>().join("\n");
            ToolResult::ok(format!("{tail}\n[exit code: {:?}]", t.child.try_wait().ok().flatten().and_then(|s| s.code())))
        }
        "kill" => {
            let mut guard = match sessions().lock() {
                Ok(g) => g,
                Err(_) => return ToolResult::err("terminal registry poisoned"),
            };
            match guard.remove(name) {
                Some(mut t) => {
                    let _ = t.child.kill();
                    ToolResult::ok(format!("terminal `{name}` closed"))
                }
                None => ToolResult::err(format!("no terminal named `{name}`")),
            }
        }
        _ => {
            let guard = match sessions().lock() {
                Ok(g) => g,
                Err(_) => return ToolResult::err("terminal registry poisoned"),
            };
            let mut lines = vec!["terminals:".to_string()];
            for (n, t) in guard.iter() {
                lines.push(format!("  {n}  {}", t.command));
            }
            ToolResult::ok(lines.join("\n"))
        }
    }
}

/// `/term` 命令：`/term` 列表，`/term <name>` 打开（不存在则把 name 当命令启动）。
pub fn command(name: &str, cwd: &std::path::Path) -> String {
    let name = name.trim();
    if name.is_empty() {
        let mut lines = vec!["terminal splits:".to_string()];
        if let Ok(g) = sessions().lock() {
            for (n, t) in g.iter() {
                lines.push(format!("  {n}  {}", t.command));
            }
        }
        return lines.join("\n");
    }
    if let Ok(mut g) = sessions().lock() {
        if let Some(t) = g.get_mut(name) {
            let mut text = t.output.lock().map(|b| b.clone()).unwrap_or_default();
            for _ in 0..4 {
                if !text.trim().is_empty() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
                text = t.output.lock().map(|b| b.clone()).unwrap_or_default();
            }
            let tail: String = text.lines().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
            return format!("{tail}\n[exit code: {:?}]", t.child.try_wait().ok().flatten().and_then(|s| s.code()));
        }
    }
    match spawn(name, name, cwd) {
        Ok(()) => format!("terminal `{name}` opened"),
        Err(e) => e,
    }
}
