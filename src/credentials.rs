use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::catalog::Catalog;
use crate::util::ante_home;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSecret {
    #[serde(default = "default_kind")]
    pub kind: String, // "secret" | "oauth"
    #[serde(default)]
    pub value: String, // api key or access token
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

pub fn credentials_dir() -> PathBuf {
    ante_home().join("auth/credentials")
}

pub fn kind_dir(kind: &str) -> PathBuf {
    credentials_dir().join(kind)
}

fn default_kind() -> String {
    "secret".into()
}

fn entry_path(name: &str) -> PathBuf {
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '/' || c == '.' { c } else { '_' })
        .collect();
    let safe = safe.replace('/', "__");
    kind_dir("secret").join(format!("{safe}.json"))
}

pub fn list() -> Vec<(String, StoredSecret)> {
    let mut out = Vec::new();
    for (sub, kind) in [("secret", "secret"), ("oauth", "oauth")] {
      let dir = kind_dir(sub);
      if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .replace("__", "/");
            if let Ok(s) = std::fs::read_to_string(&path) {
                if let Ok(mut v) = serde_json::from_str::<StoredSecret>(&s) {
                    if v.kind.is_empty() { v.kind = kind.to_string(); }
                    out.push((name, v));
                }
            }
        }
      }
    }
    // legacy api_keys.json
    let legacy = ante_home().join("api_keys.json");
    if legacy.exists() {
        if let Ok(s) = std::fs::read_to_string(&legacy) {
            if let Ok(map) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&s) {
                for (k, v) in map {
                    if let Some(key) = v.as_str() {
                        if !out.iter().any(|(n, _)| n == &k) {
                            out.push((
                                k.clone(),
                                StoredSecret {
                                    kind: "secret".into(),
                                    value: key.to_string(),
                                    refresh_token: None,
                                    expires_at: None,
                                    account: None,
                                    profile: None,
                                },
                            ));
                        }
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

pub fn get(name: &str) -> Option<StoredSecret> {
    list().into_iter().find(|(n, _)| n == name).map(|(_, v)| v)
}

pub fn save(name: &str, secret: &StoredSecret) -> Result<()> {
    let dir = kind_dir(&secret.kind);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{name}.json"));
    let json = serde_json::to_string_pretty(secret)?;
    crate::util::write_file(&path, &json)
}

pub fn add_secret(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(anyhow!("a secret cannot be empty"));
    }
    let path = entry_path(name);
    let json = serde_json::to_string_pretty(&serde_json::json!({ "value": value.trim() }))?;
    crate::util::write_file(&path, &json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn remove(spec: &str) -> Result<String> {
    let name = spec
        .strip_prefix("secret:")
        .or_else(|| spec.strip_prefix("oauth:"))
        .or_else(|| spec.strip_prefix("helper:"))
        .unwrap_or(spec);
    let path = if spec.starts_with("oauth:") {
        kind_dir("oauth").join(format!("{name}.json"))
    } else {
        entry_path(name)
    };
    let removed = std::fs::remove_file(&path).is_ok();
    // also prune legacy api_keys.json entries
    let legacy = ante_home().join("api_keys.json");
    if removed && legacy.exists() {
        // best-effort; legacy entries are read-only here
    }
    if removed {
        Ok(format!("Removed secret:{name}."))
    } else {
        Err(anyhow!("no stored credential named '{name}'"))
    }
    .map_err(|e| {
        if spec.starts_with("helper:") {
            anyhow!("no cached helper output named '{name}'")
        } else {
            e
        }
    })
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub value: String,
    pub header: String,
    pub source: String,
}

/// Resolve a provider's credential spec: `env:NAME`, `secret:NAME`, `oauth:PROFILE[/label]`,
/// `helper:NAME`, a list tried in order, or null.
pub fn resolve(spec: &serde_json::Value, catalog: &Catalog) -> Option<Resolved> {
    if spec.is_null() {
        return None;
    }
    if let Some(s) = spec.as_str() {
        return resolve_one(s, catalog);
    }
    if let Some(arr) = spec.as_array() {
        for item in arr {
            if let Some(s) = item.as_str() {
                if let Some(r) = resolve_one(s, catalog) {
                    return Some(r);
                }
            }
        }
    }
    None
}

fn resolve_one(spec: &str, catalog: &Catalog) -> Option<Resolved> {
    if let Some(var) = spec.strip_prefix("env:") {
        let v = std::env::var(var).ok().filter(|x| !x.is_empty())?;
        return Some(Resolved {
            value: v,
            header: "Authorization".into(),
            source: format!("env:{var}"),
        });
    }
    if let Some(name) = spec.strip_prefix("secret:").or_else(|| spec.strip_prefix("oauth:")) {
        let s = get(name)?;
        if spec.starts_with("oauth:") && s.kind != "oauth" {
            return None;
        }
        return Some(Resolved {
            value: s.value,
            header: if spec.starts_with("oauth:") {
                "Authorization".into()
            } else {
                "Authorization".into()
            },
            source: format!("{spec}"),
        });
    }
    if let Some(name) = spec.strip_prefix("helper:") {
        match helper_value(name, catalog) {
            Ok(v) => {
                return Some(Resolved {
                    value: v,
                    header: "Authorization".into(),
                    source: format!("helper:{name}"),
                })
            }
            Err(e) => {
                eprintln!("helper `{name}` failed: {e}");
                return None;
            }
        }
    }
    // bare name: treat as stored secret
    let s = get(spec)?;
    Some(Resolved {
        value: s.value,
        header: "Authorization".into(),
        source: format!("secret:{spec}"),
    })
}

pub fn apply_header(
    builder: reqwest::RequestBuilder,
    provider: &crate::catalog::ProviderSpec,
    cred: &Resolved,
) -> reqwest::RequestBuilder {
    let header = provider
        .credential_header
        .clone()
        .unwrap_or_else(|| "Authorization".into());
    let value = if header.eq_ignore_ascii_case("authorization") && !cred.value.starts_with("Bearer ") {
        format!("Bearer {}", cred.value)
    } else {
        cred.value.clone()
    };
    builder.header(header, value)
}

pub fn store_oauth(name: &str, access: &str, refresh: Option<String>, expires_at: Option<u64>, profile: &str) -> Result<()> {
    save(
        name,
        &StoredSecret {
            kind: "oauth".into(),
            value: access.to_string(),
            refresh_token: refresh,
            expires_at,
            account: Some(name.to_string()),
            profile: Some(profile.to_string()),
        },
    )
    .with_context(|| format!("store sign-in for {name}"))
}

/// 展开 `${NAME}` 与 `${NAME:-default}`。
fn expand_env(input: &str) -> String {
    let mut out = String::new();
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            out.push_str(&rest[start..]);
            break;
        };
        let token = &rest[start + 2..start + end];
        let (name, default) = match token.split_once(":-") {
            Some((n, d)) => (n, Some(d)),
            None => (token, None),
        };
        match std::env::var(name) {
            Ok(v) if !v.is_empty() => out.push_str(&v),
            _ => out.push_str(default.unwrap_or("")),
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

fn helper_spec<'a>(name: &str, catalog: &'a Catalog) -> Option<&'a serde_json::Value> {
    catalog.helpers.get(name)
}

fn helper_command(spec: &serde_json::Value) -> Option<Vec<String>> {
    match spec.get("command")? {
        serde_json::Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .filter_map(|v| v.as_str())
                .map(expand_env)
                .collect();
            if parts.is_empty() || parts[0].is_empty() {
                None
            } else {
                Some(parts)
            }
        }
        // 字符串按空白切分，不经过 shell，避免管道与变量注入
        serde_json::Value::String(s) => {
            let parts: Vec<String> = expand_env(s)
                .split_whitespace()
                .map(|x| x.to_string())
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts)
            }
        }
        _ => None,
    }
}

fn helper_cache_path(name: &str) -> PathBuf {
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect();
    ante_home().join("auth/helper_cache").join(format!("{safe}.json"))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 运行 helper：不经过 shell，最多 15 秒，取输出首行；成功后按 `ttl_secs` 缓存。
fn helper_value(name: &str, catalog: &Catalog) -> Result<String> {
    let spec = helper_spec(name, catalog).ok_or_else(|| anyhow!("helper `{name}` is not in catalog.json"))?;
    let ttl = spec.get("ttl_secs").and_then(|v| v.as_u64()).unwrap_or(0).min(31_536_000);
    let cache = helper_cache_path(name);
    if ttl > 0 {
        if let Ok(raw) = std::fs::read_to_string(&cache) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                let at = v.get("at").and_then(|x| x.as_u64()).unwrap_or(0);
                if now_secs().saturating_sub(at) < ttl {
                    if let Some(s) = v.get("value").and_then(|x| x.as_str()) {
                        return Ok(s.to_string());
                    }
                }
            }
        }
    }
    let parts = helper_command(spec).ok_or_else(|| anyhow!("helper `{name}` has no runnable `command`"))?;
    let mut child = std::process::Command::new(&parts[0])
        .args(&parts[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("helper `{name}` could not start `{}`: {e}", parts[0]))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = child.wait_with_output().map_err(|e| anyhow!("helper `{name}`: {e}"))?;
                if !status.success() {
                    let err = String::from_utf8_lossy(&out.stderr);
                    return Err(anyhow!(
                        "helper `{name}` exited {}: {}",
                        status.code().unwrap_or(-1),
                        err.lines().next().unwrap_or("")
                    ));
                }
                let value = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if value.is_empty() {
                    return Err(anyhow!("helper `{name}` printed no credential on its first line"));
                }
                if ttl > 0 {
                    if let Some(dir) = cache.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let payload = serde_json::json!({"value": value, "at": now_secs()});
                    let _ = crate::util::write_file(&cache, &serde_json::to_string(&payload)?);
                }
                return Ok(value);
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(anyhow!("helper `{name}` timed out after 15s"));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => return Err(anyhow!("helper `{name}`: {e}")),
        }
    }
}
