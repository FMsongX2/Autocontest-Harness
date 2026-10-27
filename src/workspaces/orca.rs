use serde_json::Value;

use super::{Open, Workspace, shell_command};
use crate::config::Config;
use crate::error::Result;
use crate::registry::Entry;
use crate::runners::which;

pub struct Orca;

/// Call `orca <args> --json` and return its `result` (Orca reports failures as `ok: false`).
pub fn orca_json(args: &[&str]) -> Result<Value> {
    let mut argv = args.to_vec();
    argv.push("--json");
    let out = crate::process::output("orca", &argv, None, crate::process::LOCAL)?;
    let joined = args.join(" ");
    let Ok(mut payload) = serde_json::from_slice::<Value>(&out.stdout) else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stderr: Vec<&str> = stderr.lines().filter(|l| !l.contains("codesign")).collect();
        let detail =
            if stderr.is_empty() { String::from_utf8_lossy(&out.stdout).trim().to_string() } else { stderr.join("\n") };
        crate::bail!("orca {joined} 실패: {}", detail.trim());
    };
    if payload.get("ok").and_then(Value::as_bool) == Some(false) {
        crate::bail!("orca {joined} 실패: {}", payload.get("error").unwrap_or(&Value::Null));
    }
    Ok(match payload.get_mut("result") {
        Some(result) => result.take(),
        None => payload,
    })
}

/// A folder-project workspace instance id: `<repoId>::<path>::workspace:<uuid>`.
pub fn is_folder_workspace(id: &str) -> bool {
    let Some((head, uuid)) = id.rsplit_once("::workspace:") else { return false };
    head.contains("::") && uuid.len() == 36 && uuid.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// The `orca` arguments (before `--json`) that forget a folder-project workspace. Orca's folder
/// branch of `worktree rm` stops its terminals and drops the metadata without touching the
/// directory; a git worktree id would delete the checkout, so it is refused.
pub fn remove_workspace_args(workspace_id: &str) -> Result<Vec<String>> {
    if !is_folder_workspace(workspace_id) {
        crate::bail!(
            "폴더 작업 공간(<repo>::<path>::workspace:<uuid>)만 제거한다(실제 폴더를 지우지 않게): {workspace_id}"
        );
    }
    Ok(vec!["worktree".into(), "rm".into(), "--worktree".into(), format!("id:{workspace_id}")])
}

/// The terminals of `workspace_id` in an answer of `orca terminal list --json` (its `result`):
/// `(handle, description)` per terminal. A truncated or malformed list is an error, so nobody
/// decides on a partial view; terminals of other workspaces are skipped.
pub fn live_terminals_in(result: &Value, workspace_id: &str) -> Result<Vec<(String, String)>> {
    if result.get("truncated").and_then(Value::as_bool) != Some(false) {
        crate::bail!("Orca 터미널 목록이 잘렸거나 truncated 값이 없다: 작업 공간 {workspace_id}");
    }
    let Some(terminals) = result.get("terminals").and_then(Value::as_array) else {
        crate::bail!("Orca 터미널 목록에 terminals가 없다: 작업 공간 {workspace_id}")
    };
    let mut out = Vec::new();
    for terminal in terminals {
        // A disconnected terminal keeps its history in Orca but runs nothing.
        if terminal.get("connected").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        if terminal.get("worktreeId").and_then(Value::as_str).is_some_and(|w| w != workspace_id) {
            continue;
        }
        let Some(handle) = terminal.get("handle").and_then(Value::as_str) else {
            crate::bail!("Orca 터미널 목록에 handle 없는 항목이 있다: {terminal}")
        };
        let agent = terminal.get("agentIdentity").and_then(Value::as_str).unwrap_or("셸/알 수 없음");
        out.push((handle.to_string(), format!("{handle}({agent})")));
    }
    Ok(out)
}

/// Orca's error codes for a handle or workspace it no longer knows.
const GONE: &[&str] = &["terminal_handle_stale", "terminal_not_found", "selector_not_found", "worktree_not_found"];

/// Treat "already gone" answers (stale handle, unknown workspace) as success.
fn ignore_gone(result: Result<()>) -> Result<()> {
    match result {
        Err(e) if GONE.iter().any(|c| e.0.contains(c)) => Ok(()),
        other => other,
    }
}

fn field<'a>(result: &'a Value, outer: &str, keys: &[&str]) -> Option<&'a str> {
    let inner = result.get(outer).unwrap_or(result);
    keys.iter()
        .find_map(|k| inner.get(*k).and_then(Value::as_str))
        .or_else(|| keys.iter().find_map(|k| result.get(*k).and_then(Value::as_str)))
}

/// The workspace id in the answer of `orca worktree create`.
pub fn created_worktree_id(result: &Value) -> Result<String> {
    match field(result, "worktree", &["id", "worktreeId"]) {
        Some(id) => Ok(id.to_string()),
        None => {
            let text: String = result.to_string().chars().take(300).collect();
            crate::bail!("Orca 작업 공간 id를 받지 못했다: {text}")
        }
    }
}

/// `--name=<folder name>`: the worktree is named after the contest folder (already restricted to
/// `[A-Za-z0-9._-]` by route), falling back to the id. One argv element, so it is never an option.
pub fn worktree_name(entry: &Entry) -> Result<String> {
    let folder = entry.folder.as_deref().map(std::path::Path::new).and_then(|p| p.file_name()).and_then(|n| n.to_str());
    let name = folder.unwrap_or(&entry.id);
    if name.is_empty()
        || name.starts_with(['-', '.'])
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        crate::bail!("Orca 작업 공간 이름으로 쓸 수 없는 폴더 이름: {name}");
    }
    Ok(format!("--name={name}"))
}

impl Workspace for Orca {
    fn name(&self) -> &'static str {
        "orca"
    }

    fn available(&self) -> bool {
        which("orca").is_some()
    }

    fn interactive(&self) -> bool {
        true
    }

    /// A worktree named after the contest folder inside the contests repository (`orca_repo_path`).
    /// The name comes from the validated folder name, never from the scraped title, and travels as
    /// one `--name=<value>` element so it cannot be parsed as an option.
    fn create(&self, entry: &Entry, config: &Config) -> Result<Option<String>> {
        let repo = format!("path:{}", config.orca_repo_path().display());
        let name = worktree_name(entry)?;
        let comment = format!("harness {}", entry.id);
        let result = orca_json(&["worktree", "create", "--repo", &repo, &name, "--no-parent", "--comment", &comment])?;
        created_worktree_id(&result).map(Some)
    }

    /// Without a workspace id Orca uses the current worktree (the main session from inside Orca).
    fn open(&self, spec: &Open<'_>) -> Result<String> {
        let command = shell_command(&spec.cwd, &spec.argv);
        let worktree = spec.workspace_id.map(|id| format!("id:{id}"));
        let mut args = vec!["terminal", "create"];
        if let Some(worktree) = &worktree {
            args.extend(["--worktree", worktree.as_str()]);
        }
        args.extend(["--title", spec.title, "--command", &command]);
        let result = orca_json(&args)?;
        match field(&result, "terminal", &["handle"]) {
            Some(handle) => Ok(handle.to_string()),
            None => crate::bail!("Orca 터미널 handle을 받지 못했다: {result}"),
        }
    }

    fn tail(&self, handle: &str, lines: usize) -> Result<String> {
        let limit = lines.to_string();
        let result = orca_json(&["terminal", "read", "--terminal", handle, "--limit", &limit])?;
        let tail = result.get("terminal").unwrap_or(&result).get("tail").cloned().unwrap_or(Value::Null);
        Ok(match tail {
            Value::Array(rows) => {
                let rows: Vec<String> =
                    rows.iter().map(|r| r.as_str().map(str::to_string).unwrap_or_else(|| r.to_string())).collect();
                rows[rows.len().saturating_sub(lines)..].join("\n")
            }
            Value::String(text) => text,
            Value::Null => String::new(),
            other => other.to_string(),
        })
    }

    /// Kills the terminal's process and drops its tab; a handle Orca no longer knows is already closed.
    fn close(&self, handle: &str) -> Result<()> {
        ignore_gone(orca_json(&["terminal", "close", "--terminal", handle]).map(drop))
    }

    /// `terminal show` keeps answering for a closed terminal (its history stays); only
    /// `connected: true` means a process still runs. A stale or unknown handle means it is gone.
    fn is_open(&self, handle: &str) -> Result<Option<bool>> {
        match orca_json(&["terminal", "show", "--terminal", handle]) {
            Ok(result) => {
                let terminal = result.get("terminal").unwrap_or(&result);
                Ok(terminal.get("connected").and_then(Value::as_bool))
            }
            Err(e) if GONE.iter().any(|c| e.0.contains(c)) => Ok(Some(false)),
            Err(e) => Err(e),
        }
    }

    /// `terminal list --worktree id:<workspace>`. Every listed terminal counts: Orca does not say
    /// whether a plain shell is idle or runs a command started by hand (`agentIdentity` is only set
    /// for agents it recognises), so none is ignored. An unknown workspace has none.
    fn live_terminals(&self, workspace_id: &str) -> Result<Option<Vec<(String, String)>>> {
        let selector = format!("id:{workspace_id}");
        let result = match orca_json(&["terminal", "list", "--worktree", &selector]) {
            Ok(result) => result,
            Err(e) if GONE.iter().any(|c| e.0.contains(c)) => return Ok(Some(Vec::new())),
            Err(e) => return Err(e),
        };
        live_terminals_in(&result, workspace_id).map(Some)
    }

    /// `worktree rm` on a folder-project workspace (`<repo>::<path>::workspace:<uuid>`) only stops
    /// its terminals and forgets the metadata; on a git worktree it would delete the checkout, so
    /// anything else is refused (the harness trashes contest folders itself).
    fn remove_workspace(&self, workspace_id: &str) -> Result<()> {
        let args = remove_workspace_args(workspace_id)?;
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        ignore_gone(orca_json(&args).map(drop))
    }
}
