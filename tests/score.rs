//! Score loop: ContestJudge record validation, gate criteria, revision-bound finals with reviewed
//! file hashes, the transition score gate and the board column.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::NaiveDate;
use harness::registry::{
    self, Entry, NewEntry, Registry, apply_gate, new_entry, record_score, score_gate, score_record, transition,
};
use harness::{board, readiness};
use serde_json::{Value, json};

fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "harness-score-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// `RUBRIC.md` with the contest basis documented (section "## 0. 채점 근거").
const RUBRIC: &str = "# 점수표\n\n## 0. 채점 근거\n\n| 항목 | 내용 | 공고 원문 위치 |\n|---|---|---|\n\
| 주최 | 예시 기관 | 공고 1쪽 |\n| 주관 | 예시 재단 | 공고 1쪽 |\n| 개최 목적 | 지역 문제 해결 아이디어 발굴 | 공고 1쪽 1항 |\n\
| 심사기준 | 참신성 30, 완성도 70 | 공고 3쪽 5항 |\n\n## 1. 기준별 풀이\n";

/// A running entry of `level` with a folder holding `out.md` and a documented `RUBRIC.md`.
fn entry(id: &str, level: u8) -> Entry {
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
    let gate = json!({"decision": "pass", "type": "idea", "level": level,
        "criteria": [{"stage": "본선", "name": "참신성", "weight": 30}, {"stage": "본선", "name": "완성도", "weight": 70}],
        "reasons": ["근거"]});
    apply_gate(&mut e, gate, "ContestGate").unwrap();
    let dir = temp_dir(id);
    fs::write(dir.join("out.md"), format!("{id} 결과물\n")).unwrap();
    fs::write(dir.join("RUBRIC.md"), RUBRIC).unwrap();
    e.folder = Some(dir.to_string_lossy().into_owned());
    transition(&mut e, "routed", "t", "").unwrap();
    transition(&mut e, "running", "t", "").unwrap();
    e
}

fn folder(e: &Entry) -> PathBuf {
    PathBuf::from(e.folder.as_deref().unwrap())
}

fn artifacts(e: &Entry) -> Value {
    let sha = readiness::sha256_file(&folder(e).join("out.md")).unwrap();
    json!([{"path": "out.md", "sha256": sha}])
}

/// Judge output whose rows add up to `a + b` out of 100.
fn judge(a: f64, b: f64, verdict: &str) -> Value {
    json!({"total": a + b, "verdict": verdict, "scores": [
        {"name": "참신성", "round": "본선", "weight": 30, "score": a, "rationale": "근거"},
        {"name": "완성도", "weight": 70, "score": b}
    ]})
}

/// Contest basis and a fresh Judge run, as the caller attaches them to a plan score.
fn plan_provenance(j: &mut Value, run_id: &str) {
    j["basis"] = json!({"organizer": "주최 기관 / 주관 기관", "purpose": "공고의 개최 목적", "criteria_source": "공고 심사기준"});
    j["evaluator"] = json!({"id": "ContestJudge", "model": "stub/judge", "run_id": run_id});
}

fn add(e: &mut Entry, stage: &str, a: f64, b: f64, verdict: &str) {
    let mut j = judge(a, b, verdict);
    if stage == "final" {
        j["artifacts"] = artifacts(e);
    }
    if stage == "plan" {
        plan_provenance(&mut j, &format!("run-{}", e.scores.len()));
    }
    record_score(e, &j, stage, "/c/.harness/judge/x.json", "").unwrap();
}

#[test]
fn score_record_keeps_the_contract_fields() {
    let r = score_record(&judge(25.0, 63.0, "revise"), "plan", "/c/.harness/judge/plan-1.json", "후보 A").unwrap();
    assert_eq!(r["stage"], "plan");
    assert_eq!((r["total"].as_f64(), r["target"].as_f64()), (Some(88.0), Some(90.0)));
    assert_eq!(r["verdict"], "revise");
    assert_eq!(r["criteria"][0], json!({"name": "참신성", "round": "본선", "weight": 30.0, "score": 25.0}));
    assert_eq!(r["criteria"][1], json!({"name": "완성도", "weight": 70.0, "score": 63.0}));
    assert_eq!((r["note"].as_str(), r["file"].as_str()), (Some("후보 A"), Some("/c/.harness/judge/plan-1.json")));
    assert!(r["at"].as_str().unwrap().ends_with("+09:00"));
    let mut with_target = judge(27.0, 63.0, "ready");
    with_target["target"] = json!(80);
    assert_eq!(score_record(&with_target, "final", "f", "").unwrap()["target"].as_f64(), Some(80.0));
    // Weights that do not add up to 100 are normalised; rounding within half a point passes.
    let scaled = json!({"total": 67, "verdict": "revise", "scores": [
        {"name": "a", "weight": 10, "score": 7}, {"name": "b", "weight": 20, "score": 13.1}]});
    assert!(score_record(&scaled, "draft", "f", "").is_ok());
}

#[test]
fn score_record_rejects_bad_judge_output() {
    let bad = |patch: Value| {
        let mut j = judge(25.0, 63.0, "ready");
        for (k, v) in patch.as_object().unwrap() {
            j[k] = v.clone();
        }
        score_record(&j, "draft", "f", "").is_err()
    };
    assert!(bad(json!({"total": 101})));
    assert!(bad(json!({"total": "88"})));
    assert!(bad(json!({"total": 95})), "total is not the criterion sum");
    assert!(bad(json!({"target": -1})));
    assert!(bad(json!({"verdict": "pass"})));
    assert!(bad(json!({"scores": []})));
    assert!(bad(json!({"scores": [{"name": "", "weight": 10, "score": 5}]})));
    assert!(bad(json!({"scores": [{"name": "a", "weight": 10, "score": 11}]})));
    assert!(bad(json!({"scores": [{"name": "a", "score": 5}]})));
    assert!(bad(json!({"total": 0, "scores": [{"name": "a", "weight": 0, "score": 0}]})), "weights are positive");
    assert!(bad(
        json!({"scores": [{"name": "a", "weight": 50, "score": 44}, {"name": "a", "weight": 50, "score": 44}]})
    ));
    assert!(bad(json!({"scores": [
        {"name": "a", "round": "서류", "weight": 50, "score": 44}, {"name": "a", "weight": 50, "score": 44}]})));
    assert!(bad(json!({"scores": [{"name": "a", "round": 1, "weight": 100, "score": 88}]})));
    // Each value is finite, but their sums overflow to infinity (NaN after normalising).
    assert!(bad(json!({"total": 50, "scores": [
        {"name": "a", "weight": 1.7e308, "score": 1.7e308}, {"name": "b", "weight": 1.7e308, "score": 1.7e308}]})));
    assert!(bad(json!({"stage": "final"})));
    assert!(score_record(&json!([1]), "draft", "f", "").is_err());
    assert!(score_record(&judge(25.0, 63.0, "ready"), "review", "f", "").is_err());
    assert!(!bad(json!({"stage": "draft"})));
    // The same name in two rounds is two criteria.
    assert!(!bad(json!({"scores": [
        {"name": "a", "round": "서류", "weight": 50, "score": 44}, {"name": "a", "round": "본선", "weight": 50, "score": 44}]})));
}

#[test]
fn record_score_checks_gate_criteria_and_reviewed_files() {
    let mut e = entry("x-gate", 2);
    let refused = |e: &mut Entry, j: &Value, stage: &str| record_score(e, j, stage, "f", "").unwrap_err().to_string();
    // Gate criteria: every row names one, every one is scored, published weights agree.
    let extra = json!({"total": 80, "verdict": "ready", "scores": [
        {"name": "참신성", "weight": 30, "score": 25}, {"name": "완성도", "weight": 70, "score": 55},
        {"name": "발표", "weight": 10, "score": 8}]});
    assert!(refused(&mut e, &extra, "draft").contains("발표"));
    let missing = json!({"total": 90, "verdict": "ready", "scores": [{"name": "참신성", "weight": 30, "score": 27}]});
    assert!(refused(&mut e, &missing, "draft").contains("완성도"));
    let weight = json!({"total": 88, "verdict": "ready", "scores": [
        {"name": "참신성", "weight": 40, "score": 35}, {"name": "완성도", "weight": 60, "score": 53}]});
    assert!(refused(&mut e, &weight, "draft").contains("배점"));
    let round = json!({"total": 88, "verdict": "ready", "scores": [
        {"name": "참신성", "round": "예선", "weight": 30, "score": 25}, {"name": "완성도", "weight": 70, "score": 63}]});
    assert!(refused(&mut e, &round, "draft").contains("참신성"));
    assert!(e.scores.is_empty(), "nothing is stored on a refusal");

    // final needs the reviewed files, matching the folder now.
    assert!(refused(&mut e, &judge(27.0, 65.0, "ready"), "final").contains("artifacts"));
    let mut j = judge(27.0, 65.0, "ready");
    j["artifacts"] = json!([{"path": "out.md", "sha256": "0".repeat(64)}]);
    assert!(refused(&mut e, &j, "final").contains("SHA-256"));
    j["artifacts"] = json!([{"path": "../out.md", "sha256": "0".repeat(64)}]);
    assert!(record_score(&mut e, &j, "final", "f", "").is_err(), "outside the folder");
    j["artifacts"] = json!([{"path": "missing.md", "sha256": "0".repeat(64)}]);
    assert!(record_score(&mut e, &j, "final", "f", "").is_err());
    let outside = temp_dir("outside");
    fs::write(outside.join("secret.md"), "x").unwrap();
    std::os::unix::fs::symlink(outside.join("secret.md"), folder(&e).join("link.md")).unwrap();
    let sha = readiness::sha256_file(&outside.join("secret.md")).unwrap();
    j["artifacts"] = json!([{"path": "link.md", "sha256": sha}]);
    assert!(record_score(&mut e, &j, "final", "f", "").is_err(), "a symlink out of the folder");
    j["artifacts"] = json!([{"path": outside.join("secret.md").to_string_lossy(), "sha256": sha}]);
    assert!(record_score(&mut e, &j, "final", "f", "").is_err(), "an absolute path outside");
    assert!(e.scores.is_empty());

    // Absolute paths inside the folder are stored relative, with the work revision.
    let mut ok = judge(27.0, 65.0, "ready");
    let sha = readiness::sha256_file(&folder(&e).join("out.md")).unwrap();
    ok["artifacts"] = json!([{"path": folder(&e).join("out.md").to_string_lossy(), "sha256": sha.to_uppercase()}]);
    let r = record_score(&mut e, &ok, "final", "f", "").unwrap();
    assert_eq!(r["artifacts"], json!([{"path": "out.md", "sha256": sha}]));
    assert_eq!(r["revision"].as_u64(), Some(registry::revision(&e) as u64));
    assert_eq!(e.scores.len(), 1);
    // plan and draft need no files.
    add(&mut e, "plan", 25.0, 60.0, "revise");
    assert_eq!(e.scores.len(), 2);
}

#[test]
fn plan_scores_need_the_contest_basis_and_a_fresh_judge_run() {
    let mut e = entry("x-1", 2);
    let refused = |e: &mut Entry, j: &Value| record_score(e, j, "plan", "f", "").unwrap_err().to_string();
    let bare = judge(25.0, 60.0, "revise");
    assert!(refused(&mut e, &bare).contains("basis"), "a plan score without the contest basis");

    let mut j = bare.clone();
    plan_provenance(&mut j, "run-a");
    j["basis"]["purpose"] = json!("  ");
    assert!(refused(&mut e, &j).contains("basis.purpose"), "a blank purpose");
    plan_provenance(&mut j, "run-a");
    j.as_object_mut().unwrap().remove("evaluator");
    assert!(refused(&mut e, &j).contains("evaluator"), "no Judge run named");
    assert!(e.scores.is_empty());

    plan_provenance(&mut j, "run-a");
    let r = record_score(&mut e, &j, "plan", "f", "").unwrap();
    assert_eq!(r["basis"]["organizer"], "주최 기관 / 주관 기관");
    assert_eq!(r["evaluator"]["run_id"], "run-a");

    // Re-scoring a revised plan with the same Judge run is refused; a new run is recorded.
    let mut again = judge(27.0, 63.0, "revise");
    plan_provenance(&mut again, "run-a");
    assert!(refused(&mut e, &again).contains("새 Judge 실행"));
    plan_provenance(&mut again, "run-b");
    record_score(&mut e, &again, "plan", "f", "").unwrap();
    assert_eq!(e.scores.len(), 2);
    // draft and final keep their own rules.
    record_score(&mut e, &judge(26.0, 61.0, "revise"), "draft", "f", "").unwrap();
}

#[test]
fn plan_scores_need_the_basis_documented_in_the_rubric_first() {
    let mut e = entry("x-2", 2);
    let rubric = folder(&e).join("RUBRIC.md");
    let mut j = judge(25.0, 60.0, "revise");
    plan_provenance(&mut j, "run-a");
    let refused = |e: &mut Entry| record_score(e, &j, "plan", "f", "").unwrap_err().to_string();

    fs::remove_file(&rubric).unwrap();
    assert!(refused(&mut e).contains("RUBRIC.md"), "no rubric yet");
    fs::write(&rubric, "# 점수표\n\n## 1. 기준별 풀이\n").unwrap();
    assert!(refused(&mut e).contains("## 0. 채점 근거"), "rubric without the basis section");
    // The template placeholders and a row without its notice location do not count.
    fs::write(&rubric, RUBRIC.replace("| 주관 | 예시 재단 | 공고 1쪽 |", "| 주관 | [ ] | [ ] |")).unwrap();
    assert!(refused(&mut e).contains("주관 행"), "{}", refused(&mut e));
    fs::write(&rubric, RUBRIC.replace("| 공고 1쪽 1항 |", "| |")).unwrap();
    assert!(refused(&mut e).contains("개최 목적 행"), "{}", refused(&mut e));
    // A row under another heading does not fill the section.
    let moved = RUBRIC.replace("## 0. 채점 근거\n", "## 0. 채점 근거\n\n## 9. 기타\n");
    fs::write(&rubric, moved).unwrap();
    assert!(refused(&mut e).contains("주최, 주관, 개최 목적, 심사기준"));
    assert!(e.scores.is_empty());

    fs::write(&rubric, RUBRIC).unwrap();
    let r = record_score(&mut e, &j, "plan", "f", "").unwrap();
    assert_eq!(r["basis"]["document"]["path"], "RUBRIC.md");
    assert_eq!(r["basis"]["document"]["sha256"].as_str().map(str::len), Some(64));
}

#[test]
fn awaiting_approval_needs_plan_and_a_passing_current_final_from_level_2() {
    let mut e = entry("x-1", 2);
    let refused = |e: &Entry| score_gate(e, "awaiting_approval", None).unwrap_err().to_string();
    assert!(refused(&e).contains("plan 점수 없음"));
    assert!(refused(&e).contains("final 점수 없음"));
    add(&mut e, "plan", 25.0, 60.0, "revise");
    add(&mut e, "final", 27.0, 65.0, "ready");
    assert_eq!(score_gate(&e, "awaiting_approval", None).unwrap(), None);
    add(&mut e, "final", 25.0, 63.0, "ready");
    assert!(refused(&e).contains("목표 미달"), "{}", refused(&e));
    add(&mut e, "final", 28.0, 67.0, "revise");
    assert!(refused(&e).contains("판정 revise"));
    let note = score_gate(&e, "awaiting_approval", Some("마감 2시간 전")).unwrap().unwrap();
    assert!(note.contains("마감 2시간 전") && note.contains("95/90"), "{note}");
    assert!(score_gate(&e, "awaiting_approval", Some("  ")).is_err());
    add(&mut e, "draft", 30.0, 69.0, "ready");
    assert!(score_gate(&e, "awaiting_approval", None).is_err(), "draft does not replace final");

    let mut only_final = entry("x-2", 3);
    add(&mut only_final, "final", 28.0, 67.0, "ready");
    assert!(refused(&only_final).contains("plan 점수 없음"));
    // Other moves are not gated.
    assert_eq!(score_gate(&only_final, "skipped", None).unwrap(), None);
}

#[test]
fn a_final_counts_only_for_the_current_revision_and_unchanged_files() {
    let mut e = entry("x-rev", 2);
    add(&mut e, "plan", 25.0, 60.0, "revise");
    add(&mut e, "final", 27.0, 65.0, "ready");
    assert_eq!(score_gate(&e, "awaiting_approval", None).unwrap(), None);

    // A changed reviewed file voids the final; restoring it brings it back.
    let out = folder(&e).join("out.md");
    let original = fs::read(&out).unwrap();
    fs::write(&out, "몰래 고친 결과물\n").unwrap();
    let err = score_gate(&e, "awaiting_approval", None).unwrap_err().to_string();
    assert!(err.contains("검토 파일"), "{err}");
    fs::write(&out, &original).unwrap();
    assert_eq!(score_gate(&e, "awaiting_approval", None).unwrap(), None);

    // A revise cycle (back to running) makes the final stale; the plan score stays valid.
    transition(&mut e, "awaiting_approval", "t", "").unwrap();
    let before = registry::revision(&e);
    transition(&mut e, "running", "t", "고친다").unwrap();
    assert!(registry::revision(&e) > before);
    let err = score_gate(&e, "awaiting_approval", None).unwrap_err().to_string();
    assert!(err.contains("이전 작업 판") && !err.contains("plan 점수 없음"), "{err}");
    add(&mut e, "final", 27.0, 65.0, "ready");
    assert_eq!(score_gate(&e, "awaiting_approval", None).unwrap(), None);

    // Same-state markers never change the revision.
    let r = registry::revision(&e);
    e.history.push(registry::HistoryItem::new("t".into(), Some("running".into()), "running", "restart", ""));
    assert_eq!(registry::revision(&e), r);
}

#[test]
fn historical_unstamped_finals_stay_readable_but_never_pass() {
    let mut e = entry("x-old", 2);
    add(&mut e, "plan", 25.0, 60.0, "revise");
    // Recorded by an older version: no revision, no artifacts.
    e.scores.push(json!({"at": "2026-10-01T10:00+09:00", "stage": "final", "total": 95, "target": 90,
        "verdict": "ready", "criteria": [], "note": "", "file": "f"}));
    assert_eq!(registry::score_label(registry::latest_score(&e, Some("final")).unwrap()), "95/90");
    let err = score_gate(&e, "awaiting_approval", None).unwrap_err().to_string();
    assert!(err.contains("작업 판 표시가 없다") && err.contains("artifacts"), "{err}");
    let codes: Vec<&str> = registry::score_problems(&e).iter().map(|p| p.code).collect();
    assert_eq!(codes, ["final_unstamped", "final_artifacts_missing"]);
    // A human may still pass the gate with a reason.
    assert!(score_gate(&e, "awaiting_approval", Some("사용자가 이전 채점을 확인함")).unwrap().is_some());
}

#[test]
fn level_1_has_no_score_gate() {
    let e = entry("x-3", 1);
    assert_eq!(score_gate(&e, "awaiting_approval", None).unwrap(), None);
}

#[test]
fn level_5_plan_review_needs_a_plan_score() {
    let mut e = entry("x-4", 5);
    assert!(score_gate(&e, "plan_review", None).is_err());
    assert!(score_gate(&e, "plan_review", Some("사유")).is_err());
    add(&mut e, "plan", 20.0, 50.0, "revise");
    assert_eq!(score_gate(&e, "plan_review", None).unwrap(), None);
    let level_4 = entry("x-5", 4);
    assert_eq!(score_gate(&level_4, "plan_review", None).unwrap(), None);
}

#[test]
fn board_shows_the_latest_score_for_active_rows() {
    let mut reg = Registry::default();
    let mut scored = entry("x-6", 2);
    add(&mut scored, "plan", 20.0, 50.0, "revise");
    add(&mut scored, "draft", 25.5, 63.0, "revise");
    let plain = entry("x-7", 3);
    let mut done = entry("x-8", 2);
    add(&mut done, "final", 28.0, 65.0, "ready");
    transition(&mut done, "awaiting_approval", "t", "").unwrap();
    transition(&mut done, "submitted", "t", "").unwrap();
    for e in [scored, plain, done] {
        reg.contests.insert(e.id.clone(), e);
    }
    let text = board::render_on(&reg, NaiveDate::from_ymd_opt(2026, 10, 15).unwrap());
    assert!(text.contains("| idea / L2 · 88.5/90 |"), "{text}");
    for id in ["x-7", "x-8"] {
        let row = text.lines().find(|row| row.contains(&format!("`{id}`"))).unwrap();
        assert!(!row.contains('·'), "unscored and completed entries must not show an active score: {row}");
    }
}
