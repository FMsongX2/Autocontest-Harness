//! Score loop: ContestJudge record validation, the transition score gate and the board column.

use chrono::NaiveDate;
use harness::board;
use harness::registry::{self, Entry, NewEntry, Registry, apply_gate, new_entry, score_gate, score_record, transition};
use serde_json::{Value, json};

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
        "criteria": [{"stage": "본선", "name": "참신성", "weight": 30}], "reasons": ["근거"]});
    apply_gate(&mut e, gate, "ContestGate").unwrap();
    transition(&mut e, "routed", "t", "").unwrap();
    transition(&mut e, "running", "t", "").unwrap();
    e
}

fn judge(total: f64, verdict: &str) -> Value {
    json!({"total": total, "verdict": verdict, "scores": [
        {"name": "참신성", "weight": 30, "score": 25, "reason": "근거"},
        {"name": "완성도", "weight": 70, "score": 63}
    ]})
}

fn add(e: &mut Entry, stage: &str, total: f64, verdict: &str) {
    e.scores.push(score_record(&judge(total, verdict), stage, "/c/.harness/judge/x.json", "").unwrap());
}

#[test]
fn score_record_keeps_the_contract_fields() {
    let r = score_record(&judge(88.0, "revise"), "plan", "/c/.harness/judge/plan-1.json", "후보 A").unwrap();
    assert_eq!(r["stage"], "plan");
    assert_eq!((r["total"].as_f64(), r["target"].as_f64()), (Some(88.0), Some(90.0)));
    assert_eq!(r["verdict"], "revise");
    assert_eq!(r["criteria"][0], json!({"name": "참신성", "weight": 30.0, "score": 25.0}));
    assert_eq!((r["note"].as_str(), r["file"].as_str()), (Some("후보 A"), Some("/c/.harness/judge/plan-1.json")));
    assert!(r["at"].as_str().unwrap().ends_with("+09:00"));
    let mut with_target = judge(80.0, "ready");
    with_target["target"] = json!(80);
    assert_eq!(score_record(&with_target, "final", "f", "").unwrap()["target"].as_f64(), Some(80.0));
}

#[test]
fn score_record_rejects_bad_judge_output() {
    let bad = |patch: Value| {
        let mut j = judge(88.0, "ready");
        for (k, v) in patch.as_object().unwrap() {
            j[k] = v.clone();
        }
        score_record(&j, "draft", "f", "").is_err()
    };
    assert!(bad(json!({"total": 101})));
    assert!(bad(json!({"total": "88"})));
    assert!(bad(json!({"target": -1})));
    assert!(bad(json!({"verdict": "pass"})));
    assert!(bad(json!({"scores": []})));
    assert!(bad(json!({"scores": [{"name": "", "weight": 10, "score": 5}]})));
    assert!(bad(json!({"scores": [{"name": "a", "weight": 10, "score": 11}]})));
    assert!(bad(json!({"scores": [{"name": "a", "score": 5}]})));
    assert!(bad(json!({"stage": "final"})));
    assert!(score_record(&json!([1]), "draft", "f", "").is_err());
    assert!(score_record(&judge(88.0, "ready"), "review", "f", "").is_err());
    assert!(!bad(json!({"stage": "draft"})));
}

#[test]
fn awaiting_approval_needs_plan_and_a_passing_final_from_level_2() {
    let mut e = entry("x-1", 2);
    let refused = |e: &Entry| score_gate(e, "awaiting_approval", None).unwrap_err().to_string();
    assert!(refused(&e).contains("plan 점수 없음"));
    assert!(refused(&e).contains("final 점수 없음"));
    add(&mut e, "plan", 85.0, "revise");
    add(&mut e, "final", 92.0, "ready");
    assert_eq!(score_gate(&e, "awaiting_approval", None).unwrap(), None);
    add(&mut e, "final", 88.0, "ready");
    assert!(refused(&e).contains("목표 미달"), "{}", refused(&e));
    add(&mut e, "final", 95.0, "revise");
    assert!(refused(&e).contains("판정 revise"));
    let note = score_gate(&e, "awaiting_approval", Some("마감 2시간 전")).unwrap().unwrap();
    assert!(note.contains("마감 2시간 전") && note.contains("95/90"), "{note}");
    assert!(score_gate(&e, "awaiting_approval", Some("  ")).is_err());
    add(&mut e, "draft", 99.0, "ready");
    assert!(score_gate(&e, "awaiting_approval", None).is_err(), "draft does not replace final");

    let mut only_final = entry("x-2", 3);
    add(&mut only_final, "final", 95.0, "ready");
    assert!(refused(&only_final).contains("plan 점수 없음"));
    // Other moves are not gated.
    assert_eq!(score_gate(&only_final, "skipped", None).unwrap(), None);
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
    add(&mut e, "plan", 70.0, "revise");
    assert_eq!(score_gate(&e, "plan_review", None).unwrap(), None);
    let level_4 = entry("x-5", 4);
    assert_eq!(score_gate(&level_4, "plan_review", None).unwrap(), None);
}

#[test]
fn board_shows_the_latest_score_for_active_rows() {
    let mut reg = Registry::default();
    let mut scored = entry("x-6", 2);
    add(&mut scored, "plan", 70.0, "revise");
    add(&mut scored, "draft", 88.5, "revise");
    let plain = entry("x-7", 3);
    let mut done = entry("x-8", 2);
    add(&mut done, "final", 93.0, "ready");
    transition(&mut done, "awaiting_approval", "t", "").unwrap();
    transition(&mut done, "submitted", "t", "").unwrap();
    for e in [scored, plain, done] {
        reg.contests.insert(e.id.clone(), e);
    }
    let text = board::render_on(&reg, NaiveDate::from_ymd_opt(2026, 10, 15).unwrap());
    assert!(text.contains("| idea / L2 · 88.5/90 |"), "{text}");
    assert!(text.contains("| idea / L3 | - | `x-7` |"), "{text}");
    assert!(text.contains("| idea / L2 | - | `x-8` |"), "{text}");
    assert_eq!(registry::points(90.0), "90");
}
