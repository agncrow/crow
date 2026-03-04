//! `web.auth` module — JWT creation and verification.
//!
//! ```text
//! use web.auth
//!
//! token = sign[{user, 'alice'}, 'secret']
//! claims = verify[token, 'secret']
//! log[claims.user]  --- 'alice' ---
//! ```

use std::collections::HashMap;

use jsonwebtoken::{
    decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use serde::{Deserialize, Serialize};

/// JWT claims — a flexible map of string key-value pairs plus standard fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrowClaims {
    /// Custom claims data.
    #[serde(flatten)]
    pub data: HashMap<String, serde_json::Value>,
    /// Expiration time (Unix timestamp). Optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<u64>,
    /// Issued at (Unix timestamp). Auto-set on sign.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iat: Option<u64>,
}

/// Sign a JWT token from claims and a secret.
///
/// Returns the encoded JWT string, or an empty string on failure.
pub fn sign(claims: serde_json::Value, secret: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut data: HashMap<String, serde_json::Value> = match claims {
        serde_json::Value::Object(map) => map.into_iter().collect(),
        _ => HashMap::new(),
    };

    // Set iat if not present
    data.entry("iat".to_string())
        .or_insert(serde_json::Value::Number(now.into()));

    // Set default exp (1 hour) if not present
    data.entry("exp".to_string())
        .or_insert(serde_json::Value::Number((now + 3600).into()));

    let crow_claims = CrowClaims {
        exp: data.remove("exp").and_then(|v| v.as_u64()),
        iat: data.remove("iat").and_then(|v| v.as_u64()),
        data,
    };

    encode(
        &Header::default(),
        &crow_claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap_or_default()
}

/// Verify a JWT token and return the claims.
///
/// Returns `None` if the token is invalid or expired.
pub fn verify(token: &str, secret: &str) -> Option<serde_json::Value> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.required_spec_claims.clear();

    let token_data = decode::<CrowClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .ok()?;

    let mut map = serde_json::Map::new();
    for (k, v) in token_data.claims.data {
        map.insert(k, v);
    }
    if let Some(exp) = token_data.claims.exp {
        map.insert("exp".to_string(), serde_json::Value::Number(exp.into()));
    }
    if let Some(iat) = token_data.claims.iat {
        map.insert("iat".to_string(), serde_json::Value::Number(iat.into()));
    }
    Some(serde_json::Value::Object(map))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_and_verify() {
        let token = sign(serde_json::json!({"user": "alice", "role": "admin"}), "secret123");
        assert!(!token.is_empty());

        let claims = verify(&token, "secret123").unwrap();
        assert_eq!(claims["user"], "alice");
        assert_eq!(claims["role"], "admin");
    }

    #[test]
    fn test_verify_wrong_secret() {
        let token = sign(serde_json::json!({"user": "bob"}), "correct");
        assert!(verify(&token, "wrong").is_none());
    }

    #[test]
    fn test_verify_invalid_token() {
        assert!(verify("not.a.token", "secret").is_none());
    }

    #[test]
    fn test_sign_sets_iat_and_exp() {
        let token = sign(serde_json::json!({"x": 1}), "secret");
        let claims = verify(&token, "secret").unwrap();
        assert!(claims["iat"].is_number());
        assert!(claims["exp"].is_number());
    }

    #[test]
    fn test_sign_empty_claims() {
        let token = sign(serde_json::json!({}), "secret");
        assert!(!token.is_empty());
        let claims = verify(&token, "secret").unwrap();
        assert!(claims["iat"].is_number());
    }
}

