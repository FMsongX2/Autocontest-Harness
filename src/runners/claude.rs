//! The Claude Code adapter (`claude` CLI).
//!
//! Flags were checked against `claude --help` of Claude Code 2.1.296 and the docs at
//! code.claude.com. Claude Code has no working-directory flag (workspaces already `cd`) and no
//! system-prompt *file* flag for interactive sessions, so the instructions travel as an `@path`
//! mention and the system addition as text. Per-agent models work through the `model` field of
//! each subagent file this adapter writes; Claude Code has no settings key that sets a model by
//! agent name.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{AgentDoc, Runner, Spec, agent_docs, link, prompt_text, system_text, write_file};
use crate::error::Result;
use crate::paths;

pub struct Claude;

/// The `model` every converted agent starts with: the model of the session that spawns it.
pub const INHERIT: &str = "inherit";

/// `claude [-p] [--model M] [--append-system-prompt TEXT] <runner_args> -- "@<instructions> <first message>"`.
/// Claude Code reads an `@path` mention into the conversation (checked in print and interactive
/// mode), which keeps the command line short for Orca and tmux; a path with whitespace, which the
/// mention syntax cannot carry, falls back to the instructions as text. There is no system-prompt
/// file flag for interactive sessions, so the system addition travels as text. `--` ends the
/// options: list options (`--add-dir`, `--tools`, ...) would otherwise swallow the prompt.
fn command(spec: &Spec, print: bool) -> Result<Vec<String>> {
    let mut argv = options(spec, print)?;
    argv.push("--".into());
    let path = spec.prompt_file.display().to_string();
    if path.chars().any(char::is_whitespace) {
        argv.push(prompt_text(spec)?);
    } else {
        argv.push(format!("@{path} {}", spec.first_message));
    }
    Ok(argv)
}

/// `claude [-p] [--model M] [--append-system-prompt TEXT] <runner_args>`.
fn options(spec: &Spec, print: bool) -> Result<Vec<String>> {
    let mut argv = vec!["claude".to_string()];
    if print {
        argv.push("-p".into());
    }
    if let Some(model) = &spec.model {
        argv.extend(["--model".into(), model.clone()]);
    }
    if let Some(text) = system_text(spec)? {
        argv.extend(["--append-system-prompt".into(), text]);
    }
    argv.extend(spec.extra_args.iter().cloned());
    Ok(argv)
}

/// Claude Code keeps each conversation as `<home>/.claude/projects/<encoded cwd>/<uuid>.jsonl`, whose
/// lines carry `sessionId` and `cwd`. The ids of the conversations started in `cwd`, newest first.
pub fn session_ids_at(home: &Path, cwd: &Path) -> Vec<String> {
    let want = super::canonical(cwd);
    let mut found: Vec<(std::time::SystemTime, String)> = Vec::new();
    for file in super::session_files(&home.join(".claude").join("projects")) {
        let Some(stem) = file.file_stem().and_then(|s| s.to_str()).filter(|s| super::valid_session_id(s)) else {
            continue;
        };
        let matches = super::head_objects(&file, 30).iter().any(|o| {
            o["sessionId"].as_str() == Some(stem)
                && o["cwd"].as_str().is_some_and(|dir| super::canonical(Path::new(dir)) == want)
        });
        if matches && !found.iter().any(|(_, id)| id == stem) {
            let modified = file.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            found.push((modified, stem.to_string()));
        }
    }
    found.sort_by_key(|a| std::cmp::Reverse(a.0));
    found.into_iter().map(|(_, id)| id).collect()
}

/// omp tool names -> Claude Code tool names. Tools Claude Code lacks (`spawns`, ...) are dropped.
/// `todo` becomes the task-list tools, which exist only on the models Claude Code enables them for.
fn tool_names(omp: &str) -> &'static [&'static str] {
    match omp {
        "read" => &["Read"],
        "write" => &["Write"],
        "edit" => &["Edit"],
        "bash" => &["Bash"],
        "glob" => &["Glob"],
        "grep" => &["Grep"],
        "web_search" => &["WebSearch", "WebFetch"],
        "fetch" => &["WebFetch"],
        "task" => &["Agent"],
        "todo" => &["TaskCreate", "TaskGet", "TaskList", "TaskUpdate"],
        _ => &[],
    }
}

/// A model selector that is safe to write as a plain YAML scalar: an alias (`opus`), a full id
/// (`claude-opus-5-5`, `anthropic.claude-x-v1:0`), or `inherit`.
pub fn valid_model(model: &str) -> bool {
    !model.is_empty()
        && !model.starts_with(['-', '[', ':'])
        && model.chars().all(|c| c.is_ascii_alphanumeric() || "-_.:/[]".contains(c))
}

/// One omp agent as a Claude Code subagent file whose model is `model`.
pub fn convert_agent(doc: &AgentDoc, model: &str) -> String {
    let mut tools: Vec<&str> = Vec::new();
    for tool in doc.tools.iter().flat_map(|t| tool_names(t)) {
        if !tools.contains(tool) {
            tools.push(tool);
        }
    }
    let quote = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let mut out = format!("---\nname: {}\ndescription: {}\n", quote(&doc.name), quote(&doc.description));
    if !tools.is_empty() {
        out.push_str(&format!("tools: {}\n", tools.join(", ")));
    }
    out.push_str(&format!("model: {model}\n"));
    if !doc.skills.is_empty() {
        out.push_str("skills:\n");
        for skill in &doc.skills {
            out.push_str(&format!("  - {}\n", quote(skill)));
        }
    }
    out.push_str("---\n\n");
    out.push_str(&doc.instructions());
    out
}

/// The `model:` value of an agent file's frontmatter.
pub fn read_model(file: &str) -> Option<String> {
    let rest = file.strip_prefix("---\n")?;
    let head = &rest[..rest.find("\n---")?];
    head.lines().find_map(|l| l.strip_prefix("model:")).map(|m| m.trim().to_string())
}

/// `file` with its frontmatter `model:` line set to `model`; `None` when there is no such line.
pub fn set_model(file: &str, model: &str) -> Option<String> {
    let end = file.strip_prefix("---\n").and_then(|rest| rest.find("\n---").map(|i| i + 4))?;
    let mut out = String::new();
    let mut found = false;
    for (n, line) in file.split_inclusive('\n').enumerate() {
        let in_head = out.len() < end && n > 0;
        if in_head && !found && line.starts_with("model:") {
            out.push_str(&format!("model: {model}\n"));
            found = true;
        } else {
            out.push_str(line);
        }
    }
    found.then_some(out)
}

/// The directory of the agent files this adapter writes. Claude Code scans `agents/` recursively
/// and identifies an agent by its `name`, so a subfolder keeps ours apart from the user's own.
fn agents_dir(home: &Path) -> PathBuf {
    home.join(".claude").join("agents").join("contest-harness")
}

/// Agents to `<home>/.claude/agents/contest-harness/`, the skill to `<home>/.claude/skills/`.
/// A model already chosen in an installed agent file (not `inherit`) is kept, so a re-install does
/// not undo `apply_agent_models`.
pub fn install_at(root: &Path, home: &Path) -> Result<Vec<String>> {
    let dir = agents_dir(home);
    let docs = agent_docs(root)?;
    let mut done = Vec::new();
    let keep: Vec<String> = docs.iter().map(|d| format!("{}.md", d.name)).collect();
    let mut stale: Vec<PathBuf> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .filter(|p| !keep.contains(&p.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string()))
        .collect();
    stale.sort();
    for old in stale {
        std::fs::remove_file(&old)?;
        done.push(format!("removed {}", old.display()));
    }
    for doc in &docs {
        let path = dir.join(format!("{}.md", doc.name));
        let model = std::fs::read_to_string(&path).ok().and_then(|f| read_model(&f)).filter(|m| valid_model(m));
        done.push(write_file(&path, &convert_agent(doc, model.as_deref().unwrap_or(INHERIT)))?);
    }
    done.push(link(
        &root.join("skills").join("contest-harness"),
        &home.join(".claude").join("skills").join("contest-harness"),
    )?);
    Ok(done)
}

/// Whether the agent files match `agents/*.md` (their models aside), no other `.md` is left, and
/// the skill link points into `root`.
pub fn installed_at(root: &Path, home: &Path) -> Option<bool> {
    let dir = agents_dir(home);
    let docs = agent_docs(root).ok()?;
    let mut files = 0;
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        if entry.path().extension().is_some_and(|x| x == "md") {
            files += 1;
        }
    }
    let agents_ok = files == docs.len()
        && docs.iter().all(|doc| {
            let Ok(file) = std::fs::read_to_string(dir.join(format!("{}.md", doc.name))) else { return false };
            let model = read_model(&file).unwrap_or_default();
            valid_model(&model) && convert_agent(doc, &model) == file
        });
    let skill = home.join(".claude").join("skills").join("contest-harness");
    let skill_ok = std::fs::symlink_metadata(&skill).is_ok_and(|m| m.file_type().is_symlink())
        && std::fs::canonicalize(&skill).ok()
            == std::fs::canonicalize(root.join("skills").join("contest-harness")).ok();
    Some(agents_ok && skill_ok)
}

/// Set (`Some`) or reset to `inherit` (`None`) the model of each named installed agent file.
/// Returns one line per agent. Other agents and every other setting stay as they are.
pub fn apply_models_at(home: &Path, models: &BTreeMap<String, Option<String>>) -> Result<Vec<String>> {
    let dir = agents_dir(home);
    let mut done = Vec::new();
    for (name, model) in models {
        let model = model.as_deref().map(str::trim).filter(|m| !m.is_empty()).unwrap_or(INHERIT);
        if !valid_model(model) {
            crate::bail!(
                "Claude Code 모델 지정이 올바르지 않다({name}): {model} (별칭 opus·sonnet·haiku·fable, 전체 모델 id, inherit)"
            );
        }
        let path = dir.join(format!("{name}.md"));
        let Ok(file) = std::fs::read_to_string(&path) else {
            crate::bail!("설치된 에이전트 파일이 없다: {} (ach install을 먼저 한다)", path.display());
        };
        let Some(updated) = set_model(&file, model) else {
            crate::bail!("{}에 model 줄이 없다", path.display());
        };
        let line = write_file(&path, &updated)?;
        done.push(if line.starts_with("ok") {
            format!("ok {name}: model {model}")
        } else {
            format!("{name}: model {model}")
        });
    }
    Ok(done)
}

/// The non-`inherit` models of the installed agent files.
pub fn models_at(home: &Path) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    for entry in std::fs::read_dir(agents_dir(home)).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|x| x != "md") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if let Some(model) = std::fs::read_to_string(&path).ok().and_then(|f| read_model(&f)).filter(|m| m != INHERIT) {
            found.insert(name.to_string(), model);
        }
    }
    found
}

impl Runner for Claude {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn binary(&self) -> &'static str {
        "claude"
    }

    /// The `Agent` tool runs the installed subagents.
    fn subagents(&self) -> bool {
        true
    }

    fn orca_agent(&self) -> Option<&'static str> {
        Some("claude")
    }

    fn interactive(&self, spec: &Spec) -> Result<Vec<String>> {
        command(spec, false)
    }

    fn oneshot(&self, spec: &Spec) -> Result<Vec<String>> {
        command(spec, true)
    }

    fn session_records(&self, cwd: &Path) -> Option<Vec<String>> {
        Some(session_ids_at(&paths::home(), cwd))
    }

    /// `--resume <id>` reopens the conversation under the same id; `message` follows after `--`.
    fn restore(&self, spec: &Spec, session_id: &str, message: &str, interactive: bool) -> Result<Vec<String>> {
        if !super::valid_session_id(session_id) {
            crate::bail!("세션 id가 UUID가 아니다: {session_id}");
        }
        let mut argv = options(spec, !interactive)?;
        argv.extend(["--resume".into(), session_id.to_string(), "--".into(), message.to_string()]);
        Ok(argv)
    }

    fn install(&self, root: &Path) -> Result<Vec<String>> {
        install_at(root, &paths::home())
    }

    fn installed(&self, root: &Path) -> Option<bool> {
        installed_at(root, &paths::home())
    }

    fn apply_agent_models(&self, models: &BTreeMap<String, Option<String>>) -> Result<Vec<String>> {
        apply_models_at(&paths::home(), models)
    }

    fn agent_models(&self) -> Option<BTreeMap<String, String>> {
        Some(models_at(&paths::home()))
    }
}
