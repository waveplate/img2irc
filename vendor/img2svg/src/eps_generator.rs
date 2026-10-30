//! EPS and minimal PDF export from vector data.

use crate::bezier_fitter::{bezier_to_svg_path_to, is_linear_curve, BezierCurve};
use crate::enhanced_vectorizer::{write_enhanced_svg_prec, EnhancedVectorData};
use crate::image_processor::Result;
use crate::svg_generator::color_with_alpha;
use crate::vectorizer::{Point, VectorizedData};
use std::fs::File;
use std::io::Write;
use std::path::Path;

fn flip_y(y: f64, height: u32) -> f64 {
    height as f64 - y
}

fn write_eps_header<W: Write>(writer: &mut W, width: u32, height: u32) -> Result<()> {
    writeln!(writer, "%!PS-Adobe-3.0 EPSF-3.0")?;
    writeln!(writer, "%%BoundingBox: 0 0 {} {}", width, height)?;
    writeln!(writer, "%%HiResBoundingBox: 0 0 {} {}", width, height)?;
    writeln!(writer, "0 {} translate", height)?;
    writeln!(writer, "1 -1 scale")?;
    Ok(())
}

fn points_to_eps_path<W: Write>(writer: &mut W, points: &[Point], closed: bool, height: u32) -> Result<()> {
    if points.is_empty() {
        return Ok(());
    }
    writeln!(
        writer,
        "{:.2} {:.2} moveto",
        points[0].x,
        flip_y(points[0].y, height)
    )?;
    for p in points.iter().skip(1) {
        writeln!(writer, "{:.2} {:.2} lineto", p.x, flip_y(p.y, height))?;
    }
    if closed {
        writeln!(writer, "closepath")?;
    }
    Ok(())
}

fn curves_to_eps_path<W: Write>(writer: &mut W, curves: &[BezierCurve], closed: bool, height: u32) -> Result<()> {
    if curves.is_empty() {
        return Ok(());
    }
    let mut buf = Vec::new();
    bezier_to_svg_path_to(&mut buf, curves, closed)?;
    let s = String::from_utf8(buf).unwrap_or_default();
    for token in s.split_whitespace() {
        if token.starts_with('M') {
            let coords: Vec<f64> = token[1..]
                .split(',')
                .filter_map(|v| v.parse().ok())
                .collect();
            if coords.len() >= 2 {
                writeln!(writer, "{:.2} {:.2} moveto", coords[0], flip_y(coords[1], height))?;
            }
        } else if token.starts_with('L') {
            let coords: Vec<f64> = token[1..]
                .split(',')
                .filter_map(|v| v.parse().ok())
                .collect();
            if coords.len() >= 2 {
                writeln!(writer, "{:.2} {:.2} lineto", coords[0], flip_y(coords[1], height))?;
            }
        } else if token == "Z" {
            writeln!(writer, "closepath")?;
        }
    }
    Ok(())
}

/// Write legacy pipeline data as EPS.
pub fn write_vectorized_eps(data: &VectorizedData, output_path: &Path) -> Result<()> {
    let mut file = File::create(output_path)?;
    write_eps_header(&mut file, data.width, data.height)?;

    let (bg, bg_op) = color_with_alpha(data.background_color);
    let bg_rgb = parse_hex(&bg);
    writeln!(
        &mut file,
        "{:.3} {:.3} {:.3} setrgbcolor",
        bg_rgb.0, bg_rgb.1, bg_rgb.2
    )?;
    writeln!(
        &mut file,
        "0 0 {} {} rectfill",
        data.width,
        data.height
    )?;
    let _ = bg_op;

    for curve in &data.curves {
        let (hex, _) = color_with_alpha(curve.color);
        let rgb = parse_hex(&hex);
        writeln!(&mut file, "{:.3} {:.3} {:.3} setrgbcolor", rgb.0, rgb.1, rgb.2)?;
        for sub in &curve.subpaths {
            points_to_eps_path(&mut file, sub, curve.is_closed, data.height)?;
            writeln!(&mut file, "fill")?;
        }
        if !curve.points.is_empty() {
            points_to_eps_path(&mut file, &curve.points, curve.is_closed, data.height)?;
            writeln!(&mut file, "fill")?;
        }
    }

    writeln!(&mut file, "showpage")?;
    Ok(())
}

/// Write enhanced pipeline data as EPS.
pub fn write_enhanced_eps(data: &EnhancedVectorData, output_path: &Path) -> Result<()> {
    let mut file = File::create(output_path)?;
    write_eps_header(&mut file, data.width, data.height)?;

    let (bg, _) = color_with_alpha(data.background_color);
    let bg_rgb = parse_hex(&bg);
    writeln!(
        &mut file,
        "{:.3} {:.3} {:.3} setrgbcolor",
        bg_rgb.0, bg_rgb.1, bg_rgb.2
    )?;
    writeln!(
        &mut file,
        "0 0 {} {} rectfill",
        data.width,
        data.height
    )?;

    for path in &data.paths {
        let (hex, _) = color_with_alpha(path.color);
        let rgb = parse_hex(&hex);
        writeln!(&mut file, "{:.3} {:.3} {:.3} setrgbcolor", rgb.0, rgb.1, rgb.2)?;
        if path.stroke_only {
            writeln!(&mut file, "1 setlinewidth")?;
        }
        if let Some(ref svg) = path.svg_override {
            write_svg_override_eps(&mut file, svg, data.height)?;
        } else {
            curves_to_eps_path(&mut file, &path.curves, !path.stroke_only, data.height)?;
        }
        if path.stroke_only {
            writeln!(&mut file, "stroke")?;
        } else {
            writeln!(&mut file, "fill")?;
        }
    }

    writeln!(&mut file, "showpage")?;
    Ok(())
}

fn write_svg_override_eps<W: Write>(writer: &mut W, svg: &str, height: u32) -> Result<()> {
    for token in svg.split_whitespace() {
        if let Some(rest) = token.strip_prefix('M') {
            let parts: Vec<f64> = rest.split(',').filter_map(|v| v.parse().ok()).collect();
            if parts.len() >= 2 {
                writeln!(writer, "{:.2} {:.2} moveto", parts[0], flip_y(parts[1], height))?;
            }
        } else if let Some(rest) = token.strip_prefix('L') {
            let parts: Vec<f64> = rest.split(',').filter_map(|v| v.parse().ok()).collect();
            if parts.len() >= 2 {
                writeln!(writer, "{:.2} {:.2} lineto", parts[0], flip_y(parts[1], height))?;
            }
        } else if token == "Z" {
            writeln!(writer, "closepath")?;
        }
    }
    Ok(())
}

/// Minimal PDF wrapper (single page, path content from EPS-style ops).
pub fn write_enhanced_pdf(data: &EnhancedVectorData, output_path: &Path) -> Result<()> {
    write_multi_page_pdf(std::slice::from_ref(data), output_path)
}

/// Write one PDF content stream for an enhanced page (background + filled paths).
///
/// Coordinates are y-flipped from image space (top-left) to PDF space (bottom-left).
fn enhanced_page_content_to<W: Write>(writer: &mut W, data: &EnhancedVectorData) -> Result<()> {
    let height = data.height;

    // Background rectangle fills the whole page.
    let (bg_hex, _) = color_with_alpha(data.background_color);
    let bg = parse_hex(&bg_hex);
    writeln!(writer, "q")?;
    writeln!(writer, "{:.3} {:.3} {:.3} rg", bg.0, bg.1, bg.2)?;
    writeln!(writer, "0 0 {} {} re f", data.width, data.height)?;
    writeln!(writer, "Q")?;

    for path in &data.paths {
        let (hex, _) = color_with_alpha(path.color);
        let rgb = parse_hex(&hex);
        writeln!(writer, "q")?;
        writeln!(writer, "{:.3} {:.3} {:.3} rg", rgb.0, rgb.1, rgb.2)?;
        if path.stroke_only {
            writeln!(writer, "1 w")?;
        }
        if let Some(ref svg) = path.svg_override {
            write_svg_override_pdf(writer, svg, height)?;
        } else {
            write_curves_pdf(writer, &path.curves, height)?;
        }
        if path.stroke_only {
            writeln!(writer, "S")?;
        } else {
            writeln!(writer, "f")?;
        }
        writeln!(writer, "Q")?;
    }
    Ok(())
}

/// Emit cubic Bézier curves as a PDF path (m/l/c/h operators, y-flipped).
fn write_curves_pdf<W: Write>(writer: &mut W, curves: &[BezierCurve], height: u32) -> Result<()> {
    if curves.is_empty() {
        return Ok(());
    }
    let first = &curves[0];
    writeln!(
        writer,
        "{:.2} {:.2} m",
        first.start.x,
        flip_y(first.start.y, height)
    )?;
    for curve in curves {
        if is_linear_curve(curve) {
            writeln!(
                writer,
                "{:.2} {:.2} l",
                curve.end.x,
                flip_y(curve.end.y, height)
            )?;
        } else {
            writeln!(
                writer,
                "{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c",
                curve.control1.x,
                flip_y(curve.control1.y, height),
                curve.control2.x,
                flip_y(curve.control2.y, height),
                curve.end.x,
                flip_y(curve.end.y, height),
            )?;
        }
    }
    writeln!(writer, "h")?;
    Ok(())
}

fn write_svg_override_pdf<W: Write>(writer: &mut W, svg: &str, height: u32) -> Result<()> {
    for token in svg.split_whitespace() {
        if let Some(rest) = token.strip_prefix('M') {
            let parts: Vec<f64> = rest.split(',').filter_map(|v| v.parse().ok()).collect();
            if parts.len() >= 2 {
                writeln!(writer, "{:.2} {:.2} m", parts[0], flip_y(parts[1], height))?;
            }
        } else if let Some(rest) = token.strip_prefix('L') {
            let parts: Vec<f64> = rest.split(',').filter_map(|v| v.parse().ok()).collect();
            if parts.len() >= 2 {
                writeln!(writer, "{:.2} {:.2} l", parts[0], flip_y(parts[1], height))?;
            }
        } else if token == "Z" {
            writeln!(writer, "h")?;
        }
    }
    Ok(())
}

/// Write a multi-page PDF where each [`EnhancedVectorData`] becomes one page.
///
/// Each page's `MediaBox` matches its source image dimensions. The cross-reference
/// table uses correct byte offsets so strict PDF readers accept the output.
pub fn write_multi_page_pdf(pages: &[EnhancedVectorData], output_path: &Path) -> Result<()> {
    use std::fmt::Write as _;
    if pages.is_empty() {
        anyhow::bail!("cannot write a multi-page PDF with zero pages");
    }

    let n = pages.len();
    let mut buf = String::new();
    let mut offsets: Vec<u64> = Vec::with_capacity(2 + 2 * n);

    buf.push_str("%PDF-1.4\n");
    // Binary marker comment so readers treat the file as binary.
    buf.push_str("%\u{00e2}\u{00e3}\u{00cf}\u{00d3}\n");

    // Object 1: Catalog
    offsets.push(buf.len() as u64);
    buf.push_str("1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    // Object 2: Pages tree
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + i * 2)).collect();
    offsets.push(buf.len() as u64);
    write!(
        buf,
        "2 0 obj\n<< /Type /Pages /Kids [{}] /Count {} >>\nendobj\n",
        kids.join(" "),
        n
    )?;

    // For each page: a Page object (parent = Pages) + a Content stream object.
    for (i, data) in pages.iter().enumerate() {
        let page_num = 3 + i * 2;
        let content_num = 4 + i * 2;

        let mut content = Vec::new();
        enhanced_page_content_to(&mut content, data)?;
        let content_str = String::from_utf8_lossy(&content);

        offsets.push(buf.len() as u64);
        write!(
            buf,
            "{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Contents {} 0 R >>\nendobj\n",
            page_num, data.width, data.height, content_num
        )?;

        offsets.push(buf.len() as u64);
        write!(
            buf,
            "{} 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
            content_num,
            content_str.len(),
            content_str
        )?;
    }

    // Cross-reference table.
    let xref_offset = buf.len() as u64;
    let size = 2 + 2 * n + 1; // objects 0..=highest, highest = 2 + 2n
    write!(buf, "xref\n0 {}\n", size)?;
    buf.push_str("0000000000 65535 f \n");
    for off in &offsets {
        writeln!(buf, "{:010} 00000 n ", off)?;
    }
    write!(buf, "trailer\n<< /Size {} /Root 1 0 R >>\n", size)?;
    write!(buf, "startxref\n{}\n", xref_offset)?;
    buf.push_str("%%EOF");

    let mut file = File::create(output_path)?;
    file.write_all(buf.as_bytes())?;
    Ok(())
}

/// Write enhanced data as Adobe Illustrator-compatible EPS (.ai).
pub fn write_enhanced_ai(data: &EnhancedVectorData, output_path: &Path) -> Result<()> {
    write_enhanced_eps(data, output_path)
}

/// Output format for enhanced pipeline file writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)] // Re-exported at crate root; clippy cannot see the chain.
pub enum EnhancedOutputFormat {
    #[default]
    Svg,
    Eps,
    Pdf,
    Ai,
}

impl EnhancedOutputFormat {
    /// Infer format from a file path extension (`svg` default).
    #[allow(dead_code)] // Re-exported at crate root; clippy cannot see the chain.
    pub fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase())
        {
            Some(ext) if ext == "eps" => Self::Eps,
            Some(ext) if ext == "pdf" => Self::Pdf,
            Some(ext) if ext == "ai" => Self::Ai,
            _ => Self::Svg,
        }
    }
    #[allow(dead_code)] // Re-exported at crate root; clippy cannot see the chain.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Eps => "eps",
            Self::Pdf => "pdf",
            Self::Ai => "ai",
        }
    }
}

/// Write enhanced vector data using the requested output format.
#[allow(dead_code)] // Re-exported at crate root; clippy cannot see the chain.
pub fn write_enhanced_output(
    data: &EnhancedVectorData,
    output_path: &Path,
    format: EnhancedOutputFormat,
) -> Result<()> {
    write_enhanced_output_prec(
        data,
        output_path,
        format,
        crate::bezier_fitter::DEFAULT_COORDINATE_PRECISION,
    )
}

/// Write enhanced vector data using the requested output format with
/// user-controlled SVG coordinate precision (1-6 decimal places, clamped).
/// Non-SVG formats always use the default precision.
pub fn write_enhanced_output_prec(
    data: &EnhancedVectorData,
    output_path: &Path,
    format: EnhancedOutputFormat,
    precision: u32,
) -> Result<()> {
    match format {
        EnhancedOutputFormat::Svg => write_enhanced_svg_prec(data, output_path, precision),
        EnhancedOutputFormat::Eps => write_enhanced_eps(data, output_path),
        EnhancedOutputFormat::Pdf => write_enhanced_pdf(data, output_path),
        EnhancedOutputFormat::Ai => write_enhanced_ai(data, output_path),
    }
}

fn parse_hex(hex: &str) -> (f64, f64, f64) {
    let h = hex.trim_start_matches('#');
    if h.len() >= 6 {
        let r = u8::from_str_radix(&h[0..2], 16).unwrap_or(0) as f64 / 255.0;
        let g = u8::from_str_radix(&h[2..4], 16).unwrap_or(0) as f64 / 255.0;
        let b = u8::from_str_radix(&h[4..6], 16).unwrap_or(0) as f64 / 255.0;
        (r, g, b)
    } else {
        (0.0, 0.0, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced_vectorizer::{EnhancedPath, EnhancedVectorData};

    fn sample_page(w: u32, h: u32) -> EnhancedVectorData {
        EnhancedVectorData {
            width: w,
            height: h,
            background_color: (255, 255, 255, 255),
            paths: vec![EnhancedPath {
                curves: vec![BezierCurve {
                    start: Point { x: 1.0, y: 1.0 },
                    control1: Point { x: 1.0, y: h as f64 - 1.0 },
                    control2: Point { x: w as f64 - 1.0, y: h as f64 - 1.0 },
                    end: Point { x: w as f64 - 1.0, y: 1.0 },
                }],
                color: (200, 30, 30, 255),
                area: 100,
                svg_override: None,
                gradient: None,
                stroke_only: false,
            }],
            group_by_color: false,
        }
    }

    #[test]
    fn test_single_page_pdf_structure() {
        let data = sample_page(20, 20);
        let path = std::path::PathBuf::from("/tmp/img2svg_single.pdf");
        write_enhanced_pdf(&data, &path).unwrap();
        let s = std::fs::read_to_string(&path).unwrap();
        assert!(s.starts_with("%PDF-1.4"));
        assert!(s.contains("/Type /Catalog"));
        assert!(s.contains("/Type /Pages"));
        assert!(s.contains("/Type /Page"));
        assert!(s.contains("/MediaBox [0 0 20 20]"));
        assert!(s.contains("/Count 1"));
        assert!(s.contains("startxref"));
        assert!(s.trim_end().ends_with("%%EOF"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_multi_page_pdf_count_and_mediaboxes() {
        let pages = vec![sample_page(10, 20), sample_page(30, 40), sample_page(50, 60)];
        let path = std::path::PathBuf::from("/tmp/img2svg_multi.pdf");
        write_multi_page_pdf(&pages, &path).unwrap();
        let s = std::fs::read_to_string(&path).unwrap();
        assert!(s.contains("/Count 3"));
        assert!(s.contains("/MediaBox [0 0 10 20]"));
        assert!(s.contains("/MediaBox [0 0 30 40]"));
        assert!(s.contains("/MediaBox [0 0 50 60]"));
        // Three pages → three Kids references. Match "/Type /Page " (trailing
        // space) so it doesn't also count the "/Type /Pages" tree object.
        assert_eq!(s.matches("/Type /Page /Parent").count(), 3);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_multi_page_pdf_xref_offsets_are_valid() {
        let pages = vec![sample_page(15, 15), sample_page(25, 25)];
        let path = std::path::PathBuf::from("/tmp/img2svg_xref.pdf");
        write_multi_page_pdf(&pages, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let s = std::fs::read_to_string(&path).unwrap();
        // Every recorded xref offset must point at "N 0 obj".
        for line in s.lines() {
            if let Some(rest) = line.strip_suffix(" 00000 n ") {
                if rest.chars().all(|c| c.is_ascii_digit()) {
                    let off: usize = rest.parse().unwrap();
                    let snippet = &String::from_utf8_lossy(&bytes[off..]);
                    assert!(
                        snippet.starts_with(|c: char| c.is_ascii_digit()),
                        "xref offset {off} does not point at an object"
                    );
                    assert!(snippet.contains(" obj"));
                }
            }
        }
        // startxref value should point at "xref".
        let sx = s.find("startxref\n").unwrap();
        let after = &s[sx + "startxref\n".len()..];
        let val: usize = after.lines().next().unwrap().parse().unwrap();
        assert_eq!(&String::from_utf8_lossy(&bytes[val..val + 4]), "xref");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_multi_page_pdf_empty_errors() {
        let path = std::path::PathBuf::from("/tmp/img2svg_empty.pdf");
        let res = write_multi_page_pdf(&[], &path);
        assert!(res.is_err());
        assert!(!path.exists());
    }

    #[test]
    fn test_page_content_y_flips_paths() {
        // A path drawn near the top (y=1) of a 20-tall image should appear with
        // pdf y close to the bottom (≈ 19) after the y-flip.
        let data = sample_page(20, 20);
        let mut out = Vec::new();
        enhanced_page_content_to(&mut out, &data).unwrap();
        let s = String::from_utf8(out).unwrap();
        // start (1,1) → pdf y = 20 - 1 = 19
        assert!(s.contains("1.00 19.00 m"));
    }
}
