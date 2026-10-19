use std::process::Command;

use serde_json::Value;

fn git_story(args: &[&str]) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_git-story"))
        .args(args)
        .output()
        .expect("run git-story");
    let json = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    (out.status.code().expect("exit code"), json)
}

#[test]
fn bare_command_shows_head() {
    let (code, json) = git_story(&["--json"]);
    assert_eq!(code, 64);
    assert_eq!(json["schema"], "gitstory.cli/v1");
    assert_eq!(json["command"], "show");
    assert_eq!(json["error"]["code"], "E_NOT_IMPLEMENTED");
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
