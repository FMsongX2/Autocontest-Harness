//! Detizen: the site reads Supabase REST (`posts`) straight from the browser. With the same
//! publishable key we read published contests whose deadline is today or later, newest first; one
//! response has host, deadline, fields, eligibility, prize and homepage, so no detail page is opened.
//! A changed key shows up as 401: find the new `sb_publishable_` key in the www.detizen.com JS bundle.

use std::collections::HashSet;
use std::sync::LazyLock;

use chrono::Utc;
use regex::Regex;
use serde_json::Value;

use super::Item;
use super::common::{Url, js, note, pause, quote, re, text, unquote, urlencode};
use crate::error::{Error, Result};
use crate::http;
use crate::text::{end_of_day, kst};

pub const NAME: &str = "detizen";
const SITE: &str = "https://detizen.com";
const API: &str = "https://tratibfvotxaiadjaijf.supabase.co/rest/v1/posts";
const API_KEY: &str = "sb_publishable_ZbFhb5Qy7vETtfPksIXrPQ_kK3sTe4F";
const PAGE_SIZE: usize = 20;
const FIELDS: &str = "id,title,slug,end_date,contest_organizer_name,homepage_url,content_eligibility,content_awards,\
                      post_contest_categories(contest_categories(name))";
const NOTE_LIMIT: usize = 120;

pub fn url_key(url: &str) -> Option<String> {
    static DETAIL_PATH: LazyLock<Regex> = LazyLock::new(|| re(r"^/contests/([^/]+)/?$"));
    let parsed = Url::parse(url);
    let host = parsed.host();
    if host != "detizen.com" && !host.ends_with(".detizen.com") {
        return None;
    }
    DETAIL_PATH.captures(parsed.path).map(|c| format!("{NAME}:{}", unquote(&c[1])))
}

/// REST URL of page `page` (from 1): published contests with no deadline or one on/after `today`, like the site.
pub fn list_url(page: u32, today: &str) -> String {
    let query = urlencode(&[
        ("select", FIELDS.into()),
        ("type", "eq.contest".into()),
        ("status", "eq.published".into()),
        ("or", format!("(end_date.is.null,end_date.gte.{today})")),
        ("order", "created_at.desc,id.desc".into()),
        ("limit", PAGE_SIZE.to_string()),
        ("offset", ((page.max(1) as usize - 1) * PAGE_SIZE).to_string()),
    ]);
    format!("{API}?{query}")
}

fn short(fragment: &str) -> String {
    let value = text(fragment);
    if value.chars().count() <= NOTE_LIMIT {
        return value;
    }
    let cut: String = value.chars().take(NOTE_LIMIT).collect();
    format!("{}…", cut.trim_end())
}

/// Contests in one REST response page. `end_date` is date-only, so 23:59 that day.
pub fn parse_list(rows: &Value) -> Result<Vec<Item>> {
    static DATE: LazyLock<Regex> = LazyLock::new(|| re(r"^(\d{4})-(\d\d)-(\d\d)$"));
    let Some(rows) = rows.as_array() else {
        let raw = rows.to_string();
        return Err(Error(format!("대티즌 응답이 목록이 아니다: {}", raw.chars().take(200).collect::<String>())));
    };
    let mut items = Vec::new();
    for row in rows {
        let (Some(id), Some(title), Some(slug)) = (row.get("id"), row["title"].as_str(), row["slug"].as_str()) else {
            return Err(Error(format!("대티즌 항목에 id·title·slug가 없다: {row}")));
        };
        let id = match id {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let end = js(row, "end_date");
        let deadline = DATE
            .captures(&end)
            .and_then(|d| Some(end_of_day(d[1].parse().ok()?, d[2].parse().ok()?, d[3].parse().ok()?)));
        let categories = row["post_contest_categories"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|link| link["contest_categories"]["name"].as_str().filter(|n| !n.is_empty()).map(String::from))
            .collect();
        let note = note(&[
            ("응모대상", &short(&js(row, "content_eligibility"))),
            ("상금", &short(&js(row, "content_awards"))),
        ]);
        let homepage = js(row, "homepage_url").trim().to_string();
        items.push(Item {
            entry_id: format!("{NAME}-{id}"),
            title: text(title),
            url: format!("{SITE}/contests/{}", quote(slug, "/")),
            organizer: js(row, "contest_organizer_name").trim().to_string(),
            deadline,
            categories,
            note: (!note.is_empty()).then_some(note),
            alt_url: (homepage.starts_with("http://") || homepage.starts_with("https://")).then_some(homepage),
        });
    }
    Ok(items)
}

/// `pages` pages (20 each) newest first; the list has everything, so `known` does not change the requests.
pub fn collect(pages: u32, _known: &HashSet<String>) -> Result<Vec<Item>> {
    let today = Utc::now().with_timezone(&kst()).format("%Y-%m-%d").to_string();
    let mut items = Vec::new();
    for page in 1..=pages {
        if page > 1 {
            pause();
        }
        let url = list_url(page, &today);
        let headers = [("Accept", "application/json"), ("apikey", API_KEY)];
        let rows = parse_list(&http::Request { url: &url, headers: &headers, body: None, agent: None }.json()?)?;
        let full = rows.len() >= PAGE_SIZE;
        items.extend(rows);
        if !full {
            break;
        }
    }
    Ok(items)
}
