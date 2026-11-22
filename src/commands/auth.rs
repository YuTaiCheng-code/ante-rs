use std::io::Read;

use crate::cli::{AuthCmd, AuthSub};

pub async fn run(cmd: AuthCmd) -> i32 {
    match cmd.command {
        AuthSub::List => {
            let entries = crate::credentials::list();
            if entries.is_empty() {
                println!("No stored credentials.");
                println!("Run `ante auth login <profile>` to sign in or `ante auth add <connector>` to store a key.");
                return 0;
            }
            let catalog = crate::catalog::Catalog::load();
            let mut rows: Vec<[String; 6]> = Vec::new();
            for (name, s) in entries {
                let used_by: Vec<String> = catalog
                    .providers
                    .iter()
                    .filter(|(_, p)| credential_mentions(&p.credential, &name))
                    .map(|(id, _)| id.clone())
                    .collect();
                rows.push([
                    name,
                    s.kind.clone(),
                    s.profile.clone().unwrap_or_else(|| "-".into()),
                    s.account.clone().unwrap_or_else(|| "-".into()),
                    "set".into(),
                    if used_by.is_empty() { "-".into() } else { used_by.join(",") },
                ]);
            }
            let headers = ["NAME", "KIND", "ISSUER", "ACCOUNT", "STATE", "USED BY"];
            let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
            for r in &rows {
                for (i, c) in r.iter().enumerate() {
                    widths[i] = widths[i].max(c.len());
                }
            }
            let mut line = String::new();
            for (i, h) in headers.iter().enumerate() {
                line.push_str(&format!("{:<width$}", h, width = widths[i] + 2));
            }
            println!("{}", line.trim_end());
            for r in &rows {
                let mut line = String::new();
                for (i, c) in r.iter().enumerate() {
                    line.push_str(&format!("{:<width$}", c, width = widths[i] + 2));
                }
                println!("{}", line.trim_end());
            }
            0
        }
        AuthSub::Add(n) => {
            let catalog = crate::catalog::Catalog::load();
            let provider = catalog.provider(&n.name).cloned();
            let has_secret_slot = provider
                .as_ref()
                .map(|p| credential_mentions(&p.credential, &format!("secret:{}", n.name)))
                .unwrap_or(true);
            if !has_secret_slot {
                let env = env_var_for(&provider.as_ref().unwrap().credential);
                match env {
                    Some(e) => println!("connector `{}` reads no stored secret; set {}", n.name, e),
                    None => println!("connector `{}` reads no stored secret", n.name),
                }
                return 1;
            }
            let mut buf = String::new();
            if std::io::stdin().read_to_string(&mut buf).is_err() {
                eprintln!("failed to read the secret from stdin");
                return 1;
            }
            if buf.trim().is_empty() {
                eprintln!("a secret cannot be empty");
                return 1;
            }
            if let Some(p) = &provider {
                match validate_key(p, buf.trim()).await {
                    Ok(true) => {}
                    Ok(false) => {}
                    Err(status) => {
                        println!(
                            "{} rejected this key (HTTP {} {}); nothing stored",
                            p.display_name, status.0, status.1
                        );
                        return 1;
                    }
                }
            }
            match crate::credentials::add_secret(&n.name, &buf) {
                Ok(_) => {
                    println!("Stored secret:{}.", n.name);
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        AuthSub::Refresh(n) => {
            let catalog = crate::catalog::Catalog::load();
            match crate::credentials::refresh_stored(&n.name, &catalog, true).await {
                Ok(true) => {
                    println!("Refreshed OAuth token for {}", n.name);
                    0
                }
                Ok(false) => {
                    println!("No refresh was needed for {}", n.name);
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        AuthSub::Rm(n) => match crate::credentials::remove(&n.name) {
            Ok(msg) => {
                println!("{msg}");
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
        AuthSub::Logout(n) => match crate::credentials::remove(&format!("oauth:{}", n.name)) {
            Ok(msg) => {
                println!("{msg}");
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
        AuthSub::Login(l) => login(l).await,
    }
}

fn env_var_for(spec: &serde_json::Value) -> Option<String> {
    match spec {
        serde_json::Value::String(s) => s.strip_prefix("env:").map(|e| e.to_string()),
        serde_json::Value::Array(a) => a.iter().find_map(env_var_for),
        _ => None,
    }
}

async fn validate_key(provider: &crate::catalog::ProviderSpec, key: &str) -> Result<bool, (u16, String)> {
    let url = format!("{}/models", provider.base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| (0u16, "client error".to_string()))?;
    let mut req = client.get(&url);
    match provider.credential_header.as_deref() {
        Some(h) if !h.eq_ignore_ascii_case("authorization") => {
            req = req.header(h, key);
        }
        _ => {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
    }
    if provider.wire_style == "AnthropicMessage" {
        req = req.header("x-api-key", key);
    }
    match req.send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                Ok(true)
            } else {
                let code = resp.status().as_u16();
                let reason = resp.status().canonical_reason().unwrap_or("").to_string();
                Err((code, reason))
            }
        }
        Err(_) => Ok(false),
    }
}

fn credential_mentions(spec: &serde_json::Value, name: &str) -> bool {
    match spec {
        serde_json::Value::String(s) => s.contains(name),
        serde_json::Value::Array(a) => a.iter().any(|v| credential_mentions(v, name)),
        _ => false,
    }
}

async fn login(l: crate::cli::AuthLogin) -> i32 {
    let catalog = crate::catalog::Catalog::load();
    let base = l.name.split('/').next().unwrap_or("");
    let profile = catalog
        .oauth_profiles
        .get(base)
        .cloned()
        .or_else(|| {
            catalog
                .providers
                .get(base)
                .and_then(|_| catalog.oauth_profiles.values().next().cloned())
        });
    let profile = match profile {
        Some(p) => p,
        None => {
            eprintln!(
                "No OAuth profile named '{base}'. Built-in OAuth sign-in is not wired in this build.\n\
                 Add an `oauth_profiles` entry to {}/catalog.json, or store an API key with `ante auth add {base}`.",
                crate::util::ante_home().display()
            );
            return 1;
        }
    };
    let auth_url = profile
        .get("auth_url")
        .or_else(|| profile.get("authorize_url"))
        .and_then(|v| v.as_str());
    let token_url = profile.get("token_url").and_then(|v| v.as_str());
    let client_id = profile.get("client_id").and_then(|v| v.as_str()).unwrap_or("ante");
    let (Some(auth_url), Some(token_url)) = (auth_url, token_url) else {
        eprintln!("oauth profile '{base}' needs `auth_url` and `token_url`");
        return 1;
    };
    let verifier = format!("{}{}", crate::util::ulid(), crate::util::ulid());
    let challenge = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(verifier.as_bytes());
        let d = h.finalize();
        base64_url(&d)
    };
    let redirect = format!(
        "http://127.0.0.1:{}/callback",
        l.callback_port.unwrap_or(54545)
    );
    let url = format!(
        "{auth_url}?response_type=code&client_id={client_id}&redirect_uri={}&code_challenge={challenge}&code_challenge_method=S256",
        urlencode(&redirect)
    );
    println!("Open this URL to sign in:\n\n{url}\n");
    if l.no_browser {
        println!("Waiting for the callback on {redirect} ...");
    }
    let code = wait_for_code(l.callback_port.unwrap_or(54545), &l.name).await;
    let code = match code {
        Some(c) => c,
        None => {
            eprintln!("sign-in did not complete");
            return 1;
        }
    };
    let http = reqwest::Client::new();
    let resp = http
        .post(token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("client_id", client_id),
            ("code_verifier", verifier.as_str()),
            ("redirect_uri", redirect.as_str()),
        ])
        .send()
        .await;
    match resp {
        Ok(r) if r.status().is_success() => {
            let v: serde_json::Value = r.json().await.unwrap_or_default();
            let access = v.get("access_token").and_then(|t| t.as_str()).unwrap_or("");
            let refresh = v.get("refresh_token").and_then(|t| t.as_str()).map(|s| s.to_string());
            let expires = v.get("expires_in").and_then(|e| e.as_u64()).map(|e| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    + e
            });
            match crate::credentials::store_oauth(&l.name, access, refresh, expires, base) {
                Ok(_) => {
                    println!("Signed in as {}", l.name);
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        Ok(r) => {
            eprintln!("token exchange failed: HTTP {}", r.status());
            1
        }
        Err(e) => {
            eprintln!("token exchange failed: {e}");
            1
        }
    }
}

async fn wait_for_code(port: u16, name: &str) -> Option<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("could not listen on 127.0.0.1:{port}: {e}");
            return None;
        }
    };
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
    loop {
        let accept = tokio::time::timeout_at(deadline, listener.accept()).await;
        let Ok(Ok((mut stream, _))) = accept else { return None };
        let mut buf = String::new();
        let _ = stream.read_to_string(&mut buf).await;
        if let Some(code) = parse_query(&buf, "code") {
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/html\r\n\r\nSigned in. You can close this window.")
                .await;
            return Some(code);
        }
        if let Some(err) = parse_query(&buf, "error") {
            eprintln!("sign-in error: {err} for {name}");
            return None;
        }
    }
}

fn parse_query(head: &str, key: &str) -> Option<String> {
    let line = head.lines().next()?;
    let path = line.split_whitespace().nth(1)?;
    let query = path.split_once('?')?.1;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key {
            return Some(url_decode(v));
        }
    }
    None
}

fn url_decode(s: &str) -> String {
    let mut out = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v as char);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(' ');
        } else {
            out.push(bytes[i] as char);
        }
        i += 1;
    }
    out
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn base64_url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
