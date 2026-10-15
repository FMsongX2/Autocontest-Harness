//! The participant profile: a Markdown file the user writes by hand (`data/profile.md`, never
//! committed). The harness only reads it. ContestGate gets the eligibility section and nothing else;
//! the personal details section is read only in an approved submission step.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::paths;

/// Headings of the section ContestGate may read (no names, phone numbers or e-mail addresses).
pub const ELIGIBILITY: &[&str] = &["참가 자격", "Eligibility"];
/// Headings of the section read only after the user approved a submission.
pub const PERSONAL: &[&str] = &["제출용 개인정보", "Submission details"];

pub fn path() -> PathBuf {
    paths::profile()
}

/// The whole profile at `path`; a missing file is an error that says how to start one.
pub fn read_at(path: &Path) -> Result<String> {
    if !path.exists() {
        return Err(Error(format!("프로필이 없다: {} (data/profile.example.md를 복사해 직접 쓴다)", path.display())));
    }
    Ok(fs::read_to_string(path)?)
}

/// The body of the first `## ` section whose heading contains one of `titles`, trimmed.
pub fn section(text: &str, titles: &[&str]) -> Option<String> {
    let mut body: Option<Vec<&str>> = None;
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if body.is_some() {
                break;
            }
            if titles.iter().any(|t| heading.contains(t)) {
                body = Some(Vec::new());
            }
        } else if let Some(lines) = body.as_mut() {
            lines.push(line);
        }
    }
    body.map(|lines| lines.join("\n").trim().to_string())
}

/// The eligibility section of `text`, or an error naming the expected heading.
pub fn eligibility(text: &str) -> Result<String> {
    section(text, ELIGIBILITY).filter(|s| !s.is_empty()).ok_or_else(|| {
        Error(format!("프로필에 `## {}` 절이 없거나 비었다(data/profile.example.md 참고)", ELIGIBILITY[0]))
    })
}
