//! One contest through the whole lifecycle with the real `ach` binary: add, gate, route, a
//! headless session on a stand-in runner, approval, submission, cleanup. Offline: the runner is a
//! script on PATH, the remote is a local bare repository, and HOME, git config and ACH_HOME
//! are temporary.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const BIN: &str = env!("CARGO_BIN_EXE_ach");

struct World {
    base: PathBuf,
    root: PathBuf,
    home: PathBuf,
    path: String,
    gitconfig: PathBuf,
}

impl World {
    fn new(tag: &str) -> World {
        let base = std::env::temp_dir().join(format!("harness-e2e-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = base.join("checkout");
        std::fs::create_dir_all(root.join("data")).unwrap();
        for dir in ["harness", "agents", "skills"] {
            std::os::unix::fs::symlink(repo.join(dir), root.join(dir)).unwrap();
        }
        for example in ["config.example.json", "profile.example.md"] {
            std::fs::copy(repo.join("data").join(example), root.join("data").join(example)).unwrap();
        }

        // The stand-in runner: prints which contest session it is and the instructions it got.
        let bin = base.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let omp = bin.join("omp");
        std::fs::write(&omp, "#!/bin/sh\necho \"stand-in runner session=$ACH_SESSION args=$*\"\n").unwrap();
        std::fs::set_permissions(&omp, std::fs::Permissions::from_mode(0o755)).unwrap();

        let home = base.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let gitconfig = base.join("gitconfig");
        std::fs::write(&gitconfig, "[user]\n\tname = E2E\n\temail = e2e@example.com\n[init]\n\tdefaultBranch = main\n")
            .unwrap();
        let path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display());
        World { base, root, home, path, gitconfig }
    }

    fn cmd(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.env_clear()
            .env("PATH", &self.path)
            .env("HOME", &self.home)
            .env("ACH_HOME", &self.root)
            .env("GIT_CONFIG_GLOBAL", &self.gitconfig)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("LANG", "C.UTF-8");
        cmd
    }

    fn harness(&self, args: &[&str]) -> Output {
        self.cmd(BIN).args(args).output().unwrap()
    }

    /// Run `ach <args> --json`, require success, return the JSON.
    fn ok(&self, args: &[&str]) -> Value {
        let mut all = args.to_vec();
        all.push("--json");
        let out = self.harness(&all);
        assert!(
            out.status.success(),
            "harness {args:?} failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
    }

    fn git(&self, dir: &Path, args: &[&str]) {
        let out = self.cmd("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

#[test]
fn a_contest_goes_from_listing_to_cleaned_up_submission() {
    let w = World::new("lifecycle");

    // Setup the way INIT.md does it: scaffold, then set values through the validated CLI.
    let init = w.ok(&["init"]);
    assert!(init["created"].to_string().contains("config.json"), "{init}");
    let contests = w.base.join("contests");
    w.ok(&["config", "set", "contests_root", contests.to_str().unwrap()]);
    w.ok(&["config", "set", "workspace", "headless"]);
    w.ok(&["config", "set", "github_org", "\"\""]);
    assert_eq!(w.ok(&["config", "get", "workspace"]), "headless");
    assert!(w.harness(&["config", "set", "workspace", "screen"]).status.code() == Some(1), "invalid value refused");
    let agents = w.ok(&["agents"]);
    assert!(agents.as_array().is_some_and(|a| a.iter().any(|x| x["name"] == "ContestJudge")), "{agents}");

    // Add and gate.
    let added =
        w.ok(&["add", "--title", "E2E 수기 공모전", "--url", "https://example.com/e2e/1", "--deadline", "2099-12-31"]);
    let id = added["added"][0]["id"].as_str().or_else(|| added["id"].as_str()).expect("added id").to_string();
    let gate = json!({"decision": "pass", "type": "essay", "level": 1, "priority": 60, "folder": "E2E-Essay",
        "deliverables": [{"name": "수기", "format": "md", "limits": "A4 2쪽"}], "reasons": ["시험"]});
    let gate_file = w.base.join("gate.json");
    std::fs::write(&gate_file, gate.to_string()).unwrap();
    w.ok(&["gate", &id, "--file", gate_file.to_str().unwrap()]);

    // Route without GitHub or Orca, then give the folder a local remote.
    w.ok(&["route", &id, "--no-repo", "--no-workspace"]);
    let folder = w.base.join("contests/E2E-Essay");
    assert!(folder.join("CONTEST.md").is_file());
    let remote = w.base.join("remote.git");
    w.git(&w.base, &["init", "--bare", "-q", remote.to_str().unwrap()]);
    w.git(&folder, &["remote", "add", "origin", remote.to_str().unwrap()]);
    w.git(&folder, &["push", "-q", "-u", "origin", "main"]);

    // The stand-in runner allocates no agent resources. Capacity refusal is covered separately;
    // this lifecycle scenario must not depend on the host's current memory pressure.
    let launched = w.ok(&["launch", &id, "--force"]);
    assert_eq!(launched["workspace"], "headless");
    let mut tail = String::new();
    for _ in 0..100 {
        tail = String::from_utf8_lossy(&w.harness(&["peek", &id]).stdout).to_string();
        if tail.contains("stand-in runner") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(tail.contains(&format!("session={id}")), "{tail}");
    assert!(tail.contains("launch-prompt.md"), "{tail}");

    // The work: a deliverable, committed and pushed. Harness working files stay out of git.
    std::fs::write(folder.join("essay.md"), "# 수기\n본문\n").unwrap();
    w.git(&folder, &["add", "-A"]);
    w.git(&folder, &["commit", "-q", "-m", "Add essay"]);
    w.git(&folder, &["push", "-q"]);
    let essay_sha = format!("{:x}", Sha256::digest(std::fs::read(folder.join("essay.md")).unwrap()));
    let request = json!({
        "submission_ready": true,
        "needs_user_input": false,
        "blockers": [],
        "followup_required": false,
        "destination": "https://example.com/e2e/form",
        "artifacts": [{"path": "essay.md", "sha256": essay_sha}]
    });
    let request_file = w.base.join("readiness.json");
    std::fs::write(&request_file, request.to_string()).unwrap();
    w.ok(&["readiness", &id, "--file", request_file.to_str().unwrap()]);
    w.ok(&["transition", &id, "awaiting_approval", "--note", "수기 완성"]);

    // Submission and personal details need the user's approval, which a contest session cannot give.
    let token = w.ok(&["approval-status", &id])["request_token"].as_str().expect("request token").to_string();
    let intent = json!({"method": "form", "destination": "https://example.com/e2e/form", "expected_request": token});
    let intent_file = w.base.join("intent.json");
    std::fs::write(&intent_file, intent.to_string()).unwrap();
    let refused = w.harness(&["submission", "intent", &id, "--file", intent_file.to_str().unwrap()]);
    assert_eq!(refused.status.code(), Some(1), "no intent without an approval");
    std::fs::write(w.root.join("data/profile.md"), "## 참가 자격\n- 대학생\n\n## 제출용 개인정보\n- 이름: 시험\n")
        .unwrap();
    std::fs::set_permissions(w.root.join("data/profile.md"), std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(w.harness(&["profile", "show", "--for", &id]).status.code(), Some(1));
    let in_session = w.cmd(BIN).env("ACH_SESSION", &id).args(["approve", &id, "--note", "승인"]).output().unwrap();
    assert_eq!(in_session.status.code(), Some(1), "a contest session must not approve itself");
    w.ok(&["approve", &id, "--note", "제출해도 좋다"]);
    let profile = w.ok(&["profile", "show", "--for", &id]);
    assert!(profile.to_string().contains("이름: 시험"));

    // The transaction: intent, start (persisted before the agent sends), then the factual receipt.
    let made = w.ok(&["submission", "intent", &id, "--file", intent_file.to_str().unwrap()]);
    let attempt = made["attempt"]["attempt_id"].as_str().expect("attempt id").to_string();
    assert_eq!(made["created"], true, "{made}");
    let started = w.ok(&["submission", "start", &id, "--attempt", &attempt]);
    assert_eq!(started["status"], "dispatching", "{started}");
    let again = w.harness(&["submission", "start", &id, "--attempt", &attempt]);
    assert_eq!(again.status.code(), Some(1), "a second start never authorizes another send");
    // The stand-in for the organizer's receipt page the agent saved after sending (no real site).
    let private = folder.join(".private");
    std::fs::create_dir_all(&private).unwrap();
    std::fs::write(private.join("receipt.html"), "<p>접수 완료: R-1</p>\n").unwrap();
    let receipt_sha = format!("{:x}", Sha256::digest(std::fs::read(private.join("receipt.html")).unwrap()));
    let observed = w.ok(&["submission", "status", &id])["latest"]["started_at"].as_str().unwrap().to_string();
    let proof = json!({"attempt_id": attempt, "receipt": "R-1", "source": "confirmation_page", "observed_at": observed,
        "destination": "https://example.com/e2e/form", "submitted_sha256": [essay_sha],
        "artifacts": [{"path": ".private/receipt.html", "sha256": receipt_sha}]});
    let proof_file = w.base.join("proof.json");
    std::fs::write(&proof_file, proof.to_string()).unwrap();

    // Submit, then the ledger blocks the same contest from coming back.
    let confirmed = w.ok(&["submit", &id, "--attempt", &attempt, "--file", proof_file.to_str().unwrap()]);
    assert_eq!(confirmed["workflow_state"], "submitted", "{confirmed}");
    assert_eq!(confirmed["review_needed"], false, "{confirmed}");
    assert!(!confirmed.to_string().contains("R-1"), "the CLI result carries no receipt text: {confirmed}");
    let kept = folder.join(format!(".private/submissions/{attempt}/{receipt_sha}.html"));
    assert!(kept.is_file(), "a read-only copy of the proof is kept");
    assert_eq!(std::fs::metadata(&kept).unwrap().permissions().mode() & 0o777, 0o400);
    let ledger = w.ok(&["submitted"]);
    assert!(ledger.to_string().contains("E2E 수기 공모전"), "{ledger}");
    let again = w.harness(&["dedupe-check", "--id", &id]);
    assert_eq!(again.status.code(), Some(3), "a submitted contest is reported as a duplicate");

    // Cleanup: the session is closed, the folder leaves contests_root, the record stays.
    let dry = w.ok(&["clean", &id, "--dry-run"]);
    let dry = if dry.is_array() { dry[0].clone() } else { dry };
    assert_eq!(dry["ok"], true, "{dry}");
    w.ok(&["clean", &id]);
    assert!(!folder.exists());
    let shown = w.ok(&["show", &id]);
    assert_eq!(shown["state"], "submitted");
    assert!(shown["folder_removed_at"].is_string(), "{shown}");
    assert!(w.ok(&["submitted"]).to_string().contains("remote.git"), "the ledger keeps the remote for restoring");
}

#[test]
fn a_moved_checkout_is_refused_instead_of_starting_an_empty_registry() {
    let w = World::new("moved");
    let gone = w.base.join("moved-away");
    let out = w.cmd(BIN).env("ACH_HOME", &gone).args(["list"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(!gone.exists(), "nothing is created at a wrong root");
    // Doctor still runs and names the problem.
    let doctor = w.cmd(BIN).env("ACH_HOME", &gone).args(["doctor", "--json"]).output().unwrap();
    assert_eq!(doctor.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert_eq!(report["checks"][0]["name"], "checkout");
    assert_eq!(report["checks"][0]["level"], "fail");
}
