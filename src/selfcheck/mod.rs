//! `crow check --self`: Crow held to itself, mechanically and the same way
//! every run. Programs are generated from the compiler's own description of
//! the language — every builtin row with the kinds its parameters take,
//! every collection with every index, every operator with every operand,
//! and the shapes the compiler and the interpreter have broken on (values
//! handed to functions, held, copied, looped over, written into, ints past
//! 64 bits) — and every case is run under each configuration that must
//! agree:
//!
//! - `interpreted`: `crow run -i`, the prepared form and parallel loops;
//! - `walked`: the interpreter walking every body in order (`CROW_WALK`);
//! - `unfused`: the lowering without its fused kernels (`CROW_NO_FUSE`);
//! - `unmoved`: the lowering without its moves and frees (`CROW_NO_MOVES`);
//! - `compiled`: `crow build`, then the binary.
//!
//! A case is lowered first, in this process: refused (its message held to
//! the rules every message keeps: it names only what the program wrote,
//! and its line), or a bug in Crow (a panic, an internal error, a
//! verifier finding). The cases that lower run in batches — a program of
//! one function per case, a marker before each, the first chosen by
//! `CROW_FROM` — so one build serves a hundred cases, and a case that
//! stops the program ends only its own run. Every disagreement between the
//! configurations, crash, hang, build failure and internal error is a
//! finding. The findings already known are listed in `tests/self/known.txt`,
//! which may only shrink: a new finding fails the check, and so does a
//! known one that no longer happens (strike it, or `--bless`).
//!
//! Agreeing is not being right, so the cases that agree are held to the
//! right answer too: a value handed back — through a function, a record, a
//! list, a loop — shows as it shows alone, and `tests/self/reference.py`
//! answers what a builtin, an operator, an index or `fmt` computes from
//! the spec's words, independently of Crow's runtime (a case it answers
//! otherwise is a finding, `wrong`). A row's parameters are varied one at a
//! time and two at a time over their kinds' edges (`PAIRED`), where a
//! row's assumptions meet. The rows that speak to a server speak to the
//! check's own, on the loopback: an echo server, a ClickHouse stand-in and
//! a scratch Postgres cluster (`Servers`).
//!
//! And what is added later is tested, or the gate names it: three lists
//! beside `known.txt`, changed only by `--bless` — the builtins no case
//! calls (`not-called.txt`), those no case of which the reference answers
//! (`agreement-only.txt`), and the forms of the language, read from
//! `src/ast.rs`, no case reaches (`forms.txt`).

mod llm;
mod web;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::table::{Kind, ROWS};

// ── values ──────────────────────────────────────────────────────────

/// A value the cases are made of: its Crow expression, the statements that
/// bind what it reads, and the modules it needs.
#[derive(Clone, Debug)]
struct Val {
    name: String,
    expr: String,
    setup: Vec<String>,
    imports: Vec<&'static str>,
}

const FRAME_SETUP: &str = "sf = frame.new[a = [1, 2, 3], b = ['x', 'y', 'z'], on = [true, false, true], f = [1.5, 2.5, 3.5]]";
/// The port of the check's own echo server, as a case's process is told it
/// (`servers`).
const PORT_SETUP: &str = "self_port = env['CROW_SELF_PORT', '9']";
const JOIN_SETUP: &str = "sj = join[frame.new[k = [1, 2, 3]], frame.new[k = [1, 3], n = [5, 7]], 'k', 'left']";

fn plain(name: &str, expr: &str) -> Val {
    Val { name: name.to_string(), expr: expr.to_string(), setup: Vec::new(), imports: Vec::new() }
}

fn framed(name: &str, expr: &str) -> Val {
    Val { name: name.to_string(), expr: expr.to_string(), setup: vec![FRAME_SETUP.to_string()], imports: vec!["frame"] }
}

/// The values, by name: every kind a program holds, its edges (zero,
/// negative, empty, past 64 bits, text that reads as a number), and values
/// that can fail — one that did not, one that did.
fn values() -> Vec<Val> {
    let mut v = vec![
        plain("int", "3"),
        plain("int0", "0"),
        plain("intneg", "-2"),
        plain("int99", "99"),
        plain("intmax", "9223372036854775807"),
        plain("big", "2 ^ 100"),
        plain("float", "2.5"),
        plain("float0", "0.0"),
        plain("floatneg", "-1.5"),
        plain("text", "'abc'"),
        plain("text0", "''"),
        plain("textnum", "'42'"),
        plain("textkey", "'a'"),
        plain("textuni", "'héllo wörld'"),
        plain("bool", "true"),
        plain("boolf", "false"),
        plain("ints", "push[push[push[[], 3], 1], 2]"),
        plain("texts", "['b', 'a', 'c']"),
        plain("floats", "push[push[push[[], 2.5], 0.5], 1.5]"),
        plain("bools", "[true, false, true]"),
        plain("empty", "[]"),
        plain("vector", "[3.0, 1.0, 2.0]"),
        plain("vectorint", "[3, 1, 2]"),
        plain("matrix", "[[1.0, 2.0], [3.0, 4.0]]"),
        plain("map", "map[a = 1, b = 2]"),
        plain("maptext", "map[a = 'x', b = 'y']"),
        plain("mapint", "map[push[push[[], 1], 2], push[push[[], 10], 20]]"),
        plain("json", "json['{\"a\": 1, \"b\": [1, 2], \"c\": \"x\"}']"),
        plain("jsonarr", "json['[1, 2, 3]']"),
        plain("moment", "moment['2026-01-02T03:04:05Z']"),
        plain("decimal", "decimal['1.25']"),
        plain("fraction", "fraction[1, 3]"),
        Val { name: "bytes".into(), expr: "bytes['abc']".into(), setup: Vec::new(), imports: vec!["web"] },
        // (the standard library's own layer — `use sys`: bytes, a list of
        // them, a connection to the check's own echo server and its
        // address; `bytes` under `use web` is a response)
        Val { name: "sysbytes".into(), expr: "bytes['abc']".into(), setup: Vec::new(), imports: vec!["sys"] },
        Val { name: "chunks".into(), expr: "push[push[[], bytes['ab']], bytes['c']]".into(), setup: Vec::new(), imports: vec!["sys"] },
        Val { name: "stream".into(), expr: "open['127.0.0.1', int[self_port], false, 5.0]".into(), setup: vec![PORT_SETUP.to_string()], imports: vec!["sys"] },
        Val { name: "localurl".into(), expr: "'http://127.0.0.1:`self_port`/self'".into(), setup: vec![PORT_SETUP.to_string()], imports: Vec::new() },
        Val { name: "sparse".into(), expr: "sparse[[[1.0, 0.0], [0.0, 2.0]]]".into(), setup: Vec::new(), imports: vec!["math"] },
        plain("colnames", "['a', 'b']"),
        // (numbers for `fmt`: thousands with a fraction, a fraction of
        // eight places, a whole number of seven digits)
        plain("thousands", "1234567.891"),
        plain("fraction8", "0.12345678"),
        plain("whole7", "1234567"),
        plain("scheme", "'base64'"),
        // (a hash's algorithm and a pattern, for the rows that take them)
        plain("algo", "'sha256'"),
        plain("pattern", "'[a-c]+'"),
        // (the files every run's directory is given, one the case writes —
        // its own, by the case's number — an address nothing answers, and a
        // database in memory: see `sandbox`)
        plain("path", "'self_text.txt'"),
        plain("csvpath", "'self_data.csv'"),
        plain("jsonpath", "'self_data.json'"),
        plain("dirpath", "'self_dir'"),
        plain("newpath", "'self_out_@N.txt'"),
        plain("url", "'http://127.0.0.1:9/self'"),
        plain("sql", "'SELECT 1 AS n'"),
        Val { name: "conn".into(), expr: "connect['sqlite://:memory:']".into(), setup: Vec::new(), imports: vec!["db"] },
        Val { name: "pred".into(), expr: "self_true".into(), setup: Vec::new(), imports: vec![] },
        // (a socket and a mailer to an address nothing answers; a model the
        // empty replay answers in place of the network)
        Val { name: "socket".into(), expr: "socket['ws://127.0.0.1:9/self']".into(), setup: Vec::new(), imports: vec!["web.ws"] },
        Val { name: "mailer".into(), expr: "smtp['127.0.0.1', 9, 'u', 'p']".into(), setup: Vec::new(), imports: vec!["web.mail"] },
        Val { name: "model".into(), expr: "reach['anthropic://claude-sonnet-5-5']".into(), setup: Vec::new(), imports: vec!["llm"] },
        Val { name: "talk".into(), expr: "ask['hello']".into(), setup: Vec::new(), imports: vec!["llm"] },
        plain("record", "SelfP[1, 2]"),
        plain("fn", "self_double"),
        framed("frame", "sf"),
        framed("colint", "sf.a"),
        framed("coltext", "sf.b"),
        framed("colbool", "sf.on"),
        framed("colfloat", "sf.f"),
        framed("maskcmp", "sf.a gt 1"),
        Val { name: "colmissing".into(), expr: "sj.n".into(), setup: vec![JOIN_SETUP.to_string()], imports: vec!["frame"] },
        // (several arguments at once, for a row that takes the rest of them)
        plain("sizes", "2, 3"),
        plain("groups", "[1.0, 2.0], [3.0, 4.0]"),
        plain("lists", "['a', 'b']"),
        plain("tensors", "push[push[[], [1.0, 2.0]], [3.0, 4.0]]"),
    ];
    // a value that can fail: one that did not, one that did
    for base in ["int", "float", "text", "ints", "texts", "vector", "matrix", "map", "frame", "colint"] {
        if let Some(b) = v.iter().find(|x| x.name == base).cloned() {
            v.push(Val { name: format!("may:{}", base), expr: format!("self_may[{}, false]", b.expr), ..b.clone() });
            v.push(Val { name: format!("fail:{}", base), expr: format!("self_may[{}, true]", b.expr), ..b });
        }
    }
    v
}

fn val<'a>(all: &'a [Val], name: &str) -> &'a Val {
    all.iter().find(|v| v.name == name).unwrap_or_else(|| panic!("selfcheck: no value '{}'", name))
}

/// The values a parameter of a kind is tried with first, to find one call
/// of a row that lowers. Every kind is named: a kind added to the table
/// must say what the check tries it with.
fn tried_first(k: Kind) -> &'static [&'static str] {
    match k {
        Kind::Key => &["textkey", "int"],
        Kind::Keys => &["texts", "ints"],
        Kind::Int => &["int", "int0", "intneg"],
        Kind::Whole => &["int", "int0"],
        Kind::Float => &["float", "int", "float0"],
        Kind::Moment => &["moment"],
        Kind::Bool => &["bool", "boolf"],
        Kind::Str => &["text", "path", "textkey", "textnum", "text0"],
        Kind::Tensor => &["vector", "matrix", "floats"],
        Kind::Sizes => &["sizes", "int"],
        Kind::Tensors => &["tensors"],
        Kind::Groups => &["groups"],
        Kind::Frame => &["frame"],
        Kind::Strs => &["texts", "colnames"],
        Kind::Bools => &["bools", "maskcmp", "colbool"],
        Kind::Cell => &["int", "float", "text", "bool"],
        Kind::Cells => &["ints", "texts", "vector", "colint"],
        Kind::Lists => &["lists"],
        Kind::Words => &["texts", "ints"],
        Kind::List | Kind::Seq => &["ints", "texts", "floats"],
        Kind::Item => &["int", "text", "float"],
        Kind::ItemRef => &["int", "text"],
        Kind::MapOf | Kind::MapRef => &["map", "maptext"],
        Kind::TextMap => &["maptext"],
        Kind::Ints | Kind::Bigs => &["ints"],
        Kind::Big => &["big", "int"],
        Kind::Decimal => &["decimal"],
        Kind::Fraction => &["fraction"],
        Kind::Floats => &["floats", "vector"],
        Kind::Texts => &["texts"],
        Kind::Json => &["json", "jsonarr"],
        Kind::JsonList => &["jsonarr"],
        Kind::Column => &["colint", "colfloat", "coltext", "colbool"],
        Kind::Bytes => &["sysbytes", "bytes"],
        Kind::Chunks => &["chunks"],
        Kind::Stream => &["stream"],
        Kind::Shown | Kind::ShownDyn | Kind::Shows => &["int", "text", "ints"],
        Kind::Encoded => &["map", "json"],
        Kind::Sparse => &["sparse"],
        Kind::Socket => &["socket"],
        Kind::Mailer => &["mailer"],
        // a function handed to a row: the check's own predicates; a
        // walked collection: the lists, a tensor, a column; a connection
        Kind::FnOf(_) => &["pred"],
        Kind::Walked => &["ints", "texts", "vector", "colint"],
        Kind::Db | Kind::Tx => &["conn"],
        Kind::Request | Kind::Found => &[],
        // given, never taken from a program; or a server's answer, a fused
        // kernel's own: no program value is one
        Kind::FloatMap
        | Kind::Unit
        | Kind::Answer
        | Kind::Response
        | Kind::Markup
        | Kind::TextRows
        | Kind::Shell
        | Kind::Reply
        | Kind::Operands
        | Kind::Operand
        | Kind::Scalars
        | Kind::Scalar => &[],
    }
}

/// The values every parameter of a row is varied over, once a call of it
/// lowers: each kind's usual value and its edges.
const VARIED: &[&str] = &[
    "int", "int0", "intneg", "intmax", "big", "float", "float0", "text", "text0", "textnum", "bool", "ints", "texts", "floats", "bools", "empty", "vector", "matrix", "map",
    "json", "moment", "frame", "colint", "coltext", "colbool", "colmissing", "may:int", "fail:int", "may:ints", "fail:vector", "path", "csvpath", "newpath",
    "dirpath", "url", "sql", "sysbytes", "localurl",
];

/// The values two parameters of a row are varied over at once: where the
/// kinds meet their edges — nothing, a negative, one past the end, past 64
/// bits, a float where an int is, empty text, an empty list, a vector, a
/// matrix, a value that could have failed. A combination one parameter at
/// a time never reaches is where a row's assumption breaks: `slice[m, -1,
/// 3]` of a matrix had counted its numbers, where the matrix and the
/// negative start had each been tried alone.
const PAIRED: &[&str] = &["int0", "intneg", "int99", "big", "float", "text0", "empty", "vector", "matrix", "may:int"];

// ── cases ───────────────────────────────────────────────────────────

/// A case: a body of statements run as one function of its own.
#[derive(Clone, Debug)]
pub(crate) struct Case {
    pub id: String,
    imports: BTreeSet<&'static str>,
    body: Vec<String>,
    /// The right answer, where the case knows it: its first logs show, one
    /// by one, what these values show alone (`shape/shown/<name>`) — a
    /// value handed to a function and back, held in a record or a list and
    /// read out, is itself.
    expect: Vec<String>,
    /// What the case computes, where tests/self/reference.py answers it
    /// from the spec without Crow's runtime: a builtin by the name it is
    /// called by (`len`, `math.sum`), an operator (`op:+`), a form of the
    /// language (`form:index`, `form:write`, `form:show`, …) — and the
    /// catalog's values it is given, by name.
    refer: Option<(String, Vec<String>)>,
    /// Whether the case talks to a server of the check's own (the scratch
    /// Postgres cluster, the ClickHouse stand-in): its run has no replay,
    /// which would answer `connect` in the server's place.
    live: bool,
    /// Lines the case puts at the top level before its function — its own
    /// functions (`@N` its number): the grad family's.
    top: Vec<String>,
    /// Whether the case runs with the GPU on (`gpu[]` before any case, in
    /// a batch of its own): the gpu family's.
    gpu: bool,
    /// Whether the case works in a directory of its own (`c<N>/`, holding
    /// the run's files): a case that writes cannot change what another
    /// reads (the io family).
    own_dir: bool,
    /// A served case: its body is a route's handler (`fn self_case_N[req]`,
    /// its own `return` the response), served at `/cN` with what the route
    /// says, and asked the route's requests (the web families).
    serve: Option<Route>,
}

/// A served case's route: its method where it names one, the path past
/// `/cN` (a param: `/:name`), a limit, and the requests the check sends it
/// — or a page's: the case's body a view, drawn at `/cN`, its answer the
/// markup the view drew (the page's root).
#[derive(Clone, Debug, Default)]
pub(crate) struct Route {
    method: Option<&'static str>,
    tail: String,
    limit: Option<u32>,
    asks: Vec<Ask>,
    page: bool,
    /// A page's own top-level lines — its facts, handlers, derivations —
    /// their names the case's (`@N` its number), and the events its page
    /// is driven with (the state family); none, the page is asked once.
    top: Vec<String>,
    steps: Vec<serde_json::Value>,
}

/// One request a served case is asked: its method, the path past `/cN`,
/// its headers and its body.
#[derive(Clone, Debug)]
pub(crate) struct Ask {
    method: &'static str,
    tail: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Ask {
    fn get(tail: impl Into<String>) -> Ask {
        Ask { method: "GET", tail: tail.into(), headers: Vec::new(), body: Vec::new() }
    }
}

impl Case {
    fn new(id: impl Into<String>, vals: &[&Val], lines: Vec<String>) -> Case {
        let mut imports = BTreeSet::new();
        let mut body: Vec<String> = Vec::new();
        for v in vals {
            imports.extend(v.imports.iter().copied());
            for s in &v.setup {
                if !body.contains(s) {
                    body.push(s.clone());
                }
            }
        }
        body.extend(lines);
        Case { id: id.into(), imports, body, expect: Vec::new(), refer: None, live: false, serve: None, own_dir: false, top: Vec::new(), gpu: false }
    }

    /// The case is a route's handler, asked the route's requests.
    fn served(mut self, route: Route) -> Case {
        self.imports.insert("web");
        if route.page {
            self.imports.insert("ui");
        }
        self.serve = Some(route);
        self
    }

    /// The case talks to a server of the check's own.
    fn live(mut self) -> Case {
        self.live = true;
        self
    }

    /// What the reference answers the case by.
    fn referring(mut self, what: impl Into<String>, names: &[&str]) -> Case {
        self.refer = Some((what.into(), names.iter().map(|n| n.to_string()).collect()));
        self
    }

    /// The case's first logs show what these values show alone.
    fn expecting(mut self, names: &[&str]) -> Case {
        self.expect = names.iter().map(|n| n.to_string()).collect();
        self
    }

    /// Whether the case draws random numbers (it runs in a process of its
    /// own, the seed fresh).
    fn draws(&self) -> bool {
        let text = self.body.join("\n");
        crate::table::with_effect(crate::effects::Effect::Random).into_iter().any(|name| names_word(&text, name) && text.contains(&format!("{}[", name)))
    }

    /// The case's lines, as a program writes them: what an id stands for.
    fn code(&self) -> String {
        self.body.join("\n")
    }

    fn with_import(mut self, module: &'static str) -> Case {
        self.imports.insert(module);
        self
    }
}

/// The helpers a case may call, each included where a case names it.
const HELPERS: &[(&str, &str)] = &[
    ("SelfP", "struct SelfP [x, y]\n"),
    ("SelfBox", "struct SelfBox [v]\n"),
    ("SelfNode", "struct SelfNode [v, next]\n"),
    ("self_may", "fn self_may[x, fail]\n    if fail\n        return error['no']\n    return x\n"),
    ("self_pass", "fn self_pass[x]\n    return x\n"),
    ("self_true", "fn self_true[x]\n    return true\n"),
    ("self_first", "fn self_first[x]\n    return x[0]\n"),
    ("self_apply", "fn self_apply[g, x]\n    return g[x]\n"),
    ("self_double", "fn self_double[x]\n    return x * 2\n"),
    ("self_fact", "fn self_fact[n]\n    if n lt 2\n        return 1\n    return n * self_fact[n - 1]\n"),
    ("self_count", "fn self_count[n]\n    if n eq 0\n        return 0\n    return 1 + self_count[n - 1]\n"),
    // (a value carried through a collecting loop, through two variables,
    // and through a branch: each hands it back)
    ("self_loop", "fn self_loop[x]\n    ys = for y in push[[], x]\n        y\n    return ys[0]\n"),
    ("self_held", "fn self_held[x]\n    t = x\n    u = t\n    return u\n"),
    ("self_branch", "fn self_branch[x, c]\n    if c\n        return x\n    return x\n"),
    // (one of the draws family's five items)
    ("self_in_five", "fn self_in_five[v]\n    return v in [5.0, 3.0, 9.0, 1.0, 7.0]\n"),
    // (a connection to the check's echo server that has been sent something,
    // and one that has asked it a request: what a read waits for is there)
    ("self_sent", "fn self_sent[]\n    s = open['127.0.0.1', int[env['CROW_SELF_PORT', '9']], false, 5.0]\n    put = send[s, bytes['abc']]\n    return s\n"),
    // (a stream's feed: the value's text, then a bar — its put named
    // `out`, since `put` is sys's and a parameter is named like no builtin)
    ("self_feed", "fn self_feed[out, x]\n    sent = out[str[x]]\n    more = out['|']\n    return\n"),
    ("self_asked", "fn self_asked[]\n    s = open['127.0.0.1', int[env['CROW_SELF_PORT', '9']], false, 5.0]\n    put = send[s, bytes['GET /self HTTP/1.1\\r\\nHost: self\\r\\n\\r\\n']]\n    return s\n"),
];

/// Every whole-word `name` in `text` written `to`.
fn renamed(text: &str, name: &str, to: &str) -> String {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(name) {
        let before = out.chars().last().or_else(|| rest[..i].chars().last());
        let before = if i > 0 { rest[..i].chars().last() } else { before };
        let after = rest[i + name.len()..].chars().next();
        out.push_str(&rest[..i]);
        if before.is_none_or(|c| !word(c)) && after.is_none_or(|c| !word(c)) {
            out.push_str(to);
        } else {
            out.push_str(name);
        }
        rest = &rest[i + name.len()..];
    }
    out.push_str(rest);
    out
}

fn names_word(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(i, _)| {
        let before = text[..i].chars().last();
        let after = text[i + name.len()..].chars().next();
        let word = |c: char| c.is_alphanumeric() || c == '_';
        before.is_none_or(|c| !word(c)) && after.is_none_or(|c| !word(c))
    })
}

/// A program of cases: their imports, the helpers they name, a function per
/// case, and the dispatch that runs them from `CROW_FROM` on, a marker
/// before each. Also the line each case's function starts on.
fn program(cases: &[&Case]) -> (String, Vec<usize>) {
    let mut imports: BTreeSet<&str> = BTreeSet::new();
    for c in cases {
        imports.extend(c.imports.iter().copied());
    }
    let mut out = String::new();
    if !imports.is_empty() {
        out.push_str(&format!("use {}\n", imports.iter().copied().collect::<Vec<_>>().join(", ")));
    }
    // (the gpu family's batch turns the device on before any case runs)
    if cases.first().is_some_and(|c| c.gpu) {
        out.push_str("gpu[]\n");
    }
    // each case with its own copy of every helper it names (`self_may_3`,
    // `SelfBox_3`): a helper one case calls at a kind no other case sees,
    // so the cases of a batch are what they are alone, and an error in the
    // generated program is placed in its case by the name's number (a
    // helper called at two kinds is a case of its own: the `twice` family)
    let mut starts = Vec::with_capacity(cases.len());
    for (k, c) in cases.iter().enumerate() {
        let all = c.body.join("\n");
        let used: Vec<&str> = HELPERS.iter().filter(|(n, _)| names_word(&all, n)).map(|(n, _)| *n).collect();
        let own = |text: &str| -> String {
            let mut t = text.replace("@N", &k.to_string());
            for name in &used {
                t = renamed(&t, name, &format!("{}_{}", name, k));
            }
            t
        };
        for (name, text) in HELPERS {
            if used.contains(name) {
                out.push_str(&own(text));
            }
        }
        starts.push(out.lines().count() + 1);
        // (a case's own functions, and a page's own facts, handlers and
        // derivations, at the top level: the case's, as a build error in
        // them is)
        for line in &c.top {
            out.push_str(&own(line));
            out.push('\n');
        }
        if let Some(r) = &c.serve {
            for line in &r.top {
                out.push_str(&own(line));
                out.push('\n');
            }
        }
        // (a served case is a route's handler, handed the request, and its
        // body answers the response; any other a function of no parameters
        // whose logs are the case's)
        match &c.serve {
            Some(r) if r.page => out.push_str(&format!("view self_case_{}[]\n", k)),
            Some(_) => out.push_str(&format!("fn self_case_{}[req]\n", k)),
            None => out.push_str(&format!("fn self_case_{}[]\n", k)),
        }
        for line in &c.body {
            for l in own(line).lines() {
                out.push_str("    ");
                out.push_str(l);
                out.push('\n');
            }
        }
        if c.serve.is_none() {
            out.push_str("    return 0\n");
        }
    }
    // a served batch: each case's route, the route that says whose server
    // answers (the token the run hands it: a port another server holds is
    // no batch's), then the server (its port the run's: CROW_PORT)
    if cases.first().is_some_and(|c| c.serve.is_some()) {
        out.push_str(&format!("fn self_ready[req]\n    return respond[200, env['{}', '']]\n", web::TOKEN));
        for (k, c) in cases.iter().enumerate() {
            let Some(r) = &c.serve else { continue };
            if r.page {
                out.push_str(&format!("page['/c{}', self_case_{}]\n", k, k));
                continue;
            }
            let method = r.method.map(|m| format!("'{}', ", m)).unwrap_or_default();
            let limit = r.limit.map(|n| format!(", limit = {}", n)).unwrap_or_default();
            out.push_str(&format!("route['/c{}{}', {}self_case_{}{}]\n", k, r.tail, method, k, limit));
        }
        out.push_str(&format!("route['{}', self_ready]\nserve[3000]\n", web::READY));
        return (out, starts);
    }
    // (the cases from CROW_FROM on, to CROW_TO where it is given)
    out.push_str("self_from = env['CROW_FROM', 'c0']\nself_to = env['CROW_TO', '']\nself_go = false\n");
    for k in 0..cases.len() {
        out.push_str(&format!("if self_from eq 'c{0}'\n    self_go = true\nif self_go\n    log['§§c{0}']\n    log[self_case_{0}[]]\nif self_to eq 'c{0}'\n    self_go = false\n", k));
    }
    (out, starts)
}

/// Every case of the language's forms over the values.
fn form_cases(all: &[Val]) -> Vec<Case> {
    let v = |n: &str| val(all, n);
    let mut cases = Vec::new();

    // a value shown, and the shapes it meets: shown, measured, spliced into
    // text, decided on, walked, collected, handed to a function and back,
    // held in a record, a list and a map, copied, read after it was lent,
    // compared with itself, written down as JSON and read back
    let shown: Vec<&str> = all.iter().map(|x| x.name.as_str()).filter(|n| !matches!(*n, "sizes" | "groups" | "lists" | "maskcmp")).collect();
    for n in &shown {
        let x = v(n);
        let e = &x.expr;
        let shapes: Vec<(&str, Vec<String>)> = vec![
            ("shown", vec![format!("log[{}]", e)]),
            ("str", vec![format!("log[str[{}]]", e)]),
            ("len", vec![format!("log[len[{}]]", e)]),
            ("type", vec![format!("log[type[{}]]", e)]),
            ("spliced", vec![format!("v = {}", e), "log['v=`v`']".to_string()]),
            ("if", vec![format!("if {}\n    log['yes']\nelse\n    log['no']", e)]),
            ("for", vec![format!("for x in {}\n    log[x]", e)]),
            ("collect", vec![format!("ys = for x in {}\n    x", e), "log[ys]".to_string()]),
            ("count", vec!["n = 0".to_string(), format!("for x in {}\n    n = n + 1", e), "log[n]".to_string()]),
            ("pushed", vec!["acc = []".to_string(), format!("for x in {}\n    acc = push[acc, x]", e), "log[acc]".to_string()]),
            ("while", vec![format!("v = {}", e), "i = 0".to_string(), "while i lt len[v]\n    log[v[i]]\n    i = i + 1".to_string()]),
            ("status", vec![format!("v = {}", e), "log[v.ok]".to_string(), "log[v.reason]".to_string()]),
            ("pass", vec![format!("log[self_pass[{}]]", e)]),
            ("first", vec![format!("log[self_first[{}]]", e)]),
            ("apply", vec![format!("log[self_apply[self_pass, {}]]", e)]),
            ("boxed", vec![format!("b = SelfBox[{}]", e), "log[b.v]".to_string(), "log[b]".to_string()]),
            ("listed", vec![format!("xs = push[push[[], {0}], {0}]", e), "log[xs]".to_string(), "log[len[xs]]".to_string()]),
            ("mapped", vec![format!("m = map[k = {}]", e), "log[m['k']]".to_string()]),
            ("copied", vec![format!("a = {}", e), "b = a".to_string(), "log[a]".to_string(), "log[b]".to_string()]),
            ("lent", vec![format!("a = {}", e), "log[self_pass[a]]".to_string(), "log[a]".to_string()]),
            ("reassigned", vec![format!("a = {}", e), "b = a".to_string(), format!("a = {}", e), "log[b]".to_string(), "log[a]".to_string()]),
            ("self-eq", vec![format!("log[{0} eq {0}]", e)]),
            ("json", vec![format!("log[json[encode[{}]]]", e)]),
            ("negated", vec![format!("log[not {}]", e)]),
            ("minus", vec![format!("log[-{}]", e)]),
            ("multi", vec![format!("a, b = {0}, {0}", e), "log[a]".to_string(), "log[b]".to_string()]),
            (
                "while-skip",
                vec![
                    format!("v = {}", e),
                    "i = 0".to_string(),
                    "n = 0".to_string(),
                    "while true\n    if i gte len[v]\n        break\n    i = i + 1\n    if i eq 1\n        continue\n    n = n + 1".to_string(),
                    "log[n]".to_string(),
                ],
            ),
        ];
        for (shape, lines) in shapes {
            // (what hands the value back shows it as it shows alone)
            // (a function handed as a value takes ints of 64 bits — spec,
            // Numbers That Keep Every Digit — so one past them is refused
            // through `apply`, by design)
            let shows: &[&str] = match shape {
                "apply" if *n == "big" => &[],
                "pass" | "apply" | "mapped" | "boxed" => &[n],
                "copied" | "lent" | "reassigned" | "multi" => &[n, n],
                _ => &[],
            };
            let mut case = Case::new(format!("shape/{}/{}", shape, n), &[x], lines).expecting(shows);
            // (what the reference answers: the value shown, measured, its
            // type, compared with itself, negated, put into text, walked)
            let refer: Option<(&str, Vec<&str>)> = match shape {
                "shown" => Some(("form:show", vec![n])),
                "str" | "len" | "type" => Some((shape, vec![n])),
                "spliced" => Some(("form:spliced", vec![n])),
                "for" => Some(("form:for", vec![n])),
                "collect" => Some(("form:collect", vec![n])),
                "count" => Some(("form:count", vec![n])),
                "self-eq" => Some(("op:eq", vec![n, n])),
                "negated" => Some(("op:not", vec![n])),
                "minus" => Some(("op:neg", vec![n])),
                _ => None,
            };
            if let Some((what, names)) = refer {
                case = case.referring(what, &names);
            }
            cases.push(case);
        }
    }

    // an index read: every collection by every index, written inline and
    // held in a variable
    let bases = [
        "ints", "texts", "floats", "bools", "empty", "vector", "vectorint", "matrix", "text", "text0", "map", "maptext", "mapint", "json", "jsonarr", "moment", "bytes", "record", "frame",
        "colint", "coltext", "colbool", "colfloat", "colmissing", "may:ints", "fail:ints", "may:vector", "may:map", "may:frame",
    ];
    let indexes = ["int", "int0", "intneg", "int99", "textkey", "float", "bool", "bools", "maskcmp", "colbool", "may:int", "fail:int", "texts", "intmax"];
    for b in bases {
        for i in indexes {
            let (bv, iv) = (v(b), v(i));
            cases.push(Case::new(format!("index/{}/{}", b, i), &[bv, iv], vec![format!("log[{}[{}]]", bv.expr, iv.expr)]).referring("form:index", &[b, i]));
            cases.push(Case::new(format!("index-held/{}/{}", b, i), &[bv, iv], vec![format!("b = {}", bv.expr), format!("log[b[{}]]", iv.expr)]).referring("form:index", &[b, i]));
        }
    }
    // a two-step read: a row of a matrix, an item of a list's item, a key's
    // item, a field's item
    for (b, path) in [
        ("matrix", "[1][0]"),
        ("matrix", "[-1][-1]"),
        ("json", "['b'][1]"),
        ("jsonarr", "[0]"),
        ("frame", ".a[2]"),
        ("frame", ".b[-1]"),
        ("colmissing", "[1]"),
        ("colmissing", "[0]"),
    ] {
        let bv = v(b);
        cases.push(Case::new(format!("path/{}/{}", b, path), &[bv], vec![format!("b = {}", bv.expr), format!("log[b{}]", path)]));
    }

    // an index written: every collection a program writes into, by every
    // index, every value
    let writable = ["ints", "texts", "floats", "vector", "vectorint", "matrix", "map", "maptext", "mapint", "json", "colint", "frame", "text"];
    let at = ["int0", "intneg", "int99", "textkey", "bools", "maskcmp", "float"];
    let put = ["int", "float", "text", "bool", "vector", "ints", "may:int", "fail:int"];
    for b in writable {
        for i in at {
            for x in put {
                let (bv, iv, xv) = (v(b), v(i), v(x));
                cases.push(
                    Case::new(format!("write/{}/{}/{}", b, i, x), &[bv, iv, xv], vec![format!("b = {}", bv.expr), format!("b[{}] = {}", iv.expr, xv.expr), "log[b]".to_string()])
                        .referring("form:write", &[b, i, x]),
                );
            }
        }
    }
    // writes through two steps and through a record's field
    for (b, line) in [
        ("matrix", "b[0][1] = 9.0"),
        ("matrix", "b[1] = [5.0, 6.0]"),
        ("matrix", "b[-1] = b[0]"),
        ("matrix", "b[0][0] = b[1][1] + 1.0"),
        ("json", "b['a'] = 5"),
        ("map", "b['c'] = b['a'] + 1"),
        ("ints", "b[0] = b[1] + b[2]"),
        ("record", "b.x = 10"),
        ("record", "b.x = b.y + 1"),
    ] {
        let bv = v(b);
        cases.push(Case::new(format!("write-path/{}/{}", b, line), &[bv], vec![format!("b = {}", bv.expr), line.to_string(), "log[b]".to_string()]));
    }
    for line in ["g.v[1] = [5.0, 6.0]", "g.v[0][1] = 7.0", "g.v = [[0.0, 0.0], [0.0, 0.0]]"] {
        cases.push(Case::new(format!("write-field/{}", line), &[], vec!["g = SelfBox[[[1.0, 2.0], [3.0, 4.0]]]".to_string(), line.to_string(), "log[g.v]".to_string()]));
    }

    // an operator over every pair of operands — every operator the AST has
    // (`operators`: one added there is in this family without a word here)
    let ops = operators();
    let operands = ["int", "float", "text", "bool", "ints", "vector", "matrix", "map", "moment", "decimal", "big", "colint", "colbool", "coltext", "may:int", "fail:int"];
    for op in ops {
        for a in operands {
            for b in operands {
                let (av, bv) = (v(a), v(b));
                cases.push(Case::new(format!("op/{}/{}/{}", op, a, b), &[av, bv], vec![format!("log[{} {} {}]", av.expr, op, bv.expr)]).referring(format!("op:{}", op), &[a, b]));
            }
        }
    }
    // membership
    for item in ["int", "text", "float", "bool", "may:int"] {
        for coll in ["ints", "texts", "floats", "vector", "text", "map", "mapint", "json", "jsonarr", "colint", "coltext", "colbool", "empty", "frame"] {
            let (iv, cv) = (v(item), v(coll));
            cases.push(Case::new(format!("in/{}/{}", item, coll), &[iv, cv], vec![format!("log[{} in {}]", iv.expr, cv.expr)]));
            cases.push(Case::new(format!("not-in/{}/{}", item, coll), &[iv, cv], vec![format!("log[{} not in {}]", iv.expr, cv.expr)]));
        }
    }

    // ints past 64 bits, where the compiled program holds an int in 64 bits
    // only where its range shows it fits
    let wide: &[(&str, &str)] = &[
        ("max-plus-1", "x = 9223372036854775807\nlog[x + 1]"),
        ("max-times-max", "x = 9223372036854775807\nlog[x * x]"),
        ("min-minus-2", "x = -9223372036854775807\nlog[x - 2]"),
        ("pow-64", "log[2 ^ 64]"),
        ("pow-neg", "log[2 ^ -1]"),
        ("div", "x = 9223372036854775807\nlog[x / 2]"),
        ("mod", "x = 9223372036854775807\nlog[x % 7]"),
        ("sum-loop", "t = 0\nfor i in range[100]\n    t = t + 922337203685477580\nlog[t]"),
        ("product-while", "f = 1\ni = 1\nwhile i lt 30\n    f = f * i\n    i = i + 1\nlog[f]"),
        ("fact-recursive", "log[self_fact[25]]"),
        ("count-recursive", "log[self_count[50]]"),
        ("parsed", "log[int['99999999999999999999'] + 1]"),
        ("list-items", "xs = push[push[[], 9223372036854775807], 1]\nlog[xs[0] + xs[1]]"),
        ("big-compare", "log[2 ^ 100 gt 9223372036854775807]"),
        ("big-to-float", "log[float[2 ^ 100]]"),
        ("literal-past-64-bits", "log[123456789012345678901234567890]"),
        ("literal-max", "log[9223372036854775807]"),
        ("literal-past-max", "log[9223372036854775808]"),
        ("big-str", "log[str[2 ^ 100]]"),
        ("countdown", "k = 10\nwhile k gt 0\n    if k eq 3\n        k = k - 2\n    else\n        k = k - 1\nlog[k]"),
        ("abs-min", "log[abs[-9223372036854775807 - 1]]"),
    ];
    for (name, body) in wide {
        cases.push(Case::new(format!("wide/{}", name), &[], vec![body.to_string()]));
    }

    // a helper called at two kinds in one program: a copy of it per
    // signature, each typed and lowered on its own (a batch had shown a
    // helper handed one kind for a parameter typed as another)
    // (a parsed value, an int past 64 bits, an empty list, a bool, a moment
    // and a decimal too: `apply` handed a parsed value and a growing int
    // had been an internal error no pair of the first twelve reached)
    let kinds = [
        "int", "float", "text", "ints", "texts", "vector", "matrix", "map", "frame", "colint", "record", "may:int", "json", "big", "empty", "bool", "moment", "decimal",
    ];
    let helpers = [("pass", "self_pass[@]"), ("first", "self_first[@]"), ("may", "self_may[@, false]"), ("apply", "self_apply[self_pass, @]"), ("boxed", "SelfBox[@].v")];
    for (h, call) in helpers {
        for (i, a) in kinds.iter().enumerate() {
            for b in kinds.iter().skip(i + 1) {
                let (av, bv) = (v(a), v(b));
                let case = Case::new(format!("twice/{}/{}+{}", h, a, b), &[av, bv], vec![format!("log[{}]", call.replace('@', &av.expr)), format!("log[{}]", call.replace('@', &bv.expr))]);
                // (by the spec, not the value as it is: a parameter handed a
                // record and an int holds a record or none, whose int is none
                // only as 0; one handed a float and an int holds a float, an
                // int past 64 bits its nearest; a function value takes ints of
                // 64 bits; and a record's field holds one kind)
                let pair = |x: &str, y: &str| (*a == x && *b == y) || (*a == y && *b == x);
                let linked = (*a == "record" || *b == "record") && [*a, *b].iter().any(|k| matches!(*k, "int" | "may:int" | "big"));
                let widened = pair("float", "big");
                let handed = h == "apply" && (*a == "big" || *b == "big");
                let by_design = h == "first" || h == "boxed" || linked || widened || handed;
                cases.push(if by_design { case } else { case.expecting(&[a, b]) });
            }
        }
    }

    // a value carried two steps — each a way a program hands a value on and
    // gets it back — shows as it shows alone: the combinations a single
    // step does not reach (a list holding an int past 64 bits read back, a
    // helper's answer handed to another helper; crow check --self had
    // found none of these at one step). Each pair alone, and again with the
    // first step's helper also called at another kind in the program, so
    // its copies per signature meet the value.
    let steps: &[(&str, &str)] = &[
        ("pass", "self_pass[@]"),
        ("may", "self_may[@, false]"),
        ("apply", "self_apply[self_pass, @]"),
        ("box", "SelfBox[@].v"),
        ("item", "push[[], @][0]"),
        ("entry", "set[map[], 'k', @]['k']"),
        ("loop", "self_loop[@]"),
        ("held", "self_held[@]"),
        ("branch", "self_branch[@, true]"),
    ];
    let carried = [
        "int", "int0", "intneg", "intmax", "big", "float", "floatneg", "text", "text0", "textuni", "bool", "ints", "texts", "floats", "bools", "empty", "vector", "vectorint",
        "matrix", "map", "maptext", "mapint", "json", "jsonarr", "moment", "decimal", "fraction", "record", "colint", "coltext", "colbool", "colmissing", "frame", "may:int",
        "fail:int", "may:ints", "fail:ints",
    ];
    for (a, first) in steps {
        for (b, second) in steps {
            for n in carried {
                let x = v(n);
                let e = second.replace('@', &first.replace('@', &x.expr));
                // (a function handed as a value takes ints of 64 bits, by
                // design: one past them is refused through `apply`)
                let by_design = n == "big" && (*a == "apply" || *b == "apply");
                let shows: &[&str] = if by_design { &[] } else { &[n] };
                cases.push(Case::new(format!("path/{}+{}/{}", a, b, n), &[x], vec![format!("log[{}]", e)]).expecting(shows));
                // (the first step's helper at another kind too)
                let other = if matches!(n, "text" | "text0" | "textuni" | "texts") { "int" } else { "text" };
                let o = v(other);
                let twin = first.replace('@', &o.expr);
                // (a record type is one: its field holds the kind the twin
                // gave it, and a value of another is refused or read as it)
                let one_record = *a == "box" || *b == "box";
                let shows: &[&str] = if by_design || one_record { &[other] } else { &[other, n] };
                cases.push(Case::new(format!("path-twice/{}+{}/{}", a, b, n), &[o, x], vec![format!("log[{}]", twin), format!("log[{}]", e)]).expecting(shows));
            }
        }
    }

    // a database: SQLite in memory, its statements and their failures, and a
    // server's connection, answered by the empty replay every run is given
    let databases: &[(&str, &str)] = &[
        ("sqlite-round-trip", "c = connect['sqlite://:memory:']\nmade = query[c, 'CREATE TABLE t (k TEXT, v REAL, n INTEGER)']\nput = query[c, 'INSERT INTO t VALUES (?, ?, ?)', ['a', 1.5, 3]]\nrows = query[c, 'SELECT k, v, n FROM t']\nlog[rows]\nlog[rows.v[0] + 1.0]"),
        ("sqlite-null", "c = connect['sqlite://:memory:']\nmade = query[c, 'CREATE TABLE t (k TEXT, v REAL)']\nput = query[c, 'INSERT INTO t VALUES (?, NULL)', ['a']]\nrows = query[c, 'SELECT k, v FROM t']\nlog[rows]\nlog[rows.v.ok]"),
        ("sqlite-bad-sql", "c = connect['sqlite://:memory:']\nbad = query[c, 'SELECT nope FROM missing']\nlog[bad.ok]\nlog[bad.reason]"),
        ("sqlite-params-list", "c = connect['sqlite://:memory:']\nmade = query[c, 'CREATE TABLE t (n INTEGER)']\nfor i in range[5]\n    put = query[c, 'INSERT INTO t VALUES (?)', [i]]\nlog[query[c, 'SELECT n FROM t WHERE n IN ?', [push[push[[], 1], 3]]]]"),
        ("sqlite-insert-frame", "c = connect['sqlite://:memory:']\nmade = query[c, 'CREATE TABLE t (a INTEGER, b TEXT)']\nf = frame.new[a = [1, 2], b = ['x', 'y']]\nput = insert[c, 't', f]\nlog[query[c, 'SELECT a, b FROM t']]"),
        ("sqlite-count", "c = connect['sqlite://:memory:']\nmade = query[c, 'CREATE TABLE t (a INTEGER)']\nput = query[c, 'INSERT INTO t VALUES (1), (2)']\nlog[put]\nlog[len[query[c, 'SELECT a FROM t']]]"),
        ("postgres-unreached", "c = connect['postgres://u@127.0.0.1:9/x']\nlog[c.ok]"),
        ("clickhouse-unreached", "c = connect['clickhouse://127.0.0.1:9/x']\nlog[c.ok]"),
    ];
    for (name, body) in databases {
        let mut case = Case::new(format!("db/{}", name), &[], vec![body.to_string()]).with_import("db");
        if body.contains("frame.new") {
            case = case.with_import("frame");
        }
        cases.push(case);
    }
    // the drivers written in Crow against servers of the check's own: a
    // scratch Postgres cluster (where initdb is on the PATH) and the
    // ClickHouse stand-in — a connection, a query of every kind and NULL,
    // parameters, a table made, written by insert and read back, a
    // statement described, a failure in the server's words
    let pg = "self_pg = env['CROW_SELF_PG', '9']\npg = connect['postgres://crow@127.0.0.1:`self_pg`/postgres']";
    let ch = "self_ch = env['CROW_SELF_CH', '9']\nch = connect['clickhouse://alice:secret@127.0.0.1:`self_ch`/stuff']";
    let live: &[(&str, &str, &str)] = &[
        ("postgres-kinds", pg, "log[query[pg, 'SELECT 1 AS n, 2.5::float8 AS x, \\'a\\' AS t, true AS b, NULL::int AS m']]"),
        ("postgres-params", pg, "rows = query[pg, 'SELECT ?::int + 1 AS n, ? AS t, ?::float8 * 2 AS x', [41, 'b', 1.25]]\nlog[rows]\nlog[rows.n[0]]"),
        ("postgres-write", pg, "made = query[pg, 'CREATE TEMP TABLE self_t (n int, name text)']\nput = insert[pg, 'self_t', frame.new[n = [1, 2], name = ['a', 'b']]]\nlog[put]\nlog[query[pg, 'UPDATE self_t SET n = n + 10']]\nlog[query[pg, 'SELECT n, name FROM self_t ORDER BY n']]"),
        ("postgres-describe", pg, "log[describe[pg, 'SELECT 1 AS n, ?::text AS t, 2.5::float8 AS x']]"),
        ("postgres-bad-sql", pg, "bad = query[pg, 'SELECT nope FROM nowhere']\nlog[bad.ok]\nlog[bad.reason]"),
        ("clickhouse-query", ch, "rows = query[ch, 'SELECT number AS n, number/7 AS x FROM numbers(3)']\nlog[rows]\nlog[int[rows.n[2]] * 2]\nlog[rows.x[1].ok]"),
        ("clickhouse-write", ch, "f = frame.new[n = [1, 2], name = ['a', 'b']]\nlog[insert[ch, 't', f]]\nlog[query[ch, 'CREATE TABLE u (n UInt64) ENGINE = Memory']]"),
        ("clickhouse-describe", ch, "log[describe[ch, 'SELECT n, x FROM t WHERE n = ?']]"),
        ("clickhouse-bad-sql", ch, "bad = query[ch, 'SELECT nope FROM nowhere']\nlog[bad.ok]\nlog[bad.reason]"),
    ];
    for (name, open, body) in live {
        let mut case = Case::new(format!("db/{}", name), &[], vec![open.to_string(), body.to_string()]).with_import("db").live();
        if body.contains("frame.new") {
            case = case.with_import("frame");
        }
        cases.push(case);
    }

    // `fmt` as programs call it, by name: decimals, commas, a width, an
    // alignment, a fill, a percent — over numbers of every sign and size and
    // over text (the row's own cases hand its seven slots catalog values,
    // which name no alignment)
    let numbers = ["int", "intneg", "float", "floatneg", "float0", "thousands", "fraction8", "whole7"];
    let number_options = [
        "decimals = 0",
        "decimals = 1",
        "decimals = 2",
        "decimals = 3",
        "commas = true",
        "commas = true, decimals = 2",
        "width = 10",
        "width = 10, align = 'left'",
        "width = 6, fill = '0'",
        "width = 3",
        "percent = true",
        "percent = true, decimals = 1",
    ];
    for n in numbers {
        for o in number_options {
            let x = v(n);
            cases.push(Case::new(format!("fmt/{}/{}", n, o), &[x], vec![format!("log[fmt[{}, {}]]", x.expr, o)]).referring(format!("fmt:{}", o), &[n]));
        }
    }
    for n in ["text", "textuni", "text0"] {
        for o in ["width = 8", "width = 8, align = 'right'", "width = 8, fill = '*'", "width = 2", "width = 8, align = 'right', fill = '.'"] {
            let x = v(n);
            cases.push(Case::new(format!("fmt/{}/{}", n, o), &[x], vec![format!("log[fmt[{}, {}]]", x.expr, o)]).referring(format!("fmt:{}", o), &[n]));
        }
    }

    // records that link records of their kind
    let linked: &[(&str, &str)] = &[
        ("read", "n = SelfNode[1, SelfNode[2, 0]]\nlog[n.next.v]"),
        ("write-through", "n = SelfNode[1, SelfNode[2, 0]]\nn.next.v = 5\nlog[n.next.v]"),
        ("none-test", "n = SelfNode[1, 0]\nlog[n.next eq 0]"),
        ("shared", "a = SelfNode[1, 0]\nb = SelfNode[2, a]\nc = SelfNode[3, a]\nb.next.v = 9\nlog[a.v]\nlog[c.next.v]"),
        ("walk", "n = SelfNode[1, SelfNode[2, SelfNode[3, 0]]]\nt = 0\nwhile n.next ne 0\n    t = t + n.v\n    n = n.next\nlog[t]"),
    ];
    for (name, body) in linked {
        cases.push(Case::new(format!("linked/{}", name), &[], vec![body.to_string()]));
    }
    cases
}

/// The rows a program cannot call, and why: the language's own layer (its
/// rows are reached through the standard modules written in Crow, which
/// the cases call), and the database drivers' (reached through `use db`'s
/// `connect`, `query` and `insert`, the `db` family).
fn skipped_module(module: &str) -> Option<&'static str> {
    match module {
        "db.postgres" | "db.clickhouse" => Some("a driver's own row, reached through use db (the db family)"),
        _ => None,
    }
}

/// Calls written out, for the rows whose arguments their kinds do not say:
/// a scheme's name, an option given by name.
const WRITTEN: &[(&str, &str)] = &[
    ("text.encode", "encode['abc', 'base64']"),
    ("text.decode", "decode['YWJj', 'base64']"),
    ("frame.window", "window[sf, 'a', 'sum', n = 2]"),
    ("frame.window_one", "window[sf, 'a', 'lag', n = 1]"),
    // (an address nothing answers; the empty replay answers a model)
    ("core.request", "request['http://127.0.0.1:9/self']"),
    ("llm.chat", "chat[reach['anthropic://claude-sonnet-5-5'], ask['hello'], system = 'One word.']"),
    ("llm.follow", "follow[reach['anthropic://claude-sonnet-5-5'], ask['hello'], system = 'One word.']"),
    // (a duration a case can wait out)
    ("core.sleep", "sleep[0.01]"),
    // (the layer's rows whose answer is the process's or the machine's —
    // asked so every configuration shows the same — and its digests by the
    // names they take)
    ("sys.process", "process[] gt 0"),
    // (a process started: a command that writes one line and ends, so every
    // configuration reads the same and nothing reaches standard error —
    // generated, `start` ran the kinds' values as commands, and what the
    // shell said of them on the program's standard error landed in the
    // other cases' outputs at no fixed moment)
    ("sys.start", "trim[text[recv[stream_of[start['echo started']], 64, 5.0], '?']]"),
    ("sys.alive", "alive[process[]]"),
    ("sys.machine", "len[machine[]] gt 0"),
    ("sys.random", "len[random[16]]"),
    ("sys.hash", "hash[bytes['abc'], 'sha256']"),
    ("sys.hmac", "hex[hmac[bytes['key'], bytes['abc'], 'sha256']]"),
    ("sys.digest", "hex[digest[bytes['abc'], 'sha1']]"),
    // (a read of what was sent, and of a request's answer: on a connection
    // with nothing to read, each waits out its time)
    ("sys.recv", "text[recv[self_sent[], 3, 5.0]]"),
    ("sys.read_head", "read_head[self_asked[], moment['2030-01-01T00:00:00Z']]"),
    ("core.sleep_ms", "sleep_ms[1]"),
];

/// The module a row is imported from in a program: its own path, where the
/// registry knows the module (`modules::module_by_path`) — so a module added
/// there has its rows' cases import it with no word here.
fn import_of(module: &'static str) -> Option<&'static str> {
    (!module.is_empty() && crate::modules::module_by_path(module).is_some()).then_some(module)
}

/// What the check writes cases of: a row of the table, or a function a
/// module written in Crow exports (crate::modules::exports), its
/// signature as the module's own tests type it — by one id (a row's its
/// own, an export's `math.stats.median`), its module, the name it is
/// called by, and the kinds its parameters take.
struct Sig {
    id: String,
    module: &'static str,
    name: String,
    params: Vec<Kind>,
}

/// Every row's signature, then every export's; and the exports no case
/// can be written for, with why.
fn sigs() -> (Vec<Sig>, Vec<NotCalled>) {
    let mut out: Vec<Sig> = ROWS.iter().map(|r| Sig { id: r.id.to_string(), module: r.module, name: r.name.to_string(), params: r.params.to_vec() }).collect();
    let (exported, unexercised) = export_sigs();
    out.extend(exported);
    (out, unexercised)
}

/// The functions the modules written in Crow export, each with the kinds
/// its parameters take as the module's own tests type them: the module
/// lowered as `crow test src/modules/<m>.co` lowers it (its tests typing
/// its functions), each function read off the program. A parameter the
/// tests leave untyped, or one no kind names (a function), makes no case:
/// listed as not called, with why.
fn export_sigs() -> (Vec<Sig>, Vec<NotCalled>) {
    let mut sigs = Vec::new();
    let mut unexercised = Vec::new();
    for m in crate::modules::MODULES.iter().filter(|m| m.source.is_some()) {
        let exports = crate::modules::exports(m.path);
        if exports.is_empty() {
            continue;
        }
        let source = m.source.unwrap_or_default();
        let file = format!("src/modules/{}.co", m.path.rsplit('.').next().unwrap_or(m.path));
        let lowered = std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(move || crate::program_of_source(source, &file, false))
            .ok()
            .and_then(|h| h.join().ok());
        let program = match lowered {
            Some(Ok(p)) => p,
            other => {
                let why = match other {
                    Some(Err(e)) => format!("its module does not lower as a program of its own: {}", e.lines().next().unwrap_or("")),
                    _ => "its module does not lower as a program of its own".to_string(),
                };
                for e in exports {
                    unexercised.push(NotCalled::new(format!("{}.{}", m.path, e), why.clone()));
                }
                continue;
            }
        };
        let fns: Vec<&crate::ir::Fn> = program.parts.iter().flat_map(|p| p.fns.iter()).collect();
        for e in exports {
            let id = format!("{}.{}", m.path, e);
            match export_params(&fns, e) {
                Ok(params) => sigs.push(Sig { id, module: m.path, name: e.to_string(), params }),
                Err(why) => unexercised.push(NotCalled::new(id, why)),
            }
        }
    }
    (sigs, unexercised)
}

/// The kinds an exported function's parameters take: the function's own
/// types where settled, else a copy's (a function called at several
/// signatures is lowered once per signature, `name__N`).
fn export_params(fns: &[&crate::ir::Fn], name: &str) -> Result<Vec<Kind>, String> {
    let is_copy = |f: &str| f.strip_prefix(name).and_then(|rest| rest.strip_prefix("__")).is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    let copies: Vec<&&crate::ir::Fn> = fns.iter().filter(|f| f.name == name || is_copy(&f.name)).collect();
    let base = copies.iter().find(|f| f.name == name).ok_or_else(|| "its module lowers no function of the name".to_string())?;
    let mut kinds = Vec::with_capacity(base.params.len());
    for (i, (p, ty)) in base.params.iter().enumerate() {
        let settled = std::iter::once(ty).chain(copies.iter().filter_map(|f| f.params.get(i).map(|(_, t)| t))).find(|t| !t.has_unknown());
        match settled.and_then(kind_of_ty) {
            Some(k) => kinds.push(k),
            None => return Err(format!("its parameter '{}' is typed by nothing the module's tests hand it ({}), or takes what no kind names", p, ty)),
        }
    }
    Ok(kinds)
}

/// The kind a parameter's type answers to, for the values the check tries
/// it with: none for a function, a record or a handle no value is.
fn kind_of_ty(ty: &crate::ir::Ty) -> Option<Kind> {
    use crate::ir::Ty;
    Some(match ty {
        Ty::Int => Kind::Int,
        Ty::Big => Kind::Big,
        Ty::Float => Kind::Float,
        Ty::Bool => Kind::Bool,
        Ty::Str => Kind::Str,
        Ty::Moment => Kind::Moment,
        Ty::Decimal => Kind::Decimal,
        Ty::Fraction => Kind::Fraction,
        Ty::Bytes => Kind::Bytes,
        Ty::Json => Kind::Json,
        Ty::Handle(h) if h.name == crate::table::SOCKET.name => Kind::Socket,
        Ty::Handle(h) if h.name == crate::table::MAILER.name => Kind::Mailer,
        Ty::Handle(h) if h.name == crate::table::STREAM.name => Kind::Stream,
        Ty::Frame(_) => Kind::Frame,
        Ty::Column(_) => Kind::Column,
        Ty::Sparse => Kind::Sparse,
        Ty::Tensor(..) => Kind::Tensor,
        Ty::List(item) => match item.as_ref() {
            Ty::Str => Kind::Strs,
            Ty::Float => Kind::Floats,
            Ty::Int => Kind::Ints,
            Ty::Bool => Kind::Bools,
            Ty::Tensor(..) => Kind::Tensors,
            _ => Kind::List,
        },
        Ty::Map(_, v) => match v.as_ref() {
            Ty::Str => Kind::TextMap,
            _ => Kind::MapOf,
        },
        Ty::Result(inner) => return kind_of_ty(inner),
        _ => return None,
    })
}

/// A call of a row or an exported function with the given arguments, as a
/// case.
fn sig_case(id: String, row: &Sig, args: &[&Val]) -> Case {
    let call = format!("{}[{}]", row.name, args.iter().map(|a| a.expr.as_str()).collect::<Vec<_>>().join(", "));
    // (the reference answers a row by the name it is called by)
    let what = if row.module.is_empty() { row.name.clone() } else { format!("{}.{}", row.module, row.name) };
    let names: Vec<&str> = args.iter().map(|a| a.name.as_str()).collect();
    let mut case = Case::new(id, args, vec![format!("log[{}]", call)]).referring(what, &names);
    if let Some(m) = import_of(row.module) {
        case = case.with_import(m);
        // (a math module's part needs math beside it)
        if m.starts_with("math.") {
            case = case.with_import("math");
        }
    }
    case
}

// ── lowering, in this process ───────────────────────────────────────

#[derive(Clone, Debug)]
enum Lowered {
    Accepted,
    Refused(String),
    Bug(String),
}

fn is_bug(message: &str) -> bool {
    message.contains("a bug in Crow") || message.contains("internal error") || message.contains("disagrees with itself") || message.contains("panicked")
}

/// A program lowered as `crow run` lowers it, on a thread of its own (a
/// panic is a finding, and a thread's state starts clean).
fn lower(source: String) -> Lowered {
    let run = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::program_of_source(&source, "target/crow_self/case.co", false)))
    });
    match run.map(|h| h.join()) {
        Ok(Ok(Ok(Ok(_)))) => Lowered::Accepted,
        Ok(Ok(Ok(Err(m)))) => {
            if is_bug(&m) {
                Lowered::Bug(m)
            } else {
                Lowered::Refused(m)
            }
        }
        Ok(Ok(Err(p))) | Ok(Err(p)) => {
            let what = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            Lowered::Bug(format!("the lowering panicked: {}", what))
        }
        Err(e) => Lowered::Bug(format!("no thread for the lowering: {}", e)),
    }
}

/// Every source lowered, across the cores, in order.
fn lower_all(sources: Vec<String>) -> Vec<Lowered> {
    let n = sources.len();
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Lowered>>> = Mutex::new(vec![None; n]);
    let sources = &sources;
    let workers = std::thread::available_parallelism().map(|c| c.get()).unwrap_or(4);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= n {
                        break;
                    }
                    let l = lower(sources[i].clone());
                    if let Ok(mut r) = results.lock() {
                        r[i] = Some(l);
                    }
                }
            });
        }
    });
    results.into_inner().unwrap_or_default().into_iter().map(|l| l.unwrap_or(Lowered::Bug("not lowered".into()))).collect()
}

/// The rows' cases: a call of each row found that lowers — its parameters
/// tried with their kinds' values — then each parameter varied over every
/// value. A row no call of which lowers is listed, with the first refusal.
fn row_cases(all: &[Val], only: &Option<String>) -> (Vec<Case>, Vec<NotCalled>) {
    let mut found: Vec<Case> = Vec::new();
    let (sigs, mut unexercised) = sigs();
    // the rows to try, each with its candidate calls
    // (each row's candidate calls, those another row's trials made already
    // last, and how many come first)
    let mut trials: Vec<(usize, Vec<Vec<&Val>>, usize)> = Vec::new();
    let mut seen_calls: HashSet<String> = HashSet::new();
    for (r, row) in sigs.iter().enumerate() {
        if row.name.starts_with("__") {
            continue;
        }
        if let Some(why) = skipped_module(row.module) {
            // (a driver's rows are reached through `use db` by the db
            // family's live cases — Postgres where a scratch cluster can be
            // made, ClickHouse against the check's stand-in — but its cancel,
            // which a stop abandoning a statement in flight calls)
            let reached = if row.module == "db.postgres" { crate::home::on_path("initdb") } else { true };
            if row.name.ends_with("_cancel") {
                unexercised.push(NotCalled::new(row.id.as_str(), format!("{}: a stop abandoning a statement in flight calls it, which tests/query_cancel.rs holds in both modes", why)));
            } else if !reached {
                unexercised.push(NotCalled { environment: true, ..NotCalled::new(row.id.as_str(), format!("{}: no initdb on the PATH for a scratch cluster", why)) });
            }
            continue;
        }
        // (every effect runs in a directory of its own, the network answered
        // by an empty replay, the clock and the draws fixed: `sandbox`; what
        // never returns is the served suites')
        if matches!(row.name.as_str(), "serve" | "listen") {
            unexercised.push(NotCalled::new(row.id.as_str(), "runs until it is stopped (crow test --served holds it)"));
            continue;
        }
        if import_of(row.module).is_none() && !row.module.is_empty() {
            unexercised.push(NotCalled::new(row.id.as_str(), format!("module '{}' is no module the registry knows", row.module)));
            continue;
        }
        if let Some(w) = only
            && !format!("row/{}", row.id).contains(w.as_str())
        {
            continue;
        }
        // (a name the parser rewrites: the row is reached through the form)
        if matches!(row.name.as_str(), "contains" | "has") {
            continue;
        }
        // (a call whose arguments no kind says: written out)
        if let Some((_, call)) = WRITTEN.iter().find(|(id, _)| *id == row.id.as_str()) {
            let mut case = Case::new(format!("row/{}/written", row.id), &[], vec![format!("log[{}]", call)]);
            if call.contains("sf") {
                case = Case::new(format!("row/{}/written", row.id), &[val(all, "frame")], vec![format!("log[{}]", call)]);
            }
            if let Some(m) = import_of(row.module) {
                case = case.with_import(m);
            }
            found.push(case);
            continue;
        }
        let pools: Vec<Vec<&Val>> = row.params.iter().map(|k| tried_first(*k).iter().map(|n| val(all, n)).collect()).collect();
        if pools.iter().any(|p| p.is_empty()) {
            let k = row.params.iter().find(|k| tried_first(**k).is_empty()).map(|k| format!("{:?}", k)).unwrap_or_default();
            unexercised.push(NotCalled::new(row.id.as_str(), format!("takes a {} no program value is", k)));
            continue;
        }
        // the first few combinations, the first value of each kind first
        let mut combos: Vec<Vec<&Val>> = vec![Vec::new()];
        for pool in &pools {
            let mut next = Vec::new();
            for c in &combos {
                for v in pool.iter().take(3) {
                    let mut c2 = c.clone();
                    c2.push(*v);
                    next.push(c2);
                }
            }
            next.truncate(27);
            combos = next;
        }
        // (one name answers several rows: a call tried once — a row whose
        // only call that lowers another row's trials made is reached by that
        // row's cases: `sort[vector]` is `sort_tensor`'s, made by
        // `sort_floats`'s trials)
        let (mut fresh, made): (Vec<Vec<&Val>>, Vec<Vec<&Val>>) = combos.into_iter().partition(|c| seen_calls.insert(format!("{}[{}]", row.name, c.iter().map(|v| v.name.as_str()).collect::<Vec<_>>().join(","))));
        let first = fresh.len();
        fresh.extend(made);
        if fresh.is_empty() {
            continue;
        }
        trials.push((r, fresh, first));
    }
    // one call that lowers, per row
    let mut sources = Vec::new();
    let mut owners = Vec::new();
    for (t, (r, combos, _)) in trials.iter().enumerate() {
        for (c, args) in combos.iter().enumerate() {
            let case = sig_case(String::new(), &sigs[*r], args);
            sources.push(program(&[&case]).0);
            owners.push((t, c));
        }
    }
    let lowered = lower_all(sources);
    let mut base: BTreeMap<usize, usize> = BTreeMap::new();
    let mut first_refusal: BTreeMap<usize, String> = BTreeMap::new();
    for ((t, c), l) in owners.iter().zip(lowered.iter()) {
        match l {
            Lowered::Accepted => {
                base.entry(*t).or_insert(*c);
            }
            Lowered::Refused(m) | Lowered::Bug(m) => {
                first_refusal.entry(*t).or_insert_with(|| m.lines().next().unwrap_or("").to_string());
            }
        }
    }
    let mut ids: HashSet<String> = HashSet::new();
    for (t, (r, combos, first)) in trials.iter().enumerate() {
        let row = &sigs[*r];
        let Some(c) = base.get(&t) else {
            unexercised.push(NotCalled::new(row.id.as_str(), format!("no call of it lowers: {}", first_refusal.get(&t).cloned().unwrap_or_default())));
            continue;
        };
        // (its call another row's cases make, with their variations)
        if *c >= *first {
            continue;
        }
        let args = &combos[*c];
        let id = format!("row/{}/base", row.id);
        if ids.insert(id.clone()) {
            found.push(sig_case(id, row, args));
        }
        for p in 0..args.len() {
            for name in VARIED {
                let x = val(all, name);
                if x.name == args[p].name {
                    continue;
                }
                let mut a = args.clone();
                a[p] = x;
                let id = format!("row/{}/{}={}", row.id, p, name);
                if ids.insert(id.clone()) {
                    found.push(sig_case(id, row, &a));
                }
            }
        }
        // two parameters at once (`PAIRED`)
        for p in 0..args.len() {
            for q in p + 1..args.len() {
                for x in PAIRED {
                    for y in PAIRED {
                        let (xv, yv) = (val(all, x), val(all, y));
                        if xv.name == args[p].name || yv.name == args[q].name {
                            continue;
                        }
                        let mut a = args.clone();
                        a[p] = xv;
                        a[q] = yv;
                        let id = format!("pair/{}/{}={},{}={}", row.id, p, x, q, y);
                        if ids.insert(id.clone()) {
                            found.push(sig_case(id, row, &a));
                        }
                    }
                }
            }
        }
    }
    (found, unexercised)
}

// ── running, under every configuration ──────────────────────────────

struct Config {
    name: &'static str,
    compiled: bool,
    env: &'static [(&'static str, &'static str)],
}

const CONFIGS: &[Config] = &[
    Config { name: "interpreted", compiled: false, env: &[] },
    Config { name: "walked", compiled: false, env: &[("CROW_WALK", "1")] },
    Config { name: "unfused", compiled: false, env: &[("CROW_NO_FUSE", "1")] },
    Config { name: "unmoved", compiled: false, env: &[("CROW_NO_MOVES", "1")] },
    Config { name: "compiled", compiled: true, env: &[] },
];

/// What a case did under one configuration: what it logged, and how its
/// run ended (`""` where it went on to the next case).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    out: String,
    end: String,
}

const LIMIT: Duration = Duration::from_secs(30);

/// What the builders' caches may hold before a run lets them go.
const CACHE_BUDGET: u64 = 80 << 30;

/// The bytes a directory holds, every file below it counted once.
fn cache_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => total += e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => {}
            }
        }
    }
    total
}

/// A batch's build: the Rust compiler's time, not a case's — a cold build
/// after the runtime's crates change compiles them again, SQLite's C among
/// them (held to a case's thirty seconds, one had been killed and named a
/// build that failed).
const BUILD_LIMIT: Duration = Duration::from_secs(1200);

/// What the machine lends a check: its memory less 9 GB kept for the system
/// (55 GB of 64; half of a machine of 18 GB or less) for every process the
/// check runs at once (`--memory GB` says another amount), and at most a
/// gigabyte for any one of them — a case that asks for the impossible
/// (`zeros[9223372036854775807]`, a range of 2^48 items) is stopped where it
/// passes it; the first version ran sixteen of those at once, unheld, and
/// took the whole machine down.
struct Budget {
    total: u64,
    each: u64,
}

static MEMORY_GB: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

fn budget() -> &'static Budget {
    static B: std::sync::OnceLock<Budget> = std::sync::OnceLock::new();
    B.get_or_init(|| {
        // SAFETY: sysconf reads a constant of the system; no memory is touched
        let (pages, size) = unsafe { (libc::sysconf(libc::_SC_PHYS_PAGES), libc::sysconf(libc::_SC_PAGESIZE)) };
        let ram = if pages > 0 && size > 0 { pages as u64 * size as u64 } else { 8 << 30 };
        let total = match MEMORY_GB.get() {
            Some(gb) => (gb << 30).min(ram),
            None if ram > 18 << 30 => ram - (9 << 30),
            None => ram / 2,
        };
        Budget { total, each: (1u64 << 30).min(total / 4) }
    })
}

/// The memory every process the check runs holds now, summed.
static IN_USE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A process's memory: its footprint (macOS), its resident pages (Linux).
fn footprint(pid: u32) -> u64 {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: the buffer is a zeroed rusage_info_v0, the flavor V0's
        let mut info: libc::rusage_info_v0 = unsafe { std::mem::zeroed() };
        let got = unsafe { libc::proc_pid_rusage(pid as libc::c_int, libc::RUSAGE_INFO_V0, &mut info as *mut libc::rusage_info_v0 as *mut libc::rusage_info_t) };
        if got == 0 { info.ri_phys_footprint } else { 0 }
    }
    #[cfg(target_os = "linux")]
    {
        // SAFETY: sysconf reads a constant of the system
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as u64;
        std::fs::read_to_string(format!("/proc/{}/statm", pid)).ok().and_then(|s| s.split_whitespace().nth(1).and_then(|r| r.parse::<u64>().ok())).map(|r| r * page).unwrap_or(0)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        0
    }
}

/// How a process ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ended {
    Exited,
    TimeLimit,
    Memory,
    Output,
}

/// What a process wrote: kept to a megabyte, the rest read and dropped
/// (a case printing gigabytes had nowhere to stop).
const OUTPUT: usize = 1 << 20;

fn read_capped(mut from: impl std::io::Read, over: std::sync::Arc<std::sync::atomic::AtomicBool>) -> String {
    let mut kept: Vec<u8> = Vec::new();
    let mut buf = [0u8; 65536];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if kept.len() + n <= OUTPUT {
                    kept.extend_from_slice(&buf[..n]);
                } else {
                    over.store(true, Ordering::SeqCst);
                }
            }
        }
    }
    String::from_utf8_lossy(&kept).to_string()
}

/// A process run to its end, its time limit, its share of memory or its
/// share of output: its exit code (none where it was stopped), what it
/// wrote, the last line of its errors, and how it ended.
fn spawn(cmd: Command) -> (Option<i32>, String, String, Ended) {
    let (code, out, err, ended) = spawn_all(cmd);
    let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").to_string();
    (code, out, last, ended)
}

/// `spawn`, every line of the errors kept.
fn spawn_all(cmd: Command) -> (Option<i32>, String, String, Ended) {
    spawn_within(cmd, LIMIT)
}

/// [`spawn_all`] with a time limit of its own.
fn spawn_within(mut cmd: Command, limit: Duration) -> (Option<i32>, String, String, Ended) {
    let b = budget();
    // (a process starts when the memory the check holds leaves room for it)
    while IN_USE.load(Ordering::SeqCst) + b.each > b.total {
        std::thread::sleep(Duration::from_millis(20));
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (None, String::new(), format!("cannot start: {}", e), Ended::Exited),
    };
    let over = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (o1, o2) = (over.clone(), over.clone());
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out = std::thread::spawn(move || stdout.map(|o| read_capped(o, o1)).unwrap_or_default());
    let err = std::thread::spawn(move || stderr.map(|e| read_capped(e, o2)).unwrap_or_default());
    let start = Instant::now();
    let pid = child.id();
    let mut held = 0u64;
    let (code, ended) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status.code(), Ended::Exited),
            Ok(None) => {
                let now = footprint(pid);
                if now >= held {
                    IN_USE.fetch_add(now - held, Ordering::SeqCst);
                } else {
                    IN_USE.fetch_sub(held - now, Ordering::SeqCst);
                }
                held = now;
                let why = if now > b.each {
                    Some(Ended::Memory)
                } else if over.load(Ordering::SeqCst) {
                    Some(Ended::Output)
                } else if start.elapsed() > limit {
                    Some(Ended::TimeLimit)
                } else {
                    None
                };
                if let Some(why) = why {
                    let _ = child.kill();
                    let _ = child.wait();
                    break (None, why);
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(_) => break (None, Ended::Exited),
        }
    };
    IN_USE.fetch_sub(held, Ordering::SeqCst);
    let out = out.join().unwrap_or_default();
    let err = err.join().unwrap_or_default();
    (code, out, err, ended)
}

/// The run's files — a text, a CSV, a JSON document, a directory — copied
/// into a case's own directory.
fn copy_files(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to.join("self_dir"))?;
    for f in ["self_text.txt", "self_data.csv", "self_data.json"] {
        std::fs::copy(from.join(f), to.join(f))?;
    }
    std::fs::copy(from.join("self_dir").join("inside.txt"), to.join("self_dir").join("inside.txt"))?;
    Ok(())
}

/// The directory a batch runs in under one configuration, made anew: the
/// files the cases read (a text, a CSV, a JSON document, a directory), and
/// nothing else — a case that writes, writes there.
fn sandbox(dir: &Path, file: &str, config: &str) -> PathBuf {
    let stem = file.trim_end_matches(".co");
    let s = dir.join(format!("run_{}_{}", stem, config));
    let _ = std::fs::remove_dir_all(&s);
    let _ = std::fs::create_dir_all(s.join("self_dir"));
    let _ = std::fs::write(s.join("self_text.txt"), "hello\nworld\n");
    let _ = std::fs::write(s.join("self_data.csv"), "a,b,c\n1,x,1.5\n2,y,2.5\n3,z,3.5\n");
    let _ = std::fs::write(s.join("self_data.json"), "{\"a\": 1, \"b\": [1, 2]}\n");
    let _ = std::fs::write(s.join("self_dir").join("inside.txt"), "inside\n");
    s
}

/// A case's process: its environment cleared — no key, no token, no
/// database's address reaches it — and given the run's own: a seed for the
/// draws, a clock that stands still, a replay with nothing recorded (every
/// request and every server's statement answered from it, so nothing goes
/// out) unless the case talks to a server of the check's own, the ports of
/// those servers, the door to the standard library's own layer (CROW_SYS),
/// and the configuration's switches.
fn case_command(program: &Path, run_dir: &Path, config: &Config, live: bool) -> Command {
    let mut cmd = Command::new(program);
    cmd.env_clear();
    for keep in ["PATH", "HOME", "TMPDIR", "USER", "LANG"] {
        if let Some(v) = std::env::var_os(keep) {
            cmd.env(keep, v);
        }
    }
    cmd.current_dir(run_dir).env("CROW_SEED", "1").env("CROW_CLOCK", "1767225600").env("CROW_SYS", "1");
    cmd.env("CROW_REPLAY", if live { Path::new("") } else { run_dir });
    for (k, v) in SERVED.get().map(|s| s.as_slice()).unwrap_or(&[]) {
        cmd.env(k, v);
    }
    for (k, v) in config.env {
        cmd.env(k, v);
    }
    cmd
}

// ── the check's own servers ─────────────────────────────────────────

/// The ports of the check's own servers, as each case's process is told
/// them: CROW_SELF_PORT (the echo server), CROW_SELF_CH (the ClickHouse
/// stand-in), CROW_SELF_PG (a scratch Postgres cluster).
static SERVED: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();

/// The servers the rows that speak to one are run against, on this
/// machine's loopback alone: an echo server that answers HTTP with `ok`
/// (the layer's streams, `exchange`, `fetch`, `put`), a ClickHouse
/// stand-in that answers its HTTP interface as the server would, and a
/// scratch Postgres cluster where `initdb` is on the PATH (stopped and
/// removed when the check ends, and before it starts where a stopped
/// check left one).
struct Servers {
    pg: Option<PathBuf>,
}

impl Servers {
    fn start(dir: &Path) -> Servers {
        let mut env = Vec::new();
        if let Ok(l) = std::net::TcpListener::bind("127.0.0.1:0") {
            if let Ok(a) = l.local_addr() {
                env.push(("CROW_SELF_PORT".to_string(), a.port().to_string()));
            }
            std::thread::spawn(move || {
                for c in l.incoming().flatten() {
                    std::thread::spawn(move || serve_echo(c));
                }
            });
        }
        if let Ok(l) = std::net::TcpListener::bind("127.0.0.1:0") {
            if let Ok(a) = l.local_addr() {
                env.push(("CROW_SELF_CH".to_string(), a.port().to_string()));
            }
            std::thread::spawn(move || {
                for c in l.incoming().flatten() {
                    std::thread::spawn(move || serve_clickhouse(c));
                }
            });
        }
        // (a WebSocket server that sends back what it is sent: web.ws's rows)
        if let Ok(l) = std::net::TcpListener::bind("127.0.0.1:0") {
            if let Ok(a) = l.local_addr() {
                env.push(("CROW_SELF_WS".to_string(), a.port().to_string()));
            }
            std::thread::spawn(move || {
                for c in l.incoming().flatten() {
                    std::thread::spawn(move || serve_ws(c));
                }
            });
        }
        // (a mail server that takes every message: web.mail's rows)
        if let Ok(l) = std::net::TcpListener::bind("127.0.0.1:0") {
            if let Ok(a) = l.local_addr() {
                env.push(("CROW_SELF_SMTP".to_string(), a.port().to_string()));
            }
            std::thread::spawn(move || {
                for c in l.incoming().flatten() {
                    std::thread::spawn(move || serve_smtp(c));
                }
            });
        }
        // (a model that speaks both wire formats: llm's rows; Anthropic's
        // base is the model's too, so a model reached as anthropic:// asks
        // this machine and nothing past it)
        if let Ok(l) = std::net::TcpListener::bind("127.0.0.1:0") {
            if let Ok(a) = l.local_addr() {
                env.push(("CROW_SELF_LLM".to_string(), a.port().to_string()));
                env.push(("ANTHROPIC_BASE_URL".to_string(), format!("http://127.0.0.1:{}", a.port())));
            }
            std::thread::spawn(move || {
                for c in l.incoming().flatten() {
                    std::thread::spawn(move || llm::serve_model(c));
                }
            });
        }
        let pg = scratch_postgres(&dir.join("pg"));
        if let Some((_, port)) = &pg {
            env.push(("CROW_SELF_PG".to_string(), port.to_string()));
        }
        let _ = SERVED.set(env);
        Servers { pg: pg.map(|(d, _)| d) }
    }
}

impl Drop for Servers {
    fn drop(&mut self) {
        if let Some(d) = &self.pg {
            let _ = Command::new("pg_ctl").arg("-D").arg(d).args(["-m", "immediate", "-w", "stop"]).output();
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

/// A scratch Postgres cluster: its directory and port, or None where the
/// machine has no `initdb` (the Postgres cases then reach no server, alike
/// in every configuration).
fn scratch_postgres(dir: &Path) -> Option<(PathBuf, u16)> {
    if !crate::home::on_path("initdb") {
        return None;
    }
    let dir = std::fs::canonicalize(dir.parent()?).ok()?.join(dir.file_name()?);
    // (a check stopped from outside left its cluster running)
    if dir.join("postmaster.pid").exists() {
        let _ = Command::new("pg_ctl").arg("-D").arg(&dir).args(["-m", "immediate", "-w", "stop"]).output();
    }
    let _ = std::fs::remove_dir_all(&dir);
    let init = Command::new("initdb").arg("-D").arg(&dir).args(["-U", "crow", "--auth=trust"]).output().ok()?;
    if !init.status.success() {
        return None;
    }
    let port = std::net::TcpListener::bind("127.0.0.1:0").ok()?.local_addr().ok()?.port();
    let opts = format!("-p {} -c unix_socket_directories='' -c listen_addresses=127.0.0.1", port);
    let started = Command::new("pg_ctl").arg("-D").arg(&dir).args(["-o", &opts, "-w", "-l"]).arg(dir.join("log")).arg("start").output().ok()?;
    started.status.success().then_some((dir, port))
}

/// One connection to the mail server, as RFC 5321 has it: a greeting,
/// `EHLO` answered with `AUTH PLAIN` offered, any credentials taken, the
/// envelope and the data (to the line holding a lone dot) each answered
/// 250, a recipient that is no address refused (550), `QUIT` answered.
fn serve_smtp(s: std::net::TcpStream) {
    use std::io::{BufRead, Write};
    let _ = s.set_read_timeout(Some(Duration::from_secs(60)));
    let Ok(mut out) = s.try_clone() else { return };
    let mut reader = std::io::BufReader::new(s);
    if out.write_all(b"220 self ESMTP\r\n").is_err() {
        return;
    }
    let mut data = false;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let said = line.trim_end_matches(['\r', '\n']);
        if data {
            if said == "." {
                data = false;
                if out.write_all(b"250 2.0.0 queued\r\n").is_err() {
                    return;
                }
            }
            continue;
        }
        let verb = said.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
        let reply: &[u8] = match verb.as_str() {
            "EHLO" | "HELO" => b"250-self\r\n250-AUTH PLAIN\r\n250 8BITMIME\r\n",
            "AUTH" => b"235 2.7.0 accepted\r\n",
            "MAIL" => b"250 2.1.0 ok\r\n",
            "RCPT" if said.contains('@') => b"250 2.1.5 ok\r\n",
            "RCPT" => b"550 5.1.3 not an address\r\n",
            "DATA" => {
                data = true;
                b"354 go on\r\n"
            }
            "RSET" | "NOOP" => b"250 ok\r\n",
            "QUIT" => {
                let _ = out.write_all(b"221 bye\r\n");
                return;
            }
            _ => b"502 5.5.2 not that\r\n",
        };
        if out.write_all(reply).is_err() {
            return;
        }
    }
}

/// One connection to the WebSocket server: each text or binary message
/// sent back as it came, a ping answered, until the client closes.
fn serve_ws(s: std::net::TcpStream) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(60)));
    let Ok(mut ws) = tungstenite::accept(s) else { return };
    while let Ok(m) = ws.read() {
        match m {
            tungstenite::Message::Text(_) | tungstenite::Message::Binary(_) => {
                if ws.send(m).is_err() {
                    return;
                }
            }
            tungstenite::Message::Close(_) => return,
            _ => {}
        }
    }
}

/// One connection to the echo server: an HTTP request — its head, then its
/// body by its length — answered `200 ok`, the connection kept; anything
/// else sent back as it came.
fn serve_echo(mut s: std::net::TcpStream) {
    use std::io::{Read, Write};
    let _ = s.set_read_timeout(Some(Duration::from_secs(60)));
    let mut held: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = match s.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        held.extend_from_slice(&buf[..n]);
        let http = [&b"GET "[..], b"POST ", b"PUT ", b"HEAD ", b"DELETE ", b"PATCH ", b"OPTIONS "].iter().any(|m| held.starts_with(m));
        if !http {
            if s.write_all(&held).is_err() {
                return;
            }
            held.clear();
            continue;
        }
        while let Some(end) = held.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&held[..end]).to_ascii_lowercase();
            let length = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
            if held.len() < end + 4 + length {
                break;
            }
            held.drain(..end + 4 + length);
            if s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\nok").is_err() {
                return;
            }
        }
    }
}

/// RowBinaryWithNamesAndTypes of two columns, `n UInt64` and `x
/// Nullable(Float64)`: (1, 2.5), (2, NULL), (3, 0.5).
fn rows_n_x() -> Vec<u8> {
    let mut b = vec![2u8, 1, b'n', 1, b'x', 6];
    b.extend_from_slice(b"UInt64");
    b.push(17);
    b.extend_from_slice(b"Nullable(Float64)");
    for (n, x) in [(1u64, Some(2.5f64)), (2, None), (3, Some(0.5))] {
        b.extend_from_slice(&n.to_le_bytes());
        match x {
            Some(x) => {
                b.push(0);
                b.extend_from_slice(&x.to_le_bytes());
            }
            None => b.push(1),
        }
    }
    b
}

/// RowBinaryWithNamesAndTypes of columns of text, as ClickHouse answers a
/// query of its own tables (`system.columns`).
fn rows_of_text(names: &[&str], rows: &[&[&str]]) -> Vec<u8> {
    let mut b = vec![names.len() as u8];
    for n in names {
        b.push(n.len() as u8);
        b.extend_from_slice(n.as_bytes());
    }
    for _ in names {
        b.push(6);
        b.extend_from_slice(b"String");
    }
    for row in rows {
        for cell in *row {
            b.push(cell.len() as u8);
            b.extend_from_slice(cell.as_bytes());
        }
    }
    b
}

/// One connection to the ClickHouse stand-in: each statement answered as
/// the server's HTTP interface answers it (tests/clickhouse_http.rs holds
/// the driver to the same answers), the connection kept.
fn serve_clickhouse(stream: std::net::TcpStream) {
    use std::io::{BufRead, Read, Write};
    let Ok(read) = stream.try_clone() else { return };
    let mut reader = std::io::BufReader::new(read);
    let mut out = stream;
    loop {
        let mut first = String::new();
        if reader.read_line(&mut first).unwrap_or(0) == 0 {
            return;
        }
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = v.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; length];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        let end = body.iter().position(|&c| c == b'\n').unwrap_or(body.len());
        let sql = String::from_utf8_lossy(&body[..end]).to_string();
        let (status, answer): (&str, Vec<u8>) = if sql.starts_with("SELECT 1") {
            ("200 OK", b"1\n".to_vec())
        } else if sql.starts_with("KILL QUERY") || sql.starts_with("SELECT DISTINCT query_id") || sql.starts_with("INSERT INTO") || sql.starts_with("CREATE") {
            ("200 OK", Vec::new())
        } else if sql.starts_with("DESCRIBE TABLE t") {
            ("200 OK", b"n\tUInt64\t\t\t\t\t\nname\tNullable(String)\t\t\t\t\t\n".to_vec())
        } else if sql.starts_with("DESCRIBE TABLE (") && !sql.contains("nope") {
            ("200 OK", b"n\tUInt64\t\t\t\t\t\nx\tNullable(Float64)\t\t\t\t\t\n".to_vec())
        } else if sql.starts_with("SELECT number AS n") {
            ("200 OK", rows_n_x())
        } else if sql.starts_with("SELECT name, type FROM system.columns") {
            // (the columns of a table a statement made: `u`'s one)
            ("200 OK", rows_of_text(&["name", "type"], &[&["n", "UInt64"]]))
        } else if sql.contains("nope") {
            ("404 Not Found", b"Code: 60. DB::Exception: Unknown table expression identifier 'nowhere' in scope SELECT nope FROM nowhere. (UNKNOWN_TABLE)\n".to_vec())
        } else {
            ("500 Internal Server Error", format!("Code: 62. DB::Exception: the check's stand-in does not know `{}`\n", sql).into_bytes())
        };
        let head = format!("HTTP/1.1 {}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: Keep-Alive\r\nX-ClickHouse-Query-Id: self\r\n\r\n", status, answer.len());
        if out.write_all(head.as_bytes()).and_then(|_| out.write_all(&answer)).and_then(|_| out.flush()).is_err() {
            return;
        }
    }
}

/// A batch's cases run under one configuration, from the first to the
/// last, each run of the batch's program from `CROW_FROM` to `CROW_TO`. A
/// case that stops the run (its words flushed) gets the stop, and the next
/// run starts after it; one that calls `exit` ends only its own part. A
/// case that draws random numbers runs in a process of its own, the seed
/// fresh, so what it draws is its own and no other case's (one case had
/// drawn in one mode and not the other, and every later draw differed). A
/// run stopped from outside — its memory, its time, its output — or one
/// that ends with no case's marker may have lost what it had not written
/// yet, so the cases from its last marker run one to a process until the
/// one that ends so is found.
#[allow(clippy::too_many_arguments)]
fn run_batch(exe: &Path, dir: &Path, file: &str, binary: Option<&Path>, config: &Config, fresh: &[bool], live: bool, own: bool) -> Vec<Seen> {
    let n = fresh.len();
    let mut seen: Vec<Option<Seen>> = vec![None; n];
    let mut from = 0;
    let mut hunting = false;
    let run_dir = sandbox(dir, file, config.name);
    // (each case's own directory, its files the run's)
    if own {
        for k in 0..n {
            let _ = copy_files(&run_dir, &run_dir.join(format!("c{}", k)));
        }
    }
    let source = dir.join(file);
    while from < n {
        let to = if hunting || fresh[from] { from } else { (from + 1..n).find(|&k| fresh[k]).map(|k| k - 1).unwrap_or(n - 1) };
        let mut cmd = match binary {
            Some(b) => case_command(b, &run_dir, config, live),
            None => {
                let mut c = case_command(exe, &run_dir, config, live);
                c.arg("run").arg("-i").arg(&source);
                c
            }
        };
        cmd.env("CROW_FROM", format!("c{}", from)).env("CROW_TO", format!("c{}", to));
        let (code, out, last, ended) = spawn(cmd);
        // the output by case
        let mut marked: Vec<usize> = Vec::new();
        let mut texts: BTreeMap<usize, String> = BTreeMap::new();
        for line in out.lines() {
            if let Some(k) = line.strip_prefix("§§c").and_then(|k| k.parse::<usize>().ok()) {
                marked.push(k);
                texts.entry(k).or_default();
                continue;
            }
            if let Some(k) = marked.last() {
                let t = texts.entry(*k).or_default();
                t.push_str(line);
                t.push('\n');
            }
        }
        let keep = |seen: &mut Vec<Option<Seen>>, below: usize| {
            for (k, t) in &texts {
                if *k < below && *k < n {
                    seen[*k] = Some(Seen { out: t.clone(), end: String::new() });
                }
            }
        };
        if code == Some(0) && ended == Ended::Exited {
            keep(&mut seen, n);
            match marked.last().copied() {
                // (a case called `exit`: the run ended there, and well)
                Some(k) if k < to => {
                    seen[k] = Some(Seen { out: texts.get(&k).cloned().unwrap_or_default(), end: "exit: 0".to_string() });
                    from = k + 1;
                }
                _ => from = to + 1,
            }
            continue;
        }
        let end = if ended == Ended::TimeLimit {
            "hang: the time limit".to_string()
        } else if ended == Ended::Memory {
            format!("memory: past {} MB", budget().each >> 20)
        } else if ended == Ended::Output {
            format!("output: past {} MB written", OUTPUT >> 20)
        } else if code.is_none() {
            format!("crash: a signal ({})", last)
        } else if last.contains("panicked") || code == Some(101) {
            format!("crash: {}", last)
        } else if last.is_empty() {
            format!("exit: {}", code.unwrap_or(0))
        } else {
            format!("stop: {}", last)
        };
        let flushed = ended == Ended::Exited && code.is_some() && !end.starts_with("crash") && !marked.is_empty();
        if !flushed && to > from {
            // (what came before the last marker is whole; from it on, one by one)
            let restart = marked.last().copied().unwrap_or(from);
            keep(&mut seen, restart);
            hunting = true;
            from = restart;
            continue;
        }
        let at = if to == from { from } else { marked.last().copied().unwrap_or(from) };
        keep(&mut seen, at);
        if at < n {
            seen[at] = Some(Seen { out: texts.get(&at).cloned().unwrap_or_default(), end });
        }
        from = at + 1;
        hunting = false;
    }
    seen.into_iter().map(|s| s.unwrap_or(Seen { out: String::new(), end: "not run".to_string() })).collect()
}

/// The cases the reference answers, written as JSON lines — each one's
/// computation and what it showed (its own lines, without the `0` its
/// function's return logged), how it stopped, or what refused it — handed
/// to tests/self/reference.py. A case it answers otherwise is a finding
/// (`wrong`); one refused that it answers is listed in `answerable.txt`
/// (a refusal may be the spec's: a kind the spec does not mix). Answers
/// what the reference said of its count, and the number wrong.
fn consult_reference(dir: &Path, cases: &[Case], seen: &HashMap<String, Seen>, refused: &HashMap<String, String>, findings: &mut Vec<Finding>) -> Referred {
    let reference = Path::new("tests/self/reference.py");
    if !reference.is_file() {
        return Referred { said: "no tests/self/reference.py here: the spec's answers were not asked".to_string(), ..Referred::default() };
    }
    let mut lines = String::new();
    for case in cases {
        let Some((what, args)) = &case.refer else { continue };
        let mut entry = serde_json::json!({ "id": case.id, "what": what, "args": args });
        if let Some(s) = seen.get(&case.id) {
            let own = if s.end.is_empty() { s.out.strip_suffix("0\n").filter(|r| r.is_empty() || r.ends_with('\n')).unwrap_or(&s.out) } else { &s.out };
            entry["seen"] = serde_json::Value::String(own.trim_end_matches('\n').to_string());
            entry["end"] = serde_json::Value::String(s.end.clone());
        } else if let Some(m) = refused.get(&case.id) {
            entry["refused"] = serde_json::Value::String(m.clone());
        } else {
            continue;
        }
        lines.push_str(&entry.to_string());
        lines.push('\n');
    }
    let input = dir.join("answers.jsonl");
    if std::fs::write(&input, &lines).is_err() {
        return Referred { said: format!("cannot write {}", input.display()), ..Referred::default() };
    }
    // (`CROW_SELF_PYTHON` names a Python with more of the libraries the
    // reference asks — pyarrow, scipy — than the one on the path)
    let python = std::env::var("CROW_SELF_PYTHON").ok().filter(|p| !p.trim().is_empty()).unwrap_or_else(|| "python3".to_string());
    let out = match Command::new(&python).arg("-E").arg(reference).arg(&input).stdin(Stdio::null()).output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        Ok(o) => return Referred { said: format!("the reference failed: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("")), ..Referred::default() },
        Err(e) => return Referred { said: format!("no {} to ask the reference: {}", python, e), ..Referred::default() },
    };
    let by_id: HashMap<&str, &Case> = cases.iter().map(|c| (c.id.as_str(), c)).collect();
    let (mut wrong, mut said, mut answerable) = (0, String::new(), String::new());
    let (mut answered, mut needs) = (HashSet::new(), BTreeMap::new());
    for line in out.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let text = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        match v.get("kind").and_then(|k| k.as_str()) {
            Some("wrong") => {
                let id = text("id");
                let Some(case) = by_id.get(id.as_str()) else { continue };
                wrong += 1;
                findings.push(Finding {
                    id: id.clone(),
                    code: case.code(),
                    class: "wrong",
                    detail: format!("answers {} where the spec answers {}", text("seen").replace('\n', " | "), text("expected").replace('\n', " | ")),
                });
            }
            Some("refused") => answerable.push_str(&format!("{}\t{}\t{}\n", text("id"), text("expected"), text("seen"))),
            Some("answered") => {
                answered.insert(text("id"));
            }
            Some("needs") => {
                needs.insert(text("id"), text("library"));
            }
            _ => said = format!("{} answered, {} with no answer of its own", v.get("answered").unwrap_or(&serde_json::Value::Null), v.get("skipped").unwrap_or(&serde_json::Value::Null)),
        }
    }
    let _ = std::fs::write(dir.join("answerable.txt"), &answerable);
    if !answerable.is_empty() {
        said = format!("{}; {} refused that it answers (answerable.txt)", said, answerable.lines().count());
    }
    let libraries: BTreeSet<&String> = needs.values().collect();
    if !libraries.is_empty() {
        said = format!("{}; {} cases need {} this machine lacks", said, needs.len(), libraries.iter().map(|l| l.as_str()).collect::<Vec<_>>().join(" and "));
    }
    Referred { said, wrong, ran: true, answered, needs }
}

/// What the reference said of a run's cases.
#[derive(Default)]
struct Referred {
    /// A line for the run's report.
    said: String,
    wrong: usize,
    /// Whether the reference ran at all (python3, and the script here).
    ran: bool,
    /// The cases it answered.
    answered: HashSet<String>,
    /// The cases it would answer with a library this machine lacks, by case.
    needs: BTreeMap<String, String>,
}

/// A case's output as two cases compare: the lines a failure names and the
/// numbers each case's own copy of a helper is given made alike (each case
/// is placed elsewhere in its batch).
fn comparable(out: &str) -> String {
    static LINE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"line \d+").unwrap_or_else(|_| unreachable!()));
    static COPY: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\b((?:Self|self_)[A-Za-z]*)_\d+\b").unwrap_or_else(|_| unreachable!()));
    let out = LINE.replace_all(out, "line N");
    COPY.replace_all(&out, "$1").to_string()
}

/// The case a generated name belongs to: `self_case_3`, `self_may_3`,
/// `self_first_3__2` (a copy per signature), `SelfBox_3`.
fn case_number(name: &str) -> Option<usize> {
    if !(name.starts_with("self_") || name.starts_with("Self")) {
        return None;
    }
    let base = name.split("__").next().unwrap_or(name);
    let digits: String = base.rsplit('_').next().unwrap_or("").to_string();
    digits.parse().ok()
}

/// A batch built for the compiled configuration. Where the Rust compiler
/// refuses the generated program, each error is placed in the case whose
/// function holds its line — that case is a finding, with the compiler's
/// words — and the rest is built again; a batch whose errors no case holds
/// is split in two.
/// A batch built: where it lies, its program's file and its binary, and its
/// cases (each with its index among all the cases).
struct Built {
    dir: PathBuf,
    file: String,
    binary: PathBuf,
    members: Vec<(usize, Case)>,
}

fn build(exe: &Path, dir: &Path, stem: &str, cases: Vec<(usize, Case)>, jobs: &str, built: &mut Vec<Built>, failed: &mut Vec<(usize, String)>) {
    let mut cases = cases;
    for _round in 0..12 {
        if cases.is_empty() {
            return;
        }
        let refs: Vec<&Case> = cases.iter().map(|(_, c)| c).collect();
        let (source, _) = program(&refs);
        let file = format!("{}.co", stem);
        if std::fs::write(dir.join(&file), &source).is_err() {
            for (i, _) in &cases {
                failed.push((*i, "cannot write the batch".to_string()));
            }
            return;
        }
        let mut cmd = Command::new(exe);
        // (no incremental data and no debug info: a batch's program is a few
        // tens of megabytes kept for the next run, where with them each
        // was hundreds, and the caches had passed 240 GB)
        cmd.args(["build", &file])
            .current_dir(dir)
            .env("CROW_BUILD_PROFILE", "dev")
            .env("CARGO_BUILD_JOBS", jobs)
            .env("CARGO_INCREMENTAL", "0")
            .env("CARGO_PROFILE_DEV_DEBUG", "0");
        let (code, out, err, ended) = spawn_within(cmd, BUILD_LIMIT);
        let binary = dir.join(stem);
        if code == Some(0) && binary.exists() {
            built.push(Built { dir: dir.to_path_buf(), file, binary, members: cases });
            return;
        }
        // each error's line in the generated program, and the case function
        // around it
        let generated = std::fs::read_to_string(dir.join(".crow_build").join("src").join("main.rs")).unwrap_or_default();
        let mut owner: Vec<Option<usize>> = Vec::new();
        let mut current = None;
        for line in generated.lines() {
            let decl = line.strip_prefix("pub ").unwrap_or(line);
            if let Some(rest) = decl.strip_prefix("fn ").or_else(|| decl.strip_prefix("struct ")).or_else(|| decl.strip_prefix("impl ")) {
                let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                current = case_number(&name);
            }
            owner.push(current);
        }
        let text = format!("{}\n{}", out, err);
        let lines: Vec<&str> = text.lines().collect();
        let mut placed: BTreeMap<usize, String> = BTreeMap::new();
        for (n, line) in lines.iter().enumerate() {
            let compiler_error = line.starts_with("error[") || (line.starts_with("error:") && !line.contains("could not compile") && !line.contains("does not build"));
            if !compiler_error {
                continue;
            }
            let at = lines.iter().skip(n + 1).take(4).find_map(|l| l.trim_start().strip_prefix("--> src/main.rs:"));
            let Some(at) = at else { continue };
            let Some(row) = at.split(':').next().and_then(|r| r.parse::<usize>().ok()) else { continue };
            if let Some(Some(k)) = owner.get(row.saturating_sub(1)) {
                // (the compiler's own detail — what it expected and found,
                // and the generated line — travels with the finding: the
                // batch's build is let go, and a case that builds alone had
                // failed only in its batch's company)
                let detail: Vec<String> = lines.iter().skip(n + 1).take(12).filter(|l| l.contains("expected") || l.contains("found") || l.trim_start().starts_with(|c: char| c.is_ascii_digit())).map(|l| l.trim().to_string()).collect();
                let generated_line = generated.lines().nth(row.saturating_sub(1)).map(|l| l.trim().chars().take(400).collect::<String>()).unwrap_or_default();
                placed.entry(*k).or_insert_with(|| format!("{} [{}] generated: {}", line, detail.join(" | "), generated_line));
            }
        }
        if placed.is_empty() {
            if cases.len() == 1 {
                let stopped = match ended {
                    Ended::TimeLimit => "the build ran past its time",
                    Ended::Memory => "the build ran past its memory",
                    Ended::Output => "the build wrote past its output",
                    Ended::Exited => "the build failed",
                };
                let why = lines.iter().find(|l| l.starts_with("error")).copied().unwrap_or(stopped).to_string();
                failed.push((cases[0].0, why));
                return;
            }
            let right = cases.split_off(cases.len() / 2);
            build(exe, dir, &format!("{}a", stem), cases, jobs, built, failed);
            build(exe, dir, &format!("{}b", stem), right, jobs, built, failed);
            return;
        }
        let mut kept = Vec::with_capacity(cases.len());
        for (k, (i, c)) in cases.into_iter().enumerate() {
            match placed.get(&k) {
                Some(why) => failed.push((i, why.clone())),
                None => kept.push((i, c)),
            }
        }
        cases = kept;
    }
    for (i, _) in &cases {
        failed.push((*i, "the batch did not build after twelve rounds".to_string()));
    }
}

// ── findings ────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct Finding {
    id: String,
    /// The case's own lines, as a program writes them.
    code: String,
    class: &'static str,
    detail: String,
}

fn known_path() -> PathBuf {
    PathBuf::from("tests/self/known.txt")
}

fn read_known() -> BTreeSet<(String, String)> {
    std::fs::read_to_string(known_path())
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t').map(|(c, i)| (c.to_string(), i.to_string())))
        .collect()
}

fn write_known(findings: &[Finding]) -> std::io::Result<()> {
    let mut lines: Vec<String> = findings.iter().map(|f| format!("{}\t{}", f.class, f.id)).collect();
    lines.sort();
    lines.dedup();
    let header = "# crow check --self: the findings known, one per line (class, a tab, the case).\n# The list only shrinks: a new finding fails the check, and so does a line\n# no run finds any more. Each is a bug in Crow to fix, with its test.\n";
    if let Some(dir) = known_path().parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(known_path(), format!("{}{}\n", header, lines.join("\n")))
}

/// What findings of one defect share: the message with the program's own
/// names and lines taken out (a build error, an internal error), or for a
/// disagreement the family and the configurations that part from the
/// interpreter.
fn group_key(f: &Finding) -> String {
    let first = f.detail.lines().next().unwrap_or("");
    let family: String = f.id.split('/').take(2).collect::<Vec<_>>().join("/");
    let mut key = if f.class == "disagree" || f.class == "memory" || f.class == "hang" {
        format!("{} — {}", family, first)
    } else if f.class == "wrong" {
        // (a wrong answer by where the value went and what it was)
        format!("{} — {}", family, f.id.rsplit('/').next().unwrap_or(""))
    } else {
        first.to_string()
    };
    // (numbers that name a place or a case, not the defect)
    let mut out = String::with_capacity(key.len());
    let mut chars = key.chars().peekable();
    let mut prev = ' ';
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() && (prev == '_' || prev == ' ' || prev == ':') {
            while chars.peek().is_some_and(|d| d.is_ascii_digit()) {
                chars.next();
            }
            out.push('N');
            prev = 'N';
            continue;
        }
        out.push(c);
        prev = c;
    }
    key = out;
    key
}

/// The rules a refusal keeps: it names a line, and only what the program
/// wrote.
fn message_findings(case: &Case, source: &str, message: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let first = message.lines().next().unwrap_or("");
    if !first.contains("line ") && !first.chars().take_while(|c| *c != ' ').any(|c| c == ':') {
        out.push(Finding { id: case.id.clone(), code: case.code(), class: "message-place", detail: first.to_string() });
    }
    // (Rust's words and the compiler's own: a message is in the program's)
    const JARGON: &[&str] = &["target type", "usize", "i64", "f64", "Vec<", "Option<", "Result<", "&str", "BTreeMap", "CrowValue", "CrowTensor", "Ty::", "Expr::", "Kind::", "Arg::", "unwrap", "panicked"];
    if let Some(word) = JARGON.iter().find(|w| first.contains(*w)) {
        out.push(Finding { id: case.id.clone(), code: case.code(), class: "message-jargon", detail: format!("{} — says '{}'", first, word) });
    }
    let foreign = crate::errors_corpus::foreign_names(source, first);
    if !foreign.is_empty() {
        out.push(Finding { id: case.id.clone(), code: case.code(), class: "message-names", detail: format!("{} — names {}", first, foreign.join(", ")) });
    }
    out
}

/// `crow check --self [--only word] [--lower-only] [--bless] [--clean]
/// [--memory GB]`.
pub fn cmd_self_check(args: &[String]) -> i32 {
    let only = args.iter().position(|a| a == "--only").and_then(|i| args.get(i + 1)).cloned();
    let lower_only = args.iter().any(|a| a == "--lower-only");
    let bless = args.iter().any(|a| a == "--bless");
    let clean = args.iter().any(|a| a == "--clean");
    if let Some(gb) = args.iter().position(|a| a == "--memory").and_then(|i| args.get(i + 1)).and_then(|g| g.parse::<u64>().ok()) {
        let _ = MEMORY_GB.set(gb.max(2));
    }
    let start = Instant::now();
    // the standard library's own layer is the check's to call, as the
    // book's page on it is (`use sys`, and `use db.postgres`'s rows): the
    // lowering here, the builds and the cases' processes read the door
    // SAFETY: set before any thread of the check starts
    unsafe { std::env::set_var("CROW_SYS", "1") };
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = PathBuf::from("target").join("crow_self");
    let _ = std::fs::create_dir_all(&dir);

    // the cases
    let all = values();
    let mut cases = form_cases(&all);
    cases.extend(web::route_cases(&all));
    cases.extend(web::view_cases(&all));
    cases.extend(web::state_cases());
    cases.extend(web::ws_cases());
    cases.extend(web::mail_cases());
    cases.extend(draw_cases());
    cases.extend(io_cases());
    cases.extend(grad_cases());
    cases.extend(llm::llm_cases());
    cases.extend(web::auth_cases());
    // (the gpu family's cases are twins of the rows' and the forms': a word
    // naming its cases names their twins' rows)
    let rows_only = only.as_ref().map(|w| w.strip_prefix("gpu/").unwrap_or(w).to_string());
    let (rows, unexercised) = row_cases(&all, &rows_only);
    cases.extend(rows);
    let twins = gpu_cases(&cases, &all);
    cases.extend(twins);
    if let Some(w) = &only {
        cases.retain(|c| c.id.contains(w.as_str()));
    }
    let row_count = cases.iter().filter(|c| c.id.starts_with("row/") || c.id.starts_with("pair/")).count();
    let mut ids = HashSet::new();
    cases.retain(|c| ids.insert(c.id.clone()));
    println!("memory: {} GB for the cases' processes, 1 GB each at most (--memory GB says another)", budget().total >> 30);
    println!("crow check --self: {} cases ({} of builtin rows, the rest of the language's forms){}\n", cases.len(), row_count, only.as_ref().map(|w| format!(", those holding '{}'", w)).unwrap_or_default());

    // lowered, each alone
    let sources: Vec<String> = cases.iter().map(|c| program(&[c]).0).collect();
    let t = Instant::now();
    let lowered = lower_all(sources.clone());
    let mut findings: Vec<Finding> = Vec::new();
    let mut accepted: Vec<usize> = Vec::new();
    let (mut refused, mut internal) = (0, 0);
    let mut refusals = String::new();
    // (what each refused case was told, for the reference)
    let mut refused_by: HashMap<String, String> = HashMap::new();
    let mut families: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
    for (i, l) in lowered.iter().enumerate() {
        let family = cases[i].id.split('/').next().unwrap_or("");
        let f = families.entry(family).or_default();
        match l {
            Lowered::Accepted => f.0 += 1,
            Lowered::Refused(_) => f.1 += 1,
            Lowered::Bug(_) => f.2 += 1,
        }
        match l {
            Lowered::Accepted => accepted.push(i),
            Lowered::Refused(m) => {
                refused += 1;
                refusals.push_str(&format!("{}\t{}\n", cases[i].id, m.lines().next().unwrap_or("")));
                refused_by.insert(cases[i].id.clone(), m.lines().next().unwrap_or("").to_string());
                findings.extend(message_findings(&cases[i], &sources[i], m));
            }
            Lowered::Bug(m) => {
                internal += 1;
                findings.push(Finding { id: cases[i].id.clone(), code: cases[i].code(), class: "internal", detail: m.lines().next().unwrap_or("").to_string() });
            }
        }
    }
    println!("lowered in {:.1}s: {} accepted, {} refused, {} a bug in Crow", t.elapsed().as_secs_f64(), accepted.len(), refused, internal);
    for (family, (a, r, b)) in &families {
        println!("  {:<12} {:>6} accepted {:>6} refused {:>4} a bug in Crow", family, a, r, b);
    }
    let _ = std::fs::write(dir.join("refusals.txt"), &refusals);
    // the forms of the language the cases that lower reach (the forms guard),
    // and the standard modules' own tests where the test-block family runs
    let mut reaching: Vec<&String> = accepted.iter().map(|&i| &sources[i]).collect();
    let module_sources: Vec<String> = if !lower_only && only.is_none() { module_files().iter().filter_map(|p| std::fs::read_to_string(p).ok()).collect() } else { Vec::new() };
    reaching.extend(module_sources.iter());
    let reached = forms_reached(&reaching);
    let forms_now: BTreeMap<String, String> = every_form().into_iter().filter(|f| !reached.contains(f)).map(|f| (f, "no case reaches it".to_string())).collect();
    println!("forms: {} of the language's {} reached", reached.len(), every_form().len());
    // the builtins held to no right answer, where the reference ran (the
    // agreement-only guard); and the builtins no case reached at its run
    let mut agreement_now: Option<BTreeMap<String, String>> = None;
    let mut not_called = unexercised;

    if !lower_only {
        // batches: the cases that lower, by the modules they import
        const BATCH: usize = 120;
        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for &i in &accepted {
            // (a case that talks to the check's own servers runs without
            // the replay: a batch of its own)
            let key = format!(
                "{}{}{}{}{}",
                cases[i].imports.iter().copied().collect::<Vec<_>>().join(","),
                if cases[i].live { "|live" } else { "" },
                if cases[i].serve.is_some() { "|served" } else { "" },
                if cases[i].own_dir { "|own" } else { "" },
                if cases[i].gpu { "|gpu" } else { "" }
            );
            groups.entry(key).or_default().push(i);
        }
        let mut batches: Vec<Vec<usize>> = Vec::new();
        for (_, members) in groups {
            for chunk in members.chunks(BATCH) {
                batches.push(chunk.to_vec());
            }
        }
        // (a batch that does not lower as a whole is split until each part
        // does: a case that lowers alone but not beside the others is a
        // finding)
        let mut ready: Vec<Vec<usize>> = Vec::new();
        let mut todo = batches;
        let mut splits = String::new();
        while let Some(b) = todo.pop() {
            let refs: Vec<&Case> = b.iter().map(|&i| &cases[i]).collect();
            match lower(program(&refs).0) {
                Lowered::Accepted => ready.push(b),
                Lowered::Refused(m) | Lowered::Bug(m) => {
                    splits.push_str(&format!("{} cases: {}\n", b.len(), m.lines().next().unwrap_or("")));
                    if b.len() == 1 {
                        findings.push(Finding { id: cases[b[0]].id.clone(), code: cases[b[0]].code(), class: "batch", detail: format!("lowers alone, not beside other cases: {}", m.lines().next().unwrap_or("")) });
                    } else {
                        let mut left = b;
                        let right = left.split_off(left.len() / 2);
                        todo.push(left);
                        todo.push(right);
                    }
                }
            }
        }
        ready.sort();
        let _ = std::fs::write(dir.join("splits.txt"), &splits);
        // compiled: each batch built, split where it does not build
        let t = Instant::now();
        let build_dir = dir.join("build");
        let _ = std::fs::create_dir_all(&build_dir);
        let build_dir = std::fs::canonicalize(&build_dir).unwrap_or(build_dir);
        // (builders in directories of their own, each with its build cache:
        // a crate's build is mostly one core's work)
        let cores = std::thread::available_parallelism().map(|c| c.get()).unwrap_or(4);
        let builders = (cores / 5).clamp(1, 3);
        let jobs = (cores / (2 * builders)).max(1).to_string();
        let next = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        let built_all: Mutex<Vec<Built>> = Mutex::new(Vec::new());
        let unbuilt_all: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());
        std::thread::scope(|s| {
            for w in 0..builders {
                let (next, done, built_all, unbuilt_all, ready, cases, exe, jobs) = (&next, &done, &built_all, &unbuilt_all, &ready, &cases, &exe, &jobs);
                let wdir = build_dir.join(format!("w{}", w));
                s.spawn(move || {
                    let _ = std::fs::create_dir_all(&wdir);
                    let (mut built, mut unbuilt) = (Vec::new(), Vec::new());
                    loop {
                        let k = next.fetch_add(1, Ordering::SeqCst);
                        if k >= ready.len() {
                            break;
                        }
                        let members: Vec<(usize, Case)> = ready[k].iter().map(|&i| (i, cases[i].clone())).collect();
                        build(exe, &wdir, &format!("self_{}", k), members, jobs, &mut built, &mut unbuilt);
                        let d = done.fetch_add(1, Ordering::SeqCst) + 1;
                        print!("\rbuilt {} of {} batches", d, ready.len());
                        let _ = std::io::Write::flush(&mut std::io::stdout());
                    }
                    if let Ok(mut b) = built_all.lock() {
                        b.extend(built);
                    }
                    if let Ok(mut u) = unbuilt_all.lock() {
                        u.extend(unbuilt);
                    }
                });
            }
        });
        let mut built = built_all.into_inner().unwrap_or_default();
        built.sort_by(|a, b| a.members.first().map(|m| m.0).cmp(&b.members.first().map(|m| m.0)));
        let unbuilt = unbuilt_all.into_inner().unwrap_or_default();
        println!("\rbuilt {} batches in {:.1}s{}", ready.len(), t.elapsed().as_secs_f64(), " ".repeat(20));
        for (i, why) in &unbuilt {
            findings.push(Finding { id: cases[*i].id.clone(), code: cases[*i].code(), class: "no-build", detail: why.clone() });
        }
        // the check's own servers, for the rows that speak to one (stopped
        // when the runs are done)
        let servers = Servers::start(&dir);
        // every configuration over every built batch, across the cores
        let t = Instant::now();
        let jobs: Vec<(usize, usize)> = (0..built.len()).flat_map(|b| (0..CONFIGS.len()).map(move |c| (b, c))).collect();
        let next = AtomicUsize::new(0);
        let results: Mutex<BTreeMap<(usize, usize), Vec<Seen>>> = Mutex::new(BTreeMap::new());
        let workers = cores.clamp(1, (budget().total / budget().each).max(1) as usize);
        let done = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..workers {
                s.spawn(|| {
                    loop {
                        let j = next.fetch_add(1, Ordering::SeqCst);
                        if j >= jobs.len() {
                            break;
                        }
                        let (b, c) = jobs[j];
                        let batch = &built[b];
                        let config = &CONFIGS[c];
                        let fresh: Vec<bool> = batch.members.iter().map(|(_, c)| c.draws()).collect();
                        let live = batch.members.iter().any(|(_, c)| c.live);
                        let binary = if config.compiled { Some(batch.binary.as_path()) } else { None };
                        let seen = if batch.members.first().is_some_and(|(_, c)| c.serve.is_some()) {
                            web::serve_batch(&exe, &batch.dir, &batch.file, binary, config, &batch.members)
                        } else {
                            let own = batch.members.first().is_some_and(|(_, c)| c.own_dir);
                            run_batch(&exe, &batch.dir, &batch.file, binary, config, &fresh, live, own)
                        };
                        if let Ok(mut r) = results.lock() {
                            r.insert((b, c), seen);
                        }
                        let d = done.fetch_add(1, Ordering::SeqCst) + 1;
                        print!("\rran {} of {} batch runs", d, jobs.len());
                        let _ = std::io::Write::flush(&mut std::io::stdout());
                    }
                });
            }
        });
        println!("\rran {} batch runs in {:.1}s{}", jobs.len(), t.elapsed().as_secs_f64(), " ".repeat(20));
        drop(servers);
        let results = results.into_inner().unwrap_or_default();
        let mut agreed = 0;
        // what each case that agreed showed, for the answers it knows
        let mut agreed_out: HashMap<String, String> = HashMap::new();
        let mut agreed_seen: HashMap<String, Seen> = HashMap::new();
        for (b, batch) in built.iter().enumerate() {
            for (k, (i, case)) in batch.members.iter().enumerate() {
                let per: Vec<(&str, Seen)> = CONFIGS
                    .iter()
                    .enumerate()
                    .map(|(c, config)| (config.name, results.get(&(b, c)).and_then(|s| s.get(k)).cloned().unwrap_or(Seen { out: String::new(), end: "not run".into() })))
                    .collect();
                let first = &per[0].1;
                let differs: Vec<&str> = per.iter().filter(|(_, s)| s != first).map(|(n, _)| *n).collect();
                let bad = per.iter().find(|(_, s)| is_bug(&s.out) || is_bug(&s.end) || ["crash", "hang", "memory", "output", "the batch"].iter().any(|w| s.end.starts_with(w)));
                if !differs.is_empty() {
                    let mut detail = format!("{} differ from interpreted", differs.join(", "));
                    for (n, s) in &per {
                        if *n == "interpreted" || differs.contains(n) {
                            detail.push_str(&format!("\n      {:<11} {}{}", n, s.out.trim().replace('\n', " | "), if s.end.is_empty() { String::new() } else { format!(" [{}]", s.end) }));
                        }
                    }
                    findings.push(Finding { id: case.id.clone(), code: case.code(), class: "disagree", detail });
                } else if let Some((n, s)) = bad {
                    let class = if s.end.starts_with("hang") {
                        "hang"
                    } else if s.end.starts_with("memory") {
                        "memory"
                    } else if s.end.starts_with("output") {
                        "output"
                    } else if s.end.starts_with("crash") {
                        "crash"
                    } else {
                        // (a bug in Crow said as the program ran, not as it lowered)
                        "internal-run"
                    };
                    findings.push(Finding { id: case.id.clone(), code: case.code(), class, detail: format!("{}: {}{}", n, s.out.trim().replace('\n', " | "), if s.end.is_empty() { String::new() } else { format!(" [{}]", s.end) }) });
                } else {
                    agreed += 1;
                    let s = &per[0].1;
                    agreed_out.insert(case.id.clone(), if s.end.is_empty() { s.out.clone() } else { format!("{}[{}]\n", s.out, s.end) });
                    agreed_seen.insert(case.id.clone(), s.clone());
                }
                let _ = i;
            }
        }
        println!("ran: {} agreed in every configuration ({})", agreed, CONFIGS.iter().map(|c| c.name).collect::<Vec<_>>().join(", "));
        // the right answers the cases know: what hands a value back — a
        // function, a record, a list, a map, a loop, a variable, a branch,
        // two of them in a row — shows it as it shows alone
        let (mut answered, mut wrong) = (0, 0);
        for case in cases.iter().filter(|c| !c.expect.is_empty()) {
            let Some(got) = agreed_out.get(&case.id) else { continue };
            // (each shown alone, without the `0` its case function's return
            // logged after it)
            let alone = |n: &String| -> Option<String> {
                let s = agreed_out.get(&format!("shape/shown/{}", n))?;
                Some(match s.strip_suffix("0\n") {
                    Some(rest) if rest.is_empty() || rest.ends_with('\n') => rest.to_string(),
                    _ => s.clone(),
                })
            };
            let expected: Option<String> = case.expect.iter().map(alone).collect();
            let Some(expected) = expected else { continue };
            answered += 1;
            let (g, e) = (comparable(got), comparable(&expected));
            if !g.starts_with(&e) {
                wrong += 1;
                findings.push(Finding {
                    id: case.id.clone(),
                    code: case.code(),
                    class: "wrong",
                    detail: format!("shows {} where the value alone shows {}", g.trim_end().replace('\n', " | "), e.trim_end().replace('\n', " | ")),
                });
            }
        }
        println!("answers: {} cases held to the right answer, {} wrong", answered, wrong);
        // the spec's answers: tests/self/reference.py computes what a case
        // computes — a builtin, an operator, an index read or written, a
        // value shown — from the spec's words, without Crow's runtime, and
        // names each case that answers otherwise
        let referred = consult_reference(&dir, &cases, &agreed_seen, &refused_by, &mut findings);
        println!("reference: {} wrong", referred.wrong);
        if !referred.said.is_empty() {
            println!("  {}", referred.said);
        }
        // a row whose cases ran, none of them answered (nor waiting on a
        // library this machine lacks), is held to agreement alone
        if referred.ran {
            // (a response form, and a way a route reads its request, are
            // entries of their own: `web.json`, `web.read.query`)
            let row_of = |id: &str| -> Option<String> {
                let mut parts = id.splitn(3, '/');
                match parts.next() {
                    Some("row" | "pair") => parts.next().map(str::to_string),
                    Some("route") => parts.next().map(|f| format!("web.{}", f)),
                    Some("route-read") => parts.next().map(|n| format!("web.read.{}", n)),
                    // (an element, an attribute, a boolean one, a layout word)
                    Some("view") => parts.next().map(|e| format!("ui.{}", e)),
                    Some("view-attr") => parts.next().map(|a| format!("ui.attr.{}", a)),
                    Some("view-class") => Some("ui.attr.class".to_string()),
                    Some("view-bool") => parts.next().map(|b| format!("ui.bool.{}", b)),
                    Some("view-layout") => parts.next().map(|a| format!("ui.layout.{}", a)),
                    Some("state") => parts.next().map(|n| format!("ui.state.{}", n)),
                    _ => None,
                }
            };
            let mut ran: BTreeMap<String, (bool, bool)> = BTreeMap::new();
            // (a live family answers every row its cases call: the socket's
            // and the mailer's rows, against the check's own servers)
            let live_rows = |id: &str| -> &'static [&'static str] {
                if id.starts_with("ws-live/") {
                    &["web.ws.socket", "web.ws.send_text", "web.ws.recv_text", "web.ws.recv_for", "web.ws.close_socket"]
                } else if id.starts_with("mail-live/") {
                    &["web.mail.smtp", "web.mail.mail"]
                } else if let Some(name) = id.strip_prefix("llm/").or_else(|| id.strip_prefix("llm-live/")) {
                    // (the llm family: the rows each case calls)
                    match name {
                        "ask" => &["llm.ask"],
                        "ask-more" | "no-talk" => &["llm.ask", "llm.ask_more"],
                        "said" | "said-no-reply" => &["llm.ask", "llm.said"],
                        "told" | "told-no-call" => &["llm.ask", "llm.said", "llm.told"],
                        "chat" | "chat-options" | "anthropic" | "refused" | "no-json" | "nobody" => &["llm.reach", "llm.ask", "llm.chat"],
                        "tool" | "anthropic-tool" => &["llm.reach", "llm.ask", "llm.chat", "llm.said", "llm.told"],
                        "follow" | "follow-tool" | "anthropic-follow" | "anthropic-follow-tool" => &["llm.reach", "llm.ask", "llm.follow", "llm.next_piece", "llm.whole"],
                        "embed" | "anthropic-embed" => &["llm.reach", "llm.embed"],
                        _ if name.starts_with("reach") => &["llm.reach"],
                        _ => &[],
                    }
                } else if let Some(name) = id.strip_prefix("auth/") {
                    // (the auth family: a token signed, read, or both)
                    match name {
                        "sign" | "record" | "no-claims" | "short-secret" | "not-claims" => &["web.auth.sign"],
                        "not-a-token" | "one-part" | "none-alg" | "known" => &["web.auth.verify"],
                        _ => &["web.auth.sign", "web.auth.verify"],
                    }
                } else if let Some(name) = id.strip_prefix("io/") {
                    // (the io family: the rows each case calls)
                    match name {
                        "read" | "read-missing" => &["io.read"],
                        "lines" => &["io.lines"],
                        "exists" => &["io.exists"],
                        "ls" => &["io.ls"],
                        "write-read" | "write-over" => &["io.write"],
                        "mkdir" | "mkdir-twice" => &["io.mkdir"],
                        "rm" | "rm-missing" => &["io.rm"],
                        "mv" => &["io.mv"],
                        "cp" => &["io.cp"],
                        "csv" => &["io.csv"],
                        _ => &[],
                    }
                } else if let Some(name) = id.strip_prefix("draw/") {
                    // (the draws family: each distribution's row, and the seed)
                    match name {
                        "randn" => &["math.randn", "math.seed"],
                        "normal" => &["math.normal"],
                        "uniform" => &["math.uniform"],
                        "exponential" => &["math.exponential"],
                        "poisson" => &["math.poisson"],
                        "bernoulli" => &["math.bernoulli"],
                        "beta" => &["math.beta_dist"],
                        "gamma" => &["math.gamma_dist"],
                        "rand" => &["core.rand"],
                        "rand-n" => &["core.rand_int"],
                        "rand-between" => &["core.rand_between"],
                        "shuffle" => &["math.shuffle"],
                        "choice" => &["math.choice"],
                        "permutation" => &["math.permutation"],
                        _ => &[],
                    }
                } else {
                    &[]
                }
            };
            for id in agreed_seen.keys() {
                let rows: Vec<String> = row_of(id).into_iter().chain(live_rows(id).iter().map(|r| r.to_string())).collect();
                for row in rows {
                    let e = ran.entry(row).or_default();
                    e.0 |= referred.answered.contains(id);
                    e.1 |= referred.needs.contains_key(id);
                }
            }
            agreement_now = Some(ran.into_iter().filter(|(_, (answered, needs))| !answered && !needs).map(|(row, _)| {
                let why = agreement_reason(&row);
                (row, why)
            }).collect());
        }
        // a live case against a server the check started answers without
        // failing (agreeing alike is not enough: two modes that cannot
        // reach the server agree); one whose server did not start leaves
        // its driver's rows not called, by the machine
        let served: HashMap<&str, &str> = SERVED.get().map(|v| v.iter().map(|(k, x)| (k.as_str(), x.as_str())).collect()).unwrap_or_default();
        for case in cases.iter().filter(|c| c.live) {
            let (driver, port) = if case.id.starts_with("ws-live/") {
                ("web.ws", "CROW_SELF_WS")
            } else if case.id.starts_with("mail-live/") {
                ("web.mail", "CROW_SELF_SMTP")
            } else if case.id.starts_with("llm-live/") {
                ("llm", "CROW_SELF_LLM")
            } else if case.id.contains("postgres") {
                ("db.postgres", "CROW_SELF_PG")
            } else {
                ("db.clickhouse", "CROW_SELF_CH")
            };
            if !served.contains_key(port) {
                for row in ROWS.iter().filter(|r| r.module == driver && !r.name.ends_with("_cancel")) {
                    if !not_called.iter().any(|n| n.id == row.id) {
                        not_called.push(NotCalled { environment: true, ..NotCalled::new(row.id, "the check's server for it did not start here") });
                    }
                }
                continue;
            }
            let Some(s) = agreed_seen.get(&case.id) else { continue };
            let failed = !s.end.is_empty() || s.out.lines().any(|l| l.starts_with("error:"));
            if failed {
                findings.push(Finding {
                    id: case.id.clone(),
                    code: case.code(),
                    class: "live",
                    detail: format!("failed against the server the check started: {}{}", s.out.trim().replace('\n', " | "), if s.end.is_empty() { String::new() } else { format!(" [{}]", s.end) }),
                });
            }
        }
        // the standard modules' own tests, under each interpreted
        // configuration (the test-block family)
        if only.as_deref().is_none_or(|w| "tests/".contains(w) || w.starts_with("tests")) {
            let (found, ran) = module_tests(&exe);
            println!("tests: {} runs of the standard modules' and the packages' own tests, {} findings", ran, found.len());
            findings.extend(found);
        }
        // (the builders' caches stay for the next run, its batches' programs
        // among them — a batch whose source is the same is not built again —
        // and go whole where they pass a budget: they had grown past 240 GB
        // and filled the disk; `--clean` removes them)
        if clean || cache_size(&build_dir) > CACHE_BUDGET {
            let _ = std::fs::remove_dir_all(&build_dir);
        }
    }

    // the report, and the list of the known
    findings.sort_by(|a, b| (a.class, &a.id).cmp(&(b.class, &b.id)));
    let mut seen_ids = HashSet::new();
    findings.retain(|f| seen_ids.insert((f.class, f.id.clone())));
    let indent = |text: &str, by: &str| text.lines().map(|l| format!("{}{}", by, l)).collect::<Vec<_>>().join("\n");
    let report: String = findings.iter().map(|f| format!("{:<14} {}\n{}\n      → {}\n", f.class, f.id, indent(&f.code, "      | "), f.detail.replace('\n', "\n        "))).collect();
    let _ = std::fs::write(dir.join("findings.txt"), &report);
    // the findings by what they share: one line per defect, its count and
    // the cases that show it first
    let mut groups: BTreeMap<(String, String), Vec<&Finding>> = BTreeMap::new();
    for f in &findings {
        groups.entry((f.class.to_string(), group_key(f))).or_default().push(f);
    }
    let mut grouped: Vec<(&(String, String), &Vec<&Finding>)> = groups.iter().collect();
    grouped.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
    let summary: String = grouped
        .iter()
        .map(|((class, key), members)| {
            let first = members[0];
            format!(
                "{:>5}  {:<10} {}\n       for example {}:\n{}\n       and {}\n",
                members.len(),
                class,
                key,
                first.id,
                indent(&first.code, "         | "),
                members.iter().skip(1).take(3).map(|f| f.id.as_str()).collect::<Vec<_>>().join(", ")
            )
        })
        .collect();
    let _ = std::fs::write(dir.join("summary.txt"), &summary);
    let mut by_class: BTreeMap<&str, usize> = BTreeMap::new();
    for f in &findings {
        *by_class.entry(f.class).or_default() += 1;
    }
    let _ = std::fs::write(dir.join("unexercised.txt"), not_called.iter().map(|n| format!("{} — {}\n", n.id, n.why)).collect::<String>());
    // the builtins no case calls: the machine's lacks noted, the rest held
    let environment: Vec<&NotCalled> = not_called.iter().filter(|n| n.environment).collect();
    let not_called_now: BTreeMap<String, String> = not_called.iter().filter(|n| !n.environment).map(|n| (n.id.clone(), n.why.clone())).collect();
    println!("\nnot called: {} builtins (listed in {}){}", not_called.len(), dir.join("unexercised.txt").display(), if environment.is_empty() { String::new() } else { format!(", {} for what this machine lacks", environment.len()) });
    println!("findings: {} ({}) in {} groups — every one in {}, the groups in {}", findings.len(), by_class.iter().map(|(c, n)| format!("{} {}", n, c)).collect::<Vec<_>>().join(", "), groups.len(), dir.join("findings.txt").display(), dir.join("summary.txt").display());
    // (each list is held where this run could make it whole: every case,
    // and for the reference's, a run that asked it)
    let whole = only.is_none();
    let lists: Vec<(&Listed, Option<&BTreeMap<String, String>>)> = vec![
        (&NOT_CALLED, whole.then_some(&not_called_now)),
        (&AGREEMENT_ONLY, if whole { agreement_now.as_ref() } else { None }),
        (&FORMS, whole.then_some(&forms_now)),
    ];
    if bless {
        if only.is_some() || lower_only {
            eprintln!("--bless writes the whole list: run it without --only and --lower-only");
            return 1;
        }
        for (list, now) in &lists {
            if let Some(now) = now {
                match list.write(now) {
                    Ok(()) => println!("wrote {} ({} entries)", list.path, now.len()),
                    Err(e) => {
                        eprintln!("cannot write {}: {}", list.path, e);
                        return 1;
                    }
                }
            }
        }
        return match write_known(&findings) {
            Ok(()) => {
                println!("wrote {} ({} findings known)", known_path().display(), findings.len());
                0
            }
            Err(e) => {
                eprintln!("cannot write {}: {}", known_path().display(), e);
                1
            }
        };
    }
    // the lists: a new entry or a gone one fails as a finding does
    let mut listed_off = 0;
    for (list, now) in &lists {
        let Some(now) = now else { continue };
        let (new, gone) = list.held(now);
        for (id, why) in &new {
            println!("  NEW   {:<14} {} — {} (list it in {} with why, or make it reached)", "listed", id, why, list.path);
        }
        for id in &gone {
            println!("  GONE  {:<14} {} — strike it from {}", "listed", id, list.path);
        }
        listed_off += new.len() + gone.len();
    }
    let known = read_known();
    let now: BTreeSet<(String, String)> = findings.iter().map(|f| (f.class.to_string(), f.id.clone())).collect();
    let new: Vec<&Finding> = findings.iter().filter(|f| !known.contains(&(f.class.to_string(), f.id.clone()))).collect();
    // (a known finding is gone only where this run could have found it)
    let ran: HashSet<&str> = cases.iter().map(|c| c.id.as_str()).collect();
    // (with --lower-only, only what the lowering finds could have gone)
    let could_find = |c: &str| !lower_only || matches!(c, "internal" | "message-place" | "message-names" | "message-jargon");
    let gone: Vec<&(String, String)> = known.iter().filter(|(c, i)| ran.contains(i.as_str()) && could_find(c) && !now.contains(&(c.clone(), i.clone()))).collect();
    for f in &new {
        println!("  NEW   {:<14} {}\n{}\n        → {}", f.class, f.id, indent(&f.code, "        | "), f.detail.replace('\n', "\n          "));
    }
    for (c, i) in &gone {
        println!("  GONE  {:<14} {} — fixed: strike it from {}", c, i, known_path().display());
    }
    println!("\n{} new, {} known, {} gone, {} off the lists ({:.1}s)", new.len(), now.len() - new.len(), gone.len(), listed_off, start.elapsed().as_secs_f64());
    if new.is_empty() && gone.is_empty() && listed_off == 0 {
        0
    } else {
        1
    }
}

// ── the guards: nothing new slips past unseen ───────────────────────

/// A list a run is held to, one entry a line — an id, a tab, why — that
/// changes only by `--bless`: an entry the run makes that the list lacks
/// fails the check, and so does a listed one the run no longer makes. So a
/// builtin, a module or a form of the language added later is tested by
/// the cases the compiler's own tables make, or the gate fails and names
/// it.
struct Listed {
    path: &'static str,
    header: &'static str,
}

/// The builtins no case calls.
const NOT_CALLED: Listed = Listed {
    path: "tests/self/not-called.txt",
    header: "# crow check --self: the builtins no case calls, one per line (the row, a tab, why).\n# A builtin no case reaches fails the check until it is listed here with why, and a\n# listed one the cases reach fails it until it is struck (--bless rewrites the list).\n",
};

/// The builtins whose cases run, none of which tests/self/reference.py
/// answers: they agree across the configurations and are held to no
/// right answer.
const AGREEMENT_ONLY: Listed = Listed {
    path: "tests/self/agreement-only.txt",
    header: "# crow check --self: the builtins whose cases run and agree, none of which\n# tests/self/reference.py answers, one per line (the row, a tab, why). A new builtin\n# gets its answer in the reference, or its line here; one the reference comes to\n# answer fails the check until it is struck (--bless rewrites the list).\n",
};

/// The forms of the language — the AST's statements, expressions and
/// operators, read from src/ast.rs — no case reaches.
const FORMS: Listed = Listed {
    path: "tests/self/forms.txt",
    header: "# crow check --self: the forms of the language (src/ast.rs's statements,\n# expressions and operators) no case reaches, one per line (the form, a tab, why).\n# A new form gets a case, or its line here (--bless rewrites the list).\n",
};

impl Listed {
    fn read(&self) -> BTreeMap<String, String> {
        std::fs::read_to_string(self.path)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(|l| match l.split_once('\t') {
                Some((id, why)) => (id.to_string(), why.to_string()),
                None => (l.to_string(), String::new()),
            })
            .collect()
    }

    /// The run's entries written, each keeping the reason a person gave it.
    fn write(&self, now: &BTreeMap<String, String>) -> std::io::Result<()> {
        let old = self.read();
        let lines: Vec<String> = now.iter().map(|(id, why)| format!("{}\t{}", id, old.get(id).filter(|w| !w.is_empty()).unwrap_or(why))).collect();
        std::fs::write(self.path, format!("{}{}\n", self.header, lines.join("\n")))
    }

    /// What the run makes that the list lacks, and what the list holds that
    /// the run no longer makes.
    fn held(&self, now: &BTreeMap<String, String>) -> (Vec<(String, String)>, Vec<String>) {
        let listed = self.read();
        let new = now.iter().filter(|(id, _)| !listed.contains_key(*id)).map(|(i, w)| (i.clone(), w.clone())).collect();
        let gone = listed.keys().filter(|id| !now.contains_key(*id)).cloned().collect();
        (new, gone)
    }
}

/// A builtin no case calls, and why — `environment` where the machine
/// lacks what it needs (a scratch cluster's `initdb`): noted, never failing.
struct NotCalled {
    id: String,
    why: String,
    environment: bool,
}

impl NotCalled {
    fn new(id: impl Into<String>, why: impl Into<String>) -> NotCalled {
        NotCalled { id: id.into(), why: why.into(), environment: false }
    }
}

/// Why a builtin is held to agreement alone, by what it is, for a line
/// `--bless` writes (a person may write a better one; `write` keeps it).
fn agreement_reason(row: &str) -> String {
    let name = row.rsplit('.').next().unwrap_or(row);
    let module = row.rsplit_once('.').map(|(m, _)| m).unwrap_or("");
    let drawn = ["rand", "randn", "randint", "uniform", "normal", "gamma_dist", "beta_dist", "poisson", "exponential", "bernoulli", "choice", "shuffle", "dropout"];
    if drawn.iter().any(|d| name.starts_with(d)) {
        return "random draws: seeded, and no reference draws the same numbers (a distribution's moments are the plan)".to_string();
    }
    match module {
        "web" => "a response no reference answers yet".to_string(),
        "ui.layout" => "the layout table's CSS: no reference models it yet (the spec's layout vocabulary is the plan)".to_string(),
        "ui" | "ui.attr" | "ui.bool" => "an element no reference reads back yet".to_string(),
        "web.read" => "a way a route reads its request no reference answers yet".to_string(),
        "llm" => "a model's reply: the empty replay answers here; tests/llm_server.rs holds the module to a server speaking both wire formats, in both modes".to_string(),
        "chart" => "an indicator: no reference yet (pandas's rolling and ewm are the plan)".to_string(),
        "sys" => "the standard library's own layer: bytes, streams and protocols no reference models yet".to_string(),
        "frame" => "frames: held to pandas by tests/parity/185, no reference per case yet".to_string(),
        "doc" => "Markdown rendered: no reference renderer".to_string(),
        m if m.starts_with("web") => "a response or a protocol no reference models yet".to_string(),
        _ => "no reference yet: add its answer to tests/self/reference.py, or say here why it agrees only".to_string(),
    }
}

/// The variants of one of the AST's enums, read from the source this
/// binary was built from: a variant added there is a form the cases must
/// reach, or `forms.txt` must name.
fn ast_variants(name: &str) -> Vec<String> {
    const AST: &str = include_str!("../ast.rs");
    let Some(start) = AST.find(&format!("pub enum {} {{", name)) else { return Vec::new() };
    let body = &AST[start..];
    let end = body.find("\n}").unwrap_or(body.len());
    body[..end]
        .lines()
        .skip(1)
        .filter_map(|l| {
            // (a variant starts its line four spaces in, capitalized; its
            // fields and its comments sit deeper or start with a slash)
            let t = l.strip_prefix("    ")?;
            let word: String = t.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            word.chars().next().filter(|c| c.is_uppercase()).map(|_| word)
        })
        .collect()
}

/// Every operator a program writes, as it writes it: the AST's own list,
/// each spelled by a match that names every one (a new operator does not
/// compile until it is spelled, and its cases are then made).
fn operators() -> Vec<&'static str> {
    ast_variants("BinOp").into_iter().filter_map(|n| serde_json::from_value::<crate::ast::BinOp>(serde_json::Value::String(n)).ok()).map(|op| spelled(&op)).collect()
}

fn spelled(op: &crate::ast::BinOp) -> &'static str {
    use crate::ast::BinOp as B;
    match op {
        B::Add => "+",
        B::Sub => "-",
        B::Mul => "*",
        B::Div => "/",
        B::Mod => "%",
        B::Pow => "^",
        B::ElemMul => ".*",
        B::ElemDiv => "./",
        B::ElemPow => ".^",
        B::Eq => "eq",
        B::Ne => "ne",
        B::Gt => "gt",
        B::Lt => "lt",
        B::Gte => "gte",
        B::Lte => "lte",
        B::And => "and",
        B::Or => "or",
    }
}

/// The forms a program's statements reach, by name (`Stmt::For`,
/// `Expr::Call`, `BinOp::Add`).
fn forms_in(stmts: &[crate::ast::Stmt], seen: &mut BTreeSet<String>) {
    for st in stmts {
        form_of_stmt(st, seen);
    }
}

fn form_of_stmt(st: &crate::ast::Stmt, seen: &mut BTreeSet<String>) {
    use crate::ast::Stmt as S;
    let name = match st {
        S::Assign { value, .. } => {
            form_of_expr(value, seen);
            "Assign"
        }
        S::MultiAssign { values, .. } => {
            values.iter().for_each(|v| form_of_expr(v, seen));
            "MultiAssign"
        }
        S::FnDecl { defaults, body, .. } => {
            defaults.iter().flatten().for_each(|d| form_of_expr(d, seen));
            forms_in(body, seen);
            "FnDecl"
        }
        S::Return(value) => {
            value.iter().for_each(|v| form_of_expr(v, seen));
            "Return"
        }
        S::ExprStmt(e) => {
            form_of_expr(e, seen);
            "ExprStmt"
        }
        S::If { condition, then_body, else_body } => {
            form_of_expr(condition, seen);
            forms_in(then_body, seen);
            else_body.iter().for_each(|b| forms_in(b, seen));
            "If"
        }
        S::For { iter, body, .. } => {
            form_of_expr(iter, seen);
            forms_in(body, seen);
            "For"
        }
        S::While { condition, body } => {
            form_of_expr(condition, seen);
            forms_in(body, seen);
            "While"
        }
        S::StructDecl { .. } => "StructDecl",
        S::FieldAssign { object, value, .. } => {
            form_of_expr(object, seen);
            form_of_expr(value, seen);
            "FieldAssign"
        }
        S::IndexAssign { object, index, value } => {
            form_of_expr(object, seen);
            form_of_expr(index, seen);
            form_of_expr(value, seen);
            "IndexAssign"
        }
        S::Derive { body, .. } => {
            forms_in(body, seen);
            "Derive"
        }
        S::TestBlock { body, .. } => {
            forms_in(body, seen);
            "TestBlock"
        }
        S::Use { .. } => "Use",
        S::Break => "Break",
        S::Continue => "Continue",
        S::Line(_) => "Line",
    };
    seen.insert(format!("Stmt::{}", name));
}

fn form_of_expr(e: &crate::ast::Expr, seen: &mut BTreeSet<String>) {
    use crate::ast::Expr as E;
    let name = match e {
        E::Int(_) => "Int",
        E::Float(_) => "Float",
        E::Str(_) => "Str",
        E::Bool(_) => "Bool",
        E::Ident(_) => "Ident",
        E::BinOp { left, op, right } => {
            seen.insert(format!("BinOp::{:?}", op));
            form_of_expr(left, seen);
            form_of_expr(right, seen);
            "BinOp"
        }
        E::UnaryOp { op, expr } => {
            seen.insert(format!("UnaryOp::{:?}", op));
            form_of_expr(expr, seen);
            "UnaryOp"
        }
        E::Call { callee, args } => {
            form_of_expr(callee, seen);
            args.iter().for_each(|a| form_of_expr(a, seen));
            "Call"
        }
        E::FieldAccess { object, .. } => {
            form_of_expr(object, seen);
            "FieldAccess"
        }
        E::Array(items) => {
            items.iter().for_each(|i| form_of_expr(i, seen));
            "Array"
        }
        E::Map(pairs) => {
            pairs.iter().for_each(|(k, v)| {
                form_of_expr(k, seen);
                form_of_expr(v, seen)
            });
            "Map"
        }
        E::Named { value, .. } => {
            form_of_expr(value, seen);
            "Named"
        }
        E::ForExpr { iter, body, .. } => {
            form_of_expr(iter, seen);
            forms_in(body, seen);
            "ForExpr"
        }
    };
    seen.insert(format!("Expr::{}", name));
}

/// Every form of the language, by name: the AST's statements, expressions
/// and operators.
fn every_form() -> BTreeSet<String> {
    let mut all = BTreeSet::new();
    for (prefix, enum_name) in [("Stmt", "Stmt"), ("Expr", "Expr"), ("BinOp", "BinOp"), ("UnaryOp", "UnaryOp")] {
        for v in ast_variants(enum_name) {
            all.insert(format!("{}::{}", prefix, v));
        }
    }
    all
}

/// The forms the programs of these cases reach, each parsed as `crow run`
/// parses it, across the cores.
/// The draws family: each distribution drawn 20,000 times from a fixed
/// seed, and what the draws show — their mean, their spread, their least
/// and greatest — logged for the reference to hold to the distribution's
/// (within four standard errors: the seed fixes the draws, so the check
/// is never one in a thousand wrong); a shuffle, a choice and a
/// permutation as what they must be (the same items, distinct, in range).
fn draw_cases() -> Vec<Case> {
    let shown = "log[mean[x]]\nlog[std[x]]\nlog[min[x]]\nlog[max[x]]";
    let table: Vec<(&str, String)> = vec![
        ("randn", format!("x = randn[20000]\n{}", shown)),
        ("normal", format!("x = normal[3.0, 2.0, 20000]\n{}", shown)),
        ("uniform", format!("x = uniform[2.0, 5.0, 20000]\n{}", shown)),
        ("exponential", format!("x = exponential[2.0, 20000]\n{}", shown)),
        ("poisson", format!("x = poisson[3.0, 20000]\n{}\nlog[sum[(x - floor[x]) eq 0.0]]", shown)),
        ("bernoulli", format!("x = bernoulli[0.3, 20000]\n{}\nlog[sum[(x - floor[x]) eq 0.0]]", shown)),
        ("beta", format!("x = beta_dist[2.0, 5.0, 20000]\n{}", shown)),
        // (shape 2, rate 4: the mean 0.5, the spread √2 / 4)
        ("gamma", format!("x = gamma_dist[2.0, 4.0, 20000]\n{}", shown)),
        ("rand", "xs = for i in range[20000]\n    rand[]\nx = tensor[xs]\nlog[mean[x]]\nlog[std[x]]\nlog[min[x]]\nlog[max[x]]".to_string()),
        ("rand-n", "xs = for i in range[20000]\n    rand[6]\nx = tensor[xs]\nlog[mean[x]]\nlog[std[x]]\nlog[min[x]]\nlog[max[x]]".to_string()),
        ("rand-between", "xs = for i in range[20000]\n    rand[-3, 3]\nx = tensor[xs]\nlog[mean[x]]\nlog[std[x]]\nlog[min[x]]\nlog[max[x]]".to_string()),
        ("shuffle", "xs = [5.0, 3.0, 9.0, 1.0, 7.0]\ny = shuffle[xs]\nlog[sort[y] eq sort[xs]]\nlog[len[y]]".to_string()),
        ("choice", "xs = [5.0, 3.0, 9.0, 1.0, 7.0]\ny = choice[xs, 3]\nlog[len[y]]\nlog[len[unique[y]]]\nlog[all[y, self_in_five]]".to_string()),
        ("permutation", "y = permutation[6]\nlog[sort[y]]".to_string()),
    ];
    table
        .into_iter()
        .map(|(name, body)| {
            let mut lines = vec!["seed[7]".to_string()];
            lines.extend(body.lines().map(str::to_string));
            let mut c = Case::new(format!("draw/{}", name), &[], lines).referring(format!("draw:{}", name), &[]);
            c.imports.insert("math");
            c.imports.insert("math.stats");
            c
        })
        .collect()
}

/// The grad family: a function of numbers differentiated at a point —
/// each differentiable kind the spec lists (arithmetic, a power, sqrt, ln,
/// exp, sin, tanh, erf, abs, floor, ceil, round), a helper the function
/// calls, a loop that accumulates, a function of two numbers and functions
/// of a tensor — its derivative held to the analytic one.
fn grad_cases() -> Vec<Case> {
    let table: &[(&str, &str, &str)] = &[
        ("square", "fn self_f_@N[x]\n    return x * x", "log[grad[self_f_@N][3.0]]"),
        ("poly", "fn self_f_@N[x]\n    return x ^ 3 - 2 * x", "log[grad[self_f_@N][2.0]]"),
        ("quotient", "fn self_f_@N[x]\n    return 1.0 / x", "log[grad[self_f_@N][2.0]]"),
        ("sqrt", "fn self_f_@N[x]\n    return sqrt[x]", "log[grad[self_f_@N][4.0]]"),
        ("ln", "fn self_f_@N[x]\n    return ln[x]", "log[grad[self_f_@N][2.0]]"),
        ("sin-exp", "fn self_f_@N[x]\n    return sin[x] * exp[x]", "log[grad[self_f_@N][0.5]]"),
        ("tanh", "fn self_f_@N[x]\n    return tanh[x]", "log[grad[self_f_@N][0.0]]"),
        ("erf", "fn self_f_@N[x]\n    return erf[x]", "log[grad[self_f_@N][0.0]]"),
        ("abs", "fn self_f_@N[x]\n    return abs[x]", "log[grad[self_f_@N][-2.0]]"),
        ("floor", "fn self_f_@N[x]\n    return floor[x] + x", "log[grad[self_f_@N][2.5]]"),
        ("helper", "fn self_g_@N[x]\n    return x * x\nfn self_f_@N[x]\n    return self_g_@N[x] + 1.0", "log[grad[self_f_@N][3.0]]"),
        ("loop", "fn self_f_@N[x]\n    s = 0.0\n    k = 0\n    while k lt 3\n        s = s + x ^ k\n        k = k + 1\n    return s", "log[grad[self_f_@N][2.0]]"),
        ("two", "fn self_f_@N[x, y]\n    return x * y + y ^ 2", "log[grad[self_f_@N][2.0, 3.0]]"),
        // (the growing whole number's forms `floor[x]` of a float of any
        // size takes: the name the program wrote answers a dual)
        ("ceil-round", "fn self_f_@N[x]\n    return ceil[x] * x + round[x] - abs[x]", "log[grad[self_f_@N][2.5]]"),
        // a function of a tensor, traced
        ("tensor", "fn self_f_@N[t]\n    return sum[t .* t]", "log[grad[self_f_@N][tensor[[1.0, 2.0, 3.0]]]]"),
        ("tensor-floor", "fn self_f_@N[t]\n    s = sum[t]\n    return floor[s] + s * s", "log[grad[self_f_@N][tensor[[1.5, 2.0]]]]"),
    ];
    table
        .iter()
        .map(|(name, top, body)| {
            let mut c = Case::new(format!("grad/{}", name), &[], vec![body.to_string()]).referring(format!("grad:{}", name), &[]);
            c.top = top.lines().map(str::to_string).collect();
            c.imports.insert("math");
            c
        })
        .collect()
}

/// The tensors the gpu family hands its cases in place of the catalog's
/// vector and matrix: the same numbers, made on the GPU — only a tensor a
/// constructor makes (`linspace`, `full`, `eye`, a draw) is placed there,
/// and an operation on tensors placed there runs there, in f32 — so the
/// reference answers each case as it answers its twin, within the f32 the
/// device computes in. (Each step of the vector is exact in f32: `[0, 1,
/// 2]` squared, scaled and shifted to `[3, 1, 2]`.)
const GPU_VECTOR: &str = "(linspace[0.0, 2.0, 3] .* linspace[0.0, 2.0, 3] * 1.5 - linspace[0.0, 2.0, 3] * 3.5 + 3.0)";
const GPU_MATRIX: &str = "reshape[linspace[1.0, 4.0, 4], 2, 2]";

/// The gpu family: every case of the rows, the operators and the forms
/// (shapes, positions, writes) that is handed the catalog's vector or
/// matrix, run again with the GPU on (`gpu[]` first, in batches of their
/// own) and those tensors on the device (`GPU_VECTOR`, `GPU_MATRIX`):
/// every configuration still agrees, and the reference holds each answer
/// to its twin's within f32 (`gpu/<twin's id>`). Where no GPU answers,
/// every tensor stays on the CPU and the twins are their twins' again.
fn gpu_cases(cases: &[Case], all: &[Val]) -> Vec<Case> {
    let (vector, matrix) = (&val(all, "vector").expr, &val(all, "matrix").expr);
    cases
        .iter()
        .filter(|c| {
            let family = c.id.split('/').next().unwrap_or("");
            matches!(family, "row" | "op" | "shape" | "index" | "index-held" | "write")
                && c.refer.as_ref().is_some_and(|(_, args)| args.iter().any(|a| a == "vector" || a == "matrix"))
        })
        .map(|c| {
            let mut twin = c.clone();
            twin.id = format!("gpu/{}", c.id);
            twin.body = c.body.iter().map(|l| l.replace(matrix.as_str(), GPU_MATRIX).replace(vector.as_str(), GPU_VECTOR)).collect();
            twin.imports.insert("math");
            twin.gpu = true;
            twin
        })
        .collect()
}

/// The io family: the file rows, each case in a directory of its own
/// holding the run's files (`c<N>/`), so what one writes no other reads —
/// read, lines, csv, exists, ls, write, mkdir, rm, mv and cp, each held to
/// what the spec says (Files).
fn io_cases() -> Vec<Case> {
    let table: &[(&str, &str)] = &[
        ("read", "log[read['c@N/self_text.txt']]"),
        ("read-missing", "r = read['c@N/nope.txt']\nlog[r.ok]\nlog[r.kind]"),
        ("lines", "log[lines['c@N/self_text.txt']]"),
        ("exists", "log[exists['c@N/self_text.txt']]\nlog[exists['c@N/nope.txt']]\nlog[exists['c@N/self_dir']]"),
        ("ls", "log[ls['c@N/self_dir']]"),
        ("write-read", "w = write['c@N/out.txt', 'abc\\ndef']\nlog[w]\nlog[read['c@N/out.txt']]"),
        ("write-over", "w = write['c@N/self_text.txt', 'new']\nlog[read['c@N/self_text.txt']]"),
        ("mkdir", "m = mkdir['c@N/made']\nlog[exists['c@N/made']]\nlog[ls['c@N/made']]"),
        ("mkdir-twice", "m = mkdir['c@N/made']\nr = mkdir['c@N/made']\nlog[r.ok]"),
        ("rm", "r = rm['c@N/self_text.txt']\nlog[exists['c@N/self_text.txt']]"),
        ("rm-missing", "r = rm['c@N/nope.txt']\nlog[r.ok]"),
        ("mv", "m = mv['c@N/self_text.txt', 'c@N/moved.txt']\nlog[exists['c@N/self_text.txt']]\nlog[read['c@N/moved.txt']]"),
        ("cp", "c = cp['c@N/self_text.txt', 'c@N/copy.txt']\nlog[exists['c@N/self_text.txt']]\nlog[read['c@N/copy.txt']]"),
        ("csv", "rows = csv['c@N/self_data.csv']\nlog[len[rows]]\nlog[rows[0]['b']]\nlog[rows[2]['c']]"),
    ];
    table
        .iter()
        .map(|(name, body)| {
            let mut c = Case::new(format!("io/{}", name), &[], body.lines().map(str::to_string).collect()).referring(format!("io:{}", name), &[]);
            c.own_dir = true;
            c.imports.insert("io");
            c
        })
        .collect()
}

/// The files whose own tests the test-block family runs: the standard
/// modules written in Crow, and each package's files that hold test blocks.
fn module_files() -> Vec<PathBuf> {
    let co = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir).map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "co")).collect()).unwrap_or_default()
    };
    let mut files = co(Path::new("src/modules"));
    let packages: Vec<PathBuf> = std::fs::read_dir("packages").map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect()).unwrap_or_default();
    for package in packages {
        files.extend(co(&package).into_iter().filter(|f| std::fs::read_to_string(f).unwrap_or_default().lines().any(|l| l.starts_with("test "))));
    }
    files.sort();
    files
}

/// The test-block family: every standard module's own tests run by `crow
/// test` under each interpreted configuration — the walker, no fusion, no
/// moves — as the gate runs them interpreted: each run passes, and all
/// say the same. A run that fails, or says otherwise than the interpreted
/// one, is a finding (`tests`). Answers the findings and how many runs.
fn module_tests(exe: &Path) -> (Vec<Finding>, usize) {
    let files = module_files();
    let interpreted: Vec<&Config> = CONFIGS.iter().filter(|c| !c.compiled).collect();
    let jobs: Vec<(&PathBuf, &Config)> = files.iter().flat_map(|f| interpreted.iter().map(move |c| (f, *c))).collect();
    let results: Mutex<Vec<(String, &str, Option<i32>, String)>> = Mutex::new(Vec::new());
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map(|c| c.get()).unwrap_or(4).clamp(1, 8);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some((file, config)) = jobs.get(i) else { break };
                // (each run in a directory of its own: a module's tests write
                // files where they run — io's `io-note.csv` — and four runs
                // at once had read each other's)
                let name = test_name(file);
                let run_dir = Path::new("target/crow_self/tests").join(format!("{}_{}", name.replace('/', "_"), config.name));
                let _ = std::fs::remove_dir_all(&run_dir);
                let _ = std::fs::create_dir_all(&run_dir);
                let absolute = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
                let mut cmd = Command::new(exe);
                cmd.arg("test").arg(&absolute);
                // (a package's tests read their references by paths from the
                // repository's root, and write nothing; a module's may write)
                if !file.starts_with("packages") {
                    cmd.current_dir(&run_dir);
                }
                for (k, v) in config.env {
                    cmd.env(k, v);
                }
                let (code, out, err, ended) = spawn_within(cmd, Duration::from_secs(600));
                let said = format!("{}\n{}", out, err);
                // (the run's last word, its colors read past)
                let last = said.lines().rev().map(strip_colors).find(|l| !l.trim().is_empty()).unwrap_or_default();
                let summary = match ended {
                    Ended::Exited => last,
                    other => format!("{:?}", other),
                };
                if let Ok(mut r) = results.lock() {
                    r.push((name, config.name, code, summary));
                }
            });
        }
    });
    let results = results.into_inner().unwrap_or_default();
    let mut findings = Vec::new();
    for file in &files {
        let name = test_name(file);
        let runs: Vec<&(String, &str, Option<i32>, String)> = results.iter().filter(|r| r.0 == name).collect();
        let first = runs.iter().find(|r| r.1 == "interpreted").map(|r| r.3.clone()).unwrap_or_default();
        for (_, config, code, summary) in runs {
            if *code != Some(0) || *summary != first {
                findings.push(Finding {
                    id: format!("tests/{}/{}", name, config),
                    code: format!("crow test {}", file.display()),
                    class: "tests",
                    detail: format!("{} — interpreted: {}", summary.trim(), first.trim()),
                });
            }
        }
    }
    (findings, results.len())
}

/// A test file's name as the family's findings say it: a module's stem
/// (`io`), a package's file past `packages/` (`gpt/model`).
fn test_name(file: &Path) -> String {
    let stem = file.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    match file.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()) {
        Some(package) if file.starts_with("packages") => format!("{}/{}", package, stem),
        _ => stem,
    }
}

/// Text with its terminal colors read past.
fn strip_colors(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn forms_reached(sources: &[&String]) -> BTreeSet<String> {
    let workers = std::thread::available_parallelism().map(|c| c.get()).unwrap_or(4);
    let chunk = sources.len().div_ceil(workers.max(1)).max(1);
    let seen: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    std::thread::scope(|s| {
        for part in sources.chunks(chunk) {
            let seen = &seen;
            s.spawn(move || {
                let mut mine = BTreeSet::new();
                for src in part {
                    if let Ok(stmts) = crate::build(src) {
                        forms_in(&stmts, &mut mine);
                    }
                }
                if let Ok(mut all) = seen.lock() {
                    all.extend(mine);
                }
            });
        }
    });
    seen.into_inner().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guards read the language from its own source: every operator
    /// of the AST is in the operator family, each spelled, and the forms
    /// are the AST's variants — never a field, a type or a comment.
    #[test]
    fn the_guards_read_the_language_from_its_source() {
        let ops = operators();
        assert_eq!(ops.len(), ast_variants("BinOp").len(), "every operator spelled: {:?}", ops);
        assert!(ops.contains(&"+") && ops.contains(&".^") && ops.contains(&"or"), "{:?}", ops);
        let stmts = ast_variants("Stmt");
        assert!(stmts.contains(&"Assign".to_string()) && stmts.contains(&"Line".to_string()) && stmts.contains(&"Break".to_string()), "{:?}", stmts);
        assert!(!stmts.iter().any(|v| matches!(v.as_str(), "Vec" | "Option" | "Expr" | "String")), "{:?}", stmts);
        let all = every_form();
        for f in ["Stmt::For", "Expr::Call", "BinOp::Add", "UnaryOp::Neg"] {
            assert!(all.contains(f), "{} in {:?}", f, all);
        }
        // a program's forms, by name
        let mut seen = BTreeSet::new();
        forms_in(&crate::build("a, b = 1, 2\nwhile a lt 3\n    a = a + 1\n    if a eq 2\n        continue\n    break\nlog[-a]\n").expect("parses"), &mut seen);
        for f in ["Stmt::MultiAssign", "Stmt::While", "Stmt::Continue", "Stmt::Break", "BinOp::Lt", "UnaryOp::Neg", "Expr::Call"] {
            assert!(seen.contains(f), "{} in {:?}", f, seen);
        }
    }

    /// A list holds a run: what the run makes that the list lacks is new,
    /// what the list holds that the run no longer makes is gone, and a
    /// written list keeps the reason a person gave an entry.
    #[test]
    fn a_list_holds_a_run_and_keeps_its_reasons() {
        let path = std::env::temp_dir().join(format!("crow_listed_{}.txt", std::process::id()));
        let list = Listed { path: Box::leak(path.display().to_string().into_boxed_str()), header: "# a list\n" };
        let _ = std::fs::write(list.path, "# a list\na\tkept by hand\nb\twhy b\n");
        let now: BTreeMap<String, String> = [("a".to_string(), "made".to_string()), ("c".to_string(), "why c".to_string())].into_iter().collect();
        let (new, gone) = list.held(&now);
        assert_eq!(new, vec![("c".to_string(), "why c".to_string())]);
        assert_eq!(gone, vec!["b".to_string()]);
        list.write(&now).expect("written");
        let read = list.read();
        assert_eq!(read.get("a").map(String::as_str), Some("kept by hand"));
        assert_eq!(read.get("c").map(String::as_str), Some("why c"));
        assert!(!read.contains_key("b"));
        let _ = std::fs::remove_file(list.path);
    }

    /// The cases are the same every run, each id once, and a case's program
    /// holds the helpers it names and no other
    #[test]
    fn the_cases_are_the_same_every_run() {
        let all = values();
        let a: Vec<String> = form_cases(&all).iter().map(|c| c.id.clone()).collect();
        let b: Vec<String> = form_cases(&all).iter().map(|c| c.id.clone()).collect();
        assert_eq!(a, b);
        let mut seen = HashSet::new();
        for id in &a {
            assert!(seen.insert(id), "a case id twice: {}", id);
        }
        let case = Case::new("t", &[], vec!["log[self_pass[self_may[1, false]]]".to_string()]);
        let (src, starts) = program(&[&case]);
        assert!(src.contains("fn self_may_0[") && src.contains("fn self_pass_0[") && !src.contains("fn self_first"), "{}", src);
        assert!(src.contains("log[self_pass_0[self_may_0[1, false]]]"), "{}", src);
        assert_eq!(starts.len(), 1);
        assert!(src.contains("log['§§c0']"), "{}", src);
    }

    /// Every value of the catalog is one the language takes, shown alone
    #[test]
    fn every_value_lowers_shown() {
        let all = values();
        let mut refused = Vec::new();
        for v in &all {
            // (a function is not shown)
            if matches!(v.name.as_str(), "sizes" | "groups" | "lists" | "fn" | "pred") {
                continue;
            }
            // (the layer's values are lowered by the check itself, its door
            // open — CROW_SYS — which a unit test does not open: the
            // variable is the process's, and another test closes it)
            if v.imports.contains(&"sys") {
                continue;
            }
            let case = Case::new(format!("shown/{}", v.name), &[v], vec![format!("log[{}]", v.expr)]);
            if let Lowered::Refused(m) | Lowered::Bug(m) = lower(program(&[&case]).0) {
                refused.push(format!("{}: {}", v.name, m.lines().next().unwrap_or("")));
            }
        }
        assert!(refused.is_empty(), "values the language refuses shown:\n{}", refused.join("\n"));
    }
}
