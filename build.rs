//! Embed the git commit the binary was built from (`HARNESS_GIT_SHA`), so `harness --version` and
//! `harness doctor` can tell when the installed binary is older than the checkout it reads.

use std::path::Path;
use std::process::Command;

fn main() {
    // Release builds from an exported tree (no .git) pass the commit in explicitly.
    println!("cargo:rerun-if-env-changed=HARNESS_GIT_SHA");
    let sha = std::env::var("HARNESS_GIT_SHA").ok().filter(|sha| !sha.is_empty()).unwrap_or_else(|| {
        Command::new("git")
            .args(["rev-parse", "--short=12", "HEAD"])
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|sha| !sha.is_empty())
            .unwrap_or_else(|| "unknown".to_string())
    });
    println!("cargo:rustc-env=HARNESS_GIT_SHA={sha}");

    // Rebuild when HEAD moves: HEAD itself, the branch it points at, and packed refs.
    let git = Path::new(".git");
    if git.is_dir() {
        println!("cargo:rerun-if-changed=.git/HEAD");
        println!("cargo:rerun-if-changed=.git/packed-refs");
        if let Ok(head) = std::fs::read_to_string(git.join("HEAD"))
            && let Some(reference) = head.trim().strip_prefix("ref: ")
        {
            println!("cargo:rerun-if-changed=.git/{reference}");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
