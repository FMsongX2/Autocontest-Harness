//! The submission ledger (`data/submissions.json`, git-ignored): every contest already submitted,
//! from the harness, by hand, or imported from the registry. Dedupe and route read it so a submitted
//! contest is never assigned again. Contract: `docs/CONTRACT.md`.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
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
use crate::text::now;

pub const SOURCES: &[&str] = &["harness", "manual", "import"];

/// Phrases that mention 제출 without meaning it happened ("제출 준비", "미제출", …).
const NOT_SUBMITTED: &[&str] = &["미제출", "제출 준비", "제출 대기", "제출 패키지", "제출 전", "제출하지", "제출 안"];

static DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"20\d\d-\d\d-\d\d").expect("regex"));
static STAMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}\+09:00)?$").expect("regex"));
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ledger {
    pub version: u32,
    pub submissions: Vec<Submission>,
}

impl Default for Ledger {
    fn default() -> Self {
        Ledger { version: 1, submissions: Vec::new() }
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

    /// Items for the same contest: a URL key shared with `urls` (each item's `url` and `alt_urls`),
    /// otherwise a similar title (non-strict). `exclude_key` is left out. Returns `(item, "url"|"title")`.
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
                } else if !title.is_empty() && dedupe::similar(title, &s.title, false) {
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

/// The ledger item `harness submit` writes for a registry entry.
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

/// A contest submitted outside the harness (`harness submitted add`).
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
        Some(date) if !valid_stamp(date) => {
            bail!("--date는 YYYY-MM-DD 또는 YYYY-MM-DDTHH:MM+09:00 형식이어야 한다: {date}")
        }
        Some(date) => date.to_string(),
        None => now(),
    };
    let slug = NON_WORD.replace_all(&title.to_lowercase(), "-").trim_matches('-').chars().take(40).collect::<String>();
    Ok(Submission {
        key: format!("manual-{}", slug.trim_end_matches('-')),
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

/// `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM+09:00` with a real calendar date.
pub fn valid_stamp(text: &str) -> bool {
    STAMP.is_match(text) && chrono::NaiveDate::parse_from_str(&text[..10], "%Y-%m-%d").is_ok()
}

pub fn path() -> PathBuf {
    paths::submissions()
}

fn load(path: &Path) -> Result<Ledger> {
    if !path.exists() {
        return Ok(Ledger::default());
    }
    serde_json::from_str(&fs::read_to_string(path)?).map_err(|err| Error(format!("{}: {err}", path.display())))
}

fn save(path: &Path, ledger: &Ledger) -> Result<()> {
    let tmp = path.with_file_name(format!(".submissions-{}.json", std::process::id()));
    let mut file = File::create(&tmp)?;
    file.write_all(serde_json::to_string_pretty(ledger)?.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn lock_file(path: &Path) -> Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    Ok(OpenOptions::new().create(true).truncate(false).write(true).open(path.with_extension("lock"))?)
}

/// Read the ledger at `path` under a shared lock (an absent file is an empty ledger).
pub fn load_at(path: &Path) -> Result<Ledger> {
    let lock = lock_file(path)?;
    lock.lock_shared()?;
    load(path)
}

/// Modify the ledger at `path` under an exclusive lock; it is saved atomically only when `f` succeeds.
pub fn write_at<T>(path: &Path, f: impl FnOnce(&mut Ledger) -> Result<T>) -> Result<T> {
    let lock = lock_file(path)?;
    lock.lock()?;
    let mut ledger = load(path)?;
    let value = f(&mut ledger)?;
    save(path, &ledger)?;
    Ok(value)
}
