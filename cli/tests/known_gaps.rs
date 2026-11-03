//! Known gaps: behavior we know is wrong, pinned by tests that pass while
//! the gap is open.
//!
//! The rules:
//! - Each test asserts what the code does **today**, and its comment says
//!   what it should do and how to fix it.
//! - When someone fixes the gap, the test fails. That is the signal: flip
//!   the assertion to the correct behavior and move the test next to the
//!   feature it covers. A gap closes only on a real pass.
//! - Never delete a gap test to get green, and never weaken one.
//! - Name them `gap_<what is wrong>`.

mod common;

use std::fs;

use common::Repo;
use serde_json::{Value, json};

fn audit(r: &Repo, corpus: &Value, env: &[(&str, &str)]) -> Value {
    let p = r.base.join("corpus.json");
    fs::write(&p, corpus.to_string()).unwrap();
    let mut c = r.cmd();
    c.args(["study", "audit", p.to_str().unwrap(), "--json"]);
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    v["data"].clone()
}

fn fate(data: &Value, claim: &str) -> String {
    data["claims"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["claim"] == claim)
        .unwrap()["fate"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Gap: a claim the validation model never rules on is kept as if it were
/// supported. `synth::validate` starts every verdict at "yes", so a model
/// that skips a claim, or an answer cut short, lets it through. That breaks
/// invariant 4 (every authoritative claim is supported).
///
/// Should: a missing verdict counts as "partly", so the claim moves to
/// hypotheses. Fix: start the verdicts at "partly" in `synth::validate`.
#[test]
fn gap_unjudged_claim_is_kept_as_supported() {
    let r = Repo::new("gap-verdict");
    let corpus = json!({"cases": [{
        "name": "skip",
        "events": [{"id": "ev_1", "type": "reason", "fact": "reason: x", "excerpt": "Use X because Y."}],
        "claims": [{"claim": "UNSUPPORTED invented reason", "evidence": ["ev_1"],
                    "truth": "planted", "plant": "invented"}]
    }]});
    // The stand-in model rejects this claim when it answers...
    assert_eq!(
        fate(&audit(&r, &corpus, &[]), "UNSUPPORTED invented reason"),
        "dropped"
    );
    // ...but returns no verdicts at all with MOCK_VERDICTS=skip.
    let skipped = audit(&r, &corpus, &[("MOCK_VERDICTS", "skip")]);
    assert_eq!(
        fate(&skipped, "UNSUPPORTED invented reason"),
        "kept", // should be "moved_to_hypotheses"
    );
}

/// Gap: a test claim whose numbers match a test fact is accepted however
/// much else it says. The mechanical check returns "supported" on matching
/// counts and overrides the model, so "64 of 64 passed, including new tests
/// for UTC-5" survives even when the model says no.
///
/// Should: matching counts can reject a claim but never accept one on their
/// own; the model's verdict on the rest of the claim still applies. Fix: in
/// `synth::validate`, let a mechanical `Some(true)` keep the model's verdict
/// and only `Some(false)` override it.
#[test]
fn gap_matching_test_counts_override_the_model() {
    let r = Repo::new("gap-counts");
    let claim = "UNSUPPORTED 64 of 64 passed, including new tests for UTC-5";
    let corpus = json!({"cases": [{
        "name": "counts",
        "events": [{"id": "ev_t", "type": "test", "fact": "test: pytest, 64/64 pass",
                    "test": {"command": "pytest", "passed": 64, "total": 64}}],
        "claims": [{"section": "evidence_summary", "claim": claim, "type": "observed",
                    "evidence": ["ev_t"], "truth": "planted", "plant": "overreach"}]
    }]});
    // The stand-in model says "no" to it; the count check says "yes" and wins.
    assert_eq!(
        fate(&audit(&r, &corpus, &[]), claim),
        "kept", // should be "dropped"
    );
}
