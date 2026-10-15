//! Install agents, the skill and the extension package for one or more runners, and apply the
//! per-agent model choices (`agent_models`) to the configured runner.

use std::path::Path;

use crate::config::Config;
use crate::error::Result;
use crate::{paths, runners};

/// `agent_models` names that are not agents in `<root>/agents/` (typos would silently do nothing).
pub fn unknown_agents(config: &Config, root: &Path) -> Result<Vec<String>> {
    let known: Vec<String> = runners::agent_docs(root)?.into_iter().map(|doc| doc.name).collect();
    Ok(config.agent_models.keys().filter(|name| !known.contains(name)).cloned().collect())
}

/// One line per action, each prefixed with the runner name.
pub fn install(config: &Config, runners: &[String]) -> Result<Vec<String>> {
    if runners.is_empty() {
        crate::bail!("설치할 러너가 없다(가능: {})", runners::names().join(", "));
    }
    let chosen = runners.iter().map(|name| runners::get(name)).collect::<Result<Vec<_>>>()?;
    let root = paths::root();
    let unknown = unknown_agents(config, &root)?;
    if !unknown.is_empty() {
        crate::bail!("agent_models에 없는 에이전트가 있다: {} (harness agents로 이름을 확인한다)", unknown.join(", "));
    }
    let mut done = Vec::new();
    for runner in chosen {
        let mut lines = runner.install(&root)?;
        // Model selectors are in the configured runner's syntax, so only that runner gets them.
        if runner.name() == config.runner {
            lines.extend(runner.apply_agent_models(&config.agent_models)?);
        }
        done.extend(lines.into_iter().map(|line| format!("[{}] {line}", runner.name())));
    }
    Ok(done)
}
