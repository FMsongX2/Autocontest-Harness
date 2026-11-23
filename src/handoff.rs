//! Delegated handoff: once a contest is `awaiting_approval` with a structured readiness record, the
//! harness itself applies the user's standing delegation. When `delegation` covers the contest and
//! `ach approval-status` is `eligible`, it records the delegated approval with the request token it
//! reviewed (the same compare-and-swap as `ach approve --delegated`) and tells the recorded contest
//! session through `orca orchestration send`; nothing is typed into a terminal.
//!
//! Every step is written to a journal file per request (`data/handoff/<id>--<token>.json`) before
//! the next one starts, under one lock, so repeated events, a restart or a partial failure never
//! approve or hand off the same request twice:
//!
//! ```text
//! claimed → approved → sending → sent
//!                         ├──→ send_failed   (Orca refused it: retried by a later run, bounded)
//!                         └──→ unknown       (no readable answer: never resent automatically)
//! ```
//!
//! Nothing is inferred. A valid approval without a journal was recorded by someone else and is
//! left to them; a submission attempt of any kind, an unverified session or an unreadable Orca
//! answer stops the handoff and is reported as a stable code. The result also tells a request
//! that waits on a reason apart from one that needs the user's own approval.

use std::path::PathBuf;

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::bail;
use crate::config::{self, Config};
use crate::error::{Error, Result};
use crate::registry::{self, Entry};
use crate::text::{kst, now};
use crate::workspaces::orca::orca_json;
use crate::{approval, launch, ledger, paths, process, readiness, runners, store, submission};

/// The newest journal format this build reads and writes.
pub const VERSION: u32 = 1;
/// Definite Orca refusals retried per request before the handoff waits for the controller.
pub const MAX_SEND_ATTEMPTS: u64 = 3;
/// Orca's answers for a terminal it no longer runs.
const GONE: &[&str] = &["terminal_handle_stale", "terminal_not_found", "selector_not_found"];

const APPROVAL_NOTE: &str =
    "현재 준비 요청에 대한 사용자 상시 위임 승인(준비 완료 자동 인계). 위임 근거와 범위는 CLI 설정 기록을 따른다.";

/// One request's journal. Ids, tokens, times and stable codes only: no titles, notes or paths.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub version: u32,
    pub id: String,
    pub request_token: String,
    pub revision: u64,
    /// `claimed`, `approved`, `sending`, `sent`, `send_failed` or `unknown`.
    pub stage: String,
    /// The event that opened the request (`transition`, `readiness`, `score`, `rules`, `manual`, …).
    pub trigger: String,
    /// `ACH_SESSION` of the command that opened the request (the contest's own session), if any.
    pub origin_session: Option<String>,
    pub claimed_at: String,
    pub approved_at: Option<String>,
    /// The session the message goes to: `{handle, workspace_id}`.
    pub target: Option<Value>,
    pub send_attempts: u64,
    pub send_started_at: Option<String>,
    pub sent_at: Option<String>,
    pub message_id: Option<String>,
    /// The last stable failure code.
    pub error: Option<String>,
    pub updated_at: String,
    /// Fields this version does not know about are kept as they are.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub struct Options<'a> {
    pub trigger: &'a str,
    pub dry_run: bool,
    /// Resend a handoff whose outcome is `unknown` or whose retries ran out, for exactly this
    /// request token. Only after the controller confirmed that the session never got the message.
    pub resend_request: Option<&'a str>,
    /// `ACH_SESSION` of the caller, recorded with a new claim.
    pub origin_session: Option<&'a str>,
}

fn dir() -> PathBuf {
    paths::data().join("handoff")
}

/// Registry ids and request tokens become a file name, so only plain name characters pass.
fn safe_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 200
        && !text.starts_with('.')
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

fn record_path(id: &str, token: &str) -> Result<PathBuf> {
    if !safe_name(id) || !safe_name(token) {
        bail!("{id}: 인계 기록 이름으로 쓸 수 없는 id 또는 요청 토큰이다");
    }
    Ok(dir().join(format!("{id}--{token}.json")))
}

fn load(id: &str, token: &str) -> Result<Option<Record>> {
    let path = record_path(id, token)?;
    let Some(text) = store::read(&path)? else { return Ok(None) };
    store::check_version(&path, &text, VERSION)?;
    let record: Record = serde_json::from_str(&text).map_err(|err| Error(format!("{}: {err}", path.display())))?;
    if record.id != id || record.request_token != token {
        bail!("{}: 인계 기록의 id·요청 토큰이 파일 이름과 다르다. 덮어쓰지 않는다", path.display());
    }
    Ok(Some(record))
}

fn save(record: &mut Record) -> Result<()> {
    record.updated_at = now();
    let mut text = serde_json::to_string_pretty(record)?;
    text.push('\n');
    store::write_atomic(&record_path(&record.id, &record.request_token)?, text.as_bytes(), false)
}

/// The latest journal of `id` whatever its token (for status output), by claim time.
fn latest_record(id: &str) -> Option<Record> {
    let prefix = format!("{id}--");
    let mut found: Vec<Record> = std::fs::read_dir(dir())
        .ok()?
        .flatten()
        .filter(|file| file.file_name().to_str().is_some_and(|name| name.starts_with(&prefix)))
        .filter_map(|file| std::fs::read_to_string(file.path()).ok())
        .filter_map(|text| serde_json::from_str::<Record>(&text).ok())
        .filter(|record| record.id == id)
        .collect();
    found.sort_by(|a, b| a.claimed_at.cmp(&b.claimed_at));
    found.pop()
}

/// A stable code out of a tool answer: plain identifier characters only, so no message text.
fn code_of(text: &str, fallback: &str) -> String {
    let code: String = text.chars().filter(|c| c.is_ascii_alphanumeric() || "_.:-".contains(*c)).take(80).collect();
    if code.is_empty() { fallback.to_string() } else { code }
}

fn outcome(entry_id: &str, status: &str, reasons: Vec<String>, token: Option<&str>, user_request: bool) -> Value {
    json!({
        "id": entry_id,
        "status": status,
        "reasons": reasons,
        "request_token": token,
        "user_approval_request": user_request,
    })
}

fn finish(mut result: Value, record: Option<&Record>, dry_run: bool) -> Value {
    result["record"] = record.map(|r| json!(r)).unwrap_or(Value::Null);
    result["dry_run"] = json!(dry_run);
    result
}

/// The deadline lies in the past for certain. A missing or unreadable deadline is not that.
fn deadline_passed(entry: &Entry, at: DateTime<FixedOffset>) -> bool {
    let Some(text) = entry.deadline.as_deref().map(str::trim).filter(|d| !d.is_empty()) else { return false };
    let end = DateTime::parse_from_rfc3339(text)
        .ok()
        .or_else(|| DateTime::parse_from_str(text, "%Y-%m-%dT%H:%M%:z").ok())
        .or_else(|| {
            let day = NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()?;
            day.and_hms_opt(23, 59, 59)?.and_local_timezone(kst()).single()
        });
    end.is_some_and(|end| end < at)
}

/// Why no new submission instruction may go out: the contest was already submitted, or an attempt
/// exists that the session started (pending, in flight, unknown) or that belongs to this work
/// revision. Empty when nothing was ever sent or started for the current work.
fn submission_reasons(entry: &Entry) -> Result<Vec<String>> {
    let mut reasons: Vec<String> = Vec::new();
    let mut push = |code: &str| {
        if !reasons.iter().any(|r| r == code) {
            reasons.push(code.to_string());
        }
    };
    let submitted = ledger::load_at(&ledger::path())?;
    if !entry.submissions.is_empty()
        || submitted.submissions.iter().any(|s| s.key == entry.id || s.id.as_deref() == Some(entry.id.as_str()))
    {
        push("already_submitted");
    }
    // The same match `ach route` refuses on: a ledger item (another key) for this contest's URL or
    // a similar title of the same edition, such as a submission recorded outside the harness.
    let urls: Vec<&str> =
        std::iter::once(entry.url.as_str()).chain(entry.alt_urls.iter().map(String::as_str)).collect();
    if !submitted.matches(&entry.title, &urls, Some(&entry.id)).is_empty() {
        push("submission_ledger_match");
    }
    let status = submission::status(entry);
    for blocker in status["blockers"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        push(blocker);
    }
    let revision = registry::revision(entry) as u64;
    for attempt in status["attempts"].as_array().into_iter().flatten() {
        if attempt["status"] == "confirmed" {
            push("already_submitted");
        } else if attempt["revision"].as_u64() == Some(revision) {
            push("submission_attempt_exists");
        }
    }
    Ok(reasons)
}

/// The recorded contest session, verified live: an Orca terminal that is connected, belongs to the
/// recorded workspace and runs omp. Anything else, or an answer that cannot be read, is a code.
fn verify_session(entry: &Entry) -> std::result::Result<Value, String> {
    let field = |key: &str| launch::session_field(entry, key).filter(|v| !v.is_empty());
    let Some(handle) = field("handle") else { return Err("session_missing".into()) };
    if field("workspace") != Some("orca") {
        return Err("session_not_orca".into());
    }
    if field("runner") != Some("omp") {
        return Err("session_not_omp".into());
    }
    let shown = match orca_json(&["terminal", "show", "--terminal", handle]) {
        Ok(shown) => shown,
        Err(err) if GONE.iter().any(|code| err.0.contains(code)) => return Err("session_not_live".into()),
        Err(_) => return Err("session_unverified".into()),
    };
    let terminal = shown.get("terminal").unwrap_or(&shown);
    match terminal.get("connected").and_then(Value::as_bool) {
        Some(true) => {}
        Some(false) => return Err("session_not_live".into()),
        None => return Err("session_unverified".into()),
    }
    let workspace_id = field("workspace_id");
    if let Some(recorded) = workspace_id
        && terminal.get("worktreeId").and_then(Value::as_str) != Some(recorded)
    {
        return Err("session_workspace_mismatch".into());
    }
    if terminal.get("agentIdentity").and_then(Value::as_str) != Some("omp") {
        return Err("session_agent_unverified".into());
    }
    Ok(json!({"handle": handle, "workspace_id": workspace_id}))
}

fn subject(record: &Record) -> String {
    format!("제출 승인 기록됨: {} ({})", record.id, record.request_token)
}

fn body(record: &Record) -> String {
    format!(
        "하네스가 사용자의 상시 위임 범위 안에서 공모전 {id}의 제출 준비 요청 {token}에 대한 제출 승인을 기록했습니다. \
이 메시지는 승인 자체가 아닙니다. 먼저 ach approval-status {id} --json의 submit_approval_valid가 true이고 \
request_token이 {token}인지 확인하고, 다르면 제출하지 말고 멈춥니다. 미해결 질문을 임의로 기본값 처리하지 않습니다. \
승인된 제출 서식의 개인정보·기존 서명 입력이 허용된 경우에만 ach profile show --for {id}로 읽어 별도 최종 사본을 만들고, \
검수한 원본은 보존하며, 바뀐 최종 사본의 전 페이지를 다시 렌더·비전 검수한 뒤 \
ach readiness {id} --file <검수된 요청 파일> --personalized로 실제 업로드 파일 해시를 연결합니다. \
본문·내용 수정은 running으로 돌아가 재채점·재승인을 받습니다. 제출은 거래 절차로만 합니다. \
공고의 접수 기간과 제출처를 확인한 뒤 ach submission intent {id} --file <요청>(승인된 공식 제출처와 현재 요청 토큰)으로 \
의도를 기록하고, 외부 전송 직전에 ach submission start {id} --attempt <A>로 dispatching을 남깁니다. \
시간 초과·응답 유실처럼 결과가 불확실하면 다시 보내지 말고 ach submission unknown {id} --attempt <A> --file <근거>로 \
기록한 뒤 공식 접수 목록·메일로 확인합니다. 접수 증빙은 .private/에 저장하고 \
ach submit {id} --attempt <A> --file <증빙 JSON>으로 확정합니다. 미제출이 공식 근거로 확인된 경우에만 \
ach submission resolve로 닫고 새 의도를 기록합니다. 접수 기간 전이거나 확인되지 않은 조건이 있으면 추정하지 말고 \
제출 준비 기록의 blockers에 적고 멈춥니다. 확정 직후 결과물을 커밋·push합니다. 제출 준비 기록의 followup_required가 \
true(본선·발표 등 후속 제작 일정)이면 followup_pending으로 옮기고 폴더를 유지합니다. false이면 접수 회신만 기다리는 \
경우에도 ach clean {id} --dry-run으로 막는 이유가 없는지 확인한 뒤 곧바로 ach clean {id}로 증빙을 보관하고 세션·폴더를 \
정리합니다. 제출 시도가 미해결이거나 결과 불명이면 정리하지 않고 그대로 보존합니다. \
새 계정·결제·새로운 동의·새 서명이나 승인 범위 밖 전송은 이 위임으로 진행하지 않습니다.",
        id = record.id,
        token = record.request_token
    )
}

/// Send the handoff message once. The `sending` stage is stored before Orca is called, so a crash
/// in between is read back as `unknown`, never as "not sent".
fn send(record: &mut Record, target: Value) -> Result<()> {
    let handle = target["handle"].as_str().unwrap_or_default().to_string();
    record.target = Some(target);
    if runners::which("orca").is_none() {
        record.stage = "send_failed".into();
        record.send_attempts += 1;
        record.error = Some("orca_unavailable".into());
        return save(record);
    }
    record.stage = "sending".into();
    record.send_attempts += 1;
    record.send_started_at = Some(now());
    record.error = None;
    save(record)?;
    let (subject, body) = (subject(record), body(record));
    let args = [
        "orchestration",
        "send",
        "--to",
        &handle,
        "--type",
        "handoff",
        "--subject",
        &subject,
        "--body",
        &body,
        "--json",
    ];
    let answer = process::output("orca", &args, None, process::LOCAL)
        .ok()
        .and_then(|out| serde_json::from_slice::<Value>(&out.stdout).ok());
    let message_id = answer
        .as_ref()
        .filter(|a| a["ok"] == true)
        .and_then(|a| a.pointer("/result/messageId"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    let refusal =
        answer.as_ref().filter(|a| a["ok"] == false).and_then(|a| a.pointer("/error/code")).and_then(Value::as_str);
    match (message_id, refusal) {
        (Some(id), _) => {
            record.stage = "sent".into();
            record.sent_at = Some(now());
            record.message_id = Some(code_of(id, "message"));
        }
        (None, Some(code)) => {
            record.stage = "send_failed".into();
            record.error = Some(code_of(code, "send_refused"));
        }
        // A timeout, a crash of the tool or an answer without a message id: it may have arrived.
        (None, None) => {
            record.stage = "unknown".into();
            record.error = Some("send_outcome_unknown".into());
        }
    }
    save(record)
}

fn after_send(entry: &Entry, record: &Record, dry_run: bool) -> Value {
    let token = Some(record.request_token.as_str());
    let (status, reasons) = match record.stage.as_str() {
        "sent" => ("handed_off", Vec::new()),
        "send_failed" => ("send_failed", record.error.iter().cloned().collect()),
        _ => ("handoff_unknown", record.error.iter().cloned().collect()),
    };
    finish(outcome(&entry.id, status, reasons, token, false), Some(record), dry_run)
}

/// Run the delegated handoff for one contest. Read-only with `dry_run`. Never fails on a refusal:
/// the result's `status` and `reasons` say what happened and what stops it.
pub fn run(id: &str, options: &Options<'_>) -> Result<Value> {
    let lock = if options.dry_run { None } else { store::lock_file(&paths::data().join("handoff.json"), true)? };
    if let Some(lock) = &lock {
        lock.lock()?;
    }
    let cfg = config::load()?;
    let entry = registry::read(|reg| Ok(reg.get(id)?.clone()))?;
    run_locked(&cfg, &entry, options)
}

fn run_locked(cfg: &Config, entry: &Entry, options: &Options<'_>) -> Result<Value> {
    let dry = options.dry_run;
    let id = entry.id.as_str();
    let done = |status: &str, reasons: Vec<String>, token: Option<&str>, user: bool, record: Option<&Record>| {
        Ok(finish(outcome(id, status, reasons, token, user), record, dry))
    };
    if entry.state != "awaiting_approval" {
        return done("not_applicable", vec!["state_not_awaiting_approval".into()], None, false, None);
    }
    if entry.source == "legacy" {
        return done("not_applicable", vec!["legacy_entry".into()], None, false, None);
    }
    let status = approval::status(cfg, entry)?;
    let token = status["request_token"].as_str();
    let valid = status["submit_approval_valid"] == true;
    let record = match token {
        Some(token) => load(id, token)?,
        None => None,
    };
    let latest = record.clone().or_else(|| latest_record(id));
    let submissions = submission_reasons(entry)?;
    // What stops even the user's own approval (`ach approve`): the core conditions with a recorded
    // score-gate override accepted, plus anything already sent or started.
    let mut human: Vec<String> =
        readiness::evaluate(entry, false, true).reasons.iter().map(|r| r.to_string()).collect();
    human.extend(submissions.iter().cloned());
    let user_request = |policy_blocked: bool| policy_blocked && !valid && human.is_empty();

    // The switch: a disabled or narrower delegation stops here, pending handoffs included.
    if !cfg.delegation.enabled {
        let mut reasons = vec!["delegation_disabled".to_string()];
        reasons.extend(human.iter().cloned());
        return done("delegation_off", reasons, token, user_request(true), latest.as_ref());
    }
    if !cfg.delegation.covers(id) {
        let mut reasons = vec!["delegation_not_granted".to_string()];
        reasons.extend(human.iter().cloned());
        return done("out_of_scope", reasons, token, user_request(true), latest.as_ref());
    }
    if !submissions.is_empty() {
        return done("submission_exists", submissions, token, false, latest.as_ref());
    }

    let fresh = match (&record, valid) {
        (Some(record), _) if record.stage == "sent" => {
            // The request was handed off. Whatever invalidated the approval since is the session's
            // to fix through a new request; this one is never sent again.
            if valid {
                return done("already_handed_off", Vec::new(), token, false, Some(record));
            }
            let mut reasons = status_reasons(&status);
            if reasons.is_empty() {
                reasons.push("approval_missing_after_handoff".into());
            }
            return done("waiting", reasons, token, false, Some(record));
        }
        (None, true) => {
            // Approved by the user, the main session or another controller: their handoff, not ours.
            return done("approved_elsewhere", Vec::new(), token, false, latest.as_ref());
        }
        (Some(record), true) if record.stage == "claimed" && !approval::delegated_approval(entry) => {
            return done("approved_elsewhere", Vec::new(), token, false, Some(record));
        }
        (_, true) => false,
        (_, false) => true,
    };

    if fresh && status["eligible"] != true {
        let mut reasons = reason_codes(&status_reasons(&status));
        let override_only = registry::score_override_recorded(entry);
        if override_only {
            reasons.push("score_override_requires_user".into());
        }
        return done("waiting", reasons, token, user_request(override_only), latest.as_ref());
    }
    let Some(token) = token else {
        return done("waiting", vec!["readiness_missing".into()], None, false, None);
    };
    if deadline_passed(entry, Utc::now().with_timezone(&kst())) {
        return done("waiting", vec!["deadline_passed".into()], Some(token), false, record.as_ref());
    }

    // A stored send whose outcome nobody read back is never repeated without the controller.
    if let Some(record) = &record {
        let resend = options.resend_request == Some(token);
        let exhausted = record.stage == "send_failed" && record.send_attempts >= MAX_SEND_ATTEMPTS;
        if matches!(record.stage.as_str(), "sending" | "unknown") && !resend {
            let mut record = record.clone();
            if record.stage == "sending" && !dry {
                record.stage = "unknown".into();
                record.error = Some("send_interrupted".into());
                save(&mut record)?;
            }
            let code = record.error.clone().unwrap_or_else(|| "send_outcome_unknown".into());
            return done("handoff_unknown", vec![code], Some(token), false, Some(&record));
        }
        if exhausted && !resend {
            return done("send_failed", vec!["send_attempts_exhausted".into()], Some(token), false, Some(record));
        }
    }

    let target = match verify_session(entry) {
        Ok(target) => target,
        Err(code) => return done("session_unavailable", vec![code], Some(token), false, record.as_ref()),
    };
    if dry {
        let status = if fresh { "would_approve_and_hand_off" } else { "would_hand_off" };
        return done(status, Vec::new(), Some(token), false, record.as_ref());
    }

    let mut record = match record {
        Some(record) => record,
        None => {
            let at = now();
            let mut record = Record {
                version: VERSION,
                id: id.to_string(),
                request_token: token.to_string(),
                revision: registry::revision(entry) as u64,
                stage: "claimed".into(),
                trigger: code_of(options.trigger, "manual"),
                origin_session: options.origin_session.map(|s| code_of(s, "session")),
                claimed_at: at.clone(),
                approved_at: None,
                target: None,
                send_attempts: 0,
                send_started_at: None,
                sent_at: None,
                message_id: None,
                error: None,
                updated_at: at,
                extra: Map::new(),
            };
            save(&mut record)?;
            record
        }
    };
    if fresh {
        // The CLI's own compare-and-swap: policy, persona, evidence, core conditions and the token
        // are rechecked under the registry lock, with the configuration as it is at that moment.
        // No session is passed: this is the harness applying the standing delegation, not a
        // contest session approving its own work (`ach approve` stays refused there).
        let approved = registry::write(|reg| {
            approval::approve_delegated(&config::load()?, reg.get_mut(id)?, APPROVAL_NOTE, None, token)
        });
        if let Err(refusal) = approved {
            record.error = Some("approval_refused".into());
            save(&mut record)?;
            let mut result = outcome(id, "approve_refused", vec!["approval_refused".into()], Some(token), false);
            result["detail"] = json!(refusal.0);
            return Ok(finish(result, Some(&record), dry));
        }
    }
    if record.stage == "claimed" {
        record.stage = "approved".into();
        record.approved_at = Some(now());
        record.error = None;
        save(&mut record)?;
    }
    send(&mut record, target)?;
    Ok(after_send(entry, &record, dry))
}

fn status_reasons(status: &Value) -> Vec<String> {
    status["reasons"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

fn reason_codes(reasons: &[String]) -> Vec<String> {
    let mut codes: Vec<String> = Vec::new();
    for reason in reasons {
        if !codes.contains(reason) {
            codes.push(reason.clone());
        }
    }
    if codes.is_empty() {
        codes.push("not_eligible".into());
    }
    codes
}

/// Run the handoff for every contest in `awaiting_approval` (legacy imports excluded). One
/// contest's error is reported in its own result and never stops the others.
pub fn sweep(options: &Options<'_>) -> Result<Value> {
    let ids: Vec<String> = registry::read(|reg| {
        Ok(reg
            .contests
            .values()
            .filter(|entry| entry.state == "awaiting_approval" && entry.source != "legacy")
            .map(|entry| entry.id.clone())
            .collect())
    })?;
    let results: Vec<Value> = ids
        .iter()
        .map(|id| {
            run(id, options).unwrap_or_else(|_| {
                finish(outcome(id, "error", vec!["handoff_error".into()], None, false), None, options.dry_run)
            })
        })
        .collect();
    Ok(json!({"results": results}))
}

/// After a command that can make a request eligible (`transition`, `readiness`, `score`, a rules
/// review): run the handoff for `id` when it now waits for approval. `session` is `ACH_SESSION`;
/// another contest's session never triggers it. Returns `None` when there is nothing to report,
/// and never an error: the command that triggered it has already succeeded.
pub fn hook(id: &str, trigger: &str, session: Option<&str>) -> Option<Value> {
    let session = session.filter(|s| !s.is_empty());
    if session.is_some_and(|s| s != id) {
        return None;
    }
    let waiting = registry::read(|reg| Ok(reg.get(id)?.state == "awaiting_approval")).unwrap_or(false);
    if !waiting {
        return None;
    }
    let options = Options { trigger, dry_run: false, resend_request: None, origin_session: session };
    Some(
        run(id, &options)
            .unwrap_or_else(|_| finish(outcome(id, "error", vec!["handoff_error".into()], None, false), None, false)),
    )
}

/// One line for the session that triggered the hook: what happened, and whether the request waits
/// on a reason or needs the user's own approval.
pub fn describe(result: &Value) -> String {
    let id = result["id"].as_str().unwrap_or("-");
    let status = result["status"].as_str().unwrap_or("error");
    let reasons: Vec<&str> = result["reasons"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    // A question or an action the readiness record asks of the user is still asked: only the
    // approval is not what this request waits for.
    let input = reasons.iter().any(|r| r.ends_with("needs_user_input"));
    let reasons = if reasons.is_empty() { "-".to_string() } else { reasons.join(", ") };
    let token = result["request_token"].as_str().unwrap_or("-");
    let request = if result["user_approval_request"] == true {
        " 사용자 승인 요청 대상이다: 사용자에게 메인 세션이나 ach approve로 승인해 달라고 요청한다."
    } else if input {
        " 사용자 승인 요청이 아니다. 제출 준비 기록에 적은 사용자 입력(질문·조치)이 먼저 필요하다."
    } else {
        " 사용자 승인 요청이 아니다."
    };
    let text = match status {
        "handed_off" => format!(
            "제출 승인을 기록하고 담당 세션에 인계했다(요청 {token}). ach approval-status {id}로 확인한 뒤 제출 트랜잭션을 진행한다."
        ),
        "already_handed_off" => format!("이 요청({token})은 이미 인계됐다. 다시 보내지 않는다."),
        "approved_elsewhere" => {
            format!("이 요청({token})의 유효한 승인이 이미 있다(이 경로가 기록한 것이 아니다). 다시 인계하지 않는다.")
        }
        "delegation_off" => format!("상시 위임이 꺼져 있어 멈춘다({reasons}).{request}"),
        "out_of_scope" => format!("이 공모전은 상시 위임 범위 밖이다({reasons}).{request}"),
        "waiting" => format!("자동 승인 조건이 맞지 않아 대기한다({reasons}).{request}"),
        "submission_exists" => format!("제출 기록이나 제출 시도가 이미 있어 인계하지 않는다({reasons})."),
        "session_unavailable" => {
            format!("담당 세션을 확인하지 못해 승인·인계하지 않았다({reasons}). 컨트롤이 세션을 확인한다.")
        }
        "approve_refused" => format!("CLI가 위임 승인을 거절했다({reasons}). ach approval-status {id}로 확인한다."),
        "send_failed" => {
            format!("승인은 기록됐지만 인계 메시지를 보내지 못했다({reasons}). 다음 실행에서 다시 시도한다.")
        }
        "handoff_unknown" => {
            format!("인계 메시지의 전달 결과를 알 수 없다({reasons}). 자동으로 다시 보내지 않는다. 컨트롤이 확인한다.")
        }
        "not_applicable" => format!("대상이 아니다({reasons})."),
        _ => format!("인계 경로 오류({reasons}). ach handoff {id} --dry-run으로 확인한다."),
    };
    format!("위임 인계[{status}] {id}: {text}")
}
