use std::path::Path;

use super::{AgentDoc, Runner, Spec, agent_docs, link, prompt_text, write_file};
use crate::error::Result;
use crate::paths;

pub struct Claude;

/// Claude Code has no cwd flag and no `@file` argument, so the instructions are the first message.
fn command(spec: &Spec, print: bool) -> Result<Vec<String>> {
    let mut argv = vec!["claude".to_string()];
    if print {
        argv.push("-p".into());
    }
    if let Some(model) = &spec.model {
        argv.extend(["--model".into(), model.clone()]);
    }
    if let Some(system) = &spec.system_append {
        argv.extend(["--append-system-prompt-file".into(), system.display().to_string()]);
    }
    argv.extend(spec.extra_args.iter().cloned());
    argv.push(prompt_text(spec)?);
    Ok(argv)
}

/// omp tool names → Claude Code tool names (tools Claude lacks are dropped).
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
        "todo" => &["TodoWrite"],
        _ => &[],
    }
}

/// One omp agent as a Claude Code subagent file.
pub fn convert_agent(doc: &AgentDoc) -> String {
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
    out.push_str("model: inherit\n");
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

/// Agents to `<home>/.claude/agents/contest-harness/`, the skill to `<home>/.claude/skills/`.
pub fn install_at(root: &Path, home: &Path) -> Result<Vec<String>> {
    let dir = home.join(".claude").join("agents").join("contest-harness");
    let docs = agent_docs(root)?;
    let mut done = Vec::new();
    let keep: Vec<String> = docs.iter().map(|d| format!("{}.md", d.name)).collect();
    for old in std::fs::read_dir(&dir).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path())) {
        let name = old.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        if name.ends_with(".md") && !keep.contains(&name) {
            std::fs::remove_file(&old)?;
            done.push(format!("removed {}", old.display()));
        }
    }
    for doc in &docs {
        done.push(write_file(&dir.join(format!("{}.md", doc.name)), &convert_agent(doc))?);
    }
    done.push(link(
        &root.join("skills").join("contest-harness"),
        &home.join(".claude").join("skills").join("contest-harness"),
    )?);
    Ok(done)
}

impl Runner for Claude {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn binary(&self) -> &'static str {
        "claude"
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
