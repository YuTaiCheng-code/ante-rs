//! Offline router lifecycle: resolve a model, launch a detached llama-server,
//! probe `/health` and `/v1/models`, persist a router record, and stop the
//! whole process group on request.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use crate::offline_engine;
use crate::util;

pub const VERIFIED_MODELS: &[&str] = &[
    "Devstral-Small-2-24B-Instruct-2512-Q4_K_M",
    "gemma-4-26B-A4B-it-UD-Q4_K_M",
    "gemma-4-E4B-it-Q4_K_M",
    "GLM-4.7-Flash-Q4_K_M",
    "Qwen3.5-122B-A10B-Q2_K",
    "Qwen3.5-9B-Q4_K_M",
    "Qwen3.6-27B-Q4_K_M",
    "Qwen3.6-35B-A3B-UD-Q4_K_M",
    "Qwen3.8-27B-Q4_K_M",
    "MiniMax-M2.7-UD-Q3_K_XL-00001-of-00004",
];

pub fn record_path() -> PathBuf {
    util::ante_home().join("router.json")
}

pub fn log_path() -> PathBuf {
    util::ante_home().join("llama-server.log")
}

pub fn config_path() -> PathBuf {
    util::ante_home().join("offline-config.json")
}

pub fn load_config() -> Value {
    let defaults = json!({
        "model_dir": util::ante_home().join("models").display().to_string(),
        "default_model": Value::Null,
        "context": Value::Null,
        "thinking": "on",
        "temperature": Value::Null,
        "extra": ""
    });
    let mut cfg: Value = fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    if let (Some(obj), Some(def)) = (cfg.as_object_mut(), defaults.as_object()) {
        for (k, v) in def {
            obj.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    cfg
}

pub fn model_dir() -> PathBuf {
    load_config()
        .get("model_dir")
        .and_then(|v| v.as_str())
        .map(|s| util::expand_tilde(s))
        .unwrap_or_else(|| util::ante_home().join("models"))
}

pub fn load_record() -> Option<Value> {
    let text = fs::read_to_string(record_path()).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    if pid_of(&value).map(pid_alive).unwrap_or(false) {
        Some(value)
    } else {
        None
    }
}

fn pid_of(record: &Value) -> Option<u32> {
    record.get("pid").and_then(|v| v.as_u64()).map(|v| v as u32)
}

fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    if cfg!(target_os = "linux") {
        Path::new(&format!("/proc/{pid}")).exists()
    } else {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

fn group_alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg("--")
        .arg(format!("-{pid}"))
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn model_matches(record: &Value, model: Option<&Path>) -> bool {
    match model {
        None => true,
        Some(p) => record
            .get("model_path")
            .and_then(|v| v.as_str())
            .map(|s| Path::new(s) == p)
            .unwrap_or(false),
    }
}

/// Resolve `--offline-model` exactly like the upstream CLI: a GGUF path, a
/// file in the model directory, a verified model name, or an http(s) URL.
pub fn resolve_model(arg: &str) -> Result<PathBuf> {
    let cfg = load_config();
    let dir = model_dir();
    fs::create_dir_all(&dir).ok();

    let raw = if arg.trim().is_empty() {
        cfg.get("default_model").and_then(|v| v.as_str()).unwrap_or("").to_string()
    } else {
        arg.to_string()
    };
    if raw.trim().is_empty() {
        return Err(anyhow!(
            "no offline model selected; pass --offline-model <GGUF|name|URL> or run `ante offline use <model>`"
        ));
    }

    if raw.starts_with("http://") || raw.starts_with("https://") {
        return download_model(&raw, &dir);
    }

    let expanded = util::expand_tilde(&raw);
    if expanded.is_file() {
        return Ok(expanded);
    }
    let in_dir = dir.join(&raw);
    if in_dir.is_file() {
        return Ok(in_dir);
    }
    let with_ext = dir.join(format!("{raw}.gguf"));
    if with_ext.is_file() {
        return Ok(with_ext);
    }
    if VERIFIED_MODELS.iter().any(|m| *m == raw) {
        return Err(anyhow!(
            "verified model `{raw}` is not downloaded; place {raw}.gguf in {}",
            dir.display()
        ));
    }
    Err(anyhow!(
        "offline model `{raw}` not found (looked in {} and {})",
        expanded.display(),
        dir.display()
    ))
}

fn download_model(url: &str, dir: &Path) -> Result<PathBuf> {
    let name = url
        .split('?')
        .next()
        .unwrap_or(url)
        .rsplit('/')
        .next()
        .filter(|s| s.ends_with(".gguf"))
        .ok_or_else(|| anyhow!("model URL must end in a .gguf filename: {url}"))?;
    let dest = dir.join(name);
    if dest.is_file() {
        return Ok(dest);
    }
    eprintln!("Downloading model... {url}");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3600))
        .connect_timeout(Duration::from_secs(20))
        .user_agent(format!("ante/{}", crate::VERSION))
        .build()?;
    let mut resp = client.get(url).send()?;
    if !resp.status().is_success() {
        return Err(anyhow!("model download failed: HTTP {}", resp.status()));
    }
    let part = dest.with_extension("gguf.part");
    let mut out = fs::File::create(&part)?;
    std::io::copy(&mut resp, &mut out)?;
    out.flush()?;
    fs::rename(&part, &dest)?;
    Ok(dest)
}

fn pick_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn http_get(port: u16, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().ok()?,
        Duration::from_millis(500),
    )
    .ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(1000))).ok()?;
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).ok()?;
    let mut buf = String::new();
    let _ = stream.read_to_string(&mut buf);
    Some(buf)
}

fn endpoint_ready(port: u16, alias: &str) -> bool {
    let health = http_get(port, "/health");
    let models = http_get(port, "/v1/models");
    let health_ok = health
        .as_deref()
        .map(|s| s.starts_with("HTTP/1.1 200") || s.starts_with("HTTP/1.0 200"))
        .unwrap_or(false);
    let model_ok = models
        .as_deref()
        .map(|s| {
            (s.starts_with("HTTP/1.1 200") || s.starts_with("HTTP/1.0 200"))
                && s.contains(alias)
        })
        .unwrap_or(false);
    health_ok && model_ok
}

fn alias_for(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("offline-model")
        .to_string()
}

fn write_record(record: &Value) -> Result<()> {
    fs::create_dir_all(util::ante_home())?;
    fs::write(record_path(), serde_json::to_string_pretty(record)?)?;
    Ok(())
}

pub struct RouterEndpoint {
    pub base_url: String,
    pub model: String,
    pub pid: u32,
    pub port: u16,
}

/// Start (or reuse) the home router for `model`.
pub fn ensure_router(model: Option<&Path>) -> Result<RouterEndpoint> {
    if let Some(record) = load_record() {
        if model_matches(&record, model) {
            let port = record.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
            let model_name = record
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("offline-model")
                .to_string();
            return Ok(RouterEndpoint {
                base_url: format!("http://127.0.0.1:{port}/v1"),
                model: model_name,
                pid: pid_of(&record).unwrap_or(0),
                port,
            });
        }
        stop()?;
    }

    if !offline_engine::installed() {
        return Err(anyhow!(
            "llama.cpp's llama multiplexer is missing; run `ante offline install` (or use /offline-mode in the TUI) to install the supported engine"
        ));
    }
    let model = match model {
        Some(p) => p.to_path_buf(),
        None => resolve_model("")?,
    };
    if !model.is_file() {
        return Err(anyhow!("offline model not found: {}", model.display()));
    }
    let alias = alias_for(&model);
    let port = pick_port()?;
    let cfg = load_config();
    let ctx = cfg
        .get("context")
        .and_then(|v| v.as_u64())
        .unwrap_or(8192)
        .max(512);

    let log = fs::File::create(log_path()).context("failed to create llama-server log file")?;
    let log_err = log.try_clone().context("failed to clone log file handle")?;

    let mut cmd = Command::new(offline_engine::engine_binary());
    cmd.arg("--model")
        .arg(&model)
        .arg("--alias")
        .arg(&alias)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--ctx-size")
        .arg(ctx.to_string())
        .arg("--n-gpu-layers")
        .arg("999")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    if let Some(extra) = cfg.get("extra").and_then(|v| v.as_str()) {
        for a in extra.split_whitespace() {
            cmd.arg(a);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let child = cmd
        .spawn()
        .with_context(|| format!("spawn {}", offline_engine::engine_binary().display()))?;
    let pid = child.id();
    drop(child);

    let deadline = Instant::now()
        + Duration::from_secs(
            std::env::var("ANTE_OFFLINE_START_TIMEOUT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(600),
        );
    let mut announced = false;
    loop {
        if endpoint_ready(port, &alias) {
            if !announced {
                eprintln!("Started llama.cpp router (PID {pid}, port {port})");
                announced = true;
            }
            break;
        }
        if !pid_alive(pid) {
            let tail = fs::read_to_string(log_path()).unwrap_or_default();
            let tail: String = tail.lines().rev().take(8).collect::<Vec<_>>().join("\n");
            return Err(anyhow!(
                "llama.cpp router exited before becoming ready:\n{tail}"
            ));
        }
        if Instant::now() >= deadline {
            let _ = stop_pid(pid, false);
            return Err(anyhow!("timed out starting llama.cpp router"));
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    let started = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let record = json!({
        "pid": pid,
        "port": port,
        "base_url": format!("http://127.0.0.1:{port}/v1"),
        "model": alias,
        "model_path": model.display().to_string(),
        "started_at_secs": started,
        "build": offline_engine::PINNED_VERSION,
        "log": log_path().display().to_string(),
    });
    write_record(&record)?;

    Ok(RouterEndpoint {
        base_url: format!("http://127.0.0.1:{port}/v1"),
        model: alias,
        pid,
        port,
    })
}

fn stop_pid(pid: u32, verbose: bool) -> Result<()> {
    if pid == 0 {
        return Ok(());
    }
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg("--")
        .arg(format!("-{pid}"))
        .stderr(Stdio::null())
        .status();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !group_alive(pid) {
            if verbose {
                eprintln!("llama.cpp router stopped");
            }
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if verbose {
        eprintln!("llama.cpp router did not stop gracefully; terminated its process group");
    }
    let _ = Command::new("kill")
        .arg("-KILL")
        .arg("--")
        .arg(format!("-{pid}"))
        .stderr(Stdio::null())
        .status();
    Ok(())
}

pub fn stop() -> Result<bool> {
    let record = match load_record() {
        Some(r) => r,
        None => {
            let _ = fs::remove_file(record_path());
            return Ok(false);
        }
    };
    let pid = pid_of(&record).unwrap_or(0);
    stop_pid(pid, true)?;
    let _ = fs::remove_file(record_path());
    Ok(true)
}

pub fn status() -> Value {
    match load_record() {
        Some(r) => json!({
            "router": "running",
            "pid": pid_of(&r),
            "port": r.get("port").cloned().unwrap_or(Value::Null),
            "model": r.get("model").cloned().unwrap_or(Value::Null),
            "base_url": r.get("base_url").cloned().unwrap_or(Value::Null),
            "started_at_secs": r.get("started_at_secs").cloned().unwrap_or(Value::Null),
            "log": log_path().display().to_string(),
        }),
        None => json!({"router": "stopped", "servers": []}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_strips_gguf() {
        assert_eq!(alias_for(Path::new("/tmp/Qwen3.5-9B-Q4_K_M.gguf")), "Qwen3.5-9B-Q4_K_M");
    }

    #[test]
    fn verified_list_is_nonempty() {
        assert!(!VERIFIED_MODELS.is_empty());
    }
}
