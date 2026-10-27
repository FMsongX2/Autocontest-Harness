//! Official rule evidence. A contest's official notice pages and attachments are captured as the
//! actual bytes (a declared local file, or one bounded HTTP GET of the official URL) into immutable,
//! content-addressed snapshots, and only typed rules a reviewer extracted with a source, hash and
//! locator decide what the change means. Metadata lives in `Entry.extra.rules`.
//!
//! - `capture` adds a candidate snapshot set; any byte change becomes a pending review. Nothing is
//!   classified automatically: a banner change and a policy change look the same until reviewed.
//! - `review` accepts the exact pending candidate with structured rules
//!   (`eligibility|deadline|ai_policy|forms|criteria|submission`). The change is derived by comparing
//!   typed values with the accepted rules: unchanged values accept without touching any stage;
//!   eligibility/ai_policy invalidate gate+plan+final, criteria plan+final, forms final, and any
//!   change (deadline and submission alone included) moves the submission epoch. Ambiguous or
//!   unresolved reviews stay pending.
//! - `acknowledge` clears one invalidated stage with an explicit reviewed pass bound to the current
//!   epochs and evidence hashes; `score_recorded` stamps scores and clears plan/final.
//!
//! Snapshot bodies are untrusted data: they are hashed, stored and searched for quoted locators,
//! never interpreted. PDFs and other binary files are never parsed; their quotes stay unverified.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use ureq::{Agent, ResponseExt};

use crate::bail;
use crate::error::{Error, Result};
use crate::registry::Entry;
use crate::text::{collapse, now, strip_tags};
use crate::{http, paths, readiness, store};

/// `Entry.extra` key of the rule evidence state.
pub const KEY: &str = "rules";
/// The newest state format this build reads and writes.
pub const SCHEMA: u32 = 1;
/// Typed rule categories a review may report.
pub const CATEGORIES: &[&str] = &["eligibility", "deadline", "ai_policy", "forms", "criteria", "submission"];
/// Stages with a rule epoch.
pub const STAGES: &[&str] = &["gate", "plan", "final", "submission"];
/// Stages `acknowledge` can clear.
pub const ACK_STAGES: &[&str] = &["gate", "plan", "final"];
pub const MAX_SOURCES: usize = 32;
pub const MAX_RULES: usize = 200;
const MAX_URL: usize = 2048;
const MAX_VALUE: usize = 8192;
const MAX_LOCATOR: usize = 2000;
const MIN_RATIONALE: usize = 10;

/// Bounds of one capture.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Largest accepted file or response body.
    pub max_bytes: u64,
    /// Whole-request timeout of one GET.
    pub timeout: Duration,
    pub max_redirects: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_bytes: 20 * 1024 * 1024, timeout: Duration::from_secs(30), max_redirects: 5 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Epochs {
    gate: u64,
    plan: u64,
    #[serde(rename = "final")]
    final_: u64,
    submission: u64,
}

impl Epochs {
    fn get(&self, stage: &str) -> u64 {
        match stage {
            "gate" => self.gate,
            "plan" => self.plan,
            "final" => self.final_,
            _ => self.submission,
        }
    }

    fn bump(&mut self, stage: &str) -> u64 {
        let slot = match stage {
            "gate" => &mut self.gate,
            "plan" => &mut self.plan,
            "final" => &mut self.final_,
            _ => &mut self.submission,
        };
        *slot += 1;
        *slot
    }

    fn json(&self) -> Value {
        json!({"gate": self.gate, "plan": self.plan, "final": self.final_, "submission": self.submission})
    }
}

/// One captured source: provenance and where the immutable bytes are.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Snapshot {
    id: String,
    url: String,
    /// `file` (declared local copy) or `http` (fetched).
    origin: String,
    /// The URL the bytes came from after redirects (`url` for files).
    final_url: String,
    sha256: String,
    size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    locator: Option<Value>,
    /// `text` (quotes can be checked) or `binary_unparsed` (PDF, HWP, images: never parsed).
    semantic: String,
    /// `folder` (`<folder>/<blob>`) or `data` (`data/<blob>`, captured before a folder existed).
    store: String,
    blob: String,
    retrieved_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    http_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_modified: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_modified_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Version {
    version: String,
    captured_at: String,
    /// `pending`, `accepted`, `retired` (replaced by a later accepted version) or `superseded`.
    status: String,
    set_sha256: String,
    sources: Vec<Snapshot>,
    #[serde(default)]
    diff: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    superseded_by: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RuleSource {
    id: String,
    sha256: String,
    locator: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Rule {
    category: String,
    #[serde(default)]
    key: String,
    value: Value,
    source: RuleSource,
    /// `quote_matched`, `position_only` or `binary_unverified`.
    verification: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Accepted {
    version: String,
    accepted_at: String,
    reviewer: String,
    rules: Vec<Rule>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Invalidation {
    since_version: String,
    /// The stage epoch before the first unacknowledged change.
    from_epoch: u64,
    epoch: u64,
    categories: Vec<String>,
    at: String,
}

/// A cleared invalidation: an explicit review (`kind: review`) or a new score (`kind: score`).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Clearance {
    at: String,
    stage: String,
    kind: String,
    rules_version: String,
    from_epoch: u64,
    epoch: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    reviewer: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    rationale: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    evidence: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    score_index: Option<usize>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    schema: u32,
    #[serde(default)]
    versions: Vec<Version>,
    #[serde(default)]
    accepted: Option<Accepted>,
    #[serde(default)]
    pending: Option<String>,
    #[serde(default)]
    epochs: Epochs,
    #[serde(default)]
    invalidated: BTreeMap<String, Invalidation>,
    #[serde(default)]
    reviews: Vec<Value>,
    #[serde(default)]
    acknowledgements: Vec<Clearance>,
    #[serde(default)]
    last_checked_at: Option<String>,
    #[serde(default)]
    last_check: Option<Value>,
    #[serde(default)]
    check_count: u64,
    #[serde(default)]
    last_evidence_at: Option<String>,
    /// Fields this version does not know about are kept as they are.
    #[serde(flatten)]
    extra: Map<String, Value>,
}

impl State {
    fn version(&self, version: &str) -> Option<&Version> {
        self.versions.iter().find(|v| v.version == version)
    }

    fn accepted_version(&self) -> Option<&Version> {
        self.accepted.as_ref().and_then(|a| self.version(&a.version))
    }

    fn set_status(&mut self, version: &str, status: &str, superseded_by: Option<&str>) {
        if let Some(v) = self.versions.iter_mut().find(|v| v.version == version) {
            v.status = status.to_string();
            if let Some(by) = superseded_by {
                v.superseded_by = Some(by.to_string());
            }
        }
    }
}

fn load(entry: &Entry) -> Result<State> {
    let Some(raw) = entry.extra.get(KEY) else { return Ok(State { schema: SCHEMA, ..State::default() }) };
    let mut state: State = serde_json::from_value(raw.clone())
        .map_err(|err| Error(format!("{}: 규칙 증거 상태(extra.rules)를 읽지 못했다: {err}", entry.id)))?;
    if state.schema > SCHEMA {
        bail!("{}: 규칙 증거 상태 schema {}는 이 빌드({SCHEMA})보다 새롭다", entry.id, state.schema);
    }
    state.schema = SCHEMA;
    Ok(state)
}

fn to_value(state: &State) -> Result<Value> {
    Ok(serde_json::to_value(state)?)
}

fn sha_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_sha(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn nonblank<'a>(raw: &'a Value, field: &str) -> Option<&'a str> {
    raw.get(field).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

// ---------------------------------------------------------------------------------------------
// Capture

struct SourceSpec {
    id: String,
    url: String,
    path: Option<String>,
    sha256: Option<String>,
    media_type: Option<String>,
    locator: Option<Value>,
}

/// An official URL: `http(s)://host...`, no credentials, no whitespace or control characters.
pub fn check_url(url: &str) -> Result<()> {
    if url.is_empty() || url.len() > MAX_URL {
        bail!("URL 길이가 올바르지 않다");
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        bail!("URL에 공백이나 제어 문자가 있다: {url}");
    }
    let lower = url.to_ascii_lowercase();
    let rest = if let Some(rest) = lower.strip_prefix("https://") {
        rest
    } else if let Some(rest) = lower.strip_prefix("http://") {
        rest
    } else {
        bail!("공식 URL은 http(s)여야 한다: {url}")
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        bail!("URL에 호스트가 없다: {url}");
    }
    if authority.contains('@') {
        bail!("URL에 계정 정보를 넣을 수 없다(로그인·자격 증명 금지)");
    }
    Ok(())
}

fn check_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('.')
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn parse_sources(raw: &Value) -> Result<Vec<SourceSpec>> {
    let Some(items) = raw.get("sources").and_then(Value::as_array).filter(|a| !a.is_empty()) else {
        bail!("sources는 비어 있지 않은 [{{id, url, path?, sha256?, media_type?, locator?}}] 배열이어야 한다")
    };
    if items.len() > MAX_SOURCES {
        bail!("sources는 {MAX_SOURCES}개 이하여야 한다");
    }
    let mut out: Vec<SourceSpec> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        if !item.is_object() {
            bail!("sources[{i}]는 객체여야 한다");
        }
        let Some(id) = item.get("id").and_then(Value::as_str).filter(|id| check_id(id)) else {
            bail!("sources[{i}].id는 영문·숫자·._- 64자 이하여야 한다")
        };
        if out.iter().any(|s| s.id == id) {
            bail!("sources에 같은 id가 두 번 있다: {id}");
        }
        let Some(url) = item.get("url").and_then(Value::as_str) else { bail!("sources[{i}]({id}).url이 없다") };
        check_url(url).map_err(|err| Error(format!("sources[{i}]({id}): {err}")))?;
        let path = match item.get("path") {
            None | Some(Value::Null) => None,
            Some(Value::String(p)) if !p.trim().is_empty() => Some(p.clone()),
            Some(_) => bail!("sources[{i}]({id}).path는 비어 있지 않은 문자열이어야 한다"),
        };
        let sha256 = match item.get("sha256") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if is_sha(&s.to_ascii_lowercase()) => Some(s.to_ascii_lowercase()),
            Some(_) => bail!("sources[{i}]({id}).sha256은 64자리 16진수여야 한다"),
        };
        let media_type = match item.get("media_type") {
            None | Some(Value::Null) => None,
            Some(Value::String(m)) if !m.trim().is_empty() && m.len() <= 200 && !m.chars().any(char::is_control) => {
                Some(m.trim().to_string())
            }
            Some(_) => bail!("sources[{i}]({id}).media_type이 올바르지 않다"),
        };
        let locator = match item.get("locator") {
            None | Some(Value::Null) => None,
            Some(v @ (Value::String(_) | Value::Object(_))) if v.to_string().len() <= MAX_LOCATOR => Some(v.clone()),
            Some(_) => bail!("sources[{i}]({id}).locator는 {MAX_LOCATOR}자 이하 문자열이나 객체여야 한다"),
        };
        out.push(SourceSpec { id: id.to_string(), url: url.to_string(), path, sha256, media_type, locator });
    }
    Ok(out)
}

struct Acquired {
    bytes: Vec<u8>,
    origin: &'static str,
    final_url: String,
    media_type: Option<String>,
    http_status: Option<u16>,
    last_modified: Option<String>,
    source_modified_at: Option<String>,
}

fn guess_media(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let media = match ext.as_str() {
        "pdf" => "application/pdf",
        "html" | "htm" => "text/html",
        "txt" => "text/plain",
        "md" => "text/markdown",
        "json" => "application/json",
        "xml" => "application/xml",
        "hwp" => "application/x-hwp",
        "hwpx" => "application/hwp+zip",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        _ => return None,
    };
    Some(media.to_string())
}

fn read_local(entry: &Entry, spec: &SourceSpec, path: &str, limits: &Limits) -> Result<Acquired> {
    let given = paths::expand(path);
    let full = if given.is_absolute() {
        given
    } else {
        match entry.folder.as_deref().filter(|f| !f.is_empty()) {
            Some(folder) => Path::new(folder).join(given),
            None => bail!("{}: 상대 경로는 공모전 폴더가 있어야 한다: {path}", spec.id),
        }
    };
    let meta = fs::metadata(&full).map_err(|err| Error(format!("{}: 파일을 열 수 없다({path}): {err}", spec.id)))?;
    if !meta.is_file() {
        bail!("{}: 일반 파일이 아니다: {path}", spec.id);
    }
    if meta.len() > limits.max_bytes {
        bail!("{}: 파일이 {}바이트 한도를 넘는다({}바이트)", spec.id, limits.max_bytes, meta.len());
    }
    let file = File::open(&full).map_err(|err| Error(format!("{}: 파일을 열 수 없다({path}): {err}", spec.id)))?;
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(limits.max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limits.max_bytes {
        bail!("{}: 파일이 {}바이트 한도를 넘는다", spec.id, limits.max_bytes);
    }
    let source_modified_at = meta.modified().ok().map(|t| DateTime::<Utc>::from(t).to_rfc3339());
    Ok(Acquired {
        bytes,
        origin: "file",
        final_url: spec.url.clone(),
        media_type: spec.media_type.clone().or_else(|| guess_media(&full)),
        http_status: None,
        last_modified: None,
        source_modified_at,
    })
}

/// One bounded GET: no cookies, no credentials, limited redirects, size and time.
fn fetch(spec: &SourceSpec, limits: &Limits) -> Result<Acquired> {
    let url = spec.url.as_str();
    let agent: Agent = Agent::config_builder()
        .timeout_global(Some(limits.timeout))
        .user_agent(http::UA)
        .http_status_as_error(false)
        .max_redirects(limits.max_redirects)
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("Accept-Language", "ko-KR,ko;q=0.9")
        .call()
        .map_err(|err| Error(format!("{}: {url}을 가져오지 못했다: {err}", spec.id)))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        bail!("{}: {url}: HTTP {status}", spec.id);
    }
    let final_url = response.get_uri().to_string();
    check_url(&final_url).map_err(|err| Error(format!("{}: 리다이렉트 끝 주소가 올바르지 않다: {err}", spec.id)))?;
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    if let Some(len) = header("content-length").and_then(|v| v.parse::<u64>().ok())
        && len > limits.max_bytes
    {
        bail!("{}: {url}: 본문이 {}바이트 한도를 넘는다({len}바이트)", spec.id, limits.max_bytes);
    }
    let content_type = header("content-type");
    let last_modified = header("last-modified");
    let bytes = response.body_mut().with_config().limit(limits.max_bytes).read_to_vec().map_err(|err| match err {
        ureq::Error::BodyExceedsLimit(_) => {
            Error(format!("{}: {url}: 본문이 {}바이트 한도를 넘는다", spec.id, limits.max_bytes))
        }
        other => Error(format!("{}: {url}: 본문을 읽지 못했다: {other}", spec.id)),
    })?;
    Ok(Acquired {
        bytes,
        origin: "http",
        final_url,
        media_type: spec.media_type.clone().or(content_type),
        http_status: Some(status),
        last_modified,
        source_modified_at: None,
    })
}

fn semantic(media: Option<&str>, bytes: &[u8]) -> &'static str {
    let media = media.unwrap_or("").to_ascii_lowercase();
    if bytes.starts_with(b"%PDF") || media.contains("pdf") || std::str::from_utf8(bytes).is_err() {
        return "binary_unparsed";
    }
    let texty = media.is_empty()
        || media.starts_with("text/")
        || media.contains("html")
        || media.contains("json")
        || media.contains("xml");
    if texty { "text" } else { "binary_unparsed" }
}

/// Where snapshots of `entry` live: `(store, directory, blob prefix)`.
fn store_root(entry: &Entry) -> Result<(&'static str, PathBuf, String)> {
    if let Some(folder) = entry.folder.as_deref().filter(|f| !f.is_empty()) {
        return Ok(("folder", Path::new(folder).join(".private").join("rules"), ".private/rules".to_string()));
    }
    if !check_id(&entry.id) {
        bail!("{}: 폴더가 없는 공모전의 id는 경로로 쓸 수 없다", entry.id);
    }
    Ok(("data", paths::data().join("rules").join(&entry.id), format!("rules/{}", entry.id)))
}

fn blob_path(entry: &Entry, snap: &Snapshot) -> Result<PathBuf> {
    match snap.store.as_str() {
        "folder" => match entry.folder.as_deref().filter(|f| !f.is_empty()) {
            Some(folder) => Ok(Path::new(folder).join(&snap.blob)),
            None => bail!("{}: 스냅샷 {}의 폴더가 기록돼 있지 않다", entry.id, snap.id),
        },
        "data" => Ok(paths::data().join(&snap.blob)),
        other => bail!("{}: 알 수 없는 스냅샷 저장소: {other}", entry.id),
    }
}

fn verify_existing(path: &Path, sha: &str) -> Result<()> {
    let actual = readiness::sha256_file(path)?;
    if actual != sha {
        bail!("{}: 기존 스냅샷이 변조됐다(기대 {sha}, 실제 {actual})", path.display());
    }
    Ok(())
}

/// Store `bytes` once under `<dir>/sha256/<sha>`, read-only; an existing blob must still hash right.
fn write_blob(dir: &Path, sha: &str, bytes: &[u8]) -> Result<()> {
    let blobs = dir.join("sha256");
    store::private_dir(&blobs)?;
    let target = blobs.join(sha);
    if target.exists() {
        return verify_existing(&target, sha);
    }
    let tmp = blobs.join(format!(".tmp-{}-{}", std::process::id(), &sha[..16]));
    let _ = fs::remove_file(&tmp);
    let written = (|| -> Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::set_permissions(&tmp, Permissions::from_mode(0o400))?;
        match fs::hard_link(&tmp, &target) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => verify_existing(&target, sha),
            Err(err) => Err(Error(format!("{}: {err}", target.display()))),
        }
    })();
    let _ = fs::remove_file(&tmp);
    written
}

fn set_sha(sources: &[Snapshot]) -> String {
    let mut sorted: Vec<&Snapshot> = sources.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let mut hasher = Sha256::new();
    for s in sorted {
        hasher.update(s.id.as_bytes());
        hasher.update([0]);
        hasher.update(s.url.as_bytes());
        hasher.update([0]);
        hasher.update(s.sha256.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

struct Diff {
    added: Vec<String>,
    removed: Vec<String>,
    changed: Vec<String>,
    unchanged: Vec<String>,
}

impl Diff {
    fn json(&self) -> Value {
        json!({"added": self.added, "removed": self.removed, "changed": self.changed, "unchanged": self.unchanged})
    }
}

fn diff(old: Option<&Version>, new: &[Snapshot]) -> Diff {
    let mut d = Diff { added: vec![], removed: vec![], changed: vec![], unchanged: vec![] };
    let old_sources: &[Snapshot] = old.map(|v| v.sources.as_slice()).unwrap_or(&[]);
    for s in new {
        match old_sources.iter().find(|o| o.id == s.id) {
            None => d.added.push(s.id.clone()),
            Some(o) if o.sha256 != s.sha256 || o.url != s.url => d.changed.push(s.id.clone()),
            Some(_) => d.unchanged.push(s.id.clone()),
        }
    }
    for o in old_sources {
        if !new.iter().any(|s| s.id == o.id) {
            d.removed.push(o.id.clone());
        }
    }
    d
}

fn source_summary(s: &Snapshot) -> Value {
    let mut v = serde_json::to_value(s).unwrap_or(Value::Null);
    if let Some(obj) = v.as_object_mut() {
        obj.remove("locator");
    }
    v
}

/// `capture` with the default [`Limits`].
pub fn capture(entry: &mut Entry, raw: &Value) -> Result<Value> {
    capture_with(entry, raw, &Limits::default())
}

/// Capture the declared official sources as immutable snapshots (`ach rules capture <id> --file F`).
/// A set equal to the pending candidate or the accepted version adds no version and invalidates
/// nothing; any other set becomes the new pending candidate (the previous pending one is
/// superseded). The accepted version and its rules stay as they are until a review.
pub fn capture_with(entry: &mut Entry, raw: &Value, limits: &Limits) -> Result<Value> {
    let mut state = load(entry)?;
    let specs = parse_sources(raw)?;
    let (store_label, dir, prefix) = store_root(entry)?;
    let mut snaps: Vec<Snapshot> = Vec::with_capacity(specs.len());
    for spec in &specs {
        let acquired = match &spec.path {
            Some(path) => read_local(entry, spec, path, limits)?,
            None => fetch(spec, limits)?,
        };
        let sha = sha_hex(&acquired.bytes);
        if let Some(expected) = &spec.sha256
            && *expected != sha
        {
            bail!("{}: SHA-256이 선언과 다르다(선언 {expected}, 실제 {sha})", spec.id);
        }
        write_blob(&dir, &sha, &acquired.bytes)?;
        snaps.push(Snapshot {
            id: spec.id.clone(),
            url: spec.url.clone(),
            origin: acquired.origin.to_string(),
            final_url: acquired.final_url,
            semantic: semantic(acquired.media_type.as_deref(), &acquired.bytes).to_string(),
            size: acquired.bytes.len() as u64,
            media_type: acquired.media_type,
            locator: spec.locator.clone(),
            store: store_label.to_string(),
            blob: format!("{prefix}/sha256/{sha}"),
            sha256: sha,
            retrieved_at: now(),
            http_status: acquired.http_status,
            last_modified: acquired.last_modified,
            source_modified_at: acquired.source_modified_at,
        });
    }
    let at = now();
    let set = set_sha(&snaps);
    let accepted_set = state.accepted_version().map(|v| v.set_sha256.clone());
    let pending_set = state.pending.as_deref().and_then(|p| state.version(p)).map(|v| v.set_sha256.clone());
    let accepted_version = state.accepted.as_ref().map(|a| a.version.clone());
    let d = diff(state.accepted_version(), &snaps);
    let (status, version) = if pending_set.as_deref() == Some(set.as_str()) {
        ("pending_unchanged", state.pending.clone())
    } else if accepted_set.as_deref() == Some(set.as_str()) {
        let status = match state.pending.take() {
            Some(old) => {
                let by = accepted_version.clone().unwrap_or_default();
                state.set_status(&old, "superseded", Some(&by));
                "reverted_to_accepted"
            }
            None => "unchanged",
        };
        (status, accepted_version.clone())
    } else {
        let version = format!("v{}", state.versions.len() + 1);
        if let Some(old) = state.pending.take() {
            state.set_status(&old, "superseded", Some(&version));
        }
        state.versions.push(Version {
            version: version.clone(),
            captured_at: at.clone(),
            status: "pending".into(),
            set_sha256: set.clone(),
            sources: snaps.clone(),
            diff: d.json(),
            superseded_by: None,
        });
        state.pending = Some(version.clone());
        state.last_evidence_at = Some(at.clone());
        (if accepted_version.is_some() { "changed_pending" } else { "baseline_pending" }, Some(version))
    };
    state.last_checked_at = Some(at.clone());
    state.last_check = Some(json!({"at": at, "status": status, "version": version, "set_sha256": set}));
    state.check_count += 1;
    let value = to_value(&state)?;
    entry.extra.insert(KEY.into(), value);
    Ok(json!({
        "id": entry.id,
        "status": status,
        "changed": matches!(status, "changed_pending" | "baseline_pending"),
        "pending": state.pending.is_some(),
        "version": version,
        "accepted_version": accepted_version,
        "pending_version": state.pending,
        "diff": d.json(),
        "sources": snaps.iter().map(source_summary).collect::<Vec<_>>(),
    }))
}

// ---------------------------------------------------------------------------------------------
// Review

/// Canonical form of a rule value: object keys sorted, string whitespace collapsed.
fn canon(value: &Value) -> Value {
    match value {
        Value::String(s) => Value::String(collapse(s)),
        Value::Array(items) => Value::Array(items.iter().map(canon).collect()),
        Value::Object(map) => {
            let sorted: BTreeMap<&String, Value> = map.iter().map(|(k, v)| (k, canon(v))).collect();
            Value::Object(sorted.into_iter().map(|(k, v)| (k.clone(), v)).collect())
        }
        other => other.clone(),
    }
}

fn empty_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(m) => m.is_empty(),
        _ => false,
    }
}

/// `string` → `{position}`; an object keeps non-empty `quote`, `position`, `page`.
fn parse_locator(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::String(s) if !s.trim().is_empty() => Some(json!({"position": s.trim()})),
        Value::Object(map) => {
            let mut out = Map::new();
            for field in ["quote", "position"] {
                if let Some(s) = map.get(field).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
                    out.insert(field.into(), Value::String(s.to_string()));
                }
            }
            match map.get("page") {
                Some(Value::Number(n)) => {
                    out.insert("page".into(), Value::Number(n.clone()));
                }
                Some(Value::String(s)) if !s.trim().is_empty() => {
                    out.insert("page".into(), Value::String(s.trim().to_string()));
                }
                _ => {}
            }
            (!out.is_empty()).then_some(Value::Object(out))
        }
        _ => None,
    }
}

/// Text views of a text snapshot for quote checks: the raw text and its tag-stripped form.
struct TextView {
    plain: String,
    stripped: String,
}

fn read_snapshot(entry: &Entry, snap: &Snapshot) -> Result<Option<TextView>> {
    let path = blob_path(entry, snap)?;
    let bytes = fs::read(&path).map_err(|err| Error(format!("{}: 스냅샷을 읽을 수 없다: {err}", snap.id)))?;
    let actual = sha_hex(&bytes);
    if actual != snap.sha256 {
        bail!("{}: 스냅샷이 변조됐다(기록 {}, 실제 {actual})", snap.id, snap.sha256);
    }
    if snap.semantic != "text" {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&bytes);
    Ok(Some(TextView { plain: collapse(&text), stripped: strip_tags(&text) }))
}

struct Parsed {
    rules: Vec<Rule>,
    ambiguous: Vec<usize>,
}

fn parse_rules(entry: &Entry, raw: &Value, candidate: &Version) -> Result<Parsed> {
    let Some(items) = raw.get("rules").and_then(Value::as_array).filter(|a| !a.is_empty()) else {
        bail!("rules는 비어 있지 않은 [{{category, key?, value, source: {{id, sha256, locator}}}}] 배열이어야 한다")
    };
    if items.len() > MAX_RULES {
        bail!("rules는 {MAX_RULES}개 이하여야 한다");
    }
    let mut views: BTreeMap<String, Option<TextView>> = BTreeMap::new();
    let mut seen: BTreeMap<(String, String), Value> = BTreeMap::new();
    let mut parsed = Parsed { rules: Vec::with_capacity(items.len()), ambiguous: Vec::new() };
    for (i, item) in items.iter().enumerate() {
        let Some(category) = item.get("category").and_then(Value::as_str).filter(|c| CATEGORIES.contains(c)) else {
            bail!("rules[{i}].category는 {} 중 하나여야 한다", CATEGORIES.join("|"))
        };
        let key = match item.get("key") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(k)) if k.len() <= 100 => collapse(k),
            Some(_) => bail!("rules[{i}].key는 100자 이하 문자열이어야 한다"),
        };
        let ambiguous = match item.get("ambiguous") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => bail!("rules[{i}].ambiguous는 bool이어야 한다"),
        };
        let value = canon(item.get("value").unwrap_or(&Value::Null));
        if empty_value(&value) {
            bail!("rules[{i}]({category}).value가 비어 있다");
        }
        if value.to_string().len() > MAX_VALUE {
            bail!("rules[{i}]({category}).value가 너무 크다");
        }
        let Some(source) = item.get("source").filter(|s| s.is_object()) else {
            bail!("rules[{i}]({category}).source {{id, sha256, locator}}가 없다")
        };
        let Some(id) = nonblank(source, "id") else { bail!("rules[{i}]({category}).source.id가 없다") };
        let Some(snap) = candidate.sources.iter().find(|s| s.id == id) else {
            bail!("rules[{i}]({category}).source.id {id}는 후보 {}에 없다", candidate.version)
        };
        let sha = source.get("sha256").and_then(Value::as_str).map(str::to_ascii_lowercase).unwrap_or_default();
        if !is_sha(&sha) {
            bail!("rules[{i}]({category}).source.sha256이 없거나 올바르지 않다");
        }
        if sha != snap.sha256 {
            bail!("rules[{i}]({category}).source.sha256이 후보 스냅샷 {id}와 다르다(후보 {}, 제시 {sha})", snap.sha256);
        }
        let Some(locator) = parse_locator(source.get("locator")) else {
            bail!("rules[{i}]({category}).source.locator(원문 위치나 인용)가 비어 있다")
        };
        if locator.to_string().len() > MAX_LOCATOR {
            bail!("rules[{i}]({category}).source.locator가 너무 길다");
        }
        if !views.contains_key(id) {
            views.insert(id.to_string(), read_snapshot(entry, snap)?);
        }
        let verification = match (locator.get("quote").and_then(Value::as_str), views.get(id).and_then(Option::as_ref))
        {
            (Some(quote), Some(view)) => {
                let quote = collapse(quote);
                if !view.plain.contains(&quote) && !view.stripped.contains(&quote) {
                    bail!("rules[{i}]({category}): 인용문이 스냅샷 {id}에 없다");
                }
                "quote_matched"
            }
            (Some(_), None) => "binary_unverified",
            (None, _) => "position_only",
        };
        if ambiguous {
            parsed.ambiguous.push(i);
        } else {
            let slot = (category.to_string(), key.clone());
            if let Some(previous) = seen.get(&slot)
                && *previous != value
            {
                bail!(
                    "rules[{i}]: {category}{} 값이 서로 충돌한다",
                    if key.is_empty() { String::new() } else { format!("/{key}") }
                );
            }
            seen.insert(slot, value.clone());
        }
        parsed.rules.push(Rule {
            category: category.to_string(),
            key,
            value,
            source: RuleSource { id: id.to_string(), sha256: sha, locator },
            verification: verification.to_string(),
        });
    }
    Ok(parsed)
}

fn rule_map(rules: &[Rule]) -> BTreeMap<(String, String), Value> {
    rules.iter().map(|r| ((r.category.clone(), r.key.clone()), r.value.clone())).collect()
}

fn changed_categories(old: &[Rule], new: &[Rule]) -> Vec<String> {
    let (old, new) = (rule_map(old), rule_map(new));
    let mut out = BTreeSet::new();
    for slot in old.keys().chain(new.keys()) {
        if old.get(slot) != new.get(slot) {
            out.insert(slot.0.clone());
        }
    }
    CATEGORIES.iter().filter(|c| out.contains(**c)).map(|c| c.to_string()).collect()
}

fn impacted(category: &str) -> &'static [&'static str] {
    match category {
        "eligibility" | "ai_policy" => &["gate", "plan", "final"],
        "criteria" => &["plan", "final"],
        "forms" => &["final"],
        _ => &[],
    }
}

/// Review the exact pending candidate (`ach rules review <id> --file F`): `{version, base_version,
/// decision: accept|unresolved, reviewer, reason?, rules[]}`. Either the whole review is applied or
/// nothing changes.
pub fn review(entry: &mut Entry, raw: &Value) -> Result<Value> {
    let mut state = load(entry)?;
    let Some(pending) = state.pending.clone() else { bail!("{}: 검토할 후보 규칙 버전이 없다", entry.id) };
    let Some(version) = raw.get("version").and_then(Value::as_str) else {
        bail!("version(검토하는 후보 버전)이 필요하다")
    };
    if version != pending {
        bail!("{}: 검토가 낡았다: 지금 후보는 {pending}이다(제시 {version})", entry.id);
    }
    let base = match raw.get("base_version") {
        None => bail!("base_version이 필요하다(지금 수락된 버전, 없으면 null)"),
        Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.as_str()),
        Some(_) => bail!("base_version은 문자열이나 null이어야 한다"),
    };
    let accepted_version = state.accepted.as_ref().map(|a| a.version.clone());
    if base != accepted_version.as_deref() {
        bail!("{}: 검토가 낡았다: 수락된 버전은 {:?}이다(제시 {base:?})", entry.id, accepted_version);
    }
    let Some(reviewer) = nonblank(raw, "reviewer") else { bail!("reviewer가 비어 있다") };
    let decision = raw.get("decision").and_then(Value::as_str).unwrap_or("");
    if !matches!(decision, "accept" | "unresolved") {
        bail!("decision은 accept 또는 unresolved여야 한다");
    }
    let Some(candidate) = state.version(&pending).cloned() else {
        bail!("{}: 후보 {pending}의 스냅샷 기록이 없다", entry.id)
    };
    let at = now();
    let unresolved = |state: &mut State, reason: &str, ambiguous: &[usize]| -> Result<Value> {
        state.reviews.push(json!({"at": at, "version": pending, "base_version": base, "decision": "unresolved",
            "reviewer": reviewer, "reason": reason, "ambiguous_rules": ambiguous}));
        Ok(json!({"id": entry.id, "status": "pending", "version": pending, "reason": reason,
            "ambiguous_rules": ambiguous, "changed_categories": [], "invalidated": [], "binding": state.epochs.json()}))
    };
    if decision == "unresolved" {
        let Some(reason) = nonblank(raw, "reason") else { bail!("unresolved 검토는 reason이 필요하다") };
        let result = unresolved(&mut state, reason, &[][..])?;
        let value = to_value(&state)?;
        entry.extra.insert(KEY.into(), value);
        return Ok(result);
    }
    let parsed = parse_rules(entry, raw, &candidate)?;
    if !parsed.ambiguous.is_empty() {
        let result = unresolved(&mut state, "ambiguous_rule", parsed.ambiguous.as_slice())?;
        let value = to_value(&state)?;
        entry.extra.insert(KEY.into(), value);
        return Ok(result);
    }
    let d = diff(state.accepted_version(), &candidate.sources);
    for id in d.added.iter().chain(&d.changed) {
        if !parsed.rules.iter().any(|r| r.source.id == *id) {
            bail!("{}: 바뀐 출처 {id}를 인용한 규칙이 없다 — 의미를 모르는 변경은 승인할 수 없다", entry.id);
        }
    }
    let baseline = state.accepted.is_none();
    let changed = match &state.accepted {
        Some(accepted) => changed_categories(&accepted.rules, &parsed.rules),
        None => Vec::new(),
    };
    let mut invalidated: Vec<&str> = Vec::new();
    if baseline {
        state.epochs = Epochs { gate: 1, plan: 1, final_: 1, submission: 1 };
    } else if !changed.is_empty() {
        for stage in ["gate", "plan", "final"] {
            let cats: Vec<String> = changed.iter().filter(|c| impacted(c).contains(&stage)).cloned().collect();
            if cats.is_empty() {
                continue;
            }
            let from = state.epochs.get(stage);
            let epoch = state.epochs.bump(stage);
            match state.invalidated.get_mut(stage) {
                Some(inv) => {
                    inv.epoch = epoch;
                    inv.since_version = pending.clone();
                    for c in cats {
                        if !inv.categories.contains(&c) {
                            inv.categories.push(c);
                        }
                    }
                    inv.at = at.clone();
                }
                None => {
                    state.invalidated.insert(
                        stage.to_string(),
                        Invalidation {
                            since_version: pending.clone(),
                            from_epoch: from,
                            epoch,
                            categories: cats,
                            at: at.clone(),
                        },
                    );
                }
            }
            invalidated.push(stage);
        }
        state.epochs.bump("submission");
    }
    if let Some(old) = &accepted_version {
        state.set_status(old, "retired", Some(&pending));
    }
    state.set_status(&pending, "accepted", None);
    state.accepted = Some(Accepted {
        version: pending.clone(),
        accepted_at: at.clone(),
        reviewer: reviewer.to_string(),
        rules: parsed.rules,
    });
    state.pending = None;
    state.last_evidence_at = Some(at.clone());
    let submission_changed = !baseline && !changed.is_empty();
    state.reviews.push(json!({"at": at, "version": pending, "base_version": base, "decision": "accept",
        "reviewer": reviewer, "baseline": baseline, "changed_categories": changed, "invalidated": invalidated,
        "submission_epoch_changed": submission_changed}));
    let value = to_value(&state)?;
    entry.extra.insert(KEY.into(), value);
    Ok(json!({"id": entry.id, "status": "accepted", "version": pending, "baseline": baseline,
        "changed_categories": changed, "invalidated": invalidated, "submission_epoch_changed": submission_changed,
        "binding": state.epochs.json()}))
}

// ---------------------------------------------------------------------------------------------
// Acknowledgement, score hooks and status

/// Acknowledgement evidence `[{path, sha256}]`, hashes checked now. With a contest folder the files
/// must be inside it (`readiness::normalize_artifacts`); before routing (a re-gate) they are
/// absolute regular files, such as the new gate.json in the gate's work directory.
fn evidence_artifacts(entry: &Entry, artifacts: &Value) -> Result<Vec<Value>> {
    if entry.folder.as_deref().is_some_and(|f| !f.is_empty()) {
        return readiness::normalize_artifacts(entry, artifacts);
    }
    let Some(items) = artifacts.as_array().filter(|a| !a.is_empty()) else {
        bail!("{}: evidence는 비어 있지 않은 [{{path, sha256}}] 배열이어야 한다", entry.id)
    };
    let mut out: Vec<Value> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let Some(path) = item.get("path").and_then(Value::as_str).filter(|p| Path::new(p).is_absolute()) else {
            bail!("{}: 폴더가 없을 때 evidence[{i}].path는 절대 경로여야 한다", entry.id)
        };
        let sha = item.get("sha256").and_then(Value::as_str).map(str::to_ascii_lowercase).unwrap_or_default();
        if !is_sha(&sha) {
            bail!("{}: evidence[{i}].sha256은 64자리 16진수여야 한다", entry.id);
        }
        let full = Path::new(path);
        if !fs::metadata(full).map(|m| m.is_file()).unwrap_or(false) {
            bail!("{}: evidence[{i}] 일반 파일이 아니다: {path}", entry.id);
        }
        let actual = readiness::sha256_file(full)?;
        if actual != sha {
            bail!("{}: evidence[{i}]의 SHA-256이 다르다(기록 {sha}, 지금 {actual})", entry.id);
        }
        if out.iter().any(|a| a["path"] == path) {
            bail!("{}: evidence에 같은 파일이 두 번 있다", entry.id);
        }
        out.push(json!({"path": path, "sha256": sha}));
    }
    Ok(out)
}

/// Clear one invalidated stage with an explicit reviewed pass (`ach rules acknowledge <id>
/// --stage gate|plan|final --file F`): `{rules_version, epoch, decision: "pass", reviewer,
/// rationale, evidence: [{path, sha256}]}` bound to the accepted version and the stage's epoch.
pub fn acknowledge(entry: &mut Entry, stage: &str, raw: &Value) -> Result<Value> {
    if !ACK_STAGES.contains(&stage) {
        bail!("stage는 gate, plan, final 중 하나여야 한다: {stage}");
    }
    let mut state = load(entry)?;
    let Some(accepted) = state.accepted.as_ref() else {
        bail!("{}: 공식 규칙이 추적되고 있지 않다", entry.id)
    };
    if let Some(pending) = &state.pending {
        bail!("{}: 규칙 후보 {pending}의 검토가 먼저다", entry.id);
    }
    let Some(inv) = state.invalidated.get(stage).cloned() else {
        bail!("{}: {stage} 단계는 무효화돼 있지 않다", entry.id)
    };
    if raw.get("rules_version").and_then(Value::as_str) != Some(accepted.version.as_str()) {
        bail!("{}: rules_version이 수락된 규칙 {}와 다르다", entry.id, accepted.version);
    }
    let epoch = state.epochs.get(stage);
    if raw.get("epoch").and_then(Value::as_u64) != Some(epoch) {
        bail!("{}: epoch가 지금 {stage} epoch {epoch}와 다르다", entry.id);
    }
    if raw.get("decision").and_then(Value::as_str) != Some("pass") {
        bail!("decision은 pass여야 한다(통과하지 못하면 다시 작업한다)");
    }
    let Some(reviewer) = nonblank(raw, "reviewer").filter(|r| !r.eq_ignore_ascii_case("ach")) else {
        bail!("reviewer가 비어 있다")
    };
    let Some(rationale) = nonblank(raw, "rationale").filter(|r| r.chars().count() >= MIN_RATIONALE) else {
        bail!("rationale은 {MIN_RATIONALE}자 이상 구체적으로 적어야 한다")
    };
    let evidence = evidence_artifacts(entry, raw.get("evidence").unwrap_or(&Value::Null))?;
    let version = accepted.version.clone();
    state.acknowledgements.push(Clearance {
        at: now(),
        stage: stage.to_string(),
        kind: "review".into(),
        rules_version: version.clone(),
        from_epoch: inv.from_epoch,
        epoch,
        reviewer: reviewer.to_string(),
        rationale: rationale.to_string(),
        evidence,
        score_index: None,
    });
    state.invalidated.remove(stage);
    let value = to_value(&state)?;
    entry.extra.insert(KEY.into(), value);
    Ok(json!({"id": entry.id, "stage": stage, "status": "acknowledged", "rules_version": version,
        "binding": state.epochs.json(), "remaining": state.invalidated.keys().collect::<Vec<_>>()}))
}

/// The epoch a score stage is judged against: plan → plan, draft and final → final.
fn score_epoch_key(stage: &str) -> Option<&'static str> {
    match stage {
        "gate" => Some("gate"),
        "plan" => Some("plan"),
        "draft" | "final" => Some("final"),
        _ => None,
    }
}

/// Stamp the just-recorded score (the last one, of `stage`) with the current binding and clear
/// the plan/final invalidation it answers. Called by the parent after `record_score`; everything is
/// validated before anything changes. Untracked rules stamp nothing. A pending source review
/// fails: a score cannot answer rules nobody has read.
pub fn score_recorded(entry: &mut Entry, stage: &str) -> Result<()> {
    if !matches!(stage, "plan" | "draft" | "final") {
        bail!("stage는 plan, draft, final 중 하나여야 한다: {stage}");
    }
    let mut state = load(entry)?;
    let index = entry.scores.len().checked_sub(1).ok_or_else(|| Error(format!("{}: 기록된 점수가 없다", entry.id)))?;
    let last = &entry.scores[index];
    if !last.is_object() || last.get("stage").and_then(Value::as_str) != Some(stage) {
        bail!("{}: 마지막 점수가 {stage} 단계가 아니다", entry.id);
    }
    if let Some(pending) = &state.pending {
        bail!("{}: 공식 규칙 후보 {pending}이 검토 대기 중이다 — rules review가 먼저다", entry.id);
    }
    let Some(accepted) = state.accepted.as_ref() else { return Ok(()) };
    let mut stamp = state.epochs.json();
    stamp["version"] = Value::String(accepted.version.clone());
    if stage != "draft"
        && let Some(inv) = state.invalidated.remove(stage)
    {
        let rules_version = accepted.version.clone();
        state.acknowledgements.push(Clearance {
            at: now(),
            stage: stage.to_string(),
            kind: "score".into(),
            rules_version,
            from_epoch: inv.from_epoch,
            epoch: state.epochs.get(stage),
            reviewer: String::new(),
            rationale: String::new(),
            evidence: Vec::new(),
            score_index: Some(index),
        });
    }
    let value = to_value(&state)?;
    if let Some(obj) = entry.scores[index].as_object_mut() {
        obj.insert("rules_binding".into(), stamp);
    }
    entry.extra.insert(KEY.into(), value);
    Ok(())
}

/// The score still answers the current rules: untracked rules accept any score; tracked rules need
/// a stamp equal to the current stage epoch, or one an explicit review carried forward.
pub fn check_score(entry: &Entry, score: &Value, stage: &str) -> bool {
    let Ok(state) = load(entry) else { return false };
    if state.accepted.is_none() {
        return true;
    }
    let Some(key) = score_epoch_key(stage) else { return false };
    let Some(stamp) = score.get("rules_binding").and_then(|b| b.get(key)).and_then(Value::as_u64) else {
        return false;
    };
    let mut epoch = state.epochs.get(key);
    loop {
        if stamp == epoch {
            return true;
        }
        let ack = state
            .acknowledgements
            .iter()
            .rev()
            .find(|a| a.kind == "review" && a.stage == key && a.epoch == epoch && a.from_epoch < epoch);
        match ack {
            Some(ack) if stamp >= ack.from_epoch && stamp < epoch => return true,
            Some(ack) => epoch = ack.from_epoch,
            None => return false,
        }
    }
}

/// Official rules are tracked: a reviewed version has been accepted.
pub fn tracked(entry: &Entry) -> bool {
    load(entry).map(|s| s.accepted.is_some()).unwrap_or(false)
}

/// The current accepted epochs `{gate, plan, final, submission}`; all zero when untracked.
pub fn binding(entry: &Entry) -> Value {
    load(entry).map(|s| s.epochs.json()).unwrap_or_else(|_| Epochs::default().json())
}

fn problems_for(entry: &Entry, stages: &[&str]) -> Vec<&'static str> {
    let Ok(state) = load(entry) else { return vec!["rules_state_invalid"] };
    let mut out = Vec::new();
    if state.pending.is_some() {
        out.push("rules_pending_review");
    }
    for (stage, code) in
        [("gate", "rules_gate_invalidated"), ("plan", "rules_plan_invalidated"), ("final", "rules_final_invalidated")]
    {
        if stages.contains(&stage) && state.invalidated.contains_key(stage) {
            out.push(code);
        }
    }
    out
}

/// Why a submission approval may not rely on the official rules now: a pending review or any
/// outstanding invalidation. Untracked rules are not a problem here (see `status().tracked`).
pub fn approval_problems(entry: &Entry) -> Vec<&'static str> {
    problems_for(entry, &["gate", "plan", "final"])
}

/// The rule problems that block `stage`: `gate` → pending + gate; `plan` (plan review, plan score)
/// → pending + gate + plan; `draft`/`final` → pending + gate + plan + final; `submit`/`submission`
/// → [`approval_problems`]. A final or submission change never blocks a plan review.
pub fn stage_problems(entry: &Entry, stage: &str) -> Vec<&'static str> {
    match stage {
        "gate" => problems_for(entry, &["gate"]),
        "plan" | "plan_review" => problems_for(entry, &["gate", "plan"]),
        _ => approval_problems(entry),
    }
}

/// `ach rules status <id>`: tracking, the accepted and pending versions, outstanding
/// invalidations, the binding and the latest check. Reads only the stored state.
pub fn status(entry: &Entry) -> Value {
    let state = match load(entry) {
        Ok(state) => state,
        Err(err) => {
            return json!({"id": entry.id, "tracked": false, "error": err.0, "problems": ["rules_state_invalid"],
                "binding": Epochs::default().json()});
        }
    };
    let pending = state.pending.as_deref().and_then(|p| state.version(p)).map(|v| {
        json!({"version": v.version, "captured_at": v.captured_at, "diff": v.diff,
            "sources": v.sources.iter().map(|s| s.id.clone()).collect::<Vec<_>>()})
    });
    let sources = state
        .accepted_version()
        .map(|v| {
            v.sources
                .iter()
                .map(|s| {
                    json!({"id": s.id, "url": s.url, "sha256": s.sha256, "semantic": s.semantic,
                        "retrieved_at": s.retrieved_at})
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let rules = state.accepted.as_ref().map(|a| {
        let mut by_category: BTreeMap<&str, usize> = BTreeMap::new();
        for r in &a.rules {
            *by_category.entry(r.category.as_str()).or_default() += 1;
        }
        json!(by_category)
    });
    json!({
        "id": entry.id,
        "tracked": state.accepted.is_some(),
        "accepted_version": state.accepted.as_ref().map(|a| a.version.clone()),
        "accepted_at": state.accepted.as_ref().map(|a| a.accepted_at.clone()),
        "rule_counts": rules,
        "pending": pending,
        "invalidated": serde_json::to_value(&state.invalidated).unwrap_or(Value::Null),
        "binding": state.epochs.json(),
        "last_checked_at": state.last_checked_at,
        "last_check_status": state.last_check.as_ref().and_then(|c| c.get("status")).cloned(),
        "last_evidence_at": state.last_evidence_at,
        "sources": sources,
        "problems": approval_problems(entry),
    })
}
