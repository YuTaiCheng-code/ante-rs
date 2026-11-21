use std::path::PathBuf;

use crate::cli::RageCmd;
use crate::settings::Settings;

pub fn run(cmd: RageCmd, settings: &Settings) -> i32 {
    let home = crate::util::ante_home();
    let stamp = crate::util::now_rfc3339()
        .replace(':', "")
        .replace('-', "")
        .replace('T', "-")
        .chars()
        .take(15)
        .collect::<String>();
    let target: PathBuf = match &cmd.output {
        Some(p) => crate::util::expand_tilde(p),
        None => PathBuf::from(format!("./ante-rage-{stamp}.tar.gz")),
    };
    let staging = std::env::temp_dir().join(format!("ante-rage-{stamp}"));
    if let Err(e) = std::fs::create_dir_all(&staging) {
        eprintln!("failed to create report directory: {e}");
        return 1;
    }

    let mut env_lines = String::new();
    for (k, v) in std::env::vars() {
        let secretish = ["KEY", "TOKEN", "SECRET", "PASSWORD"].iter().any(|s| k.contains(s));
        env_lines.push_str(&format!(
            "{k}={}\n",
            if secretish && !v.is_empty() { "<SET>" } else { v.as_str() }
        ));
    }
    let _ = std::fs::write(staging.join("env.txt"), env_lines);

    let redacted = crate::settings::settings_summary_for_rage(settings);
    let _ = std::fs::write(
        staging.join("settings.redacted.json"),
        serde_json::to_string_pretty(&redacted).unwrap_or_default(),
    );
    let mut catalog = crate::catalog::Catalog::load();
    for p in catalog.providers.values_mut() {
        p.credential = serde_json::json!("<REDACTED>");
        p.headers.clear();
    }
    let _ = std::fs::write(
        staging.join("catalog.redacted.json"),
        serde_json::to_string_pretty(&catalog.to_json()).unwrap_or_default(),
    );
    let _ = std::fs::write(
        staging.join("report.md"),
        format!(
            "# Ante bug report\n\n- version: {}\n- date: {}\n- os: {} {}\n- profile: {}\n\nSecrets are excluded. Conversation transcripts and system prompts are not included.\n",
            crate::VERSION,
            crate::util::today_utc(),
            crate::util::os_name(),
            std::env::consts::ARCH,
            std::env::var("ANTE_PROFILE").unwrap_or_else(|_| "-".into())
        ),
    );

    let logs = home.join("logs");
    let dst_logs = staging.join("logs");
    if logs.exists() {
        let _ = std::fs::create_dir_all(&dst_logs);
        let mut n = 0;
        if let Ok(rd) = std::fs::read_dir(&logs) {
            for day in rd.flatten() {
                if !day.path().is_dir() {
                    continue;
                }
                if let Ok(files) = std::fs::read_dir(day.path()) {
                    for f in files.flatten() {
                        if n > 30 {
                            break;
                        }
                        let name = f.file_name();
                        let _ = std::fs::copy(f.path(), dst_logs.join(name));
                        n += 1;
                    }
                }
            }
        }
    }

    if cmd.no_compress {
        let out = if target.extension().and_then(|e| e.to_str()) == Some("tar") || target.is_dir() {
            target
        } else {
            target.with_extension("")
        };
        let _ = std::fs::remove_dir_all(&out);
        match std::fs::rename(&staging, &out) {
            Ok(_) => {
                println!("Report written to {}", out.display());
                0
            }
            Err(e) => {
                eprintln!("failed to write report: {e}");
                1
            }
        }
    } else {
        let status = std::process::Command::new("tar")
            .arg("czf")
            .arg(&target)
            .arg("-C")
            .arg(&staging)
            .arg(".")
            .status();
        let _ = std::fs::remove_dir_all(&staging);
        match status {
            Ok(s) if s.success() => {
                println!("Report written to {}", target.display());
                0
            }
            _ => {
                eprintln!("failed to create archive (is `tar` on PATH?)");
                1
            }
        }
    }
}
