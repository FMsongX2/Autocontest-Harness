//! `harness init`: the files a new checkout needs, created once and never overwritten. The
//! conversational part of setup (what to ask the user, how to apply it) is `INIT.md`, which an
//! agent follows; this command is the part that must not depend on a model.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::{doctor, paths, store};

#[derive(Debug, Default, Serialize)]
pub struct Report {
    /// Files and folders this run created.
    pub created: Vec<String>,
    /// Ones that already existed and were left as they are.
    pub kept: Vec<String>,
    /// Existing folders that other users could open, narrowed to 0700.
    pub tightened: Vec<String>,
    /// `harness doctor` after the scaffolding.
    pub checks: Vec<doctor::Check>,
    /// The setup instructions an agent follows next.
    pub next: String,
}

/// Copy `example` to `target` with `mode` unless `target` exists.
fn copy_once(example: &Path, target: &Path, report: &mut Report) -> Result<()> {
    let name = target.display().to_string();
    if target.exists() {
        report.kept.push(name);
        return Ok(());
    }
    let text = std::fs::read(example).map_err(|e| Error(format!("{}: {e}", example.display())))?;
    store::write_atomic(target, &text, false)?;
    report.created.push(name);
    Ok(())
}

/// Create `data/` (0700), `data/config.json` and `data/profile.md` from the committed examples (0600)
/// and `data/logs/`, then run the doctor. Running it again changes nothing.
pub fn init(root: &Path) -> Result<Report> {
    let mut report = Report::default();
    let data = root.join("data");
    if data.is_dir() {
        // A clone already has `data/` (it holds the committed examples), with whatever the umask gave it.
        let at = |e: std::io::Error| Error(format!("{}: {e}", data.display()));
        if std::fs::metadata(&data).map_err(at)?.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).map_err(at)?;
            report.tightened.push(data.display().to_string());
        } else {
            report.kept.push(data.display().to_string());
        }
    } else {
        store::private_dir(&data)?;
        report.created.push(data.display().to_string());
    }
    copy_once(&data.join("config.example.json"), &paths::config_file(), &mut report)?;
    copy_once(&data.join("profile.example.md"), &paths::profile(), &mut report)?;
    let logs = data.join("logs");
    if !logs.is_dir() {
        store::private_dir(&logs)?;
        report.created.push(logs.display().to_string());
    }
    report.checks = doctor::run();
    report.next = root.join("INIT.md").display().to_string();
    Ok(report)
}

pub fn text(report: &Report) -> String {
    let mut lines: Vec<String> = report.created.iter().map(|p| format!("만듦: {p}")).collect();
    lines.extend(report.kept.iter().map(|p| format!("그대로 둠: {p}")));
    lines.extend(report.tightened.iter().map(|p| format!("권한을 0700으로 좁힘: {p}")));
    lines.push(String::new());
    lines.push(doctor::text(&report.checks));
    lines.push(String::new());
    lines.push(format!(
        "다음: 에이전트에게 {}를 읽고 설정을 마치게 한다(사용자에게 물을 것과 적용 방법이 있다)",
        report.next
    ));
    lines.join("\n")
}
