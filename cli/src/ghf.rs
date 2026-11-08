//! ghf adapter (PRD §8.11): reads traces through ghf's public JSON CLI and
//! holds them against pruning while the work unit is open. GitStory works
//! the same without ghf; every failure here means "no ghf evidence".

use std::collections::HashSet;
use std::process::Command;

use gitstory_proto::evidence::{EventType, EvidenceEvent, SourceType};
use serde_json::Value;

use crate::adapters::Ctx;
use crate::collect::{Scope, WorkUnit};
use crate::error::Result;

const HOLD: &str = "gitstory";
/// Renew a hold older than this (holds are placed for 30 days).
const RENEW_MS: i64 = 7 * 86_400_000;

/// `$GITSTORY_GHF`, else `git hf`.
fn command(ctx: &Ctx) -> Command {
    let mut c = match std::env::var("GITSTORY_GHF") {
        Ok(exe) if !exe.is_empty() => Command::new(exe),
        _ => {
            let mut c = Command::new("git");
            c.arg("hf");
            c
        }
    };
    c.current_dir(&ctx.ws.repo.root);
    c
}

/// Run a ghf command; `None` when ghf is missing or the call failed.
fn ghf(ctx: &Ctx, args: &[&str]) -> Option<Value> {
    let out = command(ctx).args(args).arg("--json").output().ok()?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    if v["ok"] == Value::Bool(true) {
        Some(v["data"].clone())
    } else {
        None
    }
}

/// Checkpoints of this worktree's traces, from the work unit's base onward,
/// as attempt, test and abandon events. Holds each trace it reads; returns
/// the events and the traces it placed or renewed a hold on, for the caller to record
/// in the work unit's meta.
pub fn traces(
    ctx: &Ctx,
    wu: &WorkUnit,
    scope: &Scope,
) -> Result<(Vec<EvidenceEvent>, Vec<String>)> {
    let mut held = Vec::new();
    let now = gitstory_proto::id::now_ms();
    if !ctx.ws.repo.common_dir.join("ghf").is_dir() {
        return Ok((Vec::new(), held));
    }
    let Some(list) = ghf(ctx, &["trace", "list"]) else {
        return Ok((Vec::new(), held));
    };
    // Commits in scope: the base and everything after it.
    let mut revs: HashSet<String> = HashSet::new();
    if let (Some(base), Some(head)) = (
        &scope.base,
        scope.tip.clone().or_else(|| ctx.ws.repo.head()),
    ) {
        revs.insert(base.clone());
        revs.extend(ctx.ws.repo.commits_between(base, &head).unwrap_or_default());
    }
    let mut out = Vec::new();
    for t in list["traces"].as_array().into_iter().flatten() {
        if t["local"] != Value::Bool(true) || t["state"] == "pruned" {
            continue;
        }
        let Some(tid) = t["id"].as_str() else {
            continue;
        };
        let Some(log) = ghf(ctx, &["trace", "log", tid]) else {
            continue;
        };
        let mut used = false;
        for c in log["checkpoints"].as_array().into_iter().flatten() {
            let base = c["base_rev"].as_str().unwrap_or("");
            if !revs.is_empty() && !revs.contains(base) {
                continue;
            }
            used = true;
            let cid = c["id"].as_str().unwrap_or("");
            let at = c["created_at"].as_i64().unwrap_or(0);
            let author = c["author"].as_str().unwrap_or("ghf");
            let paths: Vec<String> = c["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|ch| ch["path"].as_str().map(str::to_owned))
                .collect();
            let pointer = Some(format!("ghf:{tid}/{cid}"));
            let message = c["message"].as_str().filter(|m| !m.is_empty());
            let fact = format!(
                "ghf checkpoint {} on {tid} changed {} file(s){}",
                c["seq"],
                paths.len(),
                message.map(|m| format!(": {m}")).unwrap_or_default()
            );
            out.push(ctx.event(
                EventType::Attempt,
                SourceType::GhfTrace,
                author,
                at,
                &fact,
                message,
                format!("ghf\0{cid}").as_bytes(),
                pointer.clone(),
                paths.clone(),
            ));
            if let Some(r) = c["annotations"]["tests"].as_str() {
                out.push(ctx.event(
                    EventType::Test,
                    SourceType::GhfTrace,
                    author,
                    at,
                    &format!("tests {r} at ghf checkpoint {cid}"),
                    None,
                    format!("ghf-tests\0{cid}\0{r}").as_bytes(),
                    pointer,
                    paths,
                ));
            }
        }
        if !used {
            continue;
        }
        if t["abandoned"] == Value::Bool(true) {
            out.push(ctx.event(
                EventType::Abandon,
                SourceType::GhfTrace,
                "ghf",
                t["finalized_at"].as_i64().unwrap_or(0),
                &format!("ghf trace {tid} was finalized as abandoned"),
                None,
                format!("ghf-abandon\0{tid}").as_bytes(),
                Some(format!("ghf:{tid}")),
                Vec::new(),
            ));
        }
        // Holds expire after 30 days; renew one older than a week so a
        // long-running work unit never loses its trace.
        let due = wu
            .meta
            .ghf_holds
            .iter()
            .find(|h| h.id == tid)
            .is_none_or(|h| now - h.at > RENEW_MS);
        if due
            && ghf(
                ctx,
                &["trace", "hold", tid, "--name", HOLD, "--expires", "30d"],
            )
            .is_some()
        {
            held.push(tid.to_owned());
        }
    }
    Ok((out, held))
}

/// Release the work unit's holds: once its evidence is in the store, or
/// when gc removes it. Returns the traces released.
pub fn release(ctx: &Ctx, wu: &WorkUnit) -> Vec<String> {
    wu.meta
        .ghf_holds
        .iter()
        .filter(|h| ghf(ctx, &["trace", "release", &h.id, "--name", HOLD]).is_some())
        .map(|h| h.id.clone())
        .collect()
}
