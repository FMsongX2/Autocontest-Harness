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
//! A request token changes whenever a score or a readiness record is added, also when the files,
//! the destination and the rule baseline stay the same. The journal therefore keeps a hash of what
//! the request submits ([`readiness::request_content`]): a new token for content that was already
//! handed off only gets its approval (`reapproved`), not a second message. Changed files, a changed
//! destination or a new work revision are a new request and are handed off again.
//!
//! Nothing is inferred. A valid approval without a journal was recorded by someone else and is
//! left to them; a submission attempt of any kind, a submission window that has not opened, an
//! unverified session or an unreadable Orca answer stops the handoff and is reported as a stable
//! code. Every result carries a `display` state for people ([`STATES`]); a request is shown as
//! waiting for the user's approval only when `user_approval_request` is true.

use std::path::PathBuf;

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::bail;
use crate::config::{self, Config};
use crate::error::{Error, Result};
use crate::registry::{self, Entry};
use crate::text::{kst, now};
use crate::workspaces::orca::orca_json;
use crate::{approval, launch, ledger, paths, process, readiness, rules, runners, store, submission};

/// The newest journal format this build reads and writes.
pub const VERSION: u32 = 1;
/// Definite Orca refusals retried per request before the handoff waits for the controller.
pub const MAX_SEND_ATTEMPTS: u64 = 3;
/// Orca's answers for a terminal it no longer runs.
const GONE: &[&str] = &["terminal_handle_stale", "terminal_not_found", "selector_not_found"];

const APPROVAL_NOTE: &str =
    "현재 준비 요청에 대한 사용자 상시 위임 승인(준비 완료 자동 인계). 위임 근거와 범위는 CLI 설정 기록을 따른다.";

/// What a person is shown for a contest in `awaiting_approval`: `(state, label)`. Only
/// `user_approval_pending` asks the user to approve.
pub const STATES: &[(&str, &str)] = &[
    ("user_approval_pending", "사용자 승인 대기"),
    ("approved", "승인됨"),
    ("auto_approval_pending", "자동 승인 대기"),
    ("submitting", "제출 중"),
    ("sent_confirming", "이미 발송/접수 확인 중"),
    ("before_window", "접수 기간 전"),
    ("info_missing", "필수 정보 부족"),
    ("preparing", "제출 준비 중"),
    ("deadline_passed", "마감 경과"),
    ("handoff_attention", "인계 확인 필요"),
    ("not_applicable", "대상 아님"),
];

/// Readiness reasons that mean the request itself says it cannot go out yet or needs the user.
const INFO_REASONS: &[&str] = &[
    "needs_user_input",
    "original_needs_user_input",
    "blockers_present",
    "original_blockers_present",
    "readiness_not_ready",
    "original_readiness_not_ready",
];
/// Submission reasons that mean something may already be with the organizer.
const SENT_REASONS: &[&str] =
    &["submission_unknown", "already_submitted", "submission_ledger_match", "submission_record_malformed"];

/// One request's journal. Ids, tokens, times, hashes and stable codes only: no titles, notes or paths.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub version: u32,
    pub id: String,
    pub request_token: String,
    pub revision: u64,
    /// `claimed`, `approved`, `sending`, `sent`, `send_failed` or `unknown`.
    pub stage: String,
    /// The event that opened the request (`transition`, `readiness`, `score`, `rules`, `tick`, `manual`, …).
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
    /// SHA-256 of what the request submits (files and hashes, destination, opening time, fields,
    /// rule baseline, work revision). Absent in journals written before 0.9.4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// The token of the earlier request with the same content whose handoff this one continues:
    /// its approval was recorded again and no second message was sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continues: Option<String>,
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

/// Every readable journal of `id`, oldest claim first.
fn records(id: &str) -> Vec<Record> {
    let prefix = format!("{id}--");
    let Ok(files) = std::fs::read_dir(dir()) else { return Vec::new() };
    let mut found: Vec<Record> = files
        .flatten()
        .filter(|file| file.file_name().to_str().is_some_and(|name| name.starts_with(&prefix)))
        .filter_map(|file| std::fs::read_to_string(file.path()).ok())
        .filter_map(|text| serde_json::from_str::<Record>(&text).ok())
        .filter(|record| record.id == id)
        .collect();
    found.sort_by(|a, b| a.claimed_at.cmp(&b.claimed_at));
    found
}

/// A stable code out of a tool answer: plain identifier characters only, so no message text.
fn code_of(text: &str, fallback: &str) -> String {
    let code: String = text.chars().filter(|c| c.is_ascii_alphanumeric() || "_.:-".contains(*c)).take(80).collect();
    if code.is_empty() { fallback.to_string() } else { code }
}

fn hash_content(mut content: Value) -> String {
    if let Some(files) = content.get_mut("artifacts").and_then(Value::as_array_mut) {
        files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    }
    format!("{:x}", Sha256::digest(content.to_string().as_bytes()))
}

/// SHA-256 of what the current request submits, from the stored records (no file is read).
fn content_hash(entry: &Entry) -> Option<String> {
    readiness::request_content(entry).map(hash_content)
}

/// The content hash of the request a journal was written for. Journals written before 0.9.4
/// carry none; theirs is taken from the readiness record their token names
/// (`r<revision>-s<seq>-…`), when it is still there.
fn journal_content(entry: &Entry, journal: &Record) -> Option<String> {
    if journal.content.is_some() {
        return journal.content.clone();
    }
    let seq = journal.request_token.split('-').find_map(|part| part.strip_prefix('s')?.parse::<u64>().ok())?;
    readiness::content_at(entry, seq).map(hash_content)
}

fn parse_time(text: &str) -> Option<DateTime<FixedOffset>> {
    let text = text.trim();
    DateTime::parse_from_rfc3339(text).ok().or_else(|| DateTime::parse_from_str(text, "%Y-%m-%dT%H:%M%:z").ok())
}

/// The deadline lies in the past for certain. A missing or unreadable deadline is not that.
fn deadline_passed(entry: &Entry, at: DateTime<FixedOffset>) -> bool {
    let Some(text) = entry.deadline.as_deref().map(str::trim).filter(|d| !d.is_empty()) else { return false };
    let end = parse_time(text).or_else(|| {
        let day = NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()?;
        day.and_hms_opt(23, 59, 59)?.and_local_timezone(kst()).single()
    });
    end.is_some_and(|end| end < at)
}

/// The submission window of the current request, as its readiness record states it.
enum Window {
    /// No opening time recorded, or it has passed.
    Open,
    /// It opens at this recorded time, which is still ahead.
    Opens(String),
    /// An opening time is recorded but cannot be read: not taken for open.
    Unreadable,
}

fn window(entry: &Entry, at: DateTime<FixedOffset>) -> Window {
    match readiness::opens_at(entry) {
        None => Window::Open,
        Some(text) => match parse_time(text) {
            Some(opens) if opens > at => Window::Opens(text.to_string()),
            Some(_) => Window::Open,
            None => Window::Unreadable,
        },
    }
}

/// Submission codes the stored entry alone gives: a recorded submission, attempts that are
/// pending, in flight or unknown, a confirmed attempt, and any attempt of this work revision.
fn attempt_reasons(entry: &Entry) -> Vec<String> {
    let mut reasons: Vec<String> = Vec::new();
    let mut push = |code: &str| {
        if !reasons.iter().any(|r| r == code) {
            reasons.push(code.to_string());
        }
    };
    if !entry.submissions.is_empty() {
        push("already_submitted");
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
    reasons
}

/// Why no new submission instruction may go out: [`attempt_reasons`] plus the submission ledger.
/// Empty when nothing was ever sent or started for the current work.
fn submission_reasons(entry: &Entry) -> Result<Vec<String>> {
    let mut reasons = attempt_reasons(entry);
    let submitted = ledger::load_at(&ledger::path())?;
    if submitted.submissions.iter().any(|s| s.key == entry.id || s.id.as_deref() == Some(entry.id.as_str()))
        && !reasons.iter().any(|r| r == "already_submitted")
    {
        reasons.insert(0, "already_submitted".into());
    }
    // The same match `ach route` refuses on: a ledger item (another key) for this contest's URL or
    // a similar title of the same edition, such as a submission recorded outside the harness.
    let urls: Vec<&str> =
        std::iter::once(entry.url.as_str()).chain(entry.alt_urls.iter().map(String::as_str)).collect();
    if !submitted.matches(&entry.title, &urls, Some(&entry.id)).is_empty() {
        reasons.push("submission_ledger_match".into());
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
이 메시지는 승인 자체가 아닙니다. 먼저 ach approval-status {id} --json의 submit_approval_valid가 true인지 확인하고, \
false이면 제출하지 말고 멈춥니다. 같은 파일로 점수나 제출 준비 기록을 더 남겨 request_token만 바뀌었다면 승인도 새 토큰에 \
다시 기록돼 있으니, 그때의 request_token으로 진행합니다. 승인은 이미 기록됐으므로 사용자에게 승인을 다시 요청하지 않습니다. \
미해결 질문을 임의로 기본값 처리하지 않습니다. \
승인된 제출 서식의 개인정보·기존 서명 입력이 허용된 경우에만 ach profile show --for {id}로 읽어 별도 최종 사본을 만들고, \
검수한 원본은 보존하며, 바뀐 최종 사본의 전 페이지를 다시 렌더·비전 검수한 뒤 \
ach readiness {id} --file <검수된 요청 파일> --personalized로 실제 업로드 파일 해시를 연결합니다. \
본문·내용 수정은 running으로 돌아가 재채점·재승인을 받습니다. 같은 파일·같은 제출처의 제출 준비 기록을 다시 남길 \
필요는 없습니다(다시 남겨도 새 요청이 되지 않습니다). 제출은 거래 절차로만 합니다. \
공고의 접수 기간과 제출처를 확인한 뒤 ach submission intent {id} --file <요청>(승인된 공식 제출처와 현재 요청 토큰)으로 \
의도를 기록하고, 외부 전송 직전에 ach submission start {id} --attempt <A>로 dispatching을 남깁니다. \
시간 초과·응답 유실처럼 결과가 불확실하면 다시 보내지 말고 ach submission unknown {id} --attempt <A> --file <근거>로 \
기록한 뒤 공식 접수 목록·메일로 확인합니다. 접수 증빙은 .private/에 저장하고 \
ach submit {id} --attempt <A> --file <증빙 JSON>으로 확정합니다. 미제출이 공식 근거로 확인된 경우에만 \
ach submission resolve로 닫고 새 의도를 기록합니다. 접수가 아직 열리지 않았으면 보내지 말고 제출 준비 기록의 \
submission_opens_at에 공고의 접수 개시 일시를 적어 다시 기록합니다(개시 뒤 다시 승인·인계됩니다). 확인되지 않은 조건은 \
추정하지 말고 blockers에 적고 멈춥니다. 확정 직후 결과물을 커밋·push합니다. 제출 준비 기록의 followup_required가 \
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

// ----- what people see -------------------------------------------------------------------------

fn label_of(state: &str) -> &'static str {
    STATES.iter().find(|(code, _)| *code == state).map_or("확인 필요", |(_, label)| label)
}

/// `{state, label, opens_at?}`. The label of `before_window` names the recorded opening time.
fn display_json(state: &str, opens_at: Option<&str>) -> Value {
    let label = match (state, opens_at) {
        ("before_window", Some(at)) => format!("{}(개시 {})", label_of(state), at.replace('T', " ")),
        _ => label_of(state).to_string(),
    };
    let mut display = json!({"state": state, "label": label});
    if state == "before_window"
        && let Some(at) = opens_at
    {
        display["opens_at"] = json!(at);
    }
    display
}

/// The display state of an exact result. `valid` is the approval being valid right now.
fn display_state(status: &str, reasons: &[String], user_request: bool, valid: bool) -> &'static str {
    let has = |codes: &[&str]| reasons.iter().any(|r| codes.contains(&r.as_str()));
    if user_request {
        return "user_approval_pending";
    }
    match status {
        "handed_off" | "already_handed_off" | "reapproved" | "approved_elsewhere" => "approved",
        "would_approve_and_hand_off" | "would_reapprove" => "auto_approval_pending",
        "submission_exists" if has(SENT_REASONS) => "sent_confirming",
        "submission_exists" => "submitting",
        // Approved, but the session has not been told yet (or it is not known whether it was).
        "would_hand_off" | "session_unavailable" | "approve_refused" | "send_failed" | "handoff_unknown" | "error" => {
            "handoff_attention"
        }
        "not_applicable" => "not_applicable",
        _ if has(INFO_REASONS) => "info_missing",
        _ if has(&["submission_not_open", "submission_opens_at_unreadable"]) => "before_window",
        _ if has(&["deadline_passed"]) => "deadline_passed",
        // The delegation is off or narrower, but a valid approval of the user's own stands.
        "delegation_off" | "out_of_scope" if valid && reasons.len() == 1 => "approved",
        _ => "preparing",
    }
}

/// The readiness reasons the stored records alone give (no file is hashed).
fn stored_readiness_reasons(entry: &Entry) -> Vec<String> {
    let mut reasons: Vec<String> = readiness::record_reasons(entry).iter().map(|r| r.to_string()).collect();
    reasons.extend(rules::approval_problems(entry).iter().map(|r| r.to_string()));
    reasons
}

/// What a person is shown for `entry`, from stored records only: the registry entry, the
/// configuration and the journal. No file is hashed and no backend is asked, so the board and
/// `runtime::summary` can use it; `ach handoff <id> --dry-run` gives the exact result. Returns
/// `{state, label, opens_at?}` with a state of [`STATES`].
pub fn stored_display(cfg: &Config, entry: &Entry, at: DateTime<FixedOffset>) -> Value {
    if entry.state != "awaiting_approval" || entry.source == "legacy" {
        return display_json("not_applicable", None);
    }
    let attempts = attempt_reasons(entry);
    if !attempts.is_empty() {
        let sent = attempts.iter().any(|r| SENT_REASONS.contains(&r.as_str()));
        return display_json(if sent { "sent_confirming" } else { "submitting" }, None);
    }
    let reasons = stored_readiness_reasons(entry);
    if reasons.iter().any(|r| INFO_REASONS.contains(&r.as_str())) {
        return display_json("info_missing", None);
    }
    let token = readiness::request_token(entry);
    let journal = token.as_deref().and_then(|token| load(&entry.id, token).ok().flatten());
    let approval = registry::current_submit_approval(entry);
    let approved = approval.is_some_and(|marker| {
        let bound = marker.extra.get("request_token").and_then(Value::as_str);
        let delegated = marker.extra.get("delegated") == Some(&Value::Bool(true));
        bound.is_some() && bound == token.as_deref() && (!delegated || cfg.delegation.covers(&entry.id))
    });
    if !reasons.is_empty() {
        return display_json("preparing", None);
    }
    if approved {
        let pending = journal.as_ref().is_some_and(|record| record.stage != "sent");
        return display_json(if pending { "handoff_attention" } else { "approved" }, None);
    }
    match window(entry, at) {
        Window::Opens(opens) => return display_json("before_window", Some(&opens)),
        Window::Unreadable => return display_json("before_window", None),
        Window::Open => {}
    }
    if deadline_passed(entry, at) {
        return display_json("deadline_passed", None);
    }
    // A score gate passed by hand is the user's to approve while the override is still what lets it
    // through, unless the delegation exempts the contest from scores anyway (a video with
    // `video_score_exempt`).
    let exempt = cfg.delegation.video_score_exempt && entry.kind.as_deref() == Some("video");
    if !cfg.delegation.covers(&entry.id) || (registry::score_override_needed(entry, false) && !exempt) {
        return display_json("user_approval_pending", None);
    }
    display_json(if journal.is_some() { "handoff_attention" } else { "auto_approval_pending" }, None)
}

/// [`stored_display`] with the configuration as it is now. A configuration that cannot be read
/// counts as no delegation, so nothing is shown as approved or automatic on its strength.
pub fn stored_display_now(entry: &Entry, at: DateTime<FixedOffset>) -> Value {
    if entry.state != "awaiting_approval" {
        return display_json("not_applicable", None);
    }
    match config::load().or_else(|_| serde_json::from_value::<Config>(config::defaults()).map_err(Error::from)) {
        Ok(cfg) => stored_display(&cfg, entry, at),
        Err(_) => display_json("handoff_attention", None),
    }
}

// ----- the run ---------------------------------------------------------------------------------

/// What one run found, before it is turned into the JSON result.
struct Reply<'a> {
    status: &'a str,
    reasons: Vec<String>,
    token: Option<&'a str>,
    user_request: bool,
    record: Option<&'a Record>,
}

/// Facts every result of one run shares.
struct Frame<'a> {
    id: &'a str,
    dry_run: bool,
    /// The approval is valid right now.
    valid: bool,
    /// The recorded opening time when it is still ahead.
    opens_at: Option<String>,
}

impl Frame<'_> {
    /// `{id, status, reasons, request_token, user_approval_request, display, record, dry_run}`.
    fn result(&self, reply: Reply<'_>) -> Value {
        let state = display_state(reply.status, &reply.reasons, reply.user_request, self.valid);
        json!({
            "id": self.id,
            "status": reply.status,
            "reasons": reply.reasons,
            "request_token": reply.token,
            "user_approval_request": reply.user_request,
            "display": display_json(state, self.opens_at.as_deref()),
            "record": reply.record,
            "dry_run": self.dry_run,
        })
    }
}

fn error_result(id: &str, dry_run: bool) -> Value {
    let frame = Frame { id, dry_run, valid: false, opens_at: None };
    frame.result(Reply {
        status: "error",
        reasons: vec!["handoff_error".into()],
        token: None,
        user_request: false,
        record: None,
    })
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
    let status = approval::status(&cfg, &entry)?;
    run_locked(&cfg, &entry, &status, options, true)
}

/// The handoff result for `entry` as `ach approval-status` already computed it (`status`), without
/// asking Orca about the session and without writing anything: what a run would decide from the
/// records and the files, for status output. An eligible request reads `would_approve_and_hand_off`.
pub fn assess(cfg: &Config, entry: &Entry, status: &Value) -> Result<Value> {
    let options = Options { trigger: "status", dry_run: true, resend_request: None, origin_session: None };
    run_locked(cfg, entry, status, &options, false)
}

fn run_locked(cfg: &Config, entry: &Entry, status: &Value, options: &Options<'_>, probe: bool) -> Result<Value> {
    let dry = options.dry_run;
    let id = entry.id.as_str();
    let at = Utc::now().with_timezone(&kst());
    let mut frame = Frame { id, dry_run: dry, valid: false, opens_at: None };
    if entry.state != "awaiting_approval" || entry.source == "legacy" {
        let code = if entry.source == "legacy" { "legacy_entry" } else { "state_not_awaiting_approval" };
        return Ok(frame.result(Reply {
            status: "not_applicable",
            reasons: vec![code.into()],
            token: None,
            user_request: false,
            record: None,
        }));
    }
    let token = status["request_token"].as_str();
    let valid = status["submit_approval_valid"] == true;
    frame.valid = valid;
    let record = match token {
        Some(token) => load(id, token)?,
        None => None,
    };
    let journals = records(id);
    let latest = record.clone().or_else(|| journals.last().cloned());
    let submissions = submission_reasons(entry)?;
    let window = window(entry, at);
    let closed = match &window {
        Window::Open => None,
        Window::Opens(opens) => {
            frame.opens_at = Some(opens.clone());
            Some("submission_not_open")
        }
        Window::Unreadable => Some("submission_opens_at_unreadable"),
    };
    // What stops even the user's own approval (`ach approve`): the core conditions with a recorded
    // score-gate override accepted, plus anything already sent or started.
    let mut human: Vec<String> =
        readiness::evaluate(entry, false, true).reasons.iter().map(|r| r.to_string()).collect();
    human.extend(submissions.iter().cloned());
    // A request is the user's to approve only when nothing else is missing and the window is open.
    let user_request = |policy_blocked: bool| policy_blocked && !valid && human.is_empty() && closed.is_none();
    let done = |status: &str, reasons: Vec<String>, token: Option<&str>, user: bool, record: Option<&Record>| {
        Ok(frame.result(Reply { status, reasons, token, user_request: user, record }))
    };
    let with_window = |mut reasons: Vec<String>| {
        reasons.extend(closed.map(str::to_string));
        reasons
    };

    // The switch: a disabled or narrower delegation stops here, pending handoffs included.
    if !cfg.delegation.enabled || !cfg.delegation.covers(id) {
        let (status, code) = if cfg.delegation.enabled {
            ("out_of_scope", "delegation_not_granted")
        } else {
            ("delegation_off", "delegation_disabled")
        };
        let mut reasons = vec![code.to_string()];
        reasons.extend(human.iter().cloned());
        return done(status, with_window(reasons), token, user_request(true), latest.as_ref());
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
            let mut reasons = status_reasons(status);
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
        let mut reasons = reason_codes(&status_reasons(status));
        let override_only = registry::score_override_needed(entry, true);
        if override_only {
            reasons.push("score_override_requires_user".into());
        }
        return done("waiting", with_window(reasons), token, user_request(override_only), latest.as_ref());
    }
    let Some(token) = token else {
        return done("waiting", vec!["readiness_missing".into()], None, false, None);
    };
    // Before the recorded opening time nothing is approved or handed off; a later run does it.
    if let Some(code) = closed {
        return done("waiting", vec![code.into()], Some(token), false, record.as_ref());
    }
    if deadline_passed(entry, at) {
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

    // The same submission under a new token (a score or the same files recorded again): its
    // message already went out, or may have, so only the approval is recorded for the new token.
    let content = content_hash(entry);
    let earlier = match (&record, &content) {
        (None, Some(content)) if fresh => journals
            .iter()
            .rev()
            .filter(|j| j.request_token != token && j.revision == registry::revision(entry) as u64)
            .find(|j| {
                journal_content(entry, j).as_deref() == Some(content)
                    && matches!(j.stage.as_str(), "sent" | "sending" | "unknown")
            }),
        _ => None,
    };

    let target = match earlier {
        Some(earlier) => earlier.target.clone().unwrap_or(Value::Null),
        None if !probe => Value::Null,
        None => match verify_session(entry) {
            Ok(target) => target,
            Err(code) => return done("session_unavailable", vec![code], Some(token), false, record.as_ref()),
        },
    };
    if dry {
        let status = match (fresh, earlier.is_some()) {
            (_, true) => "would_reapprove",
            (true, false) => "would_approve_and_hand_off",
            (false, false) => "would_hand_off",
        };
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
                content,
                continues: earlier.map(|e| e.request_token.clone()),
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
            let mut result = frame.result(Reply {
                status: "approve_refused",
                reasons: vec!["approval_refused".into()],
                token: Some(token),
                user_request: false,
                record: Some(&record),
            });
            result["detail"] = json!(refusal.0);
            return Ok(result);
        }
    }
    // The approval is valid from here on, whatever happens to the message.
    let frame = Frame { id, dry_run: dry, valid: true, opens_at: None };
    if record.stage == "claimed" {
        record.stage = "approved".into();
        record.approved_at = Some(now());
        record.error = None;
        save(&mut record)?;
    }
    if let Some(earlier) = earlier {
        // No second message. An earlier send that nobody read back stays unknown under this token.
        record.target = Some(target);
        record.sent_at = earlier.sent_at.clone();
        record.message_id = earlier.message_id.clone();
        let delivered = earlier.stage == "sent";
        record.stage = if delivered { "sent".into() } else { "unknown".into() };
        record.error = (!delivered).then(|| "send_outcome_unknown".to_string());
        save(&mut record)?;
        let (status, reasons) = match delivered {
            true => ("reapproved", Vec::new()),
            false => ("handoff_unknown", vec!["send_outcome_unknown".to_string()]),
        };
        return Ok(frame.result(Reply {
            status,
            reasons,
            token: Some(token),
            user_request: false,
            record: Some(&record),
        }));
    }
    send(&mut record, target)?;
    let (status, reasons) = match record.stage.as_str() {
        "sent" => ("handed_off", Vec::new()),
        "send_failed" => ("send_failed", record.error.iter().cloned().collect()),
        _ => ("handoff_unknown", record.error.iter().cloned().collect()),
    };
    Ok(frame.result(Reply { status, reasons, token: Some(token), user_request: false, record: Some(&record) }))
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
/// contest's error is reported in its own result and never stops the others. This is also what
/// re-evaluates a request once its submission window has opened (`ach handoff`, `ach tick`, the
/// approve controller).
pub fn sweep(options: &Options<'_>) -> Result<Value> {
    let ids: Vec<String> = registry::read(|reg| {
        Ok(reg
            .contests
            .values()
            .filter(|entry| entry.state == "awaiting_approval" && entry.source != "legacy")
            .map(|entry| entry.id.clone())
            .collect())
    })?;
    let results: Vec<Value> =
        ids.iter().map(|id| run(id, options).unwrap_or_else(|_| error_result(id, options.dry_run))).collect();
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
    Some(run(id, &options).unwrap_or_else(|_| error_result(id, false)))
}

/// One line for the session that triggered the hook: what happened, what a person is shown, and
/// whether the request waits on a reason or needs the user's own approval.
pub fn describe(result: &Value) -> String {
    let id = result["id"].as_str().unwrap_or("-");
    let status = result["status"].as_str().unwrap_or("error");
    let reasons: Vec<&str> = result["reasons"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    // A question or an action the readiness record asks of the user is still asked: only the
    // approval is not what this request waits for.
    let input = reasons.iter().any(|r| r.ends_with("needs_user_input"));
    let blocked = reasons.iter().any(|r| r.ends_with("blockers_present") || r.ends_with("readiness_not_ready"));
    let reasons = if reasons.is_empty() { "-".to_string() } else { reasons.join(", ") };
    let token = result["request_token"].as_str().unwrap_or("-");
    let label = result["display"]["label"].as_str().unwrap_or("확인 필요");
    let request = if result["user_approval_request"] == true {
        " 사용자 승인 요청 대상이다: 사용자에게 메인 세션이나 ach approve로 승인해 달라고 요청한다."
    } else if input {
        " 사용자 승인 요청이 아니다. 제출 준비 기록에 적은 사용자 입력(질문·조치)이 먼저 필요하다."
    } else if blocked {
        " 사용자 승인 요청이 아니다. 접수 개시 전이라 막힌 것이면 blockers가 아니라 제출 준비 기록의 submission_opens_at에 공고의 개시 일시를 적어 다시 기록한다."
    } else {
        " 사용자 승인 요청이 아니다."
    };
    let text = match status {
        "handed_off" => format!(
            "제출 승인을 기록하고 담당 세션에 인계했다(요청 {token}). ach approval-status {id}로 확인한 뒤 제출 트랜잭션을 진행한다. 사용자에게 승인을 다시 요청하지 않는다."
        ),
        "reapproved" => format!(
            "같은 제출물의 요청 토큰만 바뀌어 승인을 새 토큰({token})으로 다시 기록했다. 인계 메시지는 다시 보내지 않는다. 새 토큰으로 제출 트랜잭션을 이어 간다."
        ),
        "already_handed_off" => format!("이 요청({token})은 이미 승인·인계됐다. 다시 보내지 않는다."),
        "approved_elsewhere" => {
            format!("이 요청({token})의 유효한 승인이 이미 있다(이 경로가 기록한 것이 아니다). 다시 인계하지 않는다.")
        }
        "delegation_off" => format!("상시 위임이 꺼져 있어 멈춘다({reasons}).{request}"),
        "out_of_scope" => format!("이 공모전은 상시 위임 범위 밖이다({reasons}).{request}"),
        "waiting" if result["display"]["state"] == "before_window" => format!(
            "접수 개시 전이라 승인·인계하지 않는다({reasons}). 개시 뒤 ach handoff나 정기 실행(ach tick)이 다시 검사한다. 사용자 승인 요청이 아니다."
        ),
        "waiting" => format!("자동 승인 조건이 맞지 않아 대기한다({reasons}).{request}"),
        "submission_exists" => {
            format!("제출 기록이나 제출 시도가 이미 있어 인계하지 않는다({reasons}). 사용자 승인 요청이 아니다.")
        }
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
        "would_approve_and_hand_off" => {
            format!("자격을 갖췄다(요청 {token}). 실행하면 승인을 기록하고 담당 세션에 인계한다.")
        }
        "would_reapprove" => format!("같은 제출물의 새 토큰({token})이다. 실행하면 승인만 다시 기록한다."),
        "would_hand_off" => format!("승인된 요청({token})의 인계가 남아 있다. 실행하면 담당 세션에 인계한다."),
        "not_applicable" => format!("대상이 아니다({reasons})."),
        _ => format!("인계 경로 오류({reasons}). ach handoff {id} --dry-run으로 확인한다."),
    };
    format!("위임 인계[{status}] {id} ({label}): {text}")
}
