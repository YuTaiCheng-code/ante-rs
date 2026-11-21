use crate::cli::UsageCmd;

pub async fn run(cmd: UsageCmd) -> i32 {
    let creds = crate::credentials::list();
    let targets: Vec<(String, crate::credentials::StoredSecret)> = creds
        .into_iter()
        .filter(|(_, s)| s.kind == "oauth")
        .filter(|(n, _)| cmd.name.as_ref().map(|x| x == n || n.starts_with(&format!("{x}/"))).unwrap_or(true))
        .collect();
    if targets.is_empty() {
        println!("No supported stored OAuth sign-ins.");
        return 0;
    }
    let client = reqwest::Client::new();
    let mut reports = Vec::new();
    for (name, secret) in targets {
        let base = name.split('/').next().unwrap_or("").to_string();
        let url = if base.contains("anthropic") {
            "https://api.anthropic.com/api/oauth/usage"
        } else if base.contains("openai") {
            "https://chatgpt.com/backend-api/wham/usage"
        } else {
            reports.push(serde_json::json!({
                "name": name, "status": "unavailable",
                "detail": "no usage endpoint for this provider"
            }));
            continue;
        };
        let resp = client
            .get(url)
            .header("authorization", format!("Bearer {}", secret.value))
            .header("anthropic-beta", "oauth-2025-04-20")
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {
                let body: serde_json::Value = r.json().await.unwrap_or_default();
                reports.push(serde_json::json!({"name": name, "status": "ok", "usage": body}));
            }
            Ok(r) => reports.push(serde_json::json!({
                "name": name, "status": "error", "http_status": r.status().as_u16()
            })),
            Err(e) => reports.push(serde_json::json!({
                "name": name, "status": "error", "detail": e.to_string()
            })),
        }
    }
    if cmd.json {
        println!("{}", serde_json::to_string_pretty(&reports).unwrap_or_default());
        return 0;
    }
    for r in &reports {
        let name = r.get("name").and_then(|v| v.as_str()).unwrap_or("");
        match r.get("status").and_then(|v| v.as_str()) {
            Some("ok") => {
                println!("{name}:");
                println!(
                    "  {}",
                    serde_json::to_string_pretty(r.get("usage").unwrap_or(&serde_json::Value::Null))
                        .unwrap_or_default()
                );
            }
            Some("unavailable") => println!("{name}: no usage limits reported"),
            _ => println!("{name}: usage request failed"),
        }
    }
    0
}
