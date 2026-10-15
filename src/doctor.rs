//! `harness doctor`: whether this checkout is ready to run, one check per thing that commonly goes
//! wrong on a new machine, each with the fix. Read-only: it never changes files or settings.

use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::config::{self, Config};
use crate::process::{self, LOCAL};
use crate::runners::{self, which};
use crate::{budget, ledger, paths, profile, registry, schedule, workspaces};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub level: Level,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

fn ok(name: &'static str, detail: impl Into<String>) -> Check {
    Check { name, level: Level::Ok, detail: detail.into(), fix: None }
}

fn warn(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Check {
    Check { name, level: Level::Warn, detail: detail.into(), fix: Some(fix.into()) }
}

fn fail(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Check {
    Check { name, level: Level::Fail, detail: detail.into(), fix: Some(fix.into()) }
}

/// Trimmed stdout of a successful command, `None` when it is missing, fails or times out.
fn stdout_of(program: &str, args: &[&str], cwd: Option<&Path>) -> Option<String> {
    let out = process::output(program, args, cwd, LOCAL).ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Every check, in the order a new user would fix them. Stops after the first check that makes
/// the rest meaningless (no checkout, unreadable config).
pub fn run() -> Vec<Check> {
    let mut out = Vec::new();
    let root = paths::root();
    if !paths::checkout_ok(&root) {
        out.push(fail(
            "checkout",
            format!("{}에 {}가 없다", root.display(), paths::MARKER),
            "HARNESS_HOME을 클론 경로로 지정하거나 클론에서 scripts/install.sh를 다시 실행한다",
        ));
        return out;
    }
    out.push(ok("checkout", root.display().to_string()));
    out.push(version(&root));
    out.push(platform());

    let config_file = paths::config_file();
    let config = match config::load() {
        Ok(config) => config,
        Err(err) => {
            out.push(fail("config", err.0, "data/config.example.json과 README_ko.md의 설정 표를 보고 고친다"));
            return out;
        }
    };
    out.push(if config_file.exists() {
        ok("config", config_file.display().to_string())
    } else {
        warn(
            "config",
            "data/config.json이 없어 기본값을 쓴다",
            "cp data/config.example.json data/config.json 후 contests_root·github_org·workspace를 고친다",
        )
    });
    out.extend(data_files());
    out.extend(git_and_github(&config));
    out.extend(runner_and_workspace(&config, &root));
    out.push(agent_models(&config, &root));
    out.push(contests_root(&config));
    out.extend(user_profile());
    out.push(memory());
    out.extend(scheduled_run(&config));
    out
}

/// Paths that only document the harness. Prompts, agents, the skill and `INIT.md` tell agents which
/// commands to run, so a change there still means the binary may not match the checkout.
const DOCS_ONLY: [&str; 7] = [
    ":(exclude)README*.md",
    ":(exclude)CONTRIBUTING*.md",
    ":(exclude)SECURITY*.md",
    ":(exclude)CHANGELOG*.md",
    ":(exclude)docs",
    ":(exclude)references",
    ":(exclude)LICENSE",
];

fn version(root: &Path) -> Check {
    let built = env!("HARNESS_GIT_SHA");
    let release = env!("CARGO_PKG_VERSION");
    match stdout_of("git", &["rev-parse", "--short=12", "HEAD"], Some(root)) {
        Some(head) if head == built => ok("version", format!("{release} ({built})")),
        Some(head) if docs_only_since(root, built) => {
            ok("version", format!("{release} ({built}), 클론 {head}까지 바뀐 것은 문서뿐"))
        }
        Some(head) => warn(
            "version",
            format!(
                "바이너리는 {built}에서 빌드됐고 클론은 {head}다. 지시문·에이전트는 클론 것을 쓰므로 바이너리와 어긋날 수 있다"
            ),
            "scripts/install.sh(또는 cargo install --path . --root ~/.local --locked --force)로 바이너리를 다시 설치한다",
        ),
        None => ok("version", format!("{release} ({built}), 클론이 git 저장소가 아니라 비교하지 않음")),
    }
}

/// Whether the checkout differs from commit `built` in documentation only. False when `built` is not
/// in the clone's history (a shallow clone, or a binary built elsewhere).
pub fn docs_only_since(root: &Path, built: &str) -> bool {
    let mut args = vec!["diff", "--quiet", built, "HEAD", "--", "."];
    args.extend(DOCS_ONLY);
    process::output("git", &args, Some(root), LOCAL).is_ok_and(|out| out.status.success())
}

fn platform() -> Check {
    match std::env::consts::OS {
        "macos" => ok("os", "macOS"),
        "linux" => ok("os", "Linux (launchd 스케줄러 없음: orca 또는 cron)"),
        other => fail("os", format!("{other}는 지원하지 않는다"), "macOS나 Linux에서 돌린다"),
    }
}

fn data_files() -> Vec<Check> {
    let mut out = Vec::new();
    match registry::read(|reg| Ok(reg.contests.len())) {
        Ok(count) => out.push(ok("registry", format!("공모전 {count}건"))),
        Err(err) => out.push(fail("registry", err.0, "data/registry.json을 백업에서 되살리거나 고친다")),
    }
    match ledger::load_at(&ledger::path()) {
        Ok(ledger) => out.push(ok("ledger", format!("제출 기록 {}건", ledger.submissions.len()))),
        Err(err) => out.push(fail("ledger", err.0, "data/submissions.json을 백업에서 되살리거나 고친다")),
    }
    let data = paths::data();
    if let Ok(meta) = std::fs::metadata(&data) {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            out.push(warn(
                "data",
                format!("{}의 권한이 {mode:o}라 다른 사용자가 읽을 수 있다", data.display()),
                format!("chmod 700 {}", data.display()),
            ));
        }
    }
    out
}

fn git_and_github(config: &Config) -> Vec<Check> {
    let mut out = Vec::new();
    if which("git").is_none() {
        out.push(fail("git", "git이 PATH에 없다", "git을 설치한다"));
        return out;
    }
    // Exactly what `git commit` needs (config, env or system fallback), as route's preflight checks.
    match stdout_of("git", &["var", "GIT_AUTHOR_IDENT"], None).filter(|e| !e.is_empty()) {
        Some(ident) => {
            let who = ident.rsplitn(3, ' ').nth(2).unwrap_or(&ident).to_string();
            out.push(ok("git", format!("커밋 작성자 {who}")))
        }
        None => out.push(fail(
            "git",
            "git 작성자 정보가 없어 route가 첫 커밋을 만들지 못한다",
            "git config --global user.name '<이름>' && git config --global user.email '<메일>'",
        )),
    }
    if which("gh").is_none() {
        out.push(warn("gh", "gh가 PATH에 없다. route는 --no-repo로만 된다", "GitHub CLI를 설치하고 gh auth login"));
    } else if stdout_of("gh", &["auth", "status"], None).is_none() {
        out.push(warn("gh", "gh에 로그인돼 있지 않다", "gh auth login"));
    } else {
        let owner =
            if config.github_org.is_empty() { "로그인한 계정".to_string() } else { config.github_org.clone() };
        out.push(ok("gh", format!("로그인됨, 저장소는 {owner}에 만든다")));
    }
    out
}

fn runner_and_workspace(config: &Config, root: &Path) -> Vec<Check> {
    let mut out = Vec::new();
    match runners::get(&config.runner) {
        Err(err) => out.push(fail("runner", err.0, "data/config.json의 runner를 고친다")),
        Ok(runner) if !runner.available() => out.push(fail(
            "runner",
            format!("러너 {}의 실행 파일({})이 PATH에 없다", runner.name(), runner.binary()),
            format!("{}를 설치하거나 PATH에 넣는다", runner.binary()),
        )),
        Ok(runner) => match runner.installed(root) {
            Some(false) => out.push(warn(
                "runner",
                format!("{}에 이 클론의 에이전트·스킬·확장이 설치돼 있지 않다", runner.name()),
                "harness install",
            )),
            _ => out.push(ok("runner", runner.name())),
        },
    }
    match workspaces::get(&config.workspace) {
        Err(err) => out.push(fail("workspace", err.0, "data/config.json의 workspace를 고친다")),
        Ok(ws) if !ws.available() => out.push(fail(
            "workspace",
            format!("작업 환경 {}의 실행 파일이 PATH에 없다", ws.name()),
            "그 도구를 설치하거나 workspace를 tmux·headless로 둔다",
        )),
        Ok(ws) if ws.name() == "orca" && orca_knows(&config.orca_repo_path()) == Some(false) => {
            let path = config.orca_repo_path();
            out.push(fail(
                "workspace",
                format!("orca_repo_path {}가 Orca 프로젝트가 아니다", path.display()),
                format!("orca repo add --path {} (또는 orca_repo_path를 고친다)", path.display()),
            ))
        }
        Ok(ws) => out.push(ok("workspace", ws.name())),
    }
    out
}

/// Whether Orca has a project (folder or git) at `path`; `None` when Orca cannot be asked.
fn orca_knows(path: &Path) -> Option<bool> {
    let repos = workspaces::orca::orca_json(&["repo", "list"]).ok()?;
    let wanted = std::fs::canonicalize(path).ok()?;
    let list = repos.get("repos").unwrap_or(&repos).as_array()?.clone();
    Some(
        list.iter()
            .any(|r| r["path"].as_str().and_then(|p| std::fs::canonicalize(p).ok()).is_some_and(|p| p == wanted)),
    )
}

/// `agent_models` names real agents, and the runner applies what the config says.
fn agent_models(config: &Config, root: &Path) -> Check {
    match crate::install::unknown_agents(config, root) {
        Err(err) => return fail("agent_models", err.0, "agents/*.md를 확인한다"),
        Ok(unknown) if !unknown.is_empty() => {
            return fail(
                "agent_models",
                format!("없는 에이전트: {}", unknown.join(", ")),
                "harness agents로 이름을 확인하고 harness config unset agent_models.<이름>",
            );
        }
        Ok(_) => {}
    }
    if config.agent_models.is_empty() {
        return ok("agent_models", "에이전트마다 기본 역할 모델을 쓴다");
    }
    let Some(applied) = runners::get(&config.runner).ok().and_then(|r| r.agent_models()) else {
        return ok(
            "agent_models",
            format!("{}개 설정(적용 여부는 러너가 알려 주지 않는다)", config.agent_models.len()),
        );
    };
    let stale: Vec<&str> = config
        .agent_models
        .iter()
        .filter(|(name, model)| applied.get(*name).map(String::as_str) != model.as_deref().map(str::trim))
        .map(|(name, _)| name.as_str())
        .collect();
    if stale.is_empty() {
        ok("agent_models", format!("{}개 적용됨", config.agent_models.len()))
    } else {
        warn("agent_models", format!("설정과 러너가 다르다: {}", stale.join(", ")), "harness install")
    }
}

fn contests_root(config: &Config) -> Check {
    let dir = config.contests_root();
    if !dir.is_dir() {
        return warn(
            "contests_root",
            format!("{}가 없다", dir.display()),
            format!("mkdir -p {} (또는 contests_root를 고친다)", dir.display()),
        );
    }
    let probe = dir.join(".harness-doctor-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            ok("contests_root", dir.display().to_string())
        }
        Err(err) => fail("contests_root", format!("{}에 쓸 수 없다: {err}", dir.display()), "폴더 권한을 고친다"),
    }
}

fn user_profile() -> Vec<Check> {
    let path = profile::path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return vec![warn(
            "profile",
            format!("{}가 없다. 게이트가 참가 자격을 판단하지 못한다", path.display()),
            "cp data/profile.example.md data/profile.md && chmod 600 data/profile.md 후 직접 채운다",
        )];
    };
    let problems = profile::warnings(&path, &text);
    if problems.is_empty() {
        vec![ok("profile", path.display().to_string())]
    } else {
        problems.into_iter().map(|p| warn("profile", p, "data/profile.md를 고친다")).collect()
    }
}

fn memory() -> Check {
    match budget::free_ram_gb() {
        Some(gb) => ok("memory", format!("쓸 수 있는 메모리 {gb:.1}GB")),
        None => warn(
            "memory",
            "쓸 수 있는 메모리를 읽지 못해 메모리 예산 없이 개수·토큰 상한만 적용한다",
            "macOS의 vm_stat이나 Linux의 /proc/meminfo가 읽히는지 확인한다",
        ),
    }
}

/// The key of a scheduler in `schedule status` (`cron` reports under `crontab`).
fn status_key(scheduler: &str) -> &str {
    if scheduler == "cron" { "crontab" } else { scheduler }
}

/// The configured scheduler has the run and matches the config; no other scheduler also has one.
fn scheduled_run(config: &Config) -> Vec<Check> {
    let status = match schedule::status(config) {
        Ok(status) => status,
        Err(err) => return vec![warn("schedule", err.0, "harness schedule status로 자세히 본다")],
    };
    let installed = |name: &str| -> bool {
        let part = &status[name];
        match part.get("automations").and_then(Value::as_array) {
            Some(list) => !list.is_empty(),
            None => part["installed"].as_bool().unwrap_or(false),
        }
    };
    let in_sync = |name: &str| -> bool {
        let part = &status[name];
        match part.get("automations").and_then(Value::as_array) {
            Some(list) => list.iter().any(|a| a["in_sync"] == true),
            None => part["in_sync"] == true,
        }
    };
    let wanted = config.schedule.scheduler.as_str();
    let mut out = Vec::new();
    if !installed(status_key(wanted)) {
        out.push(warn(
            "schedule",
            format!("{wanted}에 정기 실행이 없다. 수집·게이트는 손으로 돌릴 때만 된다"),
            "harness schedule install",
        ));
    } else if !in_sync(status_key(wanted)) {
        out.push(warn(
            "schedule",
            format!("{wanted}의 정기 실행이 설정(schedule.cron, runner)과 다르다"),
            "harness schedule install",
        ));
    } else {
        out.push(ok("schedule", format!("{wanted} {}", config.schedule.cron)));
    }
    for other in schedule::SCHEDULERS.iter().filter(|s| **s != wanted) {
        if installed(status_key(other)) {
            out.push(warn(
                "schedule",
                format!("{other}에도 정기 실행이 있어 한 회차가 두 번 돈다"),
                format!("harness schedule remove --scheduler {other}"),
            ));
        }
    }
    out
}

/// Human-readable report: one line per check, the fix indented under it.
pub fn text(checks: &[Check]) -> String {
    let mut lines = Vec::new();
    for c in checks {
        let tag = match c.level {
            Level::Ok => "[ok]  ",
            Level::Warn => "[경고]",
            Level::Fail => "[실패]",
        };
        lines.push(format!("{tag} {}: {}", c.name, c.detail));
        if let Some(fix) = &c.fix {
            lines.push(format!("       -> {fix}"));
        }
    }
    let fails = checks.iter().filter(|c| c.level == Level::Fail).count();
    let warns = checks.iter().filter(|c| c.level == Level::Warn).count();
    lines.push(format!("실패 {fails}건, 경고 {warns}건"));
    lines.join("\n")
}
