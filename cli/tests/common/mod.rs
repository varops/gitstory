//! Shared setup for the integration tests: a Git repository with
//! `git story init` done and the deterministic stand-in model.
#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

pub const MOCK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mock_model.py");

pub fn python() -> &'static str {
    if cfg!(windows) { "python" } else { "python3" }
}

pub fn tmp(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let d = std::env::temp_dir().join(format!("gitstory-{name}-{nanos}"));
    fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

pub struct Repo {
    pub root: PathBuf,
    pub base: PathBuf,
}

impl Repo {
    /// A Git repository with an initial commit and `git story init` done,
    /// using the mock model.
    pub fn new(name: &str) -> Self {
        let base = tmp(name);
        let root = base.join("repo");
        fs::create_dir_all(&root).unwrap();
        let r = Self { root, base };
        r.git(&["init", "-q", "-b", "main"]);
        r.git(&["config", "user.email", "dev@example.com"]);
        r.git(&["config", "user.name", "Dev"]);
        r.git(&["config", "commit.gpgsign", "false"]);
        r.write("README.md", "demo\n");
        r.git(&["add", "."]);
        r.git(&["commit", "-qm", "init"]);
        r.ok(&["init", "--repository", "acme/demo"]);
        r.set_model(&format!(
            "[model]\nprovider = \"command\"\nname = \"mock\"\ncommand = \"{} {}\"\nlocal = true\n",
            python(),
            MOCK.replace('\\', "/")
        ));
        r.git(&["add", ".gitstory"]);
        r.git(&["commit", "-qm", "add gitstory"]);
        r
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join(".gitstory")
    }

    /// Replace the `[model]` section of `.gitstory`.
    pub fn set_model(&self, section: &str) {
        let text = fs::read_to_string(self.config_path()).unwrap();
        let start = text.find("[model]").unwrap();
        let end = text[start + 1..]
            .find("\n[")
            .map(|i| start + 1 + i + 1)
            .unwrap_or(text.len());
        let new = format!("{}{}\n{}", &text[..start], section, &text[end..]);
        fs::write(self.config_path(), new).unwrap();
    }

    pub fn set(&self, from: &str, to: &str) {
        let text = fs::read_to_string(self.config_path()).unwrap();
        assert!(text.contains(from), "{from} not in .gitstory");
        fs::write(self.config_path(), text.replace(from, to)).unwrap();
    }

    pub fn write(&self, path: &str, text: &str) {
        let p = self.root.join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    pub fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    pub fn commit_all(&self, msg: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", msg]);
        self.git(&["rev-parse", "HEAD"])
    }

    pub fn cmd(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_git-story"));
        c.current_dir(&self.root)
            .env("GITSTORY_CONFIG_DIR", self.base.join("config"))
            .env("GITSTORY_CLAUDE_PROJECTS", self.base.join("claude"))
            .env("MOCK_LOG", self.base.join("mock.log"))
            .env("GIT_CEILING_DIRECTORIES", &self.base)
            .env_remove("GITSTORY_WORK_UNIT")
            .env_remove("GITSTORY_GHF")
            .env_remove("ANTHROPIC_API_KEY");
        c
    }

    pub fn run(&self, args: &[&str]) -> (i32, Value) {
        let out = self.cmd().args(args).arg("--json").output().unwrap();
        let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!(
                "git story {args:?}: not JSON ({e}): {}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (out.status.code().unwrap(), v)
    }

    pub fn ok(&self, args: &[&str]) -> Value {
        let (code, v) = self.run(args);
        assert_eq!(code, 0, "git story {args:?} failed: {v}");
        assert_eq!(v["ok"], true);
        v["data"].clone()
    }

    pub fn err(&self, args: &[&str]) -> String {
        let (code, v) = self.run(args);
        assert_ne!(code, 0, "git story {args:?} should fail: {v}");
        v["error"]["code"].as_str().unwrap().to_owned()
    }
}
