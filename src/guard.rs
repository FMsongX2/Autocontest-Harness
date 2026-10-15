//! Persona guard. The persona talks to the user; it must never reach contest deliverables (code,
//! comments, reports, slides, forms) or commit messages. `ach check` runs this scan and the
//! move to `awaiting_approval` refuses while it finds anything.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde::Serialize;

use crate::config::Config;
use crate::error::Result;

/// Not deliverables: tooling, caches, harness metadata and raw datasets (which may hold real names).
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".harness",
    "node_modules",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    "dist",
    "build",
    "data",
    "dataset",
    "datasets",
    "raw",
    "cache",
    ".cache",
];
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_TEXT_CHARS: usize = 120;

#[derive(Debug, PartialEq, Serialize)]
pub struct Finding {
    /// Path relative to the contest folder, or `commit <short hash>` for commit messages.
    pub file: String,
    pub line: usize,
    pub term: String,
    pub text: String,
}

/// The configured guard terms plus how the persona addresses the user.
pub fn terms(config: &Config) -> Vec<String> {
    let mut terms: Vec<String> = config.persona.guard_terms.iter().filter(|t| !t.trim().is_empty()).cloned().collect();
    let call = config.persona.call_user.trim();
    if !call.is_empty() && !terms.iter().any(|t| t == call) {
        terms.push(call.to_string());
    }
    terms
}

/// Scan every text file under `folder` and the git commit messages.
pub fn scan(folder: &Path, terms: &[String]) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    if terms.is_empty() || !folder.is_dir() {
        return Ok(findings);
    }
    walk(folder, folder, terms, &mut findings)?;
    scan_commits(folder, terms, &mut findings)?;
    Ok(findings)
}

fn walk(root: &Path, dir: &Path, terms: &[String], findings: &mut Vec<Finding>) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if !SKIP_DIRS.contains(&entry.file_name().to_string_lossy().as_ref()) {
                walk(root, &path, terms, findings)?;
            }
        } else if kind.is_file() && entry.metadata()?.len() <= MAX_FILE_BYTES {
            let bytes = fs::read(&path)?;
            if bytes.iter().take(8192).any(|&b| b == 0) {
                continue; // binary
            }
            let Ok(text) = String::from_utf8(bytes) else { continue };
            let file = path.strip_prefix(root).unwrap_or(&path).display().to_string();
            scan_text(&file, &text, terms, findings);
        }
    }
    Ok(())
}

fn scan_text(file: &str, text: &str, terms: &[String], findings: &mut Vec<Finding>) {
    for (index, line) in text.lines().enumerate() {
        for term in terms {
            if contains_word(line, term) {
                findings.push(Finding {
                    file: file.to_string(),
                    line: index + 1,
                    term: term.clone(),
                    text: line.trim().chars().take(MAX_TEXT_CHARS).collect(),
                });
            }
        }
    }
}

fn is_syllable(ch: char) -> bool {
    ('가'..='힣').contains(&ch)
}

/// Korean words run together, so a syllable term (`나무`) only counts at the start of a word:
/// "나무가", "(나무)" match, "소나무" does not. Jamo terms (`ㅎㅎ`) match anywhere.
fn contains_word(line: &str, term: &str) -> bool {
    if !term.chars().next().is_some_and(is_syllable) {
        return line.contains(term);
    }
    line.match_indices(term).any(|(at, _)| !line[..at].chars().next_back().is_some_and(is_syllable))
}

fn scan_commits(folder: &Path, terms: &[String], findings: &mut Vec<Finding>) -> Result<()> {
    if !folder.join(".git").exists() {
        return Ok(());
    }
    let output = Command::new("git").arg("-C").arg(folder).args(["log", "--format=%h%x00%B%x1e"]).output()?;
    if !output.status.success() {
        return Ok(()); // no commits yet
    }
    for record in String::from_utf8_lossy(&output.stdout).split('\x1e') {
        if let Some((hash, message)) = record.trim_start_matches('\n').split_once('\0') {
            scan_text(&format!("commit {hash}"), message, terms, findings);
        }
    }
    Ok(())
}

/// One line per finding, for error messages and text output.
pub fn describe(findings: &[Finding], limit: usize) -> String {
    let mut lines: Vec<String> =
        findings.iter().take(limit).map(|f| format!("{}:{} [{}] {}", f.file, f.line, f.term, f.text)).collect();
    if findings.len() > limit {
        lines.push(format!("… 외 {}건", findings.len() - limit));
    }
    lines.join("\n")
}
