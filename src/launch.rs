//! Start contest sessions and the main interface session on the configured runner and workspace.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::paths;
use crate::registry::{Entry, transition};
use crate::runners::{self, Runner, Spec};
use crate::workspaces::{self, Open, shell_command};

pub const FIRST_MESSAGE: &str = "첨부한 하네스 실행 지시를 읽고 그대로 시작해.";

pub fn playbook(kind: &str) -> &'static str {
    match kind {
        "video" | "image" | "music" | "design" => "creative-media",
        "idea" => "idea-proposal",
        "paper" => "paper",
        "model" => "model-competition",
        "research" => "research",
        "data" | "report" => "data-analysis",
        "software" => "software-hackathon",
        _ => "essay-short-doc",
    }
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| Error(format!("{}: {e}", path.display())))
}

/// Python `str.format` with named fields: `{name}` from `values`, `{{`/`}}` as literal braces.
pub fn format_named(template: &str, values: &[(&str, String)]) -> Result<String> {
    let mut out = String::with_capacity(template.len() * 2);
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
        } else if tail.starts_with('}') {
            crate::bail!("지시문 틀에 짝 없는 '}}'가 있다");
        } else {
            let Some(end) = tail.find('}') else { crate::bail!("지시문 틀에 닫히지 않은 '{{'가 있다") };
            let key = &tail[1..end];
            match values.iter().find(|(k, _)| *k == key) {
                Some((_, value)) => out.push_str(value),
                None => crate::bail!("지시문 틀의 알 수 없는 자리표시자: {{{key}}}"),
            }
            rest = &tail[end + 1..];
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// What a runner without subagents is told about steps written for subagents.
fn no_subagents_note(runner: &str) -> String {
    format!(
        "\n\n## 이 러너({runner})의 서브에이전트\n\n이 러너에는 서브에이전트 기능이 없다. 서브에이전트 기능이 없으면 같은 세션에서 차례로 수행한다. 위에서 서브에이전트(task·agent)나 병렬 웨이브로 하라는 단계는 이 세션에서 하나씩 직접 수행한다.\n"
    )
}

/// The launch instructions for `entry` (`launch.md` + `level_<n>.md`), plus a note for runners
/// without subagents.
pub fn render_prompt(entry: &Entry, runner: &dyn Runner) -> Result<String> {
    let Some(level) = entry.level else { crate::bail!("{}: 레벨이 없다", entry.id) };
    let section = read(&paths::prompts().join(format!("level_{level}.md")))?.trim().to_string();
    let template = read(&paths::prompts().join("launch.md"))?;
    let kind = entry.kind.clone();
    let values = [
        ("title", entry.title.clone()),
        ("folder", entry.folder.clone().unwrap_or_else(|| "None".into())),
        ("id", entry.id.clone()),
        ("type", kind.clone().unwrap_or_else(|| "None".into())),
        ("level", level.to_string()),
        ("deadline", entry.deadline.clone().filter(|d| !d.is_empty()).unwrap_or_else(|| "미확인".into())),
        ("url", entry.url.clone()),
        ("repo", entry.repo.clone().filter(|r| !r.is_empty()).unwrap_or_else(|| "없음".into())),
        ("playbook", playbook(kind.as_deref().unwrap_or("other")).to_string()),
        ("level_section", section),
    ];
    let mut prompt = format_named(&template, &values)?;
    if !runner.subagents() {
        prompt.push_str(&no_subagents_note(runner.name()));
    }
    Ok(prompt)
}

/// The preset for this launch: `--preset`, else `preset_by_level` for runners with presets.
/// `--preset` on a runner without presets is an error; `preset_by_level` is then ignored.
pub fn pick_preset(runner: &dyn Runner, config: &Config, level: u8, preset: Option<&str>) -> Result<Option<String>> {
    if !runner.presets() {
        if let Some(p) = preset {
            crate::bail!("러너 {}는 프리셋을 지원하지 않는다(--preset {p}). model_by_level을 쓴다", runner.name());
        }
        return Ok(None);
    }
    Ok(match preset {
        Some(p) => Some(p.to_string()),
        None => config.preset_by_level.get(&level).cloned().flatten(),
    }
    .filter(|p| !p.is_empty()))
}

fn session_field<'a>(entry: &'a Entry, key: &str) -> Option<&'a str> {
    entry.session.as_ref()?.get(key)?.as_str()
}

/// Start the contest session. With `dry_run` nothing is written or started: the prompt is rendered in
/// memory and returned as `prompt_text`, `prompt` is the path a real launch writes, and a missing
/// runner or workspace binary is reported in `warnings` instead of failing.
pub fn launch(entry: &mut Entry, config: &Config, preset: Option<&str>, dry_run: bool) -> Result<Value> {
    if entry.state != "routed" {
        crate::bail!("{}: routed 상태만 실행한다(지금 {})", entry.id, entry.state);
    }
    let Some(folder) = entry.folder.clone().filter(|f| !f.is_empty()) else {
        crate::bail!("{}: 폴더가 없다. 먼저 harness route를 실행한다", entry.id)
    };
    let Some(level) = entry.level.filter(|l| config.ram_gb_by_level.contains_key(l)) else {
        crate::bail!("{}: 레벨이 없다. 게이트(harness gate)로 레벨을 먼저 기록한다", entry.id)
    };
    let runner = runners::get(&config.runner)?;
    let workspace = workspaces::get(&config.workspace)?;
    if workspace.name() == "terminal" {
        crate::bail!(
            "terminal 작업 환경은 공모전 실행에 쓰지 않는다. data/config.json의 workspace를 orca·tmux·headless 중 하나로 둔다"
        );
    }
    let mut problems = Vec::new();
    if !runner.available() {
        problems.push(format!("러너 {}의 실행 파일({})이 PATH에 없다", runner.name(), runner.binary()));
    }
    if !workspace.available() {
        problems.push(format!("작업 환경 {}의 실행 파일이 PATH에 없다", workspace.name()));
    }
    if !dry_run && let Some(first) = problems.first() {
        crate::bail!("{first}");
    }

    let folder = PathBuf::from(folder);
    let meta = folder.join(".harness");
    let prompt_path = meta.join("launch-prompt.md");
    let prompt_text = render_prompt(entry, runner)?;
    let preset = pick_preset(runner, config, level, preset)?;
    let preset_file = match preset.as_deref() {
        None => None,
        Some(name) if dry_run => Some(dry_preset_path(runner, name, &meta)?),
        Some(name) => {
            std::fs::create_dir_all(&meta)?;
            Some(runner.preset_file(name, &meta)?)
        }
    };
    if !dry_run {
        std::fs::create_dir_all(&meta)?;
        std::fs::write(&prompt_path, &prompt_text)?;
    }
    let spec = Spec {
        cwd: folder.clone(),
        prompt_file: prompt_path.clone(),
        first_message: FIRST_MESSAGE.to_string(),
        // Contest sessions never get the persona: their code, reports and commits must not carry it.
        system_append: None,
        // A preset already picks the models.
        model: if preset_file.is_some() {
            None
        } else {
            config.model_by_level.get(&level).filter(|m| !m.is_empty()).cloned()
        },
        preset_file,
        extra_args: config.runner_args(runner.name()),
    };
    let argv = if workspace.interactive() { runner.interactive(&spec)? } else { runner.oneshot(&spec)? };
    let argv = with_session_env(&entry.id, argv);
    let existing_id = (session_field(entry, "workspace") == Some(workspace.name()))
        .then(|| session_field(entry, "workspace_id").map(str::to_string))
        .flatten();
    let mut result = json!({
        "command": shell_command(&folder, &argv),
        "prompt": prompt_path.display().to_string(),
        "preset": preset,
        "runner": runner.name(),
        "workspace": workspace.name(),
        "workspace_id": existing_id,
        "handle": null,
    });
    if dry_run {
        result["prompt_text"] = json!(prompt_text);
        result["warnings"] = json!(problems);
        return Ok(result);
    }
    let workspace_id = match existing_id {
        Some(id) => Some(id),
        None => workspace.create(entry, config)?,
    };
    // Record the workspace before opening: when `open` fails the caller still stores the session,
    // so the retry reuses this workspace instead of creating another one.
    entry.session = Some(json!({
        "workspace": workspace.name(),
        "workspace_id": workspace_id,
        "runner": runner.name(),
        "handle": null,
    }));
    let opened = workspace.open(&Open { cwd: folder, title: &entry.id, argv, workspace_id: workspace_id.as_deref() });
    let handle = match (opened, workspace_id.as_deref()) {
        (Ok(handle), _) => handle,
        (Err(err), Some(id)) => crate::bail!("{err} (작업 공간 {id}은(는) 남겨 두었다; 다시 launch하면 그대로 쓴다)"),
        (Err(err), None) => return Err(err),
    };
    entry.session = Some(json!({
        "workspace": workspace.name(),
        "workspace_id": workspace_id,
        "runner": runner.name(),
        "handle": handle,
    }));
    result["workspace_id"] = json!(workspace_id);
    result["handle"] = json!(handle);
    let note = json!({"runner": runner.name(), "workspace": workspace.name(), "preset": preset, "handle": handle});
    transition(entry, "running", "harness launch", &note.to_string())?;
    Ok(result)
}

/// Environment variable naming the contest a session was launched for. `harness approve` refuses to
/// run inside such a session and `profile show --for <id>` requires it to match.
pub const SESSION_ENV: &str = "HARNESS_SESSION";

/// `env HARNESS_SESSION=<id> <argv>`: works for terminal shell lines and direct spawns alike.
/// Only contest sessions get it; the main session (`chat`) and `autorun` do not.
pub fn with_session_env(id: &str, argv: Vec<String>) -> Vec<String> {
    let mut wrapped = vec!["env".to_string(), format!("{SESSION_ENV}={id}")];
    wrapped.extend(argv);
    wrapped
}

/// The path a real launch would give the preset file, found by writing it to a scratch directory
/// (so an unknown preset still fails the dry run) and removing that again.
fn dry_preset_path(runner: &dyn Runner, name: &str, meta: &Path) -> Result<PathBuf> {
    let scratch = std::env::temp_dir().join(format!("harness-dry-run-{}", std::process::id()));
    std::fs::create_dir_all(&scratch)?;
    let written = runner.preset_file(name, &scratch);
    let _ = std::fs::remove_dir_all(&scratch);
    let written = written?;
    Ok(written.file_name().map_or_else(|| written.clone(), |file| meta.join(file)))
}

/// The tail of a running contest session.
pub fn peek(entry: &Entry, lines: usize) -> Result<String> {
    let (Some(name), Some(handle)) = (session_field(entry, "workspace"), session_field(entry, "handle")) else {
        crate::bail!("{}: 띄운 세션이 없다(harness launch 전이거나 세션 기록이 없다)", entry.id)
    };
    workspaces::get(name)?.tail(handle, lines)
}

/// Persona (when enabled) + main role as one system-prompt file, `data/main-system.md`.
/// Only the main interface session gets it. Missing source files are errors.
pub fn main_system_prompt(config: &Config) -> Result<PathBuf> {
    let mut parts = Vec::new();
    if config.persona.enabled {
        let persona = paths::resolve(&config.persona.file);
        if !persona.exists() {
            crate::bail!(
                "페르소나 파일이 없다: {} (persona.file). data/persona.example.md를 복사해 고치거나 persona.enabled를 false로 둔다",
                persona.display()
            );
        }
        let mut text = read(&persona)?;
        let call_user = config.persona.call_user.trim();
        if !call_user.is_empty() {
            text = text.replace("{{call_user}}", call_user);
        }
        parts.push(text.trim().to_string());
    }
    let main = paths::prompts().join("main.md");
    if !main.exists() {
        crate::bail!("메인 인터페이스 지시문이 없다: {}", main.display());
    }
    parts.push(read(&main)?.trim().to_string());
    let target = paths::data().join("main-system.md");
    std::fs::create_dir_all(paths::data())?;
    std::fs::write(&target, parts.join("\n\n") + "\n")?;
    Ok(target)
}

/// The first turn of the main session (`data/main-start.md`).
fn main_start(config: &Config) -> Result<PathBuf> {
    let user = Some(config.persona.call_user.trim()).filter(|u| !u.is_empty()).unwrap_or("사용자");
    let text = format!(
        "메인 인터페이스 세션을 시작한다. `harness board --json`과 `harness capacity --json`으로 진행 중·게이트 대기·승인 대기·마감 임박 공모전과 남은 용량을 짧게 요약하고, {user}에게 물을 것(게이트 질문, 제출 승인, 레벨 5 계획 검수)이 있으면 한 번에 묶어 물은 뒤 지시를 기다린다.\n"
    );
    let target = paths::data().join("main-start.md");
    std::fs::create_dir_all(paths::data())?;
    std::fs::write(&target, text)?;
    Ok(target)
}

/// Open the main interface session at the repository root (default workspace: this terminal).
pub fn chat(config: &Config, runner: Option<&str>, workspace: Option<&str>) -> Result<Value> {
    let runner = runners::get(runner.unwrap_or(&config.runner))?;
    let workspace = workspaces::get(workspace.unwrap_or("terminal"))?;
    if !runner.available() {
        crate::bail!("러너 {}의 실행 파일({})이 PATH에 없다", runner.name(), runner.binary());
    }
    if !workspace.available() {
        crate::bail!("작업 환경 {}의 실행 파일이 PATH에 없다", workspace.name());
    }
    if !workspace.interactive() {
        crate::bail!("{} 작업 환경은 대화형이 아니라 메인 세션을 열 수 없다", workspace.name());
    }
    let root = paths::root();
    let spec = Spec {
        cwd: root.clone(),
        prompt_file: main_start(config)?,
        first_message: "첨부한 지시대로 메인 인터페이스를 시작해.".to_string(),
        system_append: Some(main_system_prompt(config)?),
        model: None,
        preset_file: None,
        extra_args: config.runner_args(runner.name()),
    };
    let argv = runner.interactive(&spec)?;
    let command = shell_command(&root, &argv);
    let handle = workspace.open(&Open { cwd: root, title: "contest-harness-main", argv, workspace_id: None })?;
    Ok(json!({
        "command": command,
        "runner": runner.name(),
        "workspace": workspace.name(),
        "workspace_id": null,
        "handle": handle,
    }))
}
