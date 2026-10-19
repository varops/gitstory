//! Read-only access to Git through the `git` executable. GitStory never
//! writes to Git (PRD §8.1).

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::Result;
use crate::{err, fail};

#[derive(Debug, Clone)]
pub struct Repo {
    pub root: PathBuf,
    pub common_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub sha: String,
    pub parents: Vec<String>,
    pub author: String,
    /// Unix seconds.
    pub time: i64,
    pub subject: String,
    pub body: String,
}

/// One file's part of a diff.
#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: String,
    /// The path before a rename, when there was one.
    pub old_path: Option<String>,
    pub text: String,
    /// Changed line ranges in the parent, as (start, count).
    pub old_ranges: Vec<(u32, u32)>,
}

fn git_in(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.current_dir(dir)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat");
    c
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).trim_end().to_owned()
}

impl Repo {
    pub fn discover(cwd: &Path) -> Result<Self> {
        let out = git_in(cwd)
            .args([
                "rev-parse",
                "--path-format=absolute",
                "--show-toplevel",
                "--git-dir",
                "--git-common-dir",
            ])
            .output()
            .map_err(|e| err!(Internal, "cannot run git: {e}"))?;
        if !out.status.success() {
            fail!(NotARepo, "not inside a Git worktree: {}", cwd.display());
        }
        let s = String::from_utf8_lossy(&out.stdout).into_owned();
        let mut l = s.lines();
        let (Some(root), Some(_git_dir), Some(common)) = (l.next(), l.next(), l.next()) else {
            fail!(NotARepo, "not inside a Git worktree");
        };
        Ok(Self {
            root: root.into(),
            common_dir: common.into(),
        })
    }

    pub fn git(&self) -> Command {
        git_in(&self.root)
    }

    pub fn run<I, S>(&self, args: I) -> Result<Vec<u8>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let out = self
            .git()
            .args(args)
            .output()
            .map_err(|e| err!(Internal, "cannot run git: {e}"))?;
        if !out.status.success() {
            fail!(
                Internal,
                "git failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(out.stdout)
    }

    fn ok<I, S>(&self, args: I) -> Option<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let out = self.git().args(args).output().ok()?;
        out.status.success().then(|| text(out.stdout))
    }

    pub fn head(&self) -> Option<String> {
        self.resolve_commit("HEAD")
    }

    pub fn resolve_commit(&self, rev: &str) -> Option<String> {
        self.ok(["rev-parse", "-q", "--verify", &format!("{rev}^{{commit}}")])
    }

    pub fn is_ancestor(&self, commit: &str, of: &str) -> bool {
        self.git()
            .args(["merge-base", "--is-ancestor", commit, of])
            .status()
            .is_ok_and(|s| s.success())
    }

    pub fn branch(&self) -> Option<String> {
        self.ok(["symbolic-ref", "--short", "-q", "HEAD"])
    }

    pub fn remote_url(&self, name: &str) -> Option<String> {
        self.ok(["remote", "get-url", name])
    }

    /// The ref landed commits must be reachable from.
    pub fn main_ref(&self, configured: Option<&str>) -> Option<String> {
        if let Some(r) = configured {
            return Some(r.to_owned());
        }
        [
            "origin/HEAD",
            "origin/main",
            "origin/master",
            "main",
            "master",
        ]
        .into_iter()
        .find(|r| self.resolve_commit(r).is_some())
        .map(str::to_owned)
    }

    pub fn commit(&self, rev: &str) -> Result<CommitInfo> {
        let sha = self
            .resolve_commit(rev)
            .ok_or_else(|| err!(CommitNotFound, "commit {rev} does not exist"))?;
        let out = self.run([
            "show",
            "-s",
            "--format=%H%x00%P%x00%an%x00%at%x00%s%x00%b",
            &sha,
        ])?;
        let s = String::from_utf8_lossy(&out).into_owned();
        let f: Vec<&str> = s.splitn(6, '\0').collect();
        Ok(CommitInfo {
            sha: f.first().copied().unwrap_or_default().trim().to_owned(),
            parents: f
                .get(1)
                .map(|p| p.split_whitespace().map(str::to_owned).collect())
                .unwrap_or_default(),
            author: f.get(2).copied().unwrap_or_default().to_owned(),
            time: f.get(3).and_then(|t| t.trim().parse().ok()).unwrap_or(0),
            subject: f.get(4).copied().unwrap_or_default().to_owned(),
            body: f.get(5).copied().unwrap_or_default().trim().to_owned(),
        })
    }

    /// Commits in `base..head`, oldest first.
    pub fn commits_between(&self, base: &str, head: &str) -> Result<Vec<String>> {
        let out = self.run(["rev-list", "--reverse", &format!("{base}..{head}")])?;
        Ok(text(out).lines().map(str::to_owned).collect())
    }

    /// Recent commits on HEAD, newest first.
    pub fn recent_commits(&self, n: usize) -> Result<Vec<String>> {
        let out = self.run(["rev-list", "-n", &n.to_string(), "HEAD"])?;
        Ok(text(out).lines().map(str::to_owned).collect())
    }

    /// The diff a commit introduced against its first parent (or the empty
    /// tree for a root commit), split per file.
    pub fn commit_diff(&self, rev: &str) -> Result<Vec<FileDiff>> {
        let out = self.run([
            "-c",
            "core.quotepath=off",
            "show",
            "--first-parent",
            "--no-color",
            "--format=",
            "--patch",
            "-M",
            "--unified=3",
            rev,
        ])?;
        Ok(parse_diff(&String::from_utf8_lossy(&out)))
    }

    /// Diff of the working tree and index against `rev` (drafts before commit).
    pub fn worktree_diff(&self, rev: &str) -> Result<Vec<FileDiff>> {
        let out = self.run([
            "-c",
            "core.quotepath=off",
            "diff",
            "--no-color",
            "-M",
            "--unified=3",
            rev,
        ])?;
        Ok(parse_diff(&String::from_utf8_lossy(&out)))
    }

    /// Commits that last touched `path` lines `start..=end` at `rev`.
    pub fn blame(
        &self,
        rev: &str,
        path: &str,
        range: Option<(u32, u32)>,
    ) -> Result<BTreeSet<String>> {
        let mut c = self.git();
        c.args(["blame", "--porcelain"]);
        if let Some((s, e)) = range {
            c.arg(format!("-L{s},{e}"));
        }
        c.arg(rev).arg("--").arg(path);
        let out = c
            .output()
            .map_err(|e| err!(Internal, "cannot run git: {e}"))?;
        if !out.status.success() {
            fail!(
                NotFound,
                "git blame {path} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let mut shas = BTreeSet::new();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let first = line.split(' ').next().unwrap_or_default();
            if first.len() >= 40
                && first.bytes().all(|b| b.is_ascii_hexdigit())
                && !first.bytes().all(|b| b == b'0')
            {
                shas.insert(first.to_owned());
            }
        }
        Ok(shas)
    }

    pub fn merge_base(&self, a: &str, b: &str) -> Option<String> {
        self.ok(["merge-base", a, b])
    }

    /// Commits a merge brought in from its other parents (empty otherwise).
    pub fn merged_commits(&self, sha: &str) -> Result<Vec<String>> {
        let out = self.run(["rev-list", &format!("{sha}^1..{sha}")])?;
        Ok(text(out)
            .lines()
            .filter(|l| !l.is_empty() && *l != sha)
            .map(str::to_owned)
            .collect())
    }

    /// Paths a commit touched.
    pub fn changed_paths(&self, rev: &str) -> Result<Vec<String>> {
        let out = self.run([
            "-c",
            "core.quotepath=off",
            "show",
            "--first-parent",
            "--format=",
            "--name-only",
            rev,
        ])?;
        Ok(text(out)
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect())
    }
}

/// Split a unified diff into per-file parts.
pub fn parse_diff(diff: &str) -> Vec<FileDiff> {
    let mut out: Vec<FileDiff> = Vec::new();
    for chunk in diff.split("\ndiff --git ").filter(|c| !c.trim().is_empty()) {
        let chunk = chunk.strip_prefix("diff --git ").unwrap_or(chunk);
        let mut path = None;
        let mut old_path = None;
        let mut old_ranges = Vec::new();
        for line in chunk.lines() {
            if let Some(p) = line.strip_prefix("+++ b/") {
                path = Some(p.to_owned());
            } else if let Some(p) = line.strip_prefix("--- a/") {
                old_path.get_or_insert_with(|| p.to_owned());
            } else if let Some(p) = line.strip_prefix("rename to ") {
                path = Some(p.to_owned());
            } else if let Some(p) = line.strip_prefix("rename from ") {
                old_path = Some(p.to_owned());
            } else if let Some(rest) = line.strip_prefix("@@ -") {
                let old = rest.split(' ').next().unwrap_or_default();
                let mut it = old.split(',');
                let start: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let count: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(1);
                if start > 0 && count > 0 {
                    old_ranges.push((start, count));
                }
            }
        }
        let path = path
            .or_else(|| old_path.clone())
            .or_else(|| {
                chunk
                    .lines()
                    .next()
                    .and_then(|h| h.split(" b/").nth(1))
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let old = old_path.filter(|o| *o != path);
        out.push(FileDiff {
            path,
            old_path: old,
            text: format!("diff --git {chunk}"),
            old_ranges,
        });
    }
    out
}

/// `owner/name` from a remote URL.
pub fn repo_name_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim_end_matches('/').trim_end_matches(".git");
    let (head, name) = trimmed.rsplit_once(['/', ':'])?;
    let owner = head.rsplit(['/', ':']).next().unwrap_or_default();
    let ok = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    };
    (ok(owner) && ok(name)).then(|| format!("{owner}/{name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_diffs() {
        let d = "diff --git a/x.rs b/x.rs\nindex 1..2 100644\n--- a/x.rs\n+++ b/x.rs\n@@ -3,2 +3,3 @@\n a\n-b\n+c\n+d\ndiff --git a/new.rs b/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/new.rs\n@@ -0,0 +1 @@\n+x\n";
        let f = parse_diff(d);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].path, "x.rs");
        assert_eq!(f[0].old_ranges, vec![(3, 2)]);
        assert_eq!(f[1].path, "new.rs");
        assert!(f[1].old_ranges.is_empty());
    }

    #[test]
    fn repo_names() {
        assert_eq!(
            repo_name_from_url("git@github.com:varops/gitstory.git").as_deref(),
            Some("varops/gitstory")
        );
        assert_eq!(repo_name_from_url("nope"), None);
    }
}
