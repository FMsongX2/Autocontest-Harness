//! `data/config.json` laid over built-in defaults. Map-valued keys (per-level tables, runner args,
//! persona) are merged key by key, so a config that only sets `{"max_running_by_level": {"5": 0}}`
//! keeps the defaults for the other levels.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::Result;
use crate::paths;

const MERGED_KEYS: &[&str] = &[
    "preset_by_level",
    "model_by_level",
    "max_running_by_level",
    "ram_gb_by_level",
    "runner_args",
    "persona",
    "schedule",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
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
pub struct Schedule {
    /// 5-field cron expression for the scheduled run (collection, gating, launching, cleanup).
    pub cron: String,
    /// What starts the run: `orca` (an Orca automation) or `launchd` (`harness autorun`).
    pub scheduler: String,
    /// Hours (a cron hour field such as `0-5`; empty = never) in which a scheduled run also runs
    /// the ContestScout web search. `harness tick` reports the decision as `scout`.
    pub scout_hours: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
        "runner_args": {"omp": []},
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
        "schedule": {"cron": "0 */6 * * *", "scheduler": "orca", "scout_hours": "0-5"}
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
    let file = paths::config_file();
    let overlay = if file.exists() { serde_json::from_str(&std::fs::read_to_string(&file)?)? } else { json!({}) };
    Ok(serde_json::from_value(merge(defaults(), overlay))?)
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
