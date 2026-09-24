//! `hash[s, algo]`, one implementation for both modes: the fast digests
//! for fingerprints, and the password hash.

/// `hash[s, algo]`: a digest in hex (`sha256`, `sha512`, `md5`); a new
/// password hash (`password`: Argon2id, salted, in its PHC string); or `s`
/// hashed the way the password hash `algo` was made — its algorithm, cost
/// and salt — so `hash[attempt, stored] eq stored` checks a password.
pub fn hash(s: &str, algo: &str) -> Result<String, String> {
    use sha2::Digest;
    match algo {
        "sha256" => Ok(format!("{:x}", sha2::Sha256::digest(s.as_bytes()))),
        "sha512" => Ok(format!("{:x}", sha2::Sha512::digest(s.as_bytes()))),
        "md5" => Ok(format!("{:x}", md5::Md5::digest(s.as_bytes()))),
        "password" => password_new(s),
        stored if stored.starts_with('$') => password_again(s, stored),
        other => Err(format!(
            "unsupported hash algorithm '{}' (use sha256, sha512, md5, password, or a stored password hash)",
            other
        )),
    }
}

fn password_new(pw: &str) -> Result<String, String> {
    use argon2::password_hash::{PasswordHasher, SaltString};
    let bytes = crate::signin::random_bytes(16).map_err(|e| format!("hash: {}", e))?;
    let salt = SaltString::encode_b64(&bytes).map_err(|e| format!("hash: {}", e))?;
    argon2::Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("hash: {}", e))
}

fn password_again(pw: &str, stored: &str) -> Result<String, String> {
    use argon2::password_hash::{PasswordHash, PasswordHasher};
    let shown: String = stored.chars().take(24).collect();
    let parsed = PasswordHash::new(stored).map_err(|e| format!("hash: '{}…' is not a password hash ({})", shown, e))?;
    let salt = parsed.salt.ok_or("hash: the stored password hash has no salt")?;
    let params = argon2::Params::try_from(&parsed).map_err(|e| format!("hash: {}", e))?;
    let algorithm = argon2::Algorithm::try_from(parsed.algorithm).map_err(|e| format!("hash: {}", e))?;
    let version = match parsed.version {
        Some(v) => argon2::Version::try_from(v).map_err(|e| format!("hash: {}", e))?,
        None => argon2::Version::default(),
    };
    let again = argon2::Argon2::new(algorithm, version, params.clone())
        .hash_password_customized(pw.as_bytes(), Some(parsed.algorithm), parsed.version, params, salt)
        .map_err(|e| format!("hash: {}", e))?;
    // the same password gives the stored string itself; the hashes are
    // compared in constant time
    if again.hash.is_some() && again.hash == parsed.hash {
        Ok(stored.to_string())
    } else {
        Ok(again.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::hash;

    #[test]
    fn the_fast_digests_are_hex() {
        assert_eq!(hash("hello", "sha256").unwrap(), "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
        assert_eq!(hash("hello", "md5").unwrap(), "5d41402abc4b2a76b9719d911017c592");
        assert!(hash("hello", "crc").unwrap_err().contains("unsupported hash algorithm"));
    }

    #[test]
    fn a_password_checks_against_its_stored_hash() {
        let stored = hash("correct horse", "password").unwrap();
        assert!(stored.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{}", stored);
        // salted: the same password twice is two strings
        assert_ne!(stored, hash("correct horse", "password").unwrap());
        assert_eq!(hash("correct horse", &stored).unwrap(), stored);
        assert_ne!(hash("battery staple", &stored).unwrap(), stored);
        assert!(hash("x", "$not-a-hash").is_err());
    }
}
