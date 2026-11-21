use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};

use crate::llm::{
    image_data_url, Block, ChatRequest, Message, Role, StreamDecoder, StreamEvent, Usage,
    Wire, WireRequest,
};

pub struct OpenAiChat;

impl Wire for OpenAiChat {
    fn name(&self) -> &'static str {
        "OpenAiCompatible"
    }

    fn build(&self, req: &ChatRequest, stream: bool) -> Result<WireRequest> {
        let mut messages: Vec<Value> = Vec::new();
        if !req.system.is_empty() {
            messages.push(json!({"role":"system","content":req.system.join("\n")}));
        }
        for m in &req.messages {
            match m.role {
                Role::Assistant => {
                    let mut content = String::new();
                    let mut calls = Vec::new();
                    for b in &m.blocks {
                        match b {
                            Block::Text(t) => {
                                if !content.is_empty() {
                                    content.push('\n');
                                }
                                content.push_str(t);
                            }
                            Block::Thinking(t) => {
                                if !content.is_empty() {
                                    content.push('\n');
                                }
                                content.push_str(t);
                            }
                            Block::ToolUse { id, name, input } => calls.push(json!({
                                "id": id, "type": "function",
                                "function": {"name": name, "arguments": input.to_string()}
                            })),
                            _ => {}
                        }
                    }
                    let mut msg = Map::new();
                    msg.insert("role".into(), json!("assistant"));
                    if !content.is_empty() {
                        msg.insert("content".into(), json!(content));
                    }
                    if !calls.is_empty() {
                        msg.insert("tool_calls".into(), json!(calls));
                    }
                    if msg.len() > 1 || content.is_empty() {
                        messages.push(Value::Object(msg));
                    }
                }
                Role::User => {
                    let mut parts: Vec<Value> = Vec::new();
                    let mut tool_msgs: Vec<Value> = Vec::new();
                    let mut has_text = false;
                    for b in &m.blocks {
                        match b {
                            Block::Text(t) => {
                                has_text = true;
                                parts.push(json!({"type":"text","text":t}));
                            }
                            Block::Image { media_type, data } => parts.push(json!({
                                "type":"image_url",
                                "image_url":{"url": image_data_url(media_type, data)}
                            })),
                            Block::ToolResult {
                                id,
                                content,
                                is_error,
                                ..
                            } => {
                                let text = if *is_error {
                                    format!("error: {content}")
                                } else {
                                    content.clone()
                                };
                                tool_msgs.push(json!({
                                    "role":"tool","tool_call_id":id,"content":text
                                }));
                            }
                            _ => {}
                        }
                    }
                    if has_text || !parts.is_empty() {
                        let content: Value = if parts.len() == 1 && parts[0].get("text").is_some() {
                            parts[0]["text"].clone()
                        } else if parts.is_empty() {
                            json!("")
                        } else {
                            json!(parts)
                        };
                        messages.push(json!({"role":"user","content":content}));
                    }
                    messages.extend(tool_msgs);
                }
            }
        }

        let mut body = Map::new();
        body.insert("model".into(), json!(req.model));
        body.insert("messages".into(), json!(messages));
        if let Some(e) = &req.effort {
            body.insert(
                "reasoning_effort".into(),
                json!(normalize_effort(e)),
            );
        }
        body.insert("max_tokens".into(), json!(req.max_tokens));
        body.insert("stream".into(), json!(stream));
        body.insert("stream_options".into(), json!({"include_usage": true}));
        body.insert("tools".into(), openai_tools(&req.tools));
        if req.tools.is_empty() {
            body.remove("tools");
        }

        Ok(WireRequest {
            url: String::new(),
            headers: vec![("accept".into(), "text/event-stream".into())],
            body: Value::Object(body),
        })
    }

    fn decoder(&self) -> Box<dyn StreamDecoder> {
        Box::new(ChatDecoder::default())
    }
}

fn normalize_effort(e: &str) -> &str {
    match e {
        "min" => "minimal",
        other => other,
    }
}

pub fn openai_tools(tools: &[Value]) -> Value {
    let arr: Vec<Value> = tools
        .iter()
        .map(|t| {
            if t.get("type").and_then(|v| v.as_str()) == Some("function") {
                t.clone()
            } else {
                json!({"type":"function","function":t})
            }
        })
        .collect();
    json!(arr.iter().map(|t| {
        let f = t.get("function").cloned().unwrap_or_default();
        json!({"type":"function","function":f})
    }).collect::<Vec<_>>())
}

#[derive(Default)]
struct ChatDecoder {
    ids: Vec<String>,
    stop: Option<String>,
}

impl StreamDecoder for ChatDecoder {
    fn feed(&mut self, _event: Option<&str>, data: &str) -> Result<Vec<StreamEvent>> {
        let data = data.trim();
        if data.is_empty() {
            return Ok(vec![]);
        }
        if data == "[DONE]" {
            let reason = self.stop.clone().unwrap_or_else(|| "stop".into());
            return Ok(vec![StreamEvent::Done {
                stop_reason: map_stop(&reason),
            }]);
        }
        let v: Value = serde_json::from_str(data)
            .map_err(|e| anyhow!("mallformed chat stream chunk: {e}: {data}"))?;
        if let Some(err) = v.get("error") {
            return Err(anyhow!("provider error: {err}"));
        }
        let mut out = Vec::new();
        if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
            out.push(StreamEvent::Usage(usage_from(u)));
        }
        let choice = v
            .get("choices")
            .and_then(|c| c.get(0))
            .cloned()
            .unwrap_or(Value::Null);
        if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
            self.stop = Some(reason.to_string());
        }
        let delta = choice.get("delta").cloned().unwrap_or(Value::Null);
        if let Some(t) = delta.get("content").and_then(|c| c.as_str()) {
            if !t.is_empty() {
                out.push(StreamEvent::TextDelta(t.to_string()));
            }
        }
        for key in ["reasoning_content", "reasoning", "thinking"] {
            if let Some(t) = delta.get(key).and_then(|c| c.as_str()) {
                if !t.is_empty() {
                    out.push(StreamEvent::ThinkingDelta(t.to_string()));
                }
            }
        }
        if let Some(calls) = delta.get("tool_calls").and_then(|c| c.as_array()) {
            for (pos, call) in calls.iter().enumerate() {
                let idx = call
                    .get("index")
                    .and_then(|i| i.as_u64())
                    .unwrap_or(pos as u64) as usize;
                while self.ids.len() <= idx {
                    self.ids.push(String::new());
                }
                let id = call
                    .get("id")
                    .and_then(|i| i.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("call_{idx}"));
                let f = call.get("function").cloned().unwrap_or(Value::Null);
                let name = f.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let started = self.ids[idx].is_empty();
                if !name.is_empty() || started {
                    if self.ids[idx].is_empty() {
                        self.ids[idx] = id.clone();
                    }
                    if !name.is_empty() {
                        out.push(StreamEvent::ToolStart {
                            id: id.clone(),
                            name: name.to_string(),
                        });
                    }
                }
                if let Some(args) = f.get("arguments").and_then(|a| a.as_str()) {
                    if !args.is_empty() {
                        out.push(StreamEvent::ToolArgsDelta {
                            id: self.ids[idx].clone(),
                            delta: args.to_string(),
                        });
                    }
                }
            }
        }
        Ok(out)
    }

    fn finish(&mut self) -> Vec<StreamEvent> {
        if let Some(r) = &self.stop {
            vec![StreamEvent::Done {
                stop_reason: map_stop(r),
            }]
        } else {
            vec![]
        }
    }
}

fn map_stop(reason: &str) -> String {
    match reason {
        "stop" => "stop",
        "tool_calls" | "function_call" => "tool_calls",
        "length" | "max_tokens" => "max_tokens",
        "content_filter" => "refusal",
        other => other,
    }
    .to_string()
}

pub fn usage_from(u: &Value) -> Usage {
    Usage {
        input_tokens: u
            .get("prompt_tokens")
            .or_else(|| u.get("input_tokens"))
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
        output_tokens: u
            .get("completion_tokens")
            .or_else(|| u.get("output_tokens"))
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
    }
}

pub fn messages_debug(m: &Message) -> String {
    format!("{}:{:?}", m.role, m.blocks.len())
}
