//! The GPU's surface in a program that uses none (the `gpu` feature off,
//! crow's emitter asking for it only where `gpu[…]` is written): there is
//! never a context, so every tensor operation takes its CPU path, and the
//! program links none of the system frameworks a GPU needs — Metal,
//! QuartzCore, Foundation, CoreGraphics — whose loading had cost every
//! compiled program four of its seven megabytes before it did anything.

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

/// No GPU: no value of this type exists.
pub enum GpuContext {}

impl GpuContext {
    pub fn get() -> Option<&'static GpuContext> {
        None
    }
}

/// A GPU buffer: none is ever made without a context.
#[derive(Debug)]
pub enum GpuBuffer {}

impl GpuBuffer {
    pub fn from_f32(ctx: &GpuContext, _data: &[f32]) -> Self {
        match *ctx {}
    }
    pub fn from_f64(ctx: &GpuContext, _data: &[f64]) -> Self {
        match *ctx {}
    }
    pub fn to_f32(&self, _ctx: &GpuContext) -> Vec<f32> {
        match *self {}
    }
    pub fn to_f64(&self, _ctx: &GpuContext) -> Vec<f64> {
        match *self {}
    }
    pub fn zeros(ctx: &GpuContext, _len: usize) -> Self {
        match *ctx {}
    }
}

#[derive(Clone, Copy, Debug)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Copy, Debug)]
pub enum UnaryOp {
    Neg,
    Abs,
    Exp,
    Log,
    Sqrt,
    Sin,
    Cos,
    Tan,
}

#[derive(Clone, Copy, Debug)]
pub enum ScalarOp {
    Add,
    Mul,
}

pub fn gpu_elementwise_binary(ctx: &GpuContext, _a: &GpuBuffer, _b: &GpuBuffer, _op: BinaryOp) -> GpuBuffer {
    match *ctx {}
}

pub fn gpu_elementwise_unary(ctx: &GpuContext, _input: &GpuBuffer, _op: UnaryOp) -> GpuBuffer {
    match *ctx {}
}

pub fn gpu_scalar_op(ctx: &GpuContext, _input: &GpuBuffer, _scalar: f32, _op: ScalarOp) -> GpuBuffer {
    match *ctx {}
}

pub fn gpu_matmul(ctx: &GpuContext, _a: &GpuBuffer, _b: &GpuBuffer, _m: u32, _k: u32, _n: u32) -> GpuBuffer {
    match *ctx {}
}

pub fn gpu_sum(ctx: &GpuContext, _input: &GpuBuffer) -> f64 {
    match *ctx {}
}

pub fn gpu_available() -> bool {
    false
}

pub fn gpu_memory_info(ctx: &GpuContext) -> (u64, u64) {
    match *ctx {}
}

pub fn gpu_try_alloc(ctx: &GpuContext, _len: usize) -> Option<GpuBuffer> {
    match *ctx {}
}
