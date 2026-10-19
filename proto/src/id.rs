//! Time-sortable identifiers: `<prefix>_<26 Crockford base32 chars>` (ULID layout).

use std::time::{SystemTime, UNIX_EPOCH};

pub const STORY: &str = "st";
pub const EVENT: &str = "ev";
pub const EGRESS: &str = "egr";
pub const PROPOSAL: &str = "lp";
pub const DRAFT: &str = "dr";

const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// Fill `buf` with OS randomness.
pub fn random_bytes(buf: &mut [u8]) {
    getrandom::fill(buf).expect("OS randomness is available");
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// New identifier with the given prefix, e.g. `ck_01j9…`.
pub fn new_id(prefix: &str) -> String {
    let mut rand = [0u8; 10];
    random_bytes(&mut rand);
    let ms = now_ms() as u128 & ((1 << 48) - 1);
    let mut value: u128 = ms << 80;
    for (i, b) in rand.iter().enumerate() {
        value |= (*b as u128) << (8 * (9 - i));
    }
    let mut out = [0u8; 26];
    for i in (0..26).rev() {
        out[i] = ALPHABET[(value & 31) as usize];
        value >>= 5;
    }
    format!("{prefix}_{}", std::str::from_utf8(&out).expect("ascii"))
}

/// Random hex token of `n` bytes.
pub fn random_hex(n: usize) -> String {
    let mut buf = vec![0u8; n];
    random_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Is `s` a well-formed id with the given prefix?
pub fn is_id(prefix: &str, s: &str) -> bool {
    s.strip_prefix(prefix)
        .and_then(|r| r.strip_prefix('_'))
        .is_some_and(|r| r.len() == 26 && r.bytes().all(|b| ALPHABET.contains(&b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_sort_by_time_and_validate() {
        let a = new_id(STORY);
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = new_id(STORY);
        assert!(a < b);
        assert!(is_id(STORY, &a));
        assert!(!is_id(EVENT, &a));
    }
}
