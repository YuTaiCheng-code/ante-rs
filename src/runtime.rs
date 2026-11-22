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
pub const SKILLS_PROMPT: &str = include_str!("../resources/skills_prompt.md");

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
    pub mcp: std::sync::Arc<crate::mcp::McpManager>,
    pub jobs: std::sync::Arc<crate::jobs::JobRegistry>,
}

pub const DEFAULT_TOOLS: &[&str] = &[
    "Agent", "AskUser", "Bash", "Edit", "Glob", "Grep", "Read", "WebFetch", "WebSearch", "Write",
];
pub const OFF_BY_DEFAULT: &[&str] = &["TodoWrite", "ViewImage"];

/// Accept the lowercase spellings used by the original CLI and settings.
pub fn canonical_tool_name(name: &str) -> String {
    match name.trim().to_ascii_lowercase().as_str() {
        "todowrite" | "todo-write" | "todo" => "TodoWrite".into(),
        "viewimage" | "view-image" | "view_image" => "ViewImage".into(),
        "websearch" | "web-search" | "web_search" => "WebSearch".into(),
        "webfetch" | "web-fetch" | "web_fetch" => "WebFetch".into(),
        "askuser" | "ask-user" | "ask_user" => "AskUser".into(),
        "agent" => "Agent".into(),
        "bash" => "Bash".into(),
        "edit" => "Edit".into(),
        "glob" => "Glob".into(),
        "grep" => "Grep".into(),
        "read" => "Read".into(),
        "write" => "Write".into(),
        "terminal" | "term" => "Terminal".into(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        }
    }
}

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
            list.into_iter().map(|t| canonical_tool_name(&t)).collect()
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
                let canonical = canonical_tool_name(t);
                if !tool_names.iter().any(|x| x == &canonical) {
                    tool_names.push(canonical);
                }
            }
        }
        let excludes: Vec<String> = opts
            .exclude_tools
            .iter()
            .chain(opts.settings.exclude_tools.iter())
            .cloned()
            .map(|t| canonical_tool_name(&t))
            .collect();
        tool_names.retain(|t| !excludes.iter().any(|e| e == t));

        let mcp = std::sync::Arc::new(crate::mcp::McpManager::new(&opts.settings.mcp_servers));
        let jobs = std::sync::Arc::new(crate::jobs::JobRegistry::new());
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
            mcp,
            jobs,
            settings: opts.settings,
        }
    }

    pub fn client(&self) -> Result<LlmClient> {
        LlmClient::new(&self.provider_id, self.provider.clone(), self.model.clone())
    }

    pub fn tool_schemas(&self) -> Vec<Value> {
        let raw: Vec<Value> = serde_json::from_str(if self.short_prompt { TOOLS_SHORT } else { TOOLS_LONG })
            .unwrap_or_default();
        let mut schemas: Vec<Value> = raw.into_iter()
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
            .collect();
        schemas.extend(self.mcp.tool_schemas());
        schemas
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
                let mut listing = String::from("<system-reminder>\n");
                for s in skills.iter().take(24) {
                    let path = s
                        .path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| format!("builtin:{}", s.name));
                    listing.push_str(&format!(
                        "- {} [{}]: {} (path: {})\n",
                        s.name,
                        s.scope,
                        crate::util::truncate(&s.description, 260, "…"),
                        path
                    ));
                }
                listing.push_str("        </system-reminder>");
                out.push(SKILLS_PROMPT.replace("{{SKILL_LIST}}", listing.trim_end()));
            }
        }

        if self.settings.auto_memory() {
            out.push(crate::memory::system_section());
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

    pub(crate) fn environment_section(&self) -> String {
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

#[cfg(test)]
mod tool_schema_tests {
    use super::*;

    #[test]
    fn lowercase_tool_aliases_are_canonicalized() {
        assert_eq!(canonical_tool_name("todowrite"), "TodoWrite");
        assert_eq!(canonical_tool_name("view_image"), "ViewImage");
        assert_eq!(canonical_tool_name("web-search"), "WebSearch");
    }

    #[test]
    fn all_default_tools_have_schemas_in_both_prompts() {
        for raw in [TOOLS_LONG, TOOLS_SHORT] {
            let schemas: Vec<Value> = serde_json::from_str(raw).unwrap();
            let names: Vec<String> = schemas
                .iter()
                .filter_map(|v| v.get("function")?.get("name")?.as_str().map(str::to_string))
                .collect();
            for tool in DEFAULT_TOOLS.iter().chain(OFF_BY_DEFAULT.iter()) {
                assert!(names.iter().any(|n| n == tool), "missing schema for {tool}");
            }
        }
    }
}
