//! ghf adapter (PRD §8.11): reads traces through ghf's public JSON CLI and
//! holds them against pruning while the work unit is open. GitStory works
//! the same without ghf; every failure here means "no ghf evidence".

use std::collections::HashSet;
use std::process::Command;

use gitstory_proto::evidence::{EventType, EvidenceEvent, SourceType};
use serde_json::Value;

use crate::adapters::Ctx;
use crate::collect::WorkUnit;
use crate::error::Result;

const HOLD: &str = "gitstory";

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
/// as attempt, test and abandon events. Holds each trace it reads.
pub fn traces(ctx: &Ctx, wu: &mut WorkUnit) -> Result<Vec<EvidenceEvent>> {
    if !ctx.ws.repo.common_dir.join("ghf").is_dir() {
        return Ok(Vec::new());
    }
    let Some(list) = ghf(ctx, &["trace", "list"]) else {
        return Ok(Vec::new());
    };
    // Commits in scope: the base and everything after it.
    let mut revs: HashSet<String> = HashSet::new();
    if let (Some(base), Some(head)) = (&wu.meta.base_rev, ctx.ws.repo.head()) {
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
        if !wu.meta.ghf_holds.iter().any(|h| h == tid)
            && ghf(
                ctx,
                &["trace", "hold", tid, "--name", HOLD, "--expires", "30d"],
            )
            .is_some()
        {
            wu.meta.ghf_holds.push(tid.to_owned());
        }
    }
    Ok(out)
}

/// Release the holds once the work unit's evidence is in the store.
pub fn release(ctx: &Ctx, wu: &mut WorkUnit) {
    wu.meta
        .ghf_holds
        .retain(|tid| ghf(ctx, &["trace", "release", tid, "--name", HOLD]).is_none());
}
