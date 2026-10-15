//! Agent harness backends: how to start a session (interactive or one-shot) and how to install the
//! harness agents, skill and extension for each CLI. Contract: `docs/CONTRACT.md`.

use std::path::{Path, PathBuf};

use crate::error::Result;

pub mod claude;
pub mod codex;
pub mod dsh;
pub mod omp;
pub mod pi;

/// Everything a runner needs to build a command line.
pub struct Spec {
    /// Working directory of the session (a contest folder or the repository root).
    pub cwd: PathBuf,
    /// The launch instructions (`.harness/launch-prompt.md` or `harness/prompts/automation.md`).
    pub prompt_file: PathBuf,
    /// The first user message that tells the agent to follow the instructions.
    pub first_message: String,
    /// Extra system prompt: the persona + main role, for the main interface session only.
    pub system_append: Option<PathBuf>,
    /// Model id/alias for runners that take one (`model_by_level`).
    pub model: Option<String>,
    /// omp only: per-run `--config` overlay built from a `modelPresets` entry.
    pub preset_overlay: Option<PathBuf>,
    /// `runner_args.<name>` from the config, appended as is.
    pub extra_args: Vec<String>,
}

pub trait Runner: Sync {
    fn name(&self) -> &'static str;
    /// Executable looked up on PATH.
    fn binary(&self) -> &'static str;
    fn available(&self) -> bool {
        which(self.binary()).is_some()
    }
    /// Whether the harness agents can run as subagents (otherwise steps run in one session).
    fn subagents(&self) -> bool;
    /// A command for an interactive session that starts by reading `spec.prompt_file`.
    fn interactive(&self, spec: &Spec) -> Result<Vec<String>>;
    /// A command that runs the instructions once without a TTY and exits.
    fn oneshot(&self, spec: &Spec) -> Result<Vec<String>>;
    /// Install agents, the skill and (where supported) the extension package from `root`.
    /// Returns one human-readable line per action.
    fn install(&self, root: &Path) -> Result<Vec<String>>;
}

pub const NAMES: &[&str] = &["omp", "pi", "claude", "codex", "dsh"];

pub fn get(name: &str) -> Result<&'static dyn Runner> {
    Ok(match name {
        "omp" => &omp::Omp,
        "pi" => &pi::Pi,
        "claude" => &claude::Claude,
        "codex" => &codex::Codex,
        "dsh" => &dsh::Dsh,
        _ => crate::bail!("알 수 없는 러너: {name} (가능: {})", NAMES.join(", ")),
    })
}

/// Full path of `binary` on PATH.
pub fn which(binary: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?.to_str()?.split(':').map(|dir| Path::new(dir).join(binary)).find(|p| p.is_file())
}

/// Read a prompt file for runners that take the instructions as message text.
pub fn prompt_text(spec: &Spec) -> Result<String> {
    Ok(std::fs::read_to_string(&spec.prompt_file)?)
}

/// Text of the system-prompt file for runners that take it inline (codex, dsh).
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
        let meta: serde_json::Value = serde_yaml_ng::from_str(head)?;
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

/// A TOML basic string (also what `codex -c key=value` parses).
pub fn toml_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Run a runner CLI for install and fail with its output.
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
