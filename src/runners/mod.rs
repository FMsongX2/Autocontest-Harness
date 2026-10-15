//! Runner adapters: how the harness drives one agent harness (omp, or another CLI added by a
//! pull request). An adapter builds the command lines for an interactive and a one-shot session,
//! says what the harness supports, and installs the agents, skill and extension for it.
//! Contract: `docs/CONTRACT.md` 10절. How to add one: `CONTRIBUTING.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::Result;

pub mod claude;
pub mod omp;

/// Every adapter compiled in. A new adapter is one module plus one line here; the contract test in
/// `tests/backends.rs` then runs against it.
pub static ALL: &[&dyn Runner] = &[&omp::Omp, &claude::Claude];

/// Everything an adapter needs to build a command line.
pub struct Spec {
    /// Working directory of the session (a contest folder or the repository root).
    pub cwd: PathBuf,
    /// The instructions (`.harness/launch-prompt.md`, `harness/prompts/automation.md`, or the main
    /// session's start file). The agent must read all of it before anything else.
    pub prompt_file: PathBuf,
    /// The same instructions already in memory, when the caller has them (`harness launch`, whose
    /// dry run writes no file). Read them through [`prompt_text`], never `prompt_file` directly.
    pub prompt: Option<String>,
    /// The first user message that tells the agent to follow the instructions. An adapter that
    /// passes the instructions inline as the first message may leave it out.
    pub first_message: String,
    /// Extra system prompt: the persona + main role, for the main interface session only. Contest
    /// sessions and scheduled runs always get `None`.
    pub system_append: Option<PathBuf>,
    /// Model id or alias (`model_by_level`); `None` keeps the harness default.
    pub model: Option<String>,
    /// The file [`Runner::preset_file`] wrote for this run (adapters with presets only).
    pub preset_file: Option<PathBuf>,
    /// `runner_args.<name>` from the config, passed through unchanged and in order.
    pub extra_args: Vec<String>,
}

/// One agent harness. Implementations are stateless unit structs registered in [`ALL`].
pub trait Runner: Sync {
    /// The config value (`runner`) that selects this adapter.
    fn name(&self) -> &'static str;
    /// Executable looked up on PATH; every command line starts with it.
    fn binary(&self) -> &'static str;
    fn available(&self) -> bool {
        which(self.binary()).is_some()
    }
    /// Whether the harness agents can run as subagents. Without them, `harness launch` tells the
    /// session to run subagent steps one by one itself.
    fn subagents(&self) -> bool;
    /// Whether the harness has named model presets (`preset_by_level`, `harness launch --preset`).
    fn presets(&self) -> bool {
        false
    }
    /// Write the per-run file for preset `name` under `dir` (the contest's `.harness/`, named
    /// `<runner>-preset.<ext>`) and return its path; it reaches the command as `Spec::preset_file`.
    fn preset_file(&self, name: &str, _dir: &Path) -> Result<PathBuf> {
        crate::bail!("러너 {}는 프리셋을 지원하지 않는다(프리셋 {name}). model_by_level을 쓴다", self.name())
    }
    /// The agent id Orca uses for this harness (`orca automations create --provider`), when Orca
    /// can start it; needed for `schedule.scheduler = "orca"`.
    fn orca_agent(&self) -> Option<&'static str> {
        None
    }
    /// A command for an interactive session (a TTY) that starts by reading `spec.prompt_file`.
    fn interactive(&self, spec: &Spec) -> Result<Vec<String>>;
    /// A command that runs the instructions once without a TTY and exits.
    fn oneshot(&self, spec: &Spec) -> Result<Vec<String>>;
    /// Install the agents (`agents/*.md`, converted if the harness uses another format), the skill
    /// (`skills/contest-harness`) and, where supported, the extension package from `root`. Must be
    /// idempotent. Returns one human-readable line per action.
    fn install(&self, root: &Path) -> Result<Vec<String>>;
    /// Whether `install` has been done for the checkout at `root` (`harness doctor`). `None` when
    /// the adapter cannot tell.
    fn installed(&self, _root: &Path) -> Option<bool> {
        None
    }
    /// Apply the per-agent model choices of `agent_models` (agent name -> selector; `None` removes
    /// the choice the harness made before) where the harness reads them. Called by `harness
    /// install`. Returns one human-readable line per action. Adapters without per-agent models
    /// refuse any `Some`.
    fn apply_agent_models(&self, models: &BTreeMap<String, Option<String>>) -> Result<Vec<String>> {
        if let Some(name) = models.iter().find_map(|(name, model)| model.as_ref().map(|_| name)) {
            crate::bail!(
                "러너 {}는 에이전트별 모델 지정을 지원하지 않는다({name}). agent_models를 비운다",
                self.name()
            );
        }
        Ok(Vec::new())
    }
    /// The per-agent models the harness currently uses (`harness agents`, `harness doctor`);
    /// `None` when the adapter cannot tell.
    fn agent_models(&self) -> Option<BTreeMap<String, String>> {
        None
    }
}

pub fn names() -> Vec<&'static str> {
    ALL.iter().map(|r| r.name()).collect()
}

pub fn get(name: &str) -> Result<&'static dyn Runner> {
    match ALL.iter().find(|r| r.name() == name) {
        Some(runner) => Ok(*runner),
        None => crate::bail!(
            "알 수 없는 러너: {name} (가능: {}). 다른 하네스는 어댑터를 추가한다(CONTRIBUTING.md)",
            names().join(", ")
        ),
    }
}

// ---- Helpers for adapters ----

/// Full path of `binary` on PATH.
pub fn which(binary: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?.to_str()?.split(':').map(|dir| Path::new(dir).join(binary)).find(|p| p.is_file())
}

/// The instructions as text, for harnesses that take them as the first message.
pub fn prompt_text(spec: &Spec) -> Result<String> {
    match &spec.prompt {
        Some(text) => Ok(text.clone()),
        None => std::fs::read_to_string(&spec.prompt_file)
            .map_err(|e| crate::error::Error(format!("{}: {e}", spec.prompt_file.display()))),
    }
}

/// The system-prompt addition as text, for harnesses that take it inline.
pub fn system_text(spec: &Spec) -> Result<Option<String>> {
    match &spec.system_append {
        Some(path) => Ok(Some(
            std::fs::read_to_string(path).map_err(|e| crate::error::Error(format!("{}: {e}", path.display())))?,
        )),
        None => Ok(None),
    }
}

/// Point `dst` at `src` (replacing an old symlink, never a real file). Returns one action line.
pub fn link(src: &Path, dst: &Path) -> Result<String> {
    if !src.exists() {
        crate::bail!("설치할 원본이 없다: {}", src.display());
    }
    if let Ok(meta) = std::fs::symlink_metadata(dst) {
        if !meta.file_type().is_symlink() {
            crate::bail!("심볼릭 링크가 아닌 파일이 이미 있다(직접 확인 필요): {}", dst.display());
        }
        if std::fs::canonicalize(dst).ok() == std::fs::canonicalize(src).ok() {
            return Ok(format!("ok {}", dst.display()));
        }
        std::fs::remove_file(dst)?;
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::os::unix::fs::symlink(src, dst)?;
    Ok(format!("linked {} -> {}", dst.display(), src.display()))
}

/// Write `content` to `path` unless it is already there. Returns one action line.
pub fn write_file(path: &Path, content: &str) -> Result<String> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == content) {
        return Ok(format!("ok {}", path.display()));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(format!("wrote {}", path.display()))
}

/// One omp task agent (`agents/*.md`): frontmatter and body.
pub struct AgentDoc {
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    pub skills: Vec<String>,
    /// The agent's default model: selectors or `@role` aliases, tried in order.
    pub model: Vec<String>,
    /// The omp `output` schema (JSON Type Definition), if any.
    pub output: Option<serde_json::Value>,
    pub body: String,
}

impl AgentDoc {
    pub fn parse(text: &str) -> Result<AgentDoc> {
        let Some(rest) = text.strip_prefix("---\n") else {
            crate::bail!("에이전트 파일에 frontmatter가 없다")
        };
        let (head, body) = match rest.find("\n---\n") {
            Some(end) => (&rest[..end], &rest[end + 5..]),
            None => match rest.strip_suffix("\n---") {
                Some(head) => (head, ""),
                None => crate::bail!("에이전트 frontmatter가 닫히지 않았다"),
            },
        };
        let meta: serde_json::Value = serde_saphyr::from_str(head)?;
        let text_of = |key: &str| meta.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let list_of = |key: &str| -> Vec<String> {
            match meta.get(key) {
                Some(serde_json::Value::Array(items)) => {
                    items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()
                }
                Some(serde_json::Value::String(s)) => {
                    s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
                }
                _ => Vec::new(),
            }
        };
        let name = text_of("name");
        if name.is_empty() {
            crate::bail!("에이전트 frontmatter에 name이 없다");
        }
        Ok(AgentDoc {
            name,
            description: text_of("description"),
            tools: list_of("tools"),
            skills: list_of("autoloadSkills"),
            model: list_of("model"),
            output: meta.get("output").filter(|v| !v.is_null()).cloned(),
            body: body.trim().to_string(),
        })
    }

    /// The body plus, when the agent has an output schema, a closing section that asks for it.
    pub fn instructions(&self) -> String {
        let mut out = self.body.clone();
        if let Some(schema) = &self.output {
            let schema = serde_json::to_string_pretty(schema).unwrap_or_default();
            out.push_str("\n\n## 마지막 답변 형식\n\n마지막 답변은 아래 JSON Type Definition(RFC 8927) 스키마에 맞는 JSON 객체 하나로만 낸다.\n\n```json\n");
            out.push_str(&schema);
            out.push_str("\n```");
        }
        out.push('\n');
        out
    }
}

/// Every `agents/*.md` under `root`, sorted by file name.
pub fn agent_docs(root: &Path) -> Result<Vec<AgentDoc>> {
    let dir = root.join("agents");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| crate::error::Error(format!("{}: {e}", dir.display())))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|p| {
            AgentDoc::parse(&std::fs::read_to_string(p)?)
                .map_err(|e| crate::error::Error(format!("{}: {e}", p.display())))
        })
        .collect()
}

/// Run a harness CLI during install and fail with its output.
pub fn run_cli(program: &str, args: &[&str]) -> Result<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| crate::error::Error(format!("{program} 실행 실패: {e}")))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        crate::bail!("{program} {} 실패: {}", args.join(" "), text.trim());
    }
    Ok(text.trim().to_string())
}
