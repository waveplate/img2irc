use wgpu::util::DeviceExt;
use crate::draw::{FastGlyph, RenderResult, RenderedCell, AnsiImage};
use crate::args::{RenderArgs, Render};
use crate::palette::{ANSI256, IRC99};
use bytemuck::{Pod, Zeroable};
use std::borrow::Cow;
use tokio::sync::oneshot;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct GpuGlyphInfo {
    width: u32,
    height: u32,
    offset: u32,
    padding: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Globals {
    img_width: u32,
    img_height: u32,
    block_width: u32,
    block_height: u32,
    num_glyphs: u32,
    mode: u32, // 0=ANSI, 1=IRC, 2=ANSI24
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct OutputCell {
    glyph_index: u32,
    fg: u32, // packed rgb
    bg: u32, // packed rgb
    flags: u32, // bit 0 = inverted
}

pub struct GpuContext {
    instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

impl GpuContext {
    pub async fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }).await.ok()?;

        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("img2irc GPU"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits {
                max_storage_buffer_binding_size: 128 << 20, // 128MB
                ..Default::default()
            },
            ..Default::default()
        }).await.ok()?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("img2irc Shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("shader.wgsl"))),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Bind Group Layout"),
            entries: &[
                // Globals
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Image Pixels
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
                // Glyph Info
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
                // Glyph Bitmaps
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
                // Output
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Palette
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Compute Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Some(Self {
            instance,
            device,
            queue,
            pipeline,
            bind_group_layout,
        })
    }

    pub fn render(
        &self,
        image: &AnsiImage,
        glyphs: &[FastGlyph],
        _args: &RenderArgs,
        render_mode: Render,
    ) -> RenderResult {
        let (gh, gw) = image.glyphs.metrics;
        let img_width = image.bitmap[0].len() as u32;
        let img_height = image.bitmap.len() as u32;
        let block_cols = (img_width + gw as u32 - 1) / gw as u32;
        let block_rows = (img_height + gh as u32 - 1) / gh as u32;
        
        let globals = Globals {
            img_width,
            img_height,
            block_width: gw as u32,
            block_height: gh as u32,
            num_glyphs: glyphs.len() as u32,
            mode: match render_mode {
                Render::Ansi => 0,
                Render::Irc => 1,
                Render::Ansi24 => 2,
            },
        };

        // Flatten Image
        let pixel_count = (img_width * img_height) as usize;
        let mut pixels = Vec::with_capacity(pixel_count);
        for row in &image.bitmap {
            pixels.extend_from_slice(row);
        }

        // Flatten Glyphs
        let mut gpu_glyph_infos = Vec::with_capacity(glyphs.len());
        let mut gpu_glyph_bitmaps = Vec::new();
        
        for g in glyphs {
            let offset = gpu_glyph_bitmaps.len() as u32;
            gpu_glyph_infos.push(GpuGlyphInfo {
                width: gw as u32,
                height: gh as u32,
                offset,
                padding: 0,
            });
            gpu_glyph_bitmaps.extend_from_slice(&g.bitmap);
        }
        while gpu_glyph_bitmaps.len() % 4 != 0 {
            gpu_glyph_bitmaps.push(0);
        }

        // Palette
        let palette = match render_mode {
            Render::Irc => &IRC99[..],
            _ => &ANSI256[..],
        };

        // Create Buffers
        let globals_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Globals Buffer"),
            contents: bytemuck::cast_slice(&[globals]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let pixels_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Pixels Buffer"),
            contents: bytemuck::cast_slice(&pixels),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let glyph_info_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Glyph Info Buffer"),
            contents: bytemuck::cast_slice(&gpu_glyph_infos),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let glyph_bitmap_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Glyph Bitmap Buffer"),
            contents: gpu_glyph_bitmaps.as_slice(),
            usage: wgpu::BufferUsages::STORAGE,
        });
        
        let output_size = (block_cols * block_rows) as u64 * std::mem::size_of::<OutputCell>() as u64;
        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Output Buffer"),
            size: output_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let staging_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Staging Buffer"),
            size: output_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let palette_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Palette Buffer"),
            contents: bytemuck::cast_slice(palette),
            usage: wgpu::BufferUsages::STORAGE,
        });

        // Bind Group
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: pixels_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: glyph_info_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: glyph_bitmap_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: output_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: palette_buffer.as_entire_binding() },
            ],
        });

        // Encode Commands
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { 
                label: None, 
                timestamp_writes: None 
            });
            cpass.set_pipeline(&self.pipeline);
            cpass.set_bind_group(0, &bind_group, &[]);
            
            let x_groups = (block_cols + 15) / 16;
            let y_groups = (block_rows + 15) / 16;
            cpass.dispatch_workgroups(x_groups, y_groups, 1);
        }
        
        encoder.copy_buffer_to_buffer(&output_buffer, 0, &staging_buffer, 0, output_size);
        self.queue.submit(Some(encoder.finish()));

        // Read result
        let buffer_slice = staging_buffer.slice(..);
        let (sender, receiver) = oneshot::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |v| {
            let _ = sender.send(v);
        });
        
        self.instance.poll_all(true);
        
        let result = tokio::task::block_in_place(|| receiver.blocking_recv());

        if let Ok(Ok(())) = result {
            let data = buffer_slice.get_mapped_range();
            let result_cells: &[OutputCell] = bytemuck::cast_slice(&data);
            
            // Reconstruct RenderResult
            let mut grid = Vec::with_capacity(block_rows as usize);
            let mut content = String::new();
            
            let mut first = true;
            let mut last_fg: Option<crate::draw::Colour> = None;
            let mut last_bg: Option<crate::draw::Colour> = None;

            for y in 0..block_rows {
                let mut row_vec = Vec::with_capacity(block_cols as usize);
                for x in 0..block_cols {
                    let idx = (y * block_cols + x) as usize;
                    if idx >= result_cells.len() { break; }
                    let cell = result_cells[idx];
                    
                    let glyph_char = glyphs[cell.glyph_index as usize].ch;
                    let fg_rgb = [
                        ((cell.fg >> 16) & 0xFF) as u8,
                        ((cell.fg >> 8) & 0xFF) as u8,
                        ((cell.fg >> 0) & 0xFF) as u8,
                    ];
                    let bg_rgb = [
                        ((cell.bg >> 16) & 0xFF) as u8,
                        ((cell.bg >> 8) & 0xFF) as u8,
                        ((cell.bg >> 0) & 0xFF) as u8,
                    ];
                    
                    let (fg_col, bg_col) = match render_mode {
                        Render::Ansi => {
                            let fg_u32 = crate::draw::make_rgb_u32(&fg_rgb);
                            let bg_u32 = crate::draw::make_rgb_u32(&bg_rgb);
                            let fg_idx = crate::draw::nearest_hex_colour_fast(fg_u32, &ANSI256);
                            let bg_idx = crate::draw::nearest_hex_colour_fast(bg_u32, &ANSI256);
                            (crate::draw::Colour::Index(fg_idx), Some(crate::draw::Colour::Index(bg_idx)))
                        },
                        Render::Irc => {
                            let fg_u32 = crate::draw::make_rgb_u32(&fg_rgb);
                            let bg_u32 = crate::draw::make_rgb_u32(&bg_rgb);
                            let fg_idx = crate::draw::nearest_hex_colour_fast(fg_u32, &IRC99);
                            let bg_idx = crate::draw::nearest_hex_colour_fast(bg_u32, &IRC99);
                            (crate::draw::Colour::Index(fg_idx), Some(crate::draw::Colour::Index(bg_idx)))
                        },
                        Render::Ansi24 => {
                            (crate::draw::Colour::RGB(fg_rgb), Some(crate::draw::Colour::RGB(bg_rgb)))
                        }
                    };

                    row_vec.push(RenderedCell {
                        char: glyph_char,
                        fg: fg_rgb,
                        bg: Some(bg_rgb), 
                    });
                    
                    crate::draw::emit_colourized(
                        &mut content,
                        render_mode,
                        fg_col,
                        bg_col,
                        glyph_char,
                        &mut first,
                        &mut last_fg,
                        &mut last_bg
                    );
                }
                content.push_str(match render_mode {
                    Render::Ansi | Render::Ansi24 => "\x1b[0m\n",
                    Render::Irc => "\x0f\n",
                });
                grid.push(row_vec);
            }
            
            drop(data);
            staging_buffer.unmap();
            
            return RenderResult {
                content: content.trim_end_matches('\n').into(),
                error_count: 0,
                total_pixels: (img_width * img_height) as u64,
                dynamic_scaling_count: 0,
                grid,
            };
        }

        RenderResult::from("GPU Error")
    }
}