//! Submission ledger: add/update, manual entries, the registry import rule, dedupe and route refusal.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use harness::config::{self, Config};
use harness::dedupe::{self, Query};
use harness::ledger::{self, Ledger, Manual, Submission};
use harness::registry::{Entry, HistoryItem, NewEntry, Registry, apply_gate, new_entry};
use harness::route;
use serde_json::{Map, Value, json};

fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "harness-ledger-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn config_at(root: &std::path::Path) -> Config {
    serde_json::from_value(config::merge(config::defaults(), json!({"contests_root": root.to_string_lossy()}))).unwrap()
}

fn entry(id: &str, title: &str, url: &str) -> Entry {
    new_entry(NewEntry {
        id,
        title,
        url,
        source: "manual",
        organizer: "",
        deadline: None,
        categories: vec![],
        note: None,
    })
}

fn closed(id: &str, title: &str, note: &str) -> Entry {
    let mut e = entry(id, title, "");
    e.state = "closed".into();
    e.history.push(HistoryItem {
        at: "2026-10-15T14:43+09:00".into(),
        from: Some("discovered".into()),
        to: "closed".into(),
        by: "t".into(),
        note: note.into(),
    });
    e
}

fn item(key: &str, title: &str, url: &str) -> Submission {
    Submission {
        key: key.into(),
        id: Some(key.into()),
        title: title.into(),
        organizer: String::new(),
        url: url.into(),
        alt_urls: vec![],
        folder: None,
        submitted_at: "2026-10-14".into(),
        method: "웹 폼".into(),
        receipt: "접수 메일".into(),
        files: vec![],
        note: String::new(),
        source: "manual".into(),
        extra: Map::new(),
    }
}

fn registry(entries: Vec<Entry>) -> Registry {
    let mut reg = Registry::default();
    for e in entries {
        reg.contests.insert(e.id.clone(), e);
    }
    reg
}

#[test]
fn upsert_adds_then_replaces_the_same_key() {
    let mut l = Ledger::default();
    assert!(l.upsert(item("a", "가 공모전", "")));
    let mut changed = item("a", "가 공모전", "");
    changed.receipt = "접수번호 12".into();
    assert!(!l.upsert(changed));
    assert!(l.upsert(item("b", "나 공모전", "")));
    assert_eq!(l.submissions.len(), 2);
    assert_eq!(l.get("a").unwrap().receipt, "접수번호 12");
}

#[test]
fn manual_items_get_a_slug_key_and_a_checked_date() {
    let base = Manual { title: "2026 모바일신분증 아이디어 공모전", ..Default::default() };
    let made = ledger::manual(Manual { date: Some("2026-10-14"), ..base }).unwrap();
    assert_eq!(made.key, "manual-2026-모바일신분증-아이디어-공모전");
    assert_eq!((made.source.as_str(), made.id.as_ref()), ("manual", None));
    assert_eq!(made.submitted_at, "2026-10-14");
    assert!(ledger::manual(Manual { date: Some("2026-10-14T13:10+09:00"), ..base }).is_ok());
    for bad in ["2026-13-01", "10/14", "2026-10-14 13:10", "2026-10-14T13:10Z"] {
        assert!(ledger::manual(Manual { date: Some(bad), ..base }).is_err(), "{bad}");
    }
    assert!(ledger::manual(Manual { title: "  ", ..base }).is_err());
    let now = ledger::manual(base).unwrap().submitted_at;
    assert!(ledger::valid_stamp(&now) && now.len() == 22, "{now}");
}

#[test]
fn import_takes_submitted_and_closed_entries_that_say_submitted() {
    let mut harnessed = entry("wevity-1", "해양 공모전", "https://e.com/1");
    harnessed.state = "submitted".into();
    harnessed
        .submissions
        .push(json!({"at": "2026-10-13T10:00+09:00", "method": "이메일", "receipt": "회신", "files": ["a.pdf"]}));
    harnessed.history.push(HistoryItem {
        at: "2026-10-13T10:00+09:00".into(),
        from: Some("awaiting_approval".into()),
        to: "submitted".into(),
        by: "t".into(),
        note: "회신".into(),
    });
    let reg = registry(vec![
        harnessed,
        closed("legacy-(2026-10-14 추가) Guro-Shorts", "구로구 유튜브 쇼츠 제출 완료", "이미 다룬 공모전"),
        closed("legacy-Paper", "논문 공모전", "2026-10-12 이메일로 제출함"),
        closed("legacy-Traffic", "교통안전 숏폼 — 중복, 미제출", ""),
        closed("legacy-Marine", "해양안전 공모전 — 제출 준비 완료", ""),
        closed("legacy-Naver", "AI 광고제 — 제출 패키지 완성, 업로드·폼 제출 대기", ""),
        closed("legacy-Old", "옛 공모전", "이미 다룬 공모전(결과 미확인)"),
        entry("manual-run", "진행 중 제출물 준비", ""),
    ]);
    let mut l = Ledger::default();
    l.upsert(item("legacy-Paper", "논문 공모전", ""));
    let report = l.import(&reg);
    let keys: Vec<&str> = report.added.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["wevity-1", "legacy-(2026-10-14 추가) Guro-Shorts"]);
    assert_eq!(report.skipped, ["legacy-Paper"]);
    let first = &report.added[0];
    assert_eq!((first.method.as_str(), first.receipt.as_str()), ("이메일", "회신"));
    assert_eq!((first.submitted_at.as_str(), first.source.as_str()), ("2026-10-13T10:00+09:00", "import"));
    assert_eq!(first.files, ["a.pdf"]);
    assert_eq!(report.added[1].submitted_at, "2026-10-14");

    let fresh = Ledger::default().import(&reg);
    let paper = fresh.added.iter().find(|s| s.key == "legacy-Paper").unwrap();
    assert_eq!((paper.submitted_at.as_str(), paper.note.as_str()), ("2026-10-12", "2026-10-12 이메일로 제출함"));
    assert_eq!(fresh.added.len(), 3);
    assert!(l.import(&reg).added.is_empty());
}

#[test]
fn ledger_round_trips_under_its_own_lock() {
    let dir = temp_dir("io");
    let path = dir.join("submissions.json");
    assert!(ledger::load_at(&path).unwrap().submissions.is_empty());
    ledger::write_at(&path, |l| Ok(l.upsert(item("a", "가", "")))).unwrap();
    assert!(dir.join("submissions.lock").exists());
    let saved: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["version"], 1);
    assert_eq!(saved["submissions"][0]["key"], "a");
    assert_eq!(ledger::load_at(&path).unwrap().submissions[0], item("a", "가", ""));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dedupe_reports_ledger_matches_first() {
    let root = temp_dir("dedupe");
    let cfg = config_at(&root);
    let reg = registry(vec![entry("wevity-9", "2026 모바일신분증 아이디어 공모전", "https://e.com/9")]);
    let mut l = Ledger::default();
    l.upsert(item("manual-mobile", "제2회 모바일신분증 아이디어 공모전", ""));
    let mut by_url = item("contest-7", "전혀 다른 이름", "https://www.example.org/notice/7/");
    by_url.alt_urls.push("https://mirror.example.net/7".into());
    l.upsert(by_url);
    let query = |title, url, exclude| Query {
        title,
        url,
        include_folders: false,
        exclude_id: exclude,
        strict: false,
        organizer: "",
        deadline: None,
        submitted: Some(&l),
    };
    let hits = dedupe::find_matches(&reg, &cfg, &query("2026 모바일신분증 아이디어 공모전", "", None));
    assert_eq!(
        hits[0],
        json!({"kind": "submitted", "id": "manual-mobile", "title": "제2회 모바일신분증 아이디어 공모전", "submitted_at": "2026-10-14", "by": "title"})
    );
    assert_eq!(hits[1]["kind"], "registry");
    let hits = dedupe::find_matches(&reg, &cfg, &query("새 이름", "http://example.org/notice/7", None));
    assert_eq!((hits[0]["id"].as_str(), hits[0]["by"].as_str()), (Some("contest-7"), Some("url")));
    let hits = dedupe::find_matches(&reg, &cfg, &query("새 이름", "https://mirror.example.net/7", None));
    assert_eq!(hits[0]["id"], "contest-7");
    let hits = dedupe::find_matches(&reg, &cfg, &query("새 이름", "https://mirror.example.net/7", Some("contest-7")));
    assert!(hits.is_empty());
    let without = Query { submitted: None, ..query("모바일신분증 아이디어 공모전", "", None) };
    assert!(dedupe::find_matches(&reg, &cfg, &without).iter().all(|h| h["kind"] != "submitted"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn route_refuses_a_contest_already_submitted() {
    let root = temp_dir("route");
    let cfg = config_at(&root);
    let mut e = entry("wevity-5", "2026 바다시 30초의 발견 공모전", "https://e.com/5");
    let gate = json!({"decision": "pass", "type": "video", "level": 1, "folder": "Bada-Shorts",
        "deliverables": [{"name": "영상"}], "reasons": ["근거"]});
    apply_gate(&mut e, gate, "ContestGate").unwrap();
    let mut l = Ledger::default();
    let mut done = item("legacy-bada", "바다시 30초의 발견", "");
    done.submitted_at = "2026-10-10T18:00+09:00".into();
    l.upsert(done);
    let err = route::route(&mut e, &cfg, &l, None, false, false).unwrap_err();
    assert_eq!(err.to_string(), "이미 제출한 공모전이다: 바다시 30초의 발견 (2026-10-10T18:00+09:00)");
    assert_eq!(e.state, "gate_pass");
    assert!(!root.join("Bada-Shorts").exists());
    route::route(&mut e, &cfg, &Ledger::default(), None, false, false).unwrap();
    assert_eq!(e.state, "routed");
    fs::remove_dir_all(&root).unwrap();
}
