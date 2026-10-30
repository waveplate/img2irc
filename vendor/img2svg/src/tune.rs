//! Interactive parameter tuning: generate HTML comparison grid.

use crate::enhanced_vectorizer::{vectorize_enhanced, EnhancedOptions};
use crate::image_processor::{load_image, Result};
use std::fs::File;
use std::io::Write;
use std::path::Path;

const GRID: &[(usize, u8, bool)] = &[
    (8, 2, false),
    (8, 5, false),
    (12, 3, true),
    (16, 5, false),
    (16, 3, true),
    (32, 5, false),
];

/// Generate an HTML page comparing parameter combinations for live review.
pub fn generate_tune_html(input: &Path, output: &Path) -> Result<()> {
    let image = load_image(input)?;
    // `writeln!` mutably borrows the file handle internally.
    #[allow(unused_mut)]
    let mut file = File::create(output)?;

    writeln!(
        &file,
        r#"<!DOCTYPE html>
<html lang="en"><head>
<meta charset="UTF-8">
<title>img2svg tune — {}</title>
<style>
  body {{ font-family: system-ui, sans-serif; background: #111; color: #eee; padding: 20px; }}
  h1 {{ text-align: center; }}
  .grid {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 16px; }}
  .card {{ background: #222; border-radius: 8px; overflow: hidden; border: 1px solid #333; }}
  .card h3 {{ margin: 0; padding: 10px; font-size: 0.85em; background: #1a1a2e; }}
  .card object {{ width: 100%; height: 260px; background: #fff; }}
  code {{ background: #333; padding: 2px 6px; border-radius: 4px; }}
</style></head><body>
<h1>img2svg parameter tune</h1>
<p>Input: <code>{}</code> — adjust CLI flags based on previews below.</p>
<div class="grid">"#,
        input.display(),
        input.display()
    )?;

    let out_dir = output.parent().unwrap_or(Path::new("."));
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();

    for &(colors, smooth, preprocess) in GRID {
        let opts = EnhancedOptions {
            num_colors: colors,
            smooth_window: smooth as usize,
            preprocess,
            ..Default::default()
        };
        let data = vectorize_enhanced(&image, &opts)?;
        let svg_name = format!("{}_tune_c{}_s{}_{}.svg", stem, colors, smooth, if preprocess { "p" } else { "n" });
        let svg_path = out_dir.join(&svg_name);
        crate::enhanced_vectorizer::write_enhanced_svg(&data, &svg_path)?;

        let cmd = format!(
            "img2svg -i {} -o out.svg -c {} -s {}{}",
            input.display(),
            colors,
            smooth,
            if preprocess { " --preprocess" } else { "" }
        );
        writeln!(
            &file,
            r#"<div class="card"><h3>{} paths · {} colors · smooth {} · preprocess {}</h3>
<object data="{}" type="image/svg+xml"></object>
<p><code>{}</code></p></div>"#,
            data.paths.len(),
            colors,
            smooth,
            if preprocess { "on" } else { "off" },
            svg_name,
            cmd
        )?;
    }

    writeln!(&file, "</div></body></html>")?;
    Ok(())
}
