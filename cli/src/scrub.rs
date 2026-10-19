//! Secret scrubbing (PRD §9.3). Every excerpt passes through here before it
//! is stored or sent. Matches are replaced with `[REDACTED:<kind>]`.

use std::sync::LazyLock;

use regex::Regex;

struct Rule {
    kind: &'static str,
    re: Regex,
    /// Capture group to redact (0 = whole match).
    group: usize,
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    let r = |kind, pat: &str, group| Rule {
        kind,
        re: Regex::new(pat).expect("valid scrub pattern"),
        group,
    };
    vec![
        r(
            "private_key",
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(-----END [A-Z ]*PRIVATE KEY-----|$)",
            0,
        ),
        r("aws_key", r"\b(AKIA|ASIA)[0-9A-Z]{16}\b", 0),
        r(
            "github_token",
            r"\b(gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,})\b",
            0,
        ),
        r("slack_token", r"\bxox[abprs]-[A-Za-z0-9-]{10,}\b", 0),
        r("anthropic_key", r"\bsk-ant-[A-Za-z0-9_-]{20,}\b", 0),
        r("api_key", r"\bsk-[A-Za-z0-9_-]{20,}\b", 0),
        r("stripe_key", r"\b[rs]k_(live|test)_[A-Za-z0-9]{16,}\b", 0),
        r("google_key", r"\bAIza[0-9A-Za-z_-]{35}\b", 0),
        r(
            "jwt",
            r"\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b",
            0,
        ),
        r("ghf_token", r"\bghf_[0-9a-f]{64}\b", 0),
        r(
            "url_credentials",
            r"[a-z][a-z0-9+.-]*://[^/\s:@]+:([^/\s@]+)@",
            1,
        ),
        // KEY=value and key: value assignments of secret-looking names (.env and config).
        r(
            "secret_assignment",
            r#"(?i)\b[A-Z0-9_.-]*(password|passwd|secret|token|api[_-]?key|access[_-]?key|private[_-]?key|credential)[A-Z0-9_.-]*\s*[:=]\s*["']?([^\s"',;]{4,})"#,
            2,
        ),
        r("bearer", r"(?i)\bbearer\s+([A-Za-z0-9._~+/-]{16,}=*)", 1),
    ]
});

static CANDIDATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9+/=_-]{32,}").expect("valid pattern"));

/// Shannon entropy in bits per character.
fn entropy(s: &str) -> f64 {
    let mut counts = [0u32; 256];
    for b in s.bytes() {
        counts[b as usize] += 1;
    }
    let n = s.len() as f64;
    counts
        .iter()
        .filter(|c| **c > 0)
        .map(|c| {
            let p = *c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

/// Looks like a hex digest (commit ids, content hashes), which is not a secret.
fn is_plain_hex(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub redactions: Vec<&'static str>,
}

/// Redact secrets in `text`.
pub fn scrub(text: &str) -> (String, Report) {
    let mut out = text.to_owned();
    let mut report = Report::default();
    for rule in RULES.iter() {
        let mut next = String::with_capacity(out.len());
        let mut last = 0;
        for caps in rule.re.captures_iter(&out) {
            let Some(m) = caps.get(rule.group) else {
                continue;
            };
            if m.as_str().starts_with("[REDACTED") {
                continue;
            }
            next.push_str(&out[last..m.start()]);
            next.push_str(&format!("[REDACTED:{}]", rule.kind));
            last = m.end();
            report.redactions.push(rule.kind);
        }
        next.push_str(&out[last..]);
        out = next;
    }
    // High-entropy tokens that no specific rule caught.
    let mut pieces = Vec::new();
    for m in CANDIDATE.find_iter(&out) {
        let s = m.as_str();
        if !s.starts_with("[REDACTED") && !is_plain_hex(s) && entropy(s) > 4.2 {
            pieces.push((m.start(), m.end()));
        }
    }
    for (start, end) in pieces.into_iter().rev() {
        out.replace_range(start..end, "[REDACTED:high_entropy]");
        report.redactions.push("high_entropy");
    }
    (out, report)
}

/// Scrub, then cap at `max` characters (on a character boundary).
pub fn excerpt(text: &str, max: usize) -> String {
    let (clean, _) = scrub(text.trim());
    if clean.chars().count() <= max {
        return clean;
    }
    let mut s: String = clean.chars().take(max.saturating_sub(1)).collect();
    s.push('…');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_known_formats() {
        let samples = [
            "key AKIAIOSFODNN7EXAMPLE here",
            "token=ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789",
            "ANTHROPIC_API_KEY=sk-ant-api03-abcdefghijklmnopqrstuvwxyz012345",
            "DATABASE_PASSWORD=hunter2hunter2",
            "postgres://admin:s3cr3tpass@db.internal:5432/app",
            "Authorization: Bearer abcdefghijklmnopqrstuvwxyz123456",
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEow\n-----END RSA PRIVATE KEY-----",
        ];
        for s in samples {
            let (out, r) = scrub(s);
            assert!(!r.redactions.is_empty(), "nothing redacted in {s:?}: {out}");
            for secret in [
                "AKIAIOSFODNN7EXAMPLE",
                "ghp_aBcD",
                "sk-ant-api03",
                "hunter2",
                "s3cr3tpass",
                "MIIEow",
            ] {
                assert!(!out.contains(secret), "{secret} leaked in {out}");
            }
        }
    }

    #[test]
    fn redacts_every_occurrence() {
        let (out, _) = scrub(
            "API_KEY=sk-ant-api03-abcdefghijklmnopqrstuvwxyz012345 DB_PASSWORD=hunter2hunter2 TOKEN=abcd1234efgh",
        );
        for secret in ["sk-ant-api03", "hunter2", "abcd1234efgh"] {
            assert!(!out.contains(secret), "{secret} leaked in {out}");
        }
    }

    #[test]
    fn keeps_ordinary_text_and_hashes() {
        let s =
            "Replaced the Redis lock in a83f2c1e4b5d6f708192a3b4c5d6e7f8091a2b3c; 50/50 tests pass";
        assert_eq!(scrub(s).0, s);
    }

    #[test]
    fn caps_excerpts() {
        assert_eq!(excerpt("abcdef", 4), "abc…");
    }
}
