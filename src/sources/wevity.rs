//! Wevity: the list has no deadline, so only new rows open the detail page for the end of the
//! application period.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

use super::Item;
use super::common::{Url, note, pause, re, text};
use crate::error::Result;
use crate::http;
use crate::text::end_of_day;

pub const NAME: &str = "wevity";
const BASE: &str = "https://www.wevity.com/";

fn list_url(page: u32) -> String {
    format!("{BASE}?c=find&s=1&gub=1&gp={page}")
}

fn view_url(ix: &str) -> String {
    format!("{BASE}?c=find&s=1&gub=1&gbn=viewok&gp=1&ix={ix}")
}

pub fn url_key(url: &str) -> Option<String> {
    let parsed = Url::parse(url);
    if !parsed.netloc.to_lowercase().contains("wevity.com") {
        return None;
    }
    let ix = parsed.param("ix");
    (!ix.is_empty()).then(|| format!("{NAME}:{ix}"))
}

/// Contest rows on one list page; the deadline is filled from the detail page.
pub fn parse_list(page: &str) -> Vec<Item> {
    static BLOCK: LazyLock<Regex> =
        LazyLock::new(|| re(r#"(?s)<li[^>]*>\s*(?:<!--[^>]*-->)?\s*<div class="tit">(.*?)</li>"#));
    static LINK: LazyLock<Regex> = LazyLock::new(|| re(r#"(?s)<a href="[^"]*ix=(\d+)[^"]*">(.*?)</a>"#));
    static STAT: LazyLock<Regex> = LazyLock::new(|| re(r"<span class='stat[^']*'>.*?</span>"));
    static SUB: LazyLock<Regex> = LazyLock::new(|| re(r#"(?s)<div class="sub-tit">\s*분야\s*:\s*(.*?)</div>"#));
    static ORGAN: LazyLock<Regex> = LazyLock::new(|| re(r#"(?s)<div class="organ">(.*?)</div>"#));
    let mut rows = Vec::new();
    for block in BLOCK.captures_iter(page) {
        let block = block.get(1).map_or("", |m| m.as_str());
        let Some(link) = LINK.captures(block) else { continue };
        let ix = &link[1];
        let categories = SUB.captures(block).map_or_else(Vec::new, |sub| {
            text(&sub[1]).split(',').map(str::trim).filter(|c| !c.is_empty()).map(String::from).collect()
        });
        rows.push(Item {
            entry_id: format!("{NAME}-{ix}"),
            title: text(&STAT.replace_all(&link[2], "")),
            url: view_url(ix),
            organizer: ORGAN.captures(block).map(|c| text(&c[1])).unwrap_or_default(),
            deadline: None,
            categories,
            ..Item::default()
        });
    }
    rows
}

#[derive(Debug, Default, PartialEq)]
pub struct Detail {
    pub deadline: Option<String>,
    pub eligibility: String,
    pub prize: String,
    pub homepage: String,
}

/// End of the application period, eligibility, prize and homepage from a detail page.
pub fn parse_detail(page: &str) -> Detail {
    static FIELD: LazyLock<Regex> = LazyLock::new(|| re(r#"(?s)<span class="tit">([^<]+)</span>(.*?)</li>"#));
    static DATE: LazyLock<Regex> = LazyLock::new(|| re(r"(20\d\d)-(\d\d)-(\d\d)"));
    static HOMEPAGE: LazyLock<Regex> = LazyLock::new(|| re(r#"<span class="tit">홈페이지</span>\s*<a href="([^"]+)""#));
    let mut info = std::collections::HashMap::new();
    for field in FIELD.captures_iter(page) {
        info.insert(field[1].trim().to_string(), text(&field[2]));
    }
    let get = |key: &str| info.get(key).cloned().unwrap_or_default();
    let period = get("접수기간");
    let deadline = DATE
        .captures_iter(&period)
        .last()
        .map(|d| end_of_day(d[1].parse().unwrap_or(0), d[2].parse().unwrap_or(0), d[3].parse().unwrap_or(0)));
    Detail {
        deadline,
        eligibility: get("응모대상"),
        prize: get("총 상금"),
        homepage: HOMEPAGE.captures(page).map(|c| c[1].to_string()).unwrap_or_default(),
    }
}

/// Every row on the first `pages` list pages; rows not in `known` get their detail page read.
pub fn collect(pages: u32, known: &HashSet<String>) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    for page in 1..=pages {
        let rows = parse_list(&http::get(&list_url(page))?);
        if rows.is_empty() {
            break;
        }
        for mut row in rows {
            if !known.contains(&row.entry_id) {
                pause();
                let detail = parse_detail(&http::get(&row.url)?);
                row.deadline = detail.deadline;
                let parts = [
                    ("응모대상", detail.eligibility.as_str()),
                    ("상금", &detail.prize),
                    ("홈페이지", &detail.homepage),
                ];
                row.note = Some(note(&parts));
                if !detail.homepage.is_empty() {
                    row.alt_url = Some(detail.homepage.clone());
                }
            }
            items.push(row);
        }
        pause();
    }
    Ok(items)
}
