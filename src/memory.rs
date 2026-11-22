//! Persistent auto memory: a prompt section plus a small `MEMORY.md` index.

use std::path::PathBuf;

use crate::util::ante_home;

pub const AUTO_MEMORY_PROMPT: &str = include_str!("../resources/auto_memory_prompt.md");

pub fn dir() -> PathBuf {
    ante_home().join("memory")
}

pub fn index_path() -> PathBuf {
    dir().join("MEMORY.md")
}

/// Render the auto-memory system section. The index is capped at 25 KB, the
/// same budget the original prompt documents.
pub fn system_section() -> String {
    let dir = dir();
    let _ = std::fs::create_dir_all(&dir);
    let prompt = AUTO_MEMORY_PROMPT.replace("{{MEMORY_DIR}}", &dir.display().to_string());
    let index = std::fs::read_to_string(index_path()).unwrap_or_default();
    let index = crate::util::truncate(&index, 25 * 1024, "\n\ntruncated after 25 KB");
    format!(
        "{prompt}\n\n## MEMORY.md\n\n{}\n",
        if index.trim().is_empty() {
            "Your MEMORY.md is currently empty. Create it only when durable context would help future sessions."
        } else {
            index.trim()
        }
    )
}

/// Import Claude Code project/user memory into a single topic file.
/// Search order matches the usual Claude Code locations, with project files
/// taking precedence over the user-level file.
pub fn import_claude(cwd: &std::path::Path) -> String {
    let mut candidates = vec![
        cwd.join("CLAUDE.md"),
        cwd.join(".claude").join("CLAUDE.md"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(std::path::PathBuf::from(home).join(".claude").join("CLAUDE.md"));
    }

    let mut found = Vec::new();
    for path in candidates {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if !text.trim().is_empty() {
                found.push((path, text));
            }
        }
    }
    if found.is_empty() {
        return "No Claude Code memory found (checked ./CLAUDE.md, ./.claude/CLAUDE.md, ~/.claude/CLAUDE.md).".into();
    }

    let dir = dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return format!("Failed to create memory directory {}: {e}", dir.display());
    }
    let topic = dir.join("claude-code.md");
    let mut body = String::from("# Imported Claude Code memory\n\n");
    for (path, text) in &found {
        body.push_str(&format!("## {}\n\n{}\n\n", path.display(), text.trim()));
    }
    if let Err(e) = std::fs::write(&topic, body) {
        return format!("Failed to write {}: {e}", topic.display());
    }

    let index = index_path();
    let existing = std::fs::read_to_string(&index).unwrap_or_default();
    if !existing.contains("claude-code.md") {
        let mut updated = existing;
        if !updated.ends_with('\n') && !updated.is_empty() {
            updated.push('\n');
        }
        updated.push_str("- [Claude Code memory](claude-code.md)\n");
        let _ = std::fs::write(&index, updated);
    }
    format!(
        "Imported {} Claude Code memory file(s) into {} and updated {}",
        found.len(),
        topic.display(),
        index.display()
    )
}

pub fn command(arg: &str) -> String {
    let dir = dir();
    match arg.trim() {
        "" | "show" => {
            let index = std::fs::read_to_string(index_path()).unwrap_or_default();
            format!(
                "Memory directory: {}\n\n{}",
                dir.display(),
                if index.trim().is_empty() {
                    "MEMORY.md is empty.".to_string()
                } else {
                    index
                }
            )
        }
        "path" => dir.display().to_string(),
        "clear" => match std::fs::remove_file(index_path()) {
            Ok(()) => format!("Removed {}", index_path().display()),
            Err(_) => "MEMORY.md does not exist.".into(),
        },
        _ => "Usage: /memory [show|path|clear]".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_claude_memory() {
        let _guard = crate::util::TEST_ENV_LOCK.lock().unwrap();
        let home = std::env::temp_dir().join(format!("ante-memory-import-{}", crate::util::ulid()));
        let cwd = home.join("project");
        std::fs::create_dir_all(&cwd).unwrap();
        std::env::set_var("ANTE_HOME", &home);
        std::env::set_var("HOME", home.join("home"));
        std::fs::write(cwd.join("CLAUDE.md"), "use tabs").unwrap();
        let msg = import_claude(&cwd);
        assert!(msg.contains("Imported 1"), "{msg}");
        assert!(index_path().exists());
        assert!(std::fs::read_to_string(index_path()).unwrap().contains("claude-code.md"));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn prompt_has_directory_and_index() {
        let _guard = crate::util::TEST_ENV_LOCK.lock().unwrap();
        let home = std::env::temp_dir().join(format!("ante-memory-test-{}", crate::util::ulid()));
        std::env::set_var("ANTE_HOME", &home);
        let section = system_section();
        assert!(section.contains(&dir().display().to_string()));
        assert!(section.contains("## MEMORY.md"));
        let _ = std::fs::remove_dir_all(home);
    }
}
