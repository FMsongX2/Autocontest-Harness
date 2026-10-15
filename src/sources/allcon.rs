//! All-con (university/general contests, t=1). The list HTML is empty; rows come from the internal
//! JSON `/page/ajax.contest_list.php`, which has the date-only application period. New rows open the
//! detail page for the untruncated host, organiser, eligibility, prize and application URL. Sponsored
//! (starred) rows repeat at the top of every page, so each id is kept once.
//!
//! The server omits its TLS intermediate certificate. Verification stays on; the public intermediate
//! its leaf's AIA points to (Sectigo DV R36, expires 2036-03-21) is trusted in addition.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use ureq::Agent;

use super::Item;
use super::common::{Url, js, note, pause, re, scalar, text, urlencode};
use crate::error::{Error, Result};
use crate::http;
use crate::text::end_of_day;

pub const NAME: &str = "allcon";
const BASE: &str = "https://www.all-con.co.kr";
const ROWS_PER_PAGE: u32 = 15;
/// The `cl_cate` badges mix fields with eligibility; eligibility names (same as the site filter) are not fields.
const TARGETS: [&str; 9] =
    ["제한없음", "일반인", "대학생", "대학원생", "초등학생", "중학생", "고등학생", "주부 및 영유아", "기타"];
const CLOSED: [&str; 2] = ["마감", "접수마감"];
/// http://crt.sectigo.com/SectigoPublicServerAuthenticationCADVR36.crt (issuer: Sectigo Public Server Authentication Root R46)
const SECTIGO_DV_R36: &str = "\
-----BEGIN CERTIFICATE-----
MIIGTDCCBDSgAwIBAgIQOXpmzCdWNi4NqofKbqvjsTANBgkqhkiG9w0BAQwFADBf
MQswCQYDVQQGEwJHQjEYMBYGA1UEChMPU2VjdGlnbyBMaW1pdGVkMTYwNAYDVQQD
Ey1TZWN0aWdvIFB1YmxpYyBTZXJ2ZXIgQXV0aGVudGljYXRpb24gUm9vdCBSNDYw
HhcNMjEwMzIyMDAwMDAwWhcNMzYwMzIxMjM1OTU5WjBgMQswCQYDVQQGEwJHQjEY
MBYGA1UEChMPU2VjdGlnbyBMaW1pdGVkMTcwNQYDVQQDEy5TZWN0aWdvIFB1Ymxp
YyBTZXJ2ZXIgQXV0aGVudGljYXRpb24gQ0EgRFYgUjM2MIIBojANBgkqhkiG9w0B
AQEFAAOCAY8AMIIBigKCAYEAljZf2HIz7+SPUPQCQObZYcrxLTHYdf1ZtMRe7Yeq
RPSwygz16qJ9cAWtWNTcuICc++p8Dct7zNGxCpqmEtqifO7NvuB5dEVexXn9RFFH
12Hm+NtPRQgXIFjx6MSJcNWuVO3XGE57L1mHlcQYj+g4hny90aFh2SCZCDEVkAja
EMMfYPKuCjHuuF+bzHFb/9gV8P9+ekcHENF2nR1efGWSKwnfG5RawlkaQDpRtZTm
M64TIsv/r7cyFO4nSjs1jLdXYdz5q3a4L0NoabZfbdxVb+CUEHfB0bpulZQtH1Rv
38e/lIdP7OTTIlZh6OYL6NhxP8So0/sht/4J9mqIGxRFc0/pC8suja+wcIUna0HB
pXKfXTKpzgis+zmXDL06ASJf5E4A2/m+Hp6b84sfPAwQ766rI65mh50S0Di9E3Pn
2WcaJc+PILsBmYpgtmgWTR9eV9otfKRUBfzHUHcVgarub/XluEpRlTtZudU5xbFN
xx/DgMrXLUAPaI60fZ6wA+PTAgMBAAGjggGBMIIBfTAfBgNVHSMEGDAWgBRWc1hk
lfmSGrASKgRieaFAFYghSTAdBgNVHQ4EFgQUaMASFhgOr872h6YyV6NGUV3LBycw
DgYDVR0PAQH/BAQDAgGGMBIGA1UdEwEB/wQIMAYBAf8CAQAwHQYDVR0lBBYwFAYI
KwYBBQUHAwEGCCsGAQUFBwMCMBsGA1UdIAQUMBIwBgYEVR0gADAIBgZngQwBAgEw
VAYDVR0fBE0wSzBJoEegRYZDaHR0cDovL2NybC5zZWN0aWdvLmNvbS9TZWN0aWdv
UHVibGljU2VydmVyQXV0aGVudGljYXRpb25Sb290UjQ2LmNybDCBhAYIKwYBBQUH
AQEEeDB2ME8GCCsGAQUFBzAChkNodHRwOi8vY3J0LnNlY3RpZ28uY29tL1NlY3Rp
Z29QdWJsaWNTZXJ2ZXJBdXRoZW50aWNhdGlvblJvb3RSNDYucDdjMCMGCCsGAQUF
BzABhhdodHRwOi8vb2NzcC5zZWN0aWdvLmNvbTANBgkqhkiG9w0BAQwFAAOCAgEA
YtOC9Fy+TqECFw40IospI92kLGgoSZGPOSQXMBqmsGWZUQ7rux7cj1du6d9rD6C8
ze1B2eQjkrGkIL/OF1s7vSmgYVafsRoZd/IHUrkoQvX8FZwUsmPu7amgBfaY3g+d
q1x0jNGKb6I6Bzdl6LgMD9qxp+3i7GQOnd9J8LFSietY6Z4jUBzVoOoz8iAU84OF
h2HhAuiPw1ai0VnY38RTI+8kepGWVfGxfBWzwH9uIjeooIeaosVFvE8cmYUB4TSH
5dUyD0jHct2+8ceKEtIoFU/FfHq/mDaVnvcDCZXtIgitdMFQdMZaVehmObyhRdDD
4NQCs0gaI9AAgFj4L9QtkARzhQLNyRf87Kln+YU0lgCGr9HLg3rGO8q+Y4ppLsOd
unQZ6ZxPNGIfOApbPVf5hCe58EZwiWdHIMn9lPP6+F404y8NNugbQixBber+x536
WrZhFZLjEkhp7fFXf9r32rNPfb74X/U90Bdy4lzp3+X1ukh1BuMxA/EEhDoTOS3l
7ABvc7BYSQubQ2490OcdkIzUh3ZwDrakMVrbaTxUM2p24N6dB+ns2zptWCva6jzW
r8IWKIMxzxLPv5Kt3ePKcUdvkBU/smqujSczTzzSjIoR5QqQA6lN1ZRSnuHIWCvh
JEltkYnTAH41QJ6SAWO66GrrUESwN/cgZzL4JLEqz1Y=
-----END CERTIFICATE-----
";

fn view_url(srl: &str) -> String {
    format!("{BASE}/view/contest/{srl}")
}

pub fn url_key(url: &str) -> Option<String> {
    static PATH: LazyLock<Regex> = LazyLock::new(|| re(r"^/view/contest/(\d+)/?$"));
    let parsed = Url::parse(url);
    let host = parsed.host();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    if host.strip_prefix("m.").unwrap_or(host) != "all-con.co.kr" {
        return None;
    }
    PATH.captures(parsed.path).map(|c| format!("{NAME}:{}", &c[1]))
}

/// End date of a `26.10.08~26.11.17` period (no time given, so 23:59 that day).
fn period_deadline(period: &str) -> Option<String> {
    static DATE: LazyLock<Regex> = LazyLock::new(|| re(r"^(\d\d)\.(\d\d)\.(\d\d)$"));
    let last = period.split('~').rfind(|p| !p.trim().is_empty())?;
    let d = DATE.captures(last.trim())?;
    Some(end_of_day(2000 + d[1].parse::<i32>().ok()?, d[2].parse().ok()?, d[3].parse().ok()?))
}

/// Rows on one list JSON page; closed ones are dropped.
pub fn parse_list(data: &Value) -> Result<Vec<Item>> {
    static BR: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)<br\s*/?>.*"));
    static NEW_ICON: LazyLock<Regex> = LazyLock::new(|| re(r#"(?s)<span class="new_icon">.*?</span>\s*</span>"#));
    static BADGE: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)<span[^>]*>(.*?)</span>"));
    let Some(list) = data.get("rows").and_then(Value::as_array) else {
        return Err(Error("allcon: 목록 응답에 rows가 없다".into()));
    };
    let mut rows = Vec::new();
    let mut closed = 0;
    for row in list {
        let srl = scalar(row.get("cl_srl")).trim().to_string();
        if srl.is_empty() || !srl.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let status = text(&BR.replace(&js(row, "cl_status"), ""));
        if CLOSED.contains(&status.as_str()) {
            closed += 1;
            continue;
        }
        let badges: Vec<String> = BADGE.captures_iter(&js(row, "cl_cate")).map(|b| text(&b[1])).collect();
        let targets: Vec<&str> = badges.iter().map(String::as_str).filter(|b| TARGETS.contains(b)).collect();
        rows.push(Item {
            entry_id: format!("{NAME}-{srl}"),
            title: text(&NEW_ICON.replace_all(&js(row, "cl_title"), "")),
            url: view_url(&srl),
            organizer: text(&js(row, "cl_host")),
            deadline: period_deadline(&js(row, "cl_date")),
            categories: badges.iter().filter(|b| !b.is_empty() && !TARGETS.contains(&b.as_str())).cloned().collect(),
            note: Some(note(&[("응모대상", &targets.join(", ")), ("상태", &status)])),
            alt_url: None,
        });
    }
    if list.iter().any(|r| !scalar(r.get("cl_srl")).is_empty()) && rows.is_empty() && closed == 0 {
        return Err(Error("allcon: 목록 행을 하나도 읽지 못했다(구조 변경 의심)".into()));
    }
    Ok(rows)
}

#[derive(Debug, Default, PartialEq)]
pub struct Detail {
    pub host: String,
    pub organizer: String,
    pub deadline: Option<String>,
    pub eligibility: String,
    pub prize: String,
    pub homepage: String,
}

/// Host, organiser, period, eligibility, prize and application URL from a detail page.
pub fn parse_detail(page: &str) -> Detail {
    static FIELD: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)<td class='desc_(\w+)'[^>]*>(.*?)</td>"));
    static HOMEPAGE: LazyLock<Regex> = LazyLock::new(|| re(r#"<a href="([^"]+)"[^>]*class="cl_web""#));
    let info: HashMap<String, String> = FIELD.captures_iter(page).map(|c| (c[1].to_string(), text(&c[2]))).collect();
    let get = |key: &str| info.get(key).cloned().unwrap_or_default();
    Detail {
        host: get("host"),
        organizer: get("organizer"),
        deadline: period_deadline(&get("date")),
        eligibility: get("target"),
        prize: get("award"),
        homepage: HOMEPAGE.captures(page).map(|c| c[1].trim().to_string()).unwrap_or_default(),
    }
}

fn fetch_list(agent: &Agent, page: u32) -> Result<Value> {
    let body = urlencode(&[
        ("sortorder", "asc".into()),
        ("page", page.to_string()),
        ("sortname", "cl_order".into()),
        ("t", "1".into()),
        ("rows", ROWS_PER_PAGE.to_string()),
    ]);
    let referer = format!("{BASE}/list/contest/1");
    let headers = [
        ("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8"),
        ("X-Requested-With", "XMLHttpRequest"),
        ("Accept", "application/json"),
        ("Referer", referer.as_str()),
    ];
    let url = format!("{BASE}/page/ajax.contest_list.php");
    http::Request { url: &url, headers: &headers, body: Some(body.as_bytes()), agent: Some(agent) }.json()
}

/// Fill a new row from its detail page: host, deadline, organiser, eligibility, prize and application URL.
pub fn apply_detail(row: &mut Item, page: String) -> Result<()> {
    let detail = parse_detail(&page);
    if !detail.host.is_empty() {
        row.organizer = detail.host;
    }
    row.deadline = detail.deadline.or(row.deadline.take());
    row.note = Some(note(&[
        ("주관", &detail.organizer),
        ("응모대상", &detail.eligibility),
        ("상금", &detail.prize),
        ("접수처", &detail.homepage),
    ]));
    if !detail.homepage.is_empty() {
        row.alt_url = Some(detail.homepage);
    }
    Ok(())
}

/// Every row on the first `pages` list pages (sponsored rows once); rows not in `known` get their detail page read.
pub fn collect(pages: u32, known: &HashSet<String>) -> Result<Vec<Item>> {
    let agent = http::agent_trusting(SECTIGO_DV_R36)?;
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for page in 1..=pages {
        let data = fetch_list(&agent, page)?;
        for mut row in parse_list(&data)? {
            if !seen.insert(row.entry_id.clone()) {
                continue;
            }
            if !known.contains(&row.entry_id) {
                pause();
                let fetched = http::Request { url: &row.url, headers: &[], body: None, agent: Some(&agent) }.send();
                super::with_detail(&mut row, fetched, apply_detail);
            }
            items.push(row);
        }
        pause();
        let total = match &data["totalPage"] {
            Value::String(s) => s.trim().parse().unwrap_or(0),
            other => other.as_u64().unwrap_or(0),
        };
        if u64::from(page) >= total {
            break;
        }
    }
    Ok(items)
}
