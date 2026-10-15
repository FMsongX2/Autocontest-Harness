//! Autocontest-Harness core. The `harness` binary in `main.rs` is a thin CLI over these modules.
//! Contract: `docs/CONTRACT.md`. Every command keeps the JSON shapes that agents and prompts rely on.

pub mod board;
pub mod budget;
pub mod clean;
pub mod collect;
pub mod config;
pub mod dedupe;
pub mod doctor;
pub mod error;
pub mod guard;
pub mod http;
pub mod install;
pub mod launch;
pub mod ledger;
pub mod paths;
pub mod process;
pub mod profile;
pub mod registry;
pub mod route;
pub mod runners;
pub mod schedule;
pub mod sources;
pub mod store;
pub mod text;
pub mod workspaces;
