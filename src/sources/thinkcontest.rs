//! Thinkgood: the list screen's internal JSON API (`subList.do`) carries deadline, host, fields and
//! homepage, so no detail page is opened. Open (ING, incl. closing soon) and upcoming (YET) are read
//! with separate process filters; without a filter ~70k closed contests come back.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use super::Item;
use super::common::{Url, js, note, pause, re};
use crate::error::{Error, Result};
use crate::http;
use crate::text::end_of_day;

pub const NAME: &str = "thinkcontest";
const BASE: &str = "https://www.thinkcontest.com/thinkgood/user/contest/";
const PROCESSES: [&str; 2] = ["ING", "YET"];

pub fn url_key(url: &str) -> Option<String> {
    static VIEW_PATH: LazyLock<Regex> = LazyLock::new(|| re(r"^/thinkgood/user/(?:contest|outside|educate)/view\.do$"));
    let parsed = Url::parse(url);
    let host = parsed.host();
    if host.strip_prefix("www.").unwrap_or(&host) != "thinkcontest.com"
        || !VIEW_PATH.is_match(parsed.path.trim_end_matches('/'))
    {
        return None;
    }
    let pk = parsed.param("contest_pk");
    (!pk.is_empty() && pk.chars().all(|c| c.is_ascii_digit())).then(|| format!("{NAME}:{pk}"))
}

pub fn payload(process: &str, page: u32) -> Value {
    json!({"currentPageNo": page, "recordsPerPage": 10, "searchStatus": "Y", "searchProcess": process,
           "sidx": "putup_sdt", "sord": "DESC"})
}

/// Actual end of application (extension applied). Date-only deadlines arrive as 00:00, read as end of day.
pub fn deadline(row: &Value) -> Option<String> {
    static DATETIME: LazyLock<Regex> = LazyLock::new(|| re(r"(20\d\d)-(\d\d)-(\d\d)[ T](\d\d):(\d\d)"));
    let period = js(row, "receivetime_period");
    let candidates = [period.rsplit('~').next().unwrap_or("").to_string(), js(row, "extend_dt"), js(row, "finish_dt")];
    candidates.iter().find_map(|value| {
        let m = DATETIME.captures(value)?;
        Some(if (&m[4], &m[5]) == ("00", "00") {
            end_of_day(m[1].parse().ok()?, m[2].parse().ok()?, m[3].parse().ok()?)
        } else {
            format!("{}-{}-{}T{}:{}+09:00", &m[1], &m[2], &m[3], &m[4], &m[5])
        })
    })
}

/// Contests on one `subList.do` page; ended ones (END) are dropped.
pub fn parse_list(data: &Value) -> Result<Vec<Item>> {
    let status = match &data["status"] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let Some(list) = data["listJsonData"].as_array().filter(|_| status == "1") else {
        return Err(Error(format!("씽굿 목록 응답이 예상과 다르다: {}", data["msg"])));
    };
    let mut items = Vec::new();
    for row in list {
        if row["process"].as_str() == Some("END") {
            continue;
        }
        let pk = js(row, "contest_pk");
        let homepage = js(row, "hompage_url").trim().to_string();
        let prize = Some(js(row, "prize_money")).filter(|p| !p.is_empty()).unwrap_or_else(|| js(row, "award_size_nm"));
        let note = note(&[("응모대상", &js(row, "enter_qualified_nm")), ("상금", &prize), ("홈페이지", &homepage)]);
        items.push(Item {
            entry_id: format!("{NAME}-{pk}"),
            title: js(row, "program_nm").trim().to_string(),
            url: format!("{BASE}view.do?contest_pk={pk}"),
            organizer: js(row, "host_company").trim().to_string(),
            deadline: deadline(row),
            categories: js(row, "contest_field_nm")
                .split(',')
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(String::from)
                .collect(),
            note: (!note.is_empty()).then_some(note),
            alt_url: (homepage.starts_with("http://") || homepage.starts_with("https://")).then_some(homepage),
        });
    }
    Ok(items)
}

/// `pages` pages (10 rows each) of open and upcoming contests; request count is independent of `known`.
pub fn collect(pages: u32, _known: &HashSet<String>) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for process in PROCESSES {
        for page in 1..=pages {
            let data = http::post_json(&format!("{BASE}subList.do"), &payload(process, page), &[])?;
            pause();
            let rows = parse_list(&data)?;
            if data["listJsonData"].as_array().is_none_or(Vec::is_empty) {
                break;
            }
            for row in rows {
                if seen.insert(row.entry_id.clone()) {
                    items.push(row);
                }
            }
        }
    }
    Ok(items)
}
