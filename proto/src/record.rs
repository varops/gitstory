//! Store records besides stories and events: links, link proposals and
//! egress manifests (PRD §7.5, §9.3).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
    /// `from`'s decisions replace `to`'s, for the same concern.
    Supersedes,
    /// Useful context, no replacement.
    Related,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkOp {
    Add,
    Remove,
}

/// One entry of the append-only link log. Removing a link is a new entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkEntry {
    pub op: LinkOp,
    pub kind: LinkKind,
    pub from: String,
    pub to: String,
    /// Unix milliseconds.
    pub at: i64,
    pub author: String,
    /// The proposal this entry accepted, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProposalStatus {
    Open,
    Accepted,
    Rejected,
    /// Accepted, then its link was removed.
    Withdrawn,
}

impl ProposalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Withdrawn => "withdrawn",
        }
    }

    /// The legal moves: `open` to `accepted` or `rejected`, and `accepted`
    /// to `withdrawn`. Staying put is always allowed, so a repeated command
    /// is a no-op.
    pub fn can_become(self, next: Self) -> bool {
        use ProposalStatus::*;
        self == next
            || matches!(
                (self, next),
                (Open, Accepted | Rejected) | (Accepted, Withdrawn)
            )
    }

    /// Move to `next`, or say why that move is not allowed.
    pub fn transition(self, next: Self) -> Result<Self, String> {
        if self.can_become(next) {
            Ok(next)
        } else {
            Err(format!(
                "a proposal that is {} cannot become {}",
                self.as_str(),
                next.as_str()
            ))
        }
    }
}

/// A link suggested by landing reconciliation. Never applied on its own.
///
/// Two machines deciding the same proposal at the same time on a shared S3
/// store can still race: the store has no conditional writes for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkProposal {
    pub id: String,
    pub kind: LinkKind,
    pub from: String,
    pub to: String,
    /// Why it was proposed, e.g. the blamed lines.
    pub reason: String,
    pub status: ProposalStatus,
    pub at: i64,
}

/// Why a model call was made. Stored as its string in
/// [`EgressCall::purpose`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Building a story (a draft, or a landing with no drafts).
    Synthesis,
    /// Landing: reconciling drafts against the landed diff.
    Reconciliation,
    /// Checking claims against their evidence.
    Validation,
    /// Rewriting partly supported claims from their own evidence.
    Repair,
    /// Checking the repaired claims.
    Revalidation,
    /// The M0 study's control arm (`land --study-baseline`).
    StudyBaseline,
}

impl Purpose {
    pub const ALL: &'static [Purpose] = &[
        Self::Synthesis,
        Self::Reconciliation,
        Self::Validation,
        Self::Repair,
        Self::Revalidation,
        Self::StudyBaseline,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Synthesis => "synthesis",
            Self::Reconciliation => "reconciliation",
            Self::Validation => "validation",
            Self::Repair => "repair",
            Self::Revalidation => "revalidation",
            Self::StudyBaseline => "study_baseline",
        }
    }
}

/// What one outbound model request carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressCall {
    /// A [`Purpose`]: `synthesis`, `reconciliation`, `validation`,
    /// `repair`, `revalidation` or `study_baseline`. Kept as a string so
    /// manifests from newer versions still load.
    pub purpose: String,
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    /// Whether the endpoint is local to this machine.
    pub local: bool,
    pub at: i64,
    pub event_ids: Vec<String>,
    /// Diff hunks sent, as `path` plus a hash of the hunk text.
    #[serde(default)]
    pub hunks: Vec<HunkRef>,
    /// Whether event excerpts were included.
    pub excerpts: bool,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HunkRef {
    pub path: String,
    pub hash: String,
}

/// The egress record for one story: every request made to build it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EgressManifest {
    pub id: String,
    #[serde(default)]
    pub calls: Vec<EgressCall>,
}

#[cfg(test)]
mod tests {
    use super::ProposalStatus::*;

    #[test]
    fn proposal_moves() {
        assert!(Open.can_become(Accepted) && Open.can_become(Rejected));
        assert!(Accepted.can_become(Withdrawn) && Accepted.can_become(Accepted));
        assert!(!Accepted.can_become(Open) && !Rejected.can_become(Accepted));
        assert!(!Withdrawn.can_become(Accepted) && !Open.can_become(Withdrawn));
    }
}
