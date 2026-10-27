//! Private state files under `data/`: the directory is created 0700, files are written 0600 through
//! a temp file + rename, newer schema versions are refused, and crashed writers' temp files are removed.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use serde::Deserialize;

use crate::bail;
use crate::error::{Error, Result};

fn at(path: &Path, err: impl std::fmt::Display) -> Error {
    Error(format!("{}: {err}", path.display()))
}

fn parent(path: &Path) -> &Path {
    path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."))
}

/// Create `dir` and its missing parents with mode 0700. An existing directory is left as it is.
pub fn private_dir(dir: &Path) -> Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|err| at(dir, err))
}

/// The lock file beside `path` (`<stem>.lock`), opened for `flock`. When the directory does not
/// exist, `create_dir` decides: read paths get `None` (they never create `data/`), write paths create it.
pub fn lock_file(path: &Path, create_dir: bool) -> Result<Option<File>> {
    let dir = parent(path);
    if !dir.is_dir() {
        if !create_dir {
            return Ok(None);
        }
        private_dir(dir)?;
    }
    let lock = path.with_extension("lock");
    let file = OpenOptions::new().create(true).truncate(false).write(true).mode(0o600).open(&lock);
    file.map(Some).map_err(|err| at(&lock, err))
}

/// The file's text, `None` when it does not exist. Other read errors name the path.
pub fn read(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(at(path, err)),
    }
}

/// Fail when the file's top-level `version` is newer than this build understands, before the
/// rest of the file is interpreted (and before a save could drop what the newer format means).
pub fn check_version(path: &Path, text: &str, supported: u32) -> Result<()> {
    #[derive(Deserialize)]
    struct Probe {
        #[serde(default)]
        version: Option<u64>,
    }
    let probe: Probe = serde_json::from_str(text).map_err(|err| at(path, err))?;
    if let Some(version) = probe.version
        && version > u64::from(supported)
    {
        bail!(
            "{}: 파일 형식 버전 {version}은 이 하네스가 아는 버전({supported})보다 새롭다. 하네스를 업그레이드한다(git pull && scripts/install.sh --from-source)",
            path.display()
        );
    }
    Ok(())
}

/// Write `bytes` to `path` atomically with mode 0600: a temp file `.<stem>-<pid>.<ext>` in the same
/// directory, fsync, rename, then fsync the directory. `exclusive` means the caller holds the
/// file's exclusive lock, so any other `.<stem>-<pid>.<ext>` there is a crashed writer's leftover
/// and is removed.
pub fn write_atomic(path: &Path, bytes: &[u8], exclusive: bool) -> Result<()> {
    let dir = parent(path);
    private_dir(dir)?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    let pid = std::process::id();
    let tmp = dir.join(format!(".{stem}-{pid}{ext}"));
    if exclusive {
        remove_leftovers(dir, &stem, &ext, pid);
    }
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|err| at(&tmp, err))?;
    // A leftover with our pid keeps its old mode through `open`, so set it explicitly.
    file.set_permissions(fs::Permissions::from_mode(0o600)).map_err(|err| at(&tmp, err))?;
    file.write_all(bytes).and_then(|()| file.sync_all()).map_err(|err| at(&tmp, err))?;
    drop(file);
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(at(path, err));
    }
    // Make the rename itself durable. Some filesystems refuse fsync on a directory; the data is
    // already safe then, only the rename may roll back after a power loss.
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
    Ok(())
}

/// Remove `.<stem>-<digits><ext>` files other than our own.
fn remove_leftovers(dir: &Path, stem: &str, ext: &str, pid: u32) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let prefix = format!(".{stem}-");
    let own = format!("{pid}");
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(middle) = name.strip_prefix(&prefix).and_then(|rest| rest.strip_suffix(ext)) else { continue };
        if !middle.is_empty() && middle.bytes().all(|b| b.is_ascii_digit()) && middle != own {
            let _ = fs::remove_file(entry.path());
        }
    }
}
