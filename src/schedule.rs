//! Scheduled runs: the `schedule` config (cron, scheduler, ContestScout hours), `harness autorun`,
//! and the two schedulers that start a run: an Orca automation or a macOS launchd job.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::runners::{self, Spec, which};
use crate::workspaces::orca::{created_worktree_id, orca_json};
use crate::workspaces::shell_command;
use crate::{paths, text};

pub const LABEL: &str = "io.github.fmsongx2.autocontest-harness";
pub const SCHEDULERS: &[&str] = &["orca", "launchd"];
/// Name of the Orca automation and of the workspace it runs in.
const ORCA_NAME: &str = "Autocontest-Harness";
/// launchd needs one calendar entry per value combination; more than this is not a sensible schedule.
const MAX_CALENDAR: usize = 1000;

unsafe extern "C" {
    fn getuid() -> u32;
}

/// A 5-field cron expression (minute hour day month weekday) in local time. A field is `*`, a
/// number, a range `a-b`, a step `*/n`, `a/n` or `a-b/n`, or a comma list of those. Names (`MON`,
/// `JAN`) and macros (`@daily`) are not accepted, so Orca and launchd read the same schedule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cron {
    /// The expression with single spaces between fields.
    pub text: String,
    /// Values per field; `None` is a bare `*`.
    pub minute: Option<Vec<u8>>,
    pub hour: Option<Vec<u8>>,
    pub day: Option<Vec<u8>>,
    pub month: Option<Vec<u8>>,
    /// 0-6 with Sunday 0 (7 is read as 0).
    pub weekday: Option<Vec<u8>>,
    /// Whether the day / weekday field starts with `*`. When neither does, cron runs on a day that
    /// matches either field; otherwise both must match.
    day_star: bool,
    weekday_star: bool,
}

fn number(text: &str) -> Result<u8> {
    text.trim().parse::<u8>().map_err(|_| Error(format!("cron 값이 숫자가 아니다: {text:?}")))
}

/// The values of one cron field within `min..=max`, sorted; `None` for a bare `*`.
pub fn parse_field(text: &str, min: u8, max: u8) -> Result<Option<Vec<u8>>> {
    let text = text.trim();
    if text == "*" {
        return Ok(None);
    }
    let mut values = Vec::new();
    for item in text.split(',') {
        let (range, step) = match item.split_once('/') {
            Some((range, step)) => (range, Some(number(step)?)),
            None => (item, None),
        };
        let (lo, hi) = match range.split_once('-') {
            _ if range == "*" => (min, max),
            Some((a, b)) => (number(a)?, number(b)?),
            None => {
                let a = number(range)?;
                (a, if step.is_some() { max } else { a })
            }
        };
        if lo < min || hi > max || lo > hi {
            crate::bail!("cron 값 {item}이 범위 {min}-{max}를 벗어난다");
        }
        let step = step.unwrap_or(1);
        if step == 0 {
            crate::bail!("cron 간격은 1 이상이어야 한다: {item}");
        }
        values.extend((lo..=hi).step_by(usize::from(step)));
    }
    values.sort_unstable();
    values.dedup();
    Ok(Some(values))
}

/// One launchd calendar entry: `(key, value)` pairs; a missing key is a wildcard.
pub type CalendarEntry = Vec<(&'static str, u8)>;

impl Cron {
    pub fn parse(text: &str) -> Result<Cron> {
        let fields: Vec<&str> = text.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields[..] else {
            crate::bail!("cron은 다섯 칸(분 시 일 월 요일)이어야 한다: {text:?}")
        };
        let weekday_values = parse_field(weekday, 0, 7)?.map(|values| {
            let mut days: Vec<u8> = values.into_iter().map(|d| d % 7).collect();
            days.sort_unstable();
            days.dedup();
            days
        });
        Ok(Cron {
            text: fields.join(" "),
            minute: parse_field(minute, 0, 59)?,
            hour: parse_field(hour, 0, 23)?,
            day: parse_field(day, 1, 31)?,
            month: parse_field(month, 1, 12)?,
            weekday: weekday_values,
            day_star: day.starts_with('*'),
            weekday_star: weekday.starts_with('*'),
        })
    }

    /// launchd `StartCalendarInterval` entries for this expression.
    pub fn calendar(&self) -> Result<Vec<CalendarEntry>> {
        type Field<'a> = (&'static str, &'a Option<Vec<u8>>);
        let base = [("Minute", &self.minute), ("Hour", &self.hour), ("Month", &self.month)];
        let groups: Vec<Vec<Field>> = if !self.day_star && !self.weekday_star {
            // Either field may match, so each gets its own set of entries.
            vec![[&base[..], &[("Day", &self.day)]].concat(), [&base[..], &[("Weekday", &self.weekday)]].concat()]
        } else {
            if self.day.is_some() && self.weekday.is_some() {
                crate::bail!("일과 요일을 함께 거는 cron({})은 launchd로 옮기지 못한다", self.text);
            }
            vec![[&base[..], &[("Day", &self.day), ("Weekday", &self.weekday)]].concat()]
        };
        let mut out = Vec::new();
        for group in groups {
            let mut entries: Vec<CalendarEntry> = vec![Vec::new()];
            for (key, values) in group {
                let Some(values) = values else { continue };
                entries = entries
                    .into_iter()
                    .flat_map(|entry| {
                        values.iter().map(move |v| {
                            let mut entry = entry.clone();
                            entry.push((key, *v));
                            entry
                        })
                    })
                    .collect();
                if entries.len() > MAX_CALENDAR {
                    crate::bail!("cron {}은 launchd 항목이 {MAX_CALENDAR}개를 넘는다", self.text);
                }
            }
            out.extend(entries);
        }
        Ok(out)
    }
}

/// Whether a scheduled run at local `hour` also runs ContestScout: `scout_hours` is a cron hour
/// field (`0-5`, `0,12`, `*`); empty means never.
pub fn scout_at(scout_hours: &str, hour: u8) -> Result<bool> {
    if scout_hours.trim().is_empty() {
        return Ok(false);
    }
    Ok(parse_field(scout_hours, 0, 23)?.is_none_or(|hours| hours.contains(&hour)))
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
        preset_file: None,
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

fn scheduler_name<'a>(config: &'a Config, scheduler: Option<&'a str>) -> Result<&'a str> {
    let name = scheduler.unwrap_or(&config.schedule.scheduler);
    if !SCHEDULERS.contains(&name) {
        crate::bail!("알 수 없는 스케줄러: {name} (가능: {})", SCHEDULERS.join(", "));
    }
    Ok(name)
}

/// Install the scheduled run on `scheduler` (default `schedule.scheduler`) with `schedule.cron`,
/// or update the one already there.
pub fn install(config: &Config, scheduler: Option<&str>) -> Result<Value> {
    let cron = Cron::parse(&config.schedule.cron)?;
    let mut result = match scheduler_name(config, scheduler)? {
        "orca" => orca_install(config, &cron)?,
        _ => launchd_install(&cron)?,
    };
    // Two schedulers would start two runs at once, gating and launching the same contests.
    let other = match result["scheduler"].as_str() {
        Some("orca") => plist_path().exists().then_some("launchd"),
        _ => (which("orca").is_some() && orca_automations().is_ok_and(|found| !found.is_empty())).then_some("orca"),
    };
    if let Some(other) = other {
        result["warning"] =
            json!(format!("{other}에도 정기 실행이 있다. 한쪽만 두려면 harness schedule remove --scheduler {other}"));
    }
    Ok(result)
}

pub fn remove(config: &Config, scheduler: Option<&str>) -> Result<Value> {
    match scheduler_name(config, scheduler)? {
        "orca" => orca_remove(),
        _ => launchd_remove(),
    }
}

/// The configured schedule and what each scheduler actually has.
pub fn status(config: &Config) -> Result<Value> {
    let schedule = &config.schedule;
    let cron = Cron::parse(&schedule.cron);
    let wanted = cron.as_ref().map(|c| c.text.clone()).unwrap_or_default();
    let agent = runners::get(&config.runner).ok().and_then(|r| r.orca_agent());
    Ok(json!({
        "cron": schedule.cron,
        "cron_error": cron.as_ref().err().map(|e| e.0.clone()),
        "scheduler": schedule.scheduler,
        "scout_hours": schedule.scout_hours,
        "launchd": launchd_status(&wanted),
        "orca": orca_status(&wanted, agent),
    }))
}

// ---- Orca automation ----

/// This checkout's automation prompt; the automation's prompt names it, which is how the harness
/// finds its own automation among the user's others.
fn automation_file() -> Result<PathBuf> {
    let file = paths::prompts().join("automation.md");
    std::fs::canonicalize(&file).map_err(|e| Error(format!("정기 실행 지시문이 없다: {}: {e}", file.display())))
}

pub fn orca_prompt(file: &Path) -> String {
    format!("{} 파일을 읽고 그 절차를 그대로 실행해.", file.display())
}

/// Orca automations whose prompt points at this checkout's `automation.md`.
fn orca_automations() -> Result<Vec<Value>> {
    let needle = automation_file()?.display().to_string();
    let result = orca_json(&["automations", "list"])?;
    let list = result.get("automations").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(list.into_iter().filter(|a| a["prompt"].as_str().is_some_and(|p| p.contains(&needle))).collect())
}

/// The IANA zone of this machine (`TZ`, then `/etc/localtime`), for the Orca automation.
fn local_timezone() -> Option<String> {
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim_start_matches(':');
        if tz.contains('/') {
            return Some(tz.to_string());
        }
    }
    let link = std::fs::read_link("/etc/localtime").ok()?;
    link.to_str()?.split_once("zoneinfo/").map(|(_, zone)| zone.to_string())
}

fn local_time(millis: Option<i64>) -> Option<String> {
    let at = chrono::DateTime::from_timestamp_millis(millis?)?;
    Some(at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
}

fn orca_install(config: &Config, cron: &Cron) -> Result<Value> {
    let runner = runners::get(&config.runner)?;
    let Some(agent) = runner.orca_agent() else {
        crate::bail!("러너 {}는 Orca automation으로 띄울 수 없다. schedule.scheduler를 launchd로 둔다", runner.name())
    };
    if which("orca").is_none() {
        crate::bail!("orca CLI가 PATH에 없다. schedule.scheduler를 launchd로 두거나 Orca를 설치한다");
    }
    let found = orca_automations()?;
    if let Some(id) = found.first().and_then(|a| a["id"].as_str()) {
        orca_json(&["automations", "edit", "--id", id, "--trigger", &cron.text, "--provider", agent, "--enabled"])?;
        return Ok(json!({
            "scheduler": "orca",
            "action": "updated",
            "id": id,
            "cron": cron.text,
            "agent": agent,
            "duplicates": found[1..].iter().filter_map(|a| a["id"].as_str()).collect::<Vec<_>>(),
        }));
    }
    let repo = format!("path:{}", config.orca_repo_path().display());
    let workspace = created_worktree_id(&orca_json(&[
        "worktree",
        "create",
        "--repo",
        &repo,
        "--name",
        ORCA_NAME,
        "--no-parent",
        "--comment",
        "harness schedule",
    ])?)?;
    let prompt = orca_prompt(&automation_file()?);
    let selector = format!("id:{workspace}");
    let timezone = local_timezone();
    let mut args = vec![
        "automations",
        "create",
        "--name",
        ORCA_NAME,
        "--trigger",
        &cron.text,
        "--prompt",
        &prompt,
        "--provider",
        agent,
        "--workspace",
        &selector,
        "--enabled",
    ];
    if let Some(zone) = &timezone {
        args.extend(["--timezone", zone.as_str()]);
    }
    let created = orca_json(&args)?;
    let id = created.get("automation").unwrap_or(&created).get("id").cloned().unwrap_or(Value::Null);
    Ok(json!({
        "scheduler": "orca",
        "action": "created",
        "id": id,
        "cron": cron.text,
        "timezone": timezone,
        "agent": agent,
        "workspace_id": workspace,
    }))
}

/// Remove the harness automations. Their workspace stays (it may hold other terminals).
fn orca_remove() -> Result<Value> {
    if which("orca").is_none() {
        crate::bail!("orca CLI가 PATH에 없다");
    }
    let mut removed = Vec::new();
    for automation in orca_automations()? {
        let Some(id) = automation["id"].as_str() else { continue };
        orca_json(&["automations", "remove", "--id", id])?;
        removed.push(json!({"id": id, "workspace_id": automation["workspaceId"]}));
    }
    Ok(json!({"scheduler": "orca", "removed": removed}))
}

fn orca_status(wanted: &str, agent: Option<&str>) -> Value {
    if which("orca").is_none() {
        return json!({"available": false, "automations": []});
    }
    match orca_automations() {
        Err(e) => json!({"available": true, "error": e.0}),
        Ok(found) => {
            let automations: Vec<Value> = found
                .iter()
                .map(|a| {
                    let cron = a["rrule"].as_str().unwrap_or_default();
                    let enabled = a["enabled"].as_bool().unwrap_or(false);
                    let in_sync = enabled && cron == wanted && agent.is_some_and(|g| a["agentId"].as_str() == Some(g));
                    json!({
                        "id": a["id"],
                        "name": a["name"],
                        "cron": cron,
                        "timezone": a["timezone"],
                        "enabled": enabled,
                        "agent": a["agentId"],
                        "workspace_id": a["workspaceId"],
                        "next_run": local_time(a["nextRunAt"].as_i64()),
                        "last_run": local_time(a["lastRunAt"].as_i64()),
                        "in_sync": in_sync,
                    })
                })
                .collect();
            json!({"available": true, "automations": automations})
        }
    }
}

// ---- launchd ----

pub fn plist_path() -> PathBuf {
    paths::home().join("Library").join("LaunchAgents").join(format!("{LABEL}.plist"))
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The launchd job: `<exe> autorun` on the calendar of `cron`, with the harness root and PATH
/// fixed. The cron text is kept in a comment so `status` can compare it with the config.
pub fn plist(exe: &Path, root: &Path, cron: &Cron, path_env: &str) -> Result<String> {
    let log = root.join("data").join("logs").join("launchd.log");
    let mut calendar = String::new();
    for entry in cron.calendar()? {
        calendar.push_str("    <dict>\n");
        for (key, value) in entry {
            calendar.push_str(&format!("      <key>{key}</key>\n      <integer>{value}</integer>\n"));
        }
        calendar.push_str("    </dict>\n");
    }
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- cron: {cron} -->
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
  <key>StartCalendarInterval</key>
  <array>
{calendar}  </array>
  <key>RunAtLoad</key>
  <false/>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#,
        cron = cron.text,
        label = LABEL,
        exe = xml(&exe.display().to_string()),
        root = xml(&root.display().to_string()),
        path = xml(path_env),
        log = xml(&log.display().to_string()),
    ))
}

/// The cron text recorded in an installed plist.
pub fn plist_cron(plist: &str) -> Option<String> {
    let rest = plist.split("<!-- cron: ").nth(1)?;
    Some(rest.split(" -->").next()?.trim().to_string())
}

fn domain() -> String {
    // SAFETY: getuid has no preconditions and cannot fail.
    format!("gui/{}", unsafe { getuid() })
}

fn launchctl(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("launchctl").args(args).stdin(Stdio::null()).output()
}

fn launchd_install(cron: &Cron) -> Result<Value> {
    let exe = std::env::current_exe()?;
    let root = std::fs::canonicalize(paths::root())?;
    std::fs::create_dir_all(root.join("data").join("logs"))?;
    let path_env = std::env::var("PATH").unwrap_or_default();
    let content = plist(&exe, &root, cron, &path_env)?;
    let target = plist_path();
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target, content)?;
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
    Ok(json!({"scheduler": "launchd", "plist": target_text, "label": LABEL, "cron": cron.text, "loaded": true}))
}

fn launchd_remove() -> Result<Value> {
    let target = plist_path();
    let out = launchctl(&["bootout", &format!("{}/{LABEL}", domain())])?;
    let existed = target.exists();
    if existed {
        std::fs::remove_file(&target)?;
    }
    Ok(json!({
        "scheduler": "launchd",
        "plist": target.display().to_string(),
        "label": LABEL,
        "removed": existed,
        "unloaded": out.status.success(),
    }))
}

fn launchd_status(wanted: &str) -> Value {
    let target = plist_path();
    let cron = std::fs::read_to_string(&target).ok().and_then(|text| plist_cron(&text));
    let print = launchctl(&["print", &format!("{}/{LABEL}", domain())]).ok();
    let details = print.as_ref().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let last_exit =
        details.lines().find_map(|l| l.trim().strip_prefix("last exit code = ")).map(|v| v.trim().to_string());
    let last_log = std::fs::read_dir(paths::data().join("logs"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("autorun-")))
        .max();
    json!({
        "plist": target.display().to_string(),
        "installed": target.exists(),
        "loaded": print.is_some_and(|o| o.status.success()),
        "cron": cron,
        "in_sync": cron.as_deref() == Some(wanted),
        "last_exit": last_exit,
        "last_log": last_log.map(|p| p.display().to_string()),
    })
}
