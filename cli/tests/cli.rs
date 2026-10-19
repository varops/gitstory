use std::process::Command;

use serde_json::Value;

fn git_story(args: &[&str]) -> (i32, Value) {
    let dir = std::env::temp_dir();
    let out = Command::new(env!("CARGO_BIN_EXE_git-story"))
        .args(args)
        .current_dir(&dir)
        .env("GIT_CEILING_DIRECTORIES", &dir)
        .output()
        .expect("run git-story");
    let json = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    (out.status.code().expect("exit code"), json)
}

#[test]
fn bare_command_shows_head() {
    let (code, json) = git_story(&["--json"]);
    assert_eq!(json["schema"], "gitstory.cli/v1");
    assert_eq!(json["command"], "show");
    assert_eq!(json["error"]["code"], "E_NOT_A_REPO");
    assert_ne!(code, 0);
}

#[test]
fn subcommands_report_their_names() {
    let (_, json) = git_story(&["blame", "src/lock.py:40-80", "--brief", "--json"]);
    assert_eq!(json["command"], "blame");
}

#[test]
fn usage_errors_are_json_when_requested() {
    let (code, json) = git_story(&["note", "--test", "pytest", "--json"]);
    assert_eq!(code, 2);
    assert_eq!(json["error"]["code"], "E_USAGE");
}

#[test]
fn skill_show_prints_the_embedded_skill() {
    let (code, json) = git_story(&["skill", "show", "--json"]);
    assert_eq!(code, 0);
    assert!(
        json["data"]["skill"]
            .as_str()
            .unwrap()
            .contains("git story")
    );
}
