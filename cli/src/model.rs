//! Model providers and the egress gate (PRD §9.3). Nothing else in GitStory
//! makes network requests to a model: every call goes through [`Gate::call`],
//! which enforces the egress policy and trust, and records what was sent.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use gitstory_proto::id;
use gitstory_proto::record::{EgressCall, HunkRef};
use serde_json::{Value, json};

use crate::config::{self, EgressConfig, ModelConfig};
use crate::error::Result;
use crate::{err, fail};

/// One structured request: instructions, input and the JSON schema the
/// answer must match.
pub struct Request {
    pub system: String,
    pub user: String,
    pub schema: Value,
    pub max_tokens: u32,
}

/// What a request carried, for the egress manifest.
pub struct Sent {
    pub purpose: &'static str,
    pub event_ids: Vec<String>,
    pub hunks: Vec<HunkRef>,
    pub excerpts: bool,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(600)))
        .timeout_connect(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .into()
}

fn is_loopback(url: &str) -> bool {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host = match rest.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => rest.split(['/', ':']).next().unwrap_or_default(),
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// Where a provider sends requests, and whether that stays on this machine.
pub fn endpoint(m: &ModelConfig) -> (String, bool) {
    match m.provider.as_str() {
        "anthropic" => {
            let url = m
                .url
                .clone()
                .unwrap_or_else(|| "https://api.anthropic.com".into());
            let local = is_loopback(&url);
            (url, local)
        }
        "openai" => {
            let url = m.url.clone().unwrap_or_default();
            let local = is_loopback(&url);
            (url, local)
        }
        _ => (
            format!("command:{}", m.command.clone().unwrap_or_default()),
            m.local,
        ),
    }
}

pub struct Gate<'a> {
    pub egress: &'a EgressConfig,
    /// Calls made so far (the manifest's body).
    pub calls: Vec<EgressCall>,
}

impl<'a> Gate<'a> {
    pub fn new(egress: &'a EgressConfig) -> Self {
        Self {
            egress,
            calls: Vec::new(),
        }
    }

    /// Check policy and trust, record the call, then send it.
    pub fn call(&mut self, m: &ModelConfig, req: &Request, sent: Sent) -> Result<Value> {
        let (url, local) = endpoint(m);
        if self.egress.inference == "local" && !local {
            fail!(
                EgressDenied,
                "egress.inference = local, but {} is not a local endpoint; nothing was sent",
                url
            );
        }
        if !local && !config::trusted(&url)? {
            fail!(
                UntrustedEndpoint,
                "{url} is not trusted to receive evidence from this machine; run `git story trust {url}`"
            );
        }
        // Recorded before sending: a failed request may still have left.
        self.calls.push(EgressCall {
            purpose: sent.purpose.to_owned(),
            provider: m.provider.clone(),
            endpoint: url.clone(),
            model: m.name.clone(),
            local,
            at: id::now_ms(),
            event_ids: sent.event_ids,
            hunks: sent.hunks,
            excerpts: sent.excerpts,
            bytes: (req.system.len() + req.user.len()) as u64,
        });
        match m.provider.as_str() {
            "anthropic" => anthropic(m, &url, req),
            "openai" => openai(m, &url, req),
            _ => command(m, req),
        }
    }
}

/// Models that accept server-side refusal fallbacks (`fallbacks: "default"`).
fn supports_fallbacks(model: &str) -> bool {
    matches!(
        model,
        "claude-opus-5-5" | "claude-opus-5" | "claude-fable-5-1" | "claude-sonnet-5-5"
    )
}

fn anthropic(m: &ModelConfig, url: &str, req: &Request) -> Result<Value> {
    let mut body = json!({
        "model": m.name,
        "max_tokens": req.max_tokens,
        "system": req.system,
        "messages": [{"role": "user", "content": req.user}],
        "output_config": {
            "effort": m.effort,
            "format": {"type": "json_schema", "schema": req.schema},
        },
    });
    let mut betas = Vec::new();
    if supports_fallbacks(&m.name) {
        body["fallbacks"] = json!("default");
        betas.push("server-side-fallback-2026-07-01");
    }
    let mut r = agent()
        .post(format!("{}/v1/messages", url.trim_end_matches('/')))
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json");
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY")
        && !key.is_empty()
    {
        r = r.header("x-api-key", &key);
    } else if let Ok(tok) = std::env::var("ANTHROPIC_AUTH_TOKEN")
        && !tok.is_empty()
    {
        r = r.header("authorization", &format!("Bearer {tok}"));
        betas.push("oauth-2025-04-20");
    } else {
        fail!(
            Auth,
            "set ANTHROPIC_API_KEY (or ANTHROPIC_AUTH_TOKEN) to use the anthropic provider"
        );
    }
    if !betas.is_empty() {
        r = r.header("anthropic-beta", &betas.join(","));
    }
    let mut resp = r
        .send_json(&body)
        .map_err(|e| err!(Remote, "cannot reach {url}: {e}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .body_mut()
        .read_json()
        .map_err(|e| err!(Remote, "bad response from {url}: {e}"))?;
    if status == 401 || status == 403 {
        fail!(Auth, "{url} rejected the credentials (HTTP {status})");
    }
    if !(200..300).contains(&status) {
        fail!(
            Remote,
            "{url} returned HTTP {status}: {}",
            v["error"]["message"].as_str().unwrap_or("no message")
        );
    }
    match v["stop_reason"].as_str() {
        Some("refusal") => fail!(
            ModelRefused,
            "the model declined ({})",
            v["stop_details"]["category"]
                .as_str()
                .unwrap_or("no category")
        ),
        Some("max_tokens") => fail!(SynthesisInvalid, "the model ran out of output tokens"),
        _ => {}
    }
    let text: String = v["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    parse_json(&text)
}

fn openai(m: &ModelConfig, url: &str, req: &Request) -> Result<Value> {
    let body = json!({
        "model": m.name,
        "messages": [
            {"role": "system", "content": format!("{}\n\nAnswer with one JSON object matching this JSON schema:\n{}", req.system, req.schema)},
            {"role": "user", "content": req.user},
        ],
        "response_format": {"type": "json_object"},
        "max_tokens": req.max_tokens,
    });
    let mut r = agent()
        .post(format!("{}/chat/completions", url.trim_end_matches('/')))
        .header("content-type", "application/json");
    if let Ok(key) = std::env::var("OPENAI_API_KEY")
        && !key.is_empty()
    {
        r = r.header("authorization", &format!("Bearer {key}"));
    }
    let mut resp = r
        .send_json(&body)
        .map_err(|e| err!(Remote, "cannot reach {url}: {e}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .body_mut()
        .read_json()
        .map_err(|e| err!(Remote, "bad response from {url}: {e}"))?;
    if !(200..300).contains(&status) {
        fail!(Remote, "{url} returned HTTP {status}");
    }
    parse_json(
        v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default(),
    )
}

/// Run a program: request JSON on stdin, answer JSON on stdout.
fn command(m: &ModelConfig, req: &Request) -> Result<Value> {
    let cmdline = m.command.clone().unwrap_or_default();
    let mut c = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", &cmdline]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", &cmdline]);
        c
    };
    let mut child = c
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| err!(Remote, "cannot run model command {cmdline:?}: {e}"))?;
    let input = serde_json::to_vec(&json!({
        "model": m.name,
        "system": req.system,
        "user": req.user,
        "schema": req.schema,
    }))?;
    let mut stdin = child.stdin.take().expect("piped");
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let out = child.wait_with_output()?;
    let _ = writer.join();
    if !out.status.success() {
        fail!(
            Remote,
            "model command failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    parse_json(&String::from_utf8_lossy(&out.stdout))
}

/// Parse a JSON object, tolerating surrounding prose or a code fence.
pub fn parse_json(text: &str) -> Result<Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Ok(v);
    }
    let start = t.find('{');
    let end = t.rfind('}');
    if let (Some(s), Some(e)) = (start, end)
        && let Ok(v) = serde_json::from_str::<Value>(&t[s..=e])
    {
        return Ok(v);
    }
    fail!(SynthesisInvalid, "the model did not return JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_detection() {
        assert!(is_loopback("http://localhost:11434/v1"));
        assert!(is_loopback("http://127.0.0.1:8080"));
        assert!(is_loopback("http://[::1]:8080"));
        assert!(!is_loopback("https://api.anthropic.com"));
    }

    #[test]
    fn json_extraction() {
        assert_eq!(parse_json("```json\n{\"a\":1}\n```").unwrap()["a"], 1);
        assert!(parse_json("no json").is_err());
    }
}
