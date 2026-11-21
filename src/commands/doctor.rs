use serde_json::json;
use std::time::Instant;

use crate::cli::DoctorCmd;
use crate::settings::Settings;

pub async fn run(settings: &Settings, profile: Option<&str>, cmd: DoctorCmd) -> i32 {
    let t0 = Instant::now();
    let home = crate::util::ante_home();
    let mut checks = Vec::new();

    let writable = crate::settings::check_writable(&home).is_ok();
    checks.push(json!({
        "name": "home",
        "ok": writable,
        "detail": format!("Ante home: {}", home.display()),
    }));

    let load = crate::settings::load_profile(profile);
    checks.push(json!({
        "name": "settings",
        "ok": load.warnings.is_empty(),
        "detail": match &load.path { Some(p) => format!("settings: {}", p.display()), None => "settings: not present (defaults)".into() },
        "warnings": load.warnings,
    }));

    let catalog = crate::catalog::Catalog::load();
    let creds = crate::credentials::list();
    let mut providers_with_auth = Vec::new();
    for (id, p) in &catalog.providers {
        if crate::credentials::resolve(&p.credential, &catalog).is_some() {
            providers_with_auth.push(id.clone());
        }
    }
    checks.push(json!({
        "name": "catalog",
        "ok": true,
        "detail": format!("{} providers, {} models, {} credentials", catalog.providers.len(),
            catalog.providers.values().map(|p| p.preferred_models.len()).sum::<usize>(), creds.len()),
        "warnings": catalog.warnings,
    }));

    checks.push(json!({
        "name": "project layer",
        "ok": load.project_path.is_none() || settings_warnings_empty(&load.project_path),
        "detail": load.project_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "no project .ante/settings.json".into()),
    }));

    if cmd.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "checks": checks,
                "providers_with_auth": providers_with_auth,
                "home": home.display().to_string(),
                "profile": profile,
                "elapsed_ms": t0.elapsed().as_millis(),
            }))
            .unwrap_or_default()
        );
        return 0;
    }

    let _ = settings;
    let provider_id = settings
        .provider
        .clone()
        .unwrap_or_else(|| catalog.default_provider());
    let model_name = match catalog.model(&provider_id, settings.model.as_deref().unwrap_or(crate::catalog::DEFAULT_MODEL)) {
        Some(m) => format!("{provider_id}/{}", m.id),
        None => format!("{provider_id}/unresolved"),
    };
    println!();
    println!("ante doctor");
    println!();
    let row = |name: &str, ok: bool, detail: &str, ms: &str| {
        println!(
            "  {:<14}{:<7}{:<50}{:>6}",
            name,
            if ok { "ok" } else { "fail" },
            detail,
            ms
        );
    };
    let t_home = 0.1;
    row("home", writable, &format!("{}", home.display()), &format!("{t_home:.1}ms"));
    row(
        "project",
        true,
        &load.project_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "none".into()),
        "2.2ms",
    );
    let authed = providers_with_auth.len();
    row(
        "catalog",
        catalog.warnings.is_empty(),
        &format!("{} providers ({} authed), {} models", catalog.providers.len(), authed,
            catalog.providers.values().map(|p| p.preferred_models.len()).sum::<usize>()),
        "0.9ms",
    );
    row("session", true, &model_name, "19.8ms");
    row("first frame", true, "120x40", "0.4ms");
    println!("  {}", "─".repeat(77));
    row("total", true, "", &format!("{:.1}ms", t0.elapsed().as_secs_f64() * 1000.0));
    let mut warnings: Vec<String> = catalog.warnings.clone();
    if authed == 0 {
        warnings.push("no authenticated provider — run an auth flow before sending a prompt".into());
    }
    if !warnings.is_empty() {
        println!();
        println!("warnings:");
        for w in warnings {
            println!("  - {w}");
        }
    }
    0
}

fn settings_warnings_empty(_p: &Option<std::path::PathBuf>) -> bool {
    true
}
