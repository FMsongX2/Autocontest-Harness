use harness::error::Error;
use harness::sources::{self, DETAIL_FAILED, allcon};
use serde_json::{Value, json};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn load(name: &str) -> Value {
    serde_json::from_str(&fixture(name)).unwrap()
}

#[test]
fn list_parses_real_api_page() {
    let rows = allcon::parse_list(&load("allcon_list.json")).unwrap();
    assert_eq!(rows.len(), 15);
    let first = &rows[0];
    assert_eq!(first.entry_id, "allcon-543498");
    assert_eq!(first.title, "[29역숏폼왕] 제1회 인스탁스 영화제");
    assert_eq!(first.url, "https://www.all-con.co.kr/view/contest/543498");
    assert!(first.organizer.starts_with("한국후지필름"));
    assert_eq!(first.deadline.as_deref(), Some("2026-11-17T23:59+09:00"));
    assert_eq!(first.categories, ["사진·영상"]);
    assert!(first.note.as_deref().unwrap().contains("응모대상: 제한없음"));
    let multi = rows.iter().find(|r| r.entry_id == "allcon-543682").unwrap();
    assert_eq!(multi.categories, ["아이디어·창업·마케팅·네이밍", "디자인·그림·웹툰"]);
}

#[test]
fn new_icon_is_stripped_and_cross_year_deadline() {
    let rows = allcon::parse_list(&load("allcon_list_page2.json")).unwrap();
    let get = |id: &str| rows.iter().find(|r| r.entry_id == id).unwrap();
    assert!(get("allcon-543464").title.ends_with("학습우수사례 영상공모전"));
    assert!(!get("allcon-543464").title.contains("새글"));
    assert_eq!(get("allcon-543426").deadline.as_deref(), Some("2027-01-31T23:59+09:00"));
}

#[test]
fn closed_rows_dropped_and_broken_response_raises() {
    let mut data = load("allcon_list.json");
    data["rows"][0]["cl_status"] = json!("<span class='badge cl_status3'>마감</span><br>D+1");
    assert!(allcon::parse_list(&data).unwrap().iter().all(|r| r.entry_id != "allcon-543498"));
    assert!(allcon::parse_list(&json!({"totalCount": 0, "rows": []})).unwrap().is_empty());
    assert!(allcon::parse_list(&json!({"error": "x"})).is_err());
}

#[test]
fn detail_parse_real_page() {
    let detail = allcon::parse_detail(&fixture("allcon_detail.html"));
    assert_eq!(detail.host, "대구광역시");
    assert_eq!(detail.organizer, "(사)대구경북디자인산업협회");
    assert_eq!(detail.deadline.as_deref(), Some("2026-10-26T23:59+09:00"));
    assert_eq!(detail.eligibility, "제한없음");
    assert_eq!(detail.prize, "총 상금 1770만원");
    assert_eq!(detail.homepage, "http://www.dida.biz");
}

#[test]
fn url_key_only_detail_pages() {
    assert_eq!(allcon::url_key("https://www.all-con.co.kr/view/contest/543682").as_deref(), Some("allcon:543682"));
    assert_eq!(
        allcon::url_key("http://all-con.co.kr/view/contest/543682/?page=1&t=1").as_deref(),
        Some("allcon:543682")
    );
    assert_eq!(allcon::url_key("https://www.all-con.co.kr/list/contest/1"), None);
    assert_eq!(allcon::url_key("https://www.all-con.co.kr/hit/contest/4564"), None);
    assert_eq!(allcon::url_key("https://www.wevity.com/?ix=1"), None);
}

#[test]
fn failed_detail_fetch_keeps_the_list_row() {
    let mut row = allcon::parse_list(&load("allcon_list.json")).unwrap().remove(0);
    let listed = row.clone();
    sources::with_detail(&mut row, Err(Error("HTTP 500 (3회 시도)".into())), allcon::apply_detail);
    assert_eq!((&row.title, &row.organizer, &row.deadline), (&listed.title, &listed.organizer, &listed.deadline));
    assert_eq!(row.deadline.as_deref(), Some("2026-11-17T23:59+09:00"));
    let note = row.note.unwrap();
    assert!(note.contains("응모대상: 제한없음") && note.ends_with(&format!("{DETAIL_FAILED}: HTTP 500 (3회 시도)")));
}
