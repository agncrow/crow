//! Derivations: the static read sets both execution modes obey.
//!
//! A `derive` block is recomputed when something it read changed. What it
//! reads is decided here, once, from the source — facts and other
//! derivations by name (transitively through the functions it calls), the
//! clock (`now[]`), and the tables its query literals name — so the
//! interpreter and the compiled program recompute at exactly the same
//! moments (derive-design.md §3.2).
use crate::ast::{Expr, Stmt};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default)]
pub struct ReadSet {
    pub facts: BTreeSet<String>,
    pub derives: BTreeSet<String>,
    /// reads `now[]` (directly or through a callee): fresh every tick
    pub now: bool,
    /// tables its queries read
    pub tables: BTreeSet<String>,
    /// reads a table this program never writes: a source outside the app,
    /// governed by the freshness window
    pub external: bool,
    /// reads no fact a handler writes and no per-session derivation: one
    /// value for the whole app, computed once and shared by every session
    /// (derive-design.md §3.5). Set by `mark_shared`.
    pub shared: bool,
    /// why one held by each session is: the first thing it reads that a
    /// session moves (`it reads 'picked', which a handler writes`)
    pub per_session: Option<String>,
    /// the tables it reads only a slice of — every query of the table fixes
    /// one column by a fact (`WHERE board = ?` with `[board_id]`) — each with
    /// that column and fact: a write that says it touched no row of the
    /// slice leaves the derivation as it is (session-layer-design.md §3.5)
    pub keys: BTreeMap<String, (String, String)>,
    /// the tables it reads whole somewhere, which no key narrows
    pub whole: BTreeSet<String>,
    /// while its own body is walked: the locals that are one fact
    /// (`sid = '`session_id`'`), so a query's parameter set from one reads a
    /// slice keyed by that fact
    aliases: BTreeMap<String, String>,
    /// each function's parameters, for the walk into its body
    params: BTreeMap<String, Vec<String>>,
    /// while a called function's body is walked: its parameters and the
    /// names it assigns, which are its own and not the program's facts or
    /// derivations (a part's `fills` beside the entry's `derive fills`)
    own: BTreeSet<String>,
}

/// The program's facts: top-level assignments that hold state. A database
/// or socket handle is infrastructure and a `fetch`/`poll` binding is a
/// source, not a fact — the rule the compiled program uses to decide what
/// lives in its reactive state, applied by both modes.
pub fn fact_names(stmts: &[Stmt]) -> BTreeSet<String> {
    stmts.iter().filter_map(|s| match s {
        Stmt::Assign { name, value } if !is_handle_or_source(value) && !crate::plain_entry_name(name) => Some(name.clone()),
        _ => None,
    }).collect()
}

fn is_handle_or_source(e: &Expr) -> bool {
    if is_source(e) {
        return true;
    }
    if let Expr::Call { callee, .. } = e {
        return match callee.as_ref() {
            Expr::Ident(n) => n == "connect" || n == "db.connect",
            Expr::FieldAccess { object, field } => matches!(object.as_ref(), Expr::Ident(m) if m == "db") && field == "connect",
            _ => false,
        };
    }
    false
}

/// The elements that bind a fact to an input: the reader's typing writes
/// the second argument, per session, with no handler in between.
const BOUND_INPUTS: &[&str] = &["field", "slider", "checkbox", "toggle", "radio", "tabs", "select"];

/// The facts a session writes — assigned by a handler or a helper a
/// handler calls, or bound to an input in a view. Every other fact is a
/// constant for the life of the program.
pub fn written_facts(fns: &BTreeMap<String, Vec<Stmt>>, facts: &BTreeSet<String>) -> BTreeSet<String> {
    // (a standard module's function, inlined, writes no fact: its locals
    // are its own, whatever the program calls its facts)
    let fns: BTreeMap<String, Vec<Stmt>> = fns.iter().filter(|(n, _)| !crate::modules::is_crow_fn(n)).map(|(n, b)| (n.clone(), b.clone())).collect();
    let fns = &fns;
    fn walk(stmts: &[Stmt], facts: &BTreeSet<String>, out: &mut BTreeSet<String>) {
        for st in stmts {
            match st {
                Stmt::Assign { name, value } => {
                    if facts.contains(name) { out.insert(name.clone()); }
                    walk_e(value, facts, out);
                }
                Stmt::MultiAssign { names, values } => {
                    for n in names { if facts.contains(n) { out.insert(n.clone()); } }
                    for v in values { walk_e(v, facts, out); }
                }
                Stmt::FieldAssign { object, value, .. } | Stmt::IndexAssign { object, value, .. } => {
                    if let Expr::Ident(n) = object { if facts.contains(n) { out.insert(n.clone()); } }
                    walk_e(value, facts, out);
                }
                Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => walk_e(e, facts, out),
                Stmt::If { condition, then_body, else_body } => {
                    walk_e(condition, facts, out);
                    walk(then_body, facts, out);
                    if let Some(eb) = else_body { walk(eb, facts, out); }
                }
                Stmt::For { iter, body, .. } => { walk_e(iter, facts, out); walk(body, facts, out); }
                Stmt::While { condition, body } => { walk_e(condition, facts, out); walk(body, facts, out); }
                _ => {}
            }
        }
    }
    fn walk_e(e: &Expr, facts: &BTreeSet<String>, out: &mut BTreeSet<String>) {
        match e {
            Expr::Call { callee, args } => {
                if let Expr::Ident(n) = callee.as_ref() {
                    if BOUND_INPUTS.contains(&n.as_str()) {
                        if let Some(Expr::Ident(v)) = args.get(1) { if facts.contains(v) { out.insert(v.clone()); } }
                    }
                }
                walk_e(callee, facts, out);
                for a in args { walk_e(a, facts, out); }
            }
            Expr::FieldAccess { object, .. } => walk_e(object, facts, out),
            Expr::BinOp { left, right, .. } => { walk_e(left, facts, out); walk_e(right, facts, out); }
            Expr::UnaryOp { expr, .. } => walk_e(expr, facts, out),
            Expr::Array(items) => for i in items { walk_e(i, facts, out) },
            Expr::Map(pairs) => for (k, v) in pairs { walk_e(k, facts, out); walk_e(v, facts, out); },
            Expr::Named { value, .. } => walk_e(value, facts, out),
            Expr::ForExpr { iter, body, .. } => { walk_e(iter, facts, out); walk(body, facts, out); }
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    for body in fns.values() { walk(body, facts, &mut out); }
    out
}

/// The names the runtime writes for each session, which no handler
/// assigns: a data source (`fetch`/`poll`), whose answers land in the
/// session that asked, and a fact a page's address carries — a `:name` of
/// its path or a name of its `url = […]` — set from the address each reader
/// opens. A derivation over one is the session's (it had been counted the
/// app's: one session's address, or the source's first answer, for all).
pub fn session_written(stmts: &[Stmt], facts: &BTreeSet<String>) -> BTreeMap<String, String> {
    const ADDRESS: &str = "which a page's address sets";
    let mut out = BTreeMap::new();
    for st in stmts {
        match st {
            Stmt::Assign { name, value } if is_source(value) => {
                out.insert(name.clone(), "a source each session's page asks".to_string());
            }
            Stmt::ExprStmt(Expr::Call { callee, args }) if matches!(callee.as_ref(), Expr::Ident(n) if n == "page") => {
                if let Some(Expr::Str(path)) = args.first() {
                    for segment in path.split('/') {
                        if let Some(f) = segment.strip_prefix(':').and_then(|n| address_fact(n, facts)) {
                            out.insert(f, ADDRESS.to_string());
                        }
                    }
                }
                // `url = […]`, or the list the lowering made of it
                let listed = args.iter().skip(2).find_map(|a| match a {
                    Expr::Named { name, value, .. } if name == "url" => Some(value.as_ref()),
                    a @ Expr::Array(_) => Some(a),
                    _ => None,
                });
                if let Some(Expr::Array(items)) = listed {
                    for item in items {
                        if let Expr::Str(n) = item
                            && let Some(f) = address_fact(n, facts)
                        {
                            out.insert(f, ADDRESS.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// A data source's binding: `fetch[url]` or `poll[url, secs]`.
fn is_source(e: &Expr) -> bool {
    matches!(e, Expr::Call { callee, args } if matches!(callee.as_ref(), Expr::Ident(n) if (n == "fetch" && args.len() == 1) || (n == "poll" && args.len() == 2)))
}

/// The fact an address names — a part's by its path (`session.day`) or its
/// last name (`:day`) — as the served page resolves it.
fn address_fact(name: &str, facts: &BTreeSet<String>) -> Option<String> {
    let flat = name.replace('.', "_");
    if facts.contains(&flat) {
        return Some(flat);
    }
    let suffix = format!("_{}", flat);
    let mut found = facts.iter().filter(|f| f.ends_with(&suffix));
    let first = found.next()?.clone();
    found.next().is_none().then_some(first)
}

/// Every derivation's read set, and who holds it: the one analysis the
/// lowering records for both backends and `crow check` prints. `derives`
/// names the program's derivations, `fns` and `params` its functions.
pub fn read_sets(stmts: &[Stmt], derives: &BTreeSet<String>, fns: &BTreeMap<String, Vec<Stmt>>, params: &BTreeMap<String, Vec<String>>) -> BTreeMap<String, ReadSet> {
    let mut facts = fact_names(stmts);
    let session = session_written(stmts, &facts);
    // a source is read as a fact is: each answer that lands moves it
    facts.extend(session.keys().cloned());
    let owned = owned_tables(stmts, fns);
    let mut sets = BTreeMap::new();
    for st in stmts {
        if let Stmt::Derive { name, body, .. } = st {
            sets.insert(name.clone(), read_set(body, &facts, derives, fns, params, &owned));
        }
    }
    let mut written: BTreeMap<String, String> = written_facts(fns, &facts).into_iter().map(|f| (f, "which a session writes".to_string())).collect();
    written.extend(session);
    mark_shared(&mut sets, &written);
    sets
}

/// Which derivations every session shares: those that read no fact a
/// session moves (`written`, each with why) and no derivation that is itself
/// per session — a fixpoint, since a derivation is per session as soon as
/// anything it reads is. A derivation over constants, tables and the clock
/// alone is computed once for the app. Each one held per session keeps why.
pub fn mark_shared(specs: &mut BTreeMap<String, ReadSet>, written: &BTreeMap<String, String>) {
    let mut why: BTreeMap<String, String> = BTreeMap::new();
    for (n, rs) in specs.iter() {
        // `reader` is written by the runtime, per session, whoever signs in
        if let Some(f) = rs.facts.iter().find(|f| *f == "reader" || written.contains_key(*f)) {
            let reason = match written.get(f) {
                Some(w) => format!("it reads '{}', {}", f, w),
                None => "it reads `reader`, the session's signed-in reader".to_string(),
            };
            why.insert(n.clone(), reason);
        }
    }
    loop {
        let before = why.len();
        for (n, rs) in specs.iter() {
            if !why.contains_key(n)
                && let Some(d) = rs.derives.iter().find(|d| *d != n && why.contains_key(*d))
            {
                why.insert(n.clone(), format!("it reads '{}', which each session holds", d));
            }
        }
        if why.len() == before {
            break;
        }
    }
    for (n, rs) in specs.iter_mut() {
        rs.per_session = why.get(n).cloned();
        rs.shared = rs.per_session.is_none();
    }
}

/// The read set of a body, given the program's facts, derivations, the
/// bodies of its functions, and the tables it writes anywhere.
pub fn read_set(
    body: &[Stmt],
    facts: &BTreeSet<String>,
    derives: &BTreeSet<String>,
    fns: &BTreeMap<String, Vec<Stmt>>,
    params: &BTreeMap<String, Vec<String>>,
    owned_tables: &BTreeSet<String>,
) -> ReadSet {
    let mut rs = ReadSet { params: params.clone(), ..ReadSet::default() };
    let mut visited: BTreeSet<String> = BTreeSet::new();
    walk_stmts(body, facts, derives, fns, &mut visited, &mut rs);
    rs.external = rs.tables.iter().any(|t| !owned_tables.contains(t));
    rs.aliases.clear();
    rs.params.clear();
    rs.own.clear();
    rs
}

/// The names a body binds: what it assigns and what its loops walk with.
fn bound_names(stmts: &[Stmt], out: &mut BTreeSet<String>) {
    for st in stmts {
        match st {
            Stmt::Assign { name, .. } => {
                out.insert(name.clone());
            }
            Stmt::MultiAssign { names, .. } => out.extend(names.iter().cloned()),
            Stmt::If { then_body, else_body, .. } => {
                bound_names(then_body, out);
                if let Some(eb) = else_body {
                    bound_names(eb, out);
                }
            }
            Stmt::For { var, body, .. } => {
                out.insert(var.clone());
                bound_names(body, out);
            }
            Stmt::While { body, .. } => bound_names(body, out),
            _ => {}
        }
    }
}

/// The fact a value is exactly: the fact itself, or a string that is only
/// its interpolation (`'`session_id`'`, the id as text for a query).
fn one_fact(e: &Expr, facts: &BTreeSet<String>) -> Option<String> {
    match e {
        Expr::Ident(f) if facts.contains(f) => Some(f.clone()),
        Expr::Str(s) => {
            let t = s.trim();
            let inner = t.strip_prefix('`')?.strip_suffix('`')?;
            if inner.contains('`') {
                return None;
            }
            match interpolation_exprs(t).as_slice() {
                [Expr::Ident(f)] if facts.contains(f) => Some(f.clone()),
                _ => None,
            }
        }
        _ => None,
    }
}

fn walk_stmts(stmts: &[Stmt], facts: &BTreeSet<String>, derives: &BTreeSet<String>, fns: &BTreeMap<String, Vec<Stmt>>, visited: &mut BTreeSet<String>, rs: &mut ReadSet) {
    for st in stmts {
        match st {
            Stmt::Assign { name, value } => {
                walk_expr(value, facts, derives, fns, visited, rs);
                match one_fact(value, facts) {
                    Some(f) => {
                        rs.aliases.insert(name.clone(), f);
                    }
                    None => {
                        rs.aliases.remove(name);
                    }
                }
            }
            Stmt::MultiAssign { values, .. } => for v in values { walk_expr(v, facts, derives, fns, visited, rs) },
            Stmt::FieldAssign { object, value, .. } => { walk_expr(object, facts, derives, fns, visited, rs); walk_expr(value, facts, derives, fns, visited, rs); }
            Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => walk_expr(e, facts, derives, fns, visited, rs),
            Stmt::If { condition, then_body, else_body } => {
                walk_expr(condition, facts, derives, fns, visited, rs);
                walk_stmts(then_body, facts, derives, fns, visited, rs);
                if let Some(eb) = else_body { walk_stmts(eb, facts, derives, fns, visited, rs); }
            }
            Stmt::For { iter, body, .. } => { walk_expr(iter, facts, derives, fns, visited, rs); walk_stmts(body, facts, derives, fns, visited, rs); }
            Stmt::While { condition, body } => { walk_expr(condition, facts, derives, fns, visited, rs); walk_stmts(body, facts, derives, fns, visited, rs); }
            _ => {}
        }
    }
}

/// The expressions inside a string's backtick spans — a derivation read
/// in an interpolation is a read like any other.
pub fn interpolation_exprs(s: &str) -> Vec<Expr> {
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '`' { i += 2; continue; }
        if chars[i] == '`' {
            let mut j = i + 1;
            let mut span = String::new();
            while j < chars.len() && chars[j] != '`' { span.push(chars[j]); j += 1; }
            if j >= chars.len() { break; }
            i = j + 1;
            if span.trim().is_empty() { continue; }
            if let Ok(tokens) = crate::lexer::Lexer::new(&span).tokenize() {
                if let Ok(stmts) = crate::parser::Parser::new(tokens).parse() {
                    let stmts = crate::lower::lower_fragment(&stmts).unwrap_or(stmts);
                    for st in stmts {
                        if let Stmt::ExprStmt(e) = st { out.push(e); }
                    }
                }
            }
            continue;
        }
        i += 1;
    }
    out
}

fn walk_expr(e: &Expr, facts: &BTreeSet<String>, derives: &BTreeSet<String>, fns: &BTreeMap<String, Vec<Stmt>>, visited: &mut BTreeSet<String>, rs: &mut ReadSet) {
    match e {
        Expr::Str(s) if s.contains('`') => {
            for inner in interpolation_exprs(s) { walk_expr(&inner, facts, derives, fns, visited, rs); }
        }
        Expr::Ident(n) => {
            if rs.own.contains(n) {}
            else if derives.contains(n) { rs.derives.insert(n.clone()); }
            else if facts.contains(n) { rs.facts.insert(n.clone()); }
        }
        Expr::Call { callee, args } => {
            if let Expr::Ident(n) = callee.as_ref() {
                if n == "now" { rs.now = true; }
                // the signed-in reader is a fact the runtime keeps: a
                // derivation over it follows sign-ins, and is the session's
                if n == "__reader" { rs.facts.insert("reader".to_string()); }
                // query[conn, 'sql', ...]: the tables the literal reads, and
                // the slice of one it reads when a fact fixes a column of it
                if n == "query" {
                    if let Some(Expr::Str(sql)) = args.get(1) {
                        let params: &[Expr] = match args.get(2) {
                            Some(Expr::Array(items)) => items,
                            _ => &[],
                        };
                        let keyed = crow_runtime::sql::read_keys(sql).and_then(|(t, ks)| {
                            ks.iter().find_map(|(col, i)| match params.get(*i) {
                                Some(Expr::Ident(f)) if facts.contains(f) => Some((t.clone(), (col.clone(), f.clone()))),
                                Some(Expr::Ident(local)) => rs.aliases.get(local).map(|f| (t.clone(), (col.clone(), f.clone()))),
                                _ => None,
                            })
                        });
                        for t in tables_of_sql(sql).0 {
                            rs.tables.insert(t.clone());
                            match &keyed {
                                Some((kt, key)) if *kt == t && !rs.whole.contains(&t) && rs.keys.get(&t).map_or(true, |k| k == key) => {
                                    rs.keys.insert(t, key.clone());
                                }
                                _ => {
                                    rs.keys.remove(&t);
                                    rs.whole.insert(t);
                                }
                            }
                        }
                    }
                }
                if let Some(body) = fns.get(n) {
                    if visited.insert(n.clone()) {
                        // a function's locals are its own: its parameters and
                        // what it binds are not the program's names — but a
                        // fact it assigns is written, not bound
                        let aliases = std::mem::take(&mut rs.aliases);
                        let mut own: BTreeSet<String> = rs.params.get(n).map(|ps| ps.iter().cloned().collect()).unwrap_or_default();
                        let mut bound = BTreeSet::new();
                        bound_names(body, &mut bound);
                        own.extend(bound.into_iter().filter(|b| !facts.contains(b)));
                        let outer = std::mem::replace(&mut rs.own, own);
                        walk_stmts(body, facts, derives, fns, visited, rs);
                        rs.own = outer;
                        rs.aliases = aliases;
                    }
                }
            } else {
                walk_expr(callee, facts, derives, fns, visited, rs);
            }
            for a in args { walk_expr(a, facts, derives, fns, visited, rs); }
        }
        Expr::FieldAccess { object, .. } => walk_expr(object, facts, derives, fns, visited, rs),
        Expr::BinOp { left, right, .. } => { walk_expr(left, facts, derives, fns, visited, rs); walk_expr(right, facts, derives, fns, visited, rs); }
        Expr::UnaryOp { expr, .. } => walk_expr(expr, facts, derives, fns, visited, rs),
        Expr::Array(items) => for i in items { walk_expr(i, facts, derives, fns, visited, rs) },
        Expr::Map(pairs) => for (k, v) in pairs { walk_expr(k, facts, derives, fns, visited, rs); walk_expr(v, facts, derives, fns, visited, rs); },
        Expr::Named { value, .. } => walk_expr(value, facts, derives, fns, visited, rs),
        Expr::ForExpr { iter, body, .. } => { walk_expr(iter, facts, derives, fns, visited, rs); walk_stmts(body, facts, derives, fns, visited, rs); }
        _ => {}
    }
}

/// The tables a SQL literal reads and writes: (reads, writes) — the one
/// reader the compiler and the database layer share.
pub use crow_runtime::sql::tables_of_sql;

/// The functions a body calls, transitively (names in `fns` only).
pub fn called_fns(body: &[Stmt], fns: &BTreeMap<String, Vec<Stmt>>) -> BTreeSet<String> {
    fn walk(stmts: &[Stmt], fns: &BTreeMap<String, Vec<Stmt>>, out: &mut BTreeSet<String>) {
        for st in stmts {
            match st {
                Stmt::Assign { value, .. } | Stmt::ExprStmt(value) | Stmt::Return(Some(value)) => walk_e(value, fns, out),
                Stmt::MultiAssign { values, .. } => for v in values { walk_e(v, fns, out) },
                Stmt::FieldAssign { object, value, .. } => { walk_e(object, fns, out); walk_e(value, fns, out); }
                Stmt::If { condition, then_body, else_body } => { walk_e(condition, fns, out); walk(then_body, fns, out); if let Some(eb) = else_body { walk(eb, fns, out); } }
                Stmt::For { iter, body, .. } => { walk_e(iter, fns, out); walk(body, fns, out); }
                Stmt::While { condition, body } => { walk_e(condition, fns, out); walk(body, fns, out); }
                _ => {}
            }
        }
    }
    fn walk_e(e: &Expr, fns: &BTreeMap<String, Vec<Stmt>>, out: &mut BTreeSet<String>) {
        match e {
            Expr::Call { callee, args } => {
                if let Expr::Ident(n) = callee.as_ref() {
                    if fns.contains_key(n) && out.insert(n.clone()) {
                        walk(&fns[n], fns, out);
                    }
                }
                walk_e(callee, fns, out);
                for a in args { walk_e(a, fns, out); }
            }
            Expr::Ident(n) => { if fns.contains_key(n) && out.insert(n.clone()) { walk(&fns[n], fns, out); } }
            Expr::Str(s) if s.contains('`') => { for inner in interpolation_exprs(s) { walk_e(&inner, fns, out); } }
            Expr::FieldAccess { object, .. } => walk_e(object, fns, out),
            Expr::BinOp { left, right, .. } => { walk_e(left, fns, out); walk_e(right, fns, out); }
            Expr::UnaryOp { expr, .. } => walk_e(expr, fns, out),
            Expr::Array(items) => for i in items { walk_e(i, fns, out) },
            Expr::Map(pairs) => for (k, v) in pairs { walk_e(k, fns, out); walk_e(v, fns, out); },
            Expr::Named { value, .. } => walk_e(value, fns, out),
            Expr::ForExpr { iter, body, .. } => { walk_e(iter, fns, out); walk(body, fns, out); }
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    walk(body, fns, &mut out);
    out
}

/// Every table the program writes: exec and insert literals anywhere, and
/// a query whose statement writes — `INSERT … RETURNING id` reads back the
/// row it wrote, and the table it wrote is written all the same.
pub fn owned_tables(stmts: &[Stmt], fns: &BTreeMap<String, Vec<Stmt>>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    fn walk(stmts: &[Stmt], out: &mut BTreeSet<String>) {
        for st in stmts {
            match st {
                Stmt::Assign { value, .. } | Stmt::ExprStmt(value) | Stmt::Return(Some(value)) => walk_e(value, out),
                Stmt::MultiAssign { values, .. } => for v in values { walk_e(v, out) },
                Stmt::If { condition, then_body, else_body } => { walk_e(condition, out); walk(then_body, out); if let Some(eb) = else_body { walk(eb, out); } }
                Stmt::For { iter, body, .. } => { walk_e(iter, out); walk(body, out); }
                Stmt::While { condition, body } => { walk_e(condition, out); walk(body, out); }
                Stmt::FnDecl { body, .. } | Stmt::Derive { body, .. } | Stmt::TestBlock { body, .. } => walk(body, out),
                _ => {}
            }
        }
    }
    fn walk_e(e: &Expr, out: &mut BTreeSet<String>) {
        match e {
            Expr::Call { callee, args } => {
                if let Expr::Ident(n) = callee.as_ref() {
                    if n == "exec" || n == "insert" || n == "query" {
                        if let Some(Expr::Str(sql)) = args.get(1) { for t in tables_of_sql(sql).1 { out.insert(t); } }
                        if n == "insert" { if let Some(Expr::Str(t)) = args.get(1) { out.insert(t.clone()); } }
                    }
                }
                walk_e(callee, out);
                for a in args { walk_e(a, out); }
            }
            Expr::FieldAccess { object, .. } => walk_e(object, out),
            Expr::BinOp { left, right, .. } => { walk_e(left, out); walk_e(right, out); }
            Expr::UnaryOp { expr, .. } => walk_e(expr, out),
            Expr::Array(items) => for i in items { walk_e(i, out) },
            Expr::Map(pairs) => for (k, v) in pairs { walk_e(k, out); walk_e(v, out); },
            Expr::Named { value, .. } => walk_e(value, out),
            Expr::ForExpr { iter, body, .. } => { walk_e(iter, out); walk(body, out); }
            _ => {}
        }
    }
    walk(stmts, &mut out);
    for body in fns.values() { walk(body, &mut out); }
    out
}

#[cfg(test)]
mod tests {
    /// A body's statements, parsed as the body of a function (a `return`
    /// is a function's, not the top level's).
    fn body_of(src: &str) -> Vec<Stmt> {
        let indented: String = src.lines().map(|l| format!("    {}\n", l)).collect();
        match crate::build(&format!("fn __body[]\n{}", indented)).unwrap().into_iter().find(|s| matches!(s, Stmt::FnDecl { .. })) {
            Some(Stmt::FnDecl { body, .. }) => body,
            _ => Vec::new(),
        }
    }

    use super::*;

    /// A function a derivation calls may name a parameter or a local as the
    /// program names a derivation: they are the function's own, not a read
    /// (finch's book, called by `books`, has a `fills` of its own beside
    /// trading's `derive fills`; the compiled program looped on the cycle)
    #[test]
    fn a_called_functions_own_names_are_not_reads() {
        let derives: BTreeSet<String> = ["fills".to_string(), "books".to_string()].into_iter().collect();
        let fns = crate::build("fn lots[fills]\n    return len[fills]\nfn walk[xs]\n    fills = []\n    for x in xs\n        fills = push[fills, x]\n    return fills\nfn reads[]\n    return fills\n").unwrap();
        let mut bodies = BTreeMap::new();
        let mut params = BTreeMap::new();
        for s in &fns {
            if let Stmt::FnDecl { name, params: ps, body, .. } = s {
                bodies.insert(name.clone(), body.clone());
                params.insert(name.clone(), ps.clone());
            }
        }
        let body = body_of("w = walk[[1, 2]]\nreturn lots[w]\n");
        let rs = read_set(&body, &BTreeSet::new(), &derives, &bodies, &params, &BTreeSet::new());
        assert!(rs.derives.is_empty(), "{:?}", rs.derives);
        let body = body_of("return reads[]\n");
        let rs = read_set(&body, &BTreeSet::new(), &derives, &bodies, &params, &BTreeSet::new());
        assert!(rs.derives.contains("fills"), "a function that reads the derivation still does");
        // a fact a function assigns is written, and its reads are reads
        let facts: BTreeSet<String> = ["playing".to_string()].into_iter().collect();
        let fns = crate::build("fn toggle[]\n    if playing\n        playing = false\n    else\n        playing = true\n").unwrap();
        let (mut bodies, mut params) = (BTreeMap::new(), BTreeMap::new());
        for s in &fns {
            if let Stmt::FnDecl { name, params: ps, body, .. } = s {
                bodies.insert(name.clone(), body.clone());
                params.insert(name.clone(), ps.clone());
            }
        }
        let body = body_of("toggle[]\n");
        let rs = read_set(&body, &facts, &BTreeSet::new(), &bodies, &params, &BTreeSet::new());
        assert!(rs.facts.contains("playing"), "{:?}", rs.facts);
    }

    #[test]
    fn a_query_that_inserts_writes_its_table() {
        let src = "fn start[book]\n    r = query[book, 'INSERT INTO sessions (day) VALUES (?) RETURNING id', ['2026-09-04']]\n    return r\n";
        let stmts = crate::build(src).unwrap();
        let written = owned_tables(&stmts, &BTreeMap::new());
        assert!(written.contains("sessions"), "INSERT … RETURNING through query writes its table: {:?}", written);
        let reads_only = crate::build("fn rows[book]\n    return query[book, 'SELECT id FROM sessions']\n").unwrap();
        assert!(owned_tables(&reads_only, &BTreeMap::new()).is_empty(), "a query that only reads writes nothing");
    }

    /// A derivation that reads a slice of a table — every query of it fixes
    /// one column by a fact — is keyed by that column and fact; a table read
    /// whole anywhere, or by two keys, is read whole
    #[test]
    fn a_derivations_keyed_reads() {
        let facts: BTreeSet<String> = ["board_id".to_string(), "day".to_string()].into_iter().collect();
        let parse = body_of;
        let body = parse("rows = query[book, 'SELECT id, title FROM cards WHERE board = ? ORDER BY id', [board_id]]\nreturn rows\n");
        let rs = read_set(&body, &facts, &BTreeSet::new(), &BTreeMap::new(), &BTreeMap::new(), &BTreeSet::new());
        assert_eq!(rs.keys.get("cards"), Some(&("board".to_string(), "board_id".to_string())));
        let body = parse("a = query[book, 'SELECT id FROM cards WHERE board = ?', [board_id]]\nb = query[book, 'SELECT count(*) AS n FROM cards']\nreturn len[a]\n");
        let rs = read_set(&body, &facts, &BTreeSet::new(), &BTreeMap::new(), &BTreeMap::new(), &BTreeSet::new());
        assert!(rs.keys.is_empty() && rs.whole.contains("cards"));
        let body = parse("a = query[book, 'SELECT id FROM cards WHERE board = ?', [board_id]]\nb = query[book, 'SELECT id FROM cards WHERE day = ?', [day]]\nreturn len[a]\n");
        let rs = read_set(&body, &facts, &BTreeSet::new(), &BTreeMap::new(), &BTreeMap::new(), &BTreeSet::new());
        assert!(rs.keys.is_empty(), "{:?}", rs.keys);
        // a local that is only the fact as text keys the slice by the fact
        let body = parse("b = '`board_id`'\nrows = query[book, 'SELECT id FROM cards WHERE board = ?', [b]]\nreturn rows\n");
        let rs = read_set(&body, &facts, &BTreeSet::new(), &BTreeMap::new(), &BTreeMap::new(), &BTreeSet::new());
        assert_eq!(rs.keys.get("cards"), Some(&("board".to_string(), "board_id".to_string())));
        let body = parse("b = 'board `board_id`'\nrows = query[book, 'SELECT id FROM cards WHERE board = ?', [b]]\nreturn rows\n");
        let rs = read_set(&body, &facts, &BTreeSet::new(), &BTreeMap::new(), &BTreeMap::new(), &BTreeSet::new());
        assert!(rs.keys.is_empty(), "text around the fact is another value");
        // a column fixed by something other than a fact reads the table whole
        let body = parse("rows = query[book, 'SELECT id FROM cards WHERE board = ?', [3]]\nreturn rows\n");
        let rs = read_set(&body, &facts, &BTreeSet::new(), &BTreeMap::new(), &BTreeMap::new(), &BTreeSet::new());
        assert!(rs.keys.is_empty());
    }

    #[test]
    fn sql_tables_read_and_written() {
        let (r, w) = tables_of_sql("SELECT a FROM trades t JOIN sessions s ON s.id = t.session_id WHERE x = ?");
        assert_eq!(r, vec!["sessions", "trades"]);
        assert!(w.is_empty());
        let (_, w) = tables_of_sql("INSERT INTO trades (a) VALUES (?)");
        assert_eq!(w, vec!["trades"]);
        let (_, w) = tables_of_sql("UPDATE trades SET exit_ts = ? WHERE id = ?");
        assert_eq!(w, vec!["trades"]);
        let (_, w) = tables_of_sql("DELETE FROM bookmarks WHERE id = ?");
        assert_eq!(w, vec!["bookmarks"]);
        let (r, _) = tables_of_sql("SELECT symbol FROM (SELECT symbol, close FROM dataset_bars WHERE date = ?) GROUP BY symbol");
        assert_eq!(r, vec!["dataset_bars"], "a subquery is not a table");
    }

    #[test]
    fn facts_skip_handles_and_sources() {
        let src = "use web, ui, db\n\nconn = db.connect['sqlite://:memory:']\nsock = connect['ws://x']\nprices = fetch['http://x']\nsym = 'IMRN'\nqty = 100\n";
        let stmts = crate::build(src).unwrap();
        let facts = fact_names(&stmts);
        assert_eq!(facts.iter().cloned().collect::<Vec<_>>(), vec!["qty", "sym"]);
    }

    #[test]
    fn shared_is_a_fixpoint_over_what_is_read() {
        let mut specs: BTreeMap<String, ReadSet> = BTreeMap::new();
        let rs = |facts: &[&str], derives: &[&str]| ReadSet {
            facts: facts.iter().map(|s| s.to_string()).collect(),
            derives: derives.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        specs.insert("universe".into(), rs(&["day"], &[]));          // day is never written: shared
        specs.insert("latest".into(), rs(&[], &["universe"]));        // reads a shared one: shared
        specs.insert("head".into(), rs(&["base", "speed"], &[]));     // base is written: per session
        specs.insert("rows".into(), rs(&[], &["head", "latest"]));    // reads a per-session one: per session
        let written: BTreeMap<String, String> = [("base".to_string(), "which a session writes".to_string())].into_iter().collect();
        mark_shared(&mut specs, &written);
        assert!(specs["universe"].shared);
        assert!(specs["latest"].shared);
        assert!(!specs["head"].shared);
        assert!(!specs["rows"].shared);
        assert_eq!(specs["head"].per_session.as_deref(), Some("it reads 'base', which a session writes"));
        assert_eq!(specs["rows"].per_session.as_deref(), Some("it reads 'head', which each session holds"));
        assert_eq!(specs["universe"].per_session, None);
    }

    /// A fact a page's address carries, and a data source, are moved by each
    /// session: a derivation over one is the session's, never the app's
    #[test]
    fn an_address_or_a_source_makes_a_derivation_the_sessions() {
        let src = "use web, ui\n\nn = 0\nday = 'd'\nk = 1\nprices = poll['http://127.0.0.1:9/p.json', 5]\n\nderive doubled\n    return n * 2\n\nderive dated\n    return day\n\nderive count\n    return len[prices]\n\nderive plain\n    return k + 1\n\nview render[]\n    return p['`doubled` `dated` `count` `plain`']\n\npage['/', render, url = ['day']]\npage['/n/:n', render]\n";
        let stmts = crate::build(src).unwrap();
        let derives: BTreeSet<String> = ["doubled", "dated", "count", "plain"].iter().map(|s| s.to_string()).collect();
        let fns: BTreeMap<String, Vec<Stmt>> = stmts.iter().filter_map(|s| if let Stmt::FnDecl { name, body, .. } = s { Some((name.clone(), body.clone())) } else { None }).collect();
        let sets = read_sets(&stmts, &derives, &fns, &BTreeMap::new());
        assert_eq!(sets["doubled"].per_session.as_deref(), Some("it reads 'n', which a page's address sets"));
        assert_eq!(sets["dated"].per_session.as_deref(), Some("it reads 'day', which a page's address sets"));
        assert_eq!(sets["count"].per_session.as_deref(), Some("it reads 'prices', a source each session's page asks"));
        assert!(sets["count"].facts.contains("prices"), "a source's answers move what reads it");
        assert!(sets["plain"].shared);
    }

    #[test]
    fn written_facts_sees_handlers_helpers_and_bound_inputs() {
        let src = "use web, ui\n\nqty = 1\nsym = 'a'\nday = 'd'\nnote = ''\nspeed = 1\n\nfn bump[]\n    qty = qty + 1\n\nfn rename[]\n    if true\n        sym = 'b'\n\nview render[]\n    return stack[field['note', note], slider['speed', speed, 1, 10, 1], p[day]]\n";
        let stmts = crate::build(src).unwrap();
        let facts = fact_names(&stmts);
        let fns: BTreeMap<String, Vec<Stmt>> = stmts.iter().filter_map(|s| if let Stmt::FnDecl { name, body, .. } = s { Some((name.clone(), body.clone())) } else { None }).collect();
        let w = written_facts(&fns, &facts);
        assert_eq!(w.iter().cloned().collect::<Vec<_>>(), vec!["note", "qty", "speed", "sym"], "day is only read: a constant");
    }
}
