//! Runner command lines, workspace helpers, launch rendering, install conversions and the launchd job.

use std::path::{Path, PathBuf};

use harness::config::{self, Config};
use harness::launch;
use harness::registry::{self, Entry, NewEntry};
use harness::runners::{self, AgentDoc, Spec, claude, codex, dsh, omp};
use harness::schedule;
use harness::workspaces::{self, Open, headless, orca, shell_command, shell_quote};
use serde_json::{Value, json};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A fresh scratch directory per test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("harness-backends-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|x| x.to_string()).collect()
}

fn spec(dir: &Path, system: bool, model: bool, overlay: bool, extra: &[&str]) -> Spec {
    let prompt = dir.join("launch-prompt.md");
    std::fs::write(&prompt, "# 지시\n본문").unwrap();
    let system_file = dir.join("system.md");
    std::fs::write(&system_file, "페르소나 \"따옴표\"\n둘째 줄").unwrap();
    Spec {
        cwd: dir.to_path_buf(),
        prompt_file: prompt,
        first_message: "시작해.".into(),
        system_append: system.then_some(system_file),
        model: model.then(|| "opus".to_string()),
        preset_overlay: overlay.then(|| dir.join("omp-preset.yml")),
        extra_args: s(extra),
    }
}

#[test]
fn omp_commands() {
    let dir = scratch("omp");
    let d = dir.display().to_string();
    let full = spec(&dir, true, false, true, &["--thinking", "high"]);
    let runner = runners::get("omp").unwrap();
    assert!(runner.subagents());
    assert_eq!(
        runner.interactive(&full).unwrap(),
        vec![
            "omp".into(),
            "--cwd".into(),
            d.clone(),
            "--config".into(),
            format!("{d}/omp-preset.yml"),
            "--append-system-prompt".into(),
            format!("{d}/system.md"),
            "--thinking".into(),
            "high".into(),
            format!("@{d}/launch-prompt.md"),
            "시작해.".into(),
        ]
    );
    let bare = spec(&dir, false, false, false, &[]);
    assert_eq!(
        runner.oneshot(&bare).unwrap(),
        vec!["omp".into(), "--cwd".into(), d.clone(), "-p".into(), format!("@{d}/launch-prompt.md"), "시작해.".into()]
    );
}

#[test]
fn pi_commands() {
    let dir = scratch("pi");
    let d = dir.display().to_string();
    let runner = runners::get("pi").unwrap();
    assert!(!runner.subagents());
    let full = spec(&dir, true, true, true, &[]);
    assert_eq!(
        runner.interactive(&full).unwrap(),
        vec![
            "pi".into(),
            "--model".into(),
            "opus".into(),
            "--append-system-prompt".into(),
            format!("{d}/system.md"),
            format!("@{d}/launch-prompt.md"),
            "시작해.".into()
        ]
    );
    let bare = spec(&dir, false, false, false, &[]);
    assert_eq!(
        runner.oneshot(&bare).unwrap(),
        vec!["pi".into(), "-p".into(), format!("@{d}/launch-prompt.md"), "시작해.".into()]
    );
}

#[test]
fn claude_commands() {
    let dir = scratch("claude");
    let d = dir.display().to_string();
    let runner = runners::get("claude").unwrap();
    let full = spec(&dir, true, true, false, &["--permission-mode", "acceptEdits"]);
    assert_eq!(
        runner.interactive(&full).unwrap(),
        vec![
            "claude".into(),
            "--model".into(),
            "opus".into(),
            "--append-system-prompt-file".into(),
            format!("{d}/system.md"),
            "--permission-mode".into(),
            "acceptEdits".into(),
            "# 지시\n본문".into(),
        ]
    );
    let bare = spec(&dir, false, false, false, &[]);
    assert_eq!(runner.oneshot(&bare).unwrap(), s(&["claude", "-p", "# 지시\n본문"]));
}

#[test]
fn codex_commands() {
    let dir = scratch("codex");
    let d = dir.display().to_string();
    let runner = runners::get("codex").unwrap();
    let full = spec(&dir, true, true, false, &["--sandbox", "workspace-write"]);
    let instructions = r#"developer_instructions="페르소나 \"따옴표\"\n둘째 줄""#.to_string();
    assert_eq!(
        runner.interactive(&full).unwrap(),
        vec![
            "codex".into(),
            "-C".into(),
            d.clone(),
            "-m".into(),
            "opus".into(),
            "-c".into(),
            instructions,
            "--sandbox".into(),
            "workspace-write".into(),
            "# 지시\n본문".into(),
        ]
    );
    // exec adds workspace-write only when runner_args has no sandbox of its own.
    let own = runner.oneshot(&full).unwrap();
    assert_eq!(own.iter().filter(|a| *a == "--sandbox").count(), 1);
    let bare = spec(&dir, false, false, false, &[]);
    assert_eq!(
        runner.oneshot(&bare).unwrap(),
        vec![
            "codex".into(),
            "exec".into(),
            "-C".into(),
            d,
            "--sandbox".into(),
            "workspace-write".into(),
            "# 지시\n본문".into()
        ]
    );
}

#[test]
fn dsh_commands() {
    let dir = scratch("dsh");
    let runner = runners::get("dsh").unwrap();
    assert!(!runner.subagents());
    let full = spec(&dir, true, true, false, &[]);
    let message = "페르소나 \"따옴표\"\n둘째 줄\n\n---\n\n# 지시\n본문".to_string();
    assert_eq!(dsh::interactive_argv(&full, true).unwrap(), vec!["dsh-tui".into(), "--".into(), message.clone()]);
    assert_eq!(
        dsh::interactive_argv(&full, false).unwrap(),
        vec!["dsh".into(), "--profile".into(), "dsh-tui".into(), "--".into(), "--".into(), message]
    );
    let bare = spec(&dir, false, false, false, &[]);
    assert_eq!(runner.oneshot(&bare).unwrap(), s(&["dsh", "--profile", "headless", "# 지시\n본문"]));
}

#[test]
fn toml_strings_escape() {
    assert_eq!(runners::toml_string("a\"b\\c\nd\te\u{1}"), r#""a\"b\\c\nd\te\u0001""#);
}

#[test]
fn shell_quoting() {
    assert_eq!(shell_quote("omp"), "omp");
    assert_eq!(shell_quote("@/tmp/a.md"), "@/tmp/a.md");
    assert_eq!(shell_quote("it's 시작"), r"'it'\''s 시작'");
    assert_eq!(shell_quote(""), "''");
    assert_eq!(shell_command(Path::new("/a b"), &s(&["pi", "-p", "x y"])), "cd '/a b' && pi -p 'x y'");
}

#[test]
fn format_named_like_python() {
    let values = [("a", "1".to_string()), ("b", "{x}".to_string())];
    assert_eq!(launch::format_named("{a}-{b} {{a}} }}", &values).unwrap(), "1-{x} {a} }");
    assert!(launch::format_named("{c}", &values).is_err());
    assert!(launch::format_named("{a", &values).is_err());
    assert!(launch::format_named("a}", &values).is_err());
}

fn routed_entry(folder: &Path) -> Entry {
    let mut entry = registry::new_entry(NewEntry {
        id: "demo-contest",
        title: "데모 공모전",
        url: "https://example.com/c/1",
        source: "manual",
        organizer: "주최",
        deadline: Some("2026-11-30T23:59+09:00".into()),
        categories: Vec::new(),
        note: None,
    });
    entry.state = "routed".into();
    entry.kind = Some("data".into());
    entry.level = Some(3);
    entry.folder = Some(folder.display().to_string());
    entry
}

#[test]
fn render_prompt_fills_placeholders_and_notes_missing_subagents() {
    let dir = scratch("render");
    let entry = routed_entry(&dir);
    let omp = launch::render_prompt(&entry, runners::get("omp").unwrap()).unwrap();
    assert!(omp.starts_with("# 하네스 실행 지시: 데모 공모전"));
    assert!(omp.contains("data-analysis"));
    assert!(omp.contains("2026-11-30T23:59+09:00"));
    assert!(omp.contains("저장소: 없음"));
    assert!(omp.contains("harness transition demo-contest awaiting_approval"));
    let level = std::fs::read_to_string(root().join("harness/prompts/level_3.md")).unwrap();
    assert!(omp.contains(level.trim()));
    assert!(!omp.contains("서브에이전트 기능이 없으면"));
    let pi = launch::render_prompt(&entry, runners::get("pi").unwrap()).unwrap();
    assert!(pi.contains("서브에이전트 기능이 없으면 같은 세션에서 차례로 수행한다"));
    assert!(pi.contains("contest_agent"));
    let dsh = launch::render_prompt(&entry, runners::get("dsh").unwrap()).unwrap();
    assert!(dsh.contains("서브에이전트 기능이 없으면 같은 세션에서 차례로 수행한다"));
    assert!(!dsh.contains("contest_agent"));
}

#[test]
fn preset_overlay_from_omp_config() {
    let dir = scratch("preset");
    let yml = dir.join("config.yml");
    std::fs::write(&yml, "modelPresets:\n  P:\n    modelRoles:\n      default: a/b:high\n      task: c/d\n    defaultThinkingLevel: high\n  Q:\n    modelRoles:\n      default: x/y\n").unwrap();
    let out = launch::preset_overlay("P", &yml, &dir.join("o.yml")).unwrap();
    let overlay: Value = serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap();
    assert_eq!(overlay, json!({"modelRoles": {"default": "a/b:high", "task": "c/d"}, "defaultThinkingLevel": "high"}));
    let q = launch::preset_overlay("Q", &yml, &dir.join("q.yml")).unwrap();
    let overlay: Value = serde_json::from_str(&std::fs::read_to_string(q).unwrap()).unwrap();
    assert_eq!(overlay, json!({"modelRoles": {"default": "x/y"}}));
    assert!(launch::preset_overlay("Missing", &yml, &dir.join("m.yml")).unwrap_err().0.contains("Missing"));
}

fn config_with(runner: &str, workspace: &str) -> Config {
    let mut value = config::defaults();
    value["runner"] = json!(runner);
    value["workspace"] = json!(workspace);
    value["preset_by_level"] = json!({"1": null, "2": null, "3": null, "4": null, "5": null});
    value["model_by_level"] = json!({"3": "opus"});
    serde_json::from_value(value).unwrap()
}

#[test]
fn launch_dry_run_per_runner() {
    let dir = scratch("launch");
    let d = dir.display().to_string();
    let prompt = format!("{d}/.harness/launch-prompt.md");
    for (runner, workspace, expect) in [
        ("omp", "tmux", format!("cd {d} && omp --cwd {d} --model opus @{prompt} ")),
        ("pi", "orca", format!("cd {d} && pi --model opus @{prompt} ")),
        ("pi", "headless", format!("cd {d} && pi --model opus -p @{prompt} ")),
        ("claude", "tmux", format!("cd {d} && claude --model opus --permission-mode acceptEdits '# 하네스")),
        ("claude", "headless", format!("cd {d} && claude -p --model opus --permission-mode acceptEdits '# 하네스")),
        ("codex", "tmux", format!("cd {d} && codex -C {d} -m opus --sandbox workspace-write '# 하네스")),
        ("codex", "headless", format!("cd {d} && codex exec -C {d} -m opus --sandbox workspace-write '# 하네스")),
        ("dsh", "headless", format!("cd {d} && dsh --profile headless '# 하네스")),
    ] {
        let cfg = config_with(runner, workspace);
        let mut entry = routed_entry(&dir);
        let result = launch::launch(&mut entry, &cfg, None, true).unwrap();
        let command = result["command"].as_str().unwrap();
        assert!(command.starts_with(&expect), "{runner}/{workspace}: {command}");
        assert_eq!(result["runner"], runner);
        assert_eq!(result["workspace"], workspace);
        assert_eq!(result["prompt"], prompt.as_str());
        assert_eq!(result["handle"], Value::Null);
        assert_eq!(entry.state, "routed");
        assert!(entry.session.is_none());
    }
}

#[test]
fn launch_rejects_wrong_state_terminal_and_foreign_preset() {
    let dir = scratch("reject");
    let mut entry = routed_entry(&dir);
    entry.state = "gate_pass".into();
    assert!(
        launch::launch(&mut entry, &config_with("omp", "tmux"), None, true).unwrap_err().0.contains("routed 상태만")
    );
    let mut entry = routed_entry(&dir);
    assert!(launch::launch(&mut entry, &config_with("omp", "terminal"), None, true).is_err());
    assert!(
        launch::launch(&mut entry, &config_with("claude", "tmux"), Some("P"), true)
            .unwrap_err()
            .0
            .contains("omp 러너 전용")
    );
    entry.level = None;
    assert!(launch::launch(&mut entry, &config_with("omp", "tmux"), None, true).unwrap_err().0.contains("레벨이 없다"));
}

#[test]
fn agent_docs_parse() {
    let docs = runners::agent_docs(&root()).unwrap();
    assert!(docs.iter().any(|d| d.name == "ContestJudge"));
    let hq = docs.iter().find(|d| d.name == "ContestHQ").unwrap();
    assert!(hq.tools.contains(&"task".to_string()));
    assert_eq!(hq.skills, vec!["contest-harness".to_string()]);
    assert!(AgentDoc::parse("no frontmatter").is_err());
}

#[test]
fn claude_agent_conversion() {
    let doc = AgentDoc::parse(
        "---\nname: Demo\ndescription: \"Does: things\"\ntools:\n  - read\n  - web_search\n  - task\n  - todo\n  - weird\nmodel:\n  - \"@slow\"\nautoloadSkills:\n  - contest-harness\noutput:\n  properties:\n    verdict:\n      enum: [ready, revise]\n---\n본문\n",
    )
    .unwrap();
    let out = claude::convert_agent(&doc);
    assert!(out.starts_with("---\nname: \"Demo\"\ndescription: \"Does: things\"\ntools: Read, WebSearch, WebFetch, Agent, TodoWrite\nmodel: inherit\nskills:\n  - \"contest-harness\"\n---\n\n본문\n\n## 마지막 답변 형식\n"));
    assert!(out.contains("\"verdict\""));
    // The frontmatter must stay valid YAML.
    let head = out.split("---\n").nth(1).unwrap();
    let meta: Value = serde_yaml_ng::from_str(head).unwrap();
    assert_eq!(meta["model"], "inherit");
}

#[test]
fn codex_agent_conversion() {
    let doc = AgentDoc::parse("---\nname: Demo\ndescription: d \"q\"\n---\n줄1\n줄2\n").unwrap();
    assert_eq!(codex::file_name(&doc), "contest-Demo.toml");
    assert_eq!(
        codex::convert_agent(&doc),
        "name = \"Demo\"\ndescription = \"d \\\"q\\\"\"\ndeveloper_instructions = \"줄1\\n줄2\\n\"\n"
    );
}

#[test]
fn install_claude_codex_dsh_into_scratch_home() {
    let home = scratch("home");
    let done = claude::install_at(&root(), &home).unwrap();
    let agents = home.join(".claude/agents/contest-harness");
    assert!(agents.join("ContestJudge.md").is_file());
    assert!(std::fs::read_to_string(agents.join("ContestJudge.md")).unwrap().contains("model: inherit"));
    let skill = home.join(".claude/skills/contest-harness");
    assert_eq!(
        std::fs::canonicalize(&skill).unwrap(),
        std::fs::canonicalize(root().join("skills/contest-harness")).unwrap()
    );
    assert!(done.iter().any(|l| l.starts_with("linked")));
    // Re-running is a no-op; stale agents are removed.
    std::fs::write(agents.join("Gone.md"), "x").unwrap();
    let again = claude::install_at(&root(), &home).unwrap();
    assert!(again.iter().any(|l| l.starts_with("removed") && l.ends_with("Gone.md")));
    assert!(again.iter().filter(|l| !l.starts_with("removed")).all(|l| l.starts_with("ok")), "{again:?}");

    codex::install_at(&root(), &home).unwrap();
    let toml = std::fs::read_to_string(home.join(".codex/agents/contest-ContestJudge.toml")).unwrap();
    assert!(toml.starts_with("name = \"ContestJudge\"\n"));
    assert_eq!(toml.lines().count(), 3);
    assert!(home.join(".agents/skills/contest-harness").is_dir());
    assert!(dsh::install_at(&root(), &home).unwrap()[0].starts_with("ok"));

    // A real directory where the link goes is left alone.
    let other = scratch("home-real");
    std::fs::create_dir_all(other.join(".agents/skills/contest-harness")).unwrap();
    assert!(dsh::install_at(&root(), &other).is_err());
}

#[test]
fn omp_removes_only_legacy_links_into_the_repo() {
    let base = scratch("omp-legacy");
    let repo = base.join("repo");
    std::fs::create_dir_all(repo.join("agents")).unwrap();
    std::fs::create_dir_all(repo.join("skills/contest-harness")).unwrap();
    std::fs::write(repo.join("agents/ContestJudge.md"), "x").unwrap();
    std::fs::write(repo.join("agents/UxReviewer.md"), "x").unwrap();
    let elsewhere = base.join("elsewhere.md");
    std::fs::write(&elsewhere, "x").unwrap();
    let agent_dir = base.join("agent");
    std::fs::create_dir_all(agent_dir.join("agents")).unwrap();
    std::fs::create_dir_all(agent_dir.join("skills")).unwrap();
    std::os::unix::fs::symlink(repo.join("agents/ContestJudge.md"), agent_dir.join("agents/ContestJudge.md")).unwrap();
    std::os::unix::fs::symlink(repo.join("agents/UxReviewer.md"), agent_dir.join("agents/UxReviewer.md")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, agent_dir.join("agents/ContestOther.md")).unwrap();
    std::os::unix::fs::symlink(repo.join("agents/ContestJudge.md"), agent_dir.join("agents/Mine.md")).unwrap();
    std::os::unix::fs::symlink(repo.join("skills/contest-harness"), agent_dir.join("skills/contest-harness")).unwrap();
    let done = omp::remove_legacy(&agent_dir, &repo).unwrap();
    assert_eq!(done.len(), 3, "{done:?}");
    assert!(std::fs::symlink_metadata(agent_dir.join("agents/ContestOther.md")).is_ok());
    assert!(std::fs::symlink_metadata(agent_dir.join("agents/Mine.md")).is_ok());
    assert!(std::fs::symlink_metadata(agent_dir.join("skills/contest-harness")).is_err());
}

#[test]
fn launchd_plist() {
    let xml = schedule::plist(Path::new("/opt/bin/harness"), Path::new("/repo & co"), 6, "/usr/bin:/bin");
    assert!(xml.contains("<string>io.github.fmsongx2.autocontest-harness</string>"));
    assert!(xml.contains("<string>/opt/bin/harness</string>\n    <string>autorun</string>"));
    assert!(xml.contains("<key>HARNESS_HOME</key>\n    <string>/repo &amp; co</string>"));
    assert!(xml.contains("<integer>21600</integer>"));
    assert!(xml.contains("<string>/repo &amp; co/data/logs/launchd.log</string>"));
    assert!(schedule::plist_path().ends_with("Library/LaunchAgents/io.github.fmsongx2.autocontest-harness.plist"));
}

#[test]
fn workspace_registry() {
    for name in workspaces::NAMES {
        assert_eq!(workspaces::get(name).unwrap().name(), *name);
    }
    assert!(!workspaces::get("headless").unwrap().interactive());
    assert!(workspaces::get("nope").is_err());
    assert!(runners::get("nope").is_err());
}

#[test]
fn headless_open_and_tail() {
    let dir = scratch("headless");
    let ws = workspaces::get("headless").unwrap();
    let handle = ws
        .open(&Open {
            cwd: dir.clone(),
            title: "t",
            argv: s(&["sh", "-c", "pwd; echo ok-headless"]),
            workspace_id: None,
        })
        .unwrap();
    assert!(handle.starts_with("pid:"));
    let mut tail = String::new();
    for _ in 0..50 {
        tail = ws.tail(&handle, 5).unwrap_or_default();
        if tail.contains("ok-headless") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(tail.contains("ok-headless"), "{tail}");
    assert!(tail.contains("harness-backends"), "{tail}");
    assert!(ws.tail("pid:1", 5).is_err());
}

fn wait_for(ws: &dyn workspaces::Workspace, handle: &str, needle: &str) -> String {
    let mut tail = String::new();
    for _ in 0..100 {
        tail = ws.tail(handle, 20).unwrap_or_default();
        if tail.contains(needle) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    tail
}

#[test]
fn headless_close_terms_the_session_group() {
    let dir = scratch("headless-close");
    let ws = workspaces::get("headless").unwrap();
    // `sleep` is a child of `sh`: the group signal must reach both, and the trap proves TERM arrived.
    let script = "trap 'echo got-term; exit 0' TERM; echo ready; while :; do sleep 0.1; done";
    let handle = ws.open(&Open { cwd: dir, title: "t", argv: s(&["sh", "-c", script]), workspace_id: None }).unwrap();
    assert!(wait_for(ws, &handle, "ready").contains("ready"));
    ws.close(&handle).unwrap();
    assert!(wait_for(ws, &handle, "got-term").contains("got-term"));
    // Bad handles are errors; an exited pid is not.
    assert!(ws.close("tmux:@1").is_err());
    assert!(ws.close("pid:1:/x").is_err());
    assert!(ws.close("pid:999999:/x").is_ok());
    assert_eq!(headless::pid_of("pid:42:/a:b.log").unwrap(), 42);
    ws.remove_workspace("anything").unwrap();
}

#[test]
fn tmux_close_kills_the_window() {
    let ws = workspaces::get("tmux").unwrap();
    if !ws.available() {
        eprintln!("tmux not installed; skipped");
        return;
    }
    // A throwaway session on the default server (the one the backend talks to), never `contest-harness`.
    let session = format!("harness-test-{}", std::process::id());
    let tmux = |args: &[&str]| std::process::Command::new("tmux").args(args).output().unwrap();
    assert!(tmux(&["new-session", "-d", "-s", &session, "sleep 30"]).status.success());
    let target = format!("{session}:");
    let out = tmux(&["new-window", "-d", "-P", "-F", "#{session_name}:#{window_id}", "-t", &target, "sleep 30"]);
    let window = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let window_id = window.split(':').nth(1).unwrap().to_string();
    ws.close(&window).unwrap();
    let left =
        String::from_utf8_lossy(&tmux(&["list-windows", "-t", &session, "-F", "#{window_id}"]).stdout).to_string();
    tmux(&["kill-session", "-t", &session]);
    assert!(!left.lines().any(|l| l == window_id), "{left}");
    // Closing again (window gone) and an unknown session count as closed.
    ws.close(&window).unwrap();
    ws.close("harness-test-none:@999999").unwrap();
    ws.remove_workspace("anything").unwrap();
}

#[test]
fn orca_remove_only_forgets_folder_workspaces() {
    let id = "00000000-0000-4000-8000-000000000000::/Users/me/contests::workspace:11111111-1111-4111-8111-111111111111";
    assert!(orca::is_folder_workspace(id));
    assert_eq!(
        orca::remove_workspace_args(id).unwrap(),
        vec!["worktree".to_string(), "rm".into(), "--worktree".into(), format!("id:{id}")]
    );
    // A git worktree or the project root would lose its directory: refused before Orca is called.
    for bad in [
        "00000000-0000-4000-8000-000000000000::/Users/me/contests",
        "repo::/Users/me/wt/fix-login",
        "::workspace:11111111-1111-4111-8111-111111111111",
        "repo::/p::workspace:not-a-uuid",
    ] {
        assert!(!orca::is_folder_workspace(bad), "{bad}");
        assert!(workspaces::get("orca").unwrap().remove_workspace(bad).is_err(), "{bad}");
    }
}

#[test]
fn terminal_close_is_a_no_op() {
    let ws = workspaces::get("terminal").unwrap();
    ws.close("anything").unwrap();
    ws.remove_workspace("anything").unwrap();
}
