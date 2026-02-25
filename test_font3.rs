use freetype::Library;
fn main() {
    let lib = Library::init().unwrap();
    let face = lib.new_face("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", 0).unwrap();
    face.set_pixel_sizes(0, 24).unwrap();
    let metrics = face.size_metrics().unwrap();
    println!("max_advance: {}", metrics.max_advance >> 6);
    
    face.load_char('X' as usize, freetype::face::LoadFlag::DEFAULT).unwrap();
    println!("X advance: {}", face.glyph().metrics().horiAdvance >> 6);
    
    println!("ascender: {}", metrics.ascender >> 6);
    println!("descender: {}", metrics.descender >> 6);
    println!("height: {}", metrics.height >> 6);
}
