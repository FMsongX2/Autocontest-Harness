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
//! Requests to the same site are at least one second apart; robots.txt disallowed paths are not read.

use std::collections::HashSet;

use serde::Serialize;

use crate::error::Result;

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
