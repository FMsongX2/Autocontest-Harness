use harness::sources::dacon;
use serde_json::{Value, json};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn list_parses_nuxt_cards() {
    let rows = dacon::parse_list(&fixture("dacon_list.html")).unwrap();
    assert_eq!(rows.len(), 15);
    let first = &rows[0];
    assert_eq!(first.entry_id, "dacon-236771");
    assert_eq!(first.title, "항만 문제해결을 위한 AI 기술 공모전");
    assert_eq!(first.url, "https://dacon.io/competitions/official/236771/overview");
    assert_eq!(first.deadline.as_deref(), Some("2026-11-02T10:00+09:00"));
    assert_eq!(first.categories, ["알고리즘", "코드 제출 평가", "개체 매칭", "해운물류"]);
    assert!(rows.iter().all(|row| row.deadline.is_some()));
}

#[test]
fn list_without_nuxt_data_raises() {
    assert!(dacon::parse_list("<html><body>점검 중</body></html>").is_err());
}

#[test]
fn detail_takes_earliest_participation_or_submission_deadline() {
    let payload: Value = serde_json::from_str(&fixture("dacon_detail.json")).unwrap();
    let detail = dacon::parse_detail(&payload).unwrap();
    assert_eq!(detail.registration_end.as_deref(), Some("2026-11-02T10:00+09:00"));
    assert_eq!(detail.submission_end.as_deref(), Some("2026-11-01T10:00+09:00"));
    assert_eq!(detail.period_end.as_deref(), Some("2026-11-02T10:00+09:00"));
    assert_eq!(detail.merge_deadline.as_deref(), Some("2026-10-26T23:59+09:00"));
    let (deadline, note) = dacon::summarize(&detail);
    assert_eq!(deadline.as_deref(), Some("2026-11-01T10:00+09:00"));
    assert!(note.contains("리더보드 제출 마감: 2026-11-01 10:00") && note.contains("대회 종료: 2026-11-02 10:00"));
    assert!(note.contains("상금: 950 만원"));
}

#[test]
fn detail_registration_closing_before_contest_end() {
    let payload = json!({"data": [{
        "period_start": "2026-07-06 10:00:00", "period_end": "2026-08-05 10:00:00", "submission_end": "2026-07-01 23:59:59",
        "merge_deadline": null, "time_line_data": "[]",
        "schedule": "<p>- 참가 신청 기간 : 2026년 6월 11일(목) ~ 2026년 7월 5일(일)</p><p>- 대회 기간 : 2026년 7월 6일(월) 10:00 ~ 2026년 8월 5일(수) 10:00</p>",
    }]});
    let detail = dacon::parse_detail(&payload).unwrap();
    assert_eq!(detail.registration_end.as_deref(), Some("2026-07-05T23:59+09:00"));
    // A value before the contest starts is a leftover from a previous contest.
    assert_eq!(detail.api_submission_end, None);
    assert_eq!(dacon::summarize(&detail).0.as_deref(), Some("2026-07-05T23:59+09:00"));
    let pm = dacon::parse_detail(&json!({"data": [{
        "schedule": "<p>- 참가 신청 기간: 2026.07.16 (목) 오전 10시 ~ 2026.08.17 (월) 오후 11시 59분</p>",
    }]}))
    .unwrap();
    assert_eq!(pm.registration_end.as_deref(), Some("2026-08-17T23:59+09:00"));
}

#[test]
fn url_key() {
    assert_eq!(
        dacon::url_key("https://dacon.io/competitions/official/236771/overview/description").as_deref(),
        Some("dacon:236771")
    );
    assert_eq!(dacon::url_key("https://www.dacon.io/competitions/official/236771/").as_deref(), Some("dacon:236771"));
    assert_eq!(
        dacon::url_key("https://dacon.io/en/competitions/official/236771/overview").as_deref(),
        Some("dacon:236771")
    );
    assert_eq!(dacon::url_key("https://dacon.io/competitions"), None);
    assert_eq!(dacon::url_key("https://dacon.io/competitions/official"), None);
    assert_eq!(dacon::url_key("https://example.com/competitions/official/236771/overview"), None);
}
