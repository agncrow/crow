//! A session's facts (typed-values-design.md §6): one store the program
//! provides. The runtime's own, [`JsonFacts`], holds each fact as JSON —
//! the interpreter's served pages use it, and so does every runtime test. A
//! compiled page program provides a struct of its own, one typed field per
//! fact, which its handlers, views and derivations read and write in place;
//! JSON is then only the edges: the harness, a bound input's text, the dev
//! panel, a kept session, a `later[]` job's changes.

use std::collections::HashMap;

/// What the runtime asks of a session's facts. A name is a fact's (or one
/// of the runtime's own keys, `__reader`, `__pages`, which a program's
/// store keeps beside its facts).
pub trait Facts: Send + Sync {
    /// A copy: a session's working copy, a new session from the app's.
    fn box_clone(&self) -> Box<dyn Facts>;
    /// A fact as JSON, or none when there is no such fact.
    fn get_json(&self, name: &str) -> Option<serde_json::Value>;
    /// A fact from JSON; false when the value does not fit the fact, which
    /// is left as it was.
    fn set_json(&mut self, name: &str, value: serde_json::Value) -> bool;
    /// Whether there is a fact of this name.
    fn has(&self, name: &str) -> bool;
    /// Every fact's name.
    fn names(&self) -> Vec<String>;
    /// One fact copied from another store of the same program: a handler's
    /// write landing as it commits.
    fn copy_fact(&mut self, from: &dyn Facts, name: &str);
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

impl Clone for Box<dyn Facts> {
    fn clone(&self) -> Self {
        self.box_clone()
    }
}

impl std::fmt::Debug for dyn Facts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut names = self.names();
        names.sort();
        write!(f, "Facts{:?}", names)
    }
}

/// Every fact of a store as JSON, by name.
pub fn facts_map(facts: &dyn Facts) -> HashMap<String, serde_json::Value> {
    facts.names().into_iter().filter_map(|n| facts.get_json(&n).map(|v| (n, v))).collect()
}

/// The runtime's own store: each fact a JSON value.
#[derive(Debug, Clone, Default)]
pub struct JsonFacts(pub HashMap<String, serde_json::Value>);

impl Facts for JsonFacts {
    fn box_clone(&self) -> Box<dyn Facts> {
        Box::new(self.clone())
    }
    fn get_json(&self, name: &str) -> Option<serde_json::Value> {
        self.0.get(name).cloned()
    }
    fn set_json(&mut self, name: &str, value: serde_json::Value) -> bool {
        self.0.insert(name.to_string(), value);
        true
    }
    fn has(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }
    fn names(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }
    fn copy_fact(&mut self, from: &dyn Facts, name: &str) {
        match from.get_json(name) {
            Some(v) => {
                self.0.insert(name.to_string(), v);
            }
            None => {
                self.0.remove(name);
            }
        }
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
