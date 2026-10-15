//! `BOARD.md`: contests grouped by state.

use std::path::PathBuf;

use chrono::{DateTime, NaiveDate, NaiveDateTime};

use crate::error::Result;
use crate::paths;
use crate::registry::{Entry, Registry, latest_score, score_label};
use crate::text::{kst, now};

const ORDER: &[(&str, &str)] = &[
    ("awaiting_approval", "제출 승인 대기"),
    ("plan_review", "레벨 5 계획 검수 대기"),
    ("running", "진행 중"),
    ("routed", "라우팅됨(실행 전)"),
    ("gate_pass", "게이트 통과(배정 대기)"),
    ("hold", "보류(확인 필요)"),
    ("discovered", "새로 발견(게이트 전)"),
    ("submitted", "제출 완료"),
];
const COUNT_ONLY: &[(&str, &str)] =
    &[("closed", "종료"), ("rejected", "게이트 탈락"), ("skipped", "하지 않기로 함"), ("duplicate", "중복")];
/// States whose rows show the latest score next to type and level.
const SCORED: &[&str] = &["running", "awaiting_approval", "plan_review"];
const DISCOVERED_LIMIT: usize = 40;

/// The calendar date of an ISO deadline in its own offset (what Python's `fromisoformat(...).date()` gives).
fn deadline_date(deadline: &str) -> Option<NaiveDate> {
    let text = deadline.replace("T24:00", "T23:59");
    if let Ok(dt) = DateTime::parse_from_rfc3339(&text) {
        return Some(dt.date_naive());
    }
    for format in ["%Y-%m-%dT%H:%M%:z", "%Y-%m-%d %H:%M%:z", "%Y-%m-%dT%H:%M:%S%.f%:z", "%Y-%m-%d %H:%M:%S%.f%:z"] {
        if let Ok(dt) = DateTime::parse_from_str(&text, format) {
            return Some(dt.date_naive());
        }
    }
    for format in [
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H",
        "%Y-%m-%d %H",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(&text, format) {
            return Some(dt.date());
        }
    }
    NaiveDate::parse_from_str(&text, "%Y-%m-%d").ok()
}

/// `D-3`, `D-day`, `D+2`; `-` without a deadline and the raw text when it does not parse.
pub fn dday(deadline: Option<&str>, today: NaiveDate) -> String {
    let Some(deadline) = deadline.filter(|d| !d.is_empty()) else { return "-".to_string() };
    let Some(end) = deadline_date(deadline) else { return deadline.to_string() };
    let days = (end - today).num_days();
    match days {
        d if d > 0 => format!("D-{d}"),
        0 => "D-day".to_string(),
        d => format!("D+{}", -d),
    }
}

fn row(e: &Entry, today: NaiveDate) -> String {
    let title = e.title.replace('|', "\\|");
    let link = if e.url.is_empty() { title } else { format!("[{title}]({})", e.url) };
    let folder = e.folder.as_deref().and_then(|f| f.rsplit('/').next()).filter(|f| !f.is_empty()).unwrap_or("-");
    let level = e.level.filter(|&l| l != 0).map(|l| format!("L{l}")).unwrap_or_else(|| "-".to_string());
    let mut kind = format!("{} / {level}", e.kind.as_deref().filter(|k| !k.is_empty()).unwrap_or("-"));
    if SCORED.contains(&e.state.as_str())
        && let Some(score) = latest_score(e, None)
    {
        kind = format!("{kind} · {}", score_label(score));
    }
    format!("| {} | {link} | {kind} | {folder} | `{}` |", dday(e.deadline.as_deref(), today), e.id)
}

pub fn render(registry: &Registry) -> String {
    render_on(registry, chrono::Utc::now().with_timezone(&kst()).date_naive())
}

/// [`render`] with a fixed "today" (for tests).
pub fn render_on(registry: &Registry, today: NaiveDate) -> String {
    let entries: Vec<&Entry> = registry.contests.values().collect();
    let count = |state: &str| entries.iter().filter(|e| e.state == state).count();
    let mut lines = vec![
        "# 공모전 보드".to_string(),
        String::new(),
        format!("갱신: {}. `harness board`가 레지스트리에서 만든다. 직접 고치지 않는다.", now()),
        String::new(),
    ];
    for &(state, label) in ORDER {
        let mut rows: Vec<&Entry> = entries.iter().copied().filter(|e| e.state == state).collect();
        rows.sort_by(|a, b| {
            let da = a.deadline.as_deref().filter(|d| !d.is_empty()).unwrap_or("9999");
            let db = b.deadline.as_deref().filter(|d| !d.is_empty()).unwrap_or("9999");
            da.cmp(db).then(b.priority.cmp(&a.priority))
        });
        if state == "discovered" {
            rows.truncate(DISCOVERED_LIMIT);
        }
        lines.push(format!("## {label} ({})", count(state)));
        lines.push(String::new());
        if rows.is_empty() {
            lines.push("(없음)".to_string());
            lines.push(String::new());
            continue;
        }
        lines.push("| 마감 | 공모전 | 유형·레벨 | 폴더 | id |".to_string());
        lines.push("|---|---|---|---|---|".to_string());
        lines.extend(rows.iter().map(|e| row(e, today)));
        lines.push(String::new());
    }
    let counts: Vec<String> = COUNT_ONLY.iter().map(|&(s, label)| format!("{label} {}", count(s))).collect();
    lines.push(format!("그 밖: {}.", counts.join(", ")));
    lines.push(String::new());
    lines.join("\n")
}

/// Write `BOARD.md` at the repository root and return its path.
pub fn write(registry: &Registry) -> Result<PathBuf> {
    let path = paths::board();
    std::fs::write(&path, render(registry))?;
    Ok(path)
}
