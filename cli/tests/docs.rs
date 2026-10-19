//! docs/cli.md stays in step with the code.

use std::process::Command;

const DOC: &str = include_str!("../../docs/cli.md");

#[test]
fn every_error_code_is_documented_with_its_exit_code() {
    for c in gitstory_proto::ErrorCode::ALL {
        let row = format!("| `{}` | {} |", c.as_str(), c.exit_code() as i32);
        assert!(
            DOC.contains(&row),
            "docs/cli.md: missing or wrong row for {c}"
        );
    }
}

#[test]
fn every_command_is_documented() {
    let out = Command::new(env!("CARGO_BIN_EXE_git-story"))
        .arg("--help")
        .output()
        .unwrap();
    let help = String::from_utf8_lossy(&out.stdout);
    let commands = help
        .split("Commands:")
        .nth(1)
        .unwrap()
        .split("\n\n")
        .next()
        .unwrap()
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|c| *c != "help");
    for c in commands {
        let heading = DOC.lines().filter(|l| l.starts_with("### ")).any(|l| {
            l.contains(&format!("`{c} "))
                || l.contains(&format!("`{c}`"))
                || l.contains(&format!("· `{c}"))
        });
        assert!(heading, "docs/cli.md has no heading for `{c}`");
    }
}
