//! ContestKorea contests (`int_gbn=1`; `int_gbn=2` is extracurricular activities and is not read).
//!
//! The list is newest-registered first with year-less `MM.DD~MM.DD` periods. The year is the one that
//! puts the start date closest to the registration date (first 8 digits of `str_no`); an end before
//! the start rolls to the next year. New rows open the detail page for the dated period, host,
//! eligibility, prize and the organiser's notice URL. Closed rows (judging, announced, …마감) are
//! mixed into the list and filtered by their status label.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use chrono::{Datelike, NaiveDate};
use regex::Regex;

use super::Item;
use super::common::{Url, note, pause, re, text};
use crate::error::{Error, Result};
use crate::http;
use crate::text::end_of_day;

pub const NAME: &str = "contestkorea";
const BASE: &str = "https://www.contestkorea.com/sub/";
const OPEN_STATES: [&str; 3] = ["접수중", "접수예정", "마감임박"];

fn view_url(bcode: &str, no: &str) -> String {
    format!("{BASE}view.php?int_gbn=1&Txt_bcode={bcode}&str_no={no}")
}

pub fn url_key(url: &str) -> Option<String> {
    let parsed = Url::parse(url);
    if !parsed.netloc.to_lowercase().ends_with("contestkorea.com") || !parsed.path.ends_with("/view.php") {
        return None;
    }
    let no = parsed.param("str_no");
    (!no.is_empty() && no.chars().all(|c| c.is_ascii_digit())).then(|| format!("{NAME}:{no}"))
}

/// Status labels after the application closed. Empty or unknown labels stay for the gate to read.
fn is_closed(state: &str) -> bool {
    if state.is_empty() || OPEN_STATES.contains(&state) {
        return false;
    }
    state.ends_with("마감") || state.contains("심사") || state.contains("발표") || state.contains("종료")
}

/// Tidy `중학생 , 해당자 ▶ (현역 군인)` into `중학생, 해당자(현역 군인)`.
fn targets(fragment: &str) -> String {
    static COMMA: LazyLock<Regex> = LazyLock::new(|| re(r"\s*,\s*"));
    static PAREN: LazyLock<Regex> = LazyLock::new(|| re(r"\s*\(\s*(.*?)\s*\)"));
    let value = COMMA.replace_all(&text(fragment).replace('▶', ""), ", ").into_owned();
    PAREN.replace_all(&value, "(${1})").trim_matches([' ', ',']).to_string()
}

/// The end of a list period `MM.DD~MM.DD`, with the year taken from the registration date `YYYYMMDD`.
pub fn infer_deadline(period: &str, registered: &str) -> Option<String> {
    static MONTH_DAY: LazyLock<Regex> = LazyLock::new(|| re(r"(\d{1,2})\.(\d{1,2})"));
    let found: Vec<(u32, u32)> =
        MONTH_DAY.captures_iter(period).filter_map(|c| Some((c[1].parse().ok()?, c[2].parse().ok()?))).collect();
    if found.is_empty() || registered.len() != 8 || !registered.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let reg = NaiveDate::from_ymd_opt(
        registered[..4].parse().ok()?,
        registered[4..6].parse().ok()?,
        registered[6..].parse().ok()?,
    )?;
    let (start, end) = (found[0], found[found.len() - 1]);
    let year = reg.year();
    let candidates: Option<Vec<(i32, i64)>> = [year - 1, year, year + 1]
        .into_iter()
        .map(|y| NaiveDate::from_ymd_opt(y, start.0, start.1).map(|d| (y, (d - reg).num_days().abs())))
        .collect();
    // Python's min() keeps the first of equal distances; an impossible date (Feb 29) falls back to the registration year.
    let start_year = candidates
        .and_then(|c| c.into_iter().reduce(|best, next| if next.1 < best.1 { next } else { best }))
        .map_or(year, |(y, _)| y);
    let end_year = start_year + i32::from(end < start);
    Some(end_of_day(end_year, end.0, end.1))
}

/// Open or upcoming rows on one list page. A page without the list area means the structure changed.
pub fn parse_list(page: &str) -> Result<Vec<Item>> {
    static SPLIT: LazyLock<Regex> =
        LazyLock::new(|| re(r#"<li(?: class="[^"]*")?>\s*<!--\s*<span class="icon_special""#));
    static LINK: LazyLock<Regex> =
        LazyLock::new(|| re(r#"href="[^"]*view\.php\?[^"]*Txt_bcode=(\d*)&(?:amp;)?str_no=(\d+)""#));
    static STATE: LazyLock<Regex> = LazyLock::new(|| re(r#"(?s)<span class="condition"[^>]*>(.*?)</span>"#));
    static TITLE: LazyLock<Regex> = LazyLock::new(|| re(r#"(?s)<span class="txt">(.*?)</span>"#));
    static CATEGORY: LazyLock<Regex> = LazyLock::new(|| re(r#"(?s)<span class="category">(.*?)</span>"#));
    static HOST: LazyLock<Regex> =
        LazyLock::new(|| re(r#"(?s)<li class="icon_1"><strong>주최</strong>\s*\.(.*?)</li>"#));
    static TARGET: LazyLock<Regex> =
        LazyLock::new(|| re(r#"(?s)<li class="icon_2"><strong>대상</strong>\s*\.(.*?)</li>"#));
    static PERIOD: LazyLock<Regex> = LazyLock::new(|| re(r#"(?s)<span class="step-1"><em>접수</em>(.*?)</span>"#));
    let Some(start) = page.find(r#"<div class="list_style_2">"#) else {
        return Err(Error("contestkorea: list_style_2 목록 영역이 없음(구조 변경 의심)".into()));
    };
    let section = match page[start..].find(r#"class="pagination""#) {
        Some(end) if start + end > 0 => &page[start..start + end],
        _ => &page[start..],
    };
    let capture = |pattern: &Regex, block: &str| pattern.captures(block).map(|c| text(&c[1]));
    let mut rows = Vec::new();
    for block in SPLIT.split(section).skip(1) {
        let Some(link) = LINK.captures(block) else { continue };
        if is_closed(&capture(&STATE, block).unwrap_or_default()) {
            continue;
        }
        let (bcode, no) = (&link[1], &link[2]);
        let eligibility = TARGET.captures(block).map(|c| targets(&c[1])).unwrap_or_default();
        rows.push(Item {
            entry_id: format!("{NAME}-{no}"),
            title: capture(&TITLE, block).unwrap_or_default(),
            url: view_url(bcode, no),
            organizer: capture(&HOST, block).unwrap_or_default(),
            deadline: capture(&PERIOD, block).and_then(|period| infer_deadline(&period, &no[..no.len().min(8)])),
            categories: capture(&CATEGORY, block).filter(|c| !c.is_empty()).into_iter().collect(),
            note: (!eligibility.is_empty()).then(|| format!("참가대상: {eligibility}")),
            alt_url: None,
        });
    }
    Ok(rows)
}

#[derive(Debug, Default, PartialEq)]
pub struct Detail {
    pub deadline: Option<String>,
    pub organizer: String,
    pub eligibility: String,
    pub prize: String,
    pub homepage: String,
}

/// Host, eligibility, end of period, prize and the organiser's notice URL from the detail table.
pub fn parse_detail(page: &str) -> Detail {
    static ROW: LazyLock<Regex> = LazyLock::new(|| re(r#"(?s)<th scope="row">(.*?)</th>\s*<td>(.*?)</td>"#));
    static DATE: LazyLock<Regex> = LazyLock::new(|| re(r"(20\d\d)\.(\d\d)\.(\d\d)"));
    static HOMEPAGE: LazyLock<Regex> = LazyLock::new(|| re(r#"val2="(https?://[^"]+)""#));
    let info: HashMap<String, &str> =
        ROW.captures_iter(page).map(|c| (text(&c[1]), c.get(2).map_or("", |m| m.as_str()))).collect();
    let get = |key: &str| info.get(key).copied().unwrap_or("");
    let period = text(get("접수기간"));
    let deadline = DATE
        .captures_iter(&period)
        .last()
        .and_then(|d| Some(end_of_day(d[1].parse().ok()?, d[2].parse().ok()?, d[3].parse().ok()?)));
    Detail {
        deadline,
        organizer: text(get("주최 . 주관")),
        eligibility: targets(get("참가대상")),
        prize: text(get("시상내역")),
        homepage: HOMEPAGE.captures(get("홈페이지")).map(|c| c[1].to_string()).unwrap_or_default(),
    }
}

/// Fill a new row from its detail page: dated deadline, host, eligibility, prize and notice URL.
pub fn apply_detail(row: &mut Item, page: String) -> Result<()> {
    let detail = parse_detail(&page);
    row.deadline = detail.deadline.or(row.deadline.take());
    if !detail.organizer.is_empty() {
        row.organizer = detail.organizer;
    }
    let joined = note(&[("참가대상", &detail.eligibility), ("시상", &detail.prize), ("홈페이지", &detail.homepage)]);
    row.note = Some(if joined.is_empty() { row.note.take().unwrap_or_default() } else { joined });
    if !detail.homepage.is_empty() {
        row.alt_url = Some(detail.homepage);
    }
    Ok(())
}

/// Open and upcoming rows on the first `pages` list pages; rows not in `known` get the dated deadline from the detail page.
pub fn collect(pages: u32, known: &HashSet<String>) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    for page in 1..=pages {
        if page > 1 {
            pause();
        }
        for mut row in parse_list(&http::get(&format!("{BASE}list.php?int_gbn=1&page={page}"))?)? {
            if !known.contains(&row.entry_id) {
                pause();
                let fetched = http::get(&row.url);
                super::with_detail(&mut row, fetched, apply_detail);
            }
            items.push(row);
        }
    }
    Ok(items)
}
