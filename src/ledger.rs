//! The submission ledger (`data/submissions.json`, git-ignored): every contest already submitted,
//! from the harness, by hand, or imported from the registry. Dedupe and route read it so a submitted
//! contest is never assigned again. Contract: `docs/CONTRACT.md`.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::bail;
use crate::dedupe;
use crate::error::{Error, Result};
use crate::paths;
use crate::registry::{Entry, Registry};
use crate::store;
use crate::text::{self, now};

pub const SOURCES: &[&str] = &["harness", "manual", "import"];

/// Phrases that mention 제출 without meaning it happened ("제출 준비", "미제출", …).
const NOT_SUBMITTED: &[&str] = &["미제출", "제출 준비", "제출 대기", "제출 패키지", "제출 전", "제출하지", "제출 안"];

static DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"20\d\d-\d\d-\d\d").expect("regex"));
static NON_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\W+").expect("regex"));

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Submission {
    /// The registry id, or `manual-<slug>` for contests submitted outside the harness.
    pub key: String,
    pub id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub organizer: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub alt_urls: Vec<String>,
    pub folder: Option<String>,
    /// KST `YYYY-MM-DDTHH:MM+09:00` or `YYYY-MM-DD`.
    pub submitted_at: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub receipt: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub note: String,
    /// `harness`, `manual` or `import`.
    pub source: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The newest ledger format this build reads and writes.
pub const VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ledger {
    pub version: u32,
    pub submissions: Vec<Submission>,
    /// Top-level fields this version does not know about are kept as they are.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Ledger {
    fn default() -> Self {
        Ledger { version: VERSION, submissions: Vec::new(), extra: Map::new() }
    }
}

/// What `import` did: the items it added and the keys it skipped because they were already there.
#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    pub added: Vec<Submission>,
    pub skipped: Vec<String>,
}

impl Ledger {
    pub fn get(&self, key: &str) -> Option<&Submission> {
        self.submissions.iter().find(|s| s.key == key)
    }

    /// Add `item`, or replace the item with the same key. Returns `true` when it was added.
    pub fn upsert(&mut self, item: Submission) -> bool {
        match self.submissions.iter_mut().find(|s| s.key == item.key) {
            Some(existing) => {
                *existing = item;
                false
            }
            None => {
                self.submissions.push(item);
                true
            }
        }
    }

    /// Add a manual item without ever replacing a different submission: an item under the same
    /// key is replaced only when it has the same title and the same date (a correction of that
    /// record); otherwise the key gets the first free suffix `-2`, `-3`, … Returns `true` when it
    /// was added, with `item.key` set to the key used.
    pub fn add_manual(&mut self, item: &mut Submission) -> bool {
        let base = item.key.clone();
        let mut n = 1;
        loop {
            let key = if n == 1 { base.clone() } else { format!("{base}-{n}") };
            match self.submissions.iter().position(|s| s.key == key) {
                None => {
                    item.key = key;
                    self.submissions.push(item.clone());
                    return true;
                }
                Some(i) if same_record(&self.submissions[i], item) => {
                    item.key = key;
                    self.submissions[i] = item.clone();
                    return false;
                }
                Some(_) => n += 1,
            }
        }
    }

    /// Items for the same contest: a URL key shared with `urls` (each item's `url` and `alt_urls`),
    /// otherwise a similar title (non-strict) of the same edition: when both titles carry a year or
    /// an edition number (`제5회`, `5th`), those must agree, so next year's contest is not taken
    /// for the one already submitted. `exclude_key` is left out. Returns `(item, "url"|"title")`.
    pub fn matches<'a>(
        &'a self,
        title: &str,
        urls: &[&str],
        exclude_key: Option<&str>,
    ) -> Vec<(&'a Submission, &'static str)> {
        let keys: Vec<String> = urls.iter().map(|u| dedupe::normalize_url(u)).filter(|k| !k.is_empty()).collect();
        self.submissions
            .iter()
            .filter(|s| Some(s.key.as_str()) != exclude_key)
            .filter_map(|s| {
                let by_url = std::iter::once(&s.url)
                    .chain(&s.alt_urls)
                    .map(|u| dedupe::normalize_url(u))
                    .any(|u| !u.is_empty() && keys.contains(&u));
                if by_url {
                    Some((s, "url"))
                } else if !title.is_empty() && dedupe::similar(title, &s.title, false) && same_edition(title, &s.title)
                {
                    Some((s, "title"))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Copy submitted contests out of the registry once: entries with `submissions` or in state
    /// `submitted`, and `closed` entries whose title or a history note says 제출 happened.
    /// Keys already in the ledger are skipped.
    pub fn import(&mut self, registry: &Registry) -> ImportReport {
        let mut report = ImportReport::default();
        for entry in registry.contests.values() {
            let Some(item) = from_registry(entry) else { continue };
            if self.get(&item.key).is_some() {
                report.skipped.push(item.key);
                continue;
            }
            self.submissions.push(item.clone());
            report.added.push(item);
        }
        report
    }
}

/// `text` says the contest was submitted (`제출`, but not `미제출`, `제출 준비`, …).
pub fn says_submitted(text: &str) -> bool {
    text.contains("제출") && !NOT_SUBMITTED.iter().any(|n| text.contains(n))
}

/// The ledger item for a registry entry the import rule selects, or `None`.
pub fn from_registry(entry: &Entry) -> Option<Submission> {
    let last = entry.submissions.last();
    let submitted_hist = entry.history.iter().rev().find(|h| h.to == "submitted");
    let closed_note = if entry.state == "closed" {
        if says_submitted(&entry.title) {
            Some(String::new())
        } else {
            entry.history.iter().rev().find(|h| says_submitted(&h.note)).map(|h| h.note.clone())
        }
    } else {
        None
    };
    if last.is_none() && entry.state != "submitted" && closed_note.is_none() {
        return None;
    }
    let field = |key: &str| last.and_then(|s| s.get(key)).and_then(Value::as_str).unwrap_or("").to_string();
    let submitted_at = Some(field("at"))
        .filter(|a| !a.is_empty())
        .or_else(|| submitted_hist.map(|h| h.at.clone()))
        .or_else(|| {
            let folder = entry.folder.as_deref().unwrap_or("");
            let note = closed_note.as_deref().unwrap_or("");
            [entry.id.as_str(), folder, entry.title.as_str(), note]
                .iter()
                .find_map(|text| DATE.find(text).map(|m| m.as_str().to_string()))
        })
        .or_else(|| entry.history.last().map(|h| h.at.chars().take(10).collect()))
        .unwrap_or_else(|| now()[..10].to_string());
    let files = last
        .and_then(|s| s.get("files"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|f| f.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let note = match (&closed_note, submitted_hist) {
        (Some(note), _) => note.clone(),
        (None, Some(h)) => h.note.clone(),
        (None, None) => String::new(),
    };
    Some(Submission {
        key: entry.id.clone(),
        id: Some(entry.id.clone()),
        title: entry.title.clone(),
        organizer: entry.organizer.clone(),
        url: entry.url.clone(),
        alt_urls: entry.alt_urls.clone(),
        folder: entry.folder.clone(),
        submitted_at,
        method: field("method"),
        receipt: field("receipt"),
        files,
        note,
        source: "import".to_string(),
        extra: Map::new(),
    })
}

/// The ledger item `ach submit` writes for a registry entry.
pub fn from_submit(entry: &Entry, method: &str, receipt: &str, files: &[String], note: &str) -> Submission {
    Submission {
        key: entry.id.clone(),
        id: Some(entry.id.clone()),
        title: entry.title.clone(),
        organizer: entry.organizer.clone(),
        url: entry.url.clone(),
        alt_urls: entry.alt_urls.clone(),
        folder: entry.folder.clone(),
        submitted_at: now(),
        method: method.to_string(),
        receipt: receipt.to_string(),
        files: files.to_vec(),
        note: note.to_string(),
        source: "harness".to_string(),
        extra: Map::new(),
    }
}

/// A contest submitted outside the harness (`ach submitted add`).
#[derive(Default)]
pub struct Manual<'a> {
    pub title: &'a str,
    pub url: &'a str,
    pub organizer: &'a str,
    pub date: Option<&'a str>,
    pub method: &'a str,
    pub receipt: &'a str,
    pub folder: Option<&'a str>,
    pub note: &'a str,
}

pub fn manual(spec: Manual<'_>) -> Result<Submission> {
    let title = spec.title.trim();
    if title.is_empty() {
        bail!("--title이 비어 있다");
    }
    let submitted_at = match spec.date.map(str::trim) {
        Some(date) => match text::normalize_stamp(date) {
            Some(stamp) => stamp,
            None => bail!("--date는 YYYY-MM-DD 또는 YYYY-MM-DDTHH:MM+09:00 형식의 실제 날짜여야 한다: {date}"),
        },
        None => now(),
    };
    let slug = NON_WORD.replace_all(&title.to_lowercase(), "-").trim_matches('-').chars().take(40).collect::<String>();
    let slug = slug.trim_end_matches('-');
    // A title without letters or digits (`!!!`) still needs a key of its own.
    let slug = if slug.is_empty() { format!("{:08x}", short_hash(title)) } else { slug.to_string() };
    Ok(Submission {
        key: format!("manual-{slug}"),
        id: None,
        title: title.to_string(),
        organizer: spec.organizer.trim().to_string(),
        url: spec.url.trim().to_string(),
        alt_urls: Vec::new(),
        folder: spec.folder.map(str::to_string),
        submitted_at,
        method: spec.method.to_string(),
        receipt: spec.receipt.to_string(),
        files: Vec::new(),
        note: spec.note.to_string(),
        source: "manual".to_string(),
        extra: Map::new(),
    })
}

/// `YYYY-MM-DD` or a real date and time (see [`text::normalize_stamp`]).
pub fn valid_stamp(text: &str) -> bool {
    text::normalize_stamp(text).is_some()
}

/// FNV-1a, folded to 32 bits: a stable short key part.
fn short_hash(text: &str) -> u32 {
    let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    (hash ^ (hash >> 32)) as u32
}

/// The same submission recorded again: same title and same calendar date.
fn same_record(a: &Submission, b: &Submission) -> bool {
    a.title.trim() == b.title.trim() && a.submitted_at.get(..10) == b.submitted_at.get(..10)
}

static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[^0-9])(20\d\d)(?:[^0-9]|$)").expect("regex"));
static EDITION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)제\s*(\d+)\s*회|(\d+)\s*(?:회|기|차)(?:\s|$|[^0-9a-z가-힣])|(\d+)(?:st|nd|rd|th)\b")
        .expect("regex")
});

/// Years and edition numbers in a title.
fn edition_numbers(title: &str) -> (Vec<String>, Vec<String>) {
    let mut years: Vec<String> = YEAR.captures_iter(title).map(|c| c[1].to_string()).collect();
    let mut editions: Vec<String> = EDITION
        .captures_iter(title)
        .filter_map(|c| c.iter().skip(1).flatten().next().map(|m| m.as_str().trim_start_matches('0').to_string()))
        .collect();
    years.sort_unstable();
    years.dedup();
    editions.sort_unstable();
    editions.dedup();
    (years, editions)
}

/// Two titles name the same edition: years agree when both have one, edition numbers likewise.
pub fn same_edition(a: &str, b: &str) -> bool {
    let ((years_a, ed_a), (years_b, ed_b)) = (edition_numbers(a), edition_numbers(b));
    let agree = |x: &[String], y: &[String]| x.is_empty() || y.is_empty() || x == y;
    agree(&years_a, &years_b) && agree(&ed_a, &ed_b)
}

pub fn path() -> PathBuf {
    paths::submissions()
}

fn load(path: &Path) -> Result<Ledger> {
    let Some(text) = store::read(path)? else { return Ok(Ledger::default()) };
    store::check_version(path, &text, VERSION)?;
    serde_json::from_str(&text).map_err(|err| Error(format!("{}: {err}", path.display())))
}

fn save(path: &Path, ledger: &Ledger) -> Result<()> {
    let mut text = serde_json::to_string_pretty(ledger)?;
    text.push('\n');
    store::write_atomic(path, text.as_bytes(), true)
}

/// Read the ledger at `path` under a shared lock (an absent file is an empty ledger).
pub fn load_at(path: &Path) -> Result<Ledger> {
    // Reading never creates the data directory; without it the ledger is empty.
    let Some(lock) = store::lock_file(path, false)? else { return Ok(Ledger::default()) };
    lock.lock_shared()?;
    load(path)
}

/// Modify the ledger at `path` under an exclusive lock; it is saved atomically only when `f` succeeds.
pub fn write_at<T>(path: &Path, f: impl FnOnce(&mut Ledger) -> Result<T>) -> Result<T> {
    let Some(lock) = store::lock_file(path, true)? else {
        bail!("{}: 잠금 파일을 만들 수 없다", path.display())
    };
    lock.lock()?;
    let mut ledger = load(path)?;
    let value = f(&mut ledger)?;
    save(path, &ledger)?;
    Ok(value)
}
