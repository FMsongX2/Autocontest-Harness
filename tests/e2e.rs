//! One contest through the whole lifecycle with the real `harness` binary: add, gate, route, a
//! headless session on a stand-in runner, approval, submission, cleanup. Offline: the runner is a
//! script on PATH, the remote is a local bare repository, and HOME, git config and HARNESS_HOME
//! are temporary.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_harness");

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
        let config = json!({"contests_root": base.join("contests"), "workspace": "headless", "github_org": ""});
        std::fs::write(root.join("data/config.json"), config.to_string()).unwrap();

        // The stand-in runner: prints which contest session it is and the instructions it got.
        let bin = base.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let omp = bin.join("omp");
        std::fs::write(&omp, "#!/bin/sh\necho \"stand-in runner session=$HARNESS_SESSION args=$*\"\n").unwrap();
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
            .env("HARNESS_HOME", &self.root)
            .env("GIT_CONFIG_GLOBAL", &self.gitconfig)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("LANG", "C.UTF-8");
        cmd
    }

    fn harness(&self, args: &[&str]) -> Output {
        self.cmd(BIN).args(args).output().unwrap()
    }

    /// Run `harness <args> --json`, require success, return the JSON.
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

    // Launch a headless session on the stand-in runner; it runs as this contest's session.
    let launched = w.ok(&["launch", &id]);
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
    w.ok(&["transition", &id, "awaiting_approval", "--note", "수기 완성"]);

    // Submission and personal details need the user's approval, which a contest session cannot give.
    let refused = w.harness(&["submit", &id, "--method", "web", "--receipt", "R-1", "--files", "essay.md"]);
    assert_eq!(refused.status.code(), Some(1));
    std::fs::write(w.root.join("data/profile.md"), "## 참가 자격\n- 대학생\n\n## 제출용 개인정보\n- 이름: 시험\n")
        .unwrap();
    std::fs::set_permissions(w.root.join("data/profile.md"), std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(w.harness(&["profile", "show", "--for", &id]).status.code(), Some(1));
    let in_session = w.cmd(BIN).env("HARNESS_SESSION", &id).args(["approve", &id, "--note", "승인"]).output().unwrap();
    assert_eq!(in_session.status.code(), Some(1), "a contest session must not approve itself");
    w.ok(&["approve", &id, "--note", "제출해도 좋다"]);
    let profile = w.ok(&["profile", "show", "--for", &id]);
    assert!(profile.to_string().contains("이름: 시험"));

    // Submit, then the ledger blocks the same contest from coming back.
    w.ok(&["submit", &id, "--method", "web", "--receipt", "R-1", "--files", "essay.md"]);
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
    let out = w.cmd(BIN).env("HARNESS_HOME", &gone).args(["list"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(!gone.exists(), "nothing is created at a wrong root");
    // Doctor still runs and names the problem.
    let doctor = w.cmd(BIN).env("HARNESS_HOME", &gone).args(["doctor", "--json"]).output().unwrap();
    assert_eq!(doctor.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert_eq!(report["checks"][0]["name"], "checkout");
    assert_eq!(report["checks"][0]["level"], "fail");
}
