use base64::Engine;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::{resolve_path, ToolCtx, ToolResult};

const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

pub async fn read(ctx: &ToolCtx, input: &Value) -> ToolResult {
    let path = match input.get("file_path").and_then(|p| p.as_str()) {
        Some(p) => resolve_path(&ctx.cwd, p),
        None => return ToolResult::err("`file_path` is required"),
    };
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => return ToolResult::err(format!("{}: {e}", path.display())),
    };
    if meta.is_dir() {
        return ToolResult::err(format!(
            "{} is a directory. Use an ls command via Bash to list it.",
            path.display()
        ));
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if IMAGE_EXT.contains(&ext.as_str()) {
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => return ToolResult::err(format!("{}: {e}", path.display())),
        };
        let media = crate::llm::guess_media_type(&path).to_string();
        let b64 = base64::engine::general_purpose::STANDARD.encode(data);
        return ToolResult::ok(format!("Image loaded: {}", path.display()))
            .with_images(vec![(media, b64)]);
    }
    if ext == "pdf" {
        return read_pdf(&path, input);
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return ToolResult::err(format!("{}: {e}", path.display())),
    };
    if text.is_empty() {
        return ToolResult::ok(
            "<system-reminder> This file exists but has empty contents.</system-reminder>",
        );
    }
    let offset = input.get("offset").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as usize;
    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(2000)
        .max(1) as usize;
    let lines: Vec<&str> = text.lines().collect();
    if offset > lines.len() {
        return ToolResult::ok(format!(
            "File has {} lines; offset {offset} is out of range.",
            lines.len()
        ));
    }
    let end = (offset - 1 + limit).min(lines.len());
    let mut out = String::new();
    for (i, line) in lines[offset - 1..end].iter().enumerate() {
        let n = offset + i;
        let line = crate::util::truncate(line, 2000, "…");
        out.push_str(&format!("{n:>6}\t{line}\n"));
    }
    if end < lines.len() {
        out.push_str(&format!(
            "\n... {} more lines (continue with offset={})",
            lines.len() - end,
            end + 1
        ));
    }
    ToolResult::ok(super::limit_output(&out))
}

fn read_pdf(path: &Path, input: &Value) -> ToolResult {
    let pages = input.get("pages").and_then(|p| p.as_str()).unwrap_or("1-5");
    let mut args: Vec<String> = vec!["-q".into()];
    if let Some((a, b)) = pages.split_once('-') {
        args.push("-f".into());
        args.push(a.to_string());
        args.push("-l".into());
        args.push(b.to_string());
    }
    args.push(path.display().to_string());
    match std::process::Command::new("pdftotext").args(&args).output() {
        Ok(o) if o.status.success() => ToolResult::ok(super::limit_output(
            &String::from_utf8_lossy(&o.stdout),
        )),
        Ok(o) => ToolResult::err(format!(
            "pdf conversion failed: {}",
            String::from_utf8_lossy(&o.stderr)
        )),
        Err(_) => ToolResult::err(
            "PDF reading needs `pdftotext` (poppler-utils) on PATH; none was found.",
        ),
    }
}

pub async fn write(ctx: &ToolCtx, input: &Value) -> ToolResult {
    let path = match input.get("file_path").and_then(|p| p.as_str()) {
        Some(p) => resolve_path(&ctx.cwd, p),
        None => return ToolResult::err("`file_path` is required"),
    };
    let content = input.get("content").and_then(|c| c.as_str()).unwrap_or("");
    if path.is_dir() {
        return ToolResult::err(format!("{} is a directory", path.display()));
    }
    let existed = path.exists();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return ToolResult::err(format!("{}: {e}", parent.display()));
        }
    }
    match std::fs::write(&path, content) {
        Ok(_) => ToolResult::ok(format!(
            "{}: {}\n{} lines written",
            if existed { "File overwritten" } else { "File created successfully at" },
            path.display(),
            content.lines().count()
        )),
        Err(e) => ToolResult::err(format!("{}: {e}", path.display())),
    }
}

pub async fn edit(ctx: &ToolCtx, input: &Value) -> ToolResult {
    let path = match input.get("file_path").and_then(|p| p.as_str()) {
        Some(p) => resolve_path(&ctx.cwd, p),
        None => return ToolResult::err("`file_path` is required"),
    };
    let old = input.get("old_string").and_then(|s| s.as_str()).unwrap_or("");
    let new = input.get("new_string").and_then(|s| s.as_str()).unwrap_or("");
    if old.is_empty() {
        return ToolResult::err("`old_string` must not be empty");
    }
    if old == new {
        return ToolResult::err("`new_string` must be different from `old_string`");
    }
    let replace_all = input.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return ToolResult::err(format!("{}: {e}", path.display())),
    };
    let count = text.matches(old).count();
    if count == 0 {
        return ToolResult::err(format!(
            "old_string was not found in {}",
            path.display()
        ));
    }
    if count > 1 && !replace_all {
        return ToolResult::err(format!(
            "old_string is not unique in {} ({count} matches); pass replace_all or add more context",
            path.display()
        ));
    }
    let updated = if replace_all {
        text.replace(old, new)
    } else {
        text.replacen(old, new, 1)
    };
    if let Err(e) = std::fs::write(&path, &updated) {
        return ToolResult::err(format!("{}: {e}", path.display()));
    }
    ToolResult::ok(format!(
        "Edited {} ({} replacement{})",
        path.display(),
        if replace_all { count } else { 1 },
        if replace_all && count != 1 { "s" } else { "" }
    ))
}

#[derive(Clone)]
struct Entry {
    path: PathBuf,
    mtime: SystemTime,
}

fn walk_files(root: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !(e.file_type().is_dir()
                && (name == ".git" || name == "target" || name == "node_modules" || name == "__pycache__"))
        })
        .flatten()
    {
        if !e.file_type().is_file() {
            continue;
        }
        let mtime = e
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        out.push(Entry {
            path: e.path().to_path_buf(),
            mtime,
        });
    }
    out
}

fn display_path(cwd: &Path, p: &Path) -> String {
    if let Ok(rel) = p.strip_prefix(cwd) {
        rel.display().to_string()
    } else {
        p.display().to_string()
    }
}

pub async fn glob(ctx: &ToolCtx, input: &Value) -> ToolResult {
    let pattern = match input.get("pattern").and_then(|p| p.as_str()) {
        Some(p) => p,
        None => return ToolResult::err("`pattern` is required"),
    };
    let search_dir = input
        .get("path")
        .and_then(|p| p.as_str())
        .map(|p| resolve_path(&ctx.cwd, p))
        .unwrap_or_else(|| ctx.cwd.clone());
    let matcher = match globset::GlobBuilder::new(pattern)
        .literal_separator(false)
        .build()
    {
        Ok(g) => g.compile_matcher(),
        Err(e) => return ToolResult::err(format!("invalid glob pattern: {e}")),
    };
    let mut files: Vec<Entry> = walk_files(&search_dir)
        .into_iter()
        .filter(|e| matcher.is_match(&e.path))
        .collect();
    files.sort_by(|a, b| b.mtime.cmp(&a.mtime));
    let limit = input.get("head_limit").and_then(|v| v.as_u64()).unwrap_or(250) as usize;
    let skip = input.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    if files.is_empty() {
        return ToolResult::ok("No files found");
    }
    let total = files.len();
    let mut out = String::new();
    for e in files.iter().skip(skip).take(limit) {
        out.push_str(&format!("{}\n", display_path(&ctx.cwd, &e.path)));
    }
    out.push_str(&format!("\n({total} files)"));
    if skip + limit < total {
        out.push_str(&format!(", continue with offset={}", skip + limit));
    }
    ToolResult::ok(out)
}

pub async fn grep(ctx: &ToolCtx, input: &Value) -> ToolResult {
    let pattern = match input.get("pattern").and_then(|p| p.as_str()) {
        Some(p) => p,
        None => return ToolResult::err("`pattern` is required"),
    };
    let search_path = input
        .get("path")
        .and_then(|p| p.as_str())
        .map(|p| resolve_path(&ctx.cwd, p))
        .unwrap_or_else(|| ctx.cwd.clone());
    let mut rb = regex::RegexBuilder::new(pattern);
    rb.case_insensitive(input.get("-i").and_then(|v| v.as_bool()).unwrap_or(false));
    rb.multi_line(input.get("multiline").and_then(|v| v.as_bool()).unwrap_or(false));
    let re = match rb.build() {
        Ok(r) => r,
        Err(e) => return ToolResult::err(format!("invalid regex: {e}")),
    };
    let mode = input
        .get("output_mode")
        .and_then(|m| m.as_str())
        .unwrap_or("files_with_matches");
    let glob_filter = input
        .get("glob")
        .and_then(|g| g.as_str())
        .and_then(|g| globset::GlobBuilder::new(g).literal_separator(false).build().ok())
        .map(|g| g.compile_matcher());
    let type_filter = input.get("type").and_then(|t| t.as_str()).map(|t| {
        let exts: Vec<&str> = match t {
            "rust" => vec!["rs"],
            "js" => vec!["js", "jsx", "mjs", "cjs"],
            "ts" => vec!["ts", "tsx"],
            "py" => vec!["py"],
            "go" => vec!["go"],
            "java" => vec!["java"],
            "c" => vec!["c", "h"],
            "cpp" => vec!["cpp", "cc", "hpp"],
            "md" => vec!["md"],
            "json" => vec!["json"],
            other => vec![other],
        };
        exts
    });

    let before = input.get("-B").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let after = input.get("-A").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let context = input.get("-C").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let before = before.max(context);
    let after = after.max(context);
    let limit = input.get("head_limit").and_then(|v| v.as_u64()).unwrap_or(250) as usize;
    let skip = input.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    let mut files = walk_files(&search_path);
    if let Some(g) = &glob_filter {
        files.retain(|e| g.is_match(&e.path));
    }
    if let Some(exts) = &type_filter {
        files.retain(|e| {
            e.path
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| exts.contains(&x))
                .unwrap_or(false)
        });
    }

    let mut out_lines: Vec<String> = Vec::new();
    let mut match_count = 0usize;
    let mut file_count = 0usize;
    let mut counts: Vec<String> = Vec::new();
    for e in files {
        let Ok(text) = std::fs::read_to_string(&e.path) else { continue };
        if text.len() > 2_000_000 {
            continue;
        }
        let lines: Vec<&str> = text.lines().collect();
        let mut matched_lines: Vec<usize> = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if re.is_match(line) {
                matched_lines.push(i);
            }
        }
        if matched_lines.is_empty() {
            continue;
        }
        file_count += 1;
        match_count += matched_lines.len();
        let display = display_path(&ctx.cwd, &e.path);
        match mode {
            "count" => counts.push(format!("{display}: {}", matched_lines.len())),
            "content" => {
                for i in matched_lines {
                    let start = i.saturating_sub(before);
                    let end = (i + after + 1).min(lines.len());
                    for j in start..end {
                        let marker = if j == i { ":" } else { "-" };
                        out_lines.push(format!("{display}{marker}{}: {}", j + 1, lines[j]));
                    }
                    if after > 0 {
                        out_lines.push("--".into());
                    }
                }
            }
            _ => out_lines.push(display),
        }
    }
    let body = match mode {
        "count" => counts.join("\n"),
        "content" => out_lines
            .iter()
            .skip(skip)
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n"),
        _ => out_lines
            .iter()
            .skip(skip)
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n"),
    };
    if file_count == 0 {
        return ToolResult::ok("No matches found");
    }
    let mut result = body;
    if mode != "count" && skip + limit < out_lines.len() {
        result.push_str(&format!(
            "\n\n(truncated; continue with offset={})",
            skip + limit
        ));
    }
    result.push_str(&format!(
        "\n\n[{file_count} files, {match_count} matches]"
    ));
    ToolResult::ok(super::limit_output(&result))
}

pub async fn view_image(ctx: &ToolCtx, input: &Value) -> ToolResult {
    let path = match input.get("file_path").and_then(|p| p.as_str()) {
        Some(p) => resolve_path(&ctx.cwd, p),
        None => return ToolResult::err("`file_path` is required"),
    };
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => return ToolResult::err(format!("{}: {e}", path.display())),
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(data);
    ToolResult::ok(format!(
        "Image: {} (detail: {})",
        path.display(),
        input.get("detail").and_then(|d| d.as_str()).unwrap_or("high")
    ))
    .with_images(vec![(
        crate::llm::guess_media_type(&path).to_string(),
        b64,
    )])
}
