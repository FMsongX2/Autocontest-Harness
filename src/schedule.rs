//! Scheduled runs outside Orca: `harness autorun` and a launchd job that calls it.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::runners::{self, Spec};
use crate::workspaces::shell_command;
use crate::{paths, text};

pub const LABEL: &str = "io.github.fmsongx2.autocontest-harness";

unsafe extern "C" {
    fn getuid() -> u32;
}

/// Run `harness/prompts/automation.md` once with the configured runner's one-shot command.
pub fn autorun(config: &Config) -> Result<Value> {
    let runner = runners::get(&config.runner)?;
    if !runner.available() {
        crate::bail!("러너 {}의 실행 파일({})이 PATH에 없다", runner.name(), runner.binary());
    }
    let root = paths::root();
    let prompt = paths::prompts().join("automation.md");
    if !prompt.exists() {
        crate::bail!("정기 실행 지시문이 없다: {}", prompt.display());
    }
    let spec = Spec {
        cwd: root.clone(),
        prompt_file: prompt,
        first_message: "첨부한 정기 실행 지시를 읽고 그대로 수행해.".to_string(),
        system_append: None,
        model: None,
        preset_overlay: None,
        extra_args: config.runner_args(runner.name()),
    };
    let argv = runner.oneshot(&spec)?;
    let logs = paths::data().join("logs");
    std::fs::create_dir_all(&logs)?;
    let stamp = chrono::Utc::now().with_timezone(&text::kst()).format("%Y%m%d-%H%M%S");
    let log = logs.join(format!("autorun-{stamp}.log"));
    let out = File::create(&log)?;
    let err = out.try_clone()?;
    let status = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .status()
        .map_err(|e| Error(format!("{} 실행 실패: {e}", argv[0])))?;
    Ok(json!({"command": shell_command(&root, &argv), "log": log.display().to_string(), "exit_code": status.code()}))
}

pub fn plist_path() -> PathBuf {
    paths::home().join("Library").join("LaunchAgents").join(format!("{LABEL}.plist"))
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The launchd job: `<exe> autorun` every `every_hours` hours with the harness root and PATH fixed.
pub fn plist(exe: &Path, root: &Path, every_hours: u32, path_env: &str) -> String {
    let log = root.join("data").join("logs").join("launchd.log");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>autorun</string>
  </array>
  <key>WorkingDirectory</key>
  <string>{root}</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>HARNESS_HOME</key>
    <string>{root}</string>
    <key>PATH</key>
    <string>{path}</string>
  </dict>
  <key>StartInterval</key>
  <integer>{interval}</integer>
  <key>RunAtLoad</key>
  <false/>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#,
        label = LABEL,
        exe = xml(&exe.display().to_string()),
        root = xml(&root.display().to_string()),
        path = xml(path_env),
        interval = u64::from(every_hours) * 3600,
        log = xml(&log.display().to_string()),
    )
}

fn domain() -> String {
    // SAFETY: getuid has no preconditions and cannot fail.
    format!("gui/{}", unsafe { getuid() })
}

fn launchctl(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("launchctl").args(args).stdin(Stdio::null()).output()
}

pub fn install(every_hours: u32) -> Result<Value> {
    if every_hours == 0 {
        crate::bail!("--every-hours는 1 이상이어야 한다");
    }
    let exe = std::env::current_exe()?;
    let root = std::fs::canonicalize(paths::root())?;
    std::fs::create_dir_all(root.join("data").join("logs"))?;
    let path_env = std::env::var("PATH").unwrap_or_default();
    let target = plist_path();
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target, plist(&exe, &root, every_hours, &path_env))?;
    let domain = domain();
    let service = format!("{domain}/{LABEL}");
    // Re-installing replaces a loaded job; bootout fails harmlessly when none is loaded.
    let _ = launchctl(&["bootout", &service]);
    let target_text = target.display().to_string();
    let out = launchctl(&["bootstrap", &domain, &target_text])?;
    if !out.status.success() {
        crate::bail!(
            "launchctl bootstrap {domain} {target_text} 실패: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(json!({"plist": target_text, "label": LABEL, "every_hours": every_hours, "loaded": true}))
}

pub fn remove() -> Result<Value> {
    let target = plist_path();
    let out = launchctl(&["bootout", &format!("{}/{LABEL}", domain())])?;
    let existed = target.exists();
    if existed {
        std::fs::remove_file(&target)?;
    }
    Ok(
        json!({"plist": target.display().to_string(), "label": LABEL, "removed": existed, "unloaded": out.status.success()}),
    )
}

pub fn status() -> Result<Value> {
    let target = plist_path();
    let installed = target.exists();
    let every_hours = std::fs::read_to_string(&target).ok().and_then(|xml| {
        let after = xml.split("<key>StartInterval</key>").nth(1)?;
        let value = after.split("<integer>").nth(1)?.split("</integer>").next()?;
        value.trim().parse::<u64>().ok().map(|s| s / 3600)
    });
    let print = launchctl(&["print", &format!("{}/{LABEL}", domain())])?;
    let loaded = print.status.success();
    let details = String::from_utf8_lossy(&print.stdout);
    let last_exit =
        details.lines().find_map(|l| l.trim().strip_prefix("last exit code = ")).map(|v| v.trim().to_string());
    let last_log = std::fs::read_dir(paths::data().join("logs"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("autorun-")))
        .max();
    Ok(json!({
        "plist": target.display().to_string(),
        "label": LABEL,
        "installed": installed,
        "loaded": loaded,
        "every_hours": every_hours,
        "last_exit": last_exit,
        "last_log": last_log.map(|p| p.display().to_string()),
    }))
}
