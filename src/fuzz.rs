//! `crow test --fuzz [n] [--seed s]`: differential fuzzing (docs/roadmap.md,
//! the 1.0 bar). Programs generated from a seed — ints, floats, text,
//! lists and maps, arithmetic and comparisons, the text and list builtins,
//! branches, loops and functions — each case a function of its own, all of
//! them in one program, run by the interpreter and compiled; a line where
//! the two disagree is a bug in Crow, and the program is kept to reproduce
//! it. The same seed makes the same programs.

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
}

const KINDS: &[Kind] = &[Kind::Int, Kind::Float, Kind::Text, Kind::Bool, Kind::Ints, Kind::Texts, Kind::Counts];

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
            Kind::Int => match self.rng.below(15) {
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
            Kind::Float => match self.rng.below(6) {
                0 => format!("({} + {})", self.expr(Kind::Float, d), self.expr(Kind::Float, d)),
                1 => format!("({} * {})", self.expr(Kind::Float, d), self.leaf(Kind::Float)),
                2 => format!("({} / {}.5)", self.expr(Kind::Float, d), 1 + self.rng.below(9)),
                3 => format!("float[{}]", self.expr(Kind::Int, d)),
                4 => format!("({} - {})", self.expr(Kind::Float, d), self.expr(Kind::Float, d)),
                _ => format!("({} + {})", self.expr(Kind::Float, d), self.expr(Kind::Int, d)),
            },
            Kind::Text => match self.rng.below(10) {
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
            Kind::Bool => match self.rng.below(10) {
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
            Kind::Texts => match self.rng.below(4) {
                3 => format!("keys[{}]", self.expr(Kind::Counts, d)),
                0 => format!("split[{}, ' ']", self.expr(Kind::Text, d)),
                1 => format!("push[{}, {}]", self.expr(Kind::Texts, d), self.expr(Kind::Text, d)),
                _ => format!("sort[{}]", self.expr(Kind::Texts, d)),
            },
        }
    }

    /// An int expression that calls no helper.
    fn expr_no_calls(&mut self, kind: Kind, depth: usize) -> String {
        loop {
            let e = self.expr(kind, depth);
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
        match self.rng.below(if budget > 0 { 9 } else { 3 }) {
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
                let acc = self.fresh(Kind::Int);
                self.lines.push(format!("{}{} = 0", pad, acc));
                let var = format!("i{}", self.lines.len());
                let n = 1 + self.rng.below(5);
                self.lines.push(format!("{}for {} in range[{}]", pad, var, n));
                let step = self.expr(Kind::Int, 2);
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
    let mut out = String::from("--- generated by crow test --fuzz\n\nstruct Point [x, y]\n\n");
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
