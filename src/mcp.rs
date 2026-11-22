//! MCP client for stdio and Streamable HTTP transports.
//!
//! Stdio uses newline-delimited JSON-RPC. HTTP uses POST requests and accepts
//! either JSON or SSE responses, preserving the `Mcp-Session-Id` header.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::timeout;

use crate::tools::ToolResult;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
enum TransportConfig {
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
        cwd: Option<String>,
    },
    Http {
        url: String,
        headers: HashMap<String, String>,
    },
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    transport: TransportConfig,
}

impl ServerConfig {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::String(s) => {
                let mut parts = s.split_whitespace();
                let command = parts.next().ok_or_else(|| anyhow!("empty MCP command"))?;
                Ok(Self {
                    transport: TransportConfig::Stdio {
                        command: command.to_string(),
                        args: parts.map(str::to_string).collect(),
                        env: HashMap::new(),
                        cwd: None,
                    },
                })
            }
            Value::Object(o) => {
                let kind = o
                    .get("transport")
                    .or_else(|| o.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let has_url = o.get("url").and_then(Value::as_str).is_some();
                let is_http = has_url
                    || matches!(
                        kind,
                        "http" | "streamable-http" | "streamable_http" | "sse"
                    );
                if is_http {
                    let url = o
                        .get("url")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("HTTP MCP server needs a `url`"))?
                        .to_string();
                    if !(url.starts_with("http://") || url.starts_with("https://")) {
                        return Err(anyhow!("MCP url must start with http:// or https://"));
                    }
                    let headers = o
                        .get("headers")
                        .and_then(Value::as_object)
                        .map(|m| {
                            m.iter()
                                .filter_map(|(k, v)| {
                                    v.as_str()
                                        .map(|s| (k.clone(), expand_env(s)))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    return Ok(Self {
                        transport: TransportConfig::Http { url, headers },
                    });
                }
                if !kind.is_empty() && kind != "stdio" {
                    return Err(anyhow!("unsupported MCP transport '{kind}'"));
                }
                let command = o
                    .get("command")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("MCP server needs a `command` or `url`"))?
                    .to_string();
                let args = o
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default();
                let env = o
                    .get("env")
                    .and_then(Value::as_object)
                    .map(|m| {
                        m.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(Self {
                    transport: TransportConfig::Stdio {
                        command,
                        args,
                        env,
                        cwd: o.get("cwd").and_then(Value::as_str).map(str::to_string),
                    },
                })
            }
            _ => Err(anyhow!("invalid MCP server configuration")),
        }
    }
}

fn expand_env(input: &str) -> String {
    let mut out = String::new();
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find('}') {
            let name = &after[..end];
            out.push_str(&std::env::var(name).unwrap_or_default());
            rest = &after[end + 1..];
        } else {
            out.push_str(&rest[start..]);
            return out;
        }
    }
    out.push_str(rest);
    out
}

#[derive(Debug, Clone)]
pub struct McpTool {
    pub server: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

impl McpTool {
    pub fn qualified_name(&self) -> String {
        format!("mcp__{}__{}", self.server, self.name)
    }
}

struct StdioClient {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl StdioClient {
    async fn spawn(name: &str, cfg: &ServerConfig) -> Result<Self> {
        let TransportConfig::Stdio {
            command,
            args,
            env,
            cwd,
        } = &cfg.transport
        else {
            return Err(anyhow!("not a stdio MCP server"));
        };
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(cwd) = cwd {
            cmd.current_dir(crate::util::expand_tilde(cwd));
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawn MCP server '{name}' ({command})"))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("MCP stdin unavailable"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("MCP stdout unavailable"))?;
        Ok(Self {
            child,
            stdin,
            lines: BufReader::new(stdout).lines(),
            next_id: 1,
        })
    }

    async fn send(&mut self, value: &Value) -> Result<()> {
        let line = serde_json::to_string(value)?;
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read_message(&mut self) -> Result<Value> {
        loop {
            let line = timeout(REQUEST_TIMEOUT, self.lines.next_line())
                .await
                .map_err(|_| anyhow!("MCP response timed out"))??
                .ok_or_else(|| anyhow!("MCP server closed stdout"))?;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(&line)
                .with_context(|| format!("invalid MCP JSON-RPC line: {line}"))?;
            if value.get("id").is_some() {
                return Ok(value);
            }
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await?;
        loop {
            let message = self.read_message().await?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(anyhow!("MCP {method} failed: {error}"));
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

struct HttpClient {
    http: reqwest::Client,
    url: String,
    headers: HashMap<String, String>,
    session_id: Option<String>,
    next_id: u64,
}

impl HttpClient {
    fn new(url: &str, headers: &HashMap<String, String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(Duration::from_secs(10))
            .user_agent(format!("ante/{}", crate::VERSION))
            .build()?;
        Ok(Self {
            http,
            url: url.to_string(),
            headers: headers.clone(),
            session_id: None,
            next_id: 1,
        })
    }

    async fn post(&mut self, payload: &Value) -> Result<Option<Value>> {
        let id = payload.get("id").and_then(Value::as_u64);
        let mut req = self
            .http
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json");
        if let Some(session) = &self.session_id {
            req = req.header("Mcp-Session-Id", session);
        }
        for (k, v) in &self.headers {
            req = req.header(k, v);
        }
        let resp = req
            .json(payload)
            .send()
            .await
            .with_context(|| format!("POST {}", self.url))?;
        if let Some(session) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            self.session_id = Some(session.to_string());
        }
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!(
                "MCP server requires OAuth (HTTP 401); add an Authorization header to the server config"
            ));
        }
        if !status.is_success() {
            return Err(anyhow!("MCP HTTP {} from {}", status.as_u16(), self.url));
        }
        if id.is_none() && status == reqwest::StatusCode::ACCEPTED {
            return Ok(None);
        }
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let body = resp.text().await.unwrap_or_default();
        if body.trim().is_empty() {
            return Ok(None);
        }
        if content_type.contains("text/event-stream") {
            parse_sse_response(&body, id)
        } else {
            let value: Value = serde_json::from_str(&body)
                .with_context(|| format!("invalid MCP JSON response: {body}"))?;
            Ok(Some(value))
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let message = self
            .post(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            }))
            .await?
            .ok_or_else(|| anyhow!("MCP {method} returned no response"))?;
        if let Some(error) = message.get("error") {
            return Err(anyhow!("MCP {method} failed: {error}"));
        }
        Ok(message.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn notify(&mut self, method: &str) -> Result<()> {
        self.post(&json!({"jsonrpc": "2.0", "method": method}))
            .await?;
        Ok(())
    }
}

fn parse_sse_response(body: &str, expected_id: Option<u64>) -> Result<Option<Value>> {
    for event in body.replace('\r', "").split("\n\n") {
        let mut data = String::new();
        for line in event.lines() {
            if let Some(value) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value.trim_start());
            }
        }
        if data.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&data)
            .with_context(|| format!("invalid MCP SSE data: {data}"))?;
        if let Some(id) = expected_id {
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                return Ok(Some(value));
            }
        } else {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

enum Client {
    Stdio(StdioClient),
    Http(HttpClient),
}

impl Client {
    async fn spawn(name: &str, cfg: &ServerConfig) -> Result<Self> {
        match &cfg.transport {
            TransportConfig::Stdio { .. } => Ok(Self::Stdio(StdioClient::spawn(name, cfg).await?)),
            TransportConfig::Http { url, headers } => {
                Ok(Self::Http(HttpClient::new(url, headers)?))
            }
        }
    }

    async fn initialize(&mut self) -> Result<()> {
        let (result, notify): (Value, Result<()>) = match self {
            Client::Stdio(c) => {
                let r = c
                    .request(
                        "initialize",
                        json!({
                            "protocolVersion": "2024-11-05",
                            "capabilities": {},
                            "clientInfo": {"name": "ante", "version": crate::VERSION},
                        }),
                    )
                    .await?;
                let n = c
                    .send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
                    .await;
                (r, n)
            }
            Client::Http(c) => {
                let r = c
                    .request(
                        "initialize",
                        json!({
                            "protocolVersion": "2024-11-05",
                            "capabilities": {},
                            "clientInfo": {"name": "ante", "version": crate::VERSION},
                        }),
                    )
                    .await?;
                let n = c.notify("notifications/initialized").await;
                (r, n)
            }
        };
        notify?;
        if result.get("protocolVersion").is_none() {
            return Err(anyhow!("MCP initialize response has no protocolVersion"));
        }
        Ok(())
    }

    async fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        let result = match self {
            Client::Stdio(c) => c.request("tools/list", json!({})).await?,
            Client::Http(c) => c.request("tools/list", json!({})).await?,
        };
        let tools = result
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("MCP tools/list response has no tools array"))?;
        Ok(tools
            .iter()
            .filter_map(|tool| {
                Some(McpTool {
                    server: String::new(),
                    name: tool.get("name")?.as_str()?.to_string(),
                    description: tool
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    input_schema: tool
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type":"object","properties":{}})),
                })
            })
            .collect())
    }

    async fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<ToolResult> {
        let result = match self {
            Client::Stdio(c) => {
                c.request("tools/call", json!({"name": name, "arguments": arguments}))
                    .await?
            }
            Client::Http(c) => {
                c.request("tools/call", json!({"name": name, "arguments": arguments}))
                    .await?
            }
        };
        let is_error = result.get("isError").and_then(Value::as_bool).unwrap_or(false);
        let mut text = String::new();
        if let Some(content) = result.get("content").and_then(Value::as_array) {
            for item in content {
                match item.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = item.get("text").and_then(Value::as_str) {
                            text.push_str(t);
                            text.push('\n');
                        }
                    }
                    Some(other) => {
                        text.push_str(&format!("[{other} content]\n"));
                    }
                    None => {}
                }
            }
        }
        if text.is_empty() {
            text = result.to_string();
        }
        Ok(ToolResult {
            content: text.trim_end().to_string(),
            is_error,
            images: Vec::new(),
        })
    }
}

pub struct McpManager {
    configs: HashMap<String, ServerConfig>,
    clients: tokio::sync::Mutex<HashMap<String, Client>>,
    tools: Mutex<Vec<McpTool>>,
}

impl McpManager {
    pub fn new(values: &std::collections::BTreeMap<String, Value>) -> Self {
        let mut configs = HashMap::new();
        for (name, value) in values {
            match ServerConfig::from_value(value) {
                Ok(cfg) => {
                    configs.insert(name.clone(), cfg);
                }
                Err(e) => eprintln!("warning: MCP server '{name}': {e}"),
            }
        }
        Self {
            configs,
            clients: tokio::sync::Mutex::new(HashMap::new()),
            tools: Mutex::new(Vec::new()),
        }
    }

    pub fn configured(&self) -> Vec<String> {
        self.configs.keys().cloned().collect()
    }

    pub fn tool_schemas(&self) -> Vec<Value> {
        self.tools
            .lock()
            .map(|tools| {
                tools
                    .iter()
                    .map(|t| {
                        json!({
                            "name": t.qualified_name(),
                            "description": t.description,
                            "parameters": t.input_schema,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub async fn start(&self) -> Result<()> {
        let mut clients = self.clients.lock().await;
        let mut discovered = Vec::new();
        let mut failures = Vec::new();
        for (name, cfg) in &self.configs {
            if clients.contains_key(name) {
                if let Some(client) = clients.get_mut(name) {
                    match client.list_tools().await {
                        Ok(mut tools) => {
                            for tool in &mut tools {
                                tool.server = name.clone();
                            }
                            discovered.extend(tools);
                        }
                        Err(e) => failures.push(format!("{name}: {e}")),
                    }
                }
                continue;
            }
            let result = async {
                let mut client = Client::spawn(name, cfg).await?;
                client.initialize().await?;
                let mut tools = client.list_tools().await?;
                for tool in &mut tools {
                    tool.server = name.clone();
                }
                discovered.extend(tools);
                clients.insert(name.clone(), client);
                Result::<()>::Ok(())
            }
            .await;
            if let Err(e) = result {
                failures.push(format!("{name}: {e}"));
            }
        }
        if let Ok(mut guard) = self.tools.lock() {
            *guard = discovered;
        }
        if !failures.is_empty() {
            return Err(anyhow!("MCP initialization failures: {}", failures.join("; ")));
        }
        Ok(())
    }

    pub async fn call(&self, qualified: &str, arguments: &Value) -> Result<ToolResult> {
        let rest = qualified
            .strip_prefix("mcp__")
            .ok_or_else(|| anyhow!("not an MCP tool: {qualified}"))?;
        let (server, tool) = rest
            .split_once("__")
            .ok_or_else(|| anyhow!("malformed MCP tool name: {qualified}"))?;
        let mut clients = self.clients.lock().await;
        let client = clients
            .get_mut(server)
            .ok_or_else(|| anyhow!("MCP server '{server}' is not connected"))?;
        client.call_tool(tool, arguments).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn stdio_discovery_and_call() {
        let script = format!("{}/tests/mcp_mock_server.py", env!("CARGO_MANIFEST_DIR"));
        let mut configs = std::collections::BTreeMap::new();
        configs.insert(
            "mock".to_string(),
            json!({"command": "python3", "args": ["-u", script]}),
        );
        let manager = McpManager::new(&configs);
        manager.start().await.expect("MCP server should start");
        let schemas = manager.tool_schemas();
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0]["name"], "mcp__mock__echo");
        let result = manager
            .call("mcp__mock__echo", &json!({"text": "hi"}))
            .await
            .expect("tool call");
        assert!(!result.is_error);
        assert_eq!(result.content, "echo:hi");
    }

    #[tokio::test]
    async fn http_sse_discovery_and_call() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(handle_http(stream));
            }
        });

        let mut configs = std::collections::BTreeMap::new();
        configs.insert(
            "httpmock".to_string(),
            json!({"url": format!("http://{addr}/mcp"), "transport": "http"}),
        );
        let manager = McpManager::new(&configs);
        manager.start().await.expect("HTTP MCP server should start");
        let schemas = manager.tool_schemas();
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0]["name"], "mcp__httpmock__echo");
        let result = manager
            .call("mcp__httpmock__echo", &json!({"text": "hi"}))
            .await
            .expect("tool call");
        assert!(!result.is_error);
        assert_eq!(result.content, "echo:hi");
    }

    async fn handle_http(stream: tokio::net::TcpStream) {
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                break;
            }
            if line == "\r\n" {
                continue;
            }
            let mut content_length = 0usize;
            loop {
                line.clear();
                let Ok(n) = reader.read_line(&mut line).await else {
                    break;
                };
                if n == 0 || line == "\r\n" {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; content_length];
            if reader.read_exact(&mut body).await.is_err() {
                break;
            }
            let request: Value = serde_json::from_slice(&body).unwrap_or_default();
            let id = request.get("id").cloned().unwrap_or(Value::Null);
            let method = request.get("method").and_then(Value::as_str).unwrap_or("");
            let result = match method {
                "initialize" => json!({"protocolVersion": "2024-11-05"}),
                "tools/list" => json!({"tools": [{
                    "name": "echo",
                    "description": "echo text",
                    "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}
                }]}),
                "tools/call" => json!({
                    "content": [{"type": "text", "text": format!(
                        "echo:{}",
                        request.pointer("/params/arguments/text").and_then(Value::as_str).unwrap_or("")
                    )}]
                }),
                _ => Value::Null,
            };
            let payload = json!({"jsonrpc": "2.0", "id": id, "result": result});
            let sse = format!("event: message\ndata: {}\n\n", payload);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nMcp-Session-Id: test-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                sse.len(),
                sse
            );
            if writer.write_all(response.as_bytes()).await.is_err() {
                break;
            }
            break;
        }
    }

    #[test]
    fn rejects_unknown_transport() {
        let error = ServerConfig::from_value(&json!({"transport": "carrier-pigeon", "command": "x"}))
            .expect_err("unknown transport must be rejected");
        assert!(error.to_string().contains("unsupported"));
    }

    #[test]
    fn expands_env_headers() {
        std::env::set_var("ANTE_MCP_TEST_TOKEN", "secret");
        assert_eq!(expand_env("Bearer ${ANTE_MCP_TEST_TOKEN}"), "Bearer secret");
    }
}
