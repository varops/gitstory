//! S3-compatible store backend (AWS S3, MinIO, R2, ...), path-style
//! requests signed with AWS Signature Version 4. Credentials come from
//! `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and optional
//! `AWS_SESSION_TOKEN`, never from `.gitstory`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::config::StoreConfig;
use crate::error::Result;
use crate::store::Backend;
use crate::{err, fail};

pub struct S3Backend {
    endpoint: String,
    bucket: String,
    region: String,
    prefix: String,
    access_key: String,
    secret_key: String,
    session_token: Option<String>,
    agent: ureq::Agent,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn sha256(b: &[u8]) -> Vec<u8> {
    Sha256::digest(b).to_vec()
}

fn hmac(key: &[u8], msg: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 64;
    let mut k = if key.len() > BLOCK {
        sha256(key)
    } else {
        key.to_vec()
    };
    k.resize(BLOCK, 0);
    let mut inner: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    inner.extend_from_slice(msg);
    let mut outer: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    outer.extend_from_slice(&sha256(&inner));
    sha256(&outer)
}

/// RFC 3986 encoding as SigV4 wants it; `/` kept when `path` is true.
fn uri_encode(s: &str, path: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if path => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `YYYYMMDDTHHMMSSZ` for a Unix time.
fn amz_date(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Signed headers for one request. `query` is already canonical.
#[allow(clippy::too_many_arguments)]
pub fn sign(
    method: &str,
    host: &str,
    path: &str,
    query: &str,
    payload_hash: &str,
    amzdate: &str,
    region: &str,
    access_key: &str,
    secret_key: &str,
    session_token: Option<&str>,
    extra: &[(&str, &str)],
) -> Vec<(String, String)> {
    let date = &amzdate[..8];
    let mut headers: Vec<(String, String)> = vec![
        ("host".into(), host.into()),
        ("x-amz-content-sha256".into(), payload_hash.into()),
        ("x-amz-date".into(), amzdate.into()),
    ];
    for (k, v) in extra {
        headers.push((k.to_lowercase(), v.trim().to_owned()));
    }
    if let Some(t) = session_token {
        headers.push(("x-amz-security-token".into(), t.into()));
    }
    headers.sort();
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed: String = headers
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let canonical =
        format!("{method}\n{path}\n{query}\n{canonical_headers}\n{signed}\n{payload_hash}");
    let scope = format!("{date}/{region}/s3/aws4_request");
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{amzdate}\n{scope}\n{}",
        hex(&sha256(canonical.as_bytes()))
    );
    let k_date = hmac(format!("AWS4{secret_key}").as_bytes(), date.as_bytes());
    let k_region = hmac(&k_date, region.as_bytes());
    let k_service = hmac(&k_region, b"s3");
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex(&hmac(&k_signing, to_sign.as_bytes()));
    let mut out: Vec<(String, String)> = headers.into_iter().filter(|(k, _)| k != "host").collect();
    out.push((
        "authorization".into(),
        format!("AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed}, Signature={signature}"),
    ));
    out
}

impl S3Backend {
    pub fn from_config(c: &StoreConfig) -> Result<Self> {
        let bucket = c
            .bucket
            .clone()
            .ok_or_else(|| err!(Config, "store.bucket is required"))?;
        let region = c.region.clone().unwrap_or_else(|| "us-east-1".into());
        let endpoint = c
            .endpoint
            .clone()
            .unwrap_or_else(|| format!("https://s3.{region}.amazonaws.com"))
            .trim_end_matches('/')
            .to_owned();
        let access_key = std::env::var("AWS_ACCESS_KEY_ID").map_err(|_| {
            err!(
                Auth,
                "set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY for the s3 store"
            )
        })?;
        let secret_key = std::env::var("AWS_SECRET_ACCESS_KEY")
            .map_err(|_| err!(Auth, "set AWS_SECRET_ACCESS_KEY for the s3 store"))?;
        Ok(Self {
            endpoint,
            bucket,
            region,
            prefix: c
                .prefix
                .clone()
                .unwrap_or_default()
                .trim_matches('/')
                .to_owned(),
            access_key,
            secret_key,
            session_token: std::env::var("AWS_SESSION_TOKEN")
                .ok()
                .filter(|t| !t.is_empty()),
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(60)))
                .http_status_as_error(false)
                .build()
                .into(),
        })
    }

    fn full_key(&self, key: &str) -> String {
        if self.prefix.is_empty() {
            key.to_owned()
        } else {
            format!("{}/{key}", self.prefix)
        }
    }

    fn host(&self) -> String {
        self.endpoint
            .split("://")
            .nth(1)
            .unwrap_or(&self.endpoint)
            .split('/')
            .next()
            .unwrap_or_default()
            .to_owned()
    }

    fn request(
        &self,
        method: &str,
        key: Option<&str>,
        query: &[(&str, String)],
        body: &[u8],
    ) -> Result<(u16, Vec<u8>)> {
        let path = match key {
            Some(k) => format!("/{}/{}", self.bucket, uri_encode(k, true)),
            None => format!("/{}", self.bucket),
        };
        let mut q: Vec<(String, String)> = query
            .iter()
            .map(|(k, v)| (uri_encode(k, false), uri_encode(v, false)))
            .collect();
        q.sort();
        let qs: String = q
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        let payload = hex(&sha256(body));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let headers = sign(
            method,
            &self.host(),
            &path,
            &qs,
            &payload,
            &amz_date(now),
            &self.region,
            &self.access_key,
            &self.secret_key,
            self.session_token.as_deref(),
            &[],
        );
        let url = if qs.is_empty() {
            format!("{}{path}", self.endpoint)
        } else {
            format!("{}{path}?{qs}", self.endpoint)
        };
        let mut resp = match method {
            "PUT" => {
                let mut r = self.agent.put(&url);
                for (k, v) in &headers {
                    r = r.header(k, v);
                }
                r.send(body)
            }
            _ => {
                let mut r = self.agent.get(&url);
                for (k, v) in &headers {
                    r = r.header(k, v);
                }
                r.call()
            }
        }
        .map_err(|e| err!(Remote, "cannot reach {}: {e}", self.endpoint))?;
        let status = resp.status().as_u16();
        let data = resp
            .body_mut()
            .with_config()
            .limit(u64::MAX)
            .read_to_vec()
            .map_err(|e| err!(Remote, "read from {}: {e}", self.endpoint))?;
        if status == 401 || status == 403 {
            fail!(
                Auth,
                "{} refused the credentials (HTTP {status})",
                self.endpoint
            );
        }
        Ok((status, data))
    }
}

fn xml_values<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        let Some(j) = after.find(&close) else { break };
        out.push(&after[..j]);
        rest = &after[j + close.len()..];
    }
    out
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

impl Backend for S3Backend {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let (status, data) = self.request("GET", Some(&self.full_key(key)), &[], b"")?;
        match status {
            200 => Ok(Some(data)),
            404 => Ok(None),
            s => fail!(Remote, "GET {key}: HTTP {s}"),
        }
    }

    fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        let (status, _) = self.request("PUT", Some(&self.full_key(key)), &[], data)?;
        if !(200..300).contains(&status) {
            fail!(Remote, "PUT {key}: HTTP {status}");
        }
        Ok(())
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let full = self.full_key(prefix);
        let strip = if self.prefix.is_empty() {
            String::new()
        } else {
            format!("{}/", self.prefix)
        };
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut q = vec![("list-type", "2".to_owned()), ("prefix", full.clone())];
            if let Some(t) = &token {
                q.push(("continuation-token", t.clone()));
            }
            let (status, data) = self.request("GET", None, &q, b"")?;
            if status != 200 {
                fail!(Remote, "list {prefix}: HTTP {status}");
            }
            let xml = String::from_utf8_lossy(&data).into_owned();
            for k in xml_values(&xml, "Key") {
                let k = xml_unescape(k);
                out.push(k.strip_prefix(&strip).unwrap_or(&k).to_owned());
            }
            match xml_values(&xml, "NextContinuationToken").first() {
                Some(t) if xml.contains("<IsTruncated>true</IsTruncated>") => {
                    token = Some(xml_unescape(t))
                }
                _ => break,
            }
        }
        out.sort();
        Ok(out)
    }

    fn describe(&self) -> String {
        format!("{}/{}/{}", self.endpoint, self.bucket, self.prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked GET example from AWS's SigV4 documentation for S3.
    #[test]
    fn matches_aws_example_signature() {
        let h = sign(
            "GET",
            "examplebucket.s3.amazonaws.com",
            "/test.txt",
            "",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "20130524T000000Z",
            "us-east-1",
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            None,
            &[("range", "bytes=0-9")],
        );
        let auth = &h.iter().find(|(k, _)| k == "authorization").unwrap().1;
        assert!(
            auth.ends_with(
                "Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
            ),
            "{auth}"
        );
    }

    #[test]
    fn hmac_matches_rfc4231_case_2() {
        assert_eq!(
            hex(&hmac(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn dates() {
        assert_eq!(amz_date(1_369_353_600), "20130524T000000Z");
        assert_eq!(amz_date(0), "19700101T000000Z");
    }

    #[test]
    fn parses_list_xml() {
        let x = "<ListBucketResult><Contents><Key>a/b&amp;c</Key></Contents><Contents><Key>d</Key></Contents></ListBucketResult>";
        assert_eq!(xml_values(x, "Key"), vec!["a/b&amp;c", "d"]);
        assert_eq!(xml_unescape("a/b&amp;c"), "a/b&c");
    }
}
