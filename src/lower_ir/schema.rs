//! The schema a program declares (spec, db, The schema a program declares):
//! every literal `CREATE TABLE` the program hands a connection declares that
//! table on it — handed directly, or through the functions the connection
//! is handed to — unless the program also drops the table there, remaking
//! it each run. Each connection's `connect` carries what is declared on it,
//! and opening it reconciles or verifies that (crow_db::schema), the same
//! call in both backends.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn bare(e: &Expr) -> &Expr {
    match e {
        Expr::Convert { expr, .. } => bare(expr),
        other => other,
    }
}

fn is_connect(e: &Expr) -> bool {
    matches!(bare(e), Expr::Effect { name, .. } if name == "db.connect" || name == "connect")
}

/// A function body's statements and the name it goes by.
struct Body<'a> {
    name: &'a str,
    params: Vec<&'a str>,
    block: &'a [Stmt],
}

/// Where the program's statements are: each function's, each derivation's
/// and test's, and the top level's (named "").
fn bodies(program: &Program) -> Vec<Body<'_>> {
    let mut out = Vec::new();
    for part in &program.parts {
        for f in &part.fns {
            out.push(Body { name: &f.name, params: f.params.iter().map(|(p, _)| p.as_str()).collect(), block: &f.body });
        }
        for d in &part.derives {
            out.push(Body { name: &d.name, params: Vec::new(), block: &d.body });
        }
        for t in &part.tests {
            out.push(Body { name: "", params: t.params.iter().map(|(p, _)| p.as_str()).collect(), block: &t.body });
        }
        out.push(Body { name: "", params: Vec::new(), block: &part.main });
    }
    out
}

/// The url a `connect` opens, when the program writes it.
fn url_of(e: &Expr) -> Option<String> {
    match bare(e) {
        Expr::Effect { args, .. } => match args.first().map(bare) {
            Some(Expr::Const(Const::Str(url), _)) => Some(url.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// The variables a program opens connections into — its facts and the
/// locals, anywhere, given a `connect` — each with the url it opens when
/// the program writes one.
fn connections(program: &Program) -> BTreeMap<String, Option<String>> {
    let mut out = BTreeMap::new();
    for part in &program.parts {
        for fact in &part.facts {
            if is_connect(&fact.default) {
                out.insert(fact.name.clone(), url_of(&fact.default));
            }
        }
    }
    fn walk(b: &[Stmt], out: &mut BTreeMap<String, Option<String>>) {
        for s in b {
            match s {
                Stmt::Let { var, expr, .. } | Stmt::Set { var, expr } if is_connect(expr) => {
                    out.insert(var.clone(), url_of(expr));
                }
                Stmt::Store { fact, expr } if is_connect(expr) => {
                    out.insert(fact.clone(), url_of(expr));
                }
                Stmt::If { then, els, .. } => {
                    walk(then, out);
                    if let Some(e) = els {
                        walk(e, out);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => walk(body, out),
                _ => {}
            }
        }
    }
    for body in bodies(program) {
        walk(body.block, &mut out);
    }
    out
}

/// The connections an expression holds, where it runs in `within`: a
/// connection variable read, or a parameter the function's callers hand one.
struct Resolver<'a> {
    conns: &'a BTreeMap<String, Option<String>>,
    /// Each function's parameters, by its name (a specialized copy's under
    /// its own name too).
    params: BTreeMap<&'a str, Vec<&'a str>>,
    /// Each call of a function: the function it is in, and its arguments.
    calls: BTreeMap<String, Vec<(String, Vec<Expr>)>>,
    seen: BTreeSet<(String, String)>,
}

impl<'a> Resolver<'a> {
    fn of(&mut self, e: &Expr, within: &str) -> BTreeSet<String> {
        let (Expr::Load(v, _) | Expr::Local(v, _) | Expr::Move(v, _)) = bare(e) else {
            return BTreeSet::new();
        };
        let param = self.params.get(within).and_then(|ps| ps.iter().position(|p| *p == v.as_str()));
        match param {
            Some(i) => {
                // a parameter holds what each call of the function hands it
                if !self.seen.insert((within.to_string(), v.clone())) {
                    return BTreeSet::new();
                }
                let mut out = BTreeSet::new();
                for (caller, args) in self.calls.get(within).cloned().unwrap_or_default() {
                    if let Some(a) = args.get(i) {
                        out.extend(self.of(a, &caller));
                    }
                }
                out
            }
            None if self.conns.contains_key(v) => [v.clone()].into_iter().collect(),
            None => BTreeSet::new(),
        }
    }
}

/// Each connection variable's declared statements, in the order the
/// program writes them.
pub(super) fn declared(program: &Program) -> BTreeMap<String, Vec<String>> {
    declared_with(program, &connections(program))
}

fn declared_with(program: &Program, conns: &BTreeMap<String, Option<String>>) -> BTreeMap<String, Vec<String>> {
    let all = bodies(program);
    let mut params: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut calls: BTreeMap<String, Vec<(String, Vec<Expr>)>> = BTreeMap::new();
    // each literal statement a connection is handed that makes a table, and
    // each that drops one: where it runs, the connection, and its text
    let mut ddl: Vec<(String, Expr, String)> = Vec::new();
    let mut drops: Vec<(String, Expr, String)> = Vec::new();
    for body in &all {
        if !body.name.is_empty() {
            params.insert(body.name, body.params.clone());
        }
        crate::ir::visit_block(body.block, &mut |e| match e {
            Expr::Call { target: Target::User(f), args, .. } => {
                calls.entry(f.clone()).or_default().push((body.name.to_string(), args.clone()));
                // a specialized copy is called under the name it was made from too
                if let Some((base, _)) = f.split_once("__") {
                    calls.entry(base.to_string()).or_default().push((body.name.to_string(), args.clone()));
                }
            }
            Expr::Effect { name, args, .. } if name == "exec" || name == "db.exec" => {
                if let (Some(conn), Some(Expr::Const(Const::Str(sql), _))) = (args.first(), args.get(1).map(bare)) {
                    if crow_db::schema::parse_table(sql).is_some() {
                        ddl.push((body.name.to_string(), conn.clone(), sql.clone()));
                    } else if let Some(table) = crow_db::schema::dropped_table(sql) {
                        drops.push((body.name.to_string(), conn.clone(), table.text.to_lowercase()));
                    }
                }
            }
            _ => {}
        });
    }
    let mut resolver = Resolver { conns, params, calls, seen: BTreeSet::new() };
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (within, conn, sql) in ddl {
        resolver.seen.clear();
        for c in resolver.of(&conn, &within) {
            let list = out.entry(c).or_default();
            if !list.contains(&sql) {
                list.push(sql.clone());
            }
        }
    }
    // a table the program drops on a connection it remakes each run: its
    // shape is the program's outright, not the database's to hold
    let mut dropped: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (within, conn, table) in drops {
        resolver.seen.clear();
        for c in resolver.of(&conn, &within) {
            dropped.entry(c).or_default().insert(table.clone());
        }
    }
    for (c, list) in out.iter_mut() {
        if let Some(gone) = dropped.get(c) {
            list.retain(|sql| crow_db::schema::parse_table(sql).is_none_or(|t| !gone.contains(&t.name.text.to_lowercase())));
        }
    }
    out.retain(|_, list| !list.is_empty());
    out
}

/// Each connection the program declares tables on, with the url it opens
/// when the program writes one and the statements declaring them: what
/// `crow db diff` reads.
pub(crate) fn plan(program: &Program) -> Vec<(String, Option<String>, Vec<String>)> {
    let conns = connections(program);
    declared_with(program, &conns).into_iter().map(|(var, ddl)| (var.clone(), conns.get(&var).cloned().flatten(), ddl)).collect()
}

/// Each connection the program declares tables on opens with them: its
/// `connect` is handed the statements (`connect(url, pool, declared)`, a
/// pool of 0 the backend's default).
pub(super) fn attach(program: &mut Program) {
    let declared = declared(program);
    if declared.is_empty() {
        return;
    }
    let with_declared = |var: &str, e: &mut Expr| {
        let Some(list) = declared.get(var) else { return };
        let mut target = e;
        while let Expr::Convert { expr, .. } = target {
            target = expr;
        }
        if let Expr::Effect { args, .. } = target {
            if args.len() == 1 {
                args.push(Expr::Const(Const::Int(0), Ty::Int));
            }
            if args.len() == 2 {
                let items = list.iter().map(|s| Expr::Const(Const::Str(s.clone()), Ty::Str)).collect();
                args.push(Expr::ListLit(items, Ty::List(Box::new(Ty::Str))));
            }
        }
    };
    fn walk(b: &mut [Stmt], f: &dyn std::ops::Fn(&str, &mut Expr)) {
        for s in b.iter_mut() {
            match s {
                Stmt::Let { var, expr, .. } | Stmt::Set { var, expr } if is_connect(expr) => f(var, expr),
                Stmt::Store { fact, expr } if is_connect(expr) => f(fact, expr),
                Stmt::If { then, els, .. } => {
                    walk(then, f);
                    if let Some(e) = els {
                        walk(e, f);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => walk(body, f),
                _ => {}
            }
        }
    }
    for part in &mut program.parts {
        for fact in &mut part.facts {
            if is_connect(&fact.default) {
                let name = fact.name.clone();
                with_declared(&name, &mut fact.default);
            }
        }
        for f in &mut part.fns {
            walk(&mut f.body, &with_declared);
        }
        for d in &mut part.derives {
            walk(&mut d.body, &with_declared);
        }
        walk(&mut part.main, &with_declared);
    }
}
