//! Where contest sessions run (the ADE/terminal side). Contract: `docs/CONTRACT.md`.
//! - `orca`: Orca workspaces and terminals (`orca` CLI).
//! - `tmux`: one tmux session (`contest-harness`), one window per contest.
//! - `headless`: a detached background process; no TTY, so the runner's one-shot command is used.
//! - `terminal`: replace the current process (used by `ach chat`).

use std::path::PathBuf;

use crate::config::Config;
use crate::error::Result;
use crate::registry::Entry;

pub mod headless;
pub mod orca;
pub mod terminal;
pub mod tmux;

/// What to open: run `argv` in `cwd`, titled `title`, inside `workspace_id` when the backend has one.
pub struct Open<'a> {
    pub cwd: PathBuf,
    pub title: &'a str,
    pub argv: Vec<String>,
    pub workspace_id: Option<&'a str>,
}

pub trait Workspace: Sync {
    fn name(&self) -> &'static str;
    /// The backend's executable is on PATH (always true for headless and terminal).
    fn available(&self) -> bool;
    /// False when sessions have no TTY and must use the runner's one-shot command.
    fn interactive(&self) -> bool;
    /// Create (or reuse) the workspace for a contest. `None` when the backend has no workspaces.
    fn create(&self, entry: &Entry, config: &Config) -> Result<Option<String>>;
    /// Start the session and return a handle that `tail` understands.
    fn open(&self, spec: &Open<'_>) -> Result<String>;
    /// The last `lines` lines of the session's screen or log.
    fn tail(&self, handle: &str, lines: usize) -> Result<String>;
    /// Stop the session behind `handle`. A session that is already gone is not an error.
    fn close(&self, handle: &str) -> Result<()>;
    /// Whether the session behind `handle` still runs: `Some(false)` once it is gone, `None` when the
    /// backend cannot tell (then a successful [`Workspace::close`] is all the confirmation there is).
    fn is_open(&self, _handle: &str) -> Result<Option<bool>> {
        Ok(None)
    }
    /// Terminals in `workspace_id` that may still run something (anything not provably idle), as
    /// short descriptions. `None` when the backend has no workspaces to list. Fails when the list
    /// cannot be read completely, so callers never decide on a partial view.
    fn live_terminals(&self, _workspace_id: &str) -> Result<Option<Vec<(String, String)>>> {
        Ok(None)
    }
    /// Forget the workspace (never the contest folder itself; the harness trashes that). Backends
    /// without workspaces do nothing.
    fn remove_workspace(&self, workspace_id: &str) -> Result<()>;
}

pub const NAMES: &[&str] = &["orca", "tmux", "headless", "terminal"];

pub fn get(name: &str) -> Result<&'static dyn Workspace> {
    Ok(match name {
        "orca" => &orca::Orca,
        "tmux" => &tmux::Tmux,
        "headless" => &headless::Headless,
        "terminal" => &terminal::Terminal,
        _ => crate::bail!("알 수 없는 작업 환경: {name} (가능: {})", NAMES.join(", ")),
    })
}

/// How long [`close_confirmed`] waits for a closed session to disappear.
pub const CLOSE_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Close `handle` and wait (up to `wait`) until the backend no longer reports it running. Fails when
/// the close fails or the session is still there afterwards, so callers keep the handle for a retry.
pub fn close_confirmed(workspace: &dyn Workspace, handle: &str, wait: std::time::Duration) -> Result<()> {
    workspace.close(handle)?;
    let deadline = std::time::Instant::now() + wait;
    loop {
        match workspace.is_open(handle)? {
            Some(true) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(200))
            }
            Some(true) => crate::bail!("{} 세션 {handle}을(를) 닫았지만 아직 실행 중이다", workspace.name()),
            Some(false) | None => return Ok(()),
        }
    }
}

/// Live terminals of `workspace_id` other than `except` (handle, description).
fn others(workspace: &dyn Workspace, workspace_id: Option<&str>, except: Option<&str>) -> Result<Vec<String>> {
    let Some(id) = workspace_id else { return Ok(Vec::new()) };
    let Some(live) = workspace.live_terminals(id)? else { return Ok(Vec::new()) };
    Ok(live.into_iter().filter(|(handle, _)| Some(handle.as_str()) != except).map(|(_, text)| text).collect())
}

/// Stop the session recorded as `handle` in `workspace_id`.
/// - The recorded handle still runs: close it and wait until it is gone ([`close_confirmed`]).
/// - It is already gone (stale after a backend restart) or there is none: the workspace must hold
///   no other live terminal. One that does may be the same agent reconnected under a new handle,
///   and which one owns the contest cannot be told, so this refuses instead of adopting one (the
///   caller must not open a duplicate or remove the workspace). An empty workspace counts as gone.
pub fn close_recorded(
    workspace: &dyn Workspace,
    handle: Option<&str>,
    workspace_id: Option<&str>,
    wait: std::time::Duration,
) -> Result<()> {
    // The recorded handle still runs, or the backend cannot tell: its confirmed close decides.
    if let Some(h) = handle
        && workspace.is_open(h)? != Some(false)
    {
        return close_confirmed(workspace, h, wait);
    }
    ensure_no_other_terminals(workspace, workspace_id, handle)
}

/// Fails when `workspace_id` has a live terminal other than `closed` (see [`close_recorded`]).
/// Call it before removing a workspace: removal stops every terminal in it.
pub fn ensure_no_other_terminals(
    workspace: &dyn Workspace,
    workspace_id: Option<&str>,
    closed: Option<&str>,
) -> Result<()> {
    let live = others(workspace, workspace_id, closed)?;
    if !live.is_empty() {
        crate::bail!(
            "{} 작업 공간 {}에 기록되지 않은 터미널이 살아 있다({}). 기록된 세션 {}과(와) 같은 세션인지 알 수 없어 자동으로 닫거나 다시 열거나 작업 공간을 지우지 않는다. 확인해 정리한 뒤 다시 실행한다",
            workspace.name(),
            workspace_id.unwrap_or("-"),
            live.join(", "),
            closed.unwrap_or("(없음)")
        );
    }
    Ok(())
}

/// Quote one word for a POSIX shell (zsh and sh agree on single quotes).
pub fn shell_quote(word: &str) -> String {
    if !word.is_empty() && word.bytes().all(|b| b.is_ascii_alphanumeric() || b"@%+=:,./_-".contains(&b)) {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// `argv` as one shell line.
pub fn shell_join(argv: &[String]) -> String {
    argv.iter().map(|w| shell_quote(w)).collect::<Vec<_>>().join(" ")
}

/// The line a terminal types: `cd <cwd> && <argv>` (pi, claude and dsh have no cwd flag).
pub fn shell_command(cwd: &std::path::Path, argv: &[String]) -> String {
    format!("cd {} && {}", shell_quote(&cwd.to_string_lossy()), shell_join(argv))
}

/// Run a backend CLI (bounded by [`crate::process::LOCAL`]) and return stdout, failing with its stderr.
pub(crate) fn run(program: &str, args: &[&str]) -> Result<String> {
    let out = crate::process::output(program, args, None, crate::process::LOCAL)?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.lines().filter(|l| !l.contains("codesign")).collect::<Vec<_>>().join("\n");
        crate::bail!("{program} {} 실패: {}", args.join(" "), err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
