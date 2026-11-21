use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;

use crate::catalog::{Catalog, ModelSpec, ProviderSpec};
use crate::client::LlmClient;
use crate::settings::{PermissionRules, Settings};
use crate::util;

pub const PROMPT_SECTIONS: &str = include_str!("../resources/prompt_sections.json");
pub const TOOLS_LONG: &str = include_str!("../resources/tools_long.json");
pub const TOOLS_SHORT: &str = include_str!("../resources/tools_short.json");
pub const BUILTIN_AGENTS: &str = include_str!("../resources/builtin_agents.json");
pub const BUILTIN_SKILLS: &str = include_str!("../resources/builtin_skills.json");

#[derive(Clone)]
pub struct Runtime {
    pub catalog: Catalog,
    pub provider_id: String,
    pub provider: ProviderSpec,
    pub model: ModelSpec,
    pub settings: Settings,
    pub effort: Option<String>,
    pub tool_names: Vec<String>,
    pub short_prompt: bool,
    pub system_override: Option<String>,
    pub append_system_prompt: Option<String>,
    pub permission_mode: crate::tools::permission::Mode,
    pub permission_rules: PermissionRules,
    pub cwd: std::path::PathBuf,
    pub session_id: String,
    pub interactive: bool,
    pub auto_compact: bool,
}

pub const DEFAULT_TOOLS: &[&str] = &[
    "Agent", "AskUser", "Bash", "Edit", "Glob", "Grep", "Read", "Terminal", "WebFetch", "Write",
];
pub const OFF_BY_DEFAULT: &[&str] = &["TodoWrite", "ViewImage"];

pub struct RuntimeOptions {
    pub settings: Settings,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
    pub tools: Option<Vec<String>>,
    pub include_tools: Vec<String>,
    pub exclude_tools: Vec<String>,
    pub skills: Option<bool>,
    pub short_prompt: Option<bool>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
    pub session_id: Option<String>,
    pub interactive: bool,
}

impl Runtime {
    pub fn build(opts: RuntimeOptions) -> Runtime {
        let catalog = Catalog::load();
        let provider_hint = opts
            .provider
            .clone()
            .or_else(|| opts.settings.provider.clone());
        let model_hint = opts.model.clone().or_else(|| opts.settings.model.clone());
        let (provider_id, model) = catalog.resolve_model(provider_hint.as_deref(), model_hint.as_deref());
        let provider = catalog.providers.get(&provider_id).cloned().unwrap_or_default();
        let effort = opts
            .effort
            .clone()
            .or_else(|| opts.settings.model_effort.clone())
            .or_else(|| model.effort.clone());

        let mut tool_names: Vec<String> = if let Some(list) = opts.tools.clone().or_else(|| opts.settings.tools.clone()) {
            list
        } else {
            DEFAULT_TOOLS.iter().map(|s| s.to_string()).collect()
        };
        for t in opts
            .include_tools
            .iter()
            .chain(opts.settings.include_tools.iter())
            .chain(std::iter::once(&String::new()))
        {
            if !t.is_empty() && !tool_names.iter().any(|x| x == t) {
                tool_names.push(t.clone());
            }
        }
        let excludes: Vec<String> = opts
            .exclude_tools
            .iter()
            .chain(opts.settings.exclude_tools.iter())
            .cloned()
            .collect();
        tool_names.retain(|t| !excludes.iter().any(|e| e == t));

        let mode = opts
            .permission_mode
            .clone()
            .or_else(|| opts.settings.permission_mode.clone())
            .and_then(|m| crate::tools::permission::Mode::parse(&m))
            .unwrap_or(crate::tools::permission::Mode::Auto);

        Runtime {
            catalog,
            provider_id,
            provider,
            model,
            effort,
            tool_names,
            short_prompt: opts.short_prompt.or(opts.settings.short_prompt).unwrap_or(false),
            system_override: opts.system_prompt.or_else(|| opts.settings.system_prompt.clone()),
            append_system_prompt: opts
                .append_system_prompt
                .or_else(|| opts.settings.append_system_prompt.clone()),
            permission_mode: mode,
            permission_rules: opts.settings.permissions.clone(),
            cwd: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            session_id: opts.session_id.unwrap_or_else(|| format!("ses_{}", util::ulid())),
            interactive: opts.interactive,
            auto_compact: opts.settings.auto_compact.unwrap_or(true),
            settings: opts.settings,
        }
    }

    pub fn client(&self) -> Result<LlmClient> {
        LlmClient::new(&self.provider_id, self.provider.clone(), self.model.clone())
    }

    pub fn tool_schemas(&self) -> Vec<Value> {
        let raw: Vec<Value> = serde_json::from_str(if self.short_prompt { TOOLS_SHORT } else { TOOLS_LONG })
            .unwrap_or_default();
        raw.into_iter()
            .filter_map(|t| {
                let f = t.get("function").cloned().unwrap_or(t.clone());
                let name = f.get("name")?.as_str()?.to_string();
                if self.tool_names.iter().any(|n| n == &name) {
                    Some(json!({
                        "name": name,
                        "description": f.get("description").cloned().unwrap_or_default(),
                        "parameters": f.get("parameters").cloned().unwrap_or(json!({"type":"object","properties":{}}))
                    }))
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn system_sections(&self) -> Vec<String> {
        if let Some(s) = &self.system_override {
            let mut v = vec![s.clone()];
            if let Some(a) = &self.append_system_prompt {
                v.push(a.clone());
            }
            return v;
        }
        let sections: serde_json::Map<String, Value> =
            serde_json::from_str(PROMPT_SECTIONS).unwrap_or_default();
        let mut out = Vec::new();
        for key in ["intro", "harness", "communicating_with_the_user", "working_defaults"] {
            if let Some(Value::String(s)) = sections.get(key) {
                out.push(s.clone());
            }
        }
        out.push(self.environment_section());
        if let Some(a) = &self.append_system_prompt {
            out.push(a.clone());
        }

        if self.settings.skills_enabled() {
            let skills = crate::skills::discover(&self.cwd);
            if !skills.is_empty() {
                let mut listing = String::from("<system-reminder>\n        ## Skills\n");
                listing.push_str("- A skill is a set of local instructions stored in a `SKILL.md` file. Read it before using a skill.\n");
                for s in skills.iter().take(12) {
                    listing.push_str(&format!("- {}: {}\n", s.name, crate::util::truncate(&s.description, 240, "…")));
                }
                listing.push_str("        </system-reminder>");
                out.push(listing);
            }
        }

        let agents: Vec<Value> = serde_json::from_str(BUILTIN_AGENTS).unwrap_or_default();
        let mut rem = String::from("<system-reminder>\n        Available agent types for the Agent tool:\n");
        for a in agents {
            let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let desc = a.get("description").and_then(|v| v.as_str()).unwrap_or("");
            let tools = if name == "explore" { "Glob, Grep, Read, Bash" } else { "all tools" };
            rem.push_str(&format!("- {name}: {desc}\n(Tools: {tools}) \n"));
        }
        rem.push_str("        </system-reminder>");
        out.push(rem);
        out
    }

    fn environment_section(&self) -> String {
        let cwd = self.cwd.display();
        let git = if util::is_git_repo(&self.cwd) { "true" } else { "false" };
        format!(
            "\n# Environment\nYou have been invoked in the following environment:\n - Primary working directory: {cwd}\n - Is a git repository: {git}\n - OS: {}\n - Shell: {}\n - Session start date: {} (today's date, unless a later message gives a newer one)\n\n{}",
            util::os_name(),
            util::shell_name(),
            util::today_utc(),
            if self.interactive {
                "".to_string()
            } else {
                "This session is unattended: no one can answer a question or send another message, and the session ends when your turn ends. There is no later turn.".to_string()
            }
        )
    }

    pub fn folder_structure(&self) -> String {
        format!(
            "<folder-structure>\n- {}/\n{}\n</folder-structure>",
            self.cwd.display(),
            util::folder_structure(Path::new(&self.cwd), 2)
        )
    }

    pub fn describe(&self) -> Value {
        json!({
            "id": self.model.id,
            "description": self.model.description,
            "max_tokens": self.model.max_tokens,
            "context_limit": self.model.context_limit,
            "effort": self.model.effort,
            "supported_efforts": self.model.supported_efforts,
            "support_vision": self.model.support_vision,
            "weight_class": self.model.weight_class,
        })
    }

    pub fn provider_json(&self) -> Value {
        json!({
            "id": self.provider_id,
            "display_name": self.provider.display_name,
            "base_url": self.provider.base_url,
        })
    }
}
