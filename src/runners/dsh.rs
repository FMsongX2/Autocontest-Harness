use std::path::Path;

use super::{Runner, Spec, link, prompt_text, system_text, which};
use crate::error::Result;
use crate::paths;

pub struct Dsh;

/// dsh has no system-prompt flag, so the extra system text leads the first message.
fn message(spec: &Spec) -> Result<String> {
    let prompt = prompt_text(spec)?;
    Ok(match system_text(spec)? {
        Some(system) => format!("{}\n\n---\n\n{prompt}", system.trim_end()),
        None => prompt,
    })
}

/// Interactive argv; `tui` says whether the `dsh-tui` launcher is installed.
pub fn interactive_argv(spec: &Spec, tui: bool) -> Result<Vec<String>> {
    let mut argv: Vec<String> =
        if tui { vec!["dsh-tui".into()] } else { vec!["dsh".into(), "--profile".into(), "dsh-tui".into()] };
    argv.extend(spec.extra_args.iter().cloned());
    // Without the launcher the first `--` ends dsh's options and the second the app's.
    if !tui {
        argv.push("--".into());
    }
    argv.push("--".into());
    argv.push(message(spec)?);
    Ok(argv)
}

impl Runner for Dsh {
    fn name(&self) -> &'static str {
        "dsh"
    }

    fn binary(&self) -> &'static str {
        "dsh"
    }

    fn subagents(&self) -> bool {
        false
    }

    fn interactive(&self, spec: &Spec) -> Result<Vec<String>> {
        interactive_argv(spec, which("dsh-tui").is_some())
    }

    fn oneshot(&self, spec: &Spec) -> Result<Vec<String>> {
        let mut argv = vec!["dsh".to_string(), "--profile".into(), "headless".into()];
        argv.extend(spec.extra_args.iter().cloned());
        argv.push(message(spec)?);
        Ok(argv)
    }

    /// dsh reads Agent Skills from `~/.agents/skills` (shared with Codex).
    fn install(&self, root: &Path) -> Result<Vec<String>> {
        install_at(root, &paths::home())
    }
}

pub fn install_at(root: &Path, home: &Path) -> Result<Vec<String>> {
    Ok(vec![link(
        &root.join("skills").join("contest-harness"),
        &home.join(".agents").join("skills").join("contest-harness"),
    )?])
}
