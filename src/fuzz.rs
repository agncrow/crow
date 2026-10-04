//! `crow test --fuzz [n] [--seed s]`: differential fuzzing (docs/roadmap.md,
//! the 1.0 bar). Programs generated from a seed — ints, floats, text,
//! lists and maps, records, parsed documents read and spliced into text,
//! vectors and matrices (their arithmetic, reductions, rows and numbers
//! walked and indexed, a record holding them), frames built in the program
//! (filtered, sorted, grouped, pivoted, windowed, their cells read), functions
//! handed as values, arithmetic and comparisons, the text, list and map
//! builtins, branches, loops and functions — each case a function of its
//! own, all of them in one program, run by the interpreter and compiled; a
//! line where the two disagree is a bug in Crow, and the program is kept to
//! reproduce it. The same seed makes the same programs.

/// A small, seedable generator (SplitMix64): the same seed, the same
/// programs, on every machine.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// The kinds a generated expression has.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Int,
    Float,
    Text,
    Bool,
    Ints,
    Texts,
    /// a map of ints
    Counts,
    /// a parsed document: an object with a text `a`, a number `n` and a
    /// list `l`, read, shown and spliced into text as it is
    Doc,
    /// a vector of three floats (every vector is, so shapes always meet)
    Vec,
    /// a matrix of two rows of three floats
    Mat,
    /// a frame of four rows: text `k` and `g`, int `a`, float `b`
    Frame,
}

const KINDS: &[Kind] = &[Kind::Int, Kind::Float, Kind::Text, Kind::Bool, Kind::Ints, Kind::Texts, Kind::Counts, Kind::Doc, Kind::Vec, Kind::Mat, Kind::Frame];

/// The helper functions every generated program declares, which its cases
/// call: two ints in, an int out.
const HELPERS: usize = 3;

/// One case: its locals and their kinds as it goes.
struct Case<'r> {
    rng: &'r mut Rng,
    vars: Vec<(String, Kind)>,
    lines: Vec<String>,
    depth: usize,
    /// names made so far: each new one is new (a block's names go out of
    /// scope with it, and a variable's type cannot change)
    made: usize,
}

impl<'r> Case<'r> {
    fn var_of(&mut self, kind: Kind) -> Option<String> {
        let found: Vec<String> = self.vars.iter().filter(|(_, k)| *k == kind).map(|(n, _)| n.clone()).collect();
        if found.is_empty() {
            None
        } else {
            Some(self.rng.pick(&found).clone())
        }
    }

    /// An expression of a kind, small enough that no value overflows: ints
    /// stay below a few thousand, a division's divisor is never zero.
    fn expr(&mut self, kind: Kind, depth: usize) -> String {
        if depth == 0 || self.rng.chance(35) {
            if let Some(v) = self.var_of(kind).filter(|_| self.rng.chance(60)) {
                return v;
            }
            return self.leaf(kind);
        }
        let d = depth - 1;
        match kind {
            Kind::Int => match self.rng.below(24) {
                16 => format!("len[{}]", self.expr(Kind::Vec, d)),
                17 => format!("len[{}]", self.expr(Kind::Mat, d)),
                18 => format!("argmax[{}]", self.expr(Kind::Vec, d)),
                19 => format!("len[flatten[{}]]", self.expr(Kind::Mat, d)),
                20 => format!("len[{}]", self.expr(Kind::Frame, d)),
                21 => format!("len[{}.k]", self.expr(Kind::Frame, d)),
                22 => {
                    let df = self.var_frame(d);
                    format!("len[filter[{}, {}.a gt {}]]", df, df, 1 + self.rng.below(3))
                }
                23 => format!("len[group[{}, 'k', ['a', 'sum']]]", self.expr(Kind::Frame, d)),
                15 => format!("len[{}.l]", self.expr(Kind::Doc, d)),
                9 => format!("floor[{}]", self.expr(Kind::Float, d)),
                10 => format!("ceil[{}]", self.expr(Kind::Float, d)),
                11 => format!("min[{}, {}]", self.expr(Kind::Int, d), self.expr(Kind::Int, d)),
                12 => format!("max[{}, {}]", self.expr(Kind::Int, d), self.expr(Kind::Int, d)),
                13 => format!("h{}[{}, {}]", self.rng.below(HELPERS as u64), self.expr(Kind::Int, d), self.expr(Kind::Int, d)),
                14 => format!("len[{}]", self.expr(Kind::Counts, d)),
                0 => format!("({} + {})", self.expr(Kind::Int, d), self.expr(Kind::Int, d)),
                1 => format!("({} - {})", self.expr(Kind::Int, d), self.expr(Kind::Int, d)),
                2 => format!("({} * {})", self.small_int(), self.expr(Kind::Int, d)),
                3 => format!("({} / {})", self.expr(Kind::Int, d), self.nonzero_int()),
                4 => format!("({} % {})", self.expr(Kind::Int, d), self.nonzero_int()),
                5 => format!("len[{}]", self.expr(Kind::Text, d)),
                6 => format!("len[{}]", self.expr(Kind::Ints, d)),
                7 => format!("abs[{}]", self.expr(Kind::Int, d)),
                _ => format!("int[{}]", self.expr(Kind::Float, d)),
            },
            Kind::Float => match self.rng.below(20) {
                6 => format!("sum[{}]", self.expr(Kind::Vec, d)),
                7 => format!("mean[{}]", self.expr(Kind::Vec, d)),
                8 => format!("max[{}]", self.expr(Kind::Vec, d)),
                9 => format!("{}[{}]", self.expr(Kind::Vec, d), self.rng.below(3)),
                10 => format!("{}[{}][{}]", self.expr(Kind::Mat, d), self.rng.below(2), self.rng.below(3)),
                11 => format!("sum[{}]", self.expr(Kind::Mat, d)),
                12 => {
                    let m = self.var_mat(d);
                    format!("sum[{} * transpose[{}]]", m, m)
                }
                13 => format!("norm[{}]", self.expr(Kind::Vec, d)),
                14 => format!("max[sum[{}, 1]]", self.expr(Kind::Mat, d)),
                15 => format!("t0[{}]", self.expr(Kind::Vec, d)),
                16 => format!("t1[{}]", self.expr(Kind::Mat, d)),
                17 => format!("sum[{}.b]", self.expr(Kind::Frame, d)),
                18 => format!("{}.a[0]", self.expr(Kind::Frame, d)),
                19 => format!("{}.b[0]", self.expr(Kind::Frame, d)),
                0 => format!("({} + {})", self.expr(Kind::Float, d), self.expr(Kind::Float, d)),
                1 => format!("({} * {})", self.expr(Kind::Float, d), self.leaf(Kind::Float)),
                2 => format!("({} / {}.5)", self.expr(Kind::Float, d), 1 + self.rng.below(9)),
                3 => format!("float[{}]", self.expr(Kind::Int, d)),
                4 => format!("({} - {})", self.expr(Kind::Float, d), self.expr(Kind::Float, d)),
                _ => format!("({} + {})", self.expr(Kind::Float, d), self.expr(Kind::Int, d)),
            },
            Kind::Text => match self.rng.below(14) {
                12 => format!("{}.k[0]", self.expr(Kind::Frame, d)),
                13 => format!("type[{}[{}]]", self.expr(Kind::Vec, d), self.rng.below(3)),
                10 => format!("str[{}.a]", self.expr(Kind::Doc, d)),
                // a document's members spliced into text — of a variable,
                // since a span holds no quotes
                11 => match self.var_of(Kind::Doc) {
                    Some(v) => format!("'a `{}.a` n `{}.n` l `{}.l`'", v, v, v),
                    None => self.leaf(Kind::Text),
                },
                7 => format!("replace[{}, 'a', 'b']", self.expr(Kind::Text, d)),
                8 => format!("repeat[{}, {}]", self.expr(Kind::Text, d), self.rng.below(3)),
                9 => format!("str[{}]", self.expr(Kind::Float, d)),
                0 => format!("join[[{}, {}], '']", self.expr(Kind::Text, d), self.expr(Kind::Text, d)),
                1 => format!("upper[{}]", self.expr(Kind::Text, d)),
                2 => format!("lower[{}]", self.expr(Kind::Text, d)),
                3 => format!("str[{}]", self.expr(Kind::Int, d)),
                4 => format!("join[{}, '-']", self.expr(Kind::Texts, d)),
                5 => format!("trim[{}]", self.expr(Kind::Text, d)),
                _ => {
                    let inside = self.var_of(Kind::Int).unwrap_or_else(|| format!("{}", self.rng.below(50)));
                    format!("'n`{}`'", inside)
                }
            },
            Kind::Bool => match self.rng.below(13) {
                10 => format!("(sum[{}] gt sum[{}])", self.expr(Kind::Vec, d), self.expr(Kind::Vec, d)),
                11 => {
                    let df = self.var_frame(d);
                    format!("({}.a[0] lt {}.b[0])", df, df)
                }
                12 => {
                    let df = self.var_frame(d);
                    format!("(len[filter[{}, {}.b gte 1.5]] eq 3)", df, df)
                }
                6 => format!("starts[{}, 'a']", self.expr(Kind::Text, d)),
                7 => format!("ends[{}, 'z']", self.expr(Kind::Text, d)),
                8 => format!("has[{}, 'a']", self.expr(Kind::Counts, d)),
                9 => format!("int[{}].ok", self.rng.pick(&["'12'", "'x1'", "'-3'", "''"])),
                0 => format!("({} lt {})", self.expr(Kind::Int, d), self.expr(Kind::Int, d)),
                1 => format!("({} eq {})", self.expr(Kind::Text, d), self.expr(Kind::Text, d)),
                2 => format!("({} and {})", self.expr(Kind::Bool, d), self.expr(Kind::Bool, d)),
                3 => format!("(not {})", self.expr(Kind::Bool, d)),
                4 => format!("contains[{}, {}]", self.expr(Kind::Ints, d), self.expr(Kind::Int, d)),
                _ => format!("({} gte {})", self.expr(Kind::Float, d), self.expr(Kind::Float, d)),
            },
            Kind::Ints => match self.rng.below(5) {
                0 => format!("push[{}, {}]", self.expr(Kind::Ints, d), self.expr(Kind::Int, d)),
                1 => format!("sort[{}]", self.expr(Kind::Ints, d)),
                2 => format!("reverse[{}]", self.expr(Kind::Ints, d)),
                3 => format!("range[{}]", self.rng.below(6)),
                _ => format!("concat[{}, {}]", self.expr(Kind::Ints, d), self.expr(Kind::Ints, d)),
            },
            Kind::Counts => match self.rng.below(2) {
                0 => format!("set[{}, '{}', {}]", self.expr(Kind::Counts, d), self.rng.pick(&["a", "b", "zz"]), self.expr(Kind::Int, d)),
                _ => format!("del[{}, 'a']", self.expr(Kind::Counts, d)),
            },
            Kind::Doc => self.leaf(Kind::Doc),
            // a vector stays three long: element-wise arithmetic, a map, a
            // sort, a matrix's row, a column sum
            Kind::Vec => match self.rng.below(13) {
                0 => format!("({} + {})", self.expr(Kind::Vec, d), self.expr(Kind::Vec, d)),
                1 => format!("({} .* {})", self.expr(Kind::Vec, d), self.expr(Kind::Vec, d)),
                2 => format!("({} * {})", self.expr(Kind::Vec, d), self.leaf(Kind::Float)),
                3 => format!("({} - {})", self.expr(Kind::Vec, d), self.expr(Kind::Vec, d)),
                4 => format!("({} + {})", self.expr(Kind::Vec, d), self.expr(Kind::Float, d)),
                5 => format!("sort[{}]", self.expr(Kind::Vec, d)),
                6 => format!("reverse[{}]", self.expr(Kind::Vec, d)),
                7 => format!("abs[{}]", self.expr(Kind::Vec, d)),
                8 => format!("cumsum[{}]", self.expr(Kind::Vec, d)),
                9 => format!("clip[{}, -1.0, 2.0]", self.expr(Kind::Vec, d)),
                10 => format!("{}[{}]", self.expr(Kind::Mat, d), self.rng.below(2)),
                11 => format!("sum[{}, 0]", self.expr(Kind::Mat, d)),
                _ => format!("relu[{}]", self.expr(Kind::Vec, d)),
            },
            // a matrix stays two by three
            Kind::Mat => match self.rng.below(10) {
                0 => format!("({} + {})", self.expr(Kind::Mat, d), self.expr(Kind::Mat, d)),
                1 => format!("({} .* {})", self.expr(Kind::Mat, d), self.expr(Kind::Mat, d)),
                2 => format!("({} * {})", self.expr(Kind::Mat, d), self.leaf(Kind::Float)),
                3 => format!("({} - {})", self.expr(Kind::Mat, d), self.expr(Kind::Float, d)),
                4 => format!("abs[{}]", self.expr(Kind::Mat, d)),
                5 => format!("relu[{}]", self.expr(Kind::Mat, d)),
                6 => format!("transpose[transpose[{}]]", self.expr(Kind::Mat, d)),
                7 => format!("reshape[flatten[{}], 2, 3]", self.expr(Kind::Mat, d)),
                8 => format!("math.stack[[{}, {}]]", self.expr(Kind::Vec, d), self.expr(Kind::Vec, d)),
                _ => format!("({} + {})", self.expr(Kind::Mat, d), self.expr(Kind::Vec, d)),
            },
            // a frame keeps its columns: filtered, sorted, cut, stacked on itself
            Kind::Frame => match self.rng.below(7) {
                0 => {
                    let df = self.var_frame(d);
                    format!("filter[{}, {}.a gt {}]", df, df, self.rng.below(4))
                }
                1 => {
                    let df = self.var_frame(d);
                    format!("filter[{}, {}.k eq 'x']", df, df)
                }
                2 => format!("sort[{}, 'b']", self.expr(Kind::Frame, d)),
                3 => format!("sort[{}, 'a', false]", self.expr(Kind::Frame, d)),
                4 => format!("head[{}, 2]", self.expr(Kind::Frame, d)),
                5 => format!("tail[{}, 3]", self.expr(Kind::Frame, d)),
                _ => {
                    let df = self.var_frame(d);
                    format!("frame.stack[{}, {}]", df, df)
                }
            },
            Kind::Texts => match self.rng.below(5) {
                4 => format!("frame.unique[{}, 'k']", self.expr(Kind::Frame, d)),
                3 => format!("keys[{}]", self.expr(Kind::Counts, d)),
                0 => format!("split[{}, ' ']", self.expr(Kind::Text, d)),
                1 => format!("push[{}, {}]", self.expr(Kind::Texts, d), self.expr(Kind::Text, d)),
                _ => format!("sort[{}]", self.expr(Kind::Texts, d)),
            },
        }
    }

    /// A frame named twice in one expression (`filter[df, df.a gt 1]`): a
    /// variable of the case's, or a fresh one bound first.
    fn var_frame(&mut self, depth: usize) -> String {
        if let Some(v) = self.var_of(Kind::Frame) {
            return v;
        }
        let e = self.expr(Kind::Frame, depth.min(1));
        let name = self.fresh(Kind::Frame);
        let pad = self.indent();
        self.lines.push(format!("{}{} = {}", pad, name, e));
        name
    }

    /// A matrix named twice (`m * transpose[m]`).
    fn var_mat(&mut self, depth: usize) -> String {
        if let Some(v) = self.var_of(Kind::Mat) {
            return v;
        }
        let e = self.expr(Kind::Mat, depth.min(1));
        let name = self.fresh(Kind::Mat);
        let pad = self.indent();
        self.lines.push(format!("{}{} = {}", pad, name, e));
        name
    }

    /// An int expression that calls no helper, and binds no variable (a
    /// helper's body is its expression alone, so a frame or a matrix a
    /// sub-expression would have named first has nowhere to be bound).
    fn expr_no_calls(&mut self, kind: Kind, depth: usize) -> String {
        loop {
            let (lines, vars) = (self.lines.len(), self.vars.len());
            let e = self.expr(kind, depth);
            if self.lines.len() != lines {
                self.lines.truncate(lines);
                self.vars.truncate(vars);
                continue;
            }
            if !e.contains("h0[") && !e.contains("h1[") && !e.contains("h2[") {
                return e;
            }
        }
    }

    fn small_int(&mut self) -> String {
        format!("{}", self.rng.below(7))
    }

    fn nonzero_int(&mut self) -> String {
        format!("{}", 1 + self.rng.below(9))
    }

    fn leaf(&mut self, kind: Kind) -> String {
        match kind {
            Kind::Int => format!("{}", self.rng.below(40) as i64 - 10),
            Kind::Float => format!("{}.{}", self.rng.below(20), self.rng.below(10)),
            Kind::Text => self.rng.pick(&["'a'", "'crow'", "'Ab Cd'", "''", "'x y z'", "' pad '"]).to_string(),
            Kind::Bool => self.rng.pick(&["true", "false"]).to_string(),
            Kind::Ints => self.rng.pick(&["[1, 2, 3]", "[5, -1]", "[0]", "[3, 1, 2, 3]"]).to_string(),
            Kind::Texts => self.rng.pick(&["['b', 'a']", "['crow']", "['x', 'y', 'z']"]).to_string(),
            Kind::Counts => self.rng.pick(&["map[a = 1, b = 2]", "map[zz = 7]", "map[b = -4, a = 0]"]).to_string(),
            Kind::Doc => self
                .rng
                .pick(&[
                    "json['{\"a\": \"x\", \"n\": 3, \"l\": [1, 2]}']",
                    "json['{\"a\": \"crow\", \"n\": -2.5, \"l\": []}']",
                    "json['{\"a\": \"\", \"n\": 0, \"l\": [\"p\", 1]}']",
                ])
                .to_string(),
            Kind::Vec => self.rng.pick(&["[1.0, 2.0, 3.0]", "[0.5, -1.5, 2.0]", "zeros[3]", "ones[3]", "linspace[0.0, 1.0, 3]", "tensor[[2.0, 2.0, 2.0]]"]).to_string(),
            Kind::Mat => self.rng.pick(&["[[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]", "zeros[2, 3]", "ones[2, 3]", "reshape[linspace[0.0, 5.0, 6], 2, 3]", "[[0.5, -1.0, 2.0], [1.5, 0.0, -2.5]]"]).to_string(),
            Kind::Frame => self
                .rng
                .pick(&[
                    "with[with[with[with[frame.new[], 'k', ['x', 'y', 'x', 'y']], 'g', ['p', 'p', 'q', 'q']], 'a', [1, 2, 3, 4]], 'b', [0.5, 1.5, 2.5, 3.5]]",
                    "with[with[with[with[frame.new[], 'k', ['y', 'y', 'x', 'x']], 'g', ['q', 'p', 'q', 'p']], 'a', [4, 3, 2, 1]], 'b', [2.0, 2.0, 1.0, 0.0]]",
                ])
                .to_string(),
        }
    }

    fn fresh(&mut self, kind: Kind) -> String {
        let name = format!("v{}", self.made);
        self.made += 1;
        self.vars.push((name.clone(), kind));
        name
    }

    fn indent(&self) -> String {
        "    ".repeat(self.depth + 1)
    }

    /// A statement: a binding, a log, a branch, or a bounded loop.
    fn stmt(&mut self, budget: usize) {
        let pad = self.indent();
        match self.rng.below(if budget > 0 { 17 } else { 3 }) {
            // a vector walked: its numbers, each a float
            9 => {
                let v = self.expr(Kind::Vec, 2);
                let var = format!("x{}", self.lines.len());
                self.lines.push(format!("{}for {} in {}", pad, var, v));
                self.lines.push(format!("{}    log['`type[{}]` `{} * 2.0`']", pad, var, var));
            }
            // a matrix walked: its rows
            10 => {
                let m = self.expr(Kind::Mat, 2);
                let var = format!("r{}", self.lines.len());
                self.lines.push(format!("{}for {} in {}", pad, var, m));
                self.lines.push(format!("{}    log[sum[{}] + len[{}]]", pad, var, var));
            }
            // a vector's item assigned
            11 => {
                let v = self.expr(Kind::Vec, 2);
                let name = self.fresh(Kind::Vec);
                self.lines.push(format!("{}{} = {}", pad, name, v));
                let (at, val) = (self.rng.below(3), self.leaf(Kind::Float));
                self.lines.push(format!("{}{}[{}] = {}", pad, name, at, val));
                self.lines.push(format!("{}log[{}]", pad, name));
            }
            // a record holding tensors, its matrix's rows walked
            12 => {
                let (v, m) = (self.expr(Kind::Vec, 2), self.expr(Kind::Mat, 2));
                let bag = format!("g{}", self.lines.len());
                self.lines.push(format!("{}{} = Bag[{}, {}]", pad, bag, v, m));
                self.lines.push(format!("{}log[{}.t[1] + {}.m[1][2]]", pad, bag, bag));
                self.lines.push(format!("{}for r in {}.m", pad, bag));
                self.lines.push(format!("{}    log[type[r]]", pad));
            }
            // a frame's column walked, shown, extended, grouped, pivoted, windowed
            13 => {
                let df = self.var_frame(2);
                match self.rng.below(6) {
                    0 => {
                        self.lines.push(format!("{}for c in {}.a", pad, df));
                        self.lines.push(format!("{}    log[c * 2]", pad));
                    }
                    1 => self.lines.push(format!("{}log[{}]", pad, df)),
                    2 => {
                        let w = format!("w{}", self.lines.len());
                        self.lines.push(format!("{}{} = with[{}, 'c', {}.a * 2]", pad, w, df, df));
                        self.lines.push(format!("{}log['`{}.c[0]` `len[{}]` `{}.shape`']", pad, w, w, w));
                    }
                    3 => {
                        let g = format!("q{}", self.lines.len());
                        self.lines.push(format!("{}{} = group[{}, ['k', 'g'], ['a', 'sum'], ['b', 'mean']]", pad, g, df));
                        self.lines.push(format!("{}log['`len[{}]` `{}.k[0]` `{}.a_sum[0]` `{}.b_mean[0]`']", pad, g, g, g, g));
                    }
                    4 => {
                        let pv = format!("p{}", self.lines.len());
                        self.lines.push(format!("{}{} = pivot[head[{}, 4], 'k', 'g', 'b']", pad, pv, df));
                        self.lines.push(format!("{}log['`len[{}]` `{}.k[0]`']", pad, pv, pv));
                    }
                    _ => {
                        let w = format!("n{}", self.lines.len());
                        self.lines.push(format!("{}{} = with[{}, 'w', window[{}, 'a', 'sum', n = 2]]", pad, w, df, df));
                        self.lines.push(format!("{}log[{}.w[len[{}] - 1]]", pad, w, w));
                    }
                }
            }
            // a function handed as a value, through a relay too
            14 => {
                let (x, y) = (self.expr(Kind::Int, 2), self.expr(Kind::Int, 2));
                let h = self.rng.below(HELPERS as u64);
                let via = *self.rng.pick(&["ap", "relay"]);
                self.lines.push(format!("{}log[{}[h{}, {}, {}]]", pad, via, h, x, y));
            }
            // a type read of a tensor's or a frame's item (of variables: a
            // span holds no quotes, and a frame's literal has them)
            15 => {
                let (m, df) = (self.var_mat(1), self.var_frame(1));
                self.lines.push(format!("{}log['`type[{}[0]]` `type[{}.a[0]]` `type[{}.b[0]]`']", pad, m, df, df));
            }
            16 => {
                let m = self.var_mat(2);
                self.lines.push(format!("{}log[{}.shape[0] + len[{}]]", pad, m, m));
                self.lines.push(format!("{}log[round[sum[{} * transpose[{}]] * 100.0] / 100.0]", pad, m, m));
            }
            6 => {
                let list = self.expr(Kind::Ints, 2);
                let name = self.fresh(Kind::Ints);
                self.lines.push(format!("{}{} = {}", pad, name, list));
                self.lines.push(format!("{}if len[{}] gt 0", pad, name));
                self.lines.push(format!("{}    log[{}[0] + {}[len[{}] - 1]]", pad, name, name, name));
            }
            7 => {
                let w = self.fresh(Kind::Int);
                let n = 1 + self.rng.below(6);
                self.lines.push(format!("{}{} = 0", pad, w));
                self.lines.push(format!("{}while {} lt {}", pad, w, n));
                self.lines.push(format!("{}    {} = {} + 1", pad, w, w));
                self.lines.push(format!("{}log[{}]", pad, w));
            }
            8 => {
                let (x, y) = (self.expr(Kind::Int, 2), self.expr(Kind::Float, 2));
                let p = self.fresh(Kind::Int);
                self.lines.push(format!("{}{} = Point[{}, {}]", pad, p, x, y));
                // (a record holds no int a later expression reads as one)
                self.vars.retain(|(n, _)| n != &p);
                self.lines.push(format!("{}log[{}.x * 2]", pad, p));
                self.lines.push(format!("{}log[{}]", pad, p));
            }
            0 | 1 => {
                let kind = *self.rng.pick(KINDS);
                let e = self.expr(kind, 3);
                let name = self.fresh(kind);
                self.lines.push(format!("{}{} = {}", pad, name, e));
            }
            2 => {
                let kind = *self.rng.pick(KINDS);
                let e = self.expr(kind, 3);
                self.lines.push(format!("{}log[{}]", pad, e));
            }
            3 => {
                let c = self.expr(Kind::Bool, 2);
                self.lines.push(format!("{}if {}", pad, c));
                self.block(budget - 1);
                if self.rng.chance(50) {
                    self.lines.push(format!("{}else", pad));
                    self.block(budget - 1);
                }
            }
            4 => {
                let list = self.expr(Kind::Ints, 2);
                let var = format!("i{}", self.lines.len());
                self.lines.push(format!("{}for {} in {}", pad, var, list));
                self.vars.push((var.clone(), Kind::Int));
                self.block(budget - 1);
                self.vars.retain(|(n, _)| n != &var);
            }
            _ => {
                // an accumulator a loop grows
                let step = self.expr(Kind::Int, 2);
                let acc = self.fresh(Kind::Int);
                self.lines.push(format!("{}{} = 0", pad, acc));
                let var = format!("i{}", self.lines.len());
                let n = 1 + self.rng.below(5);
                self.lines.push(format!("{}for {} in range[{}]", pad, var, n));
                self.lines.push(format!("{}    {} = {} + {} + {}", pad, acc, acc, var, step));
                self.lines.push(format!("{}log[{}]", pad, acc));
            }
        }
    }

    fn block(&mut self, budget: usize) {
        let scope = self.vars.len();
        self.depth += 1;
        let n = 1 + self.rng.below(3);
        for _ in 0..n {
            self.stmt(budget);
        }
        self.depth -= 1;
        // a block's bindings are its own
        self.vars.truncate(scope);
    }
}

/// A program of `cases` generated cases, each its own function the top
/// level calls, from `seed`.
pub fn program(seed: u64, cases: usize) -> String {
    let mut rng = Rng::new(seed);
    let mut out = String::from("--- generated by crow test --fuzz\nuse math, math.nn, frame\n\nstruct Point [x, y]\nstruct Bag [t, m]\n\n");
    // a vector's numbers and a matrix's rows walked, typed by their calls
    out.push_str("fn t0[v]\n    return sum[v] * 2.0 + max[v]\n\nfn t1[m]\n    total = 0.0\n    for r in m\n        total = total + sum[r]\n    return total\n\n");
    // a function handed as a value, and handed on
    out.push_str("fn ap[f, a, b]\n    return f[a, b]\n\nfn relay[f, a, b]\n    return ap[f, a, b]\n\n");
    for h in 0..HELPERS {
        let mut case = Case { rng: &mut rng, vars: vec![("a".to_string(), Kind::Int), ("b".to_string(), Kind::Int)], lines: Vec::new(), depth: 0, made: 0 };
        // a helper calls no helper (no recursion without a base)
        let body = case.expr_no_calls(Kind::Int, 2);
        out.push_str(&format!("fn h{}[a, b]\n    return {}\n\n", h, body));
    }
    for c in 0..cases {
        let mut case = Case { rng: &mut rng, vars: Vec::new(), lines: Vec::new(), depth: 0, made: 0 };
        let n = 3 + case.rng.below(6);
        for _ in 0..n {
            case.stmt(2);
        }
        out.push_str(&format!("fn case{}[]\n    log['case {}']\n", c, c));
        for l in &case.lines {
            out.push_str(l);
            out.push('\n');
        }
        out.push_str("    return 0\n\n");
    }
    for c in 0..cases {
        out.push_str(&format!("r{} = case{}[]\n", c, c));
    }
    out
}


/// What a program said in one mode: whether it ended well, its stdout,
/// and its stderr's last line.
fn run(args: &[&str], file: &std::path::Path) -> (bool, String, String) {
    let exe = std::env::current_exe().unwrap_or_default();
    match std::process::Command::new(&exe).args(args).arg(file).env("CROW_BUILD_PROFILE", "dev").output() {
        Ok(o) => (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).to_string(),
            String::from_utf8_lossy(&o.stderr).lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").to_string(),
        ),
        Err(e) => (false, String::new(), format!("failed to spawn: {}", e)),
    }
}

/// `crow test --fuzz [n] [--seed s]`: n programs of twenty cases each,
/// from seed s (the time when none is given, which the report names so a
/// run can be repeated), each run by the interpreter and compiled. A
/// program the two disagree on is kept under `target/crow_fuzz/`, with the
/// first line they differ on; a program both refuse alike is counted (the
/// generator's slip, which the report shows).
pub fn cmd_test_fuzz(args: &[String]) {
    let n: usize = args.iter().filter(|a| !a.starts_with("--")).find_map(|a| a.parse().ok()).unwrap_or(10);
    let seed: u64 = args
        .iter()
        .position(|a| a == "--seed")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(1));
    let dir = std::path::Path::new("target").join("crow_fuzz");
    let _ = std::fs::create_dir_all(&dir);
    println!("fuzz: {} programs from seed {} (crow test --fuzz {} --seed {} repeats it)\n", n, seed, n, seed);
    let start = std::time::Instant::now();
    let (mut agreed, mut refused, mut diverged) = (0, 0, 0);
    for k in 0..n {
        let s = seed.wrapping_add(k as u64);
        let source = program(s, 20);
        let file = dir.join(format!("fuzz_{}.co", s));
        if std::fs::write(&file, &source).is_err() {
            eprintln!("error: cannot write {}", file.display());
            std::process::exit(1);
        }
        let (i_ok, i_out, i_err) = run(&["run", "-i"], &file);
        let (c_ok, c_out, c_err) = run(&["run"], &file);
        let crow_bug = |e: &str| e.contains("a bug in Crow") || e.contains("panicked");
        if i_ok && c_ok && i_out == c_out {
            agreed += 1;
            let _ = std::fs::remove_file(&file);
            println!("  ok    seed {} ({} lines)", s, i_out.lines().count());
        } else if !i_ok && !c_ok && i_out == c_out && i_err == c_err && !crow_bug(&i_err) {
            refused += 1;
            let _ = std::fs::remove_file(&file);
            println!("  --    seed {} refused alike: {}", s, i_err);
        } else {
            diverged += 1;
            println!("  FAIL  seed {} — kept as {}", s, file.display());
            if !i_ok || crow_bug(&i_err) {
                println!("        interpreter: {}", i_err);
            }
            if !c_ok || crow_bug(&c_err) {
                println!("        compiled: {}", c_err);
            }
            if let Some(at) = i_out.lines().zip(c_out.lines()).position(|(a, b)| a != b) {
                println!("        line {}: interpreter {:?}, compiled {:?}", at + 1, i_out.lines().nth(at).unwrap_or(""), c_out.lines().nth(at).unwrap_or(""));
            } else if i_out.lines().count() != c_out.lines().count() {
                println!("        the interpreter said {} lines, the compiled program {}", i_out.lines().count(), c_out.lines().count());
            }
        }
    }
    println!("\n{} agreed, {} refused alike, {} diverged ({:.1}s)", agreed, refused, diverged, start.elapsed().as_secs_f64());
    if diverged > 0 {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same seed makes the same program, and a generated program is
    /// one the language takes: built, lowered and run by the interpreter
    #[test]
    fn a_seed_makes_a_program_the_language_takes() {
        assert_eq!(program(7, 5), program(7, 5));
        assert_ne!(program(7, 5), program(8, 5));
        let mut taken = 0;
        let mut why = Vec::new();
        for seed in 0..40 {
            let src = program(seed, 6);
            let stmts = crate::build(&src).unwrap_or_else(|e| panic!("seed {}: {}\n{}", seed, e, src));
            match crate::ir_interp::run_script(&stmts, "fuzz") {
                Ok(_) => taken += 1,
                Err(e) => why.push(format!("seed {}: {}", seed, e)),
            }
        }
        assert!(taken >= 36, "most generated programs run: {} of 40, and the first refused:\n{}", taken, why.iter().take(4).cloned().collect::<Vec<_>>().join("\n"));
        // the kinds that found bugs are generated: a parsed document, and its
        // members spliced into text (compiled, they had shown JSON's quotes)
        let all: String = (0..120).map(|s| program(s, 6)).collect();
        assert!(all.contains("json['{"), "no parsed document generated");
        assert!(all.contains(".a` n `"), "no document's members spliced into text");
        // and the kinds the week of 2026-10-03 found its bugs in: tensors
        // walked and indexed, frames filtered by their own columns, a
        // function handed as a value
        assert!(all.contains("for r") && all.contains("transpose["), "no matrix walked or multiplied");
        assert!(all.contains("filter[") && all.contains("frame.new[]"), "no frame built and filtered");
        assert!(all.contains("relay[h") || all.contains("ap[h"), "no function handed as a value");
    }

    /// Whatever the text, the compiler answers with a program or a reason:
    /// it never panics (tokens strung at random, a seed's worth)
    #[test]
    fn the_compiler_never_panics_on_any_text() {
        const WORDS: &[&str] = &[
            "x", "y", "f", "fn", "if", "else", "for", "in", "while", "return", "use", "math", "frame", "web", "ui", "view", "test", "derive",
            "struct", "S", "log", "len", "push", "map", "[", "]", "(", ")", "=", "+", "-", "*", "/", "%", "^", ".", ",", "eq", "lt", "and",
            "not", "1", "2.5", "-3", "'a'", "'`x`'", "true", "false", "\n", "\n    ", "\n        ", " ", "'", "`", "---", "@", "#", "{", "}",
        ];
        let mut rng = Rng::new(20260930);
        for _ in 0..3000 {
            let n = 1 + rng.below(40) as usize;
            let src: String = (0..n).map(|_| *rng.pick(WORDS)).collect::<Vec<_>>().join(" ");
            let outcome = std::panic::catch_unwind(|| {
                if let Ok(stmts) = crate::build(&src) {
                    if let Ok(lowered) = crate::lower::lower_program(&stmts) {
                        let _ = crate::infer_and_validate(&lowered, false);
                    }
                }
            });
            assert!(outcome.is_ok(), "the compiler panicked on: {:?}", src);
        }
    }
}
