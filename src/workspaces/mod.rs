//! Where contest sessions run (the ADE/terminal side). Contract: `docs/CONTRACT.md`.
//! - `orca`: Orca workspaces and terminals (`orca` CLI).
//! - `tmux`: one tmux session (`contest-harness`), one window per contest.
//! - `headless`: a detached background process; no TTY, so the runner's one-shot command is used.
//! - `terminal`: replace the current process (used by `harness chat`).

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

/// Run a backend CLI and return stdout, failing with its stderr.
pub(crate) fn run(program: &str, args: &[&str]) -> Result<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| crate::error::Error(format!("{program} 실행 실패: {e}")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.lines().filter(|l| !l.contains("codesign")).collect::<Vec<_>>().join("\n");
        crate::bail!("{program} {} 실패: {}", args.join(" "), err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
