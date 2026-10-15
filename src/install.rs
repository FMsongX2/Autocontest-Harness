//! Install agents, the skill and the extension package for one or more runners.

use crate::config::Config;
use crate::error::Result;
use crate::{paths, runners};

/// One line per action, each prefixed with the runner name.
pub fn install(_config: &Config, runners: &[String]) -> Result<Vec<String>> {
    if runners.is_empty() {
        crate::bail!("설치할 러너가 없다(가능: {})", runners::names().join(", "));
    }
    let chosen = runners.iter().map(|name| runners::get(name)).collect::<Result<Vec<_>>>()?;
    let root = paths::root();
    let mut done = Vec::new();
    for runner in chosen {
        done.extend(runner.install(&root)?.into_iter().map(|line| format!("[{}] {line}", runner.name())));
    }
    Ok(done)
}
