//! The main-session system prompt: an optional, user-supplied persona file plus the main role.
//! Own test binary because it points `ACH_HOME` at a scratch root for the whole process.

use std::path::PathBuf;

use harness::config::{self, Config};
use harness::launch;
use serde_json::json;

fn config_with(persona: serde_json::Value) -> Config {
    serde_json::from_value(config::merge(config::defaults(), json!({ "persona": persona }))).unwrap()
}

#[test]
fn main_system_prompt_uses_the_configured_persona_file() {
    let home = std::env::temp_dir().join(format!("harness-persona-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("harness")).unwrap();
    std::fs::create_dir_all(home.join("data")).unwrap();
    std::os::unix::fs::symlink(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("harness/prompts"),
        home.join("harness/prompts"),
    )
    .unwrap();
    // SAFETY: the only test in this binary, set before anything reads the environment.
    unsafe { std::env::set_var("ACH_HOME", &home) };
    let main = std::fs::read_to_string(home.join("harness/prompts/main.md")).unwrap();

    // Public defaults: no persona, no names.
    let defaults = config_with(json!({}));
    assert!(!defaults.persona.enabled);
    assert_eq!(defaults.persona.file, "data/persona.md");
    assert!(defaults.persona.call_user.is_empty() && defaults.persona.guard_terms.is_empty());
    let out = launch::main_system_prompt(&defaults).unwrap();
    assert_eq!(out, home.join("data/main-system.md"));
    assert_eq!(std::fs::read_to_string(&out).unwrap(), format!("{}\n", main.trim()));

    // Enabled without the file: a clear error that points at the example.
    let err = launch::main_system_prompt(&config_with(json!({"enabled": true}))).unwrap_err().0;
    assert!(err.contains("data/persona.md") && err.contains("data/persona.example.md"), "{err}");

    // Relative to the root, `{{call_user}}` replaced.
    std::fs::write(home.join("data/persona.md"), "나는 비서다. {{call_user}}를 돕는다.\n").unwrap();
    let out = launch::main_system_prompt(&config_with(json!({"enabled": true, "call_user": "대표님"}))).unwrap();
    assert_eq!(std::fs::read_to_string(&out).unwrap(), format!("나는 비서다. 대표님를 돕는다.\n\n{}\n", main.trim()));

    // Empty call_user leaves the placeholder; absolute paths work too.
    let elsewhere = home.join("mine.md");
    std::fs::write(&elsewhere, "{{call_user}} 전용").unwrap();
    let cfg = config_with(json!({"enabled": true, "file": elsewhere.display().to_string()}));
    let text = std::fs::read_to_string(launch::main_system_prompt(&cfg).unwrap()).unwrap();
    assert!(text.starts_with("{{call_user}} 전용\n\n"), "{text}");
}
