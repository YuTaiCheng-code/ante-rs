pub mod auth;
pub mod doctor;
pub mod offline;
pub mod rage;
pub mod serve;
pub mod update;
pub mod usage;

use crate::settings::Settings;

pub async fn dispatch(cmd: crate::cli::Command, settings: Settings, profile: Option<&str>) -> i32 {
    match cmd {
        crate::cli::Command::Catalog => {
            let catalog = crate::catalog::Catalog::load();
            for w in &catalog.warnings {
                eprintln!("{w}");
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&catalog.to_json()).unwrap_or_default()
            );
            0
        }
        crate::cli::Command::Doctor(d) => doctor::run(&settings, profile, d).await,
        crate::cli::Command::Auth(a) => auth::run(a).await,
        crate::cli::Command::Usage(u) => usage::run(u).await,
        crate::cli::Command::Rage(r) => rage::run(r, &settings),
        crate::cli::Command::Offline(o) => {
            tokio::task::spawn_blocking(move || offline::run(o, &settings))
                .await
                .unwrap_or(1)
        }
        crate::cli::Command::Update(u) => update::run(u),
        crate::cli::Command::Serve(s) => serve::run(s, settings).await,
    }
}
