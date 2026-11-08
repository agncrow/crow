//! The GPU's surface in a program that uses none (the `gpu` feature off,
//! crow's emitter asking for it only where `gpu[…]` is written): there is
//! never a context, so every tensor operation takes its CPU path, and the
//! program links none of the system frameworks a GPU needs — Metal,
//! QuartzCore, Foundation, CoreGraphics — whose loading had cost every
//! compiled program four of its seven megabytes before it did anything.
//! Each item is gpu.rs's, by name and signature; none can be reached.

use std::sync::atomic::{AtomicBool, Ordering};

static GPU_ENABLED: AtomicBool = AtomicBool::new(false);

/// `gpu[]`: tensors made from here on would be placed on the GPU — here
/// there is none, so they stay on the CPU.
pub fn set_gpu_enabled(enabled: bool) {
    GPU_ENABLED.store(enabled, Ordering::SeqCst);
}

pub fn is_gpu_enabled() -> bool {
    GPU_ENABLED.load(Ordering::SeqCst)
}

/// The most dimensions a kernel reads.
pub const MAX_DIMS: usize = 8;

/// No GPU: no value of this type exists.
pub enum GpuContext {}

impl GpuContext {
    pub fn get() -> Option<&'static GpuContext> {
        None
    }

    pub fn holds(&self, _n: usize) -> bool {
        match *self {}
    }
}

/// A GPU buffer: none is ever made, since only a context makes one. (A
/// type of its own with no value would make whatever follows a call that
/// answers one unreachable code, and the CPU's builds warned.)
#[derive(Debug)]
pub struct GpuBuffer(());

impl GpuBuffer {
    pub fn count(&self) -> usize {
        0
    }
    pub fn output(ctx: &GpuContext, _n: usize) -> Self {
        match *ctx {}
    }
    pub fn zeros(ctx: &GpuContext, _n: usize) -> Self {
        match *ctx {}
    }
    pub fn from_f32(ctx: &GpuContext, _data: &[f32]) -> Self {
        match *ctx {}
    }
    pub fn from_f64(ctx: &GpuContext, _data: &[f64]) -> Self {
        match *ctx {}
    }
    pub fn from_u32(ctx: &GpuContext, _data: &[u32]) -> Self {
        match *ctx {}
    }
    pub fn read(&self, ctx: &GpuContext, _start: usize, _len: usize) -> Vec<f32> {
        match *ctx {}
    }
    pub fn to_f32(&self, ctx: &GpuContext) -> Vec<f32> {
        match *ctx {}
    }
    pub fn places(&self) -> Vec<usize> {
        Vec::new()
    }
    pub fn to_f64(&self, ctx: &GpuContext) -> Vec<f64> {
        match *ctx {}
    }
}

pub fn copy(ctx: &GpuContext, _src: &GpuBuffer, _from: usize, _dst: &GpuBuffer, _to: usize, _len: usize) {
    match *ctx {}
}

/// An element-wise operation, as gpu.rs names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Neg,
    Abs,
    Exp,
    Log,
    Sqrt,
    Sin,
    Cos,
    Tan,
    Tanh,
    Sinh,
    Cosh,
    Asin,
    Acos,
    Atan,
    Log2,
    Log10,
    Log1p,
    Expm1,
    Floor,
    Ceil,
    Round,
    Sign,
    Erf,
    Copy,
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Mod,
    Atan2,
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
    Where,
    Clip,
}

impl Op {
    /// How many operands the operation reads.
    pub fn operands(self) -> usize {
        match self {
            Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Pow | Op::Mod | Op::Atan2 | Op::Gt | Op::Lt | Op::Ge | Op::Le | Op::Eq | Op::Ne => 2,
            Op::Where | Op::Clip => 3,
            _ => 1,
        }
    }
}

/// Where an operand's numbers lie in its buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub offset: usize,
    pub strides: Vec<usize>,
}

impl Layout {
    pub fn plain(shape: &[usize]) -> Layout {
        let mut strides = vec![1; shape.len()];
        for d in (0..shape.len().saturating_sub(1)).rev() {
            strides[d] = strides[d + 1] * shape[d + 1];
        }
        Layout { offset: 0, strides }
    }
}

/// An operand of an element-wise kernel.
pub enum Operand<'a> {
    Numbers(&'a GpuBuffer, Layout),
    Number(f32),
}

pub fn elementwise(ctx: &GpuContext, _op: Op, _shape: &[usize], _operands: &[Operand]) -> Option<GpuBuffer> {
    match *ctx {}
}

pub fn place(ctx: &GpuContext, _src: &GpuBuffer, _from: &Layout, _dst: &GpuBuffer, _to: &Layout, _shape: &[usize]) -> bool {
    match *ctx {}
}

/// A reduction along an axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reduce {
    Sum,
    Max,
    Min,
    Prod,
}

pub fn reduce_axis(ctx: &GpuContext, _op: Reduce, _src: &GpuBuffer, _offset: usize, _shape: &[usize], _strides: &[usize], _axis: usize) -> Option<GpuBuffer> {
    match *ctx {}
}

pub fn gather(ctx: &GpuContext, _src: &GpuBuffer, _offset: usize, _at: &[u32]) -> Option<GpuBuffer> {
    match *ctx {}
}

pub fn scatter_once(ctx: &GpuContext, _g: &GpuBuffer, _offset: usize, _n: usize, _at: &[u32]) -> Option<GpuBuffer> {
    match *ctx {}
}

pub fn scatter_once_at(ctx: &GpuContext, _g: &GpuBuffer, _offset: usize, _n: usize, _at: &GpuBuffer) -> Option<GpuBuffer> {
    match *ctx {}
}

#[expect(clippy::too_many_arguments, reason = "gpu.rs's signature: the reduction, the tensor's buffer and layout, and the axis")]
pub fn extreme_places(ctx: &GpuContext, _op: Reduce, _src: &GpuBuffer, _offset: usize, _shape: &[usize], _strides: &[usize], _axis: usize) -> Option<GpuBuffer> {
    match *ctx {}
}

pub fn scatter(ctx: &GpuContext, _g: &GpuBuffer, _offset: usize, _n: usize, _starts: &[u32], _reads: &[u32]) -> Option<GpuBuffer> {
    match *ctx {}
}

pub fn sum(ctx: &GpuContext, _src: &GpuBuffer, _n: usize) -> f64 {
    match *ctx {}
}

pub fn total(ctx: &GpuContext, _src: &GpuBuffer, _n: usize) -> Option<GpuBuffer> {
    match *ctx {}
}

/// Where a matrix's numbers lie in its buffer.
#[derive(Clone, Copy, Debug)]
pub struct Matrix {
    pub offset: usize,
    pub row: usize,
    pub col: usize,
}

#[expect(clippy::too_many_arguments, reason = "gpu.rs's signature: the two operands and their places, and the product's three sizes")]
pub fn matmul(ctx: &GpuContext, _a: &GpuBuffer, _at: Matrix, _b: &GpuBuffer, _bt: Matrix, _m: usize, _k: usize, _n: usize) -> Option<GpuBuffer> {
    match *ctx {}
}

pub fn gpu_available() -> bool {
    false
}
