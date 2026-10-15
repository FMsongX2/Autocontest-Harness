use harness::sources::kaggle;
use serde_json::Value;

#[test]
fn list_parses_real_api_page_without_getting_started() {
    let path = format!("{}/tests/fixtures/kaggle_list.json", env!("CARGO_MANIFEST_DIR"));
    let data: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let rows = kaggle::parse_list(&data).unwrap();
    assert_eq!(data["competitions"].as_array().unwrap().len(), 20);
    assert_eq!(rows.len(), 11); // 9 Getting Started (segment 5) dropped
    assert!(rows.iter().all(|row| row.entry_id != "kaggle-34377")); // spaceship-titanic
    let first = &rows[0];
    assert_eq!(first.entry_id, "kaggle-170484");
    assert_eq!(first.title, "Name That LLM");
    assert_eq!(first.url, "https://www.kaggle.com/competitions/name-that-llm");
    assert_eq!(first.organizer, "Kaggle");
    assert_eq!(first.categories, ["Featured", "Classification"]);
    // entry deadline 2027-01-06T23:59:00Z, final 2027-01-14T23:59:00.817Z
    assert_eq!(first.deadline.as_deref(), Some("2027-01-07T08:59+09:00"));
    assert_eq!(
        first.note.as_deref(),
        Some("상금: USD 50,000 / 최종 제출 마감: 2027-01-15T08:59+09:00 / 팀 최대 5명 / 신원 확인 필요")
    );
    let playground = rows.iter().find(|row| row.entry_id == "kaggle-125224").unwrap();
    // No entry deadline -> final 2026-10-31T23:59:00Z.
    assert_eq!(playground.deadline.as_deref(), Some("2026-11-01T08:59+09:00"));
    assert!(playground.note.as_deref().unwrap().starts_with("상금: 기념품"));
    assert!(playground.categories.iter().any(|c| c == "Playground"));
}

#[test]
fn to_kst() {
    assert_eq!(kaggle::to_kst(Some("2026-10-26T11:59:00Z")).unwrap().as_deref(), Some("2026-10-26T20:59+09:00"));
    assert_eq!(kaggle::to_kst(Some("2026-11-12T23:59:00.807Z")).unwrap().as_deref(), Some("2026-11-13T08:59+09:00"));
    assert_eq!(kaggle::to_kst(None).unwrap(), None);
    assert!(kaggle::to_kst(Some("2026-11-12")).is_err());
}

#[test]
fn url_key_competition_pages_only() {
    assert_eq!(
        kaggle::url_key("https://www.kaggle.com/competitions/name-that-llm").as_deref(),
        Some("kaggle:name-that-llm")
    );
    assert_eq!(
        kaggle::url_key("kaggle.com/c/Name-That-LLM/overview?tab=rules").as_deref(),
        Some("kaggle:name-that-llm")
    );
    assert_eq!(
        kaggle::url_key("https://www.kaggle.com/competitions/name-that-llm/").as_deref(),
        Some("kaggle:name-that-llm")
    );
    assert_eq!(kaggle::url_key("https://www.kaggle.com/competitions"), None);
    assert_eq!(kaggle::url_key("https://www.kaggle.com/competitions?listOption=active"), None);
    assert_eq!(kaggle::url_key("https://www.kaggle.com/datasets/x"), None);
    assert_eq!(kaggle::url_key("https://eurekadev.devpost.com/"), None);
}
