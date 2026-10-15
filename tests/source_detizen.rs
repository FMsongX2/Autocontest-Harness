use harness::sources::detizen;
use serde_json::{Value, json};

fn load() -> Value {
    let path = format!("{}/tests/fixtures/detizen_list.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn list_parses_real_response() {
    let rows = detizen::parse_list(&load()).unwrap();
    assert_eq!(rows.len(), 19);
    let first = &rows[0];
    assert_eq!(first.entry_id, "detizen-21627ae3-81fe-4f18-9583-814fbd6be8ec");
    assert_eq!(first.entry_id.split('-').next(), Some("detizen"));
    assert_eq!(first.title, "2026년 「청소년이 바꾸는 서울」 인스타툰 공모전");
    assert_eq!(first.url, "https://detizen.com/contests/2026-sysc-contest");
    assert_eq!(first.organizer, "서울특별시립청소년활동진흥센터");
    assert_eq!(first.deadline.as_deref(), Some("2026-11-13T23:59+09:00"));
    assert_eq!(first.categories, ["게임/웹툰/캐릭터"]);
    assert!(first.note.as_deref().unwrap().starts_with("응모대상: 서울지역 어린이·청소년(만 9세~24세)"));
    assert_eq!(first.alt_url.as_deref(), Some("https://www.sy0404.or.kr/bbs/board.php?bo_table=notice&wr_id=1472"));
    assert!(rows.iter().all(|r| r.deadline.is_some()));
    assert!(rows.iter().all(|r| detizen::url_key(&r.url).is_some()));
}

#[test]
fn parse_rejects_non_list_response() {
    assert!(detizen::parse_list(&json!({"message": "Invalid API key"})).is_err());
}

#[test]
fn list_url_filters_open_contests_by_page() {
    let url = detizen::list_url(2, "2026-10-15");
    let query = url.split_once('?').unwrap().1;
    let params: Vec<&str> = query.split('&').collect();
    assert!(params.contains(&"type=eq.contest"));
    assert!(params.contains(&"status=eq.published"));
    assert!(params.contains(&"or=%28end_date.is.null%2Cend_date.gte.2026-10-15%29"));
    assert!(params.contains(&"offset=20"));
}

#[test]
fn url_key_accepts_detail_variants_only() {
    assert_eq!(
        detizen::url_key("https://detizen.com/contests/2026-sysc-contest").as_deref(),
        Some("detizen:2026-sysc-contest")
    );
    assert_eq!(
        detizen::url_key("http://www.detizen.com/contests/2026-sysc-contest/?ref=x").as_deref(),
        Some("detizen:2026-sysc-contest")
    );
    assert_eq!(detizen::url_key("https://www.detizen.com/"), None);
    assert_eq!(detizen::url_key("https://www.detizen.com/?mode=classic&view=contest"), None);
    assert_eq!(detizen::url_key("https://notdetizen.com/contests/a"), None);
    assert_eq!(detizen::url_key("https://www.wevity.com/?ix=1"), None);
}
