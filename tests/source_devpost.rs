use harness::sources::devpost;
use serde_json::Value;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn list_parses_real_api_page_and_skips_invite_only() {
    let data: Value = serde_json::from_str(&fixture("devpost_list.json")).unwrap();
    let rows = devpost::parse_list(&data).unwrap();
    assert_eq!(data["hackathons"].as_array().unwrap().len(), 9);
    assert_eq!(rows.len(), 8); // ai-lodge-hackathon-2026 is invite only
    assert!(rows.iter().all(|row| row.entry_id != "devpost-31180"));
    let first = &rows[0];
    assert_eq!(first.entry_id, "devpost-30588");
    assert_eq!(first.title, "EurekaDev 2026");
    assert_eq!(first.url, "https://eurekadev.devpost.com/");
    assert_eq!(first.organizer, "Eureka Institute and UnivaDev");
    assert_eq!(first.categories, ["Beginner Friendly", "Open Ended", "Social Good"]);
    assert_eq!(first.deadline.as_deref(), Some("2026-10-20T23:59+09:00"));
    assert_eq!(first.note.as_deref(), Some("상금: $19,122 / 시간대 미확인"));
    assert_eq!(rows[1].deadline.as_deref(), Some("2027-01-14T23:59+09:00")); // Jul 18, 2026 - Jan 14, 2027
    assert_eq!(rows[1].note.as_deref(), Some("시간대 미확인")); // $0 prize is omitted
    assert_eq!(rows[3].deadline.as_deref(), Some("2026-10-31T23:59+09:00")); // Oct 06 - 31, 2026
}

#[test]
fn list_end_date_formats() {
    assert_eq!(devpost::list_end_date("Aug 31 - Oct 23, 2026").as_deref(), Some("2026-10-23T23:59+09:00"));
    assert_eq!(devpost::list_end_date("Oct 05 - 27, 2026").as_deref(), Some("2026-10-27T23:59+09:00"));
    assert_eq!(devpost::list_end_date("Dec 01, 2026 - Jan 15, 2027").as_deref(), Some("2027-01-15T23:59+09:00"));
    assert_eq!(devpost::list_end_date(""), None);
}

#[test]
fn detail_deadline_converted_to_kst_with_eligibility() {
    let detail = devpost::parse_detail(&fixture("devpost_detail.html"));
    assert_eq!(detail.deadline.as_deref(), Some("2026-10-27T22:00+09:00")); // 2026-10-27T09:00:00-04:00
    assert_eq!(detail.eligibility[0], "Above legal age of majority in country of residence");
    assert!(detail.excluded.iter().any(|c| c == "Korea Democratic People's Republic of"));
    assert!(!detail.korea_excluded);
    let standard = devpost::parse_detail(&fixture("devpost_detail_standard.html"));
    assert_eq!(standard.deadline.as_deref(), Some("2026-10-24T04:00+09:00")); // 2026-10-23T15:00:00-04:00
    assert_eq!(standard.excluded, ["Brazil", "Crimea", "Cuba", "Iran", "North Korea", "Quebec", "Russia"]);
    assert!(!standard.korea_excluded);
}

#[test]
fn detail_flags_south_korea_exclusion() {
    let page = r#"<ul id="eligibility-list"><li>All, excluding <span data-tooltip title="Cuba, South Korea">x</span></li></ul>"#;
    let detail = devpost::parse_detail(page);
    assert!(detail.korea_excluded);
    assert_eq!(detail.deadline, None);
}

#[test]
fn url_key_only_hackathon_subdomains() {
    assert_eq!(devpost::url_key("https://gitlab-transcend.devpost.com/").as_deref(), Some("devpost:gitlab-transcend"));
    assert_eq!(
        devpost::url_key("http://GitLab-Transcend.devpost.com/rules?x=1").as_deref(),
        Some("devpost:gitlab-transcend")
    );
    assert_eq!(devpost::url_key("https://devpost.com/hackathons"), None);
    assert_eq!(devpost::url_key("https://www.devpost.com/hackathons"), None);
    assert_eq!(devpost::url_key("https://devpost.com/api/hackathons?status[]=open"), None);
    assert_eq!(devpost::url_key("https://www.kaggle.com/competitions/x"), None);
}
