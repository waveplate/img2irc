//! img2svg MCP (Model Context Protocol) server
//!
//! This is a standalone MCP server binary that exposes img2svg functionality
//! through the Model Context Protocol, allowing AI assistants to convert images
//! to SVG format.

use img2svg::{
    batch_convert_enhanced, batch_convert_to_pdf, convert_enhanced_data, generate_svg, load_image,
    vectorize, EnhancedOptions, EnhancedOutputFormat,
};
use img2svg::image_processor::resize_if_needed;
use serde_json::{json, Map, Value};
use std::io::{self, BufRead, Write};
use std::path::Path;

/// Send a JSON-RPC notification for progress updates.
fn send_progress_notification<W: Write>(
    writer: &mut W,
    progress_token: &str,
    progress: f64,
    total: f64,
) -> io::Result<()> {
    let msg = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/progress",
        "params": {
            "progressToken": progress_token,
            "progress": progress,
            "total": total
        }
    });
    writeln!(writer, "{}", msg)?;
    writer.flush()
}

/// MCP Request structure
#[derive(Debug, serde::Deserialize)]
struct McpRequest {
    #[serde(default)]
    #[allow(dead_code)]
    jsonrpc: String,
    #[serde(flatten)]
    kind: RequestKind,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum RequestKind {
    Initialize { id: Value, params: Value },
    ToolsList { id: Value },
    ToolsCall { id: Value, params: ToolCallParams },
}

#[derive(Debug, serde::Deserialize)]
struct ToolCallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

/// MCP Response structure
#[derive(Debug, serde::Serialize)]
struct McpResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<McpError>,
}

#[derive(Debug, serde::Serialize)]
struct McpError {
    code: i32,
    message: String,
}

struct ConversionInfo {
    width: u32,
    height: u32,
    colors_used: usize,
    pipeline: &'static str,
    output_format: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Preset {
    Logo,
    Photo,
    Icon,
}

impl Preset {
    fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "logo" => Some(Self::Logo),
            "photo" => Some(Self::Photo),
            "icon" => Some(Self::Icon),
            _ => None,
        }
    }

    fn apply(self, opts: &mut EnhancedOptions) {
        match self {
            Preset::Logo => {
                opts.num_colors = 8;
                opts.preprocess = false;
                opts.logo_mode = true;
            }
            Preset::Photo => {
                opts.num_colors = 12;
                opts.preprocess = true;
            }
            Preset::Icon => {
                opts.num_colors = 12;
                opts.preprocess = false;
            }
        }
    }
}

fn count_unique_path_colors(data: &img2svg::EnhancedVectorData) -> usize {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    for p in &data.paths {
        seen.insert(p.color);
    }
    seen.len()
}

fn format_label(format: EnhancedOutputFormat) -> &'static str {
    match format {
        EnhancedOutputFormat::Eps => "eps",
        EnhancedOutputFormat::Pdf => "pdf",
        EnhancedOutputFormat::Ai => "ai",
        EnhancedOutputFormat::Svg => "svg",
    }
}

struct ConvertToolArgs {
    num_colors: usize,
    smooth_level: u8,
    threshold: f64,
    preprocess: bool,
    legacy: bool,
    centerline: bool,
    logo_mode: bool,
    superpixel: bool,
    optimize_corners: bool,
    optimize_curves: bool,
    optimize_tolerance: f64,
    min_region_area: usize,
    coordinate_precision: u32,
    detect_gradients: bool,
    subpixel_contours: bool,
    recolor_method: img2svg::RecolorMethod,
    preset: Option<Preset>,
    output_format: EnhancedOutputFormat,
    max_size: u32,
    progress_token: Option<String>,
    num_colors_explicit: bool,
    preprocess_explicit: bool,
}

impl ConvertToolArgs {
    fn parse(args: &Map<String, Value>, output_path: Option<&str>) -> Self {
        let num_colors_explicit = args.contains_key("num_colors");
        let preprocess_explicit = args.contains_key("preprocess");
        let output_format = args
            .get("output_format")
            .and_then(|v| v.as_str())
            .map(|s| match s.to_lowercase().as_str() {
                "eps" => EnhancedOutputFormat::Eps,
                "pdf" => EnhancedOutputFormat::Pdf,
                "ai" => EnhancedOutputFormat::Ai,
                _ => EnhancedOutputFormat::Svg,
            })
            .unwrap_or_else(|| {
                output_path
                    .map(Path::new)
                    .map(EnhancedOutputFormat::from_path)
                    .unwrap_or(EnhancedOutputFormat::Svg)
            });

        Self {
            num_colors: args
                .get("num_colors")
                .and_then(|v| v.as_i64())
                .unwrap_or(16) as usize,
            smooth_level: args
                .get("smooth_level")
                .and_then(|v| v.as_i64())
                .unwrap_or(5) as u8,
            threshold: args
                .get("threshold")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.1),
            preprocess: args
                .get("preprocess")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            legacy: args.get("legacy").and_then(|v| v.as_bool()).unwrap_or(false),
            centerline: args
                .get("centerline")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            logo_mode: args
                .get("logo_mode")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            superpixel: args
                .get("superpixel")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            optimize_corners: args
                .get("optimize_corners")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            optimize_curves: args
                .get("optimize_curves")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            optimize_tolerance: args
                .get("optimize_tolerance")
                .and_then(|v| v.as_f64())
                .filter(|t| *t > 0.0)
                .unwrap_or(img2svg::DEFAULT_OPTIMIZE_TOLERANCE),
            min_region_area: args
                .get("min_region_area")
                .and_then(|v| v.as_i64())
                .map(|v| v.max(0) as usize)
                .unwrap_or(20),
            coordinate_precision: args
                .get("coordinate_precision")
                .and_then(|v| v.as_i64())
                .map(|v| img2svg::clamp_precision(v.max(0) as u32))
                .unwrap_or(img2svg::DEFAULT_COORDINATE_PRECISION),
            detect_gradients: args
                .get("gradients")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            subpixel_contours: args
                .get("subpixel_contours")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            recolor_method: match args
                .get("recolor_method")
                .and_then(|v| v.as_str())
                .unwrap_or("mean")
                .to_lowercase()
                .as_str()
            {
                "median" => img2svg::RecolorMethod::Median,
                "mode" => img2svg::RecolorMethod::Mode,
                _ => img2svg::RecolorMethod::Mean,
            },
            preset: args
                .get("preset")
                .and_then(|v| v.as_str())
                .and_then(Preset::parse),
            output_format,
            max_size: args
                .get("max_size")
                .and_then(|v| v.as_i64())
                .unwrap_or(4096) as u32,
            progress_token: args
                .get("progress_token")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            num_colors_explicit,
            preprocess_explicit,
        }
    }

    fn enhanced_options(&self) -> EnhancedOptions {
        let mut opts = EnhancedOptions {
            num_colors: self.num_colors,
            preprocess: self.preprocess,
            centerline: self.centerline,
            logo_mode: self.logo_mode,
            use_superpixel: self.superpixel,
            optimize_corners: self.optimize_corners,
            optimize_curves: self.optimize_curves,
            optimize_tolerance: self.optimize_tolerance,
            min_region_area: self.min_region_area,
            coordinate_precision: self.coordinate_precision,
            detect_gradients: self.detect_gradients,
            subpixel_contours: self.subpixel_contours,
            recolor_method: self.recolor_method,
            ..Default::default()
        };
        if let Some(p) = self.preset {
            p.apply(&mut opts);
            if self.logo_mode {
                opts.logo_mode = true;
            }
            if self.preprocess_explicit {
                opts.preprocess = self.preprocess;
            }
            if self.num_colors_explicit {
                opts.num_colors = self.num_colors;
            }
        }
        opts
    }
}

fn run_single_conversion(
    input: &str,
    output: &str,
    params: &ConvertToolArgs,
    notify: &dyn Fn(f64, f64),
) -> Result<ConversionInfo, String> {
    if params.legacy {
        notify(0.0, 100.0);
        let image_data = load_image(Path::new(input)).map_err(|e| format!("Image load failed: {}", e))?;
        notify(25.0, 100.0);
        let image_data = resize_if_needed(image_data, params.max_size);
        notify(50.0, 100.0);
        let vectorized_data = vectorize(
            &image_data,
            params.num_colors,
            params.threshold,
            params.smooth_level,
            false,
        )
        .map_err(|e| format!("Vectorization failed: {}", e))?;
        notify(75.0, 100.0);
        generate_svg(&vectorized_data, Path::new(output))
            .map_err(|e| format!("SVG write failed: {}", e))?;
        notify(100.0, 100.0);
        return Ok(ConversionInfo {
            width: image_data.width,
            height: image_data.height,
            colors_used: vectorized_data.curves.len(),
            pipeline: "legacy",
            output_format: "svg",
        });
    }

    notify(0.0, 100.0);
    let image_data = load_image(Path::new(input)).map_err(|e| format!("Image load failed: {}", e))?;
    notify(20.0, 100.0);
    let image_data = resize_if_needed(image_data, params.max_size);
    notify(40.0, 100.0);
    let opts = params.enhanced_options();
    let data = convert_enhanced_data(
        &image_data,
        Path::new(output),
        &opts,
        Some(params.output_format),
    )
    .map_err(|e| format!("Conversion failed: {}", e))?;
    notify(100.0, 100.0);
    Ok(ConversionInfo {
        width: data.width,
        height: data.height,
        colors_used: count_unique_path_colors(&data),
        pipeline: "enhanced",
        output_format: format_label(params.output_format),
    })
}

fn shared_convert_properties() -> Value {
    json!({
        "num_colors": {
            "type": "integer",
            "description": "Number of colors for quantization (1-64, default: 16).",
            "minimum": 1,
            "maximum": 64,
            "default": 16
        },
        "preprocess": {
            "type": "boolean",
            "description": "Apply edge-preserving smoothing and color reduction before vectorization. Default: true.",
            "default": true
        },
        "legacy": {
            "type": "boolean",
            "description": "Use legacy pipeline (median-cut, RDP, line segments) instead of enhanced Bézier.",
            "default": false
        },
        "smooth_level": {
            "type": "integer",
            "description": "Path smoothing level (0-10, default: 5). Legacy pipeline only.",
            "minimum": 0,
            "maximum": 10,
            "default": 5
        },
        "threshold": {
            "type": "number",
            "description": "Edge detection threshold (0.0-1.0, default: 0.1). Legacy pipeline only.",
            "minimum": 0.0,
            "maximum": 1.0,
            "default": 0.1
        },
        "preset": {
            "type": "string",
            "enum": ["logo", "photo", "icon"],
            "description": "Tuned parameter bundle (logo enables logo_mode; photo enables preprocess)"
        },
        "centerline": {
            "type": "boolean",
            "description": "Centerline/stroke tracing for line art (enhanced only)",
            "default": false
        },
        "logo_mode": {
            "type": "boolean",
            "description": "Separate fill regions and outline strokes (enhanced only)",
            "default": false
        },
        "superpixel": {
            "type": "boolean",
            "description": "Use SLIC superpixel segmentation instead of k-means quantization",
            "default": false
        },
        "optimize_corners": {
            "type": "boolean",
            "description": "Potrace-style Bézier corner optimization pass",
            "default": false
        },
        "optimize_curves": {
            "type": "boolean",
            "description": "Potrace `-O` style merge of consecutive nearly-linear curve segments",
            "default": false
        },
        "optimize_tolerance": {
            "type": "number",
            "description": "Merge tolerance in pixels for optimize_curves (higher merges more). Default: 1.5.",
            "minimum": 0.1,
            "default": 1.5
        },
        "min_region_area": {
            "type": "integer",
            "description": "Despeckle: merge connected regions smaller than this (pixels). 0 = off. Default: 20.",
            "minimum": 0,
            "default": 20
        },
        "coordinate_precision": {
            "type": "integer",
            "description": "Decimal places for SVG path coordinates (1-6). Lower values shrink the output file. Default: 2.",
            "minimum": 1,
            "maximum": 6,
            "default": 2
        },
        "gradients": {
            "type": "boolean",
            "description": "Detect smooth gradients per region and render them as native SVG gradient fills.",
            "default": false
        },
        "subpixel_contours": {
            "type": "boolean",
            "description": "Use interpolated (sub-pixel) marching squares for smoother contours on diagonal/curved boundaries. Default: true.",
            "default": true
        },
        "recolor_method": {
            "type": "string",
            "enum": ["mean", "median", "mode"],
            "description": "How to derive each region's display color from the original pixels: mean (default), median, or mode. Median/mode avoid muddy averaged colors in flat-color graphics.",
            "default": "mean"
        },
        "output_format": {
            "type": "string",
            "enum": ["svg", "eps", "pdf", "ai"],
            "description": "Output format (default: svg, or inferred from output path extension)",
            "default": "svg"
        },
        "max_size": {
            "type": "integer",
            "description": "Maximum image dimension in pixels (auto-resize with Lanczos3). Default: 4096.",
            "minimum": 64,
            "maximum": 16384,
            "default": 4096
        },
        "progress_token": {
            "type": "string",
            "description": "Token for receiving real-time progress notifications during conversion."
        }
    })
}

struct Img2SvgMcpServer;

impl Img2SvgMcpServer {
    fn handle_initialize(&self, _params: Value, id: Value) -> McpResponse {
        McpResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "serverInfo": {
                    "name": "img2svg",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "tools": {
                        "listChanged": false
                    }
                }
            })),
            error: None,
        }
    }

    fn handle_tools_list(&self, id: Value) -> McpResponse {
        let shared = shared_convert_properties();
        McpResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(json!({
                "tools": [
                    {
                        "name": "convert_image_to_svg",
                        "description": "Convert a raster image (PNG, JPEG, etc.) to vector format. Default pipeline uses edge-aware quantization, marching squares, and cubic Bézier curves.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "input_path": {
                                    "type": "string",
                                    "description": "Path to the input image file"
                                },
                                "output_path": {
                                    "type": "string",
                                    "description": "Path where vector output will be saved"
                                },
                                "num_colors": shared["num_colors"],
                                "preprocess": shared["preprocess"],
                                "legacy": shared["legacy"],
                                "smooth_level": shared["smooth_level"],
                                "threshold": shared["threshold"],
                                "preset": shared["preset"],
                                "centerline": shared["centerline"],
                                "logo_mode": shared["logo_mode"],
                                "superpixel": shared["superpixel"],
                                "optimize_corners": shared["optimize_corners"],
                                "optimize_curves": shared["optimize_curves"],
                                "optimize_tolerance": shared["optimize_tolerance"],
                                "min_region_area": shared["min_region_area"],
                                "coordinate_precision": shared["coordinate_precision"],
                                "gradients": shared["gradients"],
                                "subpixel_contours": shared["subpixel_contours"],
                                "recolor_method": shared["recolor_method"],
                                "output_format": shared["output_format"],
                                "max_size": shared["max_size"],
                                "progress_token": shared["progress_token"]
                            },
                            "required": ["input_path", "output_path"]
                        }
                    },
                    {
                        "name": "convert_image_directory",
                        "description": "Batch convert all supported images in a directory to vector files (matches CLI batch mode). Enhanced pipeline by default; legacy not supported in batch.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "input_dir": {
                                    "type": "string",
                                    "description": "Directory containing input images"
                                },
                                "output_dir": {
                                    "type": "string",
                                    "description": "Directory where vector outputs will be written"
                                },
                                "num_colors": shared["num_colors"],
                                "preprocess": shared["preprocess"],
                                "preset": shared["preset"],
                                "centerline": shared["centerline"],
                                "logo_mode": shared["logo_mode"],
                                "superpixel": shared["superpixel"],
                                "optimize_corners": shared["optimize_corners"],
                                "optimize_curves": shared["optimize_curves"],
                                "optimize_tolerance": shared["optimize_tolerance"],
                                "min_region_area": shared["min_region_area"],
                                "coordinate_precision": shared["coordinate_precision"],
                                "gradients": shared["gradients"],
                                "subpixel_contours": shared["subpixel_contours"],
                                "recolor_method": shared["recolor_method"],
                                "output_format": shared["output_format"],
                                "max_size": shared["max_size"],
                                "progress_token": shared["progress_token"]
                            },
                            "required": ["input_dir", "output_dir"]
                        }
                    },
                    {
                        "name": "convert_image_directory_to_pdf",
                        "description": "Batch convert all supported images in a directory into a single multi-page PDF. Each image becomes one page sized to its own dimensions (matches `img2svg -i dir/ -o combined.pdf`).",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "input_dir": {
                                    "type": "string",
                                    "description": "Directory containing input images"
                                },
                                "output_path": {
                                    "type": "string",
                                    "description": "Path to the output multi-page PDF file"
                                },
                                "num_colors": shared["num_colors"],
                                "preprocess": shared["preprocess"],
                                "preset": shared["preset"],
                                "centerline": shared["centerline"],
                                "logo_mode": shared["logo_mode"],
                                "superpixel": shared["superpixel"],
                                "optimize_corners": shared["optimize_corners"],
                                "optimize_curves": shared["optimize_curves"],
                                "optimize_tolerance": shared["optimize_tolerance"],
                                "min_region_area": shared["min_region_area"],
                                "coordinate_precision": shared["coordinate_precision"],
                                "gradients": shared["gradients"],
                                "max_size": shared["max_size"],
                                "progress_token": shared["progress_token"]
                            },
                            "required": ["input_dir", "output_path"]
                        }
                    }
                ]
            })),
            error: None,
        }
    }

    fn handle_tools_call(&self, params: ToolCallParams, id: Value) -> McpResponse {
        let args = if let Value::Object(map) = params.arguments {
            map
        } else {
            return McpResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: None,
                error: Some(McpError {
                    code: -32602,
                    message: "Invalid arguments: expected object".to_string(),
                }),
            };
        };

        match params.name.as_str() {
            "convert_image_to_svg" => {
                let input_path = args.get("input_path").and_then(|v| v.as_str());
                let output_path = args.get("output_path").and_then(|v| v.as_str());

                match (input_path, output_path) {
                    (Some(input), Some(output)) => {
                        let tool_args = ConvertToolArgs::parse(&args, Some(output));
                        let progress_token = tool_args.progress_token.clone();
                        let notify = |stage: f64, total: f64| {
                            if let Some(ref token) = progress_token {
                                let _ = send_progress_notification(
                                    &mut io::stdout(),
                                    token,
                                    stage,
                                    total,
                                );
                            }
                        };

                        match run_single_conversion(input, output, &tool_args, &notify) {
                            Ok(info) => {
                                let output_size = std::fs::metadata(output)
                                    .map(|m| m.len())
                                    .unwrap_or(0);
                                McpResponse {
                                    jsonrpc: "2.0".to_string(),
                                    id,
                                    result: Some(json!({
                                        "content": [{
                                            "type": "text",
                                            "text": format!("Successfully converted {} to {}", input, output)
                                        }],
                                        "details": {
                                            "pipeline": info.pipeline,
                                            "output_format": info.output_format,
                                            "output_size_bytes": output_size,
                                            "width": if info.width > 0 { Some(info.width) } else { None::<u32> },
                                            "height": if info.height > 0 { Some(info.height) } else { None::<u32> },
                                            "colors_used": info.colors_used
                                        }
                                    })),
                                    error: None,
                                }
                            }
                            Err(msg) => McpResponse {
                                jsonrpc: "2.0".to_string(),
                                id,
                                result: None,
                                error: Some(McpError {
                                    code: -32000,
                                    message: msg,
                                }),
                            },
                        }
                    }
                    _ => McpResponse {
                        jsonrpc: "2.0".to_string(),
                        id,
                        result: None,
                        error: Some(McpError {
                            code: -32602,
                            message: "Missing required parameters: input_path and output_path".to_string(),
                        }),
                    },
                }
            }
            "convert_image_directory" => {
                let input_dir = args.get("input_dir").and_then(|v| v.as_str());
                let output_dir = args.get("output_dir").and_then(|v| v.as_str());

                match (input_dir, output_dir) {
                    (Some(input), Some(output)) => {
                        if args.get("legacy").and_then(|v| v.as_bool()).unwrap_or(false) {
                            return McpResponse {
                                jsonrpc: "2.0".to_string(),
                                id,
                                result: None,
                                error: Some(McpError {
                                    code: -32602,
                                    message: "Legacy pipeline is not supported for batch conversion".to_string(),
                                }),
                            };
                        }

                        let tool_args = ConvertToolArgs::parse(&args, None);
                        let progress_token = tool_args.progress_token.clone();
                        let opts = tool_args.enhanced_options();

                        match batch_convert_enhanced(
                            Path::new(input),
                            Path::new(output),
                            &opts,
                            tool_args.output_format,
                            tool_args.max_size,
                        ) {
                            Ok(summary) => {
                                if let Some(ref token) = progress_token {
                                    let _ = send_progress_notification(
                                        &mut io::stdout(),
                                        token,
                                        summary.converted as f64,
                                        summary.total.max(1) as f64,
                                    );
                                }
                                let files: Vec<Value> = summary
                                    .entries
                                    .iter()
                                    .map(|e| {
                                        json!({
                                            "input": e.input.to_string_lossy(),
                                            "output": e.output.to_string_lossy(),
                                            "success": e.success,
                                            "error": e.error
                                        })
                                    })
                                    .collect();
                                McpResponse {
                                    jsonrpc: "2.0".to_string(),
                                    id,
                                    result: Some(json!({
                                        "content": [{
                                            "type": "text",
                                            "text": format!(
                                                "Batch complete: {} converted, {} errors (of {} images)",
                                                summary.converted, summary.errors, summary.total
                                            )
                                        }],
                                        "details": {
                                            "pipeline": "enhanced",
                                            "output_format": format_label(tool_args.output_format),
                                            "total": summary.total,
                                            "converted": summary.converted,
                                            "errors": summary.errors,
                                            "files": files
                                        }
                                    })),
                                    error: None,
                                }
                            }
                            Err(e) => McpResponse {
                                jsonrpc: "2.0".to_string(),
                                id,
                                result: None,
                                error: Some(McpError {
                                    code: -32000,
                                    message: format!("Batch conversion failed: {}", e),
                                }),
                            },
                        }
                    }
                    _ => McpResponse {
                        jsonrpc: "2.0".to_string(),
                        id,
                        result: None,
                        error: Some(McpError {
                            code: -32602,
                            message: "Missing required parameters: input_dir and output_dir".to_string(),
                        }),
                    },
                }
            }
            "convert_image_directory_to_pdf" => {
                let input_dir = args.get("input_dir").and_then(|v| v.as_str());
                let output_path = args.get("output_path").and_then(|v| v.as_str());

                match (input_dir, output_path) {
                    (Some(input), Some(output)) => {
                        let tool_args = ConvertToolArgs::parse(&args, Some(output));
                        let progress_token = tool_args.progress_token.clone();
                        let opts = tool_args.enhanced_options();

                        match batch_convert_to_pdf(
                            Path::new(input),
                            Path::new(output),
                            &opts,
                            tool_args.max_size,
                        ) {
                            Ok(summary) => {
                                if let Some(ref token) = progress_token {
                                    let _ = send_progress_notification(
                                        &mut io::stdout(),
                                        token,
                                        summary.converted as f64,
                                        summary.total.max(1) as f64,
                                    );
                                }
                                let output_size = std::fs::metadata(output)
                                    .map(|m| m.len())
                                    .unwrap_or(0);
                                let files: Vec<Value> = summary
                                    .entries
                                    .iter()
                                    .map(|e| {
                                        json!({
                                            "input": e.input.to_string_lossy(),
                                            "output": e.output.to_string_lossy(),
                                            "success": e.success,
                                            "error": e.error
                                        })
                                    })
                                    .collect();
                                McpResponse {
                                    jsonrpc: "2.0".to_string(),
                                    id,
                                    result: Some(json!({
                                        "content": [{
                                            "type": "text",
                                            "text": format!(
                                                "Multi-page PDF written: {} pages, {} errors (of {} images)",
                                                summary.converted, summary.errors, summary.total
                                            )
                                        }],
                                        "details": {
                                            "pipeline": "enhanced",
                                            "output_format": "pdf",
                                            "output_size_bytes": output_size,
                                            "output_path": output,
                                            "total": summary.total,
                                            "converted": summary.converted,
                                            "errors": summary.errors,
                                            "files": files
                                        }
                                    })),
                                    error: None,
                                }
                            }
                            Err(e) => McpResponse {
                                jsonrpc: "2.0".to_string(),
                                id,
                                result: None,
                                error: Some(McpError {
                                    code: -32000,
                                    message: format!("Multi-page PDF batch failed: {}", e),
                                }),
                            },
                        }
                    }
                    _ => McpResponse {
                        jsonrpc: "2.0".to_string(),
                        id,
                        result: None,
                        error: Some(McpError {
                            code: -32602,
                            message: "Missing required parameters: input_dir and output_path".to_string(),
                        }),
                    },
                }
            }
            _ => McpResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: None,
                error: Some(McpError {
                    code: -32601,
                    message: format!("Unknown tool: {}", params.name),
                }),
            },
        }
    }

    fn run(&self) {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut stdout_lock = stdout.lock();

        for line in stdin.lock().lines() {
            if let Ok(json_str) = line {
                if let Ok(req) = serde_json::from_str::<McpRequest>(&json_str) {
                    let response = match req.kind {
                        RequestKind::Initialize { id, params } => self.handle_initialize(params, id),
                        RequestKind::ToolsList { id } => self.handle_tools_list(id),
                        RequestKind::ToolsCall { id, params } => self.handle_tools_call(params, id),
                    };

                    if let Ok(response_json) = serde_json::to_string(&response) {
                        writeln!(stdout_lock, "{}", response_json).ok();
                        stdout_lock.flush().ok();
                    }
                }
            }
        }
    }
}

fn main() {
    let server = Img2SvgMcpServer;
    server.run();
}

#[cfg(test)]
mod tests {
    use super::Img2SvgMcpServer;
    use serde_json::json;

    #[test]
    fn initialize_returns_server_info() {
        let s = Img2SvgMcpServer;
        let r = s.handle_initialize(json!({}), json!(1));
        assert!(r.error.is_none());
        let res = r.result.unwrap();
        assert_eq!(res["protocolVersion"], "2024-11-05");
        assert_eq!(res["serverInfo"]["name"], "img2svg");
    }

    #[test]
    fn tools_list_exposes_convert_tool() {
        let s = Img2SvgMcpServer;
        let r = s.handle_tools_list(json!(42));
        assert!(r.error.is_none());
        let body = r.result.unwrap();
        let tools = body["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0]["name"], "convert_image_to_svg");
        assert_eq!(tools[1]["name"], "convert_image_directory");
        assert_eq!(tools[2]["name"], "convert_image_directory_to_pdf");
        let props = tools[0]["inputSchema"]["properties"].as_object().unwrap();
        assert!(props.contains_key("legacy"));
        assert!(props.contains_key("preset"));
        assert!(props.contains_key("centerline"));
        assert!(props.contains_key("coordinate_precision"));
        assert!(props.contains_key("optimize_tolerance"));
        assert!(props.contains_key("gradients"));
        assert!(props.contains_key("subpixel_contours"));
        assert!(props.contains_key("recolor_method"));
        let required = tools[0]["inputSchema"]["required"]
            .as_array()
            .expect("required");
        assert!(required.contains(&json!("input_path")));
        let pdf_required = tools[2]["inputSchema"]["required"].as_array().expect("required");
        assert!(pdf_required.contains(&json!("input_dir")));
        assert!(pdf_required.contains(&json!("output_path")));
    }

    #[test]
    fn tools_call_rejects_non_object_arguments() {
        let s = Img2SvgMcpServer;
        let r = s.handle_tools_call(
            super::ToolCallParams {
                name: "convert_image_to_svg".to_string(),
                arguments: json!("not-an-object"),
            },
            json!(1),
        );
        assert!(r.result.is_none());
        let err = r.error.unwrap();
        assert_eq!(err.code, -32602);
    }

    #[test]
    fn tools_call_missing_paths_returns_error() {
        let s = Img2SvgMcpServer;
        let r = s.handle_tools_call(
            super::ToolCallParams {
                name: "convert_image_to_svg".to_string(),
                arguments: json!({ "num_colors": 8 }),
            },
            json!(2),
        );
        assert!(r.result.is_none());
        assert_eq!(r.error.unwrap().code, -32602);
    }

    #[test]
    fn tools_call_unknown_tool() {
        let s = Img2SvgMcpServer;
        let r = s.handle_tools_call(
            super::ToolCallParams {
                name: "nope".to_string(),
                arguments: json!({}),
            },
            json!(3),
        );
        assert_eq!(r.error.unwrap().code, -32601);
    }

    #[test]
    fn tools_call_convert_defaults_to_enhanced_pipeline() {
        let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/input/simple.png");
        if !input.exists() {
            return;
        }
        let out = std::env::temp_dir().join("img2svg_mcp_convert_test.svg");
        let _ = std::fs::remove_file(&out);

        let s = Img2SvgMcpServer;
        let r = s.handle_tools_call(
            super::ToolCallParams {
                name: "convert_image_to_svg".to_string(),
                arguments: json!({
                    "input_path": input.to_str().unwrap(),
                    "output_path": out.to_str().unwrap(),
                    "num_colors": 8,
                    "preprocess": false,
                }),
            },
            json!(99),
        );

        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(out.exists());
        let text = std::fs::read_to_string(&out).expect("read output");
        assert!(text.contains("<svg"));
        let res = r.result.unwrap();
        assert_eq!(res["details"]["pipeline"], "enhanced");
        assert_eq!(res["details"]["output_format"], "svg");
        assert!(res["details"]["output_size_bytes"].as_u64().unwrap_or(0) > 0);
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn tools_call_convert_legacy_pipeline() {
        let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/input/simple.png");
        if !input.exists() {
            return;
        }
        let out = std::env::temp_dir().join("img2svg_mcp_legacy_test.svg");
        let _ = std::fs::remove_file(&out);

        let s = Img2SvgMcpServer;
        let r = s.handle_tools_call(
            super::ToolCallParams {
                name: "convert_image_to_svg".to_string(),
                arguments: json!({
                    "input_path": input.to_str().unwrap(),
                    "output_path": out.to_str().unwrap(),
                    "num_colors": 8,
                    "legacy": true,
                }),
            },
            json!(100),
        );

        assert!(r.error.is_none(), "{:?}", r.error);
        let res = r.result.unwrap();
        assert_eq!(res["details"]["pipeline"], "legacy");
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn tools_call_convert_with_preset_logo() {
        let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/input/logo.png");
        if !input.exists() {
            return;
        }
        let out = std::env::temp_dir().join("img2svg_mcp_preset_logo.svg");
        let _ = std::fs::remove_file(&out);

        let s = Img2SvgMcpServer;
        let r = s.handle_tools_call(
            super::ToolCallParams {
                name: "convert_image_to_svg".to_string(),
                arguments: json!({
                    "input_path": input.to_str().unwrap(),
                    "output_path": out.to_str().unwrap(),
                    "preset": "logo",
                }),
            },
            json!(101),
        );

        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(out.exists());
        let res = r.result.unwrap();
        assert_eq!(res["details"]["pipeline"], "enhanced");
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn tools_call_convert_eps_output() {
        let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/input/simple.png");
        if !input.exists() {
            return;
        }
        let out = std::env::temp_dir().join("img2svg_mcp_eps_test.eps");
        let _ = std::fs::remove_file(&out);

        let s = Img2SvgMcpServer;
        let r = s.handle_tools_call(
            super::ToolCallParams {
                name: "convert_image_to_svg".to_string(),
                arguments: json!({
                    "input_path": input.to_str().unwrap(),
                    "output_path": out.to_str().unwrap(),
                    "num_colors": 8,
                    "preprocess": false,
                    "output_format": "eps",
                }),
            },
            json!(102),
        );

        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(out.exists());
        let text = std::fs::read_to_string(&out).expect("read eps");
        assert!(text.contains("%!PS-Adobe"));
        let res = r.result.unwrap();
        assert_eq!(res["details"]["output_format"], "eps");
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn tools_call_convert_with_coordinate_precision() {
        let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/input/simple.png");
        if !input.exists() {
            return;
        }
        let out = std::env::temp_dir().join("img2svg_mcp_precision.svg");
        let _ = std::fs::remove_file(&out);

        let s = Img2SvgMcpServer;
        let r = s.handle_tools_call(
            super::ToolCallParams {
                name: "convert_image_to_svg".to_string(),
                arguments: json!({
                    "input_path": input.to_str().unwrap(),
                    "output_path": out.to_str().unwrap(),
                    "num_colors": 8,
                    "preprocess": false,
                    "coordinate_precision": 1,
                }),
            },
            json!(103),
        );

        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(out.exists());
        let text = std::fs::read_to_string(&out).expect("read svg");
        assert!(text.contains("<path") || text.contains("<rect"));
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn tools_call_convert_directory_to_pdf() {
        let input_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/input");
        if !input_dir.exists() {
            return;
        }
        let out = std::env::temp_dir().join("img2svg_mcp_dir_pdf_test.pdf");
        let _ = std::fs::remove_file(&out);

        let s = Img2SvgMcpServer;
        let r = s.handle_tools_call(
            super::ToolCallParams {
                name: "convert_image_directory_to_pdf".to_string(),
                arguments: json!({
                    "input_dir": input_dir.to_str().unwrap(),
                    "output_path": out.to_str().unwrap(),
                    "num_colors": 8,
                    "preprocess": false,
                }),
            },
            json!(103),
        );

        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(out.exists());
        let pdf = std::fs::read_to_string(&out).expect("read pdf");
        assert!(pdf.starts_with("%PDF-1.4"));
        assert!(pdf.contains("/Count"));
        let res = r.result.unwrap();
        assert_eq!(res["details"]["pipeline"], "enhanced");
        assert_eq!(res["details"]["output_format"], "pdf");
        assert!(res["details"]["output_size_bytes"].as_u64().unwrap() > 0);
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn tools_call_directory_to_pdf_missing_params_errors() {
        let s = Img2SvgMcpServer;
        let r = s.handle_tools_call(
            super::ToolCallParams {
                name: "convert_image_directory_to_pdf".to_string(),
                arguments: json!({}),
            },
            json!(104),
        );
        assert!(r.error.is_some());
        assert!(r.result.is_none());
    }

    #[test]
    fn send_progress_notification_format() {
        let mut buf = Vec::new();
        super::send_progress_notification(&mut buf, "token-123", 50.0, 100.0).unwrap();
        let text = String::from_utf8(buf).unwrap();
        let msg: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(msg["jsonrpc"], "2.0");
        assert_eq!(msg["method"], "notifications/progress");
        assert_eq!(msg["params"]["progressToken"], "token-123");
        assert_eq!(msg["params"]["progress"], 50.0);
        assert_eq!(msg["params"]["total"], 100.0);
    }

    #[test]
    fn tools_call_convert_with_progress_token_succeeds() {
        let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/input/simple.png");
        if !input.exists() {
            return;
        }
        let out = std::env::temp_dir().join("img2svg_mcp_progress_test.svg");
        let _ = std::fs::remove_file(&out);

        let s = Img2SvgMcpServer;
        let r = s.handle_tools_call(
            super::ToolCallParams {
                name: "convert_image_to_svg".to_string(),
                arguments: json!({
                    "input_path": input.to_str().unwrap(),
                    "output_path": out.to_str().unwrap(),
                    "num_colors": 8,
                    "preprocess": false,
                    "progress_token": "test-token-42",
                }),
            },
            json!(103),
        );

        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(out.exists());
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn tools_call_convert_with_max_size_limits_dimensions() {
        let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/input/simple.png");
        if !input.exists() {
            return;
        }
        let out = std::env::temp_dir().join("img2svg_mcp_maxsize_test.svg");
        let _ = std::fs::remove_file(&out);

        let s = Img2SvgMcpServer;
        let r = s.handle_tools_call(
            super::ToolCallParams {
                name: "convert_image_to_svg".to_string(),
                arguments: json!({
                    "input_path": input.to_str().unwrap(),
                    "output_path": out.to_str().unwrap(),
                    "num_colors": 8,
                    "preprocess": false,
                    "max_size": 32,
                }),
            },
            json!(104),
        );

        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(out.exists());
        let res = r.result.unwrap();
        let width = res["details"]["width"].as_u64().unwrap_or(0);
        let height = res["details"]["height"].as_u64().unwrap_or(0);
        assert!(
            width <= 32 || height <= 32,
            "Expected dimensions <= 32 after resize, got {}x{}",
            width,
            height
        );
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn tools_call_batch_directory_converts_examples() {
        let input_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/input");
        if !input_dir.exists() {
            return;
        }
        let output_dir = std::env::temp_dir().join("img2svg_mcp_batch_test");
        let _ = std::fs::remove_dir_all(&output_dir);

        let s = Img2SvgMcpServer;
        let r = s.handle_tools_call(
            super::ToolCallParams {
                name: "convert_image_directory".to_string(),
                arguments: json!({
                    "input_dir": input_dir.to_str().unwrap(),
                    "output_dir": output_dir.to_str().unwrap(),
                    "num_colors": 8,
                    "preprocess": false,
                }),
            },
            json!(105),
        );

        assert!(r.error.is_none(), "{:?}", r.error);
        let res = r.result.unwrap();
        assert!(res["details"]["converted"].as_u64().unwrap_or(0) >= 2);
        assert_eq!(res["details"]["pipeline"], "enhanced");
        let _ = std::fs::remove_dir_all(&output_dir);
    }

    #[test]
    fn tools_call_batch_rejects_legacy() {
        let s = Img2SvgMcpServer;
        let r = s.handle_tools_call(
            super::ToolCallParams {
                name: "convert_image_directory".to_string(),
                arguments: json!({
                    "input_dir": "/tmp/in",
                    "output_dir": "/tmp/out",
                    "legacy": true,
                }),
            },
            json!(106),
        );
        assert_eq!(r.error.unwrap().code, -32602);
    }
}
