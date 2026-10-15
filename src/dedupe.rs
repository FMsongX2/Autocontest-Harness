//! Same-contest detection by normalised title, URL key, and organizer + deadline in folder docs.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use crate::config::Config;
use crate::ledger::Ledger;
use crate::registry::Registry;
use crate::sources;

const SIMILAR: f64 = 0.82;
const STRICT_SIMILAR: f64 = 0.9;
const STRICT_MIN_LEN: usize = 8;

static LEADING_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:[\[\(【][^\]\)】]{1,30}[\]\)】]\s*)+").expect("regex"));
static NOISE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"20\d\d년?|제\s*\d+\s*회|\d+기|공모전|경진대회|대회|챌린지|모집|안내|참가자").expect("regex")
});
static NON_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^0-9a-z가-힣]").expect("regex"));
static EXISTING_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^-\s*([^:]+):\s*(.+)$").expect("regex"));
static DEADLINE_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(20\d\d)-(\d\d)-(\d\d)").expect("regex"));
static ORG_NOISE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(주\)|주식회사|\s").expect("regex"));
static SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s").expect("regex"));
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+(?:,\d{3})*").expect("regex"));

pub struct Query<'a> {
    pub title: &'a str,
    pub url: &'a str,
    pub include_folders: bool,
    pub exclude_id: Option<&'a str>,
    pub strict: bool,
    pub organizer: &'a str,
    pub deadline: Option<&'a str>,
    /// The submission ledger. Its matches (`kind: submitted`) come before all others.
    pub submitted: Option<&'a Ledger>,
}

/// Core of a title: leading `[organizer]` tags, years, edition numbers, common words and symbols removed.
pub fn normalize_title(title: &str) -> String {
    let lower = title.to_lowercase();
    let text = LEADING_TAG.replace(&lower, "");
    let text = NOISE.replace_all(&text, "");
    NON_WORD.replace_all(&text, "").into_owned()
}

/// Number tokens left in a title after the years and edition numbers that [`normalize_title`] drops
/// (`ARC-AGI-2` → `2`, `Track 1` → `1`), without thousands separators or leading zeros.
fn numbers(title: &str) -> HashSet<String> {
    let lower = title.to_lowercase();
    let text = LEADING_TAG.replace(&lower, "");
    let text = NOISE.replace_all(&text, " ");
    NUMBER
        .find_iter(&text)
        .map(|m| {
            let digits = m.as_str().replace(',', "");
            let digits = digits.trim_start_matches('0');
            if digits.is_empty() { "0".to_string() } else { digits.to_string() }
        })
        .collect()
}

/// A source's detail URL becomes its key (`wevity:111674`); other URLs reduce to host + path + query.
pub fn normalize_url(url: &str) -> String {
    if url.is_empty() {
        return String::new();
    }
    if let Some(key) = sources::url_key(url) {
        return key;
    }
    let parts = url_parts(url.trim());
    let host = parts.netloc.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let query = if parts.query.is_empty() { String::new() } else { format!("?{}", parts.query) };
    format!("{host}{}{query}", parts.path.trim_end_matches('/'))
}

struct UrlParts<'a> {
    netloc: &'a str,
    path: &'a str,
    query: &'a str,
}

/// The pieces of Python's `urlparse` that `normalize_url` uses (params are split off the path).
fn url_parts(url: &str) -> UrlParts<'_> {
    let mut scheme = "";
    let mut rest = url;
    if let Some(colon) = url.find(':') {
        let candidate = &url[..colon];
        let valid = candidate.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && candidate.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
        if valid {
            scheme = candidate;
            rest = &url[colon + 1..];
        }
    }
    let mut netloc = "";
    if let Some(after) = rest.strip_prefix("//") {
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        netloc = &after[..end];
        rest = &after[end..];
    }
    if let Some(hash) = rest.find('#') {
        rest = &rest[..hash];
    }
    let (mut path, query) = match rest.find('?') {
        Some(q) => (&rest[..q], &rest[q + 1..]),
        None => (rest, ""),
    };
    let uses_params =
        ["", "http", "https", "ftp", "hdl", "prospero", "rtsp", "rtspu", "sip", "sips", "mms", "sftp", "tel"]
            .contains(&scheme.to_ascii_lowercase().as_str());
    if uses_params && path.contains(';') {
        let cut = match path.rfind('/') {
            Some(slash) => path[slash..].find(';').map(|i| slash + i),
            None => path.find(';'),
        };
        if let Some(cut) = cut {
            path = &path[..cut];
        }
    }
    UrlParts { netloc, path, query }
}

/// `strict` is for automatic merging while collecting: only long, nearly identical cores match.
/// Titles where each side has a number the other lacks (`ARC-AGI-2` vs `ARC-AGI-3`, `시즌2` vs
/// `시즌3`) are different contests; a number on only one side (a prize amount) does not decide.
pub fn similar(a: &str, b: &str, strict: bool) -> bool {
    let (na, nb) = (normalize_title(a), normalize_title(b));
    if na.is_empty() || nb.is_empty() {
        return false;
    }
    let (numbers_a, numbers_b) = (numbers(a), numbers(b));
    if !numbers_a.is_subset(&numbers_b) && !numbers_b.is_subset(&numbers_a) {
        return false;
    }
    let (ca, cb): (Vec<char>, Vec<char>) = (na.chars().collect(), nb.chars().collect());
    let (short, long) = if cb.len() < ca.len() { (&nb, &na) } else { (&na, &nb) };
    let short_len = ca.len().min(cb.len());
    if strict {
        return short_len >= STRICT_MIN_LEN && ratio(&ca, &cb) >= STRICT_SIMILAR;
    }
    if short_len >= 6 && long.contains(short.as_str()) {
        return true;
    }
    ratio(&ca, &cb) >= SIMILAR
}

/// `difflib.SequenceMatcher(None, a, b).ratio()`, including the autojunk rule for long `b`.
pub fn ratio(a: &[char], b: &[char]) -> f64 {
    let total = a.len() + b.len();
    if total == 0 {
        return 1.0;
    }
    let mut b2j: HashMap<char, Vec<usize>> = HashMap::new();
    for (j, &ch) in b.iter().enumerate() {
        b2j.entry(ch).or_default().push(j);
    }
    if b.len() >= 200 {
        let ntest = b.len() / 100 + 1;
        b2j.retain(|_, idx| idx.len() <= ntest);
    }
    let longest = |alo: usize, ahi: usize, blo: usize, bhi: usize| -> (usize, usize, usize) {
        let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0);
        let mut j2len: HashMap<usize, usize> = HashMap::new();
        for (i, ch) in a.iter().enumerate().take(ahi).skip(alo) {
            let mut next: HashMap<usize, usize> = HashMap::new();
            if let Some(indices) = b2j.get(ch) {
                for &j in indices {
                    if j < blo {
                        continue;
                    }
                    if j >= bhi {
                        break;
                    }
                    let k = j.checked_sub(1).and_then(|p| j2len.get(&p)).copied().unwrap_or(0) + 1;
                    next.insert(j, k);
                    if k > bestsize {
                        (besti, bestj, bestsize) = (i + 1 - k, j + 1 - k, k);
                    }
                }
            }
            j2len = next;
        }
        while besti > alo && bestj > blo && a[besti - 1] == b[bestj - 1] {
            besti -= 1;
            bestj -= 1;
            bestsize += 1;
        }
        while besti + bestsize < ahi && bestj + bestsize < bhi && a[besti + bestsize] == b[bestj + bestsize] {
            bestsize += 1;
        }
        (besti, bestj, bestsize)
    };
    let mut matched = 0;
    let mut queue = vec![(0, a.len(), 0, b.len())];
    while let Some((alo, ahi, blo, bhi)) = queue.pop() {
        let (i, j, k) = longest(alo, ahi, blo, bhi);
        if k > 0 {
            matched += k;
            if alo < i && blo < j {
                queue.push((alo, i, blo, j));
            }
            if i + k < ahi && j + k < bhi {
                queue.push((i + k, ahi, j + k, bhi));
            }
        }
    }
    2.0 * matched as f64 / total as f64
}

/// Python `str.splitlines()`: every Unicode line boundary, `\r\n` counted once.
fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        if matches!(ch, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}') {
            lines.push(&text[start..i]);
            let mut end = i + ch.len_utf8();
            if ch == '\r' && chars.peek().is_some_and(|&(_, c)| c == '\n') {
                chars.next();
                end += 1;
            }
            start = end;
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Read a file as text, dropping undecodable bytes like Python's `errors="ignore"`.
fn read_lossy(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => String::from_utf8_lossy(err.as_bytes()).replace('\u{FFFD}', ""),
    })
}

/// Sub-folders of the contests root, sorted by name.
fn folders(root: &Path) -> Vec<(String, std::path::PathBuf)> {
    let Ok(dir) = std::fs::read_dir(root) else { return Vec::new() };
    let mut out: Vec<(String, std::path::PathBuf)> = dir
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    out.sort();
    out
}

/// `(where, title)`: the first heading of each folder README and each `existing_contests.md` line.
fn folder_titles(config: &Config) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for (name, path) in folders(&config.contests_root()) {
        let readme = path.join("README.md");
        if !readme.exists() {
            continue;
        }
        let Some(text) = read_lossy(&readme) else { continue };
        if let Some(line) = split_lines(&text).into_iter().take(15).find(|l| l.starts_with('#')) {
            found.push((format!("folder:{name}"), line.trim_start_matches(['#', ' ']).trim().to_string()));
        }
    }
    if let Some(text) = read_lossy(&config.existing_contests()) {
        for line in split_lines(&text) {
            if let Some(caps) = EXISTING_LINE.captures(line) {
                found.push((format!("existing:{}", caps[1].trim()), caps[2].trim().to_string()));
            }
        }
    }
    found
}

/// Common Korean spellings of a deadline date (11/4, 11.4, 11월 4일, 2026-11-04, …).
fn date_patterns(deadline: Option<&str>) -> Vec<String> {
    let Some(caps) = DEADLINE_DATE.captures(deadline.unwrap_or("")) else { return Vec::new() };
    let y = &caps[1];
    let (Ok(m), Ok(d)) = (caps[2].parse::<u32>(), caps[3].parse::<u32>()) else { return Vec::new() };
    vec![
        format!("{m}/{d}"),
        format!("{m}.{d}"),
        format!("{m}월 {d}일"),
        format!("{m}월{d}일"),
        format!("{y}-{m:02}-{d:02}"),
        format!("{y}.{m:02}.{d:02}"),
        format!("{y}. {m}. {d}"),
    ]
}

fn organizer_key(organizer: &str) -> String {
    ORG_NOISE.replace_all(organizer, "").into_owned()
}

/// `(folder, first lines of README.md and CONTEST.md)` for every visible contest folder.
fn folder_documents(config: &Config) -> Vec<(String, String)> {
    let mut docs = Vec::new();
    for (name, path) in folders(&config.contests_root()) {
        if name.starts_with('.') {
            continue;
        }
        let mut parts: Vec<String> = Vec::new();
        for (file, limit) in [("README.md", 80), ("CONTEST.md", 40)] {
            if let Some(text) = read_lossy(&path.join(file)) {
                parts.extend(split_lines(&text).into_iter().take(limit).map(str::to_string));
            }
        }
        if !parts.is_empty() {
            docs.push((name, parts.join("\n")));
        }
    }
    docs
}

/// Folders whose docs mention both the organizer and the deadline date, which catches renamed titles.
fn organizer_deadline_hits(config: &Config, organizer: &str, deadline: Option<&str>) -> Vec<String> {
    let org = organizer_key(organizer);
    let dates = date_patterns(deadline);
    if org.chars().count() < 3 || dates.is_empty() {
        return Vec::new();
    }
    folder_documents(config)
        .into_iter()
        .filter(|(_, text)| {
            SPACE.replace_all(text, "").contains(&org) && dates.iter().any(|d| text.contains(d.as_str()))
        })
        .map(|(name, _)| name)
        .collect()
}

/// Ledger items, registry entries and folders that look like the same contest. A URL match always
/// counts; ledger titles always match non-strictly. Seen from `exclude_id`, `duplicate` entries are
/// never representatives, and another `discovered`
/// entry counts only when it was collected first and this one is also still `discovered`, so two
/// listings gated at once mark only one duplicate and a representative is not blocked by its own copy.
pub fn find_matches(registry: &Registry, config: &Config, query: &Query<'_>) -> Vec<Value> {
    let key = normalize_url(query.url);
    let own = query.exclude_id.and_then(|id| registry.contests.get(id));
    let mut matches: Vec<Value> = query
        .submitted
        .map(|ledger| {
            ledger
                .matches(query.title, &[query.url], query.exclude_id)
                .into_iter()
                .map(|(s, by)| {
                    json!({"kind": "submitted", "id": s.key, "title": s.title, "submitted_at": s.submitted_at, "by": by})
                })
                .collect()
        })
        .unwrap_or_default();
    for entry in registry.contests.values() {
        if Some(entry.id.as_str()) == query.exclude_id {
            continue;
        }
        if let Some(own) = own {
            let skip = entry.state == "duplicate"
                || (entry.state == "discovered"
                    && (own.state != "discovered"
                        || (&own.discovered_at, &own.id) < (&entry.discovered_at, &entry.id)));
            if skip {
                continue;
            }
        }
        let by_url = !key.is_empty()
            && std::iter::once(&entry.url).chain(&entry.alt_urls).map(|u| normalize_url(u)).any(|u| u == key);
        let by = if by_url {
            "url"
        } else if similar(query.title, &entry.title, query.strict) {
            "title"
        } else {
            continue;
        };
        matches.push(json!({"kind": "registry", "id": entry.id, "title": entry.title, "state": entry.state, "by": by}));
    }
    if query.include_folders {
        for (place, other) in folder_titles(config) {
            if similar(query.title, &other, query.strict) {
                let (kind, name) = place.split_once(':').unwrap_or((&place, ""));
                matches.push(json!({"kind": kind, "id": name, "title": other, "by": "title"}));
            }
        }
        if !query.strict {
            let seen: HashSet<String> = matches.iter().filter_map(|m| m["id"].as_str().map(str::to_string)).collect();
            for name in organizer_deadline_hits(config, query.organizer, query.deadline) {
                if !seen.contains(&name) {
                    matches.push(json!({"kind": "folder", "id": name, "title": "", "by": "organizer+deadline"}));
                }
            }
        }
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_matches_difflib() {
        let r = |a: &str, b: &str| ratio(&a.chars().collect::<Vec<_>>(), &b.chars().collect::<Vec<_>>());
        assert_eq!(r("abcd", "bcde"), 0.75);
        assert!(
            (r("private Thread currentThread;", "private volatile Thread currentThread;") - 0.8656716417910447).abs()
                < 1e-12
        );
        assert_eq!(r("", ""), 1.0);
    }

    #[test]
    fn url_parts_split_like_urlparse() {
        let p = url_parts("https://Www.Example.com/a/b;x=1?q=1#frag");
        assert_eq!((p.netloc, p.path, p.query), ("Www.Example.com", "/a/b", "q=1"));
        assert_eq!(url_parts("http://e.com/a;b/c").path, "/a;b/c");
        assert_eq!(split_lines("a\r\nb\n\nc\u{2028}d\n"), vec!["a", "b", "", "c", "d"]);
    }
}
