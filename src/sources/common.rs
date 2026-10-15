//! Helpers shared by the source modules, matching the Python `urllib.parse`/`html` behaviour the
//! parsers were written against.

use std::sync::LazyLock;
use std::thread::sleep;
use std::time::Duration;

use regex::Regex;
use serde_json::Value;

use crate::text::{collapse, unescape};

/// Minimum gap between two requests to the same site.
pub(crate) const DELAY: Duration = Duration::from_secs(1);

pub(crate) fn pause() {
    sleep(DELAY);
}

/// Tags become spaces, entities are decoded, whitespace collapses (Python `common.text`).
pub(crate) fn text(fragment: &str) -> String {
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
    collapse(&unescape(&TAG.replace_all(fragment, " ")))
}

/// A JSON title or organizer as one line: whitespace runs (newlines included) collapse to one
/// space and other control characters are dropped.
pub(crate) fn line(value: &str) -> String {
    collapse(&value.replace(|c: char| c.is_control() && !c.is_whitespace(), ""))
}

/// `urllib.parse.urlparse` reduced to the parts the sources look at.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Url<'a> {
    pub scheme: &'a str,
    pub netloc: &'a str,
    pub path: &'a str,
    pub query: &'a str,
}

impl<'a> Url<'a> {
    pub(crate) fn parse(url: &'a str) -> Self {
        let mut rest = url.trim();
        if let Some(hash) = rest.find('#') {
            rest = &rest[..hash];
        }
        let mut scheme = "";
        if let Some(colon) = rest.find(':') {
            let candidate = &rest[..colon];
            let valid = candidate.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && candidate.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
            if valid {
                scheme = candidate;
                rest = &rest[colon + 1..];
            }
        }
        let mut netloc = "";
        if let Some(after) = rest.strip_prefix("//") {
            let end = after.find(['/', '?']).unwrap_or(after.len());
            netloc = &after[..end];
            rest = &after[end..];
        }
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        Url { scheme, netloc, path, query }
    }

    /// Lowercased host without port (`netloc.lower().split(":")[0]`).
    pub(crate) fn host(&self) -> String {
        self.netloc.split(':').next().unwrap_or("").to_lowercase()
    }

    /// `parse_qs(query).get(key, [""])[0]`: the first non-blank value of `key`.
    pub(crate) fn param(&self, key: &str) -> String {
        query_first(self.query, key)
    }
}

pub(crate) fn query_first(query: &str, key: &str) -> String {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .filter(|(_, value)| !value.is_empty())
        .find(|(name, _)| form_decode(name) == key)
        .map(|(_, value)| form_decode(value))
        .unwrap_or_default()
}

fn form_decode(value: &str) -> String {
    unquote(&value.replace('+', " "))
}

/// Percent-decode as UTF-8, replacing invalid sequences (`urllib.parse.unquote`).
pub(crate) fn unquote(value: &str) -> String {
    if !value.contains('%') {
        return value.to_string();
    }
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    (byte as char).to_digit(16).map(|d| d as u8)
}

/// Compile a pattern known to be valid (used inside `LazyLock`).
pub(crate) fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid pattern")
}

/// `" / ".join(f"{k}: {v}" for k, v in parts if v)`.
pub(crate) fn note(parts: &[(&str, &str)]) -> String {
    parts.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join(" / ")
}

/// `urllib.parse.quote(value, safe)`: everything but ASCII alphanumerics, `_.-~` and `safe` is %-encoded.
pub(crate) fn quote(value: &str, safe: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || b"_.-~".contains(&byte) || safe.as_bytes().contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Python `f"{n:,}"`.
pub(crate) fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if n < 0 {
        out.push('-');
    }
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// `urllib.parse.urlencode(pairs)`: `quote_plus` on names and values.
pub(crate) fn urlencode(pairs: &[(&str, String)]) -> String {
    let plus = |value: &str| quote(value, " ").replace(' ', "+");
    pairs.iter().map(|(name, value)| format!("{}={}", plus(name), plus(value))).collect::<Vec<_>>().join("&")
}

/// `row.get(key) or ""` rendered like an f-string: strings as-is, numbers/bools printed, null/false/empty → "".
pub(crate) fn js(row: &Value, key: &str) -> String {
    scalar(row.get(key))
}

pub(crate) fn scalar(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) if n.as_f64() != Some(0.0) => n.to_string(),
        Some(Value::Bool(true)) => "True".into(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_parts_and_query() {
        let url = Url::parse(" https://WWW.Example.com:443/a/b.do/?x=1&contest_pk=&contest_pk=7#frag ");
        assert_eq!(url.host(), "www.example.com");
        assert_eq!(url.path, "/a/b.do/");
        assert_eq!(url.param("contest_pk"), "7");
        assert_eq!(url.param("missing"), "");
        assert_eq!(Url::parse("example.com/x").netloc, "");
        assert_eq!(unquote("%ED%95%9C%2"), "한%2");
    }

    #[test]
    fn encoding_matches_urllib() {
        assert_eq!(quote("a b/한", "/"), "a%20b/%ED%95%9C");
        assert_eq!(urlencode(&[("or", "(a,b c)".into())]), "or=%28a%2Cb+c%29");
        assert_eq!(text("<b>A</b>&amp;<i>\n B</i>"), "A & B");
    }
}
