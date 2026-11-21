use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};

use crate::llm::{
    image_data_url, Block, ChatRequest, Role, StreamDecoder, StreamEvent, Wire, WireRequest,
};

pub struct OpenAiResponses;

impl Wire for OpenAiResponses {
    fn name(&self) -> &'static str {
        "OpenAiResponse"
    }

    fn build(&self, req: &ChatRequest, stream: bool) -> Result<WireRequest> {
        let mut input: Vec<Value> = Vec::new();
        for m in &req.messages {
            match m.role {
                Role::User => {
                    let mut parts: Vec<Value> = Vec::new();
                    for b in &m.blocks {
                        match b {
                            Block::Text(t) => parts.push(json!({"type":"input_text","text":t})),
                            Block::Image { media_type, data } => parts.push(json!({
                                "type":"input_image","image_url": image_data_url(media_type, data)
                            })),
                            Block::ToolResult { id, content, is_error, .. } => {
                                let output = if *is_error { format!("error: {content}") } else { content.clone() };
                                input.push(json!({
                                    "type":"function_call_output","call_id":id,"output":output
                                }));
                            }
                            _ => {}
                        }
                    }
                    if !parts.is_empty() {
                        input.push(json!({"type":"message","role":"user","content":parts}));
                    }
                }
                Role::Assistant => {
                    let mut parts: Vec<Value> = Vec::new();
                    for b in &m.blocks {
                        match b {
                            Block::Text(t) => parts.push(json!({"type":"output_text","text":t})),
                            Block::Thinking(_) => {}
                            Block::ToolUse { id, name, input: args } => input.push(json!({
                                "type":"function_call","call_id":id,"name":name,"arguments":args.to_string()
                            })),
                            _ => {}
                        }
                    }
                    if !parts.is_empty() {
                        input.push(json!({"type":"message","role":"assistant","content":parts}));
                    }
                }
            }
        }

        let mut body = Map::new();
        body.insert("model".into(), json!(req.model));
        body.insert("instructions".into(), json!(req.system.join("\n")));
        body.insert("input".into(), json!(input));
        body.insert(
            "tools".into(),
            json!(req
                .tools
                .iter()
                .map(|t| json!({
                    "type":"function",
                    "name": t.get("name").cloned().unwrap_or_default(),
                    "description": t.get("description").cloned().unwrap_or_default(),
                    "parameters": t.get("parameters").cloned().unwrap_or(json!({"type":"object","properties":{}})),
                    "strict": false
                }))
                .collect::<Vec<_>>()),
        );
        body.insert("tool_choice".into(), json!("auto"));
        body.insert("parallel_tool_calls".into(), json!(true));
        body.insert("max_output_tokens".into(), json!(req.max_tokens));
        if let Some(e) = &req.effort {
            body.insert("reasoning".into(), json!({"effort": e}));
        }
        body.insert("stream".into(), json!(stream));
        body.insert("include".into(), json!(["reasoning.encrypted_content"]));
        body.insert("store".into(), json!(false));

        Ok(WireRequest {
            url: String::new(),
            headers: vec![("accept".into(), "text/event-stream".into())],
            body: Value::Object(body),
        })
    }

    fn decoder(&self) -> Box<dyn StreamDecoder> {
        Box::new(ResponsesDecoder::default())
    }
}

#[derive(Default)]
struct ResponsesDecoder {
    texts: Vec<String>,
}

impl StreamDecoder for ResponsesDecoder {
    fn feed(&mut self, event: Option<&str>, data: &str) -> Result<Vec<StreamEvent>> {
        if data.trim().is_empty() {
            return Ok(vec![]);
        }
        if data.trim() == "[DONE]" {
            return Ok(vec![]);
        }
        let v: Value = serde_json::from_str(data)
            .map_err(|e| anyhow!("mallformed responses stream chunk: {e}: {data}"))?;
        let ev = event
            .map(|s| s.to_string())
            .or_else(|| v.get("type").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .unwrap_or_default();
        let mut out = Vec::new();
        match ev.as_str() {
            "response.output_text.delta" => {
                if let Some(d) = v.get("delta").and_then(|d| d.as_str()) {
                    out.push(StreamEvent::TextDelta(d.to_string()));
                }
            }
            "response.reasoning_summary_text.delta"
            | "response.reasoning_text.delta"
            | "response.reasoning_summary.delta" => {
                if let Some(d) = v.get("delta").and_then(|d| d.as_str()) {
                    out.push(StreamEvent::ThinkingDelta(d.to_string()));
                }
            }
            "response.output_item.added" => {
                if let Some(item) = v.get("item") {
                    if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                        let id = item
                            .get("call_id")
                            .or_else(|| item.get("id"))
                            .and_then(|s| s.as_str())
                            .unwrap_or("call_0");
                        let name = item.get("name").and_then(|s| s.as_str()).unwrap_or("");
                        out.push(StreamEvent::ToolStart {
                            id: id.to_string(),
                            name: name.to_string(),
                        });
                    }
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(d) = v.get("delta").and_then(|d| d.as_str()) {
                    let id = v
                        .get("item_id")
                        .and_then(|s| s.as_str())
                        .unwrap_or("call_0");
                    out.push(StreamEvent::ToolArgsDelta {
                        id: id.to_string(),
                        delta: d.to_string(),
                    });
                }
            }
            "response.output_item.done" => {
                if let Some(item) = v.get("item") {
                    if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                        let id = item
                            .get("call_id")
                            .or_else(|| item.get("id"))
                            .and_then(|s| s.as_str())
                            .unwrap_or("call_0");
                        out.push(StreamEvent::ToolEnd { id: id.to_string() });
                    }
                }
            }
            "response.completed" => {
                if let Some(resp) = v.get("response") {
                    if let Some(u) = resp.get("usage") {
                        out.push(StreamEvent::Usage(crate::llm::Usage {
                            input_tokens: u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                            output_tokens: u.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                        }));
                    }
                    let status = resp.get("status").and_then(|s| s.as_str()).unwrap_or("completed");
                    out.push(StreamEvent::Done {
                        stop_reason: match status {
                            "incomplete" => "max_tokens".into(),
                            _ => "stop".into(),
                        },
                    });
                } else {
                    out.push(StreamEvent::Done { stop_reason: "stop".into() });
                }
            }
            "response.failed" | "error" => {
                let msg = v
                    .get("response")
                    .and_then(|r| r.get("error"))
                    .or_else(|| v.get("error"))
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| data.to_string());
                return Err(anyhow!("provider error: {msg}"));
            }
            _ => {}
        }
        Ok(out)
    }

    fn finish(&mut self) -> Vec<StreamEvent> {
        vec![]
    }
}
