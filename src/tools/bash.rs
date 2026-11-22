use serde_json::Value;
use std::time::Duration;

use super::{ToolCtx, ToolResult};
use crate::jobs::JobOutcome;

pub async fn run(ctx: &ToolCtx, input: &Value) -> ToolResult {
    let command = match input.get("command").and_then(|c| c.as_str()) {
        Some(c) if !c.trim().is_empty() => c.to_string(),
        _ => return ToolResult::err("`command` is empty"),
    };
    let max_wait = input
        .get("max_wait_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(120_000)
        .clamp(1, 600_000);
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
    let jobs = ctx.runtime.jobs.clone();
    let cwd = ctx.cwd.clone();
    let command_for_worker = command.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        jobs.run(
            &shell,
            &command_for_worker,
            &cwd,
            Duration::from_millis(max_wait),
        )
    })
    .await;

    let outcome = match outcome {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return ToolResult::err(format!("failed to start command: {e}")),
        Err(e) => return ToolResult::err(format!("job worker failed: {e}")),
    };

    match outcome {
        JobOutcome::Exited {
            code,
            success,
            stdout,
            stderr,
        } => {
            let mut content = String::new();
            content.push_str(if success { "status: exited\n" } else { "status: failed\n" });
            content.push_str(&format!("exit_code: {code}\n"));
            if !stdout.trim().is_empty() {
                content.push_str(&format!("\nstdout:\n{}", stdout));
            }
            if !stderr.trim().is_empty() {
                content.push_str(&format!("\nstderr:\n{}", stderr));
            }
            if stdout.trim().is_empty() && stderr.trim().is_empty() {
                content.push_str("\n(no output)");
            }
            ToolResult {
                content,
                is_error: !success,
                images: Vec::new(),
            }
        }
        JobOutcome::Running {
            id,
            pid,
            stdout_path,
            stderr_path,
            status_path,
            stdout,
            stderr,
        } => {
            let mut content = format!(
                "status: running in background\njob_id: {id}\npid: {pid}\nexit_code: null\n\
                 stdout_path: {}\nstderr_path: {}\nstatus_path: {}\n",
                stdout_path.display(),
                stderr_path.display(),
                status_path.display()
            );
            content.push_str("\nThe process was not killed. Read the paths for progress, or `kill -- -");
            content.push_str(&format!("{pid}` to stop the whole process group.\n"));
            if !stdout.trim().is_empty() {
                content.push_str(&format!("\nstdout so far:\n{}", stdout));
            }
            if !stderr.trim().is_empty() {
                content.push_str(&format!("\nstderr so far:\n{}", stderr));
            }
            ToolResult {
                content,
                is_error: false,
                images: Vec::new(),
            }
        }
    }
}
