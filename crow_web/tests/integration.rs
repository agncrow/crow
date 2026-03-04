//! Integration tests — multi-module scenarios for crow_web.
//!
//! These tests exercise realistic workflows that span multiple modules,
//! e.g. authenticating a user, storing session data, and persisting to a DB.

#[cfg(all(feature = "auth", feature = "session", feature = "db"))]
mod auth_session_db {
    use crow_web::auth;
    use crow_web::db::CrowSqlite;
    use crow_web::session::CrowSessionStore;

    /// Full login flow: authenticate → create session → store user in DB.
    #[test]
    fn login_flow() {
        // 1. Set up DB with a users table
        let db = CrowSqlite::memory().unwrap();
        db.exec(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, role TEXT)",
            &[],
        )
        .unwrap();
        db.exec(
            "INSERT INTO users (name, role) VALUES (?, ?)",
            &[serde_json::json!("alice"), serde_json::json!("admin")],
        )
        .unwrap();

        // 2. Look up user from DB
        let rows = db
            .query("SELECT * FROM users WHERE name = ?", &[serde_json::json!("alice")])
            .unwrap();
        assert_eq!(rows.len(), 1);
        let user_name = rows[0]["name"].as_str().unwrap();
        let user_role = rows[0]["role"].as_str().unwrap();

        // 3. Sign a JWT for the user
        let token = auth::sign(
            serde_json::json!({"user": user_name, "role": user_role}),
            "app-secret",
        );
        assert!(!token.is_empty());

        // 4. Verify the token and extract claims
        let claims = auth::verify(&token, "app-secret").unwrap();
        assert_eq!(claims["user"], "alice");
        assert_eq!(claims["role"], "admin");

        // 5. Create a session and store the user info
        let sessions = CrowSessionStore::new();
        let sid = sessions.create();
        sessions.set(&sid, "user", serde_json::json!(claims["user"]));
        sessions.set(&sid, "role", serde_json::json!(claims["role"]));

        // 6. Verify session data matches DB data
        assert_eq!(
            sessions.get(&sid, "user").unwrap(),
            serde_json::json!("alice")
        );
        assert_eq!(
            sessions.get(&sid, "role").unwrap(),
            serde_json::json!("admin")
        );
    }

    /// Logout flow: destroy session, token still valid but session gone.
    #[test]
    fn logout_flow() {
        let sessions = CrowSessionStore::new();
        let sid = sessions.create();
        sessions.set(&sid, "user", serde_json::json!("bob"));

        let token = auth::sign(serde_json::json!({"user": "bob"}), "secret");

        // Session exists
        assert!(sessions.exists(&sid));

        // Logout: destroy session
        sessions.destroy(&sid);
        assert!(!sessions.exists(&sid));
        assert_eq!(sessions.get(&sid, "user"), None);

        // Token is still cryptographically valid (stateless)
        assert!(auth::verify(&token, "secret").is_some());
    }

    /// DB-backed session persistence: write session data to DB, read it back.
    #[test]
    fn session_persistence_to_db() {
        let db = CrowSqlite::memory().unwrap();
        db.exec(
            "CREATE TABLE sessions (sid TEXT PRIMARY KEY, data TEXT)",
            &[],
        )
        .unwrap();

        let sessions = CrowSessionStore::new();
        let sid = sessions.create();
        sessions.set(&sid, "cart", serde_json::json!(["item1", "item2"]));

        // Serialize session to DB
        let data = sessions.all(&sid).unwrap();
        let json_str = serde_json::to_string(&data).unwrap();
        db.exec(
            "INSERT INTO sessions (sid, data) VALUES (?, ?)",
            &[serde_json::json!(sid), serde_json::json!(json_str)],
        )
        .unwrap();

        // Read back from DB
        let rows = db
            .query(
                "SELECT data FROM sessions WHERE sid = ?",
                &[serde_json::json!(sid)],
            )
            .unwrap();
        assert_eq!(rows.len(), 1);
        let restored: serde_json::Value =
            serde_json::from_str(rows[0]["data"].as_str().unwrap()).unwrap();
        assert_eq!(restored["cart"], serde_json::json!(["item1", "item2"]));
    }
}

#[cfg(all(feature = "auth", feature = "template"))]
mod auth_template {
    use crow_web::auth;
    use crow_web::template::CrowTemplates;

    /// Render a personalized page using JWT claims.
    #[test]
    fn render_authenticated_page() {
        let token = auth::sign(
            serde_json::json!({"user": "carol", "role": "editor"}),
            "key",
        );
        let claims = auth::verify(&token, "key").unwrap();

        let mut tpl = CrowTemplates::new();
        tpl.add("dashboard", "Welcome {{ user }}! Role: {{ role }}")
            .unwrap();
        let html = tpl.render("dashboard", claims).unwrap();
        assert_eq!(html, "Welcome carol! Role: editor");
    }
}

#[cfg(all(feature = "cache", feature = "db"))]
mod cache_db {
    use crow_web::cache::CrowCache;
    use crow_web::db::CrowSqlite;

    /// Cache-aside pattern: check cache first, fall back to DB.
    #[tokio::test]
    async fn cache_aside_pattern() {
        let db = CrowSqlite::memory().unwrap();
        db.exec("CREATE TABLE config (key TEXT PRIMARY KEY, val TEXT)", &[])
            .unwrap();
        db.exec(
            "INSERT INTO config VALUES (?, ?)",
            &[serde_json::json!("theme"), serde_json::json!("dark")],
        )
        .unwrap();

        let cache = CrowCache::new(100);

        // Cache miss → query DB → populate cache
        let key = "theme";
        let cached = cache.get(key).await;
        assert!(cached.is_none());

        let rows = db
            .query(
                "SELECT val FROM config WHERE key = ?",
                &[serde_json::json!(key)],
            )
            .unwrap();
        let db_val = rows[0]["val"].clone();
        cache.set(key, db_val.clone()).await;

        // Cache hit
        assert_eq!(cache.get(key).await, Some(serde_json::json!("dark")));
    }
}

