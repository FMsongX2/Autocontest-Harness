//! Clean up a finished contest: close its session and move the local folder to the Trash, but only
//! when the submission is in the ledger and everything is committed and pushed (nothing is lost;
//! the repository can be cloned again). Contract: `docs/CONTRACT.md` (harness clean).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::ledger::{self, Ledger};
use crate::paths;
use crate::registry::{Entry, HistoryItem};
use crate::text::now;
use crate::workspaces;

const CLEANABLE: &[&str] = &["submitted", "closed"];

#[derive(Debug, Default, Serialize)]
pub struct Outcome {
    pub id: String,
    pub ok: bool,
    pub problems: Vec<String>,
    pub actions: Vec<String>,
    pub trash: Option<String>,
}

#[derive(Debug, Default)]
pub struct Options {
    pub dry_run: bool,
    /// Delete instead of moving to the Trash.
    pub purge: bool,
    /// Skip the safety checks.
    pub force: bool,
    /// Where trashed folders go (default `~/.Trash`, or `data/trash` when there is none).
    pub trash_dir: Option<PathBuf>,
}

/// Entries `harness clean --all` looks at: finished, with a folder that has not been cleaned yet.
pub fn candidate(entry: &Entry) -> bool {
    CLEANABLE.contains(&entry.state.as_str())
        && entry.folder.is_some()
        && !entry.extra.contains_key("folder_removed_at")
}

fn git(folder: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").arg("-C").arg(folder).args(args).output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(Error(String::from_utf8_lossy(&out.stderr).trim().to_string()))
    }
}

/// Submission files recorded in the registry that live inside `folder`, relative to it.
fn submission_files(entry: &Entry, folder: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for submission in &entry.submissions {
        for file in submission["files"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            let path = Path::new(file);
            let relative = if path.is_absolute() { path.strip_prefix(folder).ok() } else { Some(path) };
            if let Some(rel) = relative.map(|p| p.to_string_lossy().into_owned())
                && !rel.starts_with("..")
                && !files.contains(&rel)
            {
                files.push(rel);
            }
        }
    }
    files
}

/// Why `entry` must not be cleaned yet; empty means it is safe.
pub fn problems(entry: &Entry, ledger: &Ledger) -> Vec<String> {
    let mut out = Vec::new();
    if !CLEANABLE.contains(&entry.state.as_str()) {
        out.push(format!("상태가 {}다(submitted·closed만 정리한다)", entry.state));
    }
    if ledger.get(&entry.id).is_none() {
        out.push("제출 기록부에 이 공모전이 없다(harness submitted add 또는 import)".into());
    }
    let Some(folder) = entry.folder.as_deref().map(Path::new) else {
        out.push("폴더 기록이 없다".into());
        return out;
    };
    if !folder.is_dir() {
        out.push(format!("폴더가 없다: {}", folder.display()));
        return out;
    }
    if !folder.join(".git").exists() {
        out.push("git 저장소가 아니다(지우면 되살릴 수 없다)".into());
        return out;
    }
    if git(folder, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).is_err() {
        out.push("원격 upstream이 없다(push한 적이 없다)".into());
    } else {
        let _ = git(folder, &["fetch", "--quiet"]);
        match git(folder, &["rev-list", "@{u}..HEAD"]) {
            Ok(ahead) if !ahead.trim().is_empty() => {
                out.push(format!("push 안 된 커밋이 {}개 있다", ahead.lines().count()))
            }
            Err(err) => out.push(format!("push 여부를 확인하지 못했다: {err}")),
            Ok(_) => {}
        }
    }
    match git(folder, &["status", "--porcelain"]) {
        Ok(status) if !status.trim().is_empty() => {
            let lines: Vec<&str> = status.lines().collect();
            let shown: Vec<&str> = lines.iter().take(5).map(|l| l.trim()).collect();
            out.push(format!("커밋 안 된 변경이 {}개 있다: {}", lines.len(), shown.join(", ")));
        }
        Err(err) => out.push(format!("git status 실패: {err}")),
        Ok(_) => {}
    }
    for file in submission_files(entry, folder) {
        if git(folder, &["ls-files", "--error-unmatch", "--", &file]).is_err() {
            out.push(format!("제출 파일이 git에 없다(.gitignore로 빠졌을 수 있다): {file}"));
        }
    }
    out
}

fn trash_target(folder: &Path, trash_dir: Option<&Path>) -> PathBuf {
    let dir = match trash_dir {
        Some(dir) => dir.to_path_buf(),
        None => {
            let mac = paths::home().join(".Trash");
            if mac.is_dir() { mac } else { paths::data().join("trash") }
        }
    };
    let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "contest".into());
    let stamp = now().replace([':', '+'], "").replace('T', "-");
    dir.join(format!("{name}-{}", &stamp[..15.min(stamp.len())]))
}

/// Check, then (unless `dry_run` or blocked) close the session, trash the folder and record it in the
/// registry entry and the ledger at `ledger_path`.
pub fn clean(entry: &mut Entry, ledger_path: &Path, opts: &Options) -> Result<Outcome> {
    let ledger = ledger::load_at(ledger_path)?;
    let mut outcome = Outcome { id: entry.id.clone(), problems: problems(entry, &ledger), ..Default::default() };
    if !outcome.problems.is_empty() && !opts.force {
        return Ok(outcome);
    }
    let Some(folder) = entry.folder.as_deref().map(PathBuf::from).filter(|f| f.is_dir()) else {
        crate::bail!("{}: 정리할 폴더가 없다", entry.id);
    };
    let session = entry.session.clone().unwrap_or(Value::Null);
    let backend = session["workspace"].as_str().and_then(|name| workspaces::get(name).ok());
    let handle = session["handle"].as_str().filter(|h| !h.is_empty());
    let workspace_id = session["workspace_id"].as_str().filter(|w| !w.is_empty());
    let target = (!opts.purge).then(|| trash_target(&folder, opts.trash_dir.as_deref()));
    if let (Some(ws), Some(h)) = (backend, handle) {
        outcome.actions.push(format!("세션 닫기: {} {h}", ws.name()));
    }
    if let (Some(ws), Some(id)) = (backend, workspace_id) {
        outcome.actions.push(format!("작업 공간 제거: {} {id}", ws.name()));
    }
    outcome.actions.push(match &target {
        Some(t) => format!("폴더를 휴지통으로: {} → {}", folder.display(), t.display()),
        None => format!("폴더 완전히 삭제: {}", folder.display()),
    });
    outcome.trash = target.as_ref().map(|t| t.display().to_string());
    if opts.dry_run {
        outcome.ok = true;
        return Ok(outcome);
    }

    let repo_url = git(&folder, &["remote", "get-url", "origin"]).ok().map(|u| u.trim().to_string());
    if let (Some(ws), Some(h)) = (backend, handle)
        && let Err(err) = ws.close(h)
    {
        outcome.actions.push(format!("경고: 세션을 닫지 못했다: {err}"));
    }
    if let (Some(ws), Some(id)) = (backend, workspace_id)
        && let Err(err) = ws.remove_workspace(id)
    {
        outcome.actions.push(format!("경고: 작업 공간을 제거하지 못했다: {err}"));
    }
    match &target {
        Some(t) => {
            if let Some(parent) = t.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&folder, t)
                .map_err(|err| Error(format!("폴더를 휴지통으로 옮기지 못했다({}): {err}", t.display())))?;
        }
        None => fs::remove_dir_all(&folder)?,
    }

    let at = now();
    entry.session = None;
    entry.extra.insert("folder_removed_at".into(), json!(at));
    let note = format!("로컬 폴더 정리: {}", outcome.trash.as_deref().unwrap_or("완전히 삭제"));
    entry.history.push(HistoryItem {
        at: at.clone(),
        from: Some(entry.state.clone()),
        to: entry.state.clone(),
        by: "harness clean".into(),
        note,
    });
    let id = entry.id.clone();
    ledger::write_at(ledger_path, |ledger| {
        if let Some(item) = ledger.submissions.iter_mut().find(|s| s.key == id) {
            if let Some(url) = &repo_url {
                item.extra.insert("repo".into(), json!(url));
            }
            item.extra.insert("folder_removed_at".into(), json!(at));
        }
        Ok(())
    })?;
    outcome.ok = true;
    Ok(outcome)
}
