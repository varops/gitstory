//! Lifecycle state: work units, landing, links, proposals, verdicts and the
//! other records whose state used to be spread across flags and files.
//! Every test drives the real binary with the stand-in model.

mod common;

use std::fs;
use std::path::PathBuf;

use common::Repo;
use serde_json::{Value, json};

const DAY: i64 = 86_400_000;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn slug(w: &str) -> String {
    w.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn wu_dir(r: &Repo, w: &str) -> PathBuf {
    r.root.join(".git/story/collect").join(slug(w))
}

fn read_meta(r: &Repo, w: &str) -> Value {
    serde_json::from_slice(&fs::read(wu_dir(r, w).join("meta.json")).unwrap()).unwrap()
}

fn write_meta(r: &Repo, w: &str, v: &Value) {
    fs::write(
        wu_dir(r, w).join("meta.json"),
        serde_json::to_vec_pretty(v).unwrap(),
    )
    .unwrap();
}

/// Move every landing of a work unit `age` into the past, in whichever
/// meta format the binary writes.
fn backdate_landing(r: &Repo, w: &str, age: i64) {
    let mut m = read_meta(r, w);
    let t = now_ms() - age;
    if m.get("landed_at").is_some() {
        m["landed_at"] = json!(t);
    }
    if let Some(list) = m["landed"].as_array_mut() {
        for l in list {
            if l.is_object() {
                l["at"] = json!(t);
            }
        }
    }
    write_meta(r, w, &m);
}

fn backdate_activity(r: &Repo, w: &str, age: i64) {
    let mut m = read_meta(r, w);
    let t = now_ms() - age;
    m["created_at"] = json!(t);
    if m.get("last_activity").is_some() {
        m["last_activity"] = json!(t);
    }
    write_meta(r, w, &m);
}

fn store_dir(r: &Repo) -> PathBuf {
    r.root.join(".git/story/store/repos/acme/demo")
}

/// Every entry of the link log, oldest first.
fn link_log(r: &Repo) -> Vec<Value> {
    let dir = store_dir(r).join("links");
    let mut names: Vec<PathBuf> = fs::read_dir(&dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    names.sort();
    names
        .iter()
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| serde_json::from_slice(&fs::read(p).unwrap()).unwrap())
        .collect()
}

/// Land one branch with a note; returns the story id.
fn land_branch(r: &Repo, branch: &str, file: &str, note: &[&str]) -> String {
    r.git(&["checkout", "-qb", branch]);
    r.write(file, &format!("{branch}\n"));
    let mut args = vec!["note"];
    args.extend_from_slice(note);
    r.ok(&args);
    r.commit_all(&format!("{branch} change"));
    r.git(&["checkout", "-q", "main"]);
    r.git(&[
        "merge",
        "-q",
        "--no-ff",
        branch,
        "-m",
        &format!("Merge {branch}"),
    ]);
    r.ok(&["land", "--work-unit", branch, "--to", "HEAD"])["story"]["story_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn claims(story: &Value, section: &str) -> String {
    story[section]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| c["claim"].as_str().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 1. Work unit lifecycle

/// `land --since` narrows one run only: later drafts of the same work unit
/// see commits made after the landed tip.
#[test]
fn land_since_does_not_persist_tip_or_window() {
    let r = Repo::new("since-scope");
    let base = r.git(&["rev-parse", "HEAD"]);
    r.write("one.rs", "1\n");
    r.commit_all("Add one");
    r.ok(&[
        "note",
        "Keep one file because it loads alone",
        "--work-unit",
        "run",
    ]);
    r.ok(&[
        "land",
        "--work-unit",
        "run",
        "--since",
        &base,
        "--to",
        "HEAD",
    ]);
    let meta = read_meta(&r, "run");
    assert!(meta.get("tip").is_none(), "tip saved: {meta}");
    assert!(meta.get("window").is_none(), "window saved: {meta}");
    r.write("three.rs", "3\n");
    r.commit_all("Add three");
    let d = r.ok(&["draft", "--work-unit", "run"]);
    let decisions = claims(&d["story"], "decisions");
    assert!(
        decisions.contains("three.rs"),
        "commit after the landed tip missing: {decisions}"
    );
}

/// After a landing, the next landing of the same work unit starts a new
/// round: earlier drafts and `note --supersedes` are not replayed.
#[test]
fn landing_closes_the_round() {
    let r = Repo::new("round");
    let a = land_branch(&r, "feat/a", "a.rs", &["Add a because the queue needs it"]);
    let base = r.git(&["rev-parse", "HEAD"]);
    r.write("one.rs", "1\n");
    let first = r.commit_all("Add one");
    r.ok(&[
        "note",
        "Replace a with one because it is simpler",
        "--work-unit",
        "run",
        "--supersedes",
        &a,
    ]);
    r.ok(&["draft", "--work-unit", "run"]);
    let s1 = r.ok(&[
        "land",
        "--work-unit",
        "run",
        "--since",
        &base,
        "--to",
        "HEAD",
    ]);
    let s1 = s1["story"]["story_id"].as_str().unwrap().to_owned();
    let meta = read_meta(&r, "run");
    assert!(meta.get("supersedes").is_none(), "supersedes kept: {meta}");
    assert_eq!(meta["base_rev"], first.as_str(), "base not advanced");
    assert!(
        wu_dir(&r, "run")
            .join("drafts/landed")
            .join(&first)
            .is_dir(),
        "consumed drafts not moved aside"
    );

    r.write("two.rs", "2\n");
    r.commit_all("Add two");
    r.ok(&[
        "note",
        "Add two because it loads late",
        "--work-unit",
        "run",
    ]);
    let s2 = r.ok(&[
        "land",
        "--work-unit",
        "run",
        "--since",
        &first,
        "--to",
        "HEAD",
    ]);
    // No earlier drafts: a plain synthesis, not a reconciliation.
    assert_eq!(s2["egress"]["calls"][0]["purpose"], "synthesis", "{s2}");
    let s2 = s2["story"]["story_id"].as_str().unwrap().to_owned();
    let log = link_log(&r);
    let sup: Vec<&Value> = log
        .iter()
        .filter(|l| l["kind"] == "supersedes" && l["to"] == a.as_str())
        .collect();
    assert_eq!(sup.len(), 1, "supersedes replayed: {sup:?}");
    assert_eq!(sup[0]["from"], s1.as_str());
    assert_ne!(s1, s2);
}

/// gc removes a landed work unit only when nothing happened since it
/// landed, and expires unlanded work from its last activity. `status`
/// reports the same expiry gc acts on.
#[test]
fn gc_keeps_work_that_arrived_after_landing() {
    let r = Repo::new("gc-activity");
    land_branch(&r, "feat/a", "a.rs", &["Add a because the queue needs it"]);
    land_branch(&r, "feat/b", "b.rs", &["Add b because the cache needs it"]);
    // feat/a gets new evidence after landing; feat/b stays idle.
    r.ok(&[
        "note",
        "Keep a small because it ships in the boot image",
        "--work-unit",
        "feat/a",
    ]);
    backdate_landing(&r, "feat/a", 3 * DAY);
    backdate_landing(&r, "feat/b", 3 * DAY);
    backdate_activity(&r, "feat/b", 4 * DAY);
    // Unlanded, created long ago, but active today.
    r.ok(&[
        "note",
        "Try a ring buffer because allocation is slow",
        "--work-unit",
        "idea",
    ]);
    let mut m = read_meta(&r, "idea");
    m["created_at"] = json!(now_ms() - 400 * DAY);
    write_meta(&r, "idea", &m);

    let status = r.ok(&["status"]);
    let gc = r.ok(&["gc", "--dry-run"]);
    let removed: Vec<&str> = gc["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(removed, ["feat/b"], "{gc}");
    let now = now_ms();
    for u in status["work_units"].as_array().unwrap() {
        let name = u["work_unit"].as_str().unwrap();
        let expired = u["expires_at"].as_i64().unwrap() <= now;
        assert_eq!(
            expired,
            removed.contains(&name),
            "status and gc disagree on {name}: {u}"
        );
    }
    r.ok(&["gc"]);
    assert!(wu_dir(&r, "feat/a").join("meta.json").exists());
    assert!(wu_dir(&r, "idea").join("meta.json").exists());
    assert!(!wu_dir(&r, "feat/b").exists());
}

/// Meta written before landings carried their own time still loads.
#[test]
fn old_work_unit_meta_still_loads() {
    let r = Repo::new("old-meta");
    let head = r.git(&["rev-parse", "HEAD"]);
    for d in ["events", "drafts", "egress"] {
        fs::create_dir_all(wu_dir(&r, "legacy").join(d)).unwrap();
    }
    let landed_at = now_ms() - 3 * DAY;
    write_meta(
        &r,
        "legacy",
        &json!({
            "work_unit": "legacy",
            "created_at": landed_at - DAY,
            "base_rev": head,
            "offsets": {},
            "landed": [[head, "st_old"]],
            "landed_at": landed_at,
            "tip": head,
            "window": [0, 1],
            "ghf_holds": ["tr_1"]
        }),
    );
    let status = r.ok(&["status"]);
    let u = status["work_units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["work_unit"] == "legacy")
        .unwrap()
        .clone();
    assert_eq!(u["landed"][0]["commit"], head.as_str(), "{u}");
    assert_eq!(u["landed"][0]["story"], "st_old");
    assert_eq!(u["landed"][0]["at"], landed_at);
    assert_eq!(u["state"], "landed");
    assert_eq!(r.ok(&["gc", "--dry-run"])["removed"], json!(["legacy"]));
}

/// `feat/a` and `feat_a` would share one local directory.
#[test]
fn work_unit_ids_that_share_a_directory_are_refused() {
    let r = Repo::new("slug");
    r.ok(&[
        "note",
        "Use a queue because jobs pile up",
        "--work-unit",
        "feat/a",
    ]);
    let (code, v) = r.run(&[
        "note",
        "Use a stack because order does not matter",
        "--work-unit",
        "feat_a",
    ]);
    assert_ne!(code, 0, "{v}");
    assert_eq!(v["error"]["code"], "E_USAGE");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("feat/a") && msg.contains("feat_a"), "{msg}");
}

// ---------------------------------------------------------------------------
// 2. Landing phases and idempotence

/// Make every link write fail: a plain file where the link log directory goes.
fn break_link_log(r: &Repo) {
    let dir = store_dir(r).join("links");
    if dir.is_dir() {
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(dir.parent().unwrap()).unwrap();
    fs::write(&dir, "not a directory").unwrap();
}

fn fix_link_log(r: &Repo) {
    let dir = store_dir(r).join("links");
    if dir.is_file() {
        fs::remove_file(&dir).unwrap();
    }
}

fn stories_for_commit(r: &Repo, sha: &str) -> usize {
    fs::read_dir(store_dir(r).join("commits").join(sha))
        .map(|rd| rd.count())
        .unwrap_or(0)
}

/// A landing that failed after writing its story finishes on retry instead
/// of writing a second story for the same commit; landing again is a no-op.
#[test]
fn land_retry_finishes_instead_of_duplicating() {
    let r = Repo::new("land-retry");
    let a = land_branch(&r, "feat/a", "a.rs", &["Add a because the queue needs it"]);
    r.git(&["checkout", "-qb", "feat/b"]);
    r.write("b.rs", "b\n");
    r.ok(&[
        "note",
        "Replace a with b because b is simpler",
        "--supersedes",
        &a,
    ]);
    r.commit_all("Add b");
    r.git(&["checkout", "-q", "main"]);
    r.git(&["merge", "-q", "--no-ff", "feat/b", "-m", "Merge feat/b"]);
    let sha = r.git(&["rev-parse", "HEAD"]);

    break_link_log(&r);
    let (code, v) = r.run(&["land", "--work-unit", "feat/b", "--to", "HEAD"]);
    assert_ne!(
        code, 0,
        "land should fail while links cannot be written: {v}"
    );
    fix_link_log(&r);

    let again = r.ok(&["land", "--work-unit", "feat/b", "--to", "HEAD"]);
    assert_eq!(again["resumed"], true, "{again}");
    assert_eq!(
        stories_for_commit(&r, &sha),
        1,
        "a second story was written"
    );
    let id = again["story"]["story_id"].as_str().unwrap().to_owned();
    assert_eq!(r.ok(&["validate", "HEAD"])["story"], id.as_str());
    let log = link_log(&r);
    assert!(
        log.iter()
            .any(|l| l["from"] == id.as_str() && l["to"] == a.as_str()),
        "the supersedes link was not finished: {log:?}"
    );
    // Landing the same commit once more changes nothing.
    let third = r.ok(&["land", "--work-unit", "feat/b", "--to", "HEAD"]);
    assert_eq!(third["story"]["story_id"], id.as_str());
    assert_eq!(stories_for_commit(&r, &sha), 1);
    assert_eq!(link_log(&r).len(), log.len());
    assert_eq!(
        read_meta(&r, "feat/b")["landed"].as_array().unwrap().len(),
        1
    );
}

/// A `supersedes` note that would close a cycle is skipped and reported.
#[test]
fn land_skips_and_reports_a_supersedes_cycle() {
    let r = Repo::new("land-cycle");
    let a = land_branch(&r, "feat/a", "a.rs", &["Add a because the queue needs it"]);
    r.git(&["checkout", "-qb", "feat/b"]);
    r.write("b.rs", "b\n");
    r.ok(&[
        "note",
        "Replace a with b because b is simpler",
        "--supersedes",
        &a,
    ]);
    r.commit_all("Add b");
    r.git(&["checkout", "-q", "main"]);
    r.git(&["merge", "-q", "--no-ff", "feat/b", "-m", "Merge feat/b"]);

    break_link_log(&r);
    let (code, _) = r.run(&["land", "--work-unit", "feat/b", "--to", "HEAD"]);
    assert_ne!(code, 0);
    fix_link_log(&r);
    // Meanwhile someone links the other way round.
    let b = r.ok(&["show", "HEAD"])["stories"][0]["story_id"]
        .as_str()
        .unwrap()
        .to_owned();
    r.ok(&["link", &a, "--supersedes", &b]);

    let again = r.ok(&["land", "--work-unit", "feat/b", "--to", "HEAD"]);
    assert_eq!(again["skipped_links"], json!([a]), "{again}");
    assert!(
        !link_log(&r)
            .iter()
            .any(|l| l["from"] == b.as_str() && l["to"] == a.as_str()),
        "the cycle was written"
    );
}

// ---------------------------------------------------------------------------
// 3. Story status and links come from the link log only

/// Land A, then B with `note --supersedes A`.
fn a_then_b(r: &Repo) -> (String, String) {
    let a = land_branch(r, "feat/a", "a.rs", &["Add a because the queue needs it"]);
    let b = land_branch(
        r,
        "feat/b",
        "b.rs",
        &["Replace a with b because b is simpler", "--supersedes", &a],
    );
    (a, b)
}

#[test]
fn link_remove_undoes_a_note_supersedes() {
    let r = Repo::new("unlink-note");
    let (a, b) = a_then_b(&r);
    assert_eq!(r.ok(&["show", &b])["stories"][0]["supersedes"], json!([a]));
    assert_eq!(r.ok(&["show", &a])["stories"][0]["status"], "superseded");
    r.ok(&["link", &b, "--supersedes", &a, "--remove"]);
    let sb = r.ok(&["show", &b])["stories"][0].clone();
    assert_eq!(sb["supersedes"], json!([]), "{sb}");
    assert_eq!(r.ok(&["show", &a])["stories"][0]["status"], "current");
    let h = r.ok(&["history", &b]);
    assert_eq!(h["chain"].as_array().unwrap().len(), 1, "{h}");
}

#[test]
fn edit_and_export_show_the_derived_status() {
    let r = Repo::new("derived-view");
    let (a, b) = a_then_b(&r);
    let e = r.ok(&["edit", &a, "--review", "reviewed"]);
    assert_eq!(e["story"]["status"], "superseded", "{e}");
    let out = r.base.join("export");
    r.ok(&["export", "--out", out.to_str().unwrap()]);
    let bundle: Value =
        serde_json::from_slice(&fs::read(out.join("gitstory-export.json")).unwrap()).unwrap();
    let view = |id: &str| {
        bundle["stories"]
            .as_array()
            .unwrap_or_else(|| panic!("no stories view in the export: {}", bundle["schema"]))
            .iter()
            .find(|s| s["story_id"] == id)
            .cloned()
            .unwrap()
    };
    assert_eq!(view(&a)["status"], "superseded");
    assert_eq!(view(&b)["supersedes"], json!([a]));
    // The store dump itself is unchanged, so the bundle still imports.
    let other = Repo::new("derived-view-b");
    other.ok(&["import", out.to_str().unwrap()]);
    assert_eq!(
        other.ok(&["show", &a])["stories"][0]["status"],
        "superseded"
    );
}

// ---------------------------------------------------------------------------
// 4. Link proposals and the link log

/// A and B both rewrite line 1 of lock.txt: landing B proposes B supersedes A.
fn proposal_scenario(r: &Repo) -> (String, String, String) {
    r.write("lock.txt", "x\nkeep\n");
    r.commit_all("lock file");
    let edit = |branch: &str, line: &str, note: &str| {
        r.git(&["checkout", "-qb", branch]);
        r.write("lock.txt", &format!("{line}\nkeep\n"));
        r.ok(&["note", note]);
        r.commit_all(&format!("{branch} change"));
        r.git(&["checkout", "-q", "main"]);
        r.git(&[
            "merge",
            "-q",
            "--no-ff",
            branch,
            "-m",
            &format!("Merge {branch}"),
        ]);
        r.ok(&["land", "--work-unit", branch, "--to", "HEAD"])
    };
    let a = edit(
        "feat/a",
        "lock = redis",
        "Add a Redis lock because retries double-charge",
    );
    let a = a["story"]["story_id"].as_str().unwrap().to_owned();
    let landed = edit(
        "feat/b",
        "lock = none",
        "Remove the lock because versioning replaces it",
    );
    let b = landed["story"]["story_id"].as_str().unwrap().to_owned();
    let p = landed["proposals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["to"] == a.as_str())
        .expect("B supersedes A proposed")["id"]
        .as_str()
        .unwrap()
        .to_owned();
    (a, b, p)
}

fn proposal_status(r: &Repo, pid: &str) -> Value {
    let p: Value = serde_json::from_slice(
        &fs::read(store_dir(r).join("proposals").join(format!("{pid}.json"))).unwrap(),
    )
    .unwrap();
    p["status"].clone()
}

/// An accept that appended its link but failed before saving the proposal
/// does not append a second link when run again.
#[test]
fn accepting_after_a_partial_failure_adds_no_second_link() {
    let r = Repo::new("accept-twice");
    let (a, b, pid) = proposal_scenario(&r);
    // What the failed accept left behind: the link, proposal still open.
    let at = now_ms();
    let entry = json!({"op": "add", "kind": "supersedes", "from": b, "to": a,
                       "at": at, "author": "Dev", "proposal": pid});
    fs::create_dir_all(store_dir(&r).join("links")).unwrap();
    fs::write(
        store_dir(&r)
            .join("links")
            .join(format!("{at:015}-dead.json")),
        serde_json::to_vec_pretty(&entry).unwrap(),
    )
    .unwrap();
    assert_eq!(proposal_status(&r, &pid), "open");
    r.ok(&["link", "--accept", &pid]);
    assert_eq!(proposal_status(&r, &pid), "accepted");
    let n = link_log(&r)
        .iter()
        .filter(|l| l["proposal"] == pid.as_str())
        .count();
    assert_eq!(n, 1, "a second link was appended");
    // Accepting an accepted proposal again changes nothing.
    r.ok(&["link", "--accept", &pid]);
    assert_eq!(link_log(&r).len(), 1);
}

/// Import never moves a decided proposal back to open.
#[test]
fn import_does_not_reopen_a_decided_proposal() {
    let r = Repo::new("import-proposal");
    let (_, _, pid) = proposal_scenario(&r);
    let out = r.base.join("export");
    r.ok(&["export", "--out", out.to_str().unwrap()]);
    r.ok(&["link", "--accept", &pid]);
    let (code, v) = r.run(&["import", out.to_str().unwrap()]);
    assert_ne!(code, 0, "{v}");
    assert_eq!(v["error"]["code"], "E_STORE_CONFLICT", "{v}");
    assert_eq!(proposal_status(&r, &pid), "accepted");
    assert!(
        r.ok(&["proposals"])["proposals"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

/// Removing the link an accepted proposal created withdraws the proposal.
#[test]
fn removing_an_accepted_link_withdraws_its_proposal() {
    let r = Repo::new("withdraw");
    let (a, b, pid) = proposal_scenario(&r);
    r.ok(&["link", "--accept", &pid]);
    r.ok(&["link", &b, "--supersedes", &a, "--remove"]);
    assert_eq!(proposal_status(&r, &pid), "withdrawn");
    assert_eq!(r.ok(&["show", &a])["stories"][0]["status"], "current");
    assert_eq!(r.err(&["link", "--accept", &pid]), "E_USAGE");
}

// ---------------------------------------------------------------------------
// 5. Validation verdicts

fn run_env(r: &Repo, env: &[(&str, &str)], args: &[&str]) -> Value {
    let mut c = r.cmd();
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.args(args).arg("--json").output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    v["data"].clone()
}

fn draft_with_verdicts(name: &str, mode: &str) -> Value {
    let r = Repo::new(name);
    r.git(&["checkout", "-qb", "feat/v"]);
    r.write("a.rs", "a\n");
    r.ok(&["note", "Keep a small because it ships in the boot image"]);
    r.commit_all("Add a");
    run_env(&r, &[("MOCK_VERDICTS", mode)], &["draft"])["story"].clone()
}

/// A verdict outside yes/partly/no does not count as supported.
#[test]
fn unrecognized_verdicts_are_not_supported() {
    let s = draft_with_verdicts("verdict-odd", "odd");
    assert_eq!(claims(&s, "decisions"), "", "{s}");
    assert!(claims(&s, "hypotheses").contains("Changed a.rs"));
}

/// Verdicts are read without regard to case.
#[test]
fn verdicts_ignore_case() {
    let s = draft_with_verdicts("verdict-upper", "upper");
    let d = claims(&s, "decisions");
    assert!(!d.contains("UNSUPPORTED"), "a NO verdict was kept: {d}");
    assert!(d.contains("Changed a.rs"), "a YES verdict was dropped: {d}");
}

/// `validate` counts a claim the repair step fixed as valid and lists it.
#[test]
fn validate_accepts_repaired_claims() {
    let r = Repo::new("validate-repair");
    let id = land_branch(&r, "feat/a", "a.rs", &["Add a because the queue needs it"]);
    r.ok(&["edit", &id, "--add-decision", "MAYBE keep a small"]);
    let v = r.ok(&["validate", &id]);
    assert_eq!(v["valid"], true, "{v}");
    assert_eq!(v["unsupported"], json!([]));
    assert_eq!(
        v["repairs"],
        json!([{"from": "MAYBE keep a small", "to": "keep a small"}])
    );
}

// ---------------------------------------------------------------------------
// 6. Concurrent writers on one work unit

/// A note written while `land` synthesizes survives `land`'s meta update.
#[test]
fn meta_written_during_land_is_kept() {
    let r = Repo::new("concurrent");
    let a = land_branch(&r, "feat/a", "a.rs", &["Add a because the queue needs it"]);
    r.git(&["checkout", "-qb", "feat/b"]);
    r.write("b.rs", "b\n");
    r.ok(&["note", "Add b because the cache needs it"]);
    r.commit_all("Add b");
    r.git(&["checkout", "-q", "main"]);
    r.git(&["merge", "-q", "--no-ff", "feat/b", "-m", "Merge feat/b"]);

    let log = r.base.join("mock.log");
    let lines = || {
        fs::read_to_string(&log)
            .map(|t| t.lines().count())
            .unwrap_or(0)
    };
    let before = lines();
    let child = r
        .cmd()
        .env("MOCK_SLEEP", "3")
        .args(["land", "--work-unit", "feat/b", "--to", "HEAD", "--json"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // Wait until land is inside synthesis.
    let t = std::time::Instant::now();
    while lines() == before {
        assert!(t.elapsed().as_secs() < 30, "land never reached the model");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    r.ok(&[
        "note",
        "Replace a with b because b is simpler",
        "--work-unit",
        "feat/b",
        "--supersedes",
        &a,
    ]);
    let out = child.wait_with_output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    let meta = read_meta(&r, "feat/b");
    assert_eq!(meta["supersedes"], json!([a]), "the note was lost: {meta}");
    assert_eq!(meta["landed"].as_array().unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// 7. ghf holds

/// A fake `git hf` that logs its calls and serves one trace on `base`.
#[cfg(unix)]
fn fake_ghf(r: &Repo, base: &str) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(r.root.join(".git/ghf")).unwrap();
    let fake = r.base.join("fake-ghf");
    let log = r.base.join("ghf.log");
    let script = format!(
        r#"#!/usr/bin/env python3
import json, sys
args = [a for a in sys.argv[1:] if a != "--json"]
open({log:?}, "a").write(" ".join(args) + "\n")
def ok(data): print(json.dumps({{"schema": "ghf.cli/v1", "ok": True, "data": data}}))
if args[:2] == ["trace", "list"]:
    ok({{"traces": [{{"id": "tr_1", "state": "open", "local": True}}]}})
elif args[:2] == ["trace", "log"]:
    ok({{"trace": {{"id": "tr_1"}}, "checkpoints": [
        {{"id": "ck_1", "seq": 1, "base_rev": "{base}", "created_at": 1, "author": "claude-code:s1",
          "message": "tried a lock", "changes": [{{"path": "lock.rs", "status": "added"}}]}}]}})
elif args[:2] in (["trace", "hold"], ["trace", "release"]):
    ok({{"trace": {{"id": "tr_1"}}}})
else:
    print(json.dumps({{"ok": False}})); sys.exit(1)
"#
    );
    fs::write(&fake, script).unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    (fake, log)
}

#[cfg(unix)]
fn count(log: &PathBuf, call: &str) -> usize {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with(call))
        .count()
}

/// A hold older than a week is renewed, so a long-running work unit keeps
/// its trace.
#[cfg(unix)]
#[test]
fn old_ghf_holds_are_renewed() {
    let r = Repo::new("ghf-renew");
    let base = r.git(&["rev-parse", "HEAD"]);
    let (fake, log) = fake_ghf(&r, &base);
    r.git(&["checkout", "-qb", "feat/g"]);
    r.write("lock.rs", "v\n");
    r.ok(&["note", "Use row versioning because the lock failed"]);
    r.commit_all("Row versioning");
    run_env(&r, &[("GITSTORY_GHF", fake.to_str().unwrap())], &["draft"]);
    assert_eq!(count(&log, "trace hold tr_1"), 1);
    // Drafting again within the week does not hold again.
    run_env(&r, &[("GITSTORY_GHF", fake.to_str().unwrap())], &["draft"]);
    assert_eq!(count(&log, "trace hold tr_1"), 1);
    let mut m = read_meta(&r, "feat/g");
    for h in m["ghf_holds"].as_array_mut().unwrap() {
        if h.is_object() {
            h["at"] = json!(now_ms() - 8 * DAY);
        }
    }
    write_meta(&r, "feat/g", &m);
    run_env(&r, &[("GITSTORY_GHF", fake.to_str().unwrap())], &["draft"]);
    assert_eq!(
        count(&log, "trace hold tr_1"),
        2,
        "an old hold was not renewed"
    );
    let held_at = read_meta(&r, "feat/g")["ghf_holds"][0]["at"]
        .as_i64()
        .unwrap();
    assert!(now_ms() - held_at < DAY);
}

/// gc releases the holds of the work units it removes.
#[cfg(unix)]
#[test]
fn gc_releases_ghf_holds() {
    let r = Repo::new("ghf-gc");
    let base = r.git(&["rev-parse", "HEAD"]);
    let (fake, log) = fake_ghf(&r, &base);
    r.git(&["checkout", "-qb", "feat/g"]);
    r.write("lock.rs", "v\n");
    r.ok(&["note", "Use row versioning because the lock failed"]);
    r.commit_all("Row versioning");
    run_env(&r, &[("GITSTORY_GHF", fake.to_str().unwrap())], &["draft"]);
    assert_eq!(count(&log, "trace hold tr_1"), 1);
    backdate_activity(&r, "feat/g", 400 * DAY);
    let gc = run_env(&r, &[("GITSTORY_GHF", fake.to_str().unwrap())], &["gc"]);
    assert_eq!(gc["removed"], json!(["feat/g"]));
    assert_eq!(count(&log, "trace release tr_1"), 1, "hold not released");
}

// ---------------------------------------------------------------------------
// 8. Story versions are written once

/// An edit that would replace a version someone else just wrote fails.
#[test]
fn edit_never_replaces_an_existing_version() {
    let r = Repo::new("version-race");
    let id = land_branch(&r, "feat/a", "a.rs", &["Add a because the queue needs it"]);
    let dir = store_dir(&r).join("stories").join(&id);
    // Another machine's edit wrote v2 but not yet the current copy.
    let mut theirs: Value =
        serde_json::from_slice(&fs::read(dir.join("v1.json")).unwrap()).unwrap();
    theirs["version"] = json!(2);
    theirs["summary"] = json!("their edit");
    let theirs = serde_json::to_vec_pretty(&theirs).unwrap();
    fs::write(dir.join("v2.json"), &theirs).unwrap();
    let (code, v) = r.run(&["edit", &id, "--summary", "my edit"]);
    assert_ne!(code, 0, "{v}");
    assert_eq!(v["error"]["code"], "E_STORE_CONFLICT", "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("again"));
    assert_eq!(fs::read(dir.join("v2.json")).unwrap(), theirs);
}

/// Import refuses a story whose version does not match its key.
#[test]
fn import_refuses_a_story_under_the_wrong_version() {
    let r = Repo::new("import-version");
    let id = land_branch(&r, "feat/a", "a.rs", &["Add a because the queue needs it"]);
    let out = r.base.join("export");
    r.ok(&["export", "--out", out.to_str().unwrap()]);
    let path = out.join("gitstory-export.json");
    let mut bundle: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    bundle["objects"][format!("stories/{id}/v1.json")]["version"] = json!(5);
    fs::write(&path, serde_json::to_vec_pretty(&bundle).unwrap()).unwrap();
    let other = Repo::new("import-version-b");
    let (code, v) = other.run(&["import", out.to_str().unwrap()]);
    assert_ne!(code, 0, "{v}");
    assert_eq!(v["error"]["code"], "E_USAGE", "{v}");
}

// ---------------------------------------------------------------------------
// 9. Egress manifests

fn manifest_ids(dir: &PathBuf) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .filter_map(|e| {
                    e.file_name()
                        .to_str()?
                        .strip_suffix(".json")
                        .map(str::to_owned)
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// A landing uploads the manifests of the work unit's drafts too.
#[test]
fn landing_uploads_draft_manifests() {
    let r = Repo::new("egress-upload");
    r.git(&["checkout", "-qb", "feat/a"]);
    r.write("a.rs", "a\n");
    r.ok(&["note", "Add a because the queue needs it"]);
    r.commit_all("Add a");
    let d = r.ok(&["draft"]);
    let draft_manifest = d["egress"]["id"].as_str().unwrap().to_owned();
    r.git(&["checkout", "-q", "main"]);
    r.git(&["merge", "-q", "--no-ff", "feat/a", "-m", "Merge feat/a"]);
    r.ok(&["land", "--work-unit", "feat/a", "--to", "HEAD"]);
    let stored = manifest_ids(&store_dir(&r).join("egress"));
    assert!(stored.contains(&draft_manifest), "{stored:?}");
    assert!(manifest_ids(&wu_dir(&r, "feat/a").join("egress")).is_empty());
}

/// gc never deletes a manifest that has not reached the store.
#[test]
fn gc_keeps_manifests_until_uploaded() {
    let r = Repo::new("egress-gc");
    r.git(&["checkout", "-qb", "feat/a"]);
    r.write("a.rs", "a\n");
    r.ok(&["note", "Add a because the queue needs it"]);
    r.commit_all("Add a");
    let d = r.ok(&["draft"]);
    let id = d["egress"]["id"].as_str().unwrap().to_owned();
    backdate_activity(&r, "feat/a", 400 * DAY);
    // The store cannot take manifests right now.
    let egress = store_dir(&r).join("egress");
    fs::create_dir_all(egress.parent().unwrap()).unwrap();
    fs::write(&egress, "not a directory").unwrap();
    let gc = r.ok(&["gc"]);
    assert!(
        wu_dir(&r, "feat/a")
            .join("egress")
            .join(format!("{id}.json"))
            .exists(),
        "manifest deleted before upload: {gc}"
    );
    assert!(!gc["warnings"].as_array().unwrap().is_empty(), "{gc}");
    fs::remove_file(&egress).unwrap();
    let gc = r.ok(&["gc"]);
    assert_eq!(gc["removed"], json!(["feat/a"]), "{gc}");
    assert!(manifest_ids(&egress).contains(&id));
}

// ---------------------------------------------------------------------------
// 10. Claim status matches its section

/// Import refuses a story whose claims carry the wrong status for their
/// section.
#[test]
fn import_refuses_a_claim_status_that_contradicts_its_section() {
    let r = Repo::new("claim-status");
    let id = land_branch(&r, "feat/a", "a.rs", &["Add a because the queue needs it"]);
    let out = r.base.join("export");
    r.ok(&["export", "--out", out.to_str().unwrap()]);
    let path = out.join("gitstory-export.json");
    let mut bundle: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for key in [
        format!("stories/{id}.json"),
        format!("stories/{id}/v1.json"),
    ] {
        let h = &mut bundle["objects"][&key]["hypotheses"][0];
        assert!(h.is_object(), "the story has a hypothesis");
        h["status"] = json!("implemented");
    }
    fs::write(&path, serde_json::to_vec_pretty(&bundle).unwrap()).unwrap();
    let other = Repo::new("claim-status-b");
    let (code, v) = other.run(&["import", out.to_str().unwrap()]);
    assert_ne!(code, 0, "{v}");
    assert_eq!(v["error"]["code"], "E_USAGE", "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("hypotheses"),
        "{v}"
    );
}

// ---------------------------------------------------------------------------
// 11. Review status

/// Changing a reviewed story's content makes it unreviewed again.
#[test]
fn content_edit_resets_review() {
    let r = Repo::new("review-reset");
    let id = land_branch(&r, "feat/a", "a.rs", &["Add a because the queue needs it"]);
    let e = r.ok(&["edit", &id, "--review", "reviewed"]);
    assert_eq!(e["story"]["review"]["status"], "reviewed");
    let e = r.ok(&[
        "edit",
        &id,
        "--add-unknown",
        "Why a and not b is not recorded",
    ]);
    assert_eq!(e["story"]["review"]["status"], "unreviewed", "{e}");
    // Reviewing in the same edit keeps the given status.
    let e = r.ok(&["edit", &id, "--summary", "Add a", "--review", "reviewed"]);
    assert_eq!(e["story"]["review"]["status"], "reviewed");
    // A flag stays until someone clears it.
    r.ok(&["edit", &id, "--review", "flagged"]);
    let e = r.ok(&["edit", &id, "--summary", "Add a, again"]);
    assert_eq!(e["story"]["review"]["status"], "flagged");
}

/// Import refuses a review status it does not know.
#[test]
fn import_refuses_an_unknown_review_status() {
    let r = Repo::new("review-import");
    let id = land_branch(&r, "feat/a", "a.rs", &["Add a because the queue needs it"]);
    let out = r.base.join("export");
    r.ok(&["export", "--out", out.to_str().unwrap()]);
    let path = out.join("gitstory-export.json");
    let mut bundle: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for key in [
        format!("stories/{id}.json"),
        format!("stories/{id}/v1.json"),
    ] {
        bundle["objects"][&key]["review"]["status"] = json!("approved");
    }
    fs::write(&path, serde_json::to_vec_pretty(&bundle).unwrap()).unwrap();
    let other = Repo::new("review-import-b");
    let (code, v) = other.run(&["import", out.to_str().unwrap()]);
    assert_ne!(code, 0, "{v}");
    assert_eq!(v["error"]["code"], "E_USAGE", "{v}");
}

// ---------------------------------------------------------------------------
// 12. Hook uninstall

/// Uninstall finds the hook by its marker, even when the line was edited.
#[test]
fn git_hook_uninstall_removes_an_edited_hook() {
    let r = Repo::new("hook-edit");
    let h = r.ok(&["hooks", "install", "git"]);
    let path = PathBuf::from(h["path"].as_str().unwrap());
    let text = fs::read_to_string(&path).unwrap();
    let edited = text.replace(">/dev/null 2>&1 || true", "|| echo 'draft failed'");
    assert_ne!(edited, text);
    // Something else the user keeps in the same hook.
    fs::write(&path, format!("{edited}echo committed\n")).unwrap();
    let u = r.ok(&["hooks", "uninstall", "git"]);
    assert_eq!(u["changed"], true);
    assert_eq!(u["installed"], false, "{u}");
    let left = fs::read_to_string(&path).unwrap();
    assert!(!left.contains("git story"), "{left}");
    assert!(left.contains("echo committed"), "{left}");
}

// ---------------------------------------------------------------------------
// 13. Trust

fn trust_file(r: &Repo) -> String {
    fs::read_to_string(r.base.join("config/trust.toml")).unwrap_or_default()
}

/// `https://host/v1/` and `https://host/v1` are the same endpoint.
#[test]
fn trust_ignores_a_trailing_slash() {
    let r = Repo::new("trust-slash");
    r.set_model(
        "[model]\nprovider = \"openai\"\nname = \"m\"\nurl = \"https://models.example.com/v1/\"\n",
    );
    assert_eq!(r.ok(&["status"])["model"]["trusted"], false);
    r.ok(&["trust", "https://models.example.com/v1"]);
    assert_eq!(r.ok(&["status"])["model"]["trusted"], true);
    // And the other way round.
    r.set_model(
        "[model]\nprovider = \"openai\"\nname = \"m\"\nurl = \"https://other.example.com/v1\"\n",
    );
    r.ok(&["trust", "https://other.example.com/v1/"]);
    assert_eq!(r.ok(&["status"])["model"]["trusted"], true);
}

/// `init --trust` trusts the validation endpoint too, like `trust` does.
#[test]
fn init_trust_trusts_the_validation_endpoint() {
    let r = Repo::new("trust-init");
    let mut text = fs::read_to_string(r.config_path()).unwrap();
    text.push_str(
        "\n[validation]\nprovider = \"openai\"\nname = \"checker\"\nurl = \"https://check.example.com/v1\"\n",
    );
    fs::write(r.config_path(), text).unwrap();
    r.ok(&["init", "--trust"]);
    let trusted = trust_file(&r);
    assert!(
        trusted.contains("https://check.example.com/v1"),
        "{trusted}"
    );
}
