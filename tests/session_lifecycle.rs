//! `ach restart`, `ach resume`, `ach session-close`, `ach checkpoint`, `ach reconcile` and the
//! refusals of `ach recover` against stub runners and backends: the session environment and saved
//! conversation survive a restart, capacity is checked before a resume touches anything, stale
//! expectations never close a session, close failures are kept for a retry, unknown or ambiguous
//! runtimes are never taken for missing ones, and recovery refuses anything it cannot verify.
//! Nothing here starts a real runner or touches the real `data/` or `~/.omp`. Successful recoveries
//! (which measure capacity) are unit tests in `src/runtime.rs` with a fixed capacity.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::Duration;

use harness::config::{self, Config};
use harness::lifecycle;
use harness::registry::{self, Entry, HistoryItem, NewEntry, apply_gate, new_entry};
use harness::workspaces::{self, Open, Workspace, headless::Headless};
use harness::{readiness, runtime};
use serde_json::{Value, json};

/// `omp` stub: answers `omp usage` (capacity) with no reports; as a session it appends its
/// environment and argv to `.harness/omp-calls.log` in its working directory and then waits.
const OMP: &str = r#"#!/bin/sh
if [ "$1" = "usage" ]; then echo '{"reports":[]}'; exit 0; fi
mkdir -p .harness
{
  echo "session=${ACH_SESSION:-none}"
  echo "path0=${PATH%%:*}"
  for a in "$@"; do echo "arg=$a"; done
  echo "end"
} >> .harness/omp-calls.log
exec sleep 30
"#;

/// `orca` stub: `terminal close` fails while `FAIL_CLOSE-<handle>` exists next to it and otherwise
/// marks the handle closed. Like real Orca, `terminal show` keeps answering for a closed terminal
/// with `connected: false`, and reports `STALE-<handle>` as an unknown handle; `terminal list`
/// answers with `terminals.json` next to it (a list result) or an empty, complete list. Every call
/// is logged.
const ORCA: &str = r#"#!/bin/sh
here=$(dirname "$0")
echo "$*" >> "$here/orca.log"
case "$1 $2" in
  "terminal close")
    if [ -e "$here/FAIL_CLOSE-$4" ]; then echo '{"ok":false,"error":{"code":"runtime_error","message":"close failed"}}'; exit 0; fi
    touch "$here/closed-$4"; echo '{"ok":true,"result":{}}' ;;
  "terminal show")
    if [ -e "$here/STALE-$4" ]; then echo '{"ok":false,"error":{"code":"terminal_handle_stale"}}';
    elif [ -e "$here/closed-$4" ]; then echo '{"ok":true,"result":{"terminal":{"connected":false}}}';
    else echo '{"ok":true,"result":{"terminal":{"connected":true}}}'; fi ;;
  "terminal list")
    if [ -e "$here/terminals.json" ]; then printf '{"ok":true,"result":%s}\n' "$(cat "$here/terminals.json")"; else echo '{"ok":true,"result":{"terminals":[],"truncated":false}}'; fi ;;
  *) echo '{"ok":true,"result":{}}' ;;
esac
"#;

const UUID_A: &str = "01a13e24-bbc2-764a-b27c-5f3d2d0cb5b6";
const UUID_B: &str = "01a154fb-8308-706f-adaa-8e7d56e584b2";

struct Stubs {
    dir: PathBuf,
    home: PathBuf,
}

/// Stub `omp`/`orca` first on PATH, a temporary `HOME` (omp sessions and presets) and `ACH_HOME`
/// (registry and prompts). Set once, before any test reads the environment.
static STUBS: LazyLock<Stubs> = LazyLock::new(|| {
    let dir = std::env::temp_dir().join(format!("harness-session-stubs-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (name, body) in [("omp", OMP), ("orca", ORCA)] {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let home = dir.join("home");
    let agent = home.join(".omp/agent");
    fs::create_dir_all(agent.join("sessions")).unwrap();
    fs::write(agent.join("config.yml"), "modelPresets:\n  Fast:\n    modelRoles:\n      default: stub/fast\n").unwrap();
    let ach_home = dir.join("ach-home");
    let prompts = ach_home.join("harness/prompts");
    fs::create_dir_all(&prompts).unwrap();
    for file in fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("harness/prompts")).unwrap() {
        let file = file.unwrap().path();
        fs::copy(&file, prompts.join(file.file_name().unwrap())).unwrap();
    }
    let path = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap_or_default());
    // SAFETY: runs once, inside the LazyLock, before any test of this binary reads the environment.
    unsafe {
        std::env::set_var("PATH", path);
        std::env::set_var("HOME", &home);
        std::env::set_var("ACH_HOME", &ach_home);
        std::env::remove_var("ACH_SESSION");
    }
    Stubs { dir, home }
});

/// The tests share one registry, the launch guard and `ACH_SESSION`, so they run one at a time.
fn setup() -> (&'static Stubs, MutexGuard<'static, ()>) {
    static SERIAL: Mutex<()> = Mutex::new(());
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let stubs = &*STUBS;
    fs::write(stubs.dir.join("orca.log"), "").unwrap();
    (stubs, guard)
}

fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "harness-session-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // Canonical, so it compares equal to the `cwd` recorded in session files (macOS /private/var).
    fs::canonicalize(&dir).unwrap()
}

/// Headless omp with runner args and generous count limits. Successful resumes (which measure live
/// memory pressure) are covered by the unit tests in `src/lifecycle.rs` with a fixed capacity.
fn config(overlay: Value) -> Config {
    let mut base = json!({"runner": "omp", "workspace": "headless", "runner_args": {"omp": ["--approval-mode", "yolo"]},
        "max_running": 100, "max_running_by_level": {"1": 100, "2": 100, "3": 100, "4": 100, "5": 100},
        "ram_gb_by_level": {"1": 0.0, "2": 0.0, "3": 0.0, "4": 0.0, "5": 0.0}});
    for (k, v) in overlay.as_object().unwrap() {
        base[k] = v.clone();
    }
    serde_json::from_value(config::merge(config::defaults(), base)).unwrap()
}

/// A level-2 contest in `state` whose folder is `folder`, stored in the registry.
fn stored(id: &str, state: &str, folder: &Path, session: Value) -> Entry {
    let mut e = new_entry(NewEntry {
        id,
        title: "세션 시험 공모전",
        url: &format!("https://example.com/{id}"),
        source: "manual",
        organizer: "주최",
        deadline: None,
        categories: vec![],
        note: None,
    });
    let gate = json!({"decision": "pass", "type": "idea", "level": 2, "priority": 70, "folder": "F",
        "deliverables": [{"name": "제안서", "format": "pdf", "limits": ""}],
        "criteria": [{"stage": "예비", "name": "참신성", "weight": 30}], "reasons": ["근거"]});
    apply_gate(&mut e, gate, "ContestGate").unwrap();
    e.state = state.into();
    e.folder = Some(folder.display().to_string());
    e.session = Some(session);
    if state == "submitted" || state == "followup_pending" {
        e.submissions.push(json!({"at": "2026-10-01T10:00+09:00", "method": "폼"}));
    }
    registry::write(|reg| {
        reg.contests.insert(e.id.clone(), e.clone());
        Ok(())
    })
    .unwrap();
    e
}

fn current(id: &str) -> Entry {
    registry::read(|reg| Ok(reg.get(id)?.clone())).unwrap()
}

/// An omp session file for `cwd` under the stub HOME, as omp writes it (title line, then header).
fn omp_session(stubs: &Stubs, project: &str, uuid: &str, cwd: &Path) {
    let dir = stubs.home.join(".omp/agent/sessions").join(project);
    fs::create_dir_all(&dir).unwrap();
    let title = json!({"type": "title", "v": 1, "title": "t"});
    let header = json!({"type": "session", "version": 3, "id": uuid, "cwd": cwd.display().to_string()});
    fs::write(dir.join(format!("2026-10-15T05-59-17-186Z_{uuid}.jsonl")), format!("{title}\n{header}\n")).unwrap();
}

/// A live headless session (a `sleep`) in `folder`.
fn live_session(folder: &Path, id: &str) -> String {
    Headless
        .open(&Open {
            cwd: folder.to_path_buf(),
            title: id,
            argv: vec!["sleep".into(), "30".into()],
            workspace_id: None,
        })
        .unwrap()
}

/// The stub omp's log once a session has written one complete call.
fn omp_calls(folder: &Path) -> String {
    let log = folder.join(".harness/omp-calls.log");
    for _ in 0..250 {
        let text = fs::read_to_string(&log).unwrap_or_default();
        if text.contains("end") {
            return text;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("the session never ran the runner: {}", log.display());
}

fn args(calls: &str) -> Vec<&str> {
    calls.lines().filter_map(|l| l.strip_prefix("arg=")).collect()
}

fn orca_log(stubs: &Stubs) -> String {
    fs::read_to_string(stubs.dir.join("orca.log")).unwrap_or_default()
}

#[test]
fn restart_restores_the_conversation_with_the_session_environment_preset_and_runner_args() {
    let (stubs, _serial) = setup();
    let folder = temp_dir("restart");
    omp_session(stubs, "-restart-project", UUID_A, &folder);
    // Another folder's session is not a candidate.
    omp_session(stubs, "-elsewhere", UUID_B, &temp_dir("elsewhere"));
    let old = live_session(&folder, "manual-restart");
    let before = stored(
        "manual-restart",
        "running",
        &folder,
        json!({"workspace": "headless", "workspace_id": null, "runner": "omp", "handle": old, "custom": "kept"}),
    );

    let out = lifecycle::restart("manual-restart", &config(json!({})), Some("Fast"), None, false).unwrap();
    assert_eq!(out["mode"], "restore", "{out}");
    assert_eq!(out["session_id"], UUID_A);
    assert_eq!(out["restore_source"], "discovered");
    assert_eq!(out["state"], "running");
    assert_eq!(Headless.is_open(&old).unwrap(), Some(false), "the old session must be stopped first");

    let calls = omp_calls(&folder);
    assert!(calls.contains("session=manual-restart"), "{calls}");
    assert!(calls.contains(&format!("path0={}", harness::paths::bin_dir().display())), "{calls}");
    let argv = args(&calls);
    let restore_at = argv.iter().position(|a| *a == "--restore").expect("restored");
    assert_eq!(argv[restore_at + 1], UUID_A);
    assert!(argv.windows(2).any(|w| w == ["--approval-mode", "yolo"]), "runner args: {argv:?}");
    assert!(argv.contains(&"-p"), "headless sessions run one-shot: {argv:?}");
    let config_at = argv.iter().position(|a| *a == "--config").expect("preset overlay");
    assert!(fs::read_to_string(argv[config_at + 1]).unwrap().contains("stub/fast"));
    assert!(
        argv.windows(2).any(|w| w == ["--model", "stub/fast"]),
        "a restored session switches to the new preset's default model: {argv:?}"
    );
    assert!(!argv.iter().any(|a| a.contains("다시 열었다")), "omp --restore takes no prompt: {argv:?}");
    assert!(out["warnings"].to_string().contains("메시지를 받지 않는다"), "{out}");

    let after = current("manual-restart");
    assert_eq!(after.state, "running");
    assert_eq!(after.history.len(), before.history.len(), "a restart adds no transition");
    let session = after.session.unwrap();
    assert_eq!(session["handle"], out["handle"]);
    assert_eq!(session["session_id"], UUID_A);
    assert_eq!(session["preset"], "Fast");
    assert_eq!(session["restarts"], 1);
    assert_eq!(session["runner"], "omp");
    assert_eq!(session["custom"], "kept", "unknown session fields survive");
    assert!(session.get("pending").is_none());
    let _ = Headless.close(session["handle"].as_str().unwrap());
}

#[test]
fn restart_without_one_clear_record_changes_nothing() {
    let (stubs, _serial) = setup();
    let folder = temp_dir("ambiguous");
    omp_session(stubs, "-ambiguous-1", UUID_A, &folder);
    omp_session(stubs, "-ambiguous-2", UUID_B, &folder);
    let old = live_session(&folder, "manual-ambiguous");
    let session = json!({"workspace": "headless", "workspace_id": null, "runner": "omp", "handle": old});
    let before = stored("manual-ambiguous", "awaiting_approval", &folder, session.clone());
    let cfg = config(json!({}));

    let err = lifecycle::restart("manual-ambiguous", &cfg, None, None, false).unwrap_err();
    assert!(err.0.contains("2개"), "{err}");
    let err = lifecycle::restart("manual-ambiguous", &cfg, None, Some("../../etc/passwd"), false).unwrap_err();
    assert!(err.0.contains("UUID"), "{err}");
    assert_eq!(Headless.is_open(&old).unwrap(), Some(true), "a refused restart leaves the session running");
    let after = current("manual-ambiguous");
    assert_eq!((after.state.as_str(), after.history.len()), ("awaiting_approval", before.history.len()));
    assert_eq!(after.session, Some(session));

    // No saved conversation at all: a restart never starts over silently.
    let empty = temp_dir("no-record");
    stored("manual-norecord", "running", &empty, json!({"workspace": "headless", "runner": "omp", "handle": null}));
    let err = lifecycle::restart("manual-norecord", &cfg, None, None, false).unwrap_err();
    assert!(err.0.contains("기록이 없다"), "{err}");
    assert!(!empty.join(".harness/omp-calls.log").exists());
    let _ = Headless.close(&old);
}

#[test]
fn resume_checks_capacity_before_touching_the_session() {
    let (_stubs, _serial) = setup();
    let folder = temp_dir("capacity");
    let old = live_session(&folder, "manual-full");
    let session = json!({"workspace": "headless", "runner": "omp", "handle": old, "session_id": UUID_A});
    let before = stored("manual-full", "followup_pending", &folder, session.clone());
    let cfg = config(json!({"max_running_by_level": {"2": 0}}));

    let err = lifecycle::resume("manual-full", &cfg, None, None, false).unwrap_err();
    assert!(err.0.contains("더 띄우지 않는다"), "{err}");
    assert_eq!(Headless.is_open(&old).unwrap(), Some(true));
    let after = current("manual-full");
    assert_eq!((after.state.as_str(), after.history.len()), ("followup_pending", before.history.len()));
    assert_eq!(after.session, Some(session));
    let _ = Headless.close(&old);
}

#[test]
fn session_close_refuses_stale_expectations_and_its_own_session() {
    let (stubs, _serial) = setup();
    let folder = temp_dir("stale");
    let session = json!({"workspace": "orca", "workspace_id": null, "runner": "omp", "handle": "term-stale"});
    let e = stored("manual-stale", "submitted", &folder, session.clone());
    let n = e.history.len();

    let err = lifecycle::close_idle("manual-stale", "submitted", n + 1, false).unwrap_err();
    assert!(err.0.contains("기대와 다르다"), "{err}");
    let err = lifecycle::close_idle("manual-stale", "running", n, false).unwrap_err();
    assert!(err.0.contains("session-close"), "{err}");
    // SAFETY: tests of this binary run one at a time (setup's lock).
    unsafe { std::env::set_var("ACH_SESSION", "manual-stale") };
    let inside = lifecycle::close_idle("manual-stale", "submitted", n, false);
    let restart_inside = lifecycle::restart("manual-stale", &config(json!({})), None, None, false);
    unsafe { std::env::remove_var("ACH_SESSION") };
    assert!(inside.unwrap_err().0.contains("ACH_SESSION"));
    assert!(restart_inside.unwrap_err().0.contains("ACH_SESSION"));
    assert!(!orca_log(stubs).contains("--terminal term-stale"), "nothing was closed");
    assert_eq!(current("manual-stale").session, Some(session));
}

#[test]
fn session_close_keeps_the_record_on_failure_and_clears_it_on_a_confirmed_retry() {
    let (stubs, _serial) = setup();
    let folder = temp_dir("retry");
    omp_session(stubs, "-retry-project", UUID_A, &folder);
    let e = stored(
        "manual-retry",
        "followup_pending",
        &folder,
        json!({"workspace": "orca", "workspace_id": "repo::/x::workspace:01a13e24-bbc2-764a-b27c-5f3d2d0cb5b6", "runner": "omp", "handle": "term-retry"}),
    );
    let n = e.history.len();
    let fail = stubs.dir.join("FAIL_CLOSE-term-retry");
    fs::write(&fail, "").unwrap();

    let err = lifecycle::close_idle("manual-retry", "followup_pending", n, false).unwrap_err();
    assert!(err.0.contains(&format!("{}번", lifecycle::CLOSE_ATTEMPTS)), "{err}");
    let session = current("manual-retry").session.unwrap();
    assert_eq!(session["handle"], "term-retry", "an unconfirmed close keeps the handle");
    assert_eq!(session["close_attempts"], lifecycle::CLOSE_ATTEMPTS);
    assert!(session["close_error"]["error"].as_str().unwrap().contains("close failed"));
    assert!(!orca_log(stubs).contains("worktree rm"), "the workspace stays while its terminal runs");

    fs::remove_file(&fail).unwrap();
    let out = lifecycle::close_idle("manual-retry", "followup_pending", n, false).unwrap();
    assert_eq!((out["closed"].as_bool(), out["attempts"].as_u64()), (Some(true), Some(1)), "{out}");
    let after = current("manual-retry");
    assert_eq!((after.state.as_str(), after.history.len()), ("followup_pending", n));
    let session = after.session.unwrap();
    assert_eq!(session["handle"], Value::Null);
    assert_eq!(session["workspace_id"], Value::Null);
    assert_eq!(session["session_id"], UUID_A, "the conversation id stays for a later resume");
    assert_eq!(session["runner"], "omp");
    assert!(session.get("close_error").is_none() && session.get("close_attempts").is_none());
    assert!(orca_log(stubs).contains("worktree rm"));

    // Nothing left to close is not an error.
    let again = lifecycle::close_idle("manual-retry", "followup_pending", n, false).unwrap();
    assert_eq!((again["closed"].as_bool(), again["already_closed"].as_bool()), (Some(false), Some(true)));
}

#[test]
fn a_stale_handle_with_a_live_replacement_terminal_is_neither_reopened_nor_removed() {
    let (stubs, _serial) = setup();
    let folder = temp_dir("replaced");
    let workspace = "repo::/x::workspace:01a154fb-8308-706f-adaa-8e7d56e584b2";
    // The recorded terminal went stale (an Orca restart); the agent came back under a new handle.
    fs::write(stubs.dir.join("closed-term-old"), "").unwrap();
    let live = json!({"terminals": [
        {"handle": "term-new", "connected": true, "writable": true, "agentIdentity": "omp", "worktreeId": workspace},
        {"handle": "term-other-ws", "connected": true, "writable": true, "agentIdentity": "omp", "worktreeId": "repo::/y::workspace:x"}
    ], "truncated": false});
    fs::write(stubs.dir.join("terminals.json"), live.to_string()).unwrap();
    let session = json!({"workspace": "orca", "workspace_id": workspace, "runner": "omp", "handle": "term-old", "session_id": UUID_A});
    let running = stored("manual-replaced", "running", &folder, session.clone());
    let cfg = config(json!({"workspace": "orca"}));

    let err = lifecycle::restart("manual-replaced", &cfg, None, None, false).unwrap_err();
    assert!(err.0.contains("term-new") && !err.0.contains("term-other-ws"), "{err}");
    let after = current("manual-replaced");
    assert_eq!((after.state.as_str(), after.history.len()), ("running", running.history.len()));
    assert_eq!(after.session, Some(session.clone()), "a refused restart records nothing");

    // The same workspace, now waiting after submission: closing it must not remove the workspace.
    let idle = stored("manual-replaced-idle", "submitted", &temp_dir("replaced-idle"), session.clone());
    let err = lifecycle::close_idle("manual-replaced-idle", "submitted", idle.history.len(), false).unwrap_err();
    assert!(err.0.contains("term-new"), "{err}");
    let log = orca_log(stubs);
    assert!(!log.contains("terminal create"), "no duplicate session was opened: {log}");
    assert!(!log.contains("--terminal term-new"), "the unrecorded terminal was not touched: {log}");
    assert!(!log.contains("worktree rm"), "the workspace was not removed: {log}");
    let kept = current("manual-replaced-idle").session.unwrap();
    assert_eq!(kept["handle"], "term-old");
    assert_eq!(kept["workspace_id"], workspace);
    assert!(kept["close_error"]["error"].as_str().unwrap().contains("term-new"));

    // Once the workspace is empty, the stale handle counts as gone and the close completes.
    fs::remove_file(stubs.dir.join("terminals.json")).unwrap();
    let out = lifecycle::close_idle("manual-replaced-idle", "submitted", idle.history.len(), false).unwrap();
    assert_eq!(out["closed"], true, "{out}");
    assert!(orca_log(stubs).contains("worktree rm"));
    let cleared = current("manual-replaced-idle").session.unwrap();
    assert_eq!((cleared["handle"].clone(), cleared["workspace_id"].clone()), (Value::Null, Value::Null));
    fs::remove_file(stubs.dir.join("closed-term-old")).unwrap();
}

#[test]
fn orca_terminal_lists_are_read_completely_or_not_at_all() {
    use harness::workspaces::orca::live_terminals_in;
    let ws = "repo::/x::workspace:a";
    let list = json!({"terminals": [
        {"handle": "t1", "agentIdentity": null, "connected": true, "worktreeId": ws},
        {"handle": "t2", "agentIdentity": "omp", "connected": false, "worktreeId": ws},
        {"handle": "t3", "agentIdentity": "omp", "worktreeId": "repo::/y::workspace:b"}
    ], "truncated": false});
    let found: Vec<String> = live_terminals_in(&list, ws).unwrap().into_iter().map(|(h, _)| h).collect();
    assert_eq!(found, ["t1"], "a plain shell counts; a disconnected terminal runs nothing");
    assert!(live_terminals_in(&json!({"terminals": [], "truncated": true}), ws).is_err());
    assert!(live_terminals_in(&json!({"terminals": []}), ws).is_err());
    assert!(live_terminals_in(&json!({"terminals": [{"agentIdentity": "omp"}], "truncated": false}), ws).is_err());
    assert!(live_terminals_in(&json!({"terminals": [], "truncated": false}), ws).unwrap().is_empty());
}

// ----- checkpoints, reconcile and recovery refusals ------------------------------------------

/// A headless session that ran and is now confirmed gone (the process the handle names is dead).
fn dead_session(folder: &Path, id: &str) -> String {
    let handle = live_session(folder, id);
    workspaces::close_confirmed(&Headless, &handle, Duration::from_secs(10)).unwrap();
    assert_eq!(Headless.is_open(&handle).unwrap(), Some(false));
    handle
}

/// Record a checkpoint over `draft.md` in `folder`, with `extra` fields merged into the request.
fn checkpoint(id: &str, folder: &Path, extra: Value) -> Value {
    fs::write(folder.join("draft.md"), "초안 본문").unwrap();
    let sha = readiness::sha256_file(&folder.join("draft.md")).unwrap();
    let mut raw = json!({"stage": "draft", "completed_artifacts": [{"path": "draft.md", "sha256": sha}],
        "next_action": "3장 근거 보강", "blocked_reason": null, "needs_user_input": false, "next_action_at": null});
    for (k, v) in extra.as_object().unwrap() {
        raw[k] = v.clone();
    }
    registry::write(|reg| runtime::checkpoint(reg.get_mut(id)?, &raw)).unwrap()
}

fn observed(id: &str) -> Value {
    let out = runtime::reconcile(Some(id), false).unwrap();
    assert_eq!(out["entries"].as_array().unwrap().len(), 1, "{out}");
    out["entries"][0].clone()
}

fn refusals(out: &Value) -> Vec<String> {
    out["refusals"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect()
}

#[test]
fn checkpoints_verify_artifacts_refuse_unknown_fields_and_append_history() {
    let (_stubs, _serial) = setup();
    let folder = temp_dir("checkpoint");
    stored("rt-checkpoint", "running", &folder, json!({"workspace": "headless", "runner": "omp", "handle": null}));
    fs::write(folder.join("draft.md"), "초안 본문").unwrap();
    let sha = readiness::sha256_file(&folder.join("draft.md")).unwrap();
    let good =
        json!({"stage": "draft", "completed_artifacts": [{"path": "draft.md", "sha256": sha}], "next_action": "다음"});
    let attempt = |raw: Value| registry::write(|reg| runtime::checkpoint(reg.get_mut("rt-checkpoint")?, &raw));

    let wrong = json!({"stage": "draft", "completed_artifacts": [{"path": "draft.md", "sha256": "0".repeat(64)}], "next_action": "다음"});
    assert!(attempt(wrong).unwrap_err().0.contains("SHA-256"));
    let mut unknown = good.clone();
    unknown["command"] = json!("rm -rf /");
    assert!(attempt(unknown).unwrap_err().0.contains("command"), "unknown fields (and commands) are refused");
    let mut stage = good.clone();
    stage["stage"] = json!("deploy");
    assert!(attempt(stage).is_err());
    let mut empty = good.clone();
    empty["completed_artifacts"] = json!([]);
    assert!(attempt(empty).is_err(), "a completed checkpoint needs artifacts");
    let mut when = good.clone();
    when["next_action_at"] = json!("내일");
    assert!(attempt(when).is_err());
    let mut outside = good.clone();
    outside["completed_artifacts"] = json!([{"path": "../escape.md", "sha256": sha}]);
    assert!(attempt(outside).is_err());
    assert!(current("rt-checkpoint").extra.get("runtime").is_none(), "refusals store nothing");

    let first = attempt(good.clone()).unwrap();
    let second = attempt(good).unwrap();
    assert_eq!((first["checkpoint"]["seq"].as_u64(), second["checkpoint"]["seq"].as_u64()), (Some(1), Some(2)));
    let after = current("rt-checkpoint");
    assert_eq!(after.extra["runtime"]["latest_checkpoint"], 2);
    assert_eq!(after.extra["runtime"]["checkpoints"].as_array().unwrap().len(), 2);
    assert_eq!(after.state, "running", "a checkpoint never changes the workflow state");

    stored("rt-checkpoint-routed", "routed", &folder, json!({}));
    let raw =
        json!({"stage": "draft", "completed_artifacts": [{"path": "draft.md", "sha256": sha}], "next_action": "다음"});
    assert!(registry::write(|reg| runtime::checkpoint(reg.get_mut("rt-checkpoint-routed")?, &raw)).is_err());
}

#[test]
fn reconcile_tells_missing_from_unknown_and_ambiguous_and_applies_only_snapshots() {
    let (stubs, _serial) = setup();
    // Missing: the recorded process is confirmed gone and the backend keeps no other terminal.
    let folder = temp_dir("rt-missing");
    let gone = dead_session(&folder, "rt-missing");
    let session = json!({"workspace": "headless", "runner": "omp", "handle": gone, "session_id": UUID_A});
    let before = stored("rt-missing", "running", &folder, session.clone());
    checkpoint("rt-missing", &folder, json!({}));
    let o = observed("rt-missing");
    assert_eq!(o["runtime_state"], "missing", "{o}");
    assert_eq!((o["checkpoint_valid"].as_bool(), o["recoverable"].as_bool()), (Some(true), Some(true)), "{o}");
    assert_eq!(o["next_action"], "recover_session");
    assert!(o["observation_token"].as_str().unwrap().starts_with("obs-"));
    assert_eq!(observed("rt-missing")["observation_token"], o["observation_token"], "tokens are deterministic");
    assert!(current("rt-missing").extra["runtime"].get("observation").is_none(), "read-only by default");

    // Unknown: the backend cannot read the handle. Unknown is never missing and never recoverable.
    let folder = temp_dir("rt-unknown");
    stored(
        "rt-unknown",
        "running",
        &folder,
        json!({"workspace": "headless", "runner": "omp", "handle": "pid:x:/nowhere", "session_id": UUID_A}),
    );
    checkpoint("rt-unknown", &folder, json!({}));
    let o = observed("rt-unknown");
    assert_eq!((o["runtime_state"].as_str(), o["recoverable"].as_bool()), (Some("unknown"), Some(false)), "{o}");
    assert!(o["blocked_reasons"].as_array().unwrap().contains(&json!("probe_failed")));
    assert_eq!(o["next_action"], "inspect_runtime");
    // No session record for running work is unknown too, not healthy and not missing.
    stored("rt-unrecorded", "running", &temp_dir("rt-unrecorded"), Value::Null);
    let o = observed("rt-unrecorded");
    assert_eq!(o["runtime_state"], "unknown");
    assert!(o["blocked_reasons"].as_array().unwrap().contains(&json!("session_unrecorded")));

    // Ambiguous: the recorded Orca terminal is stale but another live terminal sits in the workspace.
    let ws = "repo::/x::workspace:01a13e24-bbc2-764a-b27c-5f3d2d0cb5b6";
    fs::write(stubs.dir.join("closed-term-rt-old"), "").unwrap();
    let live = json!({"terminals": [{"handle": "term-rt-new", "connected": true, "agentIdentity": "omp", "worktreeId": ws}], "truncated": false});
    fs::write(stubs.dir.join("terminals.json"), live.to_string()).unwrap();
    let orca = json!({"workspace": "orca", "workspace_id": ws, "runner": "omp", "handle": "term-rt-old", "session_id": UUID_A});
    stored("rt-ambiguous", "running", &temp_dir("rt-ambiguous"), orca.clone());
    let o = observed("rt-ambiguous");
    assert_eq!((o["runtime_state"].as_str(), o["recoverable"].as_bool()), (Some("ambiguous"), Some(false)), "{o}");
    let applied = runtime::reconcile(Some("rt-ambiguous"), true).unwrap();
    assert_eq!(applied["changed"], json!(["rt-ambiguous"]));
    let log = orca_log(stubs);
    assert!(
        !log.contains("terminal create") && !log.contains("terminal close"),
        "never adopts, opens or closes: {log}"
    );
    assert_eq!(current("rt-ambiguous").session, Some(orca), "the replacement is not adopted");
    fs::remove_file(stubs.dir.join("terminals.json")).unwrap();
    fs::remove_file(stubs.dir.join("closed-term-rt-old")).unwrap();

    // Apply stores the snapshot only; workflow state, history, session and approvals stay.
    let applied = runtime::reconcile(Some("rt-missing"), true).unwrap();
    assert_eq!((applied["changed"].clone(), applied["conflicts"].clone()), (json!(["rt-missing"]), json!([])));
    let after = current("rt-missing");
    assert_eq!((after.state.as_str(), after.history.len()), ("running", before.history.len()));
    assert_eq!(after.session, Some(session));
    assert!(after.approvals.is_empty());
    assert_eq!(after.extra["runtime"]["observation"]["runtime_state"], "missing");
    let again = runtime::reconcile(Some("rt-missing"), true).unwrap();
    assert_eq!(again["changed"], json!([]), "an unchanged observation is not a change");

    // The pure summary uses the stored snapshot while it is current and says when it is not.
    let summary = runtime::summary(&after);
    assert_eq!(
        (summary["runtime_state"].as_str(), summary["observation"]["status"].as_str()),
        (Some("missing"), Some("current"))
    );
    assert_eq!(summary["checkpoint"]["integrity"], "valid");
    assert_eq!(summary["next_action"], "recover_session");
    let later = chrono::Utc::now().with_timezone(&harness::text::kst()) + chrono::Duration::hours(7);
    let old = runtime::summary_at(&after, later);
    assert_eq!(
        (old["runtime_state"].as_str(), old["observation"]["status"].as_str()),
        (Some("unobserved"), Some("stale"))
    );
    assert_eq!(old["checkpoint"]["integrity"], "unknown", "integrity is not assumed from an old observation");
    let mut moved = after.clone();
    moved.history.push(HistoryItem::new(harness::text::now(), Some("running".into()), "awaiting_approval", "t", ""));
    moved.state = "awaiting_approval".into();
    assert_eq!(runtime::summary(&moved)["observation"]["status"], "stale");
}

#[test]
fn recover_refuses_what_it_cannot_verify_and_never_starts_new_work() {
    let (_stubs, _serial) = setup();
    let cfg = config(json!({"new_work_frozen": true}));
    let dry = |id: &str, token: &str| runtime::recover(id, token, &cfg, true).unwrap();

    // Unknown runtime.
    let folder = temp_dir("rc-unknown");
    stored(
        "rc-unknown",
        "running",
        &folder,
        json!({"workspace": "headless", "runner": "omp", "handle": "pid:x:/nowhere", "session_id": UUID_A}),
    );
    checkpoint("rc-unknown", &folder, json!({}));
    let token = observed("rc-unknown")["observation_token"].as_str().unwrap().to_string();
    let out = dry("rc-unknown", &token);
    assert_eq!(out["recoverable"], false);
    assert!(refusals(&out).contains(&"runtime_unknown".to_string()), "{out}");

    // Missing but no checkpoint.
    let folder = temp_dir("rc-nocheck");
    let gone = dead_session(&folder, "rc-nocheck");
    stored(
        "rc-nocheck",
        "running",
        &folder,
        json!({"workspace": "headless", "runner": "omp", "handle": gone, "session_id": UUID_A}),
    );
    let token = observed("rc-nocheck")["observation_token"].as_str().unwrap().to_string();
    assert!(refusals(&dry("rc-nocheck", &token)).contains(&"checkpoint_missing".to_string()));

    // Missing with a checkpoint whose files changed, and one from an earlier work revision.
    let folder = temp_dir("rc-stale");
    let gone = dead_session(&folder, "rc-stale");
    stored(
        "rc-stale",
        "running",
        &folder,
        json!({"workspace": "headless", "runner": "omp", "handle": gone, "session_id": UUID_A}),
    );
    checkpoint("rc-stale", &folder, json!({}));
    let ok = observed("rc-stale");
    assert_eq!(ok["recoverable"], true, "{ok}");
    fs::write(folder.join("draft.md"), "고친 본문").unwrap();
    let o = observed("rc-stale");
    assert_eq!((o["checkpoint_valid"].as_bool(), o["recoverable"].as_bool()), (Some(false), Some(false)));
    assert_ne!(o["observation_token"], ok["observation_token"]);
    assert!(
        refusals(&dry("rc-stale", o["observation_token"].as_str().unwrap()))
            .contains(&"checkpoint_artifact_changed".to_string())
    );
    fs::write(folder.join("draft.md"), "초안 본문").unwrap();
    registry::write(|reg| {
        let e = reg.get_mut("rc-stale")?;
        e.history.push(HistoryItem::new(
            harness::text::now(),
            Some("awaiting_approval".into()),
            "running",
            "t",
            "재작업",
        ));
        Ok(())
    })
    .unwrap();
    let o = observed("rc-stale");
    let out = dry("rc-stale", o["observation_token"].as_str().unwrap());
    assert!(refusals(&out).contains(&"checkpoint_stale_revision".to_string()), "{out}");
    // An old token is refused even when everything else would pass.
    assert!(
        refusals(&dry("rc-stale", ok["observation_token"].as_str().unwrap()))
            .contains(&"observation_changed".to_string())
    );

    // Questions, blocks and a later due time.
    for (id, extra, code) in [
        ("rc-question", json!({"needs_user_input": true}), "needs_user_input"),
        ("rc-blocked", json!({"blocked_reason": "주최 측 답변 대기"}), "checkpoint_blocked"),
        ("rc-later", json!({"next_action_at": "2999-01-01T09:00:00+09:00"}), "next_action_not_due"),
    ] {
        let folder = temp_dir(id);
        let gone = dead_session(&folder, id);
        stored(
            id,
            "running",
            &folder,
            json!({"workspace": "headless", "runner": "omp", "handle": gone, "session_id": UUID_A}),
        );
        checkpoint(id, &folder, extra);
        let o = observed(id);
        assert_eq!(o["recoverable"], false, "{id}: {o}");
        let out = dry(id, o["observation_token"].as_str().unwrap());
        assert!(refusals(&out).contains(&code.to_string()), "{id}: {out}");
    }

    // A human plan review never progresses on its own.
    let folder = temp_dir("rc-plan");
    let gone = dead_session(&folder, "rc-plan");
    stored(
        "rc-plan",
        "plan_review",
        &folder,
        json!({"workspace": "headless", "runner": "omp", "handle": gone, "session_id": UUID_A}),
    );
    checkpoint("rc-plan", &folder, json!({"stage": "plan"}));
    let o = observed("rc-plan");
    assert_eq!(o["next_action"], "user_plan_review");
    assert!(
        refusals(&dry("rc-plan", o["observation_token"].as_str().unwrap())).contains(&"human_plan_review".to_string())
    );

    // Frozen new work: a routed contest is never started by recovery, dry or real.
    let routed = stored("rc-routed", "routed", &temp_dir("rc-routed"), Value::Null);
    let o = observed("rc-routed");
    assert_eq!(o["runtime_state"], "not_started");
    let token = o["observation_token"].as_str().unwrap();
    assert!(refusals(&dry("rc-routed", token)).contains(&"state_not_recoverable".to_string()));
    let err = runtime::recover("rc-routed", token, &cfg, false).unwrap_err();
    assert!(err.0.contains("state_not_recoverable"), "{err}");
    let after = current("rc-routed");
    assert_eq!(
        (after.state.as_str(), after.history.len(), after.session.clone()),
        ("routed", routed.history.len(), None)
    );

    // A real run with a refusal changes nothing.
    let before = current("rc-question");
    let token = observed("rc-question")["observation_token"].as_str().unwrap().to_string();
    assert!(runtime::recover("rc-question", &token, &cfg, false).unwrap_err().0.contains("needs_user_input"));
    let after = current("rc-question");
    assert_eq!((after.session, after.history.len()), (before.session, before.history.len()));
    assert!(after.extra["runtime"].get("recovery_pending").is_none());
}
