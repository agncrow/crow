//! `web.session` module — In-memory cookie-based sessions.
//!
//! ```text
//! use web.session
//!
//! store = sessions[]
//! sid = create[store]
//! set[store, sid, 'user', 'alice']
//! log[get[store, sid, 'user']]  --- 'alice' ---
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// In-memory session store. Thread-safe, shared across handlers.
#[derive(Clone)]
pub struct CrowSessionStore {
    inner: Arc<Mutex<HashMap<String, HashMap<String, serde_json::Value>>>>,
}

impl CrowSessionStore {
    /// Create a new session store.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a new session and return its ID.
    pub fn create(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let mut store = self.inner.lock().unwrap();
        store.insert(id.clone(), HashMap::new());
        id
    }

    /// Set a value in a session.
    pub fn set(&self, session_id: &str, key: &str, value: serde_json::Value) {
        let mut store = self.inner.lock().unwrap();
        if let Some(session) = store.get_mut(session_id) {
            session.insert(key.to_string(), value);
        }
    }

    /// Get a value from a session.
    pub fn get(&self, session_id: &str, key: &str) -> Option<serde_json::Value> {
        let store = self.inner.lock().unwrap();
        store.get(session_id)?.get(key).cloned()
    }

    /// Remove a value from a session.
    pub fn remove(&self, session_id: &str, key: &str) {
        let mut store = self.inner.lock().unwrap();
        if let Some(session) = store.get_mut(session_id) {
            session.remove(key);
        }
    }

    /// Destroy a session entirely.
    pub fn destroy(&self, session_id: &str) {
        let mut store = self.inner.lock().unwrap();
        store.remove(session_id);
    }

    /// Check if a session exists.
    pub fn exists(&self, session_id: &str) -> bool {
        let store = self.inner.lock().unwrap();
        store.contains_key(session_id)
    }

    /// Get all data for a session.
    pub fn all(&self, session_id: &str) -> Option<HashMap<String, serde_json::Value>> {
        let store = self.inner.lock().unwrap();
        store.get(session_id).cloned()
    }
}

impl Default for CrowSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_session() {
        let store = CrowSessionStore::new();
        let id = store.create();
        assert!(!id.is_empty());
        assert!(store.exists(&id));
    }

    #[test]
    fn test_set_and_get() {
        let store = CrowSessionStore::new();
        let id = store.create();
        store.set(&id, "user", serde_json::json!("alice"));
        assert_eq!(store.get(&id, "user"), Some(serde_json::json!("alice")));
    }

    #[test]
    fn test_get_missing_key() {
        let store = CrowSessionStore::new();
        let id = store.create();
        assert_eq!(store.get(&id, "missing"), None);
    }

    #[test]
    fn test_remove_key() {
        let store = CrowSessionStore::new();
        let id = store.create();
        store.set(&id, "key", serde_json::json!(42));
        store.remove(&id, "key");
        assert_eq!(store.get(&id, "key"), None);
    }

    #[test]
    fn test_destroy_session() {
        let store = CrowSessionStore::new();
        let id = store.create();
        store.destroy(&id);
        assert!(!store.exists(&id));
    }

    #[test]
    fn test_all_data() {
        let store = CrowSessionStore::new();
        let id = store.create();
        store.set(&id, "a", serde_json::json!(1));
        store.set(&id, "b", serde_json::json!(2));
        let data = store.all(&id).unwrap();
        assert_eq!(data.len(), 2);
    }

    #[test]
    fn test_get_invalid_session() {
        let store = CrowSessionStore::new();
        assert_eq!(store.get("nonexistent", "key"), None);
        assert!(!store.exists("nonexistent"));
    }
}

