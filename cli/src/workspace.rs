//! A GitStory-enabled repository: Git, `.gitstory`, and its store.

use std::path::{Path, PathBuf};

use crate::config::{self, RepoConfig};
use crate::error::Result;
use crate::git::{self, Repo};
use crate::store::{LocalBackend, Store};
use crate::{err, fail};

pub struct Workspace {
    pub repo: Repo,
    pub cfg: RepoConfig,
    pub store: Store,
}

/// `owner/name` for this repository: `.gitstory`, else the origin remote,
/// else `local/<directory name>`.
pub fn repo_id(repo: &Repo, cfg: &RepoConfig) -> String {
    cfg.repository
        .clone()
        .or_else(|| {
            repo.remote_url("origin")
                .and_then(|u| git::repo_name_from_url(&u))
        })
        .unwrap_or_else(|| {
            let name = repo
                .root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "repo".into());
            let clean: String = name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || "._-".contains(c) {
                        c
                    } else {
                        '-'
                    }
                })
                .collect();
            format!("local/{clean}")
        })
}

pub fn open_store(repo: &Repo, cfg: &RepoConfig) -> Result<Store> {
    let id = repo_id(repo, cfg);
    let backend: Box<dyn crate::store::Backend> = match cfg.store.kind.as_str() {
        "s3" => Box::new(crate::s3::S3Backend::from_config(&cfg.store)?),
        _ => {
            let root = match &cfg.store.path {
                Some(p) if Path::new(p).is_absolute() => PathBuf::from(p),
                Some(p) => repo.common_dir.join(p),
                None => repo.common_dir.join("story").join("store"),
            };
            Box::new(LocalBackend { root })
        }
    };
    Ok(Store { backend, repo: id })
}

impl Workspace {
    pub fn open(cwd: &Path) -> Result<Self> {
        let repo = Repo::discover(cwd)?;
        let Some(cfg) = RepoConfig::load(&repo.root)? else {
            fail!(
                NotInitialized,
                "no {} in {}; run `git story init`",
                config::FILE_NAME,
                repo.root.display()
            );
        };
        let store = open_store(&repo, &cfg)?;
        Ok(Self { repo, cfg, store })
    }

    pub fn index_path(&self) -> PathBuf {
        self.repo.common_dir.join("story").join("index.db")
    }

    pub fn index(&self) -> Result<crate::index::Index> {
        crate::index::Index::open(&self.index_path(), &self.store)
    }

    /// Who is acting: `$GITSTORY_AUTHOR`, else Git's user name.
    pub fn author(&self) -> String {
        if let Ok(a) = std::env::var("GITSTORY_AUTHOR")
            && !a.is_empty()
        {
            return a;
        }
        self.repo
            .run(["config", "user.name"])
            .ok()
            .map(|b| String::from_utf8_lossy(&b).trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "unknown".into())
    }

    pub fn require_commit(&self, rev: &str) -> Result<String> {
        self.repo
            .resolve_commit(rev)
            .ok_or_else(|| err!(CommitNotFound, "commit {rev} does not exist locally"))
    }
}
