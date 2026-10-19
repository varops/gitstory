//! Synthesis, validation and landing reconciliation (PRD §9.4–9.6).
//!
//! The model writes; local code decides what survives. Every claim must cite
//! known events; inferred claims only ever land in `hypotheses`; a separate
//! pass checks each claim against the durable parts of its citations; test
//! claims are checked mechanically against typed test facts.

use std::collections::{BTreeSet, HashMap};

use gitstory_proto::evidence::{EventType, EvidenceEvent};
use gitstory_proto::id;
use gitstory_proto::record::HunkRef;
use gitstory_proto::story::{
    Claim, ClaimStatus, ClaimType, Review, STORY_SCHEMA, Stage, Story, StoryStatus,
};
use serde_json::{Value, json};

use crate::config::RepoConfig;
use crate::error::{Failure, Result};
use crate::git::FileDiff;
use crate::model::{Gate, Request, Sent};
use crate::scrub;
use crate::{err, fail};

/// Everything one synthesis sees.
pub struct Input<'a> {
    pub work_unit: &'a str,
    pub repo: &'a str,
    pub commit: &'a str,
    pub commit_subject: &'a str,
    pub events: &'a [EvidenceEvent],
    pub diff: &'a [FileDiff],
    /// Claims from earlier drafts, for landing reconciliation.
    pub drafts: &'a [Story],
    pub stage: Stage,
    pub supersedes: Vec<String>,
}

const SYSTEM: &str = r#"You write GitStory records: the engineering decisions behind a change, for future engineers and coding agents who need to know why the code is the way it is.

You get evidence events (each with an id, a type, a typed fact and sometimes a short excerpt) and the diff. Write one story as JSON.

Rules:
- Every claim cites at least one evidence id from the input in "evidence". Cite only ids you were given. A claim you cannot cite does not belong in the story.
- "type": "observed" when the evidence itself proves it (a diff, a test result, a revert); "stated" when someone said it explicitly (the user, the agent, a commit message, a note); "inferred" when it is your own reading.
- Inferred claims go only in "hypotheses". Never put an inferred claim in decisions, rejected or discarded.
- "decisions": what the change actually does and the reasons given for it, status "implemented".
- "rejected": approaches that were tried or considered and dropped, with the evidence of the attempt or the reason, status "rejected".
- "discarded": things that appeared in drafts or attempts but are not in the final change and have no recorded reason, status "discarded".
- "evidence_summary": key supporting facts such as test results, status "none".
- "goal": the task's purpose, status "none".
- "unknowns": questions the evidence does not answer, especially the reason behind the main change when nobody stated it. Never invent intent.
- "paths": repository paths the decisions are about.
- "summary": one sentence saying what changed and why, using only supported facts.
- Be concise: short claims, no filler. Quote numbers exactly as the evidence gives them."#;

const LAND: &str = r#"

This is landing reconciliation. You also get "drafts": claims written while the work was in progress. Compare every draft claim with the landed diff:
- reflected in the landed diff -> keep it as a decision (status "implemented"),
- tried and dropped with evidence of the attempt or reason -> "rejected",
- in a draft but not in the landed diff and no evidence why -> "discarded".
A decision must describe what the landed code actually does."#;

const VALIDATE: &str = r#"You check claims against evidence. For each claim you get the claim text and the evidence it cites (typed facts and excerpts only). Answer for each claim whether the cited evidence supports it:
- "yes": the evidence alone supports the claim as written.
- "partly": plausible, but the evidence does not establish it.
- "no": unsupported or contradicted.
Judge only from the evidence given. Do not use outside knowledge."#;

fn claim_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["claim", "type", "status", "evidence"],
        "properties": {
            "claim": {"type": "string"},
            "type": {"type": "string", "enum": ["observed", "stated", "inferred"]},
            "status": {"type": "string", "enum": ["implemented", "rejected", "discarded", "none"]},
            "evidence": {"type": "array", "items": {"type": "string"}}
        }
    })
}

fn story_schema() -> Value {
    let list = json!({"type": "array", "items": claim_schema()});
    let strings = json!({"type": "array", "items": {"type": "string"}});
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["summary", "goal", "decisions", "rejected", "discarded", "evidence_summary", "hypotheses", "limitations", "unknowns", "paths"],
        "properties": {
            "summary": {"type": "string"},
            "goal": claim_schema(),
            "decisions": list,
            "rejected": list,
            "discarded": list,
            "evidence_summary": list,
            "hypotheses": list,
            "limitations": strings,
            "unknowns": strings,
            "paths": strings
        }
    })
}

fn verdict_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["verdicts"],
        "properties": {
            "verdicts": {"type": "array", "items": {
                "type": "object",
                "additionalProperties": false,
                "required": ["index", "supported", "note"],
                "properties": {
                    "index": {"type": "integer"},
                    "supported": {"type": "string", "enum": ["yes", "partly", "no"]},
                    "note": {"type": "string"}
                }
            }}
        }
    })
}

/// The events and diff the policy allows to leave, as JSON for the prompt.
fn gated_bundle(
    cfg: &RepoConfig,
    events: &[EvidenceEvent],
    diff: &[FileDiff],
    ignored: &dyn Fn(&str) -> bool,
) -> (Value, Sent) {
    let excerpts = cfg.egress.send_excerpts && cfg.collection.excerpts == "on";
    let evs: Vec<Value> = if cfg.egress.send_events {
        events.iter().map(|e| event_json(e, excerpts)).collect()
    } else {
        Vec::new()
    };
    let mut hunks = Vec::new();
    let mut files = Vec::new();
    if cfg.egress.send_diff {
        for f in diff {
            if ignored(&f.path) || f.old_path.as_deref().is_some_and(ignored) {
                continue;
            }
            // Diffs are source code: scrub secrets before they leave.
            let (text, _) = scrub::scrub(&f.text);
            let text: String = text.chars().take(20_000).collect();
            hunks.push(HunkRef {
                path: f.path.clone(),
                hash: blake3::hash(text.as_bytes()).to_hex().to_string(),
            });
            files.push(json!({"path": f.path, "diff": text}));
        }
    }
    let ids = if cfg.egress.send_events {
        events.iter().map(|e| e.id.clone()).collect()
    } else {
        Vec::new()
    };
    (
        json!({"events": evs, "diff": files}),
        Sent {
            purpose: "",
            event_ids: ids,
            hunks,
            excerpts,
        },
    )
}

fn event_json(e: &EvidenceEvent, excerpts: bool) -> Value {
    let mut v = json!({
        "id": e.id,
        "type": e.kind,
        "fact": e.durable.fact,
        "source": e.durable.source,
        "author": e.durable.author,
    });
    if let Some(t) = &e.durable.test {
        v["test"] = json!({"command": t.command, "passed": t.passed, "total": t.total});
    }
    if excerpts && let Some(x) = &e.durable.excerpt {
        v["excerpt"] = json!(x);
    }
    if !e.paths.is_empty() {
        v["paths"] = json!(e.paths);
    }
    v
}

fn parse_claim(v: &Value) -> Option<Claim> {
    let kind = match v["type"].as_str()? {
        "observed" => ClaimType::Observed,
        "stated" => ClaimType::Stated,
        _ => ClaimType::Inferred,
    };
    let status = match v["status"].as_str() {
        Some("implemented") => Some(ClaimStatus::Implemented),
        Some("rejected") => Some(ClaimStatus::Rejected),
        Some("discarded") => Some(ClaimStatus::Discarded),
        _ => None,
    };
    let claim = v["claim"].as_str()?.trim().to_owned();
    if claim.is_empty() {
        return None;
    }
    Some(Claim {
        claim: scrub::scrub(&claim).0,
        kind,
        status,
        evidence: v["evidence"]
            .as_array()?
            .iter()
            .filter_map(|e| e.as_str().map(str::to_owned))
            .collect(),
    })
}

fn claims(v: &Value) -> Vec<Claim> {
    v.as_array()
        .map(|a| a.iter().filter_map(parse_claim).collect())
        .unwrap_or_default()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str())
                .map(|s| scrub::scrub(s.trim()).0)
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Enforce the story rules locally, whatever the model wrote.
fn normalize(mut s: Story, events: &[EvidenceEvent]) -> Story {
    let known: BTreeSet<&str> = events.iter().map(|e| e.id.as_str()).collect();
    let keep = |mut c: Claim| -> Option<Claim> {
        c.evidence.retain(|e| known.contains(e.as_str()));
        c.evidence.dedup();
        (!c.evidence.is_empty()).then_some(c)
    };
    let mut hypotheses: Vec<Claim> = Vec::new();
    let mut place = |list: Vec<Claim>, status: Option<ClaimStatus>| -> Vec<Claim> {
        let mut out = Vec::new();
        for c in list.into_iter().filter_map(keep) {
            if c.kind == ClaimType::Inferred {
                hypotheses.push(Claim { status: None, ..c });
            } else {
                out.push(Claim { status, ..c });
            }
        }
        out
    };
    s.decisions = place(
        std::mem::take(&mut s.decisions),
        Some(ClaimStatus::Implemented),
    );
    s.rejected = place(std::mem::take(&mut s.rejected), Some(ClaimStatus::Rejected));
    s.discarded = place(
        std::mem::take(&mut s.discarded),
        Some(ClaimStatus::Discarded),
    );
    s.evidence_summary = place(std::mem::take(&mut s.evidence_summary), None);
    for h in std::mem::take(&mut s.hypotheses)
        .into_iter()
        .filter_map(keep)
    {
        hypotheses.push(Claim {
            kind: ClaimType::Inferred,
            status: None,
            ..h
        });
    }
    s.hypotheses = hypotheses;
    // The goal must cite something; fall back to the recorded goal event.
    let goal_ok = keep(s.goal.clone());
    s.goal = match goal_ok {
        Some(g) if g.kind != ClaimType::Inferred => Claim { status: None, ..g },
        _ => match events.iter().find(|e| e.kind == EventType::Goal) {
            Some(e) => Claim {
                claim: e
                    .durable
                    .excerpt
                    .clone()
                    .unwrap_or_else(|| e.durable.fact.clone()),
                kind: ClaimType::Stated,
                status: None,
                evidence: vec![e.id.clone()],
            },
            None => {
                let first = events.first().expect("events are not empty");
                s.unknowns
                    .push("The goal of this work is not recorded in the evidence.".into());
                Claim {
                    claim: format!("Change described by: {}", first.durable.fact),
                    kind: ClaimType::Observed,
                    status: None,
                    evidence: vec![first.id.clone()],
                }
            }
        },
    };
    // If no decision rests on a stated reason, say so (PRD §9.4).
    let by_id: HashMap<&str, &EvidenceEvent> = events.iter().map(|e| (e.id.as_str(), e)).collect();
    let reasoned = s.decisions.iter().any(|d| {
        d.kind == ClaimType::Stated
            || d.evidence.iter().any(|e| {
                by_id.get(e.as_str()).is_some_and(|ev| {
                    matches!(
                        ev.kind,
                        EventType::Reason | EventType::Decision | EventType::Constraint
                    )
                })
            })
    });
    if !reasoned && s.unknowns.is_empty() {
        s.unknowns
            .push("Why this change was made is not stated in the evidence.".into());
    }
    s.paths.sort();
    s.paths.dedup();
    s
}

/// Build a story from evidence (draft) or reconcile drafts against a landed diff.
pub fn synthesize(
    cfg: &RepoConfig,
    gate: &mut Gate,
    input: &Input,
    ignored: &dyn Fn(&str) -> bool,
) -> Result<Story> {
    if input.events.is_empty() {
        fail!(
            NoEvidence,
            "work unit {} has no evidence yet",
            input.work_unit
        );
    }
    let landing = input.stage == Stage::Landed && !input.drafts.is_empty();
    let (bundle, mut sent) = gated_bundle(cfg, input.events, input.diff, ignored);
    sent.purpose = if landing {
        "reconciliation"
    } else {
        "synthesis"
    };
    let mut user = json!({
        "work_unit": input.work_unit,
        "commit": {"sha": input.commit, "subject": scrub::scrub(input.commit_subject).0},
        "events": bundle["events"],
        "diff": bundle["diff"],
    });
    if landing {
        let draft_claims: Vec<Value> = input
            .drafts
            .iter()
            .flat_map(|d| {
                d.claims().into_iter().map(
                    |(sec, c)| json!({"section": sec, "claim": c.claim, "evidence": c.evidence}),
                )
            })
            .collect();
        user["drafts"] = json!(draft_claims);
    }
    let req = Request {
        system: if landing {
            format!("{SYSTEM}{LAND}")
        } else {
            SYSTEM.to_owned()
        },
        user: serde_json::to_string_pretty(&user)?,
        schema: story_schema(),
        max_tokens: 16_000,
    };
    let mut last: Option<Failure> = None;
    for _ in 0..2 {
        let v = match gate.call(
            &cfg.model,
            &req,
            Sent {
                ..clone_sent(&sent)
            },
        ) {
            Ok(v) => v,
            Err(f) if f.code == gitstory_proto::ErrorCode::SynthesisInvalid => {
                last = Some(f);
                continue;
            }
            Err(f) => return Err(f),
        };
        let Some(goal) = parse_claim(&v["goal"]) else {
            last = Some(err!(
                SynthesisInvalid,
                "the model's story has no valid goal"
            ));
            continue;
        };
        let story = Story {
            story_id: id::new_id(id::STORY),
            schema: STORY_SCHEMA.into(),
            repo: input.repo.into(),
            commit: input.commit.into(),
            contains: Vec::new(),
            work_unit: input.work_unit.into(),
            stage: input.stage,
            version: 1,
            created_at: id::now_ms(),
            status: StoryStatus::Current,
            supersedes: input.supersedes.clone(),
            related: Vec::new(),
            summary: scrub::scrub(v["summary"].as_str().unwrap_or_default().trim()).0,
            goal,
            decisions: claims(&v["decisions"]),
            rejected: claims(&v["rejected"]),
            discarded: claims(&v["discarded"]),
            evidence_summary: claims(&v["evidence_summary"]),
            hypotheses: claims(&v["hypotheses"]),
            limitations: strings(&v["limitations"]),
            unknowns: strings(&v["unknowns"]),
            paths: strings(&v["paths"]),
            links: Default::default(),
            review: Review {
                status: "unreviewed".into(),
            },
            egress: String::new(),
        };
        if story.summary.is_empty() {
            last = Some(err!(SynthesisInvalid, "the model's story has no summary"));
            continue;
        }
        return Ok(normalize(story, input.events));
    }
    Err(last.unwrap_or_else(|| err!(SynthesisInvalid, "synthesis failed")))
}

fn clone_sent(s: &Sent) -> Sent {
    Sent {
        purpose: s.purpose,
        event_ids: s.event_ids.clone(),
        hunks: s.hunks.clone(),
        excerpts: s.excerpts,
    }
}

/// Mechanical check of a claim that cites only test events: the typed test
/// facts must agree with what the claim says about passing or failing.
fn test_check(c: &Claim, by_id: &HashMap<&str, &EvidenceEvent>) -> Option<bool> {
    let tests: Vec<_> = c
        .evidence
        .iter()
        .filter_map(|e| by_id.get(e.as_str()))
        .filter_map(|e| e.durable.test.as_ref())
        .collect();
    if tests.is_empty() || tests.len() != c.evidence.len() {
        return None;
    }
    let l = c.claim.to_lowercase();
    let says_pass = ["pass", "green", "succeed"].iter().any(|w| l.contains(w));
    let says_fail = ["fail", "red", "broke", "flak"]
        .iter()
        .any(|w| l.contains(w));
    let all_pass = tests.iter().all(|t| t.passed == t.total && t.total > 0);
    let any_fail = tests.iter().any(|t| t.passed < t.total);
    // Numbers like "50 of 50" or "48/50" must match some test fact.
    let nums: Vec<(u32, u32)> = {
        let re = regex::Regex::new(r"(\d+)\s*(?:/|of|out of)\s*(\d+)").expect("valid regex");
        re.captures_iter(&l)
            .filter_map(|m| Some((m[1].parse().ok()?, m[2].parse().ok()?)))
            .collect()
    };
    // Exact counts decide on their own: "48 of 50 passed" is true of 48/50.
    if !nums.is_empty() {
        return Some(
            nums.iter()
                .all(|n| tests.iter().any(|t| (t.passed, t.total) == *n)),
        );
    }
    if says_pass && !says_fail {
        return Some(all_pass);
    }
    if says_fail && !says_pass {
        return Some(any_fail);
    }
    Some(true)
}

/// The validation pass (PRD §9.5): durable parts only. Unsupported claims are
/// dropped; partly supported ones become hypotheses.
pub fn validate(
    cfg: &RepoConfig,
    gate: &mut Gate,
    mut s: Story,
    events: &[EvidenceEvent],
) -> Result<Story> {
    let by_id: HashMap<&str, &EvidenceEvent> = events.iter().map(|e| (e.id.as_str(), e)).collect();
    let excerpts = cfg.egress.send_excerpts && cfg.collection.excerpts == "on";
    let sections: [&mut Vec<Claim>; 4] = [
        &mut s.decisions,
        &mut s.rejected,
        &mut s.discarded,
        &mut s.evidence_summary,
    ];
    // Index every authoritative claim.
    let mut items: Vec<(usize, usize)> = Vec::new();
    let mut prompt_items = Vec::new();
    let mut cited = BTreeSet::new();
    for (si, list) in sections.iter().enumerate() {
        for (ci, c) in list.iter().enumerate() {
            let evs: Vec<Value> = c
                .evidence
                .iter()
                .filter_map(|e| by_id.get(e.as_str()))
                .map(|e| {
                    cited.insert(e.id.clone());
                    // Durable parts only: no external pointers.
                    event_json(e, excerpts)
                })
                .collect();
            prompt_items.push(json!({"index": items.len(), "claim": c.claim, "evidence": evs}));
            items.push((si, ci));
        }
    }
    let mut verdict: Vec<&str> = vec!["yes"; items.len()];
    // Mechanical test checks first; they override the model.
    let mut mechanical: Vec<Option<bool>> = Vec::new();
    for (si, ci) in &items {
        mechanical.push(test_check(&sections[*si][*ci], &by_id));
    }
    let validation_model = cfg.validation_model();
    if !items.is_empty() && validation_model.provider != "none" {
        let req = Request {
            system: VALIDATE.into(),
            user: serde_json::to_string_pretty(&json!({"claims": prompt_items}))?,
            schema: verdict_schema(),
            max_tokens: 8_000,
        };
        let sent = Sent {
            purpose: "validation",
            event_ids: cited.into_iter().collect(),
            hunks: Vec::new(),
            excerpts,
        };
        let v = gate.call(validation_model, &req, sent)?;
        for item in v["verdicts"].as_array().cloned().unwrap_or_default() {
            if let (Some(i), Some(sup)) = (item["index"].as_u64(), item["supported"].as_str())
                && let Some(slot) = verdict.get_mut(i as usize)
            {
                *slot = match sup {
                    "no" => "no",
                    "partly" => "partly",
                    _ => "yes",
                };
            }
        }
    }
    for (i, m) in mechanical.iter().enumerate() {
        match m {
            Some(false) => verdict[i] = "no",
            Some(true) => verdict[i] = "yes",
            None => {}
        }
    }
    // Apply, back to front so indices stay valid.
    let mut moved = Vec::new();
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|a, b| items[*b].cmp(&items[*a]));
    for i in order {
        let (si, ci) = items[i];
        match verdict[i] {
            "no" => {
                sections[si].remove(ci);
            }
            "partly" => {
                let c = sections[si].remove(ci);
                moved.push(Claim {
                    kind: ClaimType::Inferred,
                    status: None,
                    ..c
                });
            }
            _ => {}
        }
    }
    s.hypotheses.extend(moved);
    if s.decisions.is_empty() && s.unknowns.is_empty() {
        s.unknowns
            .push("No decision in this change is supported by the evidence.".into());
    }
    Ok(s)
}

/// Safety net after reconciliation: a decision whose only evidence is
/// edits to files the landed commit does not touch cannot describe the
/// landed code, so it becomes discarded history.
pub fn guard_landed(
    mut s: Story,
    events: &[EvidenceEvent],
    landed_paths: &BTreeSet<String>,
) -> Story {
    let by_id: HashMap<&str, &EvidenceEvent> = events.iter().map(|e| (e.id.as_str(), e)).collect();
    let mut keep = Vec::new();
    for d in std::mem::take(&mut s.decisions) {
        let evs: Vec<&&EvidenceEvent> = d
            .evidence
            .iter()
            .filter_map(|e| by_id.get(e.as_str()))
            .collect();
        let only_attempts = !evs.is_empty()
            && evs
                .iter()
                .all(|e| e.kind == EventType::Attempt && !e.paths.is_empty());
        let touches = evs
            .iter()
            .any(|e| e.paths.iter().any(|p| landed_paths.contains(p)));
        if only_attempts && !touches {
            s.discarded.push(Claim {
                status: Some(ClaimStatus::Discarded),
                ..d
            });
        } else {
            keep.push(d);
        }
    }
    s.decisions = keep;
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitstory_proto::evidence::{Durable, SourceType, TestFact};

    fn ev(id: &str, kind: EventType, test: Option<(u32, u32)>, paths: &[&str]) -> EvidenceEvent {
        EvidenceEvent {
            id: id.into(),
            kind,
            work_unit: "w".into(),
            durable: Durable {
                fact: "f".into(),
                test: test.map(|(p, t)| TestFact {
                    command: "cargo test".into(),
                    passed: p,
                    total: t,
                }),
                excerpt: None,
                source_hash: id.into(),
                source: SourceType::TestRun,
                at: 0,
                author: "a".into(),
            },
            external: None,
            paths: paths.iter().map(|p| p.to_string()).collect(),
        }
    }

    fn claim(text: &str, ids: &[&str]) -> Claim {
        Claim {
            claim: text.into(),
            kind: ClaimType::Observed,
            status: None,
            evidence: ids.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn test_claims_are_checked_mechanically() {
        let events = [ev("ev_1", EventType::Test, Some((48, 50)), &[])];
        let by_id: HashMap<&str, &EvidenceEvent> =
            events.iter().map(|e| (e.id.as_str(), e)).collect();
        assert_eq!(
            test_check(&claim("All 50 tests pass", &["ev_1"]), &by_id),
            Some(false)
        );
        assert_eq!(
            test_check(&claim("48 of 50 tests passed", &["ev_1"]), &by_id),
            Some(true)
        );
        assert_eq!(
            test_check(&claim("50/50 passed", &["ev_1"]), &by_id),
            Some(false)
        );
    }

    #[test]
    fn landed_guard_discards_untouched_attempts() {
        let events = [
            ev("ev_1", EventType::Attempt, None, &["redis.rs"]),
            ev("ev_2", EventType::Attempt, None, &["lock.rs"]),
        ];
        let s = Story {
            story_id: "st".into(),
            schema: STORY_SCHEMA.into(),
            repo: "a/b".into(),
            commit: "c".into(),
            contains: vec![],
            work_unit: "w".into(),
            stage: Stage::Landed,
            version: 1,
            created_at: 0,
            status: StoryStatus::Current,
            supersedes: vec![],
            related: vec![],
            summary: "s".into(),
            goal: claim("g", &["ev_1"]),
            decisions: vec![
                Claim {
                    status: Some(ClaimStatus::Implemented),
                    ..claim("Added a Redis lock", &["ev_1"])
                },
                Claim {
                    status: Some(ClaimStatus::Implemented),
                    ..claim("Row versioning", &["ev_2"])
                },
            ],
            rejected: vec![],
            discarded: vec![],
            evidence_summary: vec![],
            hypotheses: vec![],
            limitations: vec![],
            unknowns: vec![],
            paths: vec![],
            links: Default::default(),
            review: Review {
                status: "unreviewed".into(),
            },
            egress: "e".into(),
        };
        let landed: BTreeSet<String> = ["lock.rs".to_string()].into();
        let s = guard_landed(s, &events, &landed);
        assert_eq!(s.decisions.len(), 1);
        assert_eq!(s.decisions[0].claim, "Row versioning");
        assert_eq!(s.discarded[0].claim, "Added a Redis lock");
    }
}
