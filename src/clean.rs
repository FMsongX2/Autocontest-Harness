//! Clean up a finished contest: close its session, keep what git does not hold and cannot be rebuilt
//! (untracked submission files and the `.private/` folder) in `data/archive/<id>/`, and move the
//! local folder to the Trash — only when the submission is in the ledger and everything else is
//! committed and pushed (the repository can be cloned again). Contract: `docs/CONTRACT.md` (harness clean).

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::ledger::{self, Ledger};
use crate::paths;
use crate::process;
use crate::registry::{self, Entry, HistoryItem};
use crate::text::now;
use crate::workspaces;

const CLEANABLE: &[&str] = &["submitted", "closed"];

/// Folder for files that must outlive the contest folder but never go to git (receipts, form edit
/// links, copies with personal details). `harness clean` archives it.
pub const PRIVATE_DIR: &str = ".private";

/// What `clean` keeps and what it leaves behind with the folder.
#[derive(Debug, Default, PartialEq)]
pub struct ArchivePlan {
    /// Folder-relative files copied to the archive.
    pub keep: Vec<String>,
    /// Other ignored files (renders, caches, environments) that go with the folder.
    pub dropped: usize,
    pub dropped_bytes: u64,
}

#[derive(Debug, Default, Serialize)]
pub struct Outcome {
    pub id: String,
    pub ok: bool,
    pub problems: Vec<String>,
    pub actions: Vec<String>,
    pub trash: Option<String>,
    /// Where untracked submission files and private notes were kept.
    pub archive: Option<String>,
}

#[derive(Debug, Default)]
pub struct Options {
    pub dry_run: bool,
    /// Delete instead of moving to the Trash.
    pub purge: bool,
    /// Skip the safety checks.
    pub force: bool,
    /// Where trashed folders go (default `~/.Trash` on macOS, the XDG trash elsewhere, or
    /// `data/trash` when there is none).
    pub trash_dir: Option<PathBuf>,
    /// Where kept files go (default `data/archive`); each contest gets `<dir>/<id>/`.
    pub archive_dir: Option<PathBuf>,
}

/// Entries `harness clean --all` looks at: finished, with a folder that has not been cleaned yet.
pub fn candidate(entry: &Entry) -> bool {
    CLEANABLE.contains(&entry.state.as_str())
        && entry.folder.is_some()
        && !entry.extra.contains_key("folder_removed_at")
}

fn git(folder: &Path, args: &[&str]) -> Result<String> {
    let mut argv =
        vec!["-C", folder.to_str().ok_or_else(|| Error(format!("경로가 UTF-8이 아니다: {}", folder.display())))?];
    argv.extend_from_slice(args);
    let out = process::output("git", &argv, None, process::LOCAL)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(Error(String::from_utf8_lossy(&out.stderr).trim().to_string()))
    }
}

/// `file` (absolute, or relative to `folder`) as a folder-relative path made only of normal
/// components; `None` when it names something outside the folder (`..`, another root).
pub fn folder_relative(folder: &Path, file: &str) -> Option<String> {
    let path = Path::new(file);
    let relative = if path.is_absolute() { path.strip_prefix(folder).ok()? } else { path };
    let normal = relative.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    let rel = relative.to_string_lossy().into_owned();
    (normal && !rel.is_empty()).then_some(rel)
}

/// Whether `folder/rel` still resolves inside `folder` once symlinks are followed.
fn resolves_inside(folder: &Path, rel: &str) -> bool {
    match (folder.canonicalize(), folder.join(rel).canonicalize()) {
        (Ok(base), Ok(path)) => path.starts_with(base),
        _ => false,
    }
}

/// Submission files recorded in the registry that live inside `folder`, relative to it.
fn submission_files(entry: &Entry, folder: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for submission in &entry.submissions {
        for file in submission["files"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if let Some(rel) = folder_relative(folder, file)
                && !files.contains(&rel)
            {
                files.push(rel);
            }
        }
    }
    files
}

/// Bring the upstream refs up to date (network, bounded by [`process::NETWORK`]) so `problems` can
/// tell whether everything is pushed. Run it before taking the registry lock. A failure is
/// returned as text; `problems` then compares against the last known upstream.
pub fn fetch(entry: &Entry) -> Option<String> {
    let folder = Path::new(entry.folder.as_deref()?);
    if !folder.join(".git").exists()
        || git(folder, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).is_err()
    {
        return None;
    }
    let path = folder.to_str()?;
    match process::output("git", &["-C", path, "fetch", "--quiet"], None, process::NETWORK) {
        Ok(out) if out.status.success() => None,
        Ok(out) => Some(format!("git fetch 실패: {}", String::from_utf8_lossy(&out.stderr).trim())),
        Err(err) => Some(err.0),
    }
}

/// Why `entry` must not be cleaned yet; empty means it is safe. Local checks only: call [`fetch`]
/// first to compare against the current upstream.
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
        if !tracked(folder, &file) && !folder.join(&file).is_file() {
            out.push(format!("제출 파일이 git에도 폴더에도 없다: {file}"));
        }
    }
    out
}

fn tracked(folder: &Path, file: &str) -> bool {
    git(folder, &["ls-files", "--error-unmatch", "--", file]).is_ok()
}

/// Untracked submission files (they are what was sent) plus everything in `.private/`. Other ignored
/// files are rebuildable byproducts and only counted.
pub fn archive_plan(entry: &Entry, folder: &Path) -> Result<ArchivePlan> {
    let mut plan = ArchivePlan {
        keep: submission_files(entry, folder)
            .into_iter()
            .filter(|f| !tracked(folder, f) && folder.join(f).is_file())
            .collect(),
        ..ArchivePlan::default()
    };
    let ignored = git(folder, &["ls-files", "--others", "--ignored", "--exclude-standard", "-z"])?;
    for file in ignored.split('\0').filter(|f| !f.is_empty()) {
        if plan.keep.iter().any(|k| k == file) {
            continue;
        }
        if Path::new(file).starts_with(PRIVATE_DIR) {
            plan.keep.push(file.to_string());
        } else if let Ok(meta) = fs::metadata(folder.join(file)) {
            plan.dropped += 1;
            plan.dropped_bytes += meta.len();
        }
    }
    // A symlink (or a `..` that slipped through) must not copy something from outside the folder.
    plan.keep.retain(|f| folder_relative(folder, f).is_some() && resolves_inside(folder, f));
    plan.keep.sort();
    Ok(plan)
}

fn archive_target(entry: &Entry, archive_dir: Option<&Path>) -> PathBuf {
    let dir = archive_dir.map(Path::to_path_buf).unwrap_or_else(|| paths::data().join("archive"));
    dir.join(entry.id.replace(['/', '\\'], "_"))
}

fn copy_into(folder: &Path, files: &[String], target: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(target)?;
    fs::set_permissions(target, fs::Permissions::from_mode(0o700))?;
    for file in files {
        if folder_relative(folder, file).is_none() || !resolves_inside(folder, file) {
            crate::bail!("폴더 밖을 가리키는 파일은 보관하지 않는다: {file}");
        }
        let to = target.join(file);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(folder.join(file), &to).map_err(|err| Error(format!("보관하지 못했다({file}): {err}")))?;
    }
    Ok(())
}

/// Where trashed folders go when `--trash-dir` is not given: `~/.Trash` on macOS; elsewhere the
/// XDG trash (`$XDG_DATA_HOME/Trash/files`, default `~/.local/share/Trash/files`) when it exists;
/// otherwise `data/trash`. Pure, so every branch is testable.
pub fn trash_dir_for(macos: bool, home: &Path, xdg_data_home: Option<&Path>, data: &Path) -> PathBuf {
    if macos {
        let mac = home.join(".Trash");
        if mac.is_dir() {
            return mac;
        }
    } else {
        let base =
            xdg_data_home.filter(|p| p.is_absolute()).map_or_else(|| home.join(".local/share"), Path::to_path_buf);
        let files = base.join("Trash").join("files");
        if files.is_dir() {
            return files;
        }
    }
    data.join("trash")
}

fn default_trash_dir() -> PathBuf {
    let xdg = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    trash_dir_for(cfg!(target_os = "macos"), &paths::home(), xdg.as_deref(), &paths::data())
}

fn trash_target(folder: &Path, trash_dir: Option<&Path>) -> PathBuf {
    let dir = trash_dir.map_or_else(default_trash_dir, Path::to_path_buf);
    let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "contest".into());
    let stamp = now().replace([':', '+'], "").replace('T', "-");
    dir.join(format!("{name}-{}", stamp.get(..15).unwrap_or(&stamp)))
}

/// An XDG trash (`…/Trash/files/<name>`) wants `…/Trash/info/<name>.trashinfo` so file managers can
/// show and restore the folder. Other trash locations need nothing.
fn write_trashinfo(original: &Path, target: &Path) -> Result<Option<PathBuf>> {
    let (Some(files), Some(name)) = (target.parent(), target.file_name()) else { return Ok(None) };
    if files.file_name().is_none_or(|n| n != "files") {
        return Ok(None);
    }
    let Some(trash) = files.parent().filter(|t| t.file_name().is_some_and(|n| n == "Trash")) else { return Ok(None) };
    let info_dir = trash.join("info");
    fs::create_dir_all(&info_dir)?;
    let mut encoded = String::new();
    for byte in original.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    let date = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S");
    let info = info_dir.join(format!("{}.trashinfo", name.to_string_lossy()));
    fs::write(&info, format!("[Trash Info]\nPath={encoded}\nDeletionDate={date}\n"))?;
    Ok(Some(info))
}

/// Move `folder` to `target` (the Trash) or delete it when there is no target.
fn remove_folder(folder: &Path, target: Option<&Path>) -> Result<()> {
    let Some(t) = target else { return Ok(fs::remove_dir_all(folder)?) };
    if let Some(parent) = t.parent() {
        fs::create_dir_all(parent)?;
    }
    let info = write_trashinfo(folder, t)?;
    fs::rename(folder, t).map_err(|err| {
        if let Some(info) = &info {
            let _ = fs::remove_file(info);
        }
        Error(format!("폴더를 휴지통으로 옮기지 못했다({}): {err}", t.display()))
    })
}

/// Check, then (unless `dry_run` or blocked) close the session, trash the folder and record it in the
/// registry entry and the ledger at `ledger_path`. The ledger is written before the folder moves and
/// restored when the move fails, so the two never disagree about a folder that still exists.
pub fn clean(entry: &mut Entry, ledger_path: &Path, opts: &Options) -> Result<Outcome> {
    let ledger = ledger::load_at(ledger_path)?;
    let mut outcome = Outcome { id: entry.id.clone(), problems: problems(entry, &ledger), ..Default::default() };
    if !outcome.problems.is_empty() && !opts.force {
        return Ok(outcome);
    }
    let Some(folder) = entry.folder.as_deref().map(PathBuf::from).filter(|f| f.is_dir()) else {
        if !outcome.problems.iter().any(|p| p.starts_with("폴더")) {
            outcome.problems.push("정리할 폴더가 없다".into());
        }
        return Ok(outcome);
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
    let plan = archive_plan(entry, &folder)?;
    let archive = (!plan.keep.is_empty()).then(|| archive_target(entry, opts.archive_dir.as_deref()));
    if let Some(a) = &archive {
        outcome.actions.push(format!("제출본·{PRIVATE_DIR} 파일 {}개 보관 → {}", plan.keep.len(), a.display()));
    }
    if plan.dropped > 0 {
        let place = if opts.purge { "함께 삭제" } else { "휴지통으로" };
        outcome.actions.push(format!(
            "그 밖의 git 제외 파일 {}개({:.1}MB, 렌더·캐시 등)는 보관하지 않음({place})",
            plan.dropped,
            plan.dropped_bytes as f64 / 1_048_576.0
        ));
    }
    outcome.actions.push(match &target {
        Some(t) => format!("폴더를 휴지통으로: {} → {}", folder.display(), t.display()),
        None => format!("폴더 완전히 삭제: {}", folder.display()),
    });
    outcome.trash = target.as_ref().map(|t| t.display().to_string());
    outcome.archive = archive.as_ref().map(|a| a.display().to_string());
    if opts.dry_run {
        outcome.ok = true;
        return Ok(outcome);
    }
    if let Some(a) = &archive {
        copy_into(&folder, &plan.keep, a)?;
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

    let at = now();
    let id = entry.id.clone();
    let mut fields = vec![("folder_removed_at", json!(at))];
    if let Some(url) = &repo_url {
        fields.push(("repo", json!(url)));
    }
    if let Some(a) = &outcome.archive {
        fields.push(("archive", json!(a)));
    }
    let previous = ledger::write_at(ledger_path, |ledger| {
        let Some(item) = ledger.submissions.iter_mut().find(|s| s.key == id) else { return Ok(None) };
        let old: Vec<(&str, Option<Value>)> = fields.iter().map(|(k, _)| (*k, item.extra.get(*k).cloned())).collect();
        for (key, value) in &fields {
            item.extra.insert((*key).into(), value.clone());
        }
        Ok(Some(old))
    })?;
    if let Err(err) = remove_folder(&folder, target.as_deref()) {
        if let Some(old) = previous {
            let restored = ledger::write_at(ledger_path, |ledger| {
                if let Some(item) = ledger.submissions.iter_mut().find(|s| s.key == id) {
                    for (key, value) in old {
                        match value {
                            Some(v) => item.extra.insert(key.into(), v),
                            None => item.extra.remove(key),
                        };
                    }
                }
                Ok(())
            });
            if let Err(e) = restored {
                crate::bail!("{err} (제출 기록부의 정리 기록도 되돌리지 못했다: {e})");
            }
        }
        return Err(err);
    }

    entry.session = None;
    entry.extra.insert("folder_removed_at".into(), json!(at));
    let mut note = format!("로컬 폴더 정리: {}", outcome.trash.as_deref().unwrap_or("완전히 삭제"));
    if let Some(a) = &outcome.archive {
        note.push_str(&format!(" / 보관: {a}"));
    }
    let state = entry.state.clone();
    entry.history.push(HistoryItem::new(at, Some(state.clone()), &state, "harness clean", &note));
    outcome.ok = true;
    Ok(outcome)
}

/// A failed outcome for an entry whose cleaning stopped with an error.
fn failed(id: &str, err: Error) -> Outcome {
    Outcome { id: id.to_string(), problems: vec![err.0], ..Default::default() }
}

/// `harness clean [<id> | --all]` against the registry at `registry_path`. The network part
/// (`git fetch`) runs before the exclusive registry lock is taken; under the lock each entry is
/// re-checked locally and cleaned. One entry's error becomes its failed outcome, so the entries
/// that were cleaned are always saved.
pub fn run(registry_path: &Path, ledger_path: &Path, id: Option<&str>, opts: &Options) -> Result<Vec<Outcome>> {
    let pick = |reg: &registry::Registry| -> Result<Vec<Entry>> {
        Ok(match id {
            Some(id) => vec![reg.get(id)?.clone()],
            None => reg.contests.values().filter(|e| candidate(e)).cloned().collect(),
        })
    };
    let entries = registry::read_at(registry_path, pick)?;
    let fetch_errors: Vec<(String, String)> =
        entries.iter().filter_map(|e| fetch(e).map(|err| (e.id.clone(), err))).collect();
    registry::write_at(registry_path, |reg| {
        let mut outcomes = Vec::new();
        for entry in &entries {
            let mut outcome = match reg.get_mut(&entry.id) {
                // Changed since the read (cleaned by another run, moved on): skip with a reason.
                Ok(current) if id.is_none() && !candidate(current) => {
                    failed(&entry.id, Error("다른 실행이 먼저 정리했거나 상태가 바뀌었다".into()))
                }
                Ok(current) => clean(current, ledger_path, opts).unwrap_or_else(|err| failed(&entry.id, err)),
                Err(err) => failed(&entry.id, err),
            };
            if !outcome.ok
                && let Some((_, err)) = fetch_errors.iter().find(|(e, _)| *e == entry.id)
            {
                outcome.problems.push(err.clone());
            }
            outcomes.push(outcome);
        }
        Ok(outcomes)
    })
}
