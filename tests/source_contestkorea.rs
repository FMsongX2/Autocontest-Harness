use harness::sources::contestkorea;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn list_and_detail_parse_real_pages() {
    let rows = contestkorea::parse_list(&fixture("contestkorea_list.html")).unwrap();
    assert_eq!(rows.len(), 12);
    let first = &rows[0];
    assert_eq!(first.entry_id, "contestkorea-202610150008");
    assert_eq!(first.title, "화이트햇 2026 국방 사이버보안 콘테스트");
    assert_eq!(
        first.url,
        "https://www.contestkorea.com/sub/view.php?int_gbn=1&Txt_bcode=030310001&str_no=202610150008"
    );
    assert_eq!(first.organizer, "국방부");
    assert_eq!(first.categories, ["학문•과학•IT"]);
    assert_eq!(first.deadline.as_deref(), Some("2026-11-02T23:59+09:00"));
    assert_eq!(first.note.as_deref(), Some("참가대상: 중학생, 고등학생, 대학생, 해당자"));
    let detail = contestkorea::parse_detail(&fixture("contestkorea_detail.html"));
    assert_eq!(detail.deadline.as_deref(), Some("2026-11-02T23:59+09:00"));
    assert_eq!(detail.organizer, "국방부/사이버작전사령부");
    assert_eq!(detail.homepage, "http://whitehat2026.kr/");
    assert_eq!(detail.eligibility, "중학생, 고등학생, 대학생, 해당자(현역 군인, 군무원, 사관생도)");
    assert!(detail.prize.contains("4,500만원"));
}

#[test]
fn closed_rows_are_dropped() {
    let rows = contestkorea::parse_list(&fixture("contestkorea_list_closed.html")).unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.entry_id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "contestkorea-202609150001",
            "contestkorea-202609150013",
            "contestkorea-202609070099",
            "contestkorea-202609070097"
        ]
    );
}

#[test]
fn list_without_list_area_raises() {
    assert!(contestkorea::parse_list("<html><body>점검 중</body></html>").is_err());
}

#[test]
fn deadline_year_from_registration_date() {
    let infer = |period, registered| contestkorea::infer_deadline(period, registered);
    assert_eq!(infer("10.02~11.02", "20261015").as_deref(), Some("2026-11-02T23:59+09:00"));
    assert_eq!(infer("12.01~01.15", "20261120").as_deref(), Some("2027-01-15T23:59+09:00"));
    assert_eq!(infer("01.05~02.10", "20261228").as_deref(), Some("2027-02-10T23:59+09:00"));
    assert_eq!(infer("12.20~03.01", "20270105").as_deref(), Some("2027-03-01T23:59+09:00"));
    assert_eq!(infer("", "20261015"), None);
}

#[test]
fn url_key_only_matches_detail_pages() {
    let key = Some("contestkorea:202610150008".to_string());
    assert_eq!(
        contestkorea::url_key(
            "https://www.contestkorea.com/sub/view.php?int_gbn=1&Txt_bcode=030310001&str_no=202610150008"
        ),
        key
    );
    assert_eq!(contestkorea::url_key("http://contestkorea.com/sub/view.php?str_no=202610150008&int_gbn=1"), key);
    assert_eq!(contestkorea::url_key("https://www.contestkorea.com/sub/list.php?int_gbn=1&page=2"), None);
    assert_eq!(contestkorea::url_key("https://www.contestkorea.com/sub/view.php?int_gbn=1"), None);
    assert_eq!(contestkorea::url_key("https://www.wevity.com/?c=find&ix=111674"), None);
}
