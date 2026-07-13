use img2irc_rs::contour_score::score_rgb;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: score_contours IMAGE [CELL_SCALE]")?;
    let cell_scale = args
        .next()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(8);
    let image = image::open(&path)?.to_rgb8();
    let pixels = image.pixels().map(|pixel| pixel.0).collect::<Vec<_>>();
    let score = score_rgb(
        &pixels,
        image.width() as usize,
        image.height() as usize,
        cell_scale,
    );
    println!(
        "total={:.6} bending={:.6} endpoints={:.6} junctions={:.6} fragments={:.6} boundary_length={}",
        score.total,
        score.bending,
        score.endpoints,
        score.junctions,
        score.fragments,
        score.boundary_length
    );
    Ok(())
}
