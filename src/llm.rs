use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub enum Block {
    Text(String),
    Thinking(String),
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        id: String,
        name: String,
        content: String,
        is_error: bool,
    },
    Image {
        media_type: String,
        data: String,
    },
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub blocks: Vec<Block>,
}

impl Message {
    pub fn user_text(text: impl Into<String>) -> Message {
        Message {
            role: Role::User,
            blocks: vec![Block::Text(text.into())],
        }
    }
    pub fn assistant_text(text: impl Into<String>) -> Message {
        Message {
            role: Role::Assistant,
            blocks: vec![Block::Text(text.into())],
        }
    }
    pub fn text(&self) -> String {
        let mut s = String::new();
        for b in &self.blocks {
            match b {
                Block::Text(t) | Block::Thinking(t) => {
                    if !s.is_empty() {
                        s.push_str("\n\n");
                    }
                    s.push_str(t)
                }
                _ => {}
            }
        }
        s
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub text: String,
    pub thinking: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub stop_reason: String,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub system: Vec<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<Value>,
    pub max_tokens: u64,
    pub effort: Option<String>,
    pub include_images: bool,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolStart { id: String, name: String },
    ToolArgsDelta { id: String, delta: String },
    ToolEnd { id: String },
    Usage(Usage),
    Done { stop_reason: String },
}

#[derive(Debug, Clone)]
pub struct WireRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

impl WireRequest {
    pub fn describe(&self) -> String {
        format!(
            "{} {}",
            self.url,
            crate::util::truncate(&self.body.to_string(), 400, "…")
        )
    }
}

pub trait StreamDecoder: Send {
    fn feed(&mut self, event: Option<&str>, data: &str) -> Result<Vec<StreamEvent>>;
    fn finish(&mut self) -> Vec<StreamEvent> {
        Vec::new()
    }
}

pub trait Wire: Send + Sync {
    fn name(&self) -> &'static str;
    fn build(&self, req: &ChatRequest, stream: bool) -> Result<WireRequest>;
    fn decoder(&self) -> Box<dyn StreamDecoder>;
}

pub fn wire_for(style: &str) -> Result<Box<dyn Wire>> {
    Ok(match style {
        "OpenAiCompatible" => Box::new(crate::wire::openai_chat::OpenAiChat),
        "OpenAiResponse" => Box::new(crate::wire::openai_responses::OpenAiResponses),
        "AnthropicMessage" => Box::new(crate::wire::anthropic::AnthropicWire),
        "Gemini" => Box::new(crate::wire::gemini::GeminiWire),
        other => return Err(anyhow!("unknown wire style '{other}'")),
    })
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Role::User => write!(f, "user"),
            Role::Assistant => write!(f, "assistant"),
        }
    }
}

/// Shared SSE data-line reader: returns (event, data) pairs from a byte stream.
pub fn parse_sse_block(block: &str) -> Option<(Option<String>, String)> {
    let mut event = None;
    let mut data = String::new();
    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if data.is_empty() && event.is_none() {
        None
    } else {
        Some((event, data))
    }
}

pub fn text_image_block(media_type: &str, data: &str) -> Value {
    json!({ "media_type": media_type, "data": data })
}

pub fn image_data_url(media_type: &str, data: &str) -> String {
    format!("data:{media_type};base64,{data}")
}

pub fn guess_media_type(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/png",
    }
}

pub fn effort_budget(effort: Option<&str>) -> u64 {
    match effort.unwrap_or("medium") {
        "min" | "minimal" => 1024,
        "low" => 2048,
        "medium" => 4096,
        "high" => 8192,
        "xhigh" => 16384,
        "max" => 32768,
        _ => 4096,
    }
}
