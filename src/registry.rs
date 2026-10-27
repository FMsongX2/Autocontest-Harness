//! The contest registry (`data/registry.json`): one entry per contest, a state machine over it, and
//! the gate record. Only the CLI writes it. Contract: `docs/CONTRACT.md` sections 3-4.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::bail;
use crate::error::{Error, Result};
use crate::paths;
use crate::readiness;
use crate::store;
use crate::text::{self, now};

pub const TYPES: &[&str] = &[
    "video", "image", "music", "idea", "paper", "report", "model", "data", "software", "essay", "design", "research",
    "other",
];
/// 1 is the smallest (a short-form video, a poster), 5 the largest (a month-long production build).
pub const LEVELS: [u8; 5] = [1, 2, 3, 4, 5];
pub const STATES: &[&str] = &[
    "discovered",
    "gate_pass",
    "rejected",
    "hold",
    "routed",
    "running",
    "plan_review",
    "awaiting_approval",
    "submitted",
    "followup_pending",
    "closed",
    "skipped",
    "duplicate",
];
/// States that occupy a session slot (counted against the concurrency limits).
pub const IN_PROGRESS: &[&str] = &["running", "plan_review"];

/// Allowed moves out of each state.
pub fn allowed(from: &str) -> &'static [&'static str] {
    match from {
        "discovered" => &["gate_pass", "rejected", "hold", "skipped", "duplicate"],
        "hold" => &["gate_pass", "rejected", "skipped", "duplicate"],
        "rejected" => &["hold", "skipped"],
        "gate_pass" => &["routed", "hold", "rejected", "skipped", "duplicate"],
        "routed" => &["running", "skipped"],
        "running" => &["plan_review", "awaiting_approval", "followup_pending", "closed", "skipped"],
        "plan_review" => &["running", "skipped"],
        "awaiting_approval" => &["running", "submitted", "skipped"],
        "submitted" => &["closed", "running", "followup_pending"],
        "followup_pending" => &["running", "closed"],
        "skipped" | "duplicate" => &["discovered"],
        _ => &[],
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryItem {
    pub at: String,
    pub from: Option<String>,
    pub to: String,
    pub by: String,
    #[serde(default)]
    pub note: String,
    /// Fields this version does not know about are kept as they are.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl HistoryItem {
    pub fn new(at: String, from: Option<String>, to: &str, by: &str, note: &str) -> Self {
        HistoryItem { at, from, to: to.to_string(), by: by.to_string(), note: note.to_string(), extra: Map::new() }
    }
}

/// The stages a human approves: the level 5 plan (`plan_review → running`) and a submission.
pub const APPROVAL_STAGES: &[&str] = &["plan", "submit"];

/// One human approval (`ach approve`): the user's words, verbatim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    pub at: String,
    /// `plan` or `submit`.
    pub stage: String,
    pub note: String,
    pub by: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub organizer: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub alt_urls: Vec<String>,
    pub source: String,
    pub deadline: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    pub discovered_at: String,
    pub state: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub level: Option<u8>,
    #[serde(default)]
    pub priority: i64,
    pub gate: Option<Value>,
    pub folder: Option<String>,
    pub repo: Option<String>,
    /// Where the contest session runs: `{workspace, workspace_id, runner, handle}`.
    #[serde(default)]
    pub session: Option<Value>,
    #[serde(default)]
    pub submissions: Vec<Value>,
    /// ContestJudge scores: `{at, stage, total, target, verdict, criteria, note, file}`.
    #[serde(default)]
    pub scores: Vec<Value>,
    #[serde(default)]
    pub history: Vec<HistoryItem>,
    /// Human approvals recorded by `ach approve` (plan review, submission).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approvals: Vec<Approval>,
    /// Fields this version does not know about are kept as they are.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The newest registry format this build reads and writes.
pub const VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Registry {
    pub version: u32,
    pub contests: IndexMap<String, Entry>,
    /// Top-level fields this version does not know about are kept as they are.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Registry {
    fn default() -> Self {
        Registry { version: VERSION, contests: IndexMap::new(), extra: Map::new() }
    }
}

impl Registry {
    pub fn get(&self, id: &str) -> Result<&Entry> {
        self.contests.get(id).ok_or_else(|| Error(format!("레지스트리에 없는 id: {id}")))
    }

    pub fn get_mut(&mut self, id: &str) -> Result<&mut Entry> {
        self.contests.get_mut(id).ok_or_else(|| Error(format!("레지스트리에 없는 id: {id}")))
    }

    /// In-progress contests per level, excluding legacy imports (they run outside the harness).
    /// Entries without a level count under 0: they use the global limit but no level limit.
    pub fn running_by_level(&self) -> BTreeMap<u8, usize> {
        let mut counts: BTreeMap<u8, usize> = LEVELS.iter().map(|&l| (l, 0)).collect();
        for entry in self.contests.values() {
            if IN_PROGRESS.contains(&entry.state.as_str()) && entry.source != "legacy" {
                *counts.entry(entry.level.unwrap_or(0)).or_default() += 1;
            }
        }
        counts
    }
}

fn load(path: &Path) -> Result<Registry> {
    let Some(text) = store::read(path)? else { return Ok(Registry::default()) };
    store::check_version(path, &text, VERSION)?;
    serde_json::from_str(&text).map_err(|err| Error(format!("{}: {err}", path.display())))
}

fn save(path: &Path, registry: &Registry) -> Result<()> {
    let mut text = serde_json::to_string_pretty(registry)?;
    text.push('\n');
    store::write_atomic(path, text.as_bytes(), true)
}

/// Read the registry under a shared lock.
pub fn read<T>(f: impl FnOnce(&Registry) -> Result<T>) -> Result<T> {
    read_at(&paths::registry(), f)
}

/// Modify the registry under an exclusive lock. It is saved (atomically) only when `f` succeeds.
pub fn write<T>(f: impl FnOnce(&mut Registry) -> Result<T>) -> Result<T> {
    write_at(&paths::registry(), f)
}

/// Without a data directory there is nothing to lock and the registry is empty; reading never creates it.
pub fn read_at<T>(path: &Path, f: impl FnOnce(&Registry) -> Result<T>) -> Result<T> {
    let Some(lock) = store::lock_file(path, false)? else { return f(&Registry::default()) };
    lock.lock_shared()?;
    f(&load(path)?)
}

pub fn write_at<T>(path: &Path, f: impl FnOnce(&mut Registry) -> Result<T>) -> Result<T> {
    let Some(lock) = store::lock_file(path, true)? else {
        bail!("{}: 잠금 파일을 만들 수 없다", path.display())
    };
    lock.lock()?;
    let mut registry = load(path)?;
    let value = f(&mut registry)?;
    save(path, &registry)?;
    Ok(value)
}

/// Serialises session launches (`launch`, `tick` with launching) across processes, so the
/// capacity check and the stored result of one launch are never interleaved with another's. Held
/// while the launch runs; the registry lock is not, so `transition`, `show` and others keep working.
pub fn launch_guard() -> Result<File> {
    launch_guard_at(&paths::data().join("launch.lock"))
}

pub fn launch_guard_at(path: &Path) -> Result<File> {
    let Some(lock) = store::lock_file(path, true)? else {
        bail!("{}: 잠금 파일을 만들 수 없다", path.display())
    };
    lock.lock()?;
    Ok(lock)
}

/// Store work done on a detached copy (`base` cloned from the registry, `work` changed by route or
/// launch without the registry lock) onto `current`, the entry as it is now. Fails, leaving
/// `current` alone, when another command moved the entry meanwhile (state or history changed).
/// Fields the work did not change keep their current value (a concurrent collect or score stays);
/// history, submissions and scores get the work's new items appended.
pub fn merge_work(current: &mut Entry, base: &Entry, work: Entry) -> Result<()> {
    if current.state != base.state || current.history.len() != base.history.len() {
        bail!(
            "{}: 작업하는 동안 다른 명령이 이 항목을 바꿨다(지금 {}, 시작할 때 {}). 결과를 저장하지 않았다. ach show {}로 확인하고 다시 실행한다",
            current.id,
            current.state,
            base.state,
            current.id
        );
    }
    let Entry {
        id: _,
        title,
        organizer,
        url,
        alt_urls,
        source,
        deadline,
        categories,
        discovered_at: _,
        state,
        kind,
        level,
        priority,
        gate,
        folder,
        repo,
        session,
        submissions,
        scores,
        history,
        approvals,
        extra,
    } = work;
    macro_rules! changed {
        ($($field:ident),*) => {$(
            if $field != base.$field {
                current.$field = $field;
            }
        )*};
    }
    changed!(title, organizer, url, alt_urls, source, deadline, categories, state, kind, level, priority, gate);
    changed!(folder, repo, session);
    for (list, base_len, target) in [
        (submissions, base.submissions.len(), &mut current.submissions),
        (scores, base.scores.len(), &mut current.scores),
    ] {
        target.extend(list.into_iter().skip(base_len));
    }
    current.history.extend(history.into_iter().skip(base.history.len()));
    current.approvals.extend(approvals.into_iter().skip(base.approvals.len()));
    for (key, value) in extra {
        if base.extra.get(&key) != Some(&value) {
            current.extra.insert(key, value);
        }
    }
    Ok(())
}

pub struct NewEntry<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub url: &'a str,
    pub source: &'a str,
    pub organizer: &'a str,
    pub deadline: Option<String>,
    pub categories: Vec<String>,
    pub note: Option<String>,
}

pub fn new_entry(spec: NewEntry<'_>) -> Entry {
    let at = now();
    Entry {
        id: spec.id.to_string(),
        title: text::one_line(spec.title),
        organizer: text::one_line(spec.organizer),
        url: spec.url.trim().to_string(),
        alt_urls: Vec::new(),
        source: spec.source.to_string(),
        deadline: spec.deadline,
        categories: spec.categories,
        discovered_at: at.clone(),
        state: "discovered".to_string(),
        kind: None,
        level: None,
        priority: 0,
        gate: None,
        folder: None,
        repo: None,
        session: None,
        submissions: Vec::new(),
        scores: Vec::new(),
        history: vec![HistoryItem::new(at, None, "discovered", spec.source, &spec.note.unwrap_or_default())],
        approvals: Vec::new(),
        extra: Map::new(),
    }
}

/// A human approved the level 5 plan (`ach approve` during `plan_review`).
pub fn plan_approved(entry: &Entry) -> bool {
    entry.approvals.iter().any(|a| a.stage == "plan")
}

/// The current work revision: the history index of the latest move into `running` from another
/// state (0 when the entry never ran). Final scores, readiness records and submission approvals
/// are stamped with it; re-entering `running` (a revise cycle, a follow-up after submission)
/// makes them stale. Same-state markers (approvals, restarts) never change it.
pub fn revision(entry: &Entry) -> usize {
    entry.history.iter().rposition(|h| h.to == "running" && h.from.as_deref() != Some("running")).unwrap_or(0)
}

/// The history marker `approve` adds for a submission approval (`extra.approval = "submit"`).
fn is_submit_approval(h: &HistoryItem) -> bool {
    h.extra.get("approval").and_then(Value::as_str) == Some("submit")
}

/// Index of the latest real move into `awaiting_approval` (approval markers stay in that state).
fn entered_awaiting(entry: &Entry) -> Option<usize> {
    entry.history.iter().rposition(|h| h.to == "awaiting_approval" && h.from.as_deref() != Some("awaiting_approval"))
}

/// The move into the current `awaiting_approval` passed the score gate on a human `--score-ok`
/// reason: the CLI marks that history item with `score_override: true` and
/// `score_override_revision` (the work revision then). Only plain `approve` accepts it.
pub fn score_override_recorded(entry: &Entry) -> bool {
    if entry.state != "awaiting_approval" {
        return false;
    }
    let Some(item) = entered_awaiting(entry).map(|i| &entry.history[i]) else { return false };
    item.extra.get("score_override").and_then(Value::as_bool) == Some(true)
        && item.extra.get("score_override_revision").and_then(Value::as_u64) == Some(revision(entry) as u64)
}

/// The submission is approved for the work as it is now: the latest `submit` approval recorded
/// after the latest move into `awaiting_approval` is bound to the current work revision and the
/// current request token (revision, readiness sequence, score count), and the readiness record,
/// its files and the score it relied on still check out. Approvals without that binding (older
/// formats) never count. Only `awaiting_approval` entries can be approved.
pub fn submit_approved(entry: &Entry) -> bool {
    if entry.state != "awaiting_approval" {
        return false;
    }
    let start = entered_awaiting(entry).map_or(0, |i| i + 1);
    let Some(marker) = entry.history.iter().skip(start).rev().find(|h| is_submit_approval(h)) else { return false };
    let binding = &marker.extra;
    let Some(token) = binding.get("request_token").and_then(Value::as_str) else { return false };
    if binding.get("revision").and_then(Value::as_u64) != Some(revision(entry) as u64) {
        return false;
    }
    let basis = binding.get("score_basis").and_then(Value::as_str).unwrap_or("");
    if !readiness::SCORE_BASES.contains(&basis) {
        return false;
    }
    let eval = readiness::evaluate(entry, basis == "video_exempt", basis == "override");
    eval.reasons.is_empty() && eval.token.as_deref() == Some(token)
}

/// Refuse `ach submit` (or `transition … submitted`) without a current submission approval.
pub fn check_submit_approved(entry: &Entry) -> Result<()> {
    if entry.state == "awaiting_approval" && !submit_approved(entry) {
        bail!(
            "{}: 지금 작업 판에 묶인 사용자 제출 승인이 없다(승인 뒤 재작업·재채점·제출 준비 기록 변경·파일 변경이 있으면 승인이 무효가 된다). ach approval-status {}로 확인하고 사용자가 다시 ach approve {} --note \"<승인 원문>\"로 기록한다",
            entry.id,
            entry.id,
            entry.id
        );
    }
    Ok(())
}

fn check_approver(entry: &Entry, note: &str, session: Option<&str>) -> Result<()> {
    if let Some(session) = session.filter(|s| !s.is_empty()) {
        bail!(
            "{}: 공모전 세션 안(ACH_SESSION={session})에서는 승인할 수 없다. 승인은 사용자 터미널이나 메인 세션에서 기록한다",
            entry.id
        );
    }
    if note.trim().is_empty() {
        bail!("{}: 승인에는 --note(또는 --note-file)에 사용자 승인 원문과 시각을 적는다", entry.id);
    }
    Ok(())
}

/// Record a submission approval bound to `eval` (the marker in history and the approval record).
fn record_submit_approval(entry: &mut Entry, note: &str, by: &str, eval: &readiness::Evaluation, delegated: bool) {
    let at = now();
    let mut binding = eval.binding();
    if delegated {
        binding.insert("delegated".into(), Value::Bool(true));
    }
    let mut marker = HistoryItem::new(at.clone(), Some(entry.state.clone()), "awaiting_approval", by, note);
    marker.extra.insert("approval".into(), Value::String("submit".into()));
    marker.extra.extend(binding.clone());
    entry.history.push(marker);
    entry.approvals.push(Approval {
        at,
        stage: "submit".into(),
        note: note.to_string(),
        by: by.to_string(),
        extra: binding,
    });
}

/// Record a human approval with the user's words. `session` is `ACH_SESSION` as the caller
/// read it: set inside contest sessions, which may never approve their own work.
/// `plan_review` → stage `plan` and the move to `running`; `awaiting_approval` → stage `submit`
/// (state unchanged), which needs the core submission conditions (`readiness::evaluate`): a
/// current structured readiness record whose files still match, and from level 2 a current
/// passing final score or a recorded human score-gate override. The approval is bound to the
/// current request token. Returns the stage.
pub fn approve(entry: &mut Entry, note: &str, by: &str, session: Option<&str>) -> Result<&'static str> {
    check_approver(entry, note, session)?;
    match entry.state.as_str() {
        "plan_review" => {
            move_to(entry, "running", by, note);
            if let Some(last) = entry.history.last_mut() {
                last.extra.insert("approval".into(), Value::String("plan".into()));
            }
            entry.approvals.push(Approval {
                at: now(),
                stage: "plan".into(),
                note: note.to_string(),
                by: by.to_string(),
                extra: Map::new(),
            });
            Ok("plan")
        }
        "awaiting_approval" => {
            let eval = readiness::evaluate(entry, false, true);
            if !eval.reasons.is_empty() {
                bail!(
                    "{}: 제출 승인 조건이 맞지 않는다({}). ach approval-status {}로 확인한다",
                    entry.id,
                    eval.reasons.join("; "),
                    entry.id
                );
            }
            record_submit_approval(entry, note, by, &eval, false);
            Ok("submit")
        }
        other => bail!("{}: {other} 상태에서는 승인할 것이 없다(plan_review나 awaiting_approval에서만)", entry.id),
    }
}

/// A submission approval under the user's standing delegation (the CLI checks the delegation
/// policy and persona first). Never a plan approval and never on a score-gate override: the core
/// status must be eligible as is (`video_exempt` only when policy grants it) and its request token
/// must equal `expected_token`, the one the delegate reviewed.
pub fn approve_delegated(
    entry: &mut Entry,
    note: &str,
    by: &str,
    session: Option<&str>,
    expected_token: &str,
    video_exempt: bool,
) -> Result<&'static str> {
    check_approver(entry, note, session)?;
    if entry.state != "awaiting_approval" {
        bail!(
            "{}: 위임 승인은 제출 승인(awaiting_approval)만 한다(지금 {}). 계획 승인은 사용자가 직접 한다",
            entry.id,
            entry.state
        );
    }
    let eval = readiness::evaluate(entry, video_exempt, false);
    if !eval.reasons.is_empty() {
        bail!("{}: 위임 승인 조건이 맞지 않는다({})", entry.id, eval.reasons.join("; "));
    }
    if eval.token.as_deref() != Some(expected_token) {
        bail!(
            "{}: 요청 토큰이 바뀌었다(기대 {expected_token}, 지금 {}). ach approval-status {}로 다시 확인한다",
            entry.id,
            eval.token.as_deref().unwrap_or("-"),
            entry.id
        );
    }
    record_submit_approval(entry, note, by, &eval, true);
    Ok("submit")
}

/// Who may read the personal profile section for `entry` (`ach profile show --for <id>`): a
/// submission approved by the user (or already submitted), and, inside a contest session, only
/// that contest's own session.
pub fn check_personal_access(entry: &Entry, session: Option<&str>) -> Result<()> {
    if let Some(session) = session.filter(|s| !s.is_empty())
        && session != entry.id
    {
        bail!("{}: 다른 공모전 세션(ACH_SESSION={session})은 이 공모전의 개인정보를 읽을 수 없다", entry.id);
    }
    let ok = entry.state == "submitted" || (entry.state == "awaiting_approval" && submit_approved(entry));
    if !ok {
        bail!(
            "{}: 개인정보는 사용자가 제출을 승인한 뒤에만 읽는다(지금 {}). 사용자가 ach approve {} --note \"<승인 원문>\"로 승인한다",
            entry.id,
            entry.state,
            entry.id
        );
    }
    Ok(())
}

fn move_to(entry: &mut Entry, to: &str, by: &str, note: &str) {
    let from = std::mem::replace(&mut entry.state, to.to_string());
    entry.history.push(HistoryItem::new(now(), Some(from), to, by, note));
}

/// Move `entry` to `to` and record it. Fails outside the allowed moves, on `plan_review → running`
/// (only `ach approve` ends a plan review) and on the level 5 plan gate.
pub fn transition(entry: &mut Entry, to: &str, by: &str, note: &str) -> Result<()> {
    if !STATES.contains(&to) {
        bail!("알 수 없는 상태: {to}");
    }
    let current = entry.state.clone();
    let options = allowed(&current);
    if !options.contains(&to) {
        let mut sorted = options.to_vec();
        sorted.sort_unstable();
        let list = if sorted.is_empty() { "없음".to_string() } else { sorted.join(", ") };
        bail!("{}: {current} → {to} 이동은 허용되지 않는다(가능: {list})", entry.id);
    }
    if to != "submitted" && crate::submission::in_flight(entry) {
        bail!(
            "{}: 외부 제출 결과가 확인되지 않은 시도가 있다. ach submission status {}로 확인하고 접수 확정 또는 미제출 증빙을 먼저 기록한다",
            entry.id,
            entry.id
        );
    }
    if to == "followup_pending" && entry.submissions.is_empty() {
        bail!("{}: 후속 일정 대기는 기존 제출 기록이 있는 공모전만 가능하다", entry.id);
    }
    if current == "plan_review" && to == "running" {
        bail!(
            "{}: 계획 검수는 사용자 승인으로만 끝난다. 사용자가 ach approve {} --note \"<승인 원문과 시각>\"로 기록한다",
            entry.id,
            entry.id
        );
    }
    if to == "awaiting_approval" && entry.level == Some(5) && !plan_approved(entry) {
        bail!("{}: 레벨 5는 계획 승인(ach approve, plan_review 단계) 기록이 있어야 제출 승인 단계로 간다", entry.id);
    }
    move_to(entry, to, by, note);
    Ok(())
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
    }
}

/// Validate a gate record (`gate.json`), copy it onto the entry and move the entry by its decision.
pub fn apply_gate(entry: &mut Entry, mut gate: Value, by: &str) -> Result<()> {
    let Some(record) = gate.as_object_mut() else { bail!("gate.json은 객체여야 한다") };
    let target = match record.get("decision").and_then(Value::as_str) {
        Some("pass") => "gate_pass",
        Some("fail") => "rejected",
        Some("hold") => "hold",
        _ => bail!("gate.decision은 pass, fail, hold 중 하나여야 한다"),
    };
    let kind = record.get("type").and_then(Value::as_str).filter(|t| TYPES.contains(t)).map(str::to_string);
    let Some(kind) = kind else {
        let mut types = TYPES.to_vec();
        types.sort_unstable();
        bail!("gate.type은 {types:?} 중 하나여야 한다")
    };
    let level = match record.get("level") {
        Some(Value::Number(n)) => n.as_u64().and_then(|l| u8::try_from(l).ok()).filter(|l| LEVELS.contains(l)),
        _ => None,
    };
    let Some(level) = level else { bail!("gate.level은 정수 1~5 중 하나여야 한다") };
    if target == "gate_pass" && !truthy(record.get("criteria")) && !truthy(record.get("deliverables")) {
        bail!("pass 판정에는 deliverables나 criteria가 있어야 한다");
    }
    if !["discovered", "hold", "rejected", "gate_pass"].contains(&entry.state.as_str()) {
        bail!("{}: {} 상태에서는 게이트를 다시 기록할 수 없다", entry.id, entry.state);
    }
    let text_field = |key: &str| record.get(key).and_then(Value::as_str).map(text::one_line).filter(|v| !v.is_empty());
    let (title, organizer) = (text_field("title"), text_field("organizer"));
    let deadline = match record.get("deadline").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()) {
        Some(raw) => match text::normalize_deadline(raw) {
            Some(deadline) => Some(deadline),
            None => bail!("gate.deadline은 YYYY-MM-DD 또는 YYYY-MM-DDTHH:MM+09:00 형식의 실제 날짜여야 한다: {raw}"),
        },
        None => None,
    };
    let official = record.get("official_url").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
    if let Some(official) = official
        && !text::is_web_url(official)
    {
        bail!("gate.official_url은 http:// 또는 https:// 주소여야 한다: {official}");
    }
    let official = official.map(str::to_string);
    if let Some(title) = title {
        entry.title = title;
    }
    if let Some(organizer) = organizer {
        entry.organizer = organizer;
    }
    if let Some(deadline) = deadline {
        record.insert("deadline".into(), Value::String(deadline.clone()));
        entry.deadline = Some(deadline);
    }
    if let Some(official) = official
        && official != entry.url
    {
        if !entry.url.is_empty() && !entry.alt_urls.contains(&entry.url) {
            entry.alt_urls.push(entry.url.clone());
        }
        entry.url = official;
    }
    entry.kind = Some(kind);
    entry.level = Some(level);
    entry.priority = record.get("priority").and_then(Value::as_f64).map(|p| p.round() as i64).unwrap_or(0);
    record.insert("checked_at".into(), Value::String(now())); // the real recording time, not the agent's
    let reasons: Vec<&str> = record
        .get("reasons")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let note: String = reasons.join("; ").chars().take(500).collect();
    entry.gate = Some(gate);
    if entry.state == target {
        entry.history.push(HistoryItem::new(
            now(),
            Some(target.to_string()),
            target,
            by,
            &format!("게이트 재기록: {note}"),
        ));
        Ok(())
    } else {
        transition(entry, target, by, &note)
    }
}

pub const SCORE_STAGES: &[&str] = &["plan", "draft", "final"];
pub const VERDICTS: &[&str] = &["ready", "revise", "reject"];
/// Target total (out of 100) when the Judge output does not give one.
pub const DEFAULT_TARGET: f64 = 90.0;

fn number(record: &Value, key: &str) -> Option<f64> {
    record.get(key).and_then(Value::as_f64).filter(|n| n.is_finite())
}

/// Points of slack between `total` and the criterion sum normalised to 100 (Judges round).
pub const TOTAL_TOLERANCE: f64 = 0.5;

/// Validate a ContestJudge output (`total`, `target`, `verdict`, `scores[{name, round?, weight,
/// score}]`) and build the `scores` record: `{at, stage, total, target, verdict, criteria, note,
/// file}`. Weights are positive, each score lies in `0..=weight`, a criterion (round + name) appears
/// once, and `total` is the criterion sum normalised to 100 (within [`TOTAL_TOLERANCE`]). Checks
/// that need the entry (gate criteria, reviewed files, revision) are in [`record_score`].
pub fn score_record(judge: &Value, stage: &str, file: &str, note: &str) -> Result<Value> {
    if !SCORE_STAGES.contains(&stage) {
        bail!("stage는 plan, draft, final 중 하나여야 한다: {stage}");
    }
    if !judge.is_object() {
        bail!("채점 파일은 JSON 객체여야 한다");
    }
    if let Some(own) = judge.get("stage").and_then(Value::as_str)
        && own != stage
    {
        bail!("채점 파일의 stage({own})가 --stage({stage})와 다르다");
    }
    let Some(total) = number(judge, "total").filter(|t| (0.0..=100.0).contains(t)) else {
        bail!("total은 0~100 사이 숫자여야 한다(100점 환산)")
    };
    let target = match judge.get("target") {
        None | Some(Value::Null) => DEFAULT_TARGET,
        Some(_) => match number(judge, "target").filter(|t| (0.0..=100.0).contains(t)) {
            Some(t) => t,
            None => bail!("target은 0~100 사이 숫자여야 한다"),
        },
    };
    let verdict = judge.get("verdict").and_then(Value::as_str).filter(|v| VERDICTS.contains(v));
    let Some(verdict) = verdict else { bail!("verdict는 ready, revise, reject 중 하나여야 한다") };
    let Some(items) = judge.get("scores").and_then(Value::as_array).filter(|a| !a.is_empty()) else {
        bail!("scores는 기준별 점수 [{{name, weight, score}}] 배열이어야 한다")
    };
    let mut criteria: Vec<Value> = Vec::with_capacity(items.len());
    let mut seen: Vec<(Option<&str>, &str)> = Vec::with_capacity(items.len());
    let (mut weights, mut points) = (0.0, 0.0);
    for (i, item) in items.iter().enumerate() {
        let name = item.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty());
        let Some(name) = name else { bail!("scores[{i}].name이 비어 있다") };
        let round = match item.get("round") {
            None | Some(Value::Null) => None,
            Some(Value::String(r)) => Some(r.trim()).filter(|r| !r.is_empty()),
            Some(_) => bail!("scores[{i}]({name}).round는 문자열이어야 한다"),
        };
        // Without a round, the same name elsewhere is the same criterion.
        if seen.iter().any(|(r, n)| *n == name && (round.is_none() || r.is_none() || *r == round)) {
            bail!("scores[{i}]({}{name}) 기준이 두 번 나온다", round.map(|r| format!("{r} ")).unwrap_or_default());
        }
        seen.push((round, name));
        let Some(weight) = number(item, "weight").filter(|w| *w > 0.0) else {
            bail!("scores[{i}]({name}).weight는 0보다 큰 숫자여야 한다")
        };
        let Some(score) = number(item, "score").filter(|s| *s >= 0.0 && *s <= weight + 1e-9) else {
            bail!("scores[{i}]({name}).score는 0 이상 weight({weight}) 이하의 숫자여야 한다")
        };
        weights += weight;
        points += score;
        let mut row = Map::new();
        row.insert("name".into(), Value::String(name.to_string()));
        if let Some(round) = round {
            row.insert("round".into(), Value::String(round.to_string()));
        }
        row.insert("weight".into(), serde_json::json!(weight));
        row.insert("score".into(), serde_json::json!(score));
        criteria.push(Value::Object(row));
    }
    let expected = points / weights * 100.0;
    if !weights.is_finite() || !points.is_finite() || !expected.is_finite() {
        bail!("scores의 배점·점수 합이 유한한 숫자가 아니다");
    }
    if (total - expected).abs() > TOTAL_TOLERANCE + 1e-9 {
        bail!(
            "total({})이 기준별 점수 합을 100점으로 환산한 값({})과 맞지 않는다(점수 합 {}/배점 합 {})",
            points_label(total),
            points_label(expected),
            points_label(points),
            points_label(weights)
        );
    }
    Ok(serde_json::json!({
        "at": now(),
        "stage": stage,
        "total": total,
        "target": target,
        "verdict": verdict,
        "criteria": criteria,
        "note": note,
        "file": file,
    }))
}

fn points_label(value: f64) -> String {
    format!("{:.2}", value).trim_end_matches('0').trim_end_matches('.').to_string()
}

type GateCriterion = (Option<String>, String, Option<f64>);

/// The criteria of `gate.json` as `(round, name, weight)`; `None` when the gate gives no usable
/// list (then the Judge rows are not compared with it).
fn gate_criteria(entry: &Entry) -> Option<Vec<GateCriterion>> {
    let items = entry.gate.as_ref()?.get("criteria")?.as_array()?;
    let parsed: Vec<_> = items
        .iter()
        .filter_map(|c| {
            let name = c.get("name")?.as_str()?.trim();
            let round = c.get("stage").and_then(Value::as_str).map(str::trim).filter(|r| !r.is_empty());
            let weight = number(c, "weight").filter(|w| *w > 0.0);
            (!name.is_empty()).then(|| (round.map(str::to_string), name.to_string(), weight))
        })
        .collect();
    (!parsed.is_empty()).then_some(parsed)
}

/// One Judge row per gate criterion: every row names exactly one gate criterion (round + name; a
/// row without a round must name a criterion that is unique by name), every gate criterion is
/// scored, and a published gate weight is the row's weight.
fn check_gate_criteria(entry: &Entry, criteria: &[Value]) -> Result<()> {
    let Some(gate) = gate_criteria(entry) else { return Ok(()) };
    let mut used = vec![false; gate.len()];
    for row in criteria {
        let name = row.get("name").and_then(Value::as_str).unwrap_or("");
        let round = row.get("round").and_then(Value::as_str);
        let matches: Vec<usize> = gate
            .iter()
            .enumerate()
            .filter(|(_, (r, n, _))| n == name && (round.is_none() || r.is_none() || r.as_deref() == round))
            .map(|(i, _)| i)
            .collect();
        let label = round.map(|r| format!("{r} {name}")).unwrap_or_else(|| name.to_string());
        let i = match matches.as_slice() {
            [i] => *i,
            [] => bail!("{}: 채점 기준 {label}이(가) gate.json criteria에 없다", entry.id),
            _ => bail!("{}: 채점 기준 {label}이(가) gate.json의 여러 심사 단계에 있다(round를 적는다)", entry.id),
        };
        if std::mem::replace(&mut used[i], true) {
            bail!("{}: gate.json 기준 {label}을(를) 두 번 채점했다", entry.id);
        }
        if let (Some(expected), Some(weight)) = (gate[i].2, number(row, "weight"))
            && (expected - weight).abs() > 1e-6
        {
            bail!(
                "{}: 채점 기준 {label}의 배점({})이 gate.json 배점({})과 다르다",
                entry.id,
                points_label(weight),
                points_label(expected)
            );
        }
    }
    let missing: Vec<String> = gate
        .iter()
        .zip(&used)
        .filter(|(_, used)| !**used)
        .map(|((r, n, _), _)| r.as_ref().map(|r| format!("{r} {n}")).unwrap_or_else(|| n.clone()))
        .collect();
    if !missing.is_empty() {
        bail!("{}: gate.json 기준을 채점하지 않았다: {}", entry.id, missing.join(", "));
    }
    Ok(())
}

/// Validate a Judge output against the entry and append it to `entry.scores`; returns the record.
/// On top of [`score_record`]: the rows match `gate.json` criteria when the gate lists them, the
/// record carries the current work `revision`, and `artifacts` (`[{path, sha256}]`, the files the
/// Judge reviewed) are checked against the contest folder as it is now and stored as
/// folder-relative paths. `final` requires them; `plan` and `draft` store them when given.
pub fn record_score(entry: &mut Entry, judge: &Value, stage: &str, file: &str, note: &str) -> Result<Value> {
    let mut record = score_record(judge, stage, file, note)?;
    if matches!(stage, "plan" | "final") {
        let pending = crate::rules::stage_problems(entry, stage);
        if pending.contains(&"rules_pending_review") || pending.contains(&"rules_state_invalid") {
            bail!("{}: 공고 변경 검토가 끝나지 않아 채점을 기록하지 않는다({})", entry.id, pending.join(", "));
        }
    }
    let independent = if stage == "final" { crate::evaluation::validate_final(entry, judge)? } else { None };
    let criteria = record["criteria"].as_array().cloned().unwrap_or_default();
    check_gate_criteria(entry, &criteria)?;
    let artifacts = match judge.get("artifacts") {
        None | Some(Value::Null) if stage != "final" => None,
        None | Some(Value::Null) => {
            bail!("{}: final 채점에는 검토한 제출 파일 artifacts [{{path, sha256}}]가 있어야 한다", entry.id)
        }
        Some(list) => Some(readiness::normalize_artifacts(entry, list)?),
    };
    let object = record.as_object_mut().expect("score_record builds an object");
    object.insert("revision".into(), serde_json::json!(revision(entry)));
    object.insert("rules_binding".into(), crate::rules::binding(entry));
    if let Some(binding) = independent {
        object.insert("independent_assessment".into(), binding);
    }
    if let Some(artifacts) = artifacts {
        object.insert("artifacts".into(), Value::Array(artifacts));
    }
    entry.scores.push(record);
    if let Err(error) = crate::rules::score_recorded(entry, stage) {
        entry.scores.pop();
        return Err(error);
    }
    Ok(entry.scores.last().expect("just recorded a score").clone())
}

/// The most recent score, of `stage` when given.
pub fn latest_score<'a>(entry: &'a Entry, stage: Option<&str>) -> Option<&'a Value> {
    entry.scores.iter().rev().find(|s| stage.is_none_or(|st| s.get("stage").and_then(Value::as_str) == Some(st)))
}

/// `88` or `88.5`.
pub fn points(value: f64) -> String {
    if value.fract() == 0.0 { format!("{value:.0}") } else { format!("{value:.1}") }
}

/// `total/target`, e.g. `88/90`.
pub fn score_label(score: &Value) -> String {
    let get = |k: &str| score.get(k).and_then(Value::as_f64).map(points).unwrap_or_else(|| "-".into());
    format!("{}/{}", get("total"), get("target"))
}

/// A reason the scores do not let a level 2-5 submission through: a stable `code` (what
/// `ach approval-status` reports) and the message for the person at the terminal.
pub struct ScoreProblem {
    pub code: &'static str,
    pub message: String,
}

/// Why the scores do not let a level 2-5 submission through: a `plan` score is required (any
/// revision: the plan stays valid across revise cycles), and the latest `final` must be stamped
/// with the current work revision, judged `ready` with `total >= target`, and its reviewed files
/// must still hash as recorded. Older unstamped finals stay readable but never pass.
pub fn score_problems(entry: &Entry) -> Vec<ScoreProblem> {
    let mut problems = Vec::new();
    let mut add = |code: &'static str, message: String| problems.push(ScoreProblem { code, message });
    if latest_score(entry, Some("plan")).is_none() {
        add("plan_score_missing", "plan 점수 없음".to_string());
    }
    if let Some(plan) = latest_score(entry, Some("plan"))
        && !crate::rules::check_score(entry, plan, "plan")
    {
        add("plan_rules_stale", "규정 변경 뒤 계획 채점이 갱신되지 않았다".into());
    }
    let Some(score) = latest_score(entry, Some("final")) else {
        add("final_score_missing", "final 점수 없음".to_string());
        return problems;
    };
    if !crate::rules::check_score(entry, score, "final") {
        add("final_rules_stale", "규정 변경 뒤 최종 채점이 갱신되지 않았다".into());
    }
    let verdict = score.get("verdict").and_then(Value::as_str).unwrap_or("-");
    let total = number(score, "total").unwrap_or(0.0);
    let target = number(score, "target").unwrap_or(DEFAULT_TARGET);
    if verdict != "ready" {
        add("final_not_ready", format!("최근 final 판정 {verdict}"));
    }
    if total < target {
        add("final_below_target", format!("최근 final {} 목표 미달", score_label(score)));
    }
    let current = revision(entry) as u64;
    match score.get("revision").and_then(Value::as_u64) {
        None => add("final_unstamped", "최근 final에 작업 판 표시가 없다(이전 형식, 다시 채점한다)".to_string()),
        Some(r) if r != current => {
            add("final_stale_revision", format!("최근 final이 이전 작업 판({r}, 지금 {current})의 것이다"))
        }
        Some(_) => {}
    }
    match score.get("artifacts") {
        None | Some(Value::Null) => {
            add("final_artifacts_missing", "최근 final에 검토 파일(artifacts) 기록이 없다".to_string())
        }
        Some(list) => {
            if let Err(err) = readiness::check_artifacts(entry, list) {
                add("final_artifact_changed", format!("최근 final 검토 파일이 지금 파일과 다르다: {err}"));
            }
        }
    }
    problems
}

/// The score gate for levels 2-5. `→ awaiting_approval` needs [`score_problems`] to be empty;
/// `score_ok` (a reason) lets it through anyway and returns the history note to add (the CLI then
/// marks that history item `score_override`). Level 5 `→ plan_review` needs a `plan` score.
pub fn score_gate(entry: &Entry, to: &str, score_ok: Option<&str>) -> Result<Option<String>> {
    let rule_problems = match to {
        "awaiting_approval" => crate::rules::approval_problems(entry),
        "plan_review" => crate::rules::stage_problems(entry, "plan"),
        _ => Vec::new(),
    };
    if !rule_problems.is_empty() {
        bail!("{}: 공고 변경 검토가 필요하다({})", entry.id, rule_problems.join(", "));
    }
    let level = entry.level.unwrap_or(0);
    if level < 2 {
        return Ok(None);
    }
    let has_plan = latest_score(entry, Some("plan")).is_some();
    if to == "plan_review" && level == 5 && !has_plan {
        bail!("{}: 레벨 5 계획 검수에는 기획 채점(ach score --stage plan) 기록이 있어야 한다", entry.id);
    }
    if to != "awaiting_approval" {
        return Ok(None);
    }
    let problems: Vec<String> = score_problems(entry).into_iter().map(|p| p.message).collect();
    let last_final = latest_score(entry, Some("final"));
    if problems.is_empty() {
        return Ok(None);
    }
    let summary = match last_final {
        Some(score) => format!(
            "{}; final {} {}",
            problems.join(", "),
            score_label(score),
            score.get("verdict").and_then(Value::as_str).unwrap_or("-")
        ),
        None => problems.join(", "),
    };
    match score_ok.map(str::trim).filter(|r| !r.is_empty()) {
        Some(reason) => Ok(Some(format!("점수 관문 예외({summary}): {reason}"))),
        None => bail!(
            "{}: 점수 관문을 넘지 못해 제출 승인 단계로 가지 않는다({summary}). 채점을 다시 기록하거나(ach score), 마감 등으로 그대로 내야 하면 --score-ok \"<사유>\"",
            entry.id
        ),
    }
}
