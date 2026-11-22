//! Background job registry for long-running Bash commands.
//!
//! A job owns a directory under `ANTE_HOME/jobs/<id>` containing `meta.json`,
//! `status.json`, `stdout`, and `stderr`. The waiter thread writes the final
//! status and queues a one-shot notification for the next agent turn.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use crate::util::ante_home;

#[derive(Debug, Clone)]
struct JobExit {
    code: i32,
    success: bool,
}

struct JobState {
    id: String,
    pid: u32,
    command: String,
    dir: PathBuf,
    started: String,
    exit: Mutex<Option<JobExit>>,
    background: AtomicBool,
}

pub struct JobRegistry {
    dir: PathBuf,
    jobs: Mutex<HashMap<String, Arc<JobState>>>,
    notifications: Arc<Mutex<Vec<String>>>,
}

pub enum JobOutcome {
    Exited {
        code: i32,
        success: bool,
        stdout: String,
        stderr: String,
    },
    Running {
        id: String,
        pid: u32,
        stdout_path: PathBuf,
        stderr_path: PathBuf,
        status_path: PathBuf,
        stdout: String,
        stderr: String,
    },
}

impl JobRegistry {
    pub fn new() -> Self {
        Self {
            dir: ante_home().join("jobs"),
            jobs: Mutex::new(HashMap::new()),
            notifications: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn run(
        &self,
        shell: &str,
        command: &str,
        cwd: &Path,
        max_wait: Duration,
    ) -> Result<JobOutcome> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("create job directory {}", self.dir.display()))?;
        let id = format!("job_{}", crate::util::ulid());
        let dir = self.dir.join(&id);
        std::fs::create_dir_all(&dir)?;
        let stdout_path = dir.join("stdout");
        let stderr_path = dir.join("stderr");
        let status_path = dir.join("status.json");
        let meta_path = dir.join("meta.json");
        let stdout_file = File::create(&stdout_path)?;
        let stderr_file = File::create(&stderr_path)?;

        let mut cmd = Command::new(shell);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Put the shell in its own process group so `kill -- -<pid>` stops
            // the whole job without touching Ante itself.
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setpgid(0, 0) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        cmd.arg("-lc")
            .arg(command)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file));
        let started = crate::util::now_rfc3339();
        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to start command: {command}"))?;
        let pid = child.id();
        let meta = json!({
            "id": id,
            "pid": pid,
            "command": command,
            "cwd": cwd.display().to_string(),
            "started": started,
            "status_path": status_path.display().to_string(),
            "stdout_path": stdout_path.display().to_string(),
            "stderr_path": stderr_path.display().to_string(),
        });
        std::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;

        let state = Arc::new(JobState {
            id: id.clone(),
            pid,
            command: command.to_string(),
            dir: dir.clone(),
            started: started.clone(),
            exit: Mutex::new(None),
            background: AtomicBool::new(false),
        });
        self.jobs
            .lock()
            .map_err(|_| anyhow!("job registry lock poisoned"))?
            .insert(id.clone(), state.clone());

        let worker = state.clone();
        let notifications = self.notifications.clone();
        std::thread::spawn(move || {
            let status = child.wait();
            let exit = match status {
                Ok(s) => JobExit {
                    code: s.code().unwrap_or(-1),
                    success: s.success(),
                },
                Err(e) => {
                    eprintln!("warning: failed to wait for job {}: {e}", worker.id);
                    JobExit {
                        code: -1,
                        success: false,
                    }
                }
            };
            if let Ok(mut guard) = worker.exit.lock() {
                *guard = Some(exit.clone());
            }
            let status_json = json!({
                "id": worker.id,
                "pid": worker.pid,
                "command": worker.command,
                "started": worker.started,
                "finished": crate::util::now_rfc3339(),
                "exit_code": exit.code,
                "success": exit.success,
            });
            let _ = std::fs::write(
                worker.dir.join("status.json"),
                serde_json::to_string_pretty(&status_json).unwrap_or_default(),
            );
            if worker.background.load(Ordering::SeqCst) {
                if let Ok(mut notes) = notifications.lock() {
                    notes.push(format!(
                        "job {} finished: exit_code={} stdout={} stderr={} status={}",
                        worker.id,
                        exit.code,
                        worker.dir.join("stdout").display(),
                        worker.dir.join("stderr").display(),
                        worker.dir.join("status.json").display()
                    ));
                }
            }
        });

        let deadline = Instant::now() + max_wait;
        loop {
            if let Some(exit) = state.exit.lock().ok().and_then(|g| g.clone()) {
                return Ok(JobOutcome::Exited {
                    code: exit.code,
                    success: exit.success,
                    stdout: read_limited(&stdout_path),
                    stderr: read_limited(&stderr_path),
                });
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        state.background.store(true, Ordering::SeqCst);
        Ok(JobOutcome::Running {
            id,
            pid,
            stdout_path,
            stderr_path,
            status_path,
            stdout: read_limited(&dir.join("stdout")),
            stderr: read_limited(&dir.join("stderr")),
        })
    }

    pub fn drain_notifications(&self) -> Vec<String> {
        self.notifications
            .lock()
            .map(|mut n| std::mem::take(&mut *n))
            .unwrap_or_default()
    }

    pub fn list(&self) -> Vec<Value> {
        let jobs = self
            .jobs
            .lock()
            .map(|g| g.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        jobs.into_iter()
            .map(|j| {
                let exit = j.exit.lock().ok().and_then(|g| g.clone());
                json!({
                    "id": j.id,
                    "pid": j.pid,
                    "command": j.command,
                    "running": exit.is_none(),
                    "exit_code": exit.map(|e| e.code),
                    "stdout": j.dir.join("stdout").display().to_string(),
                    "stderr": j.dir.join("stderr").display().to_string(),
                })
            })
            .collect()
    }
}

fn read_limited(path: &Path) -> String {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return String::new(),
    };
    let lossy = String::from_utf8_lossy(&bytes);
    crate::tools::limit_output(&lossy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_command_exits_inline_and_long_command_returns_handle() {
        let _guard = crate::util::TEST_ENV_LOCK.lock().unwrap();
        let home = std::env::temp_dir().join(format!("ante-jobs-test-{}", crate::util::ulid()));
        std::env::set_var("ANTE_HOME", &home);
        let registry = JobRegistry::new();
        let cwd = std::env::temp_dir();
        let shell = "/bin/sh";

        let short = registry
            .run(shell, "printf hello", &cwd, Duration::from_secs(2))
            .expect("short job");
        match short {
            JobOutcome::Exited { code, stdout, .. } => {
                assert_eq!(code, 0);
                assert!(stdout.contains("hello"));
            }
            JobOutcome::Running { .. } => panic!("short command should finish inline"),
        }

        let long = registry
            .run(shell, "sleep 1; printf done", &cwd, Duration::from_millis(150))
            .expect("long job");
        match long {
            JobOutcome::Running {
                id, stdout_path, ..
            } => {
                assert!(id.starts_with("job_"));
                assert!(stdout_path.exists());
                std::thread::sleep(Duration::from_millis(1200));
                let notes = registry.drain_notifications();
                assert!(notes.iter().any(|n| n.contains("finished")), "{notes:?}");
            }
            JobOutcome::Exited { .. } => panic!("long command should return a background handle"),
        }
        let _ = std::fs::remove_dir_all(home);
    }
}
