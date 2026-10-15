//! The participant profile (`data/profile.json`, git-ignored; files in `data/profile/`). The field
//! table [`FIELDS`] is the single definition `harness profile fields`, `profile init` and the
//! `/contest-harness-init` extension command ask from. Identifying values are masked unless
//! `--reveal`; ContestGate reads only [`eligibility`]. Contract: `docs/CONTRACT.md`.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use chrono::{Datelike, NaiveDate};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::bail;
use crate::error::{Error, Result};
use crate::paths;

pub const VERSION: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Text,
    Date,
    Bool,
    Int,
    File,
    List,
}

/// How a value is validated on `set` and shown without `--reveal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mask {
    /// Not identifying: shown and validated by kind only.
    None,
    /// `홍*동`.
    Name,
    /// First character only.
    Initial,
    /// `010-****-1234`; digits, `-` and `+` only.
    Phone,
    /// `a***@example.com`; must contain `@`.
    Email,
    /// An email-like account is masked as an email, anything else by its first character.
    Account,
    /// `YYYY-**-**`.
    Date,
    /// `있음` / `없음`.
    File,
}

#[derive(Debug, Serialize)]
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: Kind,
    pub example: &'static str,
    pub sensitive: bool,
    #[serde(skip)]
    pub mask: Mask,
}

const fn field(key: &'static str, label: &'static str, kind: Kind, example: &'static str, mask: Mask) -> Field {
    Field { key, label, kind, example, sensitive: !matches!(mask, Mask::None), mask }
}

pub const FIELDS: &[Field] = &[
    field("identity.name", "이름", Kind::Text, "홍길동", Mask::Name),
    field("identity.name_en", "영문 이름", Kind::Text, "Gildong Hong", Mask::Initial),
    field("identity.name_hanja", "한자 이름", Kind::Text, "洪吉童", Mask::Initial),
    field("identity.birth_date", "생년월일", Kind::Date, "2001-03-15", Mask::Date),
    field("contact.phone", "전화번호", Kind::Text, "010-1234-5678", Mask::Phone),
    field("contact.email", "이메일", Kind::Text, "name@example.com", Mask::Email),
    field("affiliation.is_student", "재학 중", Kind::Bool, "예", Mask::None),
    field("affiliation.school", "학교", Kind::Text, "한국대학교", Mask::None),
    field("affiliation.department", "학과", Kind::Text, "컴퓨터공학과", Mask::None),
    field("affiliation.level", "과정", Kind::Text, "학부", Mask::None),
    field("affiliation.year", "학년", Kind::Int, "3", Mask::None),
    field("affiliation.proof", "재학증명서 파일", Kind::File, "~/Documents/enrollment.pdf", Mask::File),
    field("residence_region", "거주 지역", Kind::Text, "서울", Mask::None),
    field("nationality", "국적", Kind::Text, "대한민국", Mask::None),
    field("participation.individual", "개인 참가 가능", Kind::Bool, "예", Mask::None),
    field("participation.team_available", "팀 참가 가능", Kind::Bool, "아니오", Mask::None),
    field("participation.team_note", "팀 참가 메모", Kind::Text, "함께할 팀원 2명", Mask::None),
    field("accounts.hancom", "한컴독스 계정", Kind::Text, "name@example.com", Mask::Account),
    field("signature", "서명 이미지", Kind::File, "~/Documents/signature.png", Mask::File),
    field("seal", "도장 이미지", Kind::File, "~/Documents/seal.png", Mask::File),
    field("strengths", "강점(쉼표 구분)", Kind::List, "AI·데이터 분석, 숏폼 영상", Mask::None),
    field("constraints", "제약(쉼표 구분)", Kind::List, "오프라인 참석 불가, 참가비 있으면 확인", Mask::None),
];

pub fn find_field(key: &str) -> Result<&'static Field> {
    FIELDS.iter().find(|f| f.key == key).ok_or_else(|| {
        let keys: Vec<&str> = FIELDS.iter().map(|f| f.key).collect();
        Error(format!("알 수 없는 항목: {key} (가능: {})", keys.join(", ")))
    })
}

/// An empty v2 profile.
pub fn template() -> Value {
    json!({
        "version": VERSION,
        "identity": {"name": "", "name_en": "", "name_hanja": "", "birth_date": ""},
        "contact": {"phone": "", "email": ""},
        "affiliation": {"is_student": null, "school": "", "department": "", "level": "", "year": null, "proof": ""},
        "residence_region": "",
        "nationality": "",
        "participation": {"individual": true, "team_available": null, "team_note": ""},
        "accounts": {"hancom": ""},
        "signature": "",
        "seal": "",
        "strengths": [],
        "constraints": []
    })
}

/// Lay `over` onto `base` object by object (non-object values replace).
fn overlay(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(base), Value::Object(over)) => {
            for (key, value) in over {
                match base.get_mut(&key) {
                    Some(slot @ Value::Object(_)) if value.is_object() => overlay(slot, value),
                    _ => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, over) => *base = over,
    }
}

/// Any stored profile as v2. v1 files (`name`, `hancom_account`, `student{...}`, …) are converted;
/// keys starting with `_` (comments) are dropped and other unknown keys kept.
pub fn upgrade(stored: Value) -> Value {
    let mut profile = template();
    let Value::Object(mut map) = stored else { return profile };
    if map.get("version").and_then(Value::as_u64) == Some(VERSION) {
        overlay(&mut profile, Value::Object(map));
        return profile;
    }
    let mut v2 = Map::new();
    let mut put = |path: &str, value: Value| {
        let mut slot = &mut v2;
        let mut parts = path.split('.').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                slot.insert(part.to_string(), value);
                return;
            }
            slot = slot.entry(part.to_string()).or_insert_with(|| json!({})).as_object_mut().expect("object");
        }
    };
    const MOVES: &[(&str, &str)] = &[
        ("name", "identity.name"),
        ("name_en", "identity.name_en"),
        ("name_hanja", "identity.name_hanja"),
        ("birth_date", "identity.birth_date"),
        ("phone", "contact.phone"),
        ("email", "contact.email"),
        ("hancom_account", "accounts.hancom"),
    ];
    for (old, new) in MOVES {
        if let Some(value) = map.remove(*old) {
            put(new, value);
        }
    }
    if let Some(Value::Object(student)) = map.remove("student") {
        for (key, value) in student {
            // v1 kept a note ("재학증명서 보유") where v2 keeps a copied file.
            let is_file = value.as_str().is_some_and(|p| !p.is_empty() && Path::new(p).is_file());
            let key = if key == "proof" && !is_file && !value.is_null() { "proof_note".to_string() } else { key };
            put(&format!("affiliation.{key}"), value);
        }
    }
    map.retain(|key, _| !key.starts_with('_') && key != "version");
    for (key, value) in map {
        put(&key, value);
    }
    overlay(&mut profile, Value::Object(v2));
    profile["version"] = json!(VERSION);
    profile
}

pub fn path() -> PathBuf {
    paths::profile()
}

/// The folder for signature, seal and proof copies beside `profile_path` (`data/profile/`).
pub fn files_dir(profile_path: &Path) -> PathBuf {
    profile_path.with_file_name("profile")
}

/// Read and upgrade the profile; an absent file is the empty template.
pub fn load_at(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(template());
    }
    let stored: Value =
        serde_json::from_str(&fs::read_to_string(path)?).map_err(|err| Error(format!("{}: {err}", path.display())))?;
    Ok(upgrade(stored))
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

/// Write the profile as v2 through a temporary file and a rename; the file ends up 0600.
pub fn save_at(path: &Path, profile: &Value) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_file_name(format!(".profile-{}.json", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    let mut stored = profile.clone();
    stored["version"] = json!(VERSION);
    file.write_all(serde_json::to_string_pretty(&stored)?.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    restrict(&tmp, 0o600)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// The value at a dotted key (`Null` when absent).
pub fn get<'a>(profile: &'a Value, key: &str) -> &'a Value {
    key.split('.').try_fold(profile, |value, part| value.get(part)).unwrap_or(&Value::Null)
}

fn put(profile: &mut Value, key: &str, value: Value) {
    let mut slot = profile;
    for part in key.split('.') {
        if !slot.is_object() {
            *slot = json!({});
        }
        slot = slot.as_object_mut().expect("object").entry(part.to_string()).or_insert(Value::Null);
    }
    *slot = value;
}

/// The value a field holds when unset.
fn empty(field: &Field) -> Value {
    match field.kind {
        Kind::Bool if field.key == "participation.individual" => json!(true),
        Kind::Bool | Kind::Int => Value::Null,
        Kind::List => json!([]),
        Kind::Text | Kind::Date | Kind::File => json!(""),
    }
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_lowercase().as_str() {
        "예" | "네" | "true" | "yes" | "y" => Some(true),
        "아니오" | "아니요" | "false" | "no" | "n" => Some(false),
        _ => None,
    }
}

/// Validate `raw` for a non-file field and return the value to store.
pub fn parse(field: &Field, raw: &str) -> Result<Value> {
    let text = raw.trim();
    Ok(match field.kind {
        Kind::Text => {
            match field.mask {
                Mask::Email if !text.is_empty() => {
                    let valid = text.split_once('@').is_some_and(|(l, d)| !l.is_empty() && !d.is_empty());
                    if !valid {
                        bail!("{}: 이메일 형식이 아니다(예: {})", field.key, field.example);
                    }
                }
                Mask::Phone if !text.is_empty() => {
                    let valid = text.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '+')
                        && text.chars().any(|c| c.is_ascii_digit());
                    if !valid {
                        bail!("{}: 전화번호는 숫자·하이픈·+만 쓴다(예: {})", field.key, field.example);
                    }
                }
                _ => {}
            }
            json!(text)
        }
        Kind::Date => {
            let valid = text.len() == 10 && NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok();
            if !valid {
                bail!("{}: 날짜는 YYYY-MM-DD 형식이어야 한다(예: {})", field.key, field.example);
            }
            json!(text)
        }
        Kind::Bool => match parse_bool(text) {
            Some(b) => json!(b),
            None => bail!("{}: 예/아니오(true/false) 중 하나로 답한다", field.key),
        },
        Kind::Int => match text.parse::<u32>() {
            Ok(n) => json!(n),
            Err(_) => bail!("{}: 0 이상의 정수여야 한다(예: {})", field.key, field.example),
        },
        Kind::List => json!(text.split(',').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>()),
        Kind::File => bail!("{}: 파일 항목은 set_field로 저장한다", field.key),
    })
}

/// Remove a copy that `set_field` made earlier under `files`.
fn remove_copy(files: &Path, stored: &Value) -> Result<()> {
    if let Some(rel) = stored.as_str().and_then(|s| s.strip_prefix("profile/")) {
        let old = files.join(rel);
        if old.is_file() {
            fs::remove_file(old)?;
        }
    }
    Ok(())
}

/// Validate and store one field. File fields copy the file into `files_dir(profile_path)` (0700,
/// copy 0600) and store the path relative to the profile's folder (`profile/signature.png`).
pub fn set_field(profile: &mut Value, profile_path: &Path, key: &str, raw: &str) -> Result<Value> {
    let field = find_field(key)?;
    let value = if field.kind == Kind::File {
        let source = paths::expand(raw.trim());
        if !source.is_file() {
            bail!("{key}: 파일이 없다: {}", source.display());
        }
        let files = files_dir(profile_path);
        fs::create_dir_all(&files)?;
        restrict(&files, 0o700)?;
        let name = key.rsplit('.').next().unwrap_or(key);
        let file_name = match source.extension().and_then(|e| e.to_str()) {
            Some(ext) => format!("{name}.{}", ext.to_lowercase()),
            None => name.to_string(),
        };
        let target = files.join(&file_name);
        let previous = get(profile, key).clone();
        if previous.as_str() != Some(format!("profile/{file_name}").as_str()) {
            remove_copy(&files, &previous)?;
        }
        fs::copy(&source, &target)?;
        restrict(&target, 0o600)?;
        json!(format!("profile/{file_name}"))
    } else {
        parse(field, raw)?
    };
    put(profile, key, value.clone());
    Ok(value)
}

/// Reset one field to its empty value (a copied file is deleted).
pub fn unset_field(profile: &mut Value, profile_path: &Path, key: &str) -> Result<()> {
    let field = find_field(key)?;
    if field.kind == Kind::File {
        remove_copy(&files_dir(profile_path), get(profile, key))?;
    }
    put(profile, key, empty(field));
    Ok(())
}

fn stars(n: usize) -> String {
    "*".repeat(n)
}

fn mask_text(mask: Mask, text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    match mask {
        Mask::None | Mask::File => text.to_string(),
        Mask::Name => match chars.len() {
            1 => "*".into(),
            2 => format!("{}*", chars[0]),
            n => format!("{}{}{}", chars[0], stars(n - 2), chars[n - 1]),
        },
        Mask::Initial => format!("{}{}", chars[0], stars(chars.len() - 1)),
        Mask::Phone => {
            let digits = chars.iter().filter(|c| c.is_ascii_digit()).count();
            let mut seen = 0;
            chars
                .iter()
                .map(|&c| {
                    if !c.is_ascii_digit() {
                        return c;
                    }
                    seen += 1;
                    if seen <= 3 || seen > digits.saturating_sub(4) { c } else { '*' }
                })
                .collect()
        }
        Mask::Email | Mask::Account => match text.split_once('@') {
            Some((local, domain)) => {
                format!("{}***@{domain}", local.chars().next().map(String::from).unwrap_or_default())
            }
            None => format!("{}***", chars[0]),
        },
        Mask::Date => format!("{}-**-**", chars.iter().take(4).collect::<String>()),
    }
}

/// The masked form of one value.
pub fn mask(field: &Field, value: &Value) -> Value {
    match (field.mask, value) {
        (Mask::None, _) => value.clone(),
        (Mask::File, Value::String(s)) if !s.is_empty() => json!("있음"),
        (Mask::File, _) => json!("없음"),
        (mask, Value::String(s)) => json!(mask_text(mask, s)),
        _ => value.clone(),
    }
}

/// The profile with every sensitive field masked (the default `profile show`).
pub fn masked(profile: &Value) -> Value {
    let mut out = profile.clone();
    for field in FIELDS.iter().filter(|f| f.sensitive) {
        put(&mut out, field.key, mask(field, get(profile, field.key)));
    }
    out
}

/// Text form of a value for `profile show` and `profile init`.
pub fn display(value: &Value) -> String {
    match value {
        Value::Null => "(미입력)".into(),
        Value::Bool(true) => "예".into(),
        Value::Bool(false) => "아니오".into(),
        Value::String(s) if s.is_empty() => "(미입력)".into(),
        Value::String(s) => s.clone(),
        Value::Array(items) if items.is_empty() => "(미입력)".into(),
        Value::Array(items) => items
            .iter()
            .map(|i| i.as_str().map_or_else(|| i.to_string(), str::to_string))
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    }
}

/// Full years between `birth` (`YYYY-MM-DD`) and `today`.
pub fn age(birth: &str, today: NaiveDate) -> Option<u32> {
    let born = NaiveDate::parse_from_str(birth, "%Y-%m-%d").ok()?;
    let mut years = today.year() - born.year();
    if (today.month(), today.day()) < (born.month(), born.day()) {
        years -= 1;
    }
    u32::try_from(years).ok()
}

/// What ContestGate may read: no name, contact, birth date, account or files — only eligibility
/// facts, with `age` computed from the birth date (or a v1 `age`).
pub fn eligibility(profile: &Value, today: NaiveDate) -> Value {
    let a = |key: &str| get(profile, &format!("affiliation.{key}")).clone();
    let age = get(profile, "identity.birth_date")
        .as_str()
        .and_then(|b| age(b, today))
        .map(Value::from)
        .or_else(|| get(profile, "age").as_u64().map(Value::from))
        .unwrap_or(Value::Null);
    json!({
        "is_student": a("is_student"),
        "level": a("level"),
        "year": a("year"),
        "school": a("school"),
        "department": a("department"),
        "residence_region": get(profile, "residence_region"),
        "nationality": get(profile, "nationality"),
        "age": age,
        "participation": get(profile, "participation"),
        "strengths": get(profile, "strengths"),
        "constraints": get(profile, "constraints"),
    })
}

/// `profile init`: ask every field on `input`, showing the current value (masked when sensitive) as
/// the default. Enter keeps it, `-` clears it, an invalid answer is asked again. Saves after each
/// change, so stopping half way (end of input) keeps what was entered.
pub fn init(profile_path: &Path, input: &mut impl BufRead, out: &mut impl Write) -> Result<Value> {
    let mut profile = load_at(profile_path)?;
    writeln!(out, "참가자 정보 입력. Enter는 현재 값 유지, `-`는 지우기. 파일: {}", profile_path.display())?;
    for field in FIELDS {
        loop {
            let current = display(&mask(field, get(&profile, field.key)));
            write!(out, "{} [{current}] (예: {}): ", field.label, field.example)?;
            out.flush()?;
            let mut line = String::new();
            if input.read_line(&mut line)? == 0 {
                writeln!(out)?;
                return Ok(profile);
            }
            let answer = line.trim();
            let result = match answer {
                "" => break,
                "-" => unset_field(&mut profile, profile_path, field.key).map(|_| ()),
                _ => set_field(&mut profile, profile_path, field.key, answer).map(|_| ()),
            };
            match result {
                Ok(()) => {
                    save_at(profile_path, &profile)?;
                    break;
                }
                Err(err) => writeln!(out, "  다시 입력: {err}")?,
            }
        }
    }
    Ok(profile)
}
