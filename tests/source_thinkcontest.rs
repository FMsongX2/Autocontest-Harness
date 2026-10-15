use harness::sources::thinkcontest;
use serde_json::{Value, json};

fn load(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/thinkcontest_{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn list_parse_real_api_page() {
    let rows = thinkcontest::parse_list(&load("ing")).unwrap();
    assert_eq!(rows.len(), 10);
    let first = &rows[0];
    assert_eq!(first.entry_id, "thinkcontest-111925");
    assert_eq!(first.title, "2026 제천 숏폼 영상 공모전");
    assert_eq!(first.url, "https://www.thinkcontest.com/thinkgood/user/contest/view.do?contest_pk=111925");
    assert_eq!(first.organizer, "제천시");
    assert_eq!(first.deadline.as_deref(), Some("2026-11-01T23:59+09:00"));
    assert_eq!(first.categories, ["영상/UCC"]);
    assert_eq!(first.alt_url.as_deref(), Some("https://www.jecheon.go.kr/www/index.do"));
    assert!(first.note.as_deref().unwrap().contains("상금: 200만원"));
    assert_eq!(rows[1].categories, ["기획/아이디어", "문학/수기", "영상/UCC"]);
    assert_eq!(rows[1].deadline.as_deref(), Some("2026-10-30T18:00+09:00"));
}

#[test]
fn deadline_uses_extension_and_treats_midnight_as_date_only() {
    let rows = thinkcontest::parse_list(&load("ing_by_finish")).unwrap();
    let get = |id: &str| rows.iter().find(|r| r.entry_id == id).unwrap().deadline.clone();
    // finish_dt 2026-09-02 18:00, extend_dt 2026-10-30 18:00
    assert_eq!(get("thinkcontest-108895").as_deref(), Some("2026-10-30T18:00+09:00"));
    // the site stores date-only deadlines as 00:00
    assert_eq!(get("thinkcontest-111419").as_deref(), Some("2026-10-15T23:59+09:00"));
}

#[test]
fn ended_rows_dropped_and_bad_response_raises() {
    let mut data = load("ing");
    data["listJsonData"][0]["process"] = json!("END");
    assert_eq!(thinkcontest::parse_list(&data).unwrap().len(), 9);
    assert!(thinkcontest::parse_list(&json!({"status": "0", "msg": "error"})).is_err());
}

#[test]
fn url_key_boundaries() {
    let key = Some("thinkcontest:111925".to_string());
    assert_eq!(
        thinkcontest::url_key("https://www.thinkcontest.com/thinkgood/user/contest/view.do?contest_pk=111925"),
        key
    );
    assert_eq!(
        thinkcontest::url_key(
            "https://thinkcontest.com:443/thinkgood/user/contest/view.do?lmValue=&contest_pk=111925&currentPageNo=2"
        ),
        key
    );
    assert_eq!(thinkcontest::url_key("http://thinkcontest.com/thinkgood/user/contest/view.do/?contest_pk=111925"), key);
    assert_eq!(thinkcontest::url_key("https://www.thinkcontest.com/thinkgood/user/contest/index.do"), None);
    assert_eq!(thinkcontest::url_key("https://www.thinkcontest.com/thinkgood/user/contest/view.do"), None);
    assert_eq!(thinkcontest::url_key("https://www.wevity.com/?c=find&ix=111925"), None);
}
