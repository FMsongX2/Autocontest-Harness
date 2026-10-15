//! Route, launch and clean against stub `gh`/`orca` on PATH and local bare remotes: retries after
//! failures, refusing repositories that are not ours, clean ignores, partial `clean --all`, archive
//! containment, headless close and bounded external commands. Nothing reaches GitHub or Orca.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use harness::clean::{self, Options};
use harness::config::{self, Config};
use harness::ledger::{self, Ledger};
use harness::registry::{self, Entry, NewEntry, apply_gate, new_entry};
use harness::workspaces::{Open, Workspace, headless::Headless, orca::Orca};
use harness::{launch, process, route};
use serde_json::{Value, json};

/// `gh` stub. Behaviour is chosen by the owner (`github_org`) so parallel tests do not interfere:
/// `lc-public` has a public repo, `lc-other` a private repo of another contest, every other owner
/// has none. `repo create` makes a bare repo next to the source folder and pushes to it; it fails
/// while `<source>/../FAIL_CREATE` exists. Every call is appended to `<stub dir>/gh.log`.
const GH: &str = r#"#!/bin/sh
here=$(dirname "$0")
echo "$*" >> "$here/gh.log"
case "$1 $2" in
  "auth status") exit 0 ;;
  "repo view")
    case "$3" in
      lc-public/*) echo '{"isPrivate":false,"isEmpty":true}'; exit 0 ;;
      lc-other/*) echo '{"isPrivate":true,"isEmpty":false}'; exit 0 ;;
      *) echo "GraphQL: Could not resolve to a Repository with the name '$3'. (repository)" >&2; exit 1 ;;
    esac ;;
  "repo create")
    src=""; prev=""
    for a in "$@"; do [ "$prev" = "--source" ] && src="$a"; prev="$a"; done
    if [ -e "$src/../FAIL_CREATE" ]; then echo "HTTP 502: create failed" >&2; exit 1; fi
    bare="$src/../remote-$(basename "$src").git"
    git init -q --bare "$bare" && git -C "$src" remote add origin "$bare" && git -C "$src" push -q -u origin main
    exit $? ;;
  "api -X") exit 0 ;;
  "api -H") echo "Not Found" >&2; exit 1 ;;
esac
echo "unexpected gh call: $*" >&2
exit 1
"#;

/// `orca` stub: logs its argv one element per line and answers like `worktree create`.
const ORCA: &str = r#"#!/bin/sh
here=$(dirname "$0")
for a in "$@"; do echo "$a"; done >> "$here/orca.log"
echo '{"ok":true,"result":{"worktree":{"id":"wt-1"}}}'
"#;

/// `omp` stub: a session that reports which contest it was launched for.
const OMP: &str = "#!/bin/sh\necho \"session=${HARNESS_SESSION:-none}\"\n";

/// `tmux` stub: every session exists; `new-window` prints a window handle.
const TMUX: &str = r#"#!/bin/sh
case "$1" in
  new-window) echo "contest-harness:@1" ;;
esac
exit 0
"#;

struct Stubs {
    dir: PathBuf,
}

/// Process-wide setup, forced by every test before it touches the environment: stub
/// `gh`/`orca`/`omp`/`tmux` first on PATH, a git identity of their own, and a temporary
/// `HARNESS_HOME` holding a copy of the prompts (so nothing reads or writes the real `data/`).
static STUBS: LazyLock<Stubs> = LazyLock::new(|| {
    let dir = std::env::temp_dir().join(format!("harness-lifecycle-stubs-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (name, body) in [("gh", GH), ("orca", ORCA), ("omp", OMP), ("tmux", TMUX)] {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let gitconfig = dir.join("gitconfig");
    fs::write(&gitconfig, "[user]\n\tname = Lifecycle Test\n\temail = lifecycle@example.com\n").unwrap();
    let home = dir.join("home");
    let prompts = home.join("harness/prompts");
    fs::create_dir_all(&prompts).unwrap();
    for file in fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("harness/prompts")).unwrap() {
        let file = file.unwrap().path();
        fs::copy(&file, prompts.join(file.file_name().unwrap())).unwrap();
    }
    let path = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap_or_default());
    // SAFETY: runs once, inside the LazyLock, before any test of this binary reads the environment.
    unsafe {
        std::env::set_var("PATH", path);
        std::env::set_var("GIT_CONFIG_GLOBAL", &gitconfig);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_var("HARNESS_HOME", &home);
        std::env::remove_var("HARNESS_SESSION");
    }
    Stubs { dir }
});

fn stubs() -> &'static Stubs {
    &STUBS
}

fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "harness-lifecycle-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn config_at(root: &Path, overlay: Value) -> Config {
    let mut base = json!({"contests_root": root.join("contests").to_string_lossy(), "workspace": "headless"});
    for (k, v) in overlay.as_object().unwrap() {
        base[k] = v.clone();
    }
    serde_json::from_value(config::merge(config::defaults(), base)).unwrap()
}

fn gated(id: &str, folder: &str) -> Entry {
    let mut e = new_entry(NewEntry {
        id,
        title: "생명주기 시험 공모전",
        url: "https://example.com/c/1",
        source: "manual",
        organizer: "주최",
        deadline: None,
        categories: vec![],
        note: None,
    });
    let gate = json!({"decision": "pass", "type": "idea", "level": 2, "priority": 70, "folder": folder,
        "deliverables": [{"name": "제안서", "format": "hwp", "limits": ""}],
        "criteria": [{"stage": "예비", "name": "참신성", "weight": 30}], "reasons": ["근거"]});
    apply_gate(&mut e, gate, "ContestGate").unwrap();
    e
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn gh_log() -> String {
    fs::read_to_string(stubs().dir.join("gh.log")).unwrap_or_default()
}

#[test]
fn route_failing_partway_can_be_retried_once_the_cause_is_fixed() {
    stubs();
    let root = temp_dir("retry");
    let cfg = config_at(&root, json!({"github_org": "lc-retry"}));
    let target = cfg.contests_root().join("Retry-Contest");
    fs::create_dir_all(cfg.contests_root()).unwrap();
    fs::write(cfg.contests_root().join("FAIL_CREATE"), "").unwrap();
    let mut e = gated("manual-retry", "Retry-Contest");

    let err = route::route(&mut e.clone(), &cfg, &Ledger::default(), None, true, false).unwrap_err();
    assert!(err.0.contains("create failed"), "{err}");
    let stub: Value = serde_json::from_str(&fs::read_to_string(target.join(".harness/contest.json")).unwrap()).unwrap();
    assert_eq!(stub["id"], "manual-retry", "the half-done folder is claimed by this contest");

    fs::remove_file(cfg.contests_root().join("FAIL_CREATE")).unwrap();
    let summary = route::route(&mut e, &cfg, &Ledger::default(), None, true, false).unwrap();
    assert_eq!(summary["repo"], "lc-retry/Retry-Contest");
    assert_eq!(e.state, "routed");
    assert!(git(&target, &["status", "--porcelain"]).trim().is_empty());
    assert_eq!(git(&target, &["rev-list", "@{u}..HEAD"]).trim(), "", "main is pushed");
    assert!(gh_log().contains("api -X PUT repos/lc-retry/Retry-Contest/actions/permissions -F enabled=false"));

    // A different contest still cannot take the folder over.
    let mut other = gated("manual-other", "Retry-Contest");
    assert!(route::route(&mut other, &cfg, &Ledger::default(), None, false, false).is_err());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn route_refuses_existing_public_or_foreign_repository_without_pushing() {
    stubs();
    for (org, needle) in [("lc-public", "비공개가 아니다"), ("lc-other", "것이 아니다")] {
        let root = temp_dir(org);
        let cfg = config_at(&root, json!({"github_org": org}));
        let mut e = gated("manual-taken", "Taken-Name");
        let err = route::route(&mut e, &cfg, &Ledger::default(), None, true, false).unwrap_err();
        assert!(err.0.contains(needle), "{org}: {err}");
        assert_eq!(e.state, "gate_pass");
        let target = cfg.contests_root().join("Taken-Name");
        assert!(git(&target, &["remote"]).trim().is_empty(), "{org}: no remote was added");
        assert!(!gh_log().contains(&format!("repo create {org}/")), "{org}: nothing was created");
        fs::remove_dir_all(&root).unwrap();
    }
}

#[test]
fn route_preflight_names_the_missing_workspace_before_writing_anything() {
    stubs();
    let root = temp_dir("preflight");
    let cfg = config_at(&root, json!({"workspace": "tmux-not-a-backend"}));
    let mut e = gated("manual-pre", "Pre-Flight");
    assert!(route::route(&mut e, &cfg, &Ledger::default(), None, false, true).is_err());
    assert!(!cfg.contests_root().join("Pre-Flight").exists(), "nothing written before the checks pass");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn route_appends_missing_ignores_and_headless_session_files_stay_out_of_git() {
    stubs();
    let root = temp_dir("ignore");
    let cfg = config_at(&root, json!({}));
    let target = cfg.contests_root().join("Ignore-Contest");
    fs::create_dir_all(target.join(".harness")).unwrap();
    fs::write(target.join(".harness/contest.json"), r#"{"id": "manual-ignore"}"#).unwrap();
    fs::write(target.join(".gitignore"), "node_modules/\n.DS_Store").unwrap();
    let mut e = gated("manual-ignore", "Ignore-Contest");
    route::route(&mut e, &cfg, &Ledger::default(), None, false, false).unwrap();

    let ignore = fs::read_to_string(target.join(".gitignore")).unwrap();
    let lines: Vec<&str> = ignore.lines().collect();
    assert_eq!(lines[0], "node_modules/", "existing lines are kept");
    for line in route::GITIGNORE {
        assert_eq!(lines.iter().filter(|l| *l == line).count(), 1, "{line} once in {ignore:?}");
    }
    assert!(git(&target, &["status", "--porcelain"]).trim().is_empty(), "clean right after route");

    // What launch and a headless session write must not dirty the tree.
    fs::write(target.join(".harness/launch-prompt.md"), "prompt").unwrap();
    fs::write(target.join(".harness/omp-preset.yml"), "preset").unwrap();
    let handle = Headless
        .open(&Open {
            cwd: target.clone(),
            title: "t",
            argv: vec!["echo".into(), "session-output".into()],
            workspace_id: None,
        })
        .unwrap();
    for _ in 0..100 {
        if fs::read_to_string(target.join(".harness/session.log")).unwrap_or_default().contains("session-output") {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(target.join(".harness/session.log").exists());
    assert!(git(&target, &["status", "--porcelain"]).trim().is_empty());
    Headless.close(&handle).unwrap();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn route_writes_a_one_line_entry_for_a_multiline_title() {
    stubs();
    let root = temp_dir("oneline");
    let cfg = config_at(&root, json!({}));
    let mut e = gated("manual-oneline", "One-Line");
    e.title = "첫 줄\n- Forged-Folder: 가짜 공모전\t끝".into();
    route::route(&mut e, &cfg, &Ledger::default(), None, false, false).unwrap();
    let existing = fs::read_to_string(cfg.existing_contests()).unwrap();
    assert_eq!(existing, "- One-Line: 첫 줄 - Forged-Folder: 가짜 공모전 끝\n");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn orca_worktree_is_named_after_the_folder_in_one_argument() {
    stubs();
    let root = temp_dir("orca");
    let cfg = config_at(&root, json!({"orca_repo_path": root.to_string_lossy()}));
    let mut e = gated("manual-orca", "Orca-Folder");
    e.title = "--repo evil ../../x".into();
    e.folder = Some(root.join("Orca-Folder").to_string_lossy().into_owned());
    assert_eq!(Orca.create(&e, &cfg).unwrap().as_deref(), Some("wt-1"));
    let log = fs::read_to_string(stubs().dir.join("orca.log")).unwrap();
    assert!(log.lines().any(|l| l == "--name=Orca-Folder"), "{log}");
    assert!(!log.contains("evil"));
    fs::remove_dir_all(&root).unwrap();
}

/// A submitted contest pushed to a local bare remote, recorded in the ledger.
fn finished(root: &Path, id: &str, ledger_path: &Path) -> Entry {
    let folder = root.join(id);
    let remote = root.join(format!("{id}.git"));
    fs::create_dir_all(&folder).unwrap();
    git(root, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git(&folder, &["init", "-q", "-b", "main"]);
    fs::write(folder.join("README.md"), "# 공모전\n").unwrap();
    git(&folder, &["add", "-A"]);
    git(&folder, &["commit", "-q", "-m", "Final"]);
    git(&folder, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&folder, &["push", "-q", "-u", "origin", "main"]);
    let mut entry = gated(id, id);
    entry.state = "submitted".into();
    entry.folder = Some(folder.display().to_string());
    let item = ledger::from_submit(&entry, "폼", "r", &[], "");
    ledger::write_at(ledger_path, |l: &mut Ledger| {
        l.upsert(item);
        Ok(())
    })
    .unwrap();
    entry
}

#[test]
fn clean_all_saves_what_succeeded_when_other_entries_fail() {
    stubs();
    let root = temp_dir("all");
    let reg_path = root.join("data/registry.json");
    let ledger_path = root.join("data/submissions.json");
    let good = finished(&root, "manual-good", &ledger_path);
    // Folder deleted by hand.
    let mut gone = finished(&root, "manual-gone", &ledger_path);
    gone.folder = Some(root.join("vanished").display().to_string());
    // A folder whose git metadata is broken: checks and the archive plan fail.
    let broken = finished(&root, "manual-broken", &ledger_path);
    let broken_folder = PathBuf::from(broken.folder.clone().unwrap());
    fs::remove_dir_all(broken_folder.join(".git")).unwrap();
    fs::write(broken_folder.join(".git"), "gitdir: /nonexistent\n").unwrap();
    registry::write_at(&reg_path, |reg| {
        for e in [&gone, &good, &broken] {
            reg.contests.insert(e.id.clone(), e.clone());
        }
        Ok(())
    })
    .unwrap();

    let opts = Options { force: true, trash_dir: Some(root.join("trash")), ..Options::default() };
    let outcomes = clean::run(&reg_path, &ledger_path, None, &opts).unwrap();
    let by_id = |id: &str| outcomes.iter().find(|o| o.id == id).unwrap();
    assert!(by_id("manual-good").ok, "{outcomes:?}");
    assert!(!by_id("manual-gone").ok && !by_id("manual-gone").problems.is_empty());
    assert!(!by_id("manual-broken").ok && !by_id("manual-broken").problems.is_empty());

    let reg = registry::read_at(&reg_path, |reg| Ok(reg.clone())).unwrap();
    let ledger = ledger::load_at(&ledger_path).unwrap();
    for (id, removed) in [("manual-good", true), ("manual-gone", false), ("manual-broken", false)] {
        assert_eq!(reg.contests[id].extra.contains_key("folder_removed_at"), removed, "registry {id}");
        assert_eq!(ledger.get(id).unwrap().extra.contains_key("folder_removed_at"), removed, "ledger {id}");
    }
    assert!(!root.join("manual-good").exists() && broken_folder.exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn archive_never_copies_anything_outside_the_folder() {
    stubs();
    let root = temp_dir("traversal");
    let folder = root.join("box/Contest");
    fs::create_dir_all(folder.join(".private")).unwrap();
    fs::write(root.join("secret"), "id_rsa").unwrap();
    fs::write(root.join("box/x"), "outside").unwrap();
    git(&folder, &["init", "-q", "-b", "main"]);
    fs::write(folder.join(".gitignore"), ".private/\n").unwrap();
    fs::write(folder.join(".private/note.md"), "receipt").unwrap();
    std::os::unix::fs::symlink(root.join("secret"), folder.join(".private/key")).unwrap();

    assert_eq!(clean::folder_relative(&folder, "a/../../x"), None);
    assert_eq!(clean::folder_relative(&folder, ".harness/../../../secret"), None);
    assert_eq!(clean::folder_relative(&folder, "/etc/passwd"), None);
    assert_eq!(clean::folder_relative(&folder, "sub/final.hwp").as_deref(), Some("sub/final.hwp"));
    let inside = folder.join("sub/final.hwp");
    assert_eq!(clean::folder_relative(&folder, inside.to_str().unwrap()).as_deref(), Some("sub/final.hwp"));

    let mut entry = gated("manual-trav", "Contest");
    entry.submissions.push(json!({"files": ["a/../../x", ".harness/../../../secret"]}));
    let plan = clean::archive_plan(&entry, &folder).unwrap();
    assert_eq!(plan.keep, vec![".private/note.md".to_string()], "no traversal, no symlink out");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn trash_location_follows_the_platform() {
    let root = temp_dir("trash");
    let (home, data) = (root.join("home"), root.join("data"));
    fs::create_dir_all(&home).unwrap();
    assert_eq!(clean::trash_dir_for(true, &home, None, &data), data.join("trash"));
    fs::create_dir_all(home.join(".Trash")).unwrap();
    assert_eq!(clean::trash_dir_for(true, &home, None, &data), home.join(".Trash"));
    assert_eq!(clean::trash_dir_for(false, &home, None, &data), data.join("trash"));
    fs::create_dir_all(home.join(".local/share/Trash/files")).unwrap();
    assert_eq!(clean::trash_dir_for(false, &home, None, &data), home.join(".local/share/Trash/files"));
    let xdg = root.join("xdg");
    assert_eq!(clean::trash_dir_for(false, &home, Some(&xdg), &data), data.join("trash"));
    fs::create_dir_all(xdg.join("Trash/files")).unwrap();
    assert_eq!(clean::trash_dir_for(false, &home, Some(&xdg), &data), xdg.join("Trash/files"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn headless_close_ignores_a_reused_pid_and_stops_its_own_session() {
    stubs();
    let mut child = Command::new("sleep").arg("30").process_group(0).spawn().unwrap();
    let pid = child.id() as i32;
    // A handle recorded for another process that had this pid: stale, so nothing is signalled.
    Headless.close(&format!("pid:{pid}/Mon_Jan__1_00.00.00_2001:/tmp/x.log")).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(child.try_wait().unwrap().is_none(), "an unrelated process must survive");

    let start = harness::workspaces::headless::start_time(pid).unwrap();
    Headless.close(&format!("pid:{pid}/{start}:/tmp/x.log")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "the session's group was not stopped");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn external_commands_are_bounded_by_their_timeout() {
    stubs();
    let started = Instant::now();
    let err = process::output("sh", &["-c", "sleep 20"], None, Duration::from_secs(1)).unwrap_err();
    assert!(err.0.starts_with("sh -c sleep 20"), "the error names the command: {err}");
    assert!(started.elapsed() < Duration::from_secs(5));

    // A grandchild that keeps the output pipe open does not hold the caller past the budget.
    let started = Instant::now();
    let out = process::output("sh", &["-c", "sleep 8 & echo done"], None, Duration::from_secs(1)).unwrap();
    assert!(out.status.success());
    assert!(started.elapsed() < Duration::from_secs(5));

    let out = process::output("sh", &["-c", "printf \"$GIT_TERMINAL_PROMPT\""], None, process::LOCAL).unwrap();
    assert_eq!(out.stdout, b"0");
    assert!(process::output("/nonexistent/program", &[], None, process::LOCAL).unwrap_err().0.contains("실행 실패"));
}

/// A routed contest on headless (no repo, no workspace).
fn routed(root: &Path, id: &str, folder: &str) -> (Config, Entry) {
    let cfg = config_at(root, json!({"runner": "omp", "workspace": "headless"}));
    let mut e = gated(id, folder);
    route::route(&mut e, &cfg, &Ledger::default(), None, false, false).unwrap();
    (cfg, e)
}

#[test]
fn launch_dry_run_writes_nothing_and_shows_the_session_marker() {
    stubs();
    let root = temp_dir("dry");
    let (cfg, mut e) = routed(&root, "manual-dry", "Dry-Run");
    let target = cfg.contests_root().join("Dry-Run");
    let before = git(&target, &["status", "--porcelain", "--ignored"]);
    let result = launch::launch(&mut e, &cfg, None, true).unwrap();
    assert!(result["command"].as_str().unwrap().contains("env HARNESS_SESSION=manual-dry "), "{result}");
    assert!(!result["prompt_text"].as_str().unwrap_or_default().is_empty());
    assert_eq!(result["warnings"], json!([]));
    assert!(!target.join(".harness/launch-prompt.md").exists());
    assert_eq!(git(&target, &["status", "--porcelain", "--ignored"]), before, "dry run changed the folder");
    assert_eq!(e.state, "routed");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn launched_session_carries_its_contest_id_and_the_main_session_does_not() {
    stubs();
    let root = temp_dir("session-env");
    let (cfg, mut e) = routed(&root, "manual-env", "Env-Contest");
    let target = cfg.contests_root().join("Env-Contest");
    let result = launch::launch(&mut e, &cfg, None, false).unwrap();
    assert_eq!(e.state, "running");
    let log = target.join(".harness/session.log");
    let mut text = String::new();
    for _ in 0..200 {
        text = fs::read_to_string(&log).unwrap_or_default();
        if text.contains("session=") {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(text.contains("session=manual-env"), "{text:?}");
    assert!(git(&target, &["status", "--porcelain"]).trim().is_empty(), "launch leaves the tree clean");
    let _ = Headless.close(result["handle"].as_str().unwrap());

    let main = launch::chat(&cfg, Some("omp"), Some("tmux")).unwrap();
    assert!(!main["command"].as_str().unwrap().contains("HARNESS_SESSION"), "{main}");
    fs::remove_dir_all(&root).unwrap();
}
