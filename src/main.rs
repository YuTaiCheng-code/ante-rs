mod agent;
mod catalog;
mod cli;
mod client;
mod commands;
mod credentials;
mod goal;
mod headless;
mod llm;
mod protocol;
mod repl;
mod runtime;
mod session;
mod settings;
mod skills;
mod tools;
mod tui;
mod util;
mod wire;

pub const VERSION: &str = "0.3.4";

use std::io::IsTerminal;
use std::sync::Arc;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    let code = rt.block_on(run(cli));
    std::process::exit(code);
}

async fn run(cli: cli::Cli) -> i32 {
    let profile = cli.run.profile.clone();
    let load = settings::load_profile(profile.as_deref());
    for w in &load.warnings {
        eprintln!("warning: {w}");
    }
    let mut settings = load.settings;

    if let Some(cmd) = cli.command {
        return commands::dispatch(cmd, settings, profile.as_deref()).await;
    }

    let run_args = cli.run;
    apply_overrides(&mut settings, &run_args);

    if let Some(p) = &run_args.offline_model {
        eprintln!("offline mode is not available in this build; use a provider with an API key");
        let _ = p;
        return 1;
    }

    let system_prompt = if let Some(path) = &run_args.system_prompt_file {
        match std::fs::read_to_string(util::expand_tilde(path)) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("failed to read system prompt file `{path}`: {e}");
                return 1;
            }
        }
    } else {
        run_args.system_prompt.clone()
    };

    let session_id = match &run_args.resume {
        Some(id) if id.is_empty() => {
            let sessions = session::Session::list();
            match sessions.first() {
                Some(m) => Some(m.id.clone()),
                None => {
                    eprintln!("cannot resume: no saved sessions");
                    return 1;
                }
            }
        }
        Some(id) => Some(id.clone()),
        None => None,
    };

    // provider validity check
    let catalog = catalog::Catalog::load();
    for w in &catalog.warnings {
        if !w.contains("skipped") || std::env::var("ANTE_DEBUG").is_ok() {
            eprintln!("{w}");
        }
    }
    if let Some(p) = &run_args.provider {
        if catalog.provider(p).is_none() {
            eprintln!(
                "Provider '{p}' is unavailable; using '{}' instead. Configure a provider:\n\
                 - Sign in: `ante auth login antix` (or `anthropic`, `openai`)\n\
                 - Offline: `ante offline install`, then re-run with `--offline-model <MODEL>`\n\
                 - API key: set the provider's API key environment variable",
                catalog.default_provider()
            );
        }
    }

    let interactive = run_args.prompt.is_none() && !stdin_has_data();
    let offline_hint = run_args.prompt.is_some() || stdin_has_data();

    let mut rt = runtime::Runtime::build(runtime::RuntimeOptions {
        settings,
        provider: run_args.provider.clone(),
        model: run_args.model.clone(),
        effort: run_args.effort.clone(),
        permission_mode: if run_args.yolo {
            Some("yolo".into())
        } else {
            run_args.permission_mode.clone()
        },
        tools: run_args.tools.clone(),
        include_tools: run_args.include_tools.clone(),
        exclude_tools: run_args.exclude_tools.clone(),
        skills: if run_args.no_skills {
            Some(false)
        } else if run_args.skills {
            Some(true)
        } else {
            None
        },
        short_prompt: if run_args.short_prompt { Some(true) } else { None },
        system_prompt,
        append_system_prompt: run_args.append_system_prompt.clone(),
        session_id,
        interactive: offline_hint,
    });

    if run_args.enable_auto_memory {
        rt.settings.auto_memory = Some(true);
    }
    if run_args.disable_auto_memory {
        rt.settings.auto_memory = Some(false);
    }
    if run_args.yolo || (interactive
        && rt.permission_mode == tools::permission::Mode::Auto
        && false)
    {
        rt.permission_mode = tools::permission::Mode::Yolo;
    }
    if std::env::var("ANTE_FORCE_REPL").is_ok() {
        rt.interactive = true;
    }
    if !rt.interactive {
        // headless runs default to yolo unless a mode was chosen explicitly
        if run_args.permission_mode.is_none() && !run_args.yolo && rt.permission_mode == tools::permission::Mode::Auto {
            rt.permission_mode = tools::permission::Mode::Yolo;
        }
    }

    let session_save = if run_args.no_session_save {
        false
    } else {
        true
    };

    if let Some(prompt) = run_args.prompt.or_else(stdin_prompt) {
        let rt = Arc::new(rt);
        let code = headless::run(
            rt,
            headless::HeadlessOptions {
                prompt,
                format: headless::OutputFormat::parse(&run_args.output_format),
                verify: run_args.check,
                session_save,
            },
        )
        .await;
        return code;
    }

    let _ = offline_hint;
    rt.interactive = true;
    let rt = Arc::new(rt);
    if std::env::var("ANTE_FORCE_REPL").is_ok() {
        return repl::run(rt, session_save).await;
    }
    if std::io::stdout().is_terminal() {
        tui::run(rt, session_save).await
    } else {
        repl::run(rt, session_save).await
    }
}

fn apply_overrides(settings: &mut settings::Settings, run: &cli::RunArgs) {
    if let Some(m) = &run.model {
        settings.model = Some(m.clone());
    }
    if let Some(p) = &run.provider {
        settings.provider = Some(p.clone());
    }
    if let Some(m) = &run.permission_mode {
        settings.permission_mode = Some(m.clone());
    }
    if let Some(e) = &run.effort {
        settings.model_effort = Some(e.clone());
    }
    if run.short_prompt {
        settings.short_prompt = Some(true);
    }
}

fn stdin_has_data() -> bool {
    !std::io::stdin().is_terminal()
}

fn stdin_prompt() -> Option<String> {
    use std::io::Read;
    if std::env::var("ANTE_FORCE_REPL").is_ok() {
        return None;
    }
    if std::io::stdin().is_terminal() {
        return None;
    }
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
