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

/// `pid:<pid>[/<start>]:<log>` → the pid.
pub fn pid_of(handle: &str) -> Result<i32> {
    Ok(parse(handle)?.0)
}

/// `pid:<pid>[/<start>]:<log>` → (pid, start time recorded at launch). Handles written before the
/// start time was recorded have none.
fn parse(handle: &str) -> Result<(i32, Option<&str>)> {
    let rest = handle.strip_prefix("pid:").ok_or_else(|| Error(format!("headless handle이 아니다: {handle}")))?;
    let head = rest.split(':').next().unwrap_or_default();
    let (pid, start) = match head.split_once('/') {
        Some((pid, start)) => (pid, Some(start).filter(|s| !s.is_empty())),
        None => (head, None),
    };
    match pid.parse::<i32>() {
        Ok(pid) if pid > 1 => Ok((pid, start)),
        _ => crate::bail!("headless handle의 pid가 잘못됐다: {handle}"),
    }
}

/// `pid:<pid>[/<start>]:<log>` → the log path (tail needs it and the trait only passes the handle).
fn log_path(handle: &str) -> Result<PathBuf> {
    let rest = handle.strip_prefix("pid:").ok_or_else(|| Error(format!("headless handle이 아니다: {handle}")))?;
    match rest.split_once(':') {
        Some((_, log)) if !log.is_empty() => Ok(PathBuf::from(log)),
        _ => crate::bail!("headless handle에 로그 경로가 없다: {handle}"),
    }
}

/// The start time of `pid` as `ps -o lstart=` prints it, with spaces and colons replaced so it fits
/// in the handle. `None` when no such process exists.
pub fn start_time(pid: i32) -> Option<String> {
    let pid = pid.to_string();
    let out = crate::process::output("ps", &["-o", "lstart=", "-p", &pid], None, crate::process::LOCAL).ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let words: Vec<&str> = text.split_whitespace().collect();
    (out.status.success() && !words.is_empty()).then(|| words.join("_").replace(':', "."))
}

/// Whether `pid` has exited and only waits to be reaped (`ps` state `Z`).
fn zombie(pid: i32) -> bool {
    let pid = pid.to_string();
    crate::process::output("ps", &["-o", "stat=", "-p", &pid], None, crate::process::LOCAL)
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).trim_start().starts_with('Z'))
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
        let pid = child.id();
        // The start time lets `close` tell this session from a later process that reused the pid.
        let start = i32::try_from(pid).ok().and_then(start_time).map(|s| format!("/{s}")).unwrap_or_default();
        Ok(format!("pid:{pid}{start}:{}", log.display()))
    }

    fn tail(&self, handle: &str, lines: usize) -> Result<String> {
        let log = log_path(handle)?;
        let text = fs::read_to_string(&log).map_err(|e| Error(format!("{}: {e}", log.display())))?;
        let rows: Vec<&str> = text.trim_end().lines().collect();
        Ok(rows[rows.len().saturating_sub(lines)..].join("\n"))
    }

    /// SIGTERM to the session's process group (`sh -c` and the runner under it) if it still runs.
    /// Never a bare pid: when the pid now belongs to a process with another start time, the session
    /// is gone and the handle is stale, which counts as closed.
    fn close(&self, handle: &str) -> Result<()> {
        let (pid, start) = parse(handle)?;
        // SAFETY: kill only sends a signal; signal 0 just probes whether the group exists.
        if unsafe { kill(-pid, 0) } != 0 {
            return Ok(());
        }
        // While any member of the group lives, its id (the leader's pid) cannot be handed out again;
        // so only a live leader with a different start time means the pid was reused.
        if let (Some(start), Some(now)) = (start, start_time(pid))
            && now != start
        {
            return Ok(());
        }
        // SAFETY: as above; the child called setsid, so its pid is its process group id.
        if unsafe { kill(-pid, SIGTERM) } != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() != Some(ESRCH) {
                crate::bail!("headless 세션(pid {pid}) 종료 실패: {err}");
            }
        }
        Ok(())
    }

    /// The session's process group still has members and its leader is the recorded process and
    /// not merely an unreaped exit (a zombie left to the process that started it).
    fn is_open(&self, handle: &str) -> Result<Option<bool>> {
        let (pid, start) = parse(handle)?;
        // SAFETY: signal 0 only probes whether the group exists.
        if unsafe { kill(-pid, 0) } != 0 {
            return Ok(Some(false));
        }
        let reused = matches!((start, start_time(pid)), (Some(start), Some(now)) if now != start);
        Ok(Some(!reused && !zombie(pid)))
    }

    fn remove_workspace(&self, _workspace_id: &str) -> Result<()> {
        Ok(())
    }
}
