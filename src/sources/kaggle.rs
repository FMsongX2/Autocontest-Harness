//! Kaggle competitions via the site's internal ListCompetitions API (no login). Prizes and UTC deadlines
//! are in the list, so detail pages are never opened.
//!
//! The active list returns Featured, Research, Playground and Getting Started; Getting Started are
//! prize-less evergreen exercises and are dropped. Community competitions are not in this list.

use std::collections::HashSet;
use std::sync::LazyLock;

use chrono::NaiveDateTime;
use regex::Regex;
use serde_json::{Value, json};

use super::{Item, common};
use crate::error::{Error, Result};
use crate::{http, text};

pub const NAME: &str = "kaggle";
const API_URL: &str = "https://www.kaggle.com/api/i/competitions.CompetitionService/ListCompetitions";
const PAGE_SIZE: u32 = 20;
const GETTING_STARTED: i64 = 5;

static URL_KEY: LazyLock<Regex> = LazyLock::new(|| {
    common::re(r"(?i)^(?:https?://)?(?:www\.)?kaggle\.com/(?:competitions|c)/([A-Za-z0-9][A-Za-z0-9_-]*)")
});
static UTC_STAMP: LazyLock<Regex> = LazyLock::new(|| common::re(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(?:\.\d+)?Z$"));

fn segment(id: i64) -> Option<&'static str> {
    Some(match id {
        1 => "Featured",
        2 => "Research",
        3 => "Recruitment",
        6 => "Masters",
        8 => "Playground",
        10 => "Community",
        _ => return None,
    })
}

pub fn url_key(url: &str) -> Option<String> {
    URL_KEY.captures(url.trim()).map(|c| format!("{NAME}:{}", c[1].to_lowercase()))
}

/// UTC stamps like `2027-01-06T23:59:00Z` or `2026-11-12T23:59:00.807Z` → KST ISO in minutes.
pub fn to_kst(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value.filter(|v| !v.is_empty()) else { return Ok(None) };
    let moment = UTC_STAMP
        .captures(value)
        .and_then(|c| NaiveDateTime::parse_from_str(&c[1], "%Y-%m-%dT%H:%M:%S").ok())
        .ok_or_else(|| Error(format!("Kaggle 마감 형식이 바뀌었다: {value}")))?;
    Ok(Some(moment.and_utc().with_timezone(&text::kst()).format("%Y-%m-%dT%H:%M%:z").to_string()))
}

/// Prize text: `USD 50,000`, swag as 기념품, knowledge/kudos as nothing.
pub fn prize(reward: &Value) -> String {
    let kind = common::js(reward, "id");
    if let Some(amount) = reward.get("quantity").and_then(Value::as_i64).filter(|&n| n != 0) {
        return format!("{kind} {}", common::thousands(amount)).trim().to_string();
    }
    match kind.as_str() {
        "SWAG" => "기념품".to_string(),
        "KNOWLEDGE" | "KUDOS" => String::new(),
        _ => kind,
    }
}

/// Competitions on one API page. The deadline is the new-entrant cutoff, else the final submission.
pub fn parse_list(data: &Value) -> Result<Vec<Item>> {
    let Some(competitions) = data.get("competitions") else {
        return Err(Error::msg("Kaggle API 응답에 competitions가 없다"));
    };
    let mut rows = Vec::new();
    for item in competitions.as_array().map(Vec::as_slice).unwrap_or_default() {
        let segment_id = item.get("competitionHostSegmentId").and_then(Value::as_i64);
        if segment_id == Some(GETTING_STARTED) {
            continue;
        }
        let final_deadline = to_kst(item.get("deadline").and_then(Value::as_str))?;
        let entry = to_kst(item.get("prohibitNewEntrantsExplicitDeadline").and_then(Value::as_str))?;
        let prize = prize(&item["reward"]);
        let mut notes = Vec::new();
        if !prize.is_empty() {
            notes.push(format!("상금: {prize}"));
        }
        if let (Some(entry), Some(last)) = (&entry, &final_deadline)
            && entry != last
        {
            notes.push(format!("최종 제출 마감: {last}"));
        }
        let team = common::js(item, "maxTeamSize");
        if !team.is_empty() {
            notes.push(format!("팀 최대 {team}명"));
        }
        if !common::js(item, "requiresIdentityVerification").is_empty() {
            notes.push("신원 확인 필요".to_string());
        }
        let mut categories: Vec<String> = segment_id.and_then(segment).map(String::from).into_iter().collect();
        if let Some(tags) = item["categories"].as_array() {
            categories.extend(tags.iter().filter_map(|tag| {
                let name = common::js(tag, "displayName");
                (!common::js(tag, "fullPath").is_empty() && !name.is_empty()).then_some(name)
            }));
        }
        let organization = common::js(&item["organization"], "name");
        let host = common::js(item, "hostName");
        rows.push(Item {
            entry_id: format!("{NAME}-{}", common::js(item, "id")),
            title: common::line(&common::js(item, "title")),
            url: format!("https://www.kaggle.com/competitions/{}", common::js(item, "competitionName")),
            organizer: common::line(if organization.is_empty() { &host } else { &organization }),
            deadline: entry.or(final_deadline),
            categories,
            note: Some(notes.join(" / ")),
            alt_url: None,
        });
    }
    Ok(rows)
}

/// `pages` pages (20 each) of active competitions, minus those whose entry deadline has passed.
pub fn collect(pages: u32, _known: &HashSet<String>) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    let mut token = String::new();
    let now = text::now();
    for _ in 0..pages {
        let mut payload = json!({
            "pageSize": PAGE_SIZE,
            "selector": {"listOption": "LIST_OPTION_ACTIVE", "sortOption": "SORT_OPTION_NEWEST"},
        });
        if !token.is_empty() {
            payload["pageToken"] = Value::String(token.clone());
        }
        let data = http::post_json(API_URL, &payload, &[])?;
        items.extend(parse_list(&data)?.into_iter().filter(|row| row.deadline.as_ref().is_none_or(|d| *d >= now)));
        token = common::js(&data, "nextPageToken");
        if token.is_empty() {
            break;
        }
        common::pause();
    }
    Ok(items)
}
