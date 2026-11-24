//! Required workflow evidence. A gate may demand proof beside the quality scores
//! (`required_evidence`, a list of codes) and name the models allowed to draft and to edit the
//! prose (`writing_models`). The readiness record carries the proof as files (`evidence`, a map
//! from code to `[{path, sha256}]`). A missing or changed file blocks every approval, whatever the
//! latest score says. For `draft_execution` and `editor_execution` the files must be the complete
//! evidence of at least one run of `controllers.writing` for this contest and work revision, by a
//! model the gate allows, and the runner's own event output must name that model and hold the
//! candidate. This checks that the local records agree with each other; it is not an attestation
//! by the model provider and says nothing about the quality of the text.
//! Contract: `docs/CONTRACT.md` section 24.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::bail;
use crate::error::Result;
use crate::readiness;
use crate::registry::{self, Entry};

/// Key of the proof files in a readiness record.
pub const KEY: &str = "evidence";
/// Evidence codes that need a verified writing run, with the `writing_models` role they belong to.
pub const EXECUTION: &[(&str, &str)] = &[("draft_execution", "draft"), ("editor_execution", "edit")];
/// Provider part of a model selector that runs on the Antigravity CLI instead of omp.
const AGY: &str = "agy";

fn stable_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

fn valid_pattern(pattern: &str) -> bool {
    !pattern.is_empty()
        && pattern.len() <= 160
        && pattern.bytes().all(|b| b.is_ascii_alphanumeric() || b"_./:~*-".contains(&b))
}

/// `*` matches any run of characters; everything else is literal.
fn pattern_matches(pattern: &str, model: &str) -> bool {
    valid_pattern(pattern)
        && regex::Regex::new(&format!("^{}$", regex::escape(pattern).replace("\\*", ".*")))
            .is_ok_and(|r| r.is_match(model))
}

/// Check a gate's `writing_models` and `required_evidence` and return the required codes.
/// `writing_models` maps `draft` and `edit` to non-empty lists of `provider/model` patterns;
/// `required_evidence` is a list of distinct codes, and an execution code needs its role's models.
pub fn required(gate: &Value) -> Result<Vec<&str>> {
    if let Some(models) = gate.get("writing_models") {
        let Some(roles) = models.as_object() else {
            bail!("writing_models는 역할별 모델 패턴 객체여야 한다")
        };
        for (role, patterns) in roles {
            let known = EXECUTION.iter().any(|(_, r)| r == role);
            let valid = patterns
                .as_array()
                .is_some_and(|items| !items.is_empty() && items.iter().all(|p| p.as_str().is_some_and(valid_pattern)));
            if !known || !valid {
                bail!(
                    "writing_models.{role}: 역할은 draft·edit이고 값은 비어 있지 않은 provider/model 패턴 배열이어야 한다"
                )
            }
        }
    }
    let mut ids = Vec::new();
    if let Some(raw) = gate.get("required_evidence") {
        let Some(items) = raw.as_array() else { bail!("required_evidence는 코드 배열이어야 한다") };
        for item in items {
            let Some(id) = item.as_str().filter(|id| stable_code(id)) else {
                bail!("required_evidence의 코드는 영문 소문자·숫자·밑줄 64자 이하여야 한다")
            };
            if ids.contains(&id) {
                bail!("required_evidence에 같은 코드가 두 번 있다: {id}")
            }
            ids.push(id);
        }
    }
    for (id, role) in EXECUTION {
        if ids.contains(id) && gate.get("writing_models").and_then(|m| m.get(role)).is_none() {
            bail!("required_evidence의 {id}에는 writing_models.{role}이 있어야 한다")
        }
    }
    Ok(ids)
}

/// The `evidence` field of a readiness request as stored: every code's files checked against the
/// contest folder as they are now ([`readiness::normalize_artifacts`]).
pub fn normalize(entry: &Entry, raw: &Value) -> Result<Value> {
    let Some(items) = raw.as_object() else {
        bail!("{}: evidence는 코드별 [{{path, sha256}}] 객체여야 한다", entry.id)
    };
    let mut normalized = Map::new();
    for (id, files) in items {
        if !stable_code(id) {
            bail!("{}: evidence의 코드는 영문 소문자·숫자·밑줄 64자 이하여야 한다", entry.id)
        }
        normalized.insert(id.clone(), json!(readiness::normalize_artifacts(entry, files)?));
    }
    Ok(Value::Object(normalized))
}

/// The part of [`problems`] the stored records alone give, without reading any file: an unreadable
/// declaration or a required code the record has no files for.
pub fn stored_problems(entry: &Entry, record: Option<&Value>) -> Vec<&'static str> {
    let Some(gate) = &entry.gate else { return Vec::new() };
    let Ok(ids) = required(gate) else { return vec!["required_evidence_ambiguous"] };
    let proof = record.and_then(|r| r.get(KEY));
    if ids.iter().any(|id| proof.and_then(|v| v.get(id)).is_none()) {
        vec!["required_evidence_missing"]
    } else {
        Vec::new()
    }
}

/// Why the required evidence does not hold for `record` (the original readiness request), as
/// stable codes: `required_evidence_ambiguous` (the gate's declaration cannot be read),
/// `required_evidence_missing`, `required_evidence_changed` (a file is gone or hashes differently),
/// `required_execution_unverified` (the files are not the evidence of a run by an allowed model).
pub fn problems(entry: &Entry, record: Option<&Value>) -> Vec<&'static str> {
    let Some(gate) = &entry.gate else { return Vec::new() };
    let Ok(ids) = required(gate) else { return vec!["required_evidence_ambiguous"] };
    let (mut missing, mut changed, mut unverified) = (false, false, false);
    for id in ids {
        match record.and_then(|r| r.get(KEY)).and_then(|v| v.get(id)) {
            None => missing = true,
            Some(files) if readiness::check_artifacts(entry, files).is_err() => changed = true,
            Some(files) => {
                if let Some((_, role)) = EXECUTION.iter().find(|(code, _)| *code == id) {
                    unverified |= !runs_verified(entry, gate, files, role);
                }
            }
        }
    }
    let mut out = Vec::new();
    for (flag, code) in [
        (missing, "required_evidence_missing"),
        (changed, "required_evidence_changed"),
        (unverified, "required_execution_unverified"),
    ] {
        if flag {
            out.push(code);
        }
    }
    out
}

/// `files` (already checked against the folder) hold at least one run and nothing but complete,
/// verified runs of `role`.
fn runs_verified(entry: &Entry, gate: &Value, files: &Value, role: &str) -> bool {
    let Ok((_, root)) = readiness::folder(entry) else { return false };
    let Some(items) = files.as_array() else { return false };
    // Group the listed files by their directory: one run per directory.
    let mut runs: BTreeMap<&str, BTreeMap<&str, &str>> = BTreeMap::new();
    for item in items {
        let (Some(path), Some(hash)) = (item["path"].as_str(), item["sha256"].as_str()) else { return false };
        let Some((dir, name)) = path.rsplit_once('/') else { return false };
        runs.entry(dir).or_default().insert(name, hash);
    }
    !runs.is_empty()
        && runs.iter().all(|(dir, listed)| run_verified(entry, gate, &root.join(dir), listed, role).is_some())
}

fn run_verified(entry: &Entry, gate: &Value, dir: &Path, listed: &BTreeMap<&str, &str>, role: &str) -> Option<()> {
    let read = |name: &str| listed.contains_key(name).then(|| std::fs::read(dir.join(name)).ok()).flatten();
    let receipt: Value = serde_json::from_slice(&read("receipt.json")?).ok()?;
    let requested = receipt["requested_model"].as_str()?;
    let runner = receipt["runner"].as_str()?;
    let patterns = gate.get("writing_models")?.get(role)?.as_array()?;
    if receipt["version"] != 1
        || receipt["stage"] != role
        || receipt["code"] != "candidate_generated"
        || receipt["returncode"] != 0
        || receipt["contest_id"] != entry.id
        || receipt["revision"] != json!(registry::revision(entry))
        || receipt["allowed_models"] != json!(patterns)
        || receipt["observed_models"] != json!([requested])
        || !patterns.iter().any(|p| p.as_str().is_some_and(|p| pattern_matches(p, requested)))
        || (runner == "agy") != requested.starts_with(&format!("{AGY}/"))
    {
        return None;
    }
    // The receipt names every file of the run with its hash; the readiness record must list the
    // same files with the same hashes (those were just compared with the files themselves).
    let named = receipt["files"].as_object()?;
    let mut expected: BTreeSet<&str> =
        ["input.md", "prompt.md", "writing-models.json", "runner-policy.json", "stdout.jsonl", "stderr.log"].into();
    expected.insert("candidate.md");
    if runner == "omp" {
        expected.extend(["runner-guard.ts", "system.md"]);
    }
    if named.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected
        || listed.len() != expected.len() + 1
        || named.iter().any(|(name, hash)| hash.as_str() != listed.get(name.as_str()).copied())
    {
        return None;
    }
    if serde_json::from_slice::<Value>(&read("writing-models.json")?).ok()? != json!(patterns) {
        return None;
    }
    let policy: Value = serde_json::from_slice(&read("runner-policy.json")?).ok()?;
    let trace = String::from_utf8(read("stdout.jsonl")?).ok()?;
    let candidate = match runner {
        "omp" => {
            if policy
                != json!({"retry": {"enabled": false, "modelFallback": false, "usageAwareFallback": false},
                    "contextPromotion": {"enabled": false}})
            {
                return None;
            }
            omp_candidate(&trace, requested)?
        }
        "agy" => {
            if policy != json!({"permissions": {"allow": [], "deny": ["*"]}}) {
                return None;
            }
            agy_candidate(&trace, requested)?
        }
        _ => return None,
    };
    (!candidate.trim().is_empty() && read("candidate.md")? == candidate.as_bytes()).then_some(())
}

fn events(trace: &str) -> Option<Vec<Value>> {
    trace.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).ok()).collect()
}

/// omp `--mode json`: exactly one assistant message, by the requested model, ended normally and
/// without a tool call, and the run ended.
fn omp_candidate(trace: &str, requested: &str) -> Option<String> {
    let events = events(trace)?;
    let messages: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "message_end" && e["message"]["role"] == "assistant")
        .map(|e| &e["message"])
        .collect();
    let [message] = messages[..] else { return None };
    if !events.iter().any(|e| e["type"] == "agent_end")
        || message["stopReason"] != "stop"
        || format!("{}/{}", message["provider"].as_str()?, message["model"].as_str()?) != requested
    {
        return None;
    }
    let content = message["content"].as_array()?;
    if content.iter().any(|p| p["type"] == "toolCall") {
        return None;
    }
    let parts: Option<Vec<&str>> = content.iter().filter(|p| p["type"] == "text").map(|p| p["text"].as_str()).collect();
    Some(parts?.join("\n"))
}

/// agy `--output-format stream-json`: one session of the requested model, nothing but the user's
/// input and the model's answer as steps, and one successful result of one turn.
fn agy_candidate(trace: &str, requested: &str) -> Option<String> {
    let events = events(trace)?;
    let inits: Vec<&Value> = events.iter().filter(|e| e["event"] == "init").collect();
    let results: Vec<&Value> = events.iter().filter(|e| e["event"] == "result").map(|e| &e["result"]).collect();
    let ([init], [result]) = (&inits[..], &results[..]) else { return None };
    if format!("{AGY}/{}", init["init"]["model"].as_str()?) != requested
        || result["status"] != "SUCCESS"
        || !result["finished_reason"].is_null()
        || result["num_turns"] != 1
        || result["conversation_id"] != init["conversation_id"]
        || events
            .iter()
            .filter(|e| e["event"] == "step_update")
            .any(|e| !["user_input", "agent_response"].contains(&e["step_update"]["step_type"].as_str().unwrap_or("")))
    {
        return None;
    }
    result["response"].as_str().map(str::to_string)
}
