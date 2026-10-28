//! Two values are compared when they are of one kind: numbers with
//! numbers, text with text, bools with bools (spec, Comparison; Vijay,
//! 2026-10-01: "a string is not a number"). A comparison of a frame's
//! column, or of a list, with one value is a mask: each cell compared as
//! two values compare, a missing cell false (spec, Boolean Masks). One
//! function decides a cell — `holds` — and every comparison whose kinds
//! show only as the program runs is decided with it: the interpreter's
//! over a list's items, the compiled program's over its typed column or
//! list, a parsed value (`json_holds`, `json_pair`), and a frame's mask
//! computed in one pass (`pick_where`, second-half-design §3). So a comparison is the same in both modes, and the same fused or
//! not.

use std::cmp::Ordering;

use rayon::prelude::*;

use crate::frame::{Column, CrowColumn, CrowFrame};

/// A comparison operator, by its word.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cmp {
    Eq,
    Ne,
    Gt,
    Lt,
    Gte,
    Lte,
}

impl Cmp {
    pub fn named(op: &str) -> Option<Cmp> {
        Some(match op {
            "eq" => Cmp::Eq,
            "ne" => Cmp::Ne,
            "gt" => Cmp::Gt,
            "lt" => Cmp::Lt,
            "gte" => Cmp::Gte,
            "lte" => Cmp::Lte,
            _ => return None,
        })
    }

    pub fn word(self) -> &'static str {
        match self {
            Cmp::Eq => "eq",
            Cmp::Ne => "ne",
            Cmp::Gt => "gt",
            Cmp::Lt => "lt",
            Cmp::Gte => "gte",
            Cmp::Lte => "lte",
        }
    }

    /// The operator with its sides exchanged: `2 lt x` is `x gt 2`.
    pub fn flipped(self) -> Cmp {
        match self {
            Cmp::Gt => Cmp::Lt,
            Cmp::Lt => Cmp::Gt,
            Cmp::Gte => Cmp::Lte,
            Cmp::Lte => Cmp::Gte,
            same => same,
        }
    }

    /// Whether it asks an order, not only equality.
    pub fn orders(self) -> bool {
        !matches!(self, Cmp::Eq | Cmp::Ne)
    }

    pub fn of(self, ord: Option<Ordering>) -> bool {
        match self {
            Cmp::Eq => ord == Some(Ordering::Equal),
            Cmp::Ne => ord != Some(Ordering::Equal),
            Cmp::Gt => ord == Some(Ordering::Greater),
            Cmp::Lt => ord == Some(Ordering::Less),
            Cmp::Gte => matches!(ord, Some(Ordering::Greater | Ordering::Equal)),
            Cmp::Lte => matches!(ord, Some(Ordering::Less | Ordering::Equal)),
        }
    }
}

/// A cell as a comparison reads it, or the value it is compared with.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Cell<'a> {
    Number(f64),
    Text(&'a str),
    Bool(bool),
    /// A moment, by its instant in milliseconds.
    Moment(i64),
    /// A value no cell is — a list, a map — named for the reason.
    Other(&'static str),
}

impl Cell<'_> {
    /// What it is, as a reason names it.
    pub fn kind(&self) -> &'static str {
        match self {
            Cell::Number(_) => "a number",
            Cell::Text(_) => "text",
            Cell::Bool(_) => "a bool",
            Cell::Moment(_) => "a moment",
            Cell::Other(k) => k,
        }
    }

    /// Whether a comparison compares it with another: two of one kind —
    /// numbers, text, bools — and an order only of numbers or of text.
    fn compares_with(&self, other: &Cell, cmp: Cmp) -> bool {
        match (self, other) {
            (Cell::Number(_), Cell::Number(_)) | (Cell::Text(_), Cell::Text(_)) | (Cell::Moment(_), Cell::Moment(_)) => true,
            (Cell::Bool(_), Cell::Bool(_)) => !cmp.orders(),
            _ => false,
        }
    }
}

impl<'a> From<&'a crate::moments::Moment> for Cell<'a> {
    fn from(m: &'a crate::moments::Moment) -> Self {
        Cell::Moment(m.ms)
    }
}

impl<'a> From<&'a i64> for Cell<'a> {
    fn from(n: &'a i64) -> Self {
        Cell::Number(*n as f64)
    }
}

impl<'a> From<&'a f64> for Cell<'a> {
    fn from(x: &'a f64) -> Self {
        Cell::Number(*x)
    }
}

impl<'a> From<&'a bool> for Cell<'a> {
    fn from(b: &'a bool) -> Self {
        Cell::Bool(*b)
    }
}

impl<'a> From<&'a String> for Cell<'a> {
    fn from(s: &'a String) -> Self {
        Cell::Text(s)
    }
}

impl<'a> From<&'a str> for Cell<'a> {
    fn from(s: &'a str) -> Self {
        Cell::Text(s)
    }
}

/// An item a list's mask compares: the compiled program's typed items.
pub trait AsCell {
    fn as_cell(&self) -> Cell<'_>;
}

impl AsCell for crate::moments::Moment {
    fn as_cell(&self) -> Cell<'_> {
        Cell::Moment(self.ms)
    }
}

impl AsCell for i64 {
    fn as_cell(&self) -> Cell<'_> {
        Cell::Number(*self as f64)
    }
}

impl AsCell for f64 {
    fn as_cell(&self) -> Cell<'_> {
        Cell::Number(*self)
    }
}

impl AsCell for bool {
    fn as_cell(&self) -> Cell<'_> {
        Cell::Bool(*self)
    }
}

impl AsCell for String {
    fn as_cell(&self) -> Cell<'_> {
        Cell::Text(self)
    }
}

/// Whether `cell cmp value` holds, `present` saying whether the cell is
/// there. Numbers compare by value, exactly — an int as its float, NaN
/// equal to nothing and unordered, so only `ne` holds for it; text by its
/// characters, ordered by character code as `sort` orders it; bools are
/// equal or not. A missing cell holds nothing, `ne` included. Values of
/// two kinds are not compared — a number is not text — and only numbers
/// and text have an order: either is the reason the comparison has no
/// answer, whether or not the cell is there.
pub fn holds(cmp: Cmp, cell: Cell, present: bool, value: Cell) -> Result<bool, String> {
    if !cell.compares_with(&value, cmp) {
        return Err(incomparable(cmp, cell.kind(), value.kind()));
    }
    Ok(decide(cmp, cell, present, value))
}

/// `holds`, for kinds known to compare.
#[inline]
fn decide(cmp: Cmp, cell: Cell, present: bool, value: Cell) -> bool {
    present
        && cmp.of(match (cell, value) {
            (Cell::Number(a), Cell::Number(b)) => a.partial_cmp(&b),
            (Cell::Text(a), Cell::Text(b)) => Some(a.cmp(b)),
            (Cell::Bool(a), Cell::Bool(b)) => Some(a.cmp(&b)),
            (Cell::Moment(a), Cell::Moment(b)) => Some(a.cmp(&b)),
            _ => None,
        })
}

/// The reason a comparison of two kinds has no answer — the words the
/// lowering refuses it with when it knows both: an order is of numbers or
/// of text, and equality of two values of one kind.
pub fn incomparable(cmp: Cmp, a: &str, b: &str) -> String {
    if cmp.orders() {
        format!("{} compares numbers with numbers or text with text, not {} with {}", cmp.word(), a, b)
    } else {
        format!("{} compares values of one kind, not {} with {}", cmp.word(), a, b)
    }
}

/// A parsed value as a comparison reads it: a number, text or a bool, a
/// list or a map named; JSON's null is none (it equals nothing).
pub fn json_cell(j: &serde_json::Value) -> Option<Cell<'_>> {
    use serde_json::Value as J;
    match j {
        J::Number(n) => n.as_f64().map(Cell::Number),
        J::String(s) => Some(Cell::Text(s)),
        J::Bool(b) => Some(Cell::Bool(*b)),
        J::Array(_) => Some(Cell::Other("a list")),
        J::Object(_) => Some(Cell::Other("a map")),
        J::Null => None,
    }
}

/// `eq` or `ne` of a parsed value with a value: compared as two values are
/// when they are of one kind, and the reason when they are not — its kind
/// shows only as the program runs. JSON's null equals nothing.
pub fn json_holds(cmp: Cmp, j: &serde_json::Value, value: Cell) -> Result<bool, String> {
    match json_cell(j) {
        Some(cell) => holds(cmp, cell, true, value),
        None => Ok(cmp == Cmp::Ne),
    }
}

/// `eq` or `ne` of two parsed values: two lists or two maps compared whole
/// as JSON, two of one kind as two values are, two of different kinds the
/// reason; JSON's null equals nothing, not even another null.
pub fn json_pair(cmp: Cmp, a: &serde_json::Value, b: &serde_json::Value) -> Result<bool, String> {
    use serde_json::Value as J;
    let eq = match (a, b) {
        (J::Null, _) | (_, J::Null) => false,
        (J::Array(_), J::Array(_)) | (J::Object(_), J::Object(_)) => a == b,
        _ => match (json_cell(a), json_cell(b)) {
            (Some(x), Some(y)) => return holds(cmp, x, true, y),
            _ => false,
        },
    };
    Ok(eq != (cmp == Cmp::Ne))
}

/// A list's mask: each item compared with the value.
pub fn list<T: AsCell>(items: &[T], cmp: Cmp, value: Cell) -> Result<Vec<bool>, String> {
    items.iter().map(|x| holds(cmp, x.as_cell(), true, value)).collect()
}

impl CrowColumn {
    /// The column's mask: each cell compared with the value, a missing
    /// cell false; a moment compared as its text, as a cell of one reads.
    pub fn mask(&self, cmp: Cmp, value: Cell) -> Result<Vec<bool>, String> {
        let n = self.len();
        let mut out = vec![false; n];
        if n > 0 {
            check(&self.data, cmp, value)?;
            leaf(&self.data, &self.valid, cmp, value, 0, &mut out);
        }
        Ok(out)
    }
}

impl CrowColumn {
    /// Row `i`'s moment as its text, as a cell of one reads.
    fn moment_text(&self, i: usize) -> Option<String> {
        match self.data.as_ref() {
            Column::DateTime(ms, off) => Some(crate::frame::format_datetime(ms[i], off[i])),
            _ => None,
        }
    }

    /// Row `i` as a comparison reads it, a moment by its text.
    fn cell<'a>(&'a self, i: usize, moment: &'a Option<String>) -> Cell<'a> {
        match (self.data.as_ref(), moment) {
            (_, Some(text)) => Cell::Text(text),
            (Column::Float(v), _) => Cell::Number(v[i]),
            (Column::Int(v), _) => Cell::Number(v[i] as f64),
            (Column::Str(v), _) => Cell::Text(&v[i]),
            (Column::Bool(v), _) => Cell::Bool(v[i]),
            (Column::DateTime(ms, _), None) => Cell::Moment(ms[i]),
            (Column::Vectors(..), _) => Cell::Other("a vector"),
        }
    }

    /// `eq` or `ne` of the column and another, whole, as two lists are
    /// compared (spec, Comparison): equal when they have the same number of
    /// cells, each there in both and equal as two values are; a missing cell
    /// equals nothing, not even another missing one. Columns of two kinds
    /// are the reason, read from their first cells (every cell is of its
    /// column's kind).
    pub fn whole(&self, cmp: Cmp, other: &CrowColumn) -> Result<bool, String> {
        if self.len() != other.len() {
            return Ok(cmp == Cmp::Ne);
        }
        // two columns of vectors: equal where every row is there in both
        // and its numbers are equal (as the interpreter compares its lists
        // of vectors)
        if let (Column::Vectors(wa, a), Column::Vectors(wb, b)) = (self.data.as_ref(), other.data.as_ref()) {
            let eq = wa == wb && (0..self.len()).all(|i| self.valid[i] && other.valid[i] && a[i * wa..(i + 1) * wa] == b[i * wb..(i + 1) * wb]);
            return Ok(eq != (cmp == Cmp::Ne));
        }
        if self.len() > 0 {
            let (a, b) = (self.moment_text(0), other.moment_text(0));
            holds(cmp, self.cell(0, &a), false, other.cell(0, &b))?;
        }
        let eq = (0..self.len()).all(|i| {
            let (a, b) = (self.moment_text(i), other.moment_text(i));
            decide(Cmp::Eq, self.cell(i, &a), self.valid[i] && other.valid[i], other.cell(i, &b))
        });
        Ok(eq != (cmp == Cmp::Ne))
    }

    /// `eq` or `ne` of the column and a list, whole, cell by item.
    pub fn whole_list<T: AsCell>(&self, cmp: Cmp, items: &[T]) -> Result<bool, String> {
        if self.len() != items.len() {
            return Ok(cmp == Cmp::Ne);
        }
        if let Some(first) = items.first() {
            let a = self.moment_text(0);
            holds(cmp, self.cell(0, &a), false, first.as_cell())?;
        }
        let eq = items.iter().enumerate().all(|(i, x)| {
            let a = self.moment_text(i);
            decide(Cmp::Eq, self.cell(i, &a), self.valid[i], x.as_cell())
        });
        Ok(eq != (cmp == Cmp::Ne))
    }

    /// `eq` or `ne` of the column and a tensor, whole: a vector's numbers,
    /// each a cell's partner (a matrix's items are rows, which no cell
    /// equals).
    pub fn whole_tensor(&self, cmp: Cmp, t: &crate::tensor::CrowTensor) -> Result<bool, String> {
        if t.ndim() != 1 {
            return Ok(cmp == Cmp::Ne);
        }
        self.whole_list(cmp, &t.to_vec())
    }
}

/// Whether a column's cells compare with a value: the reason when they do
/// not. Its cells are all of its kind, a missing one too, so the first
/// cell's comparison decides for every one.
fn check(col: &Column, cmp: Cmp, value: Cell) -> Result<(), String> {
    let kind = match col {
        Column::Int(_) | Column::Float(_) => Cell::Number(0.0),
        Column::Str(_) => Cell::Text(""),
        Column::DateTime(..) => Cell::Moment(0),
        Column::Bool(_) => Cell::Bool(false),
        Column::Vectors(..) => Cell::Other("a vector"),
    };
    holds(cmp, kind, false, value).map(|_| ())
}

/// Rows `start..start + out.len()` of a column compared with a value:
/// a block at a time, one loop per kind and operator (each cell had been
/// read into a `Cell` and decided on its own, which cost the compiled
/// mask 3.6 times the interpreter's); equality with text over a
/// dictionary column compares codes.
fn leaf(col: &Column, valid: &crate::frame::Validity, cmp: Cmp, value: Cell, start: usize, out: &mut [bool]) {
    let rows = start..start + out.len();
    let valid = &valid.slice(rows.start, rows.end)[..];
    match (col, value) {
        (Column::Float(v), Cell::Number(y)) => numbers(&v[rows], valid, cmp, y, out),
        (Column::Int(v), Cell::Number(y)) => {
            let xs: Vec<f64> = v[rows].iter().map(|&x| x as f64).collect();
            numbers(&xs, valid, cmp, y, out)
        }
        (Column::Bool(v), Cell::Bool(y)) => {
            for ((o, &x), &ok) in out.iter_mut().zip(&v[rows]).zip(valid) {
                *o = ok && cmp.of(Some(x.cmp(&y)));
            }
        }
        (Column::Str(t), Cell::Text(y)) => match (t.dictionary(), cmp) {
            (Some((codes, values)), Cmp::Eq | Cmp::Ne) => {
                let code = values.iter().position(|v| v == y).map(|c| c as u32);
                let wanted = cmp == Cmp::Eq;
                for ((o, &c), &ok) in out.iter_mut().zip(&codes[rows]).zip(valid) {
                    *o = ok && ((Some(c) == code) == wanted);
                }
            }
            _ => {
                for (k, (o, &ok)) in out.iter_mut().zip(valid).enumerate() {
                    *o = ok && cmp.of(Some(t.get(start + k).cmp(y)));
                }
            }
        },
        (Column::DateTime(ms, _), Cell::Moment(y)) => {
            for ((o, &m), &ok) in out.iter_mut().zip(&ms[rows]).zip(valid) {
                *o = ok && cmp.of(Some(m.cmp(&y)));
            }
        }
        // kinds that do not compare (refused before the rows are read):
        // nothing holds
        _ => out.fill(false),
    }
}

/// Numbers compared with one, a loop per operator: `partial_cmp`'s
/// answers, so a NaN holds only `ne`, and -0 equals 0.
fn numbers(xs: &[f64], valid: &[bool], cmp: Cmp, y: f64, out: &mut [bool]) {
    macro_rules! each {
        ($op:tt) => {
            for ((o, &x), &ok) in out.iter_mut().zip(xs).zip(valid) {
                *o = ok && (x $op y);
            }
        };
    }
    match cmp {
        Cmp::Eq => each!(==),
        Cmp::Ne => each!(!=),
        Cmp::Gt => each!(>),
        Cmp::Lt => each!(<),
        Cmp::Gte => each!(>=),
        Cmp::Lte => each!(<=),
    }
}

/// A frame's mask: comparisons of the frame's columns with values,
/// a column's `.ok`, and `and`, `or` and `not` of them.
enum Node {
    /// The column at a position of the frame, compared with a value.
    Cmp(Cmp, usize, usize),
    /// The column at a position of the frame: whether each cell is there.
    Ok(usize),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
    Not(Box<Node>),
}

enum Item {
    Column(usize),
    Value(usize),
    Node(Node),
}

/// Rows a block computes at once, and the frame size from which the
/// blocks are computed across the cores.
const BLOCK: usize = 4096;
const PARALLEL_FROM: usize = 1 << 16;

/// `f[mask]` with the mask computed in one pass,
/// row block by row block, never a mask per comparison. `program` is the
/// predicate in postfix: `c<i>` the column `columns[i]`, `v<i>` the value
/// `values[i]`, a comparison's word compares the column before it with the
/// value, `ok` asks the column before it whether each cell is there, and
/// `and`, `or` and `not` combine. Each column is found, and each
/// comparison's kinds checked, in the order the operators would read and
/// compare them: the first that fails is the reason, in the operators'
/// words — a column the frame lacks, an order between kinds with none —
/// so a fused mask stops where the unfused one would, and otherwise
/// keeps the rows its mask would.
pub fn pick_where(f: &CrowFrame, program: &str, columns: &[String], values: &[Cell]) -> Result<CrowFrame, String> {
    let bug = || format!("a bug in Crow: the predicate '{}' is not one", program);
    let index = |tok: &str, prefix: char, len: usize| tok.strip_prefix(prefix).and_then(|i| i.parse::<usize>().ok()).filter(|&i| i < len);
    let find = |c: usize| f.col_index(&columns[c]).ok_or_else(|| crate::stops::no_column(&columns[c]));
    let n = f.nrows();
    let mut stack: Vec<Item> = Vec::new();
    for tok in program.split_whitespace() {
        let item = if let Some(c) = index(tok, 'c', columns.len()) {
            Item::Column(c)
        } else if let Some(v) = index(tok, 'v', values.len()) {
            Item::Value(v)
        } else if let Some(cmp) = Cmp::named(tok) {
            let (Some(Item::Value(v)), Some(Item::Column(c))) = (stack.pop(), stack.pop()) else { return Err(bug()) };
            let at = find(c)?;
            if n > 0 {
                check(&f.data[at], cmp, values[v])?;
            }
            Item::Node(Node::Cmp(cmp, at, v))
        } else {
            match tok {
                "ok" => {
                    let Some(Item::Column(c)) = stack.pop() else { return Err(bug()) };
                    Item::Node(Node::Ok(find(c)?))
                }
                "and" | "or" => {
                    let (Some(Item::Node(r)), Some(Item::Node(l))) = (stack.pop(), stack.pop()) else { return Err(bug()) };
                    let (l, r) = (Box::new(l), Box::new(r));
                    Item::Node(if tok == "and" { Node::And(l, r) } else { Node::Or(l, r) })
                }
                "not" => {
                    let Some(Item::Node(x)) = stack.pop() else { return Err(bug()) };
                    Item::Node(Node::Not(Box::new(x)))
                }
                _ => return Err(bug()),
            }
        };
        stack.push(item);
    }
    let (Some(Item::Node(root)), true) = (stack.pop(), stack.is_empty()) else { return Err(bug()) };
    let mut mask = vec![false; n];
    let fill = |(b, out): (usize, &mut [bool])| eval(&root, f, values, b * BLOCK, out);
    if n >= PARALLEL_FROM {
        mask.par_chunks_mut(BLOCK).enumerate().for_each(fill);
    } else {
        mask.chunks_mut(BLOCK).enumerate().for_each(fill);
    }
    f.pick_rows(&mask)
}

/// A predicate over rows `start..start + out.len()`.
fn eval(node: &Node, f: &CrowFrame, values: &[Cell], start: usize, out: &mut [bool]) {
    match node {
        Node::Cmp(cmp, at, v) => leaf(&f.data[*at], &f.validity[*at], *cmp, values[*v], start, out),
        Node::Ok(at) => out.copy_from_slice(&f.validity[*at].slice(start, start + out.len())),
        Node::And(l, r) | Node::Or(l, r) => {
            eval(l, f, values, start, out);
            let mut right = vec![false; out.len()];
            eval(r, f, values, start, &mut right);
            let and = matches!(node, Node::And(..));
            for (o, r) in out.iter_mut().zip(right) {
                *o = if and { *o && r } else { *o || r };
            }
        }
        Node::Not(x) => {
            eval(x, f, values, start, out);
            for o in out.iter_mut() {
                *o = !*o;
            }
        }
    }
}

// ── a mask of a tensor's shape ──────────────────────────────────────

/// A mask of any depth: a bool, or a list of masks of one shape. A tensor
/// compared with a number answers one of the tensor's own shape — a list
/// of bools for a vector, its rows of bools for a matrix (spec, Boolean
/// Masks: a matrix's had been one flat list of its numbers' answers, which
/// `where` read as a vector and no index of the matrix took) — and every
/// reading walks it in row order. The compiled program's masks are these;
/// the interpreter keeps the same shape in its own values.
pub trait Truths: Clone {
    /// How many lists deep it is (a bool: none).
    fn depth() -> usize;
    /// Its truths, in row order, onto `out`.
    fn truths_into(&self, out: &mut Vec<bool>);
    /// Its shape: each list's length, every list at one depth one length,
    /// or the reason (a list of rows of two lengths is no mask's).
    fn shape_of(&self) -> Result<Vec<usize>, String>;
    /// Made of truths in row order, to a shape of its depth.
    fn of_truths(flat: &mut std::slice::Iter<'_, bool>, shape: &[usize]) -> Self;
    /// Each truth turned.
    fn each(&self, f: &dyn Fn(bool) -> bool) -> Self;
    /// Two masks of one shape, truth by truth, or the reason.
    fn with(&self, other: &Self, f: &dyn Fn(bool, bool) -> bool) -> Result<Self, String>;
}

impl Truths for bool {
    fn depth() -> usize {
        0
    }
    fn truths_into(&self, out: &mut Vec<bool>) {
        out.push(*self);
    }
    fn shape_of(&self) -> Result<Vec<usize>, String> {
        Ok(Vec::new())
    }
    fn of_truths(flat: &mut std::slice::Iter<'_, bool>, _shape: &[usize]) -> Self {
        flat.next().copied().unwrap_or(false)
    }
    fn each(&self, f: &dyn Fn(bool) -> bool) -> Self {
        f(*self)
    }
    fn with(&self, other: &Self, f: &dyn Fn(bool, bool) -> bool) -> Result<Self, String> {
        Ok(f(*self, *other))
    }
}

impl<T: Truths> Truths for Vec<T> {
    fn depth() -> usize {
        1 + T::depth()
    }
    fn truths_into(&self, out: &mut Vec<bool>) {
        for x in self {
            x.truths_into(out);
        }
    }
    fn shape_of(&self) -> Result<Vec<usize>, String> {
        let inner = match self.first() {
            Some(first) => first.shape_of()?,
            None => vec![0; T::depth()],
        };
        for x in self.iter().skip(1) {
            if x.shape_of()? != inner {
                return Err("a mask's rows are of different lengths".to_string());
            }
        }
        let mut shape = vec![self.len()];
        shape.extend(inner);
        Ok(shape)
    }
    fn of_truths(flat: &mut std::slice::Iter<'_, bool>, shape: &[usize]) -> Self {
        let (n, rest) = match shape.split_first() {
            Some((n, rest)) => (*n, rest),
            None => (0, &[][..]),
        };
        (0..n).map(|_| T::of_truths(flat, rest)).collect()
    }
    fn each(&self, f: &dyn Fn(bool) -> bool) -> Self {
        self.iter().map(|x| x.each(f)).collect()
    }
    fn with(&self, other: &Self, f: &dyn Fn(bool, bool) -> bool) -> Result<Self, String> {
        if self.len() != other.len() {
            return Err(format!("masks of {} and {} items are joined truth by truth: they must be one shape", self.len(), other.len()));
        }
        self.iter().zip(other).map(|(a, b)| a.with(b, f)).collect()
    }
}

/// The mask of a tensor's shape its numbers' answers make (in row order).
pub fn shaped<T: Truths>(flat: Vec<bool>, shape: &[usize]) -> T {
    // (a vector's mask is the answers as they are; a number's — no axes —
    // one answer in a list, as a vector of one)
    let shape: Vec<usize> = if shape.is_empty() { vec![flat.len()] } else { shape.to_vec() };
    T::of_truths(&mut flat.iter(), &shape)
}

/// `not` of a mask: each truth turned.
pub fn negated<T: Truths>(m: &T) -> T {
    m.each(&|b| !b)
}

/// `and`/`or` of two masks of one shape, or the reason.
pub fn joined<T: Truths>(a: &T, b: &T, both: bool) -> Result<T, String> {
    a.with(b, &|x, y| if both { x && y } else { x || y })
}

/// `and`/`or` of a mask and one bool: each truth joined with it.
pub fn joined_with<T: Truths>(m: &T, s: bool, both: bool) -> T {
    m.each(&|x| if both { x && s } else { x || s })
}

/// A mask where numbers are taken: true 1, false 0, of its own shape (a
/// list of rows of bools is a matrix, as a list of bools is a vector), or
/// the reason its rows are not one length.
pub fn tensor_of<T: Truths>(m: &T) -> Result<crate::tensor::CrowTensor, String> {
    let shape = m.shape_of()?;
    let mut flat = Vec::new();
    m.truths_into(&mut flat);
    Ok(crate::tensor::CrowTensor::from_vec(flat.into_iter().map(|b| if b { 1.0 } else { 0.0 }).collect(), shape))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cell_holds_as_two_values_compare() {
        let n = Cell::Number;
        assert_eq!(holds(Cmp::Eq, n(0.1 + 0.2), true, n(0.3)), Ok(false));
        assert_eq!(holds(Cmp::Eq, n(-0.0), true, n(0.0)), Ok(true));
        assert_eq!(holds(Cmp::Ne, n(f64::NAN), true, n(2.0)), Ok(true));
        for c in [Cmp::Eq, Cmp::Gt, Cmp::Lt, Cmp::Gte, Cmp::Lte] {
            assert_eq!(holds(c, n(f64::NAN), true, n(2.0)), Ok(false));
            assert_eq!(holds(c, n(2.0), true, n(f64::NAN)), Ok(false));
        }
        for c in [Cmp::Eq, Cmp::Ne, Cmp::Gt, Cmp::Lt, Cmp::Gte, Cmp::Lte] {
            assert_eq!(holds(c, n(2.0), false, n(2.0)), Ok(false));
            assert_eq!(holds(c, Cell::Text("a"), false, Cell::Text("b")), Ok(false));
        }
        assert_eq!(holds(Cmp::Gt, Cell::Text("b"), true, Cell::Text("a")), Ok(true));
        assert_eq!(holds(Cmp::Lt, Cell::Text("B"), true, Cell::Text("a")), Ok(true));
        assert_eq!(holds(Cmp::Gte, Cell::Text("é"), true, Cell::Text("z")), Ok(true));
        assert_eq!(holds(Cmp::Eq, Cell::Text("2"), true, n(2.0)), Err("eq compares values of one kind, not text with a number".to_string()));
        assert_eq!(holds(Cmp::Ne, Cell::Text("2"), false, n(2.0)), Err("ne compares values of one kind, not text with a number".to_string()));
        assert_eq!(holds(Cmp::Eq, Cell::Bool(true), true, Cell::Bool(true)), Ok(true));
        assert_eq!(holds(Cmp::Ne, Cell::Bool(true), true, Cell::Bool(false)), Ok(true));
        assert_eq!(holds(Cmp::Ne, Cell::Bool(true), true, n(1.0)), Err("ne compares values of one kind, not a bool with a number".to_string()));
        assert_eq!(holds(Cmp::Eq, Cell::Other("a list"), true, n(1.0)), Err("eq compares values of one kind, not a list with a number".to_string()));
        assert_eq!(holds(Cmp::Gt, Cell::Bool(true), true, Cell::Bool(false)), Err("gt compares numbers with numbers or text with text, not a bool with a bool".to_string()));
        assert_eq!(holds(Cmp::Lte, Cell::Text("2"), false, n(2.0)), Err("lte compares numbers with numbers or text with text, not text with a number".to_string()));
    }

    /// A small generator (SplitMix64): the same cases on every run.
    struct Gen(u64);
    impl Gen {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
        fn number(&mut self) -> f64 {
            match self.below(10) {
                0 => f64::NAN,
                1 => -0.0,
                2 => 0.1 + 0.2,
                _ => self.below(7) as f64 - 3.0,
            }
        }
        fn text(&mut self) -> String {
            ["", "a", "B", "ab", "é", "2", "a "][self.below(7) as usize].to_string()
        }
        fn value(&mut self) -> Drawn {
            let kind = self.below(4);
            self.of_kind(kind)
        }
        /// A value of a kind: 0 a number, 1 text, 2 a bool.
        fn of_kind(&mut self, kind: u64) -> Drawn {
            match kind {
                0 => (Some(self.number()), None, None, None),
                1 => (None, Some(self.text()), None, None),
                2 => (None, None, Some(self.below(2) == 0), None),
                _ => (None, None, None, Some(self.below(3) as i64 * 86_400_000)),
            }
        }
        /// A value for a comparison with the frame's column `c` (f, i, s, b,
        /// d, then one it lacks): of the column's kind nine times in ten.
        fn value_for(&mut self, c: usize) -> Drawn {
            let kind = [0, 0, 1, 2, 3].get(c).copied().unwrap_or(4);
            if kind == 4 || self.below(10) == 0 { self.value() } else { self.of_kind(kind) }
        }
    }

    /// A number, text, a bool or a moment (milliseconds).
    type Drawn = (Option<f64>, Option<String>, Option<bool>, Option<i64>);

    fn cell_of(v: &Drawn) -> Cell<'_> {
        match v {
            (Some(x), _, _, _) => Cell::Number(*x),
            (_, Some(s), _, _) => Cell::Text(s),
            (_, _, Some(b), _) => Cell::Bool(*b),
            (_, _, _, Some(ms)) => Cell::Moment(*ms),
            _ => Cell::Other("nothing"),
        }
    }

    impl Gen {
    }

    /// A frame with a column of each kind, cells drawn, some missing.
    fn frame(g: &mut Gen, n: usize) -> CrowFrame {
        let valid = |g: &mut Gen| (0..n).map(|_| g.below(5) != 0).collect::<Vec<bool>>();
        let columns = vec!["f".to_string(), "i".to_string(), "s".to_string(), "b".to_string(), "d".to_string()];
        let data = vec![
            Column::Float((0..n).map(|_| g.number()).collect()),
            Column::Int((0..n).map(|_| g.below(7) as i64 - 3).collect()),
            Column::Str((0..n).map(|_| g.text()).collect()),
            Column::Bool((0..n).map(|_| g.below(2) == 0).collect()),
            Column::DateTime((0..n).map(|_| g.below(3) as i64 * 86_400_000).collect(), vec![0; n]),
        ];
        let validity = (0..5).map(|_| valid(g)).collect();
        CrowFrame::new(columns, data, validity)
    }

    /// A predicate tree, written in postfix as the lowering writes it.
    enum Tree {
        Cmp(Cmp, usize, usize),
        Ok(usize),
        And(Box<Tree>, Box<Tree>),
        Or(Box<Tree>, Box<Tree>),
        Not(Box<Tree>),
    }

    /// A predicate over `columns` columns, each comparison's value drawn
    /// into `values`.
    fn tree(g: &mut Gen, columns: usize, values: &mut Vec<Drawn>, depth: u32) -> Tree {
        let ops = [Cmp::Eq, Cmp::Ne, Cmp::Gt, Cmp::Lt, Cmp::Gte, Cmp::Lte];
        match if depth == 0 { g.below(2) } else { g.below(6) } {
            0 => {
                let (op, c) = (ops[g.below(6) as usize], g.below(columns as u64) as usize);
                values.push(g.value_for(c));
                Tree::Cmp(op, c, values.len() - 1)
            }
            1 => Tree::Ok(g.below(columns as u64) as usize),
            2 => Tree::Not(Box::new(tree(g, columns, values, depth - 1))),
            3 => Tree::Or(Box::new(tree(g, columns, values, depth - 1)), Box::new(tree(g, columns, values, depth - 1))),
            _ => Tree::And(Box::new(tree(g, columns, values, depth - 1)), Box::new(tree(g, columns, values, depth - 1))),
        }
    }

    fn postfix(t: &Tree, out: &mut Vec<String>) {
        match t {
            Tree::Cmp(c, col, v) => out.extend([format!("c{}", col), format!("v{}", v), c.word().to_string()]),
            Tree::Ok(col) => out.extend([format!("c{}", col), "ok".to_string()]),
            Tree::And(l, r) | Tree::Or(l, r) => {
                postfix(l, out);
                postfix(r, out);
                out.push(if matches!(t, Tree::And(..)) { "and" } else { "or" }.to_string());
            }
            Tree::Not(x) => {
                postfix(x, out);
                out.push("not".to_string());
            }
        }
    }

    /// The unfused mask: each comparison a column's mask, each `.ok` its
    /// validity, combined row by row, left before right — as the operators
    /// compute it.
    fn unfused(t: &Tree, f: &CrowFrame, columns: &[String], values: &[Cell]) -> Result<Vec<bool>, String> {
        let column = |c: usize| f.col_native(&columns[c]).ok_or_else(|| crate::stops::no_column(&columns[c]));
        match t {
            Tree::Cmp(cmp, c, v) => column(*c)?.mask(*cmp, values[*v]),
            Tree::Ok(c) => Ok(column(*c)?.ok()),
            Tree::And(l, r) | Tree::Or(l, r) => {
                let (a, b) = (unfused(l, f, columns, values)?, unfused(r, f, columns, values)?);
                let and = matches!(t, Tree::And(..));
                Ok(a.iter().zip(&b).map(|(x, y)| if and { *x && *y } else { *x || *y }).collect())
            }
            Tree::Not(x) => Ok(unfused(x, f, columns, values)?.iter().map(|x| !x).collect()),
        }
    }

    /// A column's mask is each cell's `holds`, read cell by cell.
    #[test]
    fn a_columns_mask_is_each_cells_comparison() {
        let mut g = Gen(20261001);
        for _ in 0..300 {
            let n = 1 + g.below(40) as usize;
            let f = frame(&mut g, n);
            for name in ["f", "i", "s", "b", "d"] {
                let col = f.col_native(name).expect("a column");
                let drawn = g.value();
                let value = cell_of(&drawn);
                for cmp in [Cmp::Eq, Cmp::Ne, Cmp::Gt, Cmp::Lt, Cmp::Gte, Cmp::Lte] {
                    let cells: Result<Vec<bool>, String> = (0..col.len())
                        .map(|i| {
                            let cell = match col.data.as_ref() {
                                Column::Float(v) => Cell::Number(v[i]),
                                Column::Int(v) => Cell::Number(v[i] as f64),
                                Column::Str(v) => Cell::Text(&v[i]),
                                Column::Bool(v) => Cell::Bool(v[i]),
                                Column::DateTime(ms, _) => Cell::Moment(ms[i]),
                                Column::Vectors(..) => Cell::Other("a vector"),
                            };
                            holds(cmp, cell, col.valid[i], value)
                        })
                        .collect();
                    assert_eq!(col.mask(cmp, value), cells, "{} {} {:?}", name, cmp.word(), value);
                }
            }
        }
    }

    /// The fused mask keeps the rows the unfused one keeps, and stops with
    /// the reason the unfused one stops with first — on generated frames
    /// (missing cells, NaN, ints, text, bools, moments) and predicates, a
    /// column the frame lacks among them; small frames and ones across the
    /// cores.
    #[test]
    fn a_fused_mask_is_the_unfused_one() {
        let mut g = Gen(1001);
        let mut kept = 0;
        let mut stopped = 0;
        for case in 0..2000 {
            let n = if case % 200 == 0 { PARALLEL_FROM + 1 + g.below(5000) as usize } else { g.below(60) as usize };
            let f = frame(&mut g, n);
            let mut columns: Vec<String> = ["f", "i", "s", "b", "d"].iter().map(|c| c.to_string()).collect();
            if g.below(10) == 0 {
                columns.push("zz".to_string());
            }
            let mut drawn = Vec::new();
            let t = tree(&mut g, columns.len(), &mut drawn, 3);
            let values: Vec<Cell> = drawn.iter().map(cell_of).collect();
            let mut program = Vec::new();
            postfix(&t, &mut program);
            let program = program.join(" ");
            let fused = pick_where(&f, &program, &columns, &values);
            let by_masks = unfused(&t, &f, &columns, &values).and_then(|m| f.pick_rows(&m));
            match (&fused, &by_masks) {
                (Ok(a), Ok(b)) => {
                    assert_eq!(format!("{:?}", a), format!("{:?}", b), "{}", program);
                    kept += 1;
                }
                (Err(a), Err(b)) => {
                    assert_eq!(a, b, "{}", program);
                    stopped += 1;
                }
                _ => panic!("{}: fused {:?}, unfused {:?}", program, fused.map(|f| f.nrows()), by_masks.map(|f| f.nrows())),
            }
        }
        assert!(kept > 600 && stopped > 200, "kept {} stopped {}", kept, stopped);
    }

    /// A column compared whole is compared as a list is: cell by cell, an
    /// int as its float, a missing cell equal to nothing — whatever its
    /// column holds in its place (NaN, 0, empty text)
    #[test]
    fn a_column_compared_whole_is_compared_cell_by_cell() {
        let col = |data: Column, valid: Vec<bool>| CrowColumn { data: std::sync::Arc::new(data), valid: std::sync::Arc::new(crate::frame::Validity::from(valid)) };
        let ints = col(Column::Int(vec![1, 2]), vec![true, true]);
        let floats = col(Column::Float(vec![1.0, 2.0]), vec![true, true]);
        assert_eq!((ints.whole(Cmp::Eq, &floats), floats.whole(Cmp::Ne, &ints)), (Ok(true), Ok(false)));
        assert_eq!((ints.whole_list(Cmp::Eq, &[1.0, 2.0]), ints.whole_list(Cmp::Eq, &[1.0]), ints.whole_list(Cmp::Ne, &[1.0, 3.0])), (Ok(true), Ok(false), Ok(true)));
        assert_eq!(ints.whole_tensor(Cmp::Eq, &crate::tensor::CrowTensor::from_vec(vec![1.0, 2.0], vec![2])), Ok(true));
        assert_eq!(ints.whole_tensor(Cmp::Eq, &crate::tensor::CrowTensor::from_vec(vec![1.0, 2.0], vec![1, 2])), Ok(false));
        for missing in [col(Column::Int(vec![1, 0]), vec![true, false]), col(Column::strs(vec!["a".to_string(), String::new()]), vec![true, false]), col(Column::Float(vec![1.0, f64::NAN]), vec![true, false])] {
            assert_eq!((missing.whole(Cmp::Eq, &missing), missing.whole(Cmp::Ne, &missing)), (Ok(false), Ok(true)), "{:?}", missing);
        }
        let text = col(Column::strs(vec!["a".to_string(), String::new()]), vec![true, false]);
        assert_eq!(text.whole_list(Cmp::Eq, &["a".to_string(), String::new()]), Ok(false));
        let strs = col(Column::strs(vec!["p".to_string(), "q".to_string()]), vec![true, true]);
        assert_eq!(strs.whole_list(Cmp::Eq, &["p".to_string(), "q".to_string()]), Ok(true));
        assert_eq!(strs.whole(Cmp::Eq, &ints), Err("eq compares values of one kind, not text with a number".to_string()));
        assert_eq!(strs.whole_list(Cmp::Ne, &[1.0, 2.0]), Err("ne compares values of one kind, not text with a number".to_string()));
        assert_eq!(strs.whole_list(Cmp::Eq, &[1.0]), Ok(false), "lengths first, as two lists' are");
    }

    /// A parsed value compares with a value of its kind, and stops with the
    /// reason against another; JSON's null equals nothing
    #[test]
    fn a_parsed_value_compares_with_its_kind() {
        let j = |s: &str| serde_json::from_str::<serde_json::Value>(s).expect("json");
        assert_eq!(json_holds(Cmp::Eq, &j("1"), Cell::Number(1.0)), Ok(true));
        assert_eq!(json_holds(Cmp::Eq, &j("\"ok\""), Cell::Text("ok")), Ok(true));
        assert_eq!(json_holds(Cmp::Ne, &j("true"), Cell::Bool(true)), Ok(false));
        assert_eq!(json_holds(Cmp::Eq, &j("1"), Cell::Text("1")), Err("eq compares values of one kind, not a number with text".to_string()));
        assert_eq!(json_holds(Cmp::Eq, &j("[1]"), Cell::Number(1.0)), Err("eq compares values of one kind, not a list with a number".to_string()));
        assert_eq!((json_holds(Cmp::Eq, &j("null"), Cell::Number(1.0)), json_holds(Cmp::Ne, &j("null"), Cell::Number(1.0))), (Ok(false), Ok(true)));
        assert_eq!(json_pair(Cmp::Eq, &j("1"), &j("1.0")), Ok(true));
        assert_eq!(json_pair(Cmp::Eq, &j("[1, {\"a\": 2}]"), &j("[1, {\"a\": 2}]")), Ok(true));
        assert_eq!(json_pair(Cmp::Ne, &j("null"), &j("null")), Ok(true));
        assert_eq!(json_pair(Cmp::Eq, &j("\"1\""), &j("1")), Err("eq compares values of one kind, not text with a number".to_string()));
    }

    #[test]
    fn a_predicate_that_is_none_is_a_bug_in_crow() {
        let f = CrowFrame::new(vec!["a".to_string()], vec![Column::Int(vec![1])], vec![vec![true]]);
        let cols = ["a".to_string()];
        for p in ["c0 v0", "c0 v0 gt and", "c1 v0 gt", "c0 v0 gt c0 v0 lt", "c0 v0 xor"] {
            assert!(pick_where(&f, p, &cols, &[Cell::Number(1.0)]).unwrap_err().contains("a bug in Crow"), "{}", p);
        }
    }
}
