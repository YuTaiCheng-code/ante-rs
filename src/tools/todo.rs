use serde_json::{json, Value};

use super::{ToolCtx, ToolResult};

pub fn write(ctx: &ToolCtx, input: &Value) -> ToolResult {
    let raw = match input.get("todos").and_then(|t| t.as_array()) {
        Some(t) => t.clone(),
        None => return ToolResult::err("`todos` must be a list"),
    };
    let mut todos = Vec::new();
    for (i, t) in raw.iter().enumerate() {
        let content = match t.get("content").and_then(|c| c.as_str()) {
            Some(c) if !c.trim().is_empty() => c.to_string(),
            _ => return ToolResult::err(format!("todo {i}: `content` is required")),
        };
        let status = match t.get("status").and_then(|s| s.as_str()) {
            Some(s @ ("pending" | "in_progress" | "completed")) => s.to_string(),
            Some(other) => {
                return ToolResult::err(format!(
                    "todo {i}: invalid status '{other}' (pending | in_progress | completed)"
                ))
            }
            None => return ToolResult::err(format!("todo {i}: `status` is required")),
        };
        let active_form = t.get("activeForm").and_then(|a| a.as_str()).map(|s| s.to_string());
        todos.push(json!({
            "content": content,
            "status": status,
            "activeForm": active_form,
        }));
    }
    if let Ok(mut guard) = ctx.todos.lock() {
        *guard = todos.clone();
    }
    ToolResult::ok(render(&todos))
}

fn render(todos: &[Value]) -> String {
    if todos.is_empty() {
        return "Todo list cleared.".into();
    }
    let mut out = String::new();
    for t in todos {
        let status = t.get("status").and_then(|s| s.as_str()).unwrap_or("pending");
        let mark = match status {
            "completed" => "[x]",
            "in_progress" => "[~]",
            _ => "[ ]",
        };
        let content = t.get("content").and_then(|c| c.as_str()).unwrap_or("");
        out.push_str(&format!("{mark} {content}\n"));
        if status == "in_progress" {
            if let Some(a) = t.get("activeForm").and_then(|a| a.as_str()) {
                out.push_str(&format!("    ({a})\n"));
            }
        }
    }
    let done = todos
        .iter()
        .filter(|t| t.get("status").and_then(|s| s.as_str()) == Some("completed"))
        .count();
    out.push_str(&format!("\n{done}/{} completed", todos.len()));
    out
}
