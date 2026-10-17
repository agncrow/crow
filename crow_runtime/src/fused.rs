//! Element-wise chains of tensors computed in one pass (second-half-design
//! §3, fusion; Vijay, 2026-10-01: finish the compiler work). `x * 2.0 + x .*
//! x - 1.0` reads each element of its tensors once and writes one tensor,
//! where each operator had made a tensor of its own. An element is what the
//! operators compute, in their order and with their formulas — a tensor
//! over a number divides each number, as `t / s` does (both had multiplied
//! by the reciprocal, which rounds twice), and a number minus a tensor adds
//! the number to the tensor's negation — so the
//! answer is the operators' to the bit. A shape that does not meet stops
//! with the words the operator stops with, at the first operator, in the
//! operators' order, whose operands do not meet. A tensor on the GPU, whose
//! operators compute there in f32, is computed by the operators themselves
//! (`by_operators`), which is also what the kernel is tested against.
//!
//! The lowering writes a chain as a postfix program over its operands:
//! `#0 #1 mul #0 #0 elemmul add #2 sub` — `#i` the i-th operand, an
//! operator by the IR's name (`add`, `sub`, `mul`, `div`, `elemmul`,
//! `elemdiv`, `elempow`), `neg` a negation.

use crate::tensor::CrowTensor;

/// An operand of a chain: a tensor, or a number.
#[derive(Clone, Copy)]
pub enum Operand<'a> {
    Tensor(&'a CrowTensor),
    Number(f64),
}

impl<'a> From<&'a CrowTensor> for Operand<'a> {
    fn from(t: &'a CrowTensor) -> Self {
        Operand::Tensor(t)
    }
}

impl From<&f64> for Operand<'_> {
    fn from(x: &f64) -> Self {
        Operand::Number(*x)
    }
}

impl From<&i64> for Operand<'_> {
    fn from(n: &i64) -> Self {
        Operand::Number(*n as f64)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    ElemMul,
    ElemDiv,
    ElemPow,
}

impl Op {
    fn named(name: &str) -> Option<Op> {
        Some(match name {
            "add" => Op::Add,
            "sub" => Op::Sub,
            "mul" => Op::Mul,
            "div" => Op::Div,
            "elemmul" => Op::ElemMul,
            "elemdiv" => Op::ElemDiv,
            "elempow" => Op::ElemPow,
            _ => return None,
        })
    }

    /// The operator as the program writes it, as its refusal names it.
    fn sign(self) -> &'static str {
        match self {
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
            Op::ElemMul => ".*",
            Op::ElemDiv => "./",
            Op::ElemPow => ".^",
        }
    }
}

enum Node {
    Operand(usize),
    Neg(Box<Node>),
    Bin(Op, Box<Node>, Box<Node>),
}

/// Why a program is none the compiler writes.
fn not_a_chain(program: &str) -> String {
    format!("a fused chain the compiler wrote is not one — a bug in Crow: `{}`", program)
}

fn parse(program: &str, operands: usize) -> Result<Node, String> {
    let mut stack: Vec<Node> = Vec::new();
    for token in program.split_whitespace() {
        if let Some(i) = token.strip_prefix('#') {
            let i: usize = i.parse().map_err(|_| not_a_chain(program))?;
            if i >= operands {
                return Err(not_a_chain(program));
            }
            stack.push(Node::Operand(i));
        } else if token == "neg" {
            let x = stack.pop().ok_or_else(|| not_a_chain(program))?;
            stack.push(Node::Neg(Box::new(x)));
        } else {
            let op = Op::named(token).ok_or_else(|| not_a_chain(program))?;
            let r = stack.pop().ok_or_else(|| not_a_chain(program))?;
            let l = stack.pop().ok_or_else(|| not_a_chain(program))?;
            stack.push(Node::Bin(op, Box::new(l), Box::new(r)));
        }
    }
    match (stack.pop(), stack.is_empty()) {
        (Some(root), true) => Ok(root),
        _ => Err(not_a_chain(program)),
    }
}

/// Two shapes broadcast, after `CrowTensor::meets_shapes` said they do.
fn broadcast(a: &[usize], b: &[usize]) -> Vec<usize> {
    let n = a.len().max(b.len());
    (0..n)
        .map(|i| {
            let da = if i < n - a.len() { 1 } else { a[i - (n - a.len())] };
            let db = if i < n - b.len() { 1 } else { b[i - (n - b.len())] };
            if da == 1 { db } else { da }
        })
        .collect()
}

/// Each node's shape (none: a number), in the operators' order — the left
/// operand's, the right's, then the node's — refused where an operator's
/// operands do not meet, with its words.
fn shapes(node: &Node, operands: &[Operand], program: &str) -> Result<Option<Vec<usize>>, String> {
    match node {
        Node::Operand(i) => Ok(match operands[*i] {
            Operand::Tensor(t) => Some(t.shape().to_vec()),
            Operand::Number(_) => None,
        }),
        Node::Neg(x) => shapes(x, operands, program),
        Node::Bin(op, l, r) => {
            let (ls, rs) = (shapes(l, operands, program)?, shapes(r, operands, program)?);
            match (ls, rs, op) {
                // the compiler fuses neither the matrix product nor the
                // refused tensor-over-tensor, nor arithmetic of numbers
                (Some(_), Some(_), Op::Mul | Op::Div) | (None, None, _) => Err(not_a_chain(program)),
                (Some(a), Some(b), _) => {
                    CrowTensor::meets_shapes(&a, &b, op.sign())?;
                    Ok(Some(broadcast(&a, &b)))
                }
                (None, Some(_), Op::ElemPow) => Err(".^ requires tensors or tensor and scalar".to_string()),
                (Some(s), None, _) | (None, Some(s), _) => Ok(Some(s)),
            }
        }
    }
}

/// A chain computed: each element once, in blocks small enough to stay in
/// the cache between operators.
pub fn run(program: &str, operands: &[Operand]) -> Result<CrowTensor, String> {
    let root = parse(program, operands.len())?;
    let shape = shapes(&root, operands, program)?.ok_or_else(|| not_a_chain(program))?;
    if operands.iter().any(|o| matches!(o, Operand::Tensor(t) if t.gpu_buffer().is_some())) {
        return match by_operators(&root, operands)? {
            Value::Tensor(t) => Ok(t),
            Value::Number(_) => Err(not_a_chain(program)),
        };
    }
    let n: usize = shape.iter().product();
    // (a reading for each tensor the chain reads: its shape broadcasts to
    // the answer's)
    let mut used = vec![false; operands.len()];
    mark(&root, &mut used);
    let reads: Vec<Option<Reading<'_>>> = operands
        .iter()
        .zip(&used)
        .map(|(o, used)| match o {
            Operand::Tensor(t) if *used => Some(Reading::of(t, &shape)),
            _ => None,
        })
        .collect();
    let numbers: Vec<Option<f64>> = operands
        .iter()
        .map(|o| match o {
            Operand::Number(x) => Some(*x),
            Operand::Tensor(_) => None,
        })
        .collect();
    let mut out = vec![0.0f64; n];
    // each block on its own (an element depends on no other), across the
    // cores for a large answer
    let fill = |start: usize, chunk: &mut [f64]| match eval(&root, &numbers, &reads, &shape, start, chunk.len()) {
        Block::Values(v) => chunk.copy_from_slice(&v),
        Block::Number(x) => chunk.fill(x),
    };
    if n >= PARALLEL_FROM {
        use rayon::prelude::*;
        out.par_chunks_mut(BLOCK).enumerate().for_each(|(b, chunk)| fill(b * BLOCK, chunk));
    } else {
        out.chunks_mut(BLOCK).enumerate().for_each(|(b, chunk)| fill(b * BLOCK, chunk));
    }
    Ok(CrowTensor::from_vec(out, shape))
}

/// The operands a chain reads.
fn mark(node: &Node, used: &mut [bool]) {
    match node {
        Node::Operand(i) => used[*i] = true,
        Node::Neg(x) => mark(x, used),
        Node::Bin(_, l, r) => {
            mark(l, used);
            mark(r, used);
        }
    }
}

/// The elements a block of the answer is computed in.
const BLOCK: usize = 4096;

/// The answer's size from which its blocks are computed across the cores.
const PARALLEL_FROM: usize = 1 << 16;

/// How a tensor operand's elements are read for the answer's: its data,
/// its offset, and a stride for each of the answer's dimensions (0 where it
/// broadcasts along it).
struct Reading<'a> {
    data: std::borrow::Cow<'a, [f64]>,
    offset: usize,
    strides: Vec<usize>,
    /// its shape is the answer's and its data runs in order: read as is
    straight: bool,
}

impl<'a> Reading<'a> {
    fn of(t: &'a CrowTensor, shape: &[usize]) -> Reading<'a> {
        let (data, offset, own) = t.strided();
        let lead = shape.len() - t.shape().len();
        let strides = (0..shape.len())
            .map(|d| if d < lead || t.shape()[d - lead] == 1 && shape[d] != 1 { 0 } else { own[d - lead] })
            .collect();
        Reading { data, offset, strides, straight: t.shape() == shape && t.is_contiguous() }
    }

    fn block(&self, shape: &[usize], start: usize, len: usize) -> Vec<f64> {
        if self.straight {
            return self.data[self.offset + start..self.offset + start + len].to_vec();
        }
        // the answer's index of `start`, then counted on
        let mut index = vec![0usize; shape.len()];
        let mut rest = start;
        for d in (0..shape.len()).rev() {
            if shape[d] > 0 {
                index[d] = rest % shape[d];
                rest /= shape[d];
            }
        }
        let mut at: usize = self.offset + index.iter().zip(&self.strides).map(|(i, s)| i * s).sum::<usize>();
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            out.push(self.data[at]);
            for d in (0..shape.len()).rev() {
                index[d] += 1;
                at += self.strides[d];
                if index[d] < shape[d] {
                    break;
                }
                at -= self.strides[d] * index[d];
                index[d] = 0;
            }
        }
        out
    }
}

enum Block {
    Values(Vec<f64>),
    Number(f64),
}

fn eval(node: &Node, numbers: &[Option<f64>], reads: &[Option<Reading<'_>>], shape: &[usize], start: usize, len: usize) -> Block {
    match node {
        Node::Operand(i) => match (&numbers[*i], &reads[*i]) {
            (Some(x), _) => Block::Number(*x),
            (None, Some(r)) => Block::Values(r.block(shape, start, len)),
            (None, None) => Block::Number(f64::NAN),
        },
        Node::Neg(x) => match eval(x, numbers, reads, shape, start, len) {
            Block::Values(mut v) => {
                v.iter_mut().for_each(|a| *a = -*a);
                Block::Values(v)
            }
            Block::Number(s) => Block::Number(-s),
        },
        Node::Bin(op, l, r) => {
            let (a, b) = (eval(l, numbers, reads, shape, start, len), eval(r, numbers, reads, shape, start, len));
            Block::Values(match (a, b) {
                // the operators' formulas, element by element
                (Block::Values(mut a), Block::Values(b)) => {
                    let f: fn(f64, f64) -> f64 = match op {
                        Op::Add => |x, y| x + y,
                        Op::Sub => |x, y| x - y,
                        Op::ElemDiv => |x, y| x / y,
                        Op::ElemPow => |x, y| x.powf(y),
                        _ => |x, y| x * y,
                    };
                    a.iter_mut().zip(&b).for_each(|(x, y)| *x = f(*x, *y));
                    a
                }
                (Block::Values(mut a), Block::Number(s)) => {
                    match op {
                        Op::Add => a.iter_mut().for_each(|x| *x += s),
                        Op::Sub => a.iter_mut().for_each(|x| *x += -s),
                        Op::Div | Op::ElemDiv => a.iter_mut().for_each(|x| *x /= s),
                        Op::ElemPow => a.iter_mut().for_each(|x| *x = x.powf(s)),
                        _ => a.iter_mut().for_each(|x| *x *= s),
                    }
                    a
                }
                (Block::Number(s), Block::Values(mut b)) => {
                    match op {
                        Op::Add => b.iter_mut().for_each(|x| *x += s),
                        Op::Sub => b.iter_mut().for_each(|x| *x = -*x + s),
                        Op::Div | Op::ElemDiv => b.iter_mut().for_each(|x| *x = s / *x),
                        _ => b.iter_mut().for_each(|x| *x *= s),
                    }
                    b
                }
                (Block::Number(_), Block::Number(_)) => vec![f64::NAN; len],
            })
        }
    }
}

/// A value of the operators' evaluation: a tensor, or a number.
enum Value {
    Tensor(CrowTensor),
    Number(f64),
}

/// The chain computed by the operators themselves, as the interpreter's
/// operators compute it (src/ops.rs): a tensor on the GPU, and the
/// kernel's test.
fn by_operators(node: &Node, operands: &[Operand]) -> Result<Value, String> {
    Ok(match node {
        Node::Operand(i) => match operands[*i] {
            Operand::Tensor(t) => Value::Tensor(t.clone()),
            Operand::Number(x) => Value::Number(x),
        },
        Node::Neg(x) => match by_operators(x, operands)? {
            Value::Tensor(t) => Value::Tensor(t.neg()),
            Value::Number(s) => Value::Number(-s),
        },
        Node::Bin(op, l, r) => {
            let (a, b) = (by_operators(l, operands)?, by_operators(r, operands)?);
            Value::Tensor(match (a, b) {
                (Value::Tensor(a), Value::Tensor(b)) => match op {
                    Op::Mul => a.try_matmul(&b)?,
                    Op::Div => return Err("use ./ for element-wise tensor division".to_string()),
                    op => a.try_elementwise(&b, op.sign())?,
                },
                (Value::Tensor(a), Value::Number(s)) => match op {
                    Op::Add => a.add_scalar(s),
                    Op::Sub => a.add_scalar(-s),
                    Op::Div | Op::ElemDiv => a.div_scalar(s),
                    Op::ElemPow => a.map(|x| x.powf(s)),
                    _ => a.mul_scalar(s),
                },
                (Value::Number(s), Value::Tensor(b)) => match op {
                    Op::Add => b.add_scalar(s),
                    Op::Sub => b.neg().add_scalar(s),
                    Op::Div | Op::ElemDiv => CrowTensor::full(b.shape().to_vec(), s).div(&b),
                    Op::ElemPow => return Err(".^ requires tensors or tensor and scalar".to_string()),
                    _ => b.mul_scalar(s),
                },
                (Value::Number(a), Value::Number(b)) => return Ok(Value::Number(match op {
                    Op::Add => a + b,
                    Op::Sub => a - b,
                    Op::Div | Op::ElemDiv => a / b,
                    Op::ElemPow => a.powf(b),
                    _ => a * b,
                })),
            })
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        fn value(&mut self) -> f64 {
            // zeros, negatives, fractions and the odd large one: what a
            // division, a power and a negation meet
            match self.below(8) {
                0 => 0.0,
                1 => -0.0,
                2 => (self.below(2000) as f64 - 1000.0) / 7.0,
                3 => (self.below(100) as f64) * 1e6,
                _ => (self.below(20000) as f64 - 10000.0) / 1000.0,
            }
        }
    }

    /// A tensor of a shape, its values drawn; a transposed one now and then
    /// (strided, not contiguous).
    fn tensor(g: &mut Gen, shape: &[usize]) -> CrowTensor {
        let n: usize = shape.iter().product();
        let data: Vec<f64> = (0..n).map(|_| g.value()).collect();
        let t = CrowTensor::from_vec(data, shape.to_vec());
        if shape.len() == 2 && g.below(4) == 0 {
            // the same values read through a transpose of the transpose
            let tt = CrowTensor::from_vec(t.to_vec(), vec![shape[1], shape[0]]);
            let back = tt.transpose();
            if back.shape() == shape {
                return back;
            }
        }
        t
    }

    /// A shape the chain's tensors broadcast to, and each operand's own
    /// (the same, or one with a dimension of 1, or one with fewer).
    fn shapes(g: &mut Gen) -> (Vec<usize>, Vec<Vec<usize>>) {
        let base: Vec<usize> = match g.below(4) {
            0 => vec![5],
            1 => vec![3, 4],
            2 => vec![2, 3, 2],
            _ => vec![1037], // across blocks
        };
        let variants = (0..4)
            .map(|_| match g.below(4) {
                0 if base.len() > 1 => base[1..].to_vec(),
                1 => {
                    let mut s = base.clone();
                    let d = g.below(s.len() as u64) as usize;
                    s[d] = 1;
                    s
                }
                _ => base.clone(),
            })
            .collect();
        (base, variants)
    }

    /// A random chain over `n` operands, postfix, with at least two
    /// operations; the operands' kinds (tensor or number) decide which
    /// operators are written where.
    fn program(g: &mut Gen, kinds: &[bool]) -> Option<String> {
        // (text, is a tensor, operations)
        fn build(g: &mut Gen, kinds: &[bool], depth: u32) -> (String, bool, usize) {
            if depth == 0 || g.below(3) == 0 {
                let i = g.below(kinds.len() as u64) as usize;
                return (format!("#{}", i), kinds[i], 0);
            }
            if g.below(6) == 0 {
                let (x, t, n) = build(g, kinds, depth - 1);
                if t {
                    return (format!("{} neg", x), true, n + 1);
                }
                return (x, t, n);
            }
            let (l, lt, ln) = build(g, kinds, depth - 1);
            let (r, rt, rn) = build(g, kinds, depth - 1);
            if !lt && !rt {
                return (l, lt, ln);
            }
            let ops: &[&str] = match (lt, rt) {
                (true, true) => &["add", "sub", "elemmul", "elemdiv", "elempow"],
                (true, false) => &["add", "sub", "mul", "div", "elemmul", "elemdiv", "elempow"],
                _ => &["add", "sub", "mul", "div", "elemmul", "elemdiv"],
            };
            let op = ops[g.below(ops.len() as u64) as usize];
            (format!("{} {} {}", l, r, op), true, ln + rn + 1)
        }
        let (text, t, ops) = build(g, kinds, 4);
        (t && ops >= 2).then_some(text)
    }

    /// The kernel and the operators agree on every chain: the same shape and
    /// every element the same to the bit (NaN as NaN), or the same refusal.
    #[test]
    fn a_fused_chain_is_the_operators_to_the_bit() {
        let mut g = Gen(20261001);
        let mut checked = 0;
        for _ in 0..3000 {
            let (_, variants) = shapes(&mut g);
            let kinds: Vec<bool> = (0..4).map(|i| i == 0 || g.below(3) != 0).collect();
            let Some(prog) = program(&mut g, &kinds) else { continue };
            let tensors: Vec<CrowTensor> = variants.iter().map(|s| tensor(&mut g, s)).collect();
            let numbers: Vec<f64> = (0..4).map(|_| g.value()).collect();
            let operands: Vec<Operand> = (0..4).map(|i| if kinds[i] { Operand::Tensor(&tensors[i]) } else { Operand::Number(numbers[i]) }).collect();
            let fused = run(&prog, &operands);
            let root = parse(&prog, 4).expect("a chain");
            let by_ops = match by_operators(&root, &operands) {
                Ok(Value::Tensor(t)) => Ok(t),
                Ok(Value::Number(_)) => continue,
                Err(e) => Err(e),
            };
            match (&fused, &by_ops) {
                (Ok(a), Ok(b)) => {
                    assert_eq!(a.shape(), b.shape(), "{}", prog);
                    let (av, bv) = (a.to_vec(), b.to_vec());
                    for (i, (x, y)) in av.iter().zip(&bv).enumerate() {
                        assert!(x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()), "{} element {}: fused {} operators {}", prog, i, x, y);
                    }
                }
                (Err(a), Err(b)) => assert_eq!(a, b, "{}", prog),
                _ => panic!("{}: fused {:?}, operators {:?}", prog, fused.as_ref().map(|t| t.shape().to_vec()), by_ops.as_ref().map(|t| t.shape().to_vec())),
            }
            checked += 1;
        }
        assert!(checked > 1000, "only {} chains checked", checked);
    }

    /// A large answer is computed across the cores, block by block, and is
    /// still the operators' to the bit — a row broadcast along a matrix of
    /// many blocks included
    #[test]
    fn a_large_chain_across_the_cores_is_the_operators_to_the_bit() {
        let mut g = Gen(7);
        let m = tensor(&mut g, &[300, 1000]);
        let row = tensor(&mut g, &[1000]);
        for prog in ["#0 #1 add #0 #0 elemmul sub #2 div", "#0 neg #2 #1 elemdiv add #0 #2 elempow sub", "#2 #0 sub #1 elemmul #2 #1 div add"] {
            let operands = [Operand::Tensor(&m), Operand::Tensor(&row), Operand::Number(2.5)];
            let fused = run(prog, &operands).expect("meets");
            let Ok(Value::Tensor(by_ops)) = by_operators(&parse(prog, 3).expect("a chain"), &operands) else { panic!("{}", prog) };
            assert_eq!(fused.shape(), by_ops.shape());
            for (x, y) in fused.to_vec().iter().zip(by_ops.to_vec().iter()) {
                assert!(x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()), "{}: {} {}", prog, x, y);
            }
        }
    }

    /// A shape that does not meet stops with the operator's words, at the
    /// first operator whose operands do not meet
    #[test]
    fn a_chain_whose_shapes_do_not_meet_stops_as_the_operator_does() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let b = CrowTensor::from_vec(vec![1.0, 2.0], vec![2]);
        let err = run("#0 #1 add #0 elemmul", &[Operand::Tensor(&a), Operand::Tensor(&b)]).unwrap_err();
        assert_eq!(err, a.try_elementwise(&b, "+").unwrap_err());
        assert_eq!(run("#0 #1 elempow #1 add", &[Operand::Number(2.0), Operand::Tensor(&a)]).unwrap_err(), ".^ requires tensors or tensor and scalar");
        assert!(run("#0 #9 add", &[Operand::Tensor(&a)]).unwrap_err().contains("a bug in Crow"));
    }
}
