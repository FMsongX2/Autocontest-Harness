//! The runtime of contest work, kept apart from its workflow state. `Entry.state` stays the
//! persisted workflow state (never renamed or rewritten here); everything this module records lives
//! in `Entry.extra.runtime`:
//! - `checkpoints` (append-only) and `latest_checkpoint`: what a session verified it completed
//!   ([`checkpoint`]), with artifact hashes checked against the folder when recorded;
//! - `observation`: the last applied [`reconcile`] snapshot (runtime state, integrity results and the
//!   entry basis it was taken on);
//! - `recovery_pending` / `recoveries`: [`recover`] marks a recovery before it reopens anything and
//!   the successful restart moves the marker into the record (`lifecycle`).
//!
//! Runtime states: `running`, `missing` (confirmed gone), `ambiguous` (the recorded session is gone
//! but another live terminal may be the same work), `unknown` (the backend could not tell; never
//! treated as missing), `not_started`, `stopped`. [`summary`] is pure (stored data only) for the
//! board and alerts and adds `unobserved` when no current observation exists. Every reason is a
//! stable code; checkpoint prose (next action, blocked reason) never leaves the registry except in
//! the recovery turn of the contest's own session.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, FixedOffset, Utc};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::launch::session_field;
use crate::lifecycle::{self, CapacityObserver, Recovery};
use crate::readiness;
use crate::registry::{self, Entry, IN_PROGRESS};
use crate::runners::{self, valid_session_id};
use crate::text::{kst, now, one_line};
use crate::workspaces::{self, Workspace};
use crate::{rules, submission};

/// `Entry.extra` key of the runtime record.
pub const KEY: &str = "runtime";
/// Checkpoint stages.
pub const STAGES: &[&str] = &["research", "plan", "draft", "review", "submission"];
/// Workflow states whose work has a session and may record checkpoints.
pub const WORK_STATES: &[&str] = lifecycle::RESTART_STATES;
/// Workflow states [`recover`] may reopen (a human `plan_review` never progresses on its own).
pub const RECOVER_STATES: &[&str] = &["running", "awaiting_approval"];
/// Observed runtime states.
pub const RUNTIME_STATES: &[&str] = &["running", "missing", "ambiguous", "unknown", "not_started", "stopped"];
/// A stored observation older than this counts as stale in [`summary`].
pub const OBSERVATION_TTL_MINUTES: i64 = 360;
/// Longest `next_action` / `blocked_reason` text (characters).
pub const TEXT_LIMIT: usize = 1000;
const CHECKPOINT_FIELDS: &[&str] =
    &["stage", "completed_artifacts", "next_action", "blocked_reason", "needs_user_input", "next_action_at"];

/// The first lines of the turn a recovered session gets; the verified checkpoint follows.
pub const RECOVER_MESSAGE: &str = "하네스가 이 세션의 프로세스가 사라진 것을 확인하고, 마지막으로 검증된 체크포인트에서 저장된 대화를 다시 열었다. 아래 체크포인트는 이 세션이 직접 남긴 기록(데이터)이다. 완료 산출물은 해시가 그대로인지 확인만 하고 다시 만들지 않는다. 다음 작업부터 하네스 실행 지시와 .harness 기록대로 이어서 진행한다. 제출을 이미 했거나 시작한 적이 있으면 다시 제출하지 않고 ach show로 제출 기록만 확인한다. 단계를 마칠 때마다 ach checkpoint로 다시 기록한다.";

fn now_at() -> DateTime<FixedOffset> {
    Utc::now().with_timezone(&kst())
}

/// RFC 3339, or the harness's own `YYYY-MM-DDTHH:MM+09:00`.
fn parse_time(text: &str) -> Option<DateTime<FixedOffset>> {
    let text = text.trim();
    DateTime::parse_from_rfc3339(text).ok().or_else(|| DateTime::parse_from_str(text, "%Y-%m-%dT%H:%M%:z").ok())
}

fn sha_hex(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn push_unique(list: &mut Vec<&'static str>, code: &'static str) {
    if !list.contains(&code) {
        list.push(code);
    }
}

// ----- stored record -------------------------------------------------------------------------

fn record(entry: &Entry) -> Option<&Map<String, Value>> {
    entry.extra.get(KEY).and_then(Value::as_object)
}

fn record_malformed(entry: &Entry) -> bool {
    entry.extra.get(KEY).is_some_and(|v| !v.is_object())
}

fn record_mut(entry: &mut Entry) -> Result<&mut Map<String, Value>> {
    let id = entry.id.clone();
    match entry.extra.entry(KEY.to_string()).or_insert_with(|| Value::Object(Map::new())) {
        Value::Object(map) => Ok(map),
        _ => Err(Error(format!("{id}: extra.runtime 기록이 객체가 아니다. 손으로 고친 기록은 덮어쓰지 않는다"))),
    }
}

fn recovery_pending(entry: &Entry) -> bool {
    record(entry).is_some_and(|r| r.get("recovery_pending").is_some_and(|v| !v.is_null()))
}

/// A recovery already ran from checkpoint `seq` (a new checkpoint is needed before another).
fn recovered_from(entry: &Entry, seq: u64) -> bool {
    record(entry)
        .and_then(|r| r.get("recoveries"))
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|r| r.get("checkpoint_seq").and_then(Value::as_u64) == Some(seq)))
}

/// One stored checkpoint, read back strictly.
struct Checkpoint<'a> {
    seq: u64,
    revision: u64,
    stage: &'a str,
    recorded_at: &'a str,
    artifacts: &'a Value,
    next_action: &'a str,
    blocked: bool,
    needs_user_input: bool,
    next_action_at: Option<(DateTime<FixedOffset>, &'a str)>,
}

enum Latest<'a> {
    None,
    Invalid,
    Found(Checkpoint<'a>),
}

fn parse_checkpoint(value: &Value) -> Option<Checkpoint<'_>> {
    let o = value.as_object()?;
    let blocked = match o.get("blocked_reason") {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) if !s.is_empty() => true,
        _ => return None,
    };
    let next_action_at = match o.get("next_action_at") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some((parse_time(s)?, s.as_str())),
        _ => return None,
    };
    Some(Checkpoint {
        seq: o.get("seq")?.as_u64()?,
        revision: o.get("revision")?.as_u64()?,
        stage: o.get("stage")?.as_str().filter(|s| STAGES.contains(s))?,
        recorded_at: o.get("recorded_at")?.as_str()?,
        artifacts: o.get("completed_artifacts").filter(|a| a.as_array().is_some_and(|a| !a.is_empty()))?,
        next_action: o.get("next_action")?.as_str().filter(|s| !s.is_empty())?,
        blocked,
        needs_user_input: o.get("needs_user_input")?.as_bool()?,
        next_action_at,
    })
}

fn latest(entry: &Entry) -> Latest<'_> {
    if record_malformed(entry) {
        return Latest::Invalid;
    }
    let Some(rec) = record(entry) else { return Latest::None };
    let list = rec.get("checkpoints");
    let Some(pointer) = rec.get("latest_checkpoint").filter(|v| !v.is_null()) else {
        return match list {
            None | Some(Value::Null) => Latest::None,
            Some(Value::Array(items)) if items.is_empty() => Latest::None,
            Some(_) => Latest::Invalid,
        };
    };
    let Some(seq) = pointer.as_u64() else { return Latest::Invalid };
    let Some(item) = list
        .and_then(Value::as_array)
        .and_then(|a| a.iter().rev().find(|c| c.get("seq").and_then(Value::as_u64) == Some(seq)))
    else {
        return Latest::Invalid;
    };
    parse_checkpoint(item).map_or(Latest::Invalid, Latest::Found)
}

// ----- checkpoint ----------------------------------------------------------------------------

fn text_field(id: &str, obj: &Map<String, Value>, key: &str) -> Result<Option<String>> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let text = one_line(s);
            if text.is_empty() {
                crate::bail!("{id}: {key}가 비어 있다(없으면 null로 둔다)");
            }
            if text.chars().count() > TEXT_LIMIT {
                crate::bail!("{id}: {key}는 {TEXT_LIMIT}자 이하여야 한다");
            }
            Ok(Some(text))
        }
        Some(_) => crate::bail!("{id}: {key}는 문자열이어야 한다"),
    }
}

/// Record a checkpoint (`ach checkpoint <id> --file F`) on an entry in [`WORK_STATES`] and return
/// `{id, checkpoint}`. `raw`: `stage` (one of [`STAGES`]), non-empty `completed_artifacts
/// [{path, sha256}]` that must hash as given now, `next_action` (text), optional `blocked_reason`
/// (text or null), `needs_user_input` (bool, default false) and `next_action_at` (RFC 3339 or
/// null). Unknown fields are refused; nothing in it is ever run. The record gets the next sequence
/// number, the work revision, workflow state and history length, and becomes the latest checkpoint.
pub fn checkpoint(entry: &mut Entry, raw: &Value) -> Result<Value> {
    let id = entry.id.clone();
    if !WORK_STATES.contains(&entry.state.as_str()) {
        crate::bail!("{id}: 체크포인트는 {} 상태에서만 기록한다(지금 {})", WORK_STATES.join("·"), entry.state);
    }
    if record_malformed(entry) {
        crate::bail!("{id}: extra.runtime 기록이 객체가 아니다. 손으로 고친 기록은 덮어쓰지 않는다");
    }
    let Some(obj) = raw.as_object() else { crate::bail!("{id}: 체크포인트는 JSON 객체여야 한다") };
    if let Some(key) = obj.keys().find(|k| !CHECKPOINT_FIELDS.contains(&k.as_str())) {
        crate::bail!("{id}: 체크포인트에 모르는 필드가 있다: {key} (가능: {})", CHECKPOINT_FIELDS.join(", "));
    }
    let stage = match obj.get("stage").and_then(Value::as_str) {
        Some(stage) if STAGES.contains(&stage) => stage.to_string(),
        _ => crate::bail!("{id}: stage는 {} 중 하나여야 한다", STAGES.join("·")),
    };
    let artifacts = readiness::normalize_artifacts(entry, obj.get("completed_artifacts").unwrap_or(&Value::Null))
        .map_err(|err| Error(format!("{err} (completed_artifacts)")))?;
    let Some(next_action) = text_field(&id, obj, "next_action")? else {
        crate::bail!("{id}: next_action은 비어 있지 않은 문자열이어야 한다")
    };
    let blocked_reason = text_field(&id, obj, "blocked_reason")?;
    let needs_user_input = match obj.get("needs_user_input") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => crate::bail!("{id}: needs_user_input은 true/false여야 한다"),
    };
    let next_action_at = match obj.get("next_action_at") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if parse_time(s).is_some() => Some(s.trim().to_string()),
        Some(_) => {
            crate::bail!("{id}: next_action_at은 RFC 3339 시각(예: 2026-10-25T09:00:00+09:00)이나 null이어야 한다")
        }
    };
    let revision = registry::revision(entry);
    let history_len = entry.history.len();
    let state = entry.state.clone();
    let rec = record_mut(entry)?;
    let previous = match rec.get("checkpoints") {
        None | Some(Value::Null) => 0,
        Some(Value::Array(items)) => {
            items.iter().filter_map(|c| c.get("seq").and_then(Value::as_u64)).max().unwrap_or(0)
        }
        Some(_) => crate::bail!("{id}: extra.runtime.checkpoints가 배열이 아니다. 손으로 고친 기록은 덮어쓰지 않는다"),
    };
    let seq = previous.max(rec.get("latest_checkpoint").and_then(Value::as_u64).unwrap_or(0)) + 1;
    let at = now();
    let checkpoint = json!({
        "seq": seq,
        "recorded_at": at,
        "observed_at": at,
        "stage": stage,
        "revision": revision,
        "workflow_state": state,
        "history_len": history_len,
        "completed_artifacts": artifacts,
        "next_action": next_action,
        "blocked_reason": blocked_reason,
        "needs_user_input": needs_user_input,
        "next_action_at": next_action_at,
    });
    match rec.entry("checkpoints").or_insert_with(|| Value::Array(Vec::new())) {
        Value::Array(items) => items.push(checkpoint.clone()),
        slot => *slot = Value::Array(vec![checkpoint.clone()]),
    }
    rec.insert("latest_checkpoint".into(), json!(seq));
    Ok(json!({"id": id, "checkpoint": checkpoint}))
}

// ----- probing -------------------------------------------------------------------------------

struct Probe {
    state: &'static str,
    codes: Vec<&'static str>,
}

fn probe_of(state: &'static str, codes: &[&'static str]) -> Probe {
    Probe { state, codes: codes.to_vec() }
}

/// Live terminals in `workspace_id` other than `except`. `Ok(None)` when the backend keeps no
/// inventory (or no workspace is recorded); a list that cannot be read completely is an error.
fn inventory(backend: &dyn Workspace, workspace_id: Option<&str>, except: Option<&str>) -> Result<Option<Vec<String>>> {
    let Some(ws) = workspace_id else { return Ok(None) };
    Ok(backend
        .live_terminals(ws)?
        .map(|list| list.into_iter().filter(|(h, _)| Some(h.as_str()) != except).map(|(h, _)| h).collect()))
}

fn ever_ran(entry: &Entry) -> bool {
    entry.history.iter().any(|h| h.to == "running")
}

/// Observe the recorded session through its recorded backend only. Never adopts, opens or closes.
fn probe(entry: &Entry) -> Probe {
    let work = WORK_STATES.contains(&entry.state.as_str());
    let handle = session_field(entry, "handle").filter(|h| !h.is_empty());
    let workspace_id = session_field(entry, "workspace_id").filter(|w| !w.is_empty());
    let pending = entry.session.as_ref().and_then(|s| s.get("pending")).is_some_and(|p| !p.is_null());
    let Some(name) = session_field(entry, "workspace") else {
        return match (work, ever_ran(entry)) {
            (true, _) => probe_of("unknown", &["session_unrecorded"]),
            (false, false) => probe_of("not_started", &[]),
            (false, true) => probe_of("stopped", &[]),
        };
    };
    if !work && handle.is_none() && workspace_id.is_none() && !pending {
        return probe_of(if ever_ran(entry) { "stopped" } else { "not_started" }, &[]);
    }
    let Ok(backend) = workspaces::get(name) else { return probe_of("unknown", &["backend_unknown"]) };
    if !backend.available() {
        return probe_of("unknown", &["backend_unavailable"]);
    }
    match handle {
        Some(h) => match backend.is_open(h) {
            Ok(Some(true)) => match inventory(backend, workspace_id, Some(h)) {
                Ok(Some(others)) if !others.is_empty() => probe_of("running", &["unrecorded_live_terminal"]),
                Ok(_) => probe_of("running", &[]),
                Err(_) => probe_of("running", &["inventory_failed"]),
            },
            Ok(Some(false)) => match inventory(backend, workspace_id, Some(h)) {
                Ok(Some(others)) if !others.is_empty() => probe_of("ambiguous", &["unrecorded_live_terminal"]),
                Ok(_) => probe_of("missing", &[]),
                Err(_) => probe_of("unknown", &["inventory_failed"]),
            },
            Ok(None) => probe_of("unknown", &["backend_cannot_tell"]),
            Err(_) => probe_of("unknown", &["probe_failed"]),
        },
        None => match inventory(backend, workspace_id, None) {
            Ok(Some(others)) if !others.is_empty() => probe_of("ambiguous", &["unrecorded_live_terminal"]),
            Ok(Some(_)) if work => probe_of("missing", &[]),
            Ok(Some(_)) => probe_of("stopped", &[]),
            // Without an inventory, an interrupted reopen may have started a process nobody recorded.
            Ok(None) if pending => probe_of("unknown", &["reopen_interrupted"]),
            Ok(None) if work => probe_of("missing", &[]),
            Ok(None) => probe_of("stopped", &[]),
            Err(_) => probe_of("unknown", &["inventory_failed"]),
        },
    }
}

/// The saved conversation a restart would restore must be known and unambiguous (the same rule as
/// `lifecycle`): a recorded UUID for the recorded runner, or exactly one record for the folder.
fn conversation_problem(entry: &Entry) -> Option<&'static str> {
    let Some(runner) = session_field(entry, "runner").and_then(|r| runners::get(r).ok()) else {
        return Some("saved_conversation_unknown");
    };
    if session_field(entry, "session_id").is_some_and(valid_session_id) {
        return None;
    }
    let Some(folder) = entry.folder.as_deref().filter(|f| !f.is_empty()) else {
        return Some("saved_conversation_unknown");
    };
    match runner.session_records(std::path::Path::new(folder)) {
        Some(ids) if ids.len() == 1 => None,
        Some(ids) if ids.len() > 1 => Some("saved_conversation_ambiguous"),
        _ => Some("saved_conversation_unknown"),
    }
}

// ----- shared decisions ----------------------------------------------------------------------

/// Blockers of the work itself (pure): the current checkpoint's block or question, the current
/// readiness record's question or blockers, an unresolved submission, pending rule changes and an
/// interrupted recovery.
fn work_blockers(entry: &Entry, current: Option<&Checkpoint<'_>>) -> Vec<&'static str> {
    let mut out = Vec::new();
    let work = WORK_STATES.contains(&entry.state.as_str());
    if let Some(cp) = current {
        if cp.blocked {
            push_unique(&mut out, "checkpoint_blocked");
        }
        if cp.needs_user_input {
            push_unique(&mut out, "needs_user_input");
        }
    }
    let revision = registry::revision(entry) as u64;
    if work
        && let Some(r) =
            readiness::latest(entry).filter(|r| r.get("revision").and_then(Value::as_u64) == Some(revision))
    {
        if r.get("needs_user_input").and_then(Value::as_bool) == Some(true) {
            push_unique(&mut out, "readiness_needs_user_input");
        }
        if r.get("blockers").and_then(Value::as_array).is_some_and(|b| !b.is_empty()) {
            push_unique(&mut out, "readiness_blockers");
        }
    }
    if submission::unresolved(entry) {
        push_unique(&mut out, "submission_unresolved");
    }
    if work {
        for code in rules::approval_problems(entry) {
            push_unique(&mut out, code);
        }
    }
    if recovery_pending(entry) {
        push_unique(&mut out, "recovery_interrupted");
    }
    out
}

/// The checkpoint of the current work revision, when it reads back.
fn current_checkpoint(entry: &Entry) -> Option<Checkpoint<'_>> {
    match latest(entry) {
        Latest::Found(cp) if cp.revision == registry::revision(entry) as u64 => Some(cp),
        _ => None,
    }
}

/// Checkpoint problems that do not need the files: none, unreadable, or from another revision.
fn checkpoint_record_problem(entry: &Entry) -> Option<&'static str> {
    match latest(entry) {
        Latest::None => Some("checkpoint_missing"),
        Latest::Invalid => Some("checkpoint_record_invalid"),
        Latest::Found(cp) if cp.revision != registry::revision(entry) as u64 => Some("checkpoint_stale_revision"),
        Latest::Found(_) => None,
    }
}

struct Facts<'a> {
    state: &'a str,
    runtime: &'a str,
    blockers: &'a [&'static str],
    needs_user_input: bool,
    due: bool,
    recoverable: bool,
    approval: &'a str,
}

/// The next action as a stable code.
fn decide(f: &Facts<'_>) -> &'static str {
    let has = |code: &str| f.blockers.contains(&code);
    let work = WORK_STATES.contains(&f.state);
    if has("submission_unresolved") {
        return "resolve_submission";
    }
    if f.needs_user_input || has("checkpoint_blocked") || has("readiness_needs_user_input") || has("readiness_blockers")
    {
        return "user_input";
    }
    if f.state == "plan_review" {
        return "user_plan_review";
    }
    if work && (has("recovery_interrupted") || matches!(f.runtime, "unknown" | "ambiguous")) {
        return "inspect_runtime";
    }
    if RECOVER_STATES.contains(&f.state) && f.runtime == "missing" {
        return if f.recoverable {
            "recover_session"
        } else if !f.due {
            "wait_until_due"
        } else {
            "manual_restart"
        };
    }
    if f.state == "awaiting_approval" {
        return if f.approval == "valid" { "proceed_submission" } else { "user_submit_approval" };
    }
    if work && f.runtime == "unobserved" {
        return "observe_runtime";
    }
    if f.state == "running" {
        return if f.due { "continue_work" } else { "wait_until_due" };
    }
    "none"
}

/// What a stored observation must match to stay current.
fn basis(entry: &Entry) -> Value {
    let session = entry.session.as_ref().map(|s| sha_hex(&s.to_string()));
    json!({
        "workflow_state": entry.state,
        "history_len": entry.history.len(),
        "session": session,
        "latest_checkpoint": record(entry).and_then(|r| r.get("latest_checkpoint")).cloned().unwrap_or(Value::Null),
        "readiness_seq": readiness::latest(entry).and_then(|r| r.get("seq")).cloned().unwrap_or(Value::Null),
        "scores_len": entry.scores.len(),
        "approvals_len": entry.approvals.len(),
    })
}

// ----- observation ---------------------------------------------------------------------------

struct Observation {
    runtime_state: &'static str,
    probe_codes: Vec<&'static str>,
    checkpoint_seq: Option<u64>,
    checkpoint_valid: bool,
    checkpoint_integrity: &'static str,
    artifact_integrity: &'static str,
    approval_integrity: &'static str,
    conversation: Option<&'static str>,
    blocked_reasons: Vec<&'static str>,
    next_action: &'static str,
    next_action_at: Option<String>,
    needs_user_input: bool,
    recoverable: bool,
    token: String,
    observed_at: String,
    basis: Value,
}

/// Observe `entry` now: probe its recorded backend, verify the latest checkpoint's and readiness
/// record's files, and decide recoverability. Reads the backend and local files; writes nothing.
fn observe(entry: &Entry, at: DateTime<FixedOffset>) -> Observation {
    let probe = probe(entry);
    let work = WORK_STATES.contains(&entry.state.as_str());
    let revision = registry::revision(entry) as u64;
    let (checkpoint_seq, record_problem, files_ok) = match latest(entry) {
        Latest::None => (None, Some("checkpoint_missing"), None),
        Latest::Invalid => (None, Some("checkpoint_record_invalid"), None),
        Latest::Found(cp) => {
            let ok = readiness::check_artifacts(entry, cp.artifacts).is_ok();
            let stale = (cp.revision != revision).then_some("checkpoint_stale_revision");
            (Some(cp.seq), stale, Some(ok))
        }
    };
    let checkpoint_integrity = match (checkpoint_seq, files_ok) {
        (None, _) if record_problem == Some("checkpoint_missing") => "none",
        (None, _) => "invalid",
        (Some(_), Some(true)) => "valid",
        (Some(_), _) => "invalid",
    };
    let checkpoint_valid = record_problem.is_none() && files_ok == Some(true);
    let artifact_integrity = match readiness::latest(entry).and_then(|r| r.get("artifacts")) {
        None => "none",
        Some(files) if readiness::check_artifacts(entry, files).is_ok() => "valid",
        Some(_) => "invalid",
    };
    let approval_integrity = if entry.state == "awaiting_approval" {
        if registry::submit_approved(entry) { "valid" } else { "invalid" }
    } else {
        "not_applicable"
    };
    let current = current_checkpoint(entry);
    let work_codes = work_blockers(entry, current.as_ref());
    let needs_user_input = work_codes.iter().any(|c| matches!(*c, "needs_user_input" | "readiness_needs_user_input"));
    let next_at = current.as_ref().and_then(|cp| cp.next_action_at);
    let due = next_at.is_none_or(|(t, _)| t <= at);
    let conversation = (probe.state == "missing" && RECOVER_STATES.contains(&entry.state.as_str()))
        .then(|| conversation_problem(entry))
        .flatten();

    let mut blocked = probe.codes.clone();
    for code in &work_codes {
        push_unique(&mut blocked, code);
    }
    if work {
        if let Some(code) = record_problem {
            push_unique(&mut blocked, code);
        }
        if files_ok == Some(false) {
            push_unique(&mut blocked, "checkpoint_artifact_changed");
        }
    }
    let mut recoverable = false;
    if probe.state == "missing" && RECOVER_STATES.contains(&entry.state.as_str()) {
        if let Some(code) = conversation {
            push_unique(&mut blocked, code);
        }
        if !due {
            push_unique(&mut blocked, "next_action_not_due");
        }
        if checkpoint_seq.is_some_and(|seq| recovered_from(entry, seq)) {
            push_unique(&mut blocked, "recovery_already_done");
        }
        recoverable = checkpoint_valid && blocked.is_empty();
    }
    let next_action = decide(&Facts {
        state: &entry.state,
        runtime: probe.state,
        blockers: &blocked,
        needs_user_input,
        due,
        recoverable,
        approval: approval_integrity,
    });
    let basis = basis(entry);
    let token_source = json!({
        "id": entry.id,
        "basis": basis,
        "revision": revision,
        "runtime_state": probe.state,
        "probe_codes": probe.codes,
        "checkpoint_seq": checkpoint_seq,
        "checkpoint_valid": checkpoint_valid,
        "artifact_integrity": artifact_integrity,
        "approval_integrity": approval_integrity,
        "conversation": conversation,
        "work": work_codes,
        "recoveries": record(entry).and_then(|r| r.get("recoveries")).and_then(Value::as_array).map_or(0, Vec::len),
    });
    let token = format!("obs-{}", &sha_hex(&token_source.to_string())[..32]);
    Observation {
        runtime_state: probe.state,
        probe_codes: probe.codes,
        checkpoint_seq,
        checkpoint_valid,
        checkpoint_integrity,
        artifact_integrity,
        approval_integrity,
        conversation,
        blocked_reasons: blocked,
        next_action,
        next_action_at: next_at.map(|(_, text)| text.to_string()),
        needs_user_input,
        recoverable,
        token,
        observed_at: at.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        basis,
    }
}

fn observation_json(entry: &Entry, o: &Observation) -> Value {
    json!({
        "id": entry.id,
        "workflow_state": entry.state,
        "runtime_state": o.runtime_state,
        "observed_at": o.observed_at,
        "observation_token": o.token,
        "checkpoint_valid": o.checkpoint_valid,
        "checkpoint_seq": o.checkpoint_seq,
        "checkpoint_integrity": o.checkpoint_integrity,
        "artifact_integrity": o.artifact_integrity,
        "approval_integrity": o.approval_integrity,
        "blocked_reasons": o.blocked_reasons,
        "next_action": o.next_action,
        "next_action_at": o.next_action_at,
        "needs_user_input": o.needs_user_input,
        "recoverable": o.recoverable,
    })
}

/// The snapshot `reconcile --apply` stores.
fn snapshot(o: &Observation) -> Value {
    json!({
        "observed_at": o.observed_at,
        "observation_token": o.token,
        "basis": o.basis,
        "runtime_state": o.runtime_state,
        "probe_codes": o.probe_codes,
        "checkpoint_seq": o.checkpoint_seq,
        "checkpoint_valid": o.checkpoint_valid,
        "checkpoint_integrity": o.checkpoint_integrity,
        "artifact_integrity": o.artifact_integrity,
        "approval_integrity": o.approval_integrity,
        "conversation_ok": o.conversation.is_none(),
        "blocked_reasons": o.blocked_reasons,
    })
}

/// Entries `reconcile` without an id looks at: work states and anything with a recorded session.
fn tracked(entry: &Entry) -> bool {
    WORK_STATES.contains(&entry.state.as_str())
        || session_field(entry, "handle").is_some_and(|h| !h.is_empty())
        || session_field(entry, "workspace_id").is_some_and(|w| !w.is_empty())
}

// ----- reconcile -----------------------------------------------------------------------------

/// `ach reconcile [id] [--apply]`: observe the runtime of one entry, or of every entry in a work
/// state or with a recorded session. Read-only by default. `apply` stores each snapshot in
/// `extra.runtime.observation` only when state, history and session are still what was observed
/// (others are listed in `conflicts`). Never launches, closes, adopts a replacement terminal, or
/// touches workflow state or approvals. `changed` lists entries whose stored token changed.
pub fn reconcile(id: Option<&str>, apply: bool) -> Result<Value> {
    reconcile_at(id, apply, now_at())
}

pub(crate) fn reconcile_at(id: Option<&str>, apply: bool, at: DateTime<FixedOffset>) -> Result<Value> {
    let entries: Vec<Entry> = registry::read(|reg| match id {
        Some(id) => Ok(vec![reg.get(id)?.clone()]),
        None => Ok(reg.contests.values().filter(|e| tracked(e)).cloned().collect()),
    })?;
    let observed: Vec<(Entry, Observation)> = entries
        .into_iter()
        .map(|e| {
            let o = observe(&e, at);
            (e, o)
        })
        .collect();
    let (changed, conflicts) = if apply { store_snapshots(&observed)? } else { (Vec::new(), Vec::new()) };
    Ok(json!({
        "entries": observed.iter().map(|(e, o)| observation_json(e, o)).collect::<Vec<_>>(),
        "changed": changed,
        "conflicts": conflicts,
        "applied": apply,
    }))
}

/// Store each observation on its entry when state, history and session are still the observed
/// ones (compare-and-set); returns `(changed, conflicts)`.
fn store_snapshots(observed: &[(Entry, Observation)]) -> Result<(Vec<String>, Vec<String>)> {
    let mut changed = Vec::new();
    let mut conflicts = Vec::new();
    if observed.is_empty() {
        return Ok((changed, conflicts));
    }
    registry::write(|reg| {
        for (base, o) in observed {
            let Ok(current) = reg.get_mut(&base.id) else {
                conflicts.push(base.id.clone());
                continue;
            };
            if lifecycle::unchanged(current, base).is_err() || record_malformed(current) {
                conflicts.push(base.id.clone());
                continue;
            }
            let rec = record_mut(current)?;
            let previous = rec.get("observation").and_then(|v| v.get("observation_token")).and_then(Value::as_str);
            if previous != Some(o.token.as_str()) {
                changed.push(base.id.clone());
            }
            rec.insert("observation".into(), snapshot(o));
        }
        Ok(())
    })?;
    Ok((changed, conflicts))
}

// ----- recover -------------------------------------------------------------------------------

/// The recovery turn: [`RECOVER_MESSAGE`] and the verified checkpoint as data.
fn recovery_message(entry: &Entry) -> Result<String> {
    let Some(cp) = current_checkpoint(entry) else {
        crate::bail!("{}: 현재 작업판의 체크포인트가 없다", entry.id)
    };
    let mut text = format!(
        "{RECOVER_MESSAGE}\n\n[하네스 체크포인트 #{} · 단계 {} · 작업판 r{} · 기록 {}]\n다음 작업(세션이 남긴 기록): {}\n완료 산출물(하네스가 SHA-256을 다시 확인함):",
        cp.seq, cp.stage, cp.revision, cp.recorded_at, cp.next_action
    );
    for item in cp.artifacts.as_array().into_iter().flatten() {
        let path = item.get("path").and_then(Value::as_str).unwrap_or("-");
        let sha = item.get("sha256").and_then(Value::as_str).unwrap_or("-");
        text.push_str(&format!("\n- {path} (sha256 {sha})"));
    }
    if let Some((_, at)) = cp.next_action_at {
        text.push_str(&format!("\n다음 작업 예정 시각: {at}"));
    }
    Ok(text)
}

fn running_without(registry: &registry::Registry, entry: &Entry) -> BTreeMap<u8, usize> {
    let mut running = registry.running_by_level();
    if IN_PROGRESS.contains(&entry.state.as_str())
        && entry.source != "legacy"
        && let Some(n) = running.get_mut(&entry.level.unwrap_or(0))
    {
        *n = n.saturating_sub(1);
    }
    running
}

/// `ach recover <id> --expected-observation TOKEN [--dry-run]`: reopen the saved conversation of
/// existing work whose process is confirmed gone, from its verified checkpoint. Only `running` and
/// `awaiting_approval`; never a routed start, never `plan_review`. Under the launch guard it
/// observes again and requires: the same observation token, runtime `missing` (unknown and
/// ambiguous never count), a valid checkpoint of the current revision whose files still hash, no
/// unresolved submission, no blocker or question, a due `next_action_at`, one known saved
/// conversation, no interrupted or earlier recovery from the same checkpoint, and a free slot for
/// the entry's level counted without the entry itself. Refusals are stable codes (a dry run
/// returns them in `refusals`). The restart keeps the workflow state and revision and gets the
/// checkpoint as its next turn; a marker stored before it prevents a duplicate recovery.
pub fn recover(id: &str, expected: &str, config: &Config, dry_run: bool) -> Result<Value> {
    recover_with(id, expected, config, dry_run, &crate::budget::capacity, now_at())
}

pub(crate) fn recover_with(
    id: &str,
    expected: &str,
    config: &Config,
    dry_run: bool,
    capacity: CapacityObserver<'_>,
    at: DateTime<FixedOffset>,
) -> Result<Value> {
    let _guard = if dry_run {
        None
    } else {
        lifecycle::refuse_inside_session("recover")?;
        Some(registry::launch_guard()?)
    };
    let (base, running) = registry::read(|reg| {
        let entry = reg.get(id)?.clone();
        let running = running_without(reg, &entry);
        Ok((entry, running))
    })?;
    let o = observe(&base, at);
    let mut refusals: Vec<&'static str> = Vec::new();
    if base.state == "plan_review" {
        refusals.push("human_plan_review");
    } else if !RECOVER_STATES.contains(&base.state.as_str()) {
        refusals.push("state_not_recoverable");
    }
    if o.token != expected {
        refusals.push("observation_changed");
    }
    match o.runtime_state {
        "missing" => {}
        "running" => refusals.push("runtime_running"),
        "unknown" => refusals.push("runtime_unknown"),
        "ambiguous" => refusals.push("runtime_ambiguous"),
        "not_started" => refusals.push("runtime_not_started"),
        _ => refusals.push("runtime_stopped"),
    }
    for code in &o.blocked_reasons {
        push_unique(&mut refusals, code);
    }
    if o.runtime_state == "missing"
        && !session_field(&base, "runner").and_then(|r| runners::get(r).ok()).is_some_and(|r| r.available())
    {
        push_unique(&mut refusals, "runner_unavailable");
    }
    if !o.checkpoint_valid {
        push_unique(&mut refusals, checkpoint_record_problem(&base).unwrap_or("checkpoint_artifact_changed"));
    }
    if refusals.is_empty() && lifecycle::check_capacity(&base, config, &running, capacity).is_err() {
        refusals.push("no_capacity");
    }
    let mut out = json!({
        "id": id,
        "dry_run": dry_run,
        "workflow_state": base.state,
        "runtime_state": o.runtime_state,
        "observation_token": o.token,
        "checkpoint_seq": o.checkpoint_seq,
        "recoverable": refusals.is_empty(),
        "refusals": refusals.clone(),
        "recovered": false,
    });
    if !refusals.is_empty() {
        if dry_run {
            return Ok(out);
        }
        crate::bail!("{id}: 복구하지 않았다({}). ach reconcile {id}로 상태를 확인한다", refusals.join(", "));
    }
    let message = recovery_message(&base)?;
    let plan_recovery = Recovery { guard_held: true, expect: Some(&base), message: Some(message.as_str()) };
    // The whole restart plan (runner, workspace, conversation) is checked before anything is marked.
    let plan = lifecycle::restart_for_recovery(id, config, true, plan_recovery)?;
    out["restart"] = plan;
    if dry_run {
        return Ok(out);
    }
    let token = o.token.clone();
    let seq = o.checkpoint_seq;
    registry::write(|reg| {
        let current = reg.get_mut(id)?;
        lifecycle::unchanged(current, &base)?;
        if recovery_pending(current) {
            crate::bail!("{id}: 이전 복구가 끝나지 않았다(recovery_interrupted). 복구하지 않았다");
        }
        record_mut(current)?.insert(
            "recovery_pending".into(),
            json!({"at": now(), "observation_token": token, "checkpoint_seq": seq, "by": "ach recover"}),
        );
        Ok(())
    })?;
    // Runners whose restore takes no message still find the checkpoint turn in the folder.
    if let Some(folder) = base.folder.as_deref() {
        let path = std::path::Path::new(folder).join(".harness").join(format!("recovery-{}.md", seq.unwrap_or(0)));
        std::fs::create_dir_all(path.parent().expect("a .harness directory"))?;
        std::fs::write(&path, &message)?;
        out["recovery_file"] = json!(path.display().to_string());
    }
    let recovery = Recovery { guard_held: true, expect: Some(&base), message: Some(message.as_str()) };
    match lifecycle::restart_for_recovery(id, config, false, recovery) {
        Ok(restart) => {
            out["restart"] = restart;
            out["recovered"] = json!(true);
            Ok(out)
        }
        Err(err) => {
            // The marker stays: whatever the restart left behind needs a human look (`ach restart`
            // clears it once a session runs again).
            let message = err.0.clone();
            let _ = registry::write(|reg| {
                let current = reg.get_mut(id)?;
                record_mut(current)?
                    .insert("last_recovery_error".into(), json!({"at": now(), "observation_token": o.token}));
                Ok(())
            });
            Err(Error(format!(
                "{message} (복구 표시 recovery_pending은 남겨 두었다. 확인한 뒤 ach restart {id}로 직접 다시 연다)"
            )))
        }
    }
}

/// Called by `lifecycle` when a restart stored its new session: a pending recovery marker moves into
/// `recoveries` with the new handle.
pub(crate) fn complete_recovery(entry: &mut Entry, handle: &str, by: &str) {
    let Some(Value::Object(rec)) = entry.extra.get_mut(KEY) else { return };
    let Some(pending) = rec.remove("recovery_pending").filter(|v| !v.is_null()) else { return };
    let mut item = match pending {
        Value::Object(map) => map,
        other => {
            let mut map = Map::new();
            map.insert("marker".into(), other);
            map
        }
    };
    item.insert("completed_at".into(), json!(now()));
    item.insert("completed_by".into(), json!(by));
    item.insert("handle".into(), json!(handle));
    match rec.entry("recoveries").or_insert_with(|| Value::Array(Vec::new())) {
        Value::Array(items) => items.push(Value::Object(item)),
        slot => {
            let old = slot.take();
            *slot = Value::Array(vec![old, Value::Object(item)]);
        }
    }
    rec.remove("last_recovery_error");
}

// ----- summary -------------------------------------------------------------------------------

/// Pure stored-status summary for the board and alerts (no backend, network or file reads).
/// `{id, workflow_state, runtime_state, observation:{status, observed_at, token}, last_evidence_at,
/// checkpoint:{status, seq, stage, recorded_at, integrity}, approval_integrity, artifact_integrity,
/// blockers, gaps, next_action, next_action_at, due, needs_user_input, user_attention}`. Only codes,
/// times and sequence numbers: no checkpoint prose, readiness blocker text or notes.
pub fn summary(entry: &Entry) -> Value {
    summary_at(entry, now_at())
}

/// [`summary`] as of `at`.
pub fn summary_at(entry: &Entry, at: DateTime<FixedOffset>) -> Value {
    let work = WORK_STATES.contains(&entry.state.as_str());
    let stored = record(entry).and_then(|r| r.get("observation")).filter(|o| o.is_object());
    let observed_at = stored.and_then(|o| o.get("observed_at")).and_then(Value::as_str);
    let observation_status = match stored {
        None => "none",
        Some(o)
            if o.get("basis") == Some(&basis(entry))
                && observed_at
                    .and_then(parse_time)
                    .is_some_and(|t| t <= at && at - t <= Duration::minutes(OBSERVATION_TTL_MINUTES)) =>
        {
            "current"
        }
        Some(_) => "stale",
    };
    let obs = stored.filter(|_| observation_status == "current");
    let no_session = session_field(entry, "handle").is_none_or(str::is_empty)
        && session_field(entry, "workspace_id").is_none_or(str::is_empty);
    let runtime_state: &'static str = match known(obs, "runtime_state", RUNTIME_STATES) {
        Some(state) => state,
        None if !work && no_session => {
            if ever_ran(entry) {
                "stopped"
            } else {
                "not_started"
            }
        }
        None => "unobserved",
    };

    let latest = latest(entry);
    let (cp_status, cp_seq, cp_stage, cp_at) = match &latest {
        Latest::None => ("none", None, None, None),
        Latest::Invalid => ("invalid", None, None, None),
        Latest::Found(cp) if cp.revision != registry::revision(entry) as u64 => {
            ("stale", Some(cp.seq), Some(cp.stage), Some(cp.recorded_at))
        }
        Latest::Found(cp) => ("current", Some(cp.seq), Some(cp.stage), Some(cp.recorded_at)),
    };
    let observed_seq = obs.and_then(|o| o.get("checkpoint_seq")).and_then(Value::as_u64);
    let cp_integrity = match cp_status {
        "none" => "none",
        "invalid" => "invalid",
        _ if cp_seq.is_some() && cp_seq == observed_seq => {
            known(obs, "checkpoint_integrity", INTEGRITY).unwrap_or("unknown")
        }
        _ => "unknown",
    };
    let approval_integrity = if entry.state == "awaiting_approval" {
        known(obs, "approval_integrity", INTEGRITY).unwrap_or("unknown")
    } else {
        "not_applicable"
    };
    let artifact_integrity = if readiness::latest(entry).is_none() {
        "none"
    } else {
        known(obs, "artifact_integrity", INTEGRITY).unwrap_or("unknown")
    };

    let current = current_checkpoint(entry);
    let mut blockers = work_blockers(entry, current.as_ref());
    if let Some(codes) = obs.and_then(|o| o.get("probe_codes")).and_then(Value::as_array) {
        for code in codes.iter().filter_map(Value::as_str) {
            if let Some(known) = PROBE_CODES.iter().find(|c| **c == code) {
                push_unique(&mut blockers, known);
            }
        }
    }
    if work && cp_integrity == "invalid" && cp_status != "invalid" {
        push_unique(&mut blockers, "checkpoint_artifact_changed");
    }
    let mut gaps: Vec<&'static str> = Vec::new();
    if work {
        match observation_status {
            "none" => gaps.push("observation_none"),
            "stale" => gaps.push("observation_stale"),
            _ => {}
        }
        match cp_status {
            "none" => gaps.push("checkpoint_missing"),
            "invalid" => gaps.push("checkpoint_record_invalid"),
            "stale" => gaps.push("checkpoint_stale_revision"),
            _ if cp_integrity == "unknown" => gaps.push("checkpoint_unverified"),
            _ => {}
        }
    }
    let needs_user_input = blockers.iter().any(|c| matches!(*c, "needs_user_input" | "readiness_needs_user_input"));
    let next_at = current.as_ref().and_then(|cp| cp.next_action_at);
    let due = next_at.is_none_or(|(t, _)| t <= at);
    let recoverable = runtime_state == "missing"
        && RECOVER_STATES.contains(&entry.state.as_str())
        && cp_status == "current"
        && cp_integrity == "valid"
        && obs.and_then(|o| o.get("conversation_ok")).and_then(Value::as_bool) == Some(true)
        && blockers.is_empty()
        && due
        && !cp_seq.is_some_and(|seq| recovered_from(entry, seq));
    let next_action = decide(&Facts {
        state: &entry.state,
        runtime: runtime_state,
        blockers: &blockers,
        needs_user_input,
        due,
        recoverable,
        approval: approval_integrity,
    });
    let user_attention = !blockers.is_empty()
        || matches!(
            next_action,
            "user_input"
                | "user_plan_review"
                | "user_submit_approval"
                | "resolve_submission"
                | "inspect_runtime"
                | "manual_restart"
                | "recover_session"
        );

    let mut evidence: Vec<&str> = Vec::new();
    evidence.extend(cp_at);
    evidence.extend(observed_at);
    evidence.extend(entry.history.last().map(|h| h.at.as_str()));
    evidence.extend(entry.scores.last().and_then(|s| s.get("at")).and_then(Value::as_str));
    evidence.extend(readiness::latest(entry).and_then(|r| r.get("at")).and_then(Value::as_str));
    let last_evidence_at = evidence
        .into_iter()
        .filter_map(|text| parse_time(text).map(|t| (t, text)))
        .max_by_key(|(t, _)| *t)
        .map(|(_, text)| text.to_string());

    json!({
        "id": entry.id,
        "workflow_state": entry.state,
        "runtime_state": runtime_state,
        "observation": {
            "status": observation_status,
            "observed_at": observed_at,
            "token": stored.and_then(|o| o.get("observation_token")).cloned().unwrap_or(Value::Null),
        },
        "last_evidence_at": last_evidence_at,
        "checkpoint": {"status": cp_status, "seq": cp_seq, "stage": cp_stage, "recorded_at": cp_at, "integrity": cp_integrity},
        "approval_integrity": approval_integrity,
        "artifact_integrity": artifact_integrity,
        "blockers": blockers,
        "gaps": gaps,
        "next_action": next_action,
        "next_action_at": next_at.map(|(_, text)| text),
        "due": due,
        "needs_user_input": needs_user_input,
        "user_attention": user_attention,
    })
}

/// Probe codes a stored observation may carry (anything else is ignored by [`summary`]).
const PROBE_CODES: &[&str] = &[
    "session_unrecorded",
    "backend_unknown",
    "backend_unavailable",
    "backend_cannot_tell",
    "probe_failed",
    "inventory_failed",
    "unrecorded_live_terminal",
    "reopen_interrupted",
];

/// Integrity values a stored observation may carry.
const INTEGRITY: &[&str] = &["valid", "invalid", "unknown", "none", "not_applicable"];

/// A stored string field when it is one of `allowed` (hand-edited values read as absent).
fn known(obs: Option<&Value>, key: &str, allowed: &[&'static str]) -> Option<&'static str> {
    let value = obs?.get(key)?.as_str()?;
    allowed.iter().find(|a| **a == value).copied()
}

/// Successful recoveries with a fixed capacity observer, so live memory pressure never decides the
/// outcome. Each test runs alone in a child process of this test binary whose `HOME`, `PATH` (a
/// stub `omp` first) and `ACH_HOME` are set on the child's command line, so the real runner,
/// registry and `~/.omp` stay out of reach and this process's environment is never modified.
#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::Duration as Wait;

    use super::*;
    use crate::launch::SESSION_ENV;
    use crate::registry::{HistoryItem, NewEntry, apply_gate, new_entry};
    use crate::workspaces::headless::Headless;
    use crate::workspaces::{Open, Workspace};

    const OMP: &str = r#"#!/bin/sh
if [ "$1" = "usage" ]; then echo '{"reports":[]}'; exit 0; fi
mkdir -p .harness
{ echo "session=${ACH_SESSION:-none}"; for a in "$@"; do echo "arg=$a"; done; echo "end"; } >> .harness/omp-calls.log
exec sleep 30
"#;
    const UUID: &str = "01a154fb-8308-706f-adaa-8e7d56e584b2";
    /// Set only in the child: the fixture directory. Its presence stops the child from re-spawning.
    const CHILD: &str = "HARNESS_RUNTIME_UNIT_CHILD";

    /// In the parent: run test `name` alone in a child of this binary with its own
    /// `HOME`/`PATH`/`ACH_HOME`, assert it passed, and return `None`. In the child: build the
    /// fixture and return its directory.
    fn isolated(name: &str) -> Option<PathBuf> {
        if let Some(dir) = std::env::var_os(CHILD) {
            return Some(fixture(PathBuf::from(dir)));
        }
        let dir = std::env::temp_dir().join(format!("harness-runtime-unit-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = format!("{}:{}", dir.join("bin").display(), std::env::var("PATH").unwrap_or_default());
        let filter = format!("runtime::tests::{name}");
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

    /// Room by count only (no memory measurement, no pressure).
    fn roomy(config: &Config, running: &BTreeMap<u8, usize>) -> Value {
        crate::budget::capacity_with(config, running, &json!({"reports": []}), None, "normal")
    }

    /// One level-2 session at most: the recovering entry itself already counts as running, so a
    /// recovery succeeds only when it is left out of the count.
    fn config() -> Config {
        let overlay = json!({"runner": "omp", "workspace": "headless", "runner_args": {"omp": ["--approval-mode", "yolo"]},
            "max_running": 1, "max_running_by_level": {"1": 1, "2": 1, "3": 1, "4": 1, "5": 1}});
        serde_json::from_value(crate::config::merge(crate::config::defaults(), overlay)).unwrap()
    }

    fn dead_handle(folder: &Path) -> String {
        let handle = Headless
            .open(&Open {
                cwd: folder.to_path_buf(),
                title: "t",
                argv: vec!["sleep".into(), "30".into()],
                workspace_id: None,
            })
            .unwrap();
        workspaces::close_confirmed(&Headless, &handle, Wait::from_secs(10)).unwrap();
        handle
    }

    fn stored(env: &Path, id: &str) -> (Entry, PathBuf) {
        let folder = env.join("contests").join(id);
        fs::create_dir_all(&folder).unwrap();
        let folder = fs::canonicalize(folder).unwrap();
        let mut e = new_entry(NewEntry {
            id,
            title: "복구 시험 공모전",
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
        e.state = "running".into();
        e.folder = Some(folder.display().to_string());
        e.session = Some(json!({"workspace": "headless", "workspace_id": null, "runner": "omp",
            "handle": dead_handle(&folder), "session_id": UUID, "preset": null}));
        registry::write(|reg| {
            reg.contests.insert(e.id.clone(), e.clone());
            Ok(())
        })
        .unwrap();
        (e, folder)
    }

    fn record_checkpoint(id: &str, folder: &Path, next: &str) -> String {
        fs::write(folder.join("draft.md"), "초안 본문").unwrap();
        let sha = readiness::sha256_file(&folder.join("draft.md")).unwrap();
        let raw = json!({"stage": "draft", "completed_artifacts": [{"path": "draft.md", "sha256": sha}], "next_action": next});
        registry::write(|reg| checkpoint(reg.get_mut(id)?, &raw)).unwrap();
        sha
    }

    fn current(id: &str) -> Entry {
        registry::read(|reg| Ok(reg.get(id)?.clone())).unwrap()
    }

    fn observed(id: &str) -> Value {
        reconcile_at(Some(id), false, now_at()).unwrap()["entries"][0].clone()
    }

    fn omp_log(folder: &Path) -> String {
        let log = folder.join(".harness/omp-calls.log");
        for _ in 0..250 {
            let text = fs::read_to_string(&log).unwrap_or_default();
            if text.contains("end") {
                return text;
            }
            std::thread::sleep(Wait::from_millis(20));
        }
        panic!("the session never ran the runner: {}", log.display());
    }

    fn codes(out: &Value) -> Vec<String> {
        out["refusals"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn recovery_resumes_from_the_verified_checkpoint_once() {
        let Some(env) = isolated("recovery_resumes_from_the_verified_checkpoint_once") else { return };
        let (before, folder) = stored(&env, "unit-recover");
        let sha = record_checkpoint("unit-recover", &folder, "3장 근거 보강부터 이어서");
        let o = observed("unit-recover");
        assert_eq!((o["runtime_state"].as_str(), o["recoverable"].as_bool()), (Some("missing"), Some(true)), "{o}");
        let token = o["observation_token"].as_str().unwrap().to_string();

        let out = recover_with("unit-recover", &token, &config(), false, &roomy, now_at()).unwrap();
        assert_eq!(out["recovered"], true, "{out}");
        let log = omp_log(&folder);
        assert!(log.contains("session=unit-recover"), "{log}");
        assert!(log.contains(&format!("arg={UUID}")), "the saved conversation is restored: {log}");
        let file = std::fs::read_to_string(folder.join(".harness/recovery-1.md")).unwrap();
        assert!(
            file.contains("3장 근거 보강부터 이어서") && file.contains("draft.md") && file.contains(&sha),
            "{file}"
        );
        assert!(file.contains("다시 제출하지 않"), "the turn forbids a second submission: {file}");
        assert!(out["restart"]["warnings"].to_string().contains("메시지를 받지 않는다"), "{out}");

        let after = current("unit-recover");
        assert_eq!(
            (after.state.as_str(), after.history.len()),
            ("running", before.history.len()),
            "workflow untouched"
        );
        assert_eq!(registry::revision(&after), registry::revision(&before));
        let rec = &after.extra[KEY];
        assert!(rec.get("recovery_pending").is_none());
        let recoveries = rec["recoveries"].as_array().unwrap();
        assert_eq!(recoveries.len(), 1);
        assert_eq!(recoveries[0]["checkpoint_seq"], 1);
        assert_eq!(recoveries[0]["observation_token"], json!(token));
        let handle = after.session.as_ref().unwrap()["handle"].as_str().unwrap().to_string();
        assert_eq!(recoveries[0]["handle"], json!(handle));

        // The same observation cannot recover twice; the new session runs.
        let err = recover_with("unit-recover", &token, &config(), false, &roomy, now_at()).unwrap_err();
        assert!(err.0.contains("observation_changed") && err.0.contains("runtime_running"), "{err}");

        // It dies again: the same checkpoint is not recovered a second time.
        workspaces::close_confirmed(&Headless, &handle, Wait::from_secs(10)).unwrap();
        let o = observed("unit-recover");
        assert_eq!((o["runtime_state"].as_str(), o["recoverable"].as_bool()), (Some("missing"), Some(false)), "{o}");
        assert_eq!(o["next_action"], "manual_restart");
        let dry =
            recover_with("unit-recover", o["observation_token"].as_str().unwrap(), &config(), true, &roomy, now_at())
                .unwrap();
        assert!(codes(&dry).contains(&"recovery_already_done".to_string()), "{dry}");

        // A new checkpoint from the resumed work makes it recoverable again (dry run only).
        record_checkpoint("unit-recover", &folder, "4장");
        let o = observed("unit-recover");
        assert_eq!(o["recoverable"], true, "{o}");
        let dry =
            recover_with("unit-recover", o["observation_token"].as_str().unwrap(), &config(), true, &roomy, now_at())
                .unwrap();
        assert_eq!(dry["recoverable"], true, "{dry}");
        assert!(dry["restart"]["command"].as_str().unwrap().contains("--restore"));
        assert_eq!(current("unit-recover").session.unwrap()["handle"], json!(handle), "a dry run touches nothing");
    }

    #[test]
    fn an_interrupted_recovery_blocks_another_until_a_restart_resolves_it() {
        let Some(env) = isolated("an_interrupted_recovery_blocks_another_until_a_restart_resolves_it") else { return };
        let (_, folder) = stored(&env, "unit-crash");
        record_checkpoint("unit-crash", &folder, "다음");
        // A crash after the marker was stored, before the restart finished.
        registry::write(|reg| {
            record_mut(reg.get_mut("unit-crash")?)?.insert(
                "recovery_pending".into(),
                json!({"at": now(), "observation_token": "obs-old", "checkpoint_seq": 1}),
            );
            Ok(())
        })
        .unwrap();
        let o = observed("unit-crash");
        assert_eq!(o["recoverable"], false);
        assert!(o["blocked_reasons"].as_array().unwrap().contains(&json!("recovery_interrupted")), "{o}");
        assert_eq!(o["next_action"], "inspect_runtime");
        let dry =
            recover_with("unit-crash", o["observation_token"].as_str().unwrap(), &config(), true, &roomy, now_at())
                .unwrap();
        assert!(codes(&dry).contains(&"recovery_interrupted".to_string()));
        let summary = summary(&current("unit-crash"));
        assert_eq!(summary["user_attention"], true);
        assert!(summary["blockers"].as_array().unwrap().contains(&json!("recovery_interrupted")));

        // A person restarts by hand: the marker moves into the record.
        let out = lifecycle::restart("unit-crash", &config(), None, None, false).unwrap();
        let rec = current("unit-crash").extra[KEY].clone();
        assert!(rec.get("recovery_pending").is_none(), "{rec}");
        assert_eq!(rec["recoveries"][0]["completed_by"], "restart");
        assert_eq!(rec["recoveries"][0]["handle"], out["handle"]);
        let _ = Headless.close(out["handle"].as_str().unwrap());
    }

    #[test]
    fn apply_stores_nothing_for_entries_that_changed_since_observed() {
        let Some(env) = isolated("apply_stores_nothing_for_entries_that_changed_since_observed") else { return };
        let (_, folder) = stored(&env, "unit-cas");
        record_checkpoint("unit-cas", &folder, "다음");
        let base = current("unit-cas");
        let o = observe(&base, now_at());
        assert_eq!(o.runtime_state, "missing");
        // Another command moves the entry between the observation and the store.
        registry::write(|reg| {
            let e = reg.get_mut("unit-cas")?;
            e.history.push(HistoryItem::new(now(), Some("running".into()), "awaiting_approval", "t", ""));
            e.state = "awaiting_approval".into();
            Ok(())
        })
        .unwrap();
        let (changed, conflicts) = store_snapshots(&[(base, o)]).unwrap();
        assert_eq!((changed, conflicts), (vec![], vec!["unit-cas".to_string()]));
        let after = current("unit-cas");
        assert!(after.extra[KEY].get("observation").is_none());
        assert_eq!(after.state, "awaiting_approval", "apply never rewrites the workflow state");
    }
}
