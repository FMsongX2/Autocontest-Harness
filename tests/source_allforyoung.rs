use harness::error::Error;
use harness::sources::{self, DETAIL_FAILED, allforyoung};
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn list_and_detail_parse_real_responses() {
    let rows = allforyoung::parse_list(&fixture("allforyoung_list.json")).unwrap();
    assert_eq!(rows.len(), 20);
    let first = &rows[0];
    assert_eq!(first.entry_id, "allforyoung-88954");
    assert_eq!(first.title, "2026 고립·은둔청년 인식개선 공모전");
    assert_eq!(first.url, "https://www.allforyoung.com/posts/88954");
    assert_eq!(first.organizer, "서울청년기지개센터");
    assert_eq!(first.deadline, None);
    assert_eq!(first.categories, ["포스터/웹툰/콘텐츠", "사진/영상/UCC"]);
    let detail = allforyoung::parse_detail(&fixture("allforyoung_detail.json")).unwrap();
    assert_eq!(detail.deadline.as_deref(), Some("2026-11-08T23:59+09:00"));
    assert_eq!(detail.start, "2026-10-15");
    assert_eq!(detail.apply_url, "https://forms.gle/ieVNmMDWdmw2VayR6d");
    assert!(!detail.permanent);
}

#[test]
fn deadline_conversion_and_expired_rows() {
    assert_eq!(allforyoung::parse_deadline(Some("2026-11-08")).as_deref(), Some("2026-11-08T23:59+09:00"));
    assert_eq!(allforyoung::parse_deadline(Some("2026-11-08T09:00:00Z")).as_deref(), Some("2026-11-08T18:00+09:00"));
    assert_eq!(allforyoung::parse_deadline(Some("2026-11-08T18:00:00")).as_deref(), Some("2026-11-08T18:00+09:00"));
    assert_eq!(allforyoung::parse_deadline(Some("")), None);
    let payload = json!({"success": true, "data": [{"id": 1, "category": "공모전", "title": "x", "is_expired": true}]});
    assert!(allforyoung::parse_list(&payload).unwrap().is_empty());
    assert!(allforyoung::parse_list(&json!({"success": false})).is_err());
}

#[test]
fn url_key_accepts_detail_variants_only() {
    assert_eq!(allforyoung::url_key("https://www.allforyoung.com/posts/88954").as_deref(), Some("allforyoung:88954"));
    assert_eq!(
        allforyoung::url_key("https://allforyoung.com/posts/88954/?ref=x").as_deref(),
        Some("allforyoung:88954")
    );
    assert_eq!(allforyoung::url_key("https://www.allforyoung.com/posts/contest"), None);
    assert_eq!(allforyoung::url_key("https://linkareer.com/activity/88954"), None);
}

#[test]
fn failed_detail_fetch_or_parse_keeps_the_list_row() {
    let listed = allforyoung::parse_list(&fixture("allforyoung_list.json")).unwrap().remove(0);
    let mut fetch_failed = listed.clone();
    sources::with_detail(&mut fetch_failed, Err(Error("HTTP 404".into())), allforyoung::apply_detail);
    let mut changed_shape = listed.clone();
    sources::with_detail(&mut changed_shape, Ok(json!({"success": false})), allforyoung::apply_detail);
    for row in [fetch_failed, changed_shape] {
        assert_eq!(
            (&row.entry_id, &row.title, &row.deadline, &row.alt_url),
            (&listed.entry_id, &listed.title, &None, &None)
        );
        assert!(row.note.unwrap().starts_with(DETAIL_FAILED));
    }
    let mut ok = listed;
    sources::with_detail(&mut ok, Ok(fixture("allforyoung_detail.json")), allforyoung::apply_detail);
    assert_eq!(ok.deadline.as_deref(), Some("2026-11-08T23:59+09:00"));
}

#[test]
fn list_titles_are_single_line() {
    let payload = json!({"success": true, "data": [{"id": 7, "category": "공모전",
        "title": "  2026 청년\n아이디어\t공모전 ", "organization": "서울\r\n청년센터"}]});
    let rows = allforyoung::parse_list(&payload).unwrap();
    assert_eq!((rows[0].title.as_str(), rows[0].organizer.as_str()), ("2026 청년 아이디어 공모전", "서울 청년센터"));
}
