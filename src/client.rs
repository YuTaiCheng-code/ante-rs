use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

use crate::catalog::{ModelSpec, ProviderSpec};
use crate::credentials::{self, Resolved};
use crate::llm::{
    wire_for, ChatRequest, ChatResponse, StreamEvent, ToolCall, Usage, Wire, WireRequest,
};

#[derive(Debug, Clone)]
pub struct ProviderError {
    pub kind: String,
    pub headline: String,
    pub details: Vec<String>,
}

impl ProviderError {
    pub fn new(kind: &str, headline: &str, details: Vec<String>) -> Self {
        ProviderError {
            kind: kind.to_string(),
            headline: headline.to_string(),
            details,
        }
    }
    pub fn to_json(&self) -> Value {
        json!({"kind": self.kind, "headline": self.headline, "details": self.details})
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.headline)
    }
}

impl std::error::Error for ProviderError {}

pub struct LlmClient {
    http: reqwest::Client,
    pub provider_id: String,
    pub provider: ProviderSpec,
    pub model: ModelSpec,
    pub credential: Option<Resolved>,
    wire: Box<dyn Wire>,
}

impl LlmClient {
    pub fn new(provider_id: &str, provider: ProviderSpec, model: ModelSpec) -> Result<Self> {
        let catalog = crate::catalog::Catalog::load();
        let wire = wire_for(&provider.wire_style)?;
        let credential = credentials::resolve(&provider.credential, &catalog);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .connect_timeout(Duration::from_secs(20))
            .user_agent(format!("ante/{}", crate::VERSION))
            .build()?;
        Ok(LlmClient {
            http,
            provider_id: provider_id.to_string(),
            provider,
            model,
            credential,
            wire,
        })
    }

    pub fn has_credential(&self) -> bool {
        self.credential.is_some()
    }

    fn endpoint(&self) -> String {
        let base = self.provider.base_url.trim_end_matches('/');
        match self.provider.wire_style.as_str() {
            "AnthropicMessage" => format!("{base}/messages"),
            "OpenAiResponse" => format!("{base}/responses"),
            "Gemini" => format!(
                "{base}/models/{}:streamGenerateContent?alt=sse",
                self.model.id
            ),
            _ => format!("{base}/chat/completions"),
        }
    }

    fn request(&self, req: &ChatRequest, stream: bool) -> Result<WireRequest> {
        let mut wr = self.wire.build(req, stream)?;
        wr.url = self.endpoint();
        if let Some(cred) = &self.credential {
            let header = self
                .provider
                .credential_header
                .clone()
                .unwrap_or_else(|| "Authorization".into());
            let value = if header.eq_ignore_ascii_case("authorization")
                && !cred.value.starts_with("Bearer ")
            {
                format!("Bearer {}", cred.value)
            } else {
                cred.value.clone()
            };
            if self.provider.wire_style == "AnthropicMessage" {
                // Anthropic's native header; also send Bearer for OAuth-based gateways.
                wr.headers.push(("x-api-key".into(), cred.value.clone()));
            }
            wr.headers.push((header, value));
        }
        for (k, v) in &self.provider.headers {
            let v = v
                .replace("${session_id}", "")
                .replace("{session_id}", "");
            wr.headers.push((k.clone(), v));
        }
        Ok(wr)
    }

    pub async fn complete<F>(&self, req: ChatRequest, mut on_event: F) -> Result<ChatResponse>
    where
        F: FnMut(&StreamEvent),
    {
        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0..3u32 {
            match self.complete_once(&req, &mut on_event).await {
                Ok(r) => return Ok(r),
                Err(e) => {
                    let retryable = is_retryable(&e);
                    last_err = Some(e);
                    if !retryable || attempt == 2 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(400 * (attempt as u64 + 1))).await;
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("llm error")))
    }

    async fn complete_once<F>(&self, req: &ChatRequest, on_event: &mut F) -> Result<ChatResponse>
    where
        F: FnMut(&StreamEvent),
    {
        let wr = self.request(req, true)?;
        let mut builder = self.http.post(&wr.url).header("content-type", "application/json");
        for (k, v) in &wr.headers {
            builder = builder.header(k, v);
        }
        let resp = builder
            .json(&wr.body)
            .send()
            .await
            .map_err(|e| anyhow!(classify_transport(&e)))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(classify_http(status.as_u16(), &body, &self.provider_id)));
        }

        let mut decoder = self.wire.decoder();
        let mut stream = resp.bytes_stream();
        let mut buf = Vec::<u8>::new();
        let mut text = String::new();
        let mut thinking = String::new();
        let mut calls: Vec<ToolCall> = Vec::new();
        let mut open: BTreeMap<String, (String, String)> = BTreeMap::new();
        let mut usage = Usage::default();
        let mut done: Option<String> = None;

        'outer: while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| anyhow!(classify_transport(&e)))?;
            buf.extend_from_slice(&chunk);
            while let Some(idx) = find_sep(&buf) {
                let block = buf[..idx.0].to_vec();
                buf.drain(..idx.0 + idx.1);
                let block = String::from_utf8_lossy(&block).to_string();
                let block = block.trim();
                if block.is_empty() {
                    continue;
                }
                if let Some((event, data)) = crate::llm::parse_sse_block(block) {
                    if data.trim() == "[DONE]" {
                        // OpenAI-compatible end-of-stream marker
                    }
                    if std::env::var("ANTE_DEBUG_SSE").is_ok() { eprintln!("SSE event={:?} data={}", event, &data[..data.len().min(200)]); }
                    let events = decoder.feed(event.as_deref(), &data)?;
                    if std::env::var("ANTE_DEBUG_SSE").is_ok() { eprintln!("  -> {} events", events.len()); }
                    for ev in events {
                        apply_event(ev, on_event, &mut text, &mut thinking, &mut calls, &mut open, &mut usage, &mut done);
                        if done.is_some() {
                            break 'outer;
                        }
                    }
                }
            }
        }
        for ev in decoder.finish() {
            apply_event(ev, on_event, &mut text, &mut thinking, &mut calls, &mut open, &mut usage, &mut done);
        }

        let stop_reason = done.ok_or_else(|| anyhow!("stream ended unexpectedly"))?;
        for (id, (name, args)) in open {
            push_tool_call(&mut calls, Some(id), name, args);
        }
        Ok(ChatResponse {
            text,
            thinking,
            tool_calls: calls,
            usage,
            stop_reason,
        })
    }
}

fn push_tool_call(calls: &mut Vec<ToolCall>, id: Option<String>, name: String, args: String) {
    let input: Value = serde_json::from_str(&args).unwrap_or(Value::Null);
    let id = id.unwrap_or_else(|| format!("call_{}", calls.len()));
    calls.push(ToolCall { id, name, input });
}

fn apply_event<F>(
    ev: StreamEvent,
    on_event: &mut F,
    text: &mut String,
    thinking: &mut String,
    calls: &mut Vec<ToolCall>,
    open: &mut BTreeMap<String, (String, String)>,
    usage: &mut Usage,
    done: &mut Option<String>,
) where
    F: FnMut(&StreamEvent),
{
    match &ev {
        StreamEvent::TextDelta(t) => text.push_str(t),
        StreamEvent::ThinkingDelta(t) => thinking.push_str(t),
        StreamEvent::ToolStart { id, name } => {
            open.insert(id.clone(), (name.clone(), String::new()));
        }
        StreamEvent::ToolArgsDelta { id, delta } => {
            if let Some((_, args)) = open.get_mut(id) {
                args.push_str(delta);
            }
        }
        StreamEvent::ToolEnd { id } => {
            if let Some((name, args)) = open.remove(id) {
                push_tool_call(calls, Some(id.clone()), name, args);
            }
        }
        StreamEvent::Usage(u) => {
            usage.input_tokens = u.input_tokens;
            usage.output_tokens = u.output_tokens;
        }
        StreamEvent::Done { stop_reason } => *done = Some(stop_reason.clone()),
    }
    on_event(&ev);
}

/// Find the next SSE block boundary; returns (start_of_separator, len_of_separator).
fn find_sep(buf: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i + 1 < buf.len() {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some((i, 2));
        }
        if i + 3 < buf.len()
            && buf[i] == b'\r'
            && buf[i + 1] == b'\n'
            && buf[i + 2] == b'\r'
            && buf[i + 3] == b'\n'
        {
            return Some((i, 4));
        }
        i += 1;
    }
    None
}

fn classify_transport(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "request timeout".into()
    } else if e.is_connect() {
        "transport error: could not connect to the provider".into()
    } else {
        format!("transport error: {e}")
    }
}

pub fn classify_http(status: u16, body: &str, provider: &str) -> ProviderError {
    let detail = extract_error_message(body);
    match status {
        401 | 403 => ProviderError::new(
            "oauth",
            if status == 401 { "OAuth sign-in required" } else { "request forbidden" },
            vec![
                detail,
                format!("Run /providers to sign in, or set an API key for {provider}."),
            ],
        ),
        429 => ProviderError::new(
            "rate_limit",
            "provider overloaded",
            vec![detail, "Wait a moment and retry.".into()],
        ),
        400 | 422 => ProviderError::new("invalid_request", "invalid request", vec![detail]),
        404 => ProviderError::new("invalid_request", "provider server error", vec![detail]),
        s if s >= 500 => ProviderError::new(
            "server",
            "provider server error",
            vec![detail, "Wait a moment and retry.".into()],
        ),
        _ => ProviderError::new("transport", "transport error", vec![detail]),
    }
}

fn extract_error_message(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        if let Some(m) = v.pointer("/error/message").and_then(|m| m.as_str()) {
            return m.to_string();
        }
        if let Some(m) = v.get("message").and_then(|m| m.as_str()) {
            return m.to_string();
        }
        if let Some(m) = v.pointer("/error").and_then(|e| e.as_str()) {
            return m.to_string();
        }
    }
    crate::util::truncate(body, 400, "…")
}

fn is_retryable(e: &anyhow::Error) -> bool {
    let s = e.to_string().to_ascii_lowercase();
    s.contains("rate limit")
        || s.contains("overloaded")
        || s.contains("server error")
        || s.contains("stream ended unexpectedly")
        || s.contains("transport error")
        || s.contains("timeout")
}
