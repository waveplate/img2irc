pub type GlyphBitmap = Vec<Vec<u8>>;
fn resize_bitmap(base: &GlyphBitmap, sw: usize, sh: usize, tw: usize, th: usize) -> GlyphBitmap {
    let mut out = vec![vec![0u8; tw]; th];
    if sw == 0 || sh == 0 || tw == 0 || th == 0 {
        return out;
    }
    for ty in 0..th {
        for tx in 0..tw {
            let sy_start = ty * sh / th;
            let sy_end = ((ty + 1) * sh / th).max(sy_start + 1);
            let sx_start = tx * sw / tw;
            let sx_end = ((tx + 1) * sw / tw).max(sx_start + 1);
            let mut sum = 0;
            let mut count = 0;
            for sy in sy_start..sy_end.min(sh) {
                for sx in sx_start..sx_end.min(sw) {
                    sum += base[sy][sx] as u32;
                    count += 1;
                }
            }
            if count > 0 && sum * 3 >= count { // test with *3 >= count for slight boldness
                out[ty][tx] = 1;
            }
        }
    }
    out
}

fn main() {
    let sw = 120;
    let sh = 240;
    let mut base = vec![vec![0u8; sw]; sh];
    for y in 0..sh {
        for x in 0..sw {
            base[y][x] = ((x + y) % 2) as u8;
        }
    }
    let tw = 12;
    let th = 24;
    let start = std::time::Instant::now();
    for _ in 0..3000 {
        let _resized = resize_bitmap(&base, sw, sh, tw, th);
    }
    println!("Elapsed: {:?}", start.elapsed());
}
