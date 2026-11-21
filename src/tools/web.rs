use serde_json::Value;
use std::time::Duration;

use super::{ToolCtx, ToolResult};

pub async fn fetch(ctx: &ToolCtx, input: &Value) -> ToolResult {
    let url = match input.get("url").and_then(|u| u.as_str()) {
        Some(u) => u,
        None => return ToolResult::err("`url` is required"),
    };
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .user_agent("ante/0.3.4")
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("failed to build http client: {e}")),
    };
    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => return ToolResult::err(format!("failed to fetch {url}: {e}")),
    };
    let status = resp.status();
    if !status.is_success() {
        return ToolResult::err(format!("fetch failed: HTTP {status} for {url}"));
    }
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => return ToolResult::err(format!("failed to read body: {e}")),
    };
    let text = if content_type.contains("html") || body.trim_start().starts_with("<!") {
        html_to_text(&body)
    } else {
        body
    };
    let mut out = format!("URL: {url}\n\n");
    out.push_str(&super::limit_output(&text));
    ToolResult::ok(out)
}

fn html_to_text(html: &str) -> String {
    let mut s = html.to_string();
    for tag in ["script", "style", "noscript", "svg", "head"] {
        let open = format!("<{tag}");
        let close = format!("</{tag}>");
        loop {
            let lower = s.to_ascii_lowercase();
            let Some(start) = lower.find(&open) else { break };
            let Some(end_rel) = lower[start..].find(&close) else { break };
            let end = start + end_rel + close.len();
            s.replace_range(start..end, " ");
        }
    }
    let mut out = String::new();
    let mut in_tag = false;
    let mut last_blank = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if in_tag => {}
            '&' => out.push('&'),
            _ => {
                if c.is_whitespace() {
                    if !last_blank {
                        out.push(' ');
                        last_blank = true;
                    }
                } else {
                    out.push(c);
                    last_blank = false;
                }
            }
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    out.split('\n')
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub async fn search(_ctx: &ToolCtx, input: &Value) -> ToolResult {
    let query = match input.get("query").and_then(|q| q.as_str()) {
        Some(q) => q,
        None => return ToolResult::err("`query` is required"),
    };
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124 Safari/537.36")
        .build()
    {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("failed to build http client: {e}")),
    };
    let encoded: String = urlencode(query);
    let url = format!("https://html.duckduckgo.com/html/?q={encoded}");
    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => return ToolResult::err(format!("web search failed: {e}")),
    };
    if !resp.status().is_success() {
        return ToolResult::err(format!(
            "web search failed: HTTP {} (search backend unavailable)",
            resp.status()
        ));
    }
    let html = resp.text().await.unwrap_or_default();
    let re = regex::Regex::new(
        r#"<a[^>]*class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#,
    )
    .unwrap();
    let mut items = Vec::new();
    for cap in re.captures_iter(&html) {
        let link = html_unescape(&cap[1]);
        let title = strip_tags(&cap[2]);
        if title.trim().is_empty() {
            continue;
        }
        items.push((title, link));
        if items.len() >= 10 {
            break;
        }
    }
    if items.is_empty() {
        return ToolResult::err(format!(
            "web search returned no results for '{query}' (the search backend may be rate limited)"
        ));
    }
    let mut out = format!("Search results for: {query}\n\n");
    for (i, (title, link)) in items.iter().enumerate() {
        out.push_str(&format!("{}. {}\n   {}\n", i + 1, title, link));
    }
    ToolResult::ok(out)
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    html_unescape(&out).trim().to_string()
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
