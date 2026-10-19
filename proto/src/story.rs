//! The `gitstory/v1` story schema (PRD §7.2–7.5).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const STORY_SCHEMA: &str = "gitstory/v1";

/// How a claim is known (PRD §7.2). Inferred claims are never authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaimType {
    Observed,
    Stated,
    Inferred,
}

/// What the landed code says about a claim (PRD §7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaimStatus {
    Implemented,
    Rejected,
    Discarded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub claim: String,
    #[serde(rename = "type")]
    pub kind: ClaimType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ClaimStatus>,
    /// Evidence event ids. A claim with none is invalid and is dropped.
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StoryStatus {
    Current,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    /// `unreviewed`, `reviewed` or `flagged`.
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Story {
    pub story_id: String,
    pub schema: String,
    pub repo: String,
    pub commit: String,
    pub work_unit: String,
    /// Derived from accepted links, never guessed (PRD §8.9).
    pub status: StoryStatus,
    #[serde(default)]
    pub supersedes: Vec<String>,
    #[serde(default)]
    pub related: Vec<String>,
    pub summary: String,
    pub goal: Claim,
    /// Implemented claims only.
    #[serde(default)]
    pub decisions: Vec<Claim>,
    #[serde(default)]
    pub rejected: Vec<Claim>,
    /// Hidden from `--brief`; never presented as a decision.
    #[serde(default)]
    pub discarded: Vec<Claim>,
    #[serde(default)]
    pub evidence_summary: Vec<Claim>,
    /// Inferred claims only; excluded from `--brief`.
    #[serde(default)]
    pub hypotheses: Vec<Claim>,
    #[serde(default)]
    pub limitations: Vec<String>,
    #[serde(default)]
    pub unknowns: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub links: BTreeMap<String, String>,
    pub review: Review,
    /// Egress manifest id for this story.
    pub egress: String,
}

impl Story {
    /// Structural invariants that hold for every stored story (PRD §8.3,
    /// §8.5, §8.6). Returns the violations found.
    pub fn violations(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.schema != STORY_SCHEMA {
            out.push(format!(
                "schema is {:?}, expected {STORY_SCHEMA:?}",
                self.schema
            ));
        }
        let all = std::iter::once(&self.goal)
            .chain(&self.decisions)
            .chain(&self.rejected)
            .chain(&self.discarded)
            .chain(&self.evidence_summary)
            .chain(&self.hypotheses);
        for c in all {
            if c.evidence.is_empty() {
                out.push(format!("uncited claim: {:?}", c.claim));
            }
        }
        for c in &self.decisions {
            if c.kind == ClaimType::Inferred {
                out.push(format!("inferred claim in decisions: {:?}", c.claim));
            }
            if c.status != Some(ClaimStatus::Implemented) {
                out.push(format!("decision not implemented: {:?}", c.claim));
            }
        }
        for c in &self.hypotheses {
            if c.kind != ClaimType::Inferred {
                out.push(format!("non-inferred claim in hypotheses: {:?}", c.claim));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example from PRD §7.4, verbatim apart from the elided links.
    const PRD_EXAMPLE: &str = r#"{
      "story_id": "st_8f2c",
      "schema": "gitstory/v1",
      "repo": "org/project",
      "commit": "a83f2c1",
      "work_unit": "cloop:TCK-482",
      "status": "current",
      "supersedes": ["st_5b10"],
      "related": ["st_3a77"],
      "summary": "Replaced Redis locking with optimistic row versioning to remove retry deadlocks.",
      "goal": { "claim": "Stop duplicate job execution under retries", "type": "stated", "evidence": ["ev_02"] },
      "decisions": [
        { "claim": "Replaced the Redis lock with optimistic row versioning", "type": "observed", "status": "implemented", "evidence": ["ev_12", "ev_19"] },
        { "claim": "The Redis lock caused deadlocks when jobs retried", "type": "stated", "status": "implemented", "evidence": ["ev_15"] }
      ],
      "rejected": [
        { "claim": "Lease timeout on the lock; tests still flaked", "type": "observed", "status": "rejected", "evidence": ["ev_17", "ev_18"] }
      ],
      "discarded": [],
      "evidence_summary": [
        { "claim": "Retry suite passed 50 of 50 runs", "type": "observed", "evidence": ["ev_21"] }
      ],
      "hypotheses": [
        { "claim": "Row versioning was likely chosen to avoid a new dependency", "type": "inferred", "evidence": ["ev_12"] }
      ],
      "limitations": [],
      "unknowns": ["Why the 5s retry window was chosen is not in the evidence"],
      "paths": ["src/jobs/lock.py", "src/jobs/retry.py"],
      "links": { "pr": "https://example.com/pr/1", "ticket": "TCK-482" },
      "review": { "status": "unreviewed" },
      "egress": "egr_91c4"
    }"#;

    #[test]
    fn prd_example_parses_and_is_valid() {
        let s: Story = serde_json::from_str(PRD_EXAMPLE).unwrap();
        assert_eq!(s.decisions.len(), 2);
        assert!(s.violations().is_empty(), "{:?}", s.violations());
        let again: Story = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(again, s);
    }

    #[test]
    fn invariants_catch_misplaced_claims() {
        let mut s: Story = serde_json::from_str(PRD_EXAMPLE).unwrap();
        s.decisions.push(s.hypotheses[0].clone());
        s.hypotheses[0].kind = ClaimType::Stated;
        s.rejected[0].evidence.clear();
        let v = s.violations();
        assert!(
            v.iter()
                .any(|x| x.starts_with("inferred claim in decisions"))
        );
        assert!(
            v.iter()
                .any(|x| x.starts_with("non-inferred claim in hypotheses"))
        );
        assert!(v.iter().any(|x| x.starts_with("uncited claim")));
    }
}
