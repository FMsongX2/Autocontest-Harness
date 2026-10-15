//! Create the contest folder, its private repository, the workspace, and `CONTEST.md`.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::bail;
use crate::config::Config;
use crate::error::{Error, Result};
use crate::ledger::Ledger;
use crate::registry::{Entry, transition};
use crate::text::now;
use crate::workspaces;

static FOLDER_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]{1,60}$").expect("regex"));

const ELIGIBILITY_LABELS: &[(&str, &str)] = &[
    ("individual_ok", "개인 참가"),
    ("team_ok", "팀 참가"),
    ("student_only", "학생 한정"),
    ("region", "지역 제한"),
    ("age", "나이 제한"),
    ("fee", "참가비"),
    ("offline_required", "오프라인 참석 필수"),
    ("ai_policy", "생성형 AI"),
    ("original_work_oath", "직접 창작·서약"),
];

/// Python `repr` of a JSON value (used for nested values in table cells).
fn py_repr(value: &Value) -> String {
    match value {
        Value::String(s) => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")),
        Value::Array(items) => format!("[{}]", items.iter().map(py_repr).collect::<Vec<_>>().join(", ")),
        Value::Object(map) => {
            format!("{{{}}}", map.iter().map(|(k, v)| format!("'{k}': {}", py_repr(v))).collect::<Vec<_>>().join(", "))
        }
        other => py_str(other),
    }
}

/// Python `str` of a JSON value.
fn py_str(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => py_repr(other),
    }
}

fn cell(value: &Value) -> String {
    let text = match value {
        Value::Null => "-".to_string(),
        Value::String(s) if s.is_empty() => "-".to_string(),
        other => py_str(other),
    };
    text.replace('|', "\\|").replace('\n', " ")
}

fn table(rows: &[Vec<Value>], header: &[&str]) -> Vec<String> {
    let mut lines = vec![format!("| {} |", header.join(" | ")), format!("|{}", "---|".repeat(header.len()))];
    lines.extend(rows.iter().map(|row| format!("| {} |", row.iter().map(cell).collect::<Vec<_>>().join(" | "))));
    lines
}

fn korean(value: &Value) -> Value {
    match value {
        Value::Bool(true) => json!("예"),
        Value::Bool(false) => json!("아니오"),
        other => other.clone(),
    }
}

fn field(map: &Map<String, Value>, key: &str) -> Value {
    map.get(key).cloned().unwrap_or(Value::Null)
}

fn opt(value: Option<impl Into<Value>>) -> Value {
    value.map(Into::into).unwrap_or(Value::Null)
}

fn kind_level(entry: &Entry) -> String {
    let level = entry.level.map(|l| l.to_string()).unwrap_or_else(|| "None".into());
    format!("{} / 레벨 {level}", entry.kind.as_deref().unwrap_or("None"))
}

/// The contest summary document built from the gate record.
pub fn render_contest_md(entry: &Entry) -> String {
    let empty = Map::new();
    let gate = entry.gate.as_ref().and_then(Value::as_object).unwrap_or(&empty);
    let object = |key: &str| gate.get(key).and_then(Value::as_object).cloned().unwrap_or_default();
    let list = |key: &str| gate.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    let (elig, sub) = (object("eligibility"), object("submission"));
    let mut out = vec![format!("# {}", entry.title), String::new()];
    out.extend(table(
        &[
            vec![json!("주최·주관"), json!(entry.organizer)],
            vec![json!("공식 공고"), json!(entry.url)],
            vec![json!("접수 마감"), opt(entry.deadline.clone())],
            vec![json!("유형·레벨"), json!(kind_level(entry))],
            vec![json!("상금"), field(gate, "prize")],
            vec![json!("하네스 id"), json!(entry.id)],
        ],
        &["항목", "내용"],
    ));
    out.extend(["", "<p>표 1. 공모전 개요</p>", "", "## 참가 자격과 제약", ""].map(String::from));
    let label =
        |k: &str| ELIGIBILITY_LABELS.iter().find(|(key, _)| *key == k).map_or(k.to_string(), |(_, l)| l.to_string());
    let elig_rows: Vec<Vec<Value>> = elig.iter().map(|(k, v)| vec![json!(label(k)), korean(v)]).collect();
    out.extend(table(&elig_rows, &["항목", "판정"]));
    out.extend(["", "<p>표 2. 참가 자격과 제약(게이트 판정)</p>", "", "## 제출물", ""].map(String::from));
    let pick = |item: &Value, keys: [&str; 3]| -> Vec<Value> {
        keys.iter().map(|k| item.get(*k).cloned().unwrap_or(Value::Null)).collect()
    };
    let deliverables: Vec<Vec<Value>> =
        list("deliverables").iter().map(|d| pick(d, ["name", "format", "limits"])).collect();
    out.extend(table(&deliverables, &["제출물", "형식", "제한"]));
    out.extend(["", "<p>표 3. 제출물</p>", "", "## 심사기준", ""].map(String::from));
    let criteria: Vec<Vec<Value>> = list("criteria").iter().map(|c| pick(c, ["stage", "name", "weight"])).collect();
    out.extend(table(&criteria, &["단계", "기준", "배점"]));
    out.extend(["", "<p>표 4. 심사기준</p>", "", "## 제출 방법", ""].map(String::from));
    let sub_rows: Vec<Vec<Value>> = sub.iter().map(|(k, v)| vec![json!(k), v.clone()]).collect();
    out.extend(table(&sub_rows, &["항목", "내용"]));
    out.extend(["", "<p>표 5. 제출 방법</p>", "", "## 판정 근거", ""].map(String::from));
    let bullets = |key: &str| -> Vec<String> {
        let items: Vec<String> = list(key).iter().map(|r| format!("- {}", py_str(r))).collect();
        if items.is_empty() { vec!["- (없음)".to_string()] } else { items }
    };
    out.extend(bullets("reasons"));
    out.extend(["", "## 사용자에게 확인할 것", ""].map(String::from));
    out.extend(bullets("questions"));
    let checked = gate.get("checked_at").map_or("-".to_string(), py_str);
    out.push(String::new());
    out.push(format!("게이트 판정 시각: {checked}. 원문이 바뀌었을 수 있으므로 제출 전 공식 공고를 다시 확인한다."));
    out.push(String::new());
    out.join("\n")
}

pub fn render_readme(entry: &Entry) -> String {
    [
        format!("# {}", entry.title),
        String::new(),
        format!("- 상태: 하네스 라우팅 완료 ({}), 하네스 id `{}`", now(), entry.id),
        format!("- 공식 공고: {}", entry.url),
        format!("- 마감: {}", entry.deadline.as_deref().filter(|d| !d.is_empty()).unwrap_or("미확인")),
        format!("- 유형·레벨: {}", kind_level(entry)),
        "- 공고 정리: [CONTEST.md](./CONTEST.md)".to_string(),
        String::new(),
        "## 진행 메모".to_string(),
        String::new(),
    ]
    .join("\n")
}

fn run(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<Output> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    command.output().map_err(|err| Error(format!("{program} 실행 실패: {err}")))
}

fn run_checked(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<Output> {
    let out = run(program, args, cwd)?;
    if !out.status.success() {
        let detail =
            String::from_utf8_lossy(if out.stderr.is_empty() { &out.stdout } else { &out.stderr }).trim().to_string();
        bail!("{program} {} 실패: {detail}", args.join(" "));
    }
    Ok(out)
}

fn folder_name(entry: &Entry, explicit: Option<&str>) -> Result<String> {
    let from_gate = entry.gate.as_ref().and_then(|g| g.get("folder")).and_then(Value::as_str).filter(|f| !f.is_empty());
    let name = explicit.filter(|f| !f.is_empty()).or(from_gate).unwrap_or(&entry.id).to_string();
    if !FOLDER_RE.is_match(&name) {
        bail!("폴더 이름은 영문·숫자·하이픈만 쓴다: '{name}'");
    }
    Ok(name)
}

/// Add `- <folder>: <title>` to the shared list that dedupe reads, once.
fn append_existing(config: &Config, folder: &str, title: &str) -> Result<()> {
    let line = format!("- {folder}: {title}");
    let path = config.existing_contests();
    if path.exists() && fs::read_to_string(&path)?.contains(&line) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file = fs::OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

fn pretty(value: &impl serde::Serialize) -> Result<String> {
    Ok(serde_json::to_string_pretty(value)? + "\n")
}

/// Create the folder, repository and workspace, then move the entry to `routed`.
/// Contests already in the submission ledger are refused. Returns `{folder, repo, workspace_id}`.
pub fn route(
    entry: &mut Entry,
    config: &Config,
    submitted: &Ledger,
    folder: Option<&str>,
    repo: bool,
    workspace: bool,
) -> Result<Value> {
    if entry.state != "gate_pass" {
        bail!("{}: gate_pass 상태만 라우팅한다(지금 {})", entry.id, entry.state);
    }
    let urls: Vec<&str> =
        std::iter::once(entry.url.as_str()).chain(entry.alt_urls.iter().map(String::as_str)).collect();
    if let Some((done, _)) = submitted.matches(&entry.title, &urls, None).first() {
        bail!("이미 제출한 공모전이다: {} ({})", done.title, done.submitted_at);
    }
    let name = folder_name(entry, folder)?;
    let target = config.contests_root().join(&name);
    let meta = target.join(".harness");
    if target.exists() && fs::read_dir(&target)?.next().is_some() {
        let existing = meta.join("contest.json");
        let same = existing.exists() && {
            let previous: Value = serde_json::from_str(&fs::read_to_string(&existing)?)?;
            previous.get("id").and_then(Value::as_str) == Some(entry.id.as_str())
        };
        if !same {
            bail!("이미 다른 내용이 있는 폴더: {}", target.display());
        }
    }
    fs::create_dir_all(&meta)?;
    let target_str = target.to_string_lossy().into_owned();
    entry.folder = Some(target_str.clone());
    fs::write(target.join("CONTEST.md"), render_contest_md(entry))?;
    if !target.join("README.md").exists() {
        fs::write(target.join("README.md"), render_readme(entry))?;
    }
    if !target.join(".gitignore").exists() {
        fs::write(target.join(".gitignore"), ".DS_Store\n.venv/\n__pycache__/\n.harness/omp-preset.yml\n")?;
    }
    fs::write(meta.join("gate.json"), pretty(&entry.gate)?)?;
    let mut summary = json!({"folder": target_str, "repo": null, "workspace_id": null});

    if !target.join(".git").exists() {
        run_checked("git", &["init", "-q", "-b", "main"], Some(&target))?;
    }
    run_checked("git", &["add", "-A"], Some(&target))?;
    if !run("git", &["diff", "--cached", "--quiet"], Some(&target))?.status.success() {
        let message = format!("Route contest: {}", entry.title);
        run_checked("git", &["commit", "-q", "-m", &message], Some(&target))?;
    }
    if repo {
        // An empty `github_org` puts the private repository under the signed-in gh account.
        let owner = if config.github_org.trim().is_empty() {
            let login = run_checked("gh", &["api", "user", "--jq", ".login"], None)?;
            String::from_utf8_lossy(&login.stdout).trim().to_string()
        } else {
            config.github_org.trim().to_string()
        };
        if owner.is_empty() {
            bail!("GitHub 계정을 알 수 없다: `gh auth login` 후 다시 시도하거나 설정의 github_org를 채운다");
        }
        let full = format!("{owner}/{name}");
        if !run("gh", &["repo", "view", &full], None)?.status.success() {
            let description: String = entry.title.chars().take(300).collect();
            run_checked(
                "gh",
                &[
                    "repo",
                    "create",
                    &full,
                    "--private",
                    "--source",
                    &target_str,
                    "--remote",
                    "origin",
                    "--push",
                    "--description",
                    &description,
                ],
                None,
            )?;
        } else if String::from_utf8_lossy(&run_checked("git", &["remote"], Some(&target))?.stdout).trim().is_empty() {
            let remote = format!("https://github.com/{full}.git");
            run_checked("git", &["remote", "add", "origin", &remote], Some(&target))?;
            run_checked("git", &["push", "-q", "-u", "origin", "main"], Some(&target))?;
        }
        entry.repo = Some(full.clone());
        summary["repo"] = json!(full);
    }
    if workspace {
        let backend = workspaces::get(&config.workspace)?;
        let previous = entry
            .session
            .as_ref()
            .filter(|s| s.get("workspace").and_then(Value::as_str) == Some(backend.name()))
            .and_then(|s| s.get("workspace_id"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string);
        let workspace_id = match previous {
            Some(id) => Some(id),
            None => backend.create(entry, config)?,
        };
        entry.session = Some(json!({
            "workspace": backend.name(),
            "workspace_id": workspace_id,
            "runner": config.runner,
            "handle": null,
        }));
        summary["workspace_id"] = json!(workspace_id);
    }

    append_existing(config, &name, &entry.title)?;
    transition(entry, "routed", "harness route", &target_str)?;
    fs::write(meta.join("contest.json"), pretty(entry)?)?;
    Ok(summary)
}
