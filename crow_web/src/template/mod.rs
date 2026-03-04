//! `web.template` module — HTML templating with Tera (Jinja2-style).
//!
//! ```text
//! use web.template
//!
//! t = templates['./templates/**/*']
//! html = render[t, 'index.html', {name, 'alice'}]
//! ```

/// A compiled template engine wrapping Tera.
pub struct CrowTemplates {
    engine: tera::Tera,
}

impl CrowTemplates {
    /// Load templates from a glob pattern.
    ///
    /// Example: `CrowTemplates::from_glob("templates/**/*")`
    pub fn from_glob(pattern: &str) -> Result<Self, String> {
        tera::Tera::new(pattern)
            .map(|engine| Self { engine })
            .map_err(|e| e.to_string())
    }

    /// Create an empty template engine (for adding templates from strings).
    pub fn new() -> Self {
        Self {
            engine: tera::Tera::default(),
        }
    }

    /// Add a template from a string.
    pub fn add(&mut self, name: &str, content: &str) -> Result<(), String> {
        self.engine
            .add_raw_template(name, content)
            .map_err(|e| e.to_string())
    }

    /// Render a template with the given context data.
    ///
    /// Returns the rendered HTML string, or an error message.
    pub fn render(&self, name: &str, data: serde_json::Value) -> Result<String, String> {
        let context = tera::Context::from_value(data).map_err(|e| e.to_string())?;
        self.engine.render(name, &context).map_err(|e| e.to_string())
    }

    /// Render a one-off template string with context data.
    pub fn render_str(&mut self, template: &str, data: serde_json::Value) -> Result<String, String> {
        let context = tera::Context::from_value(data).map_err(|e| e.to_string())?;
        tera::Tera::one_off(template, &context, false).map_err(|e| e.to_string())
    }
}

impl Default for CrowTemplates {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_string_template() {
        let mut t = CrowTemplates::new();
        t.add("hello", "Hello {{ name }}!").unwrap();
        let result = t.render("hello", serde_json::json!({"name": "alice"})).unwrap();
        assert_eq!(result, "Hello alice!");
    }

    #[test]
    fn test_render_with_loop() {
        let mut t = CrowTemplates::new();
        t.add("list", "{% for item in items %}{{ item }} {% endfor %}").unwrap();
        let result = t.render("list", serde_json::json!({"items": ["a", "b", "c"]})).unwrap();
        assert_eq!(result, "a b c ");
    }

    #[test]
    fn test_render_with_conditional() {
        let mut t = CrowTemplates::new();
        t.add("cond", "{% if show %}yes{% else %}no{% endif %}").unwrap();
        assert_eq!(t.render("cond", serde_json::json!({"show": true})).unwrap(), "yes");
        assert_eq!(t.render("cond", serde_json::json!({"show": false})).unwrap(), "no");
    }

    #[test]
    fn test_render_missing_template() {
        let t = CrowTemplates::new();
        assert!(t.render("missing", serde_json::json!({})).is_err());
    }

    #[test]
    fn test_render_str() {
        let mut t = CrowTemplates::new();
        let result = t.render_str("Hi {{ who }}", serde_json::json!({"who": "world"})).unwrap();
        assert_eq!(result, "Hi world");
    }

    #[test]
    fn test_add_multiple_templates() {
        let mut t = CrowTemplates::new();
        t.add("a", "Template A: {{ x }}").unwrap();
        t.add("b", "Template B: {{ y }}").unwrap();
        assert_eq!(t.render("a", serde_json::json!({"x": 1})).unwrap(), "Template A: 1");
        assert_eq!(t.render("b", serde_json::json!({"y": 2})).unwrap(), "Template B: 2");
    }
}

