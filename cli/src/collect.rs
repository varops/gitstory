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
use crate::workspace::Workspace;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkUnitMeta {
    pub work_unit: String,
    pub created_at: i64,
    /// Branch the work unit follows, when it is a branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// HEAD when collection started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_rev: Option<String>,
    /// Per source file: how many lines were already read.
    #[serde(default)]
    pub offsets: BTreeMap<String, u64>,
    /// Commits this work unit landed as, with the landed story ids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub landed: Vec<(String, String)>,
    /// When it last landed (unix ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landed_at: Option<i64>,
    /// Stories this work explicitly supersedes (`note --supersedes`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<String>,
    /// Last commit of the work, when it landed as a run of commits on main
    /// (`land --since`); Git evidence stops here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip: Option<String>,
    /// Session evidence counts only inside this time window (unix ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<(i64, i64)>,
    /// ghf traces this work unit holds against pruning until it lands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ghf_holds: Vec<String>,
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
            Ok(d) => serde_json::from_slice(&d)?,
            Err(_) => {
                let branch = ws.repo.branch().filter(|b| b == work_unit);
                // On a branch, the work starts where it left main, so commits
                // made before GitStory first saw the branch still count.
                let main = ws.repo.main_ref(ws.cfg.main_ref());
                let base_rev = match (&branch, &main, ws.repo.head()) {
                    (Some(_), Some(m), Some(h)) => ws.repo.merge_base(m, &h).or(Some(h)),
                    (_, _, h) => h,
                };
                let m = WorkUnitMeta {
                    work_unit: work_unit.to_owned(),
                    created_at: gitstory_proto::id::now_ms(),
                    branch,
                    base_rev,
                    offsets: BTreeMap::new(),
                    landed: Vec::new(),
                    landed_at: None,
                    supersedes: Vec::new(),
                    tip: None,
                    window: None,
                    ghf_holds: Vec::new(),
                };
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
    pub fn add_events(&self, events: &[EvidenceEvent]) -> Result<usize> {
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

    pub fn put_draft(&self, s: &Story) -> Result<()> {
        fs::write(
            self.dir.join("drafts").join(format!("{}.json", s.story_id)),
            serde_json::to_vec_pretty(s)?,
        )?;
        Ok(())
    }

    /// Drafts, oldest first.
    pub fn drafts(&self) -> Result<Vec<Story>> {
        let mut out = Vec::new();
        for e in fs::read_dir(self.dir.join("drafts"))?.flatten() {
            if let Ok(s) = serde_json::from_slice::<Story>(&fs::read(e.path())?) {
                out.push(s);
            }
        }
        out.sort_by_key(|s| s.created_at);
        Ok(out)
    }

    pub fn put_egress(&self, m: &EgressManifest) -> Result<()> {
        fs::write(
            self.dir.join("egress").join(format!("{}.json", m.id)),
            serde_json::to_vec_pretty(m)?,
        )?;
        Ok(())
    }

    pub fn remove(self) -> Result<()> {
        fs::remove_dir_all(&self.dir)?;
        Ok(())
    }
}
