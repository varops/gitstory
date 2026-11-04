//! Tools for the M0 quality study (PRD §13). Hidden from `--help`: they
//! measure GitStory, they are not part of using it.
//!
//! - `study audit`: plant false claims next to true ones, run the real
//!   validator, and count what it catches.
//! - `land --study-baseline DIR`: the control arm. The same evidence and
//!   diff go to the same model in one plain call, through the same egress
//!   gate, and both write-ups land in a blind A/B rating sheet.
//!
//! Every rate is printed with its 95% Wilson interval. Overlapping
//! intervals mean the runs do not tell the rates apart.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use gitstory_proto::evidence::{Durable, EventType, EvidenceEvent, SourceType, TestFact};
use gitstory_proto::id;
use gitstory_proto::record::EgressManifest;
use gitstory_proto::story::{
    Claim, ClaimStatus, ClaimType, Review, STORY_SCHEMA, Stage, Story, StoryStatus,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::cmd::Out;
use crate::config::RepoConfig;
use crate::error::Result;
use crate::git::FileDiff;
use crate::model::{Gate, Request};
use crate::workspace::Workspace;
use crate::{err, scrub, synth};

// -- audit -------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Corpus {
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    events: Vec<CaseEvent>,
    #[serde(default)]
    diff: Vec<CaseDiff>,
    claims: Vec<CaseClaim>,
}

#[derive(Debug, Deserialize)]
struct CaseEvent {
    id: String,
    #[serde(rename = "type")]
    kind: EventType,
    fact: String,
    #[serde(default)]
    excerpt: Option<String>,
    #[serde(default)]
    test: Option<TestFact>,
    #[serde(default)]
    paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CaseDiff {
    path: String,
    diff: String,
}

#[derive(Debug, Deserialize)]
struct CaseClaim {
    #[serde(default = "decisions")]
    section: String,
    claim: String,
    #[serde(rename = "type", default = "stated")]
    kind: ClaimType,
    evidence: Vec<String>,
    /// `supported`, or `planted` for a claim the validator must catch.
    truth: String,
    /// What kind of falsehood was planted, for the breakdown.
    #[serde(default)]
    plant: Option<String>,
}

fn decisions() -> String {
    "decisions".into()
}

fn stated() -> ClaimType {
    ClaimType::Stated
}

/// Where a claim ended up after validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fate {
    Kept,
    /// Rewritten by the repair step to match its evidence, then kept.
    Repaired,
    Hypothesis,
    Dropped,
}

impl Fate {
    fn as_str(self) -> &'static str {
        match self {
            Self::Kept => "kept",
            Self::Repaired => "repaired",
            Self::Hypothesis => "moved_to_hypotheses",
            Self::Dropped => "dropped",
        }
    }
}

fn event(e: &CaseEvent) -> EvidenceEvent {
    EvidenceEvent {
        id: e.id.clone(),
        kind: e.kind,
        work_unit: "study:audit".into(),
        durable: Durable {
            fact: e.fact.clone(),
            test: e.test.clone(),
            excerpt: e.excerpt.clone(),
            source_hash: String::new(),
            source: SourceType::AgentSession,
            at: 0,
            author: "study".into(),
        },
        external: None,
        paths: e.paths.clone(),
    }
}

fn status_for(section: &str) -> Option<ClaimStatus> {
    match section {
        "decisions" => Some(ClaimStatus::Implemented),
        "rejected" => Some(ClaimStatus::Rejected),
        "discarded" => Some(ClaimStatus::Discarded),
        _ => None,
    }
}

fn story_for(case: &Case, events: &[EvidenceEvent]) -> Result<Story> {
    let mut s = Story {
        story_id: id::new_id(id::STORY),
        schema: STORY_SCHEMA.into(),
        repo: "study/audit".into(),
        commit: "0".repeat(40),
        contains: Vec::new(),
        work_unit: "study:audit".into(),
        stage: Stage::Landed,
        version: 1,
        created_at: 0,
        status: StoryStatus::Current,
        supersedes: Vec::new(),
        related: Vec::new(),
        summary: case.name.clone(),
        goal: Claim {
            claim: events
                .first()
                .map(|e| e.durable.fact.clone())
                .unwrap_or_default(),
            kind: ClaimType::Stated,
            status: None,
            evidence: events
                .first()
                .map(|e| vec![e.id.clone()])
                .unwrap_or_default(),
        },
        decisions: Vec::new(),
        rejected: Vec::new(),
        discarded: Vec::new(),
        evidence_summary: Vec::new(),
        hypotheses: Vec::new(),
        limitations: Vec::new(),
        unknowns: Vec::new(),
        paths: Vec::new(),
        links: Default::default(),
        review: Review {
            status: "unreviewed".into(),
        },
        egress: String::new(),
    };
    for c in &case.claims {
        let claim = Claim {
            claim: c.claim.clone(),
            kind: c.kind,
            status: status_for(&c.section),
            evidence: c.evidence.clone(),
        };
        let list = match c.section.as_str() {
            "decisions" => &mut s.decisions,
            "rejected" => &mut s.rejected,
            "discarded" => &mut s.discarded,
            "evidence_summary" => &mut s.evidence_summary,
            other => {
                return Err(err!(
                    Usage,
                    "case {:?}: unknown section {other:?}; use decisions, rejected, discarded or evidence_summary",
                    case.name
                ));
            }
        };
        list.push(claim);
    }
    Ok(s)
}

fn fate(s: &Story, text: &str, repairs: &[synth::Repair]) -> (Fate, Option<String>) {
    if let Some(r) = repairs.iter().find(|r| r.from == text) {
        return (Fate::Repaired, Some(r.to.clone()));
    }
    let has = |l: &[Claim]| l.iter().any(|c| c.claim == text);
    let f =
        if has(&s.decisions) || has(&s.rejected) || has(&s.discarded) || has(&s.evidence_summary) {
            Fate::Kept
        } else if has(&s.hypotheses) {
            Fate::Hypothesis
        } else {
            Fate::Dropped
        };
    (f, None)
}

/// What the audit must show: at least `min_caught` of planted claims
/// caught, at most `max_lost` of true claims lost.
pub struct Targets {
    pub min_caught: f64,
    pub max_lost: f64,
}

/// `git story study audit CORPUS`: run the real rules and validation pass
/// over claims whose truth is known, with the repository's validation model.
pub fn audit(ws: &Workspace, corpus_path: &Path, targets: &Targets) -> Result<Out> {
    let text = fs::read_to_string(corpus_path)
        .map_err(|e| err!(Usage, "cannot read {}: {e}", corpus_path.display()))?;
    let corpus: Corpus = serde_json::from_str(&text).map_err(|e| {
        err!(
            Usage,
            "{} is not a valid corpus: {e}",
            corpus_path.display()
        )
    })?;
    let mut gate = Gate::new(&ws.cfg.egress);
    let mut rows = Vec::new();
    for case in &corpus.cases {
        let events: Vec<EvidenceEvent> = case.events.iter().map(event).collect();
        let diff: Vec<FileDiff> = case
            .diff
            .iter()
            .map(|d| FileDiff {
                path: d.path.clone(),
                old_path: None,
                text: d.diff.clone(),
                old_ranges: Vec::new(),
            })
            .collect();
        let story = story_for(case, &events)?;
        // The same order as landing: local rules first, then validation.
        let (checked, log) = synth::validate_logged(
            &ws.cfg,
            &mut gate,
            synth::normalize(story, &events),
            &events,
            &diff,
            &|_| false,
        )?;
        for c in &case.claims {
            // Claims are scrubbed on the way in; compare like with like.
            let text = scrub::scrub(&c.claim).0;
            let (f, to) = fate(&checked, &text, &log.repairs);
            // Claims the local rules removed never reach the checker.
            let judged = log.judged.iter().find(|j| j.claim == text);
            rows.push(json!({
                "case": case.name,
                "claim": c.claim,
                "truth": c.truth,
                "plant": c.plant,
                "fate": f.as_str(),
                "repaired_to": to,
                "verdict": judged.map(|j| j.verdict),
                "note": judged.map(|j| j.note.clone()),
            }));
        }
    }
    let report = score(&rows, targets);
    let manifest = EgressManifest {
        id: id::new_id(id::EGRESS),
        calls: gate.calls,
    };
    let human = audit_text(&report, &rows, &ws.cfg);
    Ok(Out::new(
        json!({"report": report, "claims": rows, "egress": manifest}),
        human,
    ))
}

/// A planted claim is caught unless it survived as written. A repaired one
/// counts as caught (its wording changed) but is listed for a human to check
/// that the rewrite really dropped the falsehood. A true claim is lost when
/// it ends in hypotheses or is dropped; repaired true claims are kept.
fn score(rows: &[Value], targets: &Targets) -> Value {
    let planted: Vec<&Value> = rows.iter().filter(|r| r["truth"] == "planted").collect();
    let supported: Vec<&Value> = rows.iter().filter(|r| r["truth"] == "supported").collect();
    let caught = planted.iter().filter(|r| r["fate"] != "kept").count();
    let dropped = planted.iter().filter(|r| r["fate"] == "dropped").count();
    let lost_fate = |r: &&&Value| r["fate"] == "moved_to_hypotheses" || r["fate"] == "dropped";
    let lost = supported.iter().filter(lost_fate).count();
    let repaired = rows.iter().filter(|r| r["fate"] == "repaired").count();
    let caught_rate = rate(caught, planted.len());
    let lost_rate = rate(lost, supported.len());
    let caught_target = target(&caught_rate, targets.min_caught, true);
    let lost_target = target(&lost_rate, targets.max_lost, false);
    // The weaker of the two: shown only when both are shown.
    let overall = if caught_target == "missed" || lost_target == "missed" {
        "missed"
    } else if caught_target == "shown" && lost_target == "shown" {
        "shown"
    } else {
        "not_yet_shown"
    };
    let mut by_plant: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for r in &planted {
        let k = r["plant"].as_str().unwrap_or("unlabeled").to_owned();
        let e = by_plant.entry(k).or_default();
        e.1 += 1;
        if r["fate"] != "kept" {
            e.0 += 1;
        }
    }
    json!({
        "planted_caught": caught_rate,
        "planted_dropped": rate(dropped, planted.len()),
        "supported_lost": lost_rate,
        "repaired": repaired,
        "targets": {
            "min_caught": targets.min_caught,
            "max_lost": targets.max_lost,
            "caught": caught_target,
            "lost": lost_target,
            "overall": overall,
        },
        "by_plant": by_plant
            .into_iter()
            .map(|(k, (c, n))| (k, rate(c, n)))
            .collect::<BTreeMap<_, _>>(),
    })
}

/// `shown` when the whole interval is on the right side of the target,
/// `missed` when the rate itself is on the wrong side, `not_yet_shown` when
/// the rate meets it but the interval is too wide to be sure.
fn target(r: &Value, bound: f64, at_least: bool) -> &'static str {
    let n = r["n"].as_u64().unwrap_or(0) as f64;
    let p = if n == 0.0 {
        if at_least { 0.0 } else { 1.0 }
    } else {
        r["k"].as_u64().unwrap_or(0) as f64 / n
    };
    let (lo, hi) = (
        r["low"].as_f64().unwrap_or(0.0),
        r["high"].as_f64().unwrap_or(1.0),
    );
    match at_least {
        true if lo >= bound => "shown",
        true if p < bound => "missed",
        false if hi <= bound => "shown",
        false if p > bound => "missed",
        _ => "not_yet_shown",
    }
}

fn rate(k: usize, n: usize) -> Value {
    let (lo, hi) = wilson(k, n);
    json!({"k": k, "n": n, "low": lo, "high": hi})
}

/// 95% Wilson score interval for k successes in n trials.
pub fn wilson(k: usize, n: usize) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let z = 1.959_963_984_540_054_f64;
    let (k, n) = (k as f64, n as f64);
    let p = k / n;
    let denom = 1.0 + z * z / n;
    let center = (p + z * z / (2.0 * n)) / denom;
    let half = z * ((p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt()) / denom;
    ((center - half).max(0.0), (center + half).min(1.0))
}

fn fmt_rate(v: &Value) -> String {
    format!(
        "{}/{} [{:.2}, {:.2}]",
        v["k"],
        v["n"],
        v["low"].as_f64().unwrap_or(0.0),
        v["high"].as_f64().unwrap_or(1.0)
    )
}

fn audit_text(report: &Value, rows: &[Value], cfg: &RepoConfig) -> String {
    let m = cfg.validation_model();
    let mut t = format!(
        "validator audit · model {} ({})\n\n\
         planted claims caught   {}   target ≥ {:.0}%: {}\n\
         planted claims dropped  {}\n\
         true claims lost        {}   target ≤ {:.0}%: {}\n\
         claims repaired         {}\n",
        m.name,
        m.provider,
        fmt_rate(&report["planted_caught"]),
        report["targets"]["min_caught"].as_f64().unwrap_or(0.0) * 100.0,
        report["targets"]["caught"]
            .as_str()
            .unwrap_or_default()
            .replace('_', " "),
        fmt_rate(&report["planted_dropped"]),
        fmt_rate(&report["supported_lost"]),
        report["targets"]["max_lost"].as_f64().unwrap_or(0.0) * 100.0,
        report["targets"]["lost"]
            .as_str()
            .unwrap_or_default()
            .replace('_', " "),
        report["repaired"],
    );
    if let Some(by) = report["by_plant"].as_object() {
        t.push_str("\ncaught by kind of plant\n");
        for (k, v) in by {
            t.push_str(&format!("  {k:<22} {}\n", fmt_rate(v)));
        }
    }
    let repaired: Vec<&Value> = rows.iter().filter(|r| r["fate"] == "repaired").collect();
    if !repaired.is_empty() {
        t.push_str("\nrepaired (check that each rewrite is true)\n");
        for r in repaired {
            t.push_str(&format!(
                "  [{}] {}\n      was: {}\n      now: {}\n",
                r["case"].as_str().unwrap_or_default(),
                r["plant"].as_str().unwrap_or("true claim"),
                r["claim"].as_str().unwrap_or_default(),
                r["repaired_to"].as_str().unwrap_or_default(),
            ));
        }
    }
    let misses: Vec<&Value> = rows
        .iter()
        .filter(|r| {
            (r["truth"] == "planted" && r["fate"] == "kept")
                || (r["truth"] == "supported"
                    && (r["fate"] == "moved_to_hypotheses" || r["fate"] == "dropped"))
        })
        .collect();
    if !misses.is_empty() {
        t.push_str("\nmisses\n");
        for r in misses {
            t.push_str(&format!(
                "  [{}] {} → {}: {}\n",
                r["case"].as_str().unwrap_or_default(),
                r["plant"].as_str().unwrap_or("true claim"),
                r["fate"].as_str().unwrap_or_default(),
                r["claim"].as_str().unwrap_or_default(),
            ));
            if let Some(note) = r["note"].as_str().filter(|n| !n.is_empty()) {
                t.push_str(&format!(
                    "      checker said {}: {note}\n",
                    r["verdict"].as_str().unwrap_or_default()
                ));
            }
        }
    }
    t.push_str(&format!(
        "\ntargets: {}. 95% Wilson intervals; \"not yet shown\" means the rate meets the target but the interval is too wide to be sure. Overlapping intervals do not tell two rates apart.",
        report["targets"]["overall"]
            .as_str()
            .unwrap_or_default()
            .replace('_', " ")
    ));
    t
}

// -- control arm ---------------------------------------------------------------

const BASELINE: &str = "You explain code changes for future engineers and coding agents who need to know why the code is the way it is. You get evidence collected while the work happened and the diff of the change. Write a short explanation of what changed and why. Use only the evidence and the diff.";

/// The control arm: the same evidence and diff as landing (no drafts, no
/// rules, no schema beyond one text field, no validation), through the same
/// gate, so the call is policy-checked and lands in the same manifest.
pub fn baseline(
    cfg: &RepoConfig,
    gate: &mut Gate,
    input: &synth::Input,
    ignored: &dyn Fn(&str) -> bool,
) -> Result<String> {
    let (bundle, mut sent) = synth::gated_bundle(cfg, input.events, input.diff, ignored);
    sent.purpose = "study_baseline";
    let user = json!({
        "commit": {"sha": input.commit, "subject": scrub::scrub(input.commit_subject).0},
        "events": bundle["events"],
        "diff": bundle["diff"],
    });
    let req = Request {
        system: BASELINE.into(),
        user: serde_json::to_string_pretty(&user)?,
        schema: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["explanation"],
            "properties": {"explanation": {"type": "string"}}
        }),
        max_tokens: 4_000,
    };
    let v = gate.call(&cfg.model, &req, sent)?;
    let text = v["explanation"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_owned();
    if text.is_empty() {
        return Err(err!(
            SynthesisInvalid,
            "the baseline call returned no explanation"
        ));
    }
    Ok(scrub::scrub(&text).0)
}

/// The story as a rater sees it: the claims only, with no ids, types or
/// section names that would give away which arm wrote it.
fn claim_texts(l: &[Claim]) -> Vec<&str> {
    l.iter().map(|c| c.claim.as_str()).collect()
}

fn blind(s: &Story) -> String {
    let mut m = format!("{}\n\n**Goal:** {}\n", s.summary, s.goal.claim);
    let mut list = |title: &str, items: Vec<&str>| {
        if !items.is_empty() {
            m.push_str(&format!("\n**{title}**\n\n"));
            for i in items {
                m.push_str(&format!("- {i}\n"));
            }
        }
    };
    list("What changed and why", claim_texts(&s.decisions));
    list("Tried and dropped", claim_texts(&s.rejected));
    list("Checks", claim_texts(&s.evidence_summary));
    list(
        "Not explained by the evidence",
        s.unknowns.iter().map(String::as_str).collect(),
    );
    m
}

/// Write `DIR/<commit>.md` with both write-ups as Version A and B, and add
/// which is which to `DIR/key.jsonl`. The order comes from a hash of the
/// story id, so raters cannot predict it.
pub fn write_sheet(dir: &Path, story: &Story, subject: &str, baseline: &str) -> Result<PathBuf> {
    fs::create_dir_all(dir).map_err(|e| err!(Usage, "cannot create {}: {e}", dir.display()))?;
    let short = &story.commit[..story.commit.len().min(12)];
    let story_first = blake3::hash(story.story_id.as_bytes()).as_bytes()[0] & 1 == 0;
    let (a, b, a_arm, b_arm) = if story_first {
        (blind(story), baseline.to_owned(), "gitstory", "baseline")
    } else {
        (baseline.to_owned(), blind(story), "baseline", "gitstory")
    };
    let sheet = format!(
        "# {subject}\n\ncommit `{short}`\n\n\
         Read the change first (`git show {short}`). Then rate each version: \
         accuracy 1–5 (is every statement true of the change and its history?), \
         usefulness 1–5 (would it help you or an agent change this code safely?), \
         and which one you would rather have.\n\n\
         ## Version A\n\n{a}\n\n## Version B\n\n{b}\n"
    );
    let path = dir.join(format!("{short}.md"));
    fs::write(&path, sheet)?;
    let mut key = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("key.jsonl"))?;
    writeln!(
        key,
        "{}",
        json!({
            "commit": short,
            "work_unit": story.work_unit,
            "story_id": story.story_id,
            "egress": story.egress,
            "a": a_arm,
            "b": b_arm,
        })
    )?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wilson_matches_known_values() {
        let (lo, hi) = wilson(5, 5);
        assert!((lo - 0.566).abs() < 0.001 && hi == 1.0, "{lo} {hi}");
        let (lo, hi) = wilson(0, 5);
        assert!(lo == 0.0 && (hi - 0.434).abs() < 0.001, "{lo} {hi}");
        assert_eq!(wilson(0, 0), (0.0, 1.0));
    }

    #[test]
    fn blind_view_has_no_ids_or_types() {
        let c = |t: &str| Claim {
            claim: t.into(),
            kind: ClaimType::Stated,
            status: None,
            evidence: vec!["ev_123".into()],
        };
        let s = Story {
            story_id: "st_1".into(),
            schema: STORY_SCHEMA.into(),
            repo: "r".into(),
            commit: "abc".into(),
            contains: vec![],
            work_unit: "w".into(),
            stage: Stage::Landed,
            version: 1,
            created_at: 0,
            status: StoryStatus::Current,
            supersedes: vec![],
            related: vec![],
            summary: "Did a thing".into(),
            goal: c("Goal"),
            decisions: vec![c("Used X because Y")],
            rejected: vec![],
            discarded: vec![],
            evidence_summary: vec![],
            hypotheses: vec![c("MAYBE")],
            limitations: vec![],
            unknowns: vec!["Why 5s".into()],
            paths: vec![],
            links: Default::default(),
            review: Review {
                status: "unreviewed".into(),
            },
            egress: String::new(),
        };
        let b = blind(&s);
        assert!(b.contains("Used X because Y") && b.contains("Why 5s"));
        assert!(!b.contains("ev_") && !b.contains("stated") && !b.contains("MAYBE"));
        assert!(!b.contains("st_1"));
    }
}
