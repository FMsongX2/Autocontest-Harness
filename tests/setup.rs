//! Setup: `harness init` scaffolding, `harness config` edits, per-agent model choices.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use harness::config;
use harness::runners::omp;
use serde_json::{Value, json};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("harness-setup-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn config_set_validates_before_writing_and_unset_restores_the_default() {
    let dir = scratch("config");
    let file = dir.join("config.json");

    config::set_at(&file, "schedule.cron", json!("0 9,21 * * *")).unwrap();
    config::set_at(&file, "auto_launch", json!(true)).unwrap();
    config::set_at(&file, "model_by_level.3", json!("provider/model:high")).unwrap();
    config::set_at(&file, "agent_models.ContestJudge", json!("provider/strong:high")).unwrap();
    assert_eq!(config::get_at(&file, Some("schedule.cron")).unwrap(), "0 9,21 * * *");
    // Setting one key of a map keeps its other keys (from the file and from the defaults).
    assert_eq!(config::get_at(&file, Some("schedule.scheduler")).unwrap(), "orca");
    assert_eq!(config::get_at(&file, Some("auto_launch")).unwrap(), true);
    assert_eq!(config::get_at(&file, Some("agent_models.ContestJudge")).unwrap(), "provider/strong:high");
    assert_eq!(mode(&file), 0o600);

    // Invalid values, unknown keys and malformed keys are refused and leave the file as it was.
    let before = std::fs::read(&file).unwrap();
    for (key, value) in [
        ("schedule.cron", json!("0 25 * * *")),
        ("workspace", json!("screen")),
        ("workspce", json!("tmux")),
        ("max_running", json!("six")),
        ("model_by_level.9", json!("x")),
        ("schedule..cron", json!("0 1 * * *")),
    ] {
        assert!(config::set_at(&file, key, value.clone()).is_err(), "{key} = {value}");
        assert_eq!(std::fs::read(&file).unwrap(), before, "{key} changed the file");
    }

    // `null` keeps the key in the file: `harness install` reads it as "remove that override".
    config::set_at(&file, "agent_models.ContestJudge", Value::Null).unwrap();
    assert_eq!(config::load_at(&file).unwrap().agent_models.get("ContestJudge"), Some(&None));

    config::unset_at(&file, "schedule.cron").unwrap();
    assert_eq!(config::get_at(&file, Some("schedule.cron")).unwrap(), "0 */6 * * *");
    assert!(config::unset_at(&file, "schedule.cron").is_err(), "nothing left to unset");
    assert!(config::get_at(&file, Some("no_such_key")).is_err());
}

#[test]
fn omp_overrides_change_only_the_named_agents() {
    let current = json!({"reviewer": "x/review", "ContestJudge": "x/old", "ReferenceScout": "x/ref"});
    let current = current.as_object().unwrap().clone();
    let models: BTreeMap<String, Option<String>> = [
        ("ContestJudge", Some("x/new:high")),
        ("ContestGate", Some("x/gate")),
        ("ReferenceScout", None),
        ("OpenSourceScout", Some("  ")),
    ]
    .into_iter()
    .map(|(name, model)| (name.to_string(), model.map(str::to_string)))
    .collect();
    let merged = omp::merge_overrides(&current, &models);
    assert_eq!(
        Value::Object(merged),
        json!({"reviewer": "x/review", "ContestJudge": "x/new:high", "ContestGate": "x/gate"}),
        "a blank or null choice removes the harness override; other agents' overrides stay"
    );
}

#[test]
fn agent_models_must_name_real_agents() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut value = config::defaults();
    value["agent_models"] = json!({"ContestJudge": "x/y", "ContestJugde": "x/y"});
    let cfg: config::Config = serde_json::from_value(value).unwrap();
    assert_eq!(harness::install::unknown_agents(&cfg, &root).unwrap(), vec!["ContestJugde".to_string()]);
}

/// A checkout with the repository's prompts and agents and its own empty `data/`.
fn checkout(name: &str) -> PathBuf {
    let base = scratch(name);
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = base.join("checkout");
    std::fs::create_dir_all(root.join("data")).unwrap();
    for dir in ["harness", "agents", "skills"] {
        std::os::unix::fs::symlink(repo.join(dir), root.join(dir)).unwrap();
    }
    for example in ["config.example.json", "profile.example.md"] {
        std::fs::copy(repo.join("data").join(example), root.join("data").join(example)).unwrap();
    }
    root
}

#[test]
fn init_creates_private_files_once_and_never_overwrites() {
    let root = checkout("init");
    let run = || {
        let out = Command::new(env!("CARGO_BIN_EXE_harness"))
            .args(["init", "--json"])
            .env("HARNESS_HOME", &root)
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    let first = run();
    let created: Vec<&str> = first["created"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(created.iter().any(|p| p.ends_with("data/config.json")), "{first}");
    assert!(created.iter().any(|p| p.ends_with("data/profile.md")), "{first}");
    assert_eq!(mode(&root.join("data/config.json")), 0o600);
    assert_eq!(mode(&root.join("data/profile.md")), 0o600);
    assert!(first["checks"].as_array().is_some_and(|c| !c.is_empty()));
    assert!(first["next"].as_str().unwrap().ends_with("INIT.md"));

    // The user's edits survive a second run, which creates nothing.
    std::fs::write(root.join("data/config.json"), "{\"workspace\": \"tmux\"}\n").unwrap();
    let second = run();
    assert_eq!(second["created"], json!([]), "{second}");
    assert_eq!(std::fs::read_to_string(root.join("data/config.json")).unwrap(), "{\"workspace\": \"tmux\"}\n");
}
