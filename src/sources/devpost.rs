//! Devpost hackathons from the public JSON API (online, open). New items read the detail page's
//! deadline (with time zone) and convert it to KST.
//!
//! The list API has submission dates without time or zone. Cloudflare often answers detail requests
//! with 403; those are not worked around: the list date stays at 23:59 KST with a `시간대 미확인` note.

use std::collections::HashSet;
use std::sync::LazyLock;

use chrono::DateTime;
use regex::Regex;
use serde_json::Value;

use super::{Item, common};
use crate::error::{Error, Result};
use crate::{http, text};

pub const NAME: &str = "devpost";
const NOT_HACKATHON_HOSTS: &[&str] = &["www", "secure", "help", "info", "post", "api", "blog", "challengepost"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
pub const NO_TIMEZONE: &str = "시간대 미확인";

use super::common::re;

static RANGE_END: LazyLock<Regex> = LazyLock::new(|| {
    re(r"([A-Z][a-z]{2})\s+(\d{1,2})(?:,\s*\d{4})?\s*-\s*(?:([A-Z][a-z]{2})\s+)?(\d{1,2}),\s*(\d{4})\s*$")
});
static SINGLE_DATE: LazyLock<Regex> = LazyLock::new(|| re(r"([A-Z][a-z]{2})\s+(\d{1,2}),\s*(\d{4})\s*$"));
static TAG: LazyLock<Regex> = LazyLock::new(|| re(r"<[^>]+>"));
static NONZERO_DIGIT: LazyLock<Regex> = LazyLock::new(|| re(r"[1-9]"));
static ISO_DATE: LazyLock<Regex> = LazyLock::new(|| re(r#"data-iso-date="([^"]+)""#));
static TIME_LEFT: LazyLock<Regex> = LazyLock::new(|| re(r#"<time id="time-left"[^>]*datetime="([^"]+)""#));
static ELIGIBILITY: LazyLock<Regex> = LazyLock::new(|| re(r#"(?s)<ul id="eligibility-list"[^>]*>(.*?)</ul>"#));
static COUNTRIES_MODAL: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?s)<ul id="eligibility-countries-modal-list"[^>]*>(.*?)</ul>"#));
static LIST_ITEM: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)<li>(.*?)</li>"));
static TOOLTIP: LazyLock<Regex> = LazyLock::new(|| re(r#"data-tooltip title="([^"]+)""#));
static SOUTH_KOREA: LazyLock<Regex> = LazyLock::new(|| re(r"\bSouth Korea\b|Korea,? Republic of|^Korea$"));

/// Hackathon pages live on `<slug>.devpost.com`; sub-paths (/rules …) are the same hackathon.
pub fn url_key(url: &str) -> Option<String> {
    let host = common::Url::parse(url).host();
    let slug = host.strip_suffix(".devpost.com")?;
    if slug.is_empty() || slug.contains('.') || NOT_HACKATHON_HOSTS.contains(&slug) {
        return None;
    }
    Some(format!("{NAME}:{slug}"))
}

fn month(name: &str) -> Option<u32> {
    MONTHS.iter().position(|m| *m == name).map(|i| i as u32 + 1)
}

/// End date of `Aug 31 - Oct 23, 2026`, `Oct 05 - 27, 2026`, `Jul 16, 2026 - Jan 15, 2027` at 23:59 KST.
pub fn list_end_date(period: &str) -> Option<String> {
    if let Some(c) = RANGE_END.captures(period) {
        let month = month(c.get(3).unwrap_or_else(|| c.get(1).unwrap()).as_str())?;
        return Some(text::end_of_day(c[5].parse().ok()?, month, c[4].parse().ok()?));
    }
    let c = SINGLE_DATE.captures(period)?;
    Some(text::end_of_day(c[3].parse().ok()?, month(&c[1])?, c[2].parse().ok()?))
}

/// Hackathons on one API page, minus invite-only ones. Deadlines are list dates until the detail fixes them.
pub fn parse_list(data: &Value) -> Result<Vec<Item>> {
    let Some(hackathons) = data.get("hackathons") else {
        return Err(Error::msg("Devpost API 응답에 hackathons가 없다"));
    };
    let mut rows = Vec::new();
    for item in hackathons.as_array().map(Vec::as_slice).unwrap_or_default() {
        if !common::js(item, "invite_only").is_empty() || item["open_state"].as_str() != Some("open") {
            continue;
        }
        let prize = common::text(&TAG.replace_all(item["prize_amount"].as_str().unwrap_or(""), ""));
        let deadline = list_end_date(item["submission_period_dates"].as_str().unwrap_or(""));
        let mut note = Vec::new();
        if NONZERO_DIGIT.is_match(&prize) {
            note.push(format!("상금: {prize}"));
        }
        if deadline.is_some() {
            note.push(NO_TIMEZONE.to_string());
        }
        let categories = item["themes"]
            .as_array()
            .map(|themes| themes.iter().map(|t| common::js(t, "name")).collect())
            .unwrap_or_default();
        rows.push(Item {
            entry_id: format!("{NAME}-{}", common::js(item, "id")),
            title: common::js(item, "title"),
            url: common::js(item, "url"),
            organizer: common::js(item, "organization_name"),
            deadline,
            categories,
            note: Some(note.join(" / ")),
            alt_url: None,
        });
    }
    Ok(rows)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detail {
    pub deadline: Option<String>,
    pub eligibility: Vec<String>,
    pub excluded: Vec<String>,
    pub korea_excluded: bool,
}

/// Submission deadline (ISO with zone, converted to KST), eligibility and excluded countries.
pub fn parse_detail(page_html: &str) -> Detail {
    let iso = ISO_DATE.captures(page_html).or_else(|| TIME_LEFT.captures(page_html));
    // A zone-less or unreadable stamp leaves the deadline unknown, like a naive datetime in Python.
    let deadline = iso
        .and_then(|c| DateTime::parse_from_rfc3339(&c[1].replace('Z', "+00:00")).ok())
        .map(|moment| moment.with_timezone(&text::kst()).format("%Y-%m-%dT%H:%M%:z").to_string());
    let block = ELIGIBILITY.captures(page_html).map(|c| c.get(1).unwrap().as_str());
    let items = |fragment: &str| LIST_ITEM.captures_iter(fragment).map(|li| common::text(&li[1])).collect::<Vec<_>>();
    let eligibility = block.map(items).unwrap_or_default();
    let mut excluded = Vec::new();
    if let Some(tooltip) = block.and_then(|b| TOOLTIP.captures(b)) {
        excluded =
            common::text(&tooltip[1]).split(',').map(str::trim).filter(|c| !c.is_empty()).map(String::from).collect();
    }
    if let Some(modal) = COUNTRIES_MODAL.captures(page_html) {
        excluded = items(&modal[1]);
    }
    let korea_excluded = excluded.iter().any(|c| SOUTH_KOREA.is_match(c) && !c.contains("Democratic"));
    Detail { deadline, eligibility, excluded, korea_excluded }
}

/// Online open hackathons on `pages` API pages (9 each). New items open the detail page; a blocked
/// detail (HTTP error) keeps the list values, including the `시간대 미확인` note.
pub fn collect(pages: u32, known: &HashSet<String>) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    for page in 1..=pages {
        let url = format!("https://devpost.com/api/hackathons?challenge_type[]=online&status[]=open&page={page}");
        let data =
            http::Request { url: &url, headers: &[("Accept", "application/json")], body: None, agent: None }.json()?;
        if data["hackathons"].as_array().is_none_or(Vec::is_empty) {
            break;
        }
        for mut row in parse_list(&data)? {
            if !known.contains(&row.entry_id) {
                common::pause();
                if let Ok(page_html) = http::get(&row.url) {
                    let detail = parse_detail(&page_html);
                    let mut notes: Vec<String> = row
                        .note
                        .as_deref()
                        .unwrap_or("")
                        .split(" / ")
                        .filter(|part| !part.is_empty() && *part != NO_TIMEZONE)
                        .map(String::from)
                        .collect();
                    if detail.deadline.is_some() {
                        row.deadline = detail.deadline;
                    } else if row.deadline.is_some() {
                        notes.push(NO_TIMEZONE.to_string());
                    }
                    if !detail.eligibility.is_empty() {
                        notes.push(format!("참가자격: {}", detail.eligibility.join("; ")));
                    }
                    if !detail.excluded.is_empty() {
                        notes.push(format!("제외 국가: {}", detail.excluded.join(", ")));
                    }
                    if detail.korea_excluded {
                        notes.push("한국 참가 제외".to_string());
                    }
                    row.note = Some(notes.join(" / "));
                }
            }
            items.push(row);
        }
        common::pause();
    }
    Ok(items)
}
