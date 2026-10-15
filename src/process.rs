//! Bounded external commands: every git/gh/orca/tmux call goes through [`output`] so a hung
//! credential prompt or network stall cannot block the harness forever.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::LazyLock;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};

/// Budget for commands that talk to the network (git fetch/push, gh).
pub const NETWORK: Duration = Duration::from_secs(120);
/// Budget for local commands (git status, tmux, orca).
pub const LOCAL: Duration = Duration::from_secs(30);

const SIGKILL: i32 = 9;

unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

/// The user configured `core.sshCommand` (an identity file or proxy); `GIT_SSH_COMMAND` would
/// override it, so it is left alone then.
static USER_SSH_COMMAND: LazyLock<bool> = LazyLock::new(|| {
    Command::new("git")
        .args(["config", "--get", "core.sshCommand"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|out| out.status.success() && !out.stdout.trim_ascii().is_empty())
});

/// Runs `program args` with stdin closed and stdout/stderr captured. Git is told never to prompt.
/// The child runs in its own process group; the whole group is killed when it exceeds `timeout`.
pub fn output(program: &str, args: &[&str], cwd: Option<&Path>, timeout: Duration) -> Result<Output> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    command.env("GIT_TERMINAL_PROMPT", "0");
    if std::env::var_os("GIT_SSH_COMMAND").is_none() && !*USER_SSH_COMMAND {
        command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes -o ConnectTimeout=15");
    }
    let mut child = command.spawn().map_err(|err| Error(format!("{program} 실행 실패: {err}")))?;
    let pgid = i32::try_from(child.id()).unwrap_or(0);
    // Drain the pipes on threads so a chatty child cannot fill a pipe buffer and stall.
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);
    let started = Instant::now();
    let timed_out = || Error(format!("{program} {}: {}초 안에 끝나지 않아 멈췄다", args.join(" "), timeout.as_secs()));
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            kill_group(pgid);
            let _ = child.kill();
            let _ = child.wait();
            return Err(timed_out());
        }
        thread::sleep(Duration::from_millis(20));
    };
    // A grandchild (credential helper, ssh master) may still hold the pipes open: wait for the
    // output only within what is left of the budget, then take what was read.
    let collect = |pipe: Option<Receiver<Vec<u8>>>| {
        let left = timeout.saturating_sub(started.elapsed()).max(Duration::from_millis(100));
        pipe.and_then(|rx| rx.recv_timeout(left).ok()).unwrap_or_default()
    };
    let stdout = collect(stdout);
    let stderr = collect(stderr);
    Ok(Output { status, stdout, stderr })
}

fn kill_group(pgid: i32) {
    if pgid > 0 {
        // SAFETY: plain syscall; a negative pid addresses the group this function created.
        unsafe {
            kill(-pgid, SIGKILL);
        }
    }
}

fn drain(mut pipe: impl Read + Send + 'static) -> Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    rx
}
