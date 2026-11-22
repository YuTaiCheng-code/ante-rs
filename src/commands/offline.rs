use serde_json::{json, Value};

use crate::cli::{OfflineCmd, OfflineSub};
use crate::offline_engine;
use crate::offline_router;
use crate::settings::Settings;

fn config_path() -> std::path::PathBuf {
    offline_router::config_path()
}

fn load_config() -> Value {
    offline_router::load_config()
}

pub fn run(cmd: OfflineCmd, _settings: &Settings) -> i32 {
    match cmd.command {
        OfflineSub::Status(c) => {
            let cfg = load_config();
            let installed = offline_engine::installed();
            let model_dir = cfg
                .get("model_dir")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let router = offline_router::status();
            if c.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "engine": if installed { "installed" } else { "not installed" },
                        "engine_path": offline_engine::engine_binary().display().to_string(),
                        "pinned_version": offline_engine::PINNED_VERSION,
                        "model_dir": model_dir,
                        "default_model": cfg.get("default_model").cloned().unwrap_or(Value::Null),
                        "router": router.get("router").cloned().unwrap_or(Value::Null),
                        "pid": router.get("pid").cloned().unwrap_or(Value::Null),
                        "port": router.get("port").cloned().unwrap_or(Value::Null),
                        "servers": [],
                    }))
                    .unwrap_or_default()
                );
            } else {
                println!("Ante offline status");
                println!(
                    "  engine:       {}",
                    if installed {
                        format!("installed (llama.cpp {})", offline_engine::PINNED_VERSION)
                    } else {
                        "not installed".to_string()
                    }
                );
                println!("  engine path:  {}", offline_engine::engine_binary().display());
                println!("  model dir:    {model_dir}");
                println!(
                    "  default:      {}",
                    cfg.get("default_model").and_then(|v| v.as_str()).unwrap_or("-")
                );
                match router.get("router").and_then(|v| v.as_str()) {
                    Some("running") => {
                        println!(
                            "  router:       running (PID {}, port {})",
                            router.get("pid").and_then(|v| v.as_u64()).unwrap_or(0),
                            router.get("port").and_then(|v| v.as_u64()).unwrap_or(0)
                        );
                        println!(
                            "  model:        {}",
                            router.get("model").and_then(|v| v.as_str()).unwrap_or("-")
                        );
                        println!(
                            "  endpoint:     {}",
                            router.get("base_url").and_then(|v| v.as_str()).unwrap_or("-")
                        );
                    }
                    _ => println!("  router:       stopped"),
                }
            }
            0
        }
        OfflineSub::List(c) => {
            let dir = offline_router::model_dir();
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
                            "source": "local",
                        }));
                    }
                }
            }
            let local_names: Vec<String> = models
                .iter()
                .filter_map(|m| m.get("name").and_then(|v| v.as_str()).map(str::to_string))
                .collect();
            let mut remote = Vec::new();
            for name in offline_router::VERIFIED_MODELS {
                if !local_names.iter().any(|n| n == name) {
                    remote.push(json!({
                        "name": name,
                        "path": Value::Null,
                        "size_bytes": Value::Null,
                        "source": "verified-not-downloaded",
                    }));
                }
            }
            let all: Vec<Value> = models.iter().chain(remote.iter()).cloned().collect();
            if c.json {
                println!("{}", serde_json::to_string_pretty(&all).unwrap_or_default());
            } else if all.is_empty() {
                println!("No local GGUF models found in {}", dir.display());
            } else {
                for m in &all {
                    let source = m.get("source").and_then(|v| v.as_str()).unwrap_or("");
                    let path = m.get("path").and_then(|v| v.as_str()).unwrap_or("not downloaded");
                    println!(
                        "{}  {}  [{}]",
                        m.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        path,
                        source
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
        OfflineSub::Install => match offline_engine::install() {
            Ok(report) => {
                println!(
                    "Installed llama.cpp {} ({}, {}) at {}",
                    offline_engine::PINNED_VERSION,
                    report.artifact,
                    report.accelerator,
                    report.engine_dir.display()
                );
                println!("sha256 {}", report.sha256);
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
        OfflineSub::Stop => match offline_router::stop() {
            Ok(true) => {
                println!("Stopped the offline llama.cpp router.");
                0
            }
            Ok(false) => {
                println!("No offline router is running.");
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
    }
}
