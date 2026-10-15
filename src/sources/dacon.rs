//! DACON competitions. The `/competitions` page embeds every official card in `window.__NUXT__`;
//! new competitions open the detail API for the registration / leaderboard submission deadlines.
//!
//! `period_end` (contest end) can be later than registration or submission closing, so `deadline` is
//! the earliest of those found in the schedule text or timeline. The API `submission_end` sometimes keeps
//! a previous contest's value, so it is used only when the schedule has no submission deadline, it lies
//! within the contest period and it is not before the registration deadline.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

use super::{Item, common};
use crate::error::{Error, Result};
use crate::{http, text};

pub const NAME: &str = "dacon";
const LIST_URL: &str = "https://dacon.io/competitions";

use super::common::re;

static URL_KEY: LazyLock<Regex> = LazyLock::new(|| re(r"^/(?:en/)?competitions/(?:official|open)/(\d+)(?:/|$)"));
static NUXT: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?s)window\.__NUXT__=\(function\(([^)]*)\)\{return (.*)\}\((.*?)\)\);?\s*</script>"));
static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    re(r#"^(?:"(?:[^"\\]|\\.)*"|[A-Za-z_$][\w$]*|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?|[\[\]{}(),:]|\s+)"#)
});
static VOID: LazyLock<Regex> = LazyLock::new(|| re(r"\bvoid 0\b"));
static KST_TIME: LazyLock<Regex> = LazyLock::new(|| re(r"^(\d{4})-(\d\d)-(\d\d)[ T](\d\d):(\d\d)"));
static BLOCK_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"<(?:p|li|br|h\d)[^>]*>"));
static SCHEDULE_LABEL: LazyLock<Regex> =
    LazyLock::new(|| re(r"(참가\s*(?:신청|접수)?\s*기간|참가\s*(?:신청|접수)\s*마감|리더보드\s*제출\s*마감)\s*:?\s*"));
static ITEM_SEPARATOR: LazyLock<Regex> = LazyLock::new(|| re(r"\s-\s"));
static KOREAN_DATE: LazyLock<Regex> = LazyLock::new(|| {
    re(r"^\s*(20\d\d)\s*(?:년|[.\-/])\s*(\d{1,2})\s*(?:월|[.\-/])\s*(\d{1,2})\s*일?\s*(?:\([^)]*\))?\s*(.*)")
});
static KOREAN_CLOCK: LazyLock<Regex> =
    LazyLock::new(|| re(r"^(오전|오후)?\s*(\d{1,2})(?::(\d\d))?\s*(시)?\s*(?:(\d{1,2})\s*분)?"));
static DAY: LazyLock<Regex> = LazyLock::new(|| re(r"^\d{4}-\d\d-\d\d$"));
static CLOCK: LazyLock<Regex> = LazyLock::new(|| re(r"(\d{1,2}):(\d\d)"));
static REGISTRATION_END: LazyLock<Regex> = LazyLock::new(|| re(r"참가\s*(?:신청|접수)\s*마감"));
static SUBMISSION_END: LazyLock<Regex> = LazyLock::new(|| re(r"리더보드\s*제출\s*마감"));

pub fn url_key(url: &str) -> Option<String> {
    let parsed = common::Url::parse(url.trim());
    let host = parsed.netloc.to_lowercase();
    if host != "dacon.io" && host != "www.dacon.io" {
        return None;
    }
    URL_KEY.captures(parsed.path).map(|c| format!("{NAME}:{}", &c[1]))
}

/// Turn the JS object literal Nuxt emits (bare keys, function-argument names as values) into JSON.
fn js_to_json(source: &str, env: &Map<String, Value>) -> Result<Value> {
    let mut tokens = Vec::new();
    let mut pos = 0;
    while pos < source.len() {
        let Some(token) = TOKEN.find(&source[pos..]) else {
            let shown: String = source[pos..].chars().take(40).collect();
            return Err(Error(format!("데이콘 __NUXT__ 해석 실패: {shown:?}")));
        };
        let token = token.as_str();
        if !token.trim().is_empty() {
            tokens.push(token);
        }
        pos += token.len();
    }
    let mut out = String::with_capacity(source.len() * 2);
    for (i, token) in tokens.iter().enumerate() {
        let first = token.as_bytes()[0];
        if first.is_ascii_alphabetic() || first == b'_' || first == b'$' {
            let is_key = tokens.get(i + 1) == Some(&":") && (i == 0 || matches!(tokens[i - 1], "{" | ","));
            if is_key {
                out.push_str(&serde_json::to_string(token)?);
            } else if matches!(*token, "true" | "false" | "null") {
                out.push_str(token);
            } else if let Some(value) = env.get(*token) {
                out.push_str(&serde_json::to_string(value)?);
            } else {
                return Err(Error(format!("데이콘 __NUXT__ 모르는 이름: {token}")));
            }
        } else {
            out.push_str(token);
        }
    }
    Ok(serde_json::from_str(&out)?)
}

/// `YYYY-MM-DD HH:MM[:SS]` → KST ISO in minutes.
fn kst(value: Option<&str>) -> Option<String> {
    let c = KST_TIME.captures(value?)?;
    Some(format!("{}-{}-{}T{}:{}+09:00", &c[1], &c[2], &c[3], &c[4], &c[5]))
}

fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// Every official competition card on the list page (past ones included); `deadline` is the contest end.
pub fn parse_list(page_html: &str) -> Result<Vec<Item>> {
    let Some(nuxt) = NUXT.captures(page_html) else {
        return Err(Error::msg("데이콘 목록 데이터(window.__NUXT__)를 찾지 못했다"));
    };
    let names: Vec<&str> = nuxt[1].split(',').collect();
    let args: Vec<Value> = serde_json::from_str(&format!("[{}]", VOID.replace_all(&nuxt[3], "null")))?;
    if names.len() != args.len() {
        return Err(Error::msg("데이콘 __NUXT__ 인자 수가 맞지 않는다"));
    }
    let env: Map<String, Value> = names.into_iter().map(String::from).zip(args).collect();
    let data = js_to_json(&nuxt[2], &env)?;
    let cards = data["data"]
        .as_array()
        .and_then(|list| list.iter().find_map(|d| d.as_object().and_then(|d| d.get("compData"))))
        .ok_or_else(|| Error::msg("데이콘 __NUXT__에 compData가 없다"))?;
    let mut rows = Vec::new();
    for card in cards.as_array().map(Vec::as_slice).unwrap_or_default() {
        let cpt_id = common::js(card, "cpt_id");
        let prize = common::js(card, "prize_info");
        rows.push(Item {
            entry_id: format!("{NAME}-{cpt_id}"),
            title: card["name"].as_str().unwrap_or("").trim().to_string(),
            url: format!("https://dacon.io/competitions/official/{cpt_id}/overview"),
            organizer: common::js(card, "sponsor").trim().to_string(),
            deadline: kst(str_field(card, "period_end")),
            categories: common::js(card, "keyword")
                .split('|')
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(String::from)
                .collect(),
            note: Some(if prize.is_empty() { String::new() } else { format!("상금: {prize}") }),
            alt_url: None,
        });
    }
    Ok(rows)
}

/// Date and time at the start of `2026년 09월 29일(화) 10:00`, `2026.08.17 (월) 오후 11시 59분`, `2026년 7월 5일(일)`.
fn korean_datetime(fragment: &str) -> Option<String> {
    let date = KOREAN_DATE.captures(fragment)?;
    let (year, month, day): (i32, u32, u32) = (date[1].parse().ok()?, date[2].parse().ok()?, date[3].parse().ok()?);
    if let Some(clock) = KOREAN_CLOCK.captures(&date[4])
        && (clock.get(3).is_some() || clock.get(4).is_some())
    {
        let mut hour: u32 = clock[2].parse().ok()?;
        let minute: u32 = clock.get(3).or(clock.get(5)).map_or(Some(0), |m| m.as_str().parse().ok())?;
        if clock.get(1).map(|m| m.as_str()) == Some("오후") && hour < 12 {
            hour += 12;
        }
        if hour < 24 && minute < 60 {
            return Some(format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}+09:00"));
        }
    }
    Some(text::end_of_day(year, month, day))
}

/// Deadline candidates from the detail API.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detail {
    pub registration_end: Option<String>,
    pub submission_end: Option<String>,
    pub api_submission_end: Option<String>,
    pub period_end: Option<String>,
    pub merge_deadline: Option<String>,
    pub prize: String,
    pub sponsor: String,
}

/// `(label, rest)` pairs: each label's text runs up to the next ` - ` item separator or the end.
fn schedule_items(schedule: &str) -> Vec<(&str, &str)> {
    let mut found = Vec::new();
    let mut pos = 0;
    while let Some(caps) = SCHEDULE_LABEL.captures_at(schedule, pos) {
        let start = caps.get(0).unwrap().end();
        let end = ITEM_SEPARATOR.find_at(schedule, start).map_or(schedule.len(), |m| m.start());
        found.push((caps.get(1).unwrap().as_str(), &schedule[start..end]));
        pos = end;
    }
    found
}

pub fn parse_detail(payload: &Value) -> Result<Detail> {
    let Some(cpt) = payload["data"].as_array().and_then(|d| d.first()) else {
        return Err(Error::msg("데이콘 상세 API 응답에 data가 없다"));
    };
    let schedule = common::text(&BLOCK_TAG.replace_all(str_field(cpt, "schedule").unwrap_or(""), " - "));
    let (mut registration, mut submission) = (None, None);
    for (label, rest) in schedule_items(&schedule) {
        if label.contains("기간") {
            if let Some((_, end)) = rest.split_once('~')
                && registration.is_none()
            {
                registration = korean_datetime(end);
            }
        } else if label.contains("리더보드") {
            submission = submission.or_else(|| korean_datetime(rest));
        } else {
            registration = registration.or_else(|| korean_datetime(rest));
        }
    }
    let timeline = str_field(cpt, "time_line_data")
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or(Value::Null);
    for point in timeline.as_array().map(Vec::as_slice).unwrap_or_default() {
        let Some(point) = point.as_object() else { continue };
        for (day, label) in point {
            let Some(label) = label.as_str() else { continue };
            if !DAY.is_match(day) {
                continue;
            }
            let value = match CLOCK.captures(label) {
                Some(c) => format!("{day}T{:02}:{}+09:00", c[1].parse::<u32>().unwrap_or(0), &c[2]),
                None => {
                    let mut p = day.split('-').map(|n| n.parse::<u32>().unwrap_or(0));
                    let (y, m, d) = (p.next().unwrap_or(0), p.next().unwrap_or(0), p.next().unwrap_or(0));
                    text::end_of_day(y as i32, m, d)
                }
            };
            if REGISTRATION_END.is_match(label) {
                registration = registration.or(Some(value));
            } else if SUBMISSION_END.is_match(label) {
                submission = submission.or(Some(value));
            }
        }
    }
    let period_start = kst(str_field(cpt, "period_start"));
    let period_end = kst(str_field(cpt, "period_end"));
    let api_submission = kst(str_field(cpt, "submission_end"));
    let api_submission_end = match (&submission, &api_submission, &period_start, &period_end) {
        (None, Some(api), Some(start), Some(end))
            if start <= api && api <= end && !registration.as_ref().is_some_and(|r| api < r) =>
        {
            Some(api.clone())
        }
        _ => None,
    };
    Ok(Detail {
        registration_end: registration,
        submission_end: submission,
        api_submission_end,
        period_end,
        merge_deadline: kst(str_field(cpt, "merge_deadline")),
        prize: common::js(cpt, "prize_info").trim().to_string(),
        sponsor: common::js(cpt, "sponsor").trim().to_string(),
    })
}

/// The earliest registration/submission deadline and a note listing every candidate.
pub fn summarize(detail: &Detail) -> (Option<String>, String) {
    let deadline = [&detail.registration_end, &detail.submission_end, &detail.api_submission_end, &detail.period_end]
        .into_iter()
        .flatten()
        .min()
        .cloned();
    let labels = [
        ("참가 신청 마감", &detail.registration_end),
        ("리더보드 제출 마감", &detail.submission_end),
        ("제출 마감(API)", &detail.api_submission_end),
        ("대회 종료", &detail.period_end),
        ("팀 병합 마감", &detail.merge_deadline),
    ];
    let mut parts: Vec<String> = labels
        .iter()
        .filter_map(|(label, value)| {
            value.as_ref().map(|v| format!("{label}: {}", v.chars().take(16).collect::<String>().replace('T', " ")))
        })
        .collect();
    if !detail.prize.is_empty() {
        parts.push(format!("상금: {}", detail.prize));
    }
    (deadline, parts.join(" / "))
}

/// The single list page (DACON does not paginate) minus finished contests; only new ones open the detail API.
pub fn collect(_pages: u32, known: &HashSet<String>) -> Result<Vec<Item>> {
    let now = text::now();
    let rows = parse_list(&http::get(LIST_URL)?)?;
    if rows.is_empty() {
        return Err(Error::msg("데이콘 목록이 비었다"));
    }
    let mut items = Vec::new();
    for mut row in rows {
        if row.deadline.as_ref().is_some_and(|d| *d < now) {
            continue;
        }
        if !known.contains(&row.entry_id) {
            common::pause();
            let id = row.entry_id.split_once('-').map_or("", |(_, id)| id);
            let url = format!("https://newapi.dacon.io/competition?cpt_id={id}");
            let payload =
                http::Request { url: &url, headers: &[("Accept", "application/json")], body: None, agent: None }
                    .json()?;
            let detail = parse_detail(&payload)?;
            let (deadline, note) = summarize(&detail);
            row.deadline = deadline.or(row.deadline);
            row.note = Some(note);
            if row.organizer.is_empty() {
                row.organizer = detail.sponsor;
            }
        }
        if row.note.as_deref() == Some("") {
            row.note = None;
        }
        items.push(row);
    }
    Ok(items)
}
