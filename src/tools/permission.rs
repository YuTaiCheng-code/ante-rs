use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};

fn session_grants() -> &'static Mutex<Vec<String>> {
    static GRANTS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    GRANTS.get_or_init(|| Mutex::new(Vec::new()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Strict,
    Auto,
    Yolo,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "strict" => Some(Mode::Strict),
            "auto" => Some(Mode::Auto),
            "yolo" => Some(Mode::Yolo),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Strict => "strict",
            Mode::Auto => "auto",
            Mode::Yolo => "yolo",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Ask,
    Deny,
}

pub struct PermissionManager {
    pub mode: Mode,
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
    /// "always allow"/session grants added at runtime.
    pub grants: Vec<String>,
}

pub fn primary_arg(tool: &str, input: &serde_json::Value) -> String {
    let key = match tool {
        "Bash" => "command",
        "Read" | "Write" | "Edit" | "ViewImage" => "file_path",
        "Glob" => "pattern",
        "Grep" => "pattern",
        "WebFetch" => "url",
        "Agent" => "subagent_type",
        _ => "",
    };
    input
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

impl PermissionManager {
    pub fn new(mode: Mode, rules: &crate::settings::PermissionRules) -> Self {
        PermissionManager {
            mode,
            allow: rules.allow.clone(),
            ask: rules.ask.clone(),
            deny: rules.deny.clone(),
            grants: Vec::new(),
        }
    }

    pub fn check(&self, tool: &str, input: &serde_json::Value) -> Decision {
        let arg = primary_arg(tool, input);
        if self.mode == Mode::Yolo {
            return Decision::Allow;
        }
        if self.deny.iter().any(|m| crate::settings::matcher_matches(m, tool, &arg)) {
            return Decision::Deny;
        }
        if self.grants.iter().any(|m| crate::settings::matcher_matches(m, tool, &arg)) {
            return Decision::Allow;
        }
        if session_grants()
            .lock()
            .map(|g| g.iter().any(|m| crate::settings::matcher_matches(m, tool, &arg)))
            .unwrap_or(false)
        {
            return Decision::Allow;
        }
        if self.ask.iter().any(|m| crate::settings::matcher_matches(m, tool, &arg)) {
            return Decision::Ask;
        }
        if self.allow.iter().any(|m| crate::settings::matcher_matches(m, tool, &arg)) {
            return Decision::Allow;
        }
        match self.mode {
            Mode::Strict => {
                if is_known_safe_readonly(tool, &arg) {
                    Decision::Allow
                } else {
                    Decision::Ask
                }
            }
            Mode::Auto => {
                if tool == "Bash" && is_dangerous_command(&arg) {
                    Decision::Ask
                } else {
                    Decision::Allow
                }
            }
            Mode::Yolo => Decision::Allow,
        }
    }

    pub fn grant(&mut self, matcher: impl Into<String>) {
        self.grants.push(matcher.into());
    }

    /// 会话级授权：把批准的调用泛化成同类前缀，仅本次进程有效。
    pub fn grant_session(&mut self, tool: &str, arg: &str) -> String {
        let matcher = generalize_matcher(tool, arg);
        if let Ok(mut g) = session_grants().lock() {
            if !g.iter().any(|m| m == &matcher) {
                g.push(matcher.clone());
            }
        }
        if !self.grants.iter().any(|m| m == &matcher) {
            self.grants.push(matcher.clone());
        }
        matcher
    }

    /// 永久授权：写入 settings.json 的 permissions.allow，并对后续会话生效。
    pub fn grant_always(&mut self, tool: &str, arg: &str, settings: &mut crate::settings::Settings) -> String {
        let matcher = generalize_matcher(tool, arg);
        if !self.grants.iter().any(|m| m == &matcher) {
            self.grants.push(matcher.clone());
        }
        if !settings.permissions.allow.iter().any(|m| m == &matcher) {
            settings.permissions.allow.push(matcher.clone());
        }
        let _ = crate::settings::save_settings(settings, None);
        matcher
    }
}

fn is_known_safe_readonly(tool: &str, arg: &str) -> bool {
    if matches!(tool, "Read" | "Glob" | "Grep" | "TodoWrite" | "AskUser" | "ViewImage") {
        return true;
    }
    if tool == "WebFetch" || tool == "WebSearch" {
        return false;
    }
    false
}

fn is_dangerous_command(cmd: &str) -> bool {
    let lower = cmd.to_ascii_lowercase();
    let patterns = [
        "rm -rf", "sudo", "mkfs", "dd if=", "shutdown", "reboot", "kill -9", ":(){", "> /dev/sd",
        "chmod 777 /", "curl | sh", "curl|sh", "wget | sh", "git push --force", "git reset --hard",
    ];
    patterns.iter().any(|p| lower.contains(p))
}

/// 智能作用域泛化：`cargo test -p foo` → `Bash(cargo test *)`。
///
/// 只保留命令的前导词（跳过选项及其值），因此 `-p foo`、`--features x` 这类
/// 每次都可能不同的参数不会让规则失效。非 Bash 工具按主参数精确匹配。
pub fn generalize_matcher(tool: &str, arg: &str) -> String {
    if tool != "Bash" {
        return format!("{tool}({arg})");
    }
    let tokens: Vec<&str> = arg.split_whitespace().collect();
    let mut head: Vec<&str> = Vec::new();
    for tok in tokens {
        if tok.starts_with('-') || tok.contains('=') && tok.starts_with(|c: char| c.is_ascii_alphabetic()) && head.len() >= 2 {
            break;
        }
        if tok == "|" || tok == "&&" || tok == ";" || tok == "||" {
            break;
        }
        head.push(tok);
        if head.len() == 2 {
            break;
        }
    }
    if head.is_empty() {
        return format!("Bash({arg})");
    }
    format!("Bash({} *)", head.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generalizes_command_prefix() {
        assert_eq!(generalize_matcher("Bash", "cargo test -p foo"), "Bash(cargo test *)");
        assert_eq!(generalize_matcher("Bash", "cargo test --all-features"), "Bash(cargo test *)");
        assert_eq!(generalize_matcher("Bash", "git status"), "Bash(git status *)");
        assert_eq!(generalize_matcher("Read", "/tmp/a.rs"), "Read(/tmp/a.rs)");
    }
}
