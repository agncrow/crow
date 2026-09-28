use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use wgpu;

/// Global GPU context — initialized lazily on first use.
static GPU_CONTEXT: OnceLock<Option<GpuContext>> = OnceLock::new();

/// Global flag: when true, tensor creation functions place data on GPU.
static GPU_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enable GPU mode — subsequent tensor creation will place data on GPU.
pub fn set_gpu_enabled(enabled: bool) {
    GPU_ENABLED.store(enabled, Ordering::SeqCst);
}

/// Check if GPU mode is enabled.
pub fn is_gpu_enabled() -> bool {
    GPU_ENABLED.load(Ordering::SeqCst)
}

/// Holds the wgpu device and queue for GPU compute.
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl GpuContext {
    /// Get or initialize the global GPU context.
    /// Returns None if no GPU is available.
    pub fn get() -> Option<&'static GpuContext> {
        GPU_CONTEXT
            .get_or_init(|| {
                pollster::block_on(Self::init())
            })
            .as_ref()
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

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("crow_gpu"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            })
            .await
            .ok()?;

        Some(GpuContext { device, queue })
    }
}

/// A GPU buffer holding f32 data.
#[derive(Debug)]
pub struct GpuBuffer {
    pub buffer: wgpu::Buffer,
    pub len: usize, // number of f32 elements
}

impl GpuBuffer {
    /// Upload f32 data to GPU.
    pub fn from_f32(ctx: &GpuContext, data: &[f32]) -> Self {
        use wgpu::util::DeviceExt;
        let buffer = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("crow_tensor"),
            contents: bytemuck::cast_slice(data),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        });
        GpuBuffer {
            buffer,
            len: data.len(),
        }
    }

    /// Upload f64 CPU data to GPU as f32.
    pub fn from_f64(ctx: &GpuContext, data: &[f64]) -> Self {
        let f32_data: Vec<f32> = data.iter().map(|&x| x as f32).collect();
        Self::from_f32(ctx, &f32_data)
    }

    /// Download GPU data back to CPU as f32.
    pub fn to_f32(&self, ctx: &GpuContext) -> Vec<f32> {
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging"),
            size: (self.len * std::mem::size_of::<f32>()) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_buffer_to_buffer(
            &self.buffer,
            0,
            &staging,
            0,
            (self.len * std::mem::size_of::<f32>()) as u64,
        );
        ctx.queue.submit(Some(encoder.finish()));

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

    /// Download GPU data back to CPU as f64.
    pub fn to_f64(&self, ctx: &GpuContext) -> Vec<f64> {
        self.to_f32(ctx).iter().map(|&x| x as f64).collect()
    }

    /// Create an empty GPU buffer of given size.
    pub fn zeros(ctx: &GpuContext, len: usize) -> Self {
        let data = vec![0.0f32; len];
        Self::from_f32(ctx, &data)
    }
}

// ── WGSL Compute Shaders ──────────────────────────────────────────────

/// An element-wise operation between two buffers: a closed set, so no op
/// the shaders lack can be asked for.
#[derive(Clone, Copy, Debug)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// An element-wise operation on one buffer.
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

/// An operation between a buffer and a scalar.
#[derive(Clone, Copy, Debug)]
pub enum ScalarOp {
    Add,
    Mul,
}

/// Element-wise binary operation on GPU.
pub fn gpu_elementwise_binary(
    ctx: &GpuContext,
    a: &GpuBuffer,
    b: &GpuBuffer,
    op: BinaryOp,
) -> GpuBuffer {
    assert_eq!(a.len, b.len, "gpu binary op: mismatched lengths");

    let op_expr = match op {
        BinaryOp::Add => "a[idx] + b[idx]",
        BinaryOp::Sub => "a[idx] - b[idx]",
        BinaryOp::Mul => "a[idx] * b[idx]",
        BinaryOp::Div => "a[idx] / b[idx]",
    };

    let shader_src = format!(
        r#"
@group(0) @binding(0) var<storage, read> a: array<f32>;
@group(0) @binding(1) var<storage, read> b: array<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {{
    let idx = gid.x;
    if idx < arrayLength(&out) {{
        out[idx] = {op_expr};
    }}
}}
"#
    );

    run_binary_shader(ctx, a, b, &shader_src)
}

/// Element-wise unary operation on GPU.
pub fn gpu_elementwise_unary(
    ctx: &GpuContext,
    input: &GpuBuffer,
    op: UnaryOp,
) -> GpuBuffer {
    let op_expr = match op {
        UnaryOp::Neg => "-a[idx]",
        UnaryOp::Abs => "abs(a[idx])",
        UnaryOp::Exp => "exp(a[idx])",
        UnaryOp::Log => "log(a[idx])",
        UnaryOp::Sqrt => "sqrt(a[idx])",
        UnaryOp::Sin => "sin(a[idx])",
        UnaryOp::Cos => "cos(a[idx])",
        UnaryOp::Tan => "tan(a[idx])",
    };

    let shader_src = format!(
        r#"
@group(0) @binding(0) var<storage, read> a: array<f32>;
@group(0) @binding(1) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {{
    let idx = gid.x;
    if idx < arrayLength(&out) {{
        out[idx] = {op_expr};
    }}
}}
"#
    );

    run_unary_shader(ctx, input, &shader_src)
}

/// Scalar operation on GPU: out[i] = a[i] op scalar
pub fn gpu_scalar_op(
    ctx: &GpuContext,
    input: &GpuBuffer,
    scalar: f32,
    op: ScalarOp,
) -> GpuBuffer {
    let op_expr = match op {
        ScalarOp::Add => format!("a[idx] + {scalar:.10}"),
        ScalarOp::Mul => format!("a[idx] * {scalar:.10}"),
    };

    let shader_src = format!(
        r#"
@group(0) @binding(0) var<storage, read> a: array<f32>;
@group(0) @binding(1) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {{
    let idx = gid.x;
    if idx < arrayLength(&out) {{
        out[idx] = {op_expr};
    }}
}}
"#
    );

    run_unary_shader(ctx, input, &shader_src)
}

// ── Shader execution helpers ──────────────────────────────────────────

fn run_binary_shader(
    ctx: &GpuContext,
    a: &GpuBuffer,
    b: &GpuBuffer,
    shader_src: &str,
) -> GpuBuffer {
    let output = GpuBuffer::zeros(ctx, a.len);

    let module = ctx
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("binary_shader"),
            source: wgpu::ShaderSource::Wgsl(shader_src.into()),
        });

    let bind_group_layout =
        ctx.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[
                    storage_entry(0, true),
                    storage_entry(1, true),
                    storage_entry(2, false),
                ],
            });

    let pipeline_layout = ctx
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bind_group_layout],
            ..Default::default()
        });

    let pipeline = ctx
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

    let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: a.buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: b.buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: output.buffer.as_entire_binding(),
            },
        ],
    });

    let workgroups = (a.len as u32 + 255) / 256;
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(workgroups, 1, 1);
    }
    ctx.queue.submit(Some(encoder.finish()));

    output
}

fn run_unary_shader(
    ctx: &GpuContext,
    input: &GpuBuffer,
    shader_src: &str,
) -> GpuBuffer {
    let output = GpuBuffer::zeros(ctx, input.len);

    let module = ctx
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("unary_shader"),
            source: wgpu::ShaderSource::Wgsl(shader_src.into()),
        });

    let bind_group_layout =
        ctx.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[storage_entry(0, true), storage_entry(1, false)],
            });

    let pipeline_layout = ctx
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bind_group_layout],
            ..Default::default()
        });

    let pipeline = ctx
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

    let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.buffer.as_entire_binding(),
            },
        ],
    });

    let workgroups = (input.len as u32 + 255) / 256;
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(workgroups, 1, 1);
    }
    ctx.queue.submit(Some(encoder.finish()));

    output
}

/// Helper to create a storage buffer binding layout entry.
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


/// GPU matrix multiplication: A (m×k) * B (k×n) → C (m×n).
/// Buffers are row-major flat arrays.
pub fn gpu_matmul(
    ctx: &GpuContext,
    a: &GpuBuffer,
    b: &GpuBuffer,
    m: u32,
    k: u32,
    n: u32,
) -> GpuBuffer {
    assert_eq!(a.len, (m * k) as usize, "gpu_matmul: A size mismatch");
    assert_eq!(b.len, (k * n) as usize, "gpu_matmul: B size mismatch");

    let output = GpuBuffer::zeros(ctx, (m * n) as usize);

    // Uniform buffer for dimensions
    use wgpu::util::DeviceExt;
    let dims = [m, k, n, 0u32]; // pad to 16 bytes
    let dim_buf = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("matmul_dims"),
        contents: bytemuck::cast_slice(&dims),
        usage: wgpu::BufferUsages::UNIFORM,
    });

    let shader_src = r#"
struct Dims {
    m: u32,
    k: u32,
    n: u32,
    _pad: u32,
}

@group(0) @binding(0) var<storage, read> a: array<f32>;
@group(0) @binding(1) var<storage, read> b: array<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<f32>;
@group(0) @binding(3) var<uniform> dims: Dims;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let row = gid.x;
    let col = gid.y;
    if row >= dims.m || col >= dims.n {
        return;
    }
    var sum: f32 = 0.0;
    for (var i: u32 = 0u; i < dims.k; i = i + 1u) {
        sum = sum + a[row * dims.k + i] * b[i * dims.n + col];
    }
    out[row * dims.n + col] = sum;
}
"#;

    let module = ctx.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("matmul_shader"),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    });

    let bind_group_layout = ctx.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            storage_entry(0, true),
            storage_entry(1, true),
            storage_entry(2, false),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });

    let pipeline_layout = ctx.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&bind_group_layout],
        ..Default::default()
    });

    let pipeline = ctx.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });

    let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: a.buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: b.buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: output.buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: dim_buf.as_entire_binding() },
        ],
    });

    let wg_x = (m + 15) / 16;
    let wg_y = (n + 15) / 16;
    let mut encoder = ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(wg_x, wg_y, 1);
    }
    ctx.queue.submit(Some(encoder.finish()));

    output
}

/// GPU parallel sum reduction.
/// Uses a tree-reduction shader for large inputs, falls back to CPU for small ones.
pub fn gpu_sum(ctx: &GpuContext, input: &GpuBuffer) -> f64 {
    // For small buffers, CPU reduction is faster (avoids shader compilation overhead)
    if input.len <= 1024 {
        let data = input.to_f64(ctx);
        return data.iter().sum();
    }

    // Tree reduction: each workgroup reduces 256 elements → 1 partial sum
    let mut current = gpu_reduce_step(ctx, input);
    while current.len > 1 {
        current = gpu_reduce_step(ctx, &current);
    }
    let result = current.to_f64(ctx);
    result[0]
}

/// One step of parallel reduction: input[N] → output[ceil(N/256)]
fn gpu_reduce_step(ctx: &GpuContext, input: &GpuBuffer) -> GpuBuffer {
    let workgroup_size = 256u32;
    let n = input.len as u32;
    let num_workgroups = (n + workgroup_size - 1) / workgroup_size;
    let output = GpuBuffer::zeros(ctx, num_workgroups as usize);

    let shader_src = format!(
        r#"
@group(0) @binding(0) var<storage, read> input: array<f32>;
@group(0) @binding(1) var<storage, read_write> output: array<f32>;

var<workgroup> wg_data: array<f32, {workgroup_size}>;

@compute @workgroup_size({workgroup_size})
fn main(
    @builtin(local_invocation_id) lid: vec3<u32>,
    @builtin(workgroup_id) wid: vec3<u32>,
) {{
    let global_id = wid.x * {workgroup_size}u + lid.x;
    let n = arrayLength(&input);

    // Load into workgroup memory
    if global_id < n {{
        wg_data[lid.x] = input[global_id];
    }} else {{
        wg_data[lid.x] = 0.0;
    }}
    workgroupBarrier();

    // Tree reduction in workgroup memory
    var stride = {workgroup_size}u / 2u;
    loop {{
        if stride == 0u {{
            break;
        }}
        if lid.x < stride {{
            wg_data[lid.x] = wg_data[lid.x] + wg_data[lid.x + stride];
        }}
        workgroupBarrier();
        stride = stride / 2u;
    }}

    // First thread writes result
    if lid.x == 0u {{
        output[wid.x] = wg_data[0];
    }}
}}
"#
    );

    let module = ctx.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("reduce_shader"),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    });

    let bind_group_layout = ctx.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[storage_entry(0, true), storage_entry(1, false)],
    });

    let pipeline_layout = ctx.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&bind_group_layout],
        ..Default::default()
    });

    let pipeline = ctx.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });

    let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: input.buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: output.buffer.as_entire_binding() },
        ],
    });

    let mut encoder = ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(num_workgroups, 1, 1);
    }
    ctx.queue.submit(Some(encoder.finish()));

    output
}

/// Check if GPU is available.
pub fn gpu_available() -> bool {
    GpuContext::get().is_some()
}

/// Query GPU memory info. Returns (used_bytes_estimate, max_buffer_size).
/// `used_bytes_estimate` is approximate — wgpu doesn't expose exact VRAM usage.
pub fn gpu_memory_info(ctx: &GpuContext) -> (u64, u64) {
    let limits = ctx.device.limits();
    let max_buffer = limits.max_buffer_size;
    // wgpu doesn't expose current VRAM usage; return 0 as estimate
    (0, max_buffer)
}

/// Try to allocate a GPU buffer, returning None if allocation fails (OOM).
pub fn gpu_try_alloc(ctx: &GpuContext, len: usize) -> Option<GpuBuffer> {
    let byte_size = (len * std::mem::size_of::<f32>()) as u64;
    let limits = ctx.device.limits();
    if byte_size > limits.max_buffer_size {
        return None; // Would exceed max buffer size
    }
    // Attempt allocation — wgpu will panic on true OOM, so we check limits first
    Some(GpuBuffer::zeros(ctx, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skip_if_no_gpu() -> Option<&'static GpuContext> {
        GpuContext::get()
    }

    /// An operation with no shader reads a GPU tensor's data through a
    /// download: indexing, a broadcast and `diag` of a GPU tensor panicked
    /// ("tensor is on GPU").
    #[test]
    fn a_gpu_tensor_is_read_by_operations_without_a_shader() {
        let Some(_ctx) = skip_if_no_gpu() else { return };
        use crate::tensor::CrowTensor;
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]).to_gpu();
        assert!(t.is_gpu());
        assert_eq!(t.get_flat(1), 2.0);
        let m = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]).to_gpu();
        let row = CrowTensor::from_vec(vec![10.0, 20.0], vec![2]);
        assert_eq!(m.zip_with(&row, |a, b| a + b).to_vec(), vec![11.0, 22.0, 13.0, 24.0]);
        assert_eq!(CrowTensor::diag(&m).to_vec(), vec![1.0, 4.0]);
    }

    #[test]
    fn test_gpu_available() {
        // Just check it doesn't panic
        let _ = gpu_available();
    }

    #[test]
    fn test_upload_download() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let data = vec![1.0, 2.0, 3.0, 4.0];
        let buf = GpuBuffer::from_f64(ctx, &data);
        let result = buf.to_f64(ctx);
        for (a, b) in data.iter().zip(result.iter()) {
            assert!((a - b).abs() < 1e-5, "mismatch: {a} vs {b}");
        }
    }

    #[test]
    fn test_gpu_add() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let a = GpuBuffer::from_f64(ctx, &[1.0, 2.0, 3.0, 4.0]);
        let b = GpuBuffer::from_f64(ctx, &[10.0, 20.0, 30.0, 40.0]);
        let c = gpu_elementwise_binary(ctx, &a, &b, BinaryOp::Add);
        let result = c.to_f64(ctx);
        assert_eq!(result.len(), 4);
        for (got, expected) in result.iter().zip([11.0, 22.0, 33.0, 44.0].iter()) {
            assert!((got - expected).abs() < 1e-5);
        }
    }

    #[test]
    fn test_gpu_mul() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let a = GpuBuffer::from_f64(ctx, &[2.0, 3.0, 4.0]);
        let b = GpuBuffer::from_f64(ctx, &[5.0, 6.0, 7.0]);
        let c = gpu_elementwise_binary(ctx, &a, &b, BinaryOp::Mul);
        let result = c.to_f64(ctx);
        for (got, expected) in result.iter().zip([10.0, 18.0, 28.0].iter()) {
            assert!((got - expected).abs() < 1e-4);
        }
    }

    #[test]
    fn test_gpu_neg() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let a = GpuBuffer::from_f64(ctx, &[1.0, -2.0, 3.0]);
        let c = gpu_elementwise_unary(ctx, &a, UnaryOp::Neg);
        let result = c.to_f64(ctx);
        for (got, expected) in result.iter().zip([-1.0, 2.0, -3.0].iter()) {
            assert!((got - expected).abs() < 1e-5);
        }
    }

    #[test]
    fn test_gpu_exp() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let a = GpuBuffer::from_f64(ctx, &[0.0, 1.0]);
        let c = gpu_elementwise_unary(ctx, &a, UnaryOp::Exp);
        let result = c.to_f64(ctx);
        assert!((result[0] - 1.0).abs() < 1e-5);
        assert!((result[1] - std::f64::consts::E).abs() < 1e-3); // f32 precision
    }

    #[test]
    fn test_gpu_scalar_add() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let a = GpuBuffer::from_f64(ctx, &[1.0, 2.0, 3.0]);
        let c = gpu_scalar_op(ctx, &a, 10.0, ScalarOp::Add);
        let result = c.to_f64(ctx);
        for (got, expected) in result.iter().zip([11.0, 12.0, 13.0].iter()) {
            assert!((got - expected).abs() < 1e-5);
        }
    }

    #[test]
    fn test_gpu_sum_small() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let a = GpuBuffer::from_f64(ctx, &[1.0, 2.0, 3.0, 4.0]);
        let s = gpu_sum(ctx, &a);
        assert!((s - 10.0).abs() < 1e-4);
    }

    #[test]
    fn test_gpu_sum_large() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        // Test with >1024 elements to exercise the GPU reduction shader
        let n = 4096;
        let data: Vec<f64> = (1..=n).map(|i| i as f64).collect();
        let expected: f64 = (n * (n + 1) / 2) as f64;
        let buf = GpuBuffer::from_f64(ctx, &data);
        let s = gpu_sum(ctx, &buf);
        // f32 precision: relative error
        let rel_err = (s - expected).abs() / expected;
        assert!(rel_err < 1e-3, "gpu_sum large: expected {expected}, got {s}, rel_err {rel_err}");
    }

    #[test]
    fn test_gpu_matmul_2x2() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        // [[1,2],[3,4]] * [[5,6],[7,8]] = [[19,22],[43,50]]
        let a = GpuBuffer::from_f64(ctx, &[1.0, 2.0, 3.0, 4.0]);
        let b = GpuBuffer::from_f64(ctx, &[5.0, 6.0, 7.0, 8.0]);
        let c = gpu_matmul(ctx, &a, &b, 2, 2, 2);
        let result = c.to_f64(ctx);
        let expected = [19.0, 22.0, 43.0, 50.0];
        for (got, exp) in result.iter().zip(expected.iter()) {
            assert!((got - exp).abs() < 1e-3, "matmul 2x2: got {got}, expected {exp}");
        }
    }

    #[test]
    fn test_gpu_matmul_rect() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        // (2×3) * (3×2) = (2×2)
        // [[1,2,3],[4,5,6]] * [[7,8],[9,10],[11,12]] = [[58,64],[139,154]]
        let a = GpuBuffer::from_f64(ctx, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let b = GpuBuffer::from_f64(ctx, &[7.0, 8.0, 9.0, 10.0, 11.0, 12.0]);
        let c = gpu_matmul(ctx, &a, &b, 2, 3, 2);
        let result = c.to_f64(ctx);
        let expected = [58.0, 64.0, 139.0, 154.0];
        for (got, exp) in result.iter().zip(expected.iter()) {
            assert!((got - exp).abs() < 1e-2, "matmul rect: got {got}, expected {exp}");
        }
    }

    #[test]
    fn test_gpu_matmul_identity() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        // A * I = A
        let a = GpuBuffer::from_f64(ctx, &[1.0, 2.0, 3.0, 4.0]);
        let eye = GpuBuffer::from_f64(ctx, &[1.0, 0.0, 0.0, 1.0]);
        let c = gpu_matmul(ctx, &a, &eye, 2, 2, 2);
        let result = c.to_f64(ctx);
        for (got, exp) in result.iter().zip([1.0, 2.0, 3.0, 4.0].iter()) {
            assert!((got - exp).abs() < 1e-5, "matmul identity: got {got}, expected {exp}");
        }
    }

    #[test]
    fn test_gpu_memory_info() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let (_used, max_buf) = gpu_memory_info(ctx);
        assert!(max_buf > 0, "max_buffer_size should be > 0");
    }

    #[test]
    fn test_gpu_try_alloc_small() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let buf = gpu_try_alloc(ctx, 1024);
        assert!(buf.is_some(), "should be able to allocate 1024 f32s");
    }

    #[test]
    fn test_gpu_try_alloc_too_large() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        // Try to allocate more than max_buffer_size
        let (_used, max_buf) = gpu_memory_info(ctx);
        let too_large = (max_buf / 4) as usize + 1; // +1 element beyond max
        let buf = gpu_try_alloc(ctx, too_large);
        assert!(buf.is_none(), "should fail to allocate beyond max_buffer_size");
    }

    #[test]
    fn test_gpu_all_unary_ops() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let data = [1.0_f64, 2.0, 0.5, 0.25];
        let buf = GpuBuffer::from_f64(ctx, &data);

        // Test all unary ops produce reasonable results
        for op in [UnaryOp::Neg, UnaryOp::Abs, UnaryOp::Exp, UnaryOp::Log, UnaryOp::Sqrt, UnaryOp::Sin, UnaryOp::Cos] {
            let result = gpu_elementwise_unary(ctx, &buf, op);
            let out = result.to_f64(ctx);
            assert_eq!(out.len(), 4, "unary op {op:?} should preserve length");
            for v in &out {
                assert!(v.is_finite(), "unary op {op:?} produced non-finite: {v}");
            }
        }
    }

    #[test]
    fn test_gpu_all_binary_ops() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let a = GpuBuffer::from_f64(ctx, &[10.0, 20.0, 30.0, 40.0]);
        let b = GpuBuffer::from_f64(ctx, &[2.0, 4.0, 5.0, 8.0]);

        for op in [BinaryOp::Add, BinaryOp::Sub, BinaryOp::Mul, BinaryOp::Div] {
            let result = gpu_elementwise_binary(ctx, &a, &b, op);
            let out = result.to_f64(ctx);
            assert_eq!(out.len(), 4, "binary op {op:?} should preserve length");
            for v in &out {
                assert!(v.is_finite(), "binary op {op:?} produced non-finite: {v}");
            }
        }
    }

    // ── Benchmarks ────────────────────────────────────────────────────────
    // These tests measure GPU vs CPU performance and print results.
    // Run with: cargo test -p crow_runtime bench_gpu -- --nocapture --ignored

    #[test]
    #[ignore] // Run manually: cargo test bench_gpu_matmul -- --nocapture --ignored
    fn bench_gpu_matmul_vs_cpu() {
        use std::time::Instant;
        let Some(ctx) = skip_if_no_gpu() else {
            println!("No GPU available, skipping benchmark");
            return;
        };

        for &size in &[64, 128, 256, 512] {
            let n = size;
            let data_a: Vec<f64> = (0..n*n).map(|i| (i as f64) * 0.001).collect();
            let data_b: Vec<f64> = (0..n*n).map(|i| (i as f64) * 0.002).collect();

            // GPU timing (includes upload + compute + download)
            let t0 = Instant::now();
            let a_gpu = GpuBuffer::from_f64(ctx, &data_a);
            let b_gpu = GpuBuffer::from_f64(ctx, &data_b);
            let c_gpu = gpu_matmul(ctx, &a_gpu, &b_gpu, n as u32, n as u32, n as u32);
            let _result_gpu = c_gpu.to_f64(ctx);
            let gpu_time = t0.elapsed();

            // CPU timing (faer)
            let t0 = Instant::now();
            let a_t = crate::tensor::CrowTensor::from_vec(data_a.clone(), vec![n, n]);
            let b_t = crate::tensor::CrowTensor::from_vec(data_b.clone(), vec![n, n]);
            let _c_t = a_t.matmul(&b_t);
            let cpu_time = t0.elapsed();

            let speedup = cpu_time.as_secs_f64() / gpu_time.as_secs_f64();
            println!("matmul {n}x{n}: GPU={gpu_time:.2?}, CPU={cpu_time:.2?}, speedup={speedup:.2}x");
        }
    }

    #[test]
    #[ignore]
    fn bench_gpu_elementwise_vs_cpu() {
        use std::time::Instant;
        let Some(ctx) = skip_if_no_gpu() else { return };

        for &size in &[10_000, 100_000, 1_000_000] {
            let data: Vec<f64> = (0..size).map(|i| (i as f64) * 0.001).collect();

            // GPU: upload + exp + download
            let t0 = Instant::now();
            let buf = GpuBuffer::from_f64(ctx, &data);
            let out = gpu_elementwise_unary(ctx, &buf, UnaryOp::Exp);
            let _result = out.to_f64(ctx);
            let gpu_time = t0.elapsed();

            // CPU: exp
            let t0 = Instant::now();
            let _cpu_result: Vec<f64> = data.iter().map(|x| x.exp()).collect();
            let cpu_time = t0.elapsed();

            let speedup = cpu_time.as_secs_f64() / gpu_time.as_secs_f64();
            println!("exp[{size}]: GPU={gpu_time:.2?}, CPU={cpu_time:.2?}, speedup={speedup:.2}x");
        }
    }

    #[test]
    #[ignore]
    fn bench_gpu_reduction_vs_cpu() {
        use std::time::Instant;
        let Some(ctx) = skip_if_no_gpu() else { return };

        for &size in &[10_000, 100_000, 1_000_000] {
            let data: Vec<f64> = (0..size).map(|i| (i as f64) * 0.001).collect();

            // GPU sum
            let t0 = Instant::now();
            let buf = GpuBuffer::from_f64(ctx, &data);
            let _gpu_sum = gpu_sum(ctx, &buf);
            let gpu_time = t0.elapsed();

            // CPU sum
            let t0 = Instant::now();
            let _cpu_sum: f64 = data.iter().sum();
            let cpu_time = t0.elapsed();

            let speedup = cpu_time.as_secs_f64() / gpu_time.as_secs_f64();
            println!("sum[{size}]: GPU={gpu_time:.2?}, CPU={cpu_time:.2?}, speedup={speedup:.2}x");
        }
    }
}