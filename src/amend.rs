//! A narrow correction of a recorded gate: fill in the judging criteria a gate record left empty
//! (`ach gate-criteria <id> --file F --expected-revision N [--expected-request T]
//! [--criteria-key K | --assumed-criteria] [--dry-run]`).
//!
//! `ach gate` cannot be recorded again once a contest is routed, and an entry whose
//! `gate.criteria` is empty never gets an independent evaluation packet
//! ([`crate::evaluation::prepare`]). This command changes that one field and nothing else, and
//! only when every condition below holds; otherwise nothing is written.
//!
//! - It runs outside contest sessions (`ACH_SESSION` unset), like `ach approve`.
//! - The entry is in progress (`running`, `plan_review`, `awaiting_approval`) with no submission
//!   record and no unresolved submission attempt.
//! - `--expected-revision` and `--expected-request` equal the current work revision and request
//!   token: a compare-and-set against what the caller reviewed.
//! - The recorded gate passed and its `criteria` are empty; the corrected gate file equals the
//!   recorded gate in every other field.
//! - The new rows equal the structured `{stage?, name, weight?}` rows of the accepted official
//!   rules' `criteria` category, whose snapshots still hash as captured. When the official rules
//!   give separate criteria per division or track (one `criteria` rule per division, each with its
//!   own `key`), `--criteria-key` names the one the entry competes in: exactly one accepted
//!   `criteria` rule has that key, every element of it is a structured row, and the new rows are
//!   that rule's rows, all of them and no other. Part of a division, rows of two divisions and
//!   rows no rule has are refused, and the other divisions' rules stay as they are.
//! - `--assumed-criteria` is for a notice that publishes no criteria or weights. Official rules
//!   are not invented: the accepted rules must say so (at least one accepted `criteria` rule, none
//!   with a structured row), and the new rows are the criteria every recorded score already used,
//!   with equal weights within a stage (ContestGate's rule for unpublished weights). The request
//!   must be ready to approve but for the independent evaluation: `awaiting_approval`, a current
//!   readiness record whose files hash as recorded, and a current passing `final` that reviewed
//!   them. The corrected file also adds [`ASSUMED_REASON`] as the last line of `reasons`, the
//!   amendment is recorded with `basis: "assumed_equal_weights"`, and such an entry gets one
//!   independent final evaluation per work revision ([`crate::evaluation`]).
//! - Every recorded score already uses exactly these criteria (round, name, weight).
//! - No submission approval was recorded since the entry entered `awaiting_approval` and no
//!   evaluation packet or assessment exists, so nothing reviewed against the old gate carries over.
//! - The folder copy `.harness/gate.json` equals the recorded gate (or already the corrected one).
//!
//! The folder copy is replaced first (temp file + rename); the entry then gets the new criteria,
//! an `extra.gate_amendments` record and a same-state history marker, saved by the registry's
//! atomic write. Contract: `docs/CONTRACT.md` section 7.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::bail;
use crate::error::{Error, Result};
use crate::evaluation;
use crate::readiness;
use crate::registry::{self, Entry, HistoryItem};
use crate::rules;
use crate::submission;
use crate::text::now;

/// `Entry.extra` key: the append-only list of gate corrections.
pub const KEY: &str = "gate_amendments";
/// Who the history marker and the amendment record name.
pub const BY: &str = "ach gate-criteria";
/// Folder-relative copy of the gate that sessions and agents read.
pub const FOLDER_GATE: &str = ".harness/gate.json";
/// The gate `reasons` line that marks criteria as an assumption: what ContestGate writes for a
/// notice that publishes no weights (`agents/ContestGate.md`).
pub const ASSUMED_REASON: &str = "[가정] 배점 미공개, 균등 배분";
/// `basis` of an amendment: rows of the accepted official rules.
pub const BASIS_OFFICIAL: &str = "official_rules";
/// `basis` of an amendment: the rows the work was scored with, when the accepted official rules
/// publish none (`--assumed-criteria`).
pub const BASIS_ASSUMED: &str = "assumed_equal_weights";

const STATES: &[&str] = &["running", "plan_review", "awaiting_approval"];
const ROW_KEYS: &[&str] = &["stage", "name", "weight"];

/// One `ach gate-criteria` call.
pub struct Request<'a> {
    /// The corrected gate file as given, and its text.
    pub path: &'a Path,
    pub text: &'a str,
    pub expected_revision: usize,
    pub expected_request: Option<&'a str>,
    /// The `key` of the one accepted `criteria` rule to take the rows from (`--criteria-key`);
    /// `None` compares with the rows of every accepted `criteria` rule together.
    pub criteria_key: Option<&'a str>,
    /// The official rules publish no criteria: fill in the rows the work was scored with as an
    /// assumption (`--assumed-criteria`). Not together with `criteria_key`.
    pub assumed: bool,
    /// `ACH_SESSION` as the caller read it.
    pub session: Option<&'a str>,
    pub dry_run: bool,
}

/// `(stage, name, weight)`.
type Row = (Option<String>, String, Option<f64>);

/// The gate criteria of `entry` were filled in as an assumption (`--assumed-criteria`), not from
/// official rules. Such an entry gets one independent final evaluation per work revision.
pub fn criteria_assumed(entry: &Entry) -> bool {
    let amendments = entry.extra.get(KEY).and_then(Value::as_array);
    amendments
        .and_then(|items| items.iter().rev().find(|a| a["field"] == "criteria"))
        .is_some_and(|a| a["basis"] == BASIS_ASSUMED)
}

/// The recorded gate `reasons` with [`ASSUMED_REASON`] as the last line (added once).
fn assumed_reasons(recorded: Option<&Value>) -> Result<Value> {
    let mut lines = match recorded {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(lines)) => lines.clone(),
        Some(_) => bail!("기록된 게이트의 reasons가 배열이 아니다"),
    };
    if !lines.iter().any(|line| line.as_str() == Some(ASSUMED_REASON)) {
        lines.push(json!(ASSUMED_REASON));
    }
    Ok(Value::Array(lines))
}

/// Assumed rows follow ContestGate's rule for unpublished weights: every row carries points and
/// the rows of one stage carry the same points.
fn check_equal_weights(id: &str, rows: &[Row]) -> Result<()> {
    for (stage, name, weight) in rows {
        let Some(weight) = weight.filter(|w| *w > 0.0) else {
            bail!("{id}: 가정 기준 {name}에 0보다 큰 배점이 없다. 같은 심사 단계 안에서 균등 배분한 배점을 적는다")
        };
        if let Some((_, other, _)) = rows.iter().find(|(s, _, w)| s == stage && *w != Some(weight)) {
            bail!("{id}: 가정 기준은 같은 심사 단계 안에서 배점이 같아야 한다(균등 배분): {name}, {other}");
        }
    }
    Ok(())
}

fn sorted_keys(rows: &[Row]) -> Vec<(String, String, String)> {
    let mut keys: Vec<_> = rows
        .iter()
        .map(|(stage, name, weight)| (stage.clone().unwrap_or_default(), name.clone(), format!("{weight:?}")))
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

fn labels(rows: &[Row]) -> String {
    let label = |(stage, name, weight): &Row| {
        let stage = stage.as_ref().map(|s| format!("{s} ")).unwrap_or_default();
        let weight = weight.map(registry::points).unwrap_or_else(|| "-".into());
        format!("{stage}{name}({weight})")
    };
    rows.iter().map(label).collect::<Vec<_>>().join(", ")
}

/// The corrected file's `criteria`, strictly: a non-empty array of `{stage?, name, weight?}` with
/// no other field, no repeated stage + name and at least one criterion that carries points.
fn corrected_rows(value: Option<&Value>) -> Result<Vec<Row>> {
    let Some(items) = value.and_then(Value::as_array).filter(|a| !a.is_empty()) else {
        bail!("정정 파일의 criteria는 비어 있지 않은 [{{stage, name, weight}}] 배열이어야 한다")
    };
    let mut rows: Vec<Row> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let Some(object) = item.as_object() else { bail!("criteria[{i}]는 객체여야 한다") };
        if let Some(key) = object.keys().find(|k| !ROW_KEYS.contains(&k.as_str())) {
            bail!("criteria[{i}]: 알 수 없는 필드 {key}(허용: {})", ROW_KEYS.join(", "));
        }
        let text = |key: &str| -> Result<Option<String>> {
            match object.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) if !s.is_empty() && s.trim() == s && !s.chars().any(char::is_control) => {
                    Ok(Some(s.clone()))
                }
                Some(_) => bail!("criteria[{i}].{key}는 앞뒤 공백과 제어 문자가 없는 비어 있지 않은 문자열이어야 한다"),
            }
        };
        let Some(name) = text("name")? else { bail!("criteria[{i}].name이 없다") };
        let stage = text("stage")?;
        let weight = match object.get("weight") {
            None | Some(Value::Null) => None,
            Some(value) => match value.as_f64().filter(|w| w.is_finite() && *w >= 0.0) {
                Some(weight) => Some(weight),
                None => bail!("criteria[{i}]({name}).weight는 0 이상의 숫자여야 한다"),
            },
        };
        if rows.iter().any(|(s, n, _)| *s == stage && *n == name) {
            bail!("criteria[{i}]({name}) 기준이 두 번 나온다");
        }
        rows.push((stage, name, weight));
    }
    if rows.iter().all(|(_, _, weight)| *weight == Some(0.0)) {
        bail!("정정 파일의 criteria에 배점이 있는 기준이 없다");
    }
    Ok(rows)
}

/// A structured criterion row of an accepted `criteria` rule: an object with a `name`.
fn official_row(item: &Value) -> Option<Row> {
    let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
    let name = text("name")?;
    let weight = item.get("weight").and_then(Value::as_f64).filter(|w| w.is_finite());
    Some((text("stage").map(str::to_string), name.to_string(), weight))
}

/// `{key, source, verification}` of an accepted rule, as the amendment record cites it.
fn citation(rule: &Value) -> Value {
    json!({"key": rule["key"], "source": rule["source"], "verification": rule["verification"]})
}

/// The structured rows of the accepted `criteria` rules (array elements that are objects with a
/// `name`), and the citation of each rule that contributed one.
fn official_rows(accepted: &[Value]) -> (Vec<Row>, Vec<Value>) {
    let (mut rows, mut cited) = (Vec::new(), Vec::new());
    for rule in accepted {
        let found: Vec<Row> = rule["value"].as_array().into_iter().flatten().filter_map(official_row).collect();
        if !found.is_empty() {
            cited.push(citation(rule));
            rows.extend(found);
        }
    }
    (rows, cited)
}

/// A rule key that may be given as `--criteria-key`: letters, digits, `.`, `_` and `-`.
fn selectable_key(key: &str) -> bool {
    (1..=100).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// The distinct selectable keys of `rules` (accepted rules or their citations), sorted.
fn selectable_keys(rules: &[Value]) -> Vec<&str> {
    let mut keys: Vec<&str> = rules.iter().filter_map(|rule| rule["key"].as_str()).collect();
    keys.retain(|key| selectable_key(key));
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// What to try when the rows of several keyed `criteria` rules did not match as one list.
fn division_hint(cited: &[Value]) -> String {
    let keys = selectable_keys(cited);
    if keys.len() < 2 {
        return String::new();
    }
    format!(". 공식 규칙이 부문별 기준이면 --criteria-key로 출품 부문의 규칙 하나를 고른다(key: {})", keys.join(", "))
}

/// The rows of the one accepted `criteria` rule whose key is `key` (the criteria of one official
/// division or track) and its citation. The key must name exactly one rule and every element of
/// that rule must be a structured row, so a division is taken whole or not at all.
fn division_rows(id: &str, version: &str, accepted: &[Value], key: &str) -> Result<(Vec<Row>, Vec<Value>)> {
    let matching: Vec<&Value> = accepted.iter().filter(|rule| rule["key"].as_str() == Some(key)).collect();
    let rule = match matching.as_slice() {
        [rule] => *rule,
        [] => {
            let keys = selectable_keys(accepted);
            bail!(
                "{id}: 수락된 규칙 판 {version}에 key가 {key}인 criteria 규칙이 없다(고를 수 있는 key: {})",
                if keys.is_empty() { "없음".to_string() } else { keys.join(", ") }
            )
        }
        many => bail!(
            "{id}: 수락된 규칙 판 {version}에 key가 {key}인 criteria 규칙이 {}개다. 규칙 하나로 정해지는 key만 고른다",
            many.len()
        ),
    };
    let items = rule["value"].as_array().map(Vec::as_slice).unwrap_or_default();
    let rows: Vec<Row> = items.iter().filter_map(official_row).collect();
    if rows.is_empty() || rows.len() != items.len() {
        bail!(
            "{id}: 수락된 규칙 판 {version}의 criteria 규칙 {key}이(가) 구조화된 기준 행 [{{stage, name, weight}}]만으로 돼 있지 않다. 부문 전체를 확인할 수 없는 규칙으로는 채우지 않는다"
        );
    }
    Ok((rows, vec![citation(rule)]))
}

/// The folder copy of the gate: a regular file inside the contest folder.
fn folder_gate(entry: &Entry) -> Result<PathBuf> {
    let Some(folder) = entry.folder.as_deref().filter(|f| !f.is_empty()) else {
        bail!("{}: 공모전 폴더가 기록돼 있지 않다", entry.id)
    };
    let root = Path::new(folder).canonicalize().map_err(|err| Error(format!("{folder}: {err}")))?;
    let path = root.join(FOLDER_GATE);
    let inside = path.parent().and_then(|dir| dir.canonicalize().ok()).is_some_and(|dir| dir.starts_with(&root));
    if !inside || !fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file()) {
        bail!("{}: {} 이(가) 공모전 폴더 안의 일반 파일이 아니다", entry.id, path.display());
    }
    Ok(path)
}

/// Replace `path` with `bytes` through a temp file in the same directory and a rename, keeping
/// the file's mode.
fn replace_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let at = |err: std::io::Error| Error(format!("{}: {err}", path.display()));
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let permissions = fs::metadata(path).map_err(at)?.permissions();
    let tmp = dir.join(format!(".gate-{}.json", std::process::id()));
    let _ = fs::remove_file(&tmp);
    let written = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.set_permissions(permissions)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if let Err(err) = written {
        let _ = fs::remove_file(&tmp);
        return Err(at(err));
    }
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
    Ok(())
}

fn next_seq(entry: &Entry) -> Result<u64> {
    match entry.extra.get(KEY) {
        None => Ok(1),
        Some(Value::Array(items)) => {
            Ok(items.iter().filter_map(|a| a.get("seq").and_then(Value::as_u64)).max().unwrap_or(0) + 1)
        }
        Some(_) => bail!("{}: extra.{KEY}가 배열이 아니다. 손으로 고친 기록은 덮어쓰지 않는다", entry.id),
    }
}

/// Fill in the empty `gate.criteria` of `entry` from the corrected gate file. See the module docs
/// for the conditions. Returns `{id, dry_run, field, before, after, weight_total, criteria_key,
/// state, revision, request_token, evidence: {rules_version, rules}, scores_checked, folder_gate:
/// {path, sha256}, amendment}`; `criteria_key` is the selected rule key (`null` without one) and
/// `amendment` the record's sequence number, `null` for a dry run.
pub fn gate_criteria(entry: &mut Entry, request: &Request<'_>) -> Result<Value> {
    let id = entry.id.clone();
    if let Some(session) = request.session.filter(|s| !s.is_empty()) {
        bail!(
            "{id}: 공모전 세션 안(ACH_SESSION={session})에서는 게이트 심사기준을 보완할 수 없다. 메인 세션이나 사용자 터미널에서 실행한다"
        );
    }
    if let Some(key) = request.criteria_key.filter(|key| !selectable_key(key)) {
        bail!(
            "{id}: --criteria-key {key:?}는 고를 수 없다. 수락된 criteria 규칙의 key 가운데 영문자·숫자·._-만 쓴 100자 이하 key를 준다"
        );
    }
    if request.assumed && request.criteria_key.is_some() {
        bail!("{id}: --assumed-criteria와 --criteria-key는 함께 쓰지 않는다");
    }
    if !STATES.contains(&entry.state.as_str()) {
        bail!("{id}: 게이트 심사기준 보완은 running·plan_review·awaiting_approval에서만 한다(지금 {})", entry.state);
    }
    if !entry.submissions.is_empty() {
        bail!("{id}: 제출 기록이 있는 공모전의 게이트는 고치지 않는다");
    }
    if submission::unresolved(entry) {
        bail!("{id}: 해소되지 않은 제출 시도가 있다. ach submission status {id}로 확인한다");
    }
    let revision = registry::revision(entry);
    if revision != request.expected_revision {
        bail!(
            "{id}: 작업 판이 다르다(기대 {}, 지금 {revision}). ach approval-status {id}로 다시 확인한다",
            request.expected_revision
        );
    }
    let reviewed = readiness::evaluate(entry, false, false);
    let token = reviewed.token.clone();
    if token.as_deref() != request.expected_request {
        bail!(
            "{id}: 요청 토큰이 다르다(기대 {}, 지금 {}). ach approval-status {id}로 다시 확인한다",
            request.expected_request.unwrap_or("-"),
            token.as_deref().unwrap_or("-")
        );
    }

    let Some(gate) = entry.gate.as_ref().and_then(Value::as_object) else { bail!("{id}: 게이트 기록이 없다") };
    if gate.get("decision").and_then(Value::as_str) != Some("pass") {
        bail!("{id}: pass로 기록된 게이트만 보완한다");
    }
    let before = gate.get("criteria").cloned().unwrap_or(Value::Null);
    if !(before.is_null() || before.as_array().is_some_and(Vec::is_empty)) {
        bail!("{id}: gate.criteria가 이미 있다. 이 명령은 비어 있는 criteria만 채운다");
    }
    let corrected: Value =
        serde_json::from_str(request.text).map_err(|err| Error(format!("{}: {err}", request.path.display())))?;
    let Some(fixed) = corrected.as_object() else { bail!("정정 파일은 gate.json 객체여야 한다") };
    // With `--assumed-criteria` the file also marks the assumption in `reasons`; nothing else moves.
    let reasons_after = match request.assumed {
        true => Some(assumed_reasons(gate.get("reasons"))?),
        false => None,
    };
    let expected = |key: &str| match (&reasons_after, key) {
        (Some(reasons), "reasons") => Some(reasons),
        _ => gate.get(key),
    };
    let mut differing: Vec<&str> = gate
        .keys()
        .chain(fixed.keys())
        .map(String::as_str)
        .filter(|key| *key != "criteria" && expected(key) != fixed.get(*key))
        .collect();
    differing.sort_unstable();
    differing.dedup();
    if !differing.is_empty() {
        let allowed = match request.assumed {
            true => format!("criteria를 채우고 reasons 끝에 \"{ASSUMED_REASON}\" 한 줄을 더한 사본이어야 한다"),
            false => "criteria만 채운 사본이어야 한다".to_string(),
        };
        bail!("{id}: 정정 파일이 기록된 게이트와 criteria 밖에서 다르다({}). {allowed}", differing.join(", "));
    }
    let rows = corrected_rows(fixed.get("criteria"))?;
    let after = fixed["criteria"].clone();

    let pending = rules::approval_problems(entry);
    if !pending.is_empty() {
        bail!("{id}: 공고 변경 검토가 끝나지 않았다({})", pending.join(", "));
    }
    let Some((version, accepted)) = rules::accepted_criteria(entry)? else {
        bail!("{id}: 수락된 공식 규칙 판이 없다. ach rules capture·review로 원문과 criteria 규칙을 먼저 남긴다")
    };
    let cited = if request.assumed {
        // The accepted evidence itself must say the notice publishes no criteria: a `criteria`
        // rule without any structured row. Official rows, where they exist, are never replaced.
        if accepted.is_empty() {
            bail!(
                "{id}: 수락된 규칙 판 {version}에 criteria 규칙이 없다. 공고에 심사기준·배점이 없다는 사실을 ach rules review의 criteria 규칙으로 먼저 남긴다"
            );
        }
        let (official, _) = official_rows(&accepted);
        if !official.is_empty() {
            bail!(
                "{id}: 수락된 규칙 판 {version}의 criteria 규칙에 공식 기준 행이 있다({}). 공식 기준이 있는 공모전은 가정 기준으로 채우지 않는다",
                labels(&official)
            );
        }
        check_equal_weights(&id, &rows)?;
        if !reviewed.reasons.is_empty() || reviewed.basis != "score" {
            let why = match reviewed.reasons.is_empty() {
                true => "점수 관문이 없는 요청".to_string(),
                false => reviewed.reasons.join(", "),
            };
            bail!(
                "{id}: 가정 기준은 제출 파일과 최종 채점까지 확인된 승인 대기 요청에만 쓴다({why}). ach approval-status {id}로 확인한다"
            );
        }
        let cite = |rule: &Value| {
            let mut cited = citation(rule);
            cited["value"] = rule["value"].clone();
            cited
        };
        accepted.iter().map(cite).collect()
    } else {
        let (official, cited) = match request.criteria_key {
            Some(key) => division_rows(&id, &version, &accepted, key)?,
            None => official_rows(&accepted),
        };
        if official.is_empty() {
            bail!("{id}: 수락된 규칙 판 {version}의 criteria 규칙에 구조화된 기준 행 [{{stage, name, weight}}]이 없다");
        }
        // A selected division is compared row for row; without a selection the rules are one list
        // and a row that several of them repeat counts once.
        let same_count = request.criteria_key.is_none() || rows.len() == official.len();
        if !same_count || sorted_keys(&rows) != sorted_keys(&official) {
            let (what, hint) = match request.criteria_key {
                Some(key) => (format!("criteria 규칙 {key}의 행과"), String::new()),
                None => ("criteria와".to_string(), division_hint(&cited)),
            };
            bail!(
                "{id}: 정정 criteria가 수락된 규칙 판 {version}의 {what} 다르다(정정: {}; 공식: {}){hint}",
                labels(&rows),
                labels(&official)
            );
        }
        cited
    };
    let basis = if request.assumed { BASIS_ASSUMED } else { BASIS_OFFICIAL };

    if registry::submit_approval_recorded(entry) {
        bail!(
            "{id}: 이번 승인 대기에 제출 승인 기록이 있다. 승인받은 요청의 게이트는 고치지 않는다(수정 요청으로 running에 돌아간 새 작업 판에서 한다)"
        );
    }
    if evaluation::recorded(entry) {
        bail!("{id}: 독립 평가 묶음이나 평가 기록이 이미 있다. 비어 있는 criteria로는 생길 수 없는 기록이다");
    }

    let mut gate_after = gate.clone();
    gate_after.insert("criteria".into(), after.clone());
    if let Some(reasons) = reasons_after {
        gate_after.insert("reasons".into(), reasons);
    }
    let (gate_before, gate_after) = (Value::Object(gate.clone()), Value::Object(gate_after));
    let mut work = entry.clone();
    work.gate = Some(gate_after.clone());
    for (i, score) in entry.scores.iter().enumerate() {
        let scored = score.get("criteria").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        registry::check_gate_criteria(&work, scored).map_err(|err| {
            let stage = score.get("stage").and_then(Value::as_str).unwrap_or("-");
            Error(format!("{id}: 기록된 점수 {}({stage})이(가) 정정 criteria와 맞지 않는다: {}", i + 1, err.0))
        })?;
    }

    let folder_path = folder_gate(entry)?;
    let on_disk: Value = fs::read(&folder_path)
        .map_err(Error::from)
        .and_then(|bytes| Ok(serde_json::from_slice(&bytes)?))
        .map_err(|err| Error(format!("{}: {}", folder_path.display(), err.0)))?;
    if on_disk != gate_before && on_disk != gate_after {
        bail!("{id}: 폴더의 {FOLDER_GATE}이(가) 기록된 게이트와 다르다. 폴더 사본을 먼저 확인한다");
    }
    let mut bytes = serde_json::to_vec_pretty(&gate_after)?;
    bytes.push(b'\n');

    let weight_total: f64 = rows.iter().filter_map(|(_, _, weight)| *weight).sum();
    let evidence = json!({"rules_version": version, "rules": cited});
    let folder_record = json!({"path": FOLDER_GATE, "sha256": format!("{:x}", Sha256::digest(&bytes))});
    let mut result = json!({
        "id": id,
        "dry_run": request.dry_run,
        "field": "criteria",
        "before": before,
        "after": after,
        "weight_total": weight_total,
        "basis": basis,
        "criteria_key": request.criteria_key,
        "state": entry.state,
        "revision": revision,
        "request_token": token,
        "evidence": evidence,
        "scores_checked": entry.scores.len(),
        "folder_gate": folder_record,
        "amendment": null,
    });
    if request.dry_run {
        return Ok(result);
    }

    let seq = next_seq(entry)?;
    replace_file(&folder_path, &bytes)?;
    let at = now();
    let file = request.path.canonicalize().unwrap_or_else(|_| request.path.to_path_buf()).display().to_string();
    let record = json!({
        "seq": seq,
        "at": at,
        "by": BY,
        "field": "criteria",
        "before": result["before"],
        "after": result["after"],
        "weight_total": weight_total,
        "basis": basis,
        "criteria_key": request.criteria_key,
        "file": {"path": file, "sha256": format!("{:x}", Sha256::digest(request.text.as_bytes()))},
        "evidence": result["evidence"],
        "state": entry.state,
        "revision": revision,
        "request_token": result["request_token"],
        "scores_checked": entry.scores.len(),
        "folder_gate": result["folder_gate"],
    });
    entry.gate = Some(gate_after);
    let list = entry.extra.entry(KEY.to_string()).or_insert_with(|| Value::Array(Vec::new()));
    list.as_array_mut().expect("next_seq accepts only an absent or array-valued record").push(record);
    let note = if request.assumed {
        format!(
            "게이트 심사기준 보완 #{seq}: {ASSUMED_REASON}. 공식 규칙 판 {version}에 심사기준·배점이 없어 기록된 채점 기준 {}건(배점 합 {})을 내부 평가용으로 채움",
            rows.len(),
            registry::points(weight_total)
        )
    } else {
        let division = request.criteria_key.map(|key| format!(", 공식 부문 규칙 key {key}")).unwrap_or_default();
        format!(
            "게이트 심사기준 보완 #{seq}: 비어 있던 criteria에 {}건(배점 합 {}), 근거 규칙 판 {version}{division}",
            rows.len(),
            registry::points(weight_total)
        )
    };
    let state = entry.state.clone();
    let mut marker = HistoryItem::new(at, Some(state.clone()), &state, BY, &note);
    marker.extra.insert("gate_amendment".into(), json!(seq));
    entry.history.push(marker);
    result["amendment"] = json!(seq);
    Ok(result)
}
