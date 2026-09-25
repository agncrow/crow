//! The app's reachable states (app-baseline-design.md §7, B2): from the
//! declared facts, every state the wired handlers can bring a page to,
//! each with the controls it offers. A fact that some guard reads takes
//! small, abstract values — a literal it is declared with, written with or
//! offered by an input, or `*`, some other value — so "Buy is offered with
//! no session" is a state (`session_id = 0` offering Buy), not only a
//! path. Guards are the baseline's canonical text, read back as
//! expressions and judged true, false or unknown; unknown keeps both ways.
//! Nothing runs: a query's cell, a derivation, an input's text is unknown.

use crate::ast::{BinOp, Expr, Stmt, UnaryOp};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// One path through a handler, as its signature has it: its conditions,
/// the facts it writes and what with, and what it may write in a loop.
pub struct HandlerPath {
    pub conds: Vec<String>,
    pub writes: Vec<(String, String)>,
    pub may: Vec<(String, String)>,
}

/// What the machine is built from.
pub struct Program<'a> {
    /// fact → its declared value, canonical
    pub initial: &'a BTreeMap<String, String>,
    /// handler → its paths
    pub handlers: &'a BTreeMap<String, Vec<HandlerPath>>,
    /// view → (guard conditions, tree text) per path
    pub views: &'a BTreeMap<String, Vec<(Vec<String>, String)>>,
}

/// The most states a page's search visits before it stops and says so.
const MAX_STATES: usize = 4096;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tri {
    T,
    F,
    U,
}

impl Tri {
    fn not(self) -> Tri {
        match self {
            Tri::T => Tri::F,
            Tri::F => Tri::T,
            Tri::U => Tri::U,
        }
    }
    fn and(self, o: Tri) -> Tri {
        match (self, o) {
            (Tri::F, _) | (_, Tri::F) => Tri::F,
            (Tri::T, Tri::T) => Tri::T,
            _ => Tri::U,
        }
    }
    fn or(self, o: Tri) -> Tri {
        match (self, o) {
            (Tri::T, _) | (_, Tri::T) => Tri::T,
            (Tri::F, Tri::F) => Tri::F,
            _ => Tri::U,
        }
    }
}

/// A value as a state knows it.
#[derive(Clone, PartialEq, Debug)]
enum Val {
    /// a literal, as canonical text
    Lit(String),
    /// the fact holds some value other than its literals
    Other(String),
    Unknown,
}

/// A state: each tracked fact's value, a literal's text or `*`.
type State = BTreeMap<String, String>;

/// One control a state offers.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Control {
    /// `desc`, the handler, the arguments frozen at the control
    Fires { desc: String, handler: String, args: Vec<String> },
    /// `desc`, the fact an input binds, the literal options it offers
    Binds { desc: String, fact: String, options: Option<Vec<String>> },
}

impl Control {
    fn text(&self) -> String {
        match self {
            Control::Fires { desc, handler, args } if args.is_empty() => format!("{} → {}", desc, handler),
            Control::Fires { desc, handler, args } => format!("{} → {}[{}]", desc, handler, args.join(", ")),
            Control::Binds { desc, fact, .. } => format!("{} binds {}", desc, fact),
        }
    }
}

/// Whether canonical text is a literal: a number, a truth value, text
/// without an interpolation, or a list of literals.
fn is_literal(t: &str) -> bool {
    let t = t.trim();
    if t == "true" || t == "false" {
        return true;
    }
    if t.parse::<f64>().is_ok() {
        return true;
    }
    if let Some(inner) = t.strip_prefix("(-").and_then(|r| r.strip_suffix(')')) {
        return inner.parse::<f64>().is_ok();
    }
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') && !t.contains('`') {
        return true;
    }
    if t.starts_with('[') && t.ends_with(']') {
        let inner = &t[1..t.len() - 1];
        return inner.trim().is_empty() || split_top(inner).iter().all(|x| is_literal(x));
    }
    false
}

/// Two literals are the same value: numbers by value (`1` is `1.0`).
fn same_literal(a: &str, b: &str) -> bool {
    match (num(a), num(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a.trim() == b.trim(),
    }
}

fn num(t: &str) -> Option<f64> {
    let t = t.trim();
    if let Some(inner) = t.strip_prefix("(-").and_then(|r| r.strip_suffix(')')) {
        return inner.parse::<f64>().ok().map(|v| -v);
    }
    t.parse::<f64>().ok()
}

/// A comma-separated list split at the top level only (`a, f[b, c], 'd, e'`).
fn split_top(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '\'' => {
                in_str = !in_str;
                cur.push(c);
            }
            '[' | '(' if !in_str => {
                depth += 1;
                cur.push(c);
            }
            ']' | ')' if !in_str => {
                depth -= 1;
                cur.push(c);
            }
            ',' if !in_str && depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// The identifiers canonical text mentions.
fn idents(t: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut word = String::new();
    let mut in_str = false;
    for c in t.chars() {
        if c == '\'' {
            in_str = !in_str;
        }
        if !in_str && (c.is_alphanumeric() || c == '_') {
            word.push(c);
        } else {
            if !word.is_empty() && !word.chars().next().unwrap().is_ascii_digit() {
                out.insert(word.clone());
            }
            word.clear();
        }
    }
    if !word.is_empty() && !word.chars().next().unwrap().is_ascii_digit() {
        out.insert(word);
    }
    out
}

/// A guard's text as the machine reads it: `ne` as the language writes it,
/// and the page's reader signed in (the lowered `has[__reader[], 'name']`)
/// as the pseudo-fact `reader`, which `sign_in` and `sign_out` write.
fn normalize(t: &str) -> String {
    t.replace(" neq ", " ne ").replace("has[__reader[], 'name']", "(reader eq 'someone')")
}

/// A handler's parameters, `$1 … $n`, replaced by the arguments a control
/// freezes; unfrozen ones stay unknown.
fn substitute(t: &str, args: &[String]) -> String {
    let mut out = t.to_string();
    // longest first: `$12` before `$1`
    for (i, a) in args.iter().enumerate().rev() {
        out = out.replace(&format!("${}", i + 1), a);
    }
    out
}

/// One element line of a tree's text: its name, its attributes, its label.
fn element_line(line: &str) -> Option<(String, Vec<String>, String)> {
    let l = line.trim();
    let open = l.find('[')?;
    let name = &l[..open];
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let mut depth = 0i32;
    let mut in_str = false;
    let mut close = None;
    for (i, c) in l.char_indices().skip(open) {
        match c {
            '\'' => in_str = !in_str,
            '[' if !in_str => depth += 1,
            ']' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let attrs = split_top(&l[open + 1..close]);
    Some((name.to_string(), attrs, l[close + 1..].trim().to_string()))
}

struct Machine<'a> {
    p: &'a Program<'a>,
    /// the facts some guard reads: the only ones a state tracks
    tracked: BTreeSet<String>,
    /// fact → the literals it can hold
    literals: BTreeMap<String, Vec<String>>,
    parsed: HashMap<String, Option<Expr>>,
}

impl<'a> Machine<'a> {
    /// Canonical text as an expression, once per text.
    fn expr(&mut self, t: &str) -> Option<Expr> {
        if let Some(e) = self.parsed.get(t) {
            return e.clone();
        }
        let mut src = normalize(t);
        // a parameter no control froze is unknown
        while let Some(i) = src.find('$') {
            let digits: String = src[i + 1..].chars().take_while(|c| c.is_ascii_digit()).collect();
            src.replace_range(i..i + 1 + digits.len(), &format!("__param_{}", digits));
        }
        let parsed = crate::build(&format!("__value = {}\n", src)).ok().and_then(|stmts| {
            stmts.into_iter().find_map(|s| match s {
                Stmt::Assign { value, .. } => Some(value),
                _ => None,
            })
        });
        self.parsed.insert(t.to_string(), parsed.clone());
        parsed
    }

    fn value(&self, e: &Expr, s: &State) -> Val {
        match e {
            Expr::Ident(n) if s.contains_key(n) => {
                let v = &s[n];
                if v == "*" {
                    Val::Other(n.clone())
                } else {
                    Val::Lit(v.clone())
                }
            }
            Expr::Int(n) => Val::Lit(n.to_string()),
            Expr::Float(f) => Val::Lit(format!("{}", f)),
            Expr::Bool(b) => Val::Lit(b.to_string()),
            Expr::Str(t) if !t.contains('`') => Val::Lit(format!("'{}'", t)),
            Expr::UnaryOp { op: UnaryOp::Neg, expr } => match expr.as_ref() {
                Expr::Int(n) => Val::Lit((-n).to_string()),
                Expr::Float(f) => Val::Lit(format!("{}", -f)),
                _ => Val::Unknown,
            },
            _ => Val::Unknown,
        }
    }

    fn truth(&self, e: &Expr, s: &State) -> Tri {
        match e {
            Expr::Bool(true) => Tri::T,
            Expr::Bool(false) => Tri::F,
            Expr::UnaryOp { op: UnaryOp::Not, expr } => self.truth(expr, s).not(),
            Expr::BinOp { left, op: BinOp::And, right } => self.truth(left, s).and(self.truth(right, s)),
            Expr::BinOp { left, op: BinOp::Or, right } => self.truth(left, s).or(self.truth(right, s)),
            Expr::BinOp { left, op, right } if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte) => {
                let (l, r) = (self.value(left, s), self.value(right, s));
                let eq = match (&l, &r) {
                    (Val::Lit(a), Val::Lit(b)) => {
                        if same_literal(a, b) {
                            Tri::T
                        } else {
                            Tri::F
                        }
                    }
                    // some other value is none of the fact's literals
                    (Val::Other(f), Val::Lit(b)) | (Val::Lit(b), Val::Other(f)) => {
                        if self.literals.get(f).is_some_and(|ls| ls.iter().any(|x| same_literal(x, b))) {
                            Tri::F
                        } else {
                            Tri::U
                        }
                    }
                    _ => Tri::U,
                };
                match op {
                    BinOp::Eq => eq,
                    BinOp::Ne => eq.not(),
                    _ => match (&l, &r) {
                        // a number whose only literal is 0 and which holds
                        // something else holds a positive number: an id,
                        // zero or a session (app-baseline-design.md §7)
                        (Val::Other(f), Val::Lit(b)) | (Val::Lit(b), Val::Other(f)) if self.zero_or_some(f) && num(b).is_some_and(|y| y <= 0.0) => {
                            let other_on_left = matches!(l, Val::Other(_));
                            let greater = match op {
                                BinOp::Gt | BinOp::Gte => other_on_left,
                                _ => !other_on_left,
                            };
                            if greater {
                                Tri::T
                            } else {
                                Tri::F
                            }
                        }
                        (Val::Lit(a), Val::Lit(b)) => match (num(a), num(b)) {
                            (Some(x), Some(y)) => {
                                let holds = match op {
                                    BinOp::Gt => x > y,
                                    BinOp::Lt => x < y,
                                    BinOp::Gte => x >= y,
                                    _ => x <= y,
                                };
                                if holds {
                                    Tri::T
                                } else {
                                    Tri::F
                                }
                            }
                            _ => Tri::U,
                        },
                        _ => Tri::U,
                    },
                }
            }
            Expr::Ident(_) => match self.value(e, s) {
                Val::Lit(v) if v == "true" => Tri::T,
                Val::Lit(v) if v == "false" => Tri::F,
                _ => Tri::U,
            },
            _ => Tri::U,
        }
    }

    /// A number whose only literal is 0: `*` is some positive number.
    fn zero_or_some(&self, f: &str) -> bool {
        self.literals.get(f).is_some_and(|ls| !ls.is_empty() && ls.iter().all(|x| num(x) == Some(0.0)))
    }

    /// Whether all of a path's conditions can hold in the state.
    fn holds(&mut self, conds: &[String], s: &State, args: &[String]) -> Tri {
        let mut all = Tri::T;
        for c in conds {
            let c = substitute(c, args);
            let t = match self.expr(&c) {
                Some(e) => self.truth(&e, s),
                None => Tri::U,
            };
            all = all.and(t);
            if all == Tri::F {
                break;
            }
        }
        all
    }

    /// The controls a view offers in a state: each path whose guard can
    /// hold, its controls and its sub-views'.
    fn offers(&mut self, view: &str, s: &State, seen: &mut BTreeSet<String>, out: &mut BTreeSet<Control>) {
        if !seen.insert(view.to_string()) {
            return;
        }
        let Some(paths) = self.p.views.get(view) else { return };
        let paths: Vec<(Vec<String>, String)> = paths.clone();
        for (conds, text) in &paths {
            if self.holds(conds, s, &[]) == Tri::F {
                continue;
            }
            // a closed modal or drawer, or a hidden element, offers nothing
            // inside it, as the page offers nothing there
            let mut shut: Option<usize> = None;
            for line in text.lines() {
                let indent = line.len() - line.trim_start().len();
                if let Some(at) = shut {
                    if indent > at {
                        continue;
                    }
                    shut = None;
                }
                let l = line.trim();
                if let Some((_, attrs, _)) = element_line(l) {
                    let mut closed = false;
                    for a in &attrs {
                        let (want, x) = match (a.strip_prefix("open = "), a.strip_prefix("hidden = ")) {
                            (Some(x), _) => (Tri::F, x),
                            (_, Some(x)) => (Tri::T, x),
                            _ => continue,
                        };
                        let t = match self.expr(x) {
                            Some(e) => self.truth(&e, s),
                            None => Tri::U,
                        };
                        closed |= t == want;
                    }
                    if closed {
                        shut = Some(indent);
                        continue;
                    }
                }
                if let Some(rest) = l.strip_prefix('@') {
                    let child = rest.split('[').next().unwrap_or("");
                    self.offers(child, s, seen, out);
                    continue;
                }
                let Some((name, attrs, label)) = element_line(l) else { continue };
                // a disabled element offers nothing while its fact holds
                let mut disabled = false;
                for a in &attrs {
                    if let Some(x) = a.strip_prefix("disabled = ") {
                        let t = match self.expr(x) {
                            Some(e) => self.truth(&e, s),
                            None => Tri::U,
                        };
                        disabled |= t == Tri::T;
                    }
                }
                if disabled {
                    continue;
                }
                let desc = if label.is_empty() { name.clone() } else { format!("{} {}", name, label) };
                // options a fact holds are its declared list, when literal
                let options = attrs.iter().find_map(|a| a.strip_prefix("options = ")).map(|o| match self.p.initial.get(o) {
                    Some(v) if is_literal(v) => v.trim().to_string(),
                    _ => o.to_string(),
                });
                for a in &attrs {
                    if let Some(fact) = a.strip_prefix("binds ") {
                        let opts = options.as_ref().filter(|o| o.starts_with('[') && is_literal(o)).map(|o| split_top(&o[1..o.len() - 1]));
                        out.insert(Control::Binds { desc: desc.clone(), fact: fact.to_string(), options: opts });
                    } else if let Some((event, handler)) = a.split_once(" → ") {
                        let (h, args) = match handler.find('[') {
                            Some(i) if handler.ends_with(']') => (handler[..i].to_string(), split_top(&handler[i + 1..handler.len() - 1])),
                            _ => (handler.to_string(), Vec::new()),
                        };
                        let what = if event.starts_with("action ") { format!("{} {}", name, event) } else { desc.clone() };
                        out.insert(Control::Fires { desc: what, handler: h, args });
                    }
                }
            }
        }
        seen.remove(view);
    }

    /// The states one control can lead to from a state.
    fn successors(&mut self, s: &State, c: &Control) -> Vec<State> {
        let mut out = Vec::new();
        match c {
            Control::Fires { handler, args, .. } => {
                let Some(paths) = self.p.handlers.get(handler) else { return out };
                for path in paths {
                    if self.holds(&path.conds, s, args) == Tri::F {
                        continue;
                    }
                    let mut next = s.clone();
                    for (f, v) in &path.writes {
                        if self.tracked.contains(f) {
                            let v = self.written(&substitute(v, args), s);
                            next.insert(f.clone(), v);
                        }
                    }
                    // a write in a loop may happen or not: both
                    let mut variants = vec![next];
                    for (f, v) in &path.may {
                        if !self.tracked.contains(f) || variants.len() >= 8 {
                            continue;
                        }
                        let v = self.written(&substitute(v, args), s);
                        let with: Vec<State> = variants
                            .iter()
                            .map(|st| {
                                let mut w = st.clone();
                                w.insert(f.clone(), v.clone());
                                w
                            })
                            .collect();
                        variants.extend(with);
                    }
                    out.extend(variants);
                }
            }
            Control::Binds { fact, options, .. } => {
                if !self.tracked.contains(fact) {
                    return out;
                }
                let values: Vec<String> = match options {
                    Some(o) => o.clone(),
                    None => {
                        let mut v = self.literals.get(fact).cloned().unwrap_or_default();
                        v.push("*".into());
                        v
                    }
                };
                for v in values {
                    let mut next = s.clone();
                    next.insert(fact.clone(), v);
                    out.push(next);
                }
            }
        }
        out
    }

    /// The value a write leaves: a literal, another tracked fact's value,
    /// or some other value.
    fn written(&mut self, v: &str, s: &State) -> String {
        if is_literal(v) {
            return v.trim().to_string();
        }
        if let Some(x) = s.get(v.trim()) {
            return x.clone();
        }
        "*".into()
    }
}

/// The literals a fact can hold and whether it can hold anything else,
/// over every write, every input binding it and its declared value.
fn literal_sets(p: &Program, frozen: &BTreeMap<String, Vec<Vec<String>>>, bound: &BTreeMap<String, Option<Vec<String>>>) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut add = |f: &str, v: &str| {
        if is_literal(v) {
            let list = out.entry(f.to_string()).or_default();
            if !list.iter().any(|x| same_literal(x, v)) {
                list.push(v.trim().to_string());
            }
        }
    };
    for (f, v) in p.initial {
        add(f, v);
    }
    for (h, paths) in p.handlers {
        let calls = frozen.get(h).cloned().unwrap_or_else(|| vec![Vec::new()]);
        for args in &calls {
            for path in paths {
                for (f, v) in path.writes.iter().chain(path.may.iter()) {
                    add(f, &substitute(v, args));
                }
            }
        }
    }
    for (f, opts) in bound {
        if let Some(o) = opts {
            for v in o {
                add(f, v);
            }
        }
    }
    for list in out.values_mut() {
        list.sort();
    }
    out
}

/// The page's reachable states, as the artifact prints them.
pub fn machine(p: &Program, page: &str, root: &str) -> String {
    // the views the page can show, and the controls anywhere in them
    let mut views: BTreeSet<String> = BTreeSet::new();
    let mut stack = vec![root.to_string()];
    while let Some(v) = stack.pop() {
        if !views.insert(v.clone()) {
            continue;
        }
        for (_, text) in p.views.get(&v).into_iter().flatten() {
            for l in text.lines() {
                if let Some(rest) = l.trim().strip_prefix('@') {
                    stack.push(rest.split('[').next().unwrap_or("").to_string());
                }
            }
        }
    }
    let mut m = Machine { p, tracked: BTreeSet::new(), literals: BTreeMap::new(), parsed: HashMap::new() };
    // every control of the page, to find the facts guards read and the
    // arguments controls freeze
    let top: State = BTreeMap::new();
    let mut all: BTreeSet<Control> = BTreeSet::new();
    for v in &views {
        let mut seen = BTreeSet::new();
        // with no fact tracked every guard is unknown: every control shows
        m.offers(v, &top, &mut seen, &mut all);
    }
    let mut guards: Vec<String> = Vec::new();
    for v in &views {
        for (conds, text) in p.views.get(v).into_iter().flatten() {
            guards.extend(conds.iter().cloned());
            for l in text.lines() {
                if let Some((_, attrs, _)) = element_line(l) {
                    guards.extend(attrs.iter().filter_map(|a| a.strip_prefix("disabled = ").or_else(|| a.strip_prefix("open = ")).or_else(|| a.strip_prefix("hidden = "))).map(|x| x.to_string()));
                }
            }
        }
    }
    let mut frozen: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
    let mut bound: BTreeMap<String, Option<Vec<String>>> = BTreeMap::new();
    let mut handlers: BTreeSet<String> = BTreeSet::new();
    for c in &all {
        match c {
            Control::Fires { handler, args, .. } => {
                handlers.insert(handler.clone());
                let list = frozen.entry(handler.clone()).or_default();
                if !list.contains(args) {
                    list.push(args.clone());
                }
            }
            Control::Binds { fact, options, .. } => {
                bound.insert(fact.clone(), options.clone());
            }
        }
    }
    for h in &handlers {
        for path in p.handlers.get(h).into_iter().flatten() {
            guards.extend(path.conds.iter().cloned());
        }
    }
    for g in &guards {
        for id in idents(&normalize(g)) {
            if p.initial.contains_key(&id) {
                m.tracked.insert(id);
            }
        }
    }
    m.literals = literal_sets(p, &frozen, &bound);
    let initial: State = m
        .tracked
        .iter()
        .map(|f| {
            let v = p.initial.get(f).map(|v| v.trim().to_string()).unwrap_or_default();
            (f.clone(), if is_literal(&v) { v } else { "*".into() })
        })
        .collect();

    // the search: every state a control can lead to, breadth first
    let mut states: Vec<State> = vec![initial.clone()];
    let mut known: BTreeSet<State> = [initial.clone()].into_iter().collect();
    let mut offered: BTreeMap<State, BTreeSet<Control>> = BTreeMap::new();
    let mut i = 0;
    let mut stopped = false;
    while i < states.len() {
        let s = states[i].clone();
        i += 1;
        let mut here = BTreeSet::new();
        let mut seen = BTreeSet::new();
        m.offers(root, &s, &mut seen, &mut here);
        for c in &here {
            for next in m.successors(&s, c) {
                if known.contains(&next) {
                    continue;
                }
                if states.len() >= MAX_STATES {
                    stopped = true;
                    break;
                }
                known.insert(next.clone());
                states.push(next);
            }
        }
        offered.insert(s, here);
    }

    // the artifact: the facts and their values, then each state as it
    // differs from the first, with what it offers
    let mut text = format!("states {} ({})\n", page, root);
    if m.tracked.is_empty() {
        text.push_str("  no guard reads a fact: one state\n");
    } else {
        let facts: Vec<String> = m
            .tracked
            .iter()
            .map(|f| {
                let mut vals = m.literals.get(f).cloned().unwrap_or_default();
                let other = states.iter().any(|s| s.get(f).is_some_and(|v| v == "*"));
                if other {
                    vals.push("*".into());
                }
                format!("{} ∈ {{{}}}", f, vals.join(", "))
            })
            .collect();
        text.push_str(&format!("  facts: {}\n", facts.join(" · ")));
    }
    // one line per control: the states that offer it, told by the values
    // they all share, and the states those values also take in that do not
    // offer it, told the same way
    let varying: Vec<String> = m.tracked.iter().filter(|f| states.iter().any(|s| s.get(*f) != states[0].get(*f))).cloned().collect();
    let shared = |members: &[&State]| -> Vec<(String, String)> {
        varying
            .iter()
            .filter_map(|f| {
                let v = members.first()?.get(f)?;
                members.iter().all(|s| s.get(f) == Some(v)).then(|| (f.clone(), v.clone()))
            })
            .collect()
    };
    let matches = |s: &State, cond: &[(String, String)]| cond.iter().all(|(f, v)| s.get(f) == Some(v));
    let said = |cond: &[(String, String)]| cond.iter().map(|(f, v)| format!("{} = {}", f, v)).collect::<Vec<_>>().join(" · ");
    let first: Vec<String> = varying.iter().map(|f| format!("{} = {}", f, initial[f])).collect();
    if !first.is_empty() {
        text.push_str(&format!("  first: {}\n", first.join(" · ")));
    }
    let mut every: BTreeSet<Control> = BTreeSet::new();
    for cs in offered.values() {
        every.extend(cs.iter().cloned());
    }
    // a value the others already imply is dropped: it takes in no state
    // the rest leave out (`password = ''` beside `reader = 'someone'`, where
    // signing in clears the password)
    let minimal = |cond: Vec<(String, String)>, within: &dyn Fn(&State) -> bool| -> Vec<(String, String)> {
        let mut kept = cond.clone();
        for a in &cond {
            let without: Vec<(String, String)> = kept.iter().filter(|x| *x != a).cloned().collect();
            let same = states.iter().filter(|s| within(s) && matches(s, &without)).all(|s| matches(s, &kept));
            if same {
                kept = without;
            }
        }
        kept
    };
    for c in &every {
        let on: Vec<&State> = states.iter().filter(|s| offered.get(*s).is_some_and(|cs| cs.contains(c))).collect();
        let cond = minimal(shared(&on), &|_| true);
        let except: Vec<&State> = states.iter().filter(|s| matches(s, &cond) && !offered.get(*s).is_some_and(|cs| cs.contains(c))).collect();
        let mut when = if cond.is_empty() { "always".to_string() } else { format!("when {}", said(&cond)) };
        if !except.is_empty() {
            // what the exception adds to the condition it qualifies
            let ex: Vec<(String, String)> = shared(&except).into_iter().filter(|a| !cond.contains(a)).collect();
            let ex = minimal(ex, &|s: &State| matches(s, &cond));
            // the exception is exact when every state it takes in lacks it
            let exact = !ex.is_empty() && states.iter().filter(|s| matches(s, &cond) && matches(s, &ex)).all(|s| !offered.get(s).is_some_and(|cs| cs.contains(c)));
            if exact {
                when.push_str(&format!(", except when {}", said(&ex)));
            } else {
                when.push_str(", in some states");
            }
        }
        text.push_str(&format!("  {}: {}\n", c.text(), when));
    }
    if every.is_empty() {
        text.push_str("  offers nothing\n");
    }
    if stopped {
        text.push_str(&format!("  … the search stopped at {} states\n", MAX_STATES));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program() -> (BTreeMap<String, String>, BTreeMap<String, Vec<HandlerPath>>, BTreeMap<String, Vec<(Vec<String>, String)>>) {
        let initial: BTreeMap<String, String> = [("session", "0"), ("playing", "false"), ("status", "''")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let mut handlers = BTreeMap::new();
        handlers.insert(
            "start".to_string(),
            vec![
                HandlerPath { conds: vec!["(not q1.ok)".into()], writes: vec![("status".into(), "'no session'".into())], may: vec![] },
                HandlerPath { conds: vec!["q1.ok".into()], writes: vec![("session".into(), "q1.id[0]".into())], may: vec![] },
            ],
        );
        handlers.insert("play".to_string(), vec![HandlerPath { conds: vec!["(session gt 0)".into()], writes: vec![("playing".into(), "true".into())], may: vec![] }]);
        handlers.insert("finish".to_string(), vec![HandlerPath { conds: vec![], writes: vec![("session".into(), "0".into()), ("playing".into(), "false".into())], may: vec![] }]);
        let mut views = BTreeMap::new();
        views.insert(
            "page_view".to_string(),
            vec![
                (vec!["(session gt 0)".to_string()], "stack[]\n  button[click → play] 'Play'\n  button[click → finish] 'End'".to_string()),
                (vec!["(not (session gt 0))".to_string()], "button[click → start] 'Start'".to_string()),
            ],
        );
        (initial, handlers, views)
    }

    #[test]
    fn a_session_guard_makes_states_and_what_each_offers() {
        let (initial, handlers, views) = program();
        let p = Program { initial: &initial, handlers: &handlers, views: &views };
        let t = machine(&p, "/", "page_view");
        assert!(t.contains("facts: session ∈ {0, *}"), "{}", t);
        // no session, nothing but Start
        assert!(t.contains("  first: session = 0\n"), "{}", t);
        assert!(t.contains("  button 'Start' → start: when session = 0\n"), "{}", t);
        assert!(t.contains("  button 'End' → finish: when session = *\n  button 'Play' → play: when session = *\n"), "{}", t);
    }

    #[test]
    fn a_control_outside_its_guard_is_a_state_that_offers_it() {
        let (initial, handlers, mut views) = program();
        views.insert(
            "page_view".to_string(),
            vec![
                (vec!["(session gt 0)".to_string()], "stack[]\n  button[click → play] 'Play'\n  button[click → finish] 'End'".to_string()),
                (vec!["(not (session gt 0))".to_string()], "stack[]\n  button[click → start] 'Start'\n  button[click → play] 'Play'".to_string()),
            ],
        );
        let p = Program { initial: &initial, handlers: &handlers, views: &views };
        let t = machine(&p, "/", "page_view");
        // Play with no session: its line says always
        assert!(t.contains("  button 'Play' → play: always\n"), "{}", t);
    }

    #[test]
    fn the_reader_signed_in_is_a_value_sign_in_and_sign_out_write() {
        let initial: BTreeMap<String, String> = [("reader", "'nobody'")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let mut handlers = BTreeMap::new();
        handlers.insert("log_in".to_string(), vec![HandlerPath { conds: vec!["q1.ok".into()], writes: vec![("reader".into(), "'someone'".into())], may: vec![] }]);
        handlers.insert("log_out".to_string(), vec![HandlerPath { conds: vec![], writes: vec![("reader".into(), "'nobody'".into())], may: vec![] }]);
        let mut views = BTreeMap::new();
        views.insert(
            "door".to_string(),
            vec![
                (vec!["has[__reader[], 'name']".to_string()], "button[click → log_out] 'Sign out'".to_string()),
                (vec!["(not has[__reader[], 'name'])".to_string()], "button[click → log_in] 'Sign in'".to_string()),
            ],
        );
        let p = Program { initial: &initial, handlers: &handlers, views: &views };
        let t = machine(&p, "/", "door");
        assert!(t.contains("  button 'Sign in' → log_in: when reader = 'nobody'\n"), "{}", t);
        assert!(t.contains("  button 'Sign out' → log_out: when reader = 'someone'\n"), "{}", t);
    }

    #[test]
    fn a_frozen_argument_and_a_bound_input_set_their_facts() {
        let initial: BTreeMap<String, String> = [("speed", "1.0"), ("mode", "'scanner'")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let mut handlers = BTreeMap::new();
        handlers.insert("set_speed".to_string(), vec![HandlerPath { conds: vec![], writes: vec![("speed".into(), "$1".into())], may: vec![] }]);
        let mut views = BTreeMap::new();
        views.insert(
            "v".to_string(),
            vec![
                (vec!["(mode eq 'practice')".to_string()], "stack[]\n  tabs[binds mode, options = ['scanner', 'practice']]\n  p[] 'practising'".to_string()),
                (vec!["(not (mode eq 'practice'))".to_string()], "stack[]\n  tabs[binds mode, options = ['scanner', 'practice']]\n  button[click → set_speed[10.0], disabled = (speed eq 10.0)] '10x'".to_string()),
            ],
        );
        let p = Program { initial: &initial, handlers: &handlers, views: &views };
        let t = machine(&p, "/", "v");
        assert!(t.contains("mode ∈ {'practice', 'scanner'}"), "{}", t);
        assert!(t.contains("speed ∈ {1.0, 10.0}"), "{}", t);
        // at 10x the button is disabled, and practice has none: every
        // state but the first offers only the tabs
        assert!(t.contains("  button '10x' → set_speed[10.0]: when mode = 'scanner' · speed = 1.0\n"), "{}", t);
        assert!(t.contains("  tabs binds mode: always\n"), "{}", t);
    }
}
