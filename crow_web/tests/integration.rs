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

// ── Rate limiting ────────────────────────────────────────────────────

#[cfg(feature = "rate")]
mod rate_integration {
    use crow_web::rate::CrowRateLimiter;

    #[test]
    fn rate_limiter_per_ip_isolation() {
        let limiter = CrowRateLimiter::new(2, 60);

        // IP A uses 2 requests
        assert!(limiter.check("192.168.1.1"));
        assert!(limiter.check("192.168.1.1"));
        assert!(!limiter.check("192.168.1.1")); // blocked

        // IP B is independent
        assert!(limiter.check("192.168.1.2"));
        assert!(limiter.check("192.168.1.2"));
        assert!(!limiter.check("192.168.1.2")); // blocked

        // Reset IP A
        limiter.reset("192.168.1.1");
        assert!(limiter.check("192.168.1.1")); // allowed again
    }

    #[test]
    fn rate_limiter_remaining_accuracy() {
        let limiter = CrowRateLimiter::new(5, 60);
        assert_eq!(limiter.remaining("new-key"), 5);
        limiter.check("new-key");
        limiter.check("new-key");
        // Remaining should be around 3 (may vary slightly due to refill)
        let rem = limiter.remaining("new-key");
        assert!(rem >= 2 && rem <= 4, "remaining was {}", rem);
    }
}

// ── Queue: background jobs ───────────────────────────────────────────

// ── Template + DB: render database data ──────────────────────────────

// ── Session + Rate: session-aware rate limiting ──────────────────────

// ── Docs: OpenAPI generation ─────────────────────────────────────────

// ── Upload: multipart parsing ───────────────────────────────────────

#[cfg(feature = "upload")]
mod upload_integration {
    use crow_web::upload::{CrowUploadResult, CrowUploadedFile, parse_multipart_boundary};
    use std::collections::HashMap;

    #[test]
    fn upload_result_multi_file_lookup() {
        let result = CrowUploadResult {
            files: vec![
                CrowUploadedFile {
                    name: "doc.pdf".to_string(),
                    content_type: "application/pdf".to_string(),
                    data: vec![0x25, 0x50, 0x44, 0x46], // %PDF
                    field_name: "document".to_string(),
                },
                CrowUploadedFile {
                    name: "photo.png".to_string(),
                    content_type: "image/png".to_string(),
                    data: vec![0x89, 0x50, 0x4E, 0x47], // PNG header
                    field_name: "avatar".to_string(),
                },
            ],
            fields: HashMap::from([
                ("title".to_string(), "My Upload".to_string()),
                ("description".to_string(), "Test file".to_string()),
            ]),
        };

        assert_eq!(result.count(), 2);
        assert_eq!(result.file("document").unwrap().name, "doc.pdf");
        assert_eq!(result.file("avatar").unwrap().content_type, "image/png");
        assert_eq!(result.field("title"), Some("My Upload"));
        assert!(result.file("missing").is_none());
        assert!(result.field("missing").is_none());
    }

    #[test]
    fn upload_file_save_and_text() {
        let file = CrowUploadedFile {
            name: "test.txt".to_string(),
            content_type: "text/plain".to_string(),
            data: b"hello world".to_vec(),
            field_name: "file".to_string(),
        };
        assert_eq!(file.size(), 11);
        assert_eq!(file.text(), Some("hello world".to_string()));

        // Save to temp
        let path = std::env::temp_dir().join("crow_upload_test.txt");
        assert!(file.save(path.to_str().unwrap()));
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "hello world");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn boundary_parsing() {
        assert_eq!(
            parse_multipart_boundary("multipart/form-data; boundary=abc123"),
            Some("abc123".to_string())
        );
        assert_eq!(
            parse_multipart_boundary(r#"multipart/form-data; boundary="quoted""#),
            Some("quoted".to_string())
        );
        assert_eq!(parse_multipart_boundary("text/plain"), None);
    }
}

// ── CORS + Server: cross-origin with routing ────────────────────────

#[cfg(all(feature = "cors", feature = "http"))]
mod cors_server {
    use crow_web::core::{CrowRequest, CrowResponse};
    use crow_web::server::{CrowApp, CrowHandler};
    use crow_web::cors::CrowCors;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt;

    async fn send(router: axum::Router, req: HttpRequest<Body>) -> axum::http::Response<Body> {
        router.oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn cors_permissive_adds_headers() {
        let app = CrowApp::new()
            .route("/api", CrowHandler::new("api", |_: CrowRequest| CrowResponse::json(serde_json::json!({"ok": true}))))
            .cors_permissive();

        let router = app.into_router();

        let req = HttpRequest::builder()
            .uri("/api")
            .header("origin", "http://evil.com")
            .body(Body::empty())
            .unwrap();
        let resp = send(router, req).await;
        assert_eq!(resp.status(), 200);
        // CORS should allow any origin
        let acao = resp.headers().get("access-control-allow-origin");
        assert!(acao.is_some(), "Missing access-control-allow-origin header");
    }

    #[tokio::test]
    async fn cors_specific_origins() {
        let cors = CrowCors::origins(vec!["http://allowed.com".to_string()]);
        let app = CrowApp::new()
            .route("/data", CrowHandler::new("data", |_: CrowRequest| CrowResponse::text("ok")))
            .cors(cors);

        let router = app.into_router();
        let req = HttpRequest::builder()
            .uri("/data")
            .header("origin", "http://allowed.com")
            .body(Body::empty())
            .unwrap();
        let resp = send(router, req).await;
        assert_eq!(resp.status(), 200);
    }
}

// ── Static files + Server ───────────────────────────────────────────

#[cfg(all(feature = "static_files", feature = "http"))]
mod static_server {
    use crow_web::server::CrowApp;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt;

    async fn send(router: axum::Router, req: HttpRequest<Body>) -> axum::http::Response<Body> {
        router.oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn serve_static_file_through_app() {
        let dir = std::env::temp_dir().join("crow_static_integration");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("style.css"), "body { color: red; }").unwrap();

        let app = CrowApp::new()
            .static_dir("/assets", dir.to_str().unwrap());

        let router = app.into_router();
        let req = HttpRequest::builder()
            .uri("/assets/style.css")
            .body(Body::empty())
            .unwrap();
        let resp = send(router, req).await;
        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, "body { color: red; }");

        let _ = std::fs::remove_dir_all(&dir);
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
        assert_eq!(state.get("count"), Some(&serde_json::json!(0)));

        state.set("count", serde_json::json!(5));
        assert_eq!(state.get("count"), Some(&serde_json::json!(5)));

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

// ── WebSocket: client-side (no server needed) ───────────────────────

#[cfg(feature = "ws")]
mod ws_integration {
    use crow_web::ws::connect;

    #[test]
    fn ws_connect_to_invalid_host() {
        let sock = connect("ws://127.0.0.1:1");
        assert!(!sock.ok());
        // All operations on failed socket should be no-ops
        sock.send("test");
        assert!(sock.recv().is_none());
        sock.close();
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