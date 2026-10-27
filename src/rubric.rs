//! `RUBRIC.md` section "## 0. 채점 근거": the organizer, host, purpose and judging criteria of the
//! contest, each with its notice location, documented once when the contest starts. Plan scores
//! are recorded only while this section is filled in, and carry the hash of the section they were
//! scored against.

use std::fs;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::bail;
use crate::error::{Error, Result};
use crate::readiness;
use crate::registry::Entry;

/// The file and heading the section lives under (template `harness/templates/RUBRIC.md`).
pub const FILE: &str = "RUBRIC.md";
pub const BASIS_HEADING: &str = "## 0. 채점 근거";

/// Rows of the section's table (`| 항목 | 내용 | 공고 원문 위치 |`) that must have both cells filled.
pub const BASIS_ITEMS: &[&str] = &["주최", "주관", "개최 목적", "심사기준"];

/// The section of `RUBRIC.md` in the contest folder as `{path, sha256}` (hash of the section text),
/// refused when the file or section is missing or a required row is empty or still a template
/// placeholder (`[ ]`, `[...]`).
pub fn basis_document(entry: &Entry) -> Result<Value> {
    let (_, folder) = readiness::folder(entry)?;
    let text = fs::read_to_string(folder.join(FILE)).map_err(|err| {
        Error(format!(
            "{}: {FILE}를 읽을 수 없다({err}). 공모전을 시작할 때 `{BASIS_HEADING}` 절(주최·주관·개최 목적·심사기준과 공고 원문 위치)을 먼저 쓴다",
            entry.id
        ))
    })?;
    let Some(section) = section(&text) else {
        bail!(
            "{}: {FILE}에 `{BASIS_HEADING}` 절이 없다. 주최·주관·개최 목적·심사기준을 공고 원문 위치와 함께 먼저 문서화한다",
            entry.id
        )
    };
    let missing: Vec<&str> = BASIS_ITEMS.iter().copied().filter(|item| !row_filled(section, item)).collect();
    if !missing.is_empty() {
        bail!(
            "{}: {FILE} `{BASIS_HEADING}`의 {} 행이 비어 있다(내용과 공고 원문 위치를 모두 쓴다. 공고에 없으면 \"공고에 없음\"과 확인한 위치)",
            entry.id,
            missing.join(", ")
        );
    }
    Ok(json!({"path": FILE, "sha256": format!("{:x}", Sha256::digest(section.as_bytes()))}))
}

/// The text after the heading line up to the next `## ` heading.
fn section(text: &str) -> Option<&str> {
    let start = text.match_indices(BASIS_HEADING).map(|(i, _)| i).find(|&i| {
        (i == 0 || text[..i].ends_with('\n')) && text[i + BASIS_HEADING.len()..].starts_with(['\n', '\r', ' '])
    })?;
    let body = &text[start + BASIS_HEADING.len()..];
    Some(&body[..body.find("\n## ").unwrap_or(body.len())])
}

/// A table row whose first cell is `item` and whose next two cells are filled.
fn row_filled(section: &str, item: &str) -> bool {
    section.lines().map(str::trim).filter(|line| line.starts_with('|')).any(|line| {
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        cells.len() >= 3 && cells[0] == item && filled(cells[1]) && filled(cells[2])
    })
}

fn filled(cell: &str) -> bool {
    !cell.is_empty() && !(cell.starts_with('[') && cell.ends_with(']'))
}
