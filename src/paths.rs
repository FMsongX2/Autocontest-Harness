//! Where the harness keeps its files. The repository root is `HARNESS_HOME` when set, otherwise the
//! checkout that holds the running binary (`<clone>/target/release/harness`, `<clone>/bin/harness`),
//! otherwise the checkout the binary was built from (so `cargo install --path .` keeps working from
//! the clone). `main` refuses to run when that root is not a harness checkout ([`checkout_ok`]).

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// A file every harness checkout has; its presence is what makes a directory the root.
pub const MARKER: &str = "harness/prompts/launch.md";

static BINARY_ROOT: LazyLock<PathBuf> = LazyLock::new(|| {
    let exe = std::env::current_exe().and_then(|exe| exe.canonicalize()).ok();
    exe.and_then(|exe| exe.ancestors().skip(1).find(|dir| checkout_ok(dir)).map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
});

pub fn root() -> PathBuf {
    match std::env::var_os("HARNESS_HOME").filter(|home| !home.is_empty()) {
        Some(home) => PathBuf::from(home),
        None => BINARY_ROOT.clone(),
    }
}

/// `root` is a harness checkout (it has [`MARKER`]).
pub fn checkout_ok(root: &Path) -> bool {
    root.join(MARKER).is_file()
}

/// The root, or an error naming it when it is not a harness checkout (moved or deleted clone, typo
/// in `HARNESS_HOME`), so no command silently starts an empty registry somewhere else.
pub fn checked_root() -> crate::error::Result<PathBuf> {
    let root = root();
    if checkout_ok(&root) {
        return Ok(root);
    }
    let source = if std::env::var_os("HARNESS_HOME").is_some_and(|h| !h.is_empty()) {
        "HARNESS_HOME"
    } else {
        "실행 파일 위치나 빌드한 클론"
    };
    crate::bail!(
        "하네스 클론을 찾지 못했다: {}에 {MARKER}가 없다({source} 기준). HARNESS_HOME을 클론 경로로 지정하거나 클론에서 다시 설치한다(cargo install --path . --root ~/.local --force)",
        root.display()
    )
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// `~` and `~/...` expand to the home directory; everything else is taken as is.
pub fn expand(path: &str) -> PathBuf {
    match path.strip_prefix('~') {
        Some("") => home(),
        Some(rest) if rest.starts_with('/') => home().join(&rest[1..]),
        _ => PathBuf::from(path),
    }
}

pub fn data() -> PathBuf {
    root().join("data")
}

pub fn registry() -> PathBuf {
    data().join("registry.json")
}

pub fn config_file() -> PathBuf {
    data().join("config.json")
}

pub fn profile() -> PathBuf {
    data().join("profile.md")
}

/// The submission ledger: every contest already submitted (git-ignored).
pub fn submissions() -> PathBuf {
    data().join("submissions.json")
}

pub fn board() -> PathBuf {
    root().join("BOARD.md")
}

pub fn prompts() -> PathBuf {
    root().join("harness").join("prompts")
}

pub fn templates() -> PathBuf {
    root().join("harness").join("templates")
}

pub fn agents() -> PathBuf {
    root().join("agents")
}

pub fn skill() -> PathBuf {
    root().join("skills").join("contest-harness")
}

/// A configured file path: `~`/absolute as given, anything else relative to the repository root.
pub fn resolve(path: &str) -> PathBuf {
    let expanded = expand(path);
    if expanded.is_absolute() { expanded } else { root().join(expanded) }
}

pub fn omp_agent_dir() -> PathBuf {
    home().join(".omp").join("agent")
}
