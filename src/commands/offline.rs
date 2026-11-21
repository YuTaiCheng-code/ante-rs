use serde_json::{json, Value};

use crate::cli::{OfflineCmd, OfflineSub};
use crate::settings::Settings;

fn config_path() -> std::path::PathBuf {
    crate::util::ante_home().join("offline-config.json")
}

fn load_config() -> Value {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| {
            json!({
                "model_dir": crate::util::ante_home().join("models").display().to_string(),
                "default_model": Value::Null,
                "context": Value::Null,
                "thinking": "on",
                "temperature": Value::Null,
                "extra": ""
            })
        })
}

fn engine_path() -> std::path::PathBuf {
    crate::util::ante_home().join("offline/llama-server")
}

pub fn run(cmd: OfflineCmd, _settings: &Settings) -> i32 {
    match cmd.command {
        OfflineSub::Status(c) => {
            let cfg = load_config();
            let installed = engine_path().exists();
            let model_dir = cfg
                .get("model_dir")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if c.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "engine": if installed { "installed" } else { "not installed" },
                        "engine_path": engine_path().display().to_string(),
                        "model_dir": model_dir,
                        "default_model": cfg.get("default_model").cloned().unwrap_or(Value::Null),
                        "router": "stopped",
                        "servers": [],
                    }))
                    .unwrap_or_default()
                );
            } else {
                println!("Ante offline status");
                println!("  engine:       {}", if installed { "installed (llama.cpp)" } else { "not installed" });
                println!("  model dir:    {model_dir}");
                println!(
                    "  default:      {}",
                    cfg.get("default_model").and_then(|v| v.as_str()).unwrap_or("-")
                );
                println!("  router:       stopped");
            }
            0
        }
        OfflineSub::List(c) => {
            let cfg = load_config();
            let dir = cfg
                .get("model_dir")
                .and_then(|v| v.as_str())
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| crate::util::ante_home().join("models"));
            let mut models = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().and_then(|x| x.to_str()) == Some("gguf") {
                        let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                        models.push(json!({
                            "name": p.file_stem().and_then(|s| s.to_str()).unwrap_or(""),
                            "path": p.display().to_string(),
                            "size_bytes": size,
                        }));
                    }
                }
            }
            if c.json {
                println!("{}", serde_json::to_string_pretty(&models).unwrap_or_default());
            } else if models.is_empty() {
                println!("No local GGUF models found in {}", dir.display());
            } else {
                for m in &models {
                    println!(
                        "{}  {}",
                        m.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        m.get("path").and_then(|v| v.as_str()).unwrap_or("")
                    );
                }
            }
            0
        }
        OfflineSub::Use(u) => {
            let mut cfg = load_config();
            if let Some(o) = cfg.as_object_mut() {
                o.insert("default_model".into(), json!(u.model));
                if let Some(c) = u.context {
                    o.insert("context".into(), json!(c));
                }
                if let Some(t) = u.tokens {
                    o.insert("tokens".into(), json!(t));
                }
                if let Some(t) = u.thinking {
                    o.insert("thinking".into(), json!(t));
                }
                if let Some(t) = u.temperature {
                    o.insert("temperature".into(), json!(t));
                }
                if let Some(e) = u.extra {
                    o.insert("extra".into(), json!(e));
                }
            }
            let _ = std::fs::create_dir_all(crate::util::ante_home());
            match std::fs::write(
                config_path(),
                serde_json::to_string_pretty(&cfg).unwrap_or_default(),
            ) {
                Ok(_) => {
                    println!(
                        "Saved to {}\nDefault model set to {}",
                        config_path().display(),
                        cfg.get("default_model").and_then(|v| v.as_str()).unwrap_or("")
                    );
                    0
                }
                Err(e) => {
                    eprintln!("failed to save configuration: {e}");
                    1
                }
            }
        }
        OfflineSub::Install => {
            eprintln!(
                "Offline mode is not available in this build: it needs the pinned llama.cpp engine at {}.\n\
                 Install it with the upstream Ante release, or run against a local OpenAI-compatible server by adding a catalog provider.",
                engine_path().display()
            );
            1
        }
        OfflineSub::Stop => {
            println!("No offline router is running.");
            0
        }
    }
}
