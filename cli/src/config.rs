//! `.gitstory` (tracked, no secrets) and user configuration (trust,
//! credentials) under `~/.config/gitstory`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Context, Result};
use crate::{err, fail};

pub const FILE_NAME: &str = ".gitstory";
pub const IGNORE_FILE: &str = ".storyignore";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    pub version: u32,
    /// Repository identity in the store, `owner/name`. Defaults to the origin remote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default)]
    pub store: StoreConfig,
    #[serde(default)]
    pub collection: CollectionConfig,
    #[serde(default)]
    pub egress: EgressConfig,
    #[serde(default)]
    pub model: ModelConfig,
    /// Model for the validation pass; defaults to `[model]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ModelConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<GitConfig>,
    #[serde(default)]
    pub drafts: DraftConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreConfig {
    /// `local` or `s3`.
    #[serde(default = "local")]
    pub kind: String,
    /// Local store directory. Relative paths are relative to the Git common
    /// dir; the default is `<git-common-dir>/story/store`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// S3-compatible bucket, endpoint, region and key prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

fn local() -> String {
    "local".into()
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            kind: local(),
            path: None,
            bucket: None,
            endpoint: None,
            region: None,
            prefix: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectionConfig {
    /// Read local Claude Code session logs.
    #[serde(default = "yes")]
    pub claude_code: bool,
    /// Read ghf traces when ghf is installed.
    #[serde(default = "yes")]
    pub ghf: bool,
    /// Maximum characters per event excerpt.
    #[serde(default = "excerpt_cap")]
    pub excerpt_cap: usize,
    /// `on`, or `none` to keep typed facts only.
    #[serde(default = "on")]
    pub excerpts: String,
}

fn yes() -> bool {
    true
}
fn on() -> String {
    "on".into()
}
fn excerpt_cap() -> usize {
    400
}

impl Default for CollectionConfig {
    fn default() -> Self {
        Self {
            claude_code: true,
            ghf: true,
            excerpt_cap: excerpt_cap(),
            excerpts: on(),
        }
    }
}

/// What may leave the machine (PRD §9.3). Raw transcripts and anything the
/// scrubber flags are never sent, whatever this says.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EgressConfig {
    /// `external` or `local` (local endpoints only).
    #[serde(default = "external")]
    pub inference: String,
    #[serde(default = "yes")]
    pub send_events: bool,
    #[serde(default = "yes")]
    pub send_diff: bool,
    #[serde(default = "yes")]
    pub send_excerpts: bool,
}

fn external() -> String {
    "external".into()
}

impl Default for EgressConfig {
    fn default() -> Self {
        Self {
            inference: external(),
            send_events: true,
            send_diff: true,
            send_excerpts: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    /// `anthropic`, `openai` (any OpenAI-compatible endpoint, such as a
    /// local Ollama) or `command` (a program reading the request on stdin).
    #[serde(default = "anthropic")]
    pub provider: String,
    #[serde(default = "default_model")]
    pub name: String,
    /// Base URL override (anthropic, openai).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Program to run (command provider).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Whether a command provider runs entirely on this machine.
    #[serde(default)]
    pub local: bool,
    /// Effort for providers that support it.
    #[serde(default = "high")]
    pub effort: String,
}

fn anthropic() -> String {
    "anthropic".into()
}
fn default_model() -> String {
    "claude-opus-5-5".into()
}
fn high() -> String {
    "high".into()
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: anthropic(),
            name: default_model(),
            url: None,
            command: None,
            local: false,
            effort: high(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftConfig {
    /// Drafts of a work unit that never lands expire after this long.
    #[serde(default = "thirty_days")]
    pub expire: String,
}

fn thirty_days() -> String {
    "30d".into()
}

impl Default for DraftConfig {
    fn default() -> Self {
        Self {
            expire: thirty_days(),
        }
    }
}

impl RepoConfig {
    pub fn new(repository: Option<String>) -> Self {
        Self {
            version: 1,
            repository,
            store: StoreConfig::default(),
            collection: CollectionConfig::default(),
            egress: EgressConfig::default(),
            model: ModelConfig::default(),
            validation: None,
            git: None,
            drafts: DraftConfig::default(),
        }
    }

    pub fn load(root: &Path) -> Result<Option<Self>> {
        let path = root.join(FILE_NAME);
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => fail!(Config, "cannot read {}: {e}", path.display()),
        };
        let cfg: RepoConfig =
            toml::from_str(&text).map_err(|e| err!(Config, "invalid {FILE_NAME}: {e}"))?;
        if cfg.version != 1 {
            fail!(Config, "unsupported {FILE_NAME} version {}", cfg.version);
        }
        cfg.validate()?;
        Ok(Some(cfg))
    }

    fn validate(&self) -> Result<()> {
        if !matches!(self.store.kind.as_str(), "local" | "s3") {
            fail!(Config, "store.kind must be local or s3");
        }
        if self.store.kind == "s3" && self.store.bucket.is_none() {
            fail!(Config, "store.bucket is required for an s3 store");
        }
        if !matches!(self.egress.inference.as_str(), "external" | "local") {
            fail!(Config, "egress.inference must be external or local");
        }
        if !matches!(self.collection.excerpts.as_str(), "on" | "none") {
            fail!(Config, "collection.excerpts must be on or none");
        }
        if !matches!(
            self.model.provider.as_str(),
            "anthropic" | "openai" | "command"
        ) {
            fail!(
                Config,
                "model.provider must be anthropic, openai or command"
            );
        }
        for m in std::iter::once(&self.model).chain(self.validation.as_ref()) {
            if !matches!(
                m.provider.as_str(),
                "anthropic" | "openai" | "command" | "none"
            ) {
                fail!(
                    Config,
                    "validation.provider must be anthropic, openai, command or none"
                );
            }
            if m.provider == "command" && m.command.is_none() {
                fail!(Config, "model.command is required for the command provider");
            }
            if m.provider == "openai" && m.url.is_none() {
                fail!(Config, "model.url is required for the openai provider");
            }
        }
        if ghf_like_duration(&self.drafts.expire).is_none() {
            fail!(Config, "drafts.expire must be a duration such as 30d");
        }
        Ok(())
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("config serializes")
    }

    pub fn main_ref(&self) -> Option<&str> {
        self.git.as_ref().and_then(|g| g.main.as_deref())
    }

    pub fn validation_model(&self) -> &ModelConfig {
        self.validation.as_ref().unwrap_or(&self.model)
    }

    pub fn draft_expiry_ms(&self) -> i64 {
        ghf_like_duration(&self.drafts.expire).unwrap_or(30 * 86_400_000)
    }
}

/// Parse `30d`, `12h`, `15m`, `45s`, `500ms`.
pub fn ghf_like_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    let split = s.find(|c: char| !c.is_ascii_digit())?;
    let (num, unit) = s.split_at(split);
    let n: i64 = num.parse().ok()?;
    let mult = match unit {
        "ms" => 1,
        "s" => 1000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 7 * 86_400_000,
        _ => return None,
    };
    n.checked_mul(mult)
}

// ---------------------------------------------------------------------------
// User configuration

pub fn user_dir() -> Result<PathBuf> {
    if let Ok(d) = std::env::var("GITSTORY_CONFIG_DIR") {
        return Ok(PathBuf::from(d));
    }
    if let Ok(d) = std::env::var("XDG_CONFIG_HOME")
        && !d.is_empty()
    {
        return Ok(PathBuf::from(d).join("gitstory"));
    }
    let home = home_dir().ok_or_else(|| err!(Config, "no home directory"))?;
    Ok(home.join(".config").join("gitstory"))
}

pub fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .or_else(|| std::env::var_os("HOME"))
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

pub fn write_private(path: &Path, contents: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).context(|| format!("create {}", dir.display()))?;
    }
    let tmp = path.with_extension("tmp");
    let mut o = fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o
        .open(&tmp)
        .context(|| format!("write {}", tmp.display()))?;
    f.write_all(contents.as_bytes())?;
    f.sync_all()?;
    fs::rename(&tmp, path).context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TrustFile {
    #[serde(default)]
    endpoints: BTreeMap<String, i64>,
}

/// Is this model endpoint trusted to receive evidence from this machine?
pub fn trusted(endpoint: &str) -> Result<bool> {
    let path = user_dir()?.join("trust.toml");
    let f: TrustFile = match fs::read_to_string(&path) {
        Ok(t) => toml::from_str(&t).map_err(|e| err!(Config, "invalid {}: {e}", path.display()))?,
        Err(_) => TrustFile::default(),
    };
    Ok(f.endpoints.contains_key(endpoint))
}

pub fn trust(endpoint: &str) -> Result<()> {
    let path = user_dir()?.join("trust.toml");
    let mut f: TrustFile = match fs::read_to_string(&path) {
        Ok(t) => toml::from_str(&t).map_err(|e| err!(Config, "invalid {}: {e}", path.display()))?,
        Err(_) => TrustFile::default(),
    };
    f.endpoints
        .insert(endpoint.to_owned(), gitstory_proto::id::now_ms());
    write_private(&path, &toml::to_string(&f).expect("serializes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let c = RepoConfig::new(Some("a/b".into()));
        let back: RepoConfig = toml::from_str(&c.to_toml()).unwrap();
        assert_eq!(back.model.name, "claude-opus-5-5");
        assert_eq!(back.egress.inference, "external");
        back.validate().unwrap();
    }
}
