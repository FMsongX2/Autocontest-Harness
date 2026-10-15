use std::os::unix::process::CommandExt;
use std::process::Command;

use super::{Open, Workspace};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::registry::Entry;

pub struct Terminal;

impl Workspace for Terminal {
    fn name(&self) -> &'static str {
        "terminal"
    }

    fn available(&self) -> bool {
        true
    }

    fn interactive(&self) -> bool {
        true
    }

    fn create(&self, _entry: &Entry, _config: &Config) -> Result<Option<String>> {
        Ok(None)
    }

    /// Replaces the harness process, so it only returns on failure.
    fn open(&self, spec: &Open<'_>) -> Result<String> {
        let Some((program, args)) = spec.argv.split_first() else { crate::bail!("실행할 명령이 비었다") };
        let err = Command::new(program).args(args).current_dir(&spec.cwd).exec();
        Err(Error(format!("{program} 실행 실패: {err}")))
    }

    fn tail(&self, _handle: &str, _lines: usize) -> Result<String> {
        crate::bail!("terminal 작업 환경은 화면을 읽을 수 없다(그 터미널에서 직접 본다)")
    }

    fn close(&self, _handle: &str) -> Result<()> {
        Ok(())
    }

    fn remove_workspace(&self, _workspace_id: &str) -> Result<()> {
        Ok(())
    }
}
