//! Campuspick (Everytime careers) contests via the web app's internal list API (form POST, target=1).
//! New items open the detail HTML's `__INITIAL_STATE__` for the official homepage and first prize.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::{Item, common};
use crate::error::{Error, Result};
use crate::{http, text};

pub const NAME: &str = "campuspick";
const API_URL: &str = "https://api2.campuspick.com/find/activity/list";
const PAGE_SIZE: u32 = 20;
const CONTEST_TARGET: u32 = 1;
const HEADERS: &[(&str, &str)] = &[
    ("Origin", "https://www.campuspick.com"),
    ("Referer", "https://www.campuspick.com/contest"),
    ("Content-Type", "application/x-www-form-urlencoded"),
];

static VIEW_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^/[a-z]+/view/?$").unwrap());
static END_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(20\d\d)-(\d\d)-(\d\d)$").unwrap());
static INITIAL_STATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<script id="__INITIAL_STATE__" type="application/json">(.*?)</script>"#).unwrap()
});

/// Category ids baked into the site's JS bundle; unknown ids are dropped.
fn category(id: i64) -> Option<&'static str> {
    Some(match id {
        101 => "기획/아이디어",
        102 => "브랜드/네이밍",
        103 => "광고/마케팅",
        104 => "사진/영상/UCC",
        105 => "디자인",
        106 => "예체능",
        107 => "문학/수기/시나리오",
        108 => "IT/소프트웨어/게임",
        109 => "기타",
        _ => return None,
    })
}

fn view_url(id: &str) -> String {
    format!("https://www.campuspick.com/contest/view?id={id}")
}

pub fn url_key(url: &str) -> Option<String> {
    let parsed = common::Url::parse(url);
    if !parsed.netloc.to_lowercase().ends_with("campuspick.com") || !VIEW_PATH.is_match(parsed.path) {
        return None;
    }
    let id = parsed.param("id");
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_digit())).then(|| format!("{NAME}:{id}"))
}

/// Contests on one API page. Popular/ad bundles repeat the same contests and are ignored.
pub fn parse_list(payload: &Value) -> Result<Vec<Item>> {
    let activities = match (payload.get("status").and_then(Value::as_str), payload.pointer("/result/activities")) {
        (Some("success"), Some(Value::Array(rows))) => rows,
        _ => return Err(Error(format!("캠퍼스픽 목록 응답 형식이 바뀌었다: {}", head(&payload.to_string())))),
    };
    let mut rows = Vec::with_capacity(activities.len());
    for item in activities {
        let id = common::js(item, "id");
        let deadline = END_DATE
            .captures(item["endDate"].as_str().unwrap_or(""))
            .map(|c| text::end_of_day(c[1].parse().unwrap(), c[2].parse().unwrap(), c[3].parse().unwrap()));
        let categories = item["categories"]
            .as_array()
            .map(|ids| ids.iter().filter_map(|c| c.as_i64().and_then(category)).map(String::from).collect())
            .unwrap_or_default();
        rows.push(Item {
            entry_id: format!("{NAME}-{id}"),
            title: common::line(item["title"].as_str().unwrap_or("")),
            url: view_url(&id),
            organizer: common::line(item["company"].as_str().unwrap_or("")),
            deadline,
            categories,
            ..Item::default()
        });
    }
    Ok(rows)
}

#[derive(Debug, Default, PartialEq)]
pub struct Detail {
    pub homepage: String,
    pub prize: String,
}

/// Official homepage and first prize from the detail page's `__INITIAL_STATE__.activity`.
pub fn parse_detail(page_html: &str) -> Result<Detail> {
    let Some(state) = INITIAL_STATE.captures(page_html) else {
        return Err(Error::msg("캠퍼스픽 상세 데이터(__INITIAL_STATE__)를 찾지 못했다"));
    };
    let state: Value = serde_json::from_str(&state[1])?;
    let activity = &state["activity"];
    let website = activity["website"].as_str().unwrap_or("").trim();
    let prize = match &activity["prize_top"] {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(0),
        Value::String(s) => s.trim().parse().unwrap_or(0),
        _ => 0,
    };
    Ok(Detail {
        homepage: if website.starts_with("http") { website.to_string() } else { String::new() },
        prize: if prize != 0 { format!("{}원", common::thousands(prize)) } else { String::new() },
    })
}

/// Fill a new row from its detail page: first prize and official homepage.
pub fn apply_detail(row: &mut Item, page_html: String) -> Result<()> {
    let detail = parse_detail(&page_html)?;
    let note: Vec<String> = [("1등 상금", &detail.prize), ("홈페이지", &detail.homepage)]
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}: {v}"))
        .collect();
    row.note = Some(note.join(" / "));
    if !detail.homepage.is_empty() {
        row.alt_url = Some(detail.homepage);
    }
    Ok(())
}

/// Latest `pages` pages (20 each); only items missing from the registry open the detail page.
pub fn collect(pages: u32, known: &HashSet<String>) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    for page in 0..pages {
        let form = format!("target={CONTEST_TARGET}&limit={PAGE_SIZE}&offset={}&sort=latest", page * PAGE_SIZE);
        let body = http::Request { url: API_URL, headers: HEADERS, body: Some(form.as_bytes()), agent: None }.json()?;
        let rows = parse_list(&body)?;
        if rows.is_empty() {
            break;
        }
        for mut row in rows {
            if !known.contains(&row.entry_id) {
                common::pause();
                let fetched = http::get(&row.url);
                super::with_detail(&mut row, fetched, apply_detail);
            }
            items.push(row);
        }
        common::pause();
    }
    Ok(items)
}

fn head(text: &str) -> String {
    text.chars().take(200).collect()
}
