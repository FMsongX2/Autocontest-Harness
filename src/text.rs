//! Small text helpers shared by the source parsers and renderers.

use chrono::{FixedOffset, Utc};

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
