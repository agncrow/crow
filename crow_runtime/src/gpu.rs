use std::sync::OnceLock;
use wgpu;

/// Global GPU context — initialized lazily on first use.
static GPU_CONTEXT: OnceLock<Option<GpuContext>> = OnceLock::new();

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
            tx.send(result).unwrap();
        });
        let _ = ctx.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        rx.recv().unwrap().unwrap();

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

/// Element-wise binary operation on GPU.
/// op_code: "add", "sub", "mul", "div"
pub fn gpu_elementwise_binary(
    ctx: &GpuContext,
    a: &GpuBuffer,
    b: &GpuBuffer,
    op: &str,
) -> GpuBuffer {
    assert_eq!(a.len, b.len, "gpu binary op: mismatched lengths");

    let op_expr = match op {
        "add" => "a[idx] + b[idx]",
        "sub" => "a[idx] - b[idx]",
        "mul" => "a[idx] * b[idx]",
        "div" => "a[idx] / b[idx]",
        _ => panic!("unknown gpu binary op: {op}"),
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
/// op: "neg", "abs", "exp", "log", "sqrt", "sin", "cos"
pub fn gpu_elementwise_unary(
    ctx: &GpuContext,
    input: &GpuBuffer,
    op: &str,
) -> GpuBuffer {
    let op_expr = match op {
        "neg" => "-a[idx]",
        "abs" => "abs(a[idx])",
        "exp" => "exp(a[idx])",
        "log" => "log(a[idx])",
        "sqrt" => "sqrt(a[idx])",
        "sin" => "sin(a[idx])",
        "cos" => "cos(a[idx])",
        _ => panic!("unknown gpu unary op: {op}"),
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
    op: &str,
) -> GpuBuffer {
    let op_expr = match op {
        "add" => format!("a[idx] + {scalar:.10}"),
        "mul" => format!("a[idx] * {scalar:.10}"),
        _ => panic!("unknown gpu scalar op: {op}"),
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


/// GPU sum reduction — downloads to CPU for final reduction.
/// For large tensors, does partial GPU reduction then CPU finish.
pub fn gpu_sum(ctx: &GpuContext, input: &GpuBuffer) -> f64 {
    // For now, download and sum on CPU. A full GPU reduction tree
    // is an optimization we can add later when profiling shows need.
    let data = input.to_f64(ctx);
    data.iter().sum()
}

/// Check if GPU is available.
pub fn gpu_available() -> bool {
    GpuContext::get().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skip_if_no_gpu() -> Option<&'static GpuContext> {
        GpuContext::get()
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
        let c = gpu_elementwise_binary(ctx, &a, &b, "add");
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
        let c = gpu_elementwise_binary(ctx, &a, &b, "mul");
        let result = c.to_f64(ctx);
        for (got, expected) in result.iter().zip([10.0, 18.0, 28.0].iter()) {
            assert!((got - expected).abs() < 1e-4);
        }
    }

    #[test]
    fn test_gpu_neg() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let a = GpuBuffer::from_f64(ctx, &[1.0, -2.0, 3.0]);
        let c = gpu_elementwise_unary(ctx, &a, "neg");
        let result = c.to_f64(ctx);
        for (got, expected) in result.iter().zip([-1.0, 2.0, -3.0].iter()) {
            assert!((got - expected).abs() < 1e-5);
        }
    }

    #[test]
    fn test_gpu_exp() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let a = GpuBuffer::from_f64(ctx, &[0.0, 1.0]);
        let c = gpu_elementwise_unary(ctx, &a, "exp");
        let result = c.to_f64(ctx);
        assert!((result[0] - 1.0).abs() < 1e-5);
        assert!((result[1] - std::f64::consts::E).abs() < 1e-3); // f32 precision
    }

    #[test]
    fn test_gpu_scalar_add() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let a = GpuBuffer::from_f64(ctx, &[1.0, 2.0, 3.0]);
        let c = gpu_scalar_op(ctx, &a, 10.0, "add");
        let result = c.to_f64(ctx);
        for (got, expected) in result.iter().zip([11.0, 12.0, 13.0].iter()) {
            assert!((got - expected).abs() < 1e-5);
        }
    }

    #[test]
    fn test_gpu_sum() {
        let Some(ctx) = skip_if_no_gpu() else { return };
        let a = GpuBuffer::from_f64(ctx, &[1.0, 2.0, 3.0, 4.0]);
        let s = gpu_sum(ctx, &a);
        assert!((s - 10.0).abs() < 1e-4);
    }
}