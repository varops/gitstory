//! Evidence adapters (PRD §9.2): notes, Git, and Claude Code session logs.
//! Everything here runs locally. Raw sources are read, never copied: each
//! event keeps a typed fact, a short scrubbed excerpt, and a hash of the
//! source span so the excerpt can be checked while the source exists.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use gitstory_proto::evidence::{Durable, EventType, EvidenceEvent, External, SourceType, TestFact};
use gitstory_proto::id;
use serde_json::Value;

use crate::collect::WorkUnit;
use crate::error::Result;
use crate::scrub;
use crate::workspace::Workspace;

/// Settings every adapter needs.
pub struct Ctx<'a> {
    pub ws: &'a Workspace,
    pub work_unit: &'a str,
    pub excerpt_cap: usize,
    /// `false` keeps typed facts only (`collection.excerpts = none`).
    pub excerpts: bool,
    pub ignore: ignore::gitignore::Gitignore,
}

impl<'a> Ctx<'a> {
    pub fn new(ws: &'a Workspace, work_unit: &'a str) -> Self {
        let mut b = ignore::gitignore::GitignoreBuilder::new(&ws.repo.root);
        let p = ws.repo.root.join(crate::config::IGNORE_FILE);
        if p.exists() {
            let _ = b.add(p);
        }
        Self {
            ws,
            work_unit,
            excerpt_cap: ws.cfg.collection.excerpt_cap,
            excerpts: ws.cfg.collection.excerpts == "on",
            ignore: b
                .build()
                .unwrap_or_else(|_| ignore::gitignore::Gitignore::empty()),
        }
    }

    /// Is `path` under `.storyignore`? Such events keep the path, never content.
    pub fn ignored(&self, path: &str) -> bool {
        let rel = Path::new(path)
            .strip_prefix(&self.ws.repo.root)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| path.to_owned());
        self.ignore
            .matched_path_or_any_parents(&rel, false)
            .is_ignore()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn event(
        &self,
        kind: EventType,
        source: SourceType,
        author: &str,
        at: i64,
        fact: &str,
        excerpt: Option<&str>,
        span: &[u8],
        pointer: Option<String>,
        paths: Vec<String>,
    ) -> EvidenceEvent {
        let paths: Vec<String> = paths
            .into_iter()
            .map(|p| {
                Path::new(&p)
                    .strip_prefix(&self.ws.repo.root)
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
                    .unwrap_or(p)
            })
            .collect();
        let sensitive = paths.iter().any(|p| self.ignored(p));
        let (fact, _) = scrub::scrub(fact);
        let excerpt = excerpt
            .filter(|_| self.excerpts && !sensitive)
            .map(|e| scrub::excerpt(e, self.excerpt_cap))
            .filter(|e| !e.is_empty());
        EvidenceEvent {
            id: id::new_id(id::EVENT),
            kind,
            work_unit: self.work_unit.to_owned(),
            durable: Durable {
                fact: if sensitive {
                    format!(
                        "{} (paths under .storyignore; content withheld)",
                        kind_word(kind)
                    )
                } else {
                    fact
                },
                test: None,
                excerpt,
                source_hash: blake3::hash(span).to_hex().to_string(),
                source,
                at,
                author: author.to_owned(),
            },
            external: pointer.map(|pointer| External { pointer }),
            paths,
        }
    }
}

fn kind_word(k: EventType) -> &'static str {
    match k {
        EventType::Goal => "goal",
        EventType::Decision => "decision",
        EventType::Reason => "reason",
        EventType::Attempt => "attempt",
        EventType::Abandon => "abandon",
        EventType::Test => "test",
        EventType::Constraint => "constraint",
        EventType::Review => "review",
    }
}

// ---------------------------------------------------------------------------
// Notes

pub fn note(ctx: &Ctx, kind: EventType, text: &str) -> EvidenceEvent {
    let author = ctx.ws.author();
    ctx.event(
        kind,
        SourceType::HumanNote,
        &author,
        id::now_ms(),
        &format!("{}: {text}", kind_word(kind)),
        Some(text),
        format!("note\0{author}\0{text}\0{}", id::now_ms()).as_bytes(),
        None,
        Vec::new(),
    )
}

pub fn test_note(ctx: &Ctx, command: &str, passed: bool) -> EvidenceEvent {
    let author = ctx.ws.author();
    let mut e = ctx.event(
        EventType::Test,
        SourceType::TestRun,
        &author,
        id::now_ms(),
        &format!(
            "test: {command} -> {}",
            if passed { "pass" } else { "fail" }
        ),
        None,
        format!("test\0{command}\0{passed}\0{}", id::now_ms()).as_bytes(),
        None,
        Vec::new(),
    );
    e.durable.test = Some(TestFact {
        command: scrub::scrub(command).0,
        passed: u32::from(passed),
        total: 1,
    });
    e
}

// ---------------------------------------------------------------------------
// Git

/// Commits made in the work unit (and reverts), plus uncommitted changes.
pub fn git(ctx: &Ctx, wu: &WorkUnit) -> Result<Vec<EvidenceEvent>> {
    let repo = &ctx.ws.repo;
    // Follow the work unit's branch while it exists, so commits that reached
    // the current branch from elsewhere are not attributed to this work.
    let tip = wu
        .meta
        .branch
        .as_deref()
        .and_then(|b| repo.resolve_commit(&format!("refs/heads/{b}")))
        .or_else(|| repo.head());
    let Some(head) = tip else {
        return Ok(Vec::new());
    };
    let commits = match &wu.meta.base_rev {
        Some(base) if base != &head => repo.commits_between(base, &head)?,
        Some(_) => Vec::new(),
        None => repo.recent_commits(20)?.into_iter().rev().collect(),
    };
    let mut out = Vec::new();
    for sha in commits {
        let c = repo.commit(&sha)?;
        if c.parents.len() > 1 {
            continue;
        }
        let paths = repo.changed_paths(&sha)?;
        let short = &c.sha[..7.min(c.sha.len())];
        let pointer = Some(format!("git:{}", c.sha));
        let at = c.time * 1000;
        if let Some(reverted) = c.subject.strip_prefix("Revert \"") {
            out.push(ctx.event(
                EventType::Abandon,
                SourceType::Git,
                &c.author,
                at,
                &format!("commit {short} reverted \"{}", reverted),
                Some(&c.body),
                c.sha.as_bytes(),
                pointer,
                paths,
            ));
            continue;
        }
        out.push(ctx.event(
            EventType::Attempt,
            SourceType::Git,
            &c.author,
            at,
            &format!("commit {short}: {} ({} file(s))", c.subject, paths.len()),
            None,
            format!("commit\0{}", c.sha).as_bytes(),
            pointer.clone(),
            paths.clone(),
        ));
        if !c.body.is_empty() {
            out.push(ctx.event(
                EventType::Reason,
                SourceType::Git,
                &c.author,
                at,
                &format!("commit {short} message explains the change"),
                Some(&c.body),
                format!("body\0{}", c.sha).as_bytes(),
                pointer,
                paths,
            ));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Claude Code

/// Where Claude Code keeps session logs for a working directory.
pub fn claude_project_dir(root: &Path) -> Option<PathBuf> {
    let base = match std::env::var_os("GITSTORY_CLAUDE_PROJECTS") {
        Some(p) => PathBuf::from(p),
        None => crate::config::home_dir()?.join(".claude").join("projects"),
    };
    let slug: String = root
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    Some(base.join(slug))
}

/// Test runners whose output we can read pass/fail counts from.
const TEST_COMMANDS: &[&str] = &[
    "cargo test",
    "cargo nextest",
    "pytest",
    "python -m pytest",
    "npm test",
    "npm run test",
    "pnpm test",
    "yarn test",
    "bun test",
    "go test",
    "jest",
    "vitest",
    "mocha",
    "rspec",
    "mvn test",
    "gradle test",
    "./gradlew test",
    "make test",
    "dotnet test",
    "phpunit",
    "mix test",
];

fn is_test_command(cmd: &str) -> bool {
    let c = cmd.trim();
    TEST_COMMANDS.iter().any(|t| {
        c.starts_with(t) || c.contains(&format!("&& {t}")) || c.contains(&format!("; {t}"))
    })
}

/// Pass/fail counts from common runner summaries.
pub fn parse_test_counts(output: &str) -> Option<(u32, u32)> {
    use regex::Regex;
    use std::sync::LazyLock;
    static CARGO: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"test result: \w+\. (\d+) passed; (\d+) failed").unwrap());
    static PYTEST: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?:(\d+) failed, )?(\d+) passed").unwrap());
    static JEST: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"Tests:\s+(?:(\d+) failed, )?(?:\d+ skipped, )?(\d+) passed, (\d+) total")
            .unwrap()
    });
    static GO_FAIL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)^(FAIL|--- FAIL)").unwrap());
    let (mut passed, mut failed, mut found) = (0u32, 0u32, false);
    for c in CARGO.captures_iter(output) {
        passed += c[1].parse::<u32>().unwrap_or(0);
        failed += c[2].parse::<u32>().unwrap_or(0);
        found = true;
    }
    if found {
        return Some((passed, passed + failed));
    }
    if let Some(c) = JEST.captures(output) {
        let total: u32 = c[3].parse().unwrap_or(0);
        let p: u32 = c[2].parse().unwrap_or(0);
        return Some((p, total));
    }
    if let Some(c) = PYTEST.captures(output) {
        let f: u32 = c.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        let p: u32 = c[2].parse().unwrap_or(0);
        return Some((p, p + f));
    }
    if output.contains("\nok ") || output.starts_with("ok ") || GO_FAIL.is_match(output) {
        let fails = GO_FAIL.find_iter(output).count() as u32;
        return Some((u32::from(fails == 0), 1));
    }
    None
}

/// Phrases that usually carry a decision, a reason or an abandoned attempt.
fn classify_text(t: &str) -> Option<EventType> {
    let l = t.to_lowercase();
    const ABANDON: &[&str] = &[
        "revert",
        "roll back",
        "rolling back",
        "doesn't work",
        "does not work",
        "didn't work",
        "abandon",
        "back out",
        "undo this",
        "drop this approach",
        "go back to",
    ];
    const DECISION: &[&str] = &[
        "i'll use",
        "i will use",
        "decided",
        "instead of",
        "rather than",
        "switching to",
        "we'll go with",
        "going with",
        "the fix is",
        "chose",
        "choose",
    ];
    const REASON: &[&str] = &[
        "because",
        "root cause",
        "the issue is",
        "the problem is",
        "caused by",
        "so that",
        "the reason",
        "this means",
    ];
    const CONSTRAINT: &[&str] = &[
        "must not",
        "must be",
        "cannot",
        "can't",
        "has to",
        "requirement",
    ];
    let has = |list: &[&str]| list.iter().any(|w| l.contains(w));
    if has(ABANDON) {
        Some(EventType::Abandon)
    } else if has(DECISION) {
        Some(EventType::Decision)
    } else if has(REASON) {
        Some(EventType::Reason)
    } else if has(CONSTRAINT) {
        Some(EventType::Constraint)
    } else {
        None
    }
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn parse_time(ts: Option<&str>) -> i64 {
    ts.and_then(|t| humantime::parse_rfc3339_weak(t.trim_end_matches('Z')).ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or_else(id::now_ms, |d| d.as_millis() as i64)
}

/// Extract events from Claude Code session logs for this repository. Lines
/// already read (per `wu.meta.offsets`) are skipped; offsets are advanced.
pub fn claude_code(ctx: &Ctx, wu: &mut WorkUnit) -> Result<Vec<EvidenceEvent>> {
    let mut out = Vec::new();
    let Some(dir) = claude_project_dir(&ctx.ws.repo.root) else {
        return Ok(out);
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(out);
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    for file in files {
        let key = file.to_string_lossy().into_owned();
        let start = wu.meta.offsets.get(&key).copied().unwrap_or(0);
        let f = fs::File::open(&file)?;
        let mut pending_tests: std::collections::HashMap<String, (String, i64, u64)> =
            Default::default();
        let mut goal_seen = start > 0;
        let mut n = 0u64;
        for line in BufReader::new(f).lines() {
            let line = line?;
            n += 1;
            if n <= start {
                continue;
            }
            let Ok(d) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let typ = d.get("type").and_then(Value::as_str).unwrap_or_default();
            if typ != "user" && typ != "assistant" {
                continue;
            }
            // Stay inside the work unit: its branch, and after it started.
            let at = parse_time(d.get("timestamp").and_then(Value::as_str));
            if at < wu.meta.created_at - 6 * 3_600_000 {
                continue;
            }
            if let (Some(b), Some(lb)) =
                (&wu.meta.branch, d.get("gitBranch").and_then(Value::as_str))
                && b != lb
            {
                continue;
            }
            let session = d
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let author = format!("claude-code:{session}");
            let pointer = Some(format!("claude-code:{session}#L{n}"));
            let Some(msg) = d.get("message") else {
                continue;
            };
            let content = msg.get("content").cloned().unwrap_or(Value::Null);
            if typ == "user" {
                // The first human prompt states the goal.
                if !goal_seen && let Value::String(s) = &content {
                    goal_seen = true;
                    out.push(ctx.event(
                        EventType::Goal,
                        SourceType::AgentSession,
                        "user",
                        at,
                        "task as the user asked for it",
                        Some(s),
                        line.as_bytes(),
                        pointer.clone(),
                        Vec::new(),
                    ));
                }
                // Tool results complete pending test runs.
                if let Value::Array(items) = &content {
                    for it in items {
                        if it.get("type").and_then(Value::as_str) != Some("tool_result") {
                            continue;
                        }
                        let tid = it
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let Some((cmd, t0, line_no)) = pending_tests.remove(tid) else {
                            continue;
                        };
                        let output = text_of(it.get("content").unwrap_or(&Value::Null));
                        let errored = it.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                        let (passed, total) = parse_test_counts(&output).unwrap_or(if errored {
                            (0, 1)
                        } else {
                            (1, 1)
                        });
                        let verdict = if passed == total && !errored {
                            "pass"
                        } else {
                            "fail"
                        };
                        let mut e = ctx.event(
                            EventType::Test,
                            SourceType::TestRun,
                            &author,
                            t0,
                            &format!("test: {cmd} -> {verdict}, {passed}/{total} passed"),
                            None,
                            line.as_bytes(),
                            Some(format!("claude-code:{session}#L{line_no}")),
                            Vec::new(),
                        );
                        e.durable.test = Some(TestFact {
                            command: scrub::scrub(&cmd).0,
                            passed,
                            total,
                        });
                        out.push(e);
                    }
                }
                continue;
            }
            // Assistant: tool calls and stated reasoning.
            let Value::Array(items) = content else {
                continue;
            };
            for it in &items {
                match it.get("type").and_then(Value::as_str) {
                    Some("tool_use") => {
                        let name = it.get("name").and_then(Value::as_str).unwrap_or_default();
                        let input = it.get("input").cloned().unwrap_or(Value::Null);
                        match name {
                            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
                                let path = input
                                    .get("file_path")
                                    .or_else(|| input.get("notebook_path"))
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned();
                                let verb = if name == "Write" { "wrote" } else { "edited" };
                                out.push(ctx.event(
                                    EventType::Attempt,
                                    SourceType::AgentSession,
                                    &author,
                                    at,
                                    &format!("{verb} {}", short_path(ctx, &path)),
                                    None,
                                    line.as_bytes(),
                                    pointer.clone(),
                                    vec![path],
                                ));
                            }
                            "Bash" => {
                                let cmd = input
                                    .get("command")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                if is_test_command(cmd)
                                    && let Some(tid) = it.get("id").and_then(Value::as_str)
                                {
                                    pending_tests.insert(tid.to_owned(), (cmd.to_owned(), at, n));
                                } else if cmd.contains("git revert")
                                    || cmd.contains("git checkout --")
                                    || cmd.contains("git restore")
                                {
                                    out.push(ctx.event(
                                        EventType::Abandon,
                                        SourceType::AgentSession,
                                        &author,
                                        at,
                                        &format!("discarded changes: {}", first_line(cmd)),
                                        None,
                                        line.as_bytes(),
                                        pointer.clone(),
                                        Vec::new(),
                                    ));
                                }
                            }
                            _ => {}
                        }
                    }
                    Some("text") => {
                        let t = it.get("text").and_then(Value::as_str).unwrap_or_default();
                        for para in t.split("\n\n") {
                            let p = para.trim();
                            if p.len() < 40 {
                                continue;
                            }
                            if let Some(kind) = classify_text(p) {
                                out.push(ctx.event(
                                    kind,
                                    SourceType::AgentSession,
                                    &author,
                                    at,
                                    &format!("agent stated a {}", kind_word(kind)),
                                    Some(p),
                                    p.as_bytes(),
                                    pointer.clone(),
                                    Vec::new(),
                                ));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        wu.meta.offsets.insert(key, n);
    }
    Ok(out)
}

fn short_path(ctx: &Ctx, p: &str) -> String {
    Path::new(p)
        .strip_prefix(&ctx.ws.repo.root)
        .map(|r| r.to_string_lossy().into_owned())
        .unwrap_or_else(|_| p.to_owned())
}

fn first_line(s: &str) -> String {
    s.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(120)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_test_counts() {
        assert_eq!(
            parse_test_counts("test result: ok. 14 passed; 0 failed; 0 ignored"),
            Some((14, 14))
        );
        assert_eq!(
            parse_test_counts("==== 2 failed, 48 passed in 3.2s ===="),
            Some((48, 50))
        );
        assert_eq!(
            parse_test_counts("Tests:       1 failed, 9 passed, 10 total"),
            Some((9, 10))
        );
        assert_eq!(parse_test_counts("nothing here"), None);
    }

    #[test]
    fn classifies_statements() {
        assert_eq!(
            classify_text("I'll use optimistic row versioning instead of the Redis lock"),
            Some(EventType::Decision)
        );
        assert_eq!(
            classify_text("That didn't work, let me revert it"),
            Some(EventType::Abandon)
        );
        assert_eq!(
            classify_text("The root cause is a deadlock under retry"),
            Some(EventType::Reason)
        );
        assert_eq!(classify_text("Here is the output"), None);
    }
}
