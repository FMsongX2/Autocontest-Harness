//! Runner adapters (the contract every adapter keeps, the omp adapter, install helpers), workspace
//! helpers, launch rendering, and the schedule (cron, scout hours, launchd job).

use std::path::{Path, PathBuf};

use harness::config::{self, Config};
use harness::error::Result;
use harness::launch;
use harness::registry::{self, Entry, NewEntry};
use harness::runners::{self, AgentDoc, Runner, Spec, omp};
use harness::schedule::{self, Cron};
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
        preset_file: overlay.then(|| dir.join("omp-preset.yml")),
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

/// A minimal adapter without subagents or presets, shaped like one a contributor would add: it
/// passes the system prompt and the instructions inline. It exercises the branches omp does not.
struct Plain;

impl Runner for Plain {
    fn name(&self) -> &'static str {
        "plain"
    }

    fn binary(&self) -> &'static str {
        "plain-agent"
    }

    fn subagents(&self) -> bool {
        false
    }

    fn interactive(&self, spec: &Spec) -> Result<Vec<String>> {
        let mut argv = vec!["plain-agent".to_string()];
        if let Some(text) = runners::system_text(spec)? {
            argv.extend(["--system".into(), text]);
        }
        argv.extend(spec.extra_args.iter().cloned());
        argv.push(runners::prompt_text(spec)?);
        Ok(argv)
    }

    fn oneshot(&self, spec: &Spec) -> Result<Vec<String>> {
        let mut argv = self.interactive(spec)?;
        argv.insert(1, "--print".into());
        Ok(argv)
    }

    fn install(&self, _root: &Path) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
}

/// The rules in CONTRIBUTING.md that every adapter keeps, whatever its flags look like.
fn check_contract(runner: &dyn Runner) {
    let name = runner.name();
    let dir = scratch(&format!("contract-{name}"));
    let full = spec(&dir, true, false, false, &["--extra", "v w"]);
    let prompt = full.prompt_file.display().to_string();
    let system = full.system_append.clone().unwrap().display().to_string();
    let interactive = runner.interactive(&full).unwrap();
    let oneshot = runner.oneshot(&full).unwrap();
    assert_ne!(interactive, oneshot, "{name}: the one-shot command must differ from the interactive one");
    for argv in [&interactive, &oneshot] {
        assert_eq!(argv[0], runner.binary(), "{name}: commands start with binary(): {argv:?}");
        assert!(
            argv.iter().any(|a| a.contains(&prompt) || a.contains("# 지시\n본문")),
            "{name}: the instructions do not reach the agent: {argv:?}"
        );
        assert!(
            argv.windows(2).any(|w| w[0] == "--extra" && w[1] == "v w"),
            "{name}: runner_args must pass through unchanged and in order: {argv:?}"
        );
        assert!(
            argv.iter().any(|a| a.contains(&system) || a.contains("페르소나")),
            "{name}: the system prompt addition is missing: {argv:?}"
        );
    }
    // Contest sessions and scheduled runs have no system addition (the persona stays out of them).
    let bare = spec(&dir, false, false, false, &[]);
    for argv in [runner.interactive(&bare).unwrap(), runner.oneshot(&bare).unwrap()] {
        assert!(
            !argv.iter().any(|a| a.contains("system.md") || a.contains("페르소나")),
            "{name}: system text without system_append: {argv:?}"
        );
    }
    if !runner.presets() {
        assert!(runner.preset_file("P", &dir).is_err(), "{name}: no presets, so preset_file must refuse");
    }
}

#[test]
fn every_adapter_keeps_the_runner_contract() {
    let names = runners::names();
    let mut unique = names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), names.len(), "duplicate adapter names: {names:?}");
    for runner in runners::ALL {
        assert_eq!(runners::get(runner.name()).unwrap().name(), runner.name());
        check_contract(*runner);
    }
    check_contract(&Plain);
    assert!(runners::get("nope").err().unwrap().0.contains("CONTRIBUTING.md"));
}

#[test]
fn shell_quoting() {
    assert_eq!(shell_quote("omp"), "omp");
    assert_eq!(shell_quote("@/tmp/a.md"), "@/tmp/a.md");
    assert_eq!(shell_quote("it's 시작"), r"'it'\''s 시작'");
    assert_eq!(shell_quote(""), "''");
    assert_eq!(shell_command(Path::new("/a b"), &s(&["omp", "-p", "x y"])), "cd '/a b' && omp -p 'x y'");
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
    assert!(omp.starts_with("# 하네스 실행 지시\n"));
    assert!(omp.contains("제목: 데모 공모전"));
    assert!(omp.contains("data-analysis"));
    assert!(omp.contains("2026-11-30T23:59+09:00"));
    assert!(omp.contains("저장소: 없음"));
    assert!(omp.contains("harness transition demo-contest awaiting_approval"));
    let level = std::fs::read_to_string(root().join("harness/prompts/level_3.md")).unwrap();
    assert!(omp.contains(level.trim()));
    assert!(!omp.contains("서브에이전트 기능이 없으면"));
    let plain = launch::render_prompt(&entry, &Plain).unwrap();
    assert!(plain.contains("## 이 러너(plain)의 서브에이전트"));
    assert!(plain.contains("서브에이전트 기능이 없으면 같은 세션에서 차례로 수행한다"));
}

#[test]
fn preset_overlay_from_omp_config() {
    let dir = scratch("preset");
    let yml = dir.join("config.yml");
    std::fs::write(&yml, "modelPresets:\n  P:\n    modelRoles:\n      default: a/b:high\n      task: c/d\n    defaultThinkingLevel: high\n  Q:\n    modelRoles:\n      default: x/y\n").unwrap();
    let out = omp::preset_overlay("P", &yml, &dir.join("o.yml")).unwrap();
    let overlay: Value = serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap();
    assert_eq!(overlay, json!({"modelRoles": {"default": "a/b:high", "task": "c/d"}, "defaultThinkingLevel": "high"}));
    let q = omp::preset_overlay("Q", &yml, &dir.join("q.yml")).unwrap();
    let overlay: Value = serde_json::from_str(&std::fs::read_to_string(q).unwrap()).unwrap();
    assert_eq!(overlay, json!({"modelRoles": {"default": "x/y"}}));
    assert!(omp::preset_overlay("Missing", &yml, &dir.join("m.yml")).unwrap_err().0.contains("Missing"));
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
fn launch_dry_run_per_workspace() {
    let dir = scratch("launch");
    let d = dir.display().to_string();
    let prompt = format!("{d}/.harness/launch-prompt.md");
    for (runner, workspace, expect) in [
        ("omp", "tmux", format!("cd {d} && env HARNESS_SESSION=demo-contest omp --cwd {d} --model opus @{prompt} ")),
        ("omp", "orca", format!("cd {d} && env HARNESS_SESSION=demo-contest omp --cwd {d} --model opus @{prompt} ")),
        (
            "omp",
            "headless",
            format!("cd {d} && env HARNESS_SESSION=demo-contest omp --cwd {d} -p --model opus @{prompt} "),
        ),
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
fn launch_rejects_wrong_state_terminal_and_missing_level() {
    let dir = scratch("reject");
    let mut entry = routed_entry(&dir);
    entry.state = "gate_pass".into();
    assert!(
        launch::launch(&mut entry, &config_with("omp", "tmux"), None, true).unwrap_err().0.contains("routed 상태만")
    );
    let mut entry = routed_entry(&dir);
    assert!(launch::launch(&mut entry, &config_with("omp", "terminal"), None, true).is_err());
    assert!(launch::launch(&mut entry, &config_with("nope", "tmux"), None, true).is_err());
    entry.level = None;
    assert!(launch::launch(&mut entry, &config_with("omp", "tmux"), None, true).unwrap_err().0.contains("레벨이 없다"));
}

#[test]
fn presets_follow_the_adapter() {
    let mut cfg = config_with("omp", "tmux");
    cfg.preset_by_level.insert(3, Some("Deep".into()));
    let omp = runners::get("omp").unwrap();
    assert_eq!(launch::pick_preset(omp, &cfg, 3, None).unwrap().as_deref(), Some("Deep"));
    assert_eq!(launch::pick_preset(omp, &cfg, 3, Some("Fast")).unwrap().as_deref(), Some("Fast"));
    assert_eq!(launch::pick_preset(omp, &cfg, 2, None).unwrap(), None);
    // An adapter without presets ignores preset_by_level and refuses an explicit --preset.
    assert_eq!(launch::pick_preset(&Plain, &cfg, 3, None).unwrap(), None);
    assert!(launch::pick_preset(&Plain, &cfg, 3, Some("Fast")).unwrap_err().0.contains("프리셋을 지원하지 않는다"));
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
fn install_helpers_link_and_write_without_clobbering() {
    let base = scratch("helpers");
    let src = base.join("skill");
    let other = base.join("other");
    let real = base.join("real");
    for dir in [&src, &other, &real] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let dst = base.join("home/.agents/skills/contest-harness");
    assert!(runners::link(&src, &dst).unwrap().starts_with("linked"));
    assert!(runners::link(&src, &dst).unwrap().starts_with("ok"));
    // A stale symlink is repointed; a real directory or a missing source is refused.
    assert!(runners::link(&other, &dst).unwrap().starts_with("linked"));
    assert_eq!(std::fs::canonicalize(&dst).unwrap(), std::fs::canonicalize(&other).unwrap());
    assert!(runners::link(&src, &real).is_err());
    assert!(runners::link(&base.join("missing"), &base.join("x")).is_err());
    let file = base.join("out/agent.md");
    assert!(runners::write_file(&file, "a").unwrap().starts_with("wrote"));
    assert!(runners::write_file(&file, "a").unwrap().starts_with("ok"));
    assert!(runners::write_file(&file, "b").unwrap().starts_with("wrote"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "b");
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
fn cron_parses_fields_and_maps_to_launchd_calendar() {
    let every6 = Cron::parse(" 0  */6 * * * ").unwrap();
    assert_eq!(every6.text, "0 */6 * * *");
    assert_eq!(every6.hour, Some(vec![0, 6, 12, 18]));
    assert_eq!(every6.day, None);
    assert_eq!(
        every6.calendar().unwrap(),
        [0, 6, 12, 18].map(|h| vec![("Minute", 0), ("Hour", h)]).to_vec(),
        "a bare * is left out (launchd wildcard)"
    );
    // Lists, ranges, `a/n`, and Sunday written as 7.
    let mixed = Cron::parse("30 9,21 * 1-3 7").unwrap();
    assert_eq!((mixed.month.clone(), mixed.weekday.clone()), (Some(vec![1, 2, 3]), Some(vec![0])));
    let calendar = mixed.calendar().unwrap();
    assert_eq!(calendar.len(), 6);
    assert!(calendar.iter().all(|e| e.contains(&("Weekday", 0)) && e.contains(&("Minute", 30))));
    assert_eq!(Cron::parse("5/20 * * * *").unwrap().minute, Some(vec![5, 25, 45]));
    // Day and weekday both restricted: cron runs on either match, so each gets its own entries.
    assert_eq!(
        Cron::parse("0 8 1,15 * 1").unwrap().calendar().unwrap(),
        vec![
            vec![("Minute", 0), ("Hour", 8), ("Day", 1)],
            vec![("Minute", 0), ("Hour", 8), ("Day", 15)],
            vec![("Minute", 0), ("Hour", 8), ("Weekday", 1)],
        ]
    );
    // A starred day step with a weekday means both must match, which launchd cannot express.
    assert!(Cron::parse("0 8 */2 * 1").unwrap().calendar().is_err());
    assert!(Cron::parse("*/1 */1 * * *").unwrap().calendar().is_err(), "more entries than launchd should get");
    for bad in [
        "",
        "0 */6 * *",
        "0 */6 * * * *",
        "60 * * * *",
        "0 24 * * *",
        "0 * 0 * *",
        "0 * * 13 *",
        "0 * * * 8",
        "0 */0 * * *",
        "0 5-1 * * *",
        "0 1,,2 * * *",
        "0 * * * MON",
        "@daily",
    ] {
        assert!(Cron::parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn scout_hours_pick_the_runs_that_search_the_web() {
    assert!(schedule::scout_at("0-5", 0).unwrap());
    assert!(schedule::scout_at("0-5", 5).unwrap());
    assert!(!schedule::scout_at("0-5", 6).unwrap());
    assert!(schedule::scout_at("2,14", 14).unwrap());
    assert!(schedule::scout_at("*", 13).unwrap());
    assert!(!schedule::scout_at("", 0).unwrap(), "empty means never");
    assert!(schedule::scout_at("24", 0).is_err());
}

#[test]
fn launchd_plist() {
    let cron = Cron::parse("0 */6 * * *").unwrap();
    let xml = schedule::plist(Path::new("/opt/bin/harness"), Path::new("/repo & co"), &cron, "/usr/bin:/bin").unwrap();
    let label = schedule::label(Path::new("/repo & co"));
    assert!(xml.contains(&format!("<key>Label</key>\n  <string>{label}</string>")));
    assert_eq!(schedule::plist_root(&xml).as_deref(), Some("/repo & co"));
    assert!(xml.contains("<string>/opt/bin/harness</string>\n    <string>autorun</string>"));
    assert!(xml.contains("<key>HARNESS_HOME</key>\n    <string>/repo &amp; co</string>"));
    assert!(xml.contains(
        "<key>StartCalendarInterval</key>\n  <array>\n    <dict>\n      <key>Minute</key>\n      <integer>0</integer>\n      <key>Hour</key>\n      <integer>0</integer>\n    </dict>"
    ));
    assert_eq!(xml.matches("<key>Hour</key>").count(), 4);
    assert!(xml.contains("<string>/repo &amp; co/data/logs/launchd.log</string>"));
    assert_eq!(schedule::plist_cron(&xml).as_deref(), Some("0 */6 * * *"));
    assert!(schedule::plist_path(&label).ends_with(format!("Library/LaunchAgents/{label}.plist")));
    if Path::new("/usr/bin/plutil").exists() {
        let file = scratch("plist").join("job.plist");
        std::fs::write(&file, &xml).unwrap();
        let lint = std::process::Command::new("/usr/bin/plutil").arg("-lint").arg(&file).output().unwrap();
        assert!(lint.status.success(), "{}", String::from_utf8_lossy(&lint.stdout));
    }
}

#[test]
fn launchd_label_is_stable_per_checkout_and_distinct_across_checkouts() {
    let a = schedule::label(Path::new("/Users/me/Contest-Harness"));
    assert_eq!(a, schedule::label(Path::new("/Users/me/Contest-Harness")));
    let suffix = a.strip_prefix(&format!("{}.", schedule::LEGACY_LABEL)).unwrap();
    assert!(suffix.len() == 8 && suffix.bytes().all(|b| b.is_ascii_hexdigit()), "{a}");
    // FNV-1a is fixed by definition: the label must not change between builds or Rust versions.
    assert_eq!(schedule::label(Path::new("")), format!("{}.811c9dc5", schedule::LEGACY_LABEL));
    for other in ["/Users/me/Contest-Harness2", "/Users/you/Contest-Harness", "/Users/me/Contest-Harness/"] {
        assert_ne!(schedule::label(Path::new(other)), a, "{other}");
    }
}

#[test]
fn cron_next_run_follows_cron_day_rules() {
    let at = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
    let next = |cron: &str, after: &str| Cron::parse(cron).unwrap().next_after(at(after));
    // Strictly after: a run at exactly `after` is not the next one.
    assert_eq!(next("0 */6 * * *", "2026-10-15 06:00"), Some(at("2026-10-15 12:00")));
    assert_eq!(next("0 */6 * * *", "2026-10-15 23:59"), Some(at("2026-10-16 00:00")));
    assert_eq!(next("30 9 * * 1", "2026-10-15 10:00"), Some(at("2026-10-19 09:30")), "next Monday");
    // Day and weekday both restricted: either matches (the 20th, or the Friday before it).
    assert_eq!(next("0 9 20 * 5", "2026-10-15 10:00"), Some(at("2026-10-16 09:00")));
    assert_eq!(next("0 0 29 2 *", "2026-03-01 00:00"), Some(at("2028-02-29 00:00")), "leap day");
    assert_eq!(next("0 0 31 2 *", "2026-03-01 00:00"), None, "never runs");
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
