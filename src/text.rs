//! Small text helpers shared by the source parsers and renderers.

use chrono::{FixedOffset, NaiveDate, NaiveDateTime, Utc};

/// Korea Standard Time; every timestamp the harness writes uses it.
pub fn kst() -> FixedOffset {
    FixedOffset::east_opt(9 * 3600).expect("valid offset")
}

/// Current time as `YYYY-MM-DDTHH:MM+09:00`.
pub fn now() -> String {
    Utc::now().with_timezone(&kst()).format("%Y-%m-%dT%H:%M%:z").to_string()
}

/// A deadline known only by date is taken as 23:59 KST that day; the gate re-reads the exact time.
pub fn end_of_day(year: i32, month: u32, day: u32) -> String {
    format!("{year:04}-{month:02}-{day:02}T23:59+09:00")
}

/// Strip tags, decode entities and collapse whitespace.
pub fn strip_tags(fragment: &str) -> String {
    let mut out = String::with_capacity(fragment.len());
    let mut in_tag = false;
    for ch in fragment.chars() {
        match ch {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    collapse(&unescape(&out))
}

/// Collapse runs of whitespace to one space and trim.
pub fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One clean line for a title or organizer: whitespace runs (newlines included) become one space,
/// control and invisible formatting characters (zero-width, bidi overrides) are dropped, trimmed.
pub fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            space = true;
        } else if !(ch.is_control() || invisible(ch)) {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(ch);
        }
    }
    out
}

fn invisible(ch: char) -> bool {
    matches!(ch, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

/// `http://` or `https://` with a host.
pub fn is_web_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["http://", "https://"].iter().any(|scheme| lower.strip_prefix(scheme).is_some_and(|rest| !rest.is_empty()))
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// A date or time stamp in the harness form: `YYYY-MM-DD`, or `YYYY-MM-DDTHH:MM+09:00` (seconds
/// are dropped, `24:00` is read as `23:59`). Only the KST offset is accepted, so a stamp is never
/// silently shifted. `None` when the text is not that form or not a real calendar date or time.
pub fn normalize_stamp(text: &str) -> Option<String> {
    let text = text.trim();
    if text.len() == 10 {
        return NaiveDate::parse_from_str(text, "%Y-%m-%d").ok().map(|d| d.format("%Y-%m-%d").to_string());
    }
    let local = text.strip_suffix("+09:00")?.replace("T24:00", "T23:59");
    let time = if local.len() == 16 { format!("{local}:00") } else { local };
    let parsed = NaiveDateTime::parse_from_str(&time, "%Y-%m-%dT%H:%M:%S").ok()?;
    Some(format!("{}+09:00", parsed.format("%Y-%m-%dT%H:%M")))
}

/// A deadline as the harness stores it: [`normalize_stamp`], with a date-only value taken as
/// 23:59 KST that day.
pub fn normalize_deadline(text: &str) -> Option<String> {
    let stamp = normalize_stamp(text)?;
    Some(if stamp.len() == 10 { format!("{stamp}T23:59+09:00") } else { stamp })
}

/// Decode numeric character references and the named entities that appear on contest sites.
pub fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let decoded =
            tail.find(';').filter(|&end| end <= 12).and_then(|end| decode_entity(&tail[1..end]).map(|ch| (ch, end)));
        match decoded {
            Some((ch, end)) => {
                out.push(ch);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(code);
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "middot" => '·',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "hellip" => '…',
        "ndash" => '–',
        "mdash" => '—',
        "bull" => '•',
        "rarr" => '→',
        "larr" => '←',
        "times" => '×',
        "reg" => '®',
        "trade" => '™',
        "lt;" => '<',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_tags_decodes_and_collapses() {
        assert_eq!(strip_tags("<b>AI&nbsp;숏폼</b>\n  &amp; <i>영상</i> &#44608;"), "AI 숏폼 & 영상 김");
        assert_eq!(unescape("a & b &unknown; c"), "a & b &unknown; c");
    }
}
