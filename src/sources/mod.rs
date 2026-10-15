//! Contest listing sources. One site per module; each module exports:
//! - `NAME`: source name and registry id prefix (lowercase ASCII, no hyphen).
//! - `collect(pages, known)`: every contest visible on the first `pages` list pages (or the API
//!   equivalent). Ids already in `known` are returned from list data only (no detail fetch).
//!   An empty result is treated by the collector as "structure changed or nothing open".
//! - `url_key(url)`: `Some("<NAME>:<id>")` for this site's detail URLs (www/order/slash variants), else None.
//!
//! Items carry `entry_id` (`"<NAME>-<site id>"`), `title`, `url` (detail page on the site),
//! `organizer`, `deadline` (KST `YYYY-MM-DDTHH:MM+09:00`, date-only → 23:59), `categories`,
//! optional `note` (eligibility · prize, joined by " / ") and `alt_url` (official notice).
//! Titles and organizers are single-line (whitespace collapsed).
//! Requests to the same site are at least one second apart; robots.txt disallowed paths are not read.
//! A failed detail fetch or parse keeps the list row (see [`with_detail`]); only list failures fail a source.

use std::collections::HashSet;

use serde::Serialize;

use crate::error::Result;

/// Prefix of the note added to a row whose detail page could not be read.
pub const DETAIL_FAILED: &str = "상세 실패";

pub mod allcon;
pub mod allforyoung;
pub mod campuspick;
pub(crate) mod common;
pub mod contestkorea;
pub mod dacon;
pub mod detizen;
pub mod devpost;
pub mod kaggle;
pub mod linkareer;
pub mod thinkcontest;
pub mod wevity;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Item {
    pub entry_id: String,
    pub title: String,
    pub url: String,
    pub organizer: String,
    pub deadline: Option<String>,
    pub categories: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_url: Option<String>,
}

pub type CollectFn = fn(u32, &HashSet<String>) -> Result<Vec<Item>>;
pub type UrlKeyFn = fn(&str) -> Option<String>;

pub struct Source {
    pub name: &'static str,
    pub collect: CollectFn,
    pub url_key: UrlKeyFn,
}

macro_rules! sources {
    ($($module:ident),* $(,)?) => {
        &[$(Source { name: $module::NAME, collect: $module::collect, url_key: $module::url_key }),*]
    };
}

/// Korean aggregators first, then data/AI competition platforms, then international.
/// Not included: campusmon (robots.txt `Disallow: /`), AIConnect (shut down 2026-07-31).
pub static SOURCES: &[Source] = sources![
    linkareer,
    wevity,
    thinkcontest,
    allcon,
    contestkorea,
    detizen,
    campuspick,
    allforyoung,
    dacon,
    devpost,
    kaggle,
];

pub fn find(name: &str) -> Option<&'static Source> {
    SOURCES.iter().find(|s| s.name == name)
}

/// The first source that recognises `url` as one of its detail pages.
pub fn url_key(url: &str) -> Option<String> {
    SOURCES.iter().find_map(|s| (s.url_key)(url))
}

/// Fill a new list row from its detail page with `apply`. A failed fetch or `apply` keeps the row as
/// the list gave it (deadline from the list or `None`) and appends `상세 실패: <err>` to its note, so
/// the gate re-checks it instead of the whole source being dropped. `apply` must fail before it
/// changes the row.
pub fn with_detail<T>(row: &mut Item, fetched: Result<T>, apply: impl FnOnce(&mut Item, T) -> Result<()>) {
    if let Err(err) = fetched.and_then(|detail| apply(row, detail)) {
        let failed = format!("{DETAIL_FAILED}: {err}");
        row.note = Some(match row.note.take().filter(|note| !note.is_empty()) {
            Some(note) => format!("{note} / {failed}"),
            None => failed,
        });
    }
}
