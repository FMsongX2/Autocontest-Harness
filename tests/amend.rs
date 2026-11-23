//! `ach gate-criteria`: filling in the criteria a recorded gate left empty. The correction changes
//! that one field, only against accepted official rule evidence and the work the caller reviewed,
//! and refuses (changing nothing) otherwise. Every score here is a test fixture.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use harness::amend::{self, Request};
use harness::evaluation;
use harness::readiness;
use harness::registry::{self, Entry, HistoryItem, NewEntry, apply_gate, new_entry, transition};
use harness::rules;
use serde_json::{Value, json};

const URL: &str = "https://organizer.example/notice";
const NOTICE: &str = "<html><body><h2>심사기준</h2><p>창의성 40점, 실현 가능성 60점</p></body></html>";

fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "harness-amend-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn folder(e: &Entry) -> PathBuf {
    PathBuf::from(e.folder.as_deref().unwrap())
}

fn official() -> Value {
    json!([
        {"stage": "1차 서면심사", "name": "창의성", "weight": 40},
        {"stage": "1차 서면심사", "name": "실현 가능성", "weight": 60}
    ])
}

fn score(stage: &str, a: f64, b: f64) -> Value {
    json!({"at": "t", "stage": stage, "total": a + b, "target": 90.0, "verdict": "ready", "revision": 3, "criteria": [
        {"name": "창의성", "round": "1차 서면심사", "weight": 40.0, "score": a},
        {"name": "실현 가능성", "round": "1차 서면심사", "weight": 60.0, "score": b}
    ]})
}

/// Accept `criteria` as the structured criteria rule of a baseline rule version (v1).
fn accept_rules(e: &mut Entry, criteria: Value) {
    let inbox = folder(e).join("inbox");
    fs::create_dir_all(&inbox).unwrap();
    fs::write(inbox.join("notice.html"), NOTICE).unwrap();
    rules::capture(e, &json!({"sources": [{"id": "notice", "url": URL, "path": "inbox/notice.html"}]})).unwrap();
    let sha = readiness::sha256_file(&inbox.join("notice.html")).unwrap();
    let source = json!({"id": "notice", "sha256": sha, "locator": {"quote": "창의성 40점, 실현 가능성 60점"}});
    let out = rules::review(
        e,
        &json!({"version": "v1", "base_version": null, "decision": "accept", "reviewer": "ContestGate",
            "rules": [{"category": "criteria", "value": criteria, "source": source}]}),
    )
    .unwrap();
    assert_eq!(out["status"], "accepted");
}

/// A level 2 entry waiting for approval: the gate passed with empty criteria, the official
/// criteria are an accepted rule, two scores used them, and a readiness record gives a token.
fn entry_with(id: &str, rule_criteria: Value) -> Entry {
    let mut e = new_entry(NewEntry {
        id,
        title: "제도개선 아이디어 공모",
        url: URL,
        source: "manual",
        organizer: "",
        deadline: None,
        categories: vec![],
        note: None,
    });
    let gate = json!({"decision": "pass", "type": "idea", "level": 2, "priority": 70,
        "deliverables": [{"name": "제안서", "format": "hwpx"}], "criteria": [], "reasons": ["근거"]});
    apply_gate(&mut e, gate, "ContestGate").unwrap();
    let dir = temp_dir(id);
    e.folder = Some(dir.to_string_lossy().into_owned());
    transition(&mut e, "routed", "t", "").unwrap();
    transition(&mut e, "running", "t", "").unwrap();
    fs::create_dir_all(dir.join(".harness")).unwrap();
    fs::write(dir.join(amend::FOLDER_GATE), serde_json::to_string_pretty(&e.gate).unwrap() + "\n").unwrap();
    fs::write(dir.join("proposal.md"), "제안서 본문\n").unwrap();
    fs::write(dir.join("RUBRIC.md"), "기준 풀이\n").unwrap();
    accept_rules(&mut e, rule_criteria);
    e.scores.push(score("plan", 30.0, 50.0));
    e.scores.push(score("final", 36.0, 55.0));
    transition(&mut e, "awaiting_approval", "t", "").unwrap();
    let artifact = json!({"path": "proposal.md", "sha256": readiness::sha256_file(&dir.join("proposal.md")).unwrap()});
    readiness::record(
        &mut e,
        &json!({"submission_ready": true, "needs_user_input": false, "blockers": [], "followup_required": false,
            "artifacts": [artifact], "destination": "contest@organizer.example"}),
    )
    .unwrap();
    e
}

fn entry(id: &str) -> Entry {
    let mut rule = official();
    rule.as_array_mut().unwrap().push(json!({"note": "1차 서면심사 뒤 국민투표"}));
    entry_with(id, rule)
}

/// The recorded gate with `criteria` replaced.
fn corrected(e: &Entry, criteria: Value) -> Value {
    let mut gate = e.gate.clone().unwrap();
    gate["criteria"] = criteria;
    gate
}

fn token(e: &Entry) -> Option<String> {
    readiness::evaluate(e, false, false).token
}

struct Call {
    text: String,
    revision: usize,
    token: Option<String>,
    session: Option<&'static str>,
    dry_run: bool,
}

impl Call {
    /// What a main session passes after reviewing `e`: its revision and request token.
    fn reviewed(e: &Entry, gate: &Value) -> Call {
        Call { text: gate.to_string(), revision: registry::revision(e), token: token(e), session: None, dry_run: false }
    }

    fn run(&self, e: &mut Entry) -> harness::error::Result<Value> {
        amend::gate_criteria(
            e,
            &Request {
                path: Path::new("gate-corrected.json"),
                text: &self.text,
                expected_revision: self.revision,
                expected_request: self.token.as_deref(),
                session: self.session,
                dry_run: self.dry_run,
            },
        )
    }
}

fn folder_gate(e: &Entry) -> Vec<u8> {
    fs::read(folder(e).join(amend::FOLDER_GATE)).unwrap()
}

/// The call is refused with `needle` and neither the entry nor the folder copy changes.
fn assert_refused(e: &mut Entry, call: &Call, needle: &str) {
    let (before, file) = (serde_json::to_value(&*e).unwrap(), folder_gate(e));
    let err = call.run(e).unwrap_err();
    assert!(err.0.contains(needle), "expected {needle:?} in {:?}", err.0);
    assert_eq!(serde_json::to_value(&*e).unwrap(), before, "a refused correction changed the entry");
    assert_eq!(folder_gate(e), file, "a refused correction changed the folder copy");
}

fn packet_request(e: &Entry) -> Value {
    let file = |rel: &str| json!({"path": rel, "sha256": readiness::sha256_file(&folder(e).join(rel)).unwrap()});
    json!({
        "purpose": "independent_final",
        "candidates": [{"artifacts": [{"source": file("proposal.md"), "sanitized": file("proposal.md")}]}],
        "rubric": file("RUBRIC.md"),
        "attestation": {"by": "session", "identity_removed": true, "statement": "신원 정보가 없는 사본이다"},
    })
}

#[test]
fn fills_only_the_criteria_and_unblocks_the_evaluation_packet() {
    let mut e = entry("amend-ok");
    let request = packet_request(&e);
    assert!(evaluation::prepare(&mut e, &request).unwrap_err().0.contains("criteria"));

    let before = e.clone();
    let reviewed_token = token(&e);
    let out = Call::reviewed(&e, &corrected(&e, official())).run(&mut e).unwrap();
    assert_eq!(out["dry_run"], false);
    assert_eq!(out["before"], json!([]));
    assert_eq!(out["after"], official());
    assert_eq!(out["weight_total"], 100.0);
    assert_eq!(out["amendment"], 1);

    // Only gate.criteria moved: every other gate field, the state, the revision, the request
    // token, the scores and the readiness record are as they were.
    let mut expected_gate = before.gate.clone().unwrap();
    expected_gate["criteria"] = official();
    assert_eq!(e.gate, Some(expected_gate.clone()));
    assert_eq!(e.state, "awaiting_approval");
    assert_eq!(registry::revision(&e), registry::revision(&before));
    assert_eq!(token(&e), reviewed_token);
    assert_eq!(e.scores, before.scores);
    assert_eq!(e.extra[readiness::KEY], before.extra[readiness::KEY]);
    assert_eq!(e.extra[rules::KEY], before.extra[rules::KEY]);

    // The change is on record: one amendment with its evidence, and a same-state history marker.
    let record = &e.extra[amend::KEY][0];
    assert_eq!(record["seq"], 1);
    assert_eq!(record["before"], json!([]));
    assert_eq!(record["after"], official());
    assert_eq!(record["evidence"]["rules_version"], "v1");
    assert_eq!(record["evidence"]["rules"][0]["source"]["id"], "notice");
    assert_eq!(record["request_token"], json!(reviewed_token));
    assert_eq!(e.history.len(), before.history.len() + 1);
    let marker = e.history.last().unwrap();
    assert_eq!(
        (marker.from.as_deref(), marker.to.as_str(), marker.by.as_str()),
        (Some("awaiting_approval"), "awaiting_approval", amend::BY)
    );
    assert_eq!(marker.extra["gate_amendment"], 1);

    // The folder copy that sessions read says the same as the registry.
    let on_disk: Value = serde_json::from_slice(&folder_gate(&e)).unwrap();
    assert_eq!(on_disk, expected_gate);
    assert_eq!(out["folder_gate"]["sha256"], readiness::sha256_file(&folder(&e).join(amend::FOLDER_GATE)).unwrap());

    // The packet the empty gate refused can now be prepared, with the official criteria in it.
    let prepared = evaluation::prepare(&mut e, &request).unwrap();
    let manifest: Value = serde_json::from_slice(
        &fs::read(folder(&e).join(prepared["packet_dir"].as_str().unwrap()).join(evaluation::MANIFEST)).unwrap(),
    )
    .unwrap();
    assert_eq!(
        manifest["criteria"],
        json!([
            {"round": "1차 서면심사", "name": "창의성", "weight": 40.0},
            {"round": "1차 서면심사", "name": "실현 가능성", "weight": 60.0}
        ])
    );

    // A second correction has nothing empty left to fill.
    let again = Call::reviewed(&e, &corrected(&e, official()));
    assert_refused(&mut e, &again, "이미 있다");
}

#[test]
fn dry_run_reports_the_change_and_writes_nothing() {
    let mut e = entry("amend-dry");
    let (before, file) = (serde_json::to_value(&e).unwrap(), folder_gate(&e));
    let mut call = Call::reviewed(&e, &corrected(&e, official()));
    call.dry_run = true;
    let out = call.run(&mut e).unwrap();
    assert_eq!(out["dry_run"], true);
    assert_eq!(out["after"], official());
    assert_eq!(out["amendment"], Value::Null);
    assert_eq!(serde_json::to_value(&e).unwrap(), before);
    assert_eq!(folder_gate(&e), file);
}

#[test]
fn only_a_main_session_with_the_reviewed_revision_and_token_may_correct() {
    let mut e = entry("amend-cas");
    let gate = corrected(&e, official());

    let mut call = Call::reviewed(&e, &gate);
    call.session = Some("amend-cas");
    assert_refused(&mut e, &call, "ACH_SESSION");

    let mut call = Call::reviewed(&e, &gate);
    call.revision += 1;
    assert_refused(&mut e, &call, "작업 판이 다르다");

    let mut call = Call::reviewed(&e, &gate);
    call.token = None;
    assert_refused(&mut e, &call, "요청 토큰이 다르다");

    // A score recorded after the review changes the token: the reviewed one is stale.
    let stale = Call::reviewed(&e, &gate);
    e.scores.push(score("final", 37.0, 55.0));
    assert_refused(&mut e, &stale, "요청 토큰이 다르다");
}

#[test]
fn the_corrected_file_may_differ_from_the_recorded_gate_only_in_criteria() {
    let mut e = entry("amend-diff");

    let mut gate = corrected(&e, official());
    gate["level"] = json!(3);
    gate["prize"] = json!("상금");
    let call = Call::reviewed(&e, &gate);
    assert_refused(&mut e, &call, "criteria 밖에서 다르다(level, prize)");

    let mut gate = corrected(&e, official());
    gate.as_object_mut().unwrap().remove("deliverables");
    let call = Call::reviewed(&e, &gate);
    assert_refused(&mut e, &call, "criteria 밖에서 다르다(deliverables)");

    for (criteria, needle) in [
        (json!([]), "비어 있지 않은"),
        (json!([{"stage": "1차 서면심사", "name": "창의성", "weight": 40, "score": 40}]), "알 수 없는 필드 score"),
        (json!([{"name": "창의성", "weight": -1}]), "0 이상의 숫자"),
        (json!([{"name": "창의성", "weight": 40}, {"name": "창의성", "weight": 60}]), "두 번 나온다"),
        (json!([{"name": "주제 적합", "weight": 0}]), "배점이 있는 기준이 없다"),
    ] {
        let call = Call::reviewed(&e, &corrected(&e, criteria));
        assert_refused(&mut e, &call, needle);
    }
}

#[test]
fn criteria_must_equal_the_accepted_official_rule() {
    let mut e = entry("amend-evidence");
    for criteria in [
        // A different weight, a missing criterion, an extra one and a different stage.
        json!([{"stage": "1차 서면심사", "name": "창의성", "weight": 50}, {"stage": "1차 서면심사", "name": "실현 가능성", "weight": 50}]),
        json!([{"stage": "1차 서면심사", "name": "창의성", "weight": 40}]),
        json!([{"stage": "1차 서면심사", "name": "창의성", "weight": 40}, {"stage": "1차 서면심사", "name": "실현 가능성", "weight": 60},
            {"stage": "1차 서면심사", "name": "논리성", "weight": 20}]),
        json!([{"stage": "본선", "name": "창의성", "weight": 40}, {"stage": "본선", "name": "실현 가능성", "weight": 60}]),
    ] {
        let call = Call::reviewed(&e, &corrected(&e, criteria));
        assert_refused(&mut e, &call, "수락된 규칙 판 v1의 criteria와 다르다");
    }

    // Prose is not structured evidence.
    let mut prose = entry_with("amend-prose", json!("창의성 40점, 실현 가능성 60점"));
    let call = Call::reviewed(&prose, &corrected(&prose, official()));
    assert_refused(&mut prose, &call, "구조화된 기준 행");

    // No accepted rules at all.
    let mut untracked = entry("amend-untracked");
    untracked.extra.remove(rules::KEY);
    let call = Call::reviewed(&untracked, &corrected(&untracked, official()));
    assert_refused(&mut untracked, &call, "수락된 공식 규칙 판이 없다");

    // A changed notice waiting for review.
    let mut pending = entry("amend-pending");
    fs::write(folder(&pending).join("inbox/notice.html"), NOTICE.replace("40점", "45점")).unwrap();
    rules::capture(&mut pending, &json!({"sources": [{"id": "notice", "url": URL, "path": "inbox/notice.html"}]}))
        .unwrap();
    let call = Call::reviewed(&pending, &corrected(&pending, official()));
    assert_refused(&mut pending, &call, "rules_pending_review");
}

#[test]
fn an_altered_rule_snapshot_is_not_evidence() {
    use std::os::unix::fs::PermissionsExt;
    let mut e = entry("amend-tamper");
    let blob = folder(&e).join(e.extra[rules::KEY]["versions"][0]["sources"][0]["blob"].as_str().unwrap());
    fs::set_permissions(&blob, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&blob, "<p>창의성 90점</p>").unwrap();
    let call = Call::reviewed(&e, &corrected(&e, official()));
    assert_refused(&mut e, &call, "변조");
}

#[test]
fn work_reviewed_against_the_old_gate_is_never_carried_over() {
    // A submission approval recorded in this approval wait, even one that no longer counts.
    let mut approved = entry("amend-approved");
    let mut marker =
        HistoryItem::new("t".into(), Some("awaiting_approval".into()), "awaiting_approval", "user", "승인");
    marker.extra.insert("approval".into(), json!("submit"));
    approved.history.push(marker);
    let call = Call::reviewed(&approved, &corrected(&approved, official()));
    assert_refused(&mut approved, &call, "제출 승인 기록이 있다");

    // An evaluation record that an empty gate could not have produced.
    let mut assessed = entry("amend-assessed");
    assessed.extra.insert(evaluation::KEY.into(), json!({"packets": [{"packet_id": "p"}]}));
    let call = Call::reviewed(&assessed, &corrected(&assessed, official()));
    assert_refused(&mut assessed, &call, "독립 평가 묶음");

    // A recorded score that used other criteria than the corrected ones.
    let mut scored = entry("amend-scored");
    scored.scores.push(json!({"at": "t", "stage": "draft", "total": 80.0, "verdict": "revise", "criteria": [
        {"name": "창의성", "round": "1차 서면심사", "weight": 50.0, "score": 40.0},
        {"name": "실현 가능성", "round": "1차 서면심사", "weight": 50.0, "score": 40.0}
    ]}));
    let call = Call::reviewed(&scored, &corrected(&scored, official()));
    assert_refused(&mut scored, &call, "기록된 점수 3(draft)");
}

#[test]
fn submitted_or_in_flight_entries_are_not_corrected() {
    let mut recorded = entry("amend-submitted");
    recorded.submissions.push(json!({"at": "t", "method": "email"}));
    let call = Call::reviewed(&recorded, &corrected(&recorded, official()));
    assert_refused(&mut recorded, &call, "제출 기록이 있는");

    let mut attempt = entry("amend-attempt");
    attempt.extra.insert(
        "submission_txn".into(),
        json!({"next_attempt": 2, "attempts": [{"attempt_id": "a1", "status": "unknown"}]}),
    );
    let call = Call::reviewed(&attempt, &corrected(&attempt, official()));
    assert_refused(&mut attempt, &call, "해소되지 않은 제출 시도");

    let mut skipped = entry("amend-skipped");
    let gate = corrected(&skipped, official());
    transition(&mut skipped, "skipped", "t", "").unwrap();
    let call = Call::reviewed(&skipped, &gate);
    assert_refused(&mut skipped, &call, "running·plan_review·awaiting_approval에서만");
}

#[test]
fn a_folder_copy_that_differs_from_the_recorded_gate_is_not_overwritten() {
    let mut e = entry("amend-folder");
    let mut other = e.gate.clone().unwrap();
    other["prize"] = json!("손으로 고친 값");
    fs::write(folder(&e).join(amend::FOLDER_GATE), other.to_string()).unwrap();
    let call = Call::reviewed(&e, &corrected(&e, official()));
    assert_refused(&mut e, &call, "기록된 게이트와 다르다");

    // A copy that already holds the correction (an interrupted earlier run) is accepted.
    let fixed = corrected(&e, official());
    fs::write(folder(&e).join(amend::FOLDER_GATE), fixed.to_string()).unwrap();
    Call::reviewed(&e, &fixed).run(&mut e).unwrap();
    assert_eq!(e.gate, Some(fixed));
}
