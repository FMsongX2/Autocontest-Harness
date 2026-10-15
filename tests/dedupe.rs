use harness::dedupe::similar;

#[test]
fn titles_differing_only_by_a_number_are_different_contests() {
    for strict in [false, true] {
        assert!(!similar("ARC Prize 2026 - ARC-AGI-2", "ARC Prize 2026 - ARC-AGI-3", strict));
        assert!(!similar("데이터 분석 챌린지 Track 1 이미지 분류", "데이터 분석 챌린지 Track 2 이미지 분류", strict));
        assert!(!similar("대학생 숏폼 영상 공모전 시즌2 모집", "대학생 숏폼 영상 공모전 시즌3 모집", strict));
        assert!(!similar("Fourth Fake Contest 2026 Track 4", "Fourth Fake Contest 2026 Track 5", strict));
    }
}

#[test]
fn years_editions_and_one_sided_numbers_do_not_split_a_contest() {
    // Year and edition differences are stripped before numbers are compared.
    assert!(similar("2026 제6회 별별화성 AI CF 공모전 ARC-AGI-2", "제7회 별별화성 AI CF 공모전 ARC-AGI-2", true));
    // A number on one side only (a prize amount) leaves the decision to the title core.
    assert!(similar(
        "화성특례시 AI CF 공모전 별별화성 Awards",
        "화성특례시 AI CF 공모전 별별화성 Awards 1,000만원",
        false
    ));
    assert!(similar("ARC Prize ARC-AGI-02", "ARC Prize ARC-AGI-2", true));
}

#[test]
fn cross_site_copies_of_one_contest_still_match_strictly() {
    let wevity = "2026 화성특례시 AI CF 공모전 - 제6회 별별화성 Awards";
    let linkareer = "[화성특례시] 2026 화성특례시 AI CF 공모전 제6회 별별화성 Awards";
    assert!(similar(wevity, linkareer, true));
    assert!(similar(linkareer, wevity, false));
}
