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
                // (a control binds a fact to an input: the reader's typing
                // writes it, per session, with no handler in between)
                if let Expr::Ident(n) = callee.as_ref() {
                    if let Some(i) = crate::table::binds(n) {
                        if let Some(Expr::Ident(v)) = args.get(i) { if facts.contains(v) { out.insert(v.clone()); } }
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
    matches!(e, Expr::Call { callee, args } if matches!(callee.as_ref(), Expr::Ident(n) if crate::table::source(n, args.len())))
}

/// The fact an address names — a part's by its path (`session.day`) or its
/// last name (`:day`) — as the served page resolves it: the one rule the
/// check, both backends and the read sets call. A part's fact is its
/// prefix and the name, so `:model_name` is `myna·models·model_name` and
/// never `myna·models·new_model_name`, which ends with the same words (read
/// by the end alone when the prefix joined with an underscore, the two had
/// been ambiguous and the interpreted page refused at start, while the
/// compiled one served: Myna's model page, 2026-11-06). Where no part's
/// prefix is known (a lone file), the one fact ending in the joiner and
/// the name is it.
/// Whether a part's fact (its prefix, then its own name) is the one
/// written: its own name is the written path's last segment, and the
/// segments before it, if any, end the part's prefix (`notebooks.current`
/// is `myna·notebooks·` and `current`).
fn of_part(prefix: &str, own: &str, written: &str) -> bool {
    let (lead, last) = written.rsplit_once('.').unwrap_or(("", written));
    if own != last {
        return false;
    }
    let join = crate::PART_JOIN.to_string();
    let lead = lead.replace('.', &join);
    let part = prefix.trim_end_matches(crate::PART_JOIN);
    lead.is_empty() || part == lead || part.ends_with(&format!("{}{}", join, lead))
}

pub(crate) fn address_fact<'a>(name: &str, facts: impl IntoIterator<Item = &'a String>) -> Option<String> {
    let join = crate::PART_JOIN.to_string();
    let flat = name.replace('.', &join);
    let suffix = format!("{}{}", join, flat);
    let mut exact = None;
    let mut by_part = Vec::new();
    let mut by_end = Vec::new();
    for f in facts {
        if *f == flat {
            exact = Some(f.clone());
        } else if f.ends_with(&suffix) {
            match crate::part_prefix_of_declaration(f) {
                Some(prefix) if of_part(&prefix, &f[prefix.len()..], name) => by_part.push(f.clone()),
                Some(_) => {}
                None => by_end.push(f.clone()),
            }
        }
    }
    if exact.is_some() {
        return exact;
    }
    let found = if by_part.is_empty() { by_end } else { by_part };
    (found.len() == 1).then(|| found[0].clone())
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
    // (the files are the program's own: what it reads of them moves when
    // it writes one — noted as it runs — never on a freshness window)
    rs.external = rs.tables.iter().any(|t| t != crow_runtime::io::FILES && !owned_tables.contains(t));
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
            if let Ok(tokens) = crate::lexer::Lexer::joined(&span).tokenize() {
                if let Ok(stmts) = crate::parser::Parser::for_fragment(tokens).parse() {
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
                // a builtin that reads a file: the files are read as a table
                // is, so a handler's write to any file moves the derivation
                // (it had been computed once, and showed a file the program
                // had since written as it was)
                if !fns.contains_key(n) && crate::table::effects_of(n).contains(&crate::effects::Effect::Files) {
                    rs.tables.insert(crow_runtime::io::FILES.to_string());
                    rs.whole.insert(crow_runtime::io::FILES.to_string());
                }
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
                // (a module's reader through its name: `frame.load[path]`)
                if let Expr::FieldAccess { field, .. } = callee.as_ref()
                    && crate::table::effects_of(field).contains(&crate::effects::Effect::Files)
                {
                    rs.tables.insert(crow_runtime::io::FILES.to_string());
                    rs.whole.insert(crow_runtime::io::FILES.to_string());
                }
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

/// Every table the program writes: insert literals anywhere, and a query
/// whose statement writes.
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
                    if n == "insert" || n == "query" {
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

/// What a view reads, so a page re-runs only the views whose inputs moved
/// (one-view-design.md 3.4): the facts and the derivations it reads, itself
/// or through what it calls (another view, a helper, a function it holds
/// as a value), and why it runs at every render where it must.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewReads {
    pub facts: BTreeSet<String>,
    pub derives: BTreeSet<String>,
    /// Why the view runs at every render: it reaches a builtin that answers
    /// by the run rather than by the page's state — the clock, a draw, the
    /// session's notice, whether background work runs, a value's capture
    /// moment (`table::impure`; the console alone moves nothing the page
    /// shows).
    pub always: Option<String>,
    /// Whether a call of it can be kept by its arguments: each parameter a
    /// value that compares cheaply (a number, a text, a truth, a moment, a
    /// list, map or record of them), one its copy is handed from a source
    /// (keyed by the source's version) or one read only in its bindings
    /// (left out of the key). A call of a view that takes a frame or a
    /// function handed otherwise runs whenever its caller does.
    pub keyed: bool,
    /// The facts and derivations it reads only in its bindings, itself or
    /// through what it calls (`bind_sites`): one that moved alone is set on
    /// the page with no view run. A fact read anywhere else too is read by
    /// the view's structure, as every read of a view was before. A local
    /// assigned once from such a value, and a parameter its copy is handed
    /// such a value for (`specialize`), are read where they are read.
    pub bound: BTreeSet<String>,
    /// How each parameter is keyed (`ArgKey`, from the view's copy).
    pub args: Vec<ArgUse>,
}

/// How a view's call is keyed by one of its parameters (`ViewReads.args`).
#[derive(Debug, Clone, PartialEq)]
pub enum ArgUse {
    /// by its value's canonical text
    Value,
    /// left out of the key: read only in the view's bindings, which are
    /// computed again from the session where the value moves
    Bound,
    /// by the version of the fact or derivation it is the value of (or a
    /// field of): `derive` when a derivation
    Source { root: String, derive: bool },
}

type Fns<'a> = BTreeMap<&'a str, &'a crate::ir::Fn>;

/// Every view's read set, over the lowered program both modes run: one
/// analysis, so the interpreter and the compiled program keep and re-run
/// the same views at the same moments. `structs` names each record's
/// field types, for whether a parameter compares cheaply.
pub fn view_reads(fns: &[&crate::ir::Fn], structs: &BTreeMap<String, Vec<crate::ir::Ty>>) -> BTreeMap<String, ViewReads> {
    use crate::ir::{ArgKey, Expr as I, FnKind, Target};
    let by_name: Fns = fns.iter().map(|f| (f.name.as_str(), *f)).collect();
    let find = |n: &str| by_name.get(n).copied().or_else(|| by_name.get(crate::ir::copy_base(n)).copied());
    let pure = pure_fns(&by_name);
    // each function's reads, by how it reads them, made once
    let mut each: BTreeMap<String, Classified> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for f in fns.iter().filter(|f| f.kind == FnKind::View) {
        let mut reads = ViewReads::default();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut todo: Vec<&crate::ir::Fn> = vec![f];
        let mut structural: BTreeSet<Read> = BTreeSet::new();
        let mut in_binds: BTreeSet<Read> = BTreeSet::new();
        while let Some(g) = todo.pop() {
            if !seen.insert(g.name.clone()) {
                continue;
            }
            let c = each.entry(g.name.clone()).or_insert_with(|| {
                let env = locals_env(g, analysis_env(g), &pure);
                classify(&g.body, &env, &pure, &by_name, &|callee, j, _| find(callee).is_some_and(|h| matches!(h.args.get(j), Some(ArgKey::Bound(_)))))
            });
            structural.extend(c.structural.iter().cloned());
            in_binds.extend(c.bound.iter().cloned());
            for n in &c.calls {
                if let Some(callee) = find(n)
                    && callee.kind != FnKind::Handler
                {
                    todo.push(callee);
                }
            }
            crate::ir::visit_block(&g.body, &mut |e| {
                if let I::Call { target: Target::Row(_) | Target::Runtime(_) | Target::Module(..), .. } | I::Effect { .. } = e
                    && let Some(n) = crate::ir::call_name(e)
                {
                    let console = crate::table::effects_of(&n).iter().all(|k| *k == crate::effects::Effect::Console) && !crate::table::effects_of(&n).is_empty();
                    // the signed-in reader is a fact the runtime keeps
                    if n == "__reader" {
                        reads.facts.insert("reader".to_string());
                    } else if reads.always.is_none() && crate::table::impure(&n) && !console {
                        reads.always = Some(format!("it calls {}[], which answers by the run", n));
                    }
                }
            });
        }
        for r in structural.iter().chain(in_binds.iter()) {
            match r {
                Read::Fact(n) => {
                    reads.facts.insert(n.clone());
                }
                Read::Derive(n) => {
                    reads.derives.insert(n.clone());
                }
                Read::Name(_) => {}
            }
        }
        reads.bound = in_binds
            .iter()
            .filter(|r| !structural.contains(r))
            .filter_map(|r| match r {
                Read::Fact(n) | Read::Derive(n) => Some(n.clone()),
                Read::Name(_) => None,
            })
            .collect();
        reads.args = f
            .params
            .iter()
            .enumerate()
            .map(|(i, _)| match f.args.get(i) {
                Some(ArgKey::Bound(_)) => ArgUse::Bound,
                Some(ArgKey::Source { root, derive, .. }) => ArgUse::Source { root: root.clone(), derive: *derive },
                _ => ArgUse::Value,
            })
            .collect();
        reads.keyed = f.params.iter().zip(&reads.args).all(|((_, t), a)| *a != ArgUse::Value || keyable(t, structs, &mut BTreeSet::new()));
        out.insert(f.name.clone(), reads);
    }
    out
}

/// What a read is of: a fact, a derivation, or (for the analysis that
/// decides which parameters bind) a parameter or a local by its name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Read {
    Fact(String),
    Derive(String),
    Name(String),
}

/// A body's reads, by how it makes them.
#[derive(Debug, Default)]
struct Classified {
    /// read where what is read decides what the view makes
    structural: BTreeSet<Read>,
    /// read in a bound place, or in an argument a view's copy is handed to
    /// bind (each a fact or a derivation the session moves alone)
    bound: BTreeSet<Read>,
    /// the functions it calls, or holds as values, outside its bound places
    calls: BTreeSet<String>,
}

/// What a function's names stand for, as a binding reads them: a scalar
/// one computed from the session's scalar facts and derivations alone, by
/// that expression (`vals`); one holding a fact's or a derivation's value,
/// or a field of one, by the source and the expression (`srcs`).
#[derive(Debug, Clone, Default)]
pub(crate) struct Env {
    vals: BTreeMap<String, crate::ir::Expr>,
    srcs: BTreeMap<String, (String, bool, crate::ir::Expr)>,
}

/// A view's parameters as the analysis of its reads sees them: one its
/// copy binds stands for itself (its reads are its caller's), one handed
/// from a source is that source.
fn analysis_env(f: &crate::ir::Fn) -> Env {
    use crate::ir::{ArgKey, Expr as I};
    let mut env = Env::default();
    for (i, (p, t)) in f.params.iter().enumerate() {
        match f.args.get(i) {
            Some(ArgKey::Bound(_)) => {
                env.vals.insert(p.clone(), I::Local(p.clone(), t.clone()));
            }
            Some(ArgKey::Source { root, derive, expr }) => {
                env.srcs.insert(p.clone(), (root.clone(), *derive, expr.clone()));
            }
            _ => {}
        }
    }
    env
}

/// A view's parameters as its bindings read them: one its copy binds is
/// the expression its call sites hand it.
fn binding_env(f: &crate::ir::Fn) -> Env {
    use crate::ir::ArgKey;
    let mut env = analysis_env(f);
    for (i, (p, _)) in f.params.iter().enumerate() {
        if let Some(ArgKey::Bound(e)) = f.args.get(i) {
            env.vals.insert(p.clone(), e.clone());
        }
    }
    env
}

/// The names a body gives a value to, each with how many times it does:
/// a local's first value and every later one, a field or an item set in
/// place, a loop's variable.
fn assigned(body: &[crate::ir::Stmt]) -> BTreeMap<String, usize> {
    use crate::ir::{Expr as I, Stmt as S};
    fn walk(b: &[S], out: &mut BTreeMap<String, usize>) {
        for s in b {
            match s {
                S::Let { var, .. } | S::Set { var, .. } | S::SetPath { var, .. } => *out.entry(var.clone()).or_insert(0) += 1,
                S::For { var, body, .. } => {
                    *out.entry(var.clone()).or_insert(0) += 2;
                    walk(body, out);
                }
                S::If { then, els, .. } => {
                    walk(then, out);
                    if let Some(e) = els {
                        walk(e, out);
                    }
                }
                S::While { body, .. } => walk(body, out),
                _ => {}
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(body, &mut out);
    // a collecting loop's variable and the locals its body sets
    crate::ir::visit_block(body, &mut |e| {
        if let I::ForExpr { var, body, .. } = e {
            *out.entry(var.clone()).or_insert(0) += 2;
            walk(body, &mut out);
        }
    });
    out
}

/// `base` with the locals of `f` that bind added (one-view-design.md 3.5):
/// a local given its one value at the body's top level, never set again —
/// not in a branch, not in a loop — from an expression a binding can
/// compute (a scalar of the session's facts and derivations, through what
/// answers by its arguments alone, other such locals and the parameters
/// `base` binds) stands for that expression; one given a fact's or a
/// derivation's value, or a field of one, stands for its source. Any other
/// local, and any parameter set by the body, is the view's own.
/// Whether values are followed through locals and parameters (phase 3):
/// `CROW_FOLLOW=off` leaves every local and every argument structural, as
/// before — to compare, as `CROW_VALUES=off` does for values turns.
fn following() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !matches!(std::env::var("CROW_FOLLOW").as_deref(), Ok("off") | Ok("0")))
}

fn locals_env(f: &crate::ir::Fn, mut base: Env, pure: &BTreeSet<String>) -> Env {
    use crate::ir::Stmt as S;
    let counts = assigned(&f.body);
    // a parameter the body sets is no binding's and no source's
    for (p, _) in &f.params {
        if counts.contains_key(p) {
            base.vals.remove(p);
            base.srcs.remove(p);
        }
    }
    if !following() {
        return base;
    }
    let params: BTreeSet<&str> = f.params.iter().map(|(p, _)| p.as_str()).collect();
    for s in &f.body {
        if let S::Let { var, ty, expr } = s
            && counts.get(var) == Some(&1)
            && !params.contains(var.as_str())
        {
            if held(ty) && bindable(expr, &base, pure) {
                let x = subst(expr, &base);
                base.vals.insert(var.clone(), x);
            } else if let Some(src) = source_of(expr, &base) {
                base.srcs.insert(var.clone(), src);
            }
        }
    }
    base
}

/// The types a bound local holds: a binding's own, or a list of them.
fn held(t: &crate::ir::Ty) -> bool {
    scalar(t) || matches!(t, crate::ir::Ty::List(i) if scalar(i))
}

/// The fact or derivation an expression is the value of, or a field of
/// (through the locals and parameters `env` says hold one): its name,
/// whether it is a derivation, and the expression over it.
fn source_of(e: &crate::ir::Expr, env: &Env) -> Option<(String, bool, crate::ir::Expr)> {
    use crate::ir::Expr as I;
    match e {
        I::Load(f, _) => Some((f.clone(), false, e.clone())),
        I::Derived(d, _) => Some((d.clone(), true, e.clone())),
        I::Local(n, _) | I::Move(n, _) => env.srcs.get(n).cloned(),
        I::Field { base, name, ty } => source_of(base, env).map(|(r, d, b)| (r, d, I::Field { base: Box::new(b), name: name.clone(), ty: ty.clone() })),
        _ => None,
    }
}

/// The functions of the program a binding may call: they answer by their
/// arguments alone — no effect, no fact written, no node, no function
/// handed as a value, nothing that answers by the run, and only such
/// functions called.
fn pure_fns(fns: &Fns) -> BTreeSet<String> {
    use crate::ir::{Expr as I, FnKind, Stmt as S, Target};
    let mut pure: BTreeSet<String> = fns.values().filter(|f| f.kind == FnKind::Pure && f.args.is_empty()).map(|f| f.name.clone()).collect();
    loop {
        let before = pure.len();
        let snapshot = pure.clone();
        pure.retain(|n| {
            let Some(f) = fns.get(n.as_str()) else { return false };
            let mut ok = true;
            fn stores(b: &[S]) -> bool {
                b.iter().any(|s| match s {
                    S::Store { .. } => true,
                    S::If { then, els, .. } => stores(then) || els.as_deref().is_some_and(stores),
                    S::While { body, .. } | S::For { body, .. } => stores(body),
                    _ => false,
                })
            }
            if stores(&f.body) {
                return false;
            }
            crate::ir::visit_block(&f.body, &mut |e| match e {
                I::Effect { .. } | I::Tree { .. } | I::FnRef(..) | I::Deferred { .. } | I::Call { target: Target::Module(..), .. } => ok = false,
                I::Call { target: Target::User(g), .. } => {
                    if !snapshot.contains(g) && !snapshot.contains(crate::ir::copy_base(g)) {
                        ok = false;
                    }
                }
                I::Call { target: Target::Row(_) | Target::Runtime(_), .. } => match crate::ir::call_name(e) {
                    Some(n) if !n.starts_with("__") && !crate::table::impure(&n) => {}
                    _ => ok = false,
                },
                I::ForExpr { body, .. } if stores(body) => ok = false,
                _ => {}
            });
            ok
        });
        if pure.len() == before {
            return pure;
        }
    }
}

/// A body's reads, sorted by how it makes them (`Classified`), with what
/// `env` says its names stand for. A bound place (`sites_in`) is read in
/// its binding; an argument `bound_arg` says a view's copy binds is read
/// as its binding (it is computed again from the session); a local `env`
/// binds is read where it is read, as its expression; everything else is
/// read by the view's structure.
fn classify(body: &[crate::ir::Stmt], env: &Env, pure: &BTreeSet<String>, fns: &Fns, bound_arg: &dyn Fn(&str, usize, &crate::ir::Expr) -> bool) -> Classified {
    use crate::ir::{Expr as I, PathStep, Stmt as S, Target};
    struct W<'a> {
        env: &'a Env,
        pure: &'a BTreeSet<String>,
        fns: &'a Fns<'a>,
        bound_arg: &'a dyn Fn(&str, usize, &I) -> bool,
        out: Classified,
    }
    impl W<'_> {
        /// what a bound place or a bound argument reads
        fn bound(&mut self, e: &I) {
            let x = subst(e, self.env);
            let reads = reads_of(&x, self.fns);
            // a place or an argument that reads two of the parameters the
            // analysis holds bound reads each by the view's structure: a
            // call that binds one and not the other would leave it no
            // binding's (`binding_params`)
            let held: Vec<&String> = reads.iter().filter_map(|r| match r {
                Read::Name(n) if matches!(self.env.vals.get(n), Some(I::Local(m, _)) if m == n) => Some(n),
                _ => None,
            }).collect();
            if held.len() > 1 {
                for n in held {
                    self.out.structural.insert(Read::Name(n.clone()));
                }
            }
            for r in reads {
                self.out.bound.insert(r);
            }
        }
        fn block(&mut self, b: &[S]) {
            for s in b {
                match s {
                    // a local that binds is read where it is read
                    S::Let { var, expr, .. } => {
                        if !self.env.vals.contains_key(var) {
                            self.expr(expr);
                        }
                    }
                    S::Set { expr, .. } | S::Store { expr, .. } | S::Do(expr) => self.expr(expr),
                    S::SetPath { path, expr, .. } => {
                        for p in path {
                            if let PathStep::Index(i) = p {
                                self.expr(i);
                            }
                        }
                        self.expr(expr);
                    }
                    S::If { cond, then, els } => {
                        self.expr(cond);
                        self.block(then);
                        if let Some(e) = els {
                            self.block(e);
                        }
                    }
                    S::While { cond, body } => {
                        self.expr(cond);
                        self.block(body);
                    }
                    S::For { iter, body, .. } => {
                        self.expr(iter);
                        self.block(body);
                    }
                    S::Return(Some(e)) => self.expr(e),
                    S::Propagate { values, .. } => {
                        for v in values {
                            self.name(v);
                        }
                    }
                    _ => {}
                }
            }
        }
        fn name(&mut self, n: &str) {
            match (self.env.vals.get(n), self.env.srcs.get(n)) {
                (Some(x), _) => {
                    for r in reads_of(x, self.fns) {
                        self.out.structural.insert(r);
                    }
                }
                (None, Some((root, derive, _))) => {
                    self.out.structural.insert(if *derive { Read::Derive(root.clone()) } else { Read::Fact(root.clone()) });
                }
                _ => {}
            }
            self.out.structural.insert(Read::Name(n.to_string()));
        }
        fn expr(&mut self, e: &I) {
            match e {
                I::Load(n, _) => {
                    self.out.structural.insert(Read::Fact(n.clone()));
                }
                I::Derived(n, _) => {
                    self.out.structural.insert(Read::Derive(n.clone()));
                }
                I::Local(n, _) | I::Move(n, _) => self.name(n),
                I::FnRef(n, _) => {
                    self.out.calls.insert(n.clone());
                }
                I::Tree { element, attrs, children, .. } => {
                    let sites = sites_in(element, attrs, children, self.env, self.pure);
                    for (i, c) in children.iter().enumerate() {
                        if sites.iter().any(|(at, ..)| *at == crate::ir::BindAt::Child(i)) {
                            self.bound(c);
                        } else {
                            self.expr(c);
                        }
                    }
                    for (k, v) in attrs {
                        if sites.iter().any(|(at, ..)| matches!(at, crate::ir::BindAt::Option(n) if n == k)) {
                            self.bound(v);
                        } else {
                            self.expr(v);
                        }
                    }
                }
                I::Call { target: Target::User(g), args, .. } => {
                    self.out.calls.insert(g.clone());
                    for (j, a) in args.iter().enumerate() {
                        if (self.bound_arg)(g, j, a) {
                            self.bound(a);
                        } else {
                            self.expr(a);
                        }
                    }
                }
                I::ForExpr { iter, body, .. } => {
                    self.expr(iter);
                    self.block(body);
                }
                other => {
                    // its own parts, each read as it is read
                    for x in direct_parts(other) {
                        self.expr(x);
                    }
                }
            }
        }
    }
    let mut w = W { env, pure, fns, bound_arg, out: Classified::default() };
    w.block(body);
    w.out
}

/// An expression's own parts, one level down.
fn direct_parts(e: &crate::ir::Expr) -> Vec<&crate::ir::Expr> {
    use crate::ir::{Expr as I, Piece};
    match e {
        I::Call { args, .. } | I::Effect { args, .. } | I::Deferred { args, .. } => args.iter().collect(),
        I::Convert { expr, .. } | I::Unary { expr, .. } => vec![expr],
        I::Binary { l, r, .. } => vec![l, r],
        I::Index { base, indices, .. } => std::iter::once(&**base).chain(indices.iter()).collect(),
        I::Field { base, .. } | I::Meta { base, .. } => vec![base],
        I::Interp(pieces) => pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Expr(x) => Some(x),
                Piece::Text(_) => None,
            })
            .collect(),
        I::ListLit(items, _) => items.iter().collect(),
        I::MapLit(pairs, _) => pairs.iter().map(|(_, v)| v).collect(),
        I::RecordLit { fields, .. } => fields.iter().collect(),
        I::Tree { attrs, children, .. } => attrs.iter().map(|(_, v)| v).chain(children.iter()).collect(),
        _ => Vec::new(),
    }
}

/// The facts, derivations and names an expression reads, through the
/// functions of the program it calls.
fn reads_of<'a>(e: &crate::ir::Expr, fns: &Fns<'a>) -> BTreeSet<Read> {
    use crate::ir::{Expr as I, Target};
    let mut out = BTreeSet::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut todo: Vec<&'a crate::ir::Fn> = Vec::new();
    let note = |x: &I, out: &mut BTreeSet<Read>, todo: &mut Vec<&'a crate::ir::Fn>, seen: &mut BTreeSet<String>| match x {
        I::Load(n, _) => {
            out.insert(Read::Fact(n.clone()));
        }
        I::Derived(n, _) => {
            out.insert(Read::Derive(n.clone()));
        }
        I::Local(n, _) | I::Move(n, _) => {
            out.insert(Read::Name(n.clone()));
        }
        I::Call { target: Target::User(g), .. } => {
            if let Some(f) = fns.get(g.as_str()).or_else(|| fns.get(crate::ir::copy_base(g)))
                && seen.insert(f.name.clone())
            {
                todo.push(f);
            }
        }
        _ => {}
    };
    crate::ir::visit_expr(e, &mut |x| note(x, &mut out, &mut todo, &mut seen));
    while let Some(f) = todo.pop() {
        let mut inner = BTreeSet::new();
        crate::ir::visit_block(&f.body, &mut |x| note(x, &mut inner, &mut todo, &mut seen));
        // (a called function's own locals are none of the caller's)
        out.extend(inner.into_iter().filter(|r| !matches!(r, Read::Name(_))));
    }
    out
}

/// `e` with every local and parameter `env` binds replaced by the
/// expression it stands for.
fn subst(e: &crate::ir::Expr, env: &Env) -> crate::ir::Expr {
    use crate::ir::Expr as I;
    let mut x = e.clone();
    if env.vals.is_empty() {
        return x;
    }
    crate::ir::visit_expr_mut(&mut x, &mut |y| {
        if let I::Local(n, _) | I::Move(n, _) = y
            && let Some(v) = env.vals.get(n)
            && !matches!(v, I::Local(m, _) if m == n)
        {
            *y = v.clone();
        }
    });
    x
}

/// Copies of the views for their call sites (one-view-design.md 3.4,
/// 3.5, phase 3): where a call hands a view, for a parameter the view
/// reads only in its bindings (`binding_params`), a scalar the program
/// computes from the session's facts and derivations alone, or, for a
/// parameter of a kind that does not compare cheaply (a frame, a list, a
/// map, a record), a fact's or a derivation's value or a field of one, the
/// call is to a copy of the view (`site_name`) that says so in its `args`:
/// the first is left out of the copy's key and its places are set from the
/// caller's expression, the second keys the copy by its source's version.
/// Calls that hand the same kinds of value share a copy; a view that can
/// call itself is never copied. Run at the end of the lowering both modes
/// share, before `bind_sites`; a program that would need more copies than
/// `MOST` keeps every call as it is.
pub fn specialize(program: &mut crate::ir::Program) {
    use crate::ir::{ArgKey, Expr as I, FnKind, Target};
    const MOST: usize = 4096;
    let all: Vec<crate::ir::Fn> = program.parts.iter().flat_map(|p| p.fns.iter().cloned()).collect();
    let by_name: Fns = all.iter().map(|f| (f.name.as_str(), f)).collect();
    if !following() || !all.iter().any(|f| f.kind == FnKind::View) {
        return;
    }
    let pure = pure_fns(&by_name);
    let looping = looping_views(&by_name);
    let caps = binding_params(&by_name, &pure, &looping);
    // the view a call names, when a copy of it may be made
    let view_of = |n: &str| by_name.get(n).copied().filter(|f| f.kind == FnKind::View && f.args.is_empty() && !looping.contains(&f.name));
    // the plan: each function's calls, by their place among its calls of
    // the program's functions, that go to a copy; and the copies
    let mut copies: BTreeMap<String, String> = BTreeMap::new();
    let mut made: Vec<crate::ir::Fn> = Vec::new();
    let mut numbers: BTreeMap<String, usize> = BTreeMap::new();
    let mut rewrite: BTreeMap<(String, usize), String> = BTreeMap::new();
    let mut todo: Vec<crate::ir::Fn> = all.iter().filter(|f| f.kind == FnKind::View).cloned().collect();
    let mut at = 0;
    while at < todo.len() {
        let f = todo[at].clone();
        at += 1;
        let env = locals_env(&f, binding_env(&f), &pure);
        let mut k = 0;
        let mut over = false;
        crate::ir::visit_block(&f.body, &mut |e| {
            let I::Call { target: Target::User(g), args, .. } = e else { return };
            let here = k;
            k += 1;
            let Some(view) = view_of(g) else { return };
            if view.params.len() != args.len() || over {
                return;
            }
            let kinds: Vec<ArgKey> = view.params.iter().zip(args.iter()).enumerate().map(|(j, ((_, pty), a))| arg_kind(&view.name, j, pty, a, &env, &caps, &pure)).collect();
            if kinds.iter().all(|k| matches!(k, ArgKey::Value)) {
                return;
            }
            let key = format!("{} {:?}", view.name, kinds);
            let name = match copies.get(&key) {
                Some(n) => n.clone(),
                None => {
                    if copies.len() >= MOST {
                        over = true;
                        return;
                    }
                    let n = numbers.entry(view.name.clone()).or_insert(0);
                    *n += 1;
                    let name = crate::ir::site_name(&view.name, *n);
                    let mut copy = view.clone();
                    copy.name = name.clone();
                    copy.args = kinds;
                    copies.insert(key, name.clone());
                    made.push(copy.clone());
                    todo.push(copy);
                    name
                }
            };
            rewrite.insert((f.name.clone(), here), name);
        });
        if over {
            return;
        }
    }
    if rewrite.is_empty() {
        return;
    }
    // the copies stand beside the views they copy, in their parts
    for copy in made {
        let base = crate::ir::site_base(&copy.name).unwrap_or(&copy.name).to_string();
        let pj = program.parts.iter().position(|p| p.fns.iter().any(|f| f.name == base)).unwrap_or(0);
        program.parts[pj].fns.push(copy);
    }
    for part in &mut program.parts {
        for f in part.fns.iter_mut().filter(|f| f.kind == FnKind::View) {
            let name = f.name.clone();
            let mut k = 0;
            crate::ir::visit_block_mut(&mut f.body, &mut |e| {
                let I::Call { target: Target::User(g), .. } = e else { return };
                if let Some(to) = rewrite.get(&(name.clone(), k)) {
                    *g = to.clone();
                }
                k += 1;
            });
        }
    }
}

/// What a call hands a view's parameter `j` of type `pty` (`specialize`):
/// a binding where the parameter may be bound (`caps`), the argument is
/// of its type, a binding can compute it under the caller's `env`, and it
/// reads the session; a source where the parameter does not compare
/// cheaply and the argument is a fact's or a derivation's value or a field
/// of one; else its value.
fn arg_kind(view: &str, j: usize, pty: &crate::ir::Ty, a: &crate::ir::Expr, env: &Env, caps: &BTreeMap<String, BTreeSet<usize>>, pure: &BTreeSet<String>) -> crate::ir::ArgKey {
    use crate::ir::ArgKey;
    if caps.get(view).is_some_and(|c| c.contains(&j)) && a.ty() == *pty && bindable(a, env, pure) {
        let x = subst(a, env);
        if reads_state(&x) {
            return ArgKey::Bound(x);
        }
    }
    if !plain(pty)
        && let Some((root, derive, expr)) = source_of(a, env)
    {
        return ArgKey::Source { root, derive, expr };
    }
    ArgKey::Value
}

/// The views that can call themselves, directly or through other views:
/// never copied, since each copy's calls would make another.
fn looping_views(fns: &Fns) -> BTreeSet<String> {
    use crate::ir::{Expr as I, FnKind, Target};
    let views: BTreeMap<&str, BTreeSet<String>> = fns
        .values()
        .filter(|f| f.kind == FnKind::View)
        .map(|f| {
            let mut calls = BTreeSet::new();
            crate::ir::visit_block(&f.body, &mut |e| {
                if let I::Call { target: Target::User(g), .. } = e
                    && fns.get(g.as_str()).is_some_and(|h| h.kind == FnKind::View)
                {
                    calls.insert(g.clone());
                }
            });
            (f.name.as_str(), calls)
        })
        .collect();
    let mut out = BTreeSet::new();
    for start in views.keys() {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut todo: Vec<&str> = views[start].iter().map(|s| s.as_str()).collect();
        while let Some(v) = todo.pop() {
            if v == *start {
                out.insert(start.to_string());
                break;
            }
            if seen.insert(v)
                && let Some(next) = views.get(v)
            {
                todo.extend(next.iter().map(|s| s.as_str()));
            }
        }
    }
    out
}

/// The types a view's key compares as they are, cheaply: a source keys a
/// parameter of any other.
fn plain(t: &crate::ir::Ty) -> bool {
    use crate::ir::Ty;
    matches!(t, Ty::Int | Ty::Big | Ty::Decimal | Ty::Fraction | Ty::Float | Ty::Bool | Ty::Str | Ty::Markup | Ty::Moment)
}

/// Each view's parameters a call may bind (one-view-design.md 3.5): a
/// scalar parameter the body never sets and reads only in bound places, in
/// locals that bind, and in arguments of its type to such parameters of
/// the views it calls — never beside another such parameter in one place
/// or argument, so a call binds each on its own. The greatest such set,
/// found by striking out every parameter read anywhere else until none is.
/// A view that can call itself binds none.
fn binding_params(fns: &Fns, pure: &BTreeSet<String>, looping: &BTreeSet<String>) -> BTreeMap<String, BTreeSet<usize>> {
    use crate::ir::{Expr as I, FnKind};
    let views: Vec<&crate::ir::Fn> = fns.values().copied().filter(|f| f.kind == FnKind::View && f.args.is_empty()).collect();
    let mut caps: BTreeMap<String, BTreeSet<usize>> = views
        .iter()
        .map(|f| {
            let ps = if looping.contains(&f.name) { BTreeSet::new() } else { f.params.iter().enumerate().filter(|(_, (_, t))| scalar(t)).map(|(i, _)| i).collect() };
            (f.name.clone(), ps)
        })
        .collect();
    loop {
        let mut changed = false;
        for f in &views {
            let now = caps.get(&f.name).cloned().unwrap_or_default();
            if now.is_empty() {
                continue;
            }
            let mut env = Env::default();
            for i in &now {
                let (p, t) = &f.params[*i];
                env.vals.insert(p.clone(), I::Local(p.clone(), t.clone()));
            }
            let env = locals_env(f, env, pure);
            let held: BTreeSet<String> = now.iter().map(|i| f.params[*i].0.clone()).collect();
            let snapshot = caps.clone();
            let c = classify(&f.body, &env, pure, fns, &|g, j, a| {
                let Some(h) = fns.get(g).filter(|h| h.kind == FnKind::View && h.args.is_empty() && !looping.contains(&h.name)) else { return false };
                snapshot.get(g).is_some_and(|c| c.contains(&j)) && h.params.get(j).is_some_and(|(_, t)| a.ty() == *t) && bindable(a, &env, pure) && {
                    let x = subst(a, &env);
                    reads_state(&x) || reads_of(&x, fns).iter().any(|r| matches!(r, Read::Name(n) if held.contains(n)))
                }
            });
            let keep: BTreeSet<usize> = now.iter().copied().filter(|i| !c.structural.contains(&Read::Name(f.params[*i].0.clone())) && env.vals.contains_key(&f.params[*i].0)).collect();
            if keep != now {
                caps.insert(f.name.clone(), keep);
                changed = true;
            }
        }
        if !changed {
            return caps;
        }
    }
}

use crow_web::view::tree as vt;

/// The places of the views a live page sets with no view run
/// (one-view-design.md 3.5, phase 3): in an element a view writes, a
/// child or an option whose value the program computes from the
/// session's scalar facts and derivations alone — a text, a number or a
/// truth, of constants, facts and derivations of those types, operators,
/// spans, the builtins that answer by their arguments alone and the
/// functions of the program that do (`pure_fns`), locals given one such
/// value and parameters a view's copy is handed one for (`specialize`) —
/// or a bound control (`field`, `slider`, `checkbox`, `toggle`, `radio`,
/// `tabs`, `select`, `progress`) given only such values. A place that
/// reads any other local or parameter, a field, an index or a builtin
/// that answers by the run is no binding: what it shows is the view's to
/// compute. Each binding is numbered program-wide by its text, its locals
/// and parameters replaced by what they stand for (the same expression in
/// the same kind of place is one binding), so both modes number it alike;
/// the numbers are the part's `bindings`, and each element's own are its
/// `binds`.
pub fn bind_sites(program: &mut crate::ir::Program) {
    use crate::ir::{BindAt, Binding, Expr as I, FnKind};
    let all: Vec<crate::ir::Fn> = program.parts.iter().flat_map(|p| p.fns.iter().cloned()).collect();
    let by_name: Fns = all.iter().map(|f| (f.name.as_str(), f)).collect();
    let pure = pure_fns(&by_name);
    // every site's text, the first time it is met, in a first pass; the
    // ids, by the texts' order, in the second
    let mut found: BTreeMap<String, Binding> = BTreeMap::new();
    let mut envs: BTreeMap<String, Env> = BTreeMap::new();
    for f in all.iter().filter(|f| f.kind == FnKind::View) {
        let env = locals_env(f, binding_env(f), &pure);
        crate::ir::visit_block(&f.body, &mut |e| {
            if let I::Tree { element, attrs, children, .. } = e {
                for (at, key, x) in sites_in(element, attrs, children, &env, &pure) {
                    let inputs = inputs_of(&x, &by_name);
                    found.entry(key).or_insert_with(|| Binding { id: 0, element: element.clone(), option: matches!(at, BindAt::Option(_)), expr: x, inputs });
                }
            }
        });
        envs.insert(f.name.clone(), env);
    }
    let ids: BTreeMap<String, u32> = found.keys().enumerate().map(|(i, k)| (k.clone(), i as u32)).collect();
    let bindings: Vec<Binding> = found
        .into_values()
        .enumerate()
        .map(|(i, mut b)| {
            b.id = i as u32;
            b
        })
        .collect();
    for part in &mut program.parts {
        for f in part.fns.iter_mut().filter(|f| f.kind == FnKind::View) {
            let Some(env) = envs.get(&f.name) else { continue };
            crate::ir::visit_block_mut(&mut f.body, &mut |e| {
                if let I::Tree { element, attrs, children, binds } = e {
                    *binds = sites_in(element, attrs, children, env, &pure).into_iter().filter_map(|(at, key, _)| ids.get(&key).map(|id| (at, *id))).collect();
                }
            });
        }
    }
    if let Some(part) = program.parts.first_mut() {
        part.bindings = bindings;
    }
}

/// An element's bound places: where each stands, its text (the binding's
/// key) and its expression, its locals and parameters replaced by what
/// `env` says they stand for.
fn sites_in(element: &str, attrs: &[(String, crate::ir::Expr)], children: &[crate::ir::Expr], env: &Env, pure: &BTreeSet<String>) -> Vec<(crate::ir::BindAt, String, crate::ir::Expr)> {
    use crate::ir::BindAt;
    let mut out = Vec::new();
    if !vt::element_binds(element) {
        return out;
    }
    for (i, c) in children.iter().enumerate() {
        if (scalar(&c.ty()) && bindable(c, env, pure)) || bound_control(c, env, pure) {
            let x = subst(c, env);
            if reads_state(&x) || bound_name(&x, env) {
                out.push((BindAt::Child(i), format!("child {} {:?}", element, x), x));
            }
        }
    }
    for (k, v) in attrs {
        // (an option given twice is the last; neither is a binding)
        if attrs.iter().filter(|(n, _)| n == k).count() == 1 && vt::option_binds(element, k) && scalar(&v.ty()) && bindable(v, env, pure) {
            let x = subst(v, env);
            if reads_state(&x) || bound_name(&x, env) {
                out.push((BindAt::Option(k.clone()), format!("option {} {:?}", k, x), x));
            }
        }
    }
    out
}

/// Whether a substituted expression still reads a name `env` binds to
/// itself: a parameter the analysis holds bound while it decides which
/// are (`binding_params`). No binding's expression keeps one.
fn bound_name(x: &crate::ir::Expr, env: &Env) -> bool {
    use crate::ir::Expr as I;
    let mut found = false;
    crate::ir::visit_expr(x, &mut |y| {
        if let I::Local(n, _) | I::Move(n, _) = y
            && matches!(env.vals.get(n), Some(I::Local(m, _)) if m == n)
        {
            found = true;
        }
    });
    found
}

/// The types a binding's value is: text, a number (an int, a growing
/// int, a float, a decimal, a fraction), a truth, a moment — each shown
/// as a render shows it, by the code the view's own place runs.
fn scalar(t: &crate::ir::Ty) -> bool {
    use crate::ir::Ty;
    matches!(t, Ty::Int | Ty::Big | Ty::Float | Ty::Decimal | Ty::Fraction | Ty::Str | Ty::Bool | Ty::Moment)
}

/// The types a bound control's variable is: what the page sends back
/// into a fact as it is (text, an int, a float, a truth).
fn control_scalar(t: &crate::ir::Ty) -> bool {
    use crate::ir::Ty;
    matches!(t, Ty::Int | Ty::Float | Ty::Str | Ty::Bool)
}

/// Whether the expression is computed from the session's scalar facts and
/// derivations alone, by what answers by its arguments alone: `env`'s
/// locals and parameters stand for what it says.
fn bindable(e: &crate::ir::Expr, env: &Env, pure: &BTreeSet<String>) -> bool {
    use crate::ir::{Convert, Expr as I, Piece, Target};
    match e {
        I::Const(..) | I::Constant(..) => true,
        I::Load(_, t) | I::Derived(_, t) => scalar(t),
        I::Local(n, t) | I::Move(n, t) => held(t) && env.vals.contains_key(n),
        I::Interp(pieces) => pieces.iter().all(|p| match p {
            Piece::Text(_) => true,
            Piece::Expr(x) => !x.ty().is_result() && bindable(x, env, pure),
        }),
        I::Binary { l, r, ty, .. } => !ty.is_result() && bindable(l, env, pure) && bindable(r, env, pure),
        I::Unary { expr, ty, .. } => !ty.is_result() && bindable(expr, env, pure),
        I::Convert { how: Convert::IntToFloat | Convert::ToBig | Convert::ToDecimal | Convert::ToFraction | Convert::Proven | Convert::Known, expr, ty } => !ty.is_result() && bindable(expr, env, pure),
        I::ListLit(items, _) => items.iter().all(|x| bindable(x, env, pure)),
        // a builtin that answers a value of these types by its arguments
        // alone (a node, a control, nothing: no binding's)
        I::Call { target: Target::Row(_) | Target::Runtime(_), args, ty, .. } => {
            let Some(n) = crate::ir::call_name(e) else { return false };
            held(ty) && !ty.is_result() && !n.starts_with("__") && !crate::table::impure(&n) && args.iter().all(|x| bindable(x, env, pure))
        }
        // a function of the program's that does (`pure_fns`)
        I::Call { target: Target::User(g), args, ty, .. } => held(ty) && !ty.is_result() && (pure.contains(g) || pure.contains(crate::ir::copy_base(g))) && args.iter().all(|x| !x.ty().is_result() && bindable(x, env, pure)),
        _ => false,
    }
}

/// A control given only what a binding reads (`tree::control_binds` says
/// which controls, and where their variable is): its variable a scalar
/// fact, its other arguments computed as a binding's are.
fn bound_control(e: &crate::ir::Expr, env: &Env, pure: &BTreeSet<String>) -> bool {
    use crate::ir::{Const, Expr as I, Target};
    let I::Call { target: Target::Runtime(n), args, .. } = e else { return false };
    let kind = match args.get(2) {
        Some(I::Const(Const::Str(k), _)) => Some(k.as_str()),
        _ => None,
    };
    let Some(var) = vt::control_binds(n, args.len(), matches!(args.first(), Some(I::MapLit(..))), kind) else { return false };
    let plain = args.iter().all(|a| !a.ty().is_result() && bindable(a, env, pure));
    match var {
        Some(i) => plain && matches!(args.get(i), Some(I::Load(_, t)) if control_scalar(t)),
        None => plain,
    }
}

/// Whether the expression reads a fact or a derivation.
fn reads_state(e: &crate::ir::Expr) -> bool {
    use crate::ir::Expr as I;
    let mut found = false;
    crate::ir::visit_expr(e, &mut |x| {
        if matches!(x, I::Load(..) | I::Derived(..)) {
            found = true;
        }
    });
    found
}

/// The facts and derivations an expression reads, through the functions
/// of the program it calls, each once, in name order.
fn inputs_of(e: &crate::ir::Expr, fns: &Fns) -> Vec<String> {
    reads_of(e, fns)
        .into_iter()
        .filter_map(|r| match r {
            Read::Fact(n) | Read::Derive(n) => Some(n),
            Read::Name(_) => None,
        })
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect()
}

/// Whether a value of the type compares cheaply, as a view's argument
/// keeps the view's last tree: a number, a text, a truth, a moment, and a
/// list, a map, a record or a value that can fail of them.
fn keyable(t: &crate::ir::Ty, structs: &BTreeMap<String, Vec<crate::ir::Ty>>, seen: &mut BTreeSet<String>) -> bool {
    use crate::ir::Ty;
    match t {
        Ty::Int | Ty::Big | Ty::Decimal | Ty::Fraction | Ty::Float | Ty::Bool | Ty::Str | Ty::Markup | Ty::Moment => true,
        Ty::List(i) | Ty::Map(_, i) | Ty::Result(i) => keyable(i, structs, seen),
        Ty::Record(n) => {
            if !seen.insert(n.clone()) {
                return true;
            }
            structs.get(n).is_some_and(|fields| fields.iter().all(|f| keyable(f, structs, seen)))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    /// A body's statements, parsed as the body of a function (a `return`
    /// is a function's, not the top level's).
    fn body_of(src: &str) -> Vec<Stmt> {
        let indented: String = src.lines().map(|l| format!("    {}\n", l)).collect();
        match crate::build(&format!("fn _body[]\n{}", indented)).unwrap().into_iter().find(|s| matches!(s, Stmt::FnDecl { .. })) {
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

    // ---- phase 3: values followed through locals and parameters ----

    /// A program lowered as both modes run it: its functions (views' copies
    /// for their call sites among them) and what each view reads.
    fn lowered(src: &str) -> (Vec<crate::ir::Fn>, BTreeMap<String, ViewReads>) {
        let (program, _) = crate::ir_interp::prepare(&crate::build(src).unwrap(), "t").unwrap();
        let fns: Vec<crate::ir::Fn> = program.parts.iter().flat_map(|p| p.fns.iter().cloned()).collect();
        let refs: Vec<&crate::ir::Fn> = fns.iter().collect();
        let reads = view_reads(&refs, &BTreeMap::new());
        (fns, reads)
    }

    fn reads_of<'a>(reads: &'a BTreeMap<String, ViewReads>, view: &str) -> &'a ViewReads {
        reads.iter().find(|(n, _)| n.as_str() == view || n.ends_with(&format!("{}{}", crate::PART_JOIN, view))).map(|(_, r)| r).unwrap_or_else(|| panic!("no view {} in {:?}", view, reads.keys()))
    }

    fn copies<'a>(fns: &'a [crate::ir::Fn], view: &str) -> Vec<&'a crate::ir::Fn> {
        fns.iter().filter(|f| crate::ir::site_base(&f.name).is_some_and(|b| b == view || b.ends_with(&format!("{}{}", crate::PART_JOIN, view)))).collect()
    }

    const HEAD: &str = "use web, ui\n\nclock = 0\nflag = false\nrows = ['a', 'b']\n\nderive secs\n    return clock * 10\n\n";

    /// A local given one value from a bound expression is that expression:
    /// the place that shows it is a binding, and the derivation it reads is
    /// read only there. A local assigned again, or in a branch, is not.
    #[test]
    fn a_local_assigned_once_is_followed_to_its_value() {
        let (_, reads) = lowered(&format!("{}view once[]\n    t = secs\n    half = t / 2\n    return p['at `half`']\n\nview twice[]\n    t = secs\n    t = t + 1\n    return p['at `t`']\n\nview branch[]\n    t = 0\n    if flag\n        t = secs\n    return p['at `t`']\n\nview render[]\n    return stack[once[], twice[], branch[]]\n\npage['/', render]\n", HEAD));
        let once = reads_of(&reads, "once");
        assert!(once.bound.contains("secs") && once.derives.contains("secs"), "{:?}", once.bound);
        for v in ["twice", "branch"] {
            let r = reads_of(&reads, v);
            assert!(!r.bound.contains("secs") && r.derives.contains("secs"), "{}: {:?}", v, r.bound);
        }
    }

    /// A scalar handed to a parameter its view shows only in bound places
    /// is a bound argument: the call gets a copy of the view for its site
    /// that leaves it out of the key, and the caller reads what it handed
    /// only in bindings. A parameter the view's structure reads, or that is
    /// mixed with another bound parameter, stays keyed; a view that calls
    /// itself gets no copy.
    #[test]
    fn a_scalar_shown_only_in_bound_places_is_a_bound_argument() {
        let (fns, reads) = lowered(&format!("{}view shows[s]\n    return p['at `s`']\n\nview shapes[s]\n    if s gt 2\n        return p['late']\n    return p['early']\n\nview mixes[s, u]\n    return p['`s + u`']\n\nview nest[n]\n    if n eq 0\n        return p['end']\n    return stack[p['`n`'], nest[n - 1]]\n\nview top[]\n    t = secs\n    return stack[shows[t], shows[clock]]\n\nview other[]\n    return stack[shapes[secs], mixes[secs, clock], nest[clock]]\n\nview render[]\n    return stack[top[], other[]]\n\npage['/', render]\n", HEAD));
        let made = copies(&fns, "shows");
        assert_eq!(made.len(), 2, "a copy per expression handed: its bindings compute that expression");
        assert!(made.iter().all(|f| matches!(f.args.as_slice(), [crate::ir::ArgKey::Bound(_)])), "{:?}", made.iter().map(|f| &f.args).collect::<Vec<_>>());
        let top = reads_of(&reads, "top");
        assert!(top.bound.contains("secs") && top.bound.contains("clock"), "{:?}", top.bound);
        assert!(copies(&fns, "shapes").is_empty(), "its structure reads s");
        assert!(copies(&fns, "mixes").is_empty(), "s and u are read together");
        assert!(copies(&fns, "nest").is_empty(), "a view reached from itself");
        let other = reads_of(&reads, "other");
        assert!(!other.bound.contains("secs") && !other.bound.contains("clock"), "{:?}", other.bound);
    }

    /// A growing int (`floor[…]`, an int past 64 bits), a decimal and a
    /// moment bind as an int does: as a child, through a local assigned
    /// once and through a parameter; a fraction that can fail does not.
    #[test]
    fn a_big_a_decimal_and_a_moment_bind_as_an_int_does() {
        let (fns, reads) = lowered(&format!("{}derive whole\n    return floor[float[clock] * 1.5]\n\nderive huge\n    return 9223372036854775000 + clock * 1000\n\nderive price\n    return decimal[clock] / decimal[4]\n\nderive at\n    return moment[float[clock] * 60.0]\n\nderive part\n    return fraction[clock, 3]\n\nview head[s]\n    return p['head `s`']\n\nview shown[]\n    t = whole\n    return stack[p[huge], p['`price` `at`'], head[t]]\n\nview failing[]\n    return p['`part`']\n\nview render[]\n    return stack[shown[], failing[]]\n\npage['/', render]\n", HEAD));
        let shown = reads_of(&reads, "shown");
        for d in ["whole", "huge", "price", "at"] {
            assert!(shown.bound.contains(d), "{}: {:?}", d, shown.bound);
        }
        assert!(copies(&fns, "head").iter().all(|f| matches!(f.args.as_slice(), [crate::ir::ArgKey::Bound(_)])) && !copies(&fns, "head").is_empty(), "the growing int is a bound argument");
        assert!(!reads_of(&reads, "failing").bound.contains("part"), "a fraction that can fail is structural");
    }

    /// A list handed from a fact or a derivation (or a local alias of one)
    /// is keyed by its source's version: the call gets a copy whose
    /// argument says so. A list computed in the view stays keyed by value.
    #[test]
    fn a_list_from_its_source_is_keyed_by_the_source() {
        let (fns, _) = lowered(&format!("{}derive marked\n    out = for r in rows\n        '`r`!'\n    return out\n\nview list_of[xs]\n    out = for x in xs\n        p[x]\n    return stack[out]\n\nview render[]\n    d = marked\n    made = push[rows, 'c']\n    return stack[list_of[rows], list_of[d], list_of[made]]\n\npage['/', render]\n", HEAD));
        let made = copies(&fns, "list_of");
        let roots: BTreeSet<(String, bool)> = made
            .iter()
            .filter_map(|f| match f.args.as_slice() {
                [crate::ir::ArgKey::Source { root, derive, .. }] => Some((root.clone(), *derive)),
                _ => None,
            })
            .collect();
        assert!(roots.iter().any(|(r, d)| r.ends_with("rows") && !d), "{:?}", roots);
        assert!(roots.iter().any(|(r, d)| r.ends_with("marked") && *d), "{:?}", roots);
        assert_eq!(made.len(), 2, "the list made in the view is keyed by value: {:?}", made.iter().map(|f| &f.args).collect::<Vec<_>>());
    }

}
