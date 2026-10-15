//! `data/config.json` laid over built-in defaults. Map-valued keys (per-level tables, runner args,
//! persona) are merged key by key, so a config that only sets `{"max_running_by_level": {"5": 0}}`
//! keeps the defaults for the other levels.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::bail;
use crate::error::{Error, Result};
use crate::registry::LEVELS;
use crate::{paths, runners, schedule, workspaces};

const MERGED_KEYS: &[&str] = &[
    "preset_by_level",
    "model_by_level",
    "max_running_by_level",
    "ram_gb_by_level",
    "runner_args",
    "persona",
    "schedule",
    "agent_models",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Persona {
    pub enabled: bool,
    /// The persona text (system-prompt addition for the main session): relative to the repository
    /// root, or `~`/absolute. Personal, so the default lives under git-ignored `data/`.
    pub file: String,
    /// How the persona addresses the user; replaces `{{call_user}}` in the persona file.
    pub call_user: String,
    /// Persona traces that must never appear in contest deliverables or commit messages
    /// (`call_user` is checked too). `harness check` and the `awaiting_approval` gate scan for them.
    pub guard_terms: Vec<String>,
}

/// When scheduled runs happen (`harness schedule`), all in the machine's local time.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    /// 5-field cron expression for the scheduled run (collection, gating, launching, cleanup).
    pub cron: String,
    /// What starts the run: one of `schedule::SCHEDULERS` (`orca` automation, or `launchd`/`cron`
    /// running `harness autorun`).
    pub scheduler: String,
    /// Hours (a cron hour field such as `0-5`; empty = never) in which a scheduled run also runs
    /// the ContestScout web search. `harness tick` reports the decision as `scout`.
    pub scout_hours: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub github_org: String,
    pub contests_root: String,
    pub orca_repo_path: Option<String>,
    pub runner: String,
    pub workspace: String,
    pub preset_by_level: BTreeMap<u8, Option<String>>,
    pub model_by_level: BTreeMap<u8, String>,
    pub runner_args: BTreeMap<String, Vec<String>>,
    pub max_running: usize,
    pub max_running_by_level: BTreeMap<u8, usize>,
    pub ram_reserve_gb: f64,
    pub ram_gb_by_level: BTreeMap<u8, f64>,
    pub ram_gb_per_worker: f64,
    pub max_workers: usize,
    pub primary_provider: String,
    pub collect_pages: u32,
    pub collect_skip: Vec<String>,
    pub auto_launch: bool,
    pub gate_batch: usize,
    pub persona: Persona,
    pub schedule: Schedule,
    /// Model per harness agent (`agents/<Name>.md`), in the runner's selector syntax (omp:
    /// `provider/model:effort` or `@role`). `harness install` applies it; `null` removes a choice
    /// the harness applied before, so the agent's own default role applies again.
    pub agent_models: BTreeMap<String, Option<String>>,
}

pub fn defaults() -> Value {
    json!({
        "github_org": "",
        "contests_root": "~/contests",
        "orca_repo_path": null,
        "runner": "omp",
        "workspace": "orca",
        "preset_by_level": {"1": null, "2": null, "3": null, "4": null, "5": null},
        "model_by_level": {},
        "runner_args": {"omp": [], "claude": ["--permission-mode", "auto"]},
        "max_running": 6,
        "max_running_by_level": {"1": 3, "2": 2, "3": 2, "4": 1, "5": 1},
        "ram_reserve_gb": 8,
        "ram_gb_by_level": {"1": 2, "2": 3, "3": 4, "4": 6, "5": 8},
        "ram_gb_per_worker": 1.5,
        "max_workers": 6,
        "primary_provider": "anthropic",
        "collect_pages": 3,
        "collect_skip": [],
        "auto_launch": false,
        "gate_batch": 16,
        "persona": {"enabled": false, "file": "data/persona.md", "call_user": "", "guard_terms": []},
        "schedule": {"cron": "0 */6 * * *", "scheduler": "orca", "scout_hours": "0-5"},
        "agent_models": {}
    })
}

/// Overlay `file` onto `base`. Keys in [`MERGED_KEYS`] merge one level deep; the rest replace.
pub fn merge(mut base: Value, file: Value) -> Value {
    if let (Some(base_map), Value::Object(file_map)) = (base.as_object_mut(), file) {
        for (key, value) in file_map {
            match (base_map.get_mut(&key), value) {
                (Some(Value::Object(current)), Value::Object(update)) if MERGED_KEYS.contains(&key.as_str()) => {
                    current.extend(update);
                }
                (_, value) => {
                    base_map.insert(key, value);
                }
            }
        }
    }
    base
}

pub fn load() -> Result<Config> {
    load_at(&paths::config_file())
}

/// `path` (a missing file means the defaults) laid over the defaults and validated. Every error
/// names the file.
pub fn load_at(path: &Path) -> Result<Config> {
    let at = |err: &dyn std::fmt::Display| Error(format!("{}: {err}", path.display()));
    let overlay = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|err| at(&err))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(err) => return Err(at(&err)),
    };
    if !overlay.is_object() {
        return Err(at(&"설정 파일은 JSON 객체여야 한다"));
    }
    let config: Config = serde_json::from_value(merge(defaults(), overlay)).map_err(|err| at(&err))?;
    validate(&config).map_err(|err| at(&err))?;
    Ok(config)
}

/// The overlay as written in `path` (`{}` when missing), without defaults.
fn read_overlay(path: &Path) -> Result<serde_json::Map<String, Value>> {
    let at = |err: &dyn std::fmt::Display| Error(format!("{}: {err}", path.display()));
    match crate::store::read(path)? {
        None => Ok(serde_json::Map::new()),
        Some(text) => match serde_json::from_str(&text).map_err(|err| at(&err))? {
            Value::Object(map) => Ok(map),
            _ => Err(at(&"설정 파일은 JSON 객체여야 한다")),
        },
    }
}

/// Validate `overlay` as a whole config and write it to `path` (0600, atomic). Nothing is written
/// when the result would not load.
fn save_overlay(path: &Path, overlay: serde_json::Map<String, Value>) -> Result<Value> {
    let overlay = Value::Object(overlay);
    let at = |err: &dyn std::fmt::Display| Error(format!("{}: {err}", path.display()));
    let config: Config = serde_json::from_value(merge(defaults(), overlay.clone())).map_err(|err| at(&err))?;
    validate(&config).map_err(|err| at(&err))?;
    crate::store::write_atomic(path, (serde_json::to_string_pretty(&overlay)? + "\n").as_bytes(), false)?;
    Ok(overlay)
}

/// A dotted key (`schedule.cron`, `agent_models.ContestJudge`) as path segments.
fn segments(key: &str) -> Result<Vec<&str>> {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.iter().any(|p| p.is_empty()) {
        bail!("설정 키가 올바르지 않다: {key:?}");
    }
    Ok(parts)
}

/// Set `key` to `value` in the config file at `path` (`harness config set`). Objects on the way are
/// created; the whole config must still validate.
pub fn set_at(path: &Path, key: &str, value: Value) -> Result<Value> {
    let parts = segments(key)?;
    let mut overlay = read_overlay(path)?;
    let (last, parents) = parts.split_last().unwrap_or((&"", &[]));
    let mut node = &mut overlay;
    for part in parents {
        let entry = node.entry(part.to_string()).or_insert_with(|| json!({}));
        if !entry.is_object() {
            *entry = json!({});
        }
        let Value::Object(map) = entry else { bail!("{key}: 객체가 아닌 값 아래에 쓸 수 없다") };
        node = map;
    }
    node.insert(last.to_string(), value);
    save_overlay(path, overlay)
}

/// Remove `key` from the config file at `path`, so its default applies again (`harness config unset`).
pub fn unset_at(path: &Path, key: &str) -> Result<Value> {
    let parts = segments(key)?;
    let mut overlay = read_overlay(path)?;
    let (last, parents) = parts.split_last().unwrap_or((&"", &[]));
    let mut node = Some(&mut overlay);
    for part in parents {
        node = node.and_then(|map| map.get_mut(*part)).and_then(Value::as_object_mut);
    }
    if node.and_then(|map| map.remove(*last)).is_none() {
        bail!("{}에 {key}가 없다", path.display());
    }
    save_overlay(path, overlay)
}

/// The effective value of `key` (defaults + file), or the whole effective config without a key.
pub fn get_at(path: &Path, key: Option<&str>) -> Result<Value> {
    let config = serde_json::to_value(load_at(path)?)?;
    let Some(key) = key else { return Ok(config) };
    let mut node = &config;
    for part in segments(key)? {
        node = node.get(part).ok_or_else(|| Error(format!("알 수 없는 설정 키: {key}")))?;
    }
    Ok(node.clone())
}

/// Values serde cannot check: names of runners, workspaces and schedulers, the cron expression,
/// per-level keys and numeric ranges.
pub fn validate(config: &Config) -> Result<()> {
    let one_of = |key: &str, value: &str, names: &[&str]| -> Result<()> {
        if names.contains(&value) {
            Ok(())
        } else {
            bail!("{key}는 {} 중 하나여야 한다: {value:?}", names.join(", "))
        }
    };
    one_of("runner", &config.runner, &runners::names())?;
    one_of("workspace", &config.workspace, workspaces::NAMES)?;
    one_of("schedule.scheduler", &config.schedule.scheduler, schedule::SCHEDULERS)?;
    schedule::Cron::parse(&config.schedule.cron).map_err(|err| Error(format!("schedule.cron: {err}")))?;
    if !config.schedule.scout_hours.trim().is_empty() {
        schedule::parse_field(&config.schedule.scout_hours, 0, 23)
            .map_err(|err| Error(format!("schedule.scout_hours: {err}")))?;
    }
    let levels = [
        ("preset_by_level", config.preset_by_level.keys().copied().collect::<Vec<_>>()),
        ("model_by_level", config.model_by_level.keys().copied().collect()),
        ("max_running_by_level", config.max_running_by_level.keys().copied().collect()),
        ("ram_gb_by_level", config.ram_gb_by_level.keys().copied().collect()),
    ];
    for (key, keys) in levels {
        if let Some(bad) = keys.iter().find(|l| !LEVELS.contains(l)) {
            bail!("{key}의 키는 레벨 1~5만 쓴다: {bad}");
        }
    }
    let gb = [("ram_reserve_gb", config.ram_reserve_gb)]
        .into_iter()
        .chain(config.ram_gb_by_level.values().map(|v| ("ram_gb_by_level", *v)));
    for (key, value) in gb {
        if !value.is_finite() || value < 0.0 {
            bail!("{key}는 0 이상의 숫자여야 한다: {value}");
        }
    }
    if !config.ram_gb_per_worker.is_finite() || config.ram_gb_per_worker <= 0.0 {
        bail!("ram_gb_per_worker는 0보다 큰 숫자여야 한다: {}", config.ram_gb_per_worker);
    }
    if config.collect_pages == 0 {
        bail!("collect_pages는 1 이상이어야 한다");
    }
    Ok(())
}

impl Config {
    /// The folder that holds one sub-folder per contest.
    pub fn contests_root(&self) -> PathBuf {
        paths::expand(&self.contests_root)
    }

    pub fn shared_dir(&self) -> PathBuf {
        self.contests_root().join(".shared")
    }

    /// One line per contest folder (`- Folder: title`), read by dedupe and appended by route.
    pub fn existing_contests(&self) -> PathBuf {
        self.shared_dir().join("existing_contests.md")
    }

    /// The git repository Orca opens workspaces in (defaults to the contests root).
    pub fn orca_repo_path(&self) -> PathBuf {
        self.orca_repo_path.as_deref().map(paths::expand).unwrap_or_else(|| self.contests_root())
    }

    pub fn runner_args(&self, runner: &str) -> Vec<String> {
        self.runner_args.get(runner).cloned().unwrap_or_default()
    }
}
