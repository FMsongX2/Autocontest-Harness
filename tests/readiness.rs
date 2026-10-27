//! Structured readiness, the core approval status, revision- and token-bound submission
//! approvals (human, delegated, score override, video exemption), approved personalization and
//! the submit file check.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use harness::readiness::{self, check_submission_files, personalize, status};
use harness::registry::{
    self, Entry, NewEntry, apply_gate, approve, approve_delegated, check_personal_access, check_submit_approved,
    new_entry, record_score, submit_approved, transition,
};
use serde_json::{Value, json};

fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "harness-readiness-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A running entry of `level` and `kind` with a folder holding `form.md`.
fn running(id: &str, level: u8, kind: &str) -> Entry {
    let mut e = new_entry(NewEntry {
        id,
        title: "공모전",
        url: "https://e.com",
        source: "manual",
        organizer: "",
        deadline: None,
        categories: vec![],
        note: None,
    });
    let gate = json!({"decision": "pass", "type": kind, "level": level,
        "criteria": [{"name": "완성도", "weight": 100}], "reasons": ["근거"]});
    apply_gate(&mut e, gate, "ContestGate").unwrap();
    let dir = temp_dir(id);
    fs::write(dir.join("form.md"), "신청서 본문\n").unwrap();
    fs::write(
        dir.join("RUBRIC.md"),
        "## 0. 채점 근거\n\n| 항목 | 내용 | 공고 원문 위치 |\n|---|---|---|\n| 주최 | 기관 | 1쪽 |\n\
         | 주관 | 공고에 없음 | 1쪽 확인 |\n| 개최 목적 | 목적 | 1쪽 |\n| 심사기준 | 완성도 100 | 3쪽 |\n",
    )
    .unwrap();
    e.folder = Some(dir.to_string_lossy().into_owned());
    transition(&mut e, "routed", "t", "").unwrap();
    transition(&mut e, "running", "t", "").unwrap();
    e
}

fn folder(e: &Entry) -> PathBuf {
    PathBuf::from(e.folder.as_deref().unwrap())
}

fn files(e: &Entry, names: &[&str]) -> Value {
    Value::Array(
        names
            .iter()
            .map(|n| json!({"path": n, "sha256": readiness::sha256_file(&folder(e).join(n)).unwrap()}))
            .collect(),
    )
}

fn request(e: &Entry, names: &[&str]) -> Value {
    json!({"submission_ready": true, "needs_user_input": false, "blockers": [], "followup_required": false,
        "artifacts": files(e, names)})
}

/// Record a readiness request for `names` as they are now.
fn record(e: &mut Entry, names: &[&str]) -> harness::error::Result<Value> {
    let raw = request(e, names);
    readiness::record(e, &raw)
}

fn score(e: &mut Entry, stage: &str, total: f64, verdict: &str) {
    let mut j =
        json!({"total": total, "verdict": verdict, "scores": [{"name": "완성도", "weight": 100, "score": total}]});
    if stage == "final" {
        j["artifacts"] = files(e, &["form.md"]);
    }
    if stage == "plan" {
        j["basis"] = json!({"organizer": "주최", "purpose": "목적", "criteria_source": "공고 심사기준"});
        j["evaluator"] =
            json!({"id": "ContestJudge", "model": "stub/judge", "run_id": format!("run-{}", e.scores.len())});
    }
    record_score(e, &j, stage, "judge.json", "").unwrap();
}

/// A level 2 entry with plan + passing final, in awaiting_approval with a ready request.
fn eligible(id: &str) -> Entry {
    let mut e = running(id, 2, "idea");
    score(&mut e, "plan", 80.0, "revise");
    score(&mut e, "final", 93.0, "ready");
    transition(&mut e, "awaiting_approval", "t", "").unwrap();
    record(&mut e, &["form.md"]).unwrap();
    e
}

fn reasons(e: &Entry, video_exempt: bool) -> Vec<String> {
    status(e, video_exempt).unwrap()["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect()
}

fn token(e: &Entry) -> String {
    status(e, false).unwrap()["request_token"].as_str().unwrap().to_string()
}

#[test]
fn readiness_needs_typed_fields_and_current_files() {
    let mut e = running("r-typed", 1, "essay");
    let ok = request(&e, &["form.md"]);
    let refused = |e: &mut Entry, patch: Value| {
        let mut raw = ok.clone();
        for (k, v) in patch.as_object().unwrap() {
            if v.is_null() {
                raw.as_object_mut().unwrap().remove(k);
            } else {
                raw[k] = v.clone();
            }
        }
        readiness::record(e, &raw).is_err()
    };
    for key in ["submission_ready", "needs_user_input", "blockers", "artifacts", "followup_required"] {
        assert!(refused(&mut e, json!({key: null})), "{key} is required");
    }
    assert!(refused(&mut e, json!({"submission_ready": "true"})), "no prose booleans");
    assert!(refused(&mut e, json!({"needs_user_input": 0})));
    assert!(refused(&mut e, json!({"blockers": "없음"})));
    assert!(refused(&mut e, json!({"blockers": [""]})));
    assert!(refused(&mut e, json!({"artifacts": []})));
    assert!(refused(&mut e, json!({"artifacts": [{"path": "form.md", "sha256": "abc"}]})));
    assert!(refused(&mut e, json!({"artifacts": [{"path": "form.md", "sha256": "0".repeat(64)}]})));
    assert!(refused(&mut e, json!({"artifacts": [{"path": "../x.md", "sha256": "0".repeat(64)}]})));
    let twice = json!([ok["artifacts"][0], ok["artifacts"][0]]);
    assert!(refused(&mut e, json!({"artifacts": twice})));
    fs::create_dir_all(folder(&e).join("dir")).unwrap();
    assert!(refused(&mut e, json!({"artifacts": [{"path": "dir", "sha256": "0".repeat(64)}]})), "regular files only");
    assert!(refused(&mut e, json!({"personalization_of": "r1-s1-n0"})), "only through personalize");
    assert!(readiness::latest(&e).is_none(), "nothing stored on a refusal");

    let first = readiness::record(&mut e, &ok).unwrap();
    assert_eq!(first["seq"], 1);
    assert_eq!(first["revision"].as_u64(), Some(registry::revision(&e) as u64));
    assert_eq!(first["artifacts"][0]["path"], "form.md");
    assert!(first.get("note").is_none() && first["artifacts"][0].get("content").is_none());
    let second = readiness::record(&mut e, &ok).unwrap();
    assert_eq!(second["seq"], 2, "sequence numbers only grow");
    assert_eq!(readiness::latest(&e).unwrap()["seq"], 2);
    assert_eq!(e.extra[readiness::HISTORY_KEY].as_array().unwrap().len(), 1);
    assert_eq!(readiness::latest(&e).unwrap()["followup_required"], false);

    // Only while working or waiting for approval.
    let mut done = running("r-state", 1, "essay");
    transition(&mut done, "awaiting_approval", "t", "").unwrap();
    transition(&mut done, "submitted", "t", "").unwrap();
    assert!(record(&mut done, &["form.md"]).is_err());
}

#[test]
fn status_reports_stable_reason_codes_and_a_request_token() {
    let mut e = running("r-status", 2, "idea");
    let s = status(&e, false).unwrap();
    assert_eq!(s["eligible"], false);
    assert_eq!(s["request_token"], Value::Null);
    assert_eq!(s["submit_approval_valid"], false);
    let r = reasons(&e, false);
    for code in ["state_not_awaiting_approval", "readiness_missing", "plan_score_missing", "final_score_missing"] {
        assert!(r.contains(&code.to_string()), "{code}: {r:?}");
    }

    let mut blocked = request(&e, &["form.md"]);
    blocked["blockers"] = json!(["/Users/someone/개인 파일.pdf 서명 필요"]);
    blocked["needs_user_input"] = json!(true);
    readiness::record(&mut e, &blocked).unwrap();
    let r = reasons(&e, false);
    assert!(r.contains(&"blockers_present".to_string()) && r.contains(&"needs_user_input".to_string()));
    assert!(r.iter().all(|c| !c.contains('/') && !c.contains(' ')), "codes only: {r:?}");

    score(&mut e, "plan", 80.0, "revise");
    score(&mut e, "final", 93.0, "ready");
    transition(&mut e, "awaiting_approval", "t", "").unwrap();
    record(&mut e, &["form.md"]).unwrap();
    let s = status(&e, false).unwrap();
    assert_eq!(s["eligible"], true, "{s}");
    assert_eq!(s["reasons"], json!([]));
    let rev = registry::revision(&e);
    assert_eq!(s["request_token"], format!("r{rev}-s2-n2-g0-p0-f0-u0"));
    assert_eq!(s["revision"].as_u64(), Some(rev as u64));
    assert_eq!(s["readiness"]["seq"], 2);
    assert_eq!(s["latest_final"]["total"].as_f64(), Some(93.0));

    // A readiness file the final did not review.
    fs::write(folder(&e).join("extra.md"), "추가\n").unwrap();
    record(&mut e, &["form.md", "extra.md"]).unwrap();
    assert_eq!(reasons(&e, false), ["submission_files_not_reviewed"]);

    // Ambiguous legacy readiness never counts.
    let mut legacy = eligible("r-legacy");
    legacy.extra.insert(readiness::KEY.into(), json!({"ready": "네, 제출 준비 완료"}));
    assert_eq!(reasons(&legacy, false), ["readiness_ambiguous"]);
    assert_eq!(status(&legacy, false).unwrap()["request_token"], Value::Null);
}

#[test]
fn human_approval_binds_to_the_current_request_and_fails_closed() {
    let mut e = eligible("r-human");
    assert!(!submit_approved(&e));
    assert!(check_personal_access(&e, None).is_err());
    assert_eq!(approve(&mut e, "제출해 10/25 10:00", "user", None).unwrap(), "submit");
    assert!(submit_approved(&e));
    assert!(check_submit_approved(&e).is_ok());
    assert!(check_personal_access(&e, Some("r-human")).is_ok());
    assert_eq!(status(&e, false).unwrap()["submit_approval_valid"], true);
    let approval = e.approvals.last().unwrap();
    assert_eq!(approval.extra["request_token"], token(&e).as_str());
    assert_eq!(approval.extra["score_basis"], "score");
    assert_eq!(approval.extra["revision"].as_u64(), Some(registry::revision(&e) as u64));

    // A changed file voids it; restoring the file brings it back.
    let form = folder(&e).join("form.md");
    fs::write(&form, "몰래 바뀐 본문\n").unwrap();
    assert!(!submit_approved(&e));
    assert!(check_submit_approved(&e).is_err());
    assert!(check_personal_access(&e, None).is_err());
    fs::write(&form, "신청서 본문\n").unwrap();
    assert!(submit_approved(&e));

    // A new readiness record for the same request voids the older approval.
    record(&mut e, &["form.md"]).unwrap();
    assert!(!submit_approved(&e));
    approve(&mut e, "다시 승인", "user", None).unwrap();
    assert!(submit_approved(&e));

    // A new final after the approval (here a reject) voids it, and the reject blocks reapproval.
    score(&mut e, "final", 40.0, "reject");
    assert!(!submit_approved(&e));
    assert!(approve(&mut e, "그래도 승인", "user", None).is_err());
    score(&mut e, "final", 94.0, "ready");
    approve(&mut e, "고친 뒤 승인", "user", None).unwrap();
    assert!(submit_approved(&e));

    // A revise cycle makes readiness, final and approval stale; reapproval needs new ones.
    transition(&mut e, "running", "t", "수정").unwrap();
    transition(&mut e, "awaiting_approval", "t", "").unwrap();
    assert!(!submit_approved(&e));
    let r = reasons(&e, false);
    assert!(
        r.contains(&"readiness_stale_revision".to_string()) && r.contains(&"final_stale_revision".to_string()),
        "{r:?}"
    );
    assert!(approve(&mut e, "재승인", "user", None).is_err());
    score(&mut e, "final", 95.0, "ready");
    record(&mut e, &["form.md"]).unwrap();
    approve(&mut e, "재승인", "user", None).unwrap();
    assert!(submit_approved(&e));

    // Not from a contest session, never without the user's words.
    let mut other = eligible("r-human-2");
    assert!(approve(&mut other, "승인", "user", Some("r-human-2")).is_err());
    assert!(approve(&mut other, "  ", "user", None).is_err());
}

#[test]
fn historical_approvals_without_binding_never_count() {
    let mut e = eligible("r-hist");
    let mut marker = registry::HistoryItem::new(
        "2026-10-01T10:00+09:00".into(),
        Some("awaiting_approval".into()),
        "awaiting_approval",
        "user",
        "제출해",
    );
    marker.extra.insert("approval".into(), json!("submit"));
    e.history.push(marker);
    assert!(!submit_approved(&e));
    assert!(check_submit_approved(&e).is_err());
    assert!(check_personal_access(&e, None).is_err());
}

#[test]
fn a_human_score_override_needs_the_typed_marker_and_never_makes_status_eligible() {
    let mut e = running("r-override", 2, "idea");
    score(&mut e, "plan", 80.0, "revise");
    let note = registry::score_gate(&e, "awaiting_approval", Some("마감 직전, 사용자 확인")).unwrap().unwrap();
    // A note that only looks like an override counts for nothing.
    transition(&mut e, "awaiting_approval", "t", &note).unwrap();
    record(&mut e, &["form.md"]).unwrap();
    assert!(approve(&mut e, "제출해", "user", None).is_err());

    // The CLI marks the move it let through with the typed marker.
    let rev = registry::revision(&e);
    let entered = e.history.iter().rposition(|h| h.to == "awaiting_approval").unwrap();
    e.history[entered].extra.insert("score_override".into(), json!(true));
    e.history[entered].extra.insert("score_override_revision".into(), json!(rev));
    assert!(registry::score_override_recorded(&e));
    assert_eq!(status(&e, false).unwrap()["eligible"], false, "status never counts an override");
    let t = token(&e);
    assert!(approve_delegated(&mut e, "위임 승인", "controller-delegated", None, &t, false).is_err());
    approve(&mut e, "제출해, 점수 예외 확인", "user", None).unwrap();
    assert_eq!(e.approvals.last().unwrap().extra["score_basis"], "override");
    assert!(submit_approved(&e));

    // A marker from an earlier revision does not carry over.
    transition(&mut e, "running", "t", "").unwrap();
    transition(&mut e, "awaiting_approval", "t", "").unwrap();
    let entered = e.history.len() - 1;
    e.history[entered].extra.insert("score_override".into(), json!(true));
    e.history[entered].extra.insert("score_override_revision".into(), json!(rev));
    assert!(!registry::score_override_recorded(&e));
}

#[test]
fn delegated_approval_needs_eligibility_and_the_expected_token() {
    let mut e = eligible("r-deleg");
    let t = token(&e);
    assert!(approve_delegated(&mut e, "위임", "controller-delegated", None, "r0-s1-n0", false).is_err());
    assert!(approve_delegated(&mut e, "위임", "controller-delegated", Some("r-deleg"), &t, false).is_err());
    assert_eq!(approve_delegated(&mut e, "위임", "controller-delegated", None, &t, false).unwrap(), "submit");
    assert_eq!(e.approvals.last().unwrap().extra["delegated"], true);
    assert!(submit_approved(&e));

    // Never a plan approval.
    let mut plan = running("r-plan", 5, "software");
    score(&mut plan, "plan", 80.0, "revise");
    transition(&mut plan, "plan_review", "t", "").unwrap();
    assert!(approve_delegated(&mut plan, "위임", "controller-delegated", None, "x", false).is_err());
    assert_eq!(plan.state, "plan_review");

    // A blocker is never delegated past.
    let mut blocked = eligible("r-deleg-2");
    let mut raw = request(&blocked, &["form.md"]);
    raw["blockers"] = json!(["수상 시 수수료"]);
    readiness::record(&mut blocked, &raw).unwrap();
    let t = token(&blocked);
    assert!(approve_delegated(&mut blocked, "위임", "controller-delegated", None, &t, false).is_err());
}

#[test]
fn level_1_needs_no_score_and_video_is_exempt_only_by_flag() {
    let mut l1 = running("r-l1", 1, "essay");
    transition(&mut l1, "awaiting_approval", "t", "").unwrap();
    record(&mut l1, &["form.md"]).unwrap();
    assert_eq!(status(&l1, false).unwrap()["eligible"], true);

    let mut video = running("r-video", 2, "video");
    transition(&mut video, "awaiting_approval", "t", "").unwrap();
    record(&mut video, &["form.md"]).unwrap();
    assert_eq!(status(&video, false).unwrap()["eligible"], false);
    assert_eq!(status(&video, true).unwrap()["eligible"], true);
    let t = token(&video);
    assert!(approve_delegated(&mut video, "위임", "controller-delegated", None, &t, false).is_err());
    approve_delegated(&mut video, "위임", "controller-delegated", None, &t, true).unwrap();
    assert_eq!(video.approvals.last().unwrap().extra["score_basis"], "video_exempt");
    assert!(submit_approved(&video));

    // The flag never exempts other kinds.
    let idea = running("r-idea", 2, "idea");
    let mut idea = idea;
    transition(&mut idea, "awaiting_approval", "t", "").unwrap();
    record(&mut idea, &["form.md"]).unwrap();
    assert_eq!(status(&idea, true).unwrap()["eligible"], false);
}

#[test]
fn approved_personalization_uses_separate_copies_and_keeps_the_approval() {
    let mut e = eligible("r-person");
    fs::write(folder(&e).join("form-final.md"), "신청서 본문\n이름: (승인된 개인정보)\n").unwrap();
    let personal = |of: &str, artifacts: Value| {
        json!({"submission_ready": true, "needs_user_input": false, "blockers": [], "followup_required": false,
            "personalization_of": of, "artifacts": artifacts})
    };
    let copy = |e: &Entry| {
        let mut list = files(e, &["form-final.md"]);
        list[0]["source"] = json!("form.md");
        list
    };
    let t = token(&e);
    let raw = personal(&t, copy(&e));
    assert!(personalize(&mut e, &raw).is_err(), "no approval yet");
    approve(&mut e, "제출해", "user", None).unwrap();
    let t = token(&e);

    let raw = personal("r1-s9-n9", copy(&e));
    assert!(personalize(&mut e, &raw).is_err(), "wrong request token");
    let mut no_link = personal(&t, copy(&e));
    no_link.as_object_mut().unwrap().remove("personalization_of");
    assert!(personalize(&mut e, &no_link).is_err());
    let mut unlinked = copy(&e);
    unlinked[0].as_object_mut().unwrap().remove("source");
    let raw = personal(&t, unlinked);
    assert!(personalize(&mut e, &raw).is_err(), "a new file needs its original");
    fs::write(folder(&e).join("other.md"), "다른 내용\n").unwrap();
    let mut both = copy(&e);
    both.as_array_mut().unwrap().push(json!({"path": "other.md",
        "sha256": readiness::sha256_file(&folder(&e).join("other.md")).unwrap(), "source": "form.md"}));
    let raw = personal(&t, both);
    assert!(personalize(&mut e, &raw).is_err(), "one copy per original");
    let mut asking = personal(&t, copy(&e));
    asking["needs_user_input"] = json!(true);
    assert!(personalize(&mut e, &asking).is_err());
    let mut flipped = personal(&t, copy(&e));
    flipped["followup_required"] = json!(true);
    assert!(personalize(&mut e, &flipped).is_err(), "followup_required is the approved request's");
    assert!(submit_approved(&e), "refusals change nothing");

    let raw = personal(&t, copy(&e));
    let record = personalize(&mut e, &raw).unwrap();
    assert_eq!(record["personalized"], true);
    assert_eq!(record["personalization_of"], t.as_str());
    assert_eq!(record["artifacts"][0]["source"], "form.md");
    assert!(submit_approved(&e), "the approval carries to the copy");
    assert_eq!(token(&e), t);
    assert_eq!(status(&e, false).unwrap()["eligible"], true);

    // Submit names exactly the personalized copy.
    let copy_path = folder(&e).join("form-final.md").to_string_lossy().into_owned();
    assert!(check_submission_files(&e, std::slice::from_ref(&copy_path)).is_ok());
    assert!(check_submission_files(&e, &["form-final.md".into()]).is_ok());
    assert!(check_submission_files(&e, &["form.md".into()]).is_err(), "the original is not the final copy");
    assert!(check_submission_files(&e, &[copy_path.clone(), copy_path.clone()]).is_err());
    assert!(check_submission_files(&e, &[]).is_err());
    assert!(check_submission_files(&e, &["../form-final.md".into()]).is_err());

    // The reviewed original must stay as scored.
    let form = folder(&e).join("form.md");
    fs::write(&form, "원본을 고쳤다\n").unwrap();
    assert!(!submit_approved(&e));
    assert!(reasons(&e, false).contains(&"original_artifact_changed".to_string()));
    fs::write(&form, "신청서 본문\n").unwrap();
    assert!(submit_approved(&e));
    // So does the copy.
    fs::write(folder(&e).join("form-final.md"), "바뀐 사본\n").unwrap();
    assert!(!submit_approved(&e));
}

#[test]
fn personalization_never_overwrites_an_original() {
    let mut e = eligible("r-overwrite");
    approve(&mut e, "제출해", "user", None).unwrap();
    let t = token(&e);
    // Writing the personal fields into the reviewed file itself breaks the approval first.
    fs::write(folder(&e).join("form.md"), "신청서 본문\n이름: 홍길동\n").unwrap();
    let raw = json!({"submission_ready": true, "needs_user_input": false, "blockers": [], "followup_required": false,
        "personalization_of": t, "artifacts": files(&e, &["form.md"])});
    assert!(personalize(&mut e, &raw).is_err());
    assert_eq!(readiness::latest(&e).unwrap()["seq"], 1);
}

#[test]
fn changing_followup_required_needs_a_new_request_and_approval() {
    let mut e = eligible("r-followup");
    let mut raw = request(&e, &["form.md"]);
    raw["followup_required"] = json!(true);
    readiness::record(&mut e, &raw).unwrap();
    approve(&mut e, "제출해, 본선 일정 있음", "user", None).unwrap();
    let t = token(&e);
    fs::write(folder(&e).join("form-final.md"), "신청서 본문\n서명\n").unwrap();
    let mut list = files(&e, &["form-final.md"]);
    list[0]["source"] = json!("form.md");
    let personal = |followup: bool| {
        json!({"submission_ready": true, "needs_user_input": false, "blockers": [], "followup_required": followup,
            "personalization_of": t, "artifacts": list})
    };
    // Filling personal fields cannot drop the follow-up flag that cleanup relies on.
    assert!(personalize(&mut e, &personal(false)).is_err());
    assert_eq!(readiness::latest(&e).unwrap()["followup_required"], true);
    let kept = personalize(&mut e, &personal(true)).unwrap();
    assert_eq!(kept["followup_required"], true);
    assert!(submit_approved(&e));

    // Changing it is an ordinary new request, which voids the approval until the user approves again.
    record(&mut e, &["form.md"]).unwrap();
    assert_eq!(readiness::latest(&e).unwrap()["followup_required"], false);
    assert!(!submit_approved(&e));
    approve(&mut e, "제출해, 후속 일정 없음", "user", None).unwrap();
    assert!(submit_approved(&e));
}
