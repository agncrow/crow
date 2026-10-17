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

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::table::{Kind, Row, ROWS};

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
        Val { name: "sparse".into(), expr: "sparse[[[1.0, 0.0], [0.0, 2.0]]]".into(), setup: Vec::new(), imports: vec!["math"] },
        plain("colnames", "['a', 'b']"),
        plain("scheme", "'base64'"),
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
        Kind::Str => &["text", "textkey", "textnum", "text0"],
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
        Kind::Bytes => &["bytes"],
        Kind::Shown | Kind::ShownDyn | Kind::Shows => &["int", "text", "ints"],
        Kind::Encoded => &["map", "json"],
        // given, never taken from a program; or a handle, a stream, a
        // server's answer, a fused kernel's own: no program value is one
        Kind::Sparse => &["sparse"],
        Kind::FloatMap
        | Kind::Unit
        | Kind::Stream
        | Kind::Chunks
        | Kind::Answer
        | Kind::Response
        | Kind::Socket
        | Kind::Mailer
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
    "json", "moment", "frame", "colint", "coltext", "colbool", "colmissing", "may:int", "fail:int", "may:ints", "fail:vector",
];

// ── cases ───────────────────────────────────────────────────────────

/// A case: a body of statements run as one function of its own.
#[derive(Clone, Debug)]
pub(crate) struct Case {
    pub id: String,
    imports: BTreeSet<&'static str>,
    body: Vec<String>,
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
        Case { id: id.into(), imports, body }
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
    ("self_first", "fn self_first[x]\n    return x[0]\n"),
    ("self_apply", "fn self_apply[g, x]\n    return g[x]\n"),
    ("self_double", "fn self_double[x]\n    return x * 2\n"),
    ("self_fact", "fn self_fact[n]\n    if n lt 2\n        return 1\n    return n * self_fact[n - 1]\n"),
    ("self_count", "fn self_count[n]\n    if n eq 0\n        return 0\n    return 1 + self_count[n - 1]\n"),
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
            let mut t = text.to_string();
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
        out.push_str(&format!("fn self_case_{}[]\n", k));
        for line in &c.body {
            for l in own(line).lines() {
                out.push_str("    ");
                out.push_str(l);
                out.push('\n');
            }
        }
        out.push_str("    return 0\n");
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
        ];
        for (shape, lines) in shapes {
            cases.push(Case::new(format!("shape/{}/{}", shape, n), &[x], lines));
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
            cases.push(Case::new(format!("index/{}/{}", b, i), &[bv, iv], vec![format!("log[{}[{}]]", bv.expr, iv.expr)]));
            cases.push(Case::new(format!("index-held/{}/{}", b, i), &[bv, iv], vec![format!("b = {}", bv.expr), format!("log[b[{}]]", iv.expr)]));
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
                cases.push(Case::new(format!("write/{}/{}/{}", b, i, x), &[bv, iv, xv], vec![format!("b = {}", bv.expr), format!("b[{}] = {}", iv.expr, xv.expr), "log[b]".to_string()]));
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

    // an operator over every pair of operands
    let ops = ["+", "-", "*", "/", "%", "^", ".*", "./", ".^", "eq", "ne", "gt", "lt", "gte", "lte", "and", "or"];
    let operands = ["int", "float", "text", "bool", "ints", "vector", "matrix", "map", "moment", "decimal", "big", "colint", "colbool", "coltext", "may:int", "fail:int"];
    for op in ops {
        for a in operands {
            for b in operands {
                let (av, bv) = (v(a), v(b));
                cases.push(Case::new(format!("op/{}/{}/{}", op, a, b), &[av, bv], vec![format!("log[{} {} {}]", av.expr, op, bv.expr)]));
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
    let kinds = ["int", "float", "text", "ints", "texts", "vector", "matrix", "map", "frame", "colint", "record", "may:int"];
    let helpers = [("pass", "self_pass[@]"), ("first", "self_first[@]"), ("may", "self_may[@, false]"), ("apply", "self_apply[self_pass, @]"), ("boxed", "SelfBox[@].v")];
    for (h, call) in helpers {
        for (i, a) in kinds.iter().enumerate() {
            for b in kinds.iter().skip(i + 1) {
                let (av, bv) = (v(a), v(b));
                cases.push(Case::new(format!("twice/{}/{}+{}", h, a, b), &[av, bv], vec![format!("log[{}]", call.replace('@', &av.expr)), format!("log[{}]", call.replace('@', &bv.expr))]));
            }
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

/// The rows a program cannot run here, and why: the language's own layer,
/// the servers and the network, files and the clock.
fn skipped_module(module: &str) -> Option<&'static str> {
    match module {
        "sys" => Some("the standard library's own layer"),
        "db.postgres" | "db.clickhouse" => Some("a database server"),
        "web" | "web.mail" | "web.auth" | "web.ws" => Some("a server, the network or mail"),
        "llm" => Some("a model over the network"),
        "io" => Some("files"),
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
];

/// The module a row is imported from in a program.
fn import_of(module: &str) -> Option<&'static str> {
    match module {
        "" => None,
        "math" => Some("math"),
        "math.stats" => Some("math.stats"),
        "math.nn" => Some("math.nn"),
        "frame" => Some("frame"),
        "doc" => Some("doc"),
        "bits" => Some("bits"),
        "chart" => Some("chart"),
        _ => None,
    }
}

/// A call of a row with the given arguments, as a case.
fn row_case(id: String, row: &Row, args: &[&Val]) -> Case {
    let call = format!("{}[{}]", row.name, args.iter().map(|a| a.expr.as_str()).collect::<Vec<_>>().join(", "));
    let mut case = Case::new(id, args, vec![format!("log[{}]", call)]);
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
fn row_cases(all: &[Val], only: &Option<String>) -> (Vec<Case>, Vec<String>) {
    let mut found: Vec<Case> = Vec::new();
    let mut unexercised: Vec<String> = Vec::new();
    // the rows to try, each with its candidate calls
    let mut trials: Vec<(usize, Vec<Vec<&Val>>)> = Vec::new();
    let mut seen_calls: HashSet<String> = HashSet::new();
    for (r, row) in ROWS.iter().enumerate() {
        if row.name.starts_with("__") {
            continue;
        }
        if let Some(why) = skipped_module(row.module) {
            unexercised.push(format!("{} — {}", row.id, why));
            continue;
        }
        if crate::ir::is_effect(row.name) || crate::ir::DRAWS.contains(&row.name) {
            unexercised.push(format!("{} — an effect or a random draw", row.id));
            continue;
        }
        if import_of(row.module).is_none() && !row.module.is_empty() {
            unexercised.push(format!("{} — module '{}' has no import here", row.id, row.module));
            continue;
        }
        if let Some(w) = only
            && !format!("row/{}", row.id).contains(w.as_str())
        {
            continue;
        }
        // (a name the parser rewrites: the row is reached through the form)
        if matches!(row.name, "contains" | "has") {
            continue;
        }
        // (a call whose arguments no kind says: written out)
        if let Some((_, call)) = WRITTEN.iter().find(|(id, _)| *id == row.id) {
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
            unexercised.push(format!("{} — takes a {} no program value is", row.id, k));
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
        // (one name answers several rows: a call tried once)
        combos.retain(|c| seen_calls.insert(format!("{}[{}]", row.name, c.iter().map(|v| v.name.as_str()).collect::<Vec<_>>().join(","))));
        if combos.is_empty() {
            continue;
        }
        trials.push((r, combos));
    }
    // one call that lowers, per row
    let mut sources = Vec::new();
    let mut owners = Vec::new();
    for (t, (r, combos)) in trials.iter().enumerate() {
        for (c, args) in combos.iter().enumerate() {
            let case = row_case(String::new(), &ROWS[*r], args);
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
    for (t, (r, combos)) in trials.iter().enumerate() {
        let row = &ROWS[*r];
        let Some(c) = base.get(&t) else {
            unexercised.push(format!("{} — no call of it lowers: {}", row.id, first_refusal.get(&t).cloned().unwrap_or_default()));
            continue;
        };
        let args = &combos[*c];
        let id = format!("row/{}/base", row.id);
        if ids.insert(id.clone()) {
            found.push(row_case(id, row, args));
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
                    found.push(row_case(id, row, &a));
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
fn spawn_all(mut cmd: Command) -> (Option<i32>, String, String, Ended) {
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
                } else if start.elapsed() > LIMIT {
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

/// A batch's cases run under one configuration, from the first to the
/// last: a case that stops the run (its words flushed) gets the stop, and
/// the next run starts after it. A run stopped from outside — its memory,
/// its time, its output — or one that ends with no case's marker may have
/// lost what it had not written yet, so the cases from the last marker on
/// run one to a process until the one that ends so is found.
fn run_batch(exe: &Path, dir: &Path, file: &str, binary: Option<&Path>, config: &Config, n: usize) -> Vec<Seen> {
    let mut seen: Vec<Option<Seen>> = vec![None; n];
    let mut from = 0;
    let mut single = false;
    while from < n {
        let mut cmd = match binary {
            Some(b) => Command::new(b),
            None => {
                let mut c = Command::new(exe);
                c.args(["run", "-i", file]);
                c
            }
        };
        cmd.current_dir(dir).env("CROW_FROM", format!("c{}", from)).env("CROW_SEED", "1");
        if single {
            cmd.env("CROW_TO", format!("c{}", from));
        }
        for (k, v) in config.env {
            cmd.env(k, v);
        }
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
        let clean = code == Some(0) && ended == Ended::Exited;
        if clean {
            for (k, t) in &texts {
                if *k < n {
                    seen[*k] = Some(Seen { out: t.clone(), end: String::new() });
                }
            }
            if single {
                from += 1;
                continue;
            }
            break;
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
        } else {
            format!("stop: {}", last)
        };
        let flushed = ended == Ended::Exited && code.is_some() && !end.starts_with("crash") && !marked.is_empty();
        if !single && !flushed {
            // (what came before the last marker is whole; from it on, one by one)
            let restart = marked.last().copied().unwrap_or(from);
            for (k, t) in &texts {
                if *k < restart {
                    seen[*k] = Some(Seen { out: t.clone(), end: String::new() });
                }
            }
            single = true;
            from = restart;
            continue;
        }
        let at = if single { from } else { marked.last().copied().unwrap_or(from) };
        for (k, t) in &texts {
            if *k < at {
                seen[*k] = Some(Seen { out: t.clone(), end: String::new() });
            }
        }
        if at < n {
            seen[at] = Some(Seen { out: texts.get(&at).cloned().unwrap_or_default(), end });
        }
        from = at + 1;
        single = false;
    }
    seen.into_iter().map(|s| s.unwrap_or(Seen { out: String::new(), end: "not run".to_string() })).collect()
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
        cmd.args(["build", &file]).current_dir(dir).env("CROW_BUILD_PROFILE", "dev").env("CARGO_BUILD_JOBS", jobs);
        let (code, out, err, _) = spawn_all(cmd);
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
                placed.entry(*k).or_insert_with(|| line.to_string());
            }
        }
        if placed.is_empty() {
            if cases.len() == 1 {
                let why = lines.iter().find(|l| l.starts_with("error")).copied().unwrap_or("the build failed").to_string();
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
        out.push(Finding { id: case.id.clone(), class: "message-place", detail: first.to_string() });
    }
    // (Rust's words and the compiler's own: a message is in the program's)
    const JARGON: &[&str] = &["target type", "usize", "i64", "f64", "Vec<", "Option<", "Result<", "&str", "BTreeMap", "CrowValue", "CrowTensor", "Ty::", "Expr::", "Kind::", "Arg::", "unwrap", "panicked"];
    if let Some(word) = JARGON.iter().find(|w| first.contains(*w)) {
        out.push(Finding { id: case.id.clone(), class: "message-jargon", detail: format!("{} — says '{}'", first, word) });
    }
    let foreign = crate::errors_corpus::foreign_names(source, first);
    if !foreign.is_empty() {
        out.push(Finding { id: case.id.clone(), class: "message-names", detail: format!("{} — names {}", first, foreign.join(", ")) });
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
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = PathBuf::from("target").join("crow_self");
    let _ = std::fs::create_dir_all(&dir);

    // the cases
    let all = values();
    let mut cases = form_cases(&all);
    let (rows, unexercised) = row_cases(&all, &only);
    let row_count = rows.len();
    cases.extend(rows);
    if let Some(w) = &only {
        cases.retain(|c| c.id.contains(w.as_str()));
    }
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
                findings.extend(message_findings(&cases[i], &sources[i], m));
            }
            Lowered::Bug(m) => {
                internal += 1;
                findings.push(Finding { id: cases[i].id.clone(), class: "internal", detail: m.lines().next().unwrap_or("").to_string() });
            }
        }
    }
    println!("lowered in {:.1}s: {} accepted, {} refused, {} a bug in Crow", t.elapsed().as_secs_f64(), accepted.len(), refused, internal);
    for (family, (a, r, b)) in &families {
        println!("  {:<12} {:>6} accepted {:>6} refused {:>4} a bug in Crow", family, a, r, b);
    }
    let _ = std::fs::write(dir.join("refusals.txt"), &refusals);

    if !lower_only {
        // batches: the cases that lower, by the modules they import
        const BATCH: usize = 120;
        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for &i in &accepted {
            let key = cases[i].imports.iter().copied().collect::<Vec<_>>().join(",");
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
                        findings.push(Finding { id: cases[b[0]].id.clone(), class: "batch", detail: format!("lowers alone, not beside other cases: {}", m.lines().next().unwrap_or("")) });
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
            findings.push(Finding { id: cases[*i].id.clone(), class: "no-build", detail: why.clone() });
        }
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
                        let seen = run_batch(&exe, &batch.dir, &batch.file, if config.compiled { Some(batch.binary.as_path()) } else { None }, config, batch.members.len());
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
        let results = results.into_inner().unwrap_or_default();
        let mut agreed = 0;
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
                    findings.push(Finding { id: case.id.clone(), class: "disagree", detail });
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
                    findings.push(Finding { id: case.id.clone(), class, detail: format!("{}: {}{}", n, s.out.trim().replace('\n', " | "), if s.end.is_empty() { String::new() } else { format!(" [{}]", s.end) }) });
                } else {
                    agreed += 1;
                }
                let _ = i;
            }
        }
        println!("ran: {} agreed in every configuration ({})", agreed, CONFIGS.iter().map(|c| c.name).collect::<Vec<_>>().join(", "));
        // (the builders' caches stay for the next run; `--clean` removes them)
        if clean {
            let _ = std::fs::remove_dir_all(&build_dir);
        }
    }

    // the report, and the list of the known
    findings.sort_by(|a, b| (a.class, &a.id).cmp(&(b.class, &b.id)));
    let mut seen_ids = HashSet::new();
    findings.retain(|f| seen_ids.insert((f.class, f.id.clone())));
    let report: String = findings.iter().map(|f| format!("{:<14} {}\n      {}\n", f.class, f.id, f.detail.replace('\n', "\n      "))).collect();
    let _ = std::fs::write(dir.join("findings.txt"), &report);
    // the findings by what they share: one line per defect, its count and
    // the cases that show it first
    let mut groups: BTreeMap<(String, String), Vec<&str>> = BTreeMap::new();
    for f in &findings {
        groups.entry((f.class.to_string(), group_key(f))).or_default().push(f.id.as_str());
    }
    let mut grouped: Vec<(&(String, String), &Vec<&str>)> = groups.iter().collect();
    grouped.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
    let summary: String = grouped
        .iter()
        .map(|((class, key), ids)| format!("{:>5}  {:<10} {}\n       e.g. {}\n", ids.len(), class, key, ids.iter().take(3).copied().collect::<Vec<_>>().join(", ")))
        .collect();
    let _ = std::fs::write(dir.join("summary.txt"), &summary);
    let mut by_class: BTreeMap<&str, usize> = BTreeMap::new();
    for f in &findings {
        *by_class.entry(f.class).or_default() += 1;
    }
    println!("\nnot exercised: {} rows (listed in {})", unexercised.len(), dir.join("unexercised.txt").display());
    let _ = std::fs::write(dir.join("unexercised.txt"), unexercised.join("\n") + "\n");
    println!("findings: {} ({}) in {} groups — every one in {}, the groups in {}", findings.len(), by_class.iter().map(|(c, n)| format!("{} {}", n, c)).collect::<Vec<_>>().join(", "), groups.len(), dir.join("findings.txt").display(), dir.join("summary.txt").display());
    if bless {
        if only.is_some() || lower_only {
            eprintln!("--bless writes the whole list: run it without --only and --lower-only");
            return 1;
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
    let known = read_known();
    let now: BTreeSet<(String, String)> = findings.iter().map(|f| (f.class.to_string(), f.id.clone())).collect();
    let new: Vec<&Finding> = findings.iter().filter(|f| !known.contains(&(f.class.to_string(), f.id.clone()))).collect();
    // (a known finding is gone only where this run could have found it)
    let ran: HashSet<&str> = cases.iter().map(|c| c.id.as_str()).collect();
    // (with --lower-only, only what the lowering finds could have gone)
    let could_find = |c: &str| !lower_only || matches!(c, "internal" | "message-place" | "message-names" | "message-jargon");
    let gone: Vec<&(String, String)> = known.iter().filter(|(c, i)| ran.contains(i.as_str()) && could_find(c) && !now.contains(&(c.clone(), i.clone()))).collect();
    for f in &new {
        println!("  NEW   {:<14} {}\n        {}", f.class, f.id, f.detail.replace('\n', "\n        "));
    }
    for (c, i) in &gone {
        println!("  GONE  {:<14} {} — fixed: strike it from {}", c, i, known_path().display());
    }
    println!("\n{} new, {} known, {} gone ({:.1}s)", new.len(), now.len() - new.len(), gone.len(), start.elapsed().as_secs_f64());
    if new.is_empty() && gone.is_empty() {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            if matches!(v.name.as_str(), "sizes" | "groups" | "lists" | "fn") {
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
