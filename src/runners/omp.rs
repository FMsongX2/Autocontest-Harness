//! The omp adapter (reference implementation of [`Runner`]).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Runner, Spec, run_cli};
use crate::error::{Error, Result};
use crate::paths;

pub struct Omp;

fn command(spec: &Spec, print: bool) -> Vec<String> {
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
    argv.push(format!("@{}", spec.prompt_file.display()));
    argv.push(spec.first_message.clone());
    argv
}

/// `config_yml`'s `modelPresets.<name>` as a `--config` overlay for one run, written to `target`.
pub fn preset_overlay(name: &str, config_yml: &Path, target: &Path) -> Result<PathBuf> {
    let text = std::fs::read_to_string(config_yml).map_err(|e| Error(format!("{}: {e}", config_yml.display())))?;
    let config: Value = serde_yaml_ng::from_str(&text)?;
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

    /// `omp plugin link` loads the extension, skills, commands and agents from the package; the old
    /// per-file symlinks would shadow them, so they go.
    fn install(&self, root: &Path) -> Result<Vec<String>> {
        let root_text = root.display().to_string();
        let out = run_cli("omp", &["plugin", "link", &root_text])?;
        let mut done = vec![format!("omp plugin link {root_text}: {}", out.lines().last().unwrap_or("ok"))];
        done.extend(remove_legacy(&paths::omp_agent_dir(), root)?);
        Ok(done)
    }
}
