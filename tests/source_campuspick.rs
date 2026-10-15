use harness::sources::campuspick;
use serde_json::{Value, json};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn list_and_detail_parse_real_responses() {
    let payload: Value = serde_json::from_str(&fixture("campuspick_list.json")).unwrap();
    let rows = campuspick::parse_list(&payload).unwrap();
    assert_eq!(rows.len(), 20);
    let first = &rows[0];
    assert_eq!(first.entry_id, "campuspick-36547");
    assert_eq!(first.title, "2026 신한 스퀘어브릿지 대학생 창업 공모전 HERO IR 공모전 대학");
    assert_eq!(first.url, "https://www.campuspick.com/contest/view?id=36547");
    assert_eq!(first.organizer, "신한금융희망재단");
    assert_eq!(first.deadline.as_deref(), Some("2026-11-02T23:59+09:00"));
    assert_eq!(first.categories, ["광고/마케팅", "기획/아이디어"]);
    assert!(rows.iter().all(|row| row.deadline.is_some()));
    let detail = campuspick::parse_detail(&fixture("campuspick_detail.html")).unwrap();
    assert!(detail.homepage.starts_with("https://www.chuncheon.go.kr/cityhall/"));
    assert_eq!(detail.prize, "800,000원");
}

#[test]
fn list_rejects_changed_shape() {
    assert!(campuspick::parse_list(&json!({"status": "fail"})).is_err());
}

#[test]
fn url_key_accepts_detail_variants_only() {
    assert_eq!(
        campuspick::url_key("https://www.campuspick.com/contest/view?id=36547").as_deref(),
        Some("campuspick:36547")
    );
    assert_eq!(
        campuspick::url_key("http://campuspick.com/contest/view/?utm=x&id=36547").as_deref(),
        Some("campuspick:36547")
    );
    assert_eq!(campuspick::url_key("https://www.campuspick.com/contest"), None);
    assert_eq!(campuspick::url_key("https://www.campuspick.com/contest/view"), None);
    assert_eq!(campuspick::url_key("https://www.wevity.com/?ix=36547"), None);
}
