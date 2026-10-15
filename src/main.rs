//! `harness`: the Autocontest-Harness CLI. Agents call it; it is the only writer of the registry.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};

use harness::error::{Error, Result};
use harness::ledger;
use harness::registry::{self, Entry, NewEntry, STATES};
use harness::{
    board, budget, clean, collect, config, dedupe, guard, install, launch, profile, route, runners, schedule, sources,
    text,
};

const EXIT_DUPLICATE: u8 = 3;
const EXIT_PERSONA: u8 = 4;

#[derive(Parser)]
#[command(name = "harness", version, about = "Contest automation harness (docs/CONTRACT.md)")]
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
        #[arg(long)]
        pages: Option<u32>,
    },
    /// Add one contest by hand or from ContestScout.
    Add {
        #[arg(long)]
        title: String,
        #[arg(long)]
        url: String,
        #[arg(long)]
        org: Option<String>,
        #[arg(long)]
        deadline: Option<String>,
        #[arg(long, default_value = "manual", value_parser = ["scout", "manual"])]
        source: String,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        force: bool,
    },
    /// Find registry entries and contest folders that look like the same contest (exit 3 if any).
    DedupeCheck {
        title: String,
        #[arg(long)]
        url: Option<String>,
        /// Leave this entry out and take its URL, organizer and deadline.
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
        #[arg(long)]
        note: Option<String>,
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
    /// Record a ContestJudge score (with --stage and --file) or show the score history.
    Score {
        id: String,
        #[arg(long, value_parser = registry::SCORE_STAGES.to_vec(), requires = "file")]
        stage: Option<String>,
        /// ContestJudge output JSON (`.harness/judge/<stage>-<n>.json`).
        #[arg(long, requires = "stage")]
        file: Option<PathBuf>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Record a submission.
    Submit {
        id: String,
        #[arg(long)]
        method: String,
        #[arg(long)]
        receipt: String,
        #[arg(long, num_args = 0..)]
        files: Vec<String>,
        /// Approval evidence and other notes.
        #[arg(long)]
        note: Option<String>,
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
    /// Install agents, the skill and the extension for runners (default: the configured runner).
    Install {
        #[arg(long = "runner")]
        runners: Vec<String>,
        /// Every runner whose CLI is installed.
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
    /// Run the scheduled automation once with the configured runner (for cron/launchd).
    Autorun,
    /// Manage the launchd job that runs `harness autorun` periodically.
    Schedule {
        #[command(subcommand)]
        action: ScheduleAction,
    },
}

#[derive(Subcommand)]
enum ScheduleAction {
    Install {
        #[arg(long, default_value_t = 6)]
        every_hours: u32,
    },
    Remove,
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
    /// The whole profile, personal details included (only in an approved submission step).
    Show,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("오류: {err}");
            ExitCode::from(1)
        }
    }
}

fn emit(json_mode: bool, payload: &impl Serialize, text: Option<String>) -> Result<()> {
    match text {
        Some(text) if !json_mode => println!("{text}"),
        _ => println!("{}", serde_json::to_string_pretty(payload)?),
    }
    Ok(())
}

fn line(e: &Entry) -> String {
    let level = e.level.map(|l| format!("L{l}")).unwrap_or_else(|| "-".into());
    let today = chrono::Utc::now().with_timezone(&text::kst()).date_naive();
    let dday = board::dday(e.deadline.as_deref(), today);
    let title: String = e.title.chars().take(60).collect();
    format!("{:<28} {:<17} {:<7} {:<8} {:<3} {}", e.id, e.state, dday, e.kind.as_deref().unwrap_or("-"), level, title)
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
            let new_lines: Vec<String> = report.new.iter().map(line).collect();
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
        Command::Add { title, url, org, deadline, source, note, force } => {
            let cfg = config::load()?;
            let submitted = ledger::load_at(&ledger::path())?;
            registry::write(|reg| {
                let query = dedupe::Query {
                    title,
                    url,
                    include_folders: true,
                    exclude_id: None,
                    strict: false,
                    organizer: "",
                    deadline: None,
                    submitted: Some(&submitted),
                };
                let matches = dedupe::find_matches(reg, &cfg, &query);
                if !matches.is_empty() && !force {
                    let listing = serde_json::to_string_pretty(&matches)?;
                    emit(
                        json_mode,
                        &json!({"duplicate": true, "matches": matches}),
                        Some(format!("같은 공모전으로 보이는 항목이 있어 추가하지 않았다(--force로 강제):\n{listing}")),
                    )?;
                    return Ok(EXIT_DUPLICATE);
                }
                let mut entry_id = format!("{source}-{}", slug(url, title));
                if reg.contests.contains_key(&entry_id) {
                    entry_id = format!("{entry_id}-{}", text::now()[11..16].replace(':', ""));
                }
                let entry = registry::new_entry(NewEntry {
                    id: &entry_id,
                    title,
                    url,
                    source,
                    organizer: org.as_deref().unwrap_or(""),
                    deadline: deadline.clone(),
                    categories: Vec::new(),
                    note: note.clone(),
                });
                let text = format!("추가: {}", line(&entry));
                emit(json_mode, &entry, Some(text))?;
                reg.contests.insert(entry_id, entry);
                Ok(0)
            })
        }
        Command::DedupeCheck { title, url, id, org, deadline } => {
            let cfg = config::load()?;
            let submitted = ledger::load_at(&ledger::path())?;
            let matches = registry::read(|reg| {
                let own = id.as_deref().and_then(|i| reg.contests.get(i));
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
            let text = if rows.is_empty() {
                "(없음)".to_string()
            } else {
                rows.iter().map(line).collect::<Vec<_>>().join("\n")
            };
            emit(json_mode, &rows, Some(text))?;
            Ok(0)
        }
        Command::Show { id } => {
            let entry = registry::read(|reg| Ok(reg.get(id)?.clone()))?;
            emit(json_mode, &entry, None)?;
            Ok(0)
        }
        Command::Gate { id, file } => {
            let gate: Value = serde_json::from_str(&std::fs::read_to_string(file)?)?;
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
            let summary = registry::write(|reg| {
                route::route(reg.get_mut(id)?, &cfg, &submitted, folder.as_deref(), !no_repo, !no_workspace)
            })?;
            let text = format!(
                "라우팅: {} (저장소 {}, 작업 공간 {})",
                summary["folder"].as_str().unwrap_or("-"),
                summary["repo"].as_str().unwrap_or("None"),
                summary["workspace_id"].as_str().unwrap_or("None"),
            );
            emit(json_mode, &summary, Some(text))?;
            Ok(0)
        }
        Command::Launch { id, preset, dry_run, force } => {
            let cfg = config::load()?;
            let (level, state, running) = registry::read(|reg| {
                let entry = reg.get(id)?;
                Ok((entry.level, entry.state.clone(), reg.running_by_level()))
            })?;
            if state == "routed" && !dry_run && !force {
                let cap = budget::capacity(&cfg, &running);
                let free = level.and_then(|l| cap["slots_by_level"][l.to_string()].as_u64()).unwrap_or(0);
                if free == 0 {
                    return Err(Error(format!(
                        "{id}: 지금은 레벨 {} 공모전을 더 띄우지 않는다(진행 중 {running:?}, 레벨별 상한 {:?}, 메모리 예산 {}GB·압박 {}, 전체 여유 {}). 넘기려면 --force",
                        level.map(|l| l.to_string()).unwrap_or_else(|| "None".into()),
                        cfg.max_running_by_level,
                        cap["ram"]["budget_gb"],
                        cap["ram"]["pressure"].as_str().unwrap_or("unknown"),
                        cap["slots"],
                    )));
                }
            }
            let result = if *dry_run {
                registry::read(|reg| launch::launch(&mut reg.get(id)?.clone(), &cfg, preset.as_deref(), true))?
            } else {
                registry::write(|reg| launch::launch(reg.get_mut(id)?, &cfg, preset.as_deref(), false))?
            };
            let text = format!(
                "실행 명령: {}\n세션: {}",
                result["command"].as_str().unwrap_or(""),
                result["handle"].as_str().unwrap_or("None")
            );
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
        Command::Transition { id, state, note, by, persona_ok, score_ok } => {
            let mut note = note.clone().unwrap_or_default();
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
                registry::transition(entry, state, by, &note)?;
                Ok(entry.clone())
            })?;
            emit(json_mode, &entry, Some(format!("상태 변경: {}", line(&entry))))?;
            Ok(0)
        }
        Command::Submit { id, method, receipt, files, note, by } => {
            let entry = registry::write(|reg| {
                let entry = reg.get_mut(id)?;
                let joined: Vec<&str> =
                    [Some(receipt.as_str()), note.as_deref()].into_iter().flatten().filter(|s| !s.is_empty()).collect();
                registry::transition(entry, "submitted", by, &joined.join(" / "))?;
                entry
                    .submissions
                    .push(json!({"at": text::now(), "method": method, "receipt": receipt, "files": files}));
                Ok(entry.clone())
            })?;
            let item = ledger::from_submit(&entry, method, receipt, files, note.as_deref().unwrap_or(""));
            ledger::write_at(&ledger::path(), |l| Ok(l.upsert(item)))?;
            emit(json_mode, &entry, Some(format!("제출 기록: {}", line(&entry))))?;
            Ok(0)
        }
        Command::Score { id, stage, file, note } => {
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
            let raw = std::fs::read_to_string(file).map_err(|err| Error(format!("{}: {err}", file.display())))?;
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
            let ledger_path = ledger::path();
            let outcomes = registry::write(|reg| {
                let ids: Vec<String> = match id {
                    Some(id) => vec![reg.get(id)?.id.clone()],
                    None => reg.contests.values().filter(|e| clean::candidate(e)).map(|e| e.id.clone()).collect(),
                };
                ids.iter().map(|id| clean::clean(reg.get_mut(id)?, &ledger_path, &opts)).collect::<Result<Vec<_>>>()
            })?;
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
                runners::NAMES
                    .iter()
                    .filter(|n| runners::get(n).is_ok_and(|r| r.available()))
                    .map(|n| n.to_string())
                    .collect()
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
        Command::Schedule { action } => {
            let result = match action {
                ScheduleAction::Install { every_hours } => schedule::install(*every_hours)?,
                ScheduleAction::Remove => schedule::remove()?,
                ScheduleAction::Status => schedule::status()?,
            };
            emit(json_mode, &result, None)?;
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
            let item = ledger::manual(ledger::Manual {
                title,
                url: url.as_deref().unwrap_or(""),
                organizer: org.as_deref().unwrap_or(""),
                date: date.as_deref(),
                method: method.as_deref().unwrap_or(""),
                receipt: receipt.as_deref().unwrap_or(""),
                folder: folder.as_deref(),
                note: note.as_deref().unwrap_or(""),
            })?;
            let added = ledger::write_at(&path, |l| Ok(l.upsert(item.clone())))?;
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
            let eligibility = profile::eligibility(&profile::read_at(&path)?)?;
            emit(json_mode, &json!({"path": path, "eligibility": eligibility}), Some(eligibility.clone()))?;
        }
        ProfileAction::Show => {
            let text = profile::read_at(&path)?;
            emit(json_mode, &json!({"path": path, "text": text}), Some(text.clone()))?;
        }
    }
    Ok(0)
}

fn slug(url: &str, title: &str) -> String {
    let non_alnum = Regex::new(r"[^0-9a-z]+").expect("valid regex");
    let key: String = non_alnum
        .replace_all(&dedupe::normalize_url(url).to_lowercase(), "-")
        .trim_matches('-')
        .chars()
        .take(48)
        .collect();
    if !key.is_empty() {
        return key;
    }
    let non_word = Regex::new(r"\W+").expect("valid regex");
    non_word.replace_all(title, "-").chars().take(30).collect()
}

fn capacity_text(cap: &Value, running: &std::collections::BTreeMap<u8, usize>) -> String {
    let by_level: Vec<String> =
        registry::LEVELS.iter().map(|l| format!("L{l} {}", cap["slots_by_level"][l.to_string()])).collect();
    let now_running: Vec<String> = running.iter().filter(|(_, n)| **n > 0).map(|(l, n)| format!("L{l} {n}")).collect();
    let ram = &cap["ram"];
    format!(
        "새로 띄울 수 있는 세션 {}개({}), 세션 안 권장 워커 {}명\n진행 중 {}({}), 메모리 {}GB 중 사용자 몫 {}GB를 뺀 예산 {}GB, 압박 {}, 한도 기준 {}",
        cap["slots"],
        by_level.join(" · "),
        cap["recommended_workers"],
        cap["running"],
        if now_running.is_empty() { "없음".to_string() } else { now_running.join(" · ") },
        ram["available_gb"],
        ram["reserve_gb"],
        ram["budget_gb"],
        ram["pressure"].as_str().unwrap_or("unknown"),
        cap["limits"]["basis"].as_str().unwrap_or("unknown"),
    )
}

fn tick(json_mode: bool, auto_launch: bool, no_collect: bool) -> Result<u8> {
    let cfg = config::load()?;
    let mut report = json!({"collect": null, "needs_gate": [], "launched": [], "waiting": [], "errors": []});
    if !no_collect {
        let result = collect::collect(&cfg, "all", cfg.collect_pages)?;
        report["collect"] = serde_json::to_value(&result)?;
    }
    let (needs_gate, running) = registry::read(|reg| {
        let ids: Vec<String> =
            reg.contests.values().filter(|e| e.state == "discovered").map(|e| e.id.clone()).collect();
        Ok((ids, reg.running_by_level()))
    })?;
    report["needs_gate"] = json!(needs_gate);
    if auto_launch || cfg.auto_launch {
        let cap = budget::capacity(&cfg, &running);
        let mut left = cap["slots"].as_u64().unwrap_or(0);
        let mut ram_left = cap["ram"]["budget_gb"].as_f64().unwrap_or(0.0);
        let mut left_by_level: std::collections::BTreeMap<u8, u64> =
            registry::LEVELS.iter().map(|l| (*l, cap["slots_by_level"][l.to_string()].as_u64().unwrap_or(0))).collect();
        report["capacity"] = cap;
        let submitted = ledger::load_at(&ledger::path())?;
        let (launched, waiting, errors) = registry::write(|reg| {
            let mut ready: Vec<String> = reg
                .contests
                .values()
                .filter(|e| e.state == "gate_pass" || e.state == "routed")
                .map(|e| e.id.clone())
                .collect();
            ready.sort_by(|a, b| {
                let (ea, eb) = (&reg.contests[a], &reg.contests[b]);
                eb.priority.cmp(&ea.priority).then_with(|| {
                    ea.deadline.as_deref().unwrap_or("9999").cmp(eb.deadline.as_deref().unwrap_or("9999"))
                })
            });
            let (mut launched, mut waiting, mut errors) = (Vec::new(), Vec::new(), Vec::new());
            for id in ready {
                let entry = reg.get_mut(&id)?;
                let level = entry.level.unwrap_or(0);
                let cost = cfg.ram_gb_by_level.get(&level).copied().unwrap_or(f64::INFINITY);
                if left == 0 || left_by_level.get(&level).copied().unwrap_or(0) == 0 || cost > ram_left {
                    waiting.push(id);
                    continue;
                }
                let attempt = (|| {
                    if entry.state == "gate_pass" {
                        route::route(entry, &cfg, &submitted, None, true, true)?;
                    }
                    launch::launch(entry, &cfg, None, false)
                })();
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
            Ok((launched, waiting, errors))
        })?;
        report["launched"] = json!(launched);
        report["waiting"] = json!(waiting);
        report["errors"] = json!(errors);
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
    let mut lines = vec![
        format!(
            "수집: {}",
            if report["collect"].is_null() { "None".to_string() } else { report["collect"]["seen"].to_string() }
        ),
        format!(
            "게이트 대기 {}건: {}",
            gate_ids.len(),
            gate_ids.iter().take(20).cloned().collect::<Vec<_>>().join(", ")
        ),
        format!("실행 {}건: {}", launched_ids.len(), launched_ids.join(", ")),
        format!("상한으로 실행 대기 {}건: {}", ids("waiting").len(), ids("waiting").join(", ")),
    ];
    lines.extend(ids("errors").iter().map(|e| format!("오류: {e}")));
    emit(json_mode, &report, Some(lines.join("\n")))?;
    Ok(0)
}
