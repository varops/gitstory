//! Local search index (SQLite FTS5), built from the store and rebuildable at
//! any time (PRD §9.10). Nothing authoritative lives only here: on every
//! open it catches up with whatever the store holds.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use gitstory_proto::record::{LinkKind, LinkOp};
use gitstory_proto::story::{Story, StoryStatus};
use rusqlite::{Connection, params};

use crate::error::Result;
use crate::store::Store;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS stories (
    id TEXT PRIMARY KEY,
    version INTEGER NOT NULL,
    commit_sha TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS paths (
    story_id TEXT NOT NULL,
    path TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS paths_path ON paths(path);
CREATE TABLE IF NOT EXISTS commits (
    sha TEXT NOT NULL,
    story_id TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS commits_sha ON commits(sha);
CREATE VIRTUAL TABLE IF NOT EXISTS story_fts USING fts5(story_id UNINDEXED, body);
"#;

pub struct Index {
    db: Connection,
    /// Story id -> ids of the stories that supersede it (accepted, not removed).
    superseded_by: HashMap<String, Vec<String>>,
    /// Story id -> ids it supersedes.
    supersedes: HashMap<String, Vec<String>>,
    related: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinkGraph {
    /// (from, to) pairs currently in force.
    pub supersedes: BTreeMap<(String, String), ()>,
    pub related: BTreeMap<(String, String), ()>,
}

/// Replay the append-only link log into the links currently in force.
pub fn link_graph(store: &Store) -> Result<LinkGraph> {
    let mut g = LinkGraph::default();
    for l in store.links()? {
        let map = match l.kind {
            LinkKind::Supersedes => &mut g.supersedes,
            LinkKind::Related => &mut g.related,
        };
        match l.op {
            LinkOp::Add => {
                map.insert((l.from, l.to), ());
            }
            LinkOp::Remove => {
                map.remove(&(l.from, l.to));
            }
        }
    }
    Ok(g)
}

fn search_body(s: &Story) -> String {
    let mut parts = vec![s.summary.clone(), s.goal.claim.clone()];
    for (_, c) in s.claims() {
        parts.push(c.claim.clone());
    }
    parts.extend(s.unknowns.iter().cloned());
    parts.extend(s.paths.iter().cloned());
    parts.push(s.work_unit.clone());
    parts.join("\n")
}

impl Index {
    /// Open the index at `path` and bring it up to date with `store`.
    pub fn open(path: &Path, store: &Store) -> Result<Self> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(30))?;
        db.execute_batch(SCHEMA)?;
        let mut idx = Self {
            db,
            superseded_by: HashMap::new(),
            supersedes: HashMap::new(),
            related: HashMap::new(),
        };
        idx.sync(store)?;
        Ok(idx)
    }

    fn sync(&mut self, store: &Store) -> Result<()> {
        let have: HashMap<String, i64> = {
            let mut st = self.db.prepare("SELECT id, version FROM stories")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let ids = store.story_ids()?;
        let present: HashSet<&String> = ids.iter().collect();
        let tx = self.db.unchecked_transaction()?;
        for id in have.keys().filter(|id| !present.contains(id)) {
            tx.execute("DELETE FROM stories WHERE id = ?1", [id])?;
            tx.execute("DELETE FROM paths WHERE story_id = ?1", [id])?;
            tx.execute("DELETE FROM commits WHERE story_id = ?1", [id])?;
            tx.execute("DELETE FROM story_fts WHERE story_id = ?1", [id])?;
        }
        for id in &ids {
            let Some(s) = store.story(id)? else { continue };
            if have.get(id) == Some(&(s.version as i64)) {
                continue;
            }
            tx.execute(
                "INSERT OR REPLACE INTO stories (id, version, commit_sha, created_at, json) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![s.story_id, s.version, s.commit, s.created_at, serde_json::to_string(&s)?],
            )?;
            tx.execute("DELETE FROM commits WHERE story_id = ?1", [id])?;
            for c in std::iter::once(&s.commit).chain(&s.contains) {
                tx.execute(
                    "INSERT INTO commits (sha, story_id) VALUES (?1, ?2)",
                    params![c, id],
                )?;
            }
            tx.execute("DELETE FROM paths WHERE story_id = ?1", [id])?;
            for p in &s.paths {
                tx.execute(
                    "INSERT INTO paths (story_id, path) VALUES (?1, ?2)",
                    params![id, p],
                )?;
            }
            tx.execute("DELETE FROM story_fts WHERE story_id = ?1", [id])?;
            tx.execute(
                "INSERT INTO story_fts (story_id, body) VALUES (?1, ?2)",
                params![id, search_body(&s)],
            )?;
        }
        tx.commit()?;
        let g = link_graph(store)?;
        for (from, to) in g.supersedes.keys() {
            self.superseded_by
                .entry(to.clone())
                .or_default()
                .push(from.clone());
            self.supersedes
                .entry(from.clone())
                .or_default()
                .push(to.clone());
        }
        for (from, to) in g.related.keys() {
            self.related
                .entry(from.clone())
                .or_default()
                .push(to.clone());
            self.related
                .entry(to.clone())
                .or_default()
                .push(from.clone());
        }
        Ok(())
    }

    /// A story with link-derived fields filled in: status, supersedes, related.
    fn hydrate(&self, mut s: Story) -> Story {
        s.status = if self.superseded_by.contains_key(&s.story_id) {
            StoryStatus::Superseded
        } else {
            StoryStatus::Current
        };
        let mut sup = s.supersedes.clone();
        sup.extend(
            self.supersedes
                .get(&s.story_id)
                .cloned()
                .unwrap_or_default(),
        );
        sup.sort();
        sup.dedup();
        s.supersedes = sup;
        let mut rel = s.related.clone();
        rel.extend(self.related.get(&s.story_id).cloned().unwrap_or_default());
        rel.sort();
        rel.dedup();
        s.related = rel;
        s
    }

    fn load(&self, sql: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<Story>> {
        let mut st = self.db.prepare(sql)?;
        let rows = st.query_map(p, |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for j in rows {
            let s: Story = serde_json::from_str(&j?)?;
            out.push(self.hydrate(s));
        }
        Ok(out)
    }

    pub fn story(&self, id: &str) -> Result<Option<Story>> {
        Ok(self
            .load("SELECT json FROM stories WHERE id = ?1", &[&id])?
            .into_iter()
            .next())
    }

    pub fn ids_with_prefix(&self, prefix: &str) -> Result<Vec<String>> {
        let mut st = self
            .db
            .prepare("SELECT id FROM stories WHERE substr(id, 1, length(?1)) = ?1 LIMIT 3")?;
        let rows = st.query_map([prefix], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn for_commits(&self, shas: &[String]) -> Result<Vec<Story>> {
        let mut out: Vec<Story> = Vec::new();
        for sha in shas {
            for s in self.load(
                "SELECT s.json FROM stories s JOIN commits c ON c.story_id = s.id WHERE c.sha = ?1 ORDER BY s.created_at",
                &[sha],
            )? {
                if !out.iter().any(|o| o.story_id == s.story_id) {
                    out.push(s);
                }
            }
        }
        Ok(out)
    }

    pub fn for_path(&self, path: &str) -> Result<Vec<Story>> {
        let like = format!("{}/%", path.trim_end_matches('/'));
        self.load(
            "SELECT DISTINCT s.json FROM stories s JOIN paths p ON p.story_id = s.id WHERE p.path = ?1 OR p.path LIKE ?2 ORDER BY s.created_at DESC",
            &[&path, &like],
        )
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Story>> {
        // Quote each term so user text is never parsed as FTS syntax.
        let q: String = query
            .split_whitespace()
            .map(|t| format!("\"{}\"", t.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" OR ");
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let mut st = self.db.prepare(
            "SELECT s.json FROM story_fts f JOIN stories s ON s.id = f.story_id WHERE story_fts MATCH ?1 ORDER BY bm25(story_fts) LIMIT ?2",
        )?;
        let rows = st.query_map(params![q, limit as i64], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for j in rows {
            out.push(self.hydrate(serde_json::from_str(&j?)?));
        }
        Ok(out)
    }

    /// Stories that directly supersede `id`.
    pub fn successors(&self, id: &str) -> Vec<String> {
        self.superseded_by.get(id).cloned().unwrap_or_default()
    }

    /// Stories `id` directly supersedes.
    pub fn predecessors(&self, id: &str) -> Vec<String> {
        self.supersedes.get(id).cloned().unwrap_or_default()
    }
}
