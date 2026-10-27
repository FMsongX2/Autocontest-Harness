//! Reopen and close contest sessions after `ach launch`: `restart` (same work, new process: a preset
//! change or a crashed session), `resume` (a submitted contest's follow-up stage) and `close_idle`
//! (stop a session that waits with nothing to do). Contract: `docs/CONTRACT.md`.
//!
//! Every operation runs under the launch guard, so restarts, resumes, closes and launches never
//! interleave, and stores its result only when the entry is still what it read (state, history
//! length and session record): a concurrent transition makes it fail instead of overwriting. A
//! session runs on the runner and workspace recorded at launch, never on whatever the config says
//! now, and gets the same `ACH_SESSION`/`PATH` environment and `runner_args` as a launch. The
//! runner's saved conversation is restored through its adapter ([`Runner::restore`]); a restart
//! never silently starts over without it.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::budget;
use crate::config::Config;
use crate::error::{Error, Result};
use crate::launch::{self, SESSION_ENV, session_env, session_field};
use crate::registry::{self, Entry};
use crate::runners::{self, Runner, valid_session_id};
use crate::text::now;
use crate::workspaces::{self, Open, Workspace, shell_command};

/// States `restart` reopens: the work goes on, the state and work revision stay.
pub const RESTART_STATES: &[&str] = &["running", "awaiting_approval", "plan_review"];
/// States `resume` moves back to `running` (a real transition: a new work revision).
pub const RESUME_STATES: &[&str] = &["followup_pending", "submitted"];
/// States whose waiting session `close_idle` may stop.
pub const CLOSE_STATES: &[&str] = &["submitted", "followup_pending", "closed", "skipped", "duplicate"];
/// Close attempts per `close_idle` call; the failure is recorded and the command may be run again.
pub const CLOSE_ATTEMPTS: usize = 3;
const CLOSE_RETRY_PAUSE: Duration = Duration::from_secs(1);

/// The turn a restarted session gets after its conversation is restored.
pub const RESTART_MESSAGE: &str = "하네스가 이 세션을 다시 열었다(모델 프리셋 변경이나 세션 재시작). 끊긴 작업이 있으면 마무리하고, 하네스 실행 지시와 .harness 기록대로 이어서 진행해.";
/// The turn a resumed session gets after its conversation is restored.
pub const RESUME_MESSAGE: &str = "제출 뒤 후속 단계로 이 세션을 다시 열었다. 하네스 실행 지시, .harness 기록, 공모전 공지를 다시 확인하고 후속 작업을 이어서 진행해.";
/// The first turn of a resumed contest that has no saved conversation.
pub const FRESH_RESUME_MESSAGE: &str = "첨부한 하네스 실행 지시를 읽어. 이 공모전은 이미 제출했거나 후속 단계를 기다리던 중이다. 폴더의 결과물과 .harness 기록을 먼저 확인하고 후속 작업을 이어서 진행해.";

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Restart,
    Resume,
}

impl Action {
    fn name(self) -> &'static str {
        match self {
            Action::Restart => "restart",
            Action::Resume => "resume",
        }
    }

    fn states(self) -> &'static [&'static str] {
        match self {
            Action::Restart => RESTART_STATES,
            Action::Resume => RESUME_STATES,
        }
    }
}

/// A contest session must not reopen or close sessions: it could stop itself, and stopping waiting
/// sessions is the controller's job.
pub(crate) fn refuse_inside_session(action: &str) -> Result<()> {
    if let Some(id) = std::env::var(SESSION_ENV).ok().filter(|v| !v.is_empty()) {
        crate::bail!(
            "ach {action}은(는) 공모전 세션 안({SESSION_ENV}={id})에서 실행하지 않는다(세션이 자기 자신을 닫을 수 있다). 메인 세션이나 터미널에서 실행한다"
        );
    }
    Ok(())
}

fn session_map(entry: &Entry) -> Map<String, Value> {
    entry.session.as_ref().and_then(Value::as_object).cloned().unwrap_or_default()
}

/// Fails when another command changed the entry since `base` was read.
pub(crate) fn unchanged(current: &Entry, base: &Entry) -> Result<()> {
    if current.state != base.state || current.history.len() != base.history.len() || current.session != base.session {
        crate::bail!(
            "{}: 작업하는 동안 다른 명령이 이 항목을 바꿨다(지금 {}·기록 {}개, 시작할 때 {}·기록 {}개). ach show {}로 확인하고 다시 실행한다",
            current.id,
            current.state,
            current.history.len(),
            base.state,
            base.history.len(),
            current.id
        );
    }
    Ok(())
}

/// Apply `f` to the stored entry when it is still `base`; returns the stored entry.
fn store<T>(base: &Entry, f: impl FnOnce(&mut Entry) -> Result<T>) -> Result<(Entry, T)> {
    registry::write(|reg| {
        let current = reg.get_mut(&base.id)?;
        unchanged(current, base)?;
        let value = f(current)?;
        Ok((current.clone(), value))
    })
}

/// Which saved conversation to restore: an explicit id, the one recorded for this runner, or the
/// only one the runner keeps for the folder. `Ok(None)` means start fresh, which only `resume` may.
fn resolve_restore(
    runner: &dyn Runner,
    entry: &Entry,
    folder: &std::path::Path,
    explicit: Option<&str>,
    fresh_ok: bool,
    warnings: &mut Vec<String>,
) -> Result<Option<(String, &'static str)>> {
    if let Some(id) = explicit {
        if !valid_session_id(id) {
            crate::bail!("--restore는 세션 UUID여야 한다: {id}");
        }
        return Ok(Some((id.to_string(), "explicit")));
    }
    if session_field(entry, "runner") == Some(runner.name())
        && let Some(id) = session_field(entry, "session_id").filter(|id| valid_session_id(id))
    {
        return Ok(Some((id.to_string(), "recorded")));
    }
    match runner.session_records(folder) {
        Some(ids) if ids.len() == 1 => Ok(Some((ids[0].clone(), "discovered"))),
        Some(ids) if ids.len() > 1 => crate::bail!(
            "{}: {} 세션 기록이 {}개라 하나로 정할 수 없다({}). --restore <UUID>로 고른다",
            entry.id,
            runner.name(),
            ids.len(),
            ids.join(", ")
        ),
        Some(_) if fresh_ok => {
            warnings.push("저장된 세션 기록이 없어 실행 지시와 폴더의 결과물로 새 세션을 연다".into());
            Ok(None)
        }
        Some(_) => crate::bail!(
            "{}: 복원할 {} 세션 기록이 없다. 이전 기록을 버리지 않으려고 멈췄다. 기록이 있으면 --restore <UUID>로 지정한다",
            entry.id,
            runner.name()
        ),
        None if fresh_ok => {
            warnings.push(format!(
                "러너 {}는 저장된 세션을 찾지 못한다(복원 미지원). 실행 지시와 폴더의 결과물로 새 세션을 연다",
                runner.name()
            ));
            Ok(None)
        }
        None => crate::bail!(
            "{}: 러너 {}는 저장된 세션을 찾지 못한다. --restore <UUID>로 지정하거나, 복원 없이 다시 띄울 수 없다",
            entry.id,
            runner.name()
        ),
    }
}

/// Everything decided before any session is touched.
struct Plan {
    runner: &'static dyn Runner,
    workspace: &'static dyn Workspace,
    workspace_id: Option<String>,
    folder: PathBuf,
    preset: Option<String>,
    restore: Option<(String, &'static str)>,
    argv: Vec<String>,
    warnings: Vec<String>,
}

fn plan(
    entry: &Entry,
    config: &Config,
    action: Action,
    preset: Option<&str>,
    restore: Option<&str>,
    message: Option<&str>,
    dry_run: bool,
) -> Result<Plan> {
    let Some(folder) = entry.folder.as_deref().filter(|f| !f.is_empty()).map(PathBuf::from) else {
        crate::bail!("{}: 폴더 기록이 없다", entry.id)
    };
    if !folder.is_dir() {
        crate::bail!("{}: 폴더가 없다: {}", entry.id, folder.display());
    }
    let Some(level) = entry.level else { crate::bail!("{}: 레벨이 없다", entry.id) };
    let mut warnings = Vec::new();
    let recorded_runner = session_field(entry, "runner");
    let recorded_workspace = session_field(entry, "workspace");
    let (runner_name, workspace_name) = match (recorded_runner, recorded_workspace, action) {
        (Some(r), Some(w), _) => (r.to_string(), w.to_string()),
        (_, _, Action::Restart) => crate::bail!(
            "{}: 세션 기록(러너·작업 환경)이 없어 다시 띄울 수 없다. 기록 없는 세션은 restart하지 않는다",
            entry.id
        ),
        (r, w, Action::Resume) => {
            warnings.push("세션 기록이 없어 설정의 러너·작업 환경으로 연다".into());
            (r.unwrap_or(config.runner.as_str()).to_string(), w.unwrap_or(config.workspace.as_str()).to_string())
        }
    };
    if runner_name != config.runner {
        warnings.push(format!("기록된 러너 {runner_name}로 연다(설정은 {})", config.runner));
    }
    let runner = runners::get(&runner_name)?;
    let workspace = workspaces::get(&workspace_name)?;
    if workspace.name() == "terminal" {
        crate::bail!("terminal 작업 환경은 공모전 세션에 쓰지 않는다");
    }
    let mut problems = Vec::new();
    if !runner.available() {
        problems.push(format!("러너 {}의 실행 파일({})이 PATH에 없다", runner.name(), runner.binary()));
    }
    if !workspace.available() {
        problems.push(format!("작업 환경 {}의 실행 파일이 PATH에 없다", workspace.name()));
    }
    if !dry_run && let Some(first) = problems.first() {
        crate::bail!("{first}");
    }
    warnings.extend(problems);

    // `--preset` picks a new one; otherwise the preset recorded at launch stays (older records
    // without the field fall back to `preset_by_level`, as the launch did).
    let preset = match (preset, entry.session.as_ref().and_then(|s| s.get("preset"))) {
        (Some(p), _) => launch::pick_preset(runner, config, level, Some(p))?,
        (None, Some(recorded)) if runner.presets() => recorded.as_str().filter(|p| !p.is_empty()).map(str::to_string),
        (None, _) => launch::pick_preset(runner, config, level, None)?,
    };
    let restore = resolve_restore(runner, entry, &folder, restore, action == Action::Resume, &mut warnings)?;
    let first = if restore.is_some() { launch::FIRST_MESSAGE } else { FRESH_RESUME_MESSAGE };
    let mut spec = launch::contest_spec(runner, config, level, &folder, preset.as_deref(), first, dry_run)?;
    let argv = match &restore {
        Some((id, _)) => {
            let message = match (action, message) {
                (Action::Resume, _) => RESUME_MESSAGE,
                (Action::Restart, Some(message)) => message,
                (Action::Restart, None) => RESTART_MESSAGE,
            };
            if !runner.restore_delivers_message() {
                warnings.push(format!(
                    "러너 {}는 복원할 때 메시지를 받지 않는다. 세션은 끊긴 턴을 그대로 이어가며 하네스 안내 메시지는 전달되지 않았다",
                    runner.name()
                ));
            }
            runner.restore(&spec, id, message, workspace.interactive())?
        }
        None => {
            let prompt = launch::render_prompt(entry, runner)?;
            if !dry_run {
                std::fs::create_dir_all(folder.join(".harness"))?;
                std::fs::write(&spec.prompt_file, &prompt)?;
            }
            spec.prompt = Some(prompt);
            if workspace.interactive() { runner.interactive(&spec)? } else { runner.oneshot(&spec)? }
        }
    };
    let workspace_id = (recorded_workspace == Some(workspace.name()))
        .then(|| session_field(entry, "workspace_id").filter(|w| !w.is_empty()).map(str::to_string))
        .flatten();
    Ok(Plan {
        runner,
        workspace,
        workspace_id,
        folder,
        preset,
        restore,
        argv: session_env(Some(&entry.id), argv),
        warnings,
    })
}

/// Measures capacity for the running sessions per level ([`budget::capacity`] outside tests).
pub(crate) type CapacityObserver<'a> = &'a dyn Fn(&Config, &BTreeMap<u8, usize>) -> Value;

/// Free session slots for the entry's level, as `observe` measures them; fails when there are none.
pub(crate) fn check_capacity(
    entry: &Entry,
    config: &Config,
    running: &BTreeMap<u8, usize>,
    observe: CapacityObserver<'_>,
) -> Result<()> {
    let cap = observe(config, running);
    let level = entry.level.map(|l| l.to_string()).unwrap_or_else(|| "-".into());
    let free = cap["slots_by_level"][&level].as_u64().unwrap_or(0);
    if free == 0 {
        crate::bail!(
            "{}: 지금은 레벨 {level} 공모전 세션을 더 띄우지 않는다(진행 중 {running:?}, 레벨별 상한 {:?}, 메모리 압박 {}, 전체 여유 {}). 자리가 나면 다시 실행한다",
            entry.id,
            config.max_running_by_level,
            cap["ram"]["pressure"].as_str().unwrap_or("unknown"),
            cap["slots"]
        );
    }
    Ok(())
}

fn result(entry: &Entry, action: Action, plan: &Plan, dry_run: bool) -> Value {
    json!({
        "id": entry.id,
        "action": action.name(),
        "dry_run": dry_run,
        "state_before": entry.state,
        "state": entry.state,
        "mode": if plan.restore.is_some() { "restore" } else { "fresh" },
        "session_id": plan.restore.as_ref().map(|(id, _)| id),
        "restore_source": plan.restore.as_ref().map(|(_, source)| source),
        "runner": plan.runner.name(),
        "workspace": plan.workspace.name(),
        "workspace_id": plan.workspace_id,
        "preset": plan.preset,
        "closed_handle": session_field(entry, "handle"),
        "handle": null,
        "command": shell_command(&plan.folder, &plan.argv),
        "warnings": plan.warnings,
    })
}

/// Record why a reopen stopped after the old session was closed: `pending` goes, `last_error`
/// (with the step) stays, the state is untouched. Returns the error to report.
fn record_failure(base: &Entry, action: Action, step: &str, err: Error) -> Error {
    let message = err.0.clone();
    let saved = store(base, |entry| {
        let mut session = session_map(entry);
        session.remove("pending");
        session
            .insert("last_error".into(), json!({"action": action.name(), "step": step, "at": now(), "error": message}));
        entry.session = Some(Value::Object(session));
        Ok(())
    });
    let mut text = format!(
        "{}: {step} 단계에서 멈췄다(이전 세션은 닫혔고 상태는 {} 그대로다. ach {} {}로 다시 연다): {err}",
        base.id,
        base.state,
        action.name(),
        base.id
    );
    if let Err(save) = saved {
        text.push_str(&format!(" (실패 기록 저장도 실패: {save})"));
    }
    Error(text)
}

/// Restart or resume `id`: close the recorded session (confirmed), open the restored one in the
/// same workspace, and store it; `resume` also moves the entry to `running`, only once the new
/// session is open.
fn reopen(
    id: &str,
    config: &Config,
    action: Action,
    preset: Option<&str>,
    restore: Option<&str>,
    dry_run: bool,
    observe: CapacityObserver<'_>,
) -> Result<Value> {
    reopen_with(id, config, action, preset, restore, dry_run, observe, Recovery::default())
}

/// What a checkpoint recovery (`runtime::recover`) adds to a restart: it already holds the launch
/// guard, the entry must still be the one it observed, and the restored conversation gets the
/// verified checkpoint as its next turn instead of [`RESTART_MESSAGE`].
#[derive(Default)]
pub(crate) struct Recovery<'a> {
    /// The caller holds the launch guard (taking it again would wait on itself).
    pub guard_held: bool,
    /// The entry as observed: state, history length and session must still match.
    pub expect: Option<&'a Entry>,
    /// The turn the restored session gets.
    pub message: Option<&'a str>,
}

/// [`restart`] for `runtime::recover`: same close-then-open sequence and stale-handle protection,
/// with the caller's guard, expected entry and recovery message. Never fresh, never a resume.
pub(crate) fn restart_for_recovery(id: &str, config: &Config, dry_run: bool, recovery: Recovery<'_>) -> Result<Value> {
    reopen_with(id, config, Action::Restart, None, None, dry_run, &budget::capacity, recovery)
}

#[allow(clippy::too_many_arguments)]
fn reopen_with(
    id: &str,
    config: &Config,
    action: Action,
    preset: Option<&str>,
    restore: Option<&str>,
    dry_run: bool,
    observe: CapacityObserver<'_>,
    recovery: Recovery<'_>,
) -> Result<Value> {
    let _guard = if dry_run || recovery.guard_held {
        None
    } else {
        refuse_inside_session(action.name())?;
        Some(registry::launch_guard()?)
    };
    let (base, running) = registry::read(|reg| Ok((reg.get(id)?.clone(), reg.running_by_level())))?;
    if let Some(expect) = recovery.expect {
        unchanged(&base, expect)?;
    }
    if !action.states().contains(&base.state.as_str()) {
        crate::bail!(
            "{id}: ach {}은(는) {} 상태에서만 쓴다(지금 {})",
            action.name(),
            action.states().join("·"),
            base.state
        );
    }
    // A resumed contest takes a session slot again; a restart keeps the one it has.
    if action == Action::Resume {
        check_capacity(&base, config, &running, observe)?;
    }
    let plan = plan(&base, config, action, preset, restore, recovery.message, dry_run)?;
    let mut out = result(&base, action, &plan, dry_run);
    if dry_run {
        return Ok(out);
    }

    // 1. Under the registry lock, on the entry as it is now (same state, history and session as
    //    planned): stop the old process through its recorded backend, wait until it is gone, and
    //    record that no process runs any more. A refused comparison or a failed close changes
    //    nothing; a concurrent transition can no longer slip in between close and record. A stale
    //    handle with another live terminal in the recorded workspace (perhaps the same agent
    //    reconnected after a backend restart) is refused rather than opening a duplicate.
    let at = now();
    let mut base = store(&base, |entry| {
        let handle = session_field(entry, "handle").filter(|h| !h.is_empty());
        let recorded_ws = session_field(entry, "workspace_id").filter(|w| !w.is_empty());
        if handle.is_some() || recorded_ws.is_some() {
            let old = workspaces::get(session_field(entry, "workspace").unwrap_or(plan.workspace.name()))?;
            workspaces::close_recorded(old, handle, recorded_ws, workspaces::CLOSE_WAIT)
                .map_err(|err| Error(format!("{id}: 이전 세션을 닫지 못해 다시 띄우지 않았다: {err}")))?;
        }
        let mut session = session_map(entry);
        session.insert("workspace".into(), json!(plan.workspace.name()));
        session.insert("workspace_id".into(), json!(plan.workspace_id));
        session.insert("runner".into(), json!(plan.runner.name()));
        session.insert("handle".into(), Value::Null);
        session.insert("pending".into(), json!({"action": action.name(), "at": at, "step": "closed"}));
        entry.session = Some(Value::Object(session));
        Ok(())
    })?
    .0;
    // 2. The workspace: the recorded one, or a new one stored before opening so a failed open
    //    reuses it on the retry.
    let workspace_id = match &plan.workspace_id {
        Some(w) => Some(w.clone()),
        None => match plan.workspace.create(&base, config) {
            Ok(None) => None,
            Ok(Some(created)) => {
                let stored = store(&base, |entry| {
                    let mut session = session_map(entry);
                    session.insert("workspace_id".into(), json!(created));
                    session.insert("pending".into(), json!({"action": action.name(), "at": at, "step": "workspace"}));
                    entry.session = Some(Value::Object(session));
                    Ok(())
                });
                match stored {
                    Ok((entry, ())) => base = entry,
                    Err(err) => return Err(Error(format!("{err} (만든 작업 공간 {created}은(는) 기록하지 못했다)"))),
                }
                Some(created)
            }
            Err(err) => return Err(record_failure(&base, action, "workspace", err)),
        },
    };
    // 3. Open the restored session.
    let opened = plan.workspace.open(&Open {
        cwd: plan.folder.clone(),
        title: &base.id,
        argv: plan.argv.clone(),
        workspace_id: workspace_id.as_deref(),
    });
    let handle = match opened {
        Ok(handle) => handle,
        Err(err) => return Err(record_failure(&base, action, "open", err)),
    };
    // 4. Store the open session; resume moves the entry to `running` only now.
    let stored = store(&base, |entry| {
        let mut session = session_map(entry);
        session.remove("pending");
        session.remove("last_error");
        session.insert("handle".into(), json!(handle));
        session.insert("preset".into(), json!(plan.preset));
        if let Some((sid, source)) = &plan.restore {
            session.insert("session_id".into(), json!(sid));
            session.insert("restore_source".into(), json!(source));
        }
        session.insert("reopened_at".into(), json!(now()));
        if action == Action::Restart {
            let count = session.get("restarts").and_then(Value::as_u64).unwrap_or(0);
            session.insert("restarts".into(), json!(count + 1));
        }
        entry.session = Some(Value::Object(session));
        // A restart that succeeds completes any recovery marked before it (by `ach recover`, or
        // one interrupted earlier and now resolved by hand): the marker moves into the record.
        if action == Action::Restart {
            crate::runtime::complete_recovery(entry, &handle, action.name());
        }
        if action == Action::Resume {
            let note = json!({
                "runner": plan.runner.name(),
                "workspace": plan.workspace.name(),
                "preset": plan.preset,
                "handle": handle,
                "session_id": plan.restore.as_ref().map(|(sid, _)| sid),
                "mode": if plan.restore.is_some() { "restore" } else { "fresh" },
            });
            registry::transition(entry, "running", "ach resume", &note.to_string())?;
        }
        Ok(entry.state.clone())
    });
    match stored {
        Ok((_, state)) => {
            out["state"] = json!(state);
            out["handle"] = json!(handle);
            out["workspace_id"] = json!(workspace_id);
            Ok(out)
        }
        Err(err) => {
            // Someone moved the entry meanwhile: the session just opened must not run unrecorded.
            let stopped = workspaces::close_confirmed(plan.workspace, &handle, workspaces::CLOSE_WAIT);
            let tail = match stopped {
                Ok(()) => "새 세션은 다시 닫았다".to_string(),
                Err(e) => format!("새 세션 {handle}도 닫지 못했다: {e}"),
            };
            Err(Error(format!("{err} ({tail})")))
        }
    }
}

/// `ach restart <id> [--preset P] [--restore UUID] [--dry-run]`: reopen the session of a contest in
/// `running`, `awaiting_approval` or `plan_review` with its saved conversation. The state and the work
/// revision stay; no history item is added (the session record keeps `restarts` and `reopened_at`).
pub fn restart(id: &str, config: &Config, preset: Option<&str>, restore: Option<&str>, dry_run: bool) -> Result<Value> {
    reopen(id, config, Action::Restart, preset, restore, dry_run, &budget::capacity)
}

/// `ach resume <id> [--preset P] [--restore UUID] [--dry-run]`: reopen a `submitted` or
/// `followup_pending` contest for its follow-up stage and move it to `running` once the session is
/// open. Needs a free session slot. Without a saved conversation it starts fresh from the launch
/// instructions and the folder's results.
pub fn resume(id: &str, config: &Config, preset: Option<&str>, restore: Option<&str>, dry_run: bool) -> Result<Value> {
    reopen(id, config, Action::Resume, preset, restore, dry_run, &budget::capacity)
}

/// `ach session-close <id> --expected-state S --expected-history N [--dry-run]`: stop the waiting
/// session of a contest in one of [`CLOSE_STATES`] through its recorded backend. State and history
/// length are compared under the registry lock at the moment of closing, so a contest that moved on
/// is never closed. The handle and workspace are cleared only once the close is confirmed; a
/// failure is recorded (`close_attempts`, `close_error`) and the command can be run again. The
/// runner, preset and saved conversation id stay for a later `resume`.
pub fn close_idle(id: &str, expected_state: &str, expected_history: usize, dry_run: bool) -> Result<Value> {
    if !CLOSE_STATES.contains(&expected_state) {
        crate::bail!("session-close는 {} 상태만 닫는다(--expected-state {expected_state})", CLOSE_STATES.join("·"));
    }
    let _guard = if dry_run {
        None
    } else {
        refuse_inside_session("session-close")?;
        Some(registry::launch_guard()?)
    };
    let expect = |entry: &Entry| -> Result<()> {
        if entry.state != expected_state || entry.history.len() != expected_history {
            crate::bail!(
                "{id}: 항목이 기대와 다르다(지금 {}·기록 {}개, 기대 {expected_state}·기록 {expected_history}개). 닫지 않았다",
                entry.state,
                entry.history.len()
            );
        }
        Ok(())
    };
    let entry = registry::read(|reg| Ok(reg.get(id)?.clone()))?;
    expect(&entry)?;
    let handle = session_field(&entry, "handle").filter(|h| !h.is_empty()).map(str::to_string);
    let workspace_id = session_field(&entry, "workspace_id").filter(|w| !w.is_empty()).map(str::to_string);
    let backend_name = session_field(&entry, "workspace").map(str::to_string);
    let mut out = json!({
        "id": id,
        "state": entry.state,
        "history": entry.history.len(),
        "backend": backend_name,
        "handle": handle,
        "workspace_id": workspace_id,
        "session_id": session_field(&entry, "session_id"),
        "dry_run": dry_run,
        "closed": false,
        "already_closed": handle.is_none() && workspace_id.is_none(),
        "attempts": 0,
    });
    if dry_run || (handle.is_none() && workspace_id.is_none()) {
        return Ok(out);
    }
    let Some(backend) = backend_name.as_deref() else {
        crate::bail!("{id}: 세션 기록에 작업 환경이 없어 닫을 수 없다")
    };
    let backend = workspaces::get(backend)?;

    let outcome = registry::write(|reg| {
        let current = reg.get_mut(id)?;
        expect(current)?;
        if current.session != entry.session {
            crate::bail!("{id}: 세션 기록이 그사이 바뀌었다. 닫지 않았다");
        }
        let mut session = session_map(current);
        // Keep the conversation id for a later resume while the record is still unambiguous.
        if !session.get("session_id").and_then(Value::as_str).is_some_and(valid_session_id)
            && let (Some(runner), Some(folder)) = (
                session.get("runner").and_then(Value::as_str).and_then(|r| runners::get(r).ok()),
                current.folder.as_deref(),
            )
            && let Some(ids) = runner.session_records(std::path::Path::new(folder))
            && ids.len() == 1
        {
            session.insert("session_id".into(), json!(ids[0]));
        }
        let mut attempts = 0;
        let mut error = None;
        for attempt in 1..=CLOSE_ATTEMPTS {
            attempts = attempt;
            // A stale handle with another live terminal in the workspace is refused (it may be the
            // same agent reconnected), and removing the workspace, which stops every terminal in
            // it, waits until no unrecorded terminal is left.
            let closed =
                workspaces::close_recorded(backend, handle.as_deref(), workspace_id.as_deref(), workspaces::CLOSE_WAIT)
                    .and_then(|()| match workspace_id.as_deref() {
                        Some(w) => workspaces::ensure_no_other_terminals(backend, Some(w), handle.as_deref())
                            .and_then(|()| backend.remove_workspace(w)),
                        None => Ok(()),
                    });
            match closed {
                Ok(()) => {
                    error = None;
                    break;
                }
                Err(err) => {
                    error = Some(err.0);
                    if attempt < CLOSE_ATTEMPTS {
                        std::thread::sleep(CLOSE_RETRY_PAUSE);
                    }
                }
            }
        }
        let at = now();
        match &error {
            None => {
                session.insert("handle".into(), Value::Null);
                session.insert("workspace_id".into(), Value::Null);
                session.insert("closed_at".into(), json!(at));
                session.remove("close_error");
                session.remove("close_attempts");
            }
            Some(err) => {
                let total = session.get("close_attempts").and_then(Value::as_u64).unwrap_or(0) + attempts as u64;
                session.insert("close_attempts".into(), json!(total));
                session.insert("close_error".into(), json!({"at": at, "error": err}));
            }
        }
        current.session = Some(Value::Object(session));
        Ok((attempts, error, session_field(current, "session_id").map(str::to_string)))
    })?;
    let (attempts, error, session_id) = outcome;
    out["attempts"] = json!(attempts);
    out["session_id"] = json!(session_id);
    match error {
        None => {
            out["closed"] = json!(true);
            Ok(out)
        }
        Some(err) => Err(Error(format!(
            "{id}: 세션을 {attempts}번 닫으려 했지만 실패했다(세션 기록은 그대로 두었다; 같은 명령으로 다시 시도한다): {err}"
        ))),
    }
}

/// Successful resumes with a fixed capacity observer, so live memory pressure never decides the
/// outcome. Each test runs alone in a child process of this test binary whose `HOME`, `PATH` (a
/// stub `omp` first) and `ACH_HOME` are set on the child's command line, so the real runner,
/// registry and `~/.omp` stay out of reach and this process's environment is never modified.
#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::Duration;

    use super::*;
    use crate::registry::{NewEntry, apply_gate, new_entry};
    use crate::workspaces::headless::Headless;

    const OMP: &str = r#"#!/bin/sh
if [ "$1" = "usage" ]; then echo '{"reports":[]}'; exit 0; fi
mkdir -p .harness
{ echo "session=${ACH_SESSION:-none}"; for a in "$@"; do echo "arg=$a"; done; echo "end"; } >> .harness/omp-calls.log
exec sleep 30
"#;
    const UUID: &str = "01a154fb-8308-706f-adaa-8e7d56e584b2";
    /// Set only in the child: the fixture directory. Its presence stops the child from re-spawning.
    const CHILD: &str = "HARNESS_LIFECYCLE_UNIT_CHILD";

    /// In the parent: run test `name` alone (`--exact --test-threads=1`) in a child of this binary
    /// with its own `HOME`/`PATH`/`ACH_HOME`, assert it passed, and return `None`. In the child:
    /// build the fixture the parent pointed at and return its directory.
    fn isolated(name: &str) -> Option<PathBuf> {
        if let Some(dir) = std::env::var_os(CHILD) {
            return Some(fixture(PathBuf::from(dir)));
        }
        let dir = std::env::temp_dir().join(format!("harness-lifecycle-unit-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = format!("{}:{}", dir.join("bin").display(), std::env::var("PATH").unwrap_or_default());
        let filter = format!("lifecycle::tests::{name}");
        let out = Command::new(std::env::current_exe().unwrap())
            .args([filter.as_str(), "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, &dir)
            .env("HOME", dir.join("home"))
            .env("ACH_HOME", dir.join("ach-home"))
            .env("PATH", path)
            .env_remove(SESSION_ENV)
            .output()
            .unwrap();
        let _ = fs::remove_dir_all(&dir);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "isolated {filter} failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        None
    }

    /// The child's fixture under `dir`: the stub `omp`, an empty omp session store, and the prompts.
    fn fixture(dir: PathBuf) -> PathBuf {
        assert_eq!(std::env::var_os("HOME").map(PathBuf::from), Some(dir.join("home")), "child environment");
        let stubs = dir.join("bin");
        fs::create_dir_all(&stubs).unwrap();
        fs::write(stubs.join("omp"), OMP).unwrap();
        fs::set_permissions(stubs.join("omp"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir_all(dir.join("home/.omp/agent/sessions")).unwrap();
        let prompts = dir.join("ach-home/harness/prompts");
        fs::create_dir_all(&prompts).unwrap();
        for file in fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("harness/prompts")).unwrap() {
            let file = file.unwrap().path();
            fs::copy(&file, prompts.join(file.file_name().unwrap())).unwrap();
        }
        dir
    }

    /// Room for every level, no memory measurement, no pressure.
    fn roomy(config: &Config, running: &BTreeMap<u8, usize>) -> Value {
        budget::capacity_with(config, running, &json!({"reports": []}), None, "normal")
    }

    /// Kernel memory pressure at `warning`: no new sessions.
    fn pressured(config: &Config, running: &BTreeMap<u8, usize>) -> Value {
        budget::capacity_with(config, running, &json!({"reports": []}), None, "warning")
    }

    fn config() -> Config {
        let overlay = json!({"runner": "omp", "workspace": "headless", "runner_args": {"omp": ["--approval-mode", "yolo"]},
            "max_running": 100, "max_running_by_level": {"1": 100, "2": 100, "3": 100, "4": 100, "5": 100}});
        serde_json::from_value(crate::config::merge(crate::config::defaults(), overlay)).unwrap()
    }

    fn stored(env: &Path, id: &str, state: &str, session: Value) -> (Entry, PathBuf) {
        let folder = env.join("contests").join(id);
        fs::create_dir_all(&folder).unwrap();
        let folder = fs::canonicalize(folder).unwrap();
        let mut e = new_entry(NewEntry {
            id,
            title: "단위 시험 공모전",
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
        e.submissions.push(json!({"at": "2026-10-01T10:00+09:00", "method": "폼"}));
        registry::write(|reg| {
            reg.contests.insert(e.id.clone(), e.clone());
            Ok(())
        })
        .unwrap();
        (e, folder)
    }

    fn current(id: &str) -> Entry {
        registry::read(|reg| Ok(reg.get(id)?.clone())).unwrap()
    }

    fn omp_args(folder: &Path) -> Vec<String> {
        let log = folder.join(".harness/omp-calls.log");
        for _ in 0..250 {
            let text = fs::read_to_string(&log).unwrap_or_default();
            if text.contains("end") {
                assert!(text.contains("session="), "{text}");
                return text.lines().filter_map(|l| l.strip_prefix("arg=")).map(str::to_string).collect();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the session never ran the runner: {}", log.display());
    }

    #[test]
    fn resume_opens_the_restored_session_before_moving_to_running() {
        let Some(env) = isolated("resume_opens_the_restored_session_before_moving_to_running") else { return };
        let session = json!({"workspace": "headless", "workspace_id": null, "runner": "omp", "handle": null,
            "session_id": UUID, "preset": null});
        let (before, folder) = stored(&env, "manual-unit-resume", "submitted", session);
        let out = reopen("manual-unit-resume", &config(), Action::Resume, None, None, false, &roomy).unwrap();
        assert_eq!(
            (out["mode"].as_str(), out["restore_source"].as_str()),
            (Some("restore"), Some("recorded")),
            "{out}"
        );
        let argv = omp_args(&folder);
        let log = fs::read_to_string(folder.join(".harness/omp-calls.log")).unwrap();
        assert!(log.contains("session=manual-unit-resume"), "the session keeps ACH_SESSION: {log}");
        assert!(argv.windows(2).any(|w| w[0] == "--restore" && w[1] == UUID), "{argv:?}");
        assert!(argv.windows(2).any(|w| w[0] == "--approval-mode" && w[1] == "yolo"), "{argv:?}");
        assert!(!argv.iter().any(|a| a == "--config"), "the recorded preset (none) stays: {argv:?}");

        let after = current("manual-unit-resume");
        assert_eq!(after.state, "running");
        let last = after.history.last().unwrap();
        assert_eq!(
            (last.from.as_deref(), last.to.as_str(), last.by.as_str()),
            (Some("submitted"), "running", "ach resume")
        );
        assert_eq!(after.history.len(), before.history.len() + 1);
        let session = after.session.unwrap();
        assert_eq!(session["handle"], out["handle"]);
        assert!(session.get("pending").is_none());
        let _ = Headless.close(out["handle"].as_str().unwrap());
    }

    #[test]
    fn resume_without_a_saved_conversation_starts_from_the_instructions() {
        let Some(env) = isolated("resume_without_a_saved_conversation_starts_from_the_instructions") else { return };
        let (_, folder) = stored(
            &env,
            "manual-unit-fresh",
            "followup_pending",
            json!({"workspace": "headless", "runner": "omp", "handle": null}),
        );
        let dry = reopen("manual-unit-fresh", &config(), Action::Resume, None, None, true, &roomy).unwrap();
        assert_eq!(dry["mode"], "fresh");
        assert!(!dry["warnings"].as_array().unwrap().is_empty(), "starting over is reported: {dry}");
        assert!(!folder.join(".harness").exists(), "a dry run writes nothing");
        assert_eq!(current("manual-unit-fresh").state, "followup_pending");

        let out = reopen("manual-unit-fresh", &config(), Action::Resume, None, None, false, &roomy).unwrap();
        assert!(folder.join(".harness/launch-prompt.md").is_file());
        let argv = omp_args(&folder);
        assert!(!argv.iter().any(|a| a == "--restore"));
        assert!(argv.iter().any(|a| a.starts_with('@') && a.ends_with("launch-prompt.md")));
        assert_eq!(current("manual-unit-fresh").state, "running");
        let _ = Headless.close(out["handle"].as_str().unwrap());
    }

    #[test]
    fn memory_pressure_blocks_a_resume_before_the_session_is_touched() {
        let Some(env) = isolated("memory_pressure_blocks_a_resume_before_the_session_is_touched") else { return };
        let session = json!({"workspace": "headless", "runner": "omp", "handle": null, "session_id": UUID});
        let (before, folder) = stored(&env, "manual-unit-pressure", "followup_pending", session.clone());
        let err = reopen("manual-unit-pressure", &config(), Action::Resume, None, None, false, &pressured).unwrap_err();
        assert!(err.0.contains("warning"), "{err}");
        let after = current("manual-unit-pressure");
        assert_eq!((after.state.as_str(), after.history.len()), ("followup_pending", before.history.len()));
        assert_eq!(after.session, Some(session));
        assert!(!folder.join(".harness").exists());
    }
}
