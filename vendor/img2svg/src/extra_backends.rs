//! Additional vector export backends: DXF, GeoJSON, and GIMP path.
//!
//! These backends flatten the cubic Bézier paths produced by the enhanced
//! pipeline into linear segments suitable for CAD/GIS and graphics tooling.

use crate::bezier_fitter::BezierCurve;
use crate::enhanced_vectorizer::EnhancedVectorData;
use crate::image_processor::Result;
use crate::vectorizer::Point;
use std::fs::File;
use std::io::Write;
use std::path::Path;

/// Flatten a cubic Bézier curve into a polyline within `tolerance` in pixels.
fn flatten_bezier(curve: &BezierCurve, tolerance: f64) -> Vec<Point> {
    let mut points = vec![curve.start.clone()];
    flatten_recursive(
        &curve.start,
        &curve.control1,
        &curve.control2,
        &curve.end,
        tolerance,
        &mut points,
    );
    points.push(curve.end.clone());
    points
}

fn flatten_recursive(
    p0: &Point,
    p1: &Point,
    p2: &Point,
    p3: &Point,
    tolerance: f64,
    points: &mut Vec<Point>,
) {
    // Estimate deviation of control points from the start-end line.
    let dx = p3.x - p0.x;
    let dy = p3.y - p0.y;
    let line_len = (dx * dx + dy * dy).sqrt();
    if line_len < 1e-6 {
        return;
    }
    let d1 = ((p1.y - p0.y) * dx - (p1.x - p0.x) * dy).abs() / line_len;
    let d2 = ((p2.y - p0.y) * dx - (p2.x - p0.x) * dy).abs() / line_len;
    if d1 <= tolerance && d2 <= tolerance {
        return;
    }

    let q0 = midpoint(p0, p1);
    let q1 = midpoint(p1, p2);
    let q2 = midpoint(p2, p3);
    let r0 = midpoint(&q0, &q1);
    let r1 = midpoint(&q1, &q2);
    let m = midpoint(&r0, &r1);

    flatten_recursive(p0, &q0, &r0, &m, tolerance, points);
    points.push(m.clone());
    flatten_recursive(&m, &r1, &q2, p3, tolerance, points);
}

fn midpoint(a: &Point, b: &Point) -> Point {
    Point {
        x: (a.x + b.x) * 0.5,
        y: (a.y + b.y) * 0.5,
    }
}

/// Collect all flattened points for a closed enhanced path.
fn path_to_points(path: &crate::enhanced_vectorizer::EnhancedPath) -> Vec<Point> {
    let tolerance = 0.5;
    let mut points = Vec::new();
    for (i, curve) in path.curves.iter().enumerate() {
        let mut flat = flatten_bezier(curve, tolerance);
        if i == 0 {
            points.append(&mut flat);
        } else {
            // Avoid duplicating the start point shared with the previous curve's end.
            points.append(&mut flat.split_first().map(|(_, rest)| rest.to_vec()).unwrap_or_default());
        }
    }
    // Close the loop if the path is not stroke-only.
    if !path.stroke_only && !points.is_empty() {
        points.push(points[0].clone());
    }
    points
}

/// Write enhanced vector data as a GeoJSON FeatureCollection of polygons.
pub fn write_enhanced_geojson(data: &EnhancedVectorData, output_path: &Path) -> Result<()> {
    let mut file = File::create(output_path)?;
    writeln!(file, "{{")?;
    writeln!(file, "  \"type\": \"FeatureCollection\",")?;
    writeln!(file, "  \"features\": [")?;

    let mut first = true;
    for path in &data.paths {
        let points = path_to_points(path);
        if points.len() < 4 {
            continue;
        }
        let coords: Vec<String> = points
            .iter()
            .map(|p| format!("[{}, {}]", p.x, data.height as f64 - p.y))
            .collect();
        if !first {
            writeln!(file, ",")?;
        }
        first = false;
        let color = format!("#{:02x}{:02x}{:02x}", path.color.0, path.color.1, path.color.2);
        writeln!(file, "    {{")?;
        writeln!(file, "      \"type\": \"Feature\",")?;
        writeln!(
            file,
            "      \"properties\": {{ \"fill\": \"{}\", \"fill-opacity\": {:.3} }}",
            color,
            path.color.3 as f64 / 255.0
        )?;
        writeln!(file, "      \"geometry\": {{")?;
        writeln!(file, "        \"type\": \"Polygon\",")?;
        writeln!(file, "        \"coordinates\": [")?;
        writeln!(file, "          [ {}", coords.join(", "))?;
        writeln!(file, "          ]")?;
        writeln!(file, "        ]")?;
        writeln!(file, "      }}")?;
        writeln!(file, "    }}")?;
    }

    writeln!(file, "\n  ]")?;
    writeln!(file, "}}")?;
    Ok(())
}

/// Write enhanced vector data as a GIMP paths file (SVG path subset).
pub fn write_enhanced_gimppath(data: &EnhancedVectorData, output_path: &Path) -> Result<()> {
    let mut file = File::create(output_path)?;
    writeln!(file, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>")?;
    writeln!(file, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\">", data.width, data.height)?;
    for (idx, path) in data.paths.iter().enumerate() {
        let mut d = String::new();
        let mut first = true;
        for curve in &path.curves {
            if first {
                d.push_str(&format!("M {:.2} {:.2} ", curve.start.x, curve.start.y));
                first = false;
            }
            if crate::bezier_fitter::is_linear_curve(curve) {
                d.push_str(&format!("L {:.2} {:.2} ", curve.end.x, curve.end.y));
            } else {
                d.push_str(&format!(
                    "C {:.2} {:.2}, {:.2} {:.2}, {:.2} {:.2} ",
                    curve.control1.x,
                    curve.control1.y,
                    curve.control2.x,
                    curve.control2.y,
                    curve.end.x,
                    curve.end.y
                ));
            }
        }
        let color = format!("#{:02x}{:02x}{:02x}", path.color.0, path.color.1, path.color.2);
        writeln!(
            file,
            "  <path id=\"path-{}\" d=\"{}\" fill=\"{}\" fill-opacity=\"{:.3}\"/>",
            idx,
            d.trim(),
            color,
            path.color.3 as f64 / 255.0
        )?;
    }
    writeln!(file, "</svg>")?;
    Ok(())
}

/// Write enhanced vector data as a minimal AutoCAD DXF (R12-style ASCII).
pub fn write_enhanced_dxf(data: &EnhancedVectorData, output_path: &Path) -> Result<()> {
    let mut file = File::create(output_path)?;
    writeln!(file, "0")?;
    writeln!(file, "SECTION")?;
    writeln!(file, "2")?;
    writeln!(file, "HEADER")?;
    writeln!(file, "9")?;
    writeln!(file, "$INSUNITS")?;
    writeln!(file, "70")?;
    writeln!(file, "4")?;
    writeln!(file, "0")?;
    writeln!(file, "ENDSEC")?;
    writeln!(file, "0")?;
    writeln!(file, "SECTION")?;
    writeln!(file, "2")?;
    writeln!(file, "ENTITIES")?;

    for path in &data.paths {
        let points = path_to_points(path);
        if points.len() < 2 {
            continue;
        }
        writeln!(file, "0")?;
        writeln!(file, "LWPOLYLINE")?;
        writeln!(file, "8")?;
        writeln!(file, "0")?;
        writeln!(file, "66")?;
        writeln!(file, "1")?;
        writeln!(file, "70")?;
        if path.stroke_only {
            writeln!(file, "0")?;
        } else {
            writeln!(file, "1")?;
        }
        writeln!(file, "62")?;
        // AutoCAD color index: default to 0 (ByBlock). RGB colors are not embedded in this minimal R12 DXF.
        writeln!(file, "0")?;
        for p in &points {
            writeln!(file, "10")?;
            writeln!(file, "{:.2}", p.x)?;
            writeln!(file, "20")?;
            writeln!(file, "{:.2}", data.height as f64 - p.y)?;
        }
    }

    writeln!(file, "0")?;
    writeln!(file, "ENDSEC")?;
    writeln!(file, "0")?;
    writeln!(file, "EOF")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced_vectorizer::{EnhancedPath, EnhancedVectorData};

    #[test]
    fn test_flatten_linear_bezier() {
        let curve = BezierCurve {
            start: Point { x: 0.0, y: 0.0 },
            control1: Point { x: 5.0, y: 0.0 },
            control2: Point { x: 10.0, y: 0.0 },
            end: Point { x: 15.0, y: 0.0 },
        };
        let pts = flatten_bezier(&curve, 0.5);
        assert_eq!(pts.first().unwrap().x, 0.0);
        assert_eq!(pts.last().unwrap().x, 15.0);
    }

    #[test]
    fn test_geojson_writer_creates_feature_collection() {
        let data = EnhancedVectorData {
            width: 10,
            height: 10,
            background_color: (255, 255, 255, 255),
            paths: vec![EnhancedPath {
                curves: vec![BezierCurve {
                    start: Point { x: 0.0, y: 0.0 },
                    control1: Point { x: 0.0, y: 10.0 },
                    control2: Point { x: 10.0, y: 10.0 },
                    end: Point { x: 10.0, y: 0.0 },
                }],
                color: (255, 0, 0, 255),
                area: 100,
                svg_override: None,
                gradient: None,
                stroke_only: false,
            }],
            group_by_color: false,
        };
        let path = std::path::PathBuf::from("/tmp/img2svg_test.geojson");
        write_enhanced_geojson(&data, &path).unwrap();
        let s = std::fs::read_to_string(&path).unwrap();
        assert!(s.contains("FeatureCollection"));
        assert!(s.contains("\"type\": \"Feature\""));
        assert!(s.contains("Polygon"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_gimppath_writer_outputs_svg_path() {
        let data = EnhancedVectorData {
            width: 10,
            height: 10,
            background_color: (255, 255, 255, 255),
            paths: vec![EnhancedPath {
                curves: vec![BezierCurve {
                    start: Point { x: 0.0, y: 0.0 },
                    control1: Point { x: 5.0, y: 0.0 },
                    control2: Point { x: 10.0, y: 5.0 },
                    end: Point { x: 10.0, y: 10.0 },
                }],
                color: (0, 255, 0, 255),
                area: 100,
                svg_override: None,
                gradient: None,
                stroke_only: false,
            }],
            group_by_color: false,
        };
        let path = std::path::PathBuf::from("/tmp/img2svg_test.gimppath");
        write_enhanced_gimppath(&data, &path).unwrap();
        let s = std::fs::read_to_string(&path).unwrap();
        assert!(s.contains("<svg"));
        assert!(s.contains("<path"));
        assert!(s.contains("fill=\"#00ff00\""));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_dxf_writer_has_entities_eof() {
        let data = EnhancedVectorData {
            width: 10,
            height: 10,
            background_color: (255, 255, 255, 255),
            paths: vec![EnhancedPath {
                curves: vec![BezierCurve {
                    start: Point { x: 0.0, y: 0.0 },
                    control1: Point { x: 5.0, y: 0.0 },
                    control2: Point { x: 10.0, y: 5.0 },
                    end: Point { x: 10.0, y: 10.0 },
                }],
                color: (0, 0, 255, 255),
                area: 100,
                svg_override: None,
                gradient: None,
                stroke_only: false,
            }],
            group_by_color: false,
        };
        let path = std::path::PathBuf::from("/tmp/img2svg_test.dxf");
        write_enhanced_dxf(&data, &path).unwrap();
        let s = std::fs::read_to_string(&path).unwrap();
        assert!(s.contains("ENTITIES"));
        assert!(s.contains("LWPOLYLINE"));
        assert!(s.contains("EOF"));
        let _ = std::fs::remove_file(&path);
    }
}
