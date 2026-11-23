//! Submission-approval policy on top of the core conditions in [`crate::readiness`]: the standing
//! delegation (`config.delegation`), the persona guard, the evidence only a delegated approval
//! needs and the submission transaction. `ach approval-status`, `ach approve --delegated` and the
//! delegated handoff ([`crate::handoff`]) all decide through these functions, so they agree.

use serde_json::{Value, json};

use crate::bail;
use crate::config::Config;
use crate::error::Result;
use crate::registry::{self, Entry};
use crate::{evaluation, guard, readiness, rules, submission};

/// The actor every delegated approval is recorded under.
pub const DELEGATE: &str = "controller-delegated";

/// The latest submission approval of `entry` was a delegated one.
pub fn delegated_approval(entry: &Entry) -> bool {
    entry
        .approvals
        .iter()
        .rev()
        .find(|approval| approval.stage == "submit")
        .is_some_and(|approval| approval.extra.get("delegated") == Some(&Value::Bool(true)))
}

/// A delegated approval authorises submission steps only while the delegation still covers the
/// contest: a revoked delegation stops them.
pub fn check_submission_policy(cfg: &Config, entry: &Entry) -> Result<()> {
    if delegated_approval(entry) && !cfg.delegation.covers(&entry.id) {
        bail!("{}: 제출 위임이 철회됐거나 이 공모전은 위임 범위 밖이다", entry.id);
    }
    Ok(())
}

/// Persona-guard findings in the contest folder (none without a folder).
pub fn persona_findings(cfg: &Config, entry: &Entry) -> Result<Vec<guard::Finding>> {
    match &entry.folder {
        Some(folder) => guard::scan(std::path::Path::new(folder), &guard::terms(cfg)),
        None => Ok(Vec::new()),
    }
}

/// Evidence only delegated (automatic) approval needs; explicit human approvals keep the
/// historical path but still fail on known pending or invalidating rule changes.
pub fn delegated_evidence_problem(cfg: &Config, entry: &Entry) -> Option<&'static str> {
    if rules::status(entry)["tracked"] != true {
        return Some("rules_untracked");
    }
    let video_exempt = cfg.delegation.video_score_exempt && entry.kind.as_deref() == Some("video");
    if entry.level.unwrap_or(0) >= 2 && !video_exempt {
        return evaluation::final_problem(entry);
    }
    None
}

fn submission_reasons(entry: &Entry) -> Vec<String> {
    submission::status(entry)["blockers"]
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

/// `ach approval-status`: the core status plus the delegation, persona, delegated-evidence and
/// submission-transaction reasons. `eligible` means a delegated approval would be recorded now.
pub fn status(cfg: &Config, entry: &Entry) -> Result<Value> {
    let covered = cfg.delegation.covers(&entry.id);
    let mut status = readiness::status(entry, covered && cfg.delegation.video_score_exempt)?;
    let mut reasons = status["reasons"].as_array().cloned().unwrap_or_default();
    if !covered {
        reasons.push(json!("delegation_not_granted"));
        if delegated_approval(entry) {
            status["submit_approval_valid"] = json!(false);
        }
    }
    if !persona_findings(cfg, entry)?.is_empty() {
        reasons.push(json!("persona_findings"));
    }
    if covered && let Some(reason) = delegated_evidence_problem(cfg, entry) {
        reasons.push(json!(reason));
    }
    for reason in submission_reasons(entry) {
        reasons.push(json!(reason));
    }
    status["eligible"] = json!(status["eligible"] == true && reasons.is_empty());
    status["reasons"] = json!(reasons);
    status["allow_personalization"] = json!(
        (covered && cfg.delegation.allow_personalization)
            || (status["submit_approval_valid"] == true && !delegated_approval(entry))
    );
    Ok(status)
}

/// Record a delegated submission approval on `entry` (the caller holds the registry write lock):
/// the delegation must cover the contest, the persona guard and the delegated evidence must pass,
/// and the core conditions and `expected_token` are rechecked by [`registry::approve_delegated`].
/// `session` is `ACH_SESSION` as the caller read it; a contest session never approves.
pub fn approve_delegated(
    cfg: &Config,
    entry: &mut Entry,
    note: &str,
    session: Option<&str>,
    expected_token: &str,
) -> Result<&'static str> {
    let id = entry.id.clone();
    if !cfg.delegation.covers(&id) {
        bail!("{id}: 현재 유효한 사용자 제출 위임이 없다");
    }
    if !persona_findings(cfg, entry)?.is_empty() {
        bail!("{id}: 페르소나 검사에 실패하여 자동 승인하지 않는다");
    }
    if let Some(reason) = delegated_evidence_problem(cfg, entry) {
        bail!("{id}: 위임 승인에 필요한 근거가 없다({reason})");
    }
    let note = format!("{}\n위임 시각: {}\n위임 원문: {}", note, cfg.delegation.granted_at, cfg.delegation.note);
    registry::approve_delegated(entry, &note, DELEGATE, session, expected_token, cfg.delegation.video_score_exempt)
}
