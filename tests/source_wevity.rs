use harness::error::Error;
use harness::sources::{self, DETAIL_FAILED, wevity};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn list_and_detail_parse_real_pages() {
    let rows = wevity::parse_list(&fixture("wevity_list.html"));
    assert_eq!(rows.len(), 15);
    let first = &rows[0];
    assert_eq!(first.entry_id, "wevity-111674");
    assert_eq!(first.title, "2026 화성특례시 AI CF 공모전 - 제6회 별별화성 Awards");
    assert_eq!(first.organizer, "화성특례시");
    assert!(first.categories.iter().any(|c| c == "영상/UCC/사진"));
    let detail = wevity::parse_detail(&fixture("wevity_detail.html"));
    assert_eq!(detail.deadline.as_deref(), Some("2026-10-25T23:59+09:00"));
    assert_eq!(detail.homepage, "https://aihscity.com/");
    assert_eq!(detail.eligibility, "제한없음");
}

#[test]
fn url_key_ignores_listing_pages_without_ix() {
    assert_eq!(wevity::url_key("https://wevity.com/?ix=111674&c=find").as_deref(), Some("wevity:111674"));
    assert_eq!(wevity::url_key("https://www.wevity.com/?c=find&s=1&gub=1&gp=2"), None);
    assert_eq!(wevity::url_key("https://linkareer.com/activity/1"), None);
}

#[test]
fn failed_detail_fetch_keeps_the_list_row_with_a_note() {
    let mut row = wevity::parse_list(&fixture("wevity_list.html")).remove(0);
    let listed = row.clone();
    sources::with_detail(
        &mut row,
        Err(Error("https://www.wevity.com/?ix=111674: HTTP 404".into())),
        wevity::apply_detail,
    );
    assert_eq!(
        (&row.entry_id, &row.title, &row.deadline, &row.alt_url),
        (&listed.entry_id, &listed.title, &None, &None)
    );
    let note = row.note.unwrap();
    assert!(note.starts_with(DETAIL_FAILED) && note.contains("HTTP 404"), "{note}");

    let mut row = listed;
    sources::with_detail(&mut row, Ok(fixture("wevity_detail.html")), wevity::apply_detail);
    assert_eq!(row.deadline.as_deref(), Some("2026-10-25T23:59+09:00"));
    assert!(!row.note.unwrap().contains(DETAIL_FAILED));
}
