use harness::sources::linkareer;

#[test]
fn parse_reads_close_time_from_apollo_state() {
    let page = concat!(
        r#"<script id="__NEXT_DATA__" type="application/json">"#,
        r#"{"props":{"pageProps":{"activityItems":[{"url":"https://linkareer.com/activity/42","name":"테스트 공모전"}],"#,
        r#""__APOLLO_STATE__":{"Activity:42":{"organizationName":"주최","recruitCloseAt":1793458799999}}}}}</script>"#
    );
    let rows = linkareer::parse(page).unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.entry_id, "linkareer-42");
    assert_eq!(row.title, "테스트 공모전");
    assert_eq!(row.url, "https://linkareer.com/activity/42");
    assert_eq!(row.organizer, "주최");
    assert_eq!(row.deadline.as_deref(), Some("2026-10-31T23:59+09:00"));
    assert!(row.categories.is_empty() && row.note.is_none() && row.alt_url.is_none());
}

#[test]
fn missing_next_data_is_an_error_and_url_key() {
    assert!(linkareer::parse("<html></html>").is_err());
    assert_eq!(linkareer::url_key("https://linkareer.com/activity/123?x=1").as_deref(), Some("linkareer:123"));
    assert_eq!(linkareer::url_key("https://linkareer.com/list/contest"), None);
}
