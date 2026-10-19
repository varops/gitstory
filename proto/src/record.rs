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
}

/// A link suggested by landing reconciliation. Never applied on its own.
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

/// What one outbound model request carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressCall {
    /// `synthesis`, `validation` or `reconciliation`.
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
