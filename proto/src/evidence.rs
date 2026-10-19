//! Evidence events (PRD §7.1).
//!
//! Every event has a **durable part**, kept with the story forever and enough
//! on its own to support the claims that cite it, and an optional
//! **external part** that may stop resolving. Validation reads durable parts
//! only (PRD §8.4).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    Goal,
    Decision,
    Reason,
    Attempt,
    Abandon,
    Test,
    Constraint,
    Review,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    AgentSession,
    TestRun,
    Review,
    Ticket,
    Git,
    GhfTrace,
    HumanNote,
}

/// A test result as a typed fact, checked mechanically (PRD §9.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestFact {
    pub command: String,
    pub passed: u32,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Durable {
    /// The typed fact, for example `test: pytest tests/retry -k lock, 50/50 pass`.
    pub fact: String,
    /// Structured form of a test fact, when the event is a test.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<TestFact>,
    /// Short scrubbed excerpt, when the fact is a statement. Length-capped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    /// Hash of the source span, to verify the excerpt while the source exists.
    pub source_hash: String,
    pub source: SourceType,
    /// Unix milliseconds.
    pub at: i64,
    pub author: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct External {
    /// Session file and offset, CI run URL, ticket id, PR comment URL.
    pub pointer: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: EventType,
    pub work_unit: String,
    pub durable: Durable,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<External>,
    /// Paths the event touches; under `.storyignore` only the path is kept.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
}
