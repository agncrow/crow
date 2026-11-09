//! The GPU (spec, GPU Support): tensors held as f32 in wgpu buffers, and
//! the kernels that compute on them where they lie. A program that writes
//! `gpu[…]` places the tensors it makes there (tensor.rs, `maybe_to_gpu`),
//! and an operation meeting one runs here: an element-wise one over any
//! view of its operands, broadcast, a number beside a tensor
//! (`elementwise`); a reduction along an axis (`reduce_axis`) or of the
//! whole (`sum`); the matrix product, its operands read through their
//! strides (`matmul`); a copy into a place of another buffer (`place`).
//! Anything else reads the numbers back to the CPU (`GpuBuffer::read`).
//!
//! Each kernel is compiled once, the first time it runs (`GpuContext::kernel`),
//! and what one call needs to know — its operation, shapes, strides,
//! offsets and numbers — reaches it as data (`info`): a shader had been
//! written and compiled for every operation, a number beside a tensor
//! written into its text.
//!
//! The device's own math runs the kernels (Metal compiles them with fast
//! math): a NaN or an infinity may come out of an operation where the
//! CPU's would not, and a number is held to f32's digits.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// The GPU, found the first time a program asks for one.
static GPU_CONTEXT: OnceLock<Option<GpuContext>> = OnceLock::new();

/// `gpu[]`: tensors made from here on are placed on the GPU.
static GPU_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enable GPU mode — subsequent tensor creation will place data on GPU.
pub fn set_gpu_enabled(enabled: bool) {
    GPU_ENABLED.store(enabled, Ordering::SeqCst);
}

/// Check if GPU mode is enabled.
pub fn is_gpu_enabled() -> bool {
    GPU_ENABLED.load(Ordering::SeqCst)
}

/// The most dimensions a kernel's layout holds: a tensor of more is
/// computed on the CPU.
pub const MAX_DIMS: usize = 8;

/// The threads of a workgroup of the one-dimensional kernels.
const GROUP: usize = 256;

/// The operations sent to the device together (`Pending`).
const BATCH: usize = 64;

/// The wgpu device and queue, and the kernels compiled for them.
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    kernels: Mutex<HashMap<KernelKey, Arc<Kernel>>>,
    /// a buffer of four numbers, bound where a kernel reads nothing
    spare: wgpu::Buffer,
    /// the most numbers a buffer a kernel reads or writes may hold
    most: usize,
    /// the operations asked for and not yet sent to the device
    pending: Mutex<Pending>,
    /// buffers let go, kept for the next answer of their size
    pool: Mutex<Pool>,
}

/// Buffers let go, by their size in bytes, kept for the next answer of that
/// size (a network's step makes the same sizes each time): a buffer wgpu
/// makes is cleared to zeros before its first use, and the clear had been
/// a second pass over every answer's numbers. A buffer kept is written
/// only by an operation asked for after every one that read it, and the
/// device runs them in that order. At most `POOL_MOST` bytes are kept.
struct Pool {
    by_size: HashMap<u64, Vec<wgpu::Buffer>>,
    held: u64,
}

/// The bytes the pool keeps at most.
const POOL_MOST: u64 = 1 << 30;

/// Operations asked for and not yet sent: dispatched in one compute pass
/// of one encoder (WebGPU makes each dispatch its own scope, so what one
/// writes the next reads) and sent together where a number is read back,
/// or once there are `BATCH` of them. Each had been a pass of its own,
/// sent alone, and beginning and ending the pass was nine tenths of an
/// operation's cost (a network's step runs hundreds). A batch waits for
/// the one sent before it, so at most two are on the device: a loop that
/// reads nothing back would otherwise ask faster than the device runs,
/// each buffer its operations made held until they ran.
struct Pending {
    encoder: Option<wgpu::CommandEncoder>,
    pass: Option<wgpu::ComputePass<'static>>,
    count: usize,
    sent: Option<wgpu::SubmissionIndex>,
}

/// A kernel compiled: its pipeline, and the layout of what it binds — the
/// call's `info` first, then the buffers it reads, then the one it writes.
/// A kernel's name and the operation it is compiled for, if any.
type KernelKey = (&'static str, Option<u32>);

struct Kernel {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    reads: usize,
}

impl GpuContext {
    /// The GPU, or none where no adapter answers.
    pub fn get() -> Option<&'static GpuContext> {
        GPU_CONTEXT.get_or_init(|| pollster::block_on(Self::init())).as_ref()
    }

    async fn init() -> Option<GpuContext> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok()?;
        // (the adapter's own limits: the defaults had held a buffer to
        // 128 MB, a tensor of 33 million numbers)
        let limits = adapter.limits();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("crow_gpu"),
                required_features: wgpu::Features::empty(),
                required_limits: limits.clone(),
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            })
            .await
            .ok()?;
        // a kernel the device refuses is a bug in these kernels, never the
        // program's: a stop with the device's reason, where wgpu's own
        // handler would have panicked
        device.on_uncaptured_error(Arc::new(|e: wgpu::Error| crate::stops::halt(0, &format!("the GPU refused an operation: {}", e))));
        let spare = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("crow_spare"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let bytes = u64::from(limits.max_storage_buffer_binding_size).min(limits.max_buffer_size);
        let most = usize::try_from(bytes / 4).unwrap_or(usize::MAX).min(u32::MAX as usize);
        let pending = Mutex::new(Pending { encoder: None, pass: None, count: 0, sent: None });
        let pool = Mutex::new(Pool { by_size: HashMap::new(), held: 0 });
        Some(GpuContext { device, queue, kernels: Mutex::new(HashMap::new()), spare, most, pending, pool })
    }

    /// Whether a buffer of `n` numbers is one the kernels take: none of no
    /// numbers (wgpu binds no empty buffer), none past the device's limit.
    pub fn holds(&self, n: usize) -> bool {
        n > 0 && n <= self.most
    }

    /// The kernel of `name`, compiled from `source` the first time it is
    /// asked for.
    fn kernel(&self, name: &'static str, source: &'static str, reads: usize) -> Arc<Kernel> {
        self.kernel_of(name, source, reads, None)
    }

    /// A kernel compiled for one operation, its shader's `OP` a constant
    /// the compiler folds: the element-wise kernel's other cases are left
    /// out, and the registers they held (one shader for every operation
    /// had run a sum of two 2 MB tensors at a third of the memory's speed).
    fn kernel_of(&self, name: &'static str, source: &'static str, reads: usize, op: Option<u32>) -> Arc<Kernel> {
        let mut kernels = self.kernels.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(k) = kernels.get(&(name, op)) {
            return Arc::clone(k);
        }
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(name),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let mut entries = vec![storage_entry(0, true)];
        for r in 0..reads {
            entries.push(storage_entry(1 + r as u32, true));
        }
        entries.push(storage_entry(1 + reads as u32, false));
        let layout = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(name), entries: &entries });
        let pipeline_layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(name),
            bind_group_layouts: &[&layout],
            ..Default::default()
        });
        let constants = op.map(|op| [("OP", f64::from(op))]);
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(name),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            // (every kernel writes the workgroup memory it reads before it
            // reads it: the zeros wgpu would write first are never read)
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: constants.as_ref().map_or(&[], |c| c.as_slice()),
                zero_initialize_workgroup_memory: false,
            },
            cache: None,
        });
        let k = Arc::new(Kernel { pipeline, layout, reads });
        kernels.insert((name, op), Arc::clone(&k));
        k
    }

    /// One run of a kernel: its `info`, the buffers it reads (the spare
    /// where it reads none at a binding) and the one it writes, over a grid
    /// of workgroups.
    fn run(&self, kernel: &Kernel, info: &[u32], reads: &[&wgpu::Buffer], out: &wgpu::Buffer, grid: [u32; 3]) {
        use wgpu::util::DeviceExt;
        let info = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("crow_info"),
            contents: bytemuck::cast_slice(info),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: info.as_entire_binding() }];
        for r in 0..kernel.reads {
            let buffer = reads.get(r).copied().unwrap_or(&self.spare);
            entries.push(wgpu::BindGroupEntry { binding: 1 + r as u32, resource: buffer.as_entire_binding() });
        }
        entries.push(wgpu::BindGroupEntry { binding: 1 + kernel.reads as u32, resource: out.as_entire_binding() });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &kernel.layout, entries: &entries });
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if pending.pass.is_none() {
            let encoder = pending.encoder.get_or_insert_with(|| self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None }));
            pending.pass = Some(encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None }).forget_lifetime());
        }
        if let Some(pass) = pending.pass.as_mut() {
            pass.set_pipeline(&kernel.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(grid[0], grid[1], grid[2]);
        }
        pending.count += 1;
        if pending.count >= BATCH {
            self.send(&mut pending, true);
        }
    }

    /// A command of the encoder's own (a copy) recorded among those not
    /// yet sent, after the pass of the kernels before it.
    fn record(&self, f: impl FnOnce(&mut wgpu::CommandEncoder)) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.pass = None;
        let encoder = pending.encoder.get_or_insert_with(|| self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None }));
        f(encoder);
        pending.count += 1;
        if pending.count >= BATCH {
            self.send(&mut pending, true);
        }
    }

    /// The operations not yet sent, sent: after the batch before them has
    /// run where `wait` says.
    fn send(&self, pending: &mut Pending, wait: bool) {
        // (the pass ended before its encoder is)
        pending.pass = None;
        if let Some(encoder) = pending.encoder.take() {
            if let Some(before) = pending.sent.take().filter(|_| wait) {
                let _ = self.device.poll(wgpu::PollType::Wait { submission_index: Some(before), timeout: None });
            }
            pending.sent = Some(self.queue.submit(Some(encoder.finish())));
        }
        pending.count = 0;
    }
}

impl GpuContext {
    /// A buffer of `bytes` the pool keeps, if it keeps one.
    fn kept(&self, bytes: u64) -> Option<wgpu::Buffer> {
        let mut pool = self.pool.lock().unwrap_or_else(|e| e.into_inner());
        let buffer = pool.by_size.get_mut(&bytes)?.pop()?;
        pool.held -= bytes;
        Some(buffer)
    }

    /// A buffer let go, kept while the pool has room.
    fn keep(&self, buffer: wgpu::Buffer) {
        let bytes = buffer.size();
        let mut pool = self.pool.lock().unwrap_or_else(|e| e.into_inner());
        if pool.held + bytes <= POOL_MOST {
            pool.held += bytes;
            pool.by_size.entry(bytes).or_default().push(buffer);
        }
    }
}

/// The workgroups that run `threads` threads `GROUP` to a group: as many
/// as one dimension of the grid holds, the rest along a second (a tensor
/// past 16 million numbers had asked for more groups than one dimension
/// holds). The kernels read their thread's number as
/// `gid.x + gid.y * groups.x * GROUP`.
fn grid(threads: usize) -> [u32; 3] {
    grid_of(threads.div_ceil(GROUP))
}

/// `groups` workgroups laid over the grid's first two dimensions, as many
/// along the first as it holds.
fn grid_of(groups: usize) -> [u32; 3] {
    let groups = u32::try_from(groups).unwrap_or(u32::MAX).max(1);
    let x = groups.min(65_535);
    [x, groups.div_ceil(x), 1]
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// A GPU buffer holding f32 data.
#[derive(Debug)]
pub struct GpuBuffer {
    pub buffer: wgpu::Buffer,
    pub len: usize, // number of f32 elements
    /// The numbers, once read back whole: a buffer is written only before
    /// it is a tensor's, so a second read is answered here (a sampler's
    /// loop over one row of 59 logits had read each number back alone, a
    /// round trip of a millisecond and a half apiece).
    host: OnceLock<Vec<f32>>,
}

/// The most numbers a read of part of a buffer brings back whole, to keep
/// for the next: a vector of parameters, millions of numbers, read a slice
/// at a time, is read a slice at a time.
const HOST_MOST: usize = 1 << 16;

impl GpuBuffer {
    /// The numbers the buffer holds.
    pub fn count(&self) -> usize {
        self.len
    }

    /// A buffer of `n` numbers a kernel writes every one of: one the pool
    /// keeps, as it holds them, else a new one.
    pub fn output(ctx: &GpuContext, n: usize) -> Self {
        let bytes = (n.max(1) * 4) as u64;
        let buffer = ctx.kept(bytes).unwrap_or_else(|| Self::made(ctx, bytes));
        GpuBuffer::of(buffer, n)
    }

    /// A buffer's numbers as a tensor's storage, none read back yet.
    fn of(buffer: wgpu::Buffer, len: usize) -> Self {
        GpuBuffer { buffer, len, host: OnceLock::new() }
    }

    /// A buffer of `n` zeros: a new one (wgpu zeroes every buffer it
    /// makes), or one the pool keeps, filled by a kernel in the batch's
    /// pass (a clear between two passes had ended the pass, at a cost of a
    /// pass begun again for each).
    pub fn zeros(ctx: &GpuContext, n: usize) -> Self {
        let bytes = (n.max(1) * 4) as u64;
        let Some(buffer) = ctx.kept(bytes) else {
            return GpuBuffer::of(Self::made(ctx, bytes), n);
        };
        let out = GpuBuffer::of(buffer, n);
        if !fill(ctx, &out, 0.0) {
            ctx.record(|encoder| encoder.clear_buffer(&out.buffer, 0, None));
        }
        out
    }

    /// A new buffer of `bytes`.
    fn made(ctx: &GpuContext, bytes: u64) -> wgpu::Buffer {
        ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("crow_tensor"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Upload f32 data to GPU.
    pub fn from_f32(ctx: &GpuContext, data: &[f32]) -> Self {
        use wgpu::util::DeviceExt;
        if data.is_empty() {
            return Self::zeros(ctx, 0);
        }
        let buffer = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("crow_tensor"),
            contents: bytemuck::cast_slice(data),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        });
        let out = GpuBuffer::of(buffer, data.len());
        // (a small upload keeps the numbers it was made of, so a read of
        // them is no round trip: a batch's ids, the positions `range[t]`)
        if data.len() <= HOST_MOST {
            let _ = out.host.set(data.to_vec());
        }
        out
    }

    /// Whole numbers uploaded as they are: the positions a gather reads.
    pub fn from_u32(ctx: &GpuContext, data: &[u32]) -> Self {
        use wgpu::util::DeviceExt;
        if data.is_empty() {
            return Self::zeros(ctx, 0);
        }
        let buffer = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("crow_positions"),
            contents: bytemuck::cast_slice(data),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        });
        GpuBuffer::of(buffer, data.len())
    }

    /// Upload f64 CPU data to GPU as f32.
    pub fn from_f64(ctx: &GpuContext, data: &[f64]) -> Self {
        let f32_data: Vec<f32> = data.iter().map(|&x| x as f32).collect();
        Self::from_f32(ctx, &f32_data)
    }

    /// The `len` numbers from `start`, read back from the device: the
    /// whole buffer where it is small or asked for whole, kept for the
    /// next read, else only the run asked for (a slice of a parameter
    /// vector had read back all of it); none of an empty run (wgpu maps no
    /// empty slice).
    pub fn read(&self, ctx: &GpuContext, start: usize, len: usize) -> Vec<f32> {
        let len = len.min(self.len.saturating_sub(start));
        if len == 0 {
            return Vec::new();
        }
        if let Some(host) = self.host.get() {
            return host.get(start..start + len).map(<[f32]>::to_vec).unwrap_or_default();
        }
        if self.len <= HOST_MOST || len == self.len {
            let all = self.host.get_or_init(|| self.fetched(ctx, 0, self.len));
            return all.get(start..start + len).map(<[f32]>::to_vec).unwrap_or_default();
        }
        self.fetched(ctx, start, len)
    }

    /// The `len` numbers from `start` (a run inside the buffer), fetched
    /// from the device after every operation asked for before.
    fn fetched(&self, ctx: &GpuContext, start: usize, len: usize) -> Vec<f32> {
        let bytes = (len * 4) as u64;
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("crow_staging"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // (after every operation asked for, all sent at once)
        {
            let mut pending = ctx.pending.lock().unwrap_or_else(|e| e.into_inner());
            pending.pass = None;
            let encoder = pending.encoder.get_or_insert_with(|| ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None }));
            encoder.copy_buffer_to_buffer(&self.buffer, (start * 4) as u64, &staging, 0, bytes);
            ctx.send(&mut pending, false);
        }
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            // the receiver waits below for exactly this
            let _ = tx.send(result);
        });
        let _ = ctx.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        // a device that cannot hand its results back is the machine failing,
        // not the program: a stop with its reason (docs/bug-classes.md §3)
        match rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => crate::stops::halt(0, &format!("the GPU could not hand its results back: {}", e)),
            Err(_) => crate::stops::halt(0, "the GPU could not hand its results back"),
        }
        let data = slice.get_mapped_range();
        let result: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        staging.unmap();
        result
    }

    /// The whole-number positions a kernel wrote (`extreme_places`), read
    /// back.
    pub fn places(&self) -> Vec<usize> {
        GpuContext::get().map(|ctx| self.read(ctx, 0, self.len).iter().map(|x| x.to_bits() as usize).collect()).unwrap_or_default()
    }

    /// Download GPU data back to CPU as f32.
    pub fn to_f32(&self, ctx: &GpuContext) -> Vec<f32> {
        self.read(ctx, 0, self.len)
    }

    /// Download GPU data back to CPU as f64.
    pub fn to_f64(&self, ctx: &GpuContext) -> Vec<f64> {
        self.to_f32(ctx).iter().map(|&x| f64::from(x)).collect()
    }
}

/// A buffer let go goes back to the pool (`Pool`).
impl Drop for GpuBuffer {
    fn drop(&mut self) {
        if let Some(Some(ctx)) = GPU_CONTEXT.get() {
            ctx.keep(self.buffer.clone());
        }
    }
}

/// `len` numbers of `src` from `from` copied into `dst` from `to`, on the
/// device.
pub fn copy(ctx: &GpuContext, src: &GpuBuffer, from: usize, dst: &GpuBuffer, to: usize, len: usize) {
    if len == 0 || from + len > src.len || to + len > dst.len {
        return;
    }
    ctx.record(|encoder| encoder.copy_buffer_to_buffer(&src.buffer, (from * 4) as u64, &dst.buffer, (to * 4) as u64, (len * 4) as u64));
}

// ── Element-wise kernels ──────────────────────────────────────────────

/// An element-wise operation: a closed set, each one's code the kernel's
/// (an operation of one operand below 32, of two below 64, of three
/// after), so no operation the kernel lacks can be asked for.
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
    /// the operand's numbers, as they are: a view laid out in order
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
    /// the second operand's number where the first's is not 0, the third's
    /// where it is
    Where,
    /// the first operand's number held between the second's and the third's
    Clip,
}

impl Op {
    fn code(self) -> u32 {
        match self {
            Op::Neg => 0,
            Op::Abs => 1,
            Op::Exp => 2,
            Op::Log => 3,
            Op::Sqrt => 4,
            Op::Sin => 5,
            Op::Cos => 6,
            Op::Tan => 7,
            Op::Tanh => 8,
            Op::Sinh => 9,
            Op::Cosh => 10,
            Op::Asin => 11,
            Op::Acos => 12,
            Op::Atan => 13,
            Op::Log2 => 14,
            Op::Log10 => 15,
            Op::Log1p => 16,
            Op::Expm1 => 17,
            Op::Floor => 18,
            Op::Ceil => 19,
            Op::Round => 20,
            Op::Sign => 21,
            Op::Erf => 22,
            Op::Copy => 23,
            Op::Add => 32,
            Op::Sub => 33,
            Op::Mul => 34,
            Op::Div => 35,
            Op::Pow => 36,
            Op::Mod => 37,
            Op::Atan2 => 38,
            Op::Gt => 39,
            Op::Lt => 40,
            Op::Ge => 41,
            Op::Le => 42,
            Op::Eq => 43,
            Op::Ne => 44,
            Op::Where => 64,
            Op::Clip => 65,
        }
    }

    /// How many operands the operation reads.
    pub fn operands(self) -> usize {
        match self.code() {
            0..=31 => 1,
            32..=63 => 2,
            _ => 3,
        }
    }
}

/// Where an operand's numbers lie in its buffer for a kernel over an
/// answer of some shape: the offset of the first, and a stride for each of
/// the answer's dimensions — 0 along one the operand is broadcast over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub offset: usize,
    pub strides: Vec<usize>,
}

impl Layout {
    /// The layout of a buffer read in order over `shape`.
    pub fn plain(shape: &[usize]) -> Layout {
        let mut strides = vec![1; shape.len()];
        for d in (0..shape.len().saturating_sub(1)).rev() {
            strides[d] = strides[d + 1] * shape[d + 1];
        }
        Layout { offset: 0, strides }
    }

    /// The last position the layout reads over `shape`, where every
    /// dimension holds a number.
    fn last(&self, shape: &[usize]) -> usize {
        self.offset + shape.iter().zip(&self.strides).map(|(n, s)| n.saturating_sub(1) * s).sum::<usize>()
    }
}

/// An operand of an element-wise kernel: numbers in a buffer, read through
/// a layout, or one number beside them.
pub enum Operand<'a> {
    Numbers(&'a GpuBuffer, Layout),
    Number(f32),
}

/// An element-wise operation over an answer of `shape`: each number of the
/// answer computed from the operands' numbers at its place — or none, where
/// the kernel cannot take it (more dimensions than `MAX_DIMS`, more numbers
/// than a buffer holds, an operand read past its buffer).
pub fn elementwise(ctx: &GpuContext, op: Op, shape: &[usize], operands: &[Operand]) -> Option<GpuBuffer> {
    let n: usize = shape.iter().product();
    if shape.len() > MAX_DIMS || !ctx.holds(n) || operands.len() != op.operands() {
        return None;
    }
    let mut info = vec![0u32; 42];
    info[0] = u32::try_from(n).ok()?;
    info[1] = shape.len() as u32;
    info[2] = op.code();
    for (d, &size) in shape.iter().enumerate() {
        info[4 + d] = u32::try_from(size).ok()?;
    }
    let plain = Layout::plain(shape);
    let mut reads: Vec<&wgpu::Buffer> = Vec::new();
    for (k, operand) in operands.iter().enumerate() {
        match operand {
            Operand::Number(x) => {
                info[3] |= 1 << k;
                info[39 + k] = x.to_bits();
                reads.push(&ctx.spare);
            }
            Operand::Numbers(buf, layout) => {
                if layout.strides.len() != shape.len() || layout.last(shape) >= buf.len {
                    return None;
                }
                info[12 + 9 * k] = u32::try_from(layout.offset).ok()?;
                for (d, &s) in layout.strides.iter().enumerate() {
                    info[13 + 9 * k + d] = u32::try_from(s).ok()?;
                }
                // (an operand read in order is read at its offset plus the
                // thread's number, with no division per dimension)
                if layout.strides == plain.strides {
                    info[3] |= 1 << (4 + k);
                }
                reads.push(&buf.buffer);
            }
        }
    }
    let out = GpuBuffer::output(ctx, n);
    let kernel = ctx.kernel_of("elementwise", ELEMENTWISE, 3, Some(op.code()));
    ctx.run(&kernel, &info, &reads, &out.buffer, grid(n));
    Some(out)
}

/// Every number of `out` set to `x`, by the element-wise kernel's copy of
/// a number. False where the kernel cannot take so many.
fn fill(ctx: &GpuContext, out: &GpuBuffer, x: f32) -> bool {
    let Some(count) = u32::try_from(out.len).ok().filter(|_| ctx.holds(out.len)) else { return false };
    let mut info = vec![0u32; 42];
    info[0] = count;
    info[1] = 1;
    info[2] = Op::Copy.code();
    info[3] = 1;
    info[4] = count;
    info[39] = x.to_bits();
    let kernel = ctx.kernel_of("elementwise", ELEMENTWISE, 3, Some(Op::Copy.code()));
    ctx.run(&kernel, &info, &[], &out.buffer, grid(out.len));
    true
}

/// The numbers of `src`, read through `from` over `shape`, written into
/// `dst` at the places `to` names over the same shape — a slice's
/// gradient laid into the zeros of its whole, two tensors joined along an
/// axis. False where the kernel cannot take it.
pub fn place(ctx: &GpuContext, src: &GpuBuffer, from: &Layout, dst: &GpuBuffer, to: &Layout, shape: &[usize]) -> bool {
    let n: usize = shape.iter().product();
    if n == 0 {
        return true;
    }
    if shape.len() > MAX_DIMS || !ctx.holds(n) || from.strides.len() != shape.len() || to.strides.len() != shape.len() || from.last(shape) >= src.len || to.last(shape) >= dst.len {
        return false;
    }
    let mut info = vec![0u32; 42];
    let fits = |x: usize| u32::try_from(x).ok();
    let Some(count) = fits(n) else { return false };
    info[0] = count;
    info[1] = shape.len() as u32;
    for (d, &size) in shape.iter().enumerate() {
        info[4 + d] = size as u32;
    }
    for (k, layout) in [from, to].into_iter().enumerate() {
        let Some(offset) = fits(layout.offset) else { return false };
        info[12 + 9 * k] = offset;
        for (d, &s) in layout.strides.iter().enumerate() {
            let Some(s) = fits(s) else { return false };
            info[13 + 9 * k + d] = s;
        }
    }
    let kernel = ctx.kernel("place", PLACE, 1);
    ctx.run(&kernel, &info, &[&src.buffer], &dst.buffer, grid(n));
    true
}

// ── Reductions ──────────────────────────────────────────────────────────

/// A reduction along an axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reduce {
    Sum,
    Max,
    Min,
    Prod,
}

/// The numbers of a tensor of `shape`, read through `offset` and
/// `strides`, reduced along `axis`: the answer has the shape without it,
/// in order. None where the kernel cannot take it (the answer is a single
/// number — the caller's `sum` — or too many dimensions or numbers).
pub fn reduce_axis(ctx: &GpuContext, op: Reduce, src: &GpuBuffer, offset: usize, shape: &[usize], strides: &[usize], axis: usize) -> Option<GpuBuffer> {
    if axis >= shape.len() || shape.len() < 2 || shape.len() > MAX_DIMS + 1 || strides.len() != shape.len() {
        return None;
    }
    let mut kept = shape.to_vec();
    let along = kept.remove(axis);
    let mut kept_strides = strides.to_vec();
    let along_stride = kept_strides.remove(axis);
    let n: usize = kept.iter().product();
    let last = offset + shape.iter().zip(strides).map(|(n, s)| n.saturating_sub(1) * s).sum::<usize>();
    if !ctx.holds(n) || last >= src.len || along == 0 {
        return None;
    }
    let mut info = vec![0u32; 22];
    info[0] = u32::try_from(n).ok()?;
    info[1] = kept.len() as u32;
    info[2] = match op {
        Reduce::Sum => 0,
        Reduce::Max => 1,
        Reduce::Min => 2,
        Reduce::Prod => 3,
    };
    info[3] = u32::try_from(along).ok()?;
    for (d, &size) in kept.iter().enumerate() {
        info[4 + d] = u32::try_from(size).ok()?;
        info[13 + d] = u32::try_from(kept_strides[d]).ok()?;
    }
    info[12] = u32::try_from(offset).ok()?;
    info[21] = u32::try_from(along_stride).ok()?;
    let out = GpuBuffer::output(ctx, n);
    // (a long axis shared among sixteen threads of each answer: a bias's
    // gradient, a batch's 512 rows summed for each of 1024 columns, had
    // run on 1024 threads walking the rows one by one)
    if along >= 32 {
        let kernel = ctx.kernel("reduce_split", REDUCE_SPLIT, 1);
        ctx.run(&kernel, &info, &[&src.buffer], &out.buffer, grid_of(n.div_ceil(16)));
    } else {
        let kernel = ctx.kernel("reduce_axis", REDUCE_AXIS, 1);
        ctx.run(&kernel, &info, &[&src.buffer], &out.buffer, grid(n));
    }
    Some(out)
}

/// The numbers of `src` at `offset` plus each of `at` (positions the
/// caller checked): a gather, `t[positions]`.
pub fn gather(ctx: &GpuContext, src: &GpuBuffer, offset: usize, at: &[u32]) -> Option<GpuBuffer> {
    let n = at.len();
    if !ctx.holds(n) {
        return None;
    }
    let info = [u32::try_from(n).ok()?, u32::try_from(offset).ok()?];
    let positions = GpuBuffer::from_u32(ctx, at);
    let out = GpuBuffer::output(ctx, n);
    let kernel = ctx.kernel("gather", GATHER, 2);
    ctx.run(&kernel, &info, &[&src.buffer, &positions.buffer], &out.buffer, grid(n));
    Some(out)
}

/// A gather's gradient where each of `n` numbers was read at most once:
/// zeros, and `g`'s k-th number (from `offset`) where `at[k]` was read.
pub fn scatter_once(ctx: &GpuContext, g: &GpuBuffer, offset: usize, n: usize, at: &[u32]) -> Option<GpuBuffer> {
    if at.is_empty() {
        return ctx.holds(n).then(|| GpuBuffer::zeros(ctx, n));
    }
    scatter_once_at(ctx, g, offset, n, &GpuBuffer::from_u32(ctx, at))
}

/// `scatter_once` of places the device holds (`extreme_places`'s).
pub fn scatter_once_at(ctx: &GpuContext, g: &GpuBuffer, offset: usize, n: usize, at: &GpuBuffer) -> Option<GpuBuffer> {
    if !ctx.holds(n) || at.len > n {
        return None;
    }
    let out = GpuBuffer::zeros(ctx, n);
    if at.len == 0 {
        return Some(out);
    }
    let info = [u32::try_from(at.len).ok()?, u32::try_from(offset).ok()?];
    let kernel = ctx.kernel("scatter_once", SCATTER_ONCE, 2);
    ctx.run(&kernel, &info, &[&g.buffer, &at.buffer], &out.buffer, grid(at.len));
    Some(out)
}

/// Where along `axis` each slice's largest (`Reduce::Max`) or smallest
/// (`Reduce::Min`) number lies — the first that holds it, as the CPU's
/// `max` and `min` along an axis pass their gradient — as whole-number
/// positions in the tensor read in order. The tensor is `shape` read from
/// `offset` through `strides`.
pub fn extreme_places(ctx: &GpuContext, op: Reduce, src: &GpuBuffer, offset: usize, shape: &[usize], strides: &[usize], axis: usize) -> Option<GpuBuffer> {
    if axis >= shape.len() || shape.len() > MAX_DIMS + 1 || strides.len() != shape.len() || !matches!(op, Reduce::Max | Reduce::Min) {
        return None;
    }
    let mut kept = shape.to_vec();
    let along = kept.remove(axis);
    let mut kept_strides = strides.to_vec();
    let along_stride = kept_strides.remove(axis);
    let mut in_order = Layout::plain(shape).strides;
    let along_in_order = in_order.remove(axis);
    let n: usize = kept.iter().product();
    let last = offset + shape.iter().zip(strides).map(|(n, s)| n.saturating_sub(1) * s).sum::<usize>();
    if !ctx.holds(n) || last >= src.len || along == 0 {
        return None;
    }
    let mut info = vec![0u32; 31];
    info[0] = u32::try_from(n).ok()?;
    info[1] = kept.len() as u32;
    info[2] = if op == Reduce::Max { 1 } else { 2 };
    info[3] = u32::try_from(along).ok()?;
    for (d, &size) in kept.iter().enumerate() {
        info[4 + d] = u32::try_from(size).ok()?;
        info[13 + d] = u32::try_from(kept_strides[d]).ok()?;
        info[22 + d] = u32::try_from(in_order[d]).ok()?;
    }
    info[12] = u32::try_from(offset).ok()?;
    info[21] = u32::try_from(along_stride).ok()?;
    info[30] = u32::try_from(along_in_order).ok()?;
    let out = GpuBuffer::output(ctx, n);
    let kernel = ctx.kernel("extreme_places", EXTREME_PLACES, 1);
    ctx.run(&kernel, &info, &[&src.buffer], &out.buffer, grid(n));
    Some(out)
}

/// A gather's gradient where a number may have been read more than once:
/// each of the `n` numbers the sum of `g`'s numbers (from `offset`) read
/// from it, in the order they were read — `reads[starts[j]..starts[j +
/// 1]]` are the places of `g` read from number j — as the CPU sums them.
pub fn scatter(ctx: &GpuContext, g: &GpuBuffer, offset: usize, n: usize, starts: &[u32], reads: &[u32]) -> Option<GpuBuffer> {
    if !ctx.holds(n) || starts.len() != n + 1 || reads.is_empty() {
        return None;
    }
    let info = [u32::try_from(n).ok()?, u32::try_from(offset).ok()?];
    let first = GpuBuffer::from_u32(ctx, starts);
    let places = GpuBuffer::from_u32(ctx, reads);
    let out = GpuBuffer::output(ctx, n);
    let kernel = ctx.kernel("scatter", SCATTER, 3);
    ctx.run(&kernel, &info, &[&g.buffer, &first.buffer, &places.buffer], &out.buffer, grid(n));
    Some(out)
}

/// The sum of a buffer's first `n` numbers: a few read back and summed on
/// the CPU, more reduced on the device a workgroup at a time until one is
/// left.
pub fn sum(ctx: &GpuContext, src: &GpuBuffer, n: usize) -> f64 {
    let n = n.min(src.len);
    if n <= 1024 {
        return src.read(ctx, 0, n).iter().map(|&x| f64::from(x)).sum();
    }
    total(ctx, src, n).map_or(0.0, |b| b.read(ctx, 0, 1).first().map_or(0.0, |&x| f64::from(x)))
}

/// The sum of a buffer's first `n` numbers left on the device, a buffer of
/// one number: a loss a gradient is taken of, which nothing reads as a
/// number (a read is a round trip to the device, two of them a step of a
/// network's training). None of no numbers.
pub fn total(ctx: &GpuContext, src: &GpuBuffer, n: usize) -> Option<GpuBuffer> {
    let n = n.min(src.len);
    if n == 0 || u32::try_from(n).is_err() {
        return None;
    }
    let kernel = ctx.kernel("sum", SUM, 1);
    let mut left = n;
    let mut current: Option<GpuBuffer> = None;
    // (one number is itself: copied, so the answer is a buffer of its own)
    if left == 1 {
        let out = GpuBuffer::output(ctx, 1);
        copy(ctx, src, 0, &out, 0, 1);
        return Some(out);
    }
    while left > 1 {
        let groups = left.div_ceil(GROUP);
        let out = GpuBuffer::output(ctx, groups);
        let info = [left as u32, 0, 0, 0];
        let from = current.as_ref().map_or(&src.buffer, |b| &b.buffer);
        ctx.run(&kernel, &info, &[from], &out.buffer, grid(left));
        current = Some(out);
        left = groups;
    }
    current
}

// ── The matrix product ──────────────────────────────────────────────────

/// Where a matrix's numbers lie in its buffer: the first's place, and the
/// strides of a row and of a column (a transposed matrix is read through
/// its own, never copied).
#[derive(Clone, Copy, Debug)]
pub struct Matrix {
    pub offset: usize,
    pub row: usize,
    pub col: usize,
}

/// The product of an `m`×`k` matrix and a `k`×`n` one, each read through
/// its place, answered in order — or none where the kernel cannot take
/// it.
#[expect(clippy::too_many_arguments, reason = "the two operands and their places, and the product's three sizes")]
pub fn matmul(ctx: &GpuContext, a: &GpuBuffer, at: Matrix, b: &GpuBuffer, bt: Matrix, m: usize, k: usize, n: usize) -> Option<GpuBuffer> {
    let out_n = m.checked_mul(n)?;
    if !ctx.holds(out_n) || k == 0 || m.div_ceil(64) > 65_535 || n.div_ceil(64) > 65_535 {
        return None;
    }
    let a_last = at.offset + (m - 1) * at.row + (k - 1) * at.col;
    let b_last = bt.offset + (k - 1) * bt.row + (n - 1) * bt.col;
    if a_last >= a.len || b_last >= b.len {
        return None;
    }
    let fits = |x: usize| u32::try_from(x).ok();
    let info = [fits(m)?, fits(k)?, fits(n)?, 0, fits(at.offset)?, fits(at.row)?, fits(at.col)?, fits(bt.offset)?, fits(bt.row)?, fits(bt.col)?];
    let out = GpuBuffer::output(ctx, out_n);
    let kernel = ctx.kernel("matmul", MATMUL, 2);
    ctx.run(&kernel, &info, &[&a.buffer, &b.buffer], &out.buffer, [n.div_ceil(64) as u32, m.div_ceil(64) as u32, 1]);
    Some(out)
}

/// Check if GPU is available.
pub fn gpu_available() -> bool {
    GpuContext::get().is_some()
}

// ── The kernels ─────────────────────────────────────────────────────────

/// Each number of the answer from the operands' at its place. `info`:
/// [0] the answer's numbers, [1] its dimensions, [2] the operation, [3]
/// bit k: operand k is a number (at [39 + k]); bit 4 + k: operand k is
/// read in order from its offset; [4..12] the answer's shape; operand k's
/// offset at [12 + 9k], its strides at [13 + 9k ..].
const ELEMENTWISE: &str = r#"
@group(0) @binding(0) var<storage, read> info: array<u32>;
@group(0) @binding(1) var<storage, read> x0: array<f32>;
@group(0) @binding(2) var<storage, read> x1: array<f32>;
@group(0) @binding(3) var<storage, read> x2: array<f32>;
@group(0) @binding(4) var<storage, read_write> out: array<f32>;

// the operation, fixed when the kernel is compiled for it
override OP: u32 = 0u;

fn nan() -> f32 { return bitcast<f32>(0x7fc00000u); }
fn infinity() -> f32 { return bitcast<f32>(0x7f800000u); }

fn sign_of(x: f32) -> f32 {
    return select(select(0.0, -1.0, x < 0.0), 1.0, x > 0.0);
}

// half away from zero, as the CPU's round (WGSL's rounds half to even);
// the fraction compared exactly, never by adding a half
fn rounded(x: f32) -> f32 {
    let a = abs(x);
    if a >= 8388608.0 || a == 0.0 { return x; }
    let f = floor(a);
    let r = select(f, f + 1.0, a - f >= 0.5);
    return select(r, -r, x < 0.0);
}

// tanh held to ±1 where it is that in f32 (computed through exponentials,
// tanh of a large number is infinity over infinity)
fn tanh_of(x: f32) -> f32 {
    if abs(x) > 9.0 { return sign_of(x); }
    return tanh(x);
}

// ln(1 + x) and exp(x) − 1 by their series near 0, where 1 + x loses x
fn log1p_of(x: f32) -> f32 {
    if abs(x) < 0.1 {
        return x * (1.0 + x * (-0.5 + x * (1.0 / 3.0 + x * (-0.25 + x * (0.2 + x * (-1.0 / 6.0 + x * (1.0 / 7.0 + x * (-0.125))))))));
    }
    return log(1.0 + x);
}

fn expm1_of(x: f32) -> f32 {
    if abs(x) < 0.1 {
        return x * (1.0 + x * (0.5 + x * (1.0 / 6.0 + x * (1.0 / 24.0 + x * (1.0 / 120.0 + x * (1.0 / 720.0 + x * (1.0 / 5040.0)))))));
    }
    return exp(x) - 1.0;
}

// erf: its series below a half, Numerical Recipes' erfc beyond (a relative
// error below 1.2e-7)
fn erf_of(x: f32) -> f32 {
    let a = abs(x);
    if a < 0.5 {
        let s = x * x;
        let p = 1.0 + s * (-1.0 / 3.0 + s * (0.1 + s * (-1.0 / 42.0 + s * (1.0 / 216.0 + s * (-1.0 / 1320.0 + s * (1.0 / 9360.0))))));
        return 1.1283791670955126 * x * p;
    }
    let t = 1.0 / (1.0 + 0.5 * a);
    let r = t * exp(-a * a - 1.26551223 + t * (1.00002368 + t * (0.37409196 + t * (0.09678418 + t * (-0.18628806 + t * (0.27886807 + t * (-1.13520398 + t * (1.48851587 + t * (-0.82215223 + t * 0.17087277)))))))));
    let e = 1.0 - r;
    return select(e, -e, x < 0.0);
}

// The inverse functions from arithmetic alone, as Cephes's single
// precision writes them (a relative error near 2.5e-7): a driver's own
// differ by device — lavapipe's asin was 3.5e-4 off at -0.9 and its atan2
// 3.3e-6 off at 1 over 1, which WGSL allows (atan2 to 4096 ulps, asin
// inherited from it) — and these answer every device's digits alike
fn atan_of(v: f32) -> f32 {
    if v == 0.0 { return v; }
    var x = abs(v);
    var base = 0.0;
    if x > 2.414213562373095 {
        base = 1.5707963267948966;
        x = -1.0 / x;
    } else if x > 0.4142135623730950 {
        base = 0.7853981633974483;
        x = (x - 1.0) / (x + 1.0);
    }
    let z = x * x;
    let p = ((0.0805374449538 * z - 0.138776856032) * z + 0.199777106478) * z - 0.333329491539;
    let r = base + (p * z * x + x);
    return select(r, -r, v < 0.0);
}

fn asin_of(v: f32) -> f32 {
    let a = abs(v);
    if a > 1.0 { return nan(); }
    if a < 1.0e-4 { return v; }
    var x = a;
    var z = a * a;
    let wide = a > 0.5;
    if wide {
        z = 0.5 * (1.0 - a);
        x = sqrt(z);
    }
    var r = ((((0.042163199048 * z + 0.024181311049) * z + 0.045470025998) * z + 0.074953002686) * z + 0.16666752422) * z * x + x;
    if wide {
        r = 1.5707963267948966 - (r + r);
    }
    return select(r, -r, v < 0.0);
}

fn acos_of(v: f32) -> f32 {
    if v < -1.0 || v > 1.0 { return nan(); }
    if v < -0.5 { return 3.141592653589793 - 2.0 * asin_of(sqrt(0.5 * (1.0 + v))); }
    if v > 0.5 { return 2.0 * asin_of(sqrt(0.5 * (1.0 - v))); }
    return 1.5707963267948966 - asin_of(v);
}

// the angle of (x, y), as the CPU's atan2 answers it: a zero's sign read
// from its bits, so (±0, -1) is ±π, and both sides infinite an eighth turn
fn atan2_of(y: f32, x: f32) -> f32 {
    let pi = 3.141592653589793;
    let below = (bitcast<u32>(y) >> 31u) == 1u;
    let left = x < 0.0 || (x == 0.0 && (bitcast<u32>(x) >> 31u) == 1u);
    if y == 0.0 {
        return select(y, select(pi, -pi, below), left);
    }
    if x == 0.0 {
        return select(1.5707963267948966, -1.5707963267948966, below);
    }
    if abs(x) > 3.4028234e38 && abs(y) > 3.4028234e38 {
        let q = select(0.7853981633974483, 2.356194490192345, left);
        return select(q, -q, below);
    }
    let z = atan_of(y / x);
    if left {
        return select(z + pi, z - pi, below);
    }
    return z;
}

// x to the y as the CPU's powf answers it: 1 at a power of 0 and of 1, a
// negative number to a whole power signed by its parity, to any other NaN
fn power(x: f32, y: f32) -> f32 {
    if y == 0.0 || x == 1.0 { return 1.0; }
    if y == 1.0 { return x; }
    if y == 2.0 { return x * x; }
    let a = abs(x);
    if a == 0.0 { return select(infinity(), 0.0, y > 0.0); }
    if x < 0.0 {
        if y != floor(y) { return nan(); }
        let r = pow(a, y);
        let odd = abs(y) < 16777216.0 && (u32(abs(y)) & 1u) == 1u;
        return select(r, -r, odd);
    }
    return pow(a, y);
}

fn one(op: u32, x: f32) -> f32 {
    switch op {
        case 0u: { return -x; }
        case 1u: { return abs(x); }
        case 2u: { return exp(x); }
        case 3u: { return log(x); }
        case 4u: { return sqrt(x); }
        case 5u: { return sin(x); }
        case 6u: { return cos(x); }
        case 7u: { return tan(x); }
        case 8u: { return tanh_of(x); }
        case 9u: { return sinh(x); }
        case 10u: { return cosh(x); }
        case 11u: { return asin_of(x); }
        case 12u: { return acos_of(x); }
        case 13u: { return atan_of(x); }
        case 14u: { return log2(x); }
        case 15u: { return log(x) * 0.43429448190325176; }
        case 16u: { return log1p_of(x); }
        case 17u: { return expm1_of(x); }
        case 18u: { return floor(x); }
        case 19u: { return ceil(x); }
        case 20u: { return rounded(x); }
        case 21u: { return sign_of(x); }
        case 22u: { return erf_of(x); }
        default: { return x; }
    }
}

fn two(op: u32, x: f32, y: f32) -> f32 {
    switch op {
        case 32u: { return x + y; }
        case 33u: { return x - y; }
        case 34u: { return x * y; }
        case 35u: { return x / y; }
        case 36u: { return power(x, y); }
        case 37u: { return x % y; }
        case 38u: { return atan2_of(x, y); }
        case 39u: { return select(0.0, 1.0, x > y); }
        case 40u: { return select(0.0, 1.0, x < y); }
        case 41u: { return select(0.0, 1.0, x >= y); }
        case 42u: { return select(0.0, 1.0, x <= y); }
        case 43u: { return select(0.0, 1.0, x == y); }
        default: { return select(0.0, 1.0, x != y); }
    }
}

fn operand(i: u32, k: u32) -> f32 {
    let kinds = info[3];
    if ((kinds >> k) & 1u) == 1u {
        return bitcast<f32>(info[39u + k]);
    }
    var at = info[12u + 9u * k];
    if ((kinds >> (4u + k)) & 1u) == 1u {
        at = at + i;
    } else {
        var rest = i;
        var d = info[1];
        loop {
            if d == 0u { break; }
            d = d - 1u;
            let size = info[4u + d];
            at = at + (rest % size) * info[13u + 9u * k + d];
            rest = rest / size;
        }
    }
    switch k {
        case 0u: { return x0[at]; }
        case 1u: { return x1[at]; }
        default: { return x2[at]; }
    }
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = gid.x + gid.y * groups.x * 256u;
    if i >= info[0] { return; }
    let op = OP;
    if op < 32u {
        out[i] = one(op, operand(i, 0u));
    } else if op < 64u {
        out[i] = two(op, operand(i, 0u), operand(i, 1u));
    } else if op == 64u {
        out[i] = select(operand(i, 2u), operand(i, 1u), operand(i, 0u) != 0.0);
    } else {
        // clip: as the CPU's x.max(lo).min(hi)
        let x = operand(i, 0u);
        let lo = operand(i, 1u);
        let hi = operand(i, 2u);
        let y = select(lo, x, x > lo);
        out[i] = select(hi, y, y < hi);
    }
}
"#;

/// The source's numbers, read through operand 0's layout, written at the
/// places operand 1's layout names (`info` as the element-wise kernel's).
const PLACE: &str = r#"
@group(0) @binding(0) var<storage, read> info: array<u32>;
@group(0) @binding(1) var<storage, read> x0: array<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = gid.x + gid.y * groups.x * 256u;
    if i >= info[0] { return; }
    var src_at = info[12];
    var dst_at = info[21];
    var rest = i;
    var d = info[1];
    loop {
        if d == 0u { break; }
        d = d - 1u;
        let size = info[4u + d];
        let at = rest % size;
        src_at = src_at + at * info[13u + d];
        dst_at = dst_at + at * info[22u + d];
        rest = rest / size;
    }
    out[dst_at] = x0[src_at];
}
"#;

/// Each number of the answer the reduction of a run along the axis.
/// `info`: [0] the answer's numbers, [1] its dimensions, [2] the
/// reduction, [3] the axis's length, [4..12] the answer's shape, [12] the
/// source's offset, [13..21] its strides along the answer's dimensions,
/// [21] its stride along the axis.
const REDUCE_AXIS: &str = r#"
@group(0) @binding(0) var<storage, read> info: array<u32>;
@group(0) @binding(1) var<storage, read> x0: array<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<f32>;

// a NaN read from its bits: Metal compiles with fast math, where `v != v`
// folds to false and the select of the larger number becomes a max that
// drops a NaN — the CPU's max and min keep it
fn nan(v: f32) -> bool { return (bitcast<u32>(v) & 0x7fffffffu) > 0x7f800000u; }

fn joined(op: u32, acc: f32, v: f32) -> f32 {
    if op == 0u { return acc + v; }
    if op == 3u { return acc * v; }
    if nan(acc) { return acc; }
    if nan(v) { return v; }
    if op == 1u { return max(acc, v); }
    return min(acc, v);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = gid.x + gid.y * groups.x * 256u;
    if i >= info[0] { return; }
    var at = info[12];
    var rest = i;
    var d = info[1];
    loop {
        if d == 0u { break; }
        d = d - 1u;
        let size = info[4u + d];
        at = at + (rest % size) * info[13u + d];
        rest = rest / size;
    }
    let op = info[2];
    let along = info[3];
    let gap = info[21];
    var acc = 0.0;
    if op == 1u { acc = bitcast<f32>(0xff800000u); }
    if op == 2u { acc = bitcast<f32>(0x7f800000u); }
    if op == 3u { acc = 1.0; }
    for (var j = 0u; j < along; j = j + 1u) {
        acc = joined(op, acc, x0[at + j * gap]);
    }
    out[i] = acc;
}
"#;

/// A reduction along a long axis: sixteen numbers of the answer to a
/// workgroup of 16 × 16 threads, each number's axis shared among sixteen
/// of them (thread `ly` reads the axis's places ly, ly + 16, …) and their
/// parts joined in the workgroup's memory. `info` as `REDUCE_AXIS`'s.
const REDUCE_SPLIT: &str = r#"
@group(0) @binding(0) var<storage, read> info: array<u32>;
@group(0) @binding(1) var<storage, read> x0: array<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<f32>;

// a NaN read from its bits: Metal compiles with fast math, where `v != v`
// folds to false and the select of the larger number becomes a max that
// drops a NaN — the CPU's max and min keep it
fn nan(v: f32) -> bool { return (bitcast<u32>(v) & 0x7fffffffu) > 0x7f800000u; }

fn joined(op: u32, acc: f32, v: f32) -> f32 {
    if op == 0u { return acc + v; }
    if op == 3u { return acc * v; }
    if nan(acc) { return acc; }
    if nan(v) { return v; }
    if op == 1u { return max(acc, v); }
    return min(acc, v);
}

var<workgroup> part: array<f32, 256>;

@compute @workgroup_size(16, 16)
fn main(@builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = (wid.x + wid.y * groups.x) * 16u + lid.x;
    let op = info[2];
    var acc = 0.0;
    if op == 1u { acc = bitcast<f32>(0xff800000u); }
    if op == 2u { acc = bitcast<f32>(0x7f800000u); }
    if op == 3u { acc = 1.0; }
    if i < info[0] {
        var at = info[12];
        var rest = i;
        var d = info[1];
        loop {
            if d == 0u { break; }
            d = d - 1u;
            let size = info[4u + d];
            at = at + (rest % size) * info[13u + d];
            rest = rest / size;
        }
        let along = info[3];
        let gap = info[21];
        for (var j = lid.y; j < along; j = j + 16u) {
            acc = joined(op, acc, x0[at + j * gap]);
        }
    }
    part[lid.y * 16u + lid.x] = acc;
    workgroupBarrier();
    var stride = 8u;
    loop {
        if stride == 0u { break; }
        if lid.y < stride {
            part[lid.y * 16u + lid.x] = joined(op, part[lid.y * 16u + lid.x], part[(lid.y + stride) * 16u + lid.x]);
        }
        workgroupBarrier();
        stride = stride / 2u;
    }
    if lid.y == 0u && i < info[0] {
        out[i] = part[lid.x];
    }
}
"#;

/// A gather: `out[i] = x0[info[1] + at[i]]`, `info[0]` numbers.
const GATHER: &str = r#"
@group(0) @binding(0) var<storage, read> info: array<u32>;
@group(0) @binding(1) var<storage, read> x0: array<f32>;
@group(0) @binding(2) var<storage, read> at: array<u32>;
@group(0) @binding(3) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = gid.x + gid.y * groups.x * 256u;
    if i >= info[0] { return; }
    out[i] = x0[info[1] + at[i]];
}
"#;

/// Where each slice's extreme lies along the axis (`info` as
/// `REDUCE_AXIS`'s, and the tensor's strides in order for the kept axes
/// at 22 and for the axis at 30): the first place holding the slice's
/// largest number (op 1) or smallest (op 2), a NaN first kept as the CPU
/// keeps it, as a whole-number position in the tensor read in order.
const EXTREME_PLACES: &str = r#"
@group(0) @binding(0) var<storage, read> info: array<u32>;
@group(0) @binding(1) var<storage, read> x0: array<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<u32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = gid.x + gid.y * groups.x * 256u;
    if i >= info[0] { return; }
    var at = info[12];
    var flat = 0u;
    var rest = i;
    var d = info[1];
    loop {
        if d == 0u { break; }
        d = d - 1u;
        let size = info[4u + d];
        let idx = rest % size;
        at = at + idx * info[13u + d];
        flat = flat + idx * info[22u + d];
        rest = rest / size;
    }
    let along = info[3];
    let gap = info[21];
    let largest = info[2] == 1u;
    var best = x0[at];
    var place = 0u;
    for (var j = 1u; j < along; j = j + 1u) {
        let v = x0[at + j * gap];
        if (largest && v > best) || (!largest && v < best) {
            best = v;
            place = j;
        }
    }
    out[i] = flat + place * info[30];
}
"#;

/// A gather's gradient, each number read at most once:
/// `out[at[k]] = g[info[1] + k]` for the `info[0]` places of `at`.
const SCATTER_ONCE: &str = r#"
@group(0) @binding(0) var<storage, read> info: array<u32>;
@group(0) @binding(1) var<storage, read> g: array<f32>;
@group(0) @binding(2) var<storage, read> at: array<u32>;
@group(0) @binding(3) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let k = gid.x + gid.y * groups.x * 256u;
    if k >= info[0] { return; }
    out[at[k]] = g[info[1] + k];
}
"#;

/// A gather's gradient: each of the `info[0]` numbers the sum, in order,
/// of `g`'s numbers at the places `reads[starts[j]..starts[j + 1]]`.
const SCATTER: &str = r#"
@group(0) @binding(0) var<storage, read> info: array<u32>;
@group(0) @binding(1) var<storage, read> g: array<f32>;
@group(0) @binding(2) var<storage, read> starts: array<u32>;
@group(0) @binding(3) var<storage, read> reads: array<u32>;
@group(0) @binding(4) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let j = gid.x + gid.y * groups.x * 256u;
    if j >= info[0] { return; }
    var acc = 0.0;
    for (var q = starts[j]; q < starts[j + 1u]; q = q + 1u) {
        acc = acc + g[info[1] + reads[q]];
    }
    out[j] = acc;
}
"#;

/// One step of the whole sum: each workgroup's 256 numbers to one.
/// `info`: [0] the numbers summed.
const SUM: &str = r#"
@group(0) @binding(0) var<storage, read> info: array<u32>;
@group(0) @binding(1) var<storage, read> x0: array<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<f32>;

var<workgroup> part: array<f32, 256>;

@compute @workgroup_size(256)
fn main(@builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let w = wid.x + wid.y * groups.x;
    let i = w * 256u + lid.x;
    var v = 0.0;
    if i < info[0] { v = x0[i]; }
    part[lid.x] = v;
    workgroupBarrier();
    var stride = 128u;
    loop {
        if stride == 0u { break; }
        if lid.x < stride {
            part[lid.x] = part[lid.x] + part[lid.x + stride];
        }
        workgroupBarrier();
        stride = stride / 2u;
    }
    if lid.x == 0u {
        out[w] = part[0];
    }
}
"#;

/// The matrix product, sixty-four by sixty-four numbers of the answer to a
/// workgroup of 256 threads, each thread holding sixteen of them in its
/// registers (rows `ly + 16i`, columns `lx + 16j`): each step of sixteen
/// k is read once into the workgroup's memory, and each number read from
/// there is used four times — the first kernel, one number of the answer
/// to a thread, read two numbers there for each multiply and ran at a
/// tenth of this. Each number of the answer is its k products summed in
/// order. `info`: [0] m, [1] k, [2] n, [4] a's offset, [5] its row
/// stride, [6] its column stride, [7..10] b's.
const MATMUL: &str = r#"
@group(0) @binding(0) var<storage, read> info: array<u32>;
@group(0) @binding(1) var<storage, read> x0: array<f32>;
@group(0) @binding(2) var<storage, read> x1: array<f32>;
@group(0) @binding(3) var<storage, read_write> out: array<f32>;

var<workgroup> ta: array<f32, 1024>;
var<workgroup> tb: array<f32, 1024>;

@compute @workgroup_size(16, 16)
fn main(@builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>) {
    let m = info[0];
    let k = info[1];
    let n = info[2];
    let row0 = wid.y * 64u;
    let col0 = wid.x * 64u;
    let tid = lid.y * 16u + lid.x;
    var acc: array<f32, 16>;
    let steps = (k + 15u) / 16u;
    for (var t = 0u; t < steps; t = t + 1u) {
        for (var q = 0u; q < 4u; q = q + 1u) {
            let e = tid + q * 256u;
            let ar = row0 + e / 16u;
            let ak = t * 16u + e % 16u;
            var va = 0.0;
            if ar < m && ak < k { va = x0[info[4] + ar * info[5] + ak * info[6]]; }
            ta[e] = va;
            let bk = t * 16u + e / 64u;
            let bc = col0 + e % 64u;
            var vb = 0.0;
            if bk < k && bc < n { vb = x1[info[7] + bk * info[8] + bc * info[9]]; }
            tb[e] = vb;
        }
        workgroupBarrier();
        for (var p = 0u; p < 16u; p = p + 1u) {
            let a0 = ta[lid.y * 16u + p];
            let a1 = ta[(lid.y + 16u) * 16u + p];
            let a2 = ta[(lid.y + 32u) * 16u + p];
            let a3 = ta[(lid.y + 48u) * 16u + p];
            let b0 = tb[p * 64u + lid.x];
            let b1 = tb[p * 64u + lid.x + 16u];
            let b2 = tb[p * 64u + lid.x + 32u];
            let b3 = tb[p * 64u + lid.x + 48u];
            acc[0] = acc[0] + a0 * b0;
            acc[1] = acc[1] + a0 * b1;
            acc[2] = acc[2] + a0 * b2;
            acc[3] = acc[3] + a0 * b3;
            acc[4] = acc[4] + a1 * b0;
            acc[5] = acc[5] + a1 * b1;
            acc[6] = acc[6] + a1 * b2;
            acc[7] = acc[7] + a1 * b3;
            acc[8] = acc[8] + a2 * b0;
            acc[9] = acc[9] + a2 * b1;
            acc[10] = acc[10] + a2 * b2;
            acc[11] = acc[11] + a2 * b3;
            acc[12] = acc[12] + a3 * b0;
            acc[13] = acc[13] + a3 * b1;
            acc[14] = acc[14] + a3 * b2;
            acc[15] = acc[15] + a3 * b3;
        }
        workgroupBarrier();
    }
    for (var i = 0u; i < 4u; i = i + 1u) {
        let r = row0 + lid.y + 16u * i;
        for (var j = 0u; j < 4u; j = j + 1u) {
            let c = col0 + lid.x + 16u * j;
            if r < m && c < n {
                out[r * n + c] = acc[i * 4u + j];
            }
        }
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tensor::{Compare, CrowTensor};

    fn skip_if_no_gpu() -> Option<&'static GpuContext> {
        GpuContext::get()
    }

    /// Two runs of numbers agree to f32's digits.
    fn near(got: &[f64], want: &[f64], tol: f64) {
        assert_eq!(got.len(), want.len(), "{:?} against {:?}", got, want);
        for (g, w) in got.iter().zip(want) {
            let scale = w.abs().max(1.0);
            assert!((g - w).abs() <= tol * scale || (g.is_nan() && w.is_nan()), "{:?} against {:?}", got, want);
        }
    }

    /// An operation with no shader reads a GPU tensor's data through a
    /// download: indexing, a broadcast and `diag` of a GPU tensor panicked
    /// ("tensor is on GPU").
    #[test]
    fn a_gpu_tensor_is_read_by_operations_without_a_shader() {
        let Some(_ctx) = skip_if_no_gpu() else { return };
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]).to_gpu();
        assert!(t.is_gpu());
        assert_eq!(t.get_flat(1), 2.0);
        let m = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]).to_gpu();
        let row = CrowTensor::from_vec(vec![10.0, 20.0], vec![2]);
        assert_eq!(m.zip_with(&row, |a, b| a + b).to_vec(), vec![11.0, 22.0, 13.0, 24.0]);
        assert_eq!(CrowTensor::diag(&m).map(|d| d.to_vec()), Ok(vec![1.0, 4.0]));
    }

    /// A view of a tensor on the GPU — a transpose, a slice — is its own
    /// numbers, read and computed with: `transpose[m]` had been read,
    /// multiplied and taken `exp` of as `m`, and `slice[v, 1, 3]` read from
    /// the start (crow check --self's gpu family).
    #[test]
    fn a_view_of_a_gpu_tensor_is_its_own_numbers() {
        let Some(_ctx) = skip_if_no_gpu() else { return };
        let m = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]).to_gpu();
        let t = m.transpose();
        assert_eq!(t.to_vec(), vec![1.0, 3.0, 2.0, 4.0]);
        let exps = t.exp().to_vec();
        for (got, x) in exps.iter().zip([1.0f64, 3.0, 2.0, 4.0]) {
            assert!((got - x.exp()).abs() < 1e-5 * x.exp(), "{:?}", exps);
        }
        assert_eq!(t.matmul(&m).to_vec(), vec![10.0, 14.0, 14.0, 20.0]);
        let v = CrowTensor::from_vec(vec![3.0, 1.0, 2.0], vec![3]).to_gpu();
        let tail = crate::core::slice_tensor(&v, -2, 3);
        assert_eq!(tail.to_vec(), vec![1.0, 2.0]);
        assert_eq!(tail.sum(), 3.0);
        assert_eq!(tail.add_scalar(1.0).to_vec(), vec![2.0, 3.0]);
    }

    /// An empty tensor stays on the CPU, and an empty buffer reads back as
    /// no numbers: one put on the GPU had been read back through an empty
    /// slice, which wgpu refuses ("buffer slices can not be empty"), and
    /// every model of pie's under `gpu[]` stopped at its first empty tensor
    /// (parity 520).
    #[test]
    fn an_empty_tensor_stays_on_the_cpu() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let e = CrowTensor::from_vec(vec![], vec![0]).to_gpu();
        assert!(!e.is_gpu());
        assert!(e.to_vec().is_empty());
        assert!(GpuBuffer::from_f64(ctx, &[]).to_f64(ctx).is_empty());
        let joined = crate::core::concat_tensors(&e, &CrowTensor::from_vec(vec![1.0, 2.0], vec![2]).to_gpu());
        assert_eq!(joined.map(|t| t.to_vec()), Ok(vec![1.0, 2.0]));
    }

    /// A number beside a tensor reaches the kernel as the f32 it is, as
    /// data: written into the shader's text to ten places, `t * 1e-12` had
    /// multiplied by 0 and `t + 1e-12` added nothing.
    #[test]
    fn a_tiny_scalar_reaches_the_kernel() {
        let Some(_ctx) = skip_if_no_gpu() else { return };
        let a = CrowTensor::from_vec(vec![1.0, 2.0], vec![2]).to_gpu();
        let c = a.mul_scalar(1e-12).to_vec();
        assert!((c[0] - 1e-12).abs() < 1e-18 && (c[1] - 2e-12).abs() < 1e-18, "{:?}", c);
        let tiny = CrowTensor::from_vec(vec![1e-12, 2e-12], vec![2]).to_gpu();
        let d = tiny.add_scalar(1e-12).to_vec();
        assert!((d[0] - 2e-12).abs() < 1e-18 && (d[1] - 3e-12).abs() < 1e-18, "{:?}", d);
    }

    /// A tensor less a number, and a number over a tensor, on the GPU answer
    /// what the CPU answers to f32's digits — each by one function both
    /// modes call (the interpreter had added the negative on the GPU and
    /// divided there, the compiled program subtracted and divided on the
    /// CPU, and the two showed different digits).
    #[test]
    fn a_scalar_beside_a_gpu_tensor_is_one_operation() {
        let Some(_ctx) = skip_if_no_gpu() else { return };
        let cpu = CrowTensor::from_vec(vec![3.0, 1.0, 2.0], vec![3]);
        let gpu = cpu.to_gpu();
        assert!(gpu.sub_scalar(0.5).is_gpu());
        assert_eq!(gpu.sub_scalar(0.5).to_vec(), vec![2.5, 0.5, 1.5]);
        let q = gpu.scalar_div(2.5);
        assert!(q.is_gpu());
        near(&q.to_vec(), &cpu.scalar_div(2.5).to_vec(), 1e-6);
        let r = gpu.div_scalar(2.5);
        assert!(r.is_gpu());
        near(&r.to_vec(), &cpu.div_scalar(2.5).to_vec(), 1e-6);
    }

    /// Every operation of one operand answers what the CPU's answers, to
    /// f32's digits, on numbers where each is defined.
    #[test]
    fn each_operation_of_one_operand_is_the_cpus() {
        let Some(_ctx) = skip_if_no_gpu() else { return };
        let xs = vec![-2.5, -1.0, -0.4, -0.05, 0.0, 0.03, 0.3, 0.5, 0.75, 1.5, 3.0, 12.0];
        let unit = vec![-0.9, -0.5, -0.1, 0.0, 0.2, 0.6, 0.95];
        let pos = vec![0.01, 0.07, 0.5, 1.0, 2.0, 10.0, 1000.0];
        let t = |v: &Vec<f64>| CrowTensor::from_vec(v.clone(), vec![v.len()]);
        let both = |name: &str, v: &Vec<f64>, f: &dyn Fn(&CrowTensor) -> CrowTensor| {
            let cpu = f(&t(v));
            let gpu = f(&t(v).to_gpu());
            assert!(gpu.is_gpu(), "{} left the GPU", name);
            for (g, w) in gpu.to_vec().iter().zip(cpu.to_vec()) {
                assert!((g - w).abs() <= 2e-6 * w.abs().max(1.0), "{}: {} on the GPU, {} on the CPU", name, g, w);
            }
        };
        both("neg", &xs, &|x| x.neg());
        both("abs", &xs, &|x| x.abs());
        both("exp", &xs, &|x| x.exp());
        both("log", &pos, &|x| x.log());
        both("sqrt", &pos, &|x| x.sqrt());
        both("sin", &xs, &|x| x.sin());
        both("cos", &xs, &|x| x.cos());
        both("tan", &unit, &|x| x.tan());
        both("tanh", &xs, &|x| x.tanh());
        both("sinh", &unit, &|x| x.sinh());
        both("cosh", &unit, &|x| x.cosh());
        both("asin", &unit, &|x| x.asin());
        both("acos", &unit, &|x| x.acos());
        both("atan", &xs, &|x| x.atan());
        both("log2", &pos, &|x| x.log2());
        both("log10", &pos, &|x| x.log10());
        both("log1p", &unit, &|x| x.log1p());
        both("expm1", &xs, &|x| x.expm1());
        both("floor", &xs, &|x| x.floor());
        both("ceil", &xs, &|x| x.ceil());
        both("round", &vec![-2.5, -1.5, -0.5, -0.49999997, 0.49999997, 0.5, 1.5, 2.5, 8388609.0], &|x| x.round());
        both("sign", &xs, &|x| x.sign());
        both("erf", &xs, &|x| x.erf());
        both("clip", &xs, &|x| x.clip(-0.5, 1.0));
        // (relu, a bound an f32 holds no number past)
        both("relu", &xs, &|x| x.clip(0.0, f64::INFINITY));
        both("clip below", &xs, &|x| x.clip(f64::NEG_INFINITY, 0.25));
        // (tanh of a large number is ±1, not infinity over infinity)
        assert_eq!(t(&vec![-100.0, 100.0]).to_gpu().tanh().to_vec(), vec![-1.0, 1.0]);
    }

    /// Every operation of two operands answers the CPU's, broadcast — a
    /// row beside a matrix, a column beside it, a number on either side.
    #[test]
    fn each_operation_of_two_operands_is_the_cpus_broadcast() {
        let Some(_ctx) = skip_if_no_gpu() else { return };
        let m = CrowTensor::from_vec(vec![1.0, -2.0, 3.0, 0.5, 4.0, -1.5], vec![2, 3]);
        let row = CrowTensor::from_vec(vec![2.0, 0.5, -3.0], vec![3]);
        let col = CrowTensor::from_vec(vec![10.0, -1.0], vec![2, 1]);
        type Two = fn(&CrowTensor, &CrowTensor) -> CrowTensor;
        let ops: [(&str, Two); 6] = [
            ("add", |a, b| a.add(b)),
            ("sub", |a, b| a.sub(b)),
            ("mul", |a, b| a.mul(b)),
            ("div", |a, b| a.div(b)),
            ("pow", |a, b| a.abs().pow(b)),
            ("atan2", |a, b| a.atan2(b)),
        ];
        for (name, f) in ops {
            for other in [&row, &col, &m] {
                let cpu = f(&m, other);
                let on = f(&m.to_gpu(), &other.to_gpu());
                assert!(on.is_gpu(), "{} left the GPU", name);
                assert_eq!(on.shape(), cpu.shape(), "{}", name);
                near(&on.to_vec(), &cpu.to_vec(), 2e-6);
                // a CPU operand beside a GPU one is brought to it
                let mixed = f(&m.to_gpu(), other);
                assert!(mixed.is_gpu(), "{} beside a CPU operand left the GPU", name);
                near(&mixed.to_vec(), &cpu.to_vec(), 2e-6);
            }
        }
        // a negative number to a whole power is signed by its parity
        let neg = CrowTensor::from_vec(vec![-2.0, -2.0, -2.0, -0.5], vec![4]);
        let pw = CrowTensor::from_vec(vec![2.0, 3.0, 0.5, -1.0], vec![4]);
        let got = neg.to_gpu().pow(&pw.to_gpu()).to_vec();
        assert_eq!(got[0], 4.0);
        assert_eq!(got[1], -8.0);
        assert!(got[2].is_nan());
        assert_eq!(got[3], -2.0);
        // a number on either side
        near(&m.to_gpu().pow_scalar(2.0).to_vec(), &m.pow_scalar(2.0).to_vec(), 2e-6);
        near(&m.to_gpu().scalar_div(1.0).to_vec(), &m.scalar_div(1.0).to_vec(), 2e-6);
    }

    /// The inverse functions answer f32's digits across their domains, at
    /// numbers an f32 holds exactly, so what is compared is the function
    /// and not the number's rounding (a negative zero and an infinity are
    /// fast math's on Metal, and left out): the drivers' own asin and
    /// atan2 had been off by thousands of ulps on lavapipe, CI's Vulkan
    /// device, where Metal's were close (`asin: -1.120123267173767 on the
    /// GPU, -1.1197695149986342 on the CPU`).
    #[test]
    fn the_inverse_functions_hold_f32s_digits_across_their_domains() {
        let Some(_ctx) = skip_if_no_gpu() else { return };
        let t = |v: &[f64]| CrowTensor::from_vec(v.to_vec(), vec![v.len()]);
        let check = |name: &str, at: &[f64], got: Vec<f64>, want: Vec<f64>| {
            assert_eq!(got.len(), at.len(), "{}", name);
            for ((g, w), x) in got.iter().zip(&want).zip(at) {
                assert!((g - w).abs() <= 1e-6 * w.abs().max(1.0) || (g.is_nan() && w.is_nan()), "{} at {}: {} on the GPU, {} on the CPU", name, x, g, w);
            }
        };
        // every 1/1024 of [-1, 1], and past each end
        let unit: Vec<f64> = (-1024..=1024).map(|k| k as f64 / 1024.0).chain([1.0 + 1.0 / 1024.0, -2.0]).collect();
        check("asin", &unit, t(&unit).to_gpu().asin().to_vec(), t(&unit).asin().to_vec());
        check("acos", &unit, t(&unit).to_gpu().acos().to_vec(), t(&unit).acos().to_vec());
        // tangents from 2^-20 to 2^20 and each side of the two reductions'
        // bounds (tan π/8, tan 3π/8), both signs
        let mut wide: Vec<f64> = (-80..=80).map(|k| 2f64.powf(k as f64 / 4.0)).collect();
        wide.extend([0.0, 0.41421354, 0.41421357, 2.4142134, 2.4142137]);
        let wide: Vec<f64> = wide.iter().flat_map(|&x| [x, -x]).map(|x| x as f32 as f64).collect();
        check("atan", &wide, t(&wide).to_gpu().atan().to_vec(), t(&wide).atan().to_vec());
        // every quadrant and both axes
        let sides = [0.0, 0.5, -0.5, 1.0, -1.0, 3.0, -3.0, 1000.0, -1000.0, 1e-6, -1e-6];
        let (mut ys, mut xs) = (Vec::new(), Vec::new());
        for &y in &sides {
            for &x in &sides {
                ys.push(y as f32 as f64);
                xs.push(x as f32 as f64);
            }
        }
        let at: Vec<f64> = (0..ys.len()).map(|i| ys[i] * 1e4 + xs[i]).collect();
        check("atan2", &at, t(&ys).to_gpu().atan2(&t(&xs).to_gpu()).to_vec(), t(&ys).atan2(&t(&xs)).to_vec());
    }

    /// `where` and the comparisons the gradients read run where the
    /// numbers lie.
    #[test]
    fn where_and_the_comparisons_run_on_the_gpu() {
        let Some(_ctx) = skip_if_no_gpu() else { return };
        let c = CrowTensor::from_vec(vec![1.0, 0.0, 1.0, 0.0], vec![4]);
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![4]);
        let b = CrowTensor::from_vec(vec![-1.0, -2.0, -3.0, -4.0], vec![4]);
        let w = c.to_gpu().where_cond(&a.to_gpu(), &b);
        assert!(w.is_gpu());
        assert_eq!(w.to_vec(), vec![1.0, -2.0, 3.0, -4.0]);
        let x = CrowTensor::from_vec(vec![-1.0, 0.0, 0.5, 2.0], vec![4]).to_gpu();
        assert_eq!(x.compared(Compare::Gt, 0.0).to_vec(), vec![0.0, 0.0, 1.0, 1.0]);
        assert_eq!(x.compared(Compare::Lt, 0.5).to_vec(), vec![1.0, 1.0, 0.0, 0.0]);
    }

    /// A reduction along each axis of a view answers the CPU's.
    #[test]
    fn a_reduction_along_an_axis_is_the_cpus() {
        let Some(_ctx) = skip_if_no_gpu() else { return };
        let data: Vec<f64> = (0..24).map(|i| (i as f64 * 0.37).sin()).collect();
        let t = CrowTensor::from_vec(data, vec![2, 3, 4]);
        let views = [(t.clone(), t.to_gpu()), (t.transpose(), t.to_gpu().transpose())];
        for (view, on) in views {
            for axis in 0..3 {
                for (name, cpu, gpu) in [
                    ("sum", view.sum_axis(axis), on.sum_axis(axis)),
                    ("max", view.max_axis(axis), on.max_axis(axis)),
                    ("min", view.min_axis(axis), on.min_axis(axis)),
                    ("mean", view.mean_axis(axis), on.mean_axis(axis)),
                ] {
                    assert!(gpu.is_gpu(), "{} along {} left the GPU", name, axis);
                    assert_eq!(gpu.shape(), cpu.shape(), "{} along {}", name, axis);
                    near(&gpu.to_vec(), &cpu.to_vec(), 2e-6);
                }
            }
        }
        // a long axis, shared among a workgroup's threads: a batch's rows
        // summed for each column (a bias's gradient), read in order and
        // through a transpose, with a NaN that every reduction but the
        // sum's of another column keeps
        let mut long: Vec<f64> = (0..40 * 37).map(|i| ((i as f64) * 0.13).cos() * 0.9 + 1.05).collect();
        long[5 * 37 + 3] = f64::NAN;
        let t = CrowTensor::from_vec(long, vec![40, 37]);
        let views = [(t.clone(), t.to_gpu()), (t.transpose(), t.to_gpu().transpose())];
        for (view, on) in views {
            for axis in 0..2 {
                for (name, cpu, gpu) in [
                    ("sum", view.sum_axis(axis), on.sum_axis(axis)),
                    ("max", view.max_axis(axis), on.max_axis(axis)),
                    ("min", view.min_axis(axis), on.min_axis(axis)),
                    ("prod", view.prod_axis(axis), on.prod_axis(axis)),
                ] {
                    assert!(gpu.is_gpu(), "{} along {} left the GPU", name, axis);
                    assert_eq!(gpu.shape(), cpu.shape(), "{} along {}", name, axis);
                    near(&gpu.to_vec(), &cpu.to_vec(), 1e-5);
                }
            }
        }
        // the whole sum, past the CPU's few, through the workgroups
        let big: Vec<f64> = (0..100_000).map(|i| (i % 7) as f64 * 0.25).collect();
        let s = CrowTensor::from_vec(big.clone(), vec![100_000]).to_gpu().sum();
        let want: f64 = big.iter().sum();
        assert!((s - want).abs() < 1e-5 * want, "{} against {}", s, want);
    }

    /// `t[positions]` reads the numbers where they lie on the GPU, of a
    /// view and of a source on the CPU beside positions on the GPU, and a
    /// position read twice or counted from the end; its gradient sums back
    /// what read each number, in the order read, as the CPU's does.
    #[test]
    fn a_gather_and_its_gradient_stay_on_the_gpu() {
        let Some(_ctx) = skip_if_no_gpu() else { return };
        let data: Vec<f64> = (0..60).map(|i| (i as f64 * 0.7).sin()).collect();
        let m = CrowTensor::from_vec(data, vec![6, 10]);
        let positions = CrowTensor::from_vec(vec![3.0, 59.0, -1.0, 3.0, 0.0, 17.0], vec![2, 3]);
        for (source, on) in [(m.clone(), m.to_gpu()), (m.transpose(), m.to_gpu().transpose())] {
            let want = source.try_gather(&positions);
            let got = on.try_gather(&positions);
            assert!(got.as_ref().is_ok_and(|t| t.is_gpu() && t.shape() == [2, 3]), "{:?}", got.as_ref().map(|t| t.shape().to_vec()));
            near(&got.map(|t| t.to_vec()).unwrap_or_default(), &want.map(|t| t.to_vec()).unwrap_or_default(), 1e-6);
        }
        let mixed = m.try_gather(&positions.to_gpu());
        assert!(mixed.as_ref().is_ok_and(|t| t.is_gpu()), "a CPU source beside GPU positions is gathered on the GPU");
        // a position past the end is refused before anything runs
        let far = m.to_gpu().try_gather(&CrowTensor::from_vec(vec![60.0], vec![1]));
        assert_eq!(far.map(|_| ()), m.try_gather(&CrowTensor::from_vec(vec![60.0], vec![1])).map(|_| ()));
        // the gradient: each number read once, and read several times
        let g: Vec<f64> = (0..8).map(|i| 0.5 + f64::from(i) * 0.25).collect();
        for at in [vec![7usize, 2, 9, 0, 4, 5, 1, 3], vec![3usize, 3, 0, 9, 3, 0, 1, 1]] {
            let mut want = vec![0.0; 10];
            for (k, &p) in at.iter().enumerate() {
                want[p] += g[k];
            }
            let on = CrowTensor::from_vec(g.clone(), vec![8]).to_gpu();
            let got = CrowTensor::gpu_scattered(&on, &at, &[2, 5]);
            assert!(got.as_ref().is_some_and(|t| t.is_gpu() && t.shape() == [2, 5]));
            near(&got.map(|t| t.to_vec()).unwrap_or_default(), &want, 1e-6);
        }
    }

    /// Gradients that read places — an extreme's along an axis (ties and a
    /// NaN first among them), a gather's with a place read twice, a
    /// number's beside a tensor — are found and laid on the GPU, and are
    /// the CPU's.
    #[test]
    fn gradients_that_read_places_are_the_cpus_on_the_gpu() {
        use crate::autograd::{self, Traced};
        let Some(_ctx) = skip_if_no_gpu() else { return };
        let mut data: Vec<f64> = (0..35).map(|i| f64::from((i * 7) % 11) * 0.5).collect();
        data[7] = f64::NAN;
        let t = CrowTensor::from_vec(data, vec![5, 7]);
        let p = CrowTensor::from_vec(vec![3.0, 0.0, 3.0, 34.0, 12.0, 12.0], vec![6]);
        let run = |x: &CrowTensor, transposed: bool| {
            autograd::gradient(std::slice::from_ref(x), |v| {
                let x = if transposed { autograd::transpose(&v[0]) } else { v[0].clone() };
                let mut total = Traced::constant(CrowTensor::scalar(0.0));
                for axis in 0..2 {
                    total = autograd::add(&total, &autograd::sum(&autograd::max_axis(&x, axis)?))?;
                    total = autograd::add(&total, &autograd::sum(&autograd::min_axis(&x, axis)?))?;
                }
                let read = autograd::gather(&x, &p)?;
                let scale = Traced::constant(CrowTensor::scalar(0.5));
                total = autograd::add(&total, &autograd::sum(&autograd::mul(&read, &scale)?))?;
                Ok(total)
            })
        };
        for transposed in [false, true] {
            let want = run(&t, transposed).map(|g| g[0].to_vec());
            let got = run(&t.to_gpu(), transposed);
            assert!(got.as_ref().is_ok_and(|g| g[0].is_gpu()), "the gradient left the GPU");
            near(&got.map(|g| g[0].to_vec()).unwrap_or_default(), &want.unwrap_or_default(), 1e-6);
        }
    }

    /// A buffer let go is kept for the next of its size: one asked for as
    /// zeros is cleared, one a kernel writes is written whole, and what a
    /// pending operation still reads of it is read before it is written.
    #[test]
    fn a_buffer_let_go_is_used_again_as_new() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let ones = CrowTensor::from_vec(vec![1.0; 777], vec![777]).to_gpu();
        let twice = ones.add(&ones);
        drop(ones);
        let zeros = GpuBuffer::zeros(ctx, 777);
        assert_eq!(zeros.to_f64(ctx), vec![0.0; 777]);
        drop(zeros);
        // a sum still pending reads `a` while `a`'s buffer is let go and
        // taken by the next operation's answer
        let a = CrowTensor::from_vec((0..777).map(f64::from).collect(), vec![777]).to_gpu();
        let read_later = a.mul_scalar(2.0);
        drop(a);
        let other = twice.add_scalar(3.0);
        let want: Vec<f64> = (0..777).map(|i| f64::from(i) * 2.0).collect();
        near(&read_later.to_vec(), &want, 1e-6);
        assert_eq!(other.to_vec(), vec![5.0; 777]);
    }

    /// The matrix product reads a transposed operand through its strides
    /// and answers the CPU's, at sizes no tile divides.
    #[test]
    fn the_matrix_product_reads_its_operands_where_they_lie() {
        let Some(_ctx) = skip_if_no_gpu() else { return };
        // (within one tile of the answer, and across several with an edge
        // of each kind: rows, columns and a last step of k partly filled)
        for (m, k, n) in [(37, 23, 19), (131, 70, 97), (1, 300, 1)] {
            let a = CrowTensor::from_vec((0..m * k).map(|i| ((i * 7 % 11) as f64 - 5.0) * 0.1).collect(), vec![m, k]);
            let b = CrowTensor::from_vec((0..k * n).map(|i| ((i * 5 % 13) as f64 - 6.0) * 0.1).collect(), vec![k, n]);
            let cpu = a.matmul(&b);
            let gpu = a.to_gpu().matmul(&b.to_gpu());
            assert!(gpu.is_gpu());
            assert_eq!(gpu.shape(), &[m, n]);
            near(&gpu.to_vec(), &cpu.to_vec(), 1e-4);
            // aᵀ read through its strides: (aᵀ)ᵀ b
            let at = CrowTensor::from_vec(a.transpose().to_vec(), vec![k, m]).to_gpu().transpose();
            near(&at.matmul(&b.to_gpu()).to_vec(), &cpu.to_vec(), 1e-4);
            // a CPU operand beside a GPU one is brought to it
            let mixed = a.to_gpu().matmul(&b);
            assert!(mixed.is_gpu());
            near(&mixed.to_vec(), &cpu.to_vec(), 1e-4);
            // a weight read out of a vector of parameters, past its start
            let theta = CrowTensor::from_vec(vec![9.0; 5], vec![5]).concat(&b.flatten(), 0).to_gpu();
            let w = theta.slice(0, 5, 5 + k * n).reshape(vec![k, n]);
            near(&a.to_gpu().matmul(&w).to_vec(), &cpu.to_vec(), 1e-4);
        }
    }

    /// A view laid out in order, two tensors joined along an axis, and a
    /// tensor of more numbers than one dimension of the grid holds, on the
    /// device.
    #[test]
    fn views_joins_and_large_tensors_stay_on_the_gpu() {
        let Some(_ctx) = skip_if_no_gpu() else { return };
        let m = CrowTensor::from_vec((0..12).map(f64::from).collect(), vec![3, 4]);
        let flat = m.to_gpu().transpose().reshape(vec![12]);
        assert!(flat.is_gpu());
        assert_eq!(flat.to_vec(), m.transpose().reshape(vec![12]).to_vec());
        for axis in 0..2 {
            let joined = m.to_gpu().concat(&m.to_gpu().mul_scalar(2.0), axis);
            assert!(joined.is_gpu());
            assert_eq!(joined.to_vec(), m.concat(&m.mul_scalar(2.0), axis).to_vec());
        }
        let n = 65_535 * 256 + 1000;
        let big = CrowTensor::full(vec![n], 1.5).to_gpu().add_scalar(1.0);
        assert!(big.is_gpu());
        assert_eq!(big.get_flat(n - 1), 2.5);
        assert_eq!(big.get_flat(0), 2.5);
    }

    #[test]
    fn test_gpu_available() {
        let available = gpu_available();
        println!("GPU available: {}", available);
    }

    #[test]
    fn test_upload_download() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let buf = GpuBuffer::from_f64(ctx, &data);
        assert_eq!(buf.to_f64(ctx), data);
        assert_eq!(buf.read(ctx, 1, 3), vec![2.0, 3.0, 4.0]);
    }

    // These tests measure GPU vs CPU performance and print results.
    // Run with: cargo test -p crow_runtime --features gpu bench_gpu -- --nocapture --ignored

    /// What one operation costs: asked for in a run of 2000, a number read
    /// back, an operation over a [512, 1024] tensor of each kind, and the
    /// pieces of one run (`--release -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn bench_operation_overhead() {
        use std::time::Instant;
        let Some(ctx) = skip_if_no_gpu() else { return };
        let mut x = CrowTensor::from_vec(vec![1.0; 64], vec![64]).to_gpu();
        let _ = x.add_scalar(1.0).get_flat(0);
        let t0 = Instant::now();
        for _ in 0..2000 {
            x = x.add_scalar(1.0);
        }
        let queued = t0.elapsed();
        let last = x.get_flat(0);
        let all = t0.elapsed();
        println!("2000 operations: {queued:.2?} to ask, {all:.2?} to the answer ({last}): {:.1} µs each", all.as_secs_f64() * 1e6 / 2000.0);
        let t0 = Instant::now();
        for _ in 0..200 {
            let _ = x.get_flat(0);
        }
        println!("a number read back: {:.1} µs", t0.elapsed().as_secs_f64() * 1e6 / 200.0);
        let big = CrowTensor::from_vec(vec![0.5; 512 * 1024], vec![512, 1024]).to_gpu();
        let row = CrowTensor::from_vec(vec![0.25; 1024], vec![1024]).to_gpu();
        let _ = big.add(&big).get_flat(0);
        for (name, f) in [
            ("add of [512, 1024]", &(|| big.add(&big)) as &dyn Fn() -> CrowTensor),
            ("add of a row to it", &|| big.add(&row)),
            ("a number times it", &|| big.mul_scalar(1.5)),
            ("tanh of it", &|| big.tanh()),
            ("its sum along rows", &|| big.sum_axis(0)),
            ("its sum along columns", &|| big.sum_axis(1)),
            ("its transpose laid in order", &|| big.transpose().reshape(vec![512 * 1024])),
            ("laid into zeros of 1.3M", &|| CrowTensor::laid_into(&[1_300_000], &[1000], &big.reshape(vec![512 * 1024]))),
        ] {
            // (each kernel compiled before it is timed)
            let _ = f().get_flat(0);
            let t0 = Instant::now();
            let mut last = f();
            for _ in 1..100 {
                last = f();
            }
            let _ = last.get_flat(0);
            println!("{name}: {:.1} µs each", t0.elapsed().as_secs_f64() * 1e6 / 100.0);
        }
        let t0 = Instant::now();
        for _ in 0..2000 {
            let _b = GpuBuffer::zeros(ctx, 64);
        }
        println!("zeros of 64 from the pool: {:.1} µs", t0.elapsed().as_secs_f64() * 1e6 / 2000.0);
        let t0 = Instant::now();
        for _ in 0..2000 {
            use wgpu::util::DeviceExt;
            let _b = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&[0u32; 42]), usage: wgpu::BufferUsages::STORAGE });
        }
        println!("an info buffer: {:.1} µs", t0.elapsed().as_secs_f64() * 1e6 / 2000.0);
        let kernel = ctx.kernel_of("elementwise", ELEMENTWISE, 3, Some(Op::Add.code()));
        let a = GpuBuffer::zeros(ctx, 64);
        let o = GpuBuffer::zeros(ctx, 64);
        let t0 = Instant::now();
        for _ in 0..2000 {
            let entries = [
                wgpu::BindGroupEntry { binding: 0, resource: a.buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: a.buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: a.buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: a.buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: o.buffer.as_entire_binding() },
            ];
            let _g = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &kernel.layout, entries: &entries });
        }
        println!("a bind group: {:.1} µs", t0.elapsed().as_secs_f64() * 1e6 / 2000.0);
        let t0 = Instant::now();
        for _ in 0..2000 {
            ctx.run(&kernel, &[0u32; 42], &[&a.buffer], &o.buffer, [1, 1, 1]);
        }
        println!("a run recorded: {:.1} µs", t0.elapsed().as_secs_f64() * 1e6 / 2000.0);
    }

    #[test]
    #[ignore]
    fn bench_gpu_matmul_vs_cpu() {
        use std::time::Instant;
        let Some(_ctx) = skip_if_no_gpu() else {
            println!("No GPU available, skipping benchmark");
            return;
        };
        for &n in &[256, 512, 1024, 2048] {
            let a = CrowTensor::from_vec((0..n * n).map(|i| (i % 17) as f64 * 0.01).collect(), vec![n, n]);
            let b = CrowTensor::from_vec((0..n * n).map(|i| (i % 13) as f64 * 0.01).collect(), vec![n, n]);
            let (ga, gb) = (a.to_gpu(), b.to_gpu());
            let _ = ga.matmul(&gb).get_flat(0);
            let t0 = Instant::now();
            let _ = ga.matmul(&gb).get_flat(0);
            let gpu_time = t0.elapsed();
            let t0 = Instant::now();
            let _ = a.matmul(&b);
            let cpu_time = t0.elapsed();
            let flops = 2.0 * (n as f64).powi(3);
            println!("matmul {n}: GPU {gpu_time:.2?} ({:.0} GFLOP/s), CPU {cpu_time:.2?} ({:.0} GFLOP/s)", flops / gpu_time.as_secs_f64() / 1e9, flops / cpu_time.as_secs_f64() / 1e9);
            let _ = ga.transpose().matmul(&gb).get_flat(0);
            let t0 = Instant::now();
            let _ = ga.transpose().matmul(&gb).get_flat(0);
            let ta = t0.elapsed();
            let _ = ga.matmul(&gb.transpose()).get_flat(0);
            let t0 = Instant::now();
            let _ = ga.matmul(&gb.transpose()).get_flat(0);
            let tb = t0.elapsed();
            println!("    aᵀb {ta:.2?} ({:.0} GFLOP/s), abᵀ {tb:.2?} ({:.0} GFLOP/s)", flops / ta.as_secs_f64() / 1e9, flops / tb.as_secs_f64() / 1e9);
        }
    }
}
