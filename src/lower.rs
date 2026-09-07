//! AST lowering: resolves named arguments into plain positional calls.
//!
//! Runs once, after parsing (and module expansion), before type inference,
//! validation, interpretation, or transpilation. Downstream code never sees
//! `Expr::Named` — this is the single implementation of named-argument
//! semantics for both execution modes (shared-kernel principle, see
//! docs/design-review.md pushback 6).
//!
//! What it does per call:
//! - user functions / structs: named args are placed into their parameter
//!   slots; omitted parameters with defaults are filled with the default
//!   literal. Errors on unknown names, duplicates, and missing parameters.
//! - builtins with named parameters (axis, desc, ...): named args are
//!   reordered to their positional slots. No default filling — builtin arity
//!   is meaningful (e.g. `sum[x]` vs `sum[x, axis]` return different shapes).
//! - `map[...]`: the variadic named constructor. `map[a = 1, b = 2]` desugars
//!   to `set[set[map[], 'a', 1], 'b', 2]` — existing primitives, zero new
//!   runtime code.
//! - UI elements (`use ui`): named args become the attribute map:
//!   `button['+', click = inc]` → `button[[click = inc], '+']`.

use crate::ast::{Expr, Stmt};
use std::collections::BTreeMap;

/// HTML element constructors that accept trailing named attributes.
pub const UI_ELEMENTS: &[&str] = &[
    "div", "span", "p", "h1", "h2", "h3", "h4", "h5", "h6",
    "button", "ul", "ol", "li", "form", "a", "nav",
    "header", "footer", "section", "tr", "td", "th",
    "thead", "tbody", "option", "textarea", "label",
    "pre", "code", "input", "img", "table",
    // the layout vocabulary's containers (render as div; see crow_runtime::layout)
    "stack", "row", "grid", "overlay", "scroll",
    // revealing: disclosure, a dropdown panel, and the three layers
    "details", "summary", "dropdown", "modal", "drawer", "toast",
    // a rule between siblings
    "divider",
];

/// Builtins with named parameters: name → ordered named-parameter list.
/// Position of a named parameter = (call arity without it) + its index here.
fn builtin_named_params(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "sum" | "mean" | "min" | "max" | "prod" | "stack" | "concat" => Some(&["axis"]),
        "sort" => Some(&["desc"]),
        // test harness: app[view, width = 1280]
        "app" => Some(&["width"]),
        _ => None,
    }
}

#[derive(Clone)]
struct FnSig {
    params: Vec<String>,
    defaults: Vec<Option<Expr>>,
}

#[derive(Clone)]
pub struct Lowerer {
    fns: BTreeMap<String, FnSig>,
    structs: BTreeMap<String, Vec<String>>,
    current_line: usize,
    /// Lowering inside a `view` body — ui element named-args resolve here
    in_view: bool,
}

pub fn lower_program(stmts: &[Stmt]) -> Result<Vec<Stmt>, String> {
    let mut l = Lowerer {
        fns: BTreeMap::new(),
        structs: BTreeMap::new(),
        current_line: 0,
            in_view: false,
    };
    l.collect_sigs(stmts);
    l.lower_stmts(stmts)
}

impl Lowerer {
    pub fn new() -> Self {
        Lowerer {
            fns: BTreeMap::new(),
            structs: BTreeMap::new(),
            current_line: 0,
            in_view: false,
        }
    }

    /// Lower a batch of statements. Signatures accumulate across calls, so a
    /// persistent Lowerer (REPL, interpreter) resolves names from earlier
    /// batches.
    pub fn lower(&mut self, stmts: &[Stmt]) -> Result<Vec<Stmt>, String> {
        self.collect_sigs(stmts);
        self.lower_stmts(stmts)
    }

    fn collect_sigs(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            match s {
                Stmt::FnDecl { name, params, defaults, body, .. } => {
                    self.fns.insert(
                        name.clone(),
                        FnSig { params: params.clone(), defaults: defaults.clone() },
                    );
                    self.collect_sigs(body);
                }
                Stmt::StructDecl { name, fields } => {
                    self.structs.insert(name.clone(), fields.clone());
                }
                Stmt::If { then_body, else_body, .. } => {
                    self.collect_sigs(then_body);
                    if let Some(eb) = else_body {
                        self.collect_sigs(eb);
                    }
                }
                Stmt::For { body, .. } | Stmt::While { body, .. } => {
                    self.collect_sigs(body);
                }
                _ => {}
            }
        }
    }

    fn err(&self, msg: String) -> String {
        if self.current_line > 0 {
            format!("line {}: {}", self.current_line, msg)
        } else {
            msg
        }
    }

    fn lower_stmts(&mut self, stmts: &[Stmt]) -> Result<Vec<Stmt>, String> {
        stmts.iter().map(|s| self.lower_stmt(s)).collect()
    }

    fn lower_stmt(&mut self, stmt: &Stmt) -> Result<Stmt, String> {
        Ok(match stmt {
            Stmt::Line(n) => {
                self.current_line = *n;
                Stmt::Line(*n)
            }
            Stmt::Assign { name, value } => Stmt::Assign {
                name: name.clone(),
                value: self.lower_expr(value)?,
            },
            Stmt::MultiAssign { names, values } => Stmt::MultiAssign {
                names: names.clone(),
                values: values.iter().map(|v| self.lower_expr(v)).collect::<Result<_, _>>()?,
            },
            Stmt::FnDecl { name, params, defaults, body, is_view } => {
                // Element vocabulary is active only inside views
                let saved = self.in_view;
                self.in_view = *is_view;
                let lowered = Stmt::FnDecl {
                    name: name.clone(),
                    params: params.clone(),
                    defaults: defaults.clone(),
                    body: self.lower_stmts(body)?,
                    is_view: *is_view,
                };
                self.in_view = saved;
                lowered
            }
            Stmt::Return(e) => Stmt::Return(match e {
                Some(e) => Some(self.lower_expr(e)?),
                None => None,
            }),
            Stmt::ExprStmt(e) => Stmt::ExprStmt(self.lower_expr(e)?),
            Stmt::If { condition, then_body, else_body } => Stmt::If {
                condition: self.lower_expr(condition)?,
                then_body: self.lower_stmts(then_body)?,
                else_body: match else_body {
                    Some(eb) => Some(self.lower_stmts(eb)?),
                    None => None,
                },
            },
            Stmt::For { var, iter, body } => Stmt::For {
                var: var.clone(),
                iter: self.lower_expr(iter)?,
                body: self.lower_stmts(body)?,
            },
            Stmt::While { condition, body } => Stmt::While {
                condition: self.lower_expr(condition)?,
                body: self.lower_stmts(body)?,
            },
            Stmt::TestBlock { name, params, body } => Stmt::TestBlock {
                name: name.clone(),
                params: params.clone(),
                body: self.lower_stmts(body)?,
            },
            Stmt::Derive { name, fresh, body } => Stmt::Derive {
                name: name.clone(),
                fresh: *fresh,
                body: self.lower_stmts(body)?,
            },
            Stmt::FieldAssign { object, field, value } => Stmt::FieldAssign {
                object: self.lower_expr(object)?,
                field: field.clone(),
                value: self.lower_expr(value)?,
            },
            Stmt::IndexAssign { object, index, value } => Stmt::IndexAssign {
                object: self.lower_expr(object)?,
                index: self.lower_expr(index)?,
                value: self.lower_expr(value)?,
            },
            other => other.clone(),
        })
    }

    fn lower_expr(&mut self, expr: &Expr) -> Result<Expr, String> {
        Ok(match expr {
            // a for expression's body is a block like any other: its map
            // literals and named arguments lower the same way
            Expr::ForExpr { var, iter, body } => Expr::ForExpr {
                var: var.clone(),
                iter: Box::new(self.lower_expr(iter)?),
                body: self.lower_stmts(body)?,
            },
            Expr::Call { callee, args } => {
                let callee = self.lower_expr(callee)?;
                let args: Vec<Expr> = args.iter().map(|a| self.lower_expr(a)).collect::<Result<_, _>>()?;
                let has_named = args.iter().any(|a| matches!(a, Expr::Named { .. }));
                let callee_name = if let Expr::Ident(n) = &callee { Some(n.clone()) } else { None };
                // Module-prefixed ui elements (ui.input[...]) are ui elements too
                let ui_name = match &callee {
                    Expr::Ident(n) => Some(n.clone()),
                    Expr::FieldAccess { object, field } => {
                        if matches!(object.as_ref(), Expr::Ident(m) if m == "ui") {
                            Some(field.clone())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                // Qualified user-module call (scene.mesh[...]): the import
                // expander registered the prefixed fn/struct — resolve named
                // args against it and canonicalize the callee to the prefixed
                // name so both modes see an ordinary call
                if let Expr::FieldAccess { object, field } = &callee {
                    if let Expr::Ident(m) = object.as_ref() {
                        let prefixed = format!("{}_{}", m, field);
                        if let Some(sig) = self.fns.get(&prefixed).cloned() {
                            let resolved =
                                self.resolve_args(&prefixed, &sig.params, &sig.defaults, args)?;
                            return Ok(Expr::Call {
                                callee: Box::new(Expr::Ident(prefixed)),
                                args: resolved,
                            });
                        }
                        if let Some(fields) = self.structs.get(&prefixed).cloned() {
                            let no_defaults = vec![None; fields.len()];
                            let resolved =
                                self.resolve_args(&prefixed, &fields, &no_defaults, args)?;
                            return Ok(Expr::Call {
                                callee: Box::new(Expr::Ident(prefixed)),
                                args: resolved,
                            });
                        }
                    }
                }

                match callee_name.as_deref() {
                    Some("map") => self.lower_map_call(args)?,
                    // slider[label, var, min = 0, max = 100, step = 1]
                    // field[label, var, type = '', lines = 0] — '' means "from
                    // the variable's type"; lines > 0 is a textarea
                    Some("field") if !self.fns.contains_key("field") => {
                        let params: Vec<String> = ["label", "var", "type", "lines"].iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> = vec![None, None, Some(Expr::Str(String::new())), Some(Expr::Int(0))];
                        let resolved = self.resolve_args("field", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    // progress[value, max = 1.0]
                    Some("progress") if !self.fns.contains_key("progress") => {
                        let params: Vec<String> = ["value", "max"].iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> = vec![None, Some(Expr::Float(1.0))];
                        let resolved = self.resolve_args("progress", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    // icon[name, size = 'm', color = '']
                    Some("icon") if !self.fns.contains_key("icon") => {
                        let params: Vec<String> = ["name", "size", "color"].iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> = vec![None, Some(Expr::Str("m".to_string())), Some(Expr::Str(String::new()))];
                        let resolved = self.resolve_args("icon", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    Some("slider") if !self.fns.contains_key("slider") => {
                        let params: Vec<String> = ["label", "var", "min", "max", "step"].iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> = vec![None, None, Some(Expr::Int(0)), Some(Expr::Int(100)), Some(Expr::Int(1))];
                        let resolved = self.resolve_args("slider", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    // fmt has a full named signature with sentinel defaults
                    // (decimals < 0 / width 0 / align '' mean "unset") so both
                    // modes always receive exactly 7 positional args
                    Some("fmt") if !self.fns.contains_key("fmt") => {
                        let params: Vec<String> = ["value", "decimals", "commas", "width", "align", "fill", "percent", "time", "tz"]
                            .iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> = vec![
                            None,
                            Some(Expr::Int(-1)),
                            Some(Expr::Bool(false)),
                            Some(Expr::Int(0)),
                            Some(Expr::Str(String::new())),
                            Some(Expr::Str(" ".to_string())),
                            Some(Expr::Bool(false)),
                            Some(Expr::Str(String::new())),
                            Some(Expr::Str("UTC".to_string())),
                        ];
                        let resolved = self.resolve_args("fmt", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    // moment[year, month, day, hour = 0, minute = 0,
                    // second = 0, tz = 'UTC'] — the tz-aware constructor.
                    // A moment is a number (epoch seconds).
                    Some("moment") if !self.fns.contains_key("moment") => {
                        let params: Vec<String> =
                            ["year", "month", "day", "hour", "minute", "second", "tz"]
                                .iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> = vec![
                            None, None, None,
                            Some(Expr::Int(0)),
                            Some(Expr::Int(0)),
                            Some(Expr::Int(0)),
                            Some(Expr::Str("UTC".to_string())),
                        ];
                        let resolved = self.resolve_args("moment", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    // parts[t, tz = 'UTC'] — calendar components + weekday.
                    Some("calendar") if !self.fns.contains_key("calendar") => {
                        let params: Vec<String> =
                            ["t", "tz"].iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> =
                            vec![None, Some(Expr::Str("UTC".to_string()))];
                        let resolved = self.resolve_args("calendar", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    // parse_time[text, tz = 'UTC'] — string → moment.
                    Some("parse_time") if !self.fns.contains_key("parse_time") => {
                        let params: Vec<String> =
                            ["text", "tz"].iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> =
                            vec![None, Some(Expr::Str("UTC".to_string()))];
                        let resolved = self.resolve_args("parse_time", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    // chart[frame, x = 'ts', candles = [], lines = [],
                    // bars_y = '', marker = 0, tz = 'UTC', height = 320,
                    // panes = [], levels = 0, points = 0, seek = 0] —
                    // a chart is a view over a frame (chart-module-design).
                    // panes: lists of column names, each drawn in its own
                    // strip; levels: map[col = [values]] of reference lines
                    // for the pane that draws col; points: a frame of markers
                    // (ts, y, label, kind); seek: a handler that receives the
                    // clicked bucket's moment.
                    Some("chart") if !self.fns.contains_key("chart") => {
                        let params: Vec<String> = [
                            "frame", "x", "candles", "lines", "bars_y",
                            "marker", "tz", "height", "panes", "levels",
                            "points", "seek",
                        ]
                        .iter()
                        .map(|s| s.to_string())
                        .collect();
                        let defaults: Vec<Option<Expr>> = vec![
                            None,
                            Some(Expr::Str("ts".to_string())),
                            Some(Expr::Array(vec![])),
                            Some(Expr::Array(vec![])),
                            Some(Expr::Str(String::new())),
                            Some(Expr::Int(0)),
                            Some(Expr::Str("UTC".to_string())),
                            Some(Expr::Int(320)),
                            Some(Expr::Array(vec![])),
                            Some(Expr::Int(0)),
                            Some(Expr::Int(0)),
                            Some(Expr::Int(0)),
                        ];
                        let resolved = self.resolve_args("chart", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    // serve[port, refresh = secs, budget = ms] — 0 means unset.
                    // refresh: the runtime re-renders every session on this
                    // interval; budget: handlers/renders over this many ms are
                    // logged. Both modes receive exactly 3 positional args.
                    // Plus the look: theme ('' = system), density ('' =
                    // comfortable), accent ('' = the theme's), wide (false).
                    Some("serve") if !self.fns.contains_key("serve") => {
                        let params: Vec<String> =
                            ["port", "refresh", "budget", "theme", "density", "accent", "wide"].iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> =
                            vec![None, Some(Expr::Int(0)), Some(Expr::Int(0)), Some(Expr::Str(String::new())), Some(Expr::Str(String::new())), Some(Expr::Str(String::new())), Some(Expr::Bool(false))];
                        let resolved = self.resolve_args("serve", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    Some(name) if self.fns.contains_key(name) => {
                        let sig = self.fns.get(name).unwrap().clone();
                        let resolved = self.resolve_args(name, &sig.params, &sig.defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    Some(name) if self.structs.contains_key(name) => {
                        let fields = self.structs.get(name).unwrap().clone();
                        let no_defaults = vec![None; fields.len()];
                        let resolved = self.resolve_args(name, &fields, &no_defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    _ if self.in_view
                        && ui_name.as_deref().map_or(false, |n| UI_ELEMENTS.contains(&n))
                        && has_named =>
                    {
                        self.lower_ui_call(callee, args)
                    }
                    Some(name) if builtin_named_params(name).is_some() && has_named => {
                        let named_params = builtin_named_params(name).unwrap();
                        let resolved = self.resolve_builtin_args(name, named_params, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    _ => {
                        if has_named {
                            let target = callee_name.unwrap_or_else(|| "<expression>".to_string());
                            return Err(self.err(format!(
                                "'{}' does not accept named arguments (named arguments require a known function, struct, builtin with named parameters, or ui element)",
                                target
                            )));
                        }
                        Expr::Call { callee: Box::new(callee), args }
                    }
                }
            }
            Expr::Named { name, value } => Expr::Named {
                name: name.clone(),
                value: Box::new(self.lower_expr(value)?),
            },
            Expr::BinOp { left, op, right } => Expr::BinOp {
                left: Box::new(self.lower_expr(left)?),
                op: op.clone(),
                right: Box::new(self.lower_expr(right)?),
            },
            Expr::UnaryOp { op, expr: e } => Expr::UnaryOp {
                op: op.clone(),
                expr: Box::new(self.lower_expr(e)?),
            },
            Expr::FieldAccess { object, field } => Expr::FieldAccess {
                object: Box::new(self.lower_expr(object)?),
                field: field.clone(),
            },
            Expr::Array(items) => Expr::Array(
                items.iter().map(|i| self.lower_expr(i)).collect::<Result<_, _>>()?,
            ),
            Expr::Map(pairs) => Expr::Map(
                pairs
                    .iter()
                    .map(|(k, v)| Ok((self.lower_expr(k)?, self.lower_expr(v)?)))
                    .collect::<Result<Vec<_>, String>>()?,
            ),
            other => other.clone(),
        })
    }

    /// Place named args into parameter slots and fill omitted defaults.
    fn resolve_args(
        &self,
        fn_name: &str,
        params: &[String],
        defaults: &[Option<Expr>],
        args: Vec<Expr>,
    ) -> Result<Vec<Expr>, String> {
        let positional_count = args.iter().take_while(|a| !matches!(a, Expr::Named { .. })).count();
        if positional_count > params.len() {
            // Too many positionals — leave as-is; the validator reports arity
            // errors with its usual message.
            return Ok(args);
        }
        let mut slots: Vec<Option<Expr>> = vec![None; params.len()];
        for (i, a) in args.iter().take(positional_count).enumerate() {
            slots[i] = Some(a.clone());
        }
        for a in args.into_iter().skip(positional_count) {
            let (name, value) = match a {
                Expr::Named { name, value } => (name, *value),
                _ => unreachable!("parser rejects positional after named"),
            };
            let idx = params.iter().position(|p| p == &name).ok_or_else(|| {
                self.err(format!(
                    "'{}' has no parameter named '{}' (parameters: {})",
                    fn_name, name, params.join(", ")
                ))
            })?;
            if slots[idx].is_some() {
                return Err(self.err(format!(
                    "argument '{}' given more than once in call to '{}'",
                    name, fn_name
                )));
            }
            slots[idx] = Some(value);
        }
        // Fill remaining slots with defaults
        let mut resolved = Vec::with_capacity(params.len());
        for (i, slot) in slots.into_iter().enumerate() {
            match slot {
                Some(e) => resolved.push(e),
                None => match &defaults[i] {
                    Some(d) => resolved.push(d.clone()),
                    None => {
                        return Err(self.err(format!(
                            "missing argument '{}' in call to '{}'",
                            params[i], fn_name
                        )))
                    }
                },
            }
        }
        Ok(resolved)
    }

    /// Builtins: reorder named args after the positionals in declared order.
    /// No default filling — arity is meaningful to the builtin itself.
    fn resolve_builtin_args(
        &self,
        fn_name: &str,
        named_params: &[&str],
        args: Vec<Expr>,
    ) -> Result<Vec<Expr>, String> {
        let mut positional = Vec::new();
        let mut named: Vec<(String, Expr)> = Vec::new();
        for a in args {
            match a {
                Expr::Named { name, value } => {
                    if !named_params.contains(&name.as_str()) {
                        return Err(self.err(format!(
                            "'{}' has no named parameter '{}' (named parameters: {})",
                            fn_name, name, named_params.join(", ")
                        )));
                    }
                    if named.iter().any(|(n, _)| n == &name) {
                        return Err(self.err(format!(
                            "argument '{}' given more than once in call to '{}'",
                            name, fn_name
                        )));
                    }
                    named.push((name, *value));
                }
                other => positional.push(other),
            }
        }
        for p in named_params {
            if let Some(pos) = named.iter().position(|(n, _)| n == p) {
                positional.push(named.remove(pos).1);
            }
        }
        Ok(positional)
    }

    /// UI elements: named args become entries of the attribute map (first
    /// argument by convention). `button['+', click = inc]` →
    /// `button[[click = inc], '+']`. Merges into an explicit attribute map if
    /// one was passed.
    fn lower_ui_call(&self, callee: Expr, args: Vec<Expr>) -> Expr {
        let mut attrs: Vec<(Expr, Expr)> = Vec::new();
        let mut children = Vec::new();
        for a in args {
            match a {
                Expr::Named { name, value } => attrs.push((Expr::Str(name), *value)),
                Expr::Map(pairs) => attrs.extend(pairs),
                other => children.push(other),
            }
        }
        let mut new_args = Vec::with_capacity(children.len() + 1);
        if !attrs.is_empty() {
            new_args.push(Expr::Map(attrs));
        }
        new_args.extend(children);
        Expr::Call { callee: Box::new(callee), args: new_args }
    }

    /// map[...] — the variadic named constructor. Desugars to a set[] chain.
    fn lower_map_call(&self, args: Vec<Expr>) -> Result<Expr, String> {
        let empty = Expr::Call {
            callee: Box::new(Expr::Ident("map".to_string())),
            args: vec![],
        };
        let mut result = empty;
        for a in args {
            match a {
                Expr::Named { name, value } => {
                    result = Expr::Call {
                        callee: Box::new(Expr::Ident("set".to_string())),
                        args: vec![result, Expr::Str(name), *value],
                    };
                }
                _ => {
                    return Err(self.err(
                        "map[] takes named arguments only: map[key = value, ...]".to_string(),
                    ))
                }
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn lower_src(src: &str) -> Result<Vec<Stmt>, String> {
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex failed");
        let mut parser = Parser::new(tokens);
        let stmts = parser.parse().expect("parse failed");
        lower_program(&stmts)
    }

    fn has_named(stmts: &[Stmt]) -> bool {
        fn expr_has(e: &Expr) -> bool {
            match e {
                Expr::Named { .. } => true,
                Expr::Call { callee, args } => expr_has(callee) || args.iter().any(expr_has),
                Expr::BinOp { left, right, .. } => expr_has(left) || expr_has(right),
                Expr::UnaryOp { expr, .. } => expr_has(expr),
                Expr::FieldAccess { object, .. } => expr_has(object),
                Expr::Array(items) => items.iter().any(expr_has),
                Expr::Map(pairs) => pairs.iter().any(|(k, v)| expr_has(k) || expr_has(v)),
                _ => false,
            }
        }
        fn stmt_has(s: &Stmt) -> bool {
            match s {
                Stmt::Assign { value, .. } => expr_has(value),
                Stmt::ExprStmt(e) => expr_has(e),
                Stmt::Return(Some(e)) => expr_has(e),
                Stmt::FnDecl { body, .. } => body.iter().any(stmt_has),
                Stmt::If { condition, then_body, else_body } => {
                    expr_has(condition)
                        || then_body.iter().any(stmt_has)
                        || else_body.as_ref().map_or(false, |b| b.iter().any(stmt_has))
                }
                Stmt::For { iter, body, .. } => expr_has(iter) || body.iter().any(stmt_has),
                Stmt::While { condition, body } => expr_has(condition) || body.iter().any(stmt_has),
                _ => false,
            }
        }
        stmts.iter().any(stmt_has)
    }

    #[test]
    fn test_named_args_resolved_and_defaults_filled() {
        let stmts = lower_src("fn train[data, lr = 0.5, epochs = 10]\n    data\nr = train[1, epochs = 3]\n").unwrap();
        assert!(!has_named(&stmts));
        // r = train[1, 0.5, 3]
        let assign = stmts.iter().find_map(|s| match s {
            Stmt::Assign { name, value } if name == "r" => Some(value.clone()),
            _ => None,
        }).unwrap();
        match assign {
            Expr::Call { args, .. } => {
                assert_eq!(args.len(), 3);
                assert!(matches!(args[0], Expr::Int(1)));
                assert!(matches!(args[1], Expr::Float(f) if f == 0.5));
                assert!(matches!(args[2], Expr::Int(3)));
            }
            other => panic!("expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_unknown_named_arg_errors() {
        let err = lower_src("fn f[a, b = 1]\n    a\nr = f[1, zzz = 2]\n").unwrap_err();
        assert!(err.contains("no parameter named 'zzz'"), "{}", err);
    }

    #[test]
    fn test_duplicate_named_arg_errors() {
        let err = lower_src("fn f[a, b = 1]\n    a\nr = f[1, 2, b = 3]\n").unwrap_err();
        assert!(err.contains("more than once"), "{}", err);
    }

    #[test]
    fn test_map_constructor_desugars_to_set_chain() {
        let stmts = lower_src("m = map[a = 1, b = 2]\n").unwrap();
        assert!(!has_named(&stmts));
        let value = stmts.iter().find_map(|s| match s {
            Stmt::Assign { name, value } if name == "m" => Some(value.clone()),
            _ => None,
        }).unwrap();
        // set[set[map[], 'a', 1], 'b', 2]
        match &value {
            Expr::Call { callee, args } => {
                assert!(matches!(callee.as_ref(), Expr::Ident(n) if n == "set"));
                assert_eq!(args.len(), 3);
                assert!(matches!(&args[1], Expr::Str(s) if s == "b"));
            }
            other => panic!("expected set chain, got {:?}", other),
        }
    }

    #[test]
    fn test_builtin_axis_reordered() {
        let stmts = lower_src("s = sum[x, axis = 0]\n").unwrap();
        assert!(!has_named(&stmts));
        let value = stmts.iter().find_map(|s| match s {
            Stmt::Assign { name, value } if name == "s" => Some(value.clone()),
            _ => None,
        }).unwrap();
        match value {
            Expr::Call { args, .. } => {
                assert_eq!(args.len(), 2);
                assert!(matches!(args[1], Expr::Int(0)));
            }
            other => panic!("expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_ui_named_attrs_become_attr_map() {
        let stmts = lower_src("fn inc[]\n    1\nview v[]\n    b = button['+', click = inc]\n    return b\n").unwrap();
        assert!(!has_named(&stmts));
        // the assignment now lives inside the view body
        let view_body = stmts.iter().find_map(|s| match s {
            Stmt::FnDecl { name, body, is_view: true, .. } if name == "v" => Some(body.clone()),
            _ => None,
        }).unwrap();
        let value = view_body.iter().find_map(|s| match s {
            Stmt::Assign { name, value } if name == "b" => Some(value.clone()),
            _ => None,
        }).unwrap();
        match value {
            Expr::Call { args, .. } => {
                assert_eq!(args.len(), 2);
                assert!(matches!(&args[0], Expr::Map(pairs) if pairs.len() == 1));
                assert!(matches!(&args[1], Expr::Str(s) if s == "+"));
            }
            other => panic!("expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_indirect_call_named_arg_errors() {
        let err = lower_src("r = unknown_fn[1, a = 2]\n").unwrap_err();
        assert!(err.contains("does not accept named arguments"), "{}", err);
    }
}
