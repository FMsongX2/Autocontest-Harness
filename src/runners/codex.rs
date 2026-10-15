use std::path::Path;

use super::{AgentDoc, Runner, Spec, agent_docs, link, prompt_text, system_text, toml_string, write_file};
use crate::error::Result;
use crate::paths;

pub struct Codex;

fn command(spec: &Spec, exec: bool) -> Result<Vec<String>> {
    let mut argv = vec!["codex".to_string()];
    if exec {
        argv.push("exec".into());
    }
    argv.extend(["-C".into(), spec.cwd.display().to_string()]);
    // `exec` is read-only by default; keep the user's own sandbox choice when runner_args has one.
    if exec && !spec.extra_args.iter().any(|a| a == "--sandbox" || a == "-s" || a.starts_with("--sandbox=")) {
        argv.extend(["--sandbox".into(), "workspace-write".into()]);
    }
    if let Some(model) = &spec.model {
        argv.extend(["-m".into(), model.clone()]);
    }
    if let Some(system) = system_text(spec)? {
        argv.extend(["-c".into(), format!("developer_instructions={}", toml_string(&system))]);
    }
    argv.extend(spec.extra_args.iter().cloned());
    argv.push(prompt_text(spec)?);
    Ok(argv)
}

pub fn file_name(doc: &AgentDoc) -> String {
    format!("contest-{}.toml", doc.name)
}

/// One omp agent as a Codex custom agent (`~/.codex/agents/*.toml`).
pub fn convert_agent(doc: &AgentDoc) -> String {
    format!(
        "name = {}\ndescription = {}\ndeveloper_instructions = {}\n",
        toml_string(&doc.name),
        toml_string(&doc.description),
        toml_string(&doc.instructions())
    )
}

/// Agents to `<home>/.codex/agents/contest-*.toml`, the skill to `<home>/.agents/skills/`.
pub fn install_at(root: &Path, home: &Path) -> Result<Vec<String>> {
    let dir = home.join(".codex").join("agents");
    let docs = agent_docs(root)?;
    let keep: Vec<String> = docs.iter().map(file_name).collect();
    let mut done = Vec::new();
    for old in std::fs::read_dir(&dir).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path())) {
        let name = old.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        if name.starts_with("contest-") && name.ends_with(".toml") && !keep.contains(&name) {
            std::fs::remove_file(&old)?;
            done.push(format!("removed {}", old.display()));
        }
    }
    for doc in &docs {
        done.push(write_file(&dir.join(file_name(doc)), &convert_agent(doc))?);
    }
    done.push(link(
        &root.join("skills").join("contest-harness"),
        &home.join(".agents").join("skills").join("contest-harness"),
    )?);
    Ok(done)
}

impl Runner for Codex {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn binary(&self) -> &'static str {
        "codex"
    }

    fn subagents(&self) -> bool {
        true
    }

    fn interactive(&self, spec: &Spec) -> Result<Vec<String>> {
        command(spec, false)
    }

    fn oneshot(&self, spec: &Spec) -> Result<Vec<String>> {
        command(spec, true)
    }

    fn install(&self, root: &Path) -> Result<Vec<String>> {
        install_at(root, &paths::home())
    }
}
