//! Registry, gate, dedupe, collect, budget/capacity, board and route behaviour (ported from tests/test_harness.py).

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::NaiveDate;
use harness::budget::{self, Window};
use harness::config::{self, Config};
use harness::dedupe::{self, Query};
use harness::error::{Error, Result};
use harness::ledger::Ledger;
use harness::registry::{self, Entry, NewEntry, Registry, apply_gate, new_entry, transition};
use harness::sources::{Item, Source};
use harness::{board, collect, route};
use serde_json::{Value, json};

fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "harness-core-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn config_with(overlay: Value) -> Config {
    serde_json::from_value(config::merge(config::defaults(), overlay)).unwrap()
}

fn config_at(root: &std::path::Path) -> Config {
    config_with(json!({"contests_root": root.to_string_lossy()}))
}

fn entry(id: &str, title: &str, url: &str, source: &str) -> Entry {
    new_entry(NewEntry { id, title, url, source, organizer: "", deadline: None, categories: vec![], note: None })
}

fn gate(decision: &str, extra: Value) -> Value {
    let mut base = json!({"decision": decision, "type": "idea", "level": 2, "priority": 70,
        "deliverables": [{"name": "제안서", "format": "hwp", "limits": ""}],
        "criteria": [{"stage": "예비", "name": "참신성", "weight": 30}], "reasons": ["근거"]});
    for (k, v) in extra.as_object().unwrap() {
        base[k] = v.clone();
    }
    base
}

fn apply(e: &mut Entry, g: Value) -> Result<()> {
    apply_gate(e, g, "ContestGate")
}

#[test]
fn transition_rejects_skipping_gate_and_approval() {
    let mut e = entry("x-1", "공모전", "https://e.com", "manual");
    assert!(transition(&mut e, "routed", "t", "").is_err());
    apply(&mut e, gate("pass", json!({}))).unwrap();
    transition(&mut e, "routed", "t", "").unwrap();
    transition(&mut e, "running", "t", "").unwrap();
    assert!(transition(&mut e, "submitted", "t", "").is_err());
    transition(&mut e, "awaiting_approval", "t", "").unwrap();
    transition(&mut e, "submitted", "t", "").unwrap();
    let path: Vec<&str> = e.history.iter().map(|h| h.to.as_str()).collect();
    assert_eq!(path, ["discovered", "gate_pass", "routed", "running", "awaiting_approval", "submitted"]);
}

#[test]
fn gate_decisions_map_to_states_and_validate_fields() {
    let mut failed = entry("x-2", "공모전", "https://e.com", "manual");
    apply(&mut failed, gate("fail", json!({"reasons": ["지역 한정"]}))).unwrap();
    assert_eq!(failed.state, "rejected");
    let mut held = entry("x-3", "공모전", "https://e.com", "manual");
    apply(&mut held, gate("hold", json!({}))).unwrap();
    assert_eq!(held.state, "hold");
    let mut bad = entry("x-4", "공모전", "https://e.com", "manual");
    assert!(apply(&mut bad, gate("pass", json!({"level": 6}))).is_err());
    assert!(apply(&mut bad, gate("pass", json!({"level": "3"}))).is_err());
    assert!(apply(&mut bad, gate("pass", json!({"deliverables": [], "criteria": []}))).is_err());
}

#[test]
fn gate_cannot_be_rewritten_once_running() {
    let mut e = entry("x-5", "공모전", "https://e.com", "manual");
    apply(&mut e, gate("pass", json!({}))).unwrap();
    transition(&mut e, "routed", "t", "").unwrap();
    transition(&mut e, "running", "t", "").unwrap();
    assert!(apply(&mut e, gate("fail", json!({}))).is_err());
}

#[test]
fn level_5_needs_approved_plan_review_before_submission_approval() {
    let mut e = entry("x-6", "프로덕션 해커톤", "https://e.com", "manual");
    apply(&mut e, gate("pass", json!({"type": "software", "level": 5}))).unwrap();
    transition(&mut e, "routed", "t", "").unwrap();
    transition(&mut e, "running", "t", "").unwrap();
    assert!(transition(&mut e, "awaiting_approval", "t", "").is_err());
    transition(&mut e, "plan_review", "t", "레퍼런스·SPEC·PLAN 검수 요청").unwrap();
    assert!(transition(&mut e, "running", "t", "").is_err());
    transition(&mut e, "running", "t", "승인: '진행해' 2026-10-16 09:00").unwrap();
    transition(&mut e, "awaiting_approval", "t", "").unwrap();
    let mut small = entry("x-7", "정책 서비스", "https://e.com", "manual");
    apply(&mut small, gate("pass", json!({"type": "software", "level": 3}))).unwrap();
    for state in ["routed", "running", "awaiting_approval"] {
        transition(&mut small, state, "t", "").unwrap();
    }
}

#[test]
fn gate_official_url_replaces_aggregator_url() {
    let mut e = entry("wevity-1", "공모전", "https://www.wevity.com/?ix=1", "wevity");
    apply(&mut e, gate("pass", json!({"official_url": "https://contest.go.kr/notice/9"}))).unwrap();
    assert_eq!(e.url, "https://contest.go.kr/notice/9");
    assert!(e.alt_urls.contains(&"https://www.wevity.com/?ix=1".to_string()));
}

fn query<'a>(title: &'a str, url: &'a str) -> Query<'a> {
    Query {
        title,
        url,
        include_folders: true,
        exclude_id: None,
        strict: false,
        organizer: "",
        deadline: None,
        submitted: None,
    }
}

fn ids(matches: &[Value]) -> Vec<String> {
    matches.iter().map(|m| m["id"].as_str().unwrap().to_string()).collect()
}

#[test]
fn dedupe_finds_folder_by_organizer_and_deadline_when_titles_differ() {
    let root = temp_dir("orgdeadline");
    let folder = root.join("Expressway-RestArea-Shorts");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("README.md"),
        "# 2026 한국도로공사 휴게소 숏폼 공모전 「무엇이든 그려보길」\n\n- 마감: 요강 11/4(수)\n",
    )
    .unwrap();
    let cfg = config_at(&root);
    let reg = Registry::default();
    let title = "고속도로 숏폼 공모전 : 고속도로 휴게소 1분 로드";
    let q =
        |deadline, strict| Query {
            organizer: "한국도로공사", deadline: Some(deadline), strict, ..query(title, "")
        };
    let hits = dedupe::find_matches(&reg, &cfg, &q("2026-11-04T23:59+09:00", false));
    let pairs: Vec<(&str, &str)> =
        hits.iter().map(|h| (h["id"].as_str().unwrap(), h["by"].as_str().unwrap())).collect();
    assert_eq!(pairs, [("Expressway-RestArea-Shorts", "organizer+deadline")]);
    assert_eq!(
        hits[0],
        json!({"kind": "folder", "id": "Expressway-RestArea-Shorts", "title": "", "by": "organizer+deadline"})
    );
    assert!(dedupe::find_matches(&reg, &cfg, &q("2026-11-05T23:59+09:00", false)).is_empty());
    assert!(dedupe::find_matches(&reg, &cfg, &q("2026-11-04T23:59+09:00", true)).is_empty());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn dedupe_matches_folder_readme_and_existing_list_titles() {
    let root = temp_dir("foldertitles");
    fs::create_dir_all(root.join("Maeum-Shorts")).unwrap();
    fs::write(root.join("Maeum-Shorts/README.md"), "\n## 2026 마음동행 숏폼 공모전 함께 100초 챌린지\n").unwrap();
    fs::create_dir_all(root.join(".shared")).unwrap();
    fs::write(root.join(".shared/existing_contests.md"), "# 목록\n- Old-Contest: 여수세계섬박람회 사진 공모전\n")
        .unwrap();
    let cfg = config_at(&root);
    let reg = Registry::default();
    let hits = dedupe::find_matches(&reg, &cfg, &query("[교육부] 마음동행 숏폼 공모전「함께 100초 챌린지」", ""));
    assert_eq!(
        hits,
        [
            json!({"kind": "folder", "id": "Maeum-Shorts", "title": "2026 마음동행 숏폼 공모전 함께 100초 챌린지", "by": "title"})
        ]
    );
    let hits = dedupe::find_matches(&reg, &cfg, &query("2026 여수세계섬박람회 사진 공모전", ""));
    assert_eq!(ids(&hits), ["Old-Contest"]);
    assert_eq!(hits[0]["kind"], "existing");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn two_listings_gated_at_once_mark_only_the_later_one_duplicate() {
    let cfg = config_at(&temp_dir("twolistings"));
    let mut first =
        entry("linkareer-1", "2026 여수세계섬박람회 사진 공모전", "https://linkareer.com/activity/1", "linkareer");
    let mut second = entry("wevity-2", "여수세계섬박람회 사진 공모전", "https://www.wevity.com/?ix=2", "wevity");
    first.discovered_at = "2026-10-15T14:44+09:00".into();
    second.discovered_at = "2026-10-15T14:44+09:00".into();
    let (t1, u1, t2, u2) = (first.title.clone(), first.url.clone(), second.title.clone(), second.url.clone());
    let mut reg = Registry::default();
    reg.contests.insert(first.id.clone(), first);
    reg.contests.insert(second.id.clone(), second);
    let check = |reg: &Registry, title: &str, url: &str, own: &str| {
        let q = Query { include_folders: false, exclude_id: Some(own), ..query(title, url) };
        ids(&dedupe::find_matches(reg, &cfg, &q))
    };
    assert!(check(&reg, &t1, &u1, "linkareer-1").is_empty());
    assert_eq!(check(&reg, &t2, &u2, "wevity-2"), ["linkareer-1"]);
    reg.contests["linkareer-1"].state = "hold".into();
    assert!(check(&reg, &t1, &u1, "linkareer-1").is_empty());
    assert_eq!(check(&reg, &t2, &u2, "wevity-2"), ["linkareer-1"]);
    reg.contests["linkareer-1"].state = "discovered".into();
    reg.contests["wevity-2"].state = "duplicate".into();
    assert!(check(&reg, &t1, &u1, "linkareer-1").is_empty());
}

#[test]
fn titles_match_across_tags_years_and_brackets() {
    let a = "[교육부] 마음동행 숏폼 공모전「함께 100초 챌린지」";
    let b = "2026 마음동행 숏폼 공모전 함께 100초 챌린지";
    assert!(dedupe::similar(a, b, false));
    assert!(dedupe::similar(a, b, true));
    assert_eq!(dedupe::normalize_title(a), "마음동행숏폼함께100초");
}

#[test]
fn strict_mode_does_not_merge_short_generic_titles() {
    assert!(!dedupe::similar("AI 숏폼 공모전", "AI 숏폼 챌린지 (구로구)", true));
    assert!(!dedupe::similar("서울 29초 영화제", "부산 29초 영화제", true));
}

#[test]
fn url_keys_identify_same_listing() {
    assert_eq!(
        dedupe::normalize_url("https://www.wevity.com/?c=find&s=1&gub=1&gbn=viewok&gp=1&ix=111674"),
        "wevity:111674"
    );
    assert_eq!(dedupe::normalize_url("https://wevity.com/?ix=111674&c=find"), "wevity:111674");
    assert_eq!(dedupe::normalize_url("https://linkareer.com/activity/354786/"), "linkareer:354786");
    assert_eq!(dedupe::normalize_url("https://WWW.Example.com/a/b/?x=1#top"), "example.com/a/b?x=1");
    assert_eq!(dedupe::normalize_url(""), "");
}

fn good(_: u32, _: &HashSet<String>) -> Result<Vec<Item>> {
    Ok(vec![Item {
        entry_id: "good-1".into(),
        title: "멀쩡한 출처의 아주 긴 공모전 제목".into(),
        url: "https://good.example/1".into(),
        organizer: "주최".into(),
        deadline: None,
        categories: vec![],
        note: None,
        alt_url: None,
    }])
}

fn mirror(_: u32, _: &HashSet<String>) -> Result<Vec<Item>> {
    Ok(vec![
        Item {
            entry_id: "mirror-9".into(),
            title: "2026 멀쩡한 출처의 아주 긴 공모전 제목".into(),
            url: "https://mirror.example/9".into(),
            alt_url: Some("https://official.example/notice".into()),
            ..Item::default()
        },
        Item {
            entry_id: "good-1".into(),
            title: "멀쩡한 출처의 아주 긴 공모전 제목".into(),
            deadline: Some("2026-11-04T23:59+09:00".into()),
            ..Item::default()
        },
        Item {
            entry_id: "mirror-10".into(),
            title: "고래 보호 해양 사진 공모전 2026".into(),
            url: "https://mirror.example/10".into(),
            ..Item::default()
        },
    ])
}

fn empty(_: u32, _: &HashSet<String>) -> Result<Vec<Item>> {
    Ok(vec![])
}

fn boom(_: u32, _: &HashSet<String>) -> Result<Vec<Item>> {
    Err(Error("503".into()))
}

fn no_key(_: &str) -> Option<String> {
    None
}

static FAKE: &[Source] = &[
    Source { name: "good", collect: good, url_key: no_key },
    Source { name: "empty", collect: empty, url_key: no_key },
    Source { name: "broken", collect: boom, url_key: no_key },
];

#[test]
fn collect_keeps_going_when_one_source_fails_or_returns_nothing() {
    let root = temp_dir("collect");
    let cfg = config_at(&root);
    let reg_path = root.join("registry.json");
    let report = collect::collect_from(&reg_path, &cfg, FAKE, "all", 1).unwrap();
    assert_eq!(report.added, ["good-1"]);
    assert_eq!(report.seen.iter().map(|(k, v)| (k.as_str(), *v)).collect::<Vec<_>>(), [("good", 1), ("empty", 0)]);
    let mut kinds: Vec<&str> = report.errors.iter().map(|e| e.split(':').next().unwrap()).collect();
    kinds.sort();
    assert_eq!(kinds, ["broken", "empty"]);
    assert_eq!(report.new.len(), 1);
    assert!(registry::read_at(&reg_path, |reg| Ok(reg.contests.contains_key("good-1"))).unwrap());
    let skipped = config_with(json!({"contests_root": root.to_string_lossy(), "collect_skip": ["broken", "empty"]}));
    let report = collect::collect_from(&reg_path, &skipped, FAKE, "all", 1).unwrap();
    assert!(report.errors.is_empty() && report.added.is_empty());
    assert!(collect::collect_from(&reg_path, &cfg, FAKE, "nope", 1).is_err());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn collect_merges_same_listing_fills_deadline_and_marks_folder_duplicates() {
    let root = temp_dir("collectmerge");
    fs::create_dir_all(root.join("Whale-Photo")).unwrap();
    fs::write(root.join("Whale-Photo/README.md"), "# 고래 보호 해양 사진 공모전\n").unwrap();
    let cfg = config_at(&root);
    let reg_path = root.join("registry.json");
    collect::collect_from(&reg_path, &cfg, FAKE, "good", 1).unwrap();
    static MIRROR: &[Source] = &[Source { name: "mirror", collect: mirror, url_key: no_key }];
    let report = collect::collect_from(&reg_path, &cfg, MIRROR, "all", 1).unwrap();
    assert_eq!(report.merged, [json!({"id": "good-1", "from": "mirror-9"})]);
    assert_eq!(report.duplicates, ["mirror-10"]);
    assert!(report.added.is_empty());
    registry::read_at(&reg_path, |reg| {
        let good = &reg.contests["good-1"];
        assert_eq!(good.alt_urls, ["https://mirror.example/9", "https://official.example/notice"]);
        assert_eq!(good.deadline.as_deref(), Some("2026-11-04T23:59+09:00"));
        let dup = &reg.contests["mirror-10"];
        assert_eq!(dup.state, "duplicate");
        assert_eq!(dup.source, "mirror");
        assert_eq!(dup.history.last().unwrap().note, "기존 폴더와 같음: Whale-Photo");
        Ok(())
    })
    .unwrap();
    fs::remove_dir_all(&root).unwrap();
}

fn window(remaining: f64, time_left: f64) -> Window {
    Window {
        provider: "anthropic".into(),
        window: "7d".into(),
        remaining,
        time_left,
        pace: ((remaining / time_left) * 100.0).round() / 100.0,
        resets_at: 0,
        status: Some("ok".into()),
    }
}

#[test]
fn budget_widens_when_quota_would_go_unused_and_throttles_near_limit() {
    let rec = |w: Window| budget::recommend(&[w], 6)["anthropic"].clone();
    assert_eq!(rec(window(0.8, 0.4))["mode"], "maxing");
    assert_eq!(rec(window(0.8, 0.4))["workers"], 6);
    assert_eq!(rec(window(0.5, 0.45))["mode"], "steady");
    assert_eq!(rec(window(0.2, 0.6))["mode"], "conserve");
    let low = rec(window(0.05, 0.5));
    assert_eq!((low["workers"].clone(), low["mode"].clone()), (json!(1), json!("throttle")));
}

#[test]
fn budget_uses_tightest_window_per_provider() {
    let rec = budget::recommend(&[window(0.9, 0.5), window(0.2, 0.9)], 6);
    assert_eq!(rec["anthropic"]["tightest"]["remaining"], 0.2);
    assert_eq!(rec["anthropic"]["mode"], "conserve");
}

#[test]
fn windows_compute_time_left_from_reset() {
    let report = json!({"reports": [{"provider": "anthropic", "limits": [{"label": "Claude 5 Hour", "status": "ok",
        "amount": {"remainingFraction": 0.5}, "window": {"durationMs": 1000, "resetsAt": 1500}}]}]});
    let wins = budget::windows(&report, 1000);
    assert_eq!(wins.len(), 1);
    assert_eq!((wins[0].time_left, wins[0].pace), (0.5, 1.0));
    assert_eq!(wins[0].window, "Claude 5 Hour");
}

fn running(counts: [usize; 5]) -> BTreeMap<u8, usize> {
    (1..=5).zip(counts).collect()
}

fn capacity(free_gb: f64, run: [usize; 5], pressure: &str, report: &Value) -> Value {
    let cfg = config_with(json!({"max_running": 4, "ram_reserve_gb": 8,
        "max_running_by_level": {"1": 3, "2": 1, "3": 2, "4": 1, "5": 0}}));
    budget::capacity_with(&cfg, &running(run), report, free_gb, pressure)
}

fn slots(cap: &Value) -> Vec<u64> {
    (1..=5).map(|l| cap["slots_by_level"][l.to_string()].as_u64().unwrap()).collect()
}

#[test]
fn capacity_caps_each_level_by_count_and_memory_budget() {
    let none = json!({"reports": []});
    let roomy = capacity(20.0, [1, 1, 0, 0, 0], "normal", &none);
    assert_eq!(roomy["ram"]["budget_gb"], 12.0);
    assert_eq!(roomy["slots"], 2);
    assert_eq!(slots(&roomy), [2, 0, 2, 1, 0]);
    let tight = capacity(13.0, [1, 1, 0, 0, 0], "normal", &none);
    assert_eq!(slots(&tight), [2, 0, 1, 0, 0]);
    // Without usage data: no providers, unknown basis, token workers fall back to max_workers.
    assert_eq!(roomy["providers"], json!({}));
    assert_eq!(roomy["limits"]["basis"], "unknown");
    assert_eq!(roomy["limits"]["token_workers"], 6);
    let keys: Vec<&String> = roomy.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        ["slots", "slots_by_level", "recommended_workers", "running", "running_by_level", "ram", "limits", "providers"]
    );
    assert_eq!(roomy["running_by_level"], json!({"1": 1, "2": 1, "3": 0, "4": 0, "5": 0}));
    assert_eq!(roomy["limits"]["ram_gb_by_level"], json!({"1": 2, "2": 3, "3": 4, "4": 6, "5": 8}));
    assert_eq!(roomy["ram"]["reserve_gb"], json!(8));
}

#[test]
fn capacity_stops_on_memory_pressure_and_bounds_workers_by_memory() {
    let none = json!({"reports": []});
    assert_eq!(slots(&capacity(40.0, [0; 5], "warning", &none)), [0; 5]);
    let now = chrono::Utc::now().timestamp_millis();
    let maxing = json!({"reports": [{"provider": "anthropic", "limits": [{"label": "7d", "status": "ok",
        "amount": {"remainingFraction": 0.8}, "window": {"durationMs": 1_000_000_000i64, "resetsAt": now + 400_000_000}}]}]});
    let cap = capacity(20.0, [0; 5], "normal", &maxing);
    assert_eq!(cap["recommended_workers"], 6);
    assert_eq!(cap["limits"]["basis"], "anthropic");
    assert_eq!(cap["providers"]["anthropic"]["mode"], "maxing");
    assert_eq!(capacity(11.0, [0; 5], "normal", &maxing)["recommended_workers"], 2);
}

#[test]
fn config_override_keeps_unlisted_level_defaults() {
    let cfg = config_with(json!({"max_running_by_level": {"5": 0}}));
    assert_eq!(cfg.max_running_by_level, BTreeMap::from([(1, 3), (2, 2), (3, 2), (4, 1), (5, 0)]));
}

#[test]
fn vm_stat_parses_reusable_pages() {
    let out = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPages free:  65536.\nPages active: 1.\nPages inactive:  65536.\nPages speculative:  0.\n";
    assert_eq!(budget::parse_vm_stat(out), 2.0);
}

#[test]
fn board_groups_states_and_formats_rows() {
    let today = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
    assert_eq!(board::dday(Some("2026-10-18T23:59+09:00"), today), "D-3");
    assert_eq!(board::dday(Some("2026-10-15T24:00+09:00"), today), "D-day");
    assert_eq!(board::dday(Some("2026-10-13"), today), "D+2");
    assert_eq!(board::dday(None, today), "-");
    assert_eq!(board::dday(Some("미정"), today), "미정");

    let mut reg = Registry::default();
    let mut review = entry("x-review", "프로덕션 | 해커톤", "https://e.com/r", "manual");
    apply(&mut review, gate("pass", json!({"type": "software", "level": 5}))).unwrap();
    for state in ["routed", "running", "plan_review"] {
        transition(&mut review, state, "t", "검수").unwrap();
    }
    review.folder = Some("/tmp/contests/Prod-Hack".into());
    review.deadline = Some("2026-10-18T23:59+09:00".into());
    reg.contests.insert(review.id.clone(), review);
    let mut l3 = entry("x-l3", "정책 서비스", "", "manual");
    apply(&mut l3, gate("pass", json!({"type": "software", "level": 3}))).unwrap();
    reg.contests.insert(l3.id.clone(), l3);
    for i in 0..45 {
        let id = format!("d-{i:02}");
        reg.contests.insert(id.clone(), entry(&id, "새 공고", "", "manual"));
    }
    let text = board::render_on(&reg, today);
    assert!(text.starts_with("# 공모전 보드\n\n갱신: "));
    assert!(text.contains("## 레벨 5 계획 검수 대기 (1)\n\n| 마감 | 공모전 | 유형·레벨 | 폴더 | id |\n|---|---|---|---|---|\n| D-3 | [프로덕션 \\| 해커톤](https://e.com/r) | software / L5 | Prod-Hack | `x-review` |\n"));
    assert!(text.contains("| - | 정책 서비스 | software / L3 | - | `x-l3` |"));
    assert!(text.contains("## 제출 승인 대기 (0)\n\n(없음)\n"));
    assert!(text.contains("## 새로 발견(게이트 전) (45)"));
    assert_eq!(text.matches("| 새 공고 |").count(), 40);
    assert!(text.ends_with("그 밖: 종료 0, 게이트 탈락 0, 하지 않기로 함 0, 중복 0.\n"));
    let order: Vec<usize> = [
        "## 제출 승인 대기",
        "## 레벨 5 계획",
        "## 진행 중",
        "## 라우팅됨",
        "## 게이트 통과",
        "## 보류",
        "## 새로 발견",
        "## 제출 완료",
    ]
    .iter()
    .map(|h| text.find(h).unwrap())
    .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn route_creates_folder_documents_without_repo_or_workspace() {
    let root = temp_dir("route");
    let cfg = config_at(&root);
    let mut e = entry("wevity-77", "마음동행 숏폼 공모전", "https://www.wevity.com/?ix=77", "wevity");
    e.organizer = "교육부".into();
    apply(&mut e, gate("pass", json!({"folder": "Maeum-Shorts", "prize": "대상 300만원",
        "eligibility": {"individual_ok": true, "region": "전국"}, "submission": {"방법": "이메일"}, "questions": ["팀 구성?"]})))
    .unwrap();
    let not_ready = entry("x-9", "제목", "", "manual");
    let none = Ledger::default();
    assert!(route::route(&mut not_ready.clone(), &cfg, &none, None, false, false).is_err());
    assert!(route::route(&mut e.clone(), &cfg, &none, Some("bad name!"), false, false).is_err());

    let summary = route::route(&mut e, &cfg, &none, None, false, false).unwrap();
    let target = root.join("Maeum-Shorts");
    assert_eq!(summary, json!({"folder": target.to_string_lossy(), "repo": null, "workspace_id": null}));
    assert_eq!(e.state, "routed");
    assert_eq!(e.folder.as_deref(), Some(target.to_string_lossy().as_ref()));
    assert!(e.session.is_none());
    let contest = fs::read_to_string(target.join("CONTEST.md")).unwrap();
    assert!(contest.starts_with("# 마음동행 숏폼 공모전\n\n| 항목 | 내용 |\n|---|---|\n| 주최·주관 | 교육부 |\n"));
    assert!(contest.contains("| 유형·레벨 | idea / 레벨 2 |"));
    assert!(contest.contains("| 개인 참가 | 예 |\n| 지역 제한 | 전국 |"));
    assert!(contest.contains("| 제안서 | hwp | - |"));
    assert!(contest.contains("| 예비 | 참신성 | 30 |"));
    assert!(contest.contains("## 판정 근거\n\n- 근거\n"));
    assert!(contest.contains("## 사용자에게 확인할 것\n\n- 팀 구성?\n"));
    assert!(fs::read_to_string(target.join("README.md")).unwrap().contains("- 유형·레벨: idea / 레벨 2"));
    assert!(target.join(".gitignore").exists() && target.join(".git").exists());
    let gate_json: Value =
        serde_json::from_str(&fs::read_to_string(target.join(".harness/gate.json")).unwrap()).unwrap();
    assert_eq!(gate_json["folder"], "Maeum-Shorts");
    let saved: Value =
        serde_json::from_str(&fs::read_to_string(target.join(".harness/contest.json")).unwrap()).unwrap();
    assert_eq!((saved["id"].as_str(), saved["state"].as_str()), (Some("wevity-77"), Some("routed")));
    let existing = fs::read_to_string(cfg.existing_contests()).unwrap();
    assert_eq!(existing, "- Maeum-Shorts: 마음동행 숏폼 공모전\n");

    // Another contest cannot take over a folder that already has content.
    let mut other = entry("x-10", "다른 공모전", "", "manual");
    apply(&mut other, gate("pass", json!({"folder": "Maeum-Shorts"}))).unwrap();
    assert!(route::route(&mut other, &cfg, &none, None, false, false).is_err());
    fs::remove_dir_all(&root).unwrap();
}
