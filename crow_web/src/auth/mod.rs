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

/// A secret shorter than this is refused: a token signed with one is
/// guessed (the sign-in cookie's `CROW_SECRET` keeps the same floor).
const SECRET_FLOOR: usize = 32;

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// `sign[claims, secret]`: a token — a JSON Web Token signed with HS256 —
/// carrying the claims (a map or a record of them), issued now (`iat`) and
/// expiring in an hour (`exp`) unless the claims say when. A secret shorter
/// than 32 characters is refused, and claims that are not a map.
pub fn sign_claims(claims: &serde_json::Value, secret: &str) -> Result<String, String> {
    if secret.chars().count() < SECRET_FLOOR {
        return Err(format!("a token's secret is {} characters or more (got {})", SECRET_FLOOR, secret.chars().count()));
    }
    let serde_json::Value::Object(map) = claims else {
        return Err("a token's claims are a map or a record of them".to_string());
    };
    let mut data: HashMap<String, serde_json::Value> = map.clone().into_iter().collect();
    let issued = now();
    data.entry("iat".to_string()).or_insert(serde_json::Value::Number(issued.into()));
    data.entry("exp".to_string()).or_insert(serde_json::Value::Number((issued + 3600).into()));
    let crow_claims = CrowClaims {
        exp: data.remove("exp").and_then(|v| v.as_f64()).map(|x| x as u64),
        iat: data.remove("iat").and_then(|v| v.as_f64()).map(|x| x as u64),
        data,
    };
    encode(&Header::default(), &crow_claims, &EncodingKey::from_secret(secret.as_bytes())).map_err(|e| format!("cannot sign the token: {}", e))
}

/// `verify[token, secret]`: the claims a token carries, when it was signed
/// with this secret and has not expired; otherwise why not — the token has
/// expired, its signature does not match, it is not a token.
pub fn verify_token(token: &str, secret: &str) -> Result<serde_json::Value, String> {
    use jsonwebtoken::errors::ErrorKind;
    let mut validation = Validation::new(Algorithm::HS256);
    validation.required_spec_claims.clear();
    validation.leeway = 0;
    let token_data = decode::<CrowClaims>(token, &DecodingKey::from_secret(secret.as_bytes()), &validation).map_err(|e| match e.kind() {
        ErrorKind::ExpiredSignature => "the token has expired".to_string(),
        ErrorKind::InvalidSignature => "the token's signature does not match".to_string(),
        ErrorKind::InvalidAlgorithm => "the token is not signed with HS256".to_string(),
        _ => "not a token".to_string(),
    })?;
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
    Ok(serde_json::Value::Object(map))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "a-secret-of-at-least-thirty-two-chars";

    #[test]
    fn a_token_carries_its_claims() {
        let token = sign_claims(&serde_json::json!({"user": "alice", "role": "admin"}), SECRET).unwrap();
        let claims = verify_token(&token, SECRET).unwrap();
        assert_eq!(claims["user"], "alice");
        assert_eq!(claims["role"], "admin");
        assert!(claims["iat"].is_number() && claims["exp"].is_number());
    }

    #[test]
    fn a_token_is_refused_with_its_reason() {
        let token = sign_claims(&serde_json::json!({"user": "bob"}), SECRET).unwrap();
        assert_eq!(verify_token(&token, "another-secret-of-thirty-two-chars!!"), Err("the token's signature does not match".to_string()));
        assert_eq!(verify_token("not.a.token", SECRET), Err("not a token".to_string()));
        let old = sign_claims(&serde_json::json!({"user": "bob", "exp": 1_000}), SECRET).unwrap();
        assert_eq!(verify_token(&old, SECRET), Err("the token has expired".to_string()));
    }

    #[test]
    fn a_short_secret_or_claims_that_are_not_a_map_are_refused() {
        assert!(sign_claims(&serde_json::json!({"x": 1}), "short").unwrap_err().contains("32 characters or more (got 5)"));
        assert!(sign_claims(&serde_json::json!([1, 2]), SECRET).unwrap_err().contains("a map or a record"));
    }
}
