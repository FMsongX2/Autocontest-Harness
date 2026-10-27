//! Independent assessment packets: metadata isolation, path safety, binding to the current work,
//! exact final binding, measured dispersion, comparisons, outcomes and descriptive calibration.
//! Every score here is a test fixture, not real evaluation data.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use harness::evaluation;
use harness::readiness;
use harness::registry::{Entry, NewEntry, Registry, apply_gate, new_entry, transition};
use serde_json::{Value, json};

fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "harness-eval-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A running level 3 entry whose folder holds an original and a sanitized copy.
fn entry(id: &str) -> Entry {
    let mut e = new_entry(NewEntry {
        id,
        title: "비밀 공모전 제목",
        url: "https://e.com",
        source: "manual",
        organizer: "",
        deadline: None,
        categories: vec![],
        note: None,
    });
    let gate = json!({"decision": "pass", "type": "idea", "level": 3,
        "criteria": [{"stage": "본선", "name": "참신성", "weight": 30}, {"stage": "본선", "name": "완성도", "weight": 70}],
        "reasons": ["근거"]});
    apply_gate(&mut e, gate, "ContestGate").unwrap();
    let dir = temp_dir(id);
    fs::write(dir.join("홍길동_제안서.md"), "작성자 홍길동 결과물\n").unwrap();
    fs::create_dir_all(dir.join("eval")).unwrap();
    fs::write(dir.join("eval/clean.md"), "작성자 [익명] 결과물\n").unwrap();
    fs::write(dir.join("alt.md"), "다른 후보\n").unwrap();
    fs::write(dir.join("eval/alt-clean.md"), "다른 후보(익명)\n").unwrap();
    fs::write(dir.join("RUBRIC.md"), "기준 풀이\n").unwrap();
    fs::write(dir.join("JUDGE.md"), "이전 판정 total 88\n").unwrap();
    fs::create_dir_all(dir.join(".private")).unwrap();
    fs::write(dir.join(".private/profile.md"), "개인정보\n").unwrap();
    e.folder = Some(dir.to_string_lossy().into_owned());
    e.scores.push(json!({"stage": "draft", "total": 77.0, "verdict": "revise", "note": "이전 점수 77"}));
    transition(&mut e, "routed", "t", "").unwrap();
    transition(&mut e, "running", "t", "").unwrap();
    e
}

fn folder(e: &Entry) -> PathBuf {
    PathBuf::from(e.folder.as_deref().unwrap())
}

fn file(e: &Entry, rel: &str) -> Value {
    json!({"path": rel, "sha256": readiness::sha256_file(&folder(e).join(rel)).unwrap()})
}

fn attestation() -> Value {
    json!({"by": "session", "identity_removed": true, "statement": "본문 속 이름·학교를 지운 사본을 직접 확인했다"})
}

fn final_request(e: &Entry) -> Value {
    json!({
        "purpose": "independent_final",
        "candidates": [{"artifacts": [{"source": file(e, "홍길동_제안서.md"), "sanitized": file(e, "eval/clean.md")}]}],
        "rubric": file(e, "RUBRIC.md"),
        "attestation": attestation(),
    })
}

fn comparison_request(e: &Entry) -> Value {
    json!({
        "purpose": "comparison",
        "stage": "draft",
        "candidates": [
            {"name": "현재안", "artifacts": [{"source": file(e, "홍길동_제안서.md"), "sanitized": file(e, "eval/clean.md")}]},
            {"name": "대안", "artifacts": [{"source": file(e, "alt.md"), "sanitized": file(e, "eval/alt-clean.md")}]}
        ],
        "attestation": attestation(),
    })
}

fn manifest(e: &Entry, prepared: &Value) -> Value {
    let dir = folder(e).join(prepared["packet_dir"].as_str().unwrap());
    serde_json::from_slice(&fs::read(dir.join(evaluation::MANIFEST)).unwrap()).unwrap()
}

/// The evaluator's view: one candidate's files as listed by the packet manifest.
fn packet_files(e: &Entry, prepared: &Value, label: &str) -> Value {
    let m = manifest(e, prepared);
    let c = m["candidates"].as_array().unwrap().iter().find(|c| c["label"] == label).unwrap();
    c["files"].clone()
}

fn scores(a: f64, b: f64) -> Value {
    json!([
        {"name": "참신성", "round": "본선", "weight": 30, "score": a, "rationale": "근거"},
        {"name": "완성도", "round": "본선", "weight": 70, "score": b}
    ])
}

fn candidate(e: &Entry, prepared: &Value, label: &str, a: f64, b: f64, verdict: &str) -> Value {
    json!({"label": label, "total": a + b, "verdict": verdict, "scores": scores(a, b), "artifacts": packet_files(e, prepared, label)})
}

fn assess(e: &mut Entry, prepared: &Value, run: &str, cands: Vec<Value>) -> harness::error::Result<Value> {
    let raw = json!({
        "packet_id": prepared["packet_id"],
        "evaluator": {"id": "ContestJudge", "model": "fixture-model", "run_id": run},
        "mode": "independent",
        "candidates": cands,
    });
    evaluation::record(e, &raw)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for item in fs::read_dir(dir).unwrap() {
        let path = item.unwrap().path();
        if path.is_dir() { walk(&path, out) } else { out.push(path) }
    }
}

#[test]
fn packet_holds_only_declared_sanitized_files_without_metadata() {
    let mut e = entry("eval-leak");
    let request = final_request(&e);
    let prepared = evaluation::prepare(&mut e, &request).unwrap();
    assert_eq!(prepared["anonymization"], "caller_attested");
    assert_eq!(prepared["labels"], json!(["A"]));
    let rel = prepared["packet_dir"].as_str().unwrap();
    assert!(rel.starts_with(".harness/evaluation/") && rel.ends_with("/packet"));
    let dir = folder(&e).join(rel);
    let mut files = Vec::new();
    walk(&dir, &mut files);
    let mut names: Vec<String> =
        files.iter().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["A/file-1.md", "PACKET.json", "rubric/rubric.md"]);
    assert_eq!(fs::read_to_string(dir.join("A/file-1.md")).unwrap(), "작성자 [익명] 결과물\n");
    let text = fs::read_to_string(dir.join("PACKET.json")).unwrap();
    for leak in [
        "eval-leak",
        "비밀 공모전 제목",
        "홍길동",
        "clean.md",
        "이전 점수",
        "history",
        "JUDGE",
        "session",
        "개인정보",
        "현재안",
    ] {
        assert!(!text.contains(leak), "packet manifest leaks {leak}");
    }
    let m = manifest(&e, &prepared);
    assert!(m.get("scores").is_none() && m.get("history").is_none() && m.get("author").is_none());
    assert_eq!(m["criteria"].as_array().unwrap().len(), 2);
    // The private binding stays in the registry entry, never in the packet.
    let stored = &e.extra[evaluation::KEY]["packets"][0];
    assert_eq!(stored["candidates"][0]["sources"][0]["path"], "홍길동_제안서.md");
    assert_eq!(stored["attestation"]["identity_removed"], true);
    // The original is untouched.
    assert_eq!(fs::read_to_string(folder(&e).join("홍길동_제안서.md")).unwrap(), "작성자 홍길동 결과물\n");
}

#[test]
fn prepare_refuses_undeclared_metadata_and_missing_attestation() {
    let mut e = entry("eval-strict");
    let mut raw = final_request(&e);
    raw["previous_scores"] = json!([88]);
    assert!(evaluation::prepare(&mut e, &raw).unwrap_err().0.contains("previous_scores"));
    let mut raw = final_request(&e);
    raw["candidates"][0]["author"] = json!("홍길동");
    assert!(evaluation::prepare(&mut e, &raw).is_err());
    let mut raw = final_request(&e);
    raw.as_object_mut().unwrap().remove("attestation");
    assert!(evaluation::prepare(&mut e, &raw).is_err());
    let mut raw = final_request(&e);
    raw["attestation"]["identity_removed"] = json!(false);
    assert!(evaluation::prepare(&mut e, &raw).unwrap_err().0.contains("identity_removed"));
    let mut raw = final_request(&e);
    raw["attestation"]["statement"] = json!("  ");
    assert!(evaluation::prepare(&mut e, &raw).is_err());
    assert!(e.extra.get(evaluation::KEY).is_none(), "a refused request stores nothing");
    assert!(
        !folder(&e).join(".harness/evaluation").exists()
            || fs::read_dir(folder(&e).join(".harness/evaluation")).unwrap().next().is_none()
    );
}

#[test]
fn prepare_refuses_traversal_symlinks_private_and_judge_files() {
    let mut e = entry("eval-paths");
    let outside = temp_dir("eval-outside");
    fs::write(outside.join("x.md"), "밖\n").unwrap();
    std::os::unix::fs::symlink(outside.join("x.md"), folder(&e).join("link.md")).unwrap();
    let sha = readiness::sha256_file(&outside.join("x.md")).unwrap();
    let outside_abs = outside.join("x.md").to_string_lossy().into_owned();
    fs::create_dir_all(folder(&e).join(".harness/judge")).unwrap();
    fs::write(folder(&e).join(".harness/judge/final-1.json"), "{}\n").unwrap();
    let judge_json = file(&e, ".harness/judge/final-1.json");
    let bad_sanitized = [
        json!({"path": "../x.md", "sha256": sha}),
        json!({"path": outside_abs, "sha256": sha}),
        json!({"path": "link.md", "sha256": sha}),
        file(&e, ".private/profile.md"),
        file(&e, "JUDGE.md"),
        judge_json,
        json!({"path": "eval/clean.md", "sha256": "0".repeat(64)}),
    ];
    for bad in bad_sanitized {
        let mut raw = final_request(&e);
        raw["candidates"][0]["artifacts"][0]["sanitized"] = bad.clone();
        assert!(evaluation::prepare(&mut e, &raw).is_err(), "accepted {bad}");
    }
    let mut raw = final_request(&e);
    raw["rubric"] = file(&e, "JUDGE.md");
    assert!(evaluation::prepare(&mut e, &raw).is_err());
    let mut raw = final_request(&e);
    raw["references"] = json!([{"path": "RUBRIC.md", "sha256": file(&e, "RUBRIC.md")["sha256"], "provenance": {"source_url": "https://user:pw@e.com/a", "license": "CC-BY"}}]);
    assert!(evaluation::prepare(&mut e, &raw).is_err(), "credentials in a reference URL");
    assert!(e.extra.get(evaluation::KEY).is_none());
}

#[test]
fn record_refuses_stale_revision_rules_sources_and_tampered_packets() {
    // Revision.
    let mut e = entry("eval-stale-rev");
    let request = final_request(&e);
    let p = evaluation::prepare(&mut e, &request).unwrap();
    transition(&mut e, "awaiting_approval", "t", "").unwrap();
    transition(&mut e, "running", "t", "재작업").unwrap();
    let candidates = vec![candidate(&e, &p, "A", 25.0, 65.0, "ready")];
    let err = assess(&mut e, &p, "r1", candidates).unwrap_err();
    assert!(err.0.contains("revision_changed"), "{}", err.0);

    // Rule epoch binding.
    let mut e = entry("eval-stale-rules");
    let request = final_request(&e);
    let p = evaluation::prepare(&mut e, &request).unwrap();
    e.extra.get_mut(evaluation::KEY).unwrap()["packets"][0]["rules_binding"]["final"] = json!(9999);
    let candidates = vec![candidate(&e, &p, "A", 25.0, 65.0, "ready")];
    let err = assess(&mut e, &p, "r1", candidates).unwrap_err();
    assert!(err.0.contains("rules_changed"), "{}", err.0);

    // Original source changed after the packet was made.
    let mut e = entry("eval-stale-src");
    let request = final_request(&e);
    let p = evaluation::prepare(&mut e, &request).unwrap();
    let c = candidate(&e, &p, "A", 25.0, 65.0, "ready");
    fs::write(folder(&e).join("홍길동_제안서.md"), "고친 결과물\n").unwrap();
    assert!(assess(&mut e, &p, "r1", vec![c]).unwrap_err().0.contains("source_changed"));

    // Packet file edited.
    let mut e = entry("eval-tamper");
    let request = final_request(&e);
    let p = evaluation::prepare(&mut e, &request).unwrap();
    let c = candidate(&e, &p, "A", 25.0, 65.0, "ready");
    fs::write(folder(&e).join(p["packet_dir"].as_str().unwrap()).join("A/file-1.md"), "바뀜\n").unwrap();
    assert!(assess(&mut e, &p, "r1", vec![c]).unwrap_err().0.contains("packet_modified"));
}

#[test]
fn record_validates_scores_artifacts_and_run_identity() {
    let mut e = entry("eval-record");
    let request = final_request(&e);
    let p = evaluation::prepare(&mut e, &request).unwrap();
    // Wrong total.
    let mut c = candidate(&e, &p, "A", 25.0, 65.0, "ready");
    c["total"] = json!(99.0);
    assert!(assess(&mut e, &p, "r1", vec![c]).is_err());
    // Missing criterion.
    let mut c = candidate(&e, &p, "A", 25.0, 65.0, "ready");
    c["scores"] = json!([{"name": "참신성", "weight": 100, "score": 90}]);
    c["total"] = json!(90.0);
    assert!(assess(&mut e, &p, "r1", vec![c]).is_err());
    // Artifacts not computed from the packet.
    let mut c = candidate(&e, &p, "A", 25.0, 65.0, "ready");
    c["artifacts"][0]["sha256"] = json!("f".repeat(64));
    assert!(assess(&mut e, &p, "r1", vec![c]).is_err());
    // Not independent.
    let raw = json!({"packet_id": p["packet_id"], "evaluator": {"id": "J", "model": "m", "run_id": "x"}, "mode": "improvement", "candidates": [candidate(&e, &p, "A", 25.0, 65.0, "ready")]});
    assert!(evaluation::record(&mut e, &raw).is_err());
    // Blank model.
    let raw = json!({"packet_id": p["packet_id"], "evaluator": {"id": "J", "model": " ", "run_id": "x"}, "mode": "independent", "candidates": [candidate(&e, &p, "A", 25.0, 65.0, "ready")]});
    assert!(evaluation::record(&mut e, &raw).is_err());
    // Pairwise only for comparison packets.
    let raw = json!({"packet_id": p["packet_id"], "evaluator": {"id": "J", "model": "m", "run_id": "x"}, "mode": "independent", "candidates": [candidate(&e, &p, "A", 25.0, 65.0, "ready")], "pairwise": [{"preferred": "A", "over": "B"}]});
    assert!(evaluation::record(&mut e, &raw).is_err());

    let candidates = vec![candidate(&e, &p, "A", 25.0, 65.0, "ready")];
    let first = assess(&mut e, &p, "r1", candidates).unwrap();
    assert_eq!(first["duplicate"], false);
    let candidates = vec![candidate(&e, &p, "A", 25.0, 65.0, "ready")];
    let again = assess(&mut e, &p, "r1", candidates).unwrap();
    assert_eq!((again["duplicate"].as_bool(), &again["assessment_id"]), (Some(true), &first["assessment_id"]));
    let candidates = vec![candidate(&e, &p, "A", 20.0, 65.0, "revise")];
    assert!(assess(&mut e, &p, "r1", candidates).is_err(), "a run is immutable");
    assert_eq!(e.extra[evaluation::KEY]["assessments"].as_array().unwrap().len(), 1);
    assert!(e.scores.iter().all(|s| s.get("assessment_id").is_none()), "assessments stay apart from scores");
}

#[test]
fn validate_final_requires_an_exact_current_match() {
    let mut e = entry("eval-final");
    let request = final_request(&e);
    let p = evaluation::prepare(&mut e, &request).unwrap();
    let candidates = vec![candidate(&e, &p, "A", 25.0, 65.0, "ready")];
    let a = assess(&mut e, &p, "r1", candidates).unwrap();
    let judge = json!({"stage": "final", "total": 90.0, "verdict": "ready", "scores": scores(25.0, 65.0),
        "artifacts": [file(&e, "홍길동_제안서.md")], "evaluation_id": p["packet_id"], "assessment_id": a["assessment_id"]});

    // No ids: the explicit human path; an `independent: true` flag alone counts for nothing.
    let plain = json!({"total": 90.0, "verdict": "ready", "scores": scores(25.0, 65.0), "independent": true});
    assert!(evaluation::validate_final(&e, &plain).unwrap().is_none());

    let binding = evaluation::validate_final(&e, &judge).unwrap().unwrap();
    assert_eq!(binding["assessment_id"], a["assessment_id"]);
    assert_eq!(binding["evaluator"]["model"], "fixture-model");

    let mut other = judge.clone();
    other["total"] = json!(91.0);
    other["scores"] = scores(26.0, 65.0);
    assert!(evaluation::validate_final(&e, &other).is_err(), "different score");
    let mut other = judge.clone();
    other["verdict"] = json!("revise");
    assert!(evaluation::validate_final(&e, &other).is_err(), "different verdict");
    let mut other = judge.clone();
    other["artifacts"] = json!([file(&e, "alt.md")]);
    assert!(evaluation::validate_final(&e, &other).is_err(), "different reviewed file");
    let mut other = judge.clone();
    other["evaluation_id"] = json!("000000000000000000000000");
    assert!(evaluation::validate_final(&e, &other).is_err(), "wrong packet");
    let mut other = judge.clone();
    other.as_object_mut().unwrap().remove("assessment_id");
    assert!(evaluation::validate_final(&e, &other).is_err(), "packet without assessment");

    // final_problem reads the binding the parent stores in the final score.
    assert_eq!(evaluation::final_problem(&e), Some(evaluation::FINAL_MISSING));
    let mut final_score = json!({"stage": "final", "total": 90.0, "verdict": "ready"});
    final_score[evaluation::SCORE_KEY] = binding;
    e.scores.push(final_score);
    assert_eq!(evaluation::final_problem(&e), None);
    fs::write(folder(&e).join("홍길동_제안서.md"), "고친 결과물\n").unwrap();
    assert_eq!(evaluation::final_problem(&e), Some(evaluation::FINAL_STALE));
    assert!(evaluation::validate_final(&e, &judge).is_err(), "stale after the source changed");

    // A comparison packet never binds a final.
    let mut e = entry("eval-final-cmp");
    let request = comparison_request(&e);
    let p = evaluation::prepare(&mut e, &request).unwrap();
    let candidates = vec![candidate(&e, &p, "A", 25.0, 65.0, "ready"), candidate(&e, &p, "B", 20.0, 60.0, "revise")];
    let a = assess(&mut e, &p, "r1", candidates).unwrap();
    let judge = json!({"total": 90.0, "verdict": "ready", "scores": scores(25.0, 65.0), "artifacts": [file(&e, "홍길동_제안서.md")], "assessment_id": a["assessment_id"]});
    assert!(evaluation::validate_final(&e, &judge).is_err());
}

#[test]
fn report_keeps_dispersion_unknown_below_two_samples() {
    let mut e = entry("eval-stats");
    let request = final_request(&e);
    let p = evaluation::prepare(&mut e, &request).unwrap();
    assert_eq!(evaluation::report(&e)["packets"][0]["candidates"][0]["n"], 0);
    let candidates = vec![candidate(&e, &p, "A", 25.0, 65.0, "ready")];
    assess(&mut e, &p, "r1", candidates).unwrap();
    let g = evaluation::report(&e)["packets"][0]["candidates"][0].clone();
    assert_eq!(g["n"], 1);
    assert_eq!(g["dispersion"], "unavailable_n_lt_2");
    assert!(g["total"]["sample_stddev"].is_null(), "one sample is not zero dispersion");
    assert!(g["verdict_disagreement"].is_null());
    let candidates = vec![candidate(&e, &p, "A", 20.0, 60.0, "revise")];
    assess(&mut e, &p, "r2", candidates).unwrap();
    let g = evaluation::report(&e)["packets"][0]["candidates"][0].clone();
    assert_eq!(g["n"], 2);
    assert_eq!(g["dispersion"], "measured");
    assert_eq!(g["total"]["mean"].as_f64(), Some(85.0));
    let sd = g["total"]["sample_stddev"].as_f64().unwrap();
    assert!((sd - 50f64.sqrt()).abs() < 1e-9, "{sd}");
    assert_eq!((g["total"]["min"].as_f64(), g["total"]["max"].as_f64()), (Some(80.0), Some(90.0)));
    assert_eq!(g["verdict_disagreement"], true);
    assert_eq!(g["verdicts"], json!({"ready": 1, "revise": 1}));
    assert_eq!(g["criteria"][0]["score"]["n"], 2);

    let s = evaluation::stats(&[]);
    assert!(s["mean"].is_null() && s["sample_stddev"].is_null());
}

#[test]
fn comparison_separates_candidates_and_claims_no_order_from_small_samples() {
    let mut e = entry("eval-cmp");
    let request = comparison_request(&e);
    let p = evaluation::prepare(&mut e, &request).unwrap();
    assert_eq!(p["labels"], json!(["A", "B"]));
    let report = evaluation::report(&e);
    let cands = report["packets"][0]["candidates"].as_array().unwrap().clone();
    let label_of =
        |name: &str| cands.iter().find(|c| c["name"] == name).unwrap()["label"].as_str().unwrap().to_string();
    let (cur, alt) = (label_of("현재안"), label_of("대안"));
    let one = |e: &mut Entry, run: &str, cur_score: (f64, f64), alt_score: (f64, f64)| {
        let raw = json!({
            "packet_id": p["packet_id"],
            "evaluator": {"id": "ContestJudge", "model": "fixture-model", "run_id": run},
            "mode": "independent",
            "candidates": [candidate(e, &p, &cur, cur_score.0, cur_score.1, "revise"), candidate(e, &p, &alt, alt_score.0, alt_score.1, "revise")],
            "pairwise": [{"preferred": cur, "over": alt, "rationale": "근거"}],
        });
        evaluation::record(e, &raw).unwrap();
    };
    one(&mut e, "r1", (21.0, 60.0), (20.0, 60.0));
    let c = evaluation::report(&e)["packets"][0]["comparisons"][0].clone();
    assert_eq!(c["separation"], "insufficient_samples");
    assert!(c["higher"].is_null());
    one(&mut e, "r2", (20.0, 59.0), (20.0, 60.0));
    let c = evaluation::report(&e)["packets"][0]["comparisons"][0].clone();
    assert_eq!(c["separation"], "overlapping", "a one-point fluctuation is not an improvement");
    one(&mut e, "r3", (28.0, 66.0), (15.0, 50.0));
    one(&mut e, "r4", (27.0, 66.0), (15.0, 49.0));
    // Same packet, separate candidates: still overlapping because r1/r2 overlap.
    let report = evaluation::report(&e);
    let c = &report["packets"][0]["comparisons"][0];
    assert_eq!(c["separation"], "overlapping");
    assert_eq!(report["packets"][0]["pairwise"][0]["count"], 4);
    let groups = report["packets"][0]["candidates"].as_array().unwrap();
    assert!(groups.iter().all(|g| g["n"] == 4));
    assert_ne!(groups[0]["total"], groups[1]["total"]);
}

#[test]
fn outcomes_need_official_evidence_and_calibration_only_counts() {
    let mut won = entry("eval-won");
    let request = final_request(&won);
    let p = evaluation::prepare(&mut won, &request).unwrap();
    let candidates = vec![candidate(&won, &p, "A", 25.0, 65.0, "ready")];
    assess(&mut won, &p, "r1", candidates).unwrap();
    let raw = |e: &Entry| {
        fs::create_dir_all(folder(e).join(".private/outcome")).unwrap();
        fs::write(folder(e).join(".private/outcome/notice.html"), "발표\n").unwrap();
        json!({"result": "won", "round": "본선", "award": "대상", "source_url": "https://organizer.example/notice/1",
            "announced_at": "2026-10-20", "evidence": file(e, ".private/outcome/notice.html"), "official_score": 92, "official_max": 100})
    };
    let request = raw(&won);
    assert!(evaluation::outcome(&mut won, &request).is_err(), "only after submission");
    transition(&mut won, "awaiting_approval", "t", "").unwrap();
    transition(&mut won, "submitted", "t", "").unwrap();
    let mut bad = raw(&won);
    bad["result"] = json!("probably_won");
    assert!(evaluation::outcome(&mut won, &bad).is_err());
    let mut bad = raw(&won);
    bad["source_url"] = json!("file:///etc/passwd");
    assert!(evaluation::outcome(&mut won, &bad).is_err());
    let mut bad = raw(&won);
    bad["evidence"]["path"] = json!("../notice.html");
    assert!(evaluation::outcome(&mut won, &bad).is_err());
    let mut bad = raw(&won);
    bad["win_probability"] = json!(0.8);
    assert!(evaluation::outcome(&mut won, &bad).is_err());
    let request = raw(&won);
    let first = evaluation::outcome(&mut won, &request).unwrap();
    assert_eq!(first["link"]["basis"], "latest_independent_final_packet");
    let request = raw(&won);
    assert_eq!(evaluation::outcome(&mut won, &request).unwrap()["duplicate"], true);
    let mut conflict = raw(&won);
    conflict["result"] = json!("lost");
    assert!(evaluation::outcome(&mut won, &conflict).is_err(), "a recorded result is not rewritten");

    let mut lost = entry("eval-lost");
    transition(&mut lost, "awaiting_approval", "t", "").unwrap();
    transition(&mut lost, "submitted", "t", "").unwrap();
    let mut l = raw(&lost);
    l["result"] = json!("lost");
    l.as_object_mut().unwrap().remove("official_score");
    l.as_object_mut().unwrap().remove("award");
    evaluation::outcome(&mut lost, &l).unwrap();
    let stored = &lost.extra[evaluation::KEY]["outcomes"][0];
    assert!(
        stored["official_score"].is_null() && stored["award"].is_null() && stored["link"].is_null(),
        "unknowns stay unknown"
    );

    let report = evaluation::report(&won);
    assert_eq!(report["outcomes"][0]["linked_estimate"]["n"], 1);
    assert_eq!(report["outcomes"][0]["linked_estimate"]["dispersion"], "unavailable_n_lt_2");

    let mut registry = Registry::default();
    registry.contests.insert(won.id.clone(), won);
    registry.contests.insert(lost.id.clone(), lost);
    registry.contests.insert("untouched".into(), entry("eval-untouched"));
    let cal = evaluation::calibration(&registry);
    assert_eq!(cal["observed"], json!({"entries": 2, "outcomes": 2, "linked": 1, "unlinked": 1}));
    assert_eq!(cal["by_result"]["won"]["n"], 1);
    assert_eq!(cal["by_result"]["won"]["linked"], 1);
    assert_eq!(cal["by_result"]["won"]["independent_mean_total"]["mean"].as_f64(), Some(90.0));
    assert_eq!(cal["by_result"]["lost"]["linked"], 0);
    assert_eq!(cal["by_result"]["won"]["adequacy"], "inadequate_data");
    assert_eq!(cal["adequacy"], "inadequate_data");
    assert_eq!(cal["by_type"]["idea"], json!({"lost": 1, "won": 1}));
    assert_eq!(cal["official_score_pairs"]["n"], 1);
    assert_eq!(cal["official_score_pairs"]["rows"][0]["official_pct"].as_f64(), Some(92.0));
    let text = cal.to_string();
    assert!(!text.contains("probability\":") && !text.contains("calibrated"));
}
