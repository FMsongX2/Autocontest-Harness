//! The omp adapter (reference implementation of [`Runner`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Runner, Spec, run_cli};
use crate::error::{Error, Result};
use crate::paths;

pub struct Omp;

/// `omp --cwd <cwd> [-p] [--config <preset>] [--model M] [--append-system-prompt F] <runner_args>`.
fn base(spec: &Spec, print: bool) -> Vec<String> {
    let mut argv = vec!["omp".to_string(), "--cwd".into(), spec.cwd.display().to_string()];
    if print {
        argv.push("-p".into());
    }
    if let Some(preset) = &spec.preset_file {
        argv.extend(["--config".into(), preset.display().to_string()]);
    }
    if let Some(model) = &spec.model {
        argv.extend(["--model".into(), model.clone()]);
    }
    if let Some(system) = &spec.system_append {
        argv.extend(["--append-system-prompt".into(), system.display().to_string()]);
    }
    argv.extend(spec.extra_args.iter().cloned());
    argv
}

fn command(spec: &Spec, print: bool) -> Vec<String> {
    let mut argv = base(spec, print);
    argv.push(format!("@{}", spec.prompt_file.display()));
    argv.push(spec.first_message.clone());
    argv
}

/// omp keeps sessions under `<sessions>/<encoded cwd>/<stamp>_<uuid>.jsonl`; the `session` line
/// near the top carries `id` and `cwd`. The ids of the sessions whose `cwd` is `cwd`, newest first.
/// Every project directory is scanned instead of re-deriving omp's directory-name encoding.
pub fn session_ids_in(sessions: &Path, cwd: &Path) -> Vec<String> {
    let want = super::canonical(cwd);
    let mut found: Vec<(String, String)> = Vec::new();
    for file in super::session_files(sessions) {
        let header = super::head_objects(&file, 5).into_iter().find(|o| o["type"] == "session");
        let Some(header) = header else { continue };
        let (Some(id), Some(dir)) = (header["id"].as_str(), header["cwd"].as_str()) else { continue };
        if super::valid_session_id(id)
            && super::canonical(Path::new(dir)) == want
            && !found.iter().any(|(i, _)| i == id)
        {
            let name = file.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
            found.push((id.to_string(), name));
        }
    }
    // File names start with the creation time, so name order is age order.
    found.sort_by(|a, b| b.1.cmp(&a.1));
    found.into_iter().map(|(id, _)| id).collect()
}

/// `modelRoles.default` of a preset overlay written by [`preset_overlay`].
fn preset_default_model(path: &Path) -> Option<String> {
    let overlay: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    overlay["modelRoles"]["default"].as_str().filter(|m| !m.is_empty()).map(str::to_string)
}

/// `config_yml`'s `modelPresets.<name>` as a `--config` overlay for one run, written to `target`.
pub fn preset_overlay(name: &str, config_yml: &Path, target: &Path) -> Result<PathBuf> {
    let text = std::fs::read_to_string(config_yml).map_err(|e| Error(format!("{}: {e}", config_yml.display())))?;
    let config: Value = serde_saphyr::from_str(&text).map_err(|e| Error(format!("{}: {e}", config_yml.display())))?;
    let preset = config.get("modelPresets").and_then(|p| p.get(name)).filter(|p| !p.is_null());
    let Some(preset) = preset else { crate::bail!("omp 프리셋이 없다: {name}") };
    let mut overlay = json!({"modelRoles": preset.get("modelRoles").cloned().unwrap_or_else(|| json!({}))});
    if let Some(level) = preset.get("defaultThinkingLevel").filter(|v| !v.is_null() && v.as_str() != Some("")) {
        overlay["defaultThinkingLevel"] = level.clone();
    }
    // JSON is valid YAML, and omp reads the overlay as YAML.
    std::fs::write(target, serde_json::to_string_pretty(&overlay)? + "\n")?;
    Ok(target.to_path_buf())
}

/// Whether `link` is a symlink whose target lies inside `root` (the old per-file install).
fn links_into(link: &Path, root: &Path) -> bool {
    let Ok(target) = std::fs::read_link(link) else { return false };
    let target = if target.is_absolute() { target } else { link.parent().unwrap_or(Path::new("/")).join(target) };
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let target = std::fs::canonicalize(&target).unwrap_or(target);
    target.starts_with(&root)
}

/// The symlinks the Python harness used to install into `~/.omp/agent`.
pub fn legacy_links(agent_dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(agent_dir.join("agents"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            name.ends_with(".md")
                && (name.starts_with("Contest")
                    || ["DomainResearcher.md", "ReferenceScout.md", "UxReviewer.md"].contains(&name))
        })
        .collect();
    found.sort();
    found.push(agent_dir.join("skills").join("contest-harness"));
    found
}

/// Remove legacy symlinks under `agent_dir` that point into `root`.
pub fn remove_legacy(agent_dir: &Path, root: &Path) -> Result<Vec<String>> {
    let mut done = Vec::new();
    for link in legacy_links(agent_dir) {
        if std::fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_symlink()) && links_into(&link, root) {
            std::fs::remove_file(&link)?;
            done.push(format!("removed {}", link.display()));
        }
    }
    Ok(done)
}

impl Runner for Omp {
    fn name(&self) -> &'static str {
        "omp"
    }

    fn binary(&self) -> &'static str {
        "omp"
    }

    fn subagents(&self) -> bool {
        true
    }

    /// omp `modelPresets` (model-role sets in `~/.omp/agent/config.yml`).
    fn presets(&self) -> bool {
        true
    }

    fn preset_file(&self, name: &str, dir: &Path) -> Result<PathBuf> {
        preset_overlay(name, &paths::omp_agent_dir().join("config.yml"), &dir.join("omp-preset.yml"))
    }

    fn orca_agent(&self) -> Option<&'static str> {
        Some("omp")
    }

    fn interactive(&self, spec: &Spec) -> Result<Vec<String>> {
        Ok(command(spec, false))
    }

    fn oneshot(&self, spec: &Spec) -> Result<Vec<String>> {
        Ok(command(spec, true))
    }

    fn session_records(&self, cwd: &Path) -> Option<Vec<String>> {
        Some(session_ids_in(&paths::omp_agent_dir().join("sessions"), cwd))
    }

    /// `--restore <id>` reloads the session and finishes what its last process left unfinished.
    /// omp refuses a prompt with `--restore`, so `message` is not delivered.
    fn restore(&self, spec: &Spec, session_id: &str, _message: &str, interactive: bool) -> Result<Vec<String>> {
        if !super::valid_session_id(session_id) {
            crate::bail!("세션 id가 UUID가 아니다: {session_id}");
        }
        let mut argv = base(spec, !interactive);
        // A restored session keeps the model it last used; the preset overlay alone does not move
        // it, so the preset's default role is named explicitly.
        if spec.model.is_none()
            && let Some(model) = spec.preset_file.as_deref().and_then(preset_default_model)
        {
            argv.extend(["--model".into(), model]);
        }
        argv.extend(["--restore".into(), session_id.to_string()]);
        Ok(argv)
    }

    fn restore_delivers_message(&self) -> bool {
        false
    }

    /// `omp plugin link` loads the extension, skills, commands and agents from the package; the old
    /// per-file symlinks would shadow them, so they go.
    fn install(&self, root: &Path) -> Result<Vec<String>> {
        let root_text = root.display().to_string();
        let out = run_cli("omp", &["plugin", "link", &root_text])?;
        let mut done = vec![format!("omp plugin link {root_text}: {}", out.lines().last().unwrap_or("ok"))];
        done.extend(remove_legacy(&paths::omp_agent_dir(), root)?);
        Ok(done)
    }

    /// `omp plugin list --json` has an enabled `contest-harness` package that resolves to `root`.
    fn installed(&self, root: &Path) -> Option<bool> {
        let out = crate::process::output("omp", &["plugin", "list", "--json"], None, crate::process::LOCAL).ok()?;
        if !out.status.success() {
            return None;
        }
        let list: Value = serde_json::from_slice(&out.stdout).ok()?;
        let root = std::fs::canonicalize(root).ok()?;
        let linked = list["npm"].as_array()?.iter().any(|p| {
            p["name"] == "contest-harness"
                && p["enabled"] != false
                && p["path"].as_str().and_then(|path| std::fs::canonicalize(path).ok()).is_some_and(|p| p == root)
        });
        Some(linked)
    }

    /// omp reads per-agent models from `task.agentModelOverrides` in its global config; only the
    /// harness agents named in `models` are touched, every other override stays.
    fn apply_agent_models(&self, models: &BTreeMap<String, Option<String>>) -> Result<Vec<String>> {
        if models.is_empty() {
            return Ok(Vec::new());
        }
        let current = overrides()?;
        let merged = merge_overrides(&current, models);
        if merged == current {
            return Ok(vec![format!("ok {OVERRIDES} ({}개)", models.len())]);
        }
        let value = serde_json::to_string(&merged)?;
        run_cli("omp", &["config", "set", OVERRIDES, &value])?;
        let changed: Vec<&str> = models.keys().map(String::as_str).collect();
        Ok(vec![format!("omp config set {OVERRIDES}: {}", changed.join(", "))])
    }

    fn agent_models(&self) -> Option<BTreeMap<String, String>> {
        let current = overrides().ok()?;
        Some(current.into_iter().filter_map(|(name, model)| model.as_str().map(|m| (name, m.to_string()))).collect())
    }
}

/// The omp setting that maps an agent name to the model it runs on.
pub const OVERRIDES: &str = "task.agentModelOverrides";

/// The current `task.agentModelOverrides` record.
fn overrides() -> Result<serde_json::Map<String, Value>> {
    let out = crate::process::output("omp", &["config", "get", OVERRIDES, "--json"], None, crate::process::LOCAL)?;
    if !out.status.success() {
        crate::bail!("omp config get {OVERRIDES} 실패: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let answer: Value = serde_json::from_slice(&out.stdout)?;
    Ok(answer.get("value").and_then(Value::as_object).cloned().unwrap_or_default())
}

/// `current` with each named agent set (`Some`) or removed (`None`); other agents are untouched.
pub fn merge_overrides(
    current: &serde_json::Map<String, Value>,
    models: &BTreeMap<String, Option<String>>,
) -> serde_json::Map<String, Value> {
    let mut merged = current.clone();
    for (name, model) in models {
        match model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
            Some(model) => {
                merged.insert(name.clone(), json!(model));
            }
            None => {
                merged.remove(name);
            }
        }
    }
    merged
}
