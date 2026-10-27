//! Official rule evidence: byte snapshots from local files and a local HTTP fixture, pending
//! review, typed semantic comparison, selective stage invalidation, acknowledgement and score
//! hooks. No production URLs: every fetch goes to 127.0.0.1.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use harness::registry::{Entry, NewEntry, new_entry};
use harness::rules::{self, Limits};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "harness-rules-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// An entry with a contest folder (snapshots go to `<folder>/.private/rules`).
fn entry(id: &str) -> Entry {
    let mut e = new_entry(NewEntry {
        id,
        title: "공모전",
        url: "https://organizer.example/notice",
        source: "manual",
        organizer: "",
        deadline: None,
        categories: vec![],
        note: None,
    });
    e.folder = Some(temp_dir(id).to_string_lossy().into_owned());
    e
}

fn folder(e: &Entry) -> PathBuf {
    PathBuf::from(e.folder.as_deref().unwrap())
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

const URL: &str = "https://organizer.example/notice";

fn page(eligibility: &str, banner: &str, deadline: &str, form: &str) -> String {
    format!(
        "<html><body><div class=banner>{banner}</div><h2>참가 자격</h2><p>{eligibility}</p>\
         <h2>마감</h2><p>{deadline}</p><h2>서식</h2><p>{form}</p><h2>심사기준</h2><p>창의성 50, 완성도 50</p>\
         <h2>AI</h2><p>생성형 AI 표기 시 허용</p><h2>접수</h2><p>contest@organizer.example 메일 접수</p></body></html>"
    )
}

/// Write the notice into `inbox/notice.html` and capture it.
fn capture_page(e: &mut Entry, body: &str) -> Value {
    let inbox = folder(e).join("inbox");
    fs::create_dir_all(&inbox).unwrap();
    fs::write(inbox.join("notice.html"), body).unwrap();
    rules::capture(e, &json!({"sources": [{"id": "notice", "url": URL, "path": "inbox/notice.html"}]})).unwrap()
}

fn rule(category: &str, value: Value, sha: &str, quote: &str) -> Value {
    json!({"category": category, "value": value,
        "source": {"id": "notice", "sha256": sha, "locator": {"position": "본문", "quote": quote}}})
}

/// A full structured review of the page made by [`page`].
fn review_raw(e: &Entry, version: &str, base: Option<&str>, eligibility: &str, deadline: &str, form: &str) -> Value {
    let body = fs::read(folder(e).join("inbox/notice.html")).unwrap();
    let s = sha(&body);
    json!({"version": version, "base_version": base, "decision": "accept", "reviewer": "ContestGate",
    "rules": [
        rule("eligibility", json!(eligibility), &s, eligibility),
        rule("deadline", json!(deadline), &s, deadline),
        rule("forms", json!(form), &s, form),
        rule("criteria", json!("창의성 50, 완성도 50"), &s, "창의성 50, 완성도 50"),
        rule("ai_policy", json!("표기 조건부 허용"), &s, "생성형 AI 표기 시 허용"),
        rule("submission", json!("contest@organizer.example 메일"), &s, "contest@organizer.example 메일 접수"),
    ]})
}

const ELIG: &str = "대한민국 대학생 개인";
const DEADLINE: &str = "2026-11-30 18:00";
const FORM: &str = "신청서 양식 1부";

/// An entry with an accepted baseline (v1).
fn baseline(id: &str) -> Entry {
    let mut e = entry(id);
    let out = capture_page(&mut e, &page(ELIG, "공지", DEADLINE, FORM));
    assert_eq!(out["status"], "baseline_pending");
    let raw = review_raw(&e, "v1", None, ELIG, DEADLINE, FORM);
    let out = rules::review(&mut e, &raw).unwrap();
    assert_eq!(out["status"], "accepted");
    assert_eq!(out["baseline"], true);
    e
}

fn push_score(e: &mut Entry, stage: &str) {
    e.scores.push(json!({"at": "t", "stage": stage, "total": 90, "verdict": "ready", "criteria": []}));
}

fn binding(e: &Entry) -> Value {
    rules::binding(e)
}

#[test]
fn untracked_entry_has_zero_binding_and_no_problems() {
    let e = entry("untracked");
    assert_eq!(binding(&e), json!({"gate": 0, "plan": 0, "final": 0, "submission": 0}));
    assert!(rules::approval_problems(&e).is_empty());
    assert_eq!(rules::status(&e)["tracked"], false);
    assert!(rules::check_score(&e, &json!({"stage": "final"}), "final"));
}

#[test]
fn baseline_capture_is_pending_until_reviewed_and_snapshot_is_immutable() {
    let mut e = entry("baseline");
    let body = page(ELIG, "공지", DEADLINE, FORM);
    let out = capture_page(&mut e, &body);
    assert_eq!(out["changed"], true);
    assert_eq!(out["pending_version"], "v1");
    assert_eq!(out["sources"][0]["origin"], "file");
    assert_eq!(out["sources"][0]["semantic"], "text");
    let s = sha(body.as_bytes());
    assert_eq!(out["sources"][0]["sha256"], s.as_str());
    assert!(out["sources"][0]["source_modified_at"].is_string());
    assert_eq!(rules::approval_problems(&e), vec!["rules_pending_review"]);
    assert_eq!(rules::stage_problems(&e, "plan"), vec!["rules_pending_review"]);
    assert!(!rules::tracked(&e));

    let blob = folder(&e).join(".private/rules/sha256").join(&s);
    assert_eq!(fs::read(&blob).unwrap(), body.as_bytes());
    assert_eq!(fs::metadata(&blob).unwrap().permissions().mode() & 0o777, 0o400);

    let raw = review_raw(&e, "v1", None, ELIG, DEADLINE, FORM);
    rules::review(&mut e, &raw).unwrap();
    assert!(rules::tracked(&e));
    assert_eq!(binding(&e), json!({"gate": 1, "plan": 1, "final": 1, "submission": 1}));
    assert!(rules::approval_problems(&e).is_empty());
    let status = rules::status(&e);
    assert_eq!(status["accepted_version"], "v1");
    assert!(status["pending"].is_null());
    assert_eq!(status["sources"][0]["sha256"], s.as_str());
}

#[test]
fn unchanged_bytes_add_no_version_and_no_invalidation() {
    let mut e = baseline("unchanged");
    let before = binding(&e);
    let out = capture_page(&mut e, &page(ELIG, "공지", DEADLINE, FORM));
    assert_eq!(out["status"], "unchanged");
    assert_eq!(out["changed"], false);
    assert_eq!(out["version"], "v1");
    assert_eq!(binding(&e), before);
    assert!(rules::approval_problems(&e).is_empty());
    assert_eq!(e.extra["rules"]["versions"].as_array().unwrap().len(), 1);
    assert_eq!(e.extra["rules"]["check_count"], 2);
}

#[test]
fn banner_only_change_is_pending_then_accepted_without_resetting_work() {
    let mut e = baseline("banner");
    push_score(&mut e, "plan");
    rules::score_recorded(&mut e, "plan").unwrap();
    push_score(&mut e, "final");
    rules::score_recorded(&mut e, "final").unwrap();
    let before = binding(&e);

    let out = capture_page(&mut e, &page(ELIG, "접수 기간 연장 안내 배너", DEADLINE, FORM));
    assert_eq!(out["status"], "changed_pending");
    assert_eq!(out["diff"]["changed"], json!(["notice"]));
    // A byte change is not classified: it blocks until reviewed.
    assert_eq!(rules::approval_problems(&e), vec!["rules_pending_review"]);
    let plan_score = e.scores[0].clone();
    assert!(rules::check_score(&e, &plan_score, "plan"), "pending alone does not move epochs");

    let raw = review_raw(&e, "v2", Some("v1"), ELIG, DEADLINE, FORM);
    let out = rules::review(&mut e, &raw).unwrap();
    assert_eq!(out["status"], "accepted");
    assert_eq!(out["changed_categories"], json!([]));
    assert_eq!(out["invalidated"], json!([]));
    assert_eq!(binding(&e), before);
    assert!(rules::approval_problems(&e).is_empty());
    assert!(rules::check_score(&e, &e.scores[0].clone(), "plan"));
    assert!(rules::check_score(&e, &e.scores[1].clone(), "final"));
}

#[test]
fn eligibility_change_invalidates_gate_plan_final_and_submission() {
    let mut e = baseline("eligibility");
    push_score(&mut e, "plan");
    rules::score_recorded(&mut e, "plan").unwrap();
    let old_plan = e.scores[0].clone();

    let new_elig = "대한민국 대학원생 개인";
    capture_page(&mut e, &page(new_elig, "공지", DEADLINE, FORM));
    let raw = review_raw(&e, "v2", Some("v1"), new_elig, DEADLINE, FORM);
    let out = rules::review(&mut e, &raw).unwrap();
    assert_eq!(out["changed_categories"], json!(["eligibility"]));
    assert_eq!(out["invalidated"], json!(["gate", "plan", "final"]));
    assert_eq!(binding(&e), json!({"gate": 2, "plan": 2, "final": 2, "submission": 2}));
    assert_eq!(
        rules::approval_problems(&e),
        vec!["rules_gate_invalidated", "rules_plan_invalidated", "rules_final_invalidated"]
    );
    assert_eq!(rules::stage_problems(&e, "plan"), vec!["rules_gate_invalidated", "rules_plan_invalidated"]);
    assert!(!rules::check_score(&e, &old_plan, "plan"));

    // A new plan score clears plan only; the gate needs its own review.
    push_score(&mut e, "plan");
    rules::score_recorded(&mut e, "plan").unwrap();
    assert_eq!(e.scores[1]["rules_binding"]["plan"], 2);
    assert_eq!(e.scores[1]["rules_binding"]["version"], "v2");
    assert!(rules::check_score(&e, &e.scores[1].clone(), "plan"));
    assert_eq!(rules::approval_problems(&e), vec!["rules_gate_invalidated", "rules_final_invalidated"]);

    let gate = folder(&e).join("gate.json");
    fs::write(&gate, "{\"decision\":\"pass\"}").unwrap();
    let ack = json!({"rules_version": "v2", "epoch": 2, "decision": "pass", "reviewer": "ContestGate",
        "rationale": "대학원생 자격으로 바뀐 조건을 다시 대조해 통과로 판정했다",
        "evidence": [{"path": "gate.json", "sha256": sha(&fs::read(&gate).unwrap())}]});
    let out = rules::acknowledge(&mut e, "gate", &ack).unwrap();
    assert_eq!(out["remaining"], json!(["final"]));
    assert_eq!(rules::approval_problems(&e), vec!["rules_final_invalidated"]);
    // History is kept.
    assert_eq!(e.extra["rules"]["acknowledgements"].as_array().unwrap().len(), 2);
}

#[test]
fn forms_change_invalidates_only_final() {
    let mut e = baseline("forms");
    let new_form = "신청서 양식 1부와 서약서 1부";
    capture_page(&mut e, &page(ELIG, "공지", DEADLINE, new_form));
    let raw = review_raw(&e, "v2", Some("v1"), ELIG, DEADLINE, new_form);
    let out = rules::review(&mut e, &raw).unwrap();
    assert_eq!(out["changed_categories"], json!(["forms"]));
    assert_eq!(out["invalidated"], json!(["final"]));
    assert_eq!(binding(&e), json!({"gate": 1, "plan": 1, "final": 2, "submission": 2}));
    assert!(rules::stage_problems(&e, "plan").is_empty(), "a forms change never blocks plan review");
    assert_eq!(rules::approval_problems(&e), vec!["rules_final_invalidated"]);
}

#[test]
fn deadline_change_moves_only_the_submission_epoch() {
    let mut e = baseline("deadline");
    push_score(&mut e, "final");
    rules::score_recorded(&mut e, "final").unwrap();
    let new_deadline = "2026-12-07 18:00";
    capture_page(&mut e, &page(ELIG, "공지", new_deadline, FORM));
    let raw = review_raw(&e, "v2", Some("v1"), ELIG, new_deadline, FORM);
    let out = rules::review(&mut e, &raw).unwrap();
    assert_eq!(out["changed_categories"], json!(["deadline"]));
    assert_eq!(out["invalidated"], json!([]));
    assert_eq!(out["submission_epoch_changed"], true);
    assert_eq!(binding(&e), json!({"gate": 1, "plan": 1, "final": 1, "submission": 2}));
    assert!(rules::approval_problems(&e).is_empty());
    assert!(rules::check_score(&e, &e.scores[0].clone(), "final"), "deadline keeps the final text valid");
}

#[test]
fn criteria_and_submission_changes_hit_their_stages() {
    let mut e = baseline("criteria");
    let body = page(ELIG, "공지", DEADLINE, FORM).replace("창의성 50, 완성도 50", "창의성 30, 완성도 70");
    capture_page(&mut e, &body);
    let mut raw = review_raw(&e, "v2", Some("v1"), ELIG, DEADLINE, FORM);
    raw["rules"][3]["value"] = json!("창의성 30, 완성도 70");
    raw["rules"][3]["source"]["locator"]["quote"] = json!("창의성 30, 완성도 70");
    let out = rules::review(&mut e, &raw).unwrap();
    assert_eq!(out["invalidated"], json!(["plan", "final"]));

    let mut e = baseline("submission");
    let body =
        page(ELIG, "공지", DEADLINE, FORM).replace("메일 접수", "메일 접수 (주소 변경: apply@organizer.example)");
    capture_page(&mut e, &body);
    let mut raw = review_raw(&e, "v2", Some("v1"), ELIG, DEADLINE, FORM);
    raw["rules"][5]["value"] = json!("apply@organizer.example 메일");
    raw["rules"][5]["source"]["locator"]["quote"] = json!("apply@organizer.example");
    let out = rules::review(&mut e, &raw).unwrap();
    assert_eq!(out["changed_categories"], json!(["submission"]));
    assert_eq!(out["invalidated"], json!([]));
    assert_eq!(binding(&e), json!({"gate": 1, "plan": 1, "final": 1, "submission": 2}));
}

#[test]
fn whitespace_and_key_order_are_not_semantic_changes() {
    let mut e = baseline("canon");
    capture_page(&mut e, &page(ELIG, "배너 2", DEADLINE, FORM));
    let mut raw = review_raw(&e, "v2", Some("v1"), ELIG, DEADLINE, FORM);
    raw["rules"][0]["value"] = json!("  대한민국   대학생\n개인 ");
    let out = rules::review(&mut e, &raw).unwrap();
    assert_eq!(out["changed_categories"], json!([]));
}

#[test]
fn ambiguous_or_unresolved_review_stays_pending() {
    let mut e = baseline("ambiguous");
    capture_page(&mut e, &page("대학생 또는 졸업 예정자", "공지", DEADLINE, FORM));
    let before = binding(&e);
    let mut raw = review_raw(&e, "v2", Some("v1"), "대학생 또는 졸업 예정자", DEADLINE, FORM);
    raw["rules"][0]["ambiguous"] = json!(true);
    let out = rules::review(&mut e, &raw).unwrap();
    assert_eq!(out["status"], "pending");
    assert_eq!(out["ambiguous_rules"], json!([0]));
    assert_eq!(rules::approval_problems(&e), vec!["rules_pending_review"]);
    assert_eq!(binding(&e), before);

    let out = rules::review(
        &mut e,
        &json!({"version": "v2", "base_version": "v1", "decision": "unresolved", "reviewer": "ContestGate",
            "reason": "첨부 HWP를 읽지 못했다"}),
    )
    .unwrap();
    assert_eq!(out["status"], "pending");
    assert_eq!(rules::status(&e)["pending"]["version"], "v2");
    assert!(
        rules::review(
            &mut e,
            &json!({"version": "v2", "base_version": "v1", "decision": "unresolved", "reviewer": "ContestGate"})
        )
        .is_err()
    );
}

/// Every rejected review leaves the stored state exactly as it was.
fn assert_rejected(e: &mut Entry, raw: &Value, needle: &str) {
    let before = e.extra.get("rules").cloned();
    let err = rules::review(e, raw).unwrap_err();
    assert!(err.0.contains(needle), "{needle}: {}", err.0);
    assert_eq!(e.extra.get("rules").cloned(), before);
}

#[test]
fn invalid_reviews_are_rejected_atomically() {
    let mut e = baseline("invalid-review");
    let new_elig = "대한민국 대학원생 개인";
    capture_page(&mut e, &page(new_elig, "공지", DEADLINE, FORM));
    let good = review_raw(&e, "v2", Some("v1"), new_elig, DEADLINE, FORM);

    let mut raw = good.clone();
    raw["rules"][0]["source"]["sha256"] = json!(sha(b"other"));
    assert_rejected(&mut e, &raw, "sha256");
    let mut raw = good.clone();
    raw["rules"][0]["source"]["locator"] = json!("   ");
    assert_rejected(&mut e, &raw, "locator");
    let mut raw = good.clone();
    raw["rules"][0]["source"]["locator"] = json!({"quote": "원문에 없는 문장"});
    assert_rejected(&mut e, &raw, "인용문");
    let mut raw = good.clone();
    raw["rules"][0]["source"]["id"] = json!("guide");
    assert_rejected(&mut e, &raw, "guide");
    let mut raw = good.clone();
    let source_hash = raw["rules"][0]["source"]["sha256"].as_str().unwrap().to_owned();
    raw["rules"].as_array_mut().unwrap().push(rule("eligibility", json!("누구나"), &source_hash, new_elig));
    assert_rejected(&mut e, &raw, "충돌");
    let mut raw = good.clone();
    raw["rules"][0]["category"] = json!("banner");
    assert_rejected(&mut e, &raw, "category");
    let mut raw = good.clone();
    raw["rules"][0]["value"] = json!("");
    assert_rejected(&mut e, &raw, "value");
    let mut raw = good.clone();
    raw["base_version"] = Value::Null;
    assert_rejected(&mut e, &raw, "낡았다");
    let mut raw = good.clone();
    raw.as_object_mut().unwrap().remove("base_version");
    assert_rejected(&mut e, &raw, "base_version");
    let mut raw = good.clone();
    raw["reviewer"] = json!(" ");
    assert_rejected(&mut e, &raw, "reviewer");

    rules::review(&mut e, &good).unwrap();
}

#[test]
fn stale_review_of_superseded_candidate_is_refused() {
    let mut e = baseline("stale");
    capture_page(&mut e, &page(ELIG, "배너 A", DEADLINE, FORM));
    let stale = review_raw(&e, "v2", Some("v1"), ELIG, DEADLINE, FORM);
    let out = capture_page(&mut e, &page(ELIG, "배너 B", DEADLINE, FORM));
    assert_eq!(out["status"], "changed_pending");
    assert_eq!(out["pending_version"], "v3");
    assert_eq!(e.extra["rules"]["versions"][1]["status"], "superseded");
    assert_rejected(&mut e, &stale, "낡았다");
    let fresh = review_raw(&e, "v3", Some("v1"), ELIG, DEADLINE, FORM);
    rules::review(&mut e, &fresh).unwrap();
    // The accepted v3 now answers a later review based on v1.
    capture_page(&mut e, &page(ELIG, "배너 C", DEADLINE, FORM));
    let old_base = review_raw(&e, "v4", Some("v1"), ELIG, DEADLINE, FORM);
    assert_rejected(&mut e, &old_base, "낡았다");
}

#[test]
fn reverting_to_accepted_bytes_clears_the_pending_candidate() {
    let mut e = baseline("revert");
    capture_page(&mut e, &page(ELIG, "임시 배너", DEADLINE, FORM));
    let out = capture_page(&mut e, &page(ELIG, "공지", DEADLINE, FORM));
    assert_eq!(out["status"], "reverted_to_accepted");
    assert!(rules::approval_problems(&e).is_empty());
    let out = capture_page(&mut e, &page(ELIG, "공지", DEADLINE, FORM));
    assert_eq!(out["status"], "unchanged");
}

#[test]
fn uncited_new_source_is_an_unknown_change() {
    let mut e = baseline("uncited");
    let inbox = folder(&e).join("inbox");
    fs::write(inbox.join("guide.pdf"), b"%PDF-1.4 binary guide").unwrap();
    let out = rules::capture(
        &mut e,
        &json!({"sources": [
            {"id": "notice", "url": URL, "path": "inbox/notice.html"},
            {"id": "guide", "url": "https://organizer.example/guide.pdf", "path": "inbox/guide.pdf"}]}),
    )
    .unwrap();
    assert_eq!(out["diff"]["added"], json!(["guide"]));
    assert_eq!(out["sources"][1]["semantic"], "binary_unparsed");
    assert_eq!(out["sources"][1]["media_type"], "application/pdf");
    let raw = review_raw(&e, "v2", Some("v1"), ELIG, DEADLINE, FORM);
    assert_rejected(&mut e, &raw, "guide");

    // A PDF locator is recorded by page, never verified by parsing.
    let mut raw = raw;
    raw["rules"].as_array_mut().unwrap().push(json!({"category": "forms", "key": "guide", "value": "PDF 서식",
        "source": {"id": "guide", "sha256": sha(b"%PDF-1.4 binary guide"),
            "locator": {"page": 3, "quote": "파싱하지 않는 인용"}}}));
    let out = rules::review(&mut e, &raw).unwrap();
    assert_eq!(out["changed_categories"], json!(["forms"]));
    let accepted = &e.extra["rules"]["accepted"]["rules"];
    let pdf_rule = accepted.as_array().unwrap().iter().find(|r| r["source"]["id"] == "guide").unwrap();
    assert_eq!(pdf_rule["verification"], "binary_unverified");
}

#[test]
fn tampered_snapshot_blocks_review() {
    let mut e = baseline("tamper");
    let body = page(ELIG, "배너", DEADLINE, FORM);
    capture_page(&mut e, &body);
    let blob = folder(&e).join(".private/rules/sha256").join(sha(body.as_bytes()));
    fs::set_permissions(&blob, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&blob, "다른 내용").unwrap();
    let raw = review_raw(&e, "v2", Some("v1"), ELIG, DEADLINE, FORM);
    assert_rejected(&mut e, &raw, "변조");
}

/// Every rejected capture leaves the stored state exactly as it was.
fn assert_capture_rejected(e: &mut Entry, raw: &Value, limits: &Limits, needle: &str) {
    let before = e.extra.get("rules").cloned();
    let err = rules::capture_with(e, raw, limits).unwrap_err();
    assert!(err.0.contains(needle), "{needle}: {}", err.0);
    assert_eq!(e.extra.get("rules").cloned(), before);
}

#[test]
fn invalid_sources_are_refused() {
    let mut e = entry("bad-source");
    fs::write(folder(&e).join("a.txt"), "본문").unwrap();
    let lim = Limits::default();
    let src = |v: Value| json!({"sources": [v]});
    assert_capture_rejected(&mut e, &json!({"sources": []}), &lim, "sources");
    assert_capture_rejected(
        &mut e,
        &src(json!({"id": "a", "url": "ftp://x.example/a", "path": "a.txt"})),
        &lim,
        "http",
    );
    assert_capture_rejected(
        &mut e,
        &src(json!({"id": "a", "url": "https://user:pw@x.example/a", "path": "a.txt"})),
        &lim,
        "계정",
    );
    assert_capture_rejected(&mut e, &src(json!({"id": "../a", "url": URL, "path": "a.txt"})), &lim, "id");
    assert_capture_rejected(
        &mut e,
        &json!({"sources": [{"id": "a", "url": URL, "path": "a.txt"}, {"id": "a", "url": URL, "path": "a.txt"}]}),
        &lim,
        "같은 id",
    );
    assert_capture_rejected(
        &mut e,
        &src(json!({"id": "a", "url": URL, "path": "a.txt", "sha256": sha(b"other")})),
        &lim,
        "SHA-256",
    );
    assert_capture_rejected(&mut e, &src(json!({"id": "a", "url": URL, "path": "missing.txt"})), &lim, "열 수 없다");
    assert_capture_rejected(&mut e, &src(json!({"id": "a", "url": URL, "path": "."})), &lim, "일반 파일");
    let tiny = Limits { max_bytes: 3, ..Limits::default() };
    assert_capture_rejected(&mut e, &src(json!({"id": "a", "url": URL, "path": "a.txt"})), &tiny, "한도");

    let ok =
        rules::capture(&mut e, &src(json!({"id": "a", "url": URL, "path": "a.txt", "sha256": sha("본문".as_bytes())})))
            .unwrap();
    assert_eq!(ok["status"], "baseline_pending");
}

/// A one-shot HTTP server on 127.0.0.1 answering each connection with the next canned response.
fn serve(responses: Vec<Vec<u8>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for response in responses {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(&response);
        }
    });
    format!("http://{addr}")
}

fn response(status: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nLast-Modified: Sat, 24 Oct 2026 01:00:00 GMT\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

#[test]
fn http_fetch_captures_real_bytes_with_provenance() {
    let mut e = entry("http-ok");
    let body = page(ELIG, "공지", DEADLINE, FORM);
    let base = serve(vec![response("200 OK", "text/html; charset=utf-8", body.as_bytes())]);
    let url = format!("{base}/notice");
    let out = rules::capture(&mut e, &json!({"sources": [{"id": "notice", "url": url}]})).unwrap();
    let s = &out["sources"][0];
    assert_eq!(s["origin"], "http");
    assert_eq!(s["http_status"], 200);
    assert_eq!(s["sha256"], sha(body.as_bytes()).as_str());
    assert_eq!(s["media_type"], "text/html; charset=utf-8");
    assert_eq!(s["semantic"], "text");
    assert_eq!(s["last_modified"], "Sat, 24 Oct 2026 01:00:00 GMT");
    assert!(s["final_url"].as_str().unwrap().starts_with(&base));
    let blob = folder(&e).join(".private/rules/sha256").join(sha(body.as_bytes()));
    assert_eq!(fs::read(blob).unwrap(), body.as_bytes());
}

#[test]
fn http_failures_store_nothing() {
    let mut e = entry("http-fail");
    let lim = Limits { timeout: Duration::from_secs(2), ..Limits::default() };

    let base = serve(vec![response("404 Not Found", "text/html", b"missing")]);
    assert_capture_rejected(&mut e, &json!({"sources": [{"id": "n", "url": format!("{base}/n")}]}), &lim, "404");

    let base = serve(vec![response("200 OK", "text/plain", &[b'x'; 64])]);
    let small = Limits { max_bytes: 16, ..lim.clone() };
    assert_capture_rejected(&mut e, &json!({"sources": [{"id": "n", "url": format!("{base}/n")}]}), &small, "한도");

    // Chunked body without a length, larger than the limit.
    let mut chunked =
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
            .to_vec();
    chunked.extend_from_slice(b"40\r\n");
    chunked.extend_from_slice(&[b'y'; 64]);
    chunked.extend_from_slice(b"\r\n0\r\n\r\n");
    let base = serve(vec![chunked]);
    assert_capture_rejected(&mut e, &json!({"sources": [{"id": "n", "url": format!("{base}/n")}]}), &small, "한도");

    // Nothing listens on a port we just released.
    let closed = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    assert_capture_rejected(
        &mut e,
        &json!({"sources": [{"id": "n", "url": format!("http://{closed}/n")}]}),
        &lim,
        "가져오지 못했다",
    );

    // A server that accepts and never answers runs into the timeout.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let held = listener.accept();
        thread::sleep(Duration::from_secs(5));
        drop(held);
    });
    let quick = Limits { timeout: Duration::from_millis(500), ..lim };
    assert_capture_rejected(
        &mut e,
        &json!({"sources": [{"id": "n", "url": format!("http://{addr}/n")}]}),
        &quick,
        "n:",
    );
    assert!(e.extra.get("rules").is_none());
}

#[test]
fn acknowledge_requires_an_explicit_bound_review() {
    let mut e = baseline("ack");
    let gate = folder(&e).join("gate.json");
    fs::write(&gate, "{}").unwrap();
    let evidence = json!([{"path": "gate.json", "sha256": sha(b"{}")}]);
    let ack = |epoch: u64, rationale: &str, evidence: &Value| {
        json!({"rules_version": "v2", "epoch": epoch, "decision": "pass", "reviewer": "ContestGate",
            "rationale": rationale, "evidence": evidence})
    };
    let reason = "바뀐 자격 조건을 다시 대조해 통과로 판정했다";
    // Nothing invalidated yet.
    assert!(rules::acknowledge(&mut e, "gate", &ack(1, reason, &evidence)).is_err());

    let new_elig = "대한민국 대학원생 개인";
    capture_page(&mut e, &page(new_elig, "공지", DEADLINE, FORM));
    let raw = review_raw(&e, "v2", Some("v1"), new_elig, DEADLINE, FORM);
    rules::review(&mut e, &raw).unwrap();

    let before = e.extra["rules"].clone();
    assert!(rules::acknowledge(&mut e, "submission", &ack(2, reason, &evidence)).is_err());
    assert!(rules::acknowledge(&mut e, "gate", &ack(1, reason, &evidence)).is_err(), "stale epoch");
    assert!(rules::acknowledge(&mut e, "gate", &ack(2, "  ", &evidence)).is_err(), "blank rationale");
    assert!(rules::acknowledge(&mut e, "gate", &ack(2, "ok", &evidence)).is_err(), "token rationale");
    assert!(rules::acknowledge(&mut e, "gate", &ack(2, reason, &json!([]))).is_err(), "no evidence");
    let wrong = json!([{"path": "gate.json", "sha256": sha(b"other")}]);
    assert!(rules::acknowledge(&mut e, "gate", &ack(2, reason, &wrong)).is_err(), "evidence hash");
    let mut fail = ack(2, reason, &evidence);
    fail["decision"] = json!("fail");
    assert!(rules::acknowledge(&mut e, "gate", &fail).is_err());
    let mut version = ack(2, reason, &evidence);
    version["rules_version"] = json!("v1");
    assert!(rules::acknowledge(&mut e, "gate", &version).is_err());
    assert_eq!(e.extra["rules"], before);

    rules::acknowledge(&mut e, "gate", &ack(2, reason, &evidence)).unwrap();
    assert_eq!(rules::approval_problems(&e), vec!["rules_plan_invalidated", "rules_final_invalidated"]);
}

#[test]
fn plan_acknowledgement_carries_an_older_plan_score_forward() {
    let mut e = baseline("carry");
    push_score(&mut e, "plan");
    rules::score_recorded(&mut e, "plan").unwrap();
    let plan = e.scores[0].clone();
    let new_criteria = "창의성 40, 완성도 60";
    capture_page(&mut e, &page(ELIG, "공지", DEADLINE, FORM).replace("창의성 50, 완성도 50", new_criteria));
    let mut raw = review_raw(&e, "v2", Some("v1"), ELIG, DEADLINE, FORM);
    raw["rules"][3]["value"] = json!(new_criteria);
    raw["rules"][3]["source"]["locator"]["quote"] = json!(new_criteria);
    rules::review(&mut e, &raw).unwrap();
    assert!(!rules::check_score(&e, &plan, "plan"));

    fs::write(folder(&e).join("PLAN.md"), "계획").unwrap();
    let ack = json!({"rules_version": "v2", "epoch": 2, "decision": "pass", "reviewer": "ContestJudge",
        "rationale": "배점 변화에도 계획의 우선순위가 그대로 유효함을 확인했다",
        "evidence": [{"path": "PLAN.md", "sha256": sha("계획".as_bytes())}]});
    rules::acknowledge(&mut e, "plan", &ack).unwrap();
    assert!(rules::check_score(&e, &plan, "plan"));
    assert!(!rules::check_score(&e, &json!({"stage": "plan"}), "plan"), "an unstamped score never passes");
}

#[test]
fn score_hook_validates_before_mutating() {
    let mut e = baseline("hook");
    // No score at all, or the last score of another stage.
    assert!(rules::score_recorded(&mut e, "plan").is_err());
    push_score(&mut e, "draft");
    let before = (e.scores.clone(), e.extra["rules"].clone());
    assert!(rules::score_recorded(&mut e, "plan").is_err());
    assert!(rules::score_recorded(&mut e, "gate").is_err());
    assert_eq!((e.scores.clone(), e.extra["rules"].clone()), before);

    // A pending source review fails the hook without touching anything.
    capture_page(&mut e, &page(ELIG, "배너", DEADLINE, FORM));
    push_score(&mut e, "final");
    let before = (e.scores.clone(), e.extra["rules"].clone());
    assert!(rules::score_recorded(&mut e, "final").is_err());
    assert_eq!((e.scores.clone(), e.extra["rules"].clone()), before);
}

#[test]
fn untracked_score_hook_stamps_nothing() {
    let mut e = entry("hook-untracked");
    push_score(&mut e, "final");
    rules::score_recorded(&mut e, "final").unwrap();
    assert!(e.scores[0].get("rules_binding").is_none());
    assert!(e.extra.get("rules").is_none());
}

#[test]
fn malformed_state_is_reported_not_trusted() {
    let mut e = entry("malformed");
    e.extra.insert("rules".into(), json!({"schema": 1, "versions": "nope"}));
    assert_eq!(rules::approval_problems(&e), vec!["rules_state_invalid"]);
    assert_eq!(rules::status(&e)["problems"], json!(["rules_state_invalid"]));
    assert!(!rules::check_score(&e, &json!({}), "final"));
    let mut newer = entry("newer");
    newer.extra.insert("rules".into(), json!({"schema": 99}));
    assert!(rules::capture(&mut newer, &json!({"sources": [{"id": "a", "url": URL}]})).is_err());
}
