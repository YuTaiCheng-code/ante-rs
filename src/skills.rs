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
                });
            }
        }
    }
    if out.is_empty() {
        if let Ok(list) = serde_json::from_str::<Vec<Skill>>(crate::runtime::BUILTIN_SKILLS) {
            out = list;
        }
    }
    out
}

pub fn body(skill: &Skill) -> Option<String> {
    let p = skill.path.as_ref()?;
    let text = std::fs::read_to_string(p).ok()?;
    let (_, body) = parse_frontmatter(&text);
    Some(body)
}
