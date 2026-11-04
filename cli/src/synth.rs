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
- "summary": one plain sentence saying what changed and why, using only supported facts. No evidence ids in the summary.
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
Each claim says whether the diff applies to it ("diff_applies"). Only when it is true is the diff of the change evidence for that claim, because the claim cites an edit or commit. When it is false, ignore the diff for that claim, even if the diff would support it.
Judge each claim only from its own cited evidence, plus the diff when it applies. Evidence cited by other claims in this request does not count for it. Do not use outside knowledge.
Copy each claim's text exactly into "claim" in its verdict, so the verdict cannot attach to the wrong claim."#;

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
                "required": ["index", "claim", "supported", "note"],
                "properties": {
                    "index": {"type": "integer"},
                    "claim": {"type": "string"},
                    "supported": {"type": "string", "enum": ["yes", "partly", "no"]},
                    "note": {"type": "string"}
                }
            }}
        }
    })
}

/// The events and diff the policy allows to leave, as JSON for the prompt.
pub(crate) fn gated_bundle(
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
pub(crate) fn normalize(mut s: Story, events: &[EvidenceEvent]) -> Story {
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
            summary: strip_ids(&scrub::scrub(v["summary"].as_str().unwrap_or_default()).0),
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

/// Evidence ids belong in claims, not in prose.
fn strip_ids(s: &str) -> String {
    use std::sync::LazyLock;
    static IDS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"\s*[\[(](?:\s*,?\s*ev_[0-9a-z]+)+\s*[\])]").expect("valid regex")
    });
    IDS.replace_all(s, "").trim().to_owned()
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

/// A claim the repair step rewrote so its evidence supports it.
#[derive(Debug, Clone)]
pub struct Repair {
    pub from: String,
    pub to: String,
}

/// The first verdict on one claim, before any repair, with the checker's
/// note. For audits: it shows why a claim was kept, moved or dropped.
#[derive(Debug, Clone)]
pub struct Judged {
    pub claim: String,
    /// `yes`, `partly` or `no`, after the mechanical test check.
    pub verdict: &'static str,
    pub note: String,
}

/// What validation did, for audits.
#[derive(Debug, Clone, Default)]
pub struct ValidationLog {
    pub judged: Vec<Judged>,
    pub repairs: Vec<Repair>,
}

const REPAIR: &str = r#"You fix claims that a checker could not fully confirm. For each claim you get its text, the evidence it cites (typed facts and excerpts only), the checker's note on what is missing, and whether the diff of the change applies to it ("diff_applies").
Rewrite each claim so that its own cited evidence supports it, plus the diff only when "diff_applies" is true. Evidence cited by other claims does not count, and neither does the diff when it does not apply:
- keep what they support, remove or correct what they do not;
- use their exact numbers, names and units, as written;
- add no facts and use no outside knowledge;
- keep the claim's meaning: a reason stays a reason, an outcome stays an outcome, a decision stays a decision. Do not swap it for a different, weaker fact that happens to be supported.
When the evidence supports the claim's meaning but not its wording (for example "64 MiB" where the diff says `64 * 1024`), reword it to match the evidence; that is what this step is for. Return null only when the evidence does not support the meaning itself."#;

fn repair_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["revisions"],
        "properties": {
            "revisions": {"type": "array", "items": {
                "type": "object",
                "additionalProperties": false,
                "required": ["index", "claim"],
                "properties": {
                    "index": {"type": "integer"},
                    "claim": {"type": ["string", "null"]}
                }
            }}
        }
    })
}

/// A claim as the checker and the repair step see it.
#[derive(Debug, Clone)]
struct ToCheck {
    text: String,
    /// The durable parts of the events it cites.
    evidence: Vec<Value>,
    /// Whether the diff counts as evidence for it: only when it cites an
    /// edit or a commit, so a claim cannot borrow support from code it never
    /// pointed at.
    diff_applies: bool,
}

/// True when a claim cites an edit or a commit (an attempt that touched
/// paths), which makes the diff part of its evidence.
fn cites_code(c: &Claim, by_id: &HashMap<&str, &EvidenceEvent>) -> bool {
    c.evidence.iter().any(|e| {
        by_id
            .get(e.as_str())
            .is_some_and(|ev| ev.kind == EventType::Attempt && !ev.paths.is_empty())
    })
}

/// The request body for the checker or the repair step. The diff is sent
/// only when at least one claim may use it.
fn claims_body(items: Vec<Value>, any_diff: bool, bundle: &Value) -> Value {
    if any_diff {
        json!({"claims": items, "diff": bundle["diff"]})
    } else {
        json!({"claims": items})
    }
}

/// One model call that rules on `claims` (text plus cited evidence).
/// A claim the answer does not cover counts as "partly".
fn judge(
    cfg: &RepoConfig,
    gate: &mut Gate,
    claims: &[ToCheck],
    cited: &BTreeSet<String>,
    diff: &[FileDiff],
    ignored: &dyn Fn(&str) -> bool,
    purpose: &'static str,
) -> Result<Vec<(&'static str, String)>> {
    let excerpts = cfg.egress.send_excerpts && cfg.collection.excerpts == "on";
    // The commit's diff is durable too: it lives in Git. It is what proves
    // claims about what the code does, for claims that cite an edit.
    let any_diff = claims.iter().any(|c| c.diff_applies);
    let (bundle, gated) = gated_bundle(cfg, &[], if any_diff { diff } else { &[] }, ignored);
    let items: Vec<Value> = claims
        .iter()
        .enumerate()
        .map(|(i, c)| {
            json!({"index": i, "claim": c.text, "evidence": c.evidence, "diff_applies": c.diff_applies})
        })
        .collect();
    let req = Request {
        system: VALIDATE.into(),
        user: serde_json::to_string_pretty(&claims_body(items, any_diff, &bundle))?,
        schema: verdict_schema(),
        max_tokens: 8_000,
    };
    let sent = Sent {
        purpose,
        event_ids: cited.iter().cloned().collect(),
        hunks: gated.hunks,
        excerpts,
    };
    let v = gate.call(cfg.validation_model(), &req, sent)?;
    Ok(match_verdicts(claims, &v))
}

/// Attach each verdict to the claim whose text it echoes, not to the index
/// it names: models swap indexes (a "no" for one claim landing on its
/// neighbor). A verdict whose text matches no claim is ignored, and a claim
/// that gets two different verdicts is not judged; both stay "partly".
fn match_verdicts(claims: &[ToCheck], v: &Value) -> Vec<(&'static str, String)> {
    let norm = |t: &str| {
        t.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim_end_matches(['.', ';'])
            .to_lowercase()
    };
    let keys: Vec<String> = claims.iter().map(|c| norm(&c.text)).collect();
    let mut seen: Vec<Option<(&'static str, String)>> = vec![None; claims.len()];
    let mut conflict = vec![false; claims.len()];
    for item in v["verdicts"].as_array().cloned().unwrap_or_default() {
        let (Some(text), Some(sup)) = (item["claim"].as_str(), item["supported"].as_str()) else {
            continue;
        };
        let key = norm(text);
        // Prefer the named index when its text matches (claims can repeat).
        let named = item["index"]
            .as_u64()
            .map(|i| i as usize)
            .filter(|i| keys.get(*i) == Some(&key));
        let Some(i) =
            named.or_else(|| (0..keys.len()).find(|i| keys[*i] == key && seen[*i].is_none()))
        else {
            continue;
        };
        let sup = match sup {
            "no" => "no",
            "partly" => "partly",
            _ => "yes",
        };
        let note = item["note"].as_str().unwrap_or_default().to_owned();
        match &seen[i] {
            Some((prev, _)) if *prev != sup => conflict[i] = true,
            Some(_) => {}
            None => seen[i] = Some((sup, note)),
        }
    }
    seen.into_iter()
        .zip(conflict)
        .map(|(s, c)| match s {
            Some(v) if !c => v,
            _ => ("partly", String::new()),
        })
        .collect()
}

/// The validation pass (PRD §9.5): durable parts only. Unsupported claims are
/// dropped; partly supported ones get one repair attempt, then become
/// hypotheses if the rewrite does not pass either.
pub fn validate(
    cfg: &RepoConfig,
    gate: &mut Gate,
    s: Story,
    events: &[EvidenceEvent],
    diff: &[FileDiff],
    ignored: &dyn Fn(&str) -> bool,
) -> Result<Story> {
    validate_logged(cfg, gate, s, events, diff, ignored).map(|(s, _)| s)
}

/// [`validate`], also returning the claims the repair step rewrote.
pub fn validate_logged(
    cfg: &RepoConfig,
    gate: &mut Gate,
    mut s: Story,
    events: &[EvidenceEvent],
    diff: &[FileDiff],
    ignored: &dyn Fn(&str) -> bool,
) -> Result<(Story, ValidationLog)> {
    let by_id: HashMap<&str, &EvidenceEvent> = events.iter().map(|e| (e.id.as_str(), e)).collect();
    let excerpts = cfg.egress.send_excerpts && cfg.collection.excerpts == "on";
    let sections: [&mut Vec<Claim>; 4] = [
        &mut s.decisions,
        &mut s.rejected,
        &mut s.discarded,
        &mut s.evidence_summary,
    ];
    // Index every authoritative claim, with its cited evidence (durable
    // parts only: no external pointers).
    let mut items: Vec<(usize, usize)> = Vec::new();
    let mut claims: Vec<ToCheck> = Vec::new();
    let mut cited = BTreeSet::new();
    for (si, list) in sections.iter().enumerate() {
        for (ci, c) in list.iter().enumerate() {
            let evs: Vec<Value> = c
                .evidence
                .iter()
                .filter_map(|e| by_id.get(e.as_str()))
                .map(|e| {
                    cited.insert(e.id.clone());
                    event_json(e, excerpts)
                })
                .collect();
            claims.push(ToCheck {
                text: c.claim.clone(),
                evidence: evs,
                diff_applies: cites_code(c, &by_id),
            });
            items.push((si, ci));
        }
    }
    // With no validation model, claims stand on the local rules and the
    // mechanical checks below. With one, a claim it does not rule on is not
    // supported (see `judge`).
    let has_model = !items.is_empty() && cfg.validation_model().provider != "none";
    let mut verdict: Vec<(&str, String)> = if has_model {
        judge(cfg, gate, &claims, &cited, diff, ignored, "validation")?
    } else {
        vec![("yes", String::new()); items.len()]
    };
    // Counts that disagree with the test facts sink the claim. Counts that
    // agree prove only the counts: the rest of the claim ("including new
    // tests for UTC-5") still needs the model's yes.
    for (i, (si, ci)) in items.iter().enumerate() {
        if test_check(&sections[*si][*ci], &by_id) == Some(false) {
            verdict[i].0 = "no";
        }
    }

    let judged: Vec<Judged> = claims
        .iter()
        .zip(&verdict)
        .map(|(c, (v, note))| Judged {
            claim: c.text.clone(),
            verdict: v,
            note: note.clone(),
        })
        .collect();

    // Repair: a partly supported claim is often a true claim worded beyond
    // its evidence ("64 MiB" for `64 * 1024`). Ask once for a rewrite that
    // sticks to the evidence, then judge the rewrite like any other claim.
    let mut repairs = Vec::new();
    let partly: Vec<usize> = (0..items.len())
        .filter(|i| verdict[*i].0 == "partly")
        .collect();
    if has_model && !partly.is_empty() {
        let any_diff = partly.iter().any(|i| claims[*i].diff_applies);
        let (bundle, gated) = gated_bundle(cfg, &[], if any_diff { diff } else { &[] }, ignored);
        let asks: Vec<Value> = partly
            .iter()
            .enumerate()
            .map(|(k, i)| {
                let c = &claims[*i];
                json!({"index": k, "claim": c.text, "evidence": c.evidence,
                       "diff_applies": c.diff_applies, "note": verdict[*i].1})
            })
            .collect();
        let req = Request {
            system: REPAIR.into(),
            user: serde_json::to_string_pretty(&claims_body(asks, any_diff, &bundle))?,
            schema: repair_schema(),
            max_tokens: 8_000,
        };
        let sent = Sent {
            purpose: "repair",
            event_ids: cited.iter().cloned().collect(),
            hunks: gated.hunks,
            excerpts,
        };
        let v = gate.call(&cfg.model, &req, sent)?;
        // (index into items, rewritten text)
        let mut rewrites: Vec<(usize, String)> = Vec::new();
        for r in v["revisions"].as_array().cloned().unwrap_or_default() {
            let (Some(k), Some(text)) = (r["index"].as_u64(), r["claim"].as_str()) else {
                continue;
            };
            let Some(&i) = partly.get(k as usize) else {
                continue;
            };
            let text = strip_ids(&scrub::scrub(text.trim()).0);
            if !text.is_empty() && text != claims[i].text && !rewrites.iter().any(|(j, _)| *j == i)
            {
                rewrites.push((i, text));
            }
        }
        if !rewrites.is_empty() {
            let again: Vec<ToCheck> = rewrites
                .iter()
                .map(|(i, t)| ToCheck {
                    text: t.clone(),
                    ..claims[*i].clone()
                })
                .collect();
            let second = judge(cfg, gate, &again, &cited, diff, ignored, "revalidation")?;
            for ((i, text), (sup, _)) in rewrites.into_iter().zip(second) {
                let (si, ci) = items[i];
                let rewritten = Claim {
                    claim: text.clone(),
                    ..sections[si][ci].clone()
                };
                if sup == "yes" && test_check(&rewritten, &by_id) != Some(false) {
                    repairs.push(Repair {
                        from: claims[i].text.clone(),
                        to: text,
                    });
                    sections[si][ci] = rewritten;
                    verdict[i].0 = "yes";
                }
            }
        }
    }

    // Apply, back to front so indices stay valid.
    let mut moved = Vec::new();
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|a, b| items[*b].cmp(&items[*a]));
    for i in order {
        let (si, ci) = items[i];
        match verdict[i].0 {
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
    Ok((s, ValidationLog { judged, repairs }))
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

    fn texts(t: &[&str]) -> Vec<ToCheck> {
        t.iter()
            .map(|c| ToCheck {
                text: c.to_string(),
                evidence: Vec::new(),
                diff_applies: false,
            })
            .collect()
    }

    #[test]
    fn verdicts_attach_by_echoed_text_not_index() {
        let claims = texts(&["The suite passed 50 of 50 runs", "A 60s lease was dropped"]);
        // The model swapped the indexes but echoed the right text.
        let v = json!({"verdicts": [
            {"index": 0, "claim": "A 60s lease was dropped", "supported": "no", "note": "30s, not 60s"},
            {"index": 1, "claim": "The suite passed 50 of 50 runs.", "supported": "yes", "note": "50/50"}
        ]});
        let out = match_verdicts(&claims, &v);
        assert_eq!(out[0], ("yes", "50/50".to_owned()));
        assert_eq!(out[1], ("no", "30s, not 60s".to_owned()));
    }

    #[test]
    fn unmatched_or_conflicting_verdicts_leave_claims_unjudged() {
        let claims = texts(&["A", "B"]);
        let v = json!({"verdicts": [
            {"index": 0, "claim": "something else", "supported": "yes", "note": ""},
            {"index": 1, "claim": "B", "supported": "yes", "note": ""},
            {"index": 1, "claim": "B", "supported": "no", "note": ""}
        ]});
        let out = match_verdicts(&claims, &v);
        assert_eq!(out[0].0, "partly");
        assert_eq!(out[1].0, "partly");
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
    fn summaries_lose_evidence_ids() {
        assert_eq!(
            strip_ids("Reuse one key [ev_01ab, ev_02cd]."),
            "Reuse one key."
        );
        assert_eq!(strip_ids("Reuse one key (ev_01ab)"), "Reuse one key");
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
