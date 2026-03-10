use crate::args::Render;
use crate::draw::FastGlyph;
use crate::palette::{ANSI256, IRC99};
use bytemuck::{Pod, Zeroable};
use std::borrow::Cow;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuBlockPixel {
    r: u32,
    g: u32,
    b: u32,
    sq: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuGlyph {
    ones_starts: u32,
    ones_len: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuMetadata {
    bp: u32,
    num_glyphs: u32,
    render_mode: u32,
    tolerance: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default, Debug)]
pub struct GpuResult {
    pub best_cost: u32,
    pub best_glyph: u32,
    pub best_inv: u32,
    pub fg_r: u32,
    pub fg_g: u32,
    pub fg_b: u32,
    pub bg_r: u32,
    pub bg_g: u32,
    pub bg_b: u32,
}

static GPU_CONTEXT: std::sync::OnceLock<(wgpu::Device, wgpu::Queue)> = std::sync::OnceLock::new();

pub fn render_blocks_gpu(
    image: &crate::draw::AnsiImage,
    glyphs: &[FastGlyph],
    render_mode: Render,
    tolerance: u8,
    bp: u32,
    total_blocks: usize,
) -> Vec<GpuResult> {
    let (device, queue) = GPU_CONTEXT.get_or_init(|| {
        std::thread::spawn(|| {
            log::debug!("Initializing WGPU instance...");
            let instance = wgpu::Instance::default();
            log::debug!("Requesting adapter...");
            let adapter =
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    force_fallback_adapter: false,
                    compatible_surface: None,
                }))
                .expect("Failed to find an appropriate adapter");

            let info = adapter.get_info();
            log::info!(
                "Using WGPU Adapter: {} (Backend: {:?}, Device Type: {:?})",
                info.name,
                info.backend,
                info.device_type
            );
            log::debug!("Requesting device from adapter...");
            let (device, queue) =
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::empty(),
                    required_limits: adapter.limits(),
                    memory_hints: wgpu::MemoryHints::Performance,
                    ..Default::default()
                }))
                .expect("Failed to create device");

            log::debug!("WGPU Device and Queue obtained successfully!");
            (device, queue)
        })
        .join()
        .unwrap()
    });

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Compute Shader"),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("shader.wgsl"))),
    });
    let mut gpu_glyphs = Vec::new();
    let mut gpu_ones = Vec::new();
    for g in glyphs {
        let start = gpu_ones.len() as u32;
        for &one in &g.ones {
            gpu_ones.push(one as u32);
        }
        gpu_glyphs.push(GpuGlyph {
            ones_starts: start,
            ones_len: g.ones.len() as u32,
        });
    }

    let palette = match render_mode {
        Render::Ansi => &ANSI256[..],
        Render::Irc | Render::Preview => &IRC99[..],
        Render::Ansi24 => &[],
    };

    let mode_val = match render_mode {
        Render::Ansi => 0,
        Render::Irc | Render::Preview => 1,
        Render::Ansi24 => 2,
    };

    let meta = GpuMetadata {
        bp,
        num_glyphs: glyphs.len() as u32,
        render_mode: mode_val,
        tolerance: tolerance as u32,
    };

    use wgpu::util::DeviceExt;

    let glyphs_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Glyphs Buffer"),
        contents: bytemuck::cast_slice(&gpu_glyphs),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let ones_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Glyph Ones Buffer"),
        contents: bytemuck::cast_slice(&gpu_ones),
        usage: wgpu::BufferUsages::STORAGE,
    });

    // WebGL fallback if empty palette
    let pal = if palette.is_empty() {
        vec![0u32]
    } else {
        palette.to_vec()
    };
    let palette_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Palette Buffer"),
        contents: bytemuck::cast_slice(&pal),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let meta_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Metadata Buffer"),
        contents: bytemuck::cast_slice(&[meta]),
        usage: wgpu::BufferUsages::UNIFORM,
    });

    // Maximum blocks per batch. Reduced to 4 Million pixels to avoid potential limit issues.
    let max_pixels_per_batch = 4_000_000;
    let max_blocks_per_batch = (max_pixels_per_batch / bp as usize).max(1);

    let mut final_results = Vec::with_capacity(total_blocks);

    let mut current_gpu_blocks = Vec::with_capacity(max_blocks_per_batch * bp as usize);
    let mut batch_count = 0;

    // We prepare a resuable Results Buffer scaled to max_blocks_per_batch
    let max_results_size = (max_blocks_per_batch as u64) * std::mem::size_of::<GpuResult>() as u64;
    let results_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Results Buffer"),
        size: max_results_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });

    let blocks_buf_size =
        (max_blocks_per_batch as u64) * bp as u64 * std::mem::size_of::<GpuBlockPixel>() as u64;
    let blocks_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Blocks Buffer"),
        size: blocks_buf_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let staging_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Staging Buffer"),
        size: max_results_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: blocks_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: glyphs_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: ones_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: palette_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: meta_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: results_buf.as_entire_binding(),
            },
        ],
    });

    let t_total = std::time::Instant::now();
    let mut t_prep = std::time::Instant::now();
    let mut total_prep_time = std::time::Duration::new(0, 0);
    let mut total_gpu_time = std::time::Duration::new(0, 0);

    let (gw, gh) = image.glyphs.metrics;

    // Iterate manually to construct batches on the fly
    for row_idx in 0..image.block.len() {
        for col_idx in 0..if image.block.is_empty() {
            0
        } else {
            image.block[0].len()
        } {
            let blk = &image.block[row_idx][col_idx];
            for y in 0..gh {
                for x in 0..gw {
                    let px_orig = blk.pixels[y][x].orig;
                    let rgb = crate::draw::unpack_rgb(px_orig);
                    let p_r = rgb[0] as u32;
                    let p_g = rgb[1] as u32;
                    let p_b = rgb[2] as u32;
                    let p_sq = p_r * p_r + p_g * p_g + p_b * p_b;
                    current_gpu_blocks.push(GpuBlockPixel {
                        r: p_r,
                        g: p_g,
                        b: p_b,
                        sq: p_sq,
                    });
                }
            }
            batch_count += 1;

            if batch_count >= max_blocks_per_batch {
                total_prep_time += t_prep.elapsed();
                let t_batch_gpu = std::time::Instant::now();
                process_batch(
                    device,
                    queue,
                    &compute_pipeline,
                    &bind_group,
                    &blocks_buf,
                    &results_buf,
                    &staging_buf,
                    &current_gpu_blocks,
                    batch_count,
                    &mut final_results,
                );
                total_gpu_time += t_batch_gpu.elapsed();
                current_gpu_blocks.clear();
                batch_count = 0;
                t_prep = std::time::Instant::now();
            }
        }
    }

    if batch_count > 0 {
        total_prep_time += t_prep.elapsed();
        let t_batch_gpu = std::time::Instant::now();
        process_batch(
            device,
            queue,
            &compute_pipeline,
            &bind_group,
            &blocks_buf,
            &results_buf,
            &staging_buf,
            &current_gpu_blocks,
            batch_count,
            &mut final_results,
        );
        total_gpu_time += t_batch_gpu.elapsed();
    }

    log::debug!("Total Prep Time: {:?}", total_prep_time);
    log::debug!("Total GPU Time: {:?}", total_gpu_time);
    log::debug!("Total Render blocks gpu time: {:?}", t_total.elapsed());

    final_results
}

fn process_batch(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    compute_pipeline: &wgpu::ComputePipeline,
    bind_group: &wgpu::BindGroup,
    blocks_buf: &wgpu::Buffer,
    results_buf: &wgpu::Buffer,
    staging_buf: &wgpu::Buffer,
    gpu_blocks: &[GpuBlockPixel],
    batch_blocks: usize,
    final_results: &mut Vec<GpuResult>,
) {
    queue.write_buffer(blocks_buf, 0, bytemuck::cast_slice(gpu_blocks));

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        cpass.set_pipeline(compute_pipeline);
        cpass.set_bind_group(0, bind_group, &[]);
        let workgroups = ((batch_blocks as u32) + 63) / 64;
        cpass.dispatch_workgroups(workgroups, 1, 1);
    }

    let results_size = (batch_blocks as u64) * std::mem::size_of::<GpuResult>() as u64;

    encoder.copy_buffer_to_buffer(results_buf, 0, &staging_buf, 0, results_size);
    log::debug!("Submitting {} blocks to queue", batch_blocks);
    queue.submit(Some(encoder.finish()));

    let buffer_slice = staging_buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer_slice.map_async(wgpu::MapMode::Read, move |v| {
        log::debug!("Map Async Callback Fired! Status: {:?}", v);
        tx.send(v).unwrap()
    });

    log::debug!("Polling device Wait...");
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    log::debug!("Poll returned! Receiving from channel...");

    // Wait for the async mapping to complete
    rx.recv().unwrap().unwrap();
    log::debug!("Channel received data payload successfully!");

    let data = buffer_slice.get_mapped_range();
    let res: &[GpuResult] = bytemuck::cast_slice(&data);

    final_results.extend_from_slice(&res[..batch_blocks]);
    drop(data);
    staging_buf.unmap();
}
