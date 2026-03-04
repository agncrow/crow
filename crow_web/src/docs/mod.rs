//! `web.docs` module — Auto-generated API documentation.
//!
//! ```text
//! use web
//! use web.docs
//!
//! app = web.new[]
//! route[app, '/users', 'GET', fn[req] json[{name, 'alice'}]]
//! docs[app, '/docs']  --- serve Swagger UI at /docs ---
//! ```

/// An API endpoint documentation entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CrowEndpoint {
    pub path: String,
    pub method: String,
    pub summary: String,
    pub description: String,
    pub params: Vec<CrowParam>,
    pub response_type: String,
}

/// A parameter in an API endpoint.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CrowParam {
    pub name: String,
    pub location: String, // "path", "query", "body"
    pub param_type: String,
    pub required: bool,
    pub description: String,
}

/// API documentation registry.
#[derive(Debug, Clone)]
pub struct CrowDocs {
    pub title: String,
    pub version: String,
    pub endpoints: Vec<CrowEndpoint>,
}

impl CrowDocs {
    /// Create a new docs registry.
    pub fn new(title: &str, version: &str) -> Self {
        Self {
            title: title.to_string(),
            version: version.to_string(),
            endpoints: Vec::new(),
        }
    }

    /// Add an endpoint to the documentation.
    pub fn add(&mut self, endpoint: CrowEndpoint) {
        self.endpoints.push(endpoint);
    }

    /// Generate an OpenAPI 3.0 JSON spec.
    pub fn to_openapi_json(&self) -> serde_json::Value {
        let mut paths: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();

        for ep in &self.endpoints {
            let method = ep.method.to_lowercase();
            let params: Vec<serde_json::Value> = ep
                .params
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "name": p.name,
                        "in": p.location,
                        "required": p.required,
                        "description": p.description,
                        "schema": { "type": p.param_type }
                    })
                })
                .collect();

            let operation = serde_json::json!({
                "summary": ep.summary,
                "description": ep.description,
                "parameters": params,
                "responses": {
                    "200": {
                        "description": "Success",
                        "content": {
                            "application/json": {
                                "schema": { "type": ep.response_type }
                            }
                        }
                    }
                }
            });

            let path_entry = paths
                .entry(ep.path.clone())
                .or_insert_with(|| serde_json::json!({}));
            if let serde_json::Value::Object(map) = path_entry {
                map.insert(method, operation);
            }
        }

        serde_json::json!({
            "openapi": "3.0.0",
            "info": {
                "title": self.title,
                "version": self.version
            },
            "paths": paths
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_docs_empty() {
        let docs = CrowDocs::new("My API", "1.0.0");
        let spec = docs.to_openapi_json();
        assert_eq!(spec["openapi"], "3.0.0");
        assert_eq!(spec["info"]["title"], "My API");
    }

    #[test]
    fn test_docs_add_endpoint() {
        let mut docs = CrowDocs::new("API", "1.0");
        docs.add(CrowEndpoint {
            path: "/users".to_string(),
            method: "GET".to_string(),
            summary: "List users".to_string(),
            description: "Get all users".to_string(),
            params: vec![],
            response_type: "array".to_string(),
        });
        let spec = docs.to_openapi_json();
        assert!(spec["paths"]["/users"]["get"].is_object());
        assert_eq!(spec["paths"]["/users"]["get"]["summary"], "List users");
    }

    #[test]
    fn test_docs_with_params() {
        let mut docs = CrowDocs::new("API", "1.0");
        docs.add(CrowEndpoint {
            path: "/users/{id}".to_string(),
            method: "GET".to_string(),
            summary: "Get user".to_string(),
            description: "".to_string(),
            params: vec![CrowParam {
                name: "id".to_string(),
                location: "path".to_string(),
                param_type: "integer".to_string(),
                required: true,
                description: "User ID".to_string(),
            }],
            response_type: "object".to_string(),
        });
        let spec = docs.to_openapi_json();
        let params = &spec["paths"]["/users/{id}"]["get"]["parameters"];
        assert_eq!(params[0]["name"], "id");
        assert_eq!(params[0]["in"], "path");
    }
}

