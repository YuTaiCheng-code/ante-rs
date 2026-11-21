use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::util::{ante_home, expand_tilde};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PermissionRules {
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub permission_mode: Option<String>,
    pub permissions: PermissionRules,
    pub mcp_servers: BTreeMap<String, serde_json::Value>,
    pub model_effort: Option<String>,
    pub tools: Option<Vec<String>>,
    pub include_tools: Vec<String>,
    pub exclude_tools: Vec<String>,
    pub skills: Option<bool>,
    pub include_skills: Vec<String>,
    pub exclude_skills: Vec<String>,
    pub auto_memory: Option<bool>,
    pub session_save: Option<bool>,
    pub short_prompt: Option<bool>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
    pub theme: Option<String>,
    pub render_mode: Option<String>,
    pub status_line: Option<serde_json::Value>,
    pub status_line_command: Option<String>,
    pub channel: Option<String>,
    pub tips: Option<bool>,
    pub auto_compact: Option<bool>,
    pub group_tool_activity: Option<bool>,
    pub attention: Option<serde_json::Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            model: None,
            provider: None,
            permission_mode: None,
            permissions: PermissionRules::default(),
            mcp_servers: BTreeMap::new(),
            model_effort: None,
            tools: None,
            include_tools: Vec::new(),
            exclude_tools: Vec::new(),
            skills: None,
            include_skills: Vec::new(),
            exclude_skills: Vec::new(),
            auto_memory: None,
            session_save: None,
            short_prompt: None,
            system_prompt: None,
            append_system_prompt: None,
            theme: None,
            render_mode: None,
            status_line: None,
            status_line_command: None,
            channel: None,
            tips: None,
            auto_compact: None,
            group_tool_activity: None,
            attention: None,
        }
    }
}

impl Settings {
    pub fn session_save(&self) -> bool {
        self.session_save.unwrap_or(true)
    }
    pub fn short_prompt(&self) -> bool {
        self.short_prompt.unwrap_or(false)
    }
    pub fn skills_enabled(&self) -> bool {
        self.skills.unwrap_or(true)
    }
    pub fn auto_memory(&self) -> bool {
        self.auto_memory.unwrap_or(false)
    }
}

/// Where settings were loaded from, for `doctor` and error reporting.
#[derive(Debug, Clone)]
pub struct SettingsLoad {
    pub settings: Settings,
    pub path: Option<PathBuf>,
    pub project_path: Option<PathBuf>,
    pub warnings: Vec<String>,
}

pub fn profile_path(profile: Option<&str>) -> PathBuf {
    match profile {
        Some(p) if !p.is_empty() => ante_home().join(format!("{p}.settings.json")),
        _ => ante_home().join("settings.json"),
    }
}

pub fn load_profile(profile: Option<&str>) -> SettingsLoad {
    let mut warnings = Vec::new();
    let path = profile_path(profile);
    let mut settings = Settings::default();
    if path.exists() {
        match std::fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str::<Settings>(&s) {
                Ok(v) => settings = v,
                Err(e) => warnings.push(format!("{}: unreadable settings ({e})", path.display())),
            },
            Err(e) => warnings.push(format!("{}: {e}", path.display())),
        }
    } else if let Some(p) = profile {
        if !p.is_empty() && p != "bare" {
            warnings.push(format!(
                "profile '{p}' not found ({}); using defaults",
                path.display()
            ));
        }
    }
    let project_path = find_project_settings();
    if let Some(pp) = &project_path {
        match std::fs::read_to_string(pp) {
            Ok(s) => match serde_json::from_str::<Settings>(&s) {
                Ok(v) => settings = merge_project(settings, v),
                Err(e) => warnings.push(format!("{}: unreadable project settings ({e})", pp.display())),
            },
            Err(e) => warnings.push(format!("{}: {e}", pp.display())),
        }
    }
    SettingsLoad {
        settings,
        path: if path.exists() { Some(path) } else { None },
        project_path,
        warnings,
    }
}

fn merge_project(mut base: Settings, p: Settings) -> Settings {
    // Project settings carry session keys only; they can narrow but not widen permissions.
    if p.model.is_some() {
        // ignored by design
    }
    if p.permission_mode.is_some() {
        base.permission_mode = p.permission_mode;
    }
    base.permissions.allow.extend(p.permissions.allow);
    base.permissions.ask.extend(p.permissions.ask);
    base.permissions.deny.extend(p.permissions.deny);
    base.mcp_servers.extend(p.mcp_servers);
    if p.model_effort.is_some() {
        base.model_effort = p.model_effort;
    }
    if p.tools.is_some() {
        base.tools = p.tools;
    }
    base.include_tools.extend(p.include_tools);
    base.exclude_tools.extend(p.exclude_tools);
    if p.skills.is_some() {
        base.skills = p.skills;
    }
    base.include_skills.extend(p.include_skills);
    base.exclude_skills.extend(p.exclude_skills);
    if p.auto_memory.is_some() {
        base.auto_memory = p.auto_memory;
    }
    if p.session_save.is_some() {
        base.session_save = p.session_save;
    }
    if p.short_prompt.is_some() {
        base.short_prompt = p.short_prompt;
    }
    if p.system_prompt.is_some() {
        base.system_prompt = p.system_prompt;
    }
    if p.append_system_prompt.is_some() {
        base.append_system_prompt = p.append_system_prompt;
    }
    base
}

fn find_project_settings() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let mut cur = Some(cwd);
    while let Some(dir) = cur {
        let p = dir.join(".ante/settings.json");
        if p.exists() {
            return Some(p);
        }
        cur = dir.parent().map(|x| x.to_path_buf());
    }
    None
}

pub fn append_allow_rule(matcher: &str) -> Result<()> {
    let path = user_settings_path();
    let mut s = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Settings>(&raw).ok())
        .unwrap_or_default();
    if !s.permissions.allow.iter().any(|m| m == matcher) {
        s.permissions.allow.push(matcher.to_string());
    }
    save_settings(&s, None)
}

pub fn save_settings(s: &Settings, profile: Option<&str>) -> Result<()> {
    let path = profile_path(profile);
    let json = serde_json::to_string_pretty(s)?;
    crate::util::write_file(&path, &json).with_context(|| format!("write {}", path.display()))
}

/// Permission matcher: `Bash` or `Bash(cargo test *)` / `Read(~/.ssh/*)`.
/// The argument pattern is matched against the tool's primary argument.
pub fn matcher_matches(matcher: &str, tool: &str, primary_arg: &str) -> bool {
    let matcher = matcher.trim();
    if let Some(open) = matcher.find('(') {
        if !matcher.ends_with(')') {
            return false;
        }
        let name = &matcher[..open];
        let pat = &matcher[open + 1..matcher.len() - 1];
        if !glob_eq(name, tool) {
            return false;
        }
        glob_eq(pat, primary_arg)
    } else {
        glob_eq(matcher, tool)
    }
}

fn glob_eq(pat: &str, text: &str) -> bool {
    if pat == "*" {
        return true;
    }
    // `Bash(cargo test *)` 这类“前缀 + 可选参数”的模式，应同时匹配不带参数的形式
    if let Some(prefix) = pat.strip_suffix(" *") {
        let prefix = prefix.replace("~", &crate::util::home_dir().to_string_lossy());
        if text == prefix || text.starts_with(&(prefix.clone() + " ")) {
            return true;
        }
    }
    match globset::GlobBuilder::new(&pat.replace("~", &crate::util::home_dir().to_string_lossy()))
        .literal_separator(false)
        .build()
    {
        Ok(g) => g.compile_matcher().is_match(text),
        Err(_) => pat == text,
    }
}

pub fn settings_summary_for_rage(s: &Settings) -> serde_json::Value {
    let mut v = serde_json::to_value(s).unwrap_or(serde_json::json!({}));
    if let Some(o) = v.as_object_mut() {
        for k in ["mcp_servers", "attention", "status_line"] {
            if o.contains_key(k) {
                o.insert(k.to_string(), serde_json::json!("<REDACTED>"));
            }
        }
    }
    v
}

pub fn user_settings_path() -> PathBuf {
    ante_home().join("settings.json")
}

pub fn project_settings_path_for_display() -> Option<String> {
    find_project_settings().map(|p| p.display().to_string())
}

pub fn expand(path: &str) -> PathBuf {
    expand_tilde(path)
}

/// Ensure a directory exists and is writable.
pub fn check_writable(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let probe = dir.join(".ante-write-probe");
    std::fs::write(&probe, b"ok")?;
    std::fs::remove_file(&probe)?;
    Ok(())
}
