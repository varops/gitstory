//! PRD §14 acceptance scenarios, run against the real binary with a
//! deterministic stand-in model (`tests/fixtures/mock_model.py`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const MOCK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mock_model.py");

fn python() -> &'static str {
    if cfg!(windows) { "python" } else { "python3" }
}

fn tmp(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let d = std::env::temp_dir().join(format!("gitstory-{name}-{nanos}"));
    fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

struct Repo {
    root: PathBuf,
    base: PathBuf,
}

impl Repo {
    /// A Git repository with an initial commit and `git story init` done,
    /// using the mock model.
    fn new(name: &str) -> Self {
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

    fn config_path(&self) -> PathBuf {
        self.root.join(".gitstory")
    }

    /// Replace the `[model]` section of `.gitstory`.
    fn set_model(&self, section: &str) {
        let text = fs::read_to_string(self.config_path()).unwrap();
        let start = text.find("[model]").unwrap();
        let end = text[start + 1..]
            .find("\n[")
            .map(|i| start + 1 + i + 1)
            .unwrap_or(text.len());
        let new = format!("{}{}\n{}", &text[..start], section, &text[end..]);
        fs::write(self.config_path(), new).unwrap();
    }

    fn set(&self, from: &str, to: &str) {
        let text = fs::read_to_string(self.config_path()).unwrap();
        assert!(text.contains(from), "{from} not in .gitstory");
        fs::write(self.config_path(), text.replace(from, to)).unwrap();
    }

    fn write(&self, path: &str, text: &str) {
        let p = self.root.join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn git(&self, args: &[&str]) -> String {
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

    fn commit_all(&self, msg: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", msg]);
        self.git(&["rev-parse", "HEAD"])
    }

    fn cmd(&self) -> Command {
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

    fn run(&self, args: &[&str]) -> (i32, Value) {
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

    fn ok(&self, args: &[&str]) -> Value {
        let (code, v) = self.run(args);
        assert_eq!(code, 0, "git story {args:?} failed: {v}");
        assert_eq!(v["ok"], true);
        v["data"].clone()
    }

    fn err(&self, args: &[&str]) -> String {
        let (code, v) = self.run(args);
        assert_ne!(code, 0, "git story {args:?} should fail: {v}");
        v["error"]["code"].as_str().unwrap().to_owned()
    }

    fn mock_log(&self) -> String {
        fs::read_to_string(self.base.join("mock.log")).unwrap_or_default()
    }

    /// Branch, run `f`, merge back to main with `--no-ff`.
    fn branch_and_merge(&self, branch: &str, f: impl FnOnce()) -> String {
        self.git(&["checkout", "-qb", branch]);
        f();
        self.git(&["checkout", "-q", "main"]);
        self.git(&[
            "merge",
            "-q",
            "--no-ff",
            branch,
            "-m",
            &format!("Merge {branch}"),
        ]);
        self.git(&["rev-parse", "HEAD"])
    }

    fn land(&self, branch: &str) -> Value {
        self.ok(&["land", "--work-unit", branch, "--to", "HEAD"])
    }

    fn session_dir(&self) -> PathBuf {
        let slug: String = self
            .root
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        self.base.join("claude").join(slug)
    }

    /// Every byte GitStory wrote outside the work tree: store, index,
    /// collection and egress.
    fn all_story_bytes(&self) -> String {
        let mut out = String::new();
        walk(&self.root.join(".git/story/store"), &mut out);
        out
    }
}

fn walk(p: &Path, out: &mut String) {
    let Ok(rd) = fs::read_dir(p) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push_str(&String::from_utf8_lossy(&fs::read(&p).unwrap()));
        }
    }
}

fn claims(story: &Value, section: &str) -> Vec<String> {
    story[section]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["claim"].as_str().unwrap().to_owned())
        .collect()
}

fn now_rfc3339() -> String {
    humantime_like(std::time::SystemTime::now())
}

/// RFC 3339 in UTC without a dependency.
fn humantime_like(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil from days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

const PLANTED_KEY: &str = "sk-ant-api03-PLANTEDsecretVALUE0123456789abcdefXYZ";
const RAW_ONLY: &str = "RAW-TRANSCRIPT-ONLY-MARKER";

/// A Claude Code session on `branch`: a goal with a planted key, a dropped
/// approach on redis.rs, a stated decision, an edit to lock.rs and a test run.
fn write_session(r: &Repo, branch: &str) -> PathBuf {
    let dir = r.session_dir();
    fs::create_dir_all(&dir).unwrap();
    let ts = now_rfc3339();
    let root = r.root.to_string_lossy().replace('\\', "/");
    let line = |typ: &str, content: Value| {
        json!({"type": typ, "timestamp": ts, "gitBranch": branch, "sessionId": "s1", "message": {"role": typ, "content": content}})
            .to_string()
    };
    let lines = [
        line(
            "user",
            json!(format!(
                "Fix the duplicate retry bug. Use key {PLANTED_KEY} for the staging API."
            )),
        ),
        line(
            "assistant",
            json!([{"type": "tool_use", "id": "t1", "name": "Write", "input": {"file_path": format!("{root}/redis.rs"), "content": "redis lock"}}]),
        ),
        line(
            "assistant",
            json!([{"type": "text", "text": "That didn't work: the Redis lock deadlocks under retry, so I will revert it and drop this approach."}]),
        ),
        line(
            "assistant",
            json!([{"type": "tool_use", "id": "t2", "name": "Bash", "input": {"command": "git checkout -- redis.rs"}}]),
        ),
        line(
            "assistant",
            json!([{"type": "text", "text": "I'll use optimistic row versioning instead of a lock because the database already has a version column."}]),
        ),
        line(
            "assistant",
            json!([{"type": "tool_use", "id": "t3", "name": "Edit", "input": {"file_path": format!("{root}/lock.rs"), "old_string": "a", "new_string": "b"}}]),
        ),
        line(
            "assistant",
            json!([{"type": "tool_use", "id": "t4", "name": "Bash", "input": {"command": "cargo test"}}]),
        ),
        line(
            "user",
            json!([{"type": "tool_result", "tool_use_id": "t4", "content": format!("{RAW_ONLY}\ntest result: ok. 12 passed; 0 failed; 0 ignored")}]),
        ),
    ];
    let path = dir.join("s1.jsonl");
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    path
}

// 1. A plain Git repo with no ghf. `git story init`.
#[test]
fn s01_init_in_a_plain_repo() {
    let r = Repo::new("s01");
    let text = fs::read_to_string(r.config_path()).unwrap();
    assert!(text.contains("[egress]") && text.contains("[store]"));
    assert!(!r.root.join(".git/ghf").exists());
    let st = r.ok(&["status"]);
    assert_eq!(st["repository"], "acme/demo");
    assert_eq!(st["stories"], 0);
    // A second init leaves the file alone.
    let again = r.ok(&["init"]);
    assert_eq!(again["created"], false);
    assert_eq!(r.err(&["init", "--repository", "x/y"]), "E_CONFIG");
}

// 2, 4, 5. Agent session: one dropped approach, one note, a planted key.
#[test]
fn s02_s04_s05_session_story_scrubbing_and_egress() {
    let r = Repo::new("s02");
    r.branch_and_merge("feat/retry", || {
        write_session(&r, "feat/retry");
        r.ok(&[
            "note",
            "Retries must stay idempotent because clients resend on timeout",
            "--type",
            "reason",
        ]);
        r.write("lock.rs", "row versioning\n");
        r.commit_all("Use row versioning for retries");
        let d = r.ok(&["draft"]);
        assert_eq!(d["story"]["stage"], "draft");
    });
    let landed = r.land("feat/retry");
    let story = &landed["story"];
    assert_eq!(story["stage"], "landed");
    let decisions = claims(story, "decisions").join("\n");
    assert!(
        decisions.contains("optimistic row versioning"),
        "{decisions}"
    );
    assert!(decisions.contains("idempotent"), "{decisions}");
    let rejected = claims(story, "rejected").join("\n");
    assert!(
        rejected.to_lowercase().contains("revert") || rejected.contains("didn't work"),
        "{rejected}"
    );
    // The redis.rs attempt never landed: never a decision.
    assert!(!decisions.contains("redis.rs"), "{decisions}");
    // Invented and uncited claims never survive; inferred ones are hypotheses.
    assert!(!decisions.contains("UNSUPPORTED") && !decisions.contains("Cites nothing real"));
    assert!(
        claims(story, "hypotheses")
            .iter()
            .any(|h| h.contains("MAYBE"))
    );
    // A test fact was captured from the session.
    assert!(
        claims(story, "evidence_summary")
            .iter()
            .any(|c| c.contains("12/12"))
    );

    // 4. The planted key is nowhere: store, collection, egress, model input.
    let stored = r.all_story_bytes();
    let mut collected = String::new();
    walk(&r.root.join(".git/story/collect"), &mut collected);
    for (what, text) in [
        ("store", &stored),
        ("collection", &collected),
        ("model input", &r.mock_log()),
    ] {
        assert!(!text.contains(PLANTED_KEY), "planted key found in {what}");
        assert!(
            !text.contains("PLANTEDsecret"),
            "planted key fragment found in {what}"
        );
    }
    assert!(
        stored.contains("[REDACTED:"),
        "redaction marker expected in the stored goal"
    );

    // 5. The manifest lists exactly the events sent; no raw transcript left.
    let log = r.mock_log();
    assert!(
        !log.contains(RAW_ONLY),
        "raw tool output was sent to the model"
    );
    assert!(!log.contains("tool_use_id") && !log.contains("\"sessionId\""));
    let manifest = &landed["egress"];
    let calls = manifest["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 2, "synthesis and validation");
    assert_eq!(calls[0]["purpose"], "reconciliation");
    assert_eq!(calls[1]["purpose"], "validation");
    let requests: Vec<Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let last_synth: Value =
        serde_json::from_str(requests[requests.len() - 2]["user"].as_str().unwrap()).unwrap();
    let mut sent: Vec<String> = last_synth["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_owned())
        .collect();
    let mut listed: Vec<String> = calls[0]["event_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e.as_str().unwrap().to_owned())
        .collect();
    sent.sort();
    listed.sort();
    assert_eq!(sent, listed);
    let hunks: Vec<&str> = calls[0]["hunks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["path"].as_str().unwrap())
        .collect();
    assert_eq!(hunks, ["lock.rs"]);
    // The manifest is stored with the story.
    assert_eq!(story["egress"], manifest["id"]);
    assert!(stored.contains(manifest["id"].as_str().unwrap()));
}

// 3. A draft appears after the commit; the commit is not delayed.
#[cfg(unix)]
#[test]
fn s03_background_draft_does_not_delay_commits() {
    let r = Repo::new("s03");
    let hook = r.ok(&["hooks", "install", "git"]);
    assert_eq!(hook["installed"], true);
    // The hook calls `git story`: put this binary on PATH as git-story.
    let bin = r.base.join("bin");
    fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_git-story"), bin.join("git-story")).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    r.git(&["checkout", "-qb", "feat/bg"]);
    r.write("a.rs", "a\n");
    r.git(&["add", "a.rs"]);
    let t = Instant::now();
    let out = r
        .cmd()
        .env("PATH", &path)
        .arg("note")
        .arg("Keep a.rs tiny because it is loaded at boot")
        .output()
        .unwrap();
    assert!(out.status.success());
    let st = Command::new("git")
        .args(["commit", "-qm", "Add a.rs"])
        .current_dir(&r.root)
        .env("PATH", &path)
        .env("GITSTORY_CONFIG_DIR", r.base.join("config"))
        .env("GITSTORY_CLAUDE_PROJECTS", r.base.join("claude"))
        .env("MOCK_LOG", r.base.join("mock.log"))
        .status()
        .unwrap();
    assert!(st.success());
    let commit_time = t.elapsed();
    assert!(
        commit_time < Duration::from_secs(3),
        "commit took {commit_time:?}"
    );
    let drafts = r.root.join(".git/story/collect/feat_bg/drafts");
    let deadline = Instant::now() + Duration::from_secs(30);
    while fs::read_dir(&drafts).map(|d| d.count()).unwrap_or(0) == 0 {
        assert!(Instant::now() < deadline, "no background draft appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let shown = r.ok(&["show", "HEAD"]);
    assert_eq!(shown["stories"][0]["stage"], "draft");
    assert_eq!(r.ok(&["hooks", "uninstall", "git"])["installed"], false);
}

// 6. inference = local: nothing leaves the machine.
#[test]
fn s06_local_inference_refuses_external_endpoints() {
    let r = Repo::new("s06");
    r.set_model("[model]\nprovider = \"anthropic\"\nname = \"claude-opus-5-5\"\n");
    r.ok(&["trust", "https://api.anthropic.com"]);
    r.set("inference = \"external\"", "inference = \"local\"");
    r.git(&["checkout", "-qb", "feat/x"]);
    r.ok(&["note", "Use a bounded queue because memory is capped"]);
    assert_eq!(r.err(&["draft"]), "E_EGRESS_DENIED");
    // The refused call is not in any manifest: it never started.
    let mut collected = String::new();
    walk(&r.root.join(".git/story/collect"), &mut collected);
    assert!(!collected.contains("api.anthropic.com"));
    // A local endpoint is fine.
    r.set_model(&format!(
        "[model]\nprovider = \"command\"\nname = \"mock\"\ncommand = \"{} {}\"\nlocal = true\n",
        python(),
        MOCK.replace('\\', "/")
    ));
    r.ok(&["draft"]);
}

#[test]
fn untrusted_external_endpoint_is_refused() {
    let r = Repo::new("trust");
    r.set_model("[model]\nprovider = \"anthropic\"\nname = \"claude-opus-5-5\"\nurl = \"https://models.example.com\"\n");
    r.git(&["checkout", "-qb", "feat/x"]);
    r.ok(&["note", "Use a bounded queue because memory is capped"]);
    assert_eq!(r.err(&["draft"]), "E_UNTRUSTED_ENDPOINT");
}

// 7. Two drafts; one approach removed in review; squash-merge, land.
#[test]
fn s07_reconciliation_after_squash_merge() {
    let r = Repo::new("s07");
    r.git(&["checkout", "-qb", "feat/cache"]);
    r.write("redis_cache.rs", "redis\n");
    r.commit_all("Add a Redis cache");
    r.ok(&["draft"]);
    r.git(&["rm", "-q", "redis_cache.rs"]);
    r.write("memo.rs", "memo\n");
    r.ok(&[
        "note",
        "Use in-process memoization because review asked for no new services",
    ]);
    r.commit_all("Replace the Redis cache with memoization");
    r.ok(&["draft"]);
    r.git(&["checkout", "-q", "main"]);
    r.git(&["merge", "-q", "--squash", "feat/cache"]);
    r.git(&["commit", "-qm", "Cache results (#7)"]);
    let landed = r.land("feat/cache");
    let s = &landed["story"];
    let decisions = claims(s, "decisions").join("\n");
    assert!(
        !decisions.contains("redis_cache.rs"),
        "removed approach in decisions: {decisions}"
    );
    let dropped = [claims(s, "rejected"), claims(s, "discarded")]
        .concat()
        .join("\n");
    assert!(
        dropped.contains("redis_cache.rs"),
        "removed approach missing: {dropped}"
    );
    assert!(decisions.contains("memoization"));
}

// 8. The session log is gone; validation still passes.
#[test]
fn s08_validation_survives_lost_sources() {
    let r = Repo::new("s08");
    r.branch_and_merge("feat/retry", || {
        write_session(&r, "feat/retry");
        r.write("lock.rs", "row versioning\n");
        r.commit_all("Use row versioning");
    });
    let landed = r.land("feat/retry");
    let id = landed["story"]["story_id"].as_str().unwrap().to_owned();
    fs::remove_dir_all(r.session_dir()).unwrap();
    fs::remove_dir_all(r.root.join(".git/story/collect")).unwrap();
    let v = r.ok(&["validate", &id]);
    assert_eq!(v["valid"], true, "{v}");
    assert_eq!(v["missing_events"], json!([]));
}

/// Land one branch that rewrites line 1 of lock.txt (and appends `extra`).
fn change(r: &Repo, branch: &str, first: &str, extra: Option<&str>, note: &[&str]) -> String {
    r.branch_and_merge(branch, || {
        let mut text =
            fs::read_to_string(r.root.join("lock.txt")).unwrap_or_else(|_| "x\nkeep\n".into());
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        lines[0] = first.to_owned();
        if let Some(e) = extra {
            lines.push(e.to_owned());
        }
        text = lines.join("\n") + "\n";
        r.write("lock.txt", &text);
        let mut args = vec!["note"];
        args.extend_from_slice(note);
        r.ok(&args);
        r.commit_all(&format!("{branch} change"));
    });
    r.land(branch)["story"]["story_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

// 9, 10, 11. Supersession through blame and history.
#[test]
fn s09_s10_s11_supersession_blame_and_history() {
    let r = Repo::new("s09");
    r.write("lock.txt", "x\nkeep\n");
    r.commit_all("lock file");
    let a = change(
        &r,
        "feat/a",
        "lock = redis",
        None,
        &["Add a Redis lock because retries double-charge"],
    );
    let b = change(
        &r,
        "feat/b",
        "lock = none",
        Some("versioned = true"),
        &["Remove the lock because row versioning replaces it"],
    );
    // 10. Reconciliation proposed B supersedes A; nothing applied yet.
    let props = r.ok(&["proposals"]);
    let p = props["proposals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["from"] == b.as_str() && p["to"] == a.as_str())
        .expect("B supersedes A proposed")
        .clone();
    assert_eq!(r.ok(&["show", &a])["stories"][0]["status"], "current");
    r.ok(&["link", "--accept", p["id"].as_str().unwrap()]);
    assert_eq!(r.ok(&["show", &a])["stories"][0]["status"], "superseded");
    let c = change(
        &r,
        "feat/c",
        "lock = redis",
        None,
        &[
            "Bring the Redis lock back because versioning missed cross-row writes",
            "--supersedes",
            &b,
        ],
    );

    // Blame on C's line: C, current.
    let bl = r.ok(&["blame", "lock.txt:1"]);
    let ids: Vec<&str> = bl["stories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["story"]["story_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [c.as_str()]);
    assert_eq!(bl["stories"][0]["current"], true);
    // Blame on B's line: B, and C marked current.
    let bl = r.ok(&["blame", "lock.txt:3"]);
    let entries = bl["stories"].as_array().unwrap();
    let find = |id: &str| {
        entries
            .iter()
            .find(|e| e["story"]["story_id"] == id)
            .cloned()
    };
    assert_eq!(find(&b).expect("B")["current"], false);
    assert_eq!(find(&c).expect("C")["current"], true);
    // History: A, B, C.
    let h = r.ok(&["history", &c]);
    let chain: Vec<&str> = h["chain"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["story_id"].as_str().unwrap())
        .collect();
    assert_eq!(chain, [a.as_str(), b.as_str(), c.as_str()]);
    // Cycles are refused.
    assert_eq!(r.err(&["link", &a, "--supersedes", &c]), "E_LINK_CYCLE");

    // 11. --brief: no hypotheses, no discarded claims, current stories only.
    let brief = r.ok(&["blame", "lock.txt", "--brief"]);
    for e in brief["stories"].as_array().unwrap() {
        assert!(e["story"].get("hypotheses").is_none());
        assert!(e["story"].get("discarded").is_none());
        assert_eq!(e["current"], true);
    }
    // Search and path find them, current first.
    let p = r.ok(&["path", "lock.txt"]);
    assert_eq!(p["stories"][0]["status"], "current");
    assert!(
        !r.ok(&["search", "versioning"])["stories"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

// 12. A second machine sharing the store: blame is fast.
#[test]
fn s12_second_machine_with_shared_store() {
    let r = Repo::new("s12");
    let shared = r.base.join("shared-store");
    r.set(
        "kind = \"local\"",
        &format!(
            "kind = \"local\"\npath = \"{}\"",
            shared.to_string_lossy().replace('\\', "/")
        ),
    );
    r.commit_all("shared store");
    r.write("lock.txt", "x\n");
    r.commit_all("lock file");
    change(
        &r,
        "feat/a",
        "lock = redis",
        None,
        &["Add a Redis lock because retries double-charge"],
    );
    let clone = r.base.join("clone");
    let out = Command::new("git")
        .args(["clone", "-q"])
        .arg(&r.root)
        .arg(&clone)
        .output()
        .unwrap();
    assert!(out.status.success());
    let t = Instant::now();
    let out = r
        .cmd()
        .current_dir(&clone)
        .args(["blame", "lock.txt:1", "--brief", "--json"])
        .output()
        .unwrap();
    let took = t.elapsed();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["data"]["stories"].as_array().unwrap().len(), 1);
    let limit = if cfg!(debug_assertions) { 2000 } else { 500 };
    assert!(took < Duration::from_millis(limit), "blame took {took:?}");
}

// 13. Export and import into an empty store: identical.
#[test]
fn s13_export_import_round_trip() {
    let r = Repo::new("s13");
    r.write("lock.txt", "x\n");
    r.commit_all("lock file");
    let a = change(
        &r,
        "feat/a",
        "lock = redis",
        None,
        &["Add a Redis lock because retries double-charge"],
    );
    change(
        &r,
        "feat/b",
        "lock = none",
        None,
        &[
            "Remove the lock because row versioning replaces it",
            "--supersedes",
            &a,
        ],
    );
    r.ok(&["edit", &a, "--review", "reviewed"]);
    let out = r.base.join("export");
    r.ok(&[
        "export",
        "--format",
        "markdown",
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(out.join("index.md").exists());

    let other = Repo::new("s13b");
    let imported = other.ok(&["import", out.to_str().unwrap()]);
    assert!(imported["written"].as_u64().unwrap() > 0);
    // Importing again changes nothing.
    assert_eq!(other.ok(&["import", out.to_str().unwrap()])["written"], 0);
    let out2 = other.base.join("export");
    other.ok(&["export", "--out", out2.to_str().unwrap()]);
    let objs = |p: &Path| -> Value {
        let v: Value =
            serde_json::from_slice(&fs::read(p.join("gitstory-export.json")).unwrap()).unwrap();
        v["objects"].clone()
    };
    assert_eq!(objs(&out), objs(&out2));
    let history = other.ok(&["history", &a]);
    assert_eq!(history["chain"].as_array().unwrap().len(), 2);
}

// 14. With ghf: trace attempts are cited, a hold is placed and released,
// and the story still validates after the trace is gone.
#[cfg(unix)]
#[test]
fn s14_ghf_traces_and_holds() {
    use std::os::unix::fs::PermissionsExt;
    let r = Repo::new("s14");
    fs::create_dir_all(r.root.join(".git/ghf")).unwrap();
    let fake = r.base.join("fake-ghf");
    let log = r.base.join("ghf.log");
    let base = r.git(&["rev-parse", "HEAD"]);
    let script = format!(
        r#"#!/usr/bin/env python3
import json, sys
args = [a for a in sys.argv[1:] if a != "--json"]
open({log:?}, "a").write(" ".join(args) + "\n")
def ok(data): print(json.dumps({{"schema": "ghf.cli/v1", "ok": True, "data": data}}))
if args[:2] == ["trace", "list"]:
    ok({{"traces": [{{"id": "tr_1", "state": "open", "local": True}}]}})
elif args[:2] == ["trace", "log"]:
    ok({{"trace": {{"id": "tr_1"}}, "checkpoints": [
        {{"id": "ck_1", "seq": 1, "base_rev": "{base}", "created_at": 1, "author": "claude-code:s1",
          "message": "tried a Redis lock", "annotations": {{"tests": "fail"}}, "changes": [{{"path": "redis.rs", "status": "added"}}]}},
        {{"id": "ck_2", "seq": 2, "base_rev": "{base}", "created_at": 2, "author": "claude-code:s1",
          "annotations": {{"tests": "pass"}}, "changes": [{{"path": "lock.rs", "status": "added"}}]}}]}})
elif args[:2] in (["trace", "hold"], ["trace", "release"]):
    ok({{"trace": {{"id": "tr_1"}}}})
else:
    print(json.dumps({{"ok": False}})); sys.exit(1)
"#
    );
    fs::write(&fake, script).unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    r.git(&["checkout", "-qb", "feat/g"]);
    r.write("lock.rs", "versioned\n");
    r.ok(&[
        "note",
        "Use row versioning because the Redis lock failed tests",
    ]);
    r.commit_all("Row versioning");
    r.git(&["checkout", "-q", "main"]);
    r.git(&["merge", "-q", "--no-ff", "feat/g", "-m", "Merge feat/g"]);
    let out = r
        .cmd()
        .env("GITSTORY_GHF", &fake)
        .args(["land", "--work-unit", "feat/g", "--json"])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    let story = &v["data"]["story"];
    let all = serde_json::to_string(story).unwrap();
    let stored = r.all_story_bytes();
    assert!(stored.contains("ghf_trace"), "no ghf evidence stored");
    assert!(stored.contains("ghf:tr_1/ck_1"));
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.contains("trace hold tr_1 --name gitstory"), "{calls}");
    assert!(
        calls.contains("trace release tr_1 --name gitstory"),
        "{calls}"
    );
    // The trace is gone (pruned): the story still validates.
    let id = story["story_id"].as_str().unwrap();
    assert!(!all.is_empty());
    let v = r.ok(&["validate", id]);
    assert_eq!(v["valid"], true);
}

// 15. Git history, refs and commit messages are untouched.
#[test]
fn s15_git_is_untouched() {
    let r = Repo::new("s15");
    let snapshot = |r: &Repo| {
        (
            r.git(&["for-each-ref", "--format=%(refname) %(objectname)"]),
            r.git(&["log", "--all", "--format=%H %T %s%n%b"]),
            r.git(&["notes", "list"]),
            r.git(&["status", "--porcelain"]),
        )
    };
    r.write("lock.txt", "x\n");
    r.commit_all("lock file");
    r.git(&["checkout", "-qb", "feat/a"]);
    r.write("lock.txt", "y\n");
    r.commit_all("change");
    let before = snapshot(&r);
    r.ok(&["note", "Use y because x is deprecated"]);
    r.ok(&["draft"]);
    let after = snapshot(&r);
    assert_eq!(before, after);
    r.git(&["checkout", "-q", "main"]);
    r.git(&["merge", "-q", "--no-ff", "feat/a", "-m", "Merge"]);
    let before = snapshot(&r);
    r.land("feat/a");
    r.ok(&["blame", "lock.txt"]);
    r.ok(&["gc"]);
    assert_eq!(before, snapshot(&r));
}

#[test]
fn land_requires_a_commit_on_main() {
    let r = Repo::new("main");
    r.git(&["checkout", "-qb", "feat/x"]);
    r.write("a.rs", "a\n");
    r.commit_all("a");
    r.ok(&["note", "Keep it small because it ships in the boot image"]);
    assert_eq!(r.err(&["land", "--to", "HEAD"]), "E_COMMIT_NOT_REACHABLE");
    assert_eq!(
        r.err(&["land", "--to", "no-such-rev"]),
        "E_COMMIT_NOT_FOUND"
    );
}

#[test]
fn model_failures_keep_the_manifest() {
    let r = Repo::new("fail");
    r.git(&["checkout", "-qb", "feat/x"]);
    r.ok(&["note", "Keep it small because it ships in the boot image"]);
    let out = r
        .cmd()
        .env("MOCK_FAIL", "1")
        .args(["draft", "--json"])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "E_REMOTE");
    let mut collected = String::new();
    walk(&r.root.join(".git/story/collect"), &mut collected);
    assert!(
        collected.contains("\"purpose\": \"synthesis\""),
        "manifest of the failed call is kept"
    );
}

#[test]
fn skill_and_claude_hook_install() {
    let r = Repo::new("hooks");
    let s = r.ok(&["skill", "install"]);
    assert!(Path::new(s["path"].as_str().unwrap()).exists());
    let h = r.ok(&["hooks", "install", "claude-code"]);
    assert_eq!(h["installed"], true);
    let settings = fs::read_to_string(r.root.join(".claude/settings.json")).unwrap();
    assert!(settings.contains("git story draft --background"));
    assert_eq!(r.ok(&["hooks", "install", "claude-code"])["changed"], false);
    assert_eq!(
        r.ok(&["hooks", "uninstall", "claude-code"])["installed"],
        false
    );
}
