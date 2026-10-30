use crate::image_processor::Result;
use crate::vectorizer::{has_holes, Point, VectorizedData};
use std::fs::File;
use std::io::Write;
use std::path::Path;

/// Write SVG directly to any `Write` implementor without building the entire
/// output string in memory. This is the streaming equivalent of `generate_svg`.
pub fn generate_svg_to<W: Write>(vectorized_data: &VectorizedData, writer: &mut W) -> Result<()> {
    let (bg_str, bg_opacity) = color_with_alpha(vectorized_data.background_color);

    writeln!(
        writer,
        r#"<svg width="{}" height="{}" viewBox="0 0 {} {}" xmlns="http://www.w3.org/2000/svg">"#,
        vectorized_data.width,
        vectorized_data.height,
        vectorized_data.width,
        vectorized_data.height
    )?;
    writeln!(
        writer,
        r#"  <rect width="{}" height="{}" fill="{}"{}/>"#,
        vectorized_data.width, vectorized_data.height, bg_str, bg_opacity
    )?;

    for curve in &vectorized_data.curves {
        let (color_str, opacity_attr) = color_with_alpha(curve.color);

        // Stream path data directly to avoid building a large intermediate string
        write!(writer, r#"  <path d=""#)?;
        if !curve.subpaths.is_empty() {
            write_multi_path_to(writer, &curve.subpaths)?;
        } else if !curve.points.is_empty() {
            write_subpath_to(writer, &curve.points, curve.is_closed)?;
        } else {
            writeln!(writer, "\" fill=\"{}\"{} stroke=\"none\"/>", color_str, opacity_attr)?;
            continue;
        };

        let fill_rule = if has_holes(&curve.subpaths) {
            r#" fill-rule="evenodd""#
        } else {
            ""
        };
        writeln!(
            writer,
            r#"" fill="{}"{}{} stroke="none"/>"#,
            color_str, opacity_attr, fill_rule
        )?;
    }

    writeln!(writer, "</svg>")?;
    Ok(())
}

/// Convenience wrapper that writes to a file path.
pub fn generate_svg(vectorized_data: &VectorizedData, output_path: &Path) -> Result<()> {
    let mut file = File::create(output_path)?;
    generate_svg_to(vectorized_data, &mut file)
}

pub fn generate_svg_advanced(vectorized_data: &VectorizedData, output_path: &Path) -> Result<()> {
    // Advanced mode uses the same output — colors are already grouped by the vectorizer
    generate_svg(vectorized_data, output_path)
}

/// Stream multiple subpaths directly to a writer, separated by spaces.
pub fn write_multi_path_to<W: Write>(writer: &mut W, subpaths: &[Vec<Point>]) -> Result<()> {
    let mut first = true;
    for sp in subpaths {
        if sp.len() < 3 {
            continue;
        }
        if !first {
            write!(writer, " ")?;
        }
        write_subpath_to(writer, sp, true)?;
        first = false;
    }
    Ok(())
}

/// Return SVG hex color and optional fill-opacity attribute for (r,g,b,a).
pub fn color_with_alpha(color: (u8, u8, u8, u8)) -> (String, String) {
    let hex = format!("#{:02x}{:02x}{:02x}", color.0, color.1, color.2);
    if color.3 < 255 {
        let opacity = (color.3 as f64 / 255.0 * 100.0).round() / 100.0;
        (hex, format!(" fill-opacity=\"{}\"", opacity))
    } else {
        (hex, String::new())
    }
}

/// Format a coordinate compactly: use integer if close to whole, else 1 decimal.
fn fmt_coord(v: f64) -> String {
    let rounded = (v * 2.0).round() / 2.0; // snap to 0.5 grid
    if (rounded - rounded.round()).abs() < 0.01 {
        format!("{}", rounded.round() as i32)
    } else {
        format!("{:.1}", rounded)
    }
}

/// Stream a single subpath directly to a writer.
/// Avoids allocating an intermediate string.
pub fn write_subpath_to<W: Write>(writer: &mut W, pts: &[Point], closed: bool) -> Result<()> {
    let n = pts.len();
    if n == 0 {
        return Ok(());
    }

    write!(writer, "M{} {}", fmt_coord(pts[0].x), fmt_coord(pts[0].y))?;
    let mut prev_x = fmt_coord(pts[0].x);
    let mut prev_y = fmt_coord(pts[0].y);

    for i in 1..n {
        let x = fmt_coord(pts[i].x);
        let y = fmt_coord(pts[i].y);
        if x == prev_x && y == prev_y {
            continue;
        }
        write!(writer, "L{} {}", x, y)?;
        prev_x = x;
        prev_y = y;
    }
    if closed {
        write!(writer, "Z")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    include!("svg_generator_tests.rs");
}
