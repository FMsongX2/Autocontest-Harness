use super::{Open, Workspace, run, shell_command};
use crate::config::Config;
use crate::error::Result;
use crate::registry::Entry;
use crate::runners::which;

pub struct Tmux;

/// Every contest is a window of this one session.
pub const SESSION: &str = "contest-harness";

impl Workspace for Tmux {
    fn name(&self) -> &'static str {
        "tmux"
    }

    fn available(&self) -> bool {
        which("tmux").is_some()
    }

    fn interactive(&self) -> bool {
        true
    }

    fn create(&self, _entry: &Entry, _config: &Config) -> Result<Option<String>> {
        Ok(None)
    }

    fn open(&self, spec: &Open<'_>) -> Result<String> {
        let cwd = spec.cwd.to_string_lossy();
        if run("tmux", &["has-session", "-t", SESSION]).is_err() {
            run("tmux", &["new-session", "-d", "-s", SESSION, "-c", &cwd])?;
        }
        let command = shell_command(&spec.cwd, &spec.argv);
        let target = format!("{SESSION}:");
        let out = run(
            "tmux",
            &[
                "new-window",
                "-d",
                "-P",
                "-F",
                "#{session_name}:#{window_id}",
                "-t",
                &target,
                "-n",
                spec.title,
                "-c",
                &cwd,
                &command,
            ],
        )?;
        Ok(out.trim().to_string())
    }

    fn tail(&self, handle: &str, lines: usize) -> Result<String> {
        let start = format!("-{lines}");
        let out = run("tmux", &["capture-pane", "-p", "-t", handle, "-S", &start])?;
        // capture-pane pads the screen with blank rows below the cursor.
        let rows: Vec<&str> = out.trim_end().lines().collect();
        Ok(rows[rows.len().saturating_sub(lines)..].join("\n"))
    }

    fn close(&self, handle: &str) -> Result<()> {
        match run("tmux", &["kill-window", "-t", handle]) {
            // The window, its session or the whole server already ended.
            Err(e) if ["can't find", "no server running"].iter().any(|m| e.0.contains(m)) => Ok(()),
            other => other.map(drop),
        }
    }

    fn remove_workspace(&self, _workspace_id: &str) -> Result<()> {
        Ok(())
    }
}
