//! Linkareer: the list page's Next.js data (`__NEXT_DATA__`) already carries the close time.

use std::collections::HashSet;
use std::sync::LazyLock;

use chrono::DateTime;
use regex::Regex;
use serde_json::Value;

use super::Item;
use crate::error::{Error, Result};
use crate::http;
use crate::text::kst;

pub const NAME: &str = "linkareer";
const LIST_URL: &str = "https://linkareer.com/list/contest";

pub fn url_key(url: &str) -> Option<String> {
    static KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"linkareer\.com/activity/(\d+)").unwrap());
    KEY.captures(url).map(|c| format!("{NAME}:{}", &c[1]))
}

pub fn parse(page: &str) -> Result<Vec<Item>> {
    static DATA: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?s)<script id="__NEXT_DATA__"[^>]*>(.*?)</script>"#).unwrap());
    let json = DATA.captures(page).ok_or_else(|| Error("링커리어 목록 데이터(__NEXT_DATA__)를 찾지 못했다".into()))?;
    let data: Value = serde_json::from_str(&json[1])?;
    let props = &data["props"]["pageProps"];
    let apollo = &props["__APOLLO_STATE__"];
    let mut items = Vec::new();
    for item in props["activityItems"].as_array().into_iter().flatten() {
        let url = item["url"].as_str().ok_or_else(|| Error("링커리어 항목에 url이 없다".into()))?;
        let id = url.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
        let details = &apollo[format!("Activity:{id}").as_str()];
        let deadline = details["recruitCloseAt"]
            .as_f64()
            .filter(|&ms| ms != 0.0)
            .and_then(|ms| DateTime::from_timestamp_millis(ms as i64))
            .map(|at| at.with_timezone(&kst()).format("%Y-%m-%dT%H:%M%:z").to_string());
        items.push(Item {
            entry_id: format!("{NAME}-{id}"),
            title: item["name"].as_str().unwrap_or_default().to_string(),
            url: url.to_string(),
            organizer: details["organizationName"].as_str().unwrap_or_default().to_string(),
            deadline,
            ..Item::default()
        });
    }
    Ok(items)
}

/// Newest first page only; the list has the deadline so no detail page is opened.
pub fn collect(_pages: u32, _known: &HashSet<String>) -> Result<Vec<Item>> {
    parse(&http::get(LIST_URL)?)
}
