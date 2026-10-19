//! Hooks that draft in the background so nothing waits on synthesis
//! (PRD §11), and the agent skill.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::error::{Context, Result};
use crate::git::Repo;
use crate::{err, fail};

/// What the hooks run. Backgrounded by `draft` itself; never blocks.
pub const DRAFT_COMMAND: &str = "git story draft --background --json";
const GIT_MARKER: &str = "# gitstory: draft in the background";

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Harness {
    /// Claude Code `Stop` hook: drafts when the agent finishes a turn.
    ClaudeCode,
    /// Git `post-commit` hook.
    Git,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Scope {
    /// Shared project settings, committed with the repository.
    Project,
    /// Settings for this checkout only (Claude Code: settings.local.json).
    Local,
    /// The user's settings in the home directory.
    User,
}

pub const SKILL: &str = include_str!("../../skills/gitstory/SKILL.md");

fn home() -> Result<PathBuf> {
    crate::config::home_dir().ok_or_else(|| err!(Config, "no home directory (HOME or USERPROFILE)"))
}

pub fn install_skill(scope: Scope, root: Option<&Path>) -> Result<(PathBuf, bool)> {
    let base = match scope {
        Scope::User => home()?,
        _ => root
            .ok_or_else(|| err!(NotARepo, "not inside a Git worktree; use --scope user"))?
            .to_path_buf(),
    };
    let path = base.join(".claude/skills/gitstory/SKILL.md");
    if fs::read_to_string(&path).is_ok_and(|t| t == SKILL) {
        return Ok((path, false));
    }
    fs::create_dir_all(path.parent().expect("parent"))?;
    fs::write(&path, SKILL).context(|| format!("write {}", path.display()))?;
    Ok((path, true))
}

/// Install, remove or inspect a hook. Returns (path, changed, installed).
pub fn manage(
    h: Harness,
    scope: Scope,
    repo: Option<&Repo>,
    action: &str,
) -> Result<(PathBuf, bool, bool)> {
    match h {
        Harness::ClaudeCode => claude(scope, repo, action),
        Harness::Git => {
            let repo = repo.ok_or_else(|| err!(NotARepo, "not inside a Git worktree"))?;
            git(repo, action)
        }
    }
}

fn claude(scope: Scope, repo: Option<&Repo>, action: &str) -> Result<(PathBuf, bool, bool)> {
    let root = || {
        repo.map(|r| r.root.clone())
            .ok_or_else(|| err!(NotARepo, "not inside a Git worktree; use --scope user"))
    };
    let path = match scope {
        Scope::Project => root()?.join(".claude/settings.json"),
        Scope::Local => root()?.join(".claude/settings.local.json"),
        Scope::User => home()?.join(".claude/settings.json"),
    };
    let mut v: Value = match fs::read_to_string(&path) {
        Ok(t) if t.trim().is_empty() => json!({}),
        Ok(t) => serde_json::from_str(&t)
            .map_err(|e| err!(Config, "{} is not valid JSON: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e).context(|| format!("read {}", path.display())),
    };
    if !v.is_object() {
        fail!(Config, "{} must hold a JSON object", path.display());
    }
    let ours = |e: &Value| e.get("command").and_then(Value::as_str) == Some(DRAFT_COMMAND);
    let has = |v: &Value| {
        v.pointer("/hooks/Stop")
            .and_then(Value::as_array)
            .is_some_and(|groups| {
                groups.iter().any(|g| {
                    g.get("hooks")
                        .and_then(Value::as_array)
                        .is_some_and(|a| a.iter().any(ours))
                })
            })
    };
    let before = has(&v);
    let mut changed = false;
    match action {
        "install" if !before => {
            let list = v
                .as_object_mut()
                .expect("object")
                .entry("hooks")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or_else(|| err!(Config, "\"hooks\" in {} is not an object", path.display()))?
                .entry("Stop")
                .or_insert_with(|| json!([]));
            list.as_array_mut()
                .ok_or_else(|| err!(Config, "hooks.Stop in {} is not a list", path.display()))?
                .push(json!({"hooks": [{"type": "command", "command": DRAFT_COMMAND, "timeout": 30}]}));
            changed = true;
        }
        "uninstall" if before => {
            if let Some(arr) = v.pointer_mut("/hooks/Stop").and_then(Value::as_array_mut) {
                for g in arr.iter_mut() {
                    if let Some(inner) = g.get_mut("hooks").and_then(Value::as_array_mut) {
                        inner.retain(|e| !ours(e));
                    }
                }
                arr.retain(|g| {
                    g.get("hooks")
                        .and_then(Value::as_array)
                        .is_none_or(|a| !a.is_empty())
                });
            }
            changed = true;
        }
        _ => {}
    }
    if changed {
        if let Some(d) = path.parent() {
            fs::create_dir_all(d)?;
        }
        let mut text = serde_json::to_string_pretty(&v)?;
        text.push('\n');
        fs::write(&path, text).context(|| format!("write {}", path.display()))?;
    }
    Ok((path.clone(), changed, has(&v)))
}

fn git(repo: &Repo, action: &str) -> Result<(PathBuf, bool, bool)> {
    let dir = repo
        .run(["rev-parse", "--git-path", "hooks"])
        .ok()
        .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
        .filter(|s| !s.is_empty())
        .map(|s| {
            let p = PathBuf::from(s);
            if p.is_absolute() {
                p
            } else {
                repo.root.join(p)
            }
        })
        .unwrap_or_else(|| repo.common_dir.join("hooks"));
    let path = dir.join("post-commit");
    let text = fs::read_to_string(&path).unwrap_or_default();
    let before = text.contains(GIT_MARKER);
    let line = format!("{GIT_MARKER}\n{DRAFT_COMMAND} >/dev/null 2>&1 || true\n");
    let mut changed = false;
    match action {
        "install" if !before => {
            let new = if text.is_empty() {
                format!("#!/bin/sh\n{line}")
            } else {
                format!("{}\n{line}", text.trim_end())
            };
            fs::create_dir_all(&dir)?;
            fs::write(&path, new).context(|| format!("write {}", path.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
            }
            changed = true;
        }
        "uninstall" if before => {
            let new = text.replace(&line, "");
            if new.trim() == "#!/bin/sh" {
                fs::remove_file(&path)?;
            } else {
                fs::write(&path, new)?;
            }
            changed = true;
        }
        _ => {}
    }
    let installed = fs::read_to_string(&path).is_ok_and(|t| t.contains(GIT_MARKER));
    Ok((path, changed, installed))
}
