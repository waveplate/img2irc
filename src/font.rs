use fontconfig::Fontconfig;
use freetype::{face::LoadFlag, Library};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use toml::Value;

pub type GlyphBitmap = Vec<Vec<u8>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GlyphStore {
    pub glyphs: Vec<(char, GlyphBitmap)>,
    pub groups: HashMap<String, Vec<char>>,
    pub selected: HashSet<char>,
    pub metrics: (usize, usize),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedMetrics {
    cell_width: usize,
    cell_height: usize,
    ascender: i32,
    descender: i32,
    y_offset: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedBlock {
    glyphs: Vec<(char, GlyphBitmap)>,
}

struct FontContext {
    library: Library,
    user_faces: HashMap<String, freetype::Face>,
    fallback_faces: HashMap<String, freetype::Face>,
    font_paths: Vec<String>,
    missing_chars: HashSet<char>,
    font_size: u32,
}

impl FontContext {
    fn new(font_names: &[String], font_size: u32) -> Self {
        let library = Library::init().expect("Failed to initialise FreeType");
        let mut font_paths = Vec::new();
        let mut user_faces = HashMap::new();

        // Always ensure we have at least one font (Cascadia Code default) if none provided
        let fonts_to_load = if font_names.is_empty() {
            vec!["Cascadia Code".to_string()]
        } else {
            font_names.to_vec()
        };

        for font_name in &fonts_to_load {
            if let Some(path) = find_font_path(font_name) {
                eprintln!("Loaded font: {}", path);
                font_paths.push(path.clone());
                if let Ok(face) = library.new_face(Path::new(&path), 0) {
                     let _ = face.set_pixel_sizes(0, font_size);
                     user_faces.insert(path, face);
                }
            } else {
                eprintln!("Warning: Font '{}' not found.", font_name);
            }
        }

        if user_faces.is_empty() {
            panic!("No usable fonts found. Please install Cascadia Code or specify a valid font with --font.");
        }

        FontContext {
            library,
            user_faces,
            fallback_faces: HashMap::new(),
            font_paths,
            missing_chars: HashSet::new(),
            font_size,
        }
    }

    fn get_main_face(&self) -> &freetype::Face {
        // Return the first successfully loaded user face
        // We iterate font_paths to preserve priority order
        for path in &self.font_paths {
            if let Some(face) = self.user_faces.get(path) {
                return face;
            }
        }
        // Should be unreachable due to check in new()
        self.user_faces.values().next().unwrap()
    }
}

fn find_font_path(font_name: &str) -> Option<String> {
    let fc = Fontconfig::new()?;
    fc.find(font_name, None)
        .map(|f| f.path.display().to_string())
}

fn get_fallback_font_path(ch: char) -> Option<String> {
    let output = Command::new("fc-match")
        .arg("-f")
        .arg("%{file}")
        .arg(format!(":charset={:x}", ch as u32))
        .output()
        .ok()?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Some(path);
        }
    }
    None
}

fn load_blocks_config() -> (HashMap<String, Vec<std::ops::RangeInclusive<u32>>>, HashSet<char>) {
    let mut toml_path = None;
    if let Ok(config_dir) = env::var("IMG2IRC_CONFIG_DIR") {
        let path = Path::new(&config_dir).join("blocks.toml");
        if path.exists() { toml_path = Some(path); }
    }
    if toml_path.is_none() {
        let path = PathBuf::from("blocks.toml");
        if path.exists() { toml_path = Some(path); }
    }
    if toml_path.is_none() {
        let config_home = env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
            env::var("HOME").unwrap_or_else(|_| ".".to_string()) + "/.config"
        });
        let path = Path::new(&config_home).join("img2irc").join("blocks.toml");
        if path.exists() { toml_path = Some(path); }
    }
    if toml_path.is_none() {
        if let Ok(home) = env::var("HOME") {
            let path = Path::new(&home).join(".config").join("img2irc").join("blocks.toml");
            if path.exists() { toml_path = Some(path); }
        }
    }

    let toml_path = toml_path.expect("Failed to find blocks.toml");
    let toml_str = fs::read_to_string(toml_path).expect("Failed to read blocks.toml");
    let value: Value = toml::from_str(&toml_str).expect("Invalid TOML");

    let mut groups: HashMap<String, Vec<std::ops::RangeInclusive<u32>>> = HashMap::new();
    let mut chars = HashSet::new();

    if let Some(table) = value.as_table() {
        for (name, entry) in table {
            let mut ranges = Vec::new();
            let values: Vec<&Value> = match entry {
                Value::Table(_) => vec![entry],
                Value::Array(arr) => arr.iter().collect(),
                _ => Vec::new(),
            };

            for item in values {
                if let Some(tbl) = item.as_table() {
                    if let (Some(start), Some(end)) = (
                        tbl.get("start").and_then(Value::as_integer),
                        tbl.get("end").and_then(Value::as_integer),
                    ) {
                        let range = start as u32..=end as u32;
                        for code in range.clone() {
                            if let Some(ch) = char::from_u32(code) {
                                chars.insert(ch);
                            }
                        }
                        ranges.push(range);
                    }
                }
            }
            if !ranges.is_empty() {
                groups.insert(name.to_lowercase(), ranges);
            }
        }
    }
    chars.insert(' ');
    (groups, chars)
}

fn render_glyph_from_face(face: &mut freetype::Face, ch: char, cell_width: usize, cell_height: usize, global_ascender: i32, y_offset: i32) -> GlyphBitmap {
    face.load_char(ch as usize, LoadFlag::RENDER | LoadFlag::TARGET_MONO)
        .unwrap_or_else(|_| panic!("Failed to load glyph {}", ch));

    let glyph = face.glyph();
    let bitmap = glyph.bitmap();
    let rows = bitmap.rows() as usize;
    let width = bitmap.width() as usize;
    let pitch = bitmap.pitch().abs() as usize;

    let mut cell = vec![vec![0u8; cell_width]; cell_height];
    let glyph_ascender = glyph.bitmap_top();
    let left = glyph.bitmap_left();

    for row in 0..rows {
        let start = row * pitch;
        let end = start + pitch;
        let row_data = &bitmap.buffer()[start..end];

        for bit_idx in 0..width {
            let byte_index = bit_idx / 8;
            let bit = 7 - (bit_idx % 8);
            if row_data[byte_index] & (1 << bit) != 0 {
                let x = left + bit_idx as i32;
                let y = global_ascender - glyph_ascender + row as i32 + y_offset;
                
                if x >= 0 && x < cell_width as i32 && y >= 0 && y < cell_height as i32 {
                    cell[y as usize][x as usize] = 1;
                }
            }
        }
    }
    cell
}

pub fn clear_cache() {
    let dir = cache_dir();
    if dir.exists() {
        let _ = fs::remove_dir_all(dir);
    }
}

fn cache_dir() -> PathBuf {
    if let Ok(cache_home) = env::var("XDG_CACHE_HOME") {
        PathBuf::from(cache_home).join("img2irc")
    } else {
        let mut path = PathBuf::from(env::var("HOME").unwrap_or_else(|_| ".".to_string()));
        path.push(".cache");
        path.push("img2irc");
        path
    }
}

fn hash_context(font_paths: &[String], y_offset: i32, font_size: u32) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    "v3".hash(&mut hasher); // Invalidate old cache
    for p in font_paths {
        p.hash(&mut hasher);
    }
    y_offset.hash(&mut hasher);
    font_size.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn get_metrics(ctx: &mut FontContext, y_offset: i32) -> CachedMetrics {
    let hash = hash_context(&ctx.font_paths, y_offset, ctx.font_size);
    let path = cache_dir().join(format!("metrics_{}.bin", hash));

    if path.exists() {
        if let Ok(data) = fs::read(&path) {
            if let Ok(metrics) = bincode::deserialize(&data) {
                return metrics;
            }
        }
    }

    let main_face = ctx.get_main_face();
    let metrics = main_face.size_metrics().expect("No size metrics available");
    
    let cell_width = ((metrics.max_advance >> 6) as i32).max(1) as usize;
    let mut ascender = (metrics.ascender >> 6) as i32;
    let descender = (metrics.descender >> 6) as i32;
    let mut cell_height = (ascender + descender.abs()).max(1) as usize;

    // Calibrate against Full Block (U+2588) to ensure gapless rendering
    let full_block_char = '█';
    for path in &ctx.font_paths {
        if let Some(face) = ctx.user_faces.get(path) {
             if face.get_char_index(full_block_char as usize) != 0 {
                 if face.load_char(full_block_char as usize, LoadFlag::RENDER | LoadFlag::TARGET_MONO).is_ok() {
                    let glyph = face.glyph();
                    let bitmap = glyph.bitmap();
                    if bitmap.rows() > 0 {
                        let new_h = bitmap.rows() as usize;
                        if new_h != cell_height {
                             eprintln!("Adjusting cell height from {} to {} (based on Full Block U+2588) to prevent vertical gaps.", cell_height, new_h);
                        }
                        cell_height = new_h;
                        ascender = glyph.bitmap_top();
                        break; 
                    }
                 }
             }
        }
    }

    eprintln!("Metrics: Ascender={}, Descender={}, Height={}, Width={}, Y-Offset={}", ascender, descender, cell_height, cell_width, y_offset);

    let metrics = CachedMetrics {
        cell_width,
        cell_height,
        ascender,
        descender,
        y_offset,
    };

    if let Some(parent) = path.parent() { fs::create_dir_all(parent).ok(); }
    if let Ok(data) = bincode::serialize(&metrics) {
        fs::write(path, data).ok();
    }

    metrics
}

fn get_block_glyphs(ctx: &mut FontContext, block_name: &str, chars: &[char], metrics: &CachedMetrics) -> Vec<(char, GlyphBitmap)> {
    let hash = hash_context(&ctx.font_paths, metrics.y_offset, ctx.font_size);
    let path = cache_dir().join(format!("block_{}_{}.bin", block_name, hash));

    if path.exists() {
        if let Ok(data) = fs::read(&path) {
            if let Ok(block) = bincode::deserialize::<CachedBlock>(&data) {
                // eprintln!("Loaded block '{}' from cache", block_name);
                return block.glyphs;
            }
        }
    }

    eprintln!("Rendering block '{}' ({} glyphs)", block_name, chars.len());
    let mut glyphs = Vec::new();

    for &ch in chars {
        let mut found = false;
        
        // 0. Check if we already know this character is missing from the system
        if ctx.missing_chars.contains(&ch) {
            continue;
        }

        // 1. User fonts
        for path in &ctx.font_paths {
            if let Some(face) = ctx.user_faces.get_mut(path) {
                 if face.get_char_index(ch as usize) != 0 {
                    let bitmap = render_glyph_from_face(face, ch, metrics.cell_width, metrics.cell_height, metrics.ascender, metrics.y_offset);
                    glyphs.push((ch, bitmap));
                    found = true;
                    break;
                }
            }
        }
        if found { continue; }

        // 2. Fallback
        if let Some(path) = get_fallback_font_path(ch) {
             if !ctx.fallback_faces.contains_key(&path) {
                 if let Ok(face) = ctx.library.new_face(Path::new(&path), 0) {
                     let _ = face.set_pixel_sizes(0, ctx.font_size);
                     ctx.fallback_faces.insert(path.clone(), face);
                 }
             }
             if let Some(face) = ctx.fallback_faces.get_mut(&path) {
                 if face.get_char_index(ch as usize) != 0 {
                      let bitmap = render_glyph_from_face(face, ch, metrics.cell_width, metrics.cell_height, metrics.ascender, metrics.y_offset);
                      glyphs.push((ch, bitmap));
                      found = true;
                 }
             }
        }

        // If not found in any font, it is skipped and marked as missing
        if !found {
            ctx.missing_chars.insert(ch);
        }
    }

    let block = CachedBlock { glyphs: glyphs.clone() };
    if let Some(parent) = path.parent() { fs::create_dir_all(parent).ok(); }
    if let Ok(data) = bincode::serialize(&block) {
        fs::write(path, data).ok();
    }

    glyphs
}

fn parse_hex_range(s: &str) -> Option<std::ops::RangeInclusive<u32>> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 2 { return None; }
    let start = u32::from_str_radix(parts[0], 16).ok()?;
    let end = u32::from_str_radix(parts[1], 16).ok()?;
    Some(start..=end)
}

pub fn glyph_store(
    blocks: &[String],
    exclude_ranges: &[String],
    exclude_chars: &[char],
    include_ranges: &[String],
    include_chars: Option<&String>,
    fonts: &[String],
    y_offset: i32,
    font_size: u32
) -> GlyphStore {
    let mut ctx = FontContext::new(fonts, font_size);
    let metrics = get_metrics(&mut ctx, y_offset);
    let (config_groups, _) = load_blocks_config();

    let mut all_glyphs = BTreeMap::new();
    let mut selected_chars = HashSet::new();

    // Determine which blocks to load
    let mut blocks_to_load = HashSet::new();
    if blocks.is_empty() {
        // If no blocks specified, load ALL known blocks from config
        blocks_to_load.extend(config_groups.keys().cloned());
    } else {
        for name in blocks {
            blocks_to_load.insert(name.to_lowercase());
        }
    }

    // Iterating sorted blocks for consistent logging/order
    let mut sorted_blocks: Vec<_> = blocks_to_load.iter().collect();
    sorted_blocks.sort();

    let mut groups_out = HashMap::new();

    // Load requested blocks
    for block_name in sorted_blocks {
        if let Some(ranges) = config_groups.get(block_name) {
            let mut block_chars = Vec::new();
            for range in ranges {
                for code in range.clone() {
                    if let Some(ch) = char::from_u32(code) {
                        block_chars.push(ch);
                    }
                }
            }
            if !block_chars.is_empty() {
                let glyphs = get_block_glyphs(&mut ctx, block_name, &block_chars, &metrics);
                groups_out.insert(block_name.clone(), block_chars.clone());
                for (ch, bitmap) in glyphs {
                    all_glyphs.insert(ch, bitmap);
                    selected_chars.insert(ch);
                }
            }
        } else {
             eprintln!("Warning: Block '{}' not defined in blocks.toml", block_name);
        }
    }

    // Process explicit inclusions (treat as a virtual "included" block)
    let mut explicit_chars = Vec::new();
    if let Some(inc_str) = include_chars {
        for ch in inc_str.chars() {
            explicit_chars.push(ch);
        }
    }
    for range_str in include_ranges {
        if let Some(range) = parse_hex_range(range_str) {
            for code in range {
                if let Some(ch) = char::from_u32(code) {
                    explicit_chars.push(ch);
                }
            }
        } else {
            eprintln!("Warning: Invalid include range format '{}'. Expected hex range like '2600-26FF'.", range_str);
        }
    }

    if !explicit_chars.is_empty() {
        let glyphs = get_block_glyphs(&mut ctx, "explicit_includes", &explicit_chars, &metrics);
        groups_out.insert("explicit_includes".to_string(), explicit_chars.clone());
        for (ch, bitmap) in glyphs {
            all_glyphs.insert(ch, bitmap);
            selected_chars.insert(ch);
        }
    }

    // Process exclusions
    for range_str in exclude_ranges {
        if let Some(range) = parse_hex_range(range_str) {
            selected_chars.retain(|&ch| !range.contains(&(ch as u32)));
        } else {
            eprintln!("Warning: Invalid exclude range format '{}'. Expected hex range like '2600-26FF'.", range_str);
        }
    }
    for &ch in exclude_chars {
        selected_chars.remove(&ch);
    }

    // Ensure space is present and selected
    if !all_glyphs.contains_key(&' ') {
         let glyphs = get_block_glyphs(&mut ctx, "space", &[' '], &metrics);
         if let Some((ch, bitmap)) = glyphs.first() {
             all_glyphs.insert(*ch, bitmap.clone());
             selected_chars.insert(*ch);
         }
    } else {
        selected_chars.insert(' ');
    }

    GlyphStore {
        glyphs: all_glyphs.into_iter().collect(),
        groups: groups_out,
        selected: selected_chars,
        metrics: (metrics.cell_height, metrics.cell_width),
    }
}

pub fn list_ranges() -> String {
    let (groups, _) = load_blocks_config();
    let mut sorted_groups: Vec<_> = groups.iter().collect();
    sorted_groups.sort_by_key(|(k, _)| *k);
    
    let mut out = String::new();
    out.push_str(&format!("{:<20} | {:<20} | {}\n", "Block Name", "Range", "Count"));
    out.push_str(&format!("{:-<20}-+-{:-<20}-+-{:-<5}\n", "", "", ""));

    for (name, ranges) in sorted_groups {
        let mut count = 0;
        let mut range_strs = Vec::new();
        for range in ranges {
            count += range.end() - range.start() + 1;
            range_strs.push(format!("{:X}-{:X}", range.start(), range.end()));
        }
        out.push_str(&format!("{:<20} | {:<20} | {}\n", name, range_strs.join(", "), count));
    }
    out
}

pub fn print_ranges() -> String {
    let (groups, _) = load_blocks_config();
    let mut sorted_groups: Vec<_> = groups.iter().collect();
    sorted_groups.sort_by_key(|(k, _)| *k);

    let mut out = String::new();
    for (name, ranges) in sorted_groups {
        out.push_str(&format!("{}:\n", name));
        let mut count = 0;
        for range in ranges {
            for code in range.clone() {
                if let Some(ch) = char::from_u32(code) {
                    out.push_str(&format!("{} ", ch));
                    count += 1;
                    if count >= 32 {
                        out.push('\n');
                        count = 0;
                    }
                }
            }
        }
        if count > 0 {
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

pub fn print_blocks_config() -> String {
    let (groups, _) = load_blocks_config();
    let mut sorted_groups: Vec<_> = groups.iter().collect();
    sorted_groups.sort_by_key(|(k, _)| *k);
    let mut out = String::new();
    for (name, ranges) in sorted_groups {
        out.push_str(&format!("{}:\n", name));
        // Printing ranges is better than printing all chars for config listing
        for range in ranges {
             out.push_str(&format!("{:X}-{:X} ", range.start(), range.end()));
        }
        out.push_str("\n");
    }
    out
}

pub fn print_block_bitmaps(store: &GlyphStore) -> String {
    let glyph_map: HashMap<char, &GlyphBitmap> = store.glyphs.iter().map(|(c, b)| (*c, b)).collect();
    let mut sorted_groups: Vec<_> = store.groups.keys().collect();
    sorted_groups.sort();

    let mut out = String::new();
    for name in sorted_groups {
        let chars = &store.groups[name];
        let selected_chars: Vec<char> = chars.iter().copied().filter(|c| store.selected.contains(c)).collect();

        if selected_chars.is_empty() { continue; }

        out.push_str(&format!("Block: {}\n", name));
        for &ch in &selected_chars {
            if let Some(bitmap) = glyph_map.get(&ch) {
                out.push_str(&format!("Character: {} ({})\n", ch, ch.escape_unicode()));
                if let Some(first_row) = bitmap.first() {
                    let width = first_row.len();
                    out.push('┌');
                    for _ in 0..width { out.push_str("──"); }
                    out.push_str("┐\n");

                    for row in *bitmap {
                        out.push('│');
                        for &pixel in row {
                            if pixel > 0 { out.push_str("██"); } else { out.push_str("  "); }
                        }
                        out.push_str("│\n");
                    }
                    out.push('└');
                    for _ in 0..width { out.push_str("──"); }
                    out.push_str("┘\n");
                }
                out.push('\n');
            }
        }
        out.push_str(&format!("{}\n", "-".repeat(40)));
    }
    out
}
