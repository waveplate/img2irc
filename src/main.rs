mod args;
mod draw;
mod palette;
mod effects;
mod font;


use reqwest;
use url::Url;
use photon_rs::PhotonImage;
use std::{error::Error, io::{Cursor, Write}, process::exit};
use crate::args::{Encoding, Param, RenderArgs, Args};

use rayon::prelude::*;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::main]
async fn main() {
    let args = args::parse_args();

    if args.clear_cache {
        font::clear_cache();
        eprintln!("Cache cleared.");
        if args.image.is_none() && !args.list_blocks && !args.show_block_bitmaps {
            exit(0);
        }
    }

    if args.list_blocks {
        eprintln!("Available blocks:");
        write_encoded_output(&font::print_blocks_config(), &args.encoding);
        exit(0);
    }

    if args.list_ranges {
        write_encoded_output(&font::list_ranges(), &args.encoding);
        exit(0);
    }

    if args.print_ranges {
        write_encoded_output(&font::print_ranges(), &args.encoding);
        exit(0);
    }

    if args.show_block_bitmaps {
        let render_args = args_to_render_args(&args);
        let glyphs = font::glyph_store(
            &render_args.blocks,
            &render_args.exclude_range,
            &render_args.exclude,
            &render_args.include_range,
            render_args.include.as_ref(),
            &render_args.font,
            render_args.y_offset,
            render_args.font_size
        );
        write_encoded_output(&font::print_block_bitmaps(&glyphs), &args.encoding);
        exit(0);
    }

    if let Some(font) = &args.font.first() {
        std::env::set_var("IMG2IRC_FONT", font);
    }



    match load_image_from_url_or_path(&args.clone().image.unwrap().as_str()).await {
        Ok(image) => {
            let render_args = args_to_render_args(&args);
            
            let glyph_store = font::glyph_store(
                &render_args.blocks, 
                &render_args.exclude_range, 
                &render_args.exclude, 
                &render_args.include_range,
                render_args.include.as_ref(),
                &render_args.font,
                render_args.y_offset,
                render_args.font_size
            );            
            eprintln!("Loaded {} glyphs", glyph_store.glyphs.len());
            eprintln!("Cell size: {}x{}", glyph_store.metrics.0, glyph_store.metrics.1);
            
            // Pre-calculate FastGlyphs once
            let glyphs = draw::prepare_glyphs(&glyph_store, &render_args);
            eprintln!("Prepared {} optimized glyphs", glyphs.len());

            // Determine optimization settings
            let opt_mode = if args.auto {
                args.opt.clone().unwrap_or(args::OptimizationMode::BitmapError)
            } else if let Some(m) = &args.opt {
                m.clone()
            } else {
                // No optimization requested
                // No optimization requested
                let result = render_output(&render_args, &glyph_store, image, &glyphs);
                if args.score {
                    let smoothness = draw::calculate_smoothness(&result, &glyph_store);
                    eprintln!("Smoothness Score: {:.4} (Lower is smoother)", smoothness);
                }
                write_encoded_output(&result.content, &args.encoding);
                if let Some(path) = &args.save {
                    let smoothness = draw::calculate_smoothness(&result, &glyph_store);
                    let final_path = path.replace("%s", &format!("{:.4}", smoothness));

                    let metadata = format!("{:?}", args);
                    if let Err(e) = draw::save_as_png(&result, &glyph_store, &final_path, &metadata) {
                        eprintln!("Failed to save PNG: {}", e);
                    } else {
                        eprintln!("Saved image to {}", final_path);
                    }
                }
                return;
            };

            // Generate Tasks
            let tasks = generate_tasks(&args);
            let total_steps = tasks.len();
            
            if total_steps == 0 {
                eprintln!("No optimization tasks generated.");
                exit(1);
            }

            eprintln!("Optimizing {:?} with {} permutations...", opt_mode, total_steps);
            
            let progress = AtomicUsize::new(0);
            let best_found_metric = Arc::new(Mutex::new(f64::MAX));

            // Use CPU-side sequential iteration to prevent stalling
            let best_result = tasks.par_iter().map(|task| {
                process_task(task, &render_args, &glyph_store, &image, &glyphs, &opt_mode, total_steps, &progress, &best_found_metric, &args)
            }).min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

            if let Some((best_metric, _, final_render_args)) = best_result {
                let (final_sx, final_sy) = final_render_args.scale.unwrap_or((1.0, 1.0));
                let final_fuzz = final_render_args.fuzz;
                
                // Print detailed info about what changed
                eprintln!("\nOptimization complete. Best metric: {:.4}", best_metric);
                eprintln!("Scale: {:.3}x{:.3}, Fuzz: {:.3}", final_sx, final_sy, final_fuzz);
                
                // Re-render best result on CPU
                let final_res = render_output(&final_render_args, &glyph_store, image, &glyphs);

                if final_res.dynamic_scaling_count > 0 {
                    eprintln!("Dynamic scaling applied to {} blocks.", final_res.dynamic_scaling_count);
                }
                write_encoded_output(&final_res.content, &args.encoding);
                if let Some(path) = &args.save {
                    let metadata = format!("{:?}", args);
                    if let Err(e) = draw::save_as_png(&final_res, &glyph_store, path, &metadata) {
                        eprintln!("Failed to save PNG: {}", e);
                    }
                }
            } else {
                eprintln!("Optimization failed to produce results.");
            }
        }

        Err(e) => {
            eprintln!("Error: {}", e);
            exit(1);
        }
    }
}

// Optimization Logic Helpers

#[derive(Debug, Clone, Copy)]
enum IterField {
    ScaleX, ScaleY,
    Brightness, Contrast, Gamma, Saturation, Hue,
    LumaBrightness, LumaContrast, LumaGamma, LumaSaturation,
    Fuzz,
    Width, Height, FontSize, GrayscaleTolerance, Pixelize, GaussianBlur, LumaBlur, Rotate,
    CropX1, CropY1, CropX2, CropY2,
}

#[derive(Debug)]
struct Task {
    overrides: Vec<(IterField, f32)>, 
}

fn args_to_render_args(args: &Args) -> RenderArgs {
    let (mut sx, mut sy) = args.scale.unwrap_or((1.0, 1.0));
    if let Some(p) = &args.scalex { sx = param_base(p); }
    if let Some(p) = &args.scaley { sy = param_base(p); }

    let mut crop = args.crop;
    if args.crop_x1.is_some() || args.crop_y1.is_some() || args.crop_x2.is_some() || args.crop_y2.is_some() {
        let (mut x1, mut y1, mut x2, mut y2) = crop.unwrap_or((0, 0, 0, 0));
        if let Some(p) = &args.crop_x1 { x1 = param_base(p) as u32; }
        if let Some(p) = &args.crop_y1 { y1 = param_base(p) as u32; }
        if let Some(p) = &args.crop_x2 { x2 = param_base(p) as u32; }
        if let Some(p) = &args.crop_y2 { y2 = param_base(p) as u32; }
        crop = Some((x1, y1, x2, y2));
    }
    
    RenderArgs {
        list_blocks: args.list_blocks,
        list_ranges: args.list_ranges,
        print_ranges: args.print_ranges,
        show_block_bitmaps: args.show_block_bitmaps,
        clear_cache: args.clear_cache,
        y_offset: args.y_offset,
        image: args.image.clone(),
        width: args.width.as_ref().map(|p| param_base(p) as u32),
        height: args.height.as_ref().map(|p| param_base(p) as u32),
        scale: Some((sx, sy)),
        crop,
        trim: args.trim,
        filter: args.filter.clone(),
        rotate: param_base(&args.rotate),
        fliph: args.fliph,
        flipv: args.flipv,
        render: args.render,
        braille: args.braille,
        blocks: args.blocks.clone(),
        include_range: args.include_range.clone(),
        include: args.include.clone(),
        exclude_range: args.exclude_range.clone(),
        exclude: args.exclude.clone(),
        font: args.font.clone(),
        font_size: param_base(&args.font_size) as u32,
        check_char: args.check_char,
        grayscale_tolerance: param_base(&args.grayscale_tolerance) as u8,
        
        brightness: param_base(&args.brightness),
        contrast: param_base(&args.contrast),
        gamma: param_base(&args.gamma),
        saturation: param_base(&args.saturation),
        hue: param_base(&args.hue),
        invert: args.invert,
        dither: args.dither,
        luma_brightness: param_base(&args.luma_brightness),
        luma_contrast: param_base(&args.luma_contrast),
        luma_gamma: param_base(&args.luma_gamma),
        luma_saturation: param_base(&args.luma_saturation),
        luma_invert: args.luma_invert,
        colorspace: args.colorspace.clone(),
        encoding: args.encoding.clone(),
        grayscale: args.grayscale,
        nograyscale: args.nograyscale,
        pixelize: param_base(&args.pixelize) as i32,
        box_blur: args.box_blur,
        gaussian_blur: param_base(&args.gaussian_blur) as i32,
        luma_blur: param_base(&args.luma_blur) as i32,
        oil: args.oil.clone(),
        halftone: args.halftone,
        sepia: args.sepia,
        normalize: args.normalize,
        noise: args.noise,
        emboss: args.emboss,
        identity: args.identity,
        laplace: args.laplace,
        noise_reduction: args.noise_reduction,
        sharpen: args.sharpen,
        cali: args.cali,
        dramatic: args.dramatic,
        firenze: args.firenze,
        golden: args.golden,
        lix: args.lix,
        lofi: args.lofi,
        neue: args.neue,
        obsidian: args.obsidian,
        pastel_pink: args.pastel_pink,
        ryo: args.ryo,
        frosted_glass: args.frosted_glass,
        solarize: args.solarize,
        edge_detection: args.edge_detection,
        fuzz: param_base(&args.fuzz),
        shape: args.shape,
        smooth: args.smooth,
        score: args.score, // Added score field
        smooth_candidates: args.smooth_candidates,
        smooth_threshold: args.smooth_threshold,
    }
}

fn param_base(p: &Param) -> f32 {
    match p {
        Param::Fixed(v) => *v,
        Param::Range { start, .. } => *start,
    }
}

fn generate_tasks(args: &Args) -> Vec<Task> {
    let mut independent_axes: Vec<Vec<(IterField, f32)>> = Vec::new();
    let (base_sx, base_sy) = args.scale.unwrap_or((1.0, 1.0));

    // Scale X
    if let Some(p) = &args.scalex {
        add_param_axis(&mut independent_axes, p, IterField::ScaleX);
    } else if args.auto && args.range > 0.0 && (args.axis == args::OptimizeAxis::X || args.axis == args::OptimizeAxis::All) { 
         // Legacy behavior
         let mut vars = Vec::new();
         let start = 1.0 - args.range.abs()/100.0;
         let end = 1.0 + args.range.abs()/100.0;
         let step = if args.step <= 0.0 { 1.0 } else { args.step } / 100.0;
         let mut m = start;
         while m <= end + 0.00001 { vars.push((IterField::ScaleX, base_sx * m)); m += step; }
         independent_axes.push(vars);
    }

    // Scale Y
    if let Some(p) = &args.scaley {
        add_param_axis(&mut independent_axes, p, IterField::ScaleY);
    } else if args.auto && args.range > 0.0 && (args.axis == args::OptimizeAxis::Y || args.axis == args::OptimizeAxis::All) {
         let mut vars = Vec::new();
         let start = 1.0 - args.range.abs()/100.0;
         let end = 1.0 + args.range.abs()/100.0;
         let step = if args.step <= 0.0 { 1.0 } else { args.step } / 100.0;
         let mut m = start;
         while m <= end + 0.00001 { vars.push((IterField::ScaleY, base_sy * m)); m += step; }
         independent_axes.push(vars);
    }

    // Params
    add_param_axis(&mut independent_axes, &args.brightness, IterField::Brightness);
    add_param_axis(&mut independent_axes, &args.contrast, IterField::Contrast);
    add_param_axis(&mut independent_axes, &args.gamma, IterField::Gamma);
    add_param_axis(&mut independent_axes, &args.saturation, IterField::Saturation);
    add_param_axis(&mut independent_axes, &args.hue, IterField::Hue);
    add_param_axis(&mut independent_axes, &args.luma_brightness, IterField::LumaBrightness);
    add_param_axis(&mut independent_axes, &args.luma_contrast, IterField::LumaContrast);
    add_param_axis(&mut independent_axes, &args.luma_gamma, IterField::LumaGamma);
    add_param_axis(&mut independent_axes, &args.luma_saturation, IterField::LumaSaturation);
    add_param_axis(&mut independent_axes, &args.fuzz, IterField::Fuzz);
    
    // New numeric params
    add_param_axis(&mut independent_axes, &args.font_size, IterField::FontSize);
    add_param_axis(&mut independent_axes, &args.grayscale_tolerance, IterField::GrayscaleTolerance);
    add_param_axis(&mut independent_axes, &args.pixelize, IterField::Pixelize);
    add_param_axis(&mut independent_axes, &args.gaussian_blur, IterField::GaussianBlur);
    add_param_axis(&mut independent_axes, &args.luma_blur, IterField::LumaBlur);
    add_param_axis(&mut independent_axes, &args.rotate, IterField::Rotate);
    
    // Options
    if let Some(p) = &args.width { add_param_axis(&mut independent_axes, p, IterField::Width); }
    if let Some(p) = &args.height { add_param_axis(&mut independent_axes, p, IterField::Height); }
    
    // Crop
    if let Some(p) = &args.crop_x1 { add_param_axis(&mut independent_axes, p, IterField::CropX1); }
    if let Some(p) = &args.crop_y1 { add_param_axis(&mut independent_axes, p, IterField::CropY1); }
    if let Some(p) = &args.crop_x2 { add_param_axis(&mut independent_axes, p, IterField::CropX2); }
    if let Some(p) = &args.crop_y2 { add_param_axis(&mut independent_axes, p, IterField::CropY2); }

    // If legacy Fuzz axis was requested via --auto (AND fuzz param not set explicitly to range)
    if args.auto && args.range > 0.0 && (args.axis == args::OptimizeAxis::Fuzz || args.axis == args::OptimizeAxis::All) {
        if let Param::Fixed(base_fuzz) = args.fuzz {
             let mut vars = Vec::new();
             let start = 1.0 - args.range.abs()/100.0;
             let end = 1.0 + args.range.abs()/100.0;
             let step = if args.step <= 0.0 { 1.0 } else { args.step } / 100.0;
             let mut m = start;
             while m <= end + 0.00001 {
                 let f = (base_fuzz + (m - 1.0)).max(0.0);
                 vars.push((IterField::Fuzz, f));
                 m += step;
             }
             independent_axes.push(vars);
        }
    }

    // Cartesian Product
    if independent_axes.is_empty() {
        return vec![Task { overrides: vec![] }]; // Run once
    }

    let mut result_tasks = vec![Task { overrides: vec![] }];
    for axis in independent_axes {
        let mut next_tasks = Vec::with_capacity(result_tasks.len() * axis.len());
        for t in result_tasks {
            for val in &axis {
                let mut new_overrides = t.overrides.clone();
                new_overrides.push(*val);
                next_tasks.push(Task { overrides: new_overrides });
            }
        }
        result_tasks = next_tasks;
    }
    
    result_tasks
}

fn add_param_axis(axes: &mut Vec<Vec<(IterField, f32)>>, param: &Param, field: IterField) {
    match param {
        Param::Range { start, end, step } => {
            let mut vars = Vec::new();
            let mut v = *start;
            while v <= end + 0.00001 {
                vars.push((field, v));
                v += *step;
            }
            axes.push(vars);
        },
        _ => {} // Fixed params are handled by base args
    }
}

fn apply_task_to_args(args: &mut RenderArgs, task: &Task) {
    for (field, val) in &task.overrides {
        match field {
            IterField::ScaleX => if let Some(s) = &mut args.scale { s.0 = *val } else { args.scale = Some((*val, 1.0)) },
            IterField::ScaleY => if let Some(s) = &mut args.scale { s.1 = *val } else { args.scale = Some((1.0, *val)) },
            IterField::Brightness => args.brightness = *val,
            IterField::Contrast => args.contrast = *val,
            IterField::Gamma => args.gamma = *val,
            IterField::Saturation => args.saturation = *val,
            IterField::Hue => args.hue = *val,
            IterField::LumaBrightness => args.luma_brightness = *val,
            IterField::LumaContrast => args.luma_contrast = *val,
            IterField::LumaGamma => args.luma_gamma = *val,
            IterField::LumaSaturation => args.luma_saturation = *val,
            IterField::Fuzz => args.fuzz = *val,
            IterField::Width => args.width = Some(*val as u32),
            IterField::Height => args.height = Some(*val as u32),
            IterField::FontSize => args.font_size = *val as u32,
            IterField::GrayscaleTolerance => args.grayscale_tolerance = *val as u8,
            IterField::Pixelize => args.pixelize = *val as i32,
            IterField::GaussianBlur => args.gaussian_blur = *val as i32,
            IterField::LumaBlur => args.luma_blur = *val as i32,
            IterField::Rotate => args.rotate = *val,
            IterField::CropX1 => if let Some(c) = &mut args.crop { c.0 = *val as u32 } else { args.crop = Some((*val as u32, 0, 0, 0)) },
            IterField::CropY1 => if let Some(c) = &mut args.crop { c.1 = *val as u32 } else { args.crop = Some((0, *val as u32, 0, 0)) },
            IterField::CropX2 => if let Some(c) = &mut args.crop { c.2 = *val as u32 } else { args.crop = Some((0, 0, *val as u32, 0)) },
            IterField::CropY2 => if let Some(c) = &mut args.crop { c.3 = *val as u32 } else { args.crop = Some((0, 0, 0, *val as u32)) },
        }
    }
}

fn task_info_string(task: &Task) -> String {
    let mut s = String::new();
    for (field, val) in &task.overrides {
        s.push_str(&format!(" {:?}:{:.3}", field, val));
    }
    s
}

fn process_task(
    task: &Task,
    base_args: &RenderArgs,
    glyph_store: &font::GlyphStore,
    image: &PhotonImage,
    glyphs: &[draw::FastGlyph],
    opt_mode: &args::OptimizationMode,
    total_steps: usize,
    progress: &AtomicUsize,
    best_found_metric: &Arc<Mutex<f64>>,
    raw_cli_args: &Args,
) -> (f64, draw::RenderResult, RenderArgs) {
    let mut loop_args = base_args.clone();
    apply_task_to_args(&mut loop_args, task);

    // Prepare canvas
    let (canvas, luma_canvas) = prepare_canvas(&loop_args, glyph_store, image.clone());
    
    let result = render_from_canvas(&loop_args, &canvas, luma_canvas.as_ref(), glyphs);

    let metric = match opt_mode {
        args::OptimizationMode::TotalBytes => result.content.len() as f64,
        args::OptimizationMode::LongestLine => result.content.lines().map(|l: &str| l.len()).max().unwrap_or(0) as f64,
        args::OptimizationMode::BitmapError => {
            if result.total_pixels > 0 {
                result.error_count as f64 / result.total_pixels as f64
            } else {
                1.0 
            }
        },
        args::OptimizationMode::Smoothness => {
            draw::calculate_smoothness(&result, glyph_store)
        },
        args::OptimizationMode::Relative => {
            draw::calculate_smoothness_relative(&result, glyph_store)
        },
        args::OptimizationMode::Frequency => {
            draw::calculate_frequency_metric(&result, &canvas, glyph_store)
        }
    };

    let p = progress.fetch_add(1, Ordering::Relaxed) + 1;
    let info = task_info_string(task);

    {
        let mut best_val = best_found_metric.lock().unwrap();
        if metric < *best_val {
            *best_val = metric;
            // Clear current line before printing new best
            eprint!("\x1b[2K\r");
            eprintln!("New best! Metric {:.4} ({})", metric, info);

            // Save candidate ONLY if it's a new best
            if let Some(save_dir) = &raw_cli_args.save_candidates {
                let _ = std::fs::create_dir_all(save_dir);
                let safe_info = info.replace(" ", "_").replace(":", "-");
                let fname = format!("{}/metric_{:.4}_{}.png", save_dir, metric, safe_info);
                let metadata = format!("Metric: {}", metric);
                let _ = draw::save_as_png(&result, glyph_store, &fname, &metadata);
            }
        }
    }

    // Detailed logging every iteration as requested
    eprint!("\r[{}/{}] Metric {:.4} ({})", p, total_steps, metric, info);
    let _ = std::io::stderr().flush();

    (metric, result, loop_args)
}

fn prepare_canvas(args: &RenderArgs, glyphs: &font::GlyphStore, image: PhotonImage) -> (draw::AnsiImage, Option<draw::AnsiImage>) {
    let image = effects::apply_effects(args, glyphs, image);
    let canvas = draw::AnsiImage::new(image.clone(), glyphs.clone(), args.grayscale_tolerance);
    
    let luma_canvas = if args.braille {
        let image_luma = effects::apply_luma_effects(args, glyphs, image);
        Some(draw::AnsiImage::new(image_luma, glyphs.clone(), args.grayscale_tolerance))
    } else {
        None
    };
    (canvas, luma_canvas)
}

fn render_from_canvas(args: &RenderArgs, canvas: &draw::AnsiImage, luma_canvas: Option<&draw::AnsiImage>, glyphs: &[draw::FastGlyph]) -> draw::RenderResult {
    if args.braille {
        if let Some(lc) = luma_canvas {
            match args.render {
                args::Render::Irc => draw::render_braille(lc, canvas, args, args::Render::Irc),
                args::Render::Ansi => draw::render_braille(lc, canvas, args, args::Render::Ansi),
                args::Render::Ansi24 => draw::render_braille(lc, canvas, args, args::Render::Ansi24),
            }
        } else {
            draw::RenderResult::from("Error: Braille mode without luma canvas")
        }
    } else {
        if args.smooth {
            match args.render {
                args::Render::Irc => draw::render_blocks_smooth(canvas, args, args::Render::Irc, glyphs),
                args::Render::Ansi => draw::render_blocks_smooth(canvas, args, args::Render::Ansi, glyphs),
                args::Render::Ansi24 => draw::render_blocks_smooth(canvas, args, args::Render::Ansi24, glyphs),
            }
        } else {
            match args.render {
                args::Render::Irc => draw::render_blocks(canvas, args, args::Render::Irc, glyphs),
                args::Render::Ansi => draw::render_blocks(canvas, args, args::Render::Ansi, glyphs),
                args::Render::Ansi24 => draw::render_blocks(canvas, args, args::Render::Ansi24, glyphs),
            }
        }
    }
}

fn render_output(args: &RenderArgs, glyph_store: &font::GlyphStore, image: PhotonImage, glyphs: &[draw::FastGlyph]) -> draw::RenderResult {
    let (canvas, luma_canvas) = prepare_canvas(args, glyph_store, image);
    render_from_canvas(args, &canvas, luma_canvas.as_ref(), glyphs)
}

fn write_encoded_output(content: &str, encoding: &Encoding) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let _ = write_encoded_to(content, encoding, &mut handle);
    let _ = handle.flush();
}

fn write_encoded_to<W: Write>(content: &str, encoding: &Encoding, handle: &mut W) -> std::io::Result<()> {
    match encoding {
        Encoding::Utf8 => {
            writeln!(handle, "{}", content)?;
        }
        Encoding::Utf16 => {
            let bom = 0xFEFFu16;
            #[cfg(target_endian = "little")]
            {
                handle.write_all(&bom.to_le_bytes())?;
                write_encoded_to(content, &Encoding::Utf16le, handle)?;
            }
            #[cfg(target_endian = "big")]
            {
                handle.write_all(&bom.to_be_bytes())?;
                write_encoded_to(content, &Encoding::Utf16be, handle)?;
            }
        }
        Encoding::Utf16be => {
            for c in content.encode_utf16() {
                handle.write_all(&c.to_be_bytes())?;
            }
            handle.write_all(&('\n' as u16).to_be_bytes())?;
        }
        Encoding::Utf16le => {
            for c in content.encode_utf16() {
                handle.write_all(&c.to_le_bytes())?;
            }
            handle.write_all(&('\n' as u16).to_le_bytes())?;
        }
        Encoding::Cesu8 => {
            for u in content.encode_utf16() {
                if u <= 0x7F {
                    handle.write_all(&[u as u8])?;
                } else if u <= 0x7FF {
                    handle.write_all(&[
                        0xC0 | ((u >> 6) as u8),
                        0x80 | ((u & 0x3F) as u8),
                    ])?;
                } else {
                    handle.write_all(&[
                        0xE0 | ((u >> 12) as u8),
                        0x80 | (((u >> 6) & 0x3F) as u8),
                        0x80 | ((u & 0x3F) as u8),
                    ])?;
                }
            }
            handle.write_all(b"\n")?;
        }
    }
    Ok(())
}

async fn load_image_from_url_or_path(image: &str) -> Result<PhotonImage, Box<dyn Error>> {
    match Url::parse(image) {
        Ok(url) => {
            let response = reqwest::get(url).await?;
            let bytes = response.bytes().await?;
            let image_data = Cursor::new(bytes);
            match photon_rs::native::open_image_from_bytes(image_data.into_inner().as_ref()) {
                Ok(image) => Ok(image),
                Err(e) => Err(Box::new(e)),
            }
        }
        Err(_) => {
            match photon_rs::native::open_image(image) {
                Ok(image) => Ok(image),
                Err(e) => Err(Box::new(e)),
            }
        }
    }
}