use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const B32: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// ULID: 48-bit millisecond timestamp + 80 random bits, Crockford base32.
pub fn ulid() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut out = String::with_capacity(26);
    for i in (0..10).rev() {
        out.push(B32[((ms >> (5 * i)) & 31) as usize] as char);
    }
    let mut rng = rand::thread_rng();
    for i in 0..16 {
        // 5 bits each; the first nibble of the random part only uses 2 bits in spec,
        // 5 random bits is accepted by every ULID parser in practice.
        let v = if i == 0 {
            (rand::Rng::gen_range(&mut rng, 0..16u8)) & 0x0f
        } else {
            rand::Rng::gen_range(&mut rng, 0..32u8)
        };
        out.push(B32[v as usize] as char);
    }
    out
}

pub fn now_rfc3339() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format_rfc3339(d.as_secs(), d.subsec_nanos())
}

pub fn format_rfc3339(secs: u64, nanos: u32) -> String {
    let days = (secs / 86400) as i64;
    let rem = (secs % 86400) as i64;
    let (y, m, d) = civil_from_days(days);
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    if nanos == 0 {
        format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
    } else {
        let frac = format!("{:09}", nanos);
        let frac = frac.trim_end_matches('0');
        format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{frac}Z")
    }
}

pub fn today_utc() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let (y, m, d) = civil_from_days((secs / 86400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Truncate text to `max` bytes on a char boundary, with a trailing marker.
pub fn truncate(text: &str, max: usize, marker: &str) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}{}", &text[..end], marker)
}

pub fn home_dir() -> PathBuf {
    if let Ok(h) = std::env::var("HOME") {
        if !h.is_empty() {
            return PathBuf::from(h);
        }
    }
    PathBuf::from("/")
}

/// Ante home: $ANTE_HOME, else ~/.ante
pub fn ante_home() -> PathBuf {
    if let Ok(h) = std::env::var("ANTE_HOME") {
        if !h.is_empty() {
            return PathBuf::from(h);
        }
    }
    home_dir().join(".ante")
}

pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    if p == "~" {
        return home_dir();
    }
    PathBuf::from(p)
}

pub fn write_file(path: &Path, contents: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::File::create(path)?;
    f.write_all(contents.as_bytes())?;
    Ok(())
}

pub fn is_git_repo(dir: &Path) -> bool {
    let mut cur = Some(dir.to_path_buf());
    while let Some(d) = cur {
        if d.join(".git").exists() {
            return true;
        }
        cur = d.parent().map(|p| p.to_path_buf());
    }
    false
}

pub fn os_name() -> &'static str {
    std::env::consts::OS
}

pub fn shell_name() -> String {
    std::env::var("SHELL")
        .ok()
        .and_then(|s| s.rsplit('/').next().map(|v| v.to_string()))
        .unwrap_or_else(|| "unknown".into())
}

/// A shallow directory listing used as ambient context for a prompt.
pub fn folder_structure(dir: &Path, depth: usize) -> String {
    fn walk(dir: &Path, prefix: &str, depth: usize, out: &mut String) {
        if depth == 0 {
            return;
        }
        let mut entries: Vec<_> = match std::fs::read_dir(dir) {
            Ok(e) => e.flatten().collect(),
            Err(_) => return,
        };
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().to_string();
            if name == ".git" || name == "target" || name == "node_modules" {
                continue;
            }
            let is_dir = e.path().is_dir();
            out.push_str(&format!("{}- {}{}\n", prefix, name, if is_dir { "/" } else { "" }));
            if is_dir {
                walk(&e.path(), &format!("{}  ", prefix), depth - 1, out);
            }
        }
    }
    let mut out = String::new();
    walk(dir, "  ", depth, &mut out);
    out
}

pub fn path_from_str(p: &str) -> PathBuf {
    expand_tilde(p)
}

#[cfg(test)]
pub static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
