//! The validation pass (PRD §9.5), driven through `study audit` so each
//! claim's fate is visible.

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

/// A claim the validation model never rules on (skipped, or the answer was
/// cut short) is not supported: it moves to hypotheses.
#[test]
fn unjudged_claim_moves_to_hypotheses() {
    let r = Repo::new("val-unjudged");
    let corpus = json!({"cases": [{
        "name": "skip",
        "events": [{"id": "ev_1", "type": "reason", "fact": "reason: x", "excerpt": "Use X because Y."}],
        "claims": [
            {"claim": "UNSUPPORTED invented reason", "evidence": ["ev_1"], "truth": "planted", "plant": "invented"},
            {"claim": "Used X because Y", "evidence": ["ev_1"], "truth": "supported"}
        ]
    }]});
    // When the model answers, it rules on each claim.
    let answered = audit(&r, &corpus, &[]);
    assert_eq!(fate(&answered, "UNSUPPORTED invented reason"), "dropped");
    assert_eq!(fate(&answered, "Used X because Y"), "kept");
    // When it returns no verdicts, nothing stays authoritative.
    let skipped = audit(&r, &corpus, &[("MOCK_VERDICTS", "skip")]);
    assert_eq!(
        fate(&skipped, "UNSUPPORTED invented reason"),
        "moved_to_hypotheses"
    );
    assert_eq!(fate(&skipped, "Used X because Y"), "moved_to_hypotheses");
}

/// Test counts that match a test fact prove only the counts: the model still
/// rules on the rest of the claim. Counts that disagree reject it outright.
#[test]
fn matching_test_counts_do_not_override_the_model() {
    let r = Repo::new("val-counts");
    let overreach = "UNSUPPORTED 64 of 64 passed, including new tests for UTC-5";
    let corpus = json!({"cases": [{
        "name": "counts",
        "events": [{"id": "ev_t", "type": "test", "fact": "test: pytest, 64/64 pass",
                    "test": {"command": "pytest", "passed": 64, "total": 64}}],
        "claims": [
            {"section": "evidence_summary", "claim": overreach, "type": "observed",
             "evidence": ["ev_t"], "truth": "planted", "plant": "overreach"},
            {"section": "evidence_summary", "claim": "64 of 64 tests passed", "type": "observed",
             "evidence": ["ev_t"], "truth": "supported"},
            {"section": "evidence_summary", "claim": "63 of 64 tests passed", "type": "observed",
             "evidence": ["ev_t"], "truth": "planted", "plant": "wrong_number"}
        ]
    }]});
    let v = audit(&r, &corpus, &[]);
    assert_eq!(fate(&v, overreach), "dropped");
    assert_eq!(fate(&v, "64 of 64 tests passed"), "kept");
    assert_eq!(fate(&v, "63 of 64 tests passed"), "dropped");
}

/// Without a validation model, claims stand on the local rules and the
/// mechanical test checks.
#[test]
fn without_a_validation_model_only_mechanical_checks_apply() {
    let r = Repo::new("val-none");
    let cfg = fs::read_to_string(r.config_path()).unwrap();
    fs::write(
        r.config_path(),
        format!("{cfg}\n[validation]\nprovider = \"none\"\nname = \"none\"\n"),
    )
    .unwrap();
    let corpus = json!({"cases": [{
        "name": "none",
        "events": [
            {"id": "ev_1", "type": "reason", "fact": "reason: x", "excerpt": "Use X because Y."},
            {"id": "ev_t", "type": "test", "fact": "test: t, 9/10 pass",
             "test": {"command": "t", "passed": 9, "total": 10}}
        ],
        "claims": [
            {"claim": "Used X because Y", "evidence": ["ev_1"], "truth": "supported"},
            {"section": "evidence_summary", "claim": "All 10 tests passed", "type": "observed",
             "evidence": ["ev_t"], "truth": "planted", "plant": "test_misread"}
        ]
    }]});
    let v = audit(&r, &corpus, &[]);
    assert_eq!(fate(&v, "Used X because Y"), "kept");
    assert_eq!(fate(&v, "All 10 tests passed"), "dropped");
}

/// A partly supported claim gets one rewrite that sticks to its evidence;
/// the rewrite is judged again and kept only if it passes.
#[test]
fn partly_supported_claims_are_repaired_once() {
    let r = Repo::new("val-repair");
    let corpus = json!({"cases": [{
        "name": "repair",
        "events": [{"id": "ev_1", "type": "reason", "fact": "reason: x", "excerpt": "Use X because Y."}],
        "claims": [{"claim": "MAYBE Used X because Y", "evidence": ["ev_1"], "truth": "supported"}]
    }]});
    let v = audit(&r, &corpus, &[]);
    assert_eq!(fate(&v, "MAYBE Used X because Y"), "repaired");
    // When the rewrite gives up, the claim moves to hypotheses as before.
    let v = audit(&r, &corpus, &[("MOCK_REPAIR", "none")]);
    assert_eq!(fate(&v, "MAYBE Used X because Y"), "moved_to_hypotheses");
}

/// Landing uses the repaired text in the story.
#[test]
fn landed_story_keeps_the_repaired_wording() {
    let r = Repo::new("val-land-repair");
    r.git(&["checkout", "-qb", "feat/r"]);
    r.write("r.rs", "r\n");
    r.ok(&["note", "MAYBE Use R because the review asked for it"]);
    r.commit_all("Add r");
    r.git(&["checkout", "-q", "main"]);
    r.git(&["merge", "-q", "--no-ff", "feat/r", "-m", "Merge feat/r"]);
    let landed = r.ok(&["land", "--work-unit", "feat/r", "--to", "HEAD"]);
    let decisions: Vec<&str> = landed["story"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["claim"].as_str().unwrap())
        .collect();
    assert!(
        decisions.contains(&"Use R because the review asked for it"),
        "{decisions:?}"
    );
    assert!(
        decisions.iter().all(|d| !d.contains("MAYBE")),
        "{decisions:?}"
    );
}

/// A model that swaps indexes but echoes the right claim text cannot keep a
/// rejected claim or drop a supported one.
#[test]
fn swapped_indexes_do_not_move_verdicts() {
    let r = Repo::new("val-swap");
    let corpus = json!({"cases": [{
        "name": "swap",
        "events": [{"id": "ev_1", "type": "reason", "fact": "reason: x", "excerpt": "Use X because Y."}],
        "claims": [
            {"claim": "UNSUPPORTED invented reason", "evidence": ["ev_1"], "truth": "planted", "plant": "invented"},
            {"claim": "Used X because Y", "evidence": ["ev_1"], "truth": "supported"}
        ]
    }]});
    let v = audit(&r, &corpus, &[("MOCK_VERDICTS", "swap")]);
    assert_eq!(fate(&v, "UNSUPPORTED invented reason"), "dropped");
    assert_eq!(fate(&v, "Used X because Y"), "kept");
}

/// The diff counts as evidence only for claims that cite an edit or commit,
/// and is not sent at all when no claim does.
#[test]
fn diff_applies_only_to_claims_that_cite_an_edit() {
    let r = Repo::new("val-diff-rule");
    let log = r.base.join("mock.log");
    let requests = || -> Vec<Value> {
        fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|q| q["schema"]["properties"].get("verdicts").is_some())
            .map(|q| serde_json::from_str(q["user"].as_str().unwrap()).unwrap())
            .collect()
    };
    let diff = json!([{"path": "src/a.rs", "diff": "@@ -1 +1 @@\n-old\n+new\n"}]);
    let reason =
        json!({"id": "ev_r", "type": "reason", "fact": "reason: x", "excerpt": "Use X because Y."});
    let edit =
        json!({"id": "ev_e", "type": "attempt", "fact": "edit: src/a.rs", "paths": ["src/a.rs"]});

    // Only a reason is cited: the diff is not sent.
    let corpus = json!({"cases": [{"name": "no-edit", "events": [reason], "diff": diff,
        "claims": [{"claim": "Used X because Y", "evidence": ["ev_r"], "truth": "supported"}]}]});
    let v = audit(&r, &corpus, &[]);
    let sent = &requests()[0];
    assert!(sent.get("diff").is_none(), "{sent}");
    assert_eq!(sent["claims"][0]["diff_applies"], false);
    assert_eq!(v["egress"]["calls"][0]["hunks"], json!([]));

    // One claim cites the edit: the diff is sent, and only that claim may use it.
    fs::remove_file(&log).unwrap();
    let corpus = json!({"cases": [{"name": "edit", "events": [reason, edit], "diff": diff,
    "claims": [
        {"claim": "Used X because Y", "evidence": ["ev_r"], "truth": "supported"},
        {"claim": "Changed src/a.rs", "type": "observed", "evidence": ["ev_e"], "truth": "supported"}
    ]}]});
    audit(&r, &corpus, &[]);
    let sent = &requests()[0];
    assert_eq!(sent["diff"][0]["path"], "src/a.rs");
    assert_eq!(sent["claims"][0]["diff_applies"], false);
    assert_eq!(sent["claims"][1]["diff_applies"], true);
}
