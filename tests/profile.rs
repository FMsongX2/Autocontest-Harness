//! Participant profile: field table, set validation, masking, eligibility, v1 upgrade, file safety.

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::NaiveDate;
use harness::profile::{self, FIELDS, Kind};
use serde_json::{Value, json};

fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "harness-profile-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn set(p: &mut Value, path: &std::path::Path, key: &str, raw: &str) -> Result<Value, String> {
    profile::set_field(p, path, key, raw).map_err(|e| e.to_string())
}

#[test]
fn fields_cover_the_v2_template() {
    let template = profile::template();
    for field in FIELDS {
        assert!(
            !profile::get(&template, field.key).is_null() || matches!(field.kind, Kind::Bool | Kind::Int),
            "{}",
            field.key
        );
    }
    let listed = serde_json::to_value(FIELDS).unwrap();
    assert_eq!(
        listed[0],
        json!({"key": "identity.name", "label": "이름", "kind": "text", "example": "홍길동", "sensitive": true})
    );
    let school = FIELDS.iter().find(|f| f.key == "affiliation.school").unwrap();
    assert!(!school.sensitive);
    assert!(profile::find_field("identity.nope").is_err());
}

#[test]
fn set_validates_each_kind() {
    let dir = temp_dir("set");
    let path = dir.join("profile.json");
    let mut p = profile::template();
    set(&mut p, &path, "identity.birth_date", "2001-03-15").unwrap();
    for bad in ["2001-3-15", "2001-02-30", "20010315", "2001-03-15T00:00"] {
        assert!(set(&mut p, &path, "identity.birth_date", bad).is_err(), "{bad}");
    }
    set(&mut p, &path, "contact.email", "a@example.com").unwrap();
    for bad in ["example.com", "@example.com", "a@"] {
        assert!(set(&mut p, &path, "contact.email", bad).is_err(), "{bad}");
    }
    set(&mut p, &path, "contact.phone", "+82-10-1234-5678").unwrap();
    for bad in ["010 1234 5678", "010-1234-567a", "---"] {
        assert!(set(&mut p, &path, "contact.phone", bad).is_err(), "{bad}");
    }
    for (raw, want) in [("예", true), ("아니오", false), ("true", true), ("FALSE", false)] {
        assert_eq!(set(&mut p, &path, "affiliation.is_student", raw).unwrap(), json!(want));
    }
    assert!(set(&mut p, &path, "affiliation.is_student", "maybe").is_err());
    assert_eq!(set(&mut p, &path, "affiliation.year", "3").unwrap(), json!(3));
    assert!(set(&mut p, &path, "affiliation.year", "-1").is_err());
    assert_eq!(set(&mut p, &path, "strengths", "AI, 영상 ,, 논문").unwrap(), json!(["AI", "영상", "논문"]));
    assert!(set(&mut p, &path, "nickname", "x").is_err());
    assert!(set(&mut p, &path, "signature", dir.join("none.png").to_str().unwrap()).is_err());
    assert_eq!(p["identity"]["birth_date"], "2001-03-15");
    profile::unset_field(&mut p, &path, "strengths").unwrap();
    profile::unset_field(&mut p, &path, "affiliation.is_student").unwrap();
    assert_eq!((p["strengths"].clone(), p["affiliation"]["is_student"].clone()), (json!([]), Value::Null));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn show_masks_identifying_values() {
    let dir = temp_dir("mask");
    let path = dir.join("profile.json");
    let mut p = profile::template();
    for (key, value) in [
        ("identity.name", "홍길동"),
        ("identity.name_en", "Gildong Hong"),
        ("identity.name_hanja", "洪吉童"),
        ("identity.birth_date", "2001-03-15"),
        ("contact.phone", "010-1234-5678"),
        ("contact.email", "abc@example.com"),
        ("accounts.hancom", "hancom@example.com"),
        ("affiliation.school", "한국대학교"),
    ] {
        set(&mut p, &path, key, value).unwrap();
    }
    let m = profile::masked(&p);
    assert_eq!(m["identity"]["name"], "홍*동");
    assert_eq!(m["identity"]["name_en"], "G***********");
    assert_eq!(m["identity"]["name_hanja"], "洪**");
    assert_eq!(m["identity"]["birth_date"], "2001-**-**");
    assert_eq!(m["contact"]["phone"], "010-****-5678");
    assert_eq!(m["contact"]["email"], "a***@example.com");
    assert_eq!(m["accounts"]["hancom"], "h***@example.com");
    assert_eq!(m["affiliation"]["school"], "한국대학교");
    assert_eq!((m["signature"].as_str(), m["affiliation"]["proof"].as_str()), (Some("없음"), Some("없음")));
    let name = profile::find_field("identity.name").unwrap();
    assert_eq!(profile::mask(name, &json!("이훈")), json!("이*"));
    let phone = profile::find_field("contact.phone").unwrap();
    assert_eq!(profile::mask(phone, &json!("01012345678")), json!("010****5678"));
    let rendered = m.to_string();
    for secret in ["길동", "1234", "abc@", "03-15", "Gildong"] {
        assert!(!rendered.contains(secret), "{secret} in {rendered}");
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn eligibility_has_no_identifying_values() {
    let dir = temp_dir("elig");
    let path = dir.join("profile.json");
    let mut p = profile::template();
    for (key, value) in [
        ("identity.name", "홍길동"),
        ("identity.birth_date", "2001-03-15"),
        ("contact.phone", "010-1234-5678"),
        ("contact.email", "abc@example.com"),
        ("accounts.hancom", "hancom@example.com"),
        ("affiliation.is_student", "예"),
        ("affiliation.school", "한국대학교"),
        ("affiliation.year", "3"),
        ("residence_region", "서울"),
    ] {
        set(&mut p, &path, key, value).unwrap();
    }
    let sig = dir.join("sig.png");
    fs::write(&sig, b"png").unwrap();
    set(&mut p, &path, "signature", sig.to_str().unwrap()).unwrap();
    let e = profile::eligibility(&p, NaiveDate::from_ymd_opt(2026, 3, 14).unwrap());
    let keys: Vec<&str> = e.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "is_student",
            "level",
            "year",
            "school",
            "department",
            "residence_region",
            "nationality",
            "age",
            "participation",
            "strengths",
            "constraints"
        ]
    );
    assert_eq!((e["age"].clone(), e["is_student"].clone(), e["year"].clone()), (json!(24), json!(true), json!(3)));
    assert_eq!(profile::eligibility(&p, NaiveDate::from_ymd_opt(2026, 3, 15).unwrap())["age"], 25);
    let text = e.to_string();
    for secret in ["홍길동", "2001", "010", "example.com", "signature", "profile/"] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn v1_files_are_read_as_v2_and_saved_as_v2() {
    let dir = temp_dir("v1");
    let path = dir.join("profile.json");
    let v1 = json!({
        "_설명": "예시",
        "name": "홍길동",
        "hancom_account": "user@example.com",
        "participation": {"individual": true, "team_available": false, "team_note": "혼자"},
        "student": {"is_student": true, "level": null, "school": "한국대학교", "proof": "재학증명서 보유"},
        "residence_region": "서울",
        "age": 24,
        "nationality": null,
        "strengths": ["AI"],
        "constraints": ["오프라인 불가"]
    });
    fs::write(&path, serde_json::to_string(&v1).unwrap()).unwrap();
    let p = profile::load_at(&path).unwrap();
    assert_eq!(p["version"], 2);
    assert_eq!(p["identity"]["name"], "홍길동");
    assert_eq!(p["identity"]["birth_date"], "");
    assert_eq!(p["accounts"]["hancom"], "user@example.com");
    assert_eq!(p["affiliation"]["school"], "한국대학교");
    assert_eq!(p["affiliation"]["is_student"], true);
    assert_eq!(
        (p["affiliation"]["proof"].as_str(), p["affiliation"]["proof_note"].as_str()),
        (Some(""), Some("재학증명서 보유"))
    );
    assert_eq!(p["participation"]["team_available"], false);
    assert_eq!(p["contact"], json!({"phone": "", "email": ""}));
    assert!(p.get("_설명").is_none() && p.get("name").is_none() && p.get("student").is_none());
    assert_eq!(profile::eligibility(&p, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap())["age"], 24);
    assert_eq!(profile::masked(&p)["identity"]["name"], "홍*동");

    profile::save_at(&path, &p).unwrap();
    let saved: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["version"], 2);
    assert_eq!(profile::load_at(&path).unwrap(), p);
    let empty = profile::load_at(&dir.join("absent.json")).unwrap();
    assert_eq!(empty, profile::template());
    fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn files_are_private_and_copies_are_relative() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("perm");
    let path = dir.join("profile.json");
    fs::write(&path, "{}").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let mode = |p: &std::path::Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let mut p = profile::load_at(&path).unwrap();
    let seal = dir.join("Seal.PNG");
    fs::write(&seal, b"png").unwrap();
    assert_eq!(set(&mut p, &path, "seal", seal.to_str().unwrap()).unwrap(), json!("profile/seal.png"));
    let pdf = dir.join("proof.pdf");
    fs::write(&pdf, b"pdf").unwrap();
    assert_eq!(set(&mut p, &path, "affiliation.proof", pdf.to_str().unwrap()).unwrap(), json!("profile/proof.pdf"));
    profile::save_at(&path, &p).unwrap();
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(&dir.join("profile")), 0o700);
    assert_eq!(mode(&dir.join("profile/seal.png")), 0o600);
    assert_eq!(fs::read(dir.join("profile/proof.pdf")).unwrap(), b"pdf");
    assert_eq!(profile::masked(&p)["seal"], "있음");
    let jpg = dir.join("seal.jpg");
    fs::write(&jpg, b"jpg").unwrap();
    set(&mut p, &path, "seal", jpg.to_str().unwrap()).unwrap();
    assert!(!dir.join("profile/seal.png").exists() && dir.join("profile/seal.jpg").exists());
    profile::unset_field(&mut p, &path, "seal").unwrap();
    assert!(!dir.join("profile/seal.jpg").exists());
    assert_eq!(p["seal"], "");
    let leftovers: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".profile-"))
        .collect();
    assert!(leftovers.is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn init_keeps_on_enter_clears_on_dash_and_asks_again() {
    let dir = temp_dir("init");
    let path = dir.join("profile.json");
    let mut p = profile::template();
    set(&mut p, &path, "identity.name", "홍길동").unwrap();
    set(&mut p, &path, "identity.name_en", "Hong").unwrap();
    profile::save_at(&path, &p).unwrap();
    // name: keep, name_en: clear, name_hanja: set, birth_date: bad then good, then end of input.
    let mut input = Cursor::new("\n-\n洪吉童\n2001/03/15\n2001-03-15\n");
    let mut out = Vec::new();
    let got = profile::init(&path, &mut input, &mut out).unwrap();
    let saved = profile::load_at(&path).unwrap();
    assert_eq!(got, saved);
    assert_eq!(
        saved["identity"],
        json!({"name": "홍길동", "name_en": "", "name_hanja": "洪吉童", "birth_date": "2001-03-15"})
    );
    let shown = String::from_utf8(out).unwrap();
    assert!(shown.contains("이름 [홍*동]"), "{shown}");
    assert!(shown.contains("다시 입력"), "{shown}");
    assert!(!shown.contains("[홍길동]") && !shown.contains("[Hong]"), "{shown}");
    fs::remove_dir_all(&dir).unwrap();
}
