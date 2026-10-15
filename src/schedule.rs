//! Scheduled runs: the `schedule` config (cron, scheduler, ContestScout hours), `harness autorun`,
//! and the three schedulers that start a run: an Orca automation, a macOS launchd job, or a
//! crontab block (macOS and Linux).

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use chrono::{Datelike, NaiveDateTime, TimeZone};
use serde_json::{Value, json};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::runners::{self, Spec, which};
use crate::workspaces::orca::{created_worktree_id, orca_json};
use crate::workspaces::{shell_command, shell_quote};
use crate::{paths, process, text};

/// Label of the launchd job before it carried a per-checkout suffix; still found by status/remove.
pub const LEGACY_LABEL: &str = "io.github.fmsongx2.autocontest-harness";
pub const SCHEDULERS: &[&str] = &["orca", "launchd", "cron"];
/// Name of the Orca automation and of the workspace it runs in.
const ORCA_NAME: &str = "Autocontest-Harness";
/// Tag of the crontab block markers.
const CRON_TAG: &str = "autocontest-harness";
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

    /// Whether the expression runs on `date` (cron day rules: when both day and weekday are
    /// restricted, either may match).
    fn runs_on(&self, date: chrono::NaiveDate) -> bool {
        let has = |field: &Option<Vec<u8>>, value: u32| {
            field.as_ref().is_none_or(|v| v.iter().any(|x| u32::from(*x) == value))
        };
        if !has(&self.month, date.month()) {
            return false;
        }
        let day = has(&self.day, date.day());
        let weekday = has(&self.weekday, date.weekday().num_days_from_sunday());
        if !self.day_star && !self.weekday_star { day || weekday } else { day && weekday }
    }

    /// The first run strictly after `after` (local wall time), within five years.
    pub fn next_after(&self, after: NaiveDateTime) -> Option<NaiveDateTime> {
        let all = |field: &Option<Vec<u8>>, max: u8| field.clone().unwrap_or_else(|| (0..=max).collect());
        let (hours, minutes) = (all(&self.hour, 23), all(&self.minute, 59));
        for date in after.date().iter_days().take(366 * 5 + 1) {
            if !self.runs_on(date) {
                continue;
            }
            for hour in &hours {
                for minute in &minutes {
                    let at = date.and_hms_opt(u32::from(*hour), u32::from(*minute), 0)?;
                    if at > after {
                        return Some(at);
                    }
                }
            }
        }
        None
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
        prompt: None,
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

/// This checkout's root as the schedulers record it (canonical when possible).
fn checkout_root() -> PathBuf {
    let root = paths::root();
    std::fs::canonicalize(&root).unwrap_or(root)
}

/// Install the scheduled run on `scheduler` (default `schedule.scheduler`) with `schedule.cron`,
/// or update the one already there.
pub fn install(config: &Config, scheduler: Option<&str>) -> Result<Value> {
    let cron = Cron::parse(&config.schedule.cron)?;
    let name = scheduler_name(config, scheduler)?;
    let mut result = match name {
        "orca" => orca_install(config, &cron)?,
        "cron" => cron_install(&cron)?,
        _ => launchd_install(&cron)?,
    };
    // Two schedulers would start two runs at once, gating and launching the same contests.
    let root = checkout_root();
    let others: Vec<&str> = SCHEDULERS
        .iter()
        .copied()
        .filter(|other| *other != name)
        .filter(|other| match *other {
            "orca" => which("orca").is_some() && orca_automations().is_ok_and(|found| !found.is_empty()),
            "cron" => {
                which("crontab").is_some() && read_crontab("crontab").is_ok_and(|t| cron_job(&t, &root).is_some())
            }
            _ => launchd_installed(&root),
        })
        .collect();
    if !others.is_empty() {
        let commands: Vec<String> =
            others.iter().map(|other| format!("harness schedule remove --scheduler {other}")).collect();
        result["warning"] =
            json!(format!("{}에도 정기 실행이 있다. 한쪽만 두려면 {}", others.join(", "), commands.join(" ; ")));
    }
    Ok(result)
}

pub fn remove(config: &Config, scheduler: Option<&str>) -> Result<Value> {
    match scheduler_name(config, scheduler)? {
        "orca" => orca_remove(),
        "cron" => cron_remove(),
        _ => launchd_remove(),
    }
}

/// The configured schedule and what each scheduler actually has. `in_sync` is `null` (not
/// applicable) on the schedulers other than the configured one.
pub fn status(config: &Config) -> Result<Value> {
    let schedule = &config.schedule;
    let cron = Cron::parse(&schedule.cron);
    let wanted = cron.as_ref().map(|c| c.text.clone()).unwrap_or_default();
    let agent = runners::get(&config.runner).ok().and_then(|r| r.orca_agent());
    let root = checkout_root();
    let mut launchd = launchd_status(&wanted, &root);
    let mut orca = orca_status(&wanted, agent);
    let mut crontab = cron_status(&wanted, &root);
    for (name, value) in [("launchd", &mut launchd), ("orca", &mut orca), ("cron", &mut crontab)] {
        if name == schedule.scheduler {
            continue;
        }
        if let Some(object) = value.as_object_mut().filter(|o| o.contains_key("in_sync")) {
            object.insert("in_sync".into(), Value::Null);
        }
        for automation in value.get_mut("automations").and_then(Value::as_array_mut).into_iter().flatten() {
            automation["in_sync"] = Value::Null;
        }
    }
    Ok(json!({
        "cron": schedule.cron,
        "cron_error": cron.as_ref().err().map(|e| e.0.clone()),
        "scheduler": schedule.scheduler,
        "scout_hours": schedule.scout_hours,
        "launchd": launchd,
        "orca": orca,
        "crontab": crontab,
    }))
}

/// A short human summary of [`status`]: the configured schedule, then one line per scheduler.
pub fn status_text(status: &Value) -> String {
    let str_or = |v: &Value, empty: &str| v.as_str().filter(|s| !s.is_empty()).unwrap_or(empty).to_string();
    let sync = |v: &Value| match v.as_bool() {
        Some(true) => "설정과 일치",
        Some(false) => "설정과 다름",
        None => "일치 여부 해당 없음",
    };
    let mut lines = vec![format!(
        "설정: cron {} · 스케줄러 {} · ContestScout 시간 {}",
        str_or(&status["cron"], "-"),
        str_or(&status["scheduler"], "-"),
        str_or(&status["scout_hours"], "없음"),
    )];
    if let Some(err) = status["cron_error"].as_str() {
        lines.push(format!("cron 오류: {err}"));
    }
    let describe = |name: &str, job: &Value| -> String {
        if job["available"] == false {
            return format!("{name}: 이 기기에서 쓸 수 없음");
        }
        if let Some(err) = job["error"].as_str() {
            return format!("{name}: 확인 실패 ({err})");
        }
        if job["installed"] != true {
            return format!("{name}: 설치 안 됨");
        }
        format!(
            "{name}: 설치됨 · cron {} · {} · 다음 실행 {}",
            str_or(&job["cron"], "-"),
            sync(&job["in_sync"]),
            str_or(&job["next_run"], "-"),
        )
    };
    let orca = &status["orca"];
    match orca["automations"].as_array().filter(|a| !a.is_empty()) {
        Some(automations) if orca["available"] == true => {
            for automation in automations {
                let mut job = automation.clone();
                job["installed"] = json!(true);
                lines.push(describe("orca", &job));
            }
        }
        _ => lines.push(describe("orca", orca)),
    }
    lines.push(describe("launchd", &status["launchd"]));
    if let Some(legacy) = status["launchd"]["legacy"].as_object() {
        let hint = if legacy.get("this_checkout") == Some(&json!(true)) {
            "이 체크아웃의 예전 작업이다. harness schedule install --scheduler launchd로 새 라벨로 옮긴다"
        } else {
            "다른 체크아웃의 작업이다"
        };
        lines.push(format!(
            "launchd: 예전 라벨 {LEGACY_LABEL}의 plist가 있다 ({}, 체크아웃 {}). {hint}",
            legacy.get("plist").and_then(Value::as_str).unwrap_or("-"),
            legacy.get("root").and_then(Value::as_str).unwrap_or("알 수 없음"),
        ));
    }
    lines.push(describe("cron", &status["crontab"]));
    lines.join("\n")
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
        crate::bail!(
            "러너 {}는 Orca automation으로 띄울 수 없다. schedule.scheduler를 launchd나 cron으로 둔다",
            runner.name()
        )
    };
    if which("orca").is_none() {
        crate::bail!("orca CLI가 PATH에 없다. schedule.scheduler를 launchd나 cron으로 두거나 Orca를 설치한다");
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

/// The launchd label of the checkout at `root`: the legacy label plus a 32-bit FNV-1a hash of the
/// path, so two checkouts get two jobs. FNV is fixed by definition, so the label is stable across
/// builds.
pub fn label(root: &Path) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in root.as_os_str().as_encoded_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{LEGACY_LABEL}.{hash:08x}")
}

pub fn plist_path(label: &str) -> PathBuf {
    paths::home().join("Library").join("LaunchAgents").join(format!("{label}.plist"))
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
        label = label(root),
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

fn unxml(text: &str) -> String {
    text.replace("&quot;", "\"").replace("&gt;", ">").replace("&lt;", "<").replace("&amp;", "&")
}

/// The harness root (`HARNESS_HOME`) recorded in a plist.
pub fn plist_root(plist: &str) -> Option<String> {
    let rest = plist.split("<key>HARNESS_HOME</key>").nth(1)?;
    let value = rest.split_once("<string>")?.1.split_once("</string>")?.0;
    Some(unxml(value.trim()))
}

/// The plist with the legacy fixed label, if any: its path, text and the checkout it runs.
fn legacy_plist() -> Option<(PathBuf, String, Option<String>)> {
    let path = plist_path(LEGACY_LABEL);
    let text = std::fs::read_to_string(&path).ok()?;
    let root = plist_root(&text);
    Some((path, text, root))
}

fn is_root(recorded: Option<&str>, root: &Path) -> bool {
    recorded.is_some_and(|r| Path::new(r) == root)
}

/// Whether this checkout has a launchd job, under its own label or the legacy one.
fn launchd_installed(root: &Path) -> bool {
    plist_path(&label(root)).exists() || legacy_plist().is_some_and(|(_, _, r)| is_root(r.as_deref(), root))
}

fn domain() -> String {
    // SAFETY: getuid has no preconditions and cannot fail.
    format!("gui/{}", unsafe { getuid() })
}

fn launchctl(args: &[&str]) -> Result<std::process::Output> {
    process::output("launchctl", args, None, process::LOCAL)
}

fn macos_only() -> Result<()> {
    if !cfg!(target_os = "macos") {
        crate::bail!("launchd는 macOS에서만 쓸 수 있다. 이 기기에서는 schedule.scheduler를 cron이나 orca로 둔다");
    }
    Ok(())
}

/// The next local run of a cron text, as `YYYY-MM-DD HH:MM`.
fn next_run(cron_text: &str) -> Option<String> {
    let cron = Cron::parse(cron_text).ok()?;
    let at = cron.next_after(chrono::Local::now().naive_local())?;
    Some(chrono::Local.from_local_datetime(&at).earliest()?.format("%Y-%m-%d %H:%M").to_string())
}

fn launchd_install(cron: &Cron) -> Result<Value> {
    macos_only()?;
    let exe = std::env::current_exe()?;
    let root = std::fs::canonicalize(paths::root())?;
    let path_env = std::env::var("PATH").unwrap_or_default();
    let content = plist(&exe, &root, cron, &path_env)?;
    std::fs::create_dir_all(root.join("data").join("logs"))?;
    let label = label(&root);
    let target = plist_path(&label);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target, content)?;
    let domain = domain();
    // This checkout's job under the legacy label would run alongside the new one: retire it.
    let mut legacy_removed = None;
    if let Some((path, _, recorded)) = legacy_plist()
        && is_root(recorded.as_deref(), &root)
    {
        let _ = launchctl(&["bootout", &format!("{domain}/{LEGACY_LABEL}")]);
        std::fs::remove_file(&path)?;
        legacy_removed = Some(path.display().to_string());
    }
    // Re-installing replaces a loaded job; bootout fails harmlessly when none is loaded.
    let _ = launchctl(&["bootout", &format!("{domain}/{label}")])?;
    let target_text = target.display().to_string();
    let out = launchctl(&["bootstrap", &domain, &target_text])?;
    if !out.status.success() {
        crate::bail!(
            "launchctl bootstrap {domain} {target_text} 실패: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let mut result =
        json!({"scheduler": "launchd", "plist": target_text, "label": label, "cron": cron.text, "loaded": true});
    if let Some(path) = legacy_removed {
        result["legacy_removed"] = json!(path);
    }
    Ok(result)
}

fn launchd_remove() -> Result<Value> {
    macos_only()?;
    let root = checkout_root();
    let label = label(&root);
    let target = plist_path(&label);
    let domain = domain();
    let out = launchctl(&["bootout", &format!("{domain}/{label}")])?;
    let existed = target.exists();
    if existed {
        std::fs::remove_file(&target)?;
    }
    let mut result = json!({
        "scheduler": "launchd",
        "plist": target.display().to_string(),
        "label": label,
        "removed": existed,
        "unloaded": out.status.success(),
    });
    if let Some((path, _, recorded)) = legacy_plist() {
        let path_text = path.display().to_string();
        if is_root(recorded.as_deref(), &root) {
            let unloaded = launchctl(&["bootout", &format!("{domain}/{LEGACY_LABEL}")])?.status.success();
            std::fs::remove_file(&path)?;
            result["legacy"] = json!({"plist": path_text, "root": recorded, "removed": true, "unloaded": unloaded});
        } else {
            result["legacy"] = json!({"plist": path_text, "root": recorded, "removed": false});
            result["warning"] = json!(format!(
                "예전 라벨 {LEGACY_LABEL}의 plist는 다른 체크아웃({})의 작업이라 남겼다. 그 체크아웃에서 harness schedule remove --scheduler launchd를 돌린다",
                recorded.as_deref().unwrap_or("알 수 없음")
            ));
        }
    }
    Ok(result)
}

fn launchd_status(wanted: &str, root: &Path) -> Value {
    if !cfg!(target_os = "macos") {
        return json!({"available": false, "installed": false});
    }
    let label = label(root);
    let target = plist_path(&label);
    let domain = domain();
    let cron = std::fs::read_to_string(&target).ok().and_then(|text| plist_cron(&text));
    let print = launchctl(&["print", &format!("{domain}/{label}")]).ok();
    let details = print.as_ref().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let last_exit =
        details.lines().find_map(|l| l.trim().strip_prefix("last exit code = ")).map(|v| v.trim().to_string());
    let last_log = std::fs::read_dir(paths::data().join("logs"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("autorun-")))
        .max();
    let mut result = json!({
        "available": true,
        "label": label,
        "plist": target.display().to_string(),
        "installed": target.exists(),
        "loaded": print.is_some_and(|o| o.status.success()),
        "cron": cron,
        "in_sync": cron.as_deref() == Some(wanted),
        "next_run": cron.as_deref().and_then(next_run),
        "last_exit": last_exit,
        "last_log": last_log.map(|p| p.display().to_string()),
    });
    if let Some((path, text, recorded)) = legacy_plist() {
        let loaded = launchctl(&["print", &format!("{domain}/{LEGACY_LABEL}")]).is_ok_and(|o| o.status.success());
        result["legacy"] = json!({
            "label": LEGACY_LABEL,
            "plist": path.display().to_string(),
            "root": recorded,
            "this_checkout": is_root(recorded.as_deref(), root),
            "loaded": loaded,
            "cron": plist_cron(&text),
        });
    }
    result
}

// ---- crontab ----

fn cron_markers(root: &Path) -> (String, String) {
    let root = root.display();
    (format!("# BEGIN {CRON_TAG} {root}"), format!("# END {CRON_TAG} {root}"))
}

/// The crontab line: `<cron> cd <root> && HARNESS_HOME=<root> PATH=<path> <exe> autorun >> <log> 2>&1`.
/// Every word is shell-quoted and `%` (a newline to cron) is escaped.
pub fn cron_line(exe: &Path, root: &Path, cron: &Cron, path_env: &str) -> Result<String> {
    let log = root.join("data").join("logs").join("cron.log");
    let (exe, root, log) = (exe.display().to_string(), root.display().to_string(), log.display().to_string());
    if let Some(bad) = [&exe, &root, &log, &path_env.to_string()].into_iter().find(|w| w.contains(['\n', '\r'])) {
        crate::bail!("줄바꿈이 든 값은 crontab 줄에 넣을 수 없다: {bad:?}");
    }
    let q = |word: &str| shell_quote(word).replace('%', r"\%");
    Ok(format!(
        "{} cd {root} && HARNESS_HOME={root} PATH={path} {exe} autorun >> {log} 2>&1",
        cron.text,
        root = q(&root),
        path = q(path_env),
        exe = q(&exe),
        log = q(&log),
    ))
}

/// This checkout's crontab block: begin marker, the job line, end marker.
pub fn cron_block(exe: &Path, root: &Path, cron: &Cron, path_env: &str) -> Result<String> {
    let (begin, end) = cron_markers(root);
    Ok(format!("{begin}\n{}\n{end}\n", cron_line(exe, root, cron, path_env)?))
}

/// `crontab` text without this checkout's block; every other line is kept as it was. A begin
/// marker without its end marker is an error rather than a guess at where the block stops.
pub fn crontab_without(current: &str, root: &Path) -> Result<String> {
    let (begin, end) = cron_markers(root);
    let mut out = String::new();
    let mut inside = false;
    for line in current.lines() {
        if inside {
            inside = line.trim_end() != end;
            continue;
        }
        if line.trim_end() == begin {
            inside = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if inside {
        crate::bail!("crontab에서 `{begin}` 블록의 끝 줄 `{end}`을 찾지 못했다. crontab -e로 고친 뒤 다시 실행한다");
    }
    Ok(out)
}

/// `crontab` text with this checkout's block replaced by `block` (appended at the end).
pub fn crontab_with(current: &str, root: &Path, block: &str) -> Result<String> {
    let mut out = crontab_without(current, root)?;
    out.push_str(block);
    Ok(out)
}

/// The job line inside this checkout's block, if the block is there.
pub fn cron_job(current: &str, root: &Path) -> Option<String> {
    let (begin, end) = cron_markers(root);
    let mut lines = current.lines().skip_while(|l| l.trim_end() != begin).skip(1);
    lines
        .by_ref()
        .take_while(|l| l.trim_end() != end)
        .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(str::to_string)
}

/// The 5-field schedule of a crontab job line.
pub fn cron_job_schedule(line: &str) -> String {
    line.split_whitespace().take(5).collect::<Vec<_>>().join(" ")
}

/// Whether `crontab -l` failed only because the user has no crontab yet: vixie/cronie/macOS say
/// "no crontab for <user>", busybox says "can't open '<user>': No such file or directory".
pub fn crontab_missing(stderr: &str) -> bool {
    stderr.contains("no crontab") || (stderr.contains("can't open") && stderr.contains("No such file"))
}

/// `<program> -l`; a user without a crontab reads as empty.
pub fn read_crontab(program: &str) -> Result<String> {
    let out = process::output(program, &["-l"], None, process::LOCAL)?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if crontab_missing(&err) {
        return Ok(String::new());
    }
    crate::bail!("{program} -l 실패: {}", err.trim())
}

/// Replace the user's crontab with `text` through `<program> -`.
pub fn write_crontab(program: &str, text: &str) -> Result<()> {
    let mut child = Command::new(program)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error(format!("{program} 실행 실패: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        crate::bail!("{program} - 실패: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Install or update this checkout's block in the crontab managed by `program`.
pub fn cron_install_with(program: &str, exe: &Path, root: &Path, cron: &Cron, path_env: &str) -> Result<Value> {
    let block = cron_block(exe, root, cron, path_env)?;
    let current = read_crontab(program)?;
    let action = if cron_job(&current, root).is_some() { "updated" } else { "created" };
    let next = crontab_with(&current, root, &block)?;
    std::fs::create_dir_all(root.join("data").join("logs"))?;
    write_crontab(program, &next)?;
    Ok(json!({
        "scheduler": "cron",
        "action": action,
        "cron": cron.text,
        "line": cron_line(exe, root, cron, path_env)?,
        "log": root.join("data").join("logs").join("cron.log").display().to_string(),
    }))
}

/// Remove this checkout's block from the crontab managed by `program`; other lines stay.
pub fn cron_remove_with(program: &str, root: &Path) -> Result<Value> {
    let current = read_crontab(program)?;
    let existed = cron_job(&current, root).is_some() || current.lines().any(|l| l.trim_end() == cron_markers(root).0);
    if existed {
        write_crontab(program, &crontab_without(&current, root)?)?;
    }
    Ok(json!({"scheduler": "cron", "removed": existed}))
}

fn cron_install(cron: &Cron) -> Result<Value> {
    let exe = std::env::current_exe()?;
    let root = std::fs::canonicalize(paths::root())?;
    let path_env = std::env::var("PATH").unwrap_or_default();
    cron_install_with("crontab", &exe, &root, cron, &path_env)
}

fn cron_remove() -> Result<Value> {
    cron_remove_with("crontab", &checkout_root())
}

fn cron_status(wanted: &str, root: &Path) -> Value {
    if which("crontab").is_none() {
        return json!({"available": false, "installed": false});
    }
    match read_crontab("crontab") {
        Err(e) => json!({"available": true, "error": e.0}),
        Ok(text) => {
            let cron = cron_job(&text, root).map(|line| cron_job_schedule(&line));
            json!({
                "available": true,
                "installed": cron.is_some(),
                "cron": cron,
                "in_sync": cron.as_deref() == Some(wanted),
                "next_run": cron.as_deref().and_then(next_run),
                "log": root.join("data").join("logs").join("cron.log").display().to_string(),
            })
        }
    }
}
