//! Integration tests — multi-module scenarios for crow_web.
//!
//! These tests exercise workflows that span a module and the server: a
//! route answered, CORS, static files, uploads, a live page, mail.

// ── Core: request/response standalone ────────────────────────────────

mod core_types {
    use crow_web::core::{CrowRequest, CrowResponse};

    #[test]
    fn request_builder_round_trip() {
        let mut req = CrowRequest::new();
        req.method = "POST".to_string();
        req.path = "/api/users".to_string();
        req.body = r#"{"name":"alice"}"#.to_string();
        req.headers.insert("content-type".to_string(), "application/json".to_string());
        req.query = CrowRequest::parse_query_string("page=1&limit=20");
        req.params.insert("id".to_string(), "7".to_string());

        assert_eq!(req.method, "POST");
        assert_eq!(req.header("Content-Type"), Some("application/json"));
        assert_eq!(req.query_param("page"), Some("1"));
        assert_eq!(req.param("id"), Some("7"));
        let json = req.json().unwrap();
        assert_eq!(json["name"], "alice");
    }

    #[test]
    fn response_chaining() {
        let resp = CrowResponse::text("ok")
            .with_header("x-request-id", "abc123")
            .with_header("x-version", "1.0");
        assert!(resp.is_ok());
        assert_eq!(resp.headers.get("x-request-id").unwrap(), "abc123");
        assert_eq!(resp.headers.get("x-version").unwrap(), "1.0");
        assert_eq!(resp.body, "ok");
    }

    #[test]
    fn response_variants_status_codes() {
        assert_eq!(CrowResponse::text("t").status, 200);
        assert_eq!(CrowResponse::html("<b>h</b>").status, 200);
        assert_eq!(CrowResponse::json(serde_json::json!(1)).status, 200);
        assert_eq!(CrowResponse::redirect("/x").status, 302);
        assert_eq!(CrowResponse::respond(503, "down").status, 503);
    }
}

// ── Server: HTTP routing integration ─────────────────────────────────

#[cfg(feature = "http")]
mod server_integration {
    use crow_web::core::{CrowRequest, CrowResponse};
    use crow_web::server::{CrowApp, CrowHandler};
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt;

    fn json_handler(req: CrowRequest) -> CrowResponse {
        let name = req.param("name").unwrap_or("world").to_string();
        CrowResponse::json(serde_json::json!({"hello": name}))
    }

    fn echo_body(req: CrowRequest) -> CrowResponse {
        CrowResponse::text(req.body.clone())
    }

    async fn send(router: axum::Router, req: HttpRequest<Body>) -> axum::http::Response<Body> {
        router.oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn multiple_routes_and_methods() {
        let app = CrowApp::new()
            .route("/greet/:name", CrowHandler::new("json_handler", json_handler))
            .route_method("/echo", "POST", CrowHandler::new("echo_body", echo_body));

        let router = app.into_router();

        // Test parameterized GET
        let req = HttpRequest::builder()
            .uri("/greet/crow")
            .body(Body::empty())
            .unwrap();
        let resp = send(router.clone(), req).await;
        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["hello"], "crow");

        // Test POST echo
        let req = HttpRequest::builder()
            .method("POST")
            .uri("/echo")
            .body(Body::from("ping"))
            .unwrap();
        let resp = send(router.clone(), req).await;
        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, "ping");
    }

    #[tokio::test]
    async fn redirect_and_custom_status() {
        let app = CrowApp::new()
            .route("/old", CrowHandler::new("old", |_| CrowResponse::redirect("/new")))
            .route("/gone", CrowHandler::new("gone", |_| CrowResponse::respond(410, "gone")));

        let router = app.into_router();

        let resp = send(router.clone(),
            HttpRequest::builder().uri("/old").body(Body::empty()).unwrap()
        ).await;
        assert_eq!(resp.status(), 302);
        assert_eq!(resp.headers().get("location").unwrap(), "/new");

        let resp = send(router.clone(),
            HttpRequest::builder().uri("/gone").body(Body::empty()).unwrap()
        ).await;
        assert_eq!(resp.status(), 410);
    }
}

// ── Server options: CORS, files, rate limits, forms and uploads ─────

#[cfg(feature = "http")]
mod server_options {
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use crow_web::core::{CrowRequest, CrowResponse};
    use crow_web::server::{CrowApp, CrowHandler};
    use tower::ServiceExt;

    fn ok(_: CrowRequest) -> CrowResponse {
        CrowResponse::text("ok")
    }

    async fn body_of(resp: axum::http::Response<Body>) -> String {
        String::from_utf8_lossy(&axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap()).to_string()
    }

    /// `serve[…, cors = [origin]]`: a page on that site may call the app —
    /// its preflight is answered and its request carries the origin back —
    /// and a page on another is told nothing
    #[tokio::test]
    async fn cors_names_the_sites_that_may_call() {
        let router = CrowApp::new().route("/api", CrowHandler::new("ok", ok)).cors_origins(&["https://app.example".to_string()]).served_router();
        let pre = router.clone().oneshot(HttpRequest::builder().method("OPTIONS").uri("/api").header("origin", "https://app.example").header("access-control-request-method", "GET").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(pre.headers().get("access-control-allow-origin").unwrap(), "https://app.example");
        let other = router.clone().oneshot(HttpRequest::builder().uri("/api").header("origin", "https://evil.example").body(Body::empty()).unwrap()).await.unwrap();
        assert!(other.headers().get("access-control-allow-origin").is_none());
        let any = CrowApp::new().route("/api", CrowHandler::new("ok", ok)).cors_origins(&["*".to_string()]).served_router();
        let resp = any.oneshot(HttpRequest::builder().uri("/api").header("origin", "https://elsewhere.example").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.headers().get("access-control-allow-origin").unwrap(), "*");
    }

    /// `serve[…, files = dir]`: an address no route answers is the
    /// directory's file, a route wins over a file, and a file there is not
    /// is the app's 404
    #[tokio::test]
    async fn files_answer_what_no_route_does() {
        let dir = std::env::temp_dir().join(format!("crow_files_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("logo.txt"), "a logo").unwrap();
        std::fs::write(dir.join("api"), "shadowed").unwrap();
        let router = CrowApp::new().route("/api", CrowHandler::new("ok", ok)).files_from(dir.to_str().unwrap()).served_router();
        let file = router.clone().oneshot(HttpRequest::builder().uri("/logo.txt").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(file.status(), 200);
        assert_eq!(body_of(file).await, "a logo");
        let route = router.clone().oneshot(HttpRequest::builder().uri("/api").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(body_of(route).await, "ok");
        let missing = router.clone().oneshot(HttpRequest::builder().uri("/nope.txt").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(missing.status(), 404);
        assert!(body_of(missing).await.contains("no page at /nope.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file or directory named with a dot is no file the app serves —
    /// `files = '.'` had handed out `.env` and the key the sign-in cookies
    /// are sealed with — however the path spells the dot; `.well-known`,
    /// where a certificate authority looks, is served
    #[tokio::test]
    async fn files_named_with_a_dot_are_not_served() {
        let dir = std::env::temp_dir().join(format!("crow_dotfiles_{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".crow/keys")).unwrap();
        std::fs::create_dir_all(dir.join(".well-known/acme-challenge")).unwrap();
        std::fs::write(dir.join(".env"), "SECRET=1").unwrap();
        std::fs::write(dir.join(".crow/keys/secret"), "key").unwrap();
        std::fs::write(dir.join(".well-known/acme-challenge/token"), "proof").unwrap();
        std::fs::write(dir.join("index.html"), "home").unwrap();
        let router = CrowApp::new().route("/api", CrowHandler::new("ok", ok)).files_from(dir.to_str().unwrap()).served_router();
        for hidden in ["/.env", "/.crow/keys/secret", "/%2eenv", "/%2Ecrow/keys/secret", "/sub/../.env"] {
            let resp = router.clone().oneshot(HttpRequest::builder().uri(hidden).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(resp.status(), 404, "{} was served", hidden);
            assert!(!body_of(resp).await.contains("SECRET"), "{}", hidden);
        }
        let proof = router.clone().oneshot(HttpRequest::builder().uri("/.well-known/acme-challenge/token").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(proof.status(), 200);
        assert_eq!(body_of(proof).await, "proof");
        let home = router.clone().oneshot(HttpRequest::builder().uri("/index.html").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(body_of(home).await, "home");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `route[…, limit = 2]`: a client's third request in the minute is
    /// answered 429 with the seconds to wait
    #[tokio::test]
    async fn a_route_limits_each_client() {
        let router = CrowApp::new().route_limited("/api", "GET", CrowHandler::new("ok", ok), 2).served_router();
        let ask = || HttpRequest::builder().uri("/api").header("x-forwarded-for", "198.51.100.7").body(Body::empty()).unwrap();
        assert_eq!(router.clone().oneshot(ask()).await.unwrap().status(), 200);
        assert_eq!(router.clone().oneshot(ask()).await.unwrap().status(), 200);
        let third = router.clone().oneshot(ask()).await.unwrap();
        assert_eq!(third.status(), 429);
        assert!(third.headers().get("retry-after").is_some());
        let other = HttpRequest::builder().uri("/api").header("x-forwarded-for", "198.51.100.8").body(Body::empty()).unwrap();
        assert_eq!(router.clone().oneshot(other).await.unwrap().status(), 200);
    }

    /// A form's fields and a multipart body's files reach the handler —
    /// each file a copy it reads by its path, gone once it has answered —
    /// and an urlencoded body's fields are decoded
    #[tokio::test]
    async fn a_form_and_its_files_reach_the_handler() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let kept = seen.clone();
        let router = CrowApp::new()
            .route_method("/up", "POST", CrowHandler::new("up", move |req: CrowRequest| {
                let f = &req.files[0];
                let text = std::fs::read_to_string(&f["path"]).unwrap_or_default();
                *kept.lock().unwrap() = f["path"].clone();
                CrowResponse::text(format!("{} {} {} {} {} | {}", req.form["title"], f["field"], f["name"], f["type"], f["size"], text))
            }))
            .route_method("/form", "POST", CrowHandler::new("form", |req: CrowRequest| CrowResponse::text(req.form["q"].to_string())))
            .served_router();
        let body = "--XYZ\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nQ3 trades\r\n--XYZ\r\nContent-Disposition: form-data; name=\"sheet\"; filename=\"t.csv\"\r\nContent-Type: text/csv\r\n\r\na,b\n1,2\n\r\n--XYZ--\r\n";
        let resp = router.clone().oneshot(HttpRequest::builder().method("POST").uri("/up").header("content-type", "multipart/form-data; boundary=XYZ").body(Body::from(body)).unwrap()).await.unwrap();
        assert_eq!(body_of(resp).await, "Q3 trades sheet t.csv text/csv 8 | a,b\n1,2\n");
        let path = seen.lock().unwrap().clone();
        assert!(path.starts_with(".crow/uploads/") && path.ends_with(".csv"), "{}", path);
        assert!(!std::path::Path::new(&path).exists(), "the copy is gone once the handler has answered");
        let resp = router.clone().oneshot(HttpRequest::builder().method("POST").uri("/form").header("content-type", "application/x-www-form-urlencoded").body(Body::from("q=hello+world%21")).unwrap()).await.unwrap();
        assert_eq!(body_of(resp).await, "hello world!");
    }

    /// A body bigger than the app takes is refused before the handler
    #[tokio::test]
    async fn a_body_too_big_is_refused() {
        crow_web::server::configure_uploads(1);
        let router = CrowApp::new().route_method("/up", "POST", CrowHandler::new("ok", ok)).served_router();
        let big = vec![b'x'; 2 * 1024 * 1024];
        let resp = router.oneshot(HttpRequest::builder().method("POST").uri("/up").body(Body::from(big)).unwrap()).await.unwrap();
        crow_web::server::configure_uploads(10);
        assert_eq!(resp.status(), 413);
    }
}

// ── Live: state management + HTML diffing ───────────────────────────

#[cfg(feature = "live")]
mod live_integration {
    use crow_web::live::{CrowLiveState, live_js};

    #[test]
    fn live_state_get_set_overwrite() {
        let mut state = CrowLiveState::new();
        state.set("count", serde_json::json!(0));
        assert_eq!(state.get("count"), Some(serde_json::json!(0)));

        state.set("count", serde_json::json!(5));
        assert_eq!(state.get("count"), Some(serde_json::json!(5)));

        assert!(state.get("missing").is_none());
    }

    #[test]
    fn live_state_register_fetch_poll() {
        let mut state = CrowLiveState::new();
        state.register_fetch("data", "http://localhost/api/data");
        state.register_poll("prices", "http://localhost/api/prices", 5);
        assert_eq!(state.sources.len(), 2);
        assert!(state.sources[0].interval_secs.is_none());
        assert_eq!(state.sources[1].interval_secs, Some(5));
    }

    #[test]
    fn live_js_contains_websocket_setup() {
        let js = live_js("/ws/counter");
        assert!(js.contains("WebSocket"));
        assert!(js.contains("/ws/counter"));
        assert!(js.contains("data-crow-click"));
        assert!(js.contains("data-crow-input"));
        assert!(js.contains("data-crow-change"));
        assert!(js.contains("onmessage"));
    }
}


// ── View: VNode diffing + components ────────────────────────────────

mod view_integration {
    use crow_web::view::{CrowComponent, CrowVNode};
    use std::collections::HashMap;

    #[test]
    fn vnode_complex_tree_diff() {
        let old = CrowVNode::el("div", HashMap::new(), vec![
            CrowVNode::el("h1", HashMap::new(), vec![CrowVNode::text("Title")]),
            CrowVNode::el("ul", HashMap::new(), vec![
                CrowVNode::el("li", HashMap::new(), vec![CrowVNode::text("a")]),
                CrowVNode::el("li", HashMap::new(), vec![CrowVNode::text("b")]),
            ]),
        ]);
        let new = CrowVNode::el("div", HashMap::new(), vec![
            CrowVNode::el("h1", HashMap::new(), vec![CrowVNode::text("Title")]),
            CrowVNode::el("ul", HashMap::new(), vec![
                CrowVNode::el("li", HashMap::new(), vec![CrowVNode::text("a")]),
                CrowVNode::el("li", HashMap::new(), vec![CrowVNode::text("b")]),
                CrowVNode::el("li", HashMap::new(), vec![CrowVNode::text("c")]),
            ]),
        ]);

        let patches = CrowVNode::changes(&old, &new);
        assert!(patches.iter().any(|p| p["op"] == "insert" && p["index"] == 2), "{:?}", patches);
        let mut page = old.clone();
        page.apply(&patches).expect("the changes fit the page");
        assert_eq!(page.to_html(), new.to_html());
    }

    #[test]
    fn vnode_to_html_nested_with_attrs() {
        let node = CrowVNode::el(
            "a",
            HashMap::from([("href".to_string(), "/home".to_string())]),
            vec![CrowVNode::text("Home")],
        );
        let html = node.to_html();
        assert!(html.contains("<a"));
        assert!(html.contains("href=\"/home\""));
        assert!(html.contains("Home"));
        assert!(html.contains("</a>"));
    }

    #[test]
    fn vnode_html_escapes_xss() {
        let node = CrowVNode::text("<script>alert('xss')</script>");
        let html = node.to_html();
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn component_builder_full() {
        let comp = CrowComponent::new("todo")
            .state("items", serde_json::json!([]))
            .state("filter", serde_json::json!("all"))
            .on("add")
            .on("remove")
            .on("toggle");

        assert_eq!(comp.name, "todo");
        assert_eq!(comp.initial_state.len(), 2);
        assert_eq!(comp.events.len(), 3);

        let js = comp.to_js_module();
        assert!(js.contains("export class todo"));
        assert!(js.contains("case 'add':"));
        assert!(js.contains("case 'remove':"));
        assert!(js.contains("case 'toggle':"));
    }

    #[test]
    fn diff_type_mismatch_replaces() {
        let old = CrowVNode::text("hello");
        let new = CrowVNode::el("span", HashMap::new(), vec![CrowVNode::text("hello")]);
        let patches = CrowVNode::changes(&old, &new);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0]["op"], "replace");
    }

    #[test]
    fn vnode_h4_h5_h6_constructors() {
        assert_eq!(CrowVNode::h4(vec![CrowVNode::text("h4")]).to_html(), "<h4>h4</h4>");
        assert_eq!(CrowVNode::h5(vec![CrowVNode::text("h5")]).to_html(), "<h5>h5</h5>");
        assert_eq!(CrowVNode::h6(vec![CrowVNode::text("h6")]).to_html(), "<h6>h6</h6>");
    }

    #[test]
    fn vnode_ol_pre_code_constructors() {
        assert_eq!(CrowVNode::ol(vec![CrowVNode::li(vec![CrowVNode::text("item")])]).to_html(), "<ol><li>item</li></ol>");
        assert_eq!(CrowVNode::pre(vec![CrowVNode::text("preformatted")]).to_html(), "<pre>preformatted</pre>");
        assert_eq!(CrowVNode::code(vec![CrowVNode::text("x = 1")]).to_html(), "<code>x = 1</code>");
    }

    #[test]
    fn field_two_way_binding_html() {
        let field = CrowVNode::field("Price", "price", "42");
        let html = field.to_html();
        assert!(html.contains("class=\"crow-field\""));
        assert!(html.contains("<label>Price</label>"));
        assert!(html.contains("data-crow-input=\"__field:price\""));
        assert!(html.contains("value=\"42\""));
    }

    #[test]
    fn select_input_two_way_binding_html() {
        let options = vec!["all".to_string(), "long".to_string(), "short".to_string()];
        let sel = CrowVNode::select_input(&options, "filter", "long");
        let html = sel.to_html();
        assert!(html.contains("data-crow-change=\"__select:filter\""));
        assert!(html.contains("selected=\"selected\""));
        assert!(html.contains(">long</option>"));
    }

    #[test]
    fn table_from_data_renders_complete_table() {
        let headers = vec!["Symbol".to_string(), "Price".to_string()];
        let rows = vec![
            vec!["AAPL".to_string(), "150".to_string()],
            vec!["GOOG".to_string(), "2800".to_string()],
        ];
        let table = CrowVNode::table_from_data(&headers, &rows);
        let html = table.to_html();
        assert!(html.contains("<thead>"));
        assert!(html.contains("<th>Symbol</th>"));
        assert!(html.contains("<td>AAPL</td>"));
        assert!(html.contains("<td>2800</td>"));
    }

    #[test]
    #[cfg(feature = "live")]
    fn reactive_render_cycle_with_field() {
        use crow_web::live::CrowLiveState;

        let mut state = CrowLiveState::new();
        state.set("name", serde_json::json!("Alice"));

        // Render with current state
        let render = |s: &CrowLiveState| -> CrowVNode {
            let name = s.get_str("name", "");
            CrowVNode::div(vec![
                CrowVNode::field("Name", "name", &name),
                CrowVNode::p(vec![CrowVNode::text(&format!("Hello, {}", name))]),
            ])
        };

        let v1 = render(&state);
        assert!(v1.to_html().contains("Hello, Alice"));

        // Simulate __field:name event
        state.set("name", serde_json::json!("Bob"));
        let v2 = render(&state);
        assert!(v2.to_html().contains("Hello, Bob"));

        // the render's changes carry the new greeting
        let patches = CrowVNode::changes(&v1, &v2);
        assert!(patches.iter().any(|p| p["text"] == "Hello, Bob"), "{:?}", patches);
    }
}

// ── Mail: email struct (no SMTP server needed) ──────────────────────

#[cfg(feature = "mail")]
mod mail_integration {
    use crow_web::mail::CrowEmail;

    #[test]
    fn email_struct_plain_and_html() {
        let plain = CrowEmail {
            from: "noreply@example.com".to_string(),
            to: "alice@example.com".to_string(),
            subject: "Welcome".to_string(),
            body: "Hello Alice!".to_string(),
            html: false,
        };
        assert!(!plain.html);
        assert_eq!(plain.subject, "Welcome");

        let html_email = CrowEmail {
            from: "noreply@example.com".to_string(),
            to: "bob@example.com".to_string(),
            subject: "Newsletter".to_string(),
            body: "<h1>Hi Bob</h1><p>Check this out!</p>".to_string(),
            html: true,
        };
        assert!(html_email.html);
        assert!(html_email.body.contains("<h1>"));
    }
}