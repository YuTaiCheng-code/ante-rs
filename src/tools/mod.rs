pub mod bash;
pub mod fs_tools;
pub mod permission;
pub mod term;
pub mod todo;
pub mod web;

use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::runtime::Runtime;

#[derive(Debug, Clone, Default)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
    pub images: Vec<(String, String)>, // (media_type, base64)
}

impl ToolResult {
    pub fn ok(content: impl Into<String>) -> Self {
        ToolResult {
            content: content.into(),
            is_error: false,
            images: Vec::new(),
        }
    }
    pub fn err(content: impl Into<String>) -> Self {
        ToolResult {
            content: content.into(),
            is_error: true,
            images: Vec::new(),
        }
    }
    pub fn with_images(mut self, imgs: Vec<(String, String)>) -> Self {
        self.images = imgs;
        self
    }
}

#[derive(Clone)]
pub struct ToolCtx {
    pub runtime: Arc<Runtime>,
    pub cwd: PathBuf,
    pub todos: Arc<Mutex<Vec<Value>>>,
    pub depth: u8,
}

impl ToolCtx {
    pub fn new(runtime: Arc<Runtime>, cwd: PathBuf) -> Self {
        ToolCtx {
            runtime,
            cwd,
            todos: Arc::new(Mutex::new(Vec::new())),
            depth: 0,
        }
    }
}

pub async fn execute(ctx: &ToolCtx, name: &str, input: &Value) -> ToolResult {
    match name {
        "Bash" => bash::run(ctx, input).await,
        "Read" => fs_tools::read(ctx, input).await,
        "Write" => fs_tools::write(ctx, input).await,
        "Edit" => fs_tools::edit(ctx, input).await,
        "Glob" => fs_tools::glob(ctx, input).await,
        "Grep" => fs_tools::grep(ctx, input).await,
        "ViewImage" => fs_tools::view_image(ctx, input).await,
        "WebFetch" => web::fetch(ctx, input).await,
        "WebSearch" => web::search(ctx, input).await,
        "Terminal" => term::run(ctx, input),
        "TodoWrite" => todo::write(ctx, input),
        "AskUser" => ask_user(ctx, input),
        "Agent" => Box::pin(crate::agent::agent_tool(ctx, input)).await,
        other if other.starts_with("mcp__") => match ctx.runtime.mcp.call(other, input).await {
            Ok(result) => result,
            Err(e) => ToolResult::err(e.to_string()),
        },
        other => ToolResult::err(format!("unknown tool '{other}'")),
    }
}

fn ask_user(_ctx: &ToolCtx, input: &Value) -> ToolResult {
    let questions = input
        .get("questions")
        .and_then(|q| q.as_array())
        .cloned()
        .unwrap_or_default();
    let mut text = String::from("The user did not answer the questions (they skipped the prompt or replied with a message instead). Proceed using your best judgment based on the context so far, and do not ask these again.\n");
    for q in &questions {
        if let Some(question) = q.get("question").and_then(|v| v.as_str()) {
            text.push_str(&format!("\n- {question}"));
        }
    }
    ToolResult::ok(text)
}

/// Resolve a tool-supplied path against the workspace, expanding `~`.
pub fn resolve_path(cwd: &std::path::Path, p: &str) -> PathBuf {
    let expanded = crate::util::expand_tilde(p);
    if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    }
}

pub fn limit_output(s: &str) -> String {
    let mut lines = String::new();
    let mut n = 0;
    for line in s.lines() {
        if n >= 2000 {
            lines.push_str("\ntruncated after 2000 lines");
            break;
        }
        let l = crate::util::truncate(line, 500, "…");
        lines.push_str(&l);
        lines.push('\n');
        n += 1;
    }
    crate::util::truncate(&lines, 25 * 1024, "\n\ntruncated after 25 KB")
}
