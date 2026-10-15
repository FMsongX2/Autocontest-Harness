use std::fs;
use std::process::Command;

use harness::guard::{Finding, scan};

fn terms() -> Vec<String> {
    ["보미", "헤헤", "선배"].iter().map(|t| t.to_string()).collect()
}

#[test]
fn finds_persona_in_deliverables_and_commit_messages_but_not_in_metadata_or_data() {
    let dir = std::env::temp_dir().join(format!("guard-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("docs")).unwrap();
    fs::create_dir_all(dir.join(".harness")).unwrap();
    fs::create_dir_all(dir.join("data")).unwrap();
    fs::write(dir.join("docs/report.md"), "# 제안서\n\n선배 이 표부터 보면 돼\n정책 효과를 분석한다.\n").unwrap();
    fs::write(dir.join("main.py"), "def run():\n    return 1  # 헤헤 끝\n").unwrap();
    fs::write(dir.join(".harness/launch-prompt.md"), "보미 지시문\n").unwrap(); // harness metadata
    fs::write(dir.join("data/names.csv"), "이름\n김보미\n").unwrap(); // raw dataset
    fs::write(dir.join("notes.md"), "다보미와 왕선배 이야기는 낱말 안이라 괜찮다\nHQ(보미)에게 알린다\n").unwrap();
    fs::create_dir_all(dir.join("poc/cache")).unwrap();
    fs::write(dir.join("poc/cache/law.json"), "\"보미 판단\"\n").unwrap(); // fetched cache
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .status()
            .unwrap();
        assert!(ok.success());
    };
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "Add report", "-m", "보미가 정리함"]);

    let findings = scan(&dir, &terms()).unwrap();
    let places: Vec<(String, usize, String)> =
        findings.iter().map(|f| (f.file.clone(), f.line, f.term.clone())).collect();
    let commit =
        places.iter().find(|(file, _, _)| file.starts_with("commit ")).cloned().expect("commit message finding");
    assert_eq!(commit.2, "보미");
    assert!(places.contains(&("docs/report.md".into(), 3, "선배".into())));
    assert!(places.contains(&("main.py".into(), 2, "헤헤".into())));
    assert!(places.contains(&("notes.md".into(), 2, "보미".into())));
    assert_eq!(places.len(), 4, "word-internal hits, caches, metadata, datasets and binaries are skipped: {places:?}");
}

#[test]
fn clean_folder_has_no_findings() {
    let dir = std::env::temp_dir().join(format!("guard-clean-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("report.md"), "# 결과 보고서\n\n분석 결과를 정리한다.\n").unwrap();
    assert_eq!(scan(&dir, &terms()).unwrap(), Vec::<Finding>::new());
    fs::remove_dir_all(&dir).unwrap();
}
