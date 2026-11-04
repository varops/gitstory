//! The M0 study tools: the validator audit and the control arm on landing.

mod common;

use std::fs;

use common::Repo;
use serde_json::{Value, json};

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../study/validator-audit/corpus.json"
);

fn write_corpus(r: &Repo, corpus: &Value) -> String {
    let p = r.base.join("corpus.json");
    fs::write(&p, corpus.to_string()).unwrap();
    p.to_string_lossy().into_owned()
}

#[test]
fn audit_counts_what_the_validator_catches() {
    let r = Repo::new("audit");
    let corpus = json!({"cases": [{
        "name": "c1",
        "events": [
            {"id": "ev_1", "type": "reason", "fact": "reason: x", "excerpt": "Use X because Y."},
            {"id": "ev_t", "type": "test", "fact": "test: t, 9/10 pass",
             "test": {"command": "t", "passed": 9, "total": 10}}
        ],
        "claims": [
            {"claim": "Used X because Y", "evidence": ["ev_1"], "truth": "supported"},
            // The stand-in model rejects claims containing UNSUPPORTED and
            // half-accepts MAYBE; the test check needs no model at all.
            {"claim": "UNSUPPORTED invented reason", "evidence": ["ev_1"], "truth": "planted", "plant": "invented"},
            {"claim": "MAYBE a guess", "evidence": ["ev_1"], "truth": "planted", "plant": "overreach"},
            {"section": "evidence_summary", "claim": "All 10 tests passed", "type": "observed",
             "evidence": ["ev_t"], "truth": "planted", "plant": "test_misread"},
            {"claim": "Cites an event that does not exist", "evidence": ["ev_nope"], "truth": "planted", "plant": "invented"}
        ]
    }]});
    let v = r.ok(&["study", "audit", &write_corpus(&r, &corpus)]);
    let fates: Vec<(&str, &str)> = v["claims"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["claim"].as_str().unwrap(), c["fate"].as_str().unwrap()))
        .collect();
    assert_eq!(
        fates,
        [
            ("Used X because Y", "kept"),
            ("UNSUPPORTED invented reason", "dropped"),
            ("MAYBE a guess", "repaired"),
            ("All 10 tests passed", "dropped"),
            ("Cites an event that does not exist", "dropped"),
        ]
    );
    let rep = &v["report"];
    assert_eq!(rep["planted_caught"]["k"], 4);
    assert_eq!(rep["planted_caught"]["n"], 4);
    assert_eq!(rep["planted_dropped"]["k"], 3);
    assert_eq!(rep["supported_lost"]["k"], 0);
    assert_eq!(rep["by_plant"]["invented"]["n"], 2);
    assert_eq!(rep["repaired"], 1);
    let repaired = v["claims"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["fate"] == "repaired")
        .unwrap();
    assert_eq!(repaired["repaired_to"], "a guess");
    // Each claim carries the checker's first verdict; local rules remove
    // the claim citing a missing event before the checker sees it.
    let verdicts: Vec<&Value> = v["claims"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| &c["verdict"])
        .collect();
    assert_eq!(
        verdicts,
        [
            &json!("yes"),
            &json!("no"),
            &json!("partly"),
            &json!("no"),
            &Value::Null
        ]
    );
    // 4 of 4 planted caught meets 95% but 4 claims cannot show it; 0 of 1
    // true claims lost is the same story for the 5% ceiling.
    assert_eq!(rep["targets"]["caught"], "not_yet_shown");
    assert_eq!(rep["targets"]["lost"], "not_yet_shown");
    // Neither target is missed, but neither is shown: overall is the weaker.
    assert_eq!(rep["targets"]["overall"], "not_yet_shown");
    // Validation, repair and revalidation all went through the gate.
    let purposes: Vec<&str> = v["egress"]["calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["purpose"].as_str().unwrap())
        .collect();
    assert_eq!(purposes, ["validation", "repair", "revalidation"]);
}

#[test]
fn a_repair_that_keeps_the_falsehood_is_not_a_catch() {
    let r = Repo::new("falsehood");
    // The stand-in model half-accepts MAYBE, and its repair strips only
    // "MAYBE ". The first rewrite drops the labeled falsehood; the second
    // still says it.
    let corpus = json!({"cases": [{
        "name": "c1",
        "events": [{"id": "ev_1", "type": "reason", "fact": "reason: x", "excerpt": "Use X because Y."}],
        "claims": [
            {"claim": "Used X because Y", "evidence": ["ev_1"], "truth": "supported"},
            {"claim": "MAYBE used X because Y", "evidence": ["ev_1"], "truth": "planted",
             "plant": "one_word", "falsehood": "maybe"},
            {"claim": "MAYBE used X because of a 60s lease", "evidence": ["ev_1"], "truth": "planted",
             "plant": "wrong_number", "falsehood": "60S"}
        ]
    }]});
    let v = r.ok(&["study", "audit", &write_corpus(&r, &corpus)]);
    let rows = v["claims"].as_array().unwrap();
    assert_eq!(rows[1]["fate"], "repaired");
    assert_eq!(rows[1]["falsehood_kept"], false);
    assert_eq!(rows[2]["fate"], "repaired");
    assert_eq!(rows[2]["repaired_to"], "used X because of a 60s lease");
    // Case-insensitive: "60S" is still in the rewrite.
    assert_eq!(rows[2]["falsehood_kept"], true);
    let rep = &v["report"];
    assert_eq!(rep["planted_caught"]["k"], 1);
    assert_eq!(rep["planted_caught"]["n"], 2);
    assert_eq!(rep["by_plant"]["one_word"]["k"], 1);
    assert_eq!(rep["by_plant"]["wrong_number"]["k"], 0);
    assert_eq!(rep["repaired"], 2);
    // The human report lists it under misses and marks it in the repairs.
    let out = r
        .cmd()
        .args(["study", "audit", &write_corpus(&r, &corpus)])
        .output()
        .unwrap();
    let human = String::from_utf8_lossy(&out.stdout);
    let misses = human.split("\nmisses\n").nth(1).expect(&human);
    assert!(
        misses.contains("MAYBE used X because of a 60s lease"),
        "{human}"
    );
    assert!(!misses.contains("MAYBE used X because Y"), "{human}");
    assert!(
        human.contains("still says \"60S\": counted as kept"),
        "{human}"
    );
}

#[test]
fn the_shipped_corpus_is_well_formed() {
    let r = Repo::new("corpus");
    let v = r.ok(&["study", "audit", CORPUS]);
    let rep = &v["report"];
    assert!(rep["planted_caught"]["n"].as_u64().unwrap() >= 20, "{rep}");
    assert!(rep["supported_lost"]["n"].as_u64().unwrap() >= 15, "{rep}");
    // Every planted claim is labeled with what was planted.
    for c in v["claims"].as_array().unwrap() {
        if c["truth"] == "planted" {
            assert!(c["plant"].is_string(), "{c}");
        } else {
            assert_eq!(c["truth"], "supported", "{c}");
        }
    }
}

#[test]
fn audit_rejects_a_bad_corpus() {
    let r = Repo::new("badcorpus");
    let bad = json!({"cases": [{"name": "x", "events": [], "claims": [
        {"section": "nope", "claim": "c", "evidence": [], "truth": "supported"}
    ]}]});
    assert_eq!(
        r.err(&["study", "audit", &write_corpus(&r, &bad)]),
        "E_USAGE"
    );
}

#[test]
fn land_writes_the_control_arm_into_a_blind_sheet() {
    let r = Repo::new("baseline");
    r.git(&["checkout", "-qb", "feat/lru"]);
    r.write("cache.rs", "lru\n");
    r.ok(&[
        "note",
        "Use an in-process LRU because review asked for no new services",
    ]);
    r.commit_all("Cache search results");
    r.git(&["checkout", "-q", "main"]);
    r.git(&["merge", "-q", "--no-ff", "feat/lru", "-m", "Merge feat/lru"]);
    let dir = r.base.join("study");
    let landed = r.ok(&[
        "land",
        "--work-unit",
        "feat/lru",
        "--to",
        "HEAD",
        "--study-baseline",
        dir.to_str().unwrap(),
    ]);

    // The baseline call shares the landing's gate and manifest.
    let purposes: Vec<&str> = landed["egress"]["calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["purpose"].as_str().unwrap())
        .collect();
    assert!(purposes.contains(&"study_baseline"), "{purposes:?}");
    let baseline_call = landed["egress"]["calls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["purpose"] == "study_baseline")
        .unwrap();
    let synthesis_call = landed["egress"]["calls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["purpose"] == "synthesis" || c["purpose"] == "reconciliation")
        .unwrap();
    assert_eq!(baseline_call["event_ids"], synthesis_call["event_ids"]);

    let sheet_path = landed["study_sheet"].as_str().unwrap();
    let sheet = fs::read_to_string(sheet_path).unwrap();
    assert!(sheet.contains("## Version A") && sheet.contains("## Version B"));
    assert!(sheet.contains("Baseline: Merge feat/lru"), "{sheet}");
    assert!(sheet.contains("in-process LRU"), "{sheet}");
    assert!(!sheet.contains("ev_") && !sheet.contains("st_"), "{sheet}");

    let key: Value = serde_json::from_str(
        fs::read_to_string(dir.join("key.jsonl"))
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(key["story_id"], landed["story"]["story_id"]);
    let arms = [key["a"].as_str().unwrap(), key["b"].as_str().unwrap()];
    assert!(arms.contains(&"gitstory") && arms.contains(&"baseline"));
}

#[test]
fn land_without_the_flag_makes_no_baseline_call() {
    let r = Repo::new("nobaseline");
    r.git(&["checkout", "-qb", "feat/x"]);
    r.write("x.rs", "x\n");
    r.ok(&["note", "Use X because Y"]);
    r.commit_all("Add x");
    r.git(&["checkout", "-q", "main"]);
    r.git(&["merge", "-q", "--no-ff", "feat/x", "-m", "Merge feat/x"]);
    let landed = r.ok(&["land", "--work-unit", "feat/x", "--to", "HEAD"]);
    assert!(
        landed["egress"]["calls"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["purpose"] != "study_baseline")
    );
    assert!(landed["study_sheet"].is_null());
}

#[test]
fn score_script_unblinds_and_reports_intervals() {
    let d = common::tmp("score");
    fs::write(
        d.join("key.jsonl"),
        "{\"commit\":\"aaaaaaaaaaaa\",\"a\":\"gitstory\",\"b\":\"baseline\"}\n\
         {\"commit\":\"bbbbbbbbbbbb\",\"a\":\"baseline\",\"b\":\"gitstory\"}\n",
    )
    .unwrap();
    fs::write(
        d.join("ratings.csv"),
        "commit,rater,a_accuracy,a_usefulness,b_accuracy,b_usefulness,prefer\n\
         aaaaaaaaaaaa,r1,5,4,3,3,A\n\
         bbbbbbbbbbbb,r1,2,3,5,5,B\n",
    )
    .unwrap();
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../study/m0/score.py");
    let out = std::process::Command::new(common::python())
        .args([script, d.to_str().unwrap()])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Both preferences went to GitStory once the labels are un-blinded.
    assert!(
        text.contains("prefer gitstory   2/2 [0.34, 1.00]"),
        "{text}"
    );
    assert!(text.contains("gitstory  accuracy   mean 5.00"), "{text}");
    assert!(text.contains("do not tell the arms apart"), "{text}");
}

#[test]
fn audit_reports_missed_targets() {
    let r = Repo::new("targets");
    let mut claims =
        vec![json!({"claim": "Used X because Y", "evidence": ["ev_1"], "truth": "supported"})];
    // Planted claims the stand-in model accepts: none of them is caught.
    for i in 0..3 {
        claims.push(
            json!({"claim": format!("Planted claim {i}"), "evidence": ["ev_1"],
                           "truth": "planted", "plant": "invented"}),
        );
    }
    let corpus = json!({"cases": [{"name": "t",
        "events": [{"id": "ev_1", "type": "reason", "fact": "reason: x", "excerpt": "Use X because Y."}],
        "claims": claims}]});
    let v = r.ok(&["study", "audit", &write_corpus(&r, &corpus)]);
    assert_eq!(v["report"]["targets"]["caught"], "missed");
    assert_eq!(v["report"]["targets"]["overall"], "missed");
    // Custom targets are passed through.
    let v = r.ok(&[
        "study",
        "audit",
        &write_corpus(&r, &corpus),
        "--min-caught",
        "0",
        "--max-lost",
        "1",
    ]);
    assert_eq!(v["report"]["targets"]["overall"], "shown");
    assert_eq!(
        r.err(&[
            "study",
            "audit",
            &write_corpus(&r, &corpus),
            "--min-caught",
            "95"
        ]),
        "E_USAGE"
    );
}
