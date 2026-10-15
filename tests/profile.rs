use std::fs;

use harness::profile::{self, ELIGIBILITY, PERSONAL};

const PROFILE: &str = "# 참가자 프로필

설명 문단.

## 참가 자격

- 학생 여부: 예
- 거주 지역: 서울

## 제출용 개인정보

- 이름: 홍길동
- 휴대전화: 010-0000-0000
";

#[test]
fn eligibility_is_only_the_eligibility_section() {
    let eligibility = profile::eligibility(PROFILE).unwrap();
    assert_eq!(eligibility, "- 학생 여부: 예\n- 거주 지역: 서울");
    assert!(!eligibility.contains("홍길동") && !eligibility.contains("010"));
    assert_eq!(profile::section(PROFILE, PERSONAL).unwrap(), "- 이름: 홍길동\n- 휴대전화: 010-0000-0000");
}

#[test]
fn english_headings_work_and_missing_sections_are_errors() {
    let english = "## Eligibility\n\n- Student: yes\n\n## Submission details\n\n- Name: Jane\n";
    assert_eq!(profile::eligibility(english).unwrap(), "- Student: yes");
    assert!(profile::eligibility("# 프로필\n\n## 제출용 개인정보\n- 이름: 홍길동\n").is_err());
    assert!(
        profile::eligibility("## 참가 자격\n\n## 제출용 개인정보\n- 이름: 홍길동\n").is_err(),
        "an empty section is an error"
    );
    assert_eq!(
        profile::section("## 참가 자격 (게이트용)\n- 국적: 대한민국\n", ELIGIBILITY).unwrap(),
        "- 국적: 대한민국"
    );
}

#[test]
fn missing_profile_file_says_how_to_start() {
    let path = std::env::temp_dir().join(format!("no-profile-{}.md", std::process::id()));
    let _ = fs::remove_file(&path);
    let err = profile::read_at(&path).unwrap_err().to_string();
    assert!(err.contains("profile.example.md"), "{err}");
}
