use std::path::Path;

use super::{Runner, Spec, run_cli};
use crate::error::Result;

pub struct Pi;

/// pi has no cwd flag; the workspace starts it in `spec.cwd`.
fn command(spec: &Spec, print: bool) -> Vec<String> {
    let mut argv = vec!["pi".to_string()];
    if let Some(model) = &spec.model {
        argv.extend(["--model".into(), model.clone()]);
    }
    if let Some(system) = &spec.system_append {
        argv.extend(["--append-system-prompt".into(), system.display().to_string()]);
    }
    argv.extend(spec.extra_args.iter().cloned());
    // `-p` swallows the next word as the message unless it starts with `@` or `-`.
    if print {
        argv.push("-p".into());
    }
    argv.push(format!("@{}", spec.prompt_file.display()));
    argv.push(spec.first_message.clone());
    argv
}

impl Runner for Pi {
    fn name(&self) -> &'static str {
        "pi"
    }

    fn binary(&self) -> &'static str {
        "pi"
    }

    fn subagents(&self) -> bool {
        false
    }

    fn interactive(&self, spec: &Spec) -> Result<Vec<String>> {
        Ok(command(spec, false))
    }

    fn oneshot(&self, spec: &Spec) -> Result<Vec<String>> {
        Ok(command(spec, true))
    }

    /// A local path is registered in `~/.pi/agent/settings.json` and used in place.
    fn install(&self, root: &Path) -> Result<Vec<String>> {
        let root_text = root.display().to_string();
        let out = run_cli("pi", &["install", &root_text])?;
        Ok(vec![format!("pi install {root_text}: {}", out.lines().last().unwrap_or("ok"))])
    }
}
