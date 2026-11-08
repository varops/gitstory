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
