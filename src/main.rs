//! `harness`: the Autocontest-Harness CLI. Agents call it; it is the only writer of the registry.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::LazyLock;

use clap::{Parser, Subcommand};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use harness::error::{Error, Result};
use harness::ledger;
use harness::registry::{self, Entry, NewEntry, STATES};
use harness::{
    board, budget, clean, collect, config, dedupe, guard, install, launch, profile, route, runners, schedule, sources,
    text,
};

const EXIT_DUPLICATE: u8 = 3;
/// `0.3.0 (<git sha>)`: the commit tells whether the binary matches the checkout (`harness doctor`).
const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("HARNESS_GIT_SHA"), ")");
const EXIT_PERSONA: u8 = 4;

#[derive(Parser)]
#[command(name = "harness", version = VERSION, about = "Contest automation harness (docs/CONTRACT.md)")]
struct Cli {
    /// Print JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Collect new contests from the listing sites.
    Collect {
        /// A source name or `all`.
        #[arg(long, default_value = "all")]
        source: String,
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        pages: Option<u32>,
    },
    /// Add contests by hand or from ContestScout: one with --title/--url, or many with --from-file.
    Add {
        #[arg(long, required_unless_present = "from_file", conflicts_with = "from_file")]
        title: Option<String>,
        #[arg(long, required_unless_present = "from_file", conflicts_with = "from_file")]
        url: Option<String>,
        #[arg(long, conflicts_with = "from_file")]
        org: Option<String>,
        /// YYYY-MM-DD (taken as 23:59 KST) or YYYY-MM-DDTHH:MM+09:00.
        #[arg(long, conflicts_with = "from_file")]
        deadline: Option<String>,
        /// Default source (`scout` or `manual`); items in --from-file may set their own.
        #[arg(long, default_value = "manual", value_parser = ["scout", "manual"])]
        source: String,
        #[arg(long, conflicts_with = "from_file")]
        note: Option<String>,
        /// A JSON object or array of objects `{title, url, org?, deadline?, source?, note?, categories?}`
        /// (written with a file tool, so fetched text never passes through a shell).
        #[arg(long)]
        from_file: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    /// Find registry entries and contest folders that look like the same contest (exit 3 if any).
    DedupeCheck {
        /// The title to check (default with --id: the entry's title).
        #[arg(required_unless_present = "id")]
        title: Option<String>,
        #[arg(long)]
        url: Option<String>,
        /// Leave this entry out and take its title, URL, organizer and deadline (flags override).
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        org: Option<String>,
        #[arg(long)]
        deadline: Option<String>,
    },
    /// List contests.
    List {
        #[arg(long, value_parser = STATES.to_vec())]
        state: Option<String>,
        #[arg(long = "type")]
        kind: Option<String>,
    },
    /// Show one entry.
    Show { id: String },
    /// Record a gate result (gate.json).
    Gate {
        id: String,
        #[arg(long)]
        file: PathBuf,
    },
    /// Create the folder, repository and workspace for a gated contest.
    Route {
        id: String,
        #[arg(long)]
        folder: Option<String>,
        #[arg(long)]
        no_repo: bool,
        #[arg(long)]
        no_workspace: bool,
    },
    /// Start the contest session.
    Launch {
        id: String,
        /// omp modelPresets name (omp runner only).
        #[arg(long)]
        preset: Option<String>,
        #[arg(long)]
        dry_run: bool,
        /// Launch even when the level or memory limit is reached.
        #[arg(long)]
        force: bool,
    },
    /// Show the tail of a contest session's screen or log.
    Peek {
        id: String,
        #[arg(long, default_value_t = 40)]
        lines: usize,
    },
    /// Scan a contest folder and its commit messages for persona traces (exit 4 if any).
    Check { id: String },
    /// Move an entry to another state.
    Transition {
        id: String,
        #[arg(value_parser = STATES.to_vec())]
        state: String,
        #[arg(long, conflicts_with = "note_file")]
        note: Option<String>,
        /// Read the note from this file (verbatim), so fetched text never passes through a shell.
        #[arg(long)]
        note_file: Option<PathBuf>,
        #[arg(long, default_value = "agent")]
        by: String,
        /// Enter awaiting_approval despite persona-guard findings, with the reason
        /// (for example a character's line in a story).
        #[arg(long)]
        persona_ok: Option<String>,
        /// Enter awaiting_approval (levels 2-5) without a passing final score, with the reason
        /// (for example the deadline).
        #[arg(long)]
        score_ok: Option<String>,
    },
    /// Record the user's approval: ends a level 5 plan review (`plan_review → running`) or approves
    /// the submission (`awaiting_approval`). Refused inside contest sessions (HARNESS_SESSION set).
    Approve {
        id: String,
        /// The user's approval message, verbatim, with its time.
        #[arg(long, required_unless_present = "note_file", conflicts_with = "note_file")]
        note: Option<String>,
        /// Read the approval message from this file (verbatim).
        #[arg(long)]
        note_file: Option<PathBuf>,
        #[arg(long, default_value = "user")]
        by: String,
    },
    /// Record a ContestJudge score (with --stage and --file) or show the score history.
    Score {
        id: String,
        #[arg(long, value_parser = registry::SCORE_STAGES.to_vec(), requires = "file")]
        stage: Option<String>,
        /// ContestJudge output JSON (`.harness/judge/<stage>-<n>.json`).
        #[arg(long, requires = "stage")]
        file: Option<PathBuf>,
        #[arg(long, conflicts_with = "note_file")]
        note: Option<String>,
        /// Read the note from this file (verbatim), so fetched text never passes through a shell.
        #[arg(long)]
        note_file: Option<PathBuf>,
    },
    /// Record a submission. Re-running it on an already submitted entry with the same method and
    /// receipt changes nothing but restores a missing ledger record.
    Submit {
        id: String,
        #[arg(long)]
        method: String,
        #[arg(long, required_unless_present = "receipt_file", conflicts_with = "receipt_file")]
        receipt: Option<String>,
        /// Read the receipt from this file (verbatim).
        #[arg(long)]
        receipt_file: Option<PathBuf>,
        #[arg(long, num_args = 0..)]
        files: Vec<String>,
        /// Approval evidence and other notes.
        #[arg(long, conflicts_with = "note_file")]
        note: Option<String>,
        /// Read the note from this file (verbatim).
        #[arg(long)]
        note_file: Option<PathBuf>,
        #[arg(long, default_value = "agent")]
        by: String,
    },
    /// List contests already submitted (data/submissions.json), or add/import them.
    Submitted {
        #[command(subcommand)]
        action: Option<SubmittedAction>,
    },
    /// Participant profile, a Markdown file you write yourself (data/profile.md): path, eligibility, show.
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },
    /// Close the session and trash the local folder of a submitted contest once it is safe
    /// (recorded in the ledger, committed and pushed).
    Clean {
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        id: Option<String>,
        /// Every submitted or closed contest that still has its folder.
        #[arg(long)]
        all: bool,
        #[arg(long)]
        dry_run: bool,
        /// Delete instead of moving to the Trash.
        #[arg(long)]
        purge: bool,
        /// Skip the safety checks.
        #[arg(long)]
        force: bool,
    },
    /// Remaining quota and recommended workers per provider.
    Budget,
    /// How many sessions can start now (overall and per level).
    Capacity,
    /// Rewrite BOARD.md.
    Board,
    /// One control-loop pass: collect, list what needs a gate, optionally launch, refresh the board.
    Tick {
        #[arg(long)]
        auto_launch: bool,
        #[arg(long)]
        no_collect: bool,
    },
    /// Install agents, the skill and the extension for runner adapters (default: the configured runner).
    Install {
        #[arg(long = "runner")]
        runners: Vec<String>,
        /// Every compiled-in runner whose CLI is installed.
        #[arg(long)]
        all: bool,
    },
    /// Open the main interface session (persona + harness role) at the repository root.
    Chat {
        #[arg(long)]
        runner: Option<String>,
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Run the scheduled automation once with the configured runner (what launchd starts).
    Autorun,
    /// Check that this checkout, its config, the tools it calls and the scheduled run are ready.
    Doctor,
    /// Create the files a new checkout needs (data/, config.json, profile.md from the examples;
    /// existing files are kept), then run the doctor. An agent continues with INIT.md.
    Init,
    /// Read or change data/config.json; every change is validated before it is written.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// The harness agents: default model, the model chosen in `agent_models`, and what the runner
    /// applies now.
    Agents,
    /// Install, remove or inspect the scheduled run (config `schedule`: cron, scheduler, scout_hours).
    Schedule {
        #[command(subcommand)]
        action: ScheduleAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// The config file path.
    Path,
    /// The effective value of a dotted key (defaults + file), or the whole config.
    Get { key: Option<String> },
    /// Set a dotted key (`schedule.cron`, `agent_models.ContestJudge`). The value is read as JSON
    /// when it parses (numbers, true/false, null, "...", [...], {...}), otherwise as a string.
    Set { key: String, value: String },
    /// Remove a key from the file so its default applies again.
    Unset { key: String },
}

#[derive(Subcommand)]
enum ScheduleAction {
    /// Create or update the scheduled run on the scheduler with `schedule.cron`.
    Install {
        /// `orca`, `launchd` or `cron` (default: config `schedule.scheduler`).
        #[arg(long)]
        scheduler: Option<String>,
    },
    /// Remove the scheduled run from the scheduler.
    Remove {
        /// `orca`, `launchd` or `cron` (default: config `schedule.scheduler`).
        #[arg(long)]
        scheduler: Option<String>,
    },
    /// The configured schedule and what Orca and launchd actually have.
    Status,
}

#[derive(Subcommand)]
enum SubmittedAction {
    /// Record a contest submitted outside the harness.
    Add {
        #[arg(long)]
        title: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        org: Option<String>,
        /// YYYY-MM-DD or YYYY-MM-DDTHH:MM+09:00 (default: now).
        #[arg(long)]
        date: Option<String>,
        #[arg(long)]
        method: Option<String>,
        #[arg(long)]
        receipt: Option<String>,
        #[arg(long)]
        folder: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Copy submitted contests from the registry once (existing keys are skipped).
    Import,
}

#[derive(Subcommand)]
enum ProfileAction {
    /// The profile file path and whether it exists.
    Path,
    /// The `## 참가 자격` section only (what ContestGate reads; no personal details).
    Eligibility,
    /// The whole profile, personal details included: only for a contest whose submission the user
    /// approved (`harness approve`) or that is already submitted.
    Show {
        /// The contest id the details are for (inside a contest session: its own id).
        #[arg(long = "for")]
        for_id: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // --help and --version never get here (clap answers them); every command needs the checkout.
    // Doctor reports a missing checkout itself, with the fix.
    let result = if matches!(cli.command, Command::Doctor) {
        run(&cli)
    } else {
        harness::paths::checked_root().and_then(|_| run(&cli))
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("오류: {err}");
            ExitCode::from(1)
        }
    }
}

/// Print the text (or JSON) on stdout. A reader that went away (`| head`) is not an error: the
/// work is already done, so the rest of the output is dropped quietly.
fn emit(json_mode: bool, payload: &impl Serialize, text: Option<String>) -> Result<()> {
    use std::io::Write;
    let out = match text {
        Some(text) if !json_mode => text,
        _ => serde_json::to_string_pretty(payload)?,
    };
    let mut stdout = std::io::stdout().lock();
    match writeln!(stdout, "{out}").and_then(|()| stdout.flush()) {
        Err(err) if err.kind() != std::io::ErrorKind::BrokenPipe => Err(err.into()),
        _ => Ok(()),
    }
}

fn line(e: &Entry) -> String {
    line_at(e, e.id.chars().count())
}

/// One row per entry with the id column as wide as the longest id.
fn rows<'a>(entries: impl IntoIterator<Item = &'a Entry> + Clone) -> Vec<String> {
    let width = entries.clone().into_iter().map(|e| e.id.chars().count()).max().unwrap_or(0);
    entries.into_iter().map(|e| line_at(e, width)).collect()
}

fn line_at(e: &Entry, id_width: usize) -> String {
    let level = e.level.map(|l| format!("L{l}")).unwrap_or_else(|| "-".into());
    let today = chrono::Utc::now().with_timezone(&text::kst()).date_naive();
    let dday = board::dday(e.deadline.as_deref(), today);
    let title: String = e.title.chars().take(60).collect();
    let kind = e.kind.as_deref().unwrap_or("-");
    format!("{:<id_width$} {:<17} {:<7} {:<8} {:<3} {}", e.id, e.state, dday, kind, level, title)
}

/// A string field of a JSON result for text output: `-` when it is missing or null.
fn shown(value: &Value) -> &str {
    value.as_str().filter(|s| !s.is_empty()).unwrap_or("-")
}

/// `\n경고: …` for each string in the result's `warnings`.
fn warning_lines(result: &Value) -> String {
    let warnings = result["warnings"].as_array().map(Vec::as_slice).unwrap_or_default();
    warnings.iter().filter_map(Value::as_str).map(|w| format!("\n경고: {w}")).collect()
}

/// The contents of a `--*-file` option, verbatim; errors name the file.
fn read_file(path: &std::path::Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|err| Error(format!("{}: {err}", path.display())))
}

/// `--note`/`--note-file` (they are mutually exclusive).
fn note_value(note: &Option<String>, file: &Option<PathBuf>) -> Result<Option<String>> {
    match file {
        Some(path) => read_file(path).map(Some),
        None => Ok(note.clone()),
    }
}

fn run(cli: &Cli) -> Result<u8> {
    let json_mode = cli.json;
    match &cli.command {
        Command::Collect { source, pages } => {
            let cfg = config::load()?;
            if source != "all" && sources::find(source).is_none() {
                let names: Vec<&str> = sources::SOURCES.iter().map(|s| s.name).collect();
                return Err(Error(format!("알 수 없는 출처: {source} (가능: {}, all)", names.join(", "))));
            }
            let report = collect::collect(&cfg, source, pages.unwrap_or(cfg.collect_pages))?;
            let new_lines = rows(&report.new);
            let seen: Vec<String> = report.seen.iter().map(|(k, v)| format!("{k} {v}")).collect();
            let mut lines = vec![
                format!(
                    "새 공고 {}건, 병합 {}건, 중복 {}건",
                    report.added.len(),
                    report.merged.len(),
                    report.duplicates.len()
                ),
                format!("출처별 목록: {}", seen.join(", ")),
            ];
            lines.extend(new_lines.iter().cloned());
            lines.extend(report.errors.iter().map(|e| format!("오류: {e}")));
            let mut payload = serde_json::to_value(&report)?;
            payload["new"] = json!(new_lines);
            emit(json_mode, &payload, Some(lines.join("\n")))?;
            let failed = !report.errors.is_empty() && report.added.is_empty() && report.merged.is_empty();
            Ok(u8::from(failed))
        }
        Command::Add { title, url, org, deadline, source, note, from_file, force } => {
            let items = match from_file {
                Some(path) => add_specs_from_file(path, source)?,
                None => {
                    let spec = AddSpec {
                        title: title.clone().unwrap_or_default(),
                        url: url.clone().unwrap_or_default(),
                        org: org.clone(),
                        deadline: deadline.clone(),
                        source: None,
                        note: note.clone(),
                        categories: Vec::new(),
                    };
                    vec![add_item(spec, source)?]
                }
            };
            let cfg = config::load()?;
            let submitted = ledger::load_at(&ledger::path())?;
            let outcomes = registry::write(|reg| {
                Ok(items.into_iter().map(|item| add_one(reg, &cfg, &submitted, item, *force)).collect::<Vec<_>>())
            })?;
            let held = outcomes.iter().any(|o| matches!(o, Added::Held { .. }));
            if from_file.is_none() {
                match outcomes.into_iter().next() {
                    Some(Added::Entry(entry)) => emit(json_mode, &entry, Some(format!("추가: {}", line(&entry))))?,
                    Some(Added::Held { matches, .. }) => {
                        let listing = serde_json::to_string_pretty(&matches)?;
                        emit(
                            json_mode,
                            &json!({"duplicate": true, "matches": matches}),
                            Some(format!(
                                "같은 공모전으로 보이는 항목이 있어 추가하지 않았다(--force로 강제):\n{listing}"
                            )),
                        )?;
                    }
                    None => {}
                }
            } else {
                let added: Vec<&Entry> =
                    outcomes.iter().filter_map(|o| if let Added::Entry(e) = o { Some(&**e) } else { None }).collect();
                let duplicates: Vec<Value> = outcomes
                    .iter()
                    .filter_map(|o| match o {
                        Added::Held { title, url, matches } => {
                            Some(json!({"title": title, "url": url, "matches": matches}))
                        }
                        Added::Entry(_) => None,
                    })
                    .collect();
                let mut lines =
                    vec![format!("추가 {}건, 중복으로 보류 {}건(--force로 강제)", added.len(), duplicates.len())];
                lines.extend(rows(added.iter().copied()).into_iter().map(|l| format!("추가: {l}")));
                for dup in &duplicates {
                    lines.push(format!("보류: {} {}", shown(&dup["title"]), serde_json::to_string(&dup["matches"])?));
                }
                emit(json_mode, &json!({"added": added, "duplicates": duplicates}), Some(lines.join("\n")))?;
            }
            Ok(if held { EXIT_DUPLICATE } else { 0 })
        }
        Command::DedupeCheck { title, url, id, org, deadline } => {
            let cfg = config::load()?;
            let submitted = ledger::load_at(&ledger::path())?;
            let matches = registry::read(|reg| {
                let own = id.as_deref().and_then(|i| reg.contests.get(i));
                let title = match (title.as_deref(), own) {
                    (Some(title), _) => title,
                    (None, Some(entry)) => entry.title.as_str(),
                    (None, None) => match id {
                        Some(id) => return Err(Error(format!("레지스트리에 없는 id: {id}"))),
                        None => return Err(Error("제목이나 --id가 필요하다".into())),
                    },
                };
                let query = dedupe::Query {
                    title,
                    url: url.as_deref().or(own.map(|e| e.url.as_str())).unwrap_or(""),
                    include_folders: true,
                    exclude_id: id.as_deref(),
                    strict: false,
                    organizer: org.as_deref().or(own.map(|e| e.organizer.as_str())).unwrap_or(""),
                    deadline: deadline.as_deref().or(own.and_then(|e| e.deadline.as_deref())),
                    submitted: Some(&submitted),
                };
                Ok(dedupe::find_matches(reg, &cfg, &query))
            })?;
            let text = if matches.is_empty() {
                "겹치는 항목 없음".to_string()
            } else {
                matches.iter().map(serde_json::to_string).collect::<std::result::Result<Vec<_>, _>>()?.join("\n")
            };
            emit(json_mode, &json!({"matches": matches}), Some(text))?;
            Ok(if matches.is_empty() { 0 } else { EXIT_DUPLICATE })
        }
        Command::List { state, kind } => {
            let mut rows: Vec<Entry> = registry::read(|reg| {
                Ok(reg
                    .contests
                    .values()
                    .filter(|e| {
                        state.as_ref().is_none_or(|s| &e.state == s)
                            && kind.as_ref().is_none_or(|k| e.kind.as_ref() == Some(k))
                    })
                    .cloned()
                    .collect())
            })?;
            rows.sort_by(|a, b| a.deadline.as_deref().unwrap_or("9999").cmp(b.deadline.as_deref().unwrap_or("9999")));
            let text = if rows.is_empty() { "(없음)".to_string() } else { self::rows(&rows).join("\n") };
            emit(json_mode, &rows, Some(text))?;
            Ok(0)
        }
        Command::Show { id } => {
            let entry = registry::read(|reg| Ok(reg.get(id)?.clone()))?;
            emit(json_mode, &entry, None)?;
            Ok(0)
        }
        Command::Gate { id, file } => {
            let raw = read_file(file)?;
            let gate: Value = serde_json::from_str(&raw).map_err(|err| Error(format!("{}: {err}", file.display())))?;
            let entry = registry::write(|reg| {
                let entry = reg.get_mut(id)?;
                registry::apply_gate(entry, gate, "ContestGate")?;
                Ok(entry.clone())
            })?;
            emit(json_mode, &entry, Some(format!("게이트 기록: {}", line(&entry))))?;
            Ok(0)
        }
        Command::Route { id, folder, no_repo, no_workspace } => {
            let cfg = config::load()?;
            let submitted = ledger::load_at(&ledger::path())?;
            // gh/git/orca run on a copy without the registry lock; only a finished route is stored.
            let base = registry::read(|reg| Ok(reg.get(id)?.clone()))?;
            let (_, summary) = route_stored(&base, &cfg, &submitted, folder.as_deref(), !no_repo, !no_workspace)?;
            let mut text = format!(
                "라우팅: {} (저장소 {}, 작업 공간 {})",
                shown(&summary["folder"]),
                shown(&summary["repo"]),
                shown(&summary["workspace_id"]),
            );
            text.push_str(&warning_lines(&summary));
            emit(json_mode, &summary, Some(text))?;
            Ok(0)
        }
        Command::Launch { id, preset, dry_run, force } => {
            let cfg = config::load()?;
            let result = if *dry_run {
                let mut copy = registry::read(|reg| Ok(reg.get(id)?.clone()))?;
                launch::launch(&mut copy, &cfg, preset.as_deref(), true)?
            } else {
                // One launch at a time: the capacity seen here stays true until this one is stored.
                let _guard = registry::launch_guard()?;
                let (base, running) = registry::read(|reg| Ok((reg.get(id)?.clone(), reg.running_by_level())))?;
                if base.state == "routed" && !force {
                    let cap = budget::capacity(&cfg, &running);
                    let free = base.level.and_then(|l| cap["slots_by_level"][l.to_string()].as_u64()).unwrap_or(0);
                    if free == 0 {
                        let budget_gb = cap["ram"]["budget_gb"].as_f64().map(|gb| gb.to_string());
                        return Err(Error(format!(
                            "{id}: 지금은 레벨 {} 공모전을 더 띄우지 않는다(진행 중 {running:?}, 레벨별 상한 {:?}, 메모리 예산 {}GB·압박 {}, 전체 여유 {}). 넘기려면 --force",
                            base.level.map(|l| l.to_string()).unwrap_or_else(|| "-".into()),
                            cfg.max_running_by_level,
                            budget_gb.as_deref().unwrap_or("알 수 없음"),
                            cap["ram"]["pressure"].as_str().unwrap_or("unknown"),
                            cap["slots"],
                        )));
                    }
                }
                launch_stored(&base, &cfg, preset.as_deref())?
            };
            let mut text = format!("실행 명령: {}\n세션: {}", shown(&result["command"]), shown(&result["handle"]));
            if let Some(prompt) = result["prompt_text"].as_str() {
                text.push_str(&format!("\n\n{prompt}"));
            }
            text.push_str(&warning_lines(&result));
            emit(json_mode, &result, Some(text))?;
            Ok(0)
        }
        Command::Peek { id, lines } => {
            let entry = registry::read(|reg| Ok(reg.get(id)?.clone()))?;
            let tail = launch::peek(&entry, *lines)?;
            emit(json_mode, &json!({"id": id, "tail": tail}), Some(tail.clone()))?;
            Ok(0)
        }
        Command::Check { id } => {
            let cfg = config::load()?;
            let entry = registry::read(|reg| Ok(reg.get(id)?.clone()))?;
            let terms = guard::terms(&cfg);
            let findings = persona_findings(&cfg, &entry)?;
            let text = if terms.is_empty() {
                "검사할 페르소나 표현이 없다(설정 persona.guard_terms와 persona.call_user가 비어 있음)".to_string()
            } else if findings.is_empty() {
                "페르소나 흔적 없음".to_string()
            } else {
                guard::describe(&findings, 50)
            };
            emit(json_mode, &json!({"id": id, "terms": terms, "findings": findings}), Some(text))?;
            Ok(if findings.is_empty() { 0 } else { EXIT_PERSONA })
        }
        Command::Transition { id, state, note, note_file, by, persona_ok, score_ok } => {
            let mut note = note_value(note, note_file)?.unwrap_or_default();
            if state == "awaiting_approval" {
                let cfg = config::load()?;
                let current = registry::read(|reg| Ok(reg.get(id)?.clone()))?;
                let findings = persona_findings(&cfg, &current)?;
                if !findings.is_empty() {
                    let Some(reason) = persona_ok else {
                        return Err(Error(format!(
                            "{id}: 결과물이나 커밋 메시지에 페르소나 흔적이 있어 제출 승인 단계로 가지 않는다. 지우고 다시 시도한다(작품 속 대사처럼 의도한 표현이면 --persona-ok \"<사유>\"):\n{}",
                            guard::describe(&findings, 20)
                        )));
                    };
                    let exception = format!("페르소나 검사 예외({}건): {reason}", findings.len());
                    note = if note.is_empty() { exception } else { format!("{note} / {exception}") };
                }
            }
            let entry = registry::write(|reg| {
                let entry = reg.get_mut(id)?;
                // The score gate runs only for allowed moves, so an illegal move keeps its own error.
                let mut note = note;
                if registry::allowed(&entry.state).contains(&state.as_str())
                    && let Some(exception) = registry::score_gate(entry, state, score_ok.as_deref())?
                {
                    note = if note.is_empty() { exception } else { format!("{note} / {exception}") };
                }
                if state == "submitted" {
                    registry::check_submit_approved(entry)?;
                }
                registry::transition(entry, state, by, &note)?;
                Ok(entry.clone())
            })?;
            emit(json_mode, &entry, Some(format!("상태 변경: {}", line(&entry))))?;
            Ok(0)
        }
        Command::Approve { id, note, note_file, by } => {
            let note = note_value(note, note_file)?.unwrap_or_default();
            let session = std::env::var(launch::SESSION_ENV).ok();
            let (entry, stage) = registry::write(|reg| {
                let entry = reg.get_mut(id)?;
                let stage = registry::approve(entry, &note, by, session.as_deref())?;
                Ok((entry.clone(), stage))
            })?;
            let what = if stage == "plan" { "계획 승인(running으로)" } else { "제출 승인" };
            emit(json_mode, &entry, Some(format!("{what}: {}", line(&entry))))?;
            Ok(0)
        }
        Command::Submit { id, method, receipt, receipt_file, files, note, note_file, by } => {
            let receipt = match receipt_file {
                Some(path) => read_file(path)?,
                None => receipt.clone().unwrap_or_default(),
            };
            let note = note_value(note, note_file)?.unwrap_or_default();
            let ledger_path = ledger::path();
            // Registry and ledger change together: the ledger is written inside the registry write
            // (lock order registry → ledger, as in clean), so a ledger failure leaves the registry as
            // it was and the same command can simply be run again.
            let (entry, repeated) = registry::write(|reg| {
                let entry = reg.get_mut(id)?;
                let last = entry.submissions.last();
                let repeated = entry.state == "submitted"
                    && last.is_some_and(|s| s["method"] == json!(method) && s["receipt"] == json!(receipt));
                let at = if repeated {
                    last.and_then(|s| s["at"].as_str()).map(str::to_string).unwrap_or_else(text::now)
                } else {
                    let joined: Vec<&str> =
                        [receipt.as_str(), note.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
                    registry::check_submit_approved(entry)?;
                    registry::transition(entry, "submitted", by, &joined.join(" / "))?;
                    let at = text::now();
                    entry.submissions.push(json!({"at": at, "method": method, "receipt": receipt, "files": files}));
                    at
                };
                let mut item = ledger::from_submit(entry, method, &receipt, files, &note);
                item.submitted_at = at;
                ledger::write_at(&ledger_path, |l| {
                    // A repeat keeps a record that is already there; it only restores a missing one.
                    if !(repeated && l.get(&item.key).is_some()) {
                        l.upsert(item);
                    }
                    Ok(())
                })?;
                Ok((entry.clone(), repeated))
            })?;
            let verb = if repeated { "이미 제출 기록됨(제출 원장 확인)" } else { "제출 기록" };
            emit(json_mode, &entry, Some(format!("{verb}: {}", line(&entry))))?;
            Ok(0)
        }
        Command::Score { id, stage, file, note, note_file } => {
            let (Some(stage), Some(file)) = (stage, file) else {
                let entry = registry::read(|reg| Ok(reg.get(id)?.clone()))?;
                let text = if entry.scores.is_empty() {
                    format!("{id}: 점수 기록 없음")
                } else {
                    entry.scores.iter().map(score_line).collect::<Vec<_>>().join("\n")
                };
                emit(json_mode, &json!({"id": id, "scores": entry.scores}), Some(text))?;
                return Ok(0);
            };
            let note = note_value(note, note_file)?;
            let raw = read_file(file)?;
            let judge: Value = serde_json::from_str(&raw).map_err(|err| Error(format!("{}: {err}", file.display())))?;
            let shown = file.canonicalize().unwrap_or_else(|_| file.clone());
            let record =
                registry::score_record(&judge, stage, &shown.to_string_lossy(), note.as_deref().unwrap_or(""))?;
            registry::write(|reg| {
                reg.get_mut(id)?.scores.push(record.clone());
                Ok(())
            })?;
            emit(json_mode, &record, Some(format!("점수 기록: {id} {}", score_line(&record))))?;
            Ok(0)
        }
        Command::Submitted { action } => submitted(json_mode, action.as_ref()),
        Command::Profile { action } => profile_command(json_mode, action),
        Command::Clean { id, all: _, dry_run, purge, force } => {
            let opts = clean::Options { dry_run: *dry_run, purge: *purge, force: *force, ..clean::Options::default() };
            let outcomes = clean::run(&harness::paths::registry(), &ledger::path(), id.as_deref(), &opts)?;
            let lines: Vec<String> = outcomes
                .iter()
                .map(|o| match (o.ok, *dry_run) {
                    (true, true) => format!("{} 정리 가능: {}", o.id, o.actions.join(" / ")),
                    (true, false) => format!("{} 정리함: {}", o.id, o.actions.join(" / ")),
                    (false, _) => format!("{} 정리 안 함: {}", o.id, o.problems.join(" / ")),
                })
                .collect();
            let text = if lines.is_empty() { "정리할 공모전 없음".to_string() } else { lines.join("\n") };
            if id.is_some() {
                emit(json_mode, &outcomes[0], Some(text))?;
            } else {
                emit(json_mode, &outcomes, Some(text))?;
            }
            Ok(u8::from(id.is_some() && !outcomes[0].ok))
        }
        Command::Budget => {
            let cfg = config::load()?;
            let report = budget::usage_report();
            let wins = budget::windows(&report, chrono::Utc::now().timestamp_millis());
            let rec = budget::recommend(&wins, cfg.max_workers);
            let mut lines: Vec<String> = wins
                .iter()
                .map(|w| {
                    format!(
                        "{:<14} {:<22} 남음 {:.0}% 남은시간 {:.0}% pace {}",
                        w.provider,
                        w.window,
                        w.remaining * 100.0,
                        w.time_left * 100.0,
                        w.pace
                    )
                })
                .collect();
            lines.extend(
                rec.iter()
                    .map(|(p, r)| format!("→ {p}: 워커 {}명 ({})", r["workers"], r["mode"].as_str().unwrap_or(""))),
            );
            emit(json_mode, &json!({"windows": wins, "recommendation": rec}), Some(lines.join("\n")))?;
            Ok(0)
        }
        Command::Capacity => {
            let cfg = config::load()?;
            let running = registry::read(|reg| Ok(reg.running_by_level()))?;
            let cap = budget::capacity(&cfg, &running);
            emit(json_mode, &cap, Some(capacity_text(&cap, &running)))?;
            Ok(0)
        }
        Command::Board => {
            let path = registry::read(board::write)?;
            emit(json_mode, &json!({"board": path}), Some(format!("보드 갱신: {}", path.display())))?;
            Ok(0)
        }
        Command::Tick { auto_launch, no_collect } => tick(json_mode, *auto_launch, *no_collect),
        Command::Install { runners: chosen, all } => {
            let cfg = config::load()?;
            let names: Vec<String> = if *all {
                runners::ALL.iter().filter(|r| r.available()).map(|r| r.name().to_string()).collect()
            } else if chosen.is_empty() {
                vec![cfg.runner.clone()]
            } else {
                chosen.clone()
            };
            let done = install::install(&cfg, &names)?;
            emit(json_mode, &done, Some(done.join("\n")))?;
            Ok(0)
        }
        Command::Chat { runner, workspace } => {
            let cfg = config::load()?;
            let result = launch::chat(&cfg, runner.as_deref(), workspace.as_deref())?;
            emit(json_mode, &result, Some(format!("메인 세션: {}", result["handle"].as_str().unwrap_or("-"))))?;
            Ok(0)
        }
        Command::Autorun => {
            let cfg = config::load()?;
            let result = schedule::autorun(&cfg)?;
            emit(json_mode, &result, None)?;
            Ok(u8::from(result["exit_code"].as_i64().is_some_and(|c| c != 0)))
        }
        Command::Doctor => {
            let checks = harness::doctor::run();
            emit(json_mode, &json!({"checks": checks}), Some(harness::doctor::text(&checks)))?;
            Ok(u8::from(checks.iter().any(|c| c.level == harness::doctor::Level::Fail)))
        }
        Command::Init => {
            let report = harness::init::init(&harness::paths::root())?;
            emit(json_mode, &report, Some(harness::init::text(&report)))?;
            Ok(0)
        }
        Command::Config { action } => {
            let path = harness::paths::config_file();
            let (result, text) = match action {
                ConfigAction::Path => (json!({"path": path, "exists": path.exists()}), path.display().to_string()),
                ConfigAction::Get { key } => {
                    let value = config::get_at(&path, key.as_deref())?;
                    let text = match &value {
                        Value::String(s) => s.clone(),
                        other => serde_json::to_string_pretty(other)?,
                    };
                    (value, text)
                }
                ConfigAction::Set { key, value } => {
                    let parsed = serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.clone()));
                    let shown = serde_json::to_string(&parsed)?;
                    config::set_at(&path, key, parsed)?;
                    (
                        json!({"path": path, "key": key, "value": config::get_at(&path, Some(key))?}),
                        format!("{key} = {shown}"),
                    )
                }
                ConfigAction::Unset { key } => {
                    config::unset_at(&path, key)?;
                    let value = config::get_at(&path, Some(key)).unwrap_or(Value::Null);
                    let text = format!("{key}: 기본값으로 돌아감({})", serde_json::to_string(&value)?);
                    (json!({"path": path, "key": key, "value": value}), text)
                }
            };
            emit(json_mode, &result, Some(text))?;
            Ok(0)
        }
        Command::Agents => {
            let cfg = config::load()?;
            let runner = runners::get(&cfg.runner)?;
            let applied = runner.agent_models().unwrap_or_default();
            let rows: Vec<Value> = runners::agent_docs(&harness::paths::root())?
                .into_iter()
                .map(|doc| {
                    let configured = cfg.agent_models.get(&doc.name).cloned().flatten();
                    json!({
                        "name": doc.name,
                        "description": doc.description,
                        "default_model": doc.model,
                        "configured": configured,
                        "applied": applied.get(&doc.name),
                    })
                })
                .collect();
            let text = rows
                .iter()
                .map(|r| {
                    let default = r["default_model"]
                        .as_array()
                        .map(|m| m.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "));
                    format!(
                        "{:<17} 기본 {:<20} 설정 {:<36} 적용 {}",
                        shown(&r["name"]),
                        default.unwrap_or_else(|| "-".into()),
                        shown(&r["configured"]),
                        shown(&r["applied"])
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            emit(json_mode, &rows, Some(text))?;
            Ok(0)
        }
        Command::Schedule { action } => {
            let cfg = config::load()?;
            let (result, text) = match action {
                ScheduleAction::Install { scheduler } => (schedule::install(&cfg, scheduler.as_deref())?, None),
                ScheduleAction::Remove { scheduler } => (schedule::remove(&cfg, scheduler.as_deref())?, None),
                ScheduleAction::Status => {
                    let status = schedule::status(&cfg)?;
                    let text = schedule::status_text(&status);
                    (status, Some(text))
                }
            };
            emit(json_mode, &result, text)?;
            Ok(0)
        }
    }
}

fn persona_findings(cfg: &config::Config, entry: &Entry) -> Result<Vec<guard::Finding>> {
    match &entry.folder {
        Some(folder) => guard::scan(std::path::Path::new(folder), &guard::terms(cfg)),
        None => Ok(Vec::new()),
    }
}

/// `시각 단계 총점/목표 판정 · 깎인 기준: 이름 -n, …` (the three largest losses).
fn score_line(score: &Value) -> String {
    let mut losses: Vec<(String, f64)> = score["criteria"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|c| {
                    let lost = c["weight"].as_f64().unwrap_or(0.0) - c["score"].as_f64().unwrap_or(0.0);
                    (c["name"].as_str().unwrap_or("-").to_string(), lost)
                })
                .filter(|(_, lost)| *lost > 0.0)
                .collect()
        })
        .unwrap_or_default();
    losses.sort_by(|a, b| b.1.total_cmp(&a.1));
    let lost: Vec<String> =
        losses.iter().take(3).map(|(name, lost)| format!("{name} -{}", registry::points(*lost))).collect();
    format!(
        "{} {:<5} {} {}{}",
        score["at"].as_str().unwrap_or("-"),
        score["stage"].as_str().unwrap_or("-"),
        registry::score_label(score),
        score["verdict"].as_str().unwrap_or("-"),
        if lost.is_empty() { String::new() } else { format!(" · 깎인 기준: {}", lost.join(", ")) }
    )
}

fn submitted(json_mode: bool, action: Option<&SubmittedAction>) -> Result<u8> {
    let path = ledger::path();
    match action {
        None => {
            let mut items = ledger::load_at(&path)?.submissions;
            items.sort_by(|a, b| a.submitted_at.cmp(&b.submitted_at));
            let dash = |s: &str| if s.is_empty() { "-".to_string() } else { s.to_string() };
            let mut lines = vec![format!("제출 기록 {}건 ({})", items.len(), path.display())];
            lines.extend(items.iter().map(|s| {
                let title: String = s.title.chars().take(60).collect();
                format!(
                    "{:<16} {title} | 방법 {} | 증빙 {}",
                    s.submitted_at.chars().take(16).collect::<String>().replace('T', " "),
                    dash(&s.method),
                    dash(&s.receipt)
                )
            }));
            emit(json_mode, &items, Some(lines.join("\n")))?;
        }
        Some(SubmittedAction::Add { title, url, org, date, method, receipt, folder, note }) => {
            let mut item = ledger::manual(ledger::Manual {
                title,
                url: url.as_deref().unwrap_or(""),
                organizer: org.as_deref().unwrap_or(""),
                date: date.as_deref(),
                method: method.as_deref().unwrap_or(""),
                receipt: receipt.as_deref().unwrap_or(""),
                folder: folder.as_deref(),
                note: note.as_deref().unwrap_or(""),
            })?;
            let added = ledger::write_at(&path, |l| Ok(l.add_manual(&mut item)))?;
            let verb = if added { "추가" } else { "갱신" };
            let text = format!("제출 기록 {verb}: {} {} ({})", item.submitted_at, item.title, item.key);
            emit(json_mode, &json!({"added": added, "submission": item}), Some(text))?;
        }
        Some(SubmittedAction::Import) => {
            let reg = registry::read(|reg| Ok(reg.clone()))?;
            let report = ledger::write_at(&path, |l| Ok(l.import(&reg)))?;
            let mut lines = vec![format!(
                "레지스트리에서 {}건 옮김, 이미 있어 건너뜀 {}건",
                report.added.len(),
                report.skipped.len()
            )];
            lines.extend(report.added.iter().map(|s| format!("- {} {} ({})", s.submitted_at, s.title, s.key)));
            emit(json_mode, &report, Some(lines.join("\n")))?;
        }
    }
    Ok(0)
}

fn profile_command(json_mode: bool, action: &ProfileAction) -> Result<u8> {
    let path = profile::path();
    match action {
        ProfileAction::Path => {
            let text = if path.exists() {
                path.display().to_string()
            } else {
                format!("{} (아직 없음: data/profile.example.md를 복사해 직접 쓴다)", path.display())
            };
            emit(json_mode, &json!({"path": path, "exists": path.exists()}), Some(text))?;
        }
        ProfileAction::Eligibility => {
            let text = profile::read_at(&path)?;
            warn_profile(&path, &text);
            let eligibility = profile::eligibility(&text)?;
            emit(json_mode, &json!({"path": path, "eligibility": eligibility}), Some(eligibility.clone()))?;
        }
        ProfileAction::Show { for_id } => {
            let session = std::env::var(launch::SESSION_ENV).ok();
            registry::read(|reg| registry::check_personal_access(reg.get(for_id)?, session.as_deref()))?;
            let text = profile::read_at(&path)?;
            warn_profile(&path, &text);
            emit(json_mode, &json!({"path": path, "text": text}), Some(text.clone()))?;
        }
    }
    Ok(0)
}

/// `[a-z0-9-]` key from the URL; a URL without letters or digits falls back to a hash of the title.
fn slug(url: &str, title: &str) -> String {
    static NON_ALNUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^0-9a-z]+").expect("valid regex"));
    let key: String = NON_ALNUM
        .replace_all(&dedupe::normalize_url(url).to_lowercase(), "-")
        .trim_matches('-')
        .chars()
        .take(48)
        .collect();
    let key = key.trim_end_matches('-');
    if !key.is_empty() {
        return key.to_string();
    }
    let hash = title.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{:08x}", hash as u32)
}

/// One `add` item as given (flags or one object of `--from-file`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddSpec {
    title: String,
    url: String,
    #[serde(default, alias = "organizer")]
    org: Option<String>,
    #[serde(default)]
    deadline: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
}

/// A checked `add` item: one-line title, web URL, normalised deadline, known source.
struct AddItem {
    title: String,
    url: String,
    org: String,
    deadline: Option<String>,
    source: String,
    note: Option<String>,
    categories: Vec<String>,
}

fn add_item(spec: AddSpec, default_source: &str) -> Result<AddItem> {
    let title = text::one_line(&spec.title);
    if title.is_empty() {
        return Err(Error("title이 비어 있다".into()));
    }
    let url = spec.url.trim().to_string();
    if url.is_empty() {
        return Err(Error("url이 비어 있다".into()));
    }
    if !text::is_web_url(&url) {
        return Err(Error(format!("url은 http:// 또는 https:// 주소여야 한다: {url}")));
    }
    let deadline = match spec.deadline.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(raw) => Some(text::normalize_deadline(raw).ok_or_else(|| {
            Error(format!("deadline은 YYYY-MM-DD 또는 YYYY-MM-DDTHH:MM+09:00 형식의 실제 날짜여야 한다: {raw}"))
        })?),
        None => None,
    };
    let source = spec.source.unwrap_or_else(|| default_source.to_string());
    if !["scout", "manual"].contains(&source.as_str()) {
        return Err(Error(format!("source는 scout 또는 manual이어야 한다: {source}")));
    }
    Ok(AddItem {
        title,
        url,
        org: spec.org.unwrap_or_default(),
        deadline,
        source,
        note: spec.note,
        categories: spec.categories,
    })
}

/// Every item of an `add --from-file` file, all checked before anything is written.
fn add_specs_from_file(path: &std::path::Path, default_source: &str) -> Result<Vec<AddItem>> {
    let at = |err: &dyn std::fmt::Display| Error(format!("{}: {err}", path.display()));
    let value: Value = serde_json::from_str(&read_file(path)?).map_err(|err| at(&err))?;
    let values = match value {
        Value::Array(values) => values,
        Value::Object(_) => vec![value],
        _ => return Err(at(&"JSON 객체나 객체 배열이어야 한다")),
    };
    if values.is_empty() {
        return Err(at(&"추가할 항목이 없다"));
    }
    values
        .into_iter()
        .enumerate()
        .map(|(i, value)| {
            let spec: AddSpec = serde_json::from_value(value).map_err(|err| at(&format!("[{i}] {err}")))?;
            add_item(spec, default_source).map_err(|err| at(&format!("[{i}] {err}")))
        })
        .collect()
}

enum Added {
    Entry(Box<Entry>),
    Held { title: String, url: String, matches: Vec<Value> },
}

/// Insert one item unless it looks like a contest already known (then it is held back, unless
/// `force`). The id is `<source>-<url slug>`, with `-2`, `-3`, … when that is taken.
fn add_one(
    reg: &mut registry::Registry,
    cfg: &config::Config,
    submitted: &ledger::Ledger,
    item: AddItem,
    force: bool,
) -> Added {
    let query = dedupe::Query {
        title: &item.title,
        url: &item.url,
        include_folders: true,
        exclude_id: None,
        strict: false,
        organizer: "",
        deadline: None,
        submitted: Some(submitted),
    };
    let matches = dedupe::find_matches(reg, cfg, &query);
    if !matches.is_empty() && !force {
        return Added::Held { title: item.title, url: item.url, matches };
    }
    let base = format!("{}-{}", item.source, slug(&item.url, &item.title));
    let mut entry_id = base.clone();
    let mut n = 2;
    while reg.contests.contains_key(&entry_id) {
        entry_id = format!("{base}-{n}");
        n += 1;
    }
    let entry = registry::new_entry(NewEntry {
        id: &entry_id,
        title: &item.title,
        url: &item.url,
        source: &item.source,
        organizer: &item.org,
        deadline: item.deadline,
        categories: item.categories,
        note: item.note,
    });
    reg.contests.insert(entry_id, entry.clone());
    Added::Entry(Box::new(entry))
}

fn capacity_text(cap: &Value, running: &std::collections::BTreeMap<u8, usize>) -> String {
    let by_level: Vec<String> =
        registry::LEVELS.iter().map(|l| format!("L{l} {}", cap["slots_by_level"][l.to_string()])).collect();
    let now_running: Vec<String> = running.iter().filter(|(_, n)| **n > 0).map(|(l, n)| format!("L{l} {n}")).collect();
    let ram = &cap["ram"];
    let gb = |v: &Value| v.as_f64().map(|n| format!("{n}GB")).unwrap_or_else(|| "알 수 없음".to_string());
    let mut text = format!(
        "새로 띄울 수 있는 세션 {}개({}), 세션 안 권장 워커 {}명\n진행 중 {}({}), 메모리 {} 중 사용자 몫 {}를 뺀 예산 {}, 압박 {}, 한도 기준 {}",
        cap["slots"],
        by_level.join(" · "),
        cap["recommended_workers"],
        cap["running"],
        if now_running.is_empty() { "없음".to_string() } else { now_running.join(" · ") },
        gb(&ram["available_gb"]),
        gb(&ram["reserve_gb"]),
        gb(&ram["budget_gb"]),
        ram["pressure"].as_str().unwrap_or("unknown"),
        cap["limits"]["basis"].as_str().unwrap_or("unknown"),
    );
    if let Some(warning) = ram["warning"].as_str() {
        text.push_str(&format!("\n경고: {warning}"));
    }
    text
}

fn tick(json_mode: bool, auto_launch: bool, no_collect: bool) -> Result<u8> {
    let cfg = config::load()?;
    let mut report =
        json!({"collect": null, "scout": false, "needs_gate": [], "launched": [], "waiting": [], "errors": []});
    if !no_collect {
        let result = collect::collect(&cfg, "all", cfg.collect_pages)?;
        report["collect"] = serde_json::to_value(&result)?;
        // The scheduled run reads this to decide whether ContestScout searches the web this time.
        let hour = chrono::Timelike::hour(&chrono::Local::now()) as u8;
        report["scout"] = json!(schedule::scout_at(&cfg.schedule.scout_hours, hour)?);
    }
    let needs_gate: Vec<String> = registry::read(|reg| {
        Ok(reg.contests.values().filter(|e| e.state == "discovered").map(|e| e.id.clone()).collect())
    })?;
    report["needs_gate"] = json!(needs_gate);
    if auto_launch || cfg.auto_launch {
        // One launch pass at a time (shared with `harness launch`), so the capacity measured here
        // holds until every launch below is stored. The registry lock is taken only to read and to
        // store each result; route (gh/git push) and launch run on copies without it.
        let _guard = registry::launch_guard()?;
        let (mut ready, running) = registry::read(|reg| {
            let ready: Vec<Entry> =
                reg.contests.values().filter(|e| e.state == "gate_pass" || e.state == "routed").cloned().collect();
            Ok((ready, reg.running_by_level()))
        })?;
        ready.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.deadline.as_deref().unwrap_or("9999").cmp(b.deadline.as_deref().unwrap_or("9999")))
        });
        let cap = budget::capacity(&cfg, &running);
        let mut left = cap["slots"].as_u64().unwrap_or(0);
        // No memory reading (null budget) means no memory limit; the count limits still apply.
        let mut ram_left = cap["ram"]["budget_gb"].as_f64().unwrap_or(f64::INFINITY);
        let mut left_by_level: std::collections::BTreeMap<u8, u64> =
            registry::LEVELS.iter().map(|l| (*l, cap["slots_by_level"][l.to_string()].as_u64().unwrap_or(0))).collect();
        report["capacity"] = cap;
        let submitted = ledger::load_at(&ledger::path())?;
        let (mut launched, mut waiting, mut errors, mut warnings) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for base in ready {
            let id = base.id.clone();
            let level = base.level.unwrap_or(0);
            let cost = cfg.ram_gb_by_level.get(&level).copied().unwrap_or(f64::INFINITY);
            if left == 0 || left_by_level.get(&level).copied().unwrap_or(0) == 0 || cost > ram_left {
                waiting.push(id);
                continue;
            }
            // A finished route is stored even when the launch after it fails: the next tick then
            // only launches.
            let attempt = if base.state == "gate_pass" {
                route_stored(&base, &cfg, &submitted, None, true, true).map(|(routed, summary)| {
                    let found = summary["warnings"].as_array().map(Vec::as_slice).unwrap_or_default();
                    warnings.extend(found.iter().filter_map(Value::as_str).map(|w| format!("{id}: {w}")));
                    routed
                })
            } else {
                Ok(base)
            }
            .and_then(|routed| launch_stored(&routed, &cfg, None));
            match attempt {
                Ok(mut result) => {
                    result["id"] = json!(id);
                    launched.push(result);
                    left -= 1;
                    *left_by_level.entry(level).or_default() -= 1;
                    ram_left -= cost;
                }
                Err(err) => errors.push(format!("{id}: {err}")),
            }
        }
        report["launched"] = json!(launched);
        report["waiting"] = json!(waiting);
        report["errors"] = json!(errors);
        report["warnings"] = json!(warnings);
    }
    registry::read(board::write)?;
    let ids = |key: &str| -> Vec<String> {
        report[key]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    };
    let launched_ids: Vec<String> = report["launched"]
        .as_array()
        .map(|a| a.iter().filter_map(|r| r["id"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let gate_ids = ids("needs_gate");
    let errors = ids("errors");
    let mut lines = Vec::new();
    if !errors.is_empty() {
        let attempted = errors.len() + launched_ids.len();
        lines.push(format!("오류 {}건(실행 시도 {attempted}건 중 실패):", errors.len()));
        lines.extend(errors.iter().map(|e| format!("오류: {e}")));
    }
    lines.extend([
        format!(
            "수집: {}",
            if report["collect"].is_null() { "-".to_string() } else { report["collect"]["seen"].to_string() }
        ),
        format!("웹 검색(ContestScout) 이번 회차: {}", if report["scout"] == true { "실행" } else { "안 함" }),
        format!(
            "게이트 대기 {}건: {}",
            gate_ids.len(),
            gate_ids.iter().take(20).cloned().collect::<Vec<_>>().join(", ")
        ),
        format!("실행 {}건: {}", launched_ids.len(), launched_ids.join(", ")),
        format!("상한으로 실행 대기 {}건: {}", ids("waiting").len(), ids("waiting").join(", ")),
    ]);
    lines.extend(ids("warnings").iter().map(|w| format!("경고: {w}")));
    // Exit 0 even when launches failed: the report (needs_gate, waiting) is still valid for the
    // scheduled run, which reads `errors` from the JSON.
    emit(json_mode, &report, Some(lines.join("\n")))?;
    Ok(0)
}

/// Route `base` (a copy of the stored entry) without holding the registry lock, then store the
/// result, unless another command changed the entry meanwhile. A failed route stores nothing.
fn route_stored(
    base: &Entry,
    cfg: &config::Config,
    submitted: &ledger::Ledger,
    folder: Option<&str>,
    repo: bool,
    workspace: bool,
) -> Result<(Entry, Value)> {
    let mut work = base.clone();
    let summary = route::route(&mut work, cfg, submitted, folder, repo, workspace)?;
    let stored = registry::write(|reg| {
        let current = reg.get_mut(&base.id)?;
        registry::merge_work(current, base, work)?;
        Ok(current.clone())
    })?;
    Ok((stored, summary))
}

/// Launch `base` on a copy without the registry lock and store the result. When the launch fails
/// but left a session record behind (a workspace it created), that record alone is stored so a
/// retry reuses it instead of creating another.
fn launch_stored(base: &Entry, cfg: &config::Config, preset: Option<&str>) -> Result<Value> {
    let mut work = base.clone();
    match launch::launch(&mut work, cfg, preset, false) {
        Ok(result) => {
            registry::write(|reg| registry::merge_work(reg.get_mut(&base.id)?, base, work))
                .map_err(|err| Error(format!("{err} (세션은 이미 떴다: {})", shown(&result["handle"]))))?;
            Ok(result)
        }
        Err(err) => {
            if work.session != base.session {
                let session = work.session;
                let kept = registry::write(|reg| {
                    let current = reg.get_mut(&base.id)?;
                    if current.state == base.state && current.session == base.session {
                        current.session = session;
                    }
                    Ok(())
                });
                if let Err(save) = kept {
                    return Err(Error(format!("{err} (세션 기록 저장도 실패: {save})")));
                }
            }
            Err(err)
        }
    }
}

fn warn_profile(path: &std::path::Path, text: &str) {
    for warning in profile::warnings(path, text) {
        eprintln!("경고: {warning}");
    }
}
