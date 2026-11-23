//! Delegated handoff (`ach handoff` and the hook after `transition`, `readiness`, `score`): with the
//! standing delegation on and an eligible request the harness records the delegated approval and
//! mails the recorded contest session exactly once, whatever the order and number of events; with
//! it off, or with anything unverified, it stops and says why. Offline fixtures only: `orca` is a
//! stub on PATH, no site is contacted and nothing is submitted.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use harness::registry::{self, Entry, NewEntry, apply_gate, new_entry, transition};
use harness::{readiness, rules};
use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_ach");
const HANDLE: &str = "term_11111111-2222-3333-4444-555555555555";
const WORKSPACE: &str = "repo::/contests::workspace:00000000-0000-0000-0000-000000000001";
const DESTINATION: &str = "https://official.example/apply";

/// `orca` stub. `terminal show` answers for a live omp terminal of [`WORKSPACE`] unless a marker
/// file next to it says otherwise; `orchestration send` logs one line per call to `sends.log` and
/// answers with a message id, a refusal (`SEND_REFUSE`) or nothing readable (`SEND_GARBLE`).
const ORCA: &str = r#"#!/bin/sh
here=$(dirname "$0")
case "$1 $2" in
  "terminal show")
    if [ -e "$here/GONE" ]; then echo '{"ok":false,"error":{"code":"terminal_handle_stale"}}'; exit 0; fi
    connected=true; [ -e "$here/DEAD" ] && connected=false
    agent=omp; [ -e "$here/OTHER_AGENT" ] && agent=codex
    workspace='repo::/contests::workspace:00000000-0000-0000-0000-000000000001'
    [ -e "$here/OTHER_WORKSPACE" ] && workspace='repo::/contests::workspace:00000000-0000-0000-0000-000000000002'
    printf '{"ok":true,"result":{"terminal":{"handle":"%s","connected":%s,"worktreeId":"%s","agentIdentity":"%s"}}}\n' "$4" "$connected" "$workspace" "$agent" ;;
  "orchestration send")
    [ -e "$here/SEND_SLOW" ] && sleep 1
    echo "to=$4 type=$6 subject=$8" >> "$here/sends.log"
    if [ -e "$here/SEND_REFUSE" ]; then echo '{"ok":false,"error":{"code":"runtime_unavailable","message":"busy"}}'; exit 0; fi
    if [ -e "$here/SEND_GARBLE" ]; then exit 0; fi
    n=$(wc -l < "$here/sends.log" | tr -d ' ')
    printf '{"ok":true,"result":{"messageId":"msg_%s"}}\n' "$n" ;;
  *) echo '{"ok":true,"result":{}}' ;;
esac
"#;

struct World {
    base: PathBuf,
    root: PathBuf,
    stubs: PathBuf,
    folder: PathBuf,
    id: String,
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn sha(path: &Path) -> String {
    readiness::sha256_file(path).unwrap()
}

impl World {
    /// A checkout with the delegation `enabled` or not and one level 1 contest in `running`: an
    /// Orca omp session recorded, accepted official rules, one deliverable.
    fn new(tag: &str, enabled: bool) -> World {
        let base = std::env::temp_dir().join(format!("harness-handoff-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = base.join("checkout");
        fs::create_dir_all(root.join("data")).unwrap();
        std::os::unix::fs::symlink(repo.join("harness"), root.join("harness")).unwrap();
        let stubs = base.join("bin");
        fs::create_dir_all(&stubs).unwrap();
        fs::write(stubs.join("orca"), ORCA).unwrap();
        fs::set_permissions(stubs.join("orca"), fs::Permissions::from_mode(0o755)).unwrap();
        // Canonical, as the CLI resolves artifact paths (macOS /private/var).
        let folder = base.join("contest");
        fs::create_dir_all(folder.join(".private")).unwrap();
        let folder = fs::canonicalize(&folder).unwrap();
        fs::write(folder.join("entry.md"), "응모작 본문\n").unwrap();

        let id = format!("h-{tag}");
        let mut e = new_entry(NewEntry {
            id: &id,
            title: "시험 공모전",
            url: "https://official.example/notice",
            source: "manual",
            organizer: "",
            deadline: Some("2099-12-31T23:59+09:00".into()),
            categories: vec![],
            note: None,
        });
        let gate = json!({"decision": "pass", "type": "essay", "level": 1, "priority": 50,
            "deliverables": [{"name": "글", "format": "md", "limits": ""}], "reasons": ["시험"]});
        apply_gate(&mut e, gate, "t").unwrap();
        transition(&mut e, "routed", "t", "").unwrap();
        transition(&mut e, "running", "t", "").unwrap();
        e.folder = Some(folder.to_string_lossy().into_owned());
        e.session = Some(json!({"workspace": "orca", "workspace_id": WORKSPACE, "runner": "omp", "handle": HANDLE}));
        accept_rules(&mut e, &folder);
        let world = World { base, root, stubs, folder, id };
        registry::write_at(&world.registry(), |r| {
            r.contests.insert(e.id.clone(), e);
            Ok(())
        })
        .unwrap();
        world.delegation(enabled);
        world
    }

    fn registry(&self) -> PathBuf {
        self.root.join("data/registry.json")
    }

    fn delegation(&self, enabled: bool) {
        let config = json!({"workspace": "orca", "delegation": {"enabled": enabled,
            "granted_at": "2026-10-20T09:00:00+09:00", "note": "시험용 상시 위임", "contest_ids": [],
            "allow_personalization": false, "video_score_exempt": false, "concept_selection": false}});
        fs::write(self.root.join("data/config.json"), serde_json::to_string_pretty(&config).unwrap()).unwrap();
    }

    fn entry(&self) -> Entry {
        registry::read_at(&self.registry(), |r| Ok(r.get(&self.id)?.clone())).unwrap()
    }

    fn edit(&self, f: impl FnOnce(&mut Entry)) {
        registry::write_at(&self.registry(), |r| {
            f(r.get_mut(&self.id)?);
            Ok(())
        })
        .unwrap();
    }

    fn command(&self, session: Option<&str>, args: &[&str]) -> Command {
        let mut cmd = Command::new(BIN);
        cmd.env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", self.stubs.display()))
            .env("HOME", self.base.join("home"))
            .env("ACH_HOME", &self.root)
            .env("LANG", "C.UTF-8")
            .args(args)
            .arg("--json");
        if let Some(session) = session {
            cmd.env("ACH_SESSION", session);
        }
        cmd
    }

    /// `ach <args> --json` as the contest session (`ACH_SESSION` = the contest id).
    fn session(&self, args: &[&str]) -> Output {
        let out = self.command(Some(&self.id), args).output().unwrap();
        assert!(out.status.success(), "ach {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        out
    }

    /// `ach <args> --json` as the main session; the JSON answer.
    fn main(&self, args: &[&str]) -> Value {
        let out = self.command(None, args).output().unwrap();
        assert!(out.status.success(), "ach {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice(&out.stdout).unwrap()
    }

    fn handoff(&self) -> Value {
        self.main(&["handoff", &self.id])
    }

    /// Record a readiness request for `entry.md` as it is now.
    fn ready(&self, overlay: Value) -> Output {
        let mut request = json!({"submission_ready": true, "needs_user_input": false, "blockers": [],
            "followup_required": false, "destination": DESTINATION,
            "artifacts": [{"path": "entry.md", "sha256": sha(&self.folder.join("entry.md"))}]});
        for (key, value) in overlay.as_object().unwrap() {
            request[key] = value.clone();
        }
        let file = self.folder.join(".private/readiness.json");
        fs::write(&file, request.to_string()).unwrap();
        self.session(&["readiness", &self.id, "--file", file.to_str().unwrap()])
    }

    fn await_approval(&self) -> Output {
        self.session(&["transition", &self.id, "awaiting_approval", "--note", "제출 준비 완료"])
    }

    fn mark(&self, name: &str, on: bool) {
        let path = self.stubs.join(name);
        if on {
            fs::write(path, "").unwrap();
        } else {
            let _ = fs::remove_file(path);
        }
    }

    fn sends(&self) -> Vec<String> {
        fs::read_to_string(self.stubs.join("sends.log")).unwrap_or_default().lines().map(str::to_string).collect()
    }

    fn submit_approvals(&self) -> usize {
        self.entry().approvals.iter().filter(|a| a.stage == "submit").count()
    }

    fn token(&self) -> String {
        self.main(&["approval-status", &self.id])["request_token"].as_str().unwrap().to_string()
    }

    fn journal(&self, token: &str) -> PathBuf {
        self.root.join("data/handoff").join(format!("{}--{token}.json", self.id))
    }

    fn journal_stage(&self, token: &str, stage: &str) {
        let path = self.journal(token);
        let mut record: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        record["stage"] = json!(stage);
        fs::write(path, record.to_string()).unwrap();
    }
}

/// Capture a local notice and accept its structured review, so the rules are tracked.
fn accept_rules(e: &mut Entry, folder: &Path) {
    let inbox = folder.join("inbox");
    fs::create_dir_all(&inbox).unwrap();
    let body = "<html><body><h2>참가 자격</h2><p>누구나</p><h2>마감</h2><p>2099-12-31 23:59</p><h2>서식</h2>\
        <p>자유 양식</p><h2>심사기준</h2><p>완성도 100</p><h2>AI</h2><p>표기 시 허용</p><h2>접수</h2>\
        <p>공식 접수 페이지</p></body></html>";
    fs::write(inbox.join("notice.html"), body).unwrap();
    let url = "https://official.example/notice";
    rules::capture(e, &json!({"sources": [{"id": "notice", "url": url, "path": "inbox/notice.html"}]})).unwrap();
    let digest = sha(&inbox.join("notice.html"));
    let rule = |category: &str, value: &str, quote: &str| {
        json!({"category": category, "value": value,
            "source": {"id": "notice", "sha256": digest, "locator": {"position": "본문", "quote": quote}}})
    };
    let review = json!({"version": "v1", "base_version": null, "decision": "accept", "reviewer": "ContestGate",
        "rules": [rule("eligibility", "누구나", "누구나"), rule("deadline", "2099-12-31T23:59+09:00", "2099-12-31 23:59"),
            rule("forms", "자유 양식", "자유 양식"), rule("criteria", "완성도 100", "완성도 100"),
            rule("ai_policy", "표기 조건부 허용", "표기 시 허용"), rule("submission", "공식 접수 페이지", "공식 접수 페이지")]});
    let out = rules::review(e, &review).unwrap();
    assert_eq!(out["status"], "accepted", "{out}");
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn reasons(result: &Value) -> Vec<&str> {
    result["reasons"].as_array().unwrap().iter().filter_map(Value::as_str).collect()
}

#[test]
fn readiness_then_transition_approves_and_hands_off_once_whatever_follows() {
    let w = World::new("order-a", true);
    let out = w.ready(json!({}));
    assert!(stderr(&out).is_empty(), "nothing to do while running: {}", stderr(&out));
    let out = w.await_approval();
    assert!(stderr(&out).contains("위임 인계[handed_off]"), "{}", stderr(&out));

    let e = w.entry();
    let token = w.token();
    let approval = e.approvals.last().unwrap();
    assert_eq!((approval.stage.as_str(), approval.by.as_str()), ("submit", "controller-delegated"));
    assert_eq!(approval.extra["delegated"], true);
    assert_eq!(approval.extra["request_token"], token.as_str());
    assert!(approval.note.contains("시험용 상시 위임"), "the grant is recorded with the approval");
    assert_eq!(w.main(&["approval-status", &w.id])["submit_approval_valid"], true);
    assert_eq!(w.sends(), vec![format!("to={HANDLE} type=handoff subject=제출 승인 기록됨: {} ({token})", w.id)]);
    let record: Value = serde_json::from_str(&fs::read_to_string(w.journal(&token)).unwrap()).unwrap();
    assert_eq!(record["stage"], "sent");
    assert_eq!(record["message_id"], "msg_1");
    assert_eq!(record["trigger"], "transition");
    assert_eq!(record["origin_session"], w.id.as_str());
    assert_eq!(record["target"]["handle"], HANDLE);
    assert_eq!(fs::metadata(w.journal(&token)).unwrap().permissions().mode() & 0o777, 0o600);

    // More events, a controller run and a sweep after it: nothing is approved or sent again.
    assert_eq!(w.handoff()["status"], "already_handed_off");
    assert_eq!(w.main(&["handoff"])["results"][0]["status"], "already_handed_off");
    assert_eq!(w.main(&["handoff", &w.id, "--dry-run"])["status"], "already_handed_off");
    assert_eq!((w.sends().len(), w.submit_approvals()), (1, 1));
}

#[test]
fn transition_then_readiness_waits_first_and_a_new_request_gets_its_own_handoff() {
    let w = World::new("order-b", true);
    let out = w.await_approval();
    assert!(stderr(&out).contains("위임 인계[waiting]"), "{}", stderr(&out));
    assert!(stderr(&out).contains("readiness_missing"), "{}", stderr(&out));
    assert!(stderr(&out).contains("사용자 승인 요청이 아니다"), "{}", stderr(&out));
    assert_eq!((w.sends().len(), w.submit_approvals()), (0, 0));

    // A request that is not ready yet waits on its own reasons, not on the user's approval.
    let out = w.ready(json!({"submission_ready": false, "blockers": ["접수 개시 전"]}));
    assert!(stderr(&out).contains("위임 인계[waiting]"), "{}", stderr(&out));
    let waiting = w.handoff();
    assert_eq!(waiting["user_approval_request"], false);
    assert!(reasons(&waiting).contains(&"blockers_present"), "{waiting}");
    assert_eq!((w.sends().len(), w.submit_approvals()), (0, 0));

    let out = w.ready(json!({}));
    assert!(stderr(&out).contains("위임 인계[handed_off]"), "{}", stderr(&out));
    let first = w.token();
    assert_eq!((w.sends().len(), w.submit_approvals()), (1, 1));

    // The session records the request again: the first approval no longer counts, and the new
    // request is approved and handed off once under its own token.
    let out = w.ready(json!({}));
    assert!(stderr(&out).contains("위임 인계[handed_off]"), "{}", stderr(&out));
    let second = w.token();
    assert_ne!(first, second);
    assert_eq!((w.sends().len(), w.submit_approvals()), (2, 2));
    assert!(w.sends()[1].contains(&second), "{:?}", w.sends());
    assert_eq!(w.handoff()["status"], "already_handed_off");
    assert_eq!(w.sends().len(), 2);
}

#[test]
fn a_disabled_delegation_stops_everything_and_asks_for_the_user() {
    let w = World::new("off", false);
    w.ready(json!({}));
    let out = w.await_approval();
    assert!(stderr(&out).contains("위임 인계[delegation_off]"), "{}", stderr(&out));
    assert!(stderr(&out).contains("사용자 승인 요청 대상이다"), "{}", stderr(&out));
    let off = w.handoff();
    assert_eq!((off["status"].as_str(), off["user_approval_request"].as_bool()), (Some("delegation_off"), Some(true)));
    assert_eq!((w.sends().len(), w.submit_approvals()), (0, 0));
    assert!(!w.root.join("data/handoff").exists(), "nothing is claimed while the delegation is off");

    // Turned on, the approval is recorded but Orca refuses the message. Turned off again, the
    // pending handoff stops too; turned on once more it is sent, with no second approval.
    w.delegation(true);
    w.mark("SEND_REFUSE", true);
    let failed = w.handoff();
    assert_eq!(failed["status"], "send_failed");
    assert_eq!(reasons(&failed), vec!["runtime_unavailable"]);
    assert_eq!(w.submit_approvals(), 1);
    w.mark("SEND_REFUSE", false);
    w.delegation(false);
    assert_eq!(w.handoff()["status"], "delegation_off");
    assert_eq!(w.sends().len(), 1, "only the refused call");
    w.delegation(true);
    let sent = w.handoff();
    assert_eq!(sent["status"], "handed_off");
    assert_eq!(sent["record"]["send_attempts"], 2);
    assert_eq!((w.sends().len(), w.submit_approvals()), (2, 1));
}

#[test]
fn a_delegation_scoped_to_other_contests_is_out_of_scope() {
    let w = World::new("scope", true);
    let mut config: Value =
        serde_json::from_str(&fs::read_to_string(w.root.join("data/config.json")).unwrap()).unwrap();
    config["delegation"]["contest_ids"] = json!(["another-contest"]);
    fs::write(w.root.join("data/config.json"), config.to_string()).unwrap();
    w.ready(json!({}));
    w.await_approval();
    let out = w.handoff();
    assert_eq!(out["status"], "out_of_scope");
    assert_eq!(out["user_approval_request"], true);
    assert_eq!((w.sends().len(), w.submit_approvals()), (0, 0));
}

#[test]
fn questions_blockers_changed_files_and_an_expired_deadline_wait_without_approval() {
    let w = World::new("ineligible", true);
    w.ready(json!({"needs_user_input": true}));
    let out = w.await_approval();
    assert!(stderr(&out).contains("사용자 입력(질문·조치)이 먼저 필요하다"), "{}", stderr(&out));
    let question = w.handoff();
    assert_eq!(question["status"], "waiting");
    assert!(reasons(&question).contains(&"needs_user_input"), "{question}");
    assert_eq!(question["user_approval_request"], false);

    w.ready(json!({}));
    assert_eq!(w.submit_approvals(), 1);
    // The reviewed file changes after the handoff: the approval stops counting and nothing is resent.
    fs::write(w.folder.join("entry.md"), "바뀐 본문\n").unwrap();
    let changed = w.handoff();
    assert_eq!(changed["status"], "waiting");
    assert!(reasons(&changed).contains(&"artifact_changed"), "{changed}");
    assert_eq!(w.main(&["approval-status", &w.id])["submit_approval_valid"], false);
    assert_eq!(w.sends().len(), 1);

    // A new request after the deadline is not approved.
    w.edit(|e| e.deadline = Some("2020-01-01T23:59+09:00".into()));
    w.ready(json!({}));
    let late = w.handoff();
    assert_eq!((late["status"].as_str(), reasons(&late)), (Some("waiting"), vec!["deadline_passed"]));
    assert_eq!((w.sends().len(), w.submit_approvals()), (1, 1));
}

#[test]
fn an_approval_recorded_by_someone_else_and_any_submission_attempt_are_left_alone() {
    let w = World::new("attempts", false);
    w.ready(json!({}));
    w.await_approval();
    w.main(&["approve", &w.id, "--note", "제출해 11/24 10:00"]);
    w.delegation(true);
    assert_eq!(w.handoff()["status"], "approved_elsewhere");

    let intent = w.folder.join(".private/intent.json");
    let request = json!({"method": "form", "destination": DESTINATION, "expected_request": w.token()});
    fs::write(&intent, request.to_string()).unwrap();
    w.session(&["submission", "intent", &w.id, "--file", intent.to_str().unwrap()]);
    let pending = w.handoff();
    assert_eq!(pending["status"], "submission_exists");
    assert!(reasons(&pending).contains(&"submission_intent_pending"), "{pending}");
    w.session(&["submission", "start", &w.id, "--attempt", "a1"]);
    assert!(reasons(&w.handoff()).contains(&"submission_dispatching"));
    let unknown = w.folder.join(".private/unknown.json");
    let raw = json!({"attempt_id": "a1", "reason": "timeout", "observed_at": harness::text::now(), "evidence": []});
    fs::write(&unknown, raw.to_string()).unwrap();
    w.session(&["submission", "unknown", &w.id, "--attempt", "a1", "--file", unknown.to_str().unwrap()]);
    let lost = w.handoff();
    assert_eq!(lost["status"], "submission_exists");
    assert!(reasons(&lost).contains(&"submission_unknown"), "{lost}");
    assert_eq!(lost["user_approval_request"], false);
    assert_eq!((w.sends().len(), w.submit_approvals()), (0, 1));
    assert!(!w.root.join("data/handoff").exists());
    // An attempt whose outcome is unknown also keeps the folder: cleaning is refused.
    let clean = w.command(None, &["clean", &w.id, "--dry-run"]).output().unwrap();
    assert!(!clean.status.success(), "{}", String::from_utf8_lossy(&clean.stdout));
}

#[test]
fn an_attempt_of_this_work_revision_or_a_recorded_submission_blocks_a_new_handoff() {
    let w = World::new("resolved", true);
    w.ready(json!({}));
    w.await_approval();
    assert_eq!(w.sends().len(), 1);
    // The session started and then resolved an attempt (verified not submitted), and records the
    // request again: the work already reached the submission step, so no new instruction goes out.
    let revision = registry::revision(&w.entry());
    w.edit(|e| {
        e.extra.insert(
            "submission_txn".into(),
            json!({"next_attempt": 2, "attempts": [{"attempt_id": "a1", "status": "verified_not_submitted",
                "method": "form", "authorization": {"revision": revision}}]}),
        );
    });
    w.ready(json!({}));
    let again = w.handoff();
    assert_eq!(
        (again["status"].as_str(), reasons(&again)),
        (Some("submission_exists"), vec!["submission_attempt_exists"])
    );

    w.edit(|e| {
        e.extra.remove("submission_txn");
        e.submissions.push(json!({"at": "2026-11-24T10:00+09:00", "method": "form"}));
    });
    let submitted = w.handoff();
    assert_eq!(
        (submitted["status"].as_str(), reasons(&submitted)),
        (Some("submission_exists"), vec!["already_submitted"])
    );
    // A submission of the same contest recorded outside the harness (the ledger) stops it as well.
    w.edit(|e| e.submissions.clear());
    let ledger = json!({"version": 1, "submissions": [{"key": "manual-other", "id": null, "title": "다른 제목",
        "url": "https://official.example/notice", "folder": null, "submitted_at": "2026-11-20", "source": "manual"}]});
    fs::write(w.root.join("data/submissions.json"), ledger.to_string()).unwrap();
    let recorded = w.handoff();
    assert_eq!(
        (recorded["status"].as_str(), reasons(&recorded)),
        (Some("submission_exists"), vec!["submission_ledger_match"])
    );
    assert_eq!((w.sends().len(), w.submit_approvals()), (1, 1));
}

#[test]
fn an_unreadable_send_outcome_is_never_resent_without_the_controller() {
    let w = World::new("unknown", true);
    w.ready(json!({}));
    w.mark("SEND_GARBLE", true);
    let out = w.await_approval();
    assert!(stderr(&out).contains("위임 인계[handoff_unknown]"), "{}", stderr(&out));
    w.mark("SEND_GARBLE", false);
    let token = w.token();
    for _ in 0..2 {
        let again = w.handoff();
        assert_eq!(
            (again["status"].as_str(), reasons(&again)),
            (Some("handoff_unknown"), vec!["send_outcome_unknown"])
        );
    }
    assert_eq!(w.main(&["handoff"])["results"][0]["status"], "handoff_unknown");
    assert_eq!((w.sends().len(), w.submit_approvals()), (1, 1));

    // Another request's token does not unlock it; the reviewed one does, once.
    assert_eq!(w.main(&["handoff", &w.id, "--resend-request", "r0-s0-n0"])["status"], "handoff_unknown");
    assert_eq!(w.main(&["handoff", &w.id, "--resend-request", &token])["status"], "handed_off");
    assert_eq!(w.handoff()["status"], "already_handed_off");
    assert_eq!((w.sends().len(), w.submit_approvals()), (2, 1));
}

#[test]
fn a_restart_resumes_what_was_decided_and_never_repeats_what_may_have_gone_out() {
    let w = World::new("restart", true);
    w.ready(json!({}));
    w.mark("SEND_REFUSE", true);
    w.await_approval();
    w.mark("SEND_REFUSE", false);
    let token = w.token();
    assert_eq!((w.sends().len(), w.submit_approvals()), (1, 1));

    // Killed after the approval was stored but before the journal moved on: the approval is ours
    // (delegated, same token), so only the message is sent.
    w.journal_stage(&token, "claimed");
    let resumed = w.handoff();
    assert_eq!(resumed["status"], "handed_off");
    assert_eq!((w.sends().len(), w.submit_approvals()), (2, 1));

    // Killed while Orca was being called: the message may have arrived.
    w.journal_stage(&token, "sending");
    let interrupted = w.handoff();
    assert_eq!(
        (interrupted["status"].as_str(), reasons(&interrupted)),
        (Some("handoff_unknown"), vec!["send_interrupted"])
    );
    assert_eq!(interrupted["record"]["stage"], "unknown");
    assert_eq!((w.sends().len(), w.submit_approvals()), (2, 1));

    // Refusals are retried by later runs only up to the limit.
    w.journal_stage(&token, "approved");
    w.mark("SEND_REFUSE", true);
    let statuses: Vec<String> = (0..3).map(|_| w.handoff()["status"].as_str().unwrap().to_string()).collect();
    assert_eq!(statuses, ["send_failed", "send_failed", "send_failed"]);
    let sends = w.sends().len();
    assert_eq!(reasons(&w.handoff()), vec!["send_attempts_exhausted"]);
    assert_eq!(w.sends().len(), sends, "no call once the retries ran out");
}

#[test]
fn concurrent_events_approve_and_send_once() {
    let w = World::new("race", true);
    w.ready(json!({}));
    w.delegation(false);
    w.await_approval();
    w.delegation(true);
    w.mark("SEND_SLOW", true);
    let children: Vec<_> = (0..4)
        .map(|_| w.command(None, &["handoff", &w.id]).stdout(std::process::Stdio::null()).spawn().unwrap())
        .collect();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    assert_eq!((w.sends().len(), w.submit_approvals()), (1, 1));
    assert_eq!(w.handoff()["status"], "already_handed_off");
}

#[test]
fn only_a_verified_live_omp_session_of_the_recorded_workspace_is_approved_for() {
    let w = World::new("session", true);
    w.ready(json!({}));
    w.delegation(false);
    w.await_approval();
    w.delegation(true);
    for (marker, code) in [
        ("DEAD", "session_not_live"),
        ("GONE", "session_not_live"),
        ("OTHER_AGENT", "session_agent_unverified"),
        ("OTHER_WORKSPACE", "session_workspace_mismatch"),
    ] {
        w.mark(marker, true);
        let out = w.handoff();
        assert_eq!((out["status"].as_str(), reasons(&out)), (Some("session_unavailable"), vec![code]), "{marker}");
        w.mark(marker, false);
    }
    w.edit(|e| e.session.as_mut().unwrap()["runner"] = json!("claude"));
    assert_eq!(reasons(&w.handoff()), vec!["session_not_omp"]);
    w.edit(|e| e.session.as_mut().unwrap()["runner"] = json!("omp"));
    w.edit(|e| e.session.as_mut().unwrap()["workspace"] = json!("tmux"));
    assert_eq!(reasons(&w.handoff()), vec!["session_not_orca"]);
    w.edit(|e| e.session = None);
    assert_eq!(reasons(&w.handoff()), vec!["session_missing"]);
    assert_eq!((w.sends().len(), w.submit_approvals()), (0, 0));
    assert!(!w.root.join("data/handoff").exists(), "an unverified session is never approved for");

    w.edit(|e| {
        e.session = Some(json!({"workspace": "orca", "workspace_id": WORKSPACE, "runner": "omp", "handle": HANDLE}))
    });
    assert_eq!(w.main(&["handoff", &w.id, "--dry-run"])["status"], "would_approve_and_hand_off");
    assert_eq!((w.sends().len(), w.submit_approvals()), (0, 0));
    assert_eq!(w.handoff()["status"], "handed_off");
}

#[test]
fn contest_sessions_cannot_run_the_handoff_and_another_session_never_triggers_it() {
    let w = World::new("guard", true);
    w.ready(json!({}));
    w.delegation(false);
    w.await_approval();
    w.delegation(true);
    let refused = w.command(Some(&w.id), &["handoff", &w.id]).output().unwrap();
    assert!(!refused.status.success());
    assert!(stderr(&refused).contains("ACH_SESSION"), "{}", stderr(&refused));
    let dry = w.command(Some(&w.id), &["handoff", &w.id, "--dry-run"]).output().unwrap();
    assert!(dry.status.success(), "{}", stderr(&dry));
    let approve =
        w.command(Some(&w.id), &["approve", &w.id, "--delegated", "--expected-request", &w.token(), "--note", "x"]);
    let mut approve = approve;
    assert!(!approve.output().unwrap().status.success(), "ach approve stays refused inside a session");

    // Another contest's session records a score-free event for this one: no hook.
    let file = w.folder.join(".private/readiness.json");
    let out =
        w.command(Some("another-contest"), &["readiness", &w.id, "--file", file.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).is_empty(), "{}", stderr(&out));
    assert_eq!((w.sends().len(), w.submit_approvals()), (0, 0));
}

#[test]
fn a_score_gate_override_waits_for_the_user_not_for_the_delegation() {
    let w = World::new("override", true);
    w.ready(json!({}));
    w.edit(|e| e.level = Some(2));
    let out = w.session(&["transition", &w.id, "awaiting_approval", "--score-ok", "마감이 가까워 그대로 낸다"]);
    assert!(stderr(&out).contains("위임 인계[waiting]"), "{}", stderr(&out));
    assert!(stderr(&out).contains("사용자 승인 요청 대상이다"), "{}", stderr(&out));
    let waiting = w.handoff();
    assert_eq!(waiting["user_approval_request"], true);
    assert!(reasons(&waiting).contains(&"score_override_requires_user"), "{waiting}");
    assert_eq!((w.sends().len(), w.submit_approvals()), (0, 0));
    // The user's own approval is what the override path accepts; the handoff then leaves it alone.
    w.main(&["approve", &w.id, "--note", "그대로 제출해 11/24 10:00"]);
    let approved = w.handoff();
    assert_eq!(
        (approved["status"].as_str(), approved["user_approval_request"].as_bool()),
        (Some("approved_elsewhere"), Some(false))
    );
    assert_eq!(w.sends().len(), 0);
}
