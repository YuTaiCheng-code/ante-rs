use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};

use crate::llm::{Block, ChatRequest, Role, StreamDecoder, StreamEvent, Wire, WireRequest};

pub struct GeminiWire;

impl Wire for GeminiWire {
    fn name(&self) -> &'static str {
        "Gemini"
    }

    fn build(&self, req: &ChatRequest, stream: bool) -> Result<WireRequest> {
        let mut contents: Vec<Value> = Vec::new();
        for m in &req.messages {
            let mut parts: Vec<Value> = Vec::new();
            match m.role {
                Role::Assistant => {
                    for b in &m.blocks {
                        match b {
                            Block::Text(t) => parts.push(json!({"text":t})),
                            Block::ToolUse { name, input, .. } => {
                                parts.push(json!({"functionCall":{"name":name,"args":input}}))
                            }
                            _ => {}
                        }
                    }
                }
                Role::User => {
                    for b in &m.blocks {
                        match b {
                            Block::Text(t) => parts.push(json!({"text":t})),
                            Block::Image { media_type, data } => parts.push(json!({
                                "inlineData":{"mimeType":media_type,"data":data}
                            })),
                            Block::ToolResult { name, content, .. } => parts.push(json!({
                                "functionResponse":{"name":name,"response":{"output":content}}
                            })),
                            _ => {}
                        }
                    }
                }
            }
            if parts.is_empty() {
                continue;
            }
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "model",
            };
            contents.push(json!({"role": role, "parts": parts}));
        }

        let mut body = Map::new();
        body.insert("contents".into(), json!(contents));
        body.insert(
            "systemInstruction".into(),
            json!({"parts": req.system.iter().map(|s| json!({"text":s})).collect::<Vec<_>>()}),
        );
        let mut gen = Map::new();
        gen.insert("maxOutputTokens".into(), json!(req.max_tokens));
        if let Some(e) = &req.effort {
            gen.insert(
                "thinkingConfig".into(),
                json!({"thinkingLevel": e.to_ascii_uppercase()}),
            );
        }
        body.insert("generationConfig".into(), Value::Object(gen));
        body.insert(
            "tools".into(),
            json!([{
                "functionDeclarations": req.tools.iter().map(|t| json!({
                    "name": t.get("name").cloned().unwrap_or_default(),
                    "description": t.get("description").cloned().unwrap_or_default(),
                    "parameters": t.get("parameters").cloned().unwrap_or(json!({"type":"object","properties":{}}))
                })).collect::<Vec<_>>()
            }]),
        );
        body.insert(
            "toolConfig".into(),
            json!({"functionCallingConfig":{"mode":"AUTO"}}),
        );

        let _ = stream;
        Ok(WireRequest {
            url: String::new(),
            headers: vec![],
            body: Value::Object(body),
        })
    }

    fn decoder(&self) -> Box<dyn StreamDecoder> {
        Box::new(GeminiDecoder::default())
    }
}

#[derive(Default)]
struct GeminiDecoder {
    finished: bool,
}

impl StreamDecoder for GeminiDecoder {
    fn feed(&mut self, _event: Option<&str>, data: &str) -> Result<Vec<StreamEvent>> {
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            return Ok(vec![]);
        }
        // The endpoint streams a JSON array when SSE is not enabled; accept both.
        let v: Value = serde_json::from_str(data)
            .or_else(|_| serde_json::from_str::<Vec<Value>>(data).map(|a| json!(a)))
            .map_err(|e| anyhow!("mallformed gemini stream chunk: {e}: {data}"))?;
        let chunks: Vec<Value> = if let Some(arr) = v.as_array() {
            arr.clone()
        } else {
            vec![v]
        };
        let mut out = Vec::new();
        for c in chunks {
            if let Some(fb) = c.get("promptFeedback") {
                if let Some(reason) = fb.get("blockReason").and_then(|r| r.as_str()) {
                    return Err(anyhow!("provider error: content blocked ({reason})"));
                }
            }
            if let Some(u) = c.get("usageMetadata") {
                out.push(StreamEvent::Usage(crate::llm::Usage {
                    input_tokens: u.get("promptTokenCount").and_then(|x| x.as_u64()).unwrap_or(0),
                    output_tokens: u
                        .get("candidatesTokenCount")
                        .or_else(|| u.get("candidates_token_count"))
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0),
                }));
            }
            if let Some(cand) = c.get("candidates").and_then(|a| a.get(0)) {
                if let Some(parts) = cand
                    .get("content")
                    .and_then(|x| x.get("parts"))
                    .and_then(|p| p.as_array())
                {
                    for (i, p) in parts.iter().enumerate() {
                        if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                            if !t.is_empty() {
                                out.push(StreamEvent::TextDelta(t.to_string()));
                            }
                        }
                        if let Some(f) = p.get("functionCall") {
                            let name = f.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let id = format!("call_{name}_{i}");
                            let args = f.get("args").cloned().unwrap_or(json!({}));
                            out.push(StreamEvent::ToolStart {
                                id: id.clone(),
                                name: name.to_string(),
                            });
                            out.push(StreamEvent::ToolArgsDelta {
                                id: id.clone(),
                                delta: args.to_string(),
                            });
                            out.push(StreamEvent::ToolEnd { id });
                        }
                    }
                }
                if let Some(reason) = cand.get("finishReason").and_then(|r| r.as_str()) {
                    if reason != "STOP" || cand.get("finishReason").is_some() {
                        self.finished = true;
                        let stop = match reason {
                            "MAX_TOKENS" => "max_tokens",
                            "SAFETY" | "RECITATION" => "refusal",
                            _ => "stop",
                        };
                        out.push(StreamEvent::Done { stop_reason: stop.to_string() });
                    }
                }
            }
        }
        if !self.finished {
            // stream completed without an explicit finish reason
        }
        Ok(out)
    }

    fn finish(&mut self) -> Vec<StreamEvent> {
        if !self.finished {
            self.finished = true;
            vec![StreamEvent::Done { stop_reason: "stop".into() }]
        } else {
            vec![]
        }
    }
}
