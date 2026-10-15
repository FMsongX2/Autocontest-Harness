//! Allforyoung contests via the public JSON API the web app uses (`api.allforyoung.com/api/v2/posts/`).
//! The list only has a relative D-day, so new items read `end_at` from the detail API.

use std::collections::HashSet;
use std::sync::LazyLock;

use chrono::{DateTime, NaiveDateTime};
use regex::Regex;
use serde_json::Value;

use super::{Item, common};
use crate::error::{Error, Result};
use crate::{http, text};

pub const NAME: &str = "allforyoung";
const API: &str = "https://api.allforyoung.com/api/v2";

static POST_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:/api/v2)?/posts/(\d+)/?$").unwrap());
static DATE_ONLY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^20\d\d-\d\d-\d\d$").unwrap());

fn list_url(page: u32) -> String {
    // category=공모전, percent-encoded.
    format!("{API}/posts/?category=%EA%B3%B5%EB%AA%A8%EC%A0%84&order=latest&page={page}")
}

pub fn url_key(url: &str) -> Option<String> {
    let parsed = common::Url::parse(url.trim());
    if !parsed.netloc.to_lowercase().ends_with("allforyoung.com") {
        return None;
    }
    POST_PATH.captures(parsed.path).map(|c| format!("{NAME}:{}", &c[1]))
}

/// Open contests on one list page; deadlines come from the detail API.
pub fn parse_list(payload: &Value) -> Result<Vec<Item>> {
    let data = match (&payload["success"], &payload["data"]) {
        (Value::Bool(true), Value::Array(data)) => data,
        _ => {
            let shown: String = payload.to_string().chars().take(200).collect();
            return Err(Error(format!("요즘것들 목록 응답 형식이 바뀌었다: {shown}")));
        }
    };
    let mut rows = Vec::new();
    for item in data {
        if item["category"].as_str() != Some("공모전") || !common::js(item, "is_expired").is_empty() {
            continue;
        }
        let id = common::js(item, "id");
        let categories = item["tags"]
            .as_array()
            .map(|tags| {
                tags.iter().filter_map(|t| t["name"].as_str().filter(|n| !n.is_empty())).map(String::from).collect()
            })
            .unwrap_or_default();
        rows.push(Item {
            entry_id: format!("{NAME}-{id}"),
            title: item["title"].as_str().unwrap_or("").trim().to_string(),
            url: format!("https://www.allforyoung.com/posts/{id}"),
            organizer: item["organization"].as_str().unwrap_or("").trim().to_string(),
            deadline: None,
            categories,
            ..Item::default()
        });
    }
    Ok(rows)
}

/// `YYYY-MM-DD` → 23:59 KST that day; ISO with time → KST (no offset means KST).
pub fn parse_deadline(end_at: Option<&str>) -> Option<String> {
    let value = end_at.unwrap_or("").trim();
    if DATE_ONLY.is_match(value) {
        let mut parts = value.split('-').map(|p| p.parse::<u32>().unwrap());
        let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
        return Some(text::end_of_day(y as i32, m, d));
    }
    let value = value.replace('Z', "+00:00").replacen(' ', "T", 1);
    for format in ["%Y-%m-%dT%H:%M:%S%.f%:z", "%Y-%m-%dT%H:%M%:z"] {
        if let Ok(moment) = DateTime::parse_from_str(&value, format) {
            return Some(moment.with_timezone(&text::kst()).format("%Y-%m-%dT%H:%M%:z").to_string());
        }
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"] {
        if let Ok(moment) = NaiveDateTime::parse_from_str(&value, format) {
            return Some(moment.format("%Y-%m-%dT%H:%M+09:00").to_string());
        }
    }
    None
}

#[derive(Debug, Default, PartialEq)]
pub struct Detail {
    pub deadline: Option<String>,
    pub start: String,
    pub apply_url: String,
    pub permanent: bool,
}

/// Application deadline, apply link and always-open flag from the detail API.
pub fn parse_detail(payload: &Value) -> Result<Detail> {
    let data = match (&payload["success"], &payload["data"]) {
        (Value::Bool(true), data @ Value::Object(_)) => data,
        _ => {
            let shown: String = payload.to_string().chars().take(200).collect();
            return Err(Error(format!("요즘것들 상세 응답 형식이 바뀌었다: {shown}")));
        }
    };
    let permanent = !common::js(data, "is_permanent").is_empty();
    let apply_url = data["apply_url"].as_str().unwrap_or("").trim();
    Ok(Detail {
        deadline: if permanent { None } else { parse_deadline(data["end_at"].as_str()) },
        start: data["start_date"].as_str().unwrap_or("").trim().to_string(),
        apply_url: if apply_url.starts_with("http") { apply_url.to_string() } else { String::new() },
        permanent,
    })
}

/// Latest `pages` pages (20 each); only items missing from the registry open the detail API.
pub fn collect(pages: u32, known: &HashSet<String>) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    for page in 1..=pages {
        let payload = get_json(&list_url(page))?;
        for mut row in parse_list(&payload)? {
            if !known.contains(&row.entry_id) {
                common::pause();
                let id = row.entry_id.split_once('-').map_or("", |(_, id)| id);
                let detail = parse_detail(&get_json(&format!("{API}/posts/{id}/"))?)?;
                row.deadline = detail.deadline;
                let mut note = Vec::new();
                if !detail.start.is_empty() {
                    note.push(format!("접수 시작: {}", detail.start));
                }
                if detail.permanent {
                    note.push("상시 모집".to_string());
                }
                if !detail.apply_url.is_empty() {
                    note.push(format!("지원: {}", detail.apply_url));
                    row.alt_url = Some(detail.apply_url);
                }
                row.note = Some(note.join(" / "));
            }
            items.push(row);
        }
        if common::js(&payload["meta"]["pagination"], "has_next").is_empty() {
            break;
        }
        common::pause();
    }
    Ok(items)
}

fn get_json(url: &str) -> Result<Value> {
    http::Request { url, headers: &[("Accept", "application/json")], body: None, agent: None }.json()
}
