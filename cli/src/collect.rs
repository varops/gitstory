//! Per-work-unit collection, local to this machine:
//! `<git-common-dir>/story/collect/<work-unit>/` holds every extracted event
//! (cited or not), drafts and draft egress manifests. Only what a landed
//! story cites goes to the shared store.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use gitstory_proto::evidence::EvidenceEvent;
use gitstory_proto::record::EgressManifest;
use gitstory_proto::story::Story;
use serde::{Deserialize, Serialize};

use crate::error::{Context, Result};
use crate::fail;
use crate::workspace::Workspace;

/// One landing of a work unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Landing {
    pub commit: String,
    pub story: String,
    /// Unix ms.
    pub at: i64,
}

/// Where a work unit is in its lifecycle. Derived from the landings and the
/// last activity, so it cannot disagree with them; only [`WorkUnitMeta::touch`]
/// and [`WorkUnitMeta::record_landing`] move it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Collecting; never landed.
    Open,
    /// Landed, and nothing happened since.
    Landed,
    /// Landed before; new evidence or drafts arrived since.
    Reopened,
}

const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "MetaOnDisk")]
pub struct WorkUnitMeta {
    /// The work unit id as given; the directory name is a lossy slug of it.
    pub work_unit: String,
    pub created_at: i64,
    /// Branch the work unit follows, when it is a branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Where collection starts: HEAD (or the merge base with main) when
    /// collection started, advanced to the landed tip by `land --since`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_rev: Option<String>,
    /// Per source file: how many lines were already read.
    #[serde(default)]
    pub offsets: BTreeMap<String, u64>,
    /// Every landing, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    landed: Vec<Landing>,
    /// Last new evidence, note or draft (unix ms).
    last_activity: i64,
    /// Stories this work explicitly supersedes (`note --supersedes`), until
    /// the next landing applies them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<String>,
    /// ghf traces this work unit holds against pruning until it lands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ghf_holds: Vec<String>,
}

/// Meta as stored, including fields older versions wrote.
#[derive(Deserialize)]
struct MetaOnDisk {
    work_unit: String,
    created_at: i64,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    base_rev: Option<String>,
    #[serde(default)]
    offsets: BTreeMap<String, u64>,
    #[serde(default)]
    landed: Vec<LandingOnDisk>,
    #[serde(default)]
    last_activity: Option<i64>,
    #[serde(default)]
    supersedes: Vec<String>,
    #[serde(default)]
    ghf_holds: Vec<String>,
    /// Written by 0.0.1: the time of the last landing only.
    #[serde(default)]
    landed_at: Option<i64>,
    /// Written by 0.0.1: `land --since` saved its tip (and a session window,
    /// now ignored) for good.
    #[serde(default)]
    tip: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LandingOnDisk {
    Full(Landing),
    /// Written by 0.0.1: `[commit, story]`.
    Pair(String, String),
}

impl From<MetaOnDisk> for WorkUnitMeta {
    fn from(d: MetaOnDisk) -> Self {
        // Old pairs carry no time; the last landing time is the best known.
        let fallback = d.landed_at.unwrap_or(d.created_at);
        let landed: Vec<Landing> = d
            .landed
            .into_iter()
            .map(|l| match l {
                LandingOnDisk::Full(l) => l,
                LandingOnDisk::Pair(commit, story) => Landing {
                    commit,
                    story,
                    at: fallback,
                },
            })
            .collect();
        let last_landing = landed.iter().map(|l| l.at).max();
        let last_activity = d
            .last_activity
            .unwrap_or_else(|| last_landing.unwrap_or(d.created_at).max(d.created_at));
        // A `--since` landing left its tip behind; the next round starts there.
        let base_rev = match d.tip {
            Some(t) if landed.iter().any(|l| l.commit == t) => Some(t),
            _ => d.base_rev,
        };
        Self {
            work_unit: d.work_unit,
            created_at: d.created_at,
            branch: d.branch,
            base_rev,
            offsets: d.offsets,
            landed,
            last_activity,
            supersedes: d.supersedes,
            ghf_holds: d.ghf_holds,
        }
    }
}

impl WorkUnitMeta {
    fn new(work_unit: &str, branch: Option<String>, base_rev: Option<String>) -> Self {
        let now = gitstory_proto::id::now_ms();
        Self {
            work_unit: work_unit.to_owned(),
            created_at: now,
            branch,
            base_rev,
            offsets: BTreeMap::new(),
            landed: Vec::new(),
            last_activity: now,
            supersedes: Vec::new(),
            ghf_holds: Vec::new(),
        }
    }

    pub fn state(&self) -> State {
        match self.landed_at() {
            None => State::Open,
            Some(at) if self.last_activity > at => State::Reopened,
            Some(_) => State::Landed,
        }
    }

    pub fn landings(&self) -> &[Landing] {
        &self.landed
    }

    /// When it last landed.
    pub fn landed_at(&self) -> Option<i64> {
        self.landed.iter().map(|l| l.at).max()
    }

    pub fn last_activity(&self) -> i64 {
        self.last_activity
    }

    pub fn has_landed(&self, commit: &str) -> bool {
        self.landed.iter().any(|l| l.commit == commit)
    }

    /// New evidence, a note or a draft. Reopens a landed work unit.
    pub fn touch(&mut self, at: i64) {
        self.last_activity = self.last_activity.max(at);
    }

    /// Record a landing. Landing the same commit as the same story again
    /// is a no-op; as a different story it is refused.
    pub fn record_landing(&mut self, commit: &str, story: &str, at: i64) -> Result<()> {
        match self.landed.iter().find(|l| l.commit == commit) {
            Some(l) if l.story == story => Ok(()),
            Some(l) => fail!(
                StoreConflict,
                "work unit {} already landed {commit} as {}",
                self.work_unit,
                l.story
            ),
            None => {
                self.landed.push(Landing {
                    commit: commit.to_owned(),
                    story: story.to_owned(),
                    at,
                });
                Ok(())
            }
        }
    }

    /// When `gc` removes the local collection: a day after landing when
    /// nothing happened since, otherwise `drafts.expire` after the last
    /// activity.
    pub fn expires_at(&self, draft_expiry_ms: i64) -> i64 {
        match (self.state(), self.landed_at()) {
            (State::Landed, Some(at)) => at + DAY_MS,
            _ => self.last_activity + draft_expiry_ms,
        }
    }
}

/// What one collection run covers: Git evidence from `base` to `tip`, and
/// session evidence inside `window`. Built per run and never saved, so a
/// `land --since` range narrows that landing only.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    pub base: Option<String>,
    /// Last commit to read; default the work unit's branch, else HEAD.
    pub tip: Option<String>,
    /// Session evidence counts only inside this window (unix ms).
    pub window: Option<(i64, i64)>,
}

impl Scope {
    /// The work unit's own scope: from its base to its branch or HEAD.
    pub fn of(meta: &WorkUnitMeta) -> Self {
        Self {
            base: meta.base_rev.clone(),
            tip: None,
            window: None,
        }
    }
}

pub struct WorkUnit {
    pub dir: PathBuf,
    pub meta: WorkUnitMeta,
}

/// A filesystem-safe directory name for a work unit id.
fn slug(work_unit: &str) -> String {
    work_unit
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn root(ws: &Workspace) -> PathBuf {
    ws.repo.common_dir.join("story").join("collect")
}

/// The work unit commands default to: `$GITSTORY_WORK_UNIT`, else the
/// current branch, else `detached-<short sha>`.
pub fn default_work_unit(ws: &Workspace) -> String {
    if let Ok(w) = std::env::var("GITSTORY_WORK_UNIT")
        && !w.is_empty()
    {
        return w;
    }
    match ws.repo.branch() {
        Some(b) => b,
        None => format!(
            "detached-{}",
            ws.repo
                .head()
                .map(|h| h[..7.min(h.len())].to_owned())
                .unwrap_or_else(|| "none".into())
        ),
    }
}

impl WorkUnit {
    pub fn open(ws: &Workspace, work_unit: &str) -> Result<Self> {
        let dir = root(ws).join(slug(work_unit));
        fs::create_dir_all(dir.join("events")).context(|| format!("create {}", dir.display()))?;
        fs::create_dir_all(dir.join("drafts"))?;
        fs::create_dir_all(dir.join("egress"))?;
        let meta_path = dir.join("meta.json");
        let meta = match fs::read(&meta_path) {
            Ok(d) => {
                let m: WorkUnitMeta = serde_json::from_slice(&d)?;
                if m.work_unit != work_unit {
                    fail!(
                        Usage,
                        "work units {:?} and {:?} share the local directory {}; use another --work-unit",
                        m.work_unit,
                        work_unit,
                        dir.display()
                    );
                }
                m
            }
            Err(_) => {
                let branch = ws.repo.branch().filter(|b| b == work_unit);
                // On a branch, the work starts where it left main, so commits
                // made before GitStory first saw the branch still count.
                let main = ws.repo.main_ref(ws.cfg.main_ref());
                let base_rev = match (&branch, &main, ws.repo.head()) {
                    (Some(_), Some(m), Some(h)) => ws.repo.merge_base(m, &h).or(Some(h)),
                    (_, _, h) => h,
                };
                let m = WorkUnitMeta::new(work_unit, branch, base_rev);
                fs::write(&meta_path, serde_json::to_vec_pretty(&m)?)?;
                m
            }
        };
        Ok(Self { dir, meta })
    }

    /// Every work unit collected on this machine.
    pub fn all(ws: &Workspace) -> Result<Vec<WorkUnit>> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(root(ws)) else {
            return Ok(out);
        };
        for e in entries.flatten() {
            if let Ok(d) = fs::read(e.path().join("meta.json"))
                && let Ok(meta) = serde_json::from_slice::<WorkUnitMeta>(&d)
            {
                out.push(WorkUnit {
                    dir: e.path(),
                    meta,
                });
            }
        }
        out.sort_by_key(|w| w.meta.created_at);
        Ok(out)
    }

    pub fn save_meta(&self) -> Result<()> {
        fs::write(
            self.dir.join("meta.json"),
            serde_json::to_vec_pretty(&self.meta)?,
        )?;
        Ok(())
    }

    /// Add events, skipping any whose source span was already recorded.
    /// Returns how many were new.
    pub fn add_events(&mut self, events: &[EvidenceEvent]) -> Result<usize> {
        let known: std::collections::HashSet<String> = self
            .events()?
            .into_iter()
            .map(|e| e.durable.source_hash)
            .collect();
        let mut n = 0;
        for e in events {
            if known.contains(&e.durable.source_hash) {
                continue;
            }
            fs::write(
                self.dir.join("events").join(format!("{}.json", e.id)),
                serde_json::to_vec_pretty(e)?,
            )?;
            n += 1;
        }
        if n > 0 {
            self.meta.touch(gitstory_proto::id::now_ms());
        }
        Ok(n)
    }

    /// All events, oldest first.
    pub fn events(&self) -> Result<Vec<EvidenceEvent>> {
        let mut out = Vec::new();
        for e in fs::read_dir(self.dir.join("events"))?.flatten() {
            let d = fs::read(e.path())?;
            if let Ok(ev) = serde_json::from_slice::<EvidenceEvent>(&d) {
                out.push(ev);
            }
        }
        out.sort_by(|a, b| (a.durable.at, &a.id).cmp(&(b.durable.at, &b.id)));
        Ok(out)
    }

    pub fn put_draft(&mut self, s: &Story) -> Result<()> {
        fs::write(
            self.dir.join("drafts").join(format!("{}.json", s.story_id)),
            serde_json::to_vec_pretty(s)?,
        )?;
        self.meta.touch(gitstory_proto::id::now_ms());
        Ok(())
    }

    /// Drafts of the current round, oldest first: drafts consumed by a
    /// landing are moved aside, and drafts of an already landed commit are
    /// skipped.
    pub fn drafts(&self) -> Result<Vec<Story>> {
        let mut out = Vec::new();
        for e in fs::read_dir(self.dir.join("drafts"))?.flatten() {
            if !e.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            if let Ok(s) = serde_json::from_slice::<Story>(&fs::read(e.path())?)
                && !self.meta.has_landed(&s.commit)
            {
                out.push(s);
            }
        }
        out.sort_by_key(|s| s.created_at);
        Ok(out)
    }

    /// Close a round after landing `commit`: move the drafts it reconciled
    /// to `drafts/landed/<commit>/`. Drafts written meanwhile stay.
    pub fn consume_drafts(&self, commit: &str, drafts: &[Story]) -> Result<()> {
        let to = self.dir.join("drafts").join("landed").join(commit);
        fs::create_dir_all(&to).context(|| format!("create {}", to.display()))?;
        for d in drafts {
            let name = format!("{}.json", d.story_id);
            let from = self.dir.join("drafts").join(&name);
            if from.exists() {
                fs::rename(&from, to.join(&name)).context(|| format!("move {}", from.display()))?;
            }
        }
        Ok(())
    }

    pub fn put_egress(&mut self, m: &EgressManifest) -> Result<()> {
        fs::write(
            self.dir.join("egress").join(format!("{}.json", m.id)),
            serde_json::to_vec_pretty(m)?,
        )?;
        self.meta.touch(gitstory_proto::id::now_ms());
        Ok(())
    }

    pub fn remove(self) -> Result<()> {
        fs::remove_dir_all(&self.dir)?;
        Ok(())
    }
}
