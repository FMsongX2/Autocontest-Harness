//! Submission transactions. The agent performs the authorized form, site or email action itself;
//! the harness only keeps the durable record around it and never retries an uncertain external
//! side effect. One attempt moves `intent → dispatching → confirmed`, or through `unknown` when the
//! response was lost, and only positive evidence (`verified_not_submitted`) reopens a new attempt.
//!
//! - `intent` snapshots the approved request: revision, approval, request token, readiness
//!   destination, artifact hashes and the official-rules binding.
//! - `start` persists `dispatching` before the external action, after checking that snapshot
//!   again; a second start never authorizes another send.
//! - `unknown` records a timeout or lost response; a crash while `dispatching` is treated the same.
//! - `resolve` records positive evidence that nothing was received.
//! - `confirm` (`ach submit <id> --attempt A --file proof.json`) records the factual receipt and the
//!   ledger item together (lock order registry → ledger).
//!
//! Raw proofs stay in the contest folder's `.private/`; verified copies are kept read-only and
//! content-addressed under `.private/submissions/<attempt>/`. Registry metadata keeps the receipt
//! reference, hashes and status codes only. Stored in `Entry.extra.submission_txn`. Contract:
//! `docs/CONTRACT.md`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::bail;
use crate::clean::PRIVATE_DIR;
use crate::error::{Error, Result};
use crate::ledger::{self, Ledger, Submission};
use crate::paths;
use crate::readiness;
use crate::registry::{self, Entry};
use crate::rules;
use crate::store;
use crate::text::{self, now};

/// `Entry.extra` key of the transaction record `{next_attempt, attempts: [...]}`.
pub const KEY: &str = "submission_txn";
/// Attempt states. `expired`: an intent that was never started and whose approval no longer holds.
pub const STATUSES: &[&str] = &["intent", "dispatching", "unknown", "verified_not_submitted", "confirmed", "expired"];
/// `form` and `site` send to an http(s) URL, `email` to an address.
pub const METHODS: &[&str] = &["form", "email", "site"];
pub const UNKNOWN_REASONS: &[&str] = &["timeout", "response_lost", "crash", "ambiguous_response", "other"];
pub const RESOLVE_SOURCES: &[&str] = &["portal_listing", "organizer_reply", "mailbox_search"];
pub const PROOF_SOURCES: &[&str] = &["confirmation_page", "receipt_email", "portal_listing", "organizer_reply"];
/// Verified proof copies: `<folder>/.private/submissions/<attempt>/<sha256>[.ext]`.
pub const PROOF_DIR: &str = "submissions";
const RECEIPT_MAX: usize = 200;
/// States whose external side effect may have happened (or is about to).
const IN_FLIGHT: &[&str] = &["dispatching", "unknown"];

// ---- reading ----

fn txn(entry: &Entry) -> Option<&Value> {
    entry.extra.get(KEY)
}

fn malformed(entry: &Entry) -> bool {
    txn(entry).is_some_and(|t| !t.get("attempts").is_some_and(Value::is_array))
}

fn attempts(entry: &Entry) -> &[Value] {
    txn(entry).and_then(|t| t.get("attempts")).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn status_of(attempt: &Value) -> &str {
    str_of(attempt, "status")
}

fn find<'a>(entry: &'a Entry, attempt: &str) -> Result<&'a Value> {
    if malformed(entry) {
        bail!("{}: {KEY} 기록 형식을 읽을 수 없다. 손으로 고치지 말고 기록을 확인한다", entry.id);
    }
    if !valid_attempt_id(attempt) {
        bail!("{}: 시도 id 형식이 아니다(a<숫자>): {attempt}", entry.id);
    }
    attempts(entry)
        .iter()
        .find(|a| str_of(a, "attempt_id") == attempt)
        .ok_or_else(|| Error(format!("{}: 제출 시도 {attempt}가 없다", entry.id)))
}

fn valid_attempt_id(attempt: &str) -> bool {
    attempt.strip_prefix('a').is_some_and(|n| !n.is_empty() && n.len() <= 9 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Some attempt is `intent`, `dispatching` or `unknown` (or the record is unreadable): no new
/// intent, no recovery and no clean until it is confirmed, resolved or expired.
pub fn unresolved(entry: &Entry) -> bool {
    malformed(entry) || attempts(entry).iter().any(|a| matches!(status_of(a), "intent" | "dispatching" | "unknown"))
}

/// Some attempt may already have reached the organizer (`dispatching` or `unknown`), or the record is
/// unreadable: the workflow must not move on as if nothing had been sent.
pub fn in_flight(entry: &Entry) -> bool {
    malformed(entry) || attempts(entry).iter().any(|a| IN_FLIGHT.contains(&status_of(a)))
}

/// Safe per-attempt summary: ids, states, times, reason codes and hashes. Never the receipt text,
/// the destination or file contents.
fn public_view(attempt: &Value) -> Value {
    let confirmation = attempt.get("confirmation");
    fn hashes(record: Option<&Value>, key: &str) -> Vec<Value> {
        record
            .and_then(|r| r.get(key))
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(|i| i.get("sha256").cloned()).collect())
            .unwrap_or_default()
    }
    json!({
        "attempt_id": attempt.get("attempt_id"),
        "status": attempt.get("status"),
        "method": attempt.get("method"),
        "revision": attempt.pointer("/authorization/revision"),
        "created_at": attempt.get("created_at"),
        "started_at": attempt.get("started_at"),
        "unknown_reason": attempt.pointer("/unknown/reason"),
        "resolved_at": attempt.pointer("/resolution/recorded_at"),
        "confirmed_at": confirmation.and_then(|c| c.get("confirmed_at")),
        "receipt_recorded": confirmation.and_then(|c| c.get("receipt")).and_then(Value::as_str).is_some_and(|r| !r.is_empty()),
        "proof_sha256": hashes(confirmation, "proof"),
        "review_needed": confirmation.and_then(|c| c.get("review_needed")).and_then(Value::as_bool).unwrap_or(false),
        "review_reasons": confirmation.and_then(|c| c.get("review_reasons")).cloned().unwrap_or_else(|| json!([])),
    })
}

/// Pure status for board, alerts, recover and clean: `{tracked, unresolved, in_flight, blockers,
/// review_needed, latest, attempts}`. Blockers are stable codes.
pub fn status(entry: &Entry) -> Value {
    let list: Vec<Value> = attempts(entry).iter().map(public_view).collect();
    let mut blockers: Vec<&'static str> = Vec::new();
    if malformed(entry) {
        blockers.push("submission_record_malformed");
    }
    for attempt in attempts(entry) {
        let code = match status_of(attempt) {
            "intent" => "submission_intent_pending",
            "dispatching" => "submission_dispatching",
            "unknown" => "submission_unknown",
            _ => continue,
        };
        if !blockers.contains(&code) {
            blockers.push(code);
        }
    }
    let review_needed = list.iter().any(|v| v["review_needed"] == true);
    // The state that matters most: an in-flight attempt, then a pending intent, then the latest.
    let current = attempts(entry);
    let state: Option<String> = if malformed(entry) {
        Some("malformed".to_string())
    } else {
        ["unknown", "dispatching", "intent"]
            .into_iter()
            .find(|s| current.iter().any(|a| status_of(a) == *s))
            .map(str::to_string)
            .or_else(|| current.last().map(|a| status_of(a).to_string()))
    };
    json!({
        "tracked": txn(entry).is_some(),
        "state": state,
        "unresolved": unresolved(entry),
        "in_flight": in_flight(entry),
        "blockers": blockers,
        "review_needed": review_needed,
        "latest": list.last(),
        "attempts": list,
    })
}

// ---- writing helpers ----

fn txn_mut(entry: &mut Entry) -> Result<&mut Map<String, Value>> {
    let id = entry.id.clone();
    let slot = entry.extra.entry(KEY.to_string()).or_insert_with(|| json!({"next_attempt": 1, "attempts": []}));
    match slot.as_object_mut() {
        Some(record) if record.get("attempts").is_some_and(Value::is_array) => Ok(record),
        _ => bail!("{id}: {KEY} 기록 형식을 읽을 수 없다. 덮어쓰지 않는다"),
    }
}

fn attempt_mut<'a>(entry: &'a mut Entry, attempt: &str) -> Result<&'a mut Map<String, Value>> {
    let id = entry.id.clone();
    let record = txn_mut(entry)?;
    record
        .get_mut("attempts")
        .and_then(Value::as_array_mut)
        .and_then(|list| list.iter_mut().find(|a| str_of(a, "attempt_id") == attempt))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| Error(format!("{id}: 제출 시도 {attempt}가 없다")))
}

fn push_event(attempt: &mut Map<String, Value>, from: &str, to: &str, reason: Option<&str>) {
    let mut event = json!({"at": now(), "from": from, "to": to});
    if let Some(reason) = reason {
        event["reason"] = json!(reason);
    }
    match attempt.get_mut("events").and_then(Value::as_array_mut) {
        Some(list) => list.push(event),
        None => {
            attempt.insert("events".into(), json!([event]));
        }
    }
}

fn object<'a>(entry: &Entry, raw: &'a Value, what: &str) -> Result<&'a Map<String, Value>> {
    raw.as_object().ok_or_else(|| Error(format!("{}: {what}은(는) JSON 객체여야 한다", entry.id)))
}

fn required_str<'a>(entry: &Entry, raw: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    match raw.get(key).and_then(Value::as_str).map(str::trim) {
        Some(value) if !value.is_empty() => Ok(value),
        _ => bail!("{}: {key}는 비어 있지 않은 문자열이어야 한다", entry.id),
    }
}

fn one_of<'a>(entry: &Entry, raw: &'a Map<String, Value>, key: &str, allowed: &[&str]) -> Result<&'a str> {
    let value = required_str(entry, raw, key)?;
    if !allowed.contains(&value) {
        bail!("{}: {key}는 {} 중 하나여야 한다: {value}", entry.id, allowed.join("|"));
    }
    Ok(value)
}

/// `raw.attempt_id` must name the attempt the command was given.
fn check_attempt_field(entry: &Entry, raw: &Map<String, Value>, attempt: &str) -> Result<()> {
    let given = required_str(entry, raw, "attempt_id")?;
    if given != attempt {
        bail!("{}: 증빙의 attempt_id({given})가 --attempt({attempt})와 다르다", entry.id);
    }
    Ok(())
}

/// A real KST time (not a date), not in the future and, when given, not before `started_at`.
fn observed_at(entry: &Entry, raw: &Map<String, Value>, started: Option<&str>) -> Result<String> {
    let given = required_str(entry, raw, "observed_at")?;
    let Some(stamp) = text::normalize_stamp(given).filter(|s| s.len() > 10) else {
        bail!("{}: observed_at은 YYYY-MM-DDTHH:MM+09:00 형식의 실제 시각이어야 한다: {given}", entry.id)
    };
    if stamp > now() {
        bail!("{}: observed_at이 미래 시각이다: {stamp}", entry.id);
    }
    if let Some(started) = started.filter(|s| !s.is_empty())
        && stamp.as_str() < started
    {
        bail!("{}: observed_at({stamp})이 발송 시작({started})보다 앞서 이 시도의 증빙이 될 수 없다", entry.id);
    }
    Ok(stamp)
}

fn is_email(text: &str) -> bool {
    let Some((local, domain)) = text.split_once('@') else { return false };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains('@')
        && !text.chars().any(|c| c.is_whitespace() || c.is_control() || "<>,;:\"'()[]\\".contains(c))
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_sha(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A fingerprint of the evidence as declared: the same claim twice gives the same value.
fn fingerprint(parts: &[&str], hashes: &[String]) -> String {
    let mut text = parts.join("\n");
    for hash in hashes {
        text.push('\n');
        text.push_str(hash);
    }
    sha256_hex(text.as_bytes())
}

/// Declared artifact hashes, sorted, without reading files (for the repeat check).
fn declared_hashes(entry: &Entry, artifacts: Option<&Value>, required: bool) -> Result<Vec<String>> {
    let items = match artifacts {
        None | Some(Value::Null) if !required => return Ok(Vec::new()),
        Some(Value::Array(items)) if !items.is_empty() || !required => items,
        _ => bail!("{}: artifacts는 비어 있지 않은 [{{path, sha256}}] 배열이어야 한다", entry.id),
    };
    let mut hashes = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let sha = item.get("sha256").and_then(Value::as_str).map(str::to_ascii_lowercase).unwrap_or_default();
        if !valid_sha(&sha) {
            bail!("{}: artifacts[{i}].sha256은 64자리 16진수 SHA-256이어야 한다", entry.id);
        }
        hashes.push(sha);
    }
    hashes.sort();
    Ok(hashes)
}

fn folder(entry: &Entry) -> Result<PathBuf> {
    let Some(raw) = entry.folder.as_deref().filter(|f| !f.is_empty()) else {
        bail!("{}: 공모전 폴더가 기록돼 있지 않다", entry.id)
    };
    PathBuf::from(raw).canonicalize().map_err(|err| Error(format!("{}: 폴더를 열 수 없다: {err}", entry.id)))
}

fn extension(rel: &str) -> String {
    Path::new(rel)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|e| !e.is_empty() && e.len() <= 10 && e.bytes().all(|b| b.is_ascii_alphanumeric()))
        .map(|e| format!(".{e}"))
        .unwrap_or_default()
}

/// Verify evidence files under `.private/` (hash, non-empty, not a submitted artifact) and keep a
/// read-only content-addressed copy of each under `.private/submissions/<attempt>/`. Returns
/// `[{path, sha256}]` of the copies (folder-relative). An existing copy must still hash the same.
fn keep_proofs(entry: &Entry, attempt: &str, artifacts: &Value, submitted: &[String]) -> Result<Vec<Value>> {
    let items = readiness::normalize_artifacts(entry, artifacts)?;
    let root = folder(entry)?;
    let private_prefix = format!("{PRIVATE_DIR}/");
    let dir_rel = format!("{PRIVATE_DIR}/{PROOF_DIR}/{attempt}");
    let dir = root.join(&dir_rel);
    let mut out = Vec::with_capacity(items.len());
    for item in &items {
        let rel = str_of(item, "path");
        let sha = str_of(item, "sha256");
        if !rel.starts_with(&private_prefix) {
            bail!("{}: 증빙 파일은 공모전 폴더의 {PRIVATE_DIR}/ 안에 둔다: {rel}", entry.id);
        }
        if submitted.iter().any(|s| s == sha) {
            bail!("{}: 제출한 파일 자체는 접수·확인 증빙이 아니다: {rel}", entry.id);
        }
        let source = root.join(rel);
        if fs::metadata(&source)?.len() == 0 {
            bail!("{}: 증빙 파일이 비어 있다: {rel}", entry.id);
        }
        let name = format!("{sha}{}", extension(rel));
        let target = dir.join(&name);
        if target.exists() {
            let kept = readiness::sha256_file(&target)?;
            if kept != sha {
                bail!("{}: 보관된 증빙 사본이 바뀌었다({dir_rel}/{name}). 덮어쓰지 않는다", entry.id);
            }
        } else {
            store::private_dir(&dir)?;
            let temp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
            fs::copy(&source, &temp)?;
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o400))?;
            if readiness::sha256_file(&temp)? != sha {
                let _ = fs::remove_file(&temp);
                bail!("{}: 증빙 파일이 복사 중에 바뀌었다: {rel}", entry.id);
            }
            fs::rename(&temp, &target)?;
        }
        out.push(json!({"path": format!("{dir_rel}/{name}"), "sha256": sha}));
    }
    Ok(out)
}

// ---- authorization snapshot ----

/// The destination the approved readiness request names (the original request's for a
/// personalized record).
fn readiness_destination(entry: &Entry, record: &Value) -> Option<String> {
    let own = record.get("destination").and_then(Value::as_str).map(str::trim).filter(|d| !d.is_empty());
    if let Some(own) = own {
        return Some(own.to_string());
    }
    let root_seq = record.get("personalization_root_seq").and_then(Value::as_u64)?;
    let history = entry.extra.get(readiness::HISTORY_KEY).and_then(Value::as_array)?;
    history
        .iter()
        .find(|r| r.get("seq").and_then(Value::as_u64) == Some(root_seq))
        .and_then(|r| r.get("destination"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string)
}

/// What the current approval authorizes, checked now: `awaiting_approval`, a valid submission
/// approval bound to the current request, no pending official-rule problems, a readiness
/// destination and artifacts that still hash as recorded.
fn current_authorization(entry: &Entry) -> Result<Value> {
    if entry.state != "awaiting_approval" {
        bail!("{}: 제출 시도는 awaiting_approval에서만 만든다(지금 {})", entry.id, entry.state);
    }
    if !registry::submit_approved(entry) {
        bail!(
            "{}: 지금 요청에 묶인 유효한 제출 승인이 없다. ach approval-status {}로 확인하고 사용자 승인을 다시 받는다",
            entry.id,
            entry.id
        );
    }
    let problems = rules::approval_problems(entry);
    if !problems.is_empty() {
        bail!("{}: 공식 규칙 검토가 끝나지 않았다({})", entry.id, problems.join(", "));
    }
    let Some(approval) = entry.approvals.iter().rev().find(|a| a.stage == "submit") else {
        bail!("{}: 제출 승인 기록이 없다", entry.id)
    };
    let Some(token) = approval.extra.get("request_token").and_then(Value::as_str).filter(|t| !t.is_empty()) else {
        bail!("{}: 제출 승인에 요청 토큰이 없다", entry.id)
    };
    let Some(record) = readiness::latest(entry) else { bail!("{}: 제출 준비 기록이 없다", entry.id) };
    let Some(destination) = readiness_destination(entry, record) else {
        bail!(
            "{}: 승인된 제출 준비 기록에 destination(접수처)이 없다. destination을 넣은 새 readiness와 새 승인이 필요하다",
            entry.id
        )
    };
    let artifacts = readiness::normalize_artifacts(entry, record.get("artifacts").unwrap_or(&Value::Null))?;
    Ok(json!({
        "revision": registry::revision(entry),
        "request_token": token,
        "readiness_seq": record.get("seq"),
        "approval": {
            "at": approval.at,
            "by": approval.by,
            "delegated": approval.extra.get("delegated") == Some(&Value::Bool(true)),
            "score_basis": approval.extra.get("score_basis"),
        },
        "rules": rules::binding(entry),
        "destination": destination,
        "artifacts": artifacts,
    }))
}

/// The attempt's snapshot still matches what is authorized now.
fn check_still_authorized(entry: &Entry, attempt: &Value) -> Result<()> {
    let current = current_authorization(entry)?;
    let snapshot = attempt.get("authorization").cloned().unwrap_or(Value::Null);
    let mut changed: Vec<&str> = Vec::new();
    for (key, code) in [
        ("revision", "revision_changed"),
        ("request_token", "request_changed"),
        ("readiness_seq", "readiness_changed"),
        ("approval", "approval_changed"),
        ("rules", "rules_changed"),
    ] {
        if snapshot.get(key) != current.get(key) {
            changed.push(code);
        }
    }
    if attempt.get("destination") != current.get("destination") {
        changed.push("destination_changed");
    }
    if attempt.get("artifacts") != current.get("artifacts") {
        changed.push("artifacts_changed");
    }
    if !changed.is_empty() {
        bail!(
            "{}: 시도 {}를 만든 뒤 승인 조건이 바뀌었다({}). 보내지 말고 새 승인과 새 intent를 만든다",
            entry.id,
            str_of(attempt, "attempt_id"),
            changed.join(", ")
        );
    }
    Ok(())
}

fn submitted_hashes(attempt: &Value) -> Vec<String> {
    let mut hashes: Vec<String> = attempt
        .get("artifacts")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(|i| str_of(i, "sha256").to_string()).collect())
        .unwrap_or_default();
    hashes.sort();
    hashes
}

// ---- intent / start / unknown / resolve ----

/// Record the intent to submit (`ach submission intent <id> --file F`). `raw`: `{method:
/// form|email|site, destination, expected_request}`. `destination` must equal the approved
/// readiness destination and `expected_request` the approval's request token. An existing intent
/// for the same exact request is returned unchanged (`created: false`); a different one that is
/// still authorized is refused; a never-started intent whose approval no longer holds expires.
/// Any `dispatching` or `unknown` attempt refuses a new intent. The delegation policy is checked
/// by the CLI first.
pub fn intent(entry: &mut Entry, raw: &Value) -> Result<Value> {
    let raw_obj = object(entry, raw, "제출 intent")?;
    let method = one_of(entry, raw_obj, "method", METHODS)?.to_string();
    let destination = required_str(entry, raw_obj, "destination")?.to_string();
    let expected = required_str(entry, raw_obj, "expected_request")?.to_string();
    let fits = if method == "email" { is_email(&destination) } else { text::is_web_url(&destination) };
    if !fits {
        bail!("{}: method {method}에 맞지 않는 destination이다(form/site는 http(s) 주소, email은 메일 주소)", entry.id);
    }
    if malformed(entry) {
        bail!("{}: {KEY} 기록 형식을 읽을 수 없다. 새 시도를 만들지 않는다", entry.id);
    }
    if let Some(open) = attempts(entry).iter().find(|a| IN_FLIGHT.contains(&status_of(a))) {
        bail!(
            "{}: 이미 보냈을 수 있는 시도 {}({})가 있다. 접수 증빙(ach submit) 또는 미제출 확인(ach submission resolve) 전에는 새 시도를 만들지 않는다",
            entry.id,
            str_of(open, "attempt_id"),
            status_of(open)
        );
    }
    let current = current_authorization(entry)?;
    if current["request_token"] != expected.as_str() {
        bail!(
            "{}: expected_request({expected})가 지금 승인된 요청 토큰({})과 다르다",
            entry.id,
            current["request_token"].as_str().unwrap_or("-")
        );
    }
    if current["destination"] != destination.as_str() {
        bail!("{}: destination이 승인된 제출 준비 기록의 접수처와 정확히 같지 않다", entry.id);
    }
    let mut expire: Vec<String> = Vec::new();
    for pending in attempts(entry).iter().filter(|a| status_of(a) == "intent") {
        if check_still_authorized(entry, pending).is_ok() {
            if str_of(pending, "method") == method
                && str_of(pending, "destination") == destination
                && str_of(pending, "expected_request") == expected
            {
                return Ok(json!({"created": false, "attempt": pending.clone()}));
            }
            bail!(
                "{}: 아직 시작하지 않은 다른 제출 시도 {}가 있다(같은 요청이면 같은 내용으로 다시 실행한다)",
                entry.id,
                str_of(pending, "attempt_id")
            );
        }
        expire.push(str_of(pending, "attempt_id").to_string());
    }
    for old in &expire {
        let attempt = attempt_mut(entry, old)?;
        attempt.insert("status".into(), json!("expired"));
        push_event(attempt, "intent", "expired", Some("authorization_changed"));
    }
    let used = attempts(entry)
        .iter()
        .filter_map(|a| str_of(a, "attempt_id").strip_prefix('a').and_then(|n| n.parse::<u64>().ok()))
        .max()
        .unwrap_or(0);
    let record = txn_mut(entry)?;
    let n = record.get("next_attempt").and_then(Value::as_u64).unwrap_or(1).max(used + 1);
    record.insert("next_attempt".into(), json!(n + 1));
    let mut authorization = current;
    let artifacts = authorization.as_object_mut().and_then(|a| a.remove("artifacts")).unwrap_or(Value::Null);
    if let Some(a) = authorization.as_object_mut() {
        a.remove("destination");
    }
    let created_at = now();
    let attempt = json!({
        "attempt_id": format!("a{n}"),
        "status": "intent",
        "method": method,
        "destination": destination,
        "expected_request": expected,
        "created_at": created_at,
        "authorization": authorization,
        "artifacts": artifacts,
        "events": [{"at": created_at, "from": null, "to": "intent"}],
    });
    if let Some(list) = record.get_mut("attempts").and_then(Value::as_array_mut) {
        list.push(attempt.clone());
    }
    Ok(json!({"created": true, "expired": expire, "attempt": attempt}))
}

/// Move an intent to `dispatching` (`ach submission start <id> --attempt A`), persisted before the
/// agent performs the external action. The snapshot is checked again first. Starting an attempt
/// that already started never authorizes another send: it fails.
pub fn start(entry: &mut Entry, attempt: &str) -> Result<Value> {
    let current = find(entry, attempt)?;
    match status_of(current) {
        "intent" => {}
        status @ ("dispatching" | "unknown" | "confirmed") => bail!(
            "{}: 시도 {attempt}는 이미 시작됐다({status}). 다시 보내지 않는다. 접수 여부를 확인해 ach submit, ach submission unknown 또는 resolve로 기록한다",
            entry.id
        ),
        status => bail!("{}: 시도 {attempt}는 {status}라 시작할 수 없다. 새 intent를 만든다", entry.id),
    }
    if let Some(open) = attempts(entry).iter().find(|a| IN_FLIGHT.contains(&status_of(a))) {
        bail!("{}: 이미 보냈을 수 있는 시도 {}가 있다", entry.id, str_of(open, "attempt_id"));
    }
    check_still_authorized(entry, current)?;
    let started_at = now();
    let record = attempt_mut(entry, attempt)?;
    record.insert("status".into(), json!("dispatching"));
    record.insert("started_at".into(), json!(started_at));
    push_event(record, "intent", "dispatching", None);
    Ok(Value::Object(record.clone()))
}

/// Record that the outcome of a dispatched attempt is not known (`ach submission unknown <id>
/// --attempt A --file F`). `raw`: `{attempt_id, reason, observed_at, evidence?: [{path, sha256}]}`
/// with evidence files under `.private/`. Only from `dispatching`; the exact same record again
/// changes nothing. Nothing is retried.
pub fn unknown(entry: &mut Entry, attempt: &str, raw: &Value) -> Result<Value> {
    let current = find(entry, attempt)?.clone();
    let raw_obj = object(entry, raw, "unknown 기록")?;
    check_attempt_field(entry, raw_obj, attempt)?;
    let reason = one_of(entry, raw_obj, "reason", UNKNOWN_REASONS)?.to_string();
    let declared = declared_hashes(entry, raw_obj.get("evidence"), false)?;
    let observed_raw = text::normalize_stamp(required_str(entry, raw_obj, "observed_at")?).unwrap_or_default();
    let print = fingerprint(&[attempt, "unknown", &reason, &observed_raw], &declared);
    match status_of(&current) {
        "dispatching" => {}
        "unknown" if current.pointer("/unknown/fingerprint").and_then(Value::as_str) == Some(print.as_str()) => {
            return Ok(current);
        }
        status => bail!("{}: 시도 {attempt}는 {status}라 unknown으로 기록할 수 없다(dispatching만)", entry.id),
    }
    let observed = observed_at(entry, raw_obj, current.get("started_at").and_then(Value::as_str))?;
    let evidence = match raw_obj.get("evidence") {
        None | Some(Value::Null) => Vec::new(),
        Some(items) if items.as_array().is_some_and(Vec::is_empty) => Vec::new(),
        Some(items) => keep_proofs(entry, attempt, items, &submitted_hashes(&current))?,
    };
    let record = attempt_mut(entry, attempt)?;
    record.insert("status".into(), json!("unknown"));
    record.insert(
        "unknown".into(),
        json!({"reason": reason, "observed_at": observed, "recorded_at": now(), "evidence": evidence, "fingerprint": print}),
    );
    push_event(record, "dispatching", "unknown", Some(&reason));
    Ok(Value::Object(record.clone()))
}

/// Record positive evidence that a dispatched attempt was not received (`ach submission resolve
/// <id> --attempt A --file F`). `raw`: `{attempt_id, verified_not_submitted: true, source,
/// observed_at, artifacts: [{path, sha256}]}`; the evidence must be observed after the dispatch
/// started and its files live under `.private/`. From `dispatching` or `unknown`; only this
/// reopens a new intent.
pub fn resolve(entry: &mut Entry, attempt: &str, raw: &Value) -> Result<Value> {
    let current = find(entry, attempt)?.clone();
    let raw_obj = object(entry, raw, "미제출 확인")?;
    check_attempt_field(entry, raw_obj, attempt)?;
    if raw_obj.get("verified_not_submitted") != Some(&Value::Bool(true)) {
        bail!("{}: verified_not_submitted: true인 확인 증빙만 시도를 닫는다", entry.id);
    }
    let source = one_of(entry, raw_obj, "source", RESOLVE_SOURCES)?.to_string();
    let declared = declared_hashes(entry, raw_obj.get("artifacts"), true)?;
    let observed_raw = text::normalize_stamp(required_str(entry, raw_obj, "observed_at")?).unwrap_or_default();
    let print = fingerprint(&[attempt, "verified_not_submitted", &source, &observed_raw], &declared);
    let from = status_of(&current).to_string();
    match from.as_str() {
        "dispatching" | "unknown" => {}
        "verified_not_submitted"
            if current.pointer("/resolution/fingerprint").and_then(Value::as_str) == Some(print.as_str()) =>
        {
            return Ok(current);
        }
        status => bail!("{}: 시도 {attempt}는 {status}라 미제출로 닫을 수 없다(dispatching, unknown만)", entry.id),
    }
    let observed = observed_at(entry, raw_obj, current.get("started_at").and_then(Value::as_str))?;
    let evidence =
        keep_proofs(entry, attempt, raw_obj.get("artifacts").unwrap_or(&Value::Null), &submitted_hashes(&current))?;
    let record = attempt_mut(entry, attempt)?;
    record.insert("status".into(), json!("verified_not_submitted"));
    record.insert(
        "resolution".into(),
        json!({"source": source, "observed_at": observed, "recorded_at": now(), "evidence": evidence, "fingerprint": print}),
    );
    push_event(record, &from, "verified_not_submitted", Some(&source));
    Ok(Value::Object(record.clone()))
}

// ---- confirm ----

/// The receipt proof as declared, validated without reading files.
struct Proof {
    receipt: String,
    source: String,
    observed: String,
    destination: String,
    submitted: Vec<String>,
    fingerprint: String,
}

fn declared_proof(entry: &Entry, attempt: &str, raw: &Value) -> Result<Proof> {
    let raw_obj = object(entry, raw, "제출 증빙")?;
    check_attempt_field(entry, raw_obj, attempt)?;
    let receipt = text::one_line(required_str(entry, raw_obj, "receipt")?);
    if receipt.is_empty() || receipt.chars().count() > RECEIPT_MAX {
        bail!("{}: receipt는 {RECEIPT_MAX}자 이하의 접수번호·접수 참조 한 줄이어야 한다", entry.id);
    }
    let source = one_of(entry, raw_obj, "source", PROOF_SOURCES)?.to_string();
    let observed_raw = required_str(entry, raw_obj, "observed_at")?;
    let Some(observed) = text::normalize_stamp(observed_raw).filter(|s| s.len() > 10) else {
        bail!("{}: observed_at은 YYYY-MM-DDTHH:MM+09:00 형식의 실제 시각이어야 한다: {observed_raw}", entry.id)
    };
    let destination = required_str(entry, raw_obj, "destination")?.to_string();
    let Some(list) = raw_obj.get("submitted_sha256").and_then(Value::as_array).filter(|l| !l.is_empty()) else {
        bail!("{}: submitted_sha256는 보낸 파일 SHA-256의 비어 있지 않은 배열이어야 한다", entry.id)
    };
    let mut submitted = Vec::with_capacity(list.len());
    for item in list {
        let sha = item.as_str().map(str::to_ascii_lowercase).unwrap_or_default();
        if !valid_sha(&sha) || submitted.contains(&sha) {
            bail!("{}: submitted_sha256 항목은 중복 없는 64자리 SHA-256이어야 한다", entry.id);
        }
        submitted.push(sha);
    }
    submitted.sort();
    let proofs = declared_hashes(entry, raw_obj.get("artifacts"), true)?;
    let mut all = submitted.clone();
    all.push("--".into());
    all.extend(proofs);
    let fingerprint = fingerprint(&[attempt, "confirmed", &receipt, &source, &observed, &destination], &all);
    Ok(Proof { receipt, source, observed, destination, submitted, fingerprint })
}

/// What `confirm` stores in the ledger for a confirmed attempt.
fn ledger_item(entry: &Entry, attempt: &Value) -> Submission {
    let confirmation = attempt.get("confirmation").cloned().unwrap_or(Value::Null);
    let files: Vec<String> = attempt
        .get("artifacts")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(|i| str_of(i, "path").to_string()).collect())
        .unwrap_or_default();
    let mut item = ledger::from_submit(entry, str_of(attempt, "method"), str_of(&confirmation, "receipt"), &files, "");
    item.submitted_at = str_of(&confirmation, "observed_at").to_string();
    let proof: Vec<Value> = confirmation
        .get("proof")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(|i| i.get("sha256").cloned()).collect())
        .unwrap_or_default();
    item.extra.insert("attempt_id".into(), attempt.get("attempt_id").cloned().unwrap_or(Value::Null));
    item.extra.insert("destination".into(), attempt.get("destination").cloned().unwrap_or(Value::Null));
    item.extra.insert("proof_source".into(), confirmation.get("source").cloned().unwrap_or(Value::Null));
    item.extra.insert("proof_sha256".into(), Value::Array(proof));
    item.extra.insert("proof_fingerprint".into(), confirmation.get("fingerprint").cloned().unwrap_or(Value::Null));
    item.extra.insert("confirmed_at".into(), confirmation.get("confirmed_at").cloned().unwrap_or(Value::Null));
    item
}

fn round_of(item: &Submission) -> Value {
    let mut round = json!({
        "submitted_at": item.submitted_at,
        "method": item.method,
        "receipt": item.receipt,
        "files": item.files,
    });
    for (key, value) in &item.extra {
        round[key.as_str()] = value.clone();
    }
    round
}

/// Put the confirmed attempt into the ledger without losing anything: a new key is added; an
/// item already holding this attempt must carry the same evidence (then nothing changes); any
/// other item under the key (an earlier round, a manual or imported record) keeps every field and
/// gets the attempt appended to `extra.rounds`. Returns `true` when the ledger changed.
fn record_ledger(ledger: &mut Ledger, item: Submission) -> Result<bool> {
    let attempt = item.extra.get("attempt_id").cloned().unwrap_or(Value::Null);
    let print = item.extra.get("proof_fingerprint").cloned().unwrap_or(Value::Null);
    let message = format!("{}: 원장에 같은 시도 {attempt}가 다른 증빙으로 이미 있다. 덮어쓰지 않는다", item.key);
    let conflict = move || Error(message);
    let Some(existing) = ledger.submissions.iter_mut().find(|s| s.key == item.key) else {
        ledger.submissions.push(item);
        return Ok(true);
    };
    if existing.extra.get("attempt_id") == Some(&attempt) {
        return if existing.extra.get("proof_fingerprint") == Some(&print) { Ok(false) } else { Err(conflict()) };
    }
    let rounds = existing.extra.entry("rounds".to_string()).or_insert_with(|| Value::Array(Vec::new()));
    let Some(list) = rounds.as_array_mut() else {
        bail!("{}: 원장 항목의 rounds 형식을 읽을 수 없다. 덮어쓰지 않는다", item.key)
    };
    if let Some(round) = list.iter().find(|r| r.get("attempt_id") == Some(&attempt)) {
        return if round.get("proof_fingerprint") == Some(&print) { Ok(false) } else { Err(conflict()) };
    }
    list.push(round_of(&item));
    Ok(true)
}

/// Confirm an attempt on the entry (no ledger). Returns `(result, ledger item, repeated)`.
fn confirm_entry(entry: &mut Entry, attempt: &str, raw: &Value) -> Result<(Value, Submission, bool)> {
    let current = find(entry, attempt)?.clone();
    let proof = declared_proof(entry, attempt, raw)?;
    let from = status_of(&current).to_string();
    match from.as_str() {
        "confirmed" => {
            if current.pointer("/confirmation/fingerprint").and_then(Value::as_str) != Some(proof.fingerprint.as_str())
            {
                bail!("{}: 시도 {attempt}는 다른 증빙으로 이미 확인됐다. 증빙을 바꾸지 않는다", entry.id);
            }
            let item = ledger_item(entry, &current);
            return Ok((current, item, true));
        }
        "dispatching" | "unknown" | "verified_not_submitted" => {}
        "intent" => bail!(
            "{}: 시도 {attempt}는 시작(ach submission start)되지 않았다. 보내지 않은 시도는 제출로 확인하지 않는다",
            entry.id
        ),
        status => bail!("{}: 시도 {attempt}는 {status}라 제출로 확인할 수 없다", entry.id),
    }
    let raw_obj = object(entry, raw, "제출 증빙")?;
    observed_at(entry, raw_obj, current.get("started_at").and_then(Value::as_str))?;
    if proof.destination != str_of(&current, "destination") {
        bail!("{}: 증빙의 destination이 시도 {attempt}의 승인된 접수처와 다르다", entry.id);
    }
    let submitted = submitted_hashes(&current);
    if proof.submitted != submitted {
        bail!("{}: submitted_sha256가 시도 {attempt}에 묶인 승인 파일과 다르다", entry.id);
    }
    let kept = keep_proofs(entry, attempt, raw_obj.get("artifacts").unwrap_or(&Value::Null), &submitted)?;
    let authorized_revision = current.pointer("/authorization/revision").and_then(Value::as_u64);
    let mut reasons: Vec<&'static str> = Vec::new();
    if from == "verified_not_submitted" {
        reasons.push("contradicts_verified_not_submitted");
    }
    if attempts(entry).iter().any(|a| status_of(a) == "confirmed") {
        reasons.push("multiple_confirmed");
    }
    let workflow_current =
        entry.state == "awaiting_approval" && authorized_revision == Some(registry::revision(entry) as u64);
    if !workflow_current {
        reasons.push("workflow_changed");
    }
    let confirmed_at = now();
    let record = attempt_mut(entry, attempt)?;
    record.insert("status".into(), json!("confirmed"));
    record.insert(
        "confirmation".into(),
        json!({
            "receipt": proof.receipt,
            "source": proof.source,
            "observed_at": proof.observed,
            "confirmed_at": confirmed_at,
            "proof": kept,
            "fingerprint": proof.fingerprint,
            "review_needed": !reasons.is_empty(),
            "review_reasons": reasons,
        }),
    );
    push_event(record, &from, "confirmed", Some(&proof.source));
    let confirmed = Value::Object(record.clone());
    let files: Vec<String> = submitted_files(&confirmed);
    let proof_hashes: Vec<Value> = kept.iter().filter_map(|k| k.get("sha256").cloned()).collect();
    entry.submissions.push(json!({
        "at": proof.observed,
        "method": confirmed["method"],
        "receipt": confirmed["confirmation"]["receipt"],
        "files": files,
        "attempt_id": attempt,
        "proof_sha256": proof_hashes,
    }));
    if workflow_current {
        registry::transition(entry, "submitted", "agent", &format!("제출 확인: 시도 {attempt}"))?;
    }
    let item = ledger_item(entry, &confirmed);
    Ok((confirmed, item, false))
}

fn submitted_files(attempt: &Value) -> Vec<String> {
    attempt
        .get("artifacts")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(|i| str_of(i, "path").to_string()).collect())
        .unwrap_or_default()
}

/// Confirm a dispatched attempt with its receipt (`ach submit <id> --attempt A --file proof.json`)
/// against the default registry and ledger.
pub fn confirm(id: &str, attempt: &str, raw: &Value) -> Result<Value> {
    confirm_at(&paths::registry(), &ledger::path(), id, attempt, raw)
}

/// `confirm` on explicit registry and ledger paths. `raw`: `{attempt_id, receipt, source,
/// observed_at, destination, submitted_sha256: [...], artifacts: [{path, sha256}]}` with the
/// receipt, email or listing files under `.private/`. The proof is checked for form and binding
/// only: no claim is made that its content was read. Allowed from `dispatching`, `unknown` and
/// (flagged for review) `verified_not_submitted`; never from `intent`. The registry and the ledger
/// change together (the ledger is written inside the registry write). When the workflow moved on
/// since the approval (another state or revision), the receipt and the ledger item are still
/// recorded, the state is left alone and `review_needed` is set. The exact same proof again
/// changes nothing except restoring a lost ledger item; a different proof is refused.
pub fn confirm_at(registry_path: &Path, ledger_path: &Path, id: &str, attempt: &str, raw: &Value) -> Result<Value> {
    registry::write_at(registry_path, |reg| {
        let entry = reg.get_mut(id)?;
        let (confirmed, item, repeated) = confirm_entry(entry, attempt, raw)?;
        let ledger_added = ledger::write_at(ledger_path, |l| record_ledger(l, item))?;
        let view = public_view(&confirmed);
        Ok(json!({
            "id": id,
            "attempt": view,
            "repeated": repeated,
            "ledger_changed": ledger_added,
            "review_needed": view["review_needed"],
            "workflow_state": entry.state,
        }))
    })
}
