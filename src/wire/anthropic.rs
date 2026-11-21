use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};

use crate::llm::{
    effort_budget, Block, ChatRequest, Role, StreamDecoder, StreamEvent, Wire, WireRequest,
};

pub struct AnthropicWire;

impl Wire for AnthropicWire {
    fn name(&self) -> &'static str {
        "AnthropicMessage"
    }

    fn build(&self, req: &ChatRequest, stream: bool) -> Result<WireRequest> {
        let mut messages: Vec<Value> = Vec::new();
        for m in &req.messages {
            let mut content: Vec<Value> = Vec::new();
            match m.role {
                Role::Assistant => {
                    for b in &m.blocks {
                        match b {
                            Block::Text(t) => content.push(json!({"type":"text","text":t})),
                            Block::Thinking(t) => {
                                content.push(json!({"type":"thinking","thinking":t,"signature":""}))
                            }
                            Block::ToolUse { id, name, input } => content.push(json!({
                                "type":"tool_use","id":id,"name":name,"input":input
                            })),
                            _ => {}
                        }
                    }
                }
                Role::User => {
                    for b in &m.blocks {
                        match b {
                            Block::Text(t) => content.push(json!({"type":"text","text":t})),
                            Block::Image { media_type, data } => content.push(json!({
                                "type":"image",
                                "source":{"type":"base64","media_type":media_type,"data":data}
                            })),
                            Block::ToolResult { id, content: c, is_error, .. } => content.push(json!({
                                "type":"tool_result","tool_use_id":id,"content":[{"type":"text","text":c}],
                                "is_error": is_error
                            })),
                            _ => {}
                        }
                    }
                }
            }
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            if content.is_empty() {
                continue;
            }
            // merge consecutive same-role messages like the API requires
            if let Some(last) = messages.last_mut() {
                if last.get("role").and_then(|r| r.as_str()) == Some(role) {
                    if let Some(arr) = last.get_mut("content").and_then(|c| c.as_array_mut()) {
                        arr.extend(content);
                        continue;
                    }
                }
            }
            messages.push(json!({"role": role, "content": content}));
        }

        let system: Vec<Value> = req
            .system
            .iter()
            .map(|s| json!({"type":"text","text":s}))
            .collect();

        let mut body = Map::new();
        body.insert("model".into(), json!(req.model));
        body.insert("max_tokens".into(), json!(req.max_tokens));
        body.insert("messages".into(), json!(messages));
        body.insert("system".into(), json!(system));
        body.insert(
            "tools".into(),
            json!(req
                .tools
                .iter()
                .map(|t| json!({
                    "name": t.get("name").cloned().unwrap_or_default(),
                    "description": t.get("description").cloned().unwrap_or_default(),
                    "input_schema": t.get("parameters").cloned().unwrap_or(json!({"type":"object","properties":{}}))
                }))
                .collect::<Vec<_>>()),
        );
        body.insert("stream".into(), json!(stream));
        if let Some(e) = &req.effort {
            body.insert(
                "thinking".into(),
                json!({"type":"enabled","budget_tokens": effort_budget(Some(e))}),
            );
        }
        body.insert("cache_control".into(), json!({"type":"ephemeral"}));

        Ok(WireRequest {
            url: String::new(),
            headers: vec![
                ("accept".into(), "text/event-stream".into()),
                ("anthropic-version".into(), "2023-06-01".into()),
                ("anthropic-beta".into(), "oauth-2025-04-20".into()),
            ],
            body: Value::Object(body),
        })
    }

    fn decoder(&self) -> Box<dyn StreamDecoder> {
        Box::new(AnthropicDecoder::default())
    }
}

#[derive(Default)]
struct AnthropicDecoder {
    tool_blocks: Vec<(usize, String)>,
    input_tokens: u64,
    stop_reason: Option<String>,
}

impl StreamDecoder for AnthropicDecoder {
    fn feed(&mut self, event: Option<&str>, data: &str) -> Result<Vec<StreamEvent>> {
        if data.trim().is_empty() {
            return Ok(vec![]);
        }
        let v: Value = serde_json::from_str(data)
            .map_err(|e| anyhow!("mallformed anthropic stream chunk: {e}: {data}"))?;
        let ev = event
            .map(|s| s.to_string())
            .or_else(|| v.get("type").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .unwrap_or_default();
        let mut out = Vec::new();
        match ev.as_str() {
            "message_start" => {
                if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
                    self.input_tokens = u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
                }
            }
            "content_block_start" => {
                let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                if let Some(cb) = v.get("content_block") {
                    if cb.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        let id = cb.get("id").and_then(|s| s.as_str()).unwrap_or("call_0").to_string();
                        let name = cb.get("name").and_then(|s| s.as_str()).unwrap_or("").to_string();
                        self.tool_blocks.push((idx, id.clone()));
                        out.push(StreamEvent::ToolStart { id, name });
                    }
                }
            }
            "content_block_delta" => {
                if let Some(d) = v.get("delta") {
                    match d.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                        "text_delta" => {
                            if let Some(t) = d.get("text").and_then(|s| s.as_str()) {
                                out.push(StreamEvent::TextDelta(t.to_string()));
                            }
                        }
                        "thinking_delta" => {
                            if let Some(t) = d.get("thinking").and_then(|s| s.as_str()) {
                                out.push(StreamEvent::ThinkingDelta(t.to_string()));
                            }
                        }
                        "input_json_delta" => {
                            let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                            if let Some((_, id)) = self.tool_blocks.iter().find(|(i, _)| *i == idx) {
                                if let Some(p) = d.get("partial_json").and_then(|s| s.as_str()) {
                                    out.push(StreamEvent::ToolArgsDelta { id: id.clone(), delta: p.to_string() });
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "content_block_stop" => {
                let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                if let Some((_, id)) = self.tool_blocks.iter().find(|(i, _)| *i == idx) {
                    out.push(StreamEvent::ToolEnd { id: id.clone() });
                }
            }
            "message_delta" => {
                if let Some(reason) = v.get("delta").and_then(|d| d.get("stop_reason")).and_then(|r| r.as_str()) {
                    self.stop_reason = Some(reason.to_string());
                }
                if let Some(u) = v.get("usage") {
                    out.push(StreamEvent::Usage(crate::llm::Usage {
                        input_tokens: self.input_tokens,
                        output_tokens: u.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                    }));
                }
            }
            "message_stop" => {
                let reason = match self.stop_reason.as_deref().unwrap_or("end_turn") {
                    "tool_use" => "tool_calls",
                    "max_tokens" => "max_tokens",
                    _ => "stop",
                };
                out.push(StreamEvent::Done { stop_reason: reason.to_string() });
            }
            "error" => {
                return Err(anyhow!("provider error: {}", v.get("error").map(|e| e.to_string()).unwrap_or(data.to_string())));
            }
            _ => {}
        }
        Ok(out)
    }

    fn finish(&mut self) -> Vec<StreamEvent> {
        vec![]
    }
}
