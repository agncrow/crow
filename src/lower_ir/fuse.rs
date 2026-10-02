//! Element-wise chains of tensors fused into one call (second-half-design
//! §3; crow_runtime::fused). A tree of `+`, `-`, `.*`, `./`, `.^`, `*` and
//! `/` between a tensor and a number, and negation, whose value is a tensor,
//! with two operations or more, and whose operands only read — a local, a
//! fact, a derivation, a number written, a record's field of one — is one
//! call of the runtime's kernel, which reads each element once and writes
//! one tensor. Both backends call the same function, so the modes agree by
//! construction; the kernel is tested against the operators themselves.
//! An operand that could act or stop (a call, an index) keeps the chain to
//! the operators, which evaluate it where the program does.
//! `CROW_NO_FUSE=1` leaves every chain to the operators: a fused build and
//! an unfused one, compared.

use crate::ir::*;

/// Every chain of the program, fused.
pub(super) fn tensors(program: &mut Program) {
    if std::env::var("CROW_NO_FUSE").is_ok_and(|v| !v.is_empty() && v != "0") {
        return;
    }
    let Some(row) = crate::table::row("tensor.fused") else { return };
    for part in &mut program.parts {
        for f in &mut part.fns {
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

fn block(b: &mut Block, row: &'static crate::table::Row) {
    for s in b.iter_mut() {
        stmt(s, row);
    }
}

fn stmt(s: &mut Stmt, row: &'static crate::table::Row) {
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

fn expr(e: &mut Expr, row: &'static crate::table::Row) {
    // the widest chain rooted here, or the chains inside it
    let mut program = Vec::new();
    let mut operands = Vec::new();
    if let Some(ops) = chain(e, &mut program, &mut operands)
        && ops >= 2
    {
        let ty = e.ty();
        let mut args = vec![Expr::Const(Const::Str(program.join(" ")), Ty::Str)];
        args.extend(operands);
        *e = Expr::Call { target: Target::Row(row), args, ty, propagates: false };
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
