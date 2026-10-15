use std::fs::{self, File};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::{Open, Workspace, shell_join};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::registry::Entry;

pub struct Headless;

const SIGTERM: i32 = 15;
const ESRCH: i32 = 3;

unsafe extern "C" {
    fn setsid() -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
}

/// `pid:<pid>:<log>` → the pid.
pub fn pid_of(handle: &str) -> Result<i32> {
    let rest = handle.strip_prefix("pid:").ok_or_else(|| Error(format!("headless handle이 아니다: {handle}")))?;
    match rest.split(':').next().and_then(|p| p.parse::<i32>().ok()) {
        Some(pid) if pid > 1 => Ok(pid),
        _ => crate::bail!("headless handle의 pid가 잘못됐다: {handle}"),
    }
}

/// `pid:<pid>:<log>` → the log path (tail needs it and the trait only passes the handle).
fn log_path(handle: &str) -> Result<PathBuf> {
    let rest = handle.strip_prefix("pid:").ok_or_else(|| Error(format!("headless handle이 아니다: {handle}")))?;
    match rest.split_once(':') {
        Some((_, log)) if !log.is_empty() => Ok(PathBuf::from(log)),
        _ => crate::bail!("headless handle에 로그 경로가 없다: {handle}"),
    }
}

impl Workspace for Headless {
    fn name(&self) -> &'static str {
        "headless"
    }

    fn available(&self) -> bool {
        true
    }

    fn interactive(&self) -> bool {
        false
    }

    fn create(&self, _entry: &Entry, _config: &Config) -> Result<Option<String>> {
        Ok(None)
    }

    /// Detached in a new session with stdin closed; stdout and stderr go to `.harness/session.log`.
    fn open(&self, spec: &Open<'_>) -> Result<String> {
        let meta = spec.cwd.join(".harness");
        fs::create_dir_all(&meta)?;
        let log = meta.join("session.log");
        let out = File::options().create(true).append(true).open(&log)?;
        let err = out.try_clone()?;
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(shell_join(&spec.argv))
            .current_dir(&spec.cwd)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        // SAFETY: setsid is async-signal-safe and touches no memory of the parent.
        unsafe {
            command.pre_exec(|| if setsid() < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) });
        }
        let child = command.spawn().map_err(|e| Error(format!("headless 실행 실패: {e}")))?;
        Ok(format!("pid:{}:{}", child.id(), log.display()))
    }

    fn tail(&self, handle: &str, lines: usize) -> Result<String> {
        let log = log_path(handle)?;
        let text = fs::read_to_string(&log).map_err(|e| Error(format!("{}: {e}", log.display())))?;
        let rows: Vec<&str> = text.trim_end().lines().collect();
        Ok(rows[rows.len().saturating_sub(lines)..].join("\n"))
    }

    /// SIGTERM to the session's process group (`sh -c` and the runner under it) if it still runs.
    fn close(&self, handle: &str) -> Result<()> {
        let pid = pid_of(handle)?;
        // SAFETY: kill only sends a signal; signal 0 just probes for existence.
        unsafe {
            if kill(pid, 0) != 0 {
                return Ok(());
            }
            // The child called setsid, so its pid is also its process group id.
            if kill(-pid, SIGTERM) != 0 && kill(pid, SIGTERM) != 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() != Some(ESRCH) {
                    crate::bail!("headless 세션(pid {pid}) 종료 실패: {err}");
                }
            }
        }
        Ok(())
    }

    fn remove_workspace(&self, _workspace_id: &str) -> Result<()> {
        Ok(())
    }
}
