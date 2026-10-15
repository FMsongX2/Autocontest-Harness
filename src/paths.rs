//! Where the harness keeps its files. The repository root is `HARNESS_HOME` when set, otherwise the
//! checkout the binary was built from (so `cargo install --path .` keeps working from the clone).

use std::path::PathBuf;

pub fn root() -> PathBuf {
    std::env::var_os("HARNESS_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
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
    data().join("profile.json")
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
