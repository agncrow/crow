//! Semantic tokens: every name of a document told by what it is — a
//! parameter, a variable, a function, a struct, a field, a module — for an
//! editor to colour by. A grammar sees text: it takes `b[0]` for a call, as
//! it takes `App[3, 2]`, and it cannot tell `draw` in `sys.frames[draw]`
//! from a variable. The server can: each name is read off the document's
//! tokens, as a rename reads them — its declarations, each declaration's
//! parameters and locals, the top level's facts, the modules it uses — and
//! off the analysis's functions and structs. Keywords, strings, numbers and
//! comments stay the grammar's, but for `test` and `as`, which are words
//! only where they declare.

use super::analysis::Analysis;
use crate::token::{SpannedToken, Token};
use lsp_types::{SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokensLegend};
use std::collections::{HashMap, HashSet};

/// What a name is: its token type, numbered by its place in the legend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Namespace,
    Struct,
    Parameter,
    Variable,
    Property,
    Function,
    Keyword,
}

/// The legend's token types, in `Kind`'s order.
const TYPES: [SemanticTokenType; 7] = [
    SemanticTokenType::NAMESPACE,
    SemanticTokenType::STRUCT,
    SemanticTokenType::PARAMETER,
    SemanticTokenType::VARIABLE,
    SemanticTokenType::PROPERTY,
    SemanticTokenType::FUNCTION,
    SemanticTokenType::KEYWORD,
];

/// Where a name is declared: a function's, a struct's or a derivation's
/// name, a parameter, a field, a module's name in its `use`, a variable's
/// first assignment.
pub const DECLARATION: u32 = 1;
/// A value nothing assigns again: a derivation, a constant, a module's value.
pub const READONLY: u32 = 1 << 1;
/// Crow's own: a builtin, a constant, a standard module's function.
pub const DEFAULT_LIBRARY: u32 = 1 << 2;

/// The legend's modifiers, each the bit above.
const MODIFIERS: [SemanticTokenModifier; 3] =
    [SemanticTokenModifier::DECLARATION, SemanticTokenModifier::READONLY, SemanticTokenModifier::DEFAULT_LIBRARY];

/// The core's constants, read as values where they are not called.
const CONSTANTS: [&str; 4] = ["pi", "e", "inf", "nan"];

/// The token types and modifiers the server tells names by, as it
/// announces them: the client numbers each token by them.
pub fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend { token_types: TYPES.to_vec(), token_modifiers: MODIFIERS.to_vec() }
}

/// A name told: its line and its first character's column, both from 1,
/// the column in characters as the lexer counts them, its length in
/// characters, and what it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Told {
    pub line: usize,
    pub col: usize,
    pub len: usize,
    pub kind: Kind,
    pub mods: u32,
}

/// The document's names as the protocol sends them: each one's place
/// counted from the one before it, in UTF-16 units.
pub fn tokens(a: &Analysis) -> Vec<SemanticToken> {
    encode(&a.source, &told(a))
}

/// Whether the document does not lex, so no name of it is known: the
/// lexer refuses a whole file for one stray character, and a client keeps
/// the colours it has until the document lexes again (the server answers
/// that it has none to give, not that there are none).
pub fn unlexed(a: &Analysis) -> bool {
    a.tokens.is_empty() && !a.source.trim().is_empty()
}

/// Every name of the document told, in order.
pub fn told(a: &Analysis) -> Vec<Told> {
    if a.tokens.is_empty() {
        return Vec::new();
    }
    let doc = Doc::read(a);
    let mut out = Vec::new();
    doc.tell(&mut out);
    out.sort_by_key(|t| (t.line, t.col));
    out
}

/// What a top-level declaration's lines are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Decl {
    /// the top level's own statements, and the lines indented under them
    Top,
    /// a `fn` or a `view`
    Fn,
    Test,
    Derive,
    Struct,
    Use,
}

/// A declaration's tokens: from its first, at column 1, to the next
/// declaration's first (a declaration runs to the next line that starts at
/// column 1, as a rename reads it), with the names it binds.
struct Region {
    decl: Decl,
    start: usize,
    end: usize,
    params: HashSet<String>,
    locals: HashSet<String>,
}

/// What the document declares and binds, read before any name is told:
/// a name may be read above the line that declares it.
struct Doc<'a> {
    a: &'a Analysis,
    toks: &'a [SpannedToken],
    regions: Vec<Region>,
    /// each token's region
    region_of: Vec<usize>,
    /// what a token is where its place alone says: a declaration's name,
    /// a parameter, a field, a module's path, a variable's first assignment
    fixed: Vec<Option<(Kind, u32)>>,
    /// whether a token starts a statement: the first on its line, outside
    /// brackets
    starts: Vec<bool>,
    /// the modules the document uses, by the name it calls each, and
    /// whether each is one of Crow's own
    modules: HashMap<String, bool>,
    /// the names the standard modules it uses make, bare where a module's
    /// bare import makes them
    library: HashSet<&'static str>,
    fns: HashSet<String>,
    structs: HashSet<String>,
    derives: HashSet<String>,
    /// the names the top level assigns: its facts, and its loops' names
    facts: HashSet<String>,
    /// the names a part's functions are called by after the part's name
    /// (`total` of `calc.total`)
    members: HashSet<String>,
    /// the source's lines as characters, where a string interpolates
    lines: Vec<Vec<char>>,
}

impl<'a> Doc<'a> {
    fn read(a: &'a Analysis) -> Doc<'a> {
        let toks = &a.tokens[..];
        let n = toks.len();
        let tok = |k: usize| toks.get(k).map(|t| &t.token);
        let mut fixed: Vec<Option<(Kind, u32)>> = vec![None; n];

        // the declarations, and where statements start: a declaration's
        // word at column 1 starts one whatever brackets are open, so one
        // left open as it is typed does not run the rest of the file into
        // one declaration
        let mut starts = vec![false; n];
        let mut firsts: Vec<usize> = vec![0];
        let (mut depth, mut line) = (0usize, 0usize);
        for (i, t) in toks.iter().enumerate() {
            if matches!(t.token, Token::Newline | Token::Indent | Token::Dedent | Token::Eof) {
                continue;
            }
            if t.col == 1 && (depth == 0 || opens_decl(toks, i)) {
                depth = 0;
                if i > 0 {
                    firsts.push(i);
                }
            }
            starts[i] = depth == 0 && t.line != line;
            line = t.line;
            match t.token {
                Token::LBracket | Token::LParen => depth += 1,
                Token::RBracket | Token::RParen => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        let mut regions: Vec<Region> = firsts
            .iter()
            .enumerate()
            .map(|(k, &s)| Region { decl: decl_at(toks, s), start: s, end: firsts.get(k + 1).copied().unwrap_or(n), params: HashSet::new(), locals: HashSet::new() })
            .collect();
        let mut region_of = vec![0usize; n];
        for (k, r) in regions.iter().enumerate() {
            region_of[r.start..r.end].fill(k);
        }

        // the modules the document uses, by the names it calls them, and
        // the names Crow's own modules make (`recall` of `use memory`)
        let mut modules = HashMap::new();
        let mut standard = Vec::new();
        for (i, t) in toks.iter().enumerate() {
            if matches!(t.token, Token::Use) {
                read_use(toks, i, &mut fixed, &mut modules, &mut standard);
            }
        }
        for name in a.parts.keys() {
            modules.entry(name.clone()).or_insert(false);
        }
        let library: HashSet<&'static str> = standard.iter().flat_map(|path| crate::lower_ir::names_of(path)).collect();

        // each declaration's name and parameters, then the names its lines
        // assign
        let (mut fns, mut structs, mut derives, mut facts) = (HashSet::new(), HashSet::new(), HashSet::new(), HashSet::new());
        for r in regions.iter_mut() {
            let mut body = r.start;
            if matches!(r.decl, Decl::Use) {
                continue;
            }
            if !matches!(r.decl, Decl::Top) {
                if r.decl == Decl::Test {
                    fixed[r.start] = Some((Kind::Keyword, 0));
                }
                let mut j = r.start + 1;
                if let Some(Token::Ident(name)) = tok(j) {
                    let (kind, names) = match r.decl {
                        Decl::Struct => ((Kind::Struct, DECLARATION), &mut structs),
                        Decl::Derive => ((Kind::Variable, READONLY | DECLARATION), &mut derives),
                        _ => ((Kind::Function, DECLARATION), &mut fns),
                    };
                    fixed[j] = Some(kind);
                    names.insert(name.clone());
                    j += 1;
                }
                body = j;
                // its brackets: a function's or a test's parameters, a
                // struct's fields, a derivation's options
                if matches!(tok(j), Some(Token::LBracket)) {
                    let close = closing(toks, j, r.end);
                    let mut d = 0usize;
                    for k in j + 1..close {
                        match &toks[k].token {
                            Token::LBracket | Token::LParen => d += 1,
                            Token::RBracket | Token::RParen => d = d.saturating_sub(1),
                            Token::Ident(p) if d == 0 && matches!(tok(k - 1), Some(Token::LBracket | Token::Comma)) => {
                                fixed[k] = Some(match r.decl {
                                    Decl::Struct => (Kind::Property, DECLARATION),
                                    Decl::Derive => (Kind::Parameter, 0),
                                    _ => {
                                        r.params.insert(p.clone());
                                        (Kind::Parameter, DECLARATION)
                                    }
                                });
                            }
                            _ => {}
                        }
                    }
                    body = close + 1;
                }
                if r.decl == Decl::Struct {
                    continue;
                }
            }
            for i in body..r.end {
                if starts[i]
                    && let Some((names, plain)) = assigned(toks, i, r.end, &starts)
                {
                    for k in names {
                        // (a module's member is assigned into by no program)
                        if !plain && matches!(tok(k), Some(Token::Ident(m)) if modules.contains_key(m)) {
                            continue;
                        }
                        bind(toks, k, r, &mut facts, &mut fixed, plain);
                    }
                }
                if matches!(toks[i].token, Token::For) {
                    bind(toks, i + 1, r, &mut facts, &mut fixed, true);
                }
            }
        }

        let members = a.ctx.functions.keys().filter_map(|f| f.rsplit_once(crate::PART_JOIN).map(|(_, m)| m.to_string())).collect();
        let interpolates = toks.iter().any(|t| matches!(&t.token, Token::Str(s) if s.contains('`')));
        let lines = if interpolates { a.source.split('\n').map(|l| l.chars().collect()).collect() } else { Vec::new() };
        Doc { a, toks, regions, region_of, fixed, starts, modules, library, fns, structs, derives, facts, members, lines }
    }

    /// Every name told, in the order of the tokens, a string's
    /// interpolations where the string is.
    fn tell(&self, out: &mut Vec<Told>) {
        let mut region = usize::MAX;
        let mut callees: Vec<Option<Kind>> = Vec::new();
        let mut last: Option<Kind> = None;
        for (i, t) in self.toks.iter().enumerate() {
            if self.region_of[i] != region {
                region = self.region_of[i];
                callees.clear();
            }
            let mut now = None;
            match &t.token {
                Token::Ident(name) => {
                    let told = self.fixed[i].or_else(|| self.name(self.toks, i, &callees, &self.regions[region], self.starts[i]));
                    if let Some((kind, mods)) = told {
                        out.push(Told { line: t.line, col: t.col, len: name.chars().count(), kind, mods });
                        now = Some(kind);
                    }
                }
                Token::LBracket => callees.push(opened(last, i.checked_sub(1).map(|k| &self.toks[k].token))),
                Token::LParen => callees.push(None),
                Token::RBracket | Token::RParen => {
                    callees.pop();
                }
                Token::Str(s) if s.contains('`') => self.pieces(t, &self.regions[region], out),
                _ => {}
            }
            last = now;
        }
    }

    /// A string's interpolations, each told as the code it is:
    /// `'`a.copies` copies'` reads the variable `a` and its field.
    fn pieces(&self, t: &SpannedToken, region: &Region, out: &mut Vec<Told>) {
        for (line, col, code) in pieces_at(&self.lines, t.line, t.col) {
            let Ok(toks) = crate::lexer::Lexer::new(&code).tokenize() else { continue };
            let mut callees: Vec<Option<Kind>> = Vec::new();
            let mut last: Option<Kind> = None;
            for (i, p) in toks.iter().enumerate() {
                let mut now = None;
                match &p.token {
                    Token::Ident(name) => {
                        if let Some((kind, mods)) = self.name(&toks, i, &callees, region, false) {
                            let (l, c) = if p.line == 1 { (line, col + p.col - 1) } else { (line + p.line - 1, p.col) };
                            out.push(Told { line: l, col: c, len: name.chars().count(), kind, mods });
                            now = Some(kind);
                        }
                    }
                    Token::LBracket => callees.push(opened(last, i.checked_sub(1).map(|k| &toks[k].token))),
                    Token::LParen => callees.push(None),
                    Token::RBracket | Token::RParen => {
                        callees.pop();
                    }
                    _ => {}
                }
                last = now;
            }
        }
    }

    /// What the name at `toks[i]` is where it is read: a module's member,
    /// a value's field, a named argument's key, or what its declaration
    /// says. `callees` is what each open bracket follows; `start`, whether
    /// the name starts a statement.
    fn name(&self, toks: &[SpannedToken], i: usize, callees: &[Option<Kind>], region: &Region, start: bool) -> Option<(Kind, u32)> {
        let Token::Ident(n) = &toks[i].token else { return None };
        // (a column's name with a computed piece, `df.lag_`k``, is the
        // grammar's to colour)
        if n.contains('`') {
            return None;
        }
        let tok = |k: Option<usize>| k.and_then(|k| toks.get(k)).map(|t| &t.token);
        let prev = tok(i.checked_sub(1));
        let next = tok(Some(i + 1));
        let calls = matches!(next, Some(Token::LBracket));
        let capital = n.starts_with(|c: char| c.is_ascii_uppercase());
        // after a dot: a module's function, struct or value, or a field
        if matches!(prev, Some(Token::Dot)) {
            if let Some(Token::Ident(m)) = tok(i.checked_sub(2)) {
                let chained = matches!(tok(i.checked_sub(3)), Some(Token::Dot));
                if let (false, Some(own)) = (chained, self.module(m, region)) {
                    let lib = if own { DEFAULT_LIBRARY } else { 0 };
                    return Some(if capital {
                        (Kind::Struct, lib)
                    } else if calls || self.members.contains(n.as_str()) {
                        (Kind::Function, lib)
                    } else {
                        (Kind::Variable, READONLY | lib)
                    });
                }
            }
            return Some((Kind::Property, 0));
        }
        // a named argument's key: a parameter of the function called, a
        // field of the struct made
        if matches!(next, Some(Token::Assign))
            && matches!(prev, Some(Token::LBracket | Token::Comma))
            && let Some(Some(callee)) = callees.last()
        {
            return Some(if *callee == Kind::Struct { (Kind::Property, 0) } else { (Kind::Parameter, 0) });
        }
        // a test that starts below the top level
        if start && n == "test" && matches!(next, None | Some(Token::Ident(_) | Token::LBracket | Token::Newline | Token::Eof)) {
            return Some((Kind::Keyword, 0));
        }
        // before a dot: a module, where no value here has its name
        if matches!(next, Some(Token::Dot)) && self.module(n, region).is_some() {
            return Some((Kind::Namespace, 0));
        }
        Some(self.lookup(n, region, prev, calls, capital))
    }

    /// A plain name, by what it names here: a parameter or a local of the
    /// declaration it is in, then what the document declares, then Crow's
    /// own names. One that names nothing known is a function where it is
    /// called (a struct, capitalised), and a variable elsewhere.
    fn lookup(&self, n: &str, region: &Region, prev: Option<&Token>, calls: bool, capital: bool) -> (Kind, u32) {
        if region.params.contains(n) {
            return (Kind::Parameter, 0);
        }
        if region.locals.contains(n) || (region.decl == Decl::Top && self.facts.contains(n)) {
            return (Kind::Variable, 0);
        }
        if self.fns.contains(n) {
            return (Kind::Function, 0);
        }
        if self.structs.contains(n) {
            return (Kind::Struct, 0);
        }
        if self.derives.contains(n) {
            return (Kind::Variable, READONLY);
        }
        // (a view and a handler read the top level's state)
        if self.facts.contains(n) {
            return (Kind::Variable, 0);
        }
        if self.modules.contains_key(n) {
            return (Kind::Namespace, 0);
        }
        if !calls && CONSTANTS.contains(&n) {
            return (Kind::Variable, READONLY | DEFAULT_LIBRARY);
        }
        if crate::lower_ir::BUILTINS.binary_search(&n).is_ok() || self.library.contains(n) {
            return (Kind::Function, DEFAULT_LIBRARY);
        }
        if self.a.ctx.functions.contains_key(n) {
            return (Kind::Function, 0);
        }
        if self.a.ctx.structs.contains_key(n) {
            return (Kind::Struct, 0);
        }
        // a chain's step that is a function's name: `s then trim`
        if matches!(prev, Some(Token::Then)) {
            return (Kind::Function, 0);
        }
        match (calls, capital) {
            (true, true) => (Kind::Struct, 0),
            (true, false) => (Kind::Function, 0),
            _ => (Kind::Variable, 0),
        }
    }

    /// Whether `m`, before a dot, names a module here, and whether the
    /// module is one of Crow's own: one the document uses, or a standard
    /// module the tooling knows without an import (`db`). A parameter, a
    /// local or a fact of the name is a value, its fields read by the dot.
    fn module(&self, m: &str, region: &Region) -> Option<bool> {
        if region.params.contains(m) || region.locals.contains(m) || (region.decl == Decl::Top && self.facts.contains(m)) {
            return None;
        }
        if let Some(own) = self.modules.get(m) {
            return Some(*own);
        }
        crate::modules::by_path(m).map(|_| true)
    }
}

/// What a `[` follows, for its named arguments' keys: the kind of the name
/// before it — a function's keys are its parameters — with a map made
/// (`map[k = v]`) read as a struct is, its keys its fields.
fn opened(last: Option<Kind>, prev: Option<&Token>) -> Option<Kind> {
    match (last, prev) {
        (Some(Kind::Function), Some(Token::Ident(n))) if n == "map" => Some(Kind::Struct),
        _ => last,
    }
}

/// Whether the token at `i` starts a declaration: `fn`, `view`, `derive`,
/// `struct`, `use`, or `test` as the parser reads it.
fn opens_decl(toks: &[SpannedToken], i: usize) -> bool {
    matches!(toks[i].token, Token::Fn | Token::View | Token::Derive | Token::Struct | Token::Use) || opens_test(toks, i)
}

/// `test` where a test starts: before its name, its parameters or its
/// lines (the parser's reading; elsewhere `test` is a name).
fn opens_test(toks: &[SpannedToken], i: usize) -> bool {
    matches!(&toks[i].token, Token::Ident(w) if w == "test")
        && matches!(toks.get(i + 1).map(|t| &t.token), None | Some(Token::Ident(_) | Token::LBracket | Token::Newline | Token::Eof))
}

fn decl_at(toks: &[SpannedToken], i: usize) -> Decl {
    match toks[i].token {
        Token::Fn | Token::View => Decl::Fn,
        Token::Derive => Decl::Derive,
        Token::Struct => Decl::Struct,
        Token::Use => Decl::Use,
        _ if opens_test(toks, i) => Decl::Test,
        _ => Decl::Top,
    }
}

/// The `]` that closes the `[` at `open`, or `end` where none does before it.
fn closing(toks: &[SpannedToken], open: usize, end: usize) -> usize {
    let mut depth = 0usize;
    for (k, t) in toks.iter().enumerate().take(end).skip(open) {
        match t.token {
            Token::LBracket | Token::LParen => depth += 1,
            Token::RBracket | Token::RParen => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return k;
                }
            }
            _ => {}
        }
    }
    end
}

/// A `use` line's modules: each path's pieces are a module's, and the name
/// the document calls the module by — its alias, or its path's last piece —
/// is declared there. The paths of Crow's own modules go to `standard`.
fn read_use(toks: &[SpannedToken], at: usize, fixed: &mut [Option<(Kind, u32)>], modules: &mut HashMap<String, bool>, standard: &mut Vec<String>) {
    let tok = |k: usize| toks.get(k).map(|t| &t.token);
    let mut j = at + 1;
    loop {
        let mut path = String::new();
        let mut named = None;
        while let Some(Token::Ident(piece)) = tok(j) {
            fixed[j] = Some((Kind::Namespace, 0));
            path.push_str(piece);
            named = Some(j);
            j += 1;
            if matches!(tok(j), Some(Token::Dot | Token::Slash)) && matches!(tok(j + 1), Some(Token::Ident(_))) {
                path.push('.');
                j += 1;
            } else {
                break;
            }
        }
        let Some(mut named) = named else { return };
        if matches!(tok(j), Some(Token::Ident(w)) if w == "as") && matches!(tok(j + 1), Some(Token::Ident(_))) {
            fixed[j] = Some((Kind::Keyword, 0));
            named = j + 1;
            j += 2;
        }
        if let Some(Token::Ident(name)) = tok(named) {
            fixed[named] = Some((Kind::Namespace, DECLARATION));
            let own = crate::modules::by_path(&path).is_some();
            modules.insert(name.clone(), own);
            if own {
                standard.push(path);
            }
        }
        if !matches!(tok(j), Some(Token::Comma)) {
            return;
        }
        j += 1;
    }
}

/// What the statement starting at `s` assigns, and whether it gives each
/// a value: the names before its `=` (`x = …`, `x, y = …`), or the one it
/// assigns into (`x.f = …`, `x[k] = …`); none for a statement that assigns
/// nothing.
fn assigned(toks: &[SpannedToken], s: usize, end: usize, starts: &[bool]) -> Option<(Vec<usize>, bool)> {
    if !matches!(toks[s].token, Token::Ident(_)) {
        return None;
    }
    let mut depth = 0usize;
    let mut j = s;
    while j < end && (j == s || !starts[j]) {
        match toks[j].token {
            Token::LBracket | Token::LParen => depth += 1,
            Token::RBracket | Token::RParen => depth = depth.saturating_sub(1),
            Token::Assign if depth == 0 => {
                let listed = (j - s) % 2 == 1
                    && (s..j).all(|k| if (k - s).is_multiple_of(2) { matches!(toks[k].token, Token::Ident(_)) } else { matches!(toks[k].token, Token::Comma) });
                return Some(if listed { ((s..j).step_by(2).collect(), true) } else { (vec![s], false) });
            }
            Token::Newline => break,
            _ => {}
        }
        j += 1;
    }
    None
}

/// A name a statement binds: a local of its declaration's, or a fact of
/// the top level's — declared where it is first given a value.
fn bind(toks: &[SpannedToken], k: usize, r: &mut Region, facts: &mut HashSet<String>, fixed: &mut [Option<(Kind, u32)>], gives: bool) {
    let Some(Token::Ident(name)) = toks.get(k).map(|t| &t.token) else { return };
    if name.contains('`') || r.params.contains(name) {
        return;
    }
    let scope = if r.decl == Decl::Top { facts } else { &mut r.locals };
    if scope.insert(name.clone()) && gives {
        fixed[k] = Some((Kind::Variable, DECLARATION));
    }
}

/// The interpolations of the string whose quote is at (line, col), both
/// from 1: each one's first character's line and column, and its code.
/// Read as the lexer reads a string: a backslash takes the character after
/// it, a backtick opens or closes an interpolation, a quote ends the string.
fn pieces_at(lines: &[Vec<char>], line: usize, col: usize) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut open: Option<(usize, usize, String)> = None;
    // (0-based: the line, and the character after the quote)
    let (mut l, mut c) = (line - 1, col);
    while let Some(text) = lines.get(l) {
        let Some(&ch) = text.get(c) else {
            // the string goes on on the next line
            if let Some((_, _, code)) = open.as_mut() {
                code.push('\n');
            }
            l += 1;
            c = 0;
            continue;
        };
        match ch {
            '\\' => {
                if let Some((_, _, code)) = open.as_mut() {
                    code.extend(text[c..text.len().min(c + 2)].iter());
                }
                c += 2;
            }
            '`' => {
                match open.take() {
                    Some(piece) => out.push(piece),
                    None => open = Some((l + 1, c + 2, String::new())),
                }
                c += 1;
            }
            '\'' => break,
            _ => {
                if let Some((_, _, code)) = open.as_mut() {
                    code.push(ch);
                }
                c += 1;
            }
        }
    }
    out
}

/// The names in the protocol's form: each one's line and start counted
/// from the one before it, and its start and length in UTF-16 units.
fn encode(source: &str, told: &[Told]) -> Vec<SemanticToken> {
    let lines: Vec<&str> = source.split('\n').collect();
    let mut out = Vec::with_capacity(told.len());
    let (mut prev_line, mut prev_start) = (0u32, 0u32);
    // (the line being encoded, and its characters' UTF-16 offsets where it
    // is not ASCII)
    let mut units: (usize, Option<Vec<u32>>) = (usize::MAX, None);
    for t in told {
        let text = lines.get(t.line - 1).copied().unwrap_or("");
        if units.0 != t.line {
            units = (t.line, (!text.is_ascii()).then(|| utf16_offsets(text)));
        }
        let (from, to) = (t.col - 1, t.col - 1 + t.len);
        let (start, end) = match &units.1 {
            None => (from as u32, to as u32),
            Some(o) => (o[from.min(o.len() - 1)], o[to.min(o.len() - 1)]),
        };
        let line = (t.line - 1) as u32;
        out.push(SemanticToken {
            delta_line: line - prev_line,
            delta_start: if line == prev_line { start - prev_start } else { start },
            length: end - start,
            token_type: t.kind as u32,
            token_modifiers_bitset: t.mods,
        });
        (prev_line, prev_start) = (line, start);
    }
    out
}

/// Each character's offset in a line, in UTF-16 units, and the line's end.
fn utf16_offsets(text: &str) -> Vec<u32> {
    let mut out = Vec::with_capacity(text.len() + 1);
    let mut at = 0u32;
    for ch in text.chars() {
        out.push(at);
        at += ch.len_utf16() as u32;
    }
    out.push(at);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::analysis::analyze;

    /// Each name told, as its text, kind and modifiers, in order.
    fn named(src: &str) -> Vec<(String, Kind, u32)> {
        let a = analyze(src, None, None, "t.co");
        let lines: Vec<Vec<char>> = src.split('\n').map(|l| l.chars().collect()).collect();
        told(&a).into_iter().map(|t| (lines[t.line - 1][t.col - 1..t.col - 1 + t.len].iter().collect(), t.kind, t.mods)).collect()
    }

    /// The kinds told of a word, each time it is told, in order.
    fn of(names: &[(String, Kind, u32)], word: &str) -> Vec<(Kind, u32)> {
        names.iter().filter(|(w, _, _)| w == word).map(|(_, k, m)| (*k, *m)).collect()
    }

    use Kind::*;

    /// What a grammar misreads: a variable indexed is a variable, a struct
    /// made is a struct, a function handed on is a function, a field read
    /// after a dot is a field, and a standard module's function is Crow's
    #[test]
    fn a_name_is_told_by_what_it_names() {
        let src = "\
use sys

struct App [copies, stars]

fn fresh[]
    App[3, 2]

fn first[b]
    b[0]

fn draw[frame]
    a = fresh[]
    a.copies = first[[a.stars, len[frame]]]
    a

sys.frames[draw]
";
        let n = named(src);
        assert_eq!(of(&n, "sys"), vec![(Namespace, DECLARATION), (Namespace, 0)]);
        assert_eq!(of(&n, "App"), vec![(Struct, DECLARATION), (Struct, 0)]);
        assert_eq!(of(&n, "copies"), vec![(Property, DECLARATION), (Property, 0)]);
        assert_eq!(of(&n, "stars"), vec![(Property, DECLARATION), (Property, 0)]);
        assert_eq!(of(&n, "fresh"), vec![(Function, DECLARATION), (Function, 0)]);
        assert_eq!(of(&n, "b"), vec![(Parameter, DECLARATION), (Parameter, 0)]);
        assert_eq!(of(&n, "a"), vec![(Variable, DECLARATION), (Variable, 0), (Variable, 0), (Variable, 0)]);
        assert_eq!(of(&n, "first"), vec![(Function, DECLARATION), (Function, 0)]);
        assert_eq!(of(&n, "len"), vec![(Function, DEFAULT_LIBRARY)]);
        assert_eq!(of(&n, "frames"), vec![(Function, DEFAULT_LIBRARY)]);
        assert_eq!(of(&n, "draw"), vec![(Function, DECLARATION), (Function, 0)]);
        assert_eq!(of(&n, "frame"), vec![(Parameter, DECLARATION), (Parameter, 0)]);
    }

    /// A local is its function's: the same name is a parameter of another
    /// function, and a fact of the top level
    #[test]
    fn a_name_is_what_it_is_in_its_declaration() {
        let n = named("fn one[]\n    t = 1\n    t\n\nfn two[t]\n    t\n\nt = 3\nlog[t]\n");
        assert_eq!(
            of(&n, "t"),
            vec![(Variable, DECLARATION), (Variable, 0), (Parameter, DECLARATION), (Parameter, 0), (Variable, DECLARATION), (Variable, 0)]
        );
    }

    /// A named argument's key is a parameter of the function called, a
    /// field of the struct made and a key of the map made; a parameter's
    /// default is its declaration's
    #[test]
    fn a_named_arguments_key_is_a_parameter_or_a_field() {
        let n = named("fn go[speed, far = 2]\n    speed + far\n\nstruct P [x, y]\n\nlog[go[1, far = 3]]\np = P[x = 1, y = 2]\nlog[p.x]\nm = map[x = 1, w = go[speed = 2]]\n");
        assert_eq!(of(&n, "far"), vec![(Parameter, DECLARATION), (Parameter, 0), (Parameter, 0)]);
        assert_eq!(of(&n, "x"), vec![(Property, DECLARATION), (Property, 0), (Property, 0), (Property, 0)]);
        assert_eq!(of(&n, "w"), vec![(Property, 0)]);
        assert_eq!(of(&n, "speed"), vec![(Parameter, DECLARATION), (Parameter, 0), (Parameter, 0)]);
        assert_eq!(of(&n, "p"), vec![(Variable, DECLARATION), (Variable, 0)]);
        assert_eq!(of(&n, "log"), vec![(Function, DEFAULT_LIBRARY), (Function, DEFAULT_LIBRARY)]);
    }

    /// A `use` line's paths are modules, its alias declared and `as` its
    /// word; a module's member is its function, struct or value
    #[test]
    fn modules_and_their_members() {
        let n = named("use lory.gui\nuse lib.calc as c\nuse memory\nx = gui.row[[gui.Box[1]]]\ny = c.total\nz = math.pi\nw = recall['w', 0]\n");
        // (a standard module's name, made bare by its import, is Crow's)
        assert_eq!(of(&n, "recall"), vec![(Function, DEFAULT_LIBRARY)]);
        assert_eq!(of(&n, "lory"), vec![(Namespace, 0)]);
        assert_eq!(of(&n, "gui"), vec![(Namespace, DECLARATION), (Namespace, 0), (Namespace, 0)]);
        assert_eq!(of(&n, "as"), vec![(Keyword, 0)]);
        assert_eq!(of(&n, "c"), vec![(Namespace, DECLARATION), (Namespace, 0)]);
        assert_eq!(of(&n, "row"), vec![(Function, 0)]);
        assert_eq!(of(&n, "Box"), vec![(Struct, 0)]);
        assert_eq!(of(&n, "total"), vec![(Variable, READONLY)]);
        // (a standard module read without its import is still Crow's)
        assert_eq!(of(&n, "math"), vec![(Namespace, 0)]);
        assert_eq!(of(&n, "pi"), vec![(Variable, READONLY | DEFAULT_LIBRARY)]);
    }

    /// `test` is a word where a test starts and its name a declaration; a
    /// derivation is a value nothing assigns, its option a key
    #[test]
    fn tests_and_derivations() {
        let n = named("derive total[fresh = 5]\n    3\n\ntest adds_up[n]\n    n gt 0\n    total gt 0\n");
        assert_eq!(of(&n, "total"), vec![(Variable, READONLY | DECLARATION), (Variable, READONLY)]);
        assert_eq!(of(&n, "fresh"), vec![(Parameter, 0)]);
        assert_eq!(of(&n, "test"), vec![(Keyword, 0)]);
        assert_eq!(of(&n, "adds_up"), vec![(Function, DECLARATION)]);
        assert_eq!(of(&n, "n"), vec![(Parameter, DECLARATION), (Parameter, 0)]);
    }

    /// A loop's name is a local; a chain's step names a function, and a
    /// constant is Crow's own value
    #[test]
    fn loops_chains_and_constants() {
        let n = named("fn twice[x]\n    x * 2\n\nfn f[xs]\n    out = for x in xs\n        x * pi\n    n = xs then len\n    m = n then twice\n    out\n");
        assert_eq!(of(&n, "x"), vec![(Parameter, DECLARATION), (Parameter, 0), (Variable, DECLARATION), (Variable, 0)]);
        assert_eq!(of(&n, "len"), vec![(Function, DEFAULT_LIBRARY)]);
        assert_eq!(of(&n, "twice"), vec![(Function, DECLARATION), (Function, 0)]);
        assert_eq!(of(&n, "pi"), vec![(Variable, READONLY | DEFAULT_LIBRARY)]);
    }

    /// A string's interpolations are code: each name in them told where it
    /// is in the line, after text that is not ASCII too
    #[test]
    fn an_interpolations_names_are_told_where_they_are() {
        let src = "fn label[id, value]\n    '− `id`:less `value.size` \\` `len[id]`'\n";
        let a = analyze(src, None, None, "t.co");
        let lines: Vec<Vec<char>> = src.split('\n').map(|l| l.chars().collect()).collect();
        let told: Vec<(String, usize, Kind)> =
            told(&a).into_iter().filter(|t| t.line == 2).map(|t| (lines[1][t.col - 1..t.col - 1 + t.len].iter().collect(), t.col, t.kind)).collect();
        assert_eq!(
            told,
            vec![
                ("id".to_string(), 9, Parameter),
                ("value".to_string(), 19, Parameter),
                ("size".to_string(), 25, Property),
                ("len".to_string(), 35, Function),
                ("id".to_string(), 39, Parameter),
            ]
        );
    }

    /// A bracket left open as it is typed does not run the declarations
    /// below it together
    #[test]
    fn a_bracket_left_open_ends_at_the_next_declaration() {
        let n = named("x = log[1,\n\nfn g[y]\n    y\n");
        assert_eq!(of(&n, "y"), vec![(Parameter, DECLARATION), (Parameter, 0)]);
    }

    /// The protocol's numbers: each place counted from the one before, in
    /// UTF-16 units (an emoji is two)
    #[test]
    fn tokens_are_encoded_relative_and_in_utf16() {
        let a = analyze("y = 2\nlog['😀', y]\n", None, None, "t.co");
        let data: Vec<[u32; 5]> = tokens(&a).into_iter().map(|t| [t.delta_line, t.delta_start, t.length, t.token_type, t.token_modifiers_bitset]).collect();
        assert_eq!(
            data,
            vec![
                [0, 0, 1, Variable as u32, DECLARATION],
                [1, 0, 3, Function as u32, DEFAULT_LIBRARY],
                [0, 10, 1, Variable as u32, 0],
            ]
        );
    }

    /// A document the lexer refuses has no names to tell, which is not
    /// a document with none
    #[test]
    fn a_document_that_does_not_lex_is_told_so() {
        assert!(unlexed(&analyze("x = 'never closed\n", None, None, "t.co")));
        assert!(!unlexed(&analyze("\n", None, None, "t.co")));
        assert!(!unlexed(&analyze("x = 1\n", None, None, "t.co")));
    }

    /// Every program of the checkout's own told without a fault: each
    /// name's place holds that name, in order, and a word of the language
    /// is told only as one. (On a thread of its own: an analysis of a deep
    /// program, unoptimised, needs more stack than a test's thread has.)
    #[test]
    fn every_program_in_the_checkout_is_told_in_place() {
        std::thread::Builder::new().stack_size(256 << 20).spawn(tell_every_program).unwrap().join().unwrap();
    }

    fn tell_every_program() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        let mut todo = vec![root.join("examples"), root.join("src/modules"), root.join("packages")];
        while let Some(d) = todo.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    todo.push(p);
                } else if p.extension().is_some_and(|x| x == "co") {
                    files.push(p);
                }
            }
        }
        assert!(files.len() > 20, "the checkout's programs: {}", files.len());
        let mut names = 0;
        for f in &files {
            let src = std::fs::read_to_string(f).unwrap();
            let a = analyze(&src, None, None, "t.co");
            let lines: Vec<Vec<char>> = src.split('\n').map(|l| l.chars().collect()).collect();
            let told = told(&a);
            for w in told.windows(2) {
                assert!((w[0].line, w[0].col + w[0].len) <= (w[1].line, w[1].col), "{}: {:?} overlaps {:?}", f.display(), w[0], w[1]);
            }
            for t in &told {
                let line = &lines[t.line - 1];
                assert!(t.col - 1 + t.len <= line.len(), "{}: {:?} is past its line", f.display(), t);
                let text: String = line[t.col - 1..t.col - 1 + t.len].iter().collect();
                assert!(text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{}:{}: {:?} is told as {:?}", f.display(), t.line, text, t.kind);
                assert!(t.kind == Kind::Keyword || !crate::lexer::KEYWORDS.contains(&text.as_str()), "{}:{}: the word {:?} is told as {:?}", f.display(), t.line, text, t.kind);
            }
            names += told.len();
            let _ = tokens(&a);
        }
        assert!(names > 1000, "{} names in {} files", names, files.len());
    }
}
