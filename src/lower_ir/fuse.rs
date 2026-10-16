//! Chains computed in one pass (second-half-design §3), each one call of
//! a runtime function both backends call, so the modes agree by
//! construction, and each tested against the operators it replaces.
//!
//! - An element-wise chain of tensors (crow_runtime::fused): a tree of
//!   `+`, `-`, `.*`, `./`, `.^`, `*` and `/` between a tensor and a number,
//!   and negation, whose value is a tensor, with two operations or more,
//!   and whose operands only read — a local, a fact, a derivation, a number
//!   written, a record's field of one. The kernel reads each element once
//!   and writes one tensor.
//! - A frame's mask (crow_runtime::mask::pick_where): `f[p]`
//!   whose `p` compares `f`'s own columns with values that only read —
//!   `f.age gt 40`, `f.name ne 'bob'`, `40 lt f.age` — asks a column's
//!   `.ok`, and combines those with `and`, `or` and `not`. The kernel
//!   computes the predicate row block by row block, with no column read
//!   into a list and no mask per comparison, deciding each cell with the
//!   function every mask uses (crow_runtime::mask::holds).
//!
//! An operand that could act or stop (a call, an index) keeps its chain to
//! the operators, which evaluate it where the program does.
//! `CROW_NO_FUSE=1` leaves every chain to the operators: a fused build and
//! an unfused one, compared.

use crate::ir::*;
use crate::table::Row;

/// The rows the fused calls name.
#[derive(Clone, Copy)]
struct Rows {
    tensor: &'static Row,
    pick: &'static Row,
}

/// Every chain of the program, fused.
pub(super) fn chains(program: &mut Program) {
    if std::env::var("CROW_NO_FUSE").is_ok_and(|v| !v.is_empty() && v != "0") {
        return;
    }
    let (Some(tensor), Some(pick)) = (crate::table::row("tensor.fused"), crate::table::row("core.__pick_where")) else { return };
    let row = Rows { tensor, pick };
    for part in &mut program.parts {
        // (a function grad differentiates is left to the operators the tape
        // records: a fused kernel would be opaque to it)
        let traced = part.traced.clone();
        for f in part.fns.iter_mut().filter(|f| !traced.contains(&f.name)) {
            block(&mut f.body, row);
        }
        for d in &mut part.derives {
            block(&mut d.body, row);
        }
        for t in &mut part.tests {
            block(&mut t.body, row);
        }
        for fact in &mut part.facts {
            expr(&mut fact.default, row);
        }
        block(&mut part.main, row);
    }
}

fn block(b: &mut Block, row: Rows) {
    for s in b.iter_mut() {
        stmt(s, row);
    }
}

fn stmt(s: &mut Stmt, row: Rows) {
    match s {
        Stmt::Let { expr: e, .. } | Stmt::Set { expr: e, .. } | Stmt::Store { expr: e, .. } | Stmt::Do(e) | Stmt::Return(Some(e)) => expr(e, row),
        Stmt::SetPath { path, expr: e, .. } => {
            for p in path.iter_mut() {
                if let PathStep::Index(i) = p {
                    expr(i, row);
                }
            }
            expr(e, row);
        }
        Stmt::If { cond, then, els } => {
            expr(cond, row);
            block(then, row);
            if let Some(e) = els {
                block(e, row);
            }
        }
        Stmt::While { cond, body } => {
            expr(cond, row);
            block(body, row);
        }
        Stmt::For { iter, body, .. } => {
            expr(iter, row);
            block(body, row);
        }
        _ => {}
    }
}

fn expr(e: &mut Expr, row: Rows) {
    // the widest chain rooted here, or the chains inside it
    let mut program = Vec::new();
    let mut operands = Vec::new();
    if let Some(ops) = chain(e, &mut program, &mut operands)
        && ops >= 2
    {
        let ty = e.ty();
        let mut args = vec![Expr::Const(Const::Str(program.join(" ")), Ty::Str)];
        args.extend(operands);
        *e = Expr::Call { target: Target::Row(row.tensor), args, ty, propagates: false };
        return;
    }
    if let Some(fused) = picked(e, row.pick) {
        // (its frame and values only read: no chain is inside them)
        *e = fused;
        return;
    }
    match e {
        Expr::Call { args, .. } | Expr::Effect { args, .. } | Expr::Deferred { args, .. } => args.iter_mut().for_each(|a| expr(a, row)),
        Expr::Convert { expr: x, .. } | Expr::Unary { expr: x, .. } => expr(x, row),
        Expr::Binary { l, r, .. } => {
            expr(l, row);
            expr(r, row);
        }
        Expr::Index { base, indices, .. } => {
            expr(base, row);
            indices.iter_mut().for_each(|i| expr(i, row));
        }
        Expr::Field { base, .. } | Expr::Meta { base, .. } => expr(base, row),
        Expr::Interp(pieces) => {
            for p in pieces.iter_mut() {
                if let Piece::Expr(x) = p {
                    expr(x, row);
                }
            }
        }
        Expr::ListLit(items, _) => items.iter_mut().for_each(|i| expr(i, row)),
        Expr::MapLit(pairs, _) => pairs.iter_mut().for_each(|(_, v)| expr(v, row)),
        Expr::RecordLit { fields, .. } => fields.iter_mut().for_each(|f| expr(f, row)),
        Expr::Tree { attrs, children, .. } => {
            attrs.iter_mut().for_each(|(_, v)| expr(v, row));
            children.iter_mut().for_each(|c| expr(c, row));
        }
        Expr::ForExpr { iter, body, .. } => {
            expr(iter, row);
            block(body, row);
        }
        _ => {}
    }
}

fn tensor(t: &Ty) -> bool {
    matches!(t, Ty::Tensor(..))
}

/// The chain an expression is, written into `program` (postfix) and
/// `operands`: how many operations it has, or none when it is no chain.
fn chain(e: &Expr, program: &mut Vec<String>, operands: &mut Vec<Expr>) -> Option<usize> {
    match e {
        Expr::Binary { op, l, r, ty } if tensor(ty) && fusible(op, l, r) => {
            let a = operand(l, program, operands)?;
            let b = operand(r, program, operands)?;
            program.push(op.clone());
            Some(a + b + 1)
        }
        Expr::Unary { op, expr: x, ty } if op == "neg" && tensor(ty) => {
            let a = operand(x, program, operands)?;
            program.push("neg".to_string());
            Some(a + 1)
        }
        _ => None,
    }
}

/// An operator's operand: a chain inside, or a value that only reads.
fn operand(e: &Expr, program: &mut Vec<String>, operands: &mut Vec<Expr>) -> Option<usize> {
    if let Some(n) = chain(e, program, operands) {
        return Some(n);
    }
    let t = e.ty();
    if (tensor(&t) || matches!(t, Ty::Int | Ty::Float)) && only_reads(e) {
        // a variable read twice is one operand, read once
        let same = |o: &Expr| match (o, e) {
            (Expr::Local(a, _) | Expr::Move(a, _), Expr::Local(b, _) | Expr::Move(b, _)) | (Expr::Load(a, _), Expr::Load(b, _)) | (Expr::Derived(a, _), Expr::Derived(b, _)) => a == b,
            _ => false,
        };
        let at = match operands.iter().position(same) {
            Some(i) => {
                // (the last read of a local moves it: the operand read once is that one)
                if matches!(e, Expr::Move(..)) {
                    operands[i] = e.clone();
                }
                i
            }
            None => {
                operands.push(e.clone());
                operands.len() - 1
            }
        };
        program.push(format!("#{}", at));
        return Some(0);
    }
    None
}

/// The operators the kernel computes: element-wise ones, and a product or
/// a quotient of a tensor and a number (a product of tensors is the matrix
/// product; a tensor over a tensor is refused; a number raised to a tensor
/// is the operator's error).
fn fusible(op: &str, l: &Expr, r: &Expr) -> bool {
    let (lt, rt) = (tensor(&l.ty()), tensor(&r.ty()));
    match op {
        "add" | "sub" | "elemmul" | "elemdiv" => true,
        "elempow" => lt,
        "mul" | "div" => lt != rt,
        _ => false,
    }
}

/// Whether reading an expression only reads: nothing it does can act or
/// stop, so where the kernel evaluates it changes nothing.
fn only_reads(e: &Expr) -> bool {
    match e {
        Expr::Const(Const::Int(_) | Const::Float(_), _) => true,
        Expr::Local(..) | Expr::Move(..) | Expr::Load(..) | Expr::Derived(..) | Expr::Constant(..) => true,
        Expr::Field { base, .. } => matches!(base.ty(), Ty::Record(_)) && only_reads(base),
        Expr::Convert { how: Convert::IntToFloat, expr: x, .. } => only_reads(x),
        _ => false,
    }
}

// ── a frame's masks ─────────────────────────────────────────────────

/// The fused pick a call is, when it is one: `f[p]` whose mask the kernel
/// computes.
fn picked(e: &Expr, row: &'static Row) -> Option<Expr> {
    let Expr::Call { target: Target::Row(r), args, ty, propagates } = e else { return None };
    if r.id != "core.__pick_frame" || args.len() != 2 {
        return None;
    }
    let frame = read_name(&args[0])?;
    let mut fused = Predicate { frame, program: Vec::new(), columns: Vec::new(), values: Vec::new() };
    fused.node(&args[1])?;
    let columns = fused.columns.into_iter().map(|c| Expr::Const(Const::Str(c), Ty::Str)).collect();
    let mut call = vec![
        args[0].clone(),
        Expr::Const(Const::Str(fused.program.join(" ")), Ty::Str),
        Expr::ListLit(columns, Ty::List(Box::new(Ty::Str))),
    ];
    call.extend(fused.values);
    Some(Expr::Call { target: Target::Row(row), args: call, ty: ty.clone(), propagates: *propagates })
}

/// The variable an expression reads, through the lowering's marks of a
/// payload: `df`, `unwrap df`.
fn read_name(e: &Expr) -> Option<&str> {
    match e {
        Expr::Local(n, _) | Expr::Move(n, _) | Expr::Load(n, _) | Expr::Derived(n, _) => Some(n),
        Expr::Convert { how: Convert::Unwrap | Convert::Known, expr, .. } => read_name(expr),
        _ => None,
    }
}

/// A mask being written: in postfix, over the picked frame's
/// columns (each once) and the values its comparisons compare them with.
struct Predicate<'a> {
    frame: &'a str,
    program: Vec<String>,
    columns: Vec<String>,
    values: Vec<Expr>,
}

impl Predicate<'_> {
    /// A node of the predicate, written: none when the kernel does not
    /// compute it.
    fn node(&mut self, e: &Expr) -> Option<()> {
        match e {
            Expr::Binary { op, l, r, .. } if op == "and" || op == "or" => {
                self.node(l)?;
                self.node(r)?;
                self.program.push(op.clone());
            }
            Expr::Unary { op, expr: x, .. } if op == "not" => {
                self.node(x)?;
                self.program.push("not".to_string());
            }
            Expr::Binary { op, l, r, .. } => {
                let cmp = crow_runtime::mask::Cmp::named(op)?;
                // the column on either side: a value written first compares
                // with the operator turned round (`40 lt f.age` is `f.age gt 40`)
                let (col, value, cmp) = match (self.column(l), self.column(r)) {
                    (Some(c), None) => (c, r, cmp),
                    (None, Some(c)) => (c, l, cmp.flipped()),
                    _ => return None,
                };
                if !matches!(value.ty(), Ty::Int | Ty::Float | Ty::Str | Ty::Markup | Ty::Bool) || !value_reads(value) {
                    return None;
                }
                let c = self.column_at(col);
                self.values.push((**value).clone());
                self.program.extend([format!("c{}", c), format!("v{}", self.values.len() - 1), cmp.word().to_string()]);
            }
            Expr::Meta { base, which: Meta::Valid } => {
                let c = self.column(base)?;
                let c = self.column_at(c);
                self.program.extend([format!("c{}", c), "ok".to_string()]);
            }
            _ => return None,
        }
        Some(())
    }

    /// The column of the picked frame an expression reads.
    fn column(&self, e: &Expr) -> Option<String> {
        match e {
            Expr::Field { base, name, ty: Ty::Column(_) } if read_name(base) == Some(self.frame) => Some(name.clone()),
            Expr::Convert { how: Convert::Unwrap | Convert::Known, expr, .. } => self.column(expr),
            _ => None,
        }
    }

    fn column_at(&mut self, name: String) -> usize {
        match self.columns.iter().position(|c| *c == name) {
            Some(i) => i,
            None => {
                self.columns.push(name);
                self.columns.len() - 1
            }
        }
    }
}

/// Whether a value a predicate compares with only reads: what `only_reads`
/// allows, text and bools written, and the payload of one a propagation
/// already covered.
fn value_reads(e: &Expr) -> bool {
    match e {
        Expr::Const(..) => true,
        Expr::Convert { how: Convert::Unwrap | Convert::Known, expr, .. } => value_reads(expr),
        _ => only_reads(e),
    }
}
