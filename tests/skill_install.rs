// These tests isolate installations via HOME, which dirs honors on Unix.
#![cfg(unix)]

mod support;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use support::*;

fn install(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jit"))
        .args(["skill", "install"])
        .args(args)
        .env("HOME", home)
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .output()
        .unwrap()
}

#[test]
fn installs_bundled_skill_for_all_supported_agents_without_network_or_jira_config() {
    let temp = TempConfig::empty();
    let home = Path::new(temp.path_str()).parent().unwrap();
    let codex = home.join(".codex/skills/jit/SKILL.md");
    fs::create_dir_all(codex.parent().unwrap()).unwrap();
    fs::write(&codex, "outdated skill").unwrap();
    for config in [".claude", ".pi/agent", ".cursor", ".gemini/config"] {
        fs::create_dir_all(home.join(config)).unwrap();
    }
    let output = install(home, &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    for path in [
        codex,
        home.join(".claude/skills/jit/SKILL.md"),
        home.join(".pi/agent/skills/jit/SKILL.md"),
        home.join(".cursor/skills/jit/SKILL.md"),
        home.join(".gemini/config/skills/jit/SKILL.md"),
    ] {
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            include_str!("../SKILL.md")
        );
    }
    assert!(stdout(&output).contains("Installed codex skill"));
    assert!(stdout(&output).contains("Installed claude skill"));
}

#[test]
fn installs_only_selected_agent_and_leaves_other_skill_untouched() {
    let temp = TempConfig::empty();
    let home = Path::new(temp.path_str()).parent().unwrap();
    let claude = home.join(".claude/skills/jit/SKILL.md");
    fs::create_dir_all(claude.parent().unwrap()).unwrap();
    fs::write(&claude, "existing Claude skill").unwrap();
    let output = install(home, &["--agent", "codex"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        fs::read_to_string(home.join(".codex/skills/jit/SKILL.md")).unwrap(),
        include_str!("../SKILL.md")
    );
    assert_eq!(fs::read_to_string(claude).unwrap(), "existing Claude skill");
}

#[test]
fn rejects_unknown_agent_without_creating_install_directories() {
    let temp = TempConfig::empty();
    let home = Path::new(temp.path_str()).parent().unwrap();
    let output = install(home, &["--agent", "unknown"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("Unknown agent"));
    assert!(!home.join(".codex").exists());
    assert!(!home.join(".claude").exists());
}

#[test]
fn selects_multiple_agents_with_commas_and_repeated_flags_without_duplicates() {
    let temp = TempConfig::empty();
    let home = Path::new(temp.path_str()).parent().unwrap();
    let output = install(home, &["--agent", "pi,cursor", "--agent", "ANTIGRAVITY,pi"]);
    assert!(output.status.success(), "{}", stderr(&output));
    for path in [
        ".pi/agent/skills/jit/SKILL.md",
        ".cursor/skills/jit/SKILL.md",
        ".gemini/config/skills/jit/SKILL.md",
    ] {
        assert_eq!(
            fs::read_to_string(home.join(path)).unwrap(),
            include_str!("../SKILL.md")
        );
    }
    assert!(!home.join(".codex").exists());
    assert!(!home.join(".claude").exists());
    assert_eq!(stdout(&output).matches("Installed pi skill").count(), 1);
}

#[test]
fn validates_all_selected_agents_before_writing_any_files() {
    let temp = TempConfig::empty();
    let home = Path::new(temp.path_str()).parent().unwrap();
    let output = install(home, &["--agent", "codex,unknown"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("Unknown agent"));
    assert!(!home.join(".codex").exists());
}

#[test]
fn default_install_detects_config_folders_and_skips_absent_agents() {
    let temp = TempConfig::empty();
    let home = Path::new(temp.path_str()).parent().unwrap();
    fs::create_dir_all(home.join(".claude")).unwrap();
    fs::create_dir_all(home.join(".pi/agent")).unwrap();
    let output = install(home, &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    for path in [
        ".claude/skills/jit/SKILL.md",
        ".pi/agent/skills/jit/SKILL.md",
    ] {
        assert_eq!(
            fs::read_to_string(home.join(path)).unwrap(),
            include_str!("../SKILL.md")
        );
    }
    for absent in [".codex", ".cursor", ".gemini"] {
        assert!(
            !home.join(absent).exists(),
            "Created absent agent folder {absent}"
        );
    }
}

#[test]
fn default_install_without_agent_configs_is_a_clear_noop() {
    let temp = TempConfig::empty();
    let home = Path::new(temp.path_str()).parent().unwrap();
    // The generic Gemini directory alone does not identify Antigravity.
    fs::create_dir(home.join(".gemini")).unwrap();
    fs::write(home.join(".cursor"), "not a config directory").unwrap();
    let output = install(home, &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("No supported agent config folders found"));
    assert!(!stdout(&output).contains("Installed"));
    for absent in [
        ".codex",
        ".claude",
        ".pi",
        ".gemini/config",
        ".gemini/antigravity",
    ] {
        assert!(!home.join(absent).exists());
    }
}

#[test]
fn default_install_uses_existing_legacy_antigravity_config() {
    let temp = TempConfig::empty();
    let home = Path::new(temp.path_str()).parent().unwrap();
    fs::create_dir_all(home.join(".gemini/antigravity")).unwrap();
    let output = install(home, &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        fs::read_to_string(home.join(".gemini/antigravity/skills/jit/SKILL.md")).unwrap(),
        include_str!("../SKILL.md")
    );
    assert!(!home.join(".gemini/config").exists());
    assert!(!home.join(".codex").exists());
}
