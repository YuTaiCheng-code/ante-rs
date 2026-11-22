use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub scope: String,
    #[serde(default)]
    pub argument_hint: Option<String>,
    #[serde(skip_serializing, default)]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing, default)]
    pub user_invocable: bool,
    #[serde(skip_serializing, default)]
    pub body_text: Option<String>,
}

const BUILTIN_SKILL_FILES: &[(&str, &str)] = &[
    ("add-provider", include_str!("../resources/builtin_skills/add-provider.md")),
    ("ante-guide", include_str!("../resources/builtin_skills/ante-guide.md")),
    ("resume-claude", include_str!("../resources/builtin_skills/resume-claude.md")),
    ("resume-codex", include_str!("../resources/builtin_skills/resume-codex.md")),
    ("skill-creator", include_str!("../resources/builtin_skills/skill-creator.md")),
    ("tmux", include_str!("../resources/builtin_skills/tmux.md")),
];

fn builtin_skills() -> Vec<Skill> {
    let mut out = Vec::new();
    for (fallback_name, text) in BUILTIN_SKILL_FILES {
        let (fm, _) = parse_frontmatter(text);
        out.push(Skill {
            name: fm.get("name").cloned().unwrap_or_else(|| (*fallback_name).to_string()),
            description: fm.get("description").cloned().unwrap_or_default(),
            scope: "builtin".into(),
            argument_hint: fm.get("argument-hint").cloned(),
            user_invocable: fm.get("user-invocable").map(|v| v != "false").unwrap_or(true),
            path: None,
            body_text: Some(text.to_string()),
        });
    }
    out
}

fn parse_frontmatter(text: &str) -> (std::collections::BTreeMap<String, String>, String) {
    let mut map = std::collections::BTreeMap::new();
    let mut body = text.to_string();
    if let Some(rest) = text.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let fm = &rest[..end];
            body = rest[end + 4..].trim_start_matches('\n').to_string();
            for line in fm.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    map.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
                }
            }
        }
    }
    (map, body)
}

pub fn discover(cwd: &Path) -> Vec<Skill> {
    let mut out: Vec<Skill> = Vec::new();
    let mut dirs: Vec<(PathBuf, &str)> = vec![
        (crate::util::ante_home().join("skills"), "user"),
        (cwd.join(".ante/skills"), "project"),
        (cwd.join(".agents/skills"), "project"),
        (cwd.join(".claude/skills"), "project"),
    ];
    if let Ok(h) = std::env::var("HOME") {
        dirs.push((PathBuf::from(h).join(".claude/skills"), "user"));
    }
    for (dir, scope) in dirs {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let path = e.path().join("SKILL.md");
            if !path.exists() {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                let (fm, _) = parse_frontmatter(&text);
                let name = fm
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| e.file_name().to_string_lossy().to_string());
                if out.iter().any(|s| s.name == name) {
                    continue;
                }
                out.push(Skill {
                    name,
                    description: fm.get("description").cloned().unwrap_or_default(),
                    scope: scope.into(),
                    argument_hint: fm.get("argument-hint").cloned(),
                    user_invocable: fm
                        .get("user-invocable")
                        .map(|v| v != "false")
                        .unwrap_or(true),
                    path: Some(path),
                    body_text: None,
                });
            }
        }
    }
    // Built-in skills are always available; an on-disk skill with the same
    // name shadows the built-in version.
    for builtin in builtin_skills() {
        if !out.iter().any(|s| s.name == builtin.name) {
            out.push(builtin);
        }
    }
    out
}

pub fn body(skill: &Skill) -> Option<String> {
    if let Some(text) = &skill.body_text {
        let (_, body) = parse_frontmatter(text);
        return Some(body);
    }
    let p = skill.path.as_ref()?;
    let text = std::fs::read_to_string(p).ok()?;
    let (_, body) = parse_frontmatter(&text);
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_skills_are_always_available_with_bodies() {
        let skills = discover(Path::new("/"));
        for name in ["add-provider", "ante-guide", "resume-claude", "resume-codex", "skill-creator", "tmux"] {
            let skill = skills.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(skill.scope, "builtin");
            let body = body(skill).unwrap();
            assert!(body.len() > 500, "{name} body too short");
        }
    }
}
