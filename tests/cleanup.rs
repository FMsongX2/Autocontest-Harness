//! Cleanup after a confirmed submission against a stub `orca` and local bare remotes: the scheduled
//! sweep (`ach tick`) cleans what is provably finished, closes only shells proven untouched,
//! keeps every receipt, hash and record, never touches a shell with work in it or an unresolved
//! submission, closes only the session of a contest with a follow-up, and after a restart finishes
//! only the closes that were left owed. Nothing here touches the real `data/`, Orca or `~/.Trash`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};

use chrono::{DateTime, Duration, FixedOffset, Utc};
use harness::clean::{self, Options, PENDING_CLOSE};
use harness::ledger::{self, Ledger};
use harness::readiness::sha256_file;
use harness::registry::{self, Entry, HistoryItem, NewEntry, new_entry};
use harness::workspaces::orca::{idle_default_shell, owner_comment};
use serde_json::{Value, json};

/// `orca` stub over `state.json` next to it: `{workspaces: {id: comment}, terminals: [listed
/// terminal], reads: {handle: read result}}`. `terminal close` marks a handle closed and
/// `worktree rm` a workspace removed; both then drop out of `terminal list`, and `terminal show`
/// answers `connected: false` for them. Every call is logged to `orca.log`.
const ORCA: &str = r#"#!/usr/bin/env python3
import json, os, sys
here = os.path.dirname(os.path.abspath(__file__))
args = sys.argv[1:]
with open(os.path.join(here, 'orca.log'), 'a') as log:
    log.write(' '.join(args) + '\n')
path = os.path.join(here, 'state.json')
state = json.load(open(path)) if os.path.exists(path) else {}
workspaces, terminals, reads = state.get('workspaces', {}), state.get('terminals', []), state.get('reads', {})
def mark(kind, key):
    return os.path.join(here, kind + '-' + key.replace('/', '_').replace(':', '_'))
def gone(t):
    return os.path.exists(mark('closed', t['handle'])) or os.path.exists(mark('removed', t['worktreeId']))
def arg(name):
    return args[args.index(name) + 1] if name in args else ''
def ok(result):
    print(json.dumps({'ok': True, 'result': result}))
def err(code):
    print(json.dumps({'ok': False, 'error': {'code': code}}))
cmd = ' '.join(args[:2])
if cmd == 'worktree show':
    ws = arg('--worktree')[3:]
    if ws in workspaces and not os.path.exists(mark('removed', ws)):
        ok({'worktree': {'id': ws, 'comment': workspaces[ws]}})
    else:
        err('selector_not_found')
elif cmd == 'worktree rm':
    open(mark('removed', arg('--worktree')[3:]), 'w').close()
    ok({})
elif cmd == 'terminal list':
    ws = arg('--worktree')[3:]
    ok({'terminals': [t for t in terminals if t['worktreeId'] == ws and not gone(t)], 'truncated': False})
elif cmd == 'terminal read':
    handle = arg('--terminal')
    ok({'terminal': reads[handle]}) if handle in reads else err('terminal_not_found')
elif cmd == 'terminal show':
    found = [t for t in terminals if t['handle'] == arg('--terminal')]
    ok({'terminal': {'connected': not gone(found[0])}}) if found else err('terminal_handle_stale')
elif cmd == 'terminal close':
    open(mark('closed', arg('--terminal')), 'w').close()
    ok({})
else:
    ok({})
"#;

struct Stubs {
    dir: PathBuf,
}

/// Stub `orca` first on PATH, a temporary `HOME` (no `~/.Trash`, so folders go to `data/trash`)
/// and `ACH_HOME`. Set once, before any test of this binary reads the environment.
static STUBS: LazyLock<Stubs> = LazyLock::new(|| {
    let dir = std::env::temp_dir().join(format!("harness-cleanup-stubs-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("home")).unwrap();
    fs::create_dir_all(dir.join("ach-home/data")).unwrap();
    let orca = dir.join("orca");
    fs::write(&orca, ORCA).unwrap();
    fs::set_permissions(&orca, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap_or_default());
    // SAFETY: runs once, inside the LazyLock, before any test of this binary reads the environment.
    unsafe {
        std::env::set_var("PATH", path);
        std::env::set_var("HOME", dir.join("home"));
        std::env::set_var("ACH_HOME", dir.join("ach-home"));
        std::env::remove_var("ACH_SESSION");
    }
    Stubs { dir }
});

/// The tests share one registry and one stub state, so they run one at a time, each from a clean
/// stub state.
fn setup() -> (&'static Stubs, MutexGuard<'static, ()>) {
    static SERIAL: Mutex<()> = Mutex::new(());
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let stubs = &*STUBS;
    for file in fs::read_dir(&stubs.dir).unwrap() {
        let path = file.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("closed-") || name.starts_with("removed-") || name == "state.json" {
            fs::remove_file(path).unwrap();
        }
    }
    fs::write(stubs.dir.join("orca.log"), "").unwrap();
    // Each test starts from an empty registry and ledger; the sweep looks at every entry.
    let data = harness::paths::data();
    for file in ["registry.json", "submissions.json"] {
        let _ = fs::remove_file(data.join(file));
    }
    (stubs, guard)
}

fn orca_log(stubs: &Stubs) -> String {
    fs::read_to_string(stubs.dir.join("orca.log")).unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn kst_now() -> DateTime<FixedOffset> {
    Utc::now().with_timezone(&harness::text::kst())
}

fn stamp(at: DateTime<FixedOffset>) -> String {
    at.format("%Y-%m-%dT%H:%M%:z").to_string()
}

const RECEIPT: &[u8] = b"receipt screenshot bytes";
const BRIEF_TOP: &str = "Current assignment: individual entry. Report progress to the main session.";

/// One contest folder: committed and pushed, with a receipt kept under `.private/submissions/a1/`
/// (as `ach submit` keeps it) and the harness brief prepended to `CONTEST.md` without a commit.
struct Contest {
    id: String,
    folder: PathBuf,
    workspace: String,
    receipt_sha: String,
}

fn contest(tag: &str) -> Contest {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("harness-cleanup-{tag}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let remote = root.join("remote.git");
    let folder = root.join(format!("Contest-{tag}-{n}"));
    fs::create_dir_all(folder.join("submission")).unwrap();
    git(&root, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git(&folder, &["init", "-q", "-b", "main"]);
    fs::write(folder.join(".gitignore"), ".private/\n").unwrap();
    let committed = "# Contest\n\n| 하네스 id | x |\n";
    fs::write(folder.join("CONTEST.md"), committed).unwrap();
    fs::write(folder.join("submission/final.pdf"), "final").unwrap();
    git(&folder, &["add", "-A"]);
    git(&folder, &["commit", "-q", "-m", "Final"]);
    git(&folder, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&folder, &["push", "-q", "-u", "origin", "main"]);
    let receipt_sha = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(RECEIPT));
    let kept = folder.join(".private/submissions/a1");
    fs::create_dir_all(&kept).unwrap();
    fs::write(kept.join(format!("{receipt_sha}.png")), RECEIPT).unwrap();
    fs::write(folder.join(".private/controller-brief.md"), format!("{BRIEF_TOP}\n\n# 실행 인계: Contest\n")).unwrap();
    fs::write(folder.join("CONTEST.md"), format!("{BRIEF_TOP}\n\n\n---\n\n{committed}")).unwrap();
    let id = format!("manual-cleanup-{tag}-{n}");
    let workspace = format!("repo::{}::workspace:01a13e24-bbc2-764a-b27c-5f3d2d0c{n:04}", root.display());
    Contest { id, folder, workspace, receipt_sha }
}

/// The registry entry of `c` in `state`, `minutes` after it entered that state, with a confirmed
/// receipt (attempt `a1`), readiness that rules out a follow-up, a ledger item and an Orca session.
fn stored(c: &Contest, state: &str, minutes: i64, followup: bool) -> Entry {
    let mut e = new_entry(NewEntry {
        id: &c.id,
        title: "정리 시험 공모전",
        url: &format!("https://example.com/{}", c.id),
        source: "manual",
        organizer: "주최",
        deadline: None,
        categories: vec![],
        note: None,
    });
    e.state = state.into();
    e.folder = Some(c.folder.display().to_string());
    e.session = Some(
        json!({"workspace": "orca", "workspace_id": c.workspace, "runner": "omp", "handle": format!("{}-agent", c.id)}),
    );
    let at = stamp(kst_now() - Duration::minutes(minutes));
    e.history.push(HistoryItem::new(
        at.clone(),
        Some("awaiting_approval".into()),
        "submitted",
        "agent",
        "제출 확인: 시도 a1",
    ));
    if state != "submitted" {
        e.history.push(HistoryItem::new(at.clone(), Some("submitted".into()), state, "agent", "후속 일정"));
    }
    e.extra.insert("submission_readiness".into(), json!({"seq": 1, "followup_required": followup}));
    let proof = json!([{"path": format!(".private/submissions/a1/{}.png", c.receipt_sha), "sha256": c.receipt_sha}]);
    e.extra.insert(
        "submission_txn".into(),
        json!({"attempts": [{"attempt_id": "a1", "status": "confirmed",
            "confirmation": {"receipt": "R-1", "confirmed_at": at, "proof": proof, "review_needed": false}}]}),
    );
    e.submissions.push(json!({"at": at, "method": "폼", "receipt": "R-1", "files": ["submission/final.pdf"],
        "attempt_id": "a1", "proof_sha256": [c.receipt_sha]}));
    registry::write(|reg| {
        reg.contests.insert(e.id.clone(), e.clone());
        Ok(())
    })
    .unwrap();
    let item = ledger::from_submit(&e, "폼", "R-1", &["submission/final.pdf".to_string()], "");
    ledger::write_at(&ledger::path(), |l: &mut Ledger| {
        l.upsert(item);
        Ok(())
    })
    .unwrap();
    e
}

fn current(id: &str) -> Entry {
    registry::read(|reg| Ok(reg.get(id)?.clone())).unwrap()
}

fn terminal(handle: &str, workspace: &str, title: &str, agent: Option<&str>) -> Value {
    json!({"handle": handle, "worktreeId": workspace, "title": title, "connected": true, "agentIdentity": agent})
}

fn read(lines: &[&str]) -> Value {
    json!({"status": "running", "tail": lines, "truncated": false, "limited": false})
}

const PROMPT: &str = "user@host Hackerton %";

/// `state.json` for the stub: each contest's workspace (owned by it), its agent and the given
/// extra terminals with their reads.
fn orca_state(stubs: &Stubs, contests: &[&Contest], extra: &[(Value, Value)]) {
    let mut workspaces = serde_json::Map::new();
    let mut terminals = Vec::new();
    let mut reads = serde_json::Map::new();
    for c in contests {
        workspaces.insert(c.workspace.clone(), json!(owner_comment(&c.id)));
        let agent = format!("{}-agent", c.id);
        terminals.push(terminal(&agent, &c.workspace, "omp - contest", Some("omp")));
        reads.insert(agent, read(&["agent at work"]));
    }
    for (t, r) in extra {
        reads.insert(t["handle"].as_str().unwrap().to_string(), r.clone());
        terminals.push(t.clone());
    }
    let state = json!({"workspaces": workspaces, "terminals": terminals, "reads": reads});
    fs::write(stubs.dir.join("state.json"), state.to_string()).unwrap();
}

fn ids(result: &Value, key: &str) -> Vec<String> {
    result[key].as_array().unwrap().iter().map(|v| v["id"].as_str().unwrap().to_string()).collect()
}

#[test]
fn a_confirmed_submission_is_cleaned_with_its_idle_default_shell_and_keeps_every_receipt() {
    let (stubs, _serial) = setup();
    let c = contest("done");
    let before = stored(&c, "submitted", 45, false);
    let shell = terminal("term-shell-done", &c.workspace, "Terminal 1", None);
    orca_state(stubs, &[&c], &[(shell, read(&[PROMPT, PROMPT]))]);

    let result = clean::sweep(kst_now()).unwrap();
    assert_eq!(ids(&result, "cleaned"), vec![c.id.clone()], "{result}");
    let log = orca_log(stubs);
    assert!(log.contains("terminal close --terminal term-shell-done"), "{log}");
    assert!(log.contains(&format!("terminal close --terminal {}-agent", c.id)), "{log}");
    assert!(log.contains(&format!("worktree rm --worktree id:{}", c.workspace)), "{log}");
    assert!(!c.folder.exists(), "the folder went to the trash");

    // The receipt kept by `ach submit` and the brief-carrying CONTEST.md are in the archive, byte
    // for byte, and every record of the submission is unchanged.
    let archive = harness::paths::data().join("archive").join(&c.id);
    let receipt = archive.join(format!(".private/submissions/a1/{}.png", c.receipt_sha));
    assert_eq!(sha256_file(&receipt).unwrap(), c.receipt_sha);
    assert!(fs::read_to_string(archive.join("CONTEST.md")).unwrap().starts_with(BRIEF_TOP));
    let after = current(&c.id);
    assert_eq!(after.state, "submitted");
    assert_eq!(after.submissions, before.submissions);
    assert_eq!(after.extra["submission_txn"], before.extra["submission_txn"]);
    assert!(after.session.is_none() && after.extra.contains_key("folder_removed_at"));
    let item = ledger::load_at(&ledger::path()).unwrap().get(&c.id).cloned().unwrap();
    assert_eq!(item.receipt, "R-1");
    assert_eq!(item.extra["archive"].as_str().unwrap(), archive.display().to_string());

    // A second sweep finds nothing left to do for it.
    fs::write(stubs.dir.join("orca.log"), "").unwrap();
    let again = clean::sweep(kst_now()).unwrap();
    assert!(ids(&again, "cleaned").is_empty() && ids(&again, "sessions_closed").is_empty(), "{again}");
    assert!(!orca_log(stubs).contains(&c.id), "a finished cleanup is not touched again");
}

#[test]
fn a_shell_with_work_in_it_is_never_closed_and_keeps_the_folder() {
    let (stubs, _serial) = setup();
    let c = contest("busy");
    stored(&c, "submitted", 45, false);
    let busy = terminal("term-busy", &c.workspace, "Terminal 1", None);
    orca_state(stubs, &[&c], &[(busy, read(&[PROMPT, &format!("{PROMPT} npm run dev"), "ready on :3000"]))]);

    let result = clean::sweep(kst_now()).unwrap();
    assert!(ids(&result, "cleaned").is_empty(), "{result}");
    let refused = result["refused"].to_string();
    assert!(refused.contains(&c.id) && refused.contains("term-busy"), "{result}");
    let log = orca_log(stubs);
    assert!(!log.contains("terminal close --terminal term-busy"), "{log}");
    assert!(!log.contains("worktree rm"), "{log}");
    assert!(c.folder.join("submission/final.pdf").exists(), "the folder stays");
    let after = current(&c.id);
    assert!(!after.extra.contains_key("folder_removed_at"));
    assert_eq!(after.session.unwrap()["workspace_id"], c.workspace, "the session record is kept for a retry");
}

#[test]
fn only_untouched_default_shells_in_the_contests_own_workspace_are_idle() {
    let ws = "repo::/x::workspace:a";
    let shell = terminal("t", ws, "Terminal 1", None);
    assert!(idle_default_shell(&shell, &read(&[PROMPT, "", PROMPT])));
    // A command typed, output printed, text in the composer, a lost history, a named tab, an agent.
    assert!(!idle_default_shell(&shell, &read(&[&format!("{PROMPT} ls")])));
    assert!(!idle_default_shell(&shell, &read(&[PROMPT, "file.txt", PROMPT])));
    let mut draft = read(&[PROMPT]);
    draft["draft"] = json!("rm -rf build");
    assert!(!idle_default_shell(&shell, &draft));
    let mut lost = read(&[PROMPT]);
    lost["truncated"] = json!(true);
    assert!(!idle_default_shell(&shell, &lost));
    assert!(!idle_default_shell(&shell, &json!({"status": "exited", "tail": [PROMPT], "truncated": false})));
    assert!(!idle_default_shell(&shell, &json!({"status": "running", "tail": [], "truncated": false})));
    assert!(!idle_default_shell(&terminal("t", ws, "dev server", None), &read(&[PROMPT])));
    assert!(!idle_default_shell(&terminal("t", ws, "Terminal 1", Some("omp")), &read(&[PROMPT])));
}

#[test]
fn a_followup_closes_only_the_session_and_open_or_unknown_submissions_keep_everything() {
    let (stubs, _serial) = setup();
    let follow = contest("follow");
    stored(&follow, "followup_pending", 45, true);
    let unknown = contest("unknown");
    let mut e = stored(&unknown, "submitted", 45, false);
    e.extra["submission_txn"]["attempts"]
        .as_array_mut()
        .unwrap()
        .push(json!({"attempt_id": "a2", "status": "unknown", "unknown": {"reason": "timeout"}}));
    registry::write(|reg| {
        reg.contests.insert(e.id.clone(), e.clone());
        Ok(())
    })
    .unwrap();
    let fresh = contest("fresh");
    stored(&fresh, "submitted", 3, false);
    let follow_shell = terminal("term-shell-follow", &follow.workspace, "Terminal 1", None);
    let unknown_shell = terminal("term-shell-unknown", &unknown.workspace, "Terminal 1", None);
    orca_state(
        stubs,
        &[&follow, &unknown, &fresh],
        &[(follow_shell, read(&[PROMPT])), (unknown_shell, read(&[PROMPT]))],
    );

    let result = clean::sweep(kst_now()).unwrap();
    assert_eq!(ids(&result, "sessions_closed"), vec![follow.id.clone()], "{result}");
    assert!(ids(&result, "cleaned").is_empty(), "{result}");
    let log = orca_log(stubs);
    assert!(log.contains("terminal close --terminal term-shell-follow"), "{log}");
    assert!(log.contains(&format!("worktree rm --worktree id:{}", follow.workspace)), "{log}");
    assert!(follow.folder.join("submission/final.pdf").exists(), "a follow-up keeps its folder");
    let kept = current(&follow.id);
    assert_eq!(kept.state, "followup_pending");
    assert!(!kept.extra.contains_key("folder_removed_at"));
    assert_eq!(kept.session.unwrap()["handle"], Value::Null);

    for (c, what) in [(&unknown, "an unknown attempt"), (&fresh, "a submission inside its grace period")] {
        assert!(!log.contains(&c.id) && !log.contains(&c.workspace), "{what} is not touched: {log}");
        assert!(c.folder.exists(), "{what} keeps its folder");
        assert_eq!(current(&c.id).session.unwrap()["workspace_id"], c.workspace, "{what} keeps its session");
    }
}

#[test]
fn after_a_restart_only_the_close_left_owed_is_finished() {
    let (stubs, _serial) = setup();
    // Cleaned from inside its own session: the record is saved first and the close is owed.
    let own = contest("own");
    stored(&own, "submitted", 45, false);
    let done = contest("done-before");
    stored(&done, "submitted", 45, false);
    let shell = terminal("term-shell-own", &own.workspace, "Terminal 1", None);
    orca_state(stubs, &[&own, &done], &[(shell, read(&[PROMPT]))]);
    let opts = Options { own_session: Some(own.id.clone()), ..Options::default() };
    registry::write(|reg| {
        let entry = reg.get_mut(&own.id)?;
        let outcome = clean::clean(entry, &ledger::path(), &opts)?;
        assert!(outcome.ok, "{outcome:?}");
        Ok(())
    })
    .unwrap();
    let owed = current(&own.id);
    assert!(owed.session.is_none() && owed.extra.contains_key("folder_removed_at"));
    assert_eq!(owed.extra[PENDING_CLOSE]["handle"], format!("{}-agent", own.id));
    let closed_agent = format!("terminal close --terminal {}-agent", own.id);
    assert!(!orca_log(stubs).contains(&closed_agent), "its own terminal is never closed first");

    // The other contest was finished long ago: no session, folder gone.
    registry::write(|reg| {
        let entry = reg.get_mut(&done.id)?;
        entry.session = None;
        entry.extra.insert("folder_removed_at".into(), json!("2026-11-01T00:00+09:00"));
        Ok(())
    })
    .unwrap();
    fs::write(stubs.dir.join("orca.log"), "").unwrap();

    // The process stopped before closing (a restart): the next sweep finishes only that close.
    let result = clean::sweep(kst_now()).unwrap();
    assert_eq!(result["recovered"], json!([{"id": own.id, "closed": true}]), "{result}");
    let log = orca_log(stubs);
    assert!(log.contains(&format!("worktree rm --worktree id:{}", own.workspace)), "{log}");
    assert!(!log.contains(&done.id) && !log.contains(&done.workspace), "the finished contest is untouched: {log}");
    assert!(!current(&own.id).extra.contains_key(PENDING_CLOSE));
    let again = clean::sweep(kst_now()).unwrap();
    assert_eq!(again["recovered"], json!([]), "{again}");
}
