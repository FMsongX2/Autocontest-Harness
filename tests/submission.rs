//! Submission transactions: intent, start before the external send, unknown outcomes, verified
//! non-submission, factual receipt confirmation, immutable proofs and the ledger. Offline fixtures
//! only: no site is contacted and nothing is sent.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use harness::ledger::{self, Ledger, Submission};
use harness::registry::{self, Entry, NewEntry, apply_gate, new_entry, transition};
use harness::{readiness, submission, text};
use serde_json::{Map, Value, json};

const DESTINATION: &str = "https://official.example/apply";

struct Fx {
    root: PathBuf,
    folder: PathBuf,
    reg: PathBuf,
    ledger: PathBuf,
    id: String,
}

impl Drop for Fx {
    fn drop(&mut self) {
        // Proof copies are read-only; make them removable before deleting the fixture.
        let kept = self.folder.join(".private/submissions");
        if let Ok(dirs) = fs::read_dir(&kept) {
            for dir in dirs.flatten() {
                for file in fs::read_dir(dir.path()).into_iter().flatten().flatten() {
                    let _ = fs::set_permissions(file.path(), fs::Permissions::from_mode(0o600));
                }
            }
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "harness-submission-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn sha(path: &Path) -> String {
    readiness::sha256_file(path).unwrap()
}

fn readiness_request(folder: &Path) -> Value {
    json!({"submission_ready": true, "needs_user_input": false, "blockers": [], "followup_required": false,
        "destination": DESTINATION, "artifacts": [{"path": "entry.md", "sha256": sha(&folder.join("entry.md"))}]})
}

/// A level 1 contest in `awaiting_approval` with a readiness record naming the destination and a
/// current human submission approval, stored in a temporary registry.
fn fixture(tag: &str) -> Fx {
    let root = temp_dir(tag);
    let folder = root.join("contest");
    fs::create_dir_all(folder.join(".private")).unwrap();
    fs::write(folder.join("entry.md"), "응모작 본문\n").unwrap();
    let id = format!("s-{tag}");
    let mut e = new_entry(NewEntry {
        id: &id,
        title: "시험 공모전",
        url: "https://official.example/notice",
        source: "manual",
        organizer: "",
        deadline: None,
        categories: vec![],
        note: None,
    });
    let gate = json!({"decision": "pass", "type": "essay", "level": 1, "priority": 50,
        "deliverables": [{"name": "글", "format": "md", "limits": ""}], "reasons": ["시험"]});
    apply_gate(&mut e, gate, "t").unwrap();
    transition(&mut e, "routed", "t", "").unwrap();
    transition(&mut e, "running", "t", "").unwrap();
    e.folder = Some(folder.to_string_lossy().into_owned());
    readiness::record(&mut e, &readiness_request(&folder)).unwrap();
    transition(&mut e, "awaiting_approval", "t", "").unwrap();
    registry::approve(&mut e, "제출해 10/25 10:00", "user", None).unwrap();
    let reg = root.join("data/registry.json");
    registry::write_at(&reg, |r| {
        r.contests.insert(e.id.clone(), e);
        Ok(())
    })
    .unwrap();
    Fx { ledger: root.join("data/submissions.json"), root, folder, reg, id }
}

impl Fx {
    fn entry(&self) -> Entry {
        registry::read_at(&self.reg, |r| Ok(r.get(&self.id)?.clone())).unwrap()
    }

    /// Run `f` on the stored entry under the registry lock (saved only on success), as the CLI does.
    fn with<T>(&self, f: impl FnOnce(&mut Entry) -> harness::error::Result<T>) -> harness::error::Result<T> {
        registry::write_at(&self.reg, |r| f(r.get_mut(&self.id)?))
    }

    fn token(&self) -> String {
        readiness::status(&self.entry(), false).unwrap()["request_token"].as_str().unwrap().to_string()
    }

    fn intent(&self) -> harness::error::Result<Value> {
        let raw = json!({"method": "form", "destination": DESTINATION, "expected_request": self.token()});
        self.with(|e| submission::intent(e, &raw))
    }

    fn attempt(&self, attempt: &str) -> Value {
        let e = self.entry();
        let list = e.extra["submission_txn"]["attempts"].as_array().unwrap().clone();
        list.into_iter().find(|a| a["attempt_id"] == attempt).unwrap()
    }

    /// Write an evidence file under `.private/` and return its `{path, sha256}`.
    fn evidence(&self, name: &str, body: &str) -> Value {
        let path = self.folder.join(".private").join(name);
        fs::write(&path, body).unwrap();
        json!({"path": format!(".private/{name}"), "sha256": sha(&path)})
    }

    fn proof(&self, attempt: &str, receipt: &str) -> Value {
        json!({
            "attempt_id": attempt,
            "receipt": receipt,
            "source": "receipt_email",
            "observed_at": text::now(),
            "destination": DESTINATION,
            "submitted_sha256": [sha(&self.folder.join("entry.md"))],
            "artifacts": [self.evidence(&format!("receipt-{attempt}.eml"), &format!("접수되었습니다 {attempt}\n"))],
        })
    }

    fn confirm(&self, attempt: &str, raw: &Value) -> harness::error::Result<Value> {
        submission::confirm_at(&self.reg, &self.ledger, &self.id, attempt, raw)
    }

    fn bytes(&self) -> (Vec<u8>, Option<Vec<u8>>) {
        (fs::read(&self.reg).unwrap(), fs::read(&self.ledger).ok())
    }
}

#[test]
fn a_crash_before_dispatch_repeats_the_same_intent_and_start_checks_the_approval_again() {
    let fx = fixture("before");
    let first = fx.intent().unwrap();
    assert_eq!(first["created"], true, "{first}");
    let attempt = first["attempt"]["attempt_id"].as_str().unwrap().to_string();
    assert_eq!(attempt, "a1");
    assert_eq!(fx.attempt(&attempt)["status"], "intent");
    assert!(submission::unresolved(&fx.entry()));
    assert!(!submission::in_flight(&fx.entry()), "an intent sends nothing");

    // The process died before `start`: the same request again returns the same attempt.
    let again = fx.intent().unwrap();
    assert_eq!((again["created"].clone(), again["attempt"]["attempt_id"].clone()), (json!(false), json!("a1")));
    let other = json!({"method": "email", "destination": "apply@official.example", "expected_request": fx.token()});
    assert!(fx.with(|e| submission::intent(e, &other)).is_err(), "another destination than the approved one");

    // A file changed after the intent: start refuses (and the approval is void anyway).
    let entry_md = fx.folder.join("entry.md");
    fs::write(&entry_md, "몰래 바뀐 본문\n").unwrap();
    assert!(fx.with(|e| submission::start(e, &attempt)).is_err());
    assert_eq!(fx.attempt(&attempt)["status"], "intent", "a refused start changes nothing");
    fs::write(&entry_md, "응모작 본문\n").unwrap();

    let started = fx.with(|e| submission::start(e, &attempt)).unwrap();
    assert_eq!(started["status"], "dispatching");
    assert!(started["started_at"].is_string());
    assert!(submission::in_flight(&fx.entry()));
    let status = submission::status(&fx.entry());
    assert_eq!(status["state"], "dispatching", "{status}");
    assert_eq!(status["blockers"], json!(["submission_dispatching"]));
}

#[test]
fn a_crash_after_dispatch_never_sends_again_until_evidence_resolves_it() {
    let fx = fixture("after");
    fx.intent().unwrap();
    fx.with(|e| submission::start(e, "a1")).unwrap();
    // The process died after the send began: no second start, no new intent.
    let err = fx.with(|e| submission::start(e, "a1")).unwrap_err();
    assert!(err.0.contains("다시 보내지 않는다"), "{err}");
    assert!(fx.intent().is_err());

    // Response lost: recorded as unknown, still no resend.
    let unknown = json!({"attempt_id": "a1", "reason": "crash", "observed_at": text::now()});
    assert!(
        fx.with(|e| submission::unknown(
            e,
            "a1",
            &json!({"attempt_id": "a1", "reason": "guess", "observed_at": text::now()})
        ))
        .is_err()
    );
    let recorded = fx.with(|e| submission::unknown(e, "a1", &unknown)).unwrap();
    assert_eq!(recorded["status"], "unknown");
    let before = fx.bytes();
    fx.with(|e| submission::unknown(e, "a1", &unknown)).unwrap();
    assert_eq!(fx.bytes(), before, "the same unknown record again changes nothing");
    assert!(fx.with(|e| submission::start(e, "a1")).is_err());
    assert!(fx.intent().is_err());
    assert_eq!(submission::status(&fx.entry())["state"], "unknown");

    // Only positive evidence, observed after the send began, closes the attempt.
    let listing = fx.evidence("listing.png", "내 접수 내역: 없음");
    let resolve = |observed: &str, verified: bool, artifacts: Value| {
        json!({"attempt_id": "a1", "verified_not_submitted": verified, "source": "portal_listing",
            "observed_at": observed, "artifacts": artifacts})
    };
    for (what, raw) in [
        ("not asserted", resolve(&text::now(), false, json!([listing]))),
        ("older than the send", resolve("2020-01-01T09:00+09:00", true, json!([listing]))),
        ("no evidence", resolve(&text::now(), true, json!([]))),
        (
            "forged hash",
            resolve(&text::now(), true, json!([{"path": ".private/listing.png", "sha256": "0".repeat(64)}])),
        ),
    ] {
        assert!(fx.with(|e| submission::resolve(e, "a1", &raw)).is_err(), "{what}");
    }
    assert_eq!(fx.attempt("a1")["status"], "unknown");
    let resolved = fx.with(|e| submission::resolve(e, "a1", &resolve(&text::now(), true, json!([listing])))).unwrap();
    assert_eq!(resolved["status"], "verified_not_submitted");
    assert!(!submission::unresolved(&fx.entry()));

    // Now a new attempt may be made and started.
    let next = fx.intent().unwrap();
    assert_eq!(next["attempt"]["attempt_id"], "a2");
    fx.with(|e| submission::start(e, "a2")).unwrap();
}

#[test]
fn forged_blank_and_mismatched_proofs_are_refused_without_any_change() {
    let fx = fixture("forged");
    fx.intent().unwrap();
    // An intent that never started is never a submission.
    assert!(fx.confirm("a1", &fx.proof("a1", "R-1")).unwrap_err().0.contains("submission start"));
    fx.with(|e| submission::start(e, "a1")).unwrap();
    let blank = fx.evidence("blank.eml", "");
    let copy = fx.evidence("copy.md", "응모작 본문\n");
    fs::write(fx.folder.join("outside.eml"), "접수").unwrap();
    let outside = json!({"path": "outside.eml", "sha256": sha(&fx.folder.join("outside.eml"))});
    let mut cases: Vec<(&str, Value)> = Vec::new();
    let mut case = |what: &'static str, edit: &dyn Fn(&mut Value)| {
        let mut p = fx.proof("a1", "R-1");
        edit(&mut p);
        cases.push((what, p));
    };
    case("blank receipt", &|p| p["receipt"] = json!(" \n "));
    case("receipt too long", &|p| p["receipt"] = json!("가".repeat(201)));
    case("unknown source", &|p| p["source"] = json!("trust_me"));
    case("other attempt", &|p| p["attempt_id"] = json!("a2"));
    case("other destination", &|p| p["destination"] = json!("https://elsewhere.example/apply"));
    case("other files", &|p| p["submitted_sha256"] = json!(["f".repeat(64)]));
    case("missing files", &|p| p["submitted_sha256"] = json!([]));
    case("date only", &|p| p["observed_at"] = json!("2026-10-25"));
    case("future", &|p| p["observed_at"] = json!("2999-01-01T09:00+09:00"));
    case("before the send", &|p| p["observed_at"] = json!("2020-01-01T09:00+09:00"));
    case("no proof files", &|p| p["artifacts"] = json!([]));
    case("forged proof hash", &|p| p["artifacts"][0]["sha256"] = json!("0".repeat(64)));
    case("blank proof file", &|p| p["artifacts"] = json!([blank.clone()]));
    case("submitted file as receipt", &|p| p["artifacts"] = json!([copy.clone()]));
    case("proof outside .private", &|p| p["artifacts"] = json!([outside.clone()]));
    let before = fx.bytes();
    for (what, raw) in cases {
        assert!(fx.confirm("a1", &raw).is_err(), "{what}");
        assert_eq!(fx.bytes(), before, "{what} changed nothing");
    }
    assert_eq!(fx.entry().state, "awaiting_approval");
    assert!(fx.entry().submissions.is_empty());
}

#[test]
fn a_confirmed_receipt_is_recorded_once_with_an_immutable_proof_and_minimal_metadata() {
    let fx = fixture("confirm");
    fx.intent().unwrap();
    fx.with(|e| submission::start(e, "a1")).unwrap();
    let proof = fx.proof("a1", "접수번호 2026-777 $(id)");
    let out = fx.confirm("a1", &proof).unwrap();
    assert_eq!((out["workflow_state"].as_str(), out["review_needed"].as_bool()), (Some("submitted"), Some(false)));
    assert!(!out.to_string().contains("2026-777"), "no receipt text in the result: {out}");
    let e = fx.entry();
    assert_eq!(e.state, "submitted");
    assert_eq!(e.submissions.len(), 1);
    assert_eq!(e.submissions[0]["receipt"], "접수번호 2026-777 $(id)", "kept verbatim as data");
    assert_eq!(e.submissions[0]["files"], json!(["entry.md"]));
    let status = submission::status(&e);
    assert_eq!(status["state"], "confirmed");
    assert!(!status.to_string().contains("2026-777") && !status.to_string().contains(DESTINATION), "{status}");

    // The proof is copied read-only, content-addressed, and its source can change afterwards.
    let hash = proof["artifacts"][0]["sha256"].as_str().unwrap();
    let kept = fx.folder.join(format!(".private/submissions/a1/{hash}.eml"));
    assert_eq!(sha(&kept), hash);
    assert_eq!(fs::metadata(&kept).unwrap().permissions().mode() & 0o777, 0o400);

    let item = ledger::load_at(&fx.ledger).unwrap().get(&fx.id).cloned().unwrap();
    assert_eq!((item.source.as_str(), item.receipt.as_str()), ("harness", "접수번호 2026-777 $(id)"));
    assert_eq!(item.extra["attempt_id"], "a1");
    assert_eq!(item.extra["proof_sha256"], json!([hash]));

    // Exactly the same proof again: byte-for-byte nothing changes.
    let before = fx.bytes();
    let repeat = fx.confirm("a1", &proof).unwrap();
    assert_eq!(repeat["repeated"], true);
    assert_eq!(fx.bytes(), before);
    // A conflicting proof for the same attempt is refused.
    let mut other = proof.clone();
    other["receipt"] = json!("다른 번호");
    assert!(fx.confirm("a1", &other).is_err());
    assert_eq!(fx.bytes(), before);
    // A repeat neither rereads nor rewrites the kept copy (tampering stays visible, not repaired).
    fs::set_permissions(&kept, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&kept, "변조").unwrap();
    fs::set_permissions(&kept, fs::Permissions::from_mode(0o400)).unwrap();
    assert_eq!(fx.confirm("a1", &proof).unwrap()["repeated"], true);
    assert_eq!(fs::read_to_string(&kept).unwrap(), "변조");

    // A lost ledger item is restored with the original time.
    fs::remove_file(&fx.ledger).unwrap();
    fx.confirm("a1", &proof).unwrap();
    let restored = ledger::load_at(&fx.ledger).unwrap().get(&fx.id).cloned().unwrap();
    assert_eq!(restored, item);
}

#[test]
fn a_receipt_found_after_the_work_moved_on_is_recorded_without_overwriting_the_new_work() {
    let fx = fixture("recover");
    fx.intent().unwrap();
    fx.with(|e| submission::start(e, "a1")).unwrap();
    fx.with(|e| {
        submission::unknown(
            e,
            "a1",
            &json!({"attempt_id": "a1", "reason": "response_lost", "observed_at": text::now()}),
        )
    })
    .unwrap();
    let empty = fx.evidence("search.txt", "메일함 검색 결과 없음");
    let raw = json!({"attempt_id": "a1", "verified_not_submitted": true, "source": "mailbox_search",
        "observed_at": text::now(), "artifacts": [empty]});
    fx.with(|e| submission::resolve(e, "a1", &raw)).unwrap();
    // The work is revised: a new revision is running.
    fx.with(|e| transition(e, "running", "t", "수정")).unwrap();
    let revision = registry::revision(&fx.entry());

    // The organizer's receipt for a1 arrives late: the fact is recorded, the new work stays.
    let out = fx.confirm("a1", &fx.proof("a1", "R-late")).unwrap();
    assert_eq!(out["review_needed"], true, "{out}");
    assert_eq!(out["workflow_state"], "running");
    let reasons = out["attempt"]["review_reasons"].clone();
    assert!(reasons.to_string().contains("workflow_changed"), "{reasons}");
    assert!(reasons.to_string().contains("contradicts_verified_not_submitted"), "{reasons}");
    let e = fx.entry();
    assert_eq!((e.state.as_str(), registry::revision(&e)), ("running", revision));
    assert_eq!(e.submissions.len(), 1);
    assert_eq!(submission::status(&e)["review_needed"], true);
    assert_eq!(ledger::load_at(&fx.ledger).unwrap().get(&fx.id).unwrap().extra["attempt_id"], "a1");
}

#[test]
fn an_existing_ledger_item_keeps_its_fields_and_gets_the_new_round() {
    let fx = fixture("rounds");
    let mut extra = Map::new();
    extra.insert("repo".into(), json!("git@example.com:owner/old.git"));
    extra.insert("owner_note".into(), json!("1차 접수"));
    let earlier = Submission {
        key: fx.id.clone(),
        id: Some(fx.id.clone()),
        title: "시험 공모전".into(),
        organizer: String::new(),
        url: "https://official.example/notice".into(),
        alt_urls: vec![],
        folder: None,
        submitted_at: "2026-09-01".into(),
        method: "메일".into(),
        receipt: "1차".into(),
        files: vec!["old.pdf".into()],
        note: String::new(),
        source: "import".into(),
        extra,
    };
    ledger::write_at(&fx.ledger, |l: &mut Ledger| {
        l.upsert(earlier.clone());
        l.extra.insert("owner".into(), json!("kept"));
        Ok(())
    })
    .unwrap();
    fx.intent().unwrap();
    fx.with(|e| submission::start(e, "a1")).unwrap();
    let proof = fx.proof("a1", "R-2");
    fx.confirm("a1", &proof).unwrap();
    let l = ledger::load_at(&fx.ledger).unwrap();
    assert_eq!(l.extra["owner"], "kept");
    assert_eq!(l.submissions.len(), 1);
    let item = l.get(&fx.id).unwrap();
    assert_eq!(
        (item.receipt.as_str(), item.submitted_at.as_str(), item.source.as_str()),
        ("1차", "2026-09-01", "import")
    );
    assert_eq!(item.extra["repo"], "git@example.com:owner/old.git");
    assert_eq!(item.extra["owner_note"], "1차 접수");
    let rounds = item.extra["rounds"].as_array().unwrap();
    assert_eq!(rounds.len(), 1);
    assert_eq!((rounds[0]["attempt_id"].as_str(), rounds[0]["receipt"].as_str()), (Some("a1"), Some("R-2")));
    let before = fx.bytes();
    fx.confirm("a1", &proof).unwrap();
    assert_eq!(fx.bytes(), before, "the round is not appended twice");
}

#[test]
fn an_unreadable_transaction_record_is_never_overwritten() {
    let fx = fixture("malformed");
    fx.with(|e| {
        e.extra.insert("submission_txn".into(), json!("손으로 고친 값"));
        Ok(())
    })
    .unwrap();
    let e = fx.entry();
    assert!(submission::unresolved(&e) && submission::in_flight(&e));
    assert_eq!(submission::status(&e)["state"], "malformed");
    assert!(fx.intent().is_err());
    assert_eq!(fx.entry().extra["submission_txn"], "손으로 고친 값");
}
