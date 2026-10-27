//! Structured submission readiness and the core approval status. A contest session records
//! readiness (`ach readiness <id> --file F`) as typed fields with the SHA-256 of every submission
//! file; nothing is inferred from prose. `status` is the core part of `ach approval-status`: the
//! approval request token and why the entry may not be approved now. Persona traces and delegation
//! policy are checked by the CLI on top of it. Contract: `docs/CONTRACT.md`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::bail;
use crate::error::{Error, Result};
use crate::registry::{self, Entry};
use crate::text::now;

/// `Entry.extra` key of the latest readiness record (an object).
pub const KEY: &str = "submission_readiness";
/// `Entry.extra` key of the earlier records, oldest first.
pub const HISTORY_KEY: &str = "submission_readiness_history";
/// States in which a session may record readiness.
pub const RECORD_STATES: &[&str] = &["running", "awaiting_approval"];
/// What a submission approval relied on for the score requirement: `none` (level 1), `score` (a
/// current passing final), `video_exempt` (delegated video policy), `override` (a human score-gate
/// reason recorded on the move into `awaiting_approval`).
pub const SCORE_BASES: &[&str] = &["none", "score", "video_exempt", "override"];

/// The latest readiness record, when it is an object.
pub fn latest(entry: &Entry) -> Option<&Value> {
    entry.extra.get(KEY).filter(|v| v.is_object())
}

fn history(entry: &Entry) -> &[Value] {
    entry.extra.get(HISTORY_KEY).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

/// SHA-256 of a file as lowercase hex.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|err| Error(format!("{}: {err}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

thread_local! {
    /// File hashes read during one evaluation (`HashScope`), keyed by canonical path, so a file
    /// listed by both the readiness record and the final score, or checked again by
    /// `submit_approved` inside `status`, is read once.
    static HASHES: RefCell<Option<HashMap<PathBuf, String>>> = const { RefCell::new(None) };
}

/// While alive, `cached_sha256` reuses hashes; the outermost scope clears them on drop, so the
/// next command (or the next call) always reads the files again.
struct HashScope {
    outer: bool,
}

impl HashScope {
    fn enter() -> Self {
        let outer = HASHES.with(|h| {
            let mut h = h.borrow_mut();
            let outer = h.is_none();
            if outer {
                *h = Some(HashMap::new());
            }
            outer
        });
        HashScope { outer }
    }
}

impl Drop for HashScope {
    fn drop(&mut self) {
        if self.outer {
            HASHES.with(|h| *h.borrow_mut() = None);
        }
    }
}

fn cached_sha256(path: &Path) -> Result<String> {
    if let Some(hit) = HASHES.with(|h| h.borrow().as_ref().and_then(|m| m.get(path).cloned())) {
        return Ok(hit);
    }
    let sha = sha256_file(path)?;
    HASHES.with(|h| {
        if let Some(map) = h.borrow_mut().as_mut() {
            map.insert(path.to_path_buf(), sha.clone());
        }
    });
    Ok(sha)
}

/// The contest folder as recorded and canonicalised.
fn folder(entry: &Entry) -> Result<(PathBuf, PathBuf)> {
    let Some(raw) = entry.folder.as_deref().filter(|f| !f.is_empty()) else {
        bail!("{}: 공모전 폴더가 기록돼 있지 않다", entry.id)
    };
    let raw = PathBuf::from(raw);
    let canonical = raw.canonicalize().map_err(|err| Error(format!("{}: 폴더를 열 수 없다: {err}", entry.id)))?;
    if !canonical.is_dir() {
        bail!("{}: 폴더가 디렉터리가 아니다: {}", entry.id, raw.display());
    }
    Ok((raw, canonical))
}

/// `path` (folder-relative, or absolute under the folder) as a `/`-joined relative path made only
/// of normal components.
fn relative(raw: &Path, canonical: &Path, path: &str) -> Result<String> {
    let given = Path::new(path);
    let rel = if given.is_absolute() {
        given
            .strip_prefix(canonical)
            .or_else(|_| given.strip_prefix(raw))
            .map_err(|_| Error(format!("파일이 공모전 폴더 밖에 있다: {path}")))?
    } else {
        given
    };
    let mut parts = Vec::new();
    for component in rel.components() {
        match component {
            Component::Normal(part) => match part.to_str() {
                Some(part) => parts.push(part),
                None => bail!("경로가 UTF-8이 아니다: {path}"),
            },
            Component::CurDir => {}
            _ => bail!("폴더 안의 상대 경로여야 한다(.. 금지): {path}"),
        }
    }
    if parts.is_empty() {
        bail!("파일 경로가 비어 있다: {path}");
    }
    Ok(parts.join("/"))
}

/// The regular file `rel` resolves to, refusing anything that leaves the folder through symlinks.
fn resolve(canonical: &Path, rel: &str) -> Result<PathBuf> {
    let full = canonical.join(rel).canonicalize().map_err(|err| Error(format!("{rel}: 파일을 열 수 없다: {err}")))?;
    if !full.starts_with(canonical) {
        bail!("{rel}: 실제 위치가 공모전 폴더 밖이다");
    }
    if !std::fs::metadata(&full)?.is_file() {
        bail!("{rel}: 일반 파일이 아니다");
    }
    Ok(full)
}

/// Check `[{path, sha256}]` against the contest folder as it is now and return it as
/// `[{path, sha256}]` with folder-relative paths and lowercase hashes. Non-empty, no duplicate
/// paths, regular files inside the folder (after symlinks), and each file's SHA-256 equal to the
/// given one. Only paths and hashes are kept, never contents.
pub fn normalize_artifacts(entry: &Entry, artifacts: &Value) -> Result<Vec<Value>> {
    let Some(items) = artifacts.as_array().filter(|a| !a.is_empty()) else {
        bail!("{}: artifacts는 비어 있지 않은 [{{path, sha256}}] 배열이어야 한다", entry.id)
    };
    let (raw, canonical) = folder(entry)?;
    let mut out: Vec<Value> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let Some(path) = item.get("path").and_then(Value::as_str).filter(|p| !p.trim().is_empty()) else {
            bail!("{}: artifacts[{i}].path가 비어 있다", entry.id)
        };
        let sha = item.get("sha256").and_then(Value::as_str).map(str::to_ascii_lowercase);
        let Some(sha) = sha.filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())) else {
            bail!("{}: artifacts[{i}].sha256은 64자리 16진수 SHA-256이어야 한다", entry.id)
        };
        let rel = relative(&raw, &canonical, path).map_err(|err| Error(format!("{}: {err}", entry.id)))?;
        if out.iter().any(|a| a["path"] == rel.as_str()) {
            bail!("{}: artifacts에 같은 파일이 두 번 있다: {rel}", entry.id);
        }
        let full = resolve(&canonical, &rel).map_err(|err| Error(format!("{}: {err}", entry.id)))?;
        let actual = cached_sha256(&full)?;
        if actual != sha {
            bail!("{}: {rel}의 SHA-256이 기록과 다르다(기록 {sha}, 지금 {actual})", entry.id);
        }
        out.push(json!({"path": rel, "sha256": sha}));
    }
    Ok(out)
}

/// `normalize_artifacts` without the result.
pub fn check_artifacts(entry: &Entry, artifacts: &Value) -> Result<()> {
    normalize_artifacts(entry, artifacts).map(|_| ())
}

/// The five typed fields of a readiness request.
struct Fields {
    submission_ready: bool,
    needs_user_input: bool,
    blockers: Vec<String>,
    followup_required: bool,
}

fn fields(entry: &Entry, raw: &Value) -> Result<Fields> {
    let Some(object) = raw.as_object() else { bail!("{}: 제출 준비 기록은 JSON 객체여야 한다", entry.id) };
    let flag = |key: &str| match object.get(key) {
        Some(Value::Bool(b)) => Ok(*b),
        _ => Err(Error(format!("{}: {key}는 true/false여야 한다", entry.id))),
    };
    let Some(list) = object.get("blockers").and_then(Value::as_array) else {
        bail!("{}: blockers는 문자열 배열이어야 한다(없으면 [])", entry.id)
    };
    let mut blockers = Vec::with_capacity(list.len());
    for item in list {
        match item.as_str().map(str::trim) {
            Some(text) if !text.is_empty() => blockers.push(text.to_string()),
            _ => bail!("{}: blockers의 항목은 비어 있지 않은 문자열이어야 한다", entry.id),
        }
    }
    Ok(Fields {
        submission_ready: flag("submission_ready")?,
        needs_user_input: flag("needs_user_input")?,
        blockers,
        followup_required: flag("followup_required")?,
    })
}

/// The next readiness sequence number (1 for the first record).
fn next_seq(entry: &Entry) -> u64 {
    history(entry).iter().chain(latest(entry)).filter_map(|r| r.get("seq").and_then(Value::as_u64)).max().unwrap_or(0)
        + 1
}

/// Keep the current latest record (whatever its shape) in the history and store `record` as latest.
fn push(entry: &mut Entry, record: Value) {
    if let Some(previous) = entry.extra.remove(KEY) {
        let list = entry.extra.entry(HISTORY_KEY.to_string()).or_insert_with(|| Value::Array(Vec::new()));
        if !list.is_array() {
            let old = list.take();
            *list = Value::Array(vec![old]);
        }
        if let Value::Array(items) = list {
            items.push(previous);
        }
    }
    entry.extra.insert(KEY.to_string(), record);
}

fn base_record(entry: &Entry, f: &Fields, artifacts: Vec<Value>) -> Map<String, Value> {
    let mut record = Map::new();
    record.insert("seq".into(), json!(next_seq(entry)));
    record.insert("at".into(), Value::String(now()));
    record.insert("revision".into(), json!(registry::revision(entry)));
    record.insert("state".into(), Value::String(entry.state.clone()));
    record.insert("submission_ready".into(), Value::Bool(f.submission_ready));
    record.insert("needs_user_input".into(), Value::Bool(f.needs_user_input));
    record.insert("blockers".into(), json!(f.blockers));
    record.insert("artifacts".into(), Value::Array(artifacts));
    record.insert("followup_required".into(), Value::Bool(f.followup_required));
    record.insert("rules_binding".into(), crate::rules::binding(entry));
    record
}

/// Record a readiness request (`ach readiness <id> --file F`) and return it. `raw` needs
/// `submission_ready`, `needs_user_input`, `followup_required` (booleans), `blockers` (strings)
/// and non-empty `artifacts [{path, sha256}]` matching the files now. Only in `running` or
/// `awaiting_approval`. Each record gets the next sequence number and the current work revision;
/// a new record changes the request token, so an earlier approval no longer counts.
pub fn record(entry: &mut Entry, raw: &Value) -> Result<Value> {
    if !RECORD_STATES.contains(&entry.state.as_str()) {
        bail!("{}: 제출 준비는 running이나 awaiting_approval에서만 기록한다(지금 {})", entry.id, entry.state);
    }
    if raw.get("personalization_of").is_some() || raw.get("personalized").is_some() {
        bail!("{}: 승인된 개인정보 반영은 ach readiness {} --personalized로 기록한다", entry.id, entry.id);
    }
    let f = fields(entry, raw)?;
    let artifacts = normalize_artifacts(entry, raw.get("artifacts").unwrap_or(&Value::Null))?;
    let mut record = Value::Object(base_record(entry, &f, artifacts));
    if let Some(value) = raw.get("destination") {
        let destination = value
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| Error("destination은 공고가 지정한 URL 또는 이메일 문자열이어야 한다".into()))?;
        if destination.trim() != destination || destination.chars().any(char::is_control) {
            bail!("destination에 앞뒤 공백이나 제어 문자를 넣을 수 없다");
        }
        if !(crate::text::is_web_url(destination) || valid_email(destination)) {
            bail!("destination은 http(s) URL 또는 이메일이어야 한다");
        }
        record["destination"] = json!(destination);
    }
    push(entry, record.clone());
    Ok(record)
}

fn valid_email(value: &str) -> bool {
    let value = value.strip_prefix("mailto:").unwrap_or(value);
    value.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty()
            && !domain.is_empty()
            && domain.contains('.')
            && !value.chars().any(char::is_whitespace)
            && !domain.contains('@')
            && !value.contains(['?', '#', '/', '\\'])
    })
}

/// A stored record read back strictly; `None` for an ambiguous (older or hand-edited) shape.
struct Stored<'a> {
    seq: u64,
    revision: u64,
    fields: Fields,
    artifacts: &'a Value,
    root_seq: Option<u64>,
}

fn stored<'a>(entry: &Entry, value: &'a Value) -> Option<Stored<'a>> {
    let artifacts = value.get("artifacts").filter(|a| a.as_array().is_some_and(|a| !a.is_empty()))?;
    let personalized = value.get("personalized").and_then(Value::as_bool) == Some(true);
    let root_seq = value.get("personalization_root_seq").and_then(Value::as_u64);
    if personalized != root_seq.is_some() {
        return None;
    }
    Some(Stored {
        seq: value.get("seq")?.as_u64()?,
        revision: value.get("revision")?.as_u64()?,
        fields: fields(entry, value).ok()?,
        artifacts,
        root_seq,
    })
}

fn find_seq(entry: &Entry, seq: u64) -> Option<&Value> {
    latest(entry).into_iter().chain(history(entry)).find(|r| r.get("seq").and_then(Value::as_u64) == Some(seq))
}

fn token(entry: &Entry, revision: usize, seq: u64, scores: usize) -> String {
    let binding = crate::rules::binding(entry);
    format!(
        "r{revision}-s{seq}-n{scores}-g{}-p{}-f{}-u{}",
        binding["gate"].as_u64().unwrap_or(0),
        binding["plan"].as_u64().unwrap_or(0),
        binding["final"].as_u64().unwrap_or(0),
        binding["submission"].as_u64().unwrap_or(0)
    )
}

/// The core approval conditions as of now.
pub struct Evaluation {
    pub revision: usize,
    /// `r<revision>-s<readiness seq>-n<score count>`; the seq is the original (pre-personalization)
    /// request. `None` without a readable readiness record.
    pub token: Option<String>,
    /// Stable reason codes; empty means eligible.
    pub reasons: Vec<&'static str>,
    /// One of [`SCORE_BASES`].
    pub basis: &'static str,
    pub readiness_seq: Option<u64>,
    pub scores_len: usize,
}

impl Evaluation {
    /// What a submission approval stores to stay bound to this request.
    pub fn binding(&self) -> Map<String, Value> {
        let mut binding = Map::new();
        binding.insert("revision".into(), json!(self.revision));
        binding.insert("request_token".into(), json!(self.token));
        binding.insert("readiness_seq".into(), json!(self.readiness_seq));
        binding.insert("scores_len".into(), json!(self.scores_len));
        binding.insert("score_basis".into(), Value::String(self.basis.to_string()));
        binding
    }
}

/// Push the stable reason codes for `record`; `original` selects the codes for the approved
/// request behind a personalized record.
fn check_stored(entry: &Entry, record: &Stored<'_>, revision: usize, original: bool, reasons: &mut Vec<&'static str>) {
    let pick = |plain: &'static str, root: &'static str| if original { root } else { plain };
    if record.revision != revision as u64 {
        reasons.push(pick("readiness_stale_revision", "original_readiness_stale_revision"));
    }
    if !record.fields.submission_ready {
        reasons.push(pick("readiness_not_ready", "original_readiness_not_ready"));
    }
    if record.fields.needs_user_input {
        reasons.push(pick("needs_user_input", "original_needs_user_input"));
    }
    if !record.fields.blockers.is_empty() {
        reasons.push(pick("blockers_present", "original_blockers_present"));
    }
    if check_artifacts(entry, record.artifacts).is_err() {
        reasons.push(pick("artifact_changed", "original_artifact_changed"));
    }
}

/// Evaluate the core submission-approval conditions: `awaiting_approval`; a current readiness
/// record (ready, no question, no blockers, files matching; a personalized record also needs its
/// original request intact); and from level 2 a plan score plus a current passing final whose
/// reviewed files include every original readiness file, unless `video_exempt` applies to a video
/// entry or `allow_override` accepts a recorded human score-gate override. Reasons are stable codes
/// only (no blocker text, paths or notes): automation logs them. Explicit commands (`ach
/// readiness`, `ach score`, `ach transition`) give the details.
pub fn evaluate(entry: &Entry, video_exempt: bool, allow_override: bool) -> Evaluation {
    let _hashes = HashScope::enter();
    let revision = registry::revision(entry);
    let mut reasons: Vec<&'static str> = Vec::new();
    reasons.extend(crate::rules::approval_problems(entry));
    if entry.state != "awaiting_approval" {
        reasons.push("state_not_awaiting_approval");
    }
    let mut readiness_seq = None;
    let mut reviewed: Option<&Value> = None;
    match (entry.extra.get(KEY), latest(entry).and_then(|v| stored(entry, v))) {
        (None, _) => reasons.push("readiness_missing"),
        (Some(_), None) => reasons.push("readiness_ambiguous"),
        (Some(_), Some(current)) => {
            check_stored(entry, &current, revision, false, &mut reasons);
            if crate::rules::status(entry)["tracked"] == true
                && latest(entry).and_then(|r| r.get("rules_binding")) != Some(&crate::rules::binding(entry))
            {
                reasons.push("readiness_rules_stale");
            }
            match current.root_seq {
                None => {
                    readiness_seq = Some(current.seq);
                    reviewed = Some(current.artifacts);
                }
                Some(root_seq) => match find_seq(entry, root_seq).and_then(|v| stored(entry, v)) {
                    Some(root) if root.root_seq.is_none() => {
                        check_stored(entry, &root, revision, true, &mut reasons);
                        readiness_seq = Some(root.seq);
                        reviewed = Some(root.artifacts);
                    }
                    _ => reasons.push("personalization_original_missing"),
                },
            }
        }
    }
    let scores_len = entry.scores.len();
    let token = readiness_seq.map(|seq| token(entry, revision, seq, scores_len));
    let basis = match entry.level {
        None => {
            reasons.push("level_missing");
            "none"
        }
        Some(level) if level < 2 => "none",
        Some(_) if video_exempt && entry.kind.as_deref() == Some("video") => "video_exempt",
        Some(_) => {
            let mut problems: Vec<&'static str> = registry::score_problems(entry).into_iter().map(|p| p.code).collect();
            let final_files =
                registry::latest_score(entry, Some("final")).and_then(|s| s.get("artifacts")).and_then(Value::as_array);
            if problems.is_empty()
                && let (Some(files), Some(final_files)) = (reviewed.and_then(Value::as_array), final_files)
                && files.iter().any(|f| {
                    !final_files.iter().any(|g| g.get("path") == f.get("path") && g.get("sha256") == f.get("sha256"))
                })
            {
                problems.push("submission_files_not_reviewed");
            }
            if problems.is_empty() {
                "score"
            } else if allow_override && registry::score_override_recorded(entry) {
                "override"
            } else {
                reasons.extend(problems);
                "score"
            }
        }
    };
    Evaluation { revision, token, reasons, basis, readiness_seq, scores_len }
}

/// The core approval status (`ach approval-status`): `{id, revision, request_token,
/// submit_approval_valid, eligible, reasons, readiness, latest_final}`. `eligible` never counts a
/// score-gate override; `video_exempt` only as granted by policy.
pub fn status(entry: &Entry, video_exempt: bool) -> Result<Value> {
    let _hashes = HashScope::enter();
    let eval = evaluate(entry, video_exempt, false);
    Ok(json!({
        "id": entry.id,
        "revision": eval.revision,
        "request_token": eval.token,
        "submit_approval_valid": registry::submit_approved(entry),
        "eligible": eval.reasons.is_empty(),
        "reasons": eval.reasons,
        "readiness": latest(entry),
        "latest_final": registry::latest_score(entry, Some("final")),
    }))
}

/// Approved personalization (`ach readiness <id> --file F --personalized`): after a valid
/// submission approval, put the approved personal fields or signature into separate final copies.
/// `raw` has the five readiness fields (ready, no question, no blockers) and `personalization_of`,
/// the approved request token. Each artifact names its original with `source` (or is the
/// unchanged original itself); every original is covered exactly once, a copy never overwrites an
/// original, and the originals must still hash as reviewed, so scores stay linked to them. The
/// request token does not change, so the approval keeps counting. Policy checks are the CLI's.
pub fn personalize(entry: &mut Entry, raw: &Value) -> Result<Value> {
    if entry.state != "awaiting_approval" {
        bail!("{}: 개인정보 반영은 제출 승인 뒤 awaiting_approval에서만 한다(지금 {})", entry.id, entry.state);
    }
    if !registry::submit_approved(entry) {
        bail!("{}: 지금 요청에 묶인 유효한 제출 승인이 없어 개인정보 반영을 기록하지 않는다", entry.id);
    }
    let f = fields(entry, raw)?;
    if !f.submission_ready || f.needs_user_input || !f.blockers.is_empty() {
        bail!("{}: 개인정보 반영 기록은 submission_ready=true, needs_user_input=false, blockers=[]여야 한다", entry.id);
    }
    let approved = evaluate(entry, false, true).token;
    let Some(of) = raw.get("personalization_of").and_then(Value::as_str) else {
        bail!("{}: personalization_of에 승인된 요청 토큰을 적는다", entry.id)
    };
    if approved.as_deref() != Some(of) {
        bail!(
            "{}: personalization_of({of})가 승인된 요청 토큰({})과 다르다",
            entry.id,
            approved.as_deref().unwrap_or("-")
        );
    }
    let view: &Entry = entry;
    let current = latest(view)
        .and_then(|v| stored(view, v))
        .ok_or_else(|| Error(format!("{}: 제출 준비 기록이 없다", view.id)))?;
    let root_seq = current.root_seq.unwrap_or(current.seq);
    let root = find_seq(view, root_seq)
        .and_then(|v| stored(view, v))
        .ok_or_else(|| Error(format!("{}: 원본 요청(seq {root_seq})을 찾을 수 없다", view.id)))?;
    if f.followup_required != root.fields.followup_required {
        bail!(
            "{}: 개인정보 반영으로 followup_required를 바꿀 수 없다(승인된 요청은 {}). 바뀌었으면 ach readiness로 새 요청을 기록하고 다시 승인받는다",
            view.id,
            root.fields.followup_required
        );
    }
    let originals = normalize_artifacts(view, root.artifacts)?;
    let items = raw.get("artifacts").and_then(Value::as_array).cloned().unwrap_or_default();
    let normalized = normalize_artifacts(view, &Value::Array(items.clone()))?;
    let (folder_raw, canonical) = folder(view)?;
    let mut covered = vec![false; originals.len()];
    let mut artifacts = Vec::with_capacity(normalized.len());
    for (item, file) in items.iter().zip(normalized) {
        let path = file["path"].as_str().unwrap_or_default().to_string();
        let source = match item.get("source") {
            None | Some(Value::Null) => path.clone(),
            Some(Value::String(s)) => {
                relative(&folder_raw, &canonical, s).map_err(|err| Error(format!("{}: {err}", entry.id)))?
            }
            Some(_) => bail!("{}: artifacts[].source는 원본 파일 경로 문자열이어야 한다", entry.id),
        };
        let Some(i) = originals.iter().position(|o| o["path"] == source.as_str()) else {
            bail!("{}: {path}의 원본({source})이 승인된 제출 파일에 없다", entry.id)
        };
        if std::mem::replace(&mut covered[i], true) {
            bail!("{}: 승인된 원본 {source}에 개인정보 반영 파일이 둘 이상이다", entry.id);
        }
        if path != source && originals.iter().any(|o| o["path"] == path.as_str()) {
            bail!("{}: {path}는 승인된 원본 파일이다. 원본은 그대로 두고 별도 사본에 반영한다", entry.id);
        }
        if path == source && file["sha256"] != originals[i]["sha256"] {
            bail!("{}: 원본 {path}이 바뀌었다. 원본은 그대로 두고 별도 사본에 반영한다", entry.id);
        }
        artifacts.push(json!({"path": path, "sha256": file["sha256"], "source": source}));
    }
    if let Some(i) = covered.iter().position(|c| !c) {
        bail!("{}: 승인된 원본 {}에 대응하는 제출 파일이 없다", entry.id, originals[i]["path"].as_str().unwrap_or("-"));
    }
    let mut record = base_record(entry, &f, artifacts);
    record.insert("personalized".into(), Value::Bool(true));
    record.insert("personalization_of".into(), Value::String(of.to_string()));
    record.insert("personalization_root_seq".into(), json!(root_seq));
    let record = Value::Object(record);
    push(entry, record.clone());
    if !registry::submit_approved(entry) {
        bail!("{}: 개인정보 반영 뒤 승인 확인이 실패했다. 기록하지 않았다", entry.id);
    }
    Ok(record)
}

/// The files `ach submit --files` names must be exactly the latest readiness artifacts (the
/// personalized copies after an approved personalization): regular files inside the folder,
/// without duplicates. Call it after `registry::check_submit_approved`, which verifies the hashes.
pub fn check_submission_files(entry: &Entry, files: &[String]) -> Result<()> {
    let Some(record) = latest(entry).and_then(|v| stored(entry, v)) else {
        bail!("{}: 제출 파일을 대조할 제출 준비 기록이 없다", entry.id)
    };
    // Hashes are not read again here: `registry::check_submit_approved` (run first by `ach
    // submit`) has just verified every recorded file. This only binds the named files to them.
    let recorded = record.artifacts.as_array().cloned().unwrap_or_default();
    let (raw, canonical) = folder(entry)?;
    let mut given: Vec<String> = Vec::with_capacity(files.len());
    for file in files {
        let rel = relative(&raw, &canonical, file).map_err(|err| Error(format!("{}: {err}", entry.id)))?;
        resolve(&canonical, &rel).map_err(|err| Error(format!("{}: {err}", entry.id)))?;
        if given.contains(&rel) {
            bail!("{}: --files에 같은 파일이 두 번 있다: {rel}", entry.id);
        }
        given.push(rel);
    }
    let expected: Vec<&str> = recorded.iter().filter_map(|a| a.get("path").and_then(Value::as_str)).collect();
    let extra: Vec<&str> = given.iter().map(String::as_str).filter(|g| !expected.contains(g)).collect();
    let missing: Vec<&str> = expected.iter().copied().filter(|e| !given.iter().any(|g| g == e)).collect();
    if !extra.is_empty() || !missing.is_empty() {
        bail!(
            "{}: 제출 파일이 승인된 제출 준비 기록과 다르다(기록에 없음: {}; 빠짐: {})",
            entry.id,
            if extra.is_empty() { "-".to_string() } else { extra.join(", ") },
            if missing.is_empty() { "-".to_string() } else { missing.join(", ") }
        );
    }
    Ok(())
}
