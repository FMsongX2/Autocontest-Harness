//! The contest registry (`data/registry.json`): one entry per contest, a state machine over it, and
//! the gate record. Only the CLI writes it. Contract: `docs/CONTRACT.md` sections 3-4.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::bail;
use crate::error::{Error, Result};
use crate::paths;
use crate::text::now;

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
        "running" => &["plan_review", "awaiting_approval", "closed", "skipped"],
        "plan_review" => &["running", "skipped"],
        "awaiting_approval" => &["running", "submitted", "skipped"],
        "submitted" => &["closed", "running"],
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
    /// Fields this version does not know about are kept as they are.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Registry {
    pub version: u32,
    pub contests: IndexMap<String, Entry>,
}

impl Default for Registry {
    fn default() -> Self {
        Registry { version: 1, contests: IndexMap::new() }
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
    if !path.exists() {
        return Ok(Registry::default());
    }
    serde_json::from_str(&fs::read_to_string(path)?).map_err(|err| Error(format!("{}: {err}", path.display())))
}

fn save(path: &Path, registry: &Registry) -> Result<()> {
    let tmp = path.with_file_name(format!(".registry-{}.json", std::process::id()));
    let mut file = File::create(&tmp)?;
    file.write_all(serde_json::to_string_pretty(registry)?.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn lock_file(path: &Path) -> Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    Ok(OpenOptions::new().create(true).truncate(false).write(true).open(path.with_extension("lock"))?)
}

/// Read the registry under a shared lock.
pub fn read<T>(f: impl FnOnce(&Registry) -> Result<T>) -> Result<T> {
    read_at(&paths::registry(), f)
}

/// Modify the registry under an exclusive lock. It is saved (atomically) only when `f` succeeds.
pub fn write<T>(f: impl FnOnce(&mut Registry) -> Result<T>) -> Result<T> {
    write_at(&paths::registry(), f)
}

pub fn read_at<T>(path: &Path, f: impl FnOnce(&Registry) -> Result<T>) -> Result<T> {
    let lock = lock_file(path)?;
    lock.lock_shared()?;
    f(&load(path)?)
}

pub fn write_at<T>(path: &Path, f: impl FnOnce(&mut Registry) -> Result<T>) -> Result<T> {
    let lock = lock_file(path)?;
    lock.lock()?;
    let mut registry = load(path)?;
    let value = f(&mut registry)?;
    save(path, &registry)?;
    Ok(value)
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
        title: spec.title.trim().to_string(),
        organizer: spec.organizer.trim().to_string(),
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
        history: vec![HistoryItem {
            at,
            from: None,
            to: "discovered".to_string(),
            by: spec.source.to_string(),
            note: spec.note.unwrap_or_default(),
        }],
        extra: Map::new(),
    }
}

pub fn plan_approved(entry: &Entry) -> bool {
    entry.history.iter().any(|h| h.from.as_deref() == Some("plan_review") && h.to == "running")
}

/// Move `entry` to `to` and record it. Fails outside the allowed moves and on the level 5 plan gate.
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
    if current == "plan_review" && to == "running" && note.trim().is_empty() {
        bail!("{}: 계획 검수를 끝내려면 --note에 승인 근거(승인 메시지 원문과 시각)를 적는다", entry.id);
    }
    if to == "awaiting_approval" && entry.level == Some(5) && !plan_approved(entry) {
        bail!("{}: 레벨 5는 계획 검수(plan_review → running) 승인 기록이 있어야 제출 승인 단계로 간다", entry.id);
    }
    entry.state = to.to_string();
    entry.history.push(HistoryItem {
        at: now(),
        from: Some(current),
        to: to.to_string(),
        by: by.to_string(),
        note: note.to_string(),
    });
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
    for (key, field) in [("title", &mut entry.title), ("organizer", &mut entry.organizer)] {
        if let Some(value) = record.get(key).and_then(Value::as_str).filter(|v| !v.is_empty()) {
            *field = value.to_string();
        }
    }
    if let Some(deadline) = record.get("deadline").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        entry.deadline = Some(deadline.to_string());
    }
    if let Some(official) = record.get("official_url").and_then(Value::as_str).filter(|v| !v.is_empty())
        && official != entry.url
    {
        if !entry.url.is_empty() && !entry.alt_urls.contains(&entry.url) {
            entry.alt_urls.push(entry.url.clone());
        }
        entry.url = official.to_string();
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
        entry.history.push(HistoryItem {
            at: now(),
            from: Some(target.to_string()),
            to: target.to_string(),
            by: by.to_string(),
            note: format!("게이트 재기록: {note}"),
        });
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

/// Validate a ContestJudge output (`total`, `target`, `verdict`, `scores[{name, weight, score}]`)
/// and build the `scores` record: `{at, stage, total, target, verdict, criteria, note, file}`.
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
    let mut criteria = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let name = item.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty());
        let Some(name) = name else { bail!("scores[{i}].name이 비어 있다") };
        let Some(weight) = number(item, "weight").filter(|w| *w >= 0.0) else {
            bail!("scores[{i}]({name}).weight는 0 이상의 숫자여야 한다")
        };
        let Some(score) = number(item, "score").filter(|s| *s >= 0.0 && *s <= weight + 1e-9) else {
            bail!("scores[{i}]({name}).score는 0 이상 weight({weight}) 이하의 숫자여야 한다")
        };
        criteria.push(serde_json::json!({"name": name, "weight": weight, "score": score}));
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

/// The score gate for levels 2-5. `→ awaiting_approval` needs a `plan` score and a latest `final`
/// score with verdict `ready` and `total >= target`; `score_ok` (a reason) lets it through anyway and
/// returns the history note to add. Level 5 `→ plan_review` needs a `plan` score.
pub fn score_gate(entry: &Entry, to: &str, score_ok: Option<&str>) -> Result<Option<String>> {
    let level = entry.level.unwrap_or(0);
    if level < 2 {
        return Ok(None);
    }
    let has_plan = latest_score(entry, Some("plan")).is_some();
    if to == "plan_review" && level == 5 && !has_plan {
        bail!("{}: 레벨 5 계획 검수에는 기획 채점(harness score --stage plan) 기록이 있어야 한다", entry.id);
    }
    if to != "awaiting_approval" {
        return Ok(None);
    }
    let mut problems = Vec::new();
    if !has_plan {
        problems.push("plan 점수 없음".to_string());
    }
    let last_final = latest_score(entry, Some("final"));
    match last_final {
        None => problems.push("final 점수 없음".to_string()),
        Some(score) => {
            let verdict = score.get("verdict").and_then(Value::as_str).unwrap_or("-");
            let total = number(score, "total").unwrap_or(0.0);
            let target = number(score, "target").unwrap_or(DEFAULT_TARGET);
            if verdict != "ready" {
                problems.push(format!("최근 final 판정 {verdict}"));
            }
            if total < target {
                problems.push(format!("최근 final {} 목표 미달", score_label(score)));
            }
        }
    }
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
            "{}: 점수 관문을 넘지 못해 제출 승인 단계로 가지 않는다({summary}). 채점을 다시 기록하거나(harness score), 마감 등으로 그대로 내야 하면 --score-ok \"<사유>\"",
            entry.id
        ),
    }
}
