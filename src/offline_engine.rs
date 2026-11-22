//! Offline engine installer.
//!
//! This mirrors the upstream Ante 0.3.4 offline pipeline: a pinned llama.cpp
//! release (b12182), a table of platform/accelerator artifacts with SHA-256
//! digests, staged download, hash verification, extraction, a post-install
//! self-check, and an atomic swap into `<ante-home>/offline`.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::util;

pub const PINNED_VERSION: &str = "b12182";
pub const MIRROR_BASE: &str = "https://download.ante.run/llama-cpp";
pub const GITHUB_BASE: &str = "https://github.com/ggml-org/llama.cpp/releases/download";

/// Every artifact shipped by the pinned release, with its embedded digest.
pub const ARTIFACTS: &[(&str, &str)] = &[
    ("llama-b12182-bin-macos-arm64.tar.gz", "71cfee075e0c54852e225dafdc8dc79c32b26e8d45f052369b7d4f78d2ccf1a6"),
    ("llama-b12182-bin-macos-x64.tar.gz", "8447e61fb17a6b878c958750b181e75fa3295a07b3e5922529e8c259d8e175ac"),
    ("llama-b12182-bin-cpu-kk-linux-arm64.tar.gz", "6ac98fe7d19a37283d33c5a7dbdb68990ea4b7f2407f0f718a0156dd8bc7c552"),
    ("llama-b12182-bin-cpu-nk-linux-arm64.tar.gz", "3a933d0100c3ca488b0d9b5780bc67c0a704f2bd343f40ed4abdd33db474dee2"),
    ("llama-b12182-bin-cpu-rk-linux-arm64.tar.gz", "d83e87e175251a754645653870f9beb5f075f93de63b3b94ef98b084c197526f"),
    ("llama-b12182-bin-cpu-ql-linux-arm64.tar.gz", "98adbb4bcda98c9e9119d365ed5defb54900bc5444397ea420d3c36a05d587d5"),
    ("llama-b12182-bin-cpu-kkkkk-linux-x64.tar.gz", "576724a6a42ded1cd2954cbbc77fec4c2ca1ce2f182477fc493c0266bc355b50"),
    ("llama-b12182-bin-cpu-ylkkk-linux-x64.tar.gz", "7d076f0ed92c695f697850a18253c1676516dc44671c747920cdca850c387a2c"),
    ("llama-b12182-bin-cpu-qnkkk-linux-x64.tar.gz", "1637222656def9066a4985d8f66065aa88a88ed8115ddaab416d40e4ccc42a91"),
    ("llama-b12182-bin-cpu-klzkk-linux-x64.tar.gz", "30532e7cd6313035862642df73dc7d24ce4a6bde2c1ccef030969e9d20990af5"),
    ("llama-b12182-bin-cpu-klzlk-linux-x64.tar.gz", "0e34e3b8402bbbfe03889e553575a139002c8403c5449a19bdafb2d243131ba1"),
    ("llama-b12182-bin-vulkan-kk-linux-arm64.tar.gz", "bd3a0dad891c3d08ba2504469530b99d99cd57bf5f14c6a9d9f21be3092e659f"),
    ("llama-b12182-bin-vulkan-nk-linux-arm64.tar.gz", "30cc20ba1a551aef95f811c85848d3161ce880fafdb86ec2a70faaf79b570f66"),
    ("llama-b12182-bin-vulkan-rk-linux-arm64.tar.gz", "1c858671e78ac4ac5989c73234539c84d6612b6a8dd51249d12f3cc241f33c47"),
    ("llama-b12182-bin-vulkan-ql-linux-arm64.tar.gz", "7d574b570d6cb144dd09a3ef50adc27fef03d318767eb16db97a281caa231f04"),
    ("llama-b12182-bin-vulkan-kkkkk-linux-x64.tar.gz", "cabd8926776e36edf0c38fa1ad7d449c4a027816d0b9933f511443557c823ea2"),
    ("llama-b12182-bin-vulkan-ylkkk-linux-x64.tar.gz", "50d0189d747415a99d8a694aa482e69b3e2dec9c30d229d7fe670ea4ac378132"),
    ("llama-b12182-bin-vulkan-qnkkk-linux-x64.tar.gz", "86750ea20d71ee4b3a802895f117f8a693b113ff7f308dd9ba1cb9677df7ba94"),
    ("llama-b12182-bin-vulkan-klzkk-linux-x64.tar.gz", "4364a49b1fb64a163a39de6c1e64b45740bc05cc5eb9ab5f3456d8fc32e9bf3e"),
    ("llama-b12182-bin-vulkan-klzlk-linux-x64.tar.gz", "87ec4bc0895f4f8a8acb1a6edd70022da824d7cefecb7faa5541c189fc2569c5"),
    ("llama-b12182-bin-cuda-86-linux-arm64.tar.gz", "4939e401b9d8e7b712a77267bc8347deef5504c133a5620aeac3c163ba42b237"),
    ("llama-b12182-bin-cuda-89-linux-arm64.tar.gz", "134e369b458f8231810048678bbe7ecce52207b5603c959349fa9da2c951b519"),
    ("llama-b12182-bin-cuda-90-linux-arm64.tar.gz", "2a70ffe355023b90a8b473fe790f83b3a59b2ae90862f69c861e7857cccca1d9"),
    ("llama-b12182-bin-cuda-100-linux-arm64.tar.gz", "5bd458b1d03333707685378f0c9126903cf3e5543561ff88637921928e373bdf"),
    ("llama-b12182-bin-cuda-120-linux-arm64.tar.gz", "c43fe1c7d716382a0f303b5e5bb376232aa6ea9b5e51644572bda4fff4549bd2"),
    ("llama-b12182-bin-cuda-86-linux-x64.tar.gz", "18e77c9ba525eea514c29179d0efc29a193a0be96c2edb488a81e96c6721d8c2"),
    ("llama-b12182-bin-cuda-89-linux-x64.tar.gz", "dec64798d5d0b9cf3dfd97413f8913f2424a56bcb002e0f8bd4a3803319814da"),
    ("llama-b12182-bin-cuda-90-linux-x64.tar.gz", "458e8cfbacdc32a0d5756406d3f16d20c1267c0b28b2ab078d9850e8d9b04adc"),
    ("llama-b12182-bin-cuda-100-linux-x64.tar.gz", "5ff6e3c972feaa64819dfc2df182c4aeda51547e731b961f590426ce164f4445"),
    ("llama-b12182-bin-cuda-120-linux-x64.tar.gz", "5168dac9cb9222d9d328659729df0699635bc089070f79319a8867d04bb7a618"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accelerator {
    Cpu(String),
    Cuda(u32),
    Vulkan(String),
}

impl std::fmt::Display for Accelerator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Accelerator::Cpu(tag) => write!(f, "cpu-{tag}"),
            Accelerator::Cuda(cap) => write!(f, "cuda-{cap}"),
            Accelerator::Vulkan(tag) => write!(f, "vulkan-{tag}"),
        }
    }
}

pub fn engine_dir() -> PathBuf {
    std::env::var_os("ANTE_OFFLINE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| util::ante_home().join("offline"))
}

pub fn engine_binary() -> PathBuf {
    engine_dir().join("llama-server")
}

pub fn installed() -> bool {
    engine_binary().is_file()
}

pub fn platform_tag() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("linux-x64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("macos", "aarch64") => Ok("macos-arm64"),
        ("macos", "x86_64") => Ok("macos-x64"),
        (os, arch) => Err(anyhow!("unsupported offline platform: {os}/{arch}")),
    }
}

fn read_proc_cpuinfo() -> String {
    fs::read_to_string("/proc/cpuinfo").unwrap_or_default()
}

/// Pick the x86-64 CPU microarchitecture bucket used by the pinned release.
fn cpu_tag() -> String {
    if std::env::consts::ARCH == "aarch64" {
        return "kk".to_string();
    }
    let info = read_proc_cpuinfo().to_lowercase();
    let has = |f: &str| info.split_whitespace().any(|x| x == f);
    if has("avx512f") && has("avx512vbmi") {
        "klzlk".to_string()
    } else if has("avx512f") {
        "klzkk".to_string()
    } else if has("avx2") && has("fma") && has("f16c") {
        "qnkkk".to_string()
    } else if has("avx2") {
        "ylkkk".to_string()
    } else {
        "kkkkk".to_string()
    }
}

fn nvidia_compute_cap() -> Option<u32> {
    let out = Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,name,memory.total,memory.free,compute_cap",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut best = None;
    for line in text.lines() {
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        if let Some(cap) = fields.last() {
            if let Some((major, minor)) = cap.split_once('.') {
                if let (Ok(a), Ok(b)) = (major.parse::<u32>(), minor.parse::<u32>()) {
                    best = Some(best.map_or(a * 10 + b, |x: u32| x.max(a * 10 + b)));
                }
            }
        }
    }
    best
}

fn vulkan_available() -> bool {
    if std::env::var_os("VK_ICD_FILENAMES").is_some() {
        return true;
    }
    ["/usr/share/vulkan/icd.d", "/etc/vulkan/icd.d"]
        .iter()
        .any(|d| fs::read_dir(d).map(|mut r| r.next().is_some()).unwrap_or(false))
}

pub fn detect_accelerator() -> Accelerator {
    let forced = std::env::var("ANTE_LLAMA_ACCEL").unwrap_or_default().to_lowercase();
    match forced.as_str() {
        "cpu" => return Accelerator::Cpu(cpu_tag()),
        "vulkan" => return Accelerator::Vulkan(cpu_tag()),
        "cuda" => {
            if let Some(cap) = nvidia_compute_cap() {
                return Accelerator::Cuda(cuda_bucket(cap));
            }
        }
        _ => {}
    }
    if std::env::consts::OS == "linux" {
        if let Some(cap) = nvidia_compute_cap() {
            if let Some(bucket) = compatible_cuda(cap) {
                return Accelerator::Cuda(bucket);
            }
        }
        if vulkan_available() {
            return Accelerator::Vulkan(cpu_tag());
        }
    }
    Accelerator::Cpu(cpu_tag())
}

fn cuda_bucket(cap: u32) -> u32 {
    compatible_cuda(cap).unwrap_or(cap)
}

fn compatible_cuda(cap: u32) -> Option<u32> {
    [86u32, 89, 90, 100, 120].into_iter().filter(|b| *b <= cap).max()
}

pub fn artifact_for(accel: &Accelerator, platform: &str) -> Result<(&'static str, &'static str)> {
    let variant = match accel {
        Accelerator::Cpu(tag) => format!("cpu-{tag}"),
        Accelerator::Cuda(cap) => format!("cuda-{cap}"),
        Accelerator::Vulkan(tag) => format!("vulkan-{tag}"),
    };
    let name = format!("llama-{PINNED_VERSION}-bin-{variant}-{platform}.tar.gz");
    ARTIFACTS
        .iter()
        .find(|(n, _)| *n == name)
        .copied()
        .ok_or_else(|| anyhow!("no pinned llama.cpp artifact for this target ({variant}-{platform})"))
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn download_urls(artifact: &str) -> Vec<String> {
    if let Ok(url) = std::env::var("ANTE_LLAMA_CPP_ARTIFACT_URL") {
        return url.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
    }
    let bases = std::env::var("ANTE_LLAMA_CPP_BASE_URL").unwrap_or_else(|_| {
        format!("{MIRROR_BASE},{GITHUB_BASE}")
    });
    bases
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|b| format!("{}/{PINNED_VERSION}/{artifact}", b.trim_end_matches('/')))
        .collect()
}

fn cached_artifact(artifact: &str) -> Option<PathBuf> {
    let cache = std::env::var_os("LLAMA_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| util::ante_home().join("cache"));
    let p = cache.join(artifact);
    p.is_file().then_some(p)
}

fn copy_or_download(artifact: &str, dest: &Path) -> Result<()> {
    if let Ok(local) = std::env::var("ANTE_LLAMA_ARTIFACT") {
        let src = util::expand_tilde(&local);
        fs::copy(&src, dest)
            .with_context(|| format!("copy local artifact {}", src.display()))?;
        return Ok(());
    }
    if let Some(cached) = cached_artifact(artifact) {
        fs::copy(&cached, dest)
            .with_context(|| format!("copy cached artifact {}", cached.display()))?;
        return Ok(());
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(600))
        .connect_timeout(Duration::from_secs(20))
        .user_agent(format!("ante/{}", crate::VERSION))
        .build()?;
    let mut last = None;
    for url in download_urls(artifact) {
        eprintln!("Downloading prebuilt binary... {url}");
        match client.get(&url).send() {
            Ok(mut resp) if resp.status().is_success() => {
                let mut out = fs::File::create(dest)?;
                io::copy(&mut resp, &mut out)?;
                out.flush()?;
                return Ok(());
            }
            Ok(resp) => last = Some(format!("{url}: HTTP {}", resp.status())),
            Err(e) => last = Some(format!("{url}: {e}")),
        }
    }
    Err(anyhow!(
        "failed to download {artifact}: {}",
        last.unwrap_or_else(|| "no download source configured".into())
    ))
}

fn verify_binary(dir: &Path) -> Result<()> {
    let bin = dir.join("llama-server");
    if !bin.is_file() {
        return Err(anyhow!("extracted archive does not contain llama-server"));
    }
    let out = Command::new(&bin)
        .arg("--version")
        .output()
        .with_context(|| format!("run {} --version", bin.display()))?;
    if !out.status.success() {
        return Err(anyhow!(
            "{} --version failed: {}",
            bin.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

fn extract(archive: &Path, dest: &Path) -> Result<()> {
    eprintln!("Extracting...");
    let status = Command::new("tar")
        .arg("xzf")
        .arg(archive)
        .arg("--strip-components=1")
        .arg("-C")
        .arg(dest)
        .status()
        .context("run tar")?;
    if !status.success() {
        return Err(anyhow!("failed to extract {}", archive.display()));
    }
    eprintln!("Extraction complete");
    Ok(())
}

fn merge_config(fields: Value) -> Result<()> {
    let path = util::ante_home().join("offline-config.json");
    let mut cfg: Value = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    if let (Some(obj), Some(extra)) = (cfg.as_object_mut(), fields.as_object()) {
        for (k, v) in extra {
            obj.insert(k.clone(), v.clone());
        }
    }
    fs::create_dir_all(util::ante_home())?;
    fs::write(&path, serde_json::to_string_pretty(&cfg)?)?;
    Ok(())
}

pub struct InstallReport {
    pub artifact: &'static str,
    pub sha256: &'static str,
    pub accelerator: Accelerator,
    pub engine_dir: PathBuf,
}

/// Install the pinned engine. Honors `ANTE_LLAMA_ARTIFACT` (local archive),
/// `ANTE_LLAMA_ARTIFACT_URL` and `ANTE_LLAMA_CPP_BASE_URL` for testing/mirrors.
pub fn install() -> Result<InstallReport> {
    let platform = platform_tag()?;
    let accel = detect_accelerator();
    let (artifact, expected) = artifact_for(&accel, platform)?;
    eprintln!("Pinned release selected: {PINNED_VERSION} ({artifact})");

    let staging = std::env::temp_dir().join(format!("ante-llama-download-{}", util::ulid()));
    fs::create_dir_all(&staging).context("failed to create download staging directory")?;
    let archive = staging.join(artifact);
    let result = (|| -> Result<()> {
        copy_or_download(artifact, &archive)?;
        let actual = sha256_file(&archive)?;
        if actual != expected {
            let allow = std::env::var("ANTE_LLAMA_ALLOW_UNVERIFIED").ok().as_deref() == Some("1");
            if !allow {
                return Err(anyhow!(
                    "downloaded llama.cpp artifact hash mismatch: expected {expected}, got {actual}"
                ));
            }
            eprintln!("warning: unverified llama.cpp artifact accepted (ANTE_LLAMA_ALLOW_UNVERIFIED=1)");
        } else {
            eprintln!("Download verified");
        }

        let extract_dir = staging.join("extract");
        fs::create_dir_all(&extract_dir)?;
        extract(&archive, &extract_dir)?;

        eprintln!("Verifying installation...");
        verify_binary(&extract_dir)?;
        eprintln!("Installation verified");

        let engine = engine_dir();
        if let Some(parent) = engine.parent() {
            fs::create_dir_all(parent)?;
        }
        let backup = engine.with_file_name(format!("offline.old-{}", util::ulid()));
        if engine.exists() {
            fs::rename(&engine, &backup)
                .with_context(|| format!("move old engine {}", engine.display()))?;
        }
        if let Err(e) = fs::rename(&extract_dir, &engine) {
            if backup.exists() {
                let _ = fs::rename(&backup, &engine);
            }
            return Err(anyhow!("failed to install engine: {e}"));
        }
        let _ = fs::remove_dir_all(&backup);
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    result?;

    eprintln!("Saving configuration...");
    merge_config(json!({
        "engine": "llama.cpp",
        "engine_version": PINNED_VERSION,
        "engine_path": engine_dir().display().to_string(),
        "engine_artifact": artifact,
        "engine_sha256": expected,
        "engine_accelerator": accel.to_string(),
        "engine_installed_at": util::now_rfc3339(),
    }))?;
    eprintln!("Installation complete!");
    Ok(InstallReport {
        artifact,
        sha256: expected,
        accelerator: accel,
        engine_dir: engine_dir(),
    })
}

pub fn status_json() -> Value {
    json!({
        "engine": if installed() { "installed" } else { "not installed" },
        "engine_binary": engine_binary().display().to_string(),
        "engine_dir": engine_dir().display().to_string(),
        "pinned_version": PINNED_VERSION,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_unique_artifacts_and_hashes() {
        let mut names = std::collections::HashSet::new();
        for (name, hash) in ARTIFACTS {
            assert!(names.insert(*name), "duplicate {name}");
            assert_eq!(hash.len(), 64);
            assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn linux_x64_cpu_artifacts_exist() {
        for tag in ["kkkkk", "ylkkk", "qnkkk", "klzkk", "klzlk"] {
            artifact_for(&Accelerator::Cpu(tag.to_string()), "linux-x64").unwrap();
        }
    }

    #[test]
    fn cuda_bucket_never_exceeds_capability() {
        assert_eq!(compatible_cuda(86), Some(86));
        assert_eq!(compatible_cuda(88), Some(86));
        assert_eq!(compatible_cuda(120), Some(120));
        assert_eq!(compatible_cuda(80), None);
    }
}
