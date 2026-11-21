use serde_json::Value;
use std::time::Duration;

use super::{ToolCtx, ToolResult};

pub async fn run(ctx: &ToolCtx, input: &Value) -> ToolResult {
    let command = match input.get("command").and_then(|c| c.as_str()) {
        Some(c) if !c.trim().is_empty() => c.to_string(),
        _ => return ToolResult::err("`command` is empty"),
    };
    let max_wait = input
        .get("max_wait_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(120_000)
        .min(600_000);
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
    let mut cmd = tokio::process::Command::new(&shell);
    cmd.arg("-lc")
        .arg(&command)
        .current_dir(&ctx.cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let started = std::time::Instant::now();
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("failed to start command: {e}")),
    };
    let out = match tokio::time::timeout(Duration::from_millis(max_wait), child.wait_with_output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return ToolResult::err(format!("command failed: {e}")),
        Err(_) => {
            return ToolResult {
                content: format!(
                    "status: timed out after {} ms\n\nThe command is still running; re-run with a longer `max_wait_ms` if you need its result.",
                    max_wait
                ),
                is_error: true,
                images: Vec::new(),
            }
        }
    };
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut content = String::new();
    if out.status.success() {
        content.push_str("status: exited\n");
    } else {
        content.push_str("status: failed\n");
    }
    content.push_str(&format!("exit_code: {code}\n"));
    if !stdout.trim().is_empty() {
        content.push_str(&format!("\nstdout:\n{}", super::limit_output(&stdout)));
    }
    if !stderr.trim().is_empty() {
        content.push_str(&format!("\nstderr:\n{}", super::limit_output(&stderr)));
    }
    if stdout.trim().is_empty() && stderr.trim().is_empty() {
        content.push_str("\n(no output)");
    }
    ToolResult {
        content,
        is_error: !out.status.success(),
        images: Vec::new(),
    }
}
