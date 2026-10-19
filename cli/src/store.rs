//! The story store (PRD §9.10): one format, two backends (a local directory
//! and an S3-compatible bucket). Keys under `repos/<owner>/<name>/`:
//!
//! | Key | Contents |
//! |---|---|
//! | `stories/<id>.json` | Current version of a story |
//! | `stories/<id>/v<N>.json` | Every version, including the current one |
//! | `events/<id>.json` | Cited evidence events (durable parts) |
//! | `egress/<id>.json` | Egress manifests |
//! | `links/<at>-<rand>.json` | Append-only link log, one entry per object |
//! | `proposals/<id>.json` | Link proposals |
//! | `commits/<sha>/<story-id>` | Commit to story association (empty object) |
//!
//! Every object is written whole and never edited in place except proposal
//! status, so the same layout works on plain object storage.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use gitstory_proto::evidence::EvidenceEvent;
use gitstory_proto::record::{EgressManifest, LinkEntry, LinkProposal};
use gitstory_proto::story::Story;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Context, Result};
use crate::{err, fail};

pub trait Backend {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    fn put(&self, key: &str, data: &[u8]) -> Result<()>;
    /// Keys under `prefix`, sorted.
    fn list(&self, prefix: &str) -> Result<Vec<String>>;
    fn describe(&self) -> String;
}

/// A directory on disk. Writes are atomic (temp file, fsync, rename).
pub struct LocalBackend {
    pub root: PathBuf,
}

impl Backend for LocalBackend {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        match fs::read(self.root.join(key)) {
            Ok(d) => Ok(Some(d)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).context(|| format!("read {key}")),
        }
    }

    fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        let path = self.root.join(key);
        let dir = path.parent().expect("key has a parent");
        fs::create_dir_all(dir).context(|| format!("create {}", dir.display()))?;
        let tmp = dir.join(format!(".tmp-{}", gitstory_proto::id::random_hex(6)));
        let mut f = fs::File::create(&tmp).context(|| format!("write {key}"))?;
        f.write_all(data)?;
        f.sync_all()?;
        fs::rename(&tmp, &path).context(|| format!("write {key}"))?;
        Ok(())
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let base = self.root.join(prefix);
        let mut out = Vec::new();
        fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
            let Ok(entries) = fs::read_dir(dir) else {
                return Ok(());
            };
            for e in entries {
                let e = e?;
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with(".tmp-") {
                    continue;
                }
                let p = e.path();
                if e.file_type()?.is_dir() {
                    walk(root, &p, out)?;
                } else if let Ok(rel) = p.strip_prefix(root) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
            Ok(())
        }
        walk(&self.root, &base, &mut out)?;
        out.sort();
        Ok(out)
    }

    fn describe(&self) -> String {
        self.root.display().to_string()
    }
}

/// A repository's slice of the store.
pub struct Store {
    pub backend: Box<dyn Backend>,
    pub repo: String,
}

impl Store {
    fn key(&self, rest: &str) -> String {
        format!("repos/{}/{rest}", self.repo)
    }

    fn get_json<T: DeserializeOwned>(&self, rest: &str) -> Result<Option<T>> {
        match self.backend.get(&self.key(rest))? {
            Some(d) => Ok(Some(
                serde_json::from_slice(&d).map_err(|e| err!(Internal, "corrupt {rest}: {e}"))?,
            )),
            None => Ok(None),
        }
    }

    fn put_json<T: Serialize>(&self, rest: &str, v: &T) -> Result<()> {
        let mut data = serde_json::to_vec_pretty(v)?;
        data.push(b'\n');
        self.backend.put(&self.key(rest), &data)
    }

    fn list_ids(&self, dir: &str, suffix: &str) -> Result<Vec<String>> {
        let prefix = self.key(dir);
        Ok(self
            .backend
            .list(&prefix)?
            .into_iter()
            .filter_map(|k| {
                let rest = k.strip_prefix(&prefix)?.trim_start_matches('/').to_owned();
                (!rest.contains('/'))
                    .then(|| rest.strip_suffix(suffix).map(str::to_owned))
                    .flatten()
            })
            .collect())
    }

    // -- stories ------------------------------------------------------------

    /// Write a new version of a story and associate it with its commit.
    pub fn put_story(&self, s: &Story) -> Result<()> {
        let violations = s.violations();
        if !violations.is_empty() {
            fail!(
                SynthesisInvalid,
                "refusing to store an invalid story: {}",
                violations.join("; ")
            );
        }
        self.put_json(&format!("stories/{}/v{}.json", s.story_id, s.version), s)?;
        self.put_json(&format!("stories/{}.json", s.story_id), s)?;
        for c in std::iter::once(&s.commit).chain(&s.contains) {
            self.backend
                .put(&self.key(&format!("commits/{c}/{}", s.story_id)), b"")?;
        }
        Ok(())
    }

    pub fn story(&self, id: &str) -> Result<Option<Story>> {
        self.get_json(&format!("stories/{id}.json"))
    }

    pub fn story_ids(&self) -> Result<Vec<String>> {
        self.list_ids("stories", ".json")
    }

    #[cfg(test)]
    pub fn stories_for_commit(&self, sha: &str) -> Result<Vec<String>> {
        let prefix = self.key(&format!("commits/{sha}/"));
        Ok(self
            .backend
            .list(&prefix)?
            .into_iter()
            .filter_map(|k| k.strip_prefix(&prefix).map(str::to_owned))
            .collect())
    }

    /// Every commit with an associated story.
    #[cfg(test)]
    pub fn commits(&self) -> Result<Vec<String>> {
        let prefix = self.key("commits/");
        let mut v: Vec<String> = self
            .backend
            .list(&prefix)?
            .into_iter()
            .filter_map(|k| {
                k.strip_prefix(&prefix)
                    .and_then(|r| r.split('/').next())
                    .map(str::to_owned)
            })
            .collect();
        v.dedup();
        Ok(v)
    }

    // -- events, egress -----------------------------------------------------------

    pub fn put_event(&self, e: &EvidenceEvent) -> Result<()> {
        self.put_json(&format!("events/{}.json", e.id), e)
    }

    pub fn event(&self, id: &str) -> Result<Option<EvidenceEvent>> {
        self.get_json(&format!("events/{id}.json"))
    }

    pub fn put_egress(&self, m: &EgressManifest) -> Result<()> {
        self.put_json(&format!("egress/{}.json", m.id), m)
    }

    // -- links, proposals ---------------------------------------------------------

    pub fn append_link(&self, l: &LinkEntry) -> Result<()> {
        self.put_json(
            &format!(
                "links/{:015}-{}.json",
                l.at,
                gitstory_proto::id::random_hex(4)
            ),
            l,
        )
    }

    /// The whole link log, oldest first.
    pub fn links(&self) -> Result<Vec<LinkEntry>> {
        let mut out = Vec::new();
        for name in self.list_ids("links", ".json")? {
            if let Some(l) = self.get_json(&format!("links/{name}.json"))? {
                out.push(l);
            }
        }
        Ok(out)
    }

    pub fn put_proposal(&self, p: &LinkProposal) -> Result<()> {
        self.put_json(&format!("proposals/{}.json", p.id), p)
    }

    pub fn proposal(&self, id: &str) -> Result<Option<LinkProposal>> {
        self.get_json(&format!("proposals/{id}.json"))
    }

    pub fn proposals(&self) -> Result<Vec<LinkProposal>> {
        let mut out = Vec::new();
        for id in self.list_ids("proposals", ".json")? {
            if let Some(p) = self.proposal(&id)? {
                out.push(p);
            }
        }
        Ok(out)
    }

    /// Every object key of this repository (for export).
    pub fn all_keys(&self) -> Result<Vec<String>> {
        let prefix = self.key("");
        Ok(self
            .backend
            .list(&prefix)?
            .into_iter()
            .filter_map(|k| k.strip_prefix(&prefix).map(str::to_owned))
            .collect())
    }

    pub fn get_raw(&self, rest: &str) -> Result<Option<Vec<u8>>> {
        self.backend.get(&self.key(rest))
    }

    pub fn put_raw(&self, rest: &str, data: &[u8]) -> Result<()> {
        self.backend.put(&self.key(rest), data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_backend_lists_and_round_trips() {
        let dir =
            std::env::temp_dir().join(format!("gs-store-{}", gitstory_proto::id::random_hex(4)));
        let b = LocalBackend { root: dir.clone() };
        b.put("repos/a/b/stories/st_1.json", b"{}").unwrap();
        b.put("repos/a/b/stories/st_1/v1.json", b"{}").unwrap();
        b.put("repos/a/b/commits/abc/st_1", b"").unwrap();
        let store = Store {
            backend: Box::new(b),
            repo: "a/b".into(),
        };
        assert_eq!(store.story_ids().unwrap(), vec!["st_1"]);
        assert_eq!(store.stories_for_commit("abc").unwrap(), vec!["st_1"]);
        assert_eq!(store.commits().unwrap(), vec!["abc"]);
        fs::remove_dir_all(dir).unwrap();
    }
}
