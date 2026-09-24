//! The page's signed-in reader (session-layer-design.md §4.1): the rule
//! both modes check a record against, and the randomness the runtime draws
//! for salts, secrets and tickets.

use std::collections::BTreeMap;

/// The most a reader record may hold, as JSON: it travels in a cookie.
pub const MAX_RECORD: usize = 1024;

/// A reader record fit to sign in: small enough for its cookie.
pub fn check(record: &BTreeMap<String, String>) -> Result<(), String> {
    let size = serde_json::to_string(record).map(|s| s.len()).unwrap_or(usize::MAX);
    if size > MAX_RECORD {
        return Err(format!(
            "sign_in[]: the reader is {} bytes — keep it under 1 KB (an id, a name, a role) and look the rest up",
            size
        ));
    }
    Ok(())
}

/// `n` bytes from the operating system's generator — never the seeded one
/// `rand[]` draws from, which a test can make repeat.
pub fn random_bytes(n: usize) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; n];
    getrandom::fill(&mut out).map_err(|e| format!("no randomness from the system: {}", e))?;
    Ok(out)
}

/// `n` random bytes as hex.
pub fn random_hex(n: usize) -> Result<String, String> {
    Ok(random_bytes(n)?.iter().map(|b| format!("{:02x}", b)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_that_fits_a_cookie_signs_in() {
        let mut r = BTreeMap::new();
        r.insert("id".to_string(), "42".to_string());
        r.insert("name".to_string(), "Ada".to_string());
        assert!(check(&r).is_ok());
        r.insert("notes".to_string(), "x".repeat(2000));
        assert!(check(&r).unwrap_err().contains("under 1 KB"));
    }

    #[test]
    fn random_hex_differs_every_time() {
        let a = random_hex(16).unwrap();
        let b = random_hex(16).unwrap();
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
