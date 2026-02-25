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
    pub float_metrics: (f32, f32),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedMetrics {
    cell_width: usize,
    cell_height: usize,
    ascender: i32,
    descender: i32,
    float_width: f32,
    float_height: f32,
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
                log::info!("Loaded font: {}", path);
                font_paths.push(path.clone());
                if let Ok(face) = library.new_face(Path::new(&path), 0) {
                    let _ = face.set_pixel_sizes(0, font_size);
                    user_faces.insert(path, face);
                }
            } else {
                log::info!("Warning: Font '{}' not found.", font_name);
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

#[derive(Debug, Deserialize)]
struct BlockEntry {
    glyphs: Option<Vec<u32>>,
    ranges: Option<Vec<[u32; 2]>>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<u32>>,
}

pub fn load_blocks_config() -> (HashMap<String, Vec<u32>>, HashSet<char>) {
    let mut toml_path = None;
    if let Ok(config_dir) = env::var("IMG2IRC_CONFIG_DIR") {
        let path = Path::new(&config_dir).join("blocks.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }
    if toml_path.is_none() {
        let path = PathBuf::from("blocks.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }
    if toml_path.is_none() {
        let config_home = env::var("XDG_CONFIG_HOME")
            .unwrap_or_else(|_| env::var("HOME").unwrap_or_else(|_| ".".to_string()) + "/.config");
        let path = Path::new(&config_home).join("img2irc").join("blocks.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }
    if toml_path.is_none() {
        if let Ok(home) = env::var("HOME") {
            let path = Path::new(&home)
                .join(".config")
                .join("img2irc")
                .join("blocks.toml");
            if path.exists() {
                toml_path = Some(path);
            }
        }
    }

    let toml_path = toml_path.expect("Failed to find blocks.toml");
    let toml_str = fs::read_to_string(toml_path).expect("Failed to read blocks.toml");
    let raw_config: BTreeMap<String, Value> = toml::from_str(&toml_str).expect("Invalid TOML");

    let mut blocks: HashMap<String, BlockEntry> = HashMap::new();

    // Flatten hierarchical keys like [legacy.base.sextants] into "legacy.base.sextants"
    fn flatten_toml(prefix: &str, v: &Value, map: &mut HashMap<String, BlockEntry>) {
        if let Some(table) = v.as_table() {
            // Check if this table is actually a BlockEntry (has glyphs, ranges, or include)
            if table.contains_key("glyphs")
                || table.contains_key("ranges")
                || table.contains_key("include")
                || table.contains_key("exclude")
            {
                if let Ok(entry) = BlockEntry::deserialize(v.clone()) {
                    map.insert(prefix.trim_start_matches('.').to_string(), entry);
                    return;
                }
            }
            for (k, val) in table {
                let new_prefix = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{}.{}", prefix, k)
                };
                flatten_toml(&new_prefix, val, map);
            }
        }
    }

    for (k, v) in raw_config {
        flatten_toml(&k, &v, &mut blocks);
    }

    let mut resolved_groups: HashMap<String, Vec<u32>> = HashMap::new();
    let mut all_chars = HashSet::new();

    fn resolve_block(
        name: &str,
        blocks: &HashMap<String, BlockEntry>,
        resolved: &mut HashMap<String, Vec<u32>>,
        visited: &mut HashSet<String>,
    ) -> Vec<u32> {
        if let Some(res) = resolved.get(name) {
            return res.clone();
        }
        if visited.contains(name) {
            return Vec::new(); // Cycle detected
        }
        visited.insert(name.to_string());

        let mut glyphs = Vec::new();
        if let Some(entry) = blocks.get(name) {
            if let Some(g) = &entry.glyphs {
                glyphs.extend(g);
            }
            if let Some(r) = &entry.ranges {
                for range in r {
                    for code in range[0]..=range[1] {
                        glyphs.push(code);
                    }
                }
            }
            if let Some(inc) = &entry.include {
                for other in inc {
                    glyphs.extend(resolve_block(other, blocks, resolved, visited));
                }
            }
            if let Some(exc) = &entry.exclude {
                let exc_set: HashSet<_> = exc.iter().collect();
                glyphs.retain(|g| !exc_set.contains(g));
            }
        }

        glyphs.sort();
        glyphs.dedup();
        resolved.insert(name.to_string(), glyphs.clone());
        glyphs
    }

    for name in blocks.keys() {
        let mut visited = HashSet::new();
        let glyphs = resolve_block(name, &blocks, &mut resolved_groups, &mut visited);
        for &code in &glyphs {
            if let Some(ch) = char::from_u32(code) {
                all_chars.insert(ch);
            }
        }
    }

    all_chars.insert(' ');
    (resolved_groups, all_chars)
}

fn render_glyph_from_face(
    face: &mut freetype::Face,
    ch: char,
    cell_width: usize,
    cell_height: usize,
    global_ascender: i32,
) -> GlyphBitmap {
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
                let y = global_ascender - glyph_ascender + row as i32;

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

fn hash_context(font_paths: &[String], font_size: u32) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    "v3".hash(&mut hasher); // Invalidate old cache
    for p in font_paths {
        p.hash(&mut hasher);
    }
    font_size.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn get_metrics(ctx: &mut FontContext) -> CachedMetrics {
    let hash = hash_context(&ctx.font_paths, ctx.font_size);
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

    // max_advance is often bloated by wide characters (like ligatures or CJK).
    // Measure a standard character like 'X' or '0' to get the true monospace cell width.
    let mut standard_advance = metrics.max_advance;
    if let Ok(_) = main_face.load_char('X' as usize, freetype::face::LoadFlag::DEFAULT) {
        standard_advance = main_face.glyph().metrics().horiAdvance;
    } else if let Ok(_) = main_face.load_char('0' as usize, freetype::face::LoadFlag::DEFAULT) {
        standard_advance = main_face.glyph().metrics().horiAdvance;
    }

    let cell_width = ((standard_advance >> 6) as i32).max(1) as usize;
    let ascender = (metrics.ascender >> 6) as i32;
    let descender = (metrics.descender >> 6) as i32;
    let cell_height = ((metrics.height >> 6) as i32).max(1) as usize;

    /*
    // Calibrate against Full Block (U+2588) to ensure gapless rendering
    let full_block_char = '█';
    for path in &ctx.font_paths {
        if let Some(face) = ctx.user_faces.get(path) {
            if face.get_char_index(full_block_char as usize) != 0 {
                if face
                    .load_char(
                        full_block_char as usize,
                        LoadFlag::RENDER | LoadFlag::TARGET_MONO,
                    )
                    .is_ok()
                {
                    let glyph = face.glyph();
                    let bitmap = glyph.bitmap();
                    if bitmap.rows() > 0 {
                        let new_h = bitmap.rows() as usize;
                        if new_h != cell_height {
                            log::info!("Adjusting cell height from {} to {} (based on Full Block U+2588) to prevent vertical gaps.", cell_height, new_h);
                        }
                        cell_height = new_h;
                        ascender = glyph.bitmap_top();
                        break;
                    }
                }
            }
        }
    }
    */
    
    let float_width = standard_advance as f32 / 64.0;
    let float_height = metrics.height as f32 / 64.0;

    log::info!(
        "Metrics: Ascender={}, Descender={}, Height={}, Width={}, FloatW={}, FloatH={}",
        ascender,
        descender,
        cell_height,
        cell_width,
        float_width,
        float_height
    );

    let metrics = CachedMetrics {
        cell_width,
        cell_height,
        ascender,
        descender,
        float_width,
        float_height,
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    if let Ok(data) = bincode::serialize(&metrics) {
        fs::write(path, data).ok();
    }

    metrics
}

fn get_block_glyphs(
    ctx: &mut FontContext,
    block_name: &str,
    chars: &[char],
    metrics: &CachedMetrics,
) -> Vec<(char, GlyphBitmap)> {
    let hash = hash_context(&ctx.font_paths, ctx.font_size);
    let path = cache_dir().join(format!("block_{}_{}.bin", block_name, hash));

    if path.exists() {
        if let Ok(data) = fs::read(&path) {
            if let Ok(block) = bincode::deserialize::<CachedBlock>(&data) {
                // log::info!("Loaded block '{}' from cache", block_name);
                return block.glyphs;
            }
        }
    }

    log::info!("Rendering block '{}' ({} glyphs)", block_name, chars.len());
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
                    let bitmap = render_glyph_from_face(
                        face,
                        ch,
                        metrics.cell_width,
                        metrics.cell_height,
                        metrics.ascender,
                    );
                    glyphs.push((ch, bitmap));
                    found = true;
                    break;
                }
            }
        }
        if found {
            continue;
        }

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
                    let bitmap = render_glyph_from_face(
                        face,
                        ch,
                        metrics.cell_width,
                        metrics.cell_height,
                        metrics.ascender,
                    );
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

    let block = CachedBlock {
        glyphs: glyphs.clone(),
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    if let Ok(data) = bincode::serialize(&block) {
        fs::write(path, data).ok();
    }

    glyphs
}

fn parse_hex_range(s: &str) -> Option<std::ops::RangeInclusive<u32>> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 2 {
        return None;
    }
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
    font_size: u32,
    load_all: bool,
) -> GlyphStore {
    let mut ctx = FontContext::new(fonts, font_size);
    let metrics = get_metrics(&mut ctx);
    let (config_groups, _) = load_blocks_config();

    let mut all_glyphs = BTreeMap::new();
    let mut selected_chars = HashSet::new();

    // Determine which blocks to load
    let mut blocks_to_load = HashSet::new();
    let mut blocks_to_select = HashSet::new();

    // Always select specified blocks
    for name in blocks {
        blocks_to_select.insert(name.to_lowercase());
        blocks_to_load.insert(name.to_lowercase());
    }

    if blocks.is_empty() && !load_all {
        // If no blocks specified and not loading all, load ALL known blocks (legacy behavior?)
        // Wait, if blocks is empty, args.blocks is empty.
        // Old behavior: "If no blocks specified, load ALL known blocks from config"
        blocks_to_load.extend(config_groups.keys().cloned());
        blocks_to_select.extend(config_groups.keys().cloned());
    } else if load_all {
        // Load all, but only select specified
        blocks_to_load.extend(config_groups.keys().cloned());
    }

    // Iterating sorted blocks for consistent logging/order
    let mut sorted_blocks: Vec<_> = blocks_to_load.iter().collect();
    sorted_blocks.sort();

    let mut groups_out = HashMap::new();

    // Load requested blocks
    for block_name in sorted_blocks {
        if let Some(codes) = config_groups.get(block_name) {
            let mut block_chars = Vec::new();
            for &code in codes {
                if let Some(ch) = char::from_u32(code) {
                    block_chars.push(ch);
                }
            }
            if !block_chars.is_empty() {
                let glyphs = get_block_glyphs(&mut ctx, block_name, &block_chars, &metrics);
                groups_out.insert(block_name.clone(), block_chars.clone());

                let is_selected = blocks_to_select.contains(block_name);

                for (ch, bitmap) in glyphs {
                    all_glyphs.insert(ch, bitmap);
                    if is_selected {
                        selected_chars.insert(ch);
                    }
                }
            }
        } else {
            log::info!("Warning: Block '{}' not defined in blocks.toml", block_name);
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
            log::info!(
                "Warning: Invalid include range format '{}'. Expected hex range like '2600-26FF'.",
                range_str
            );
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
            log::info!(
                "Warning: Invalid exclude range format '{}'. Expected hex range like '2600-26FF'.",
                range_str
            );
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
        metrics: (metrics.cell_width, metrics.cell_height),
        float_metrics: (metrics.float_width, metrics.float_height),
    }
}

pub fn list_ranges() -> String {
    let (groups, _) = load_blocks_config();
    let mut sorted_groups: Vec<_> = groups.iter().collect();
    sorted_groups.sort_by_key(|(k, _)| *k);

    let mut out = String::new();
    out.push_str(&format!("{:<30} | {}\n", "Block Name", "Count"));
    out.push_str(&format!("{:-<30}-+-{:-<5}\n", "", ""));

    for (name, codes) in sorted_groups {
        out.push_str(&format!("{:<30} | {}\n", name, codes.len()));
    }
    out
}

pub fn print_ranges() -> String {
    let (groups, _) = load_blocks_config();
    let mut sorted_groups: Vec<_> = groups.iter().collect();
    sorted_groups.sort_by_key(|(k, _)| *k);

    let mut out = String::new();
    for (name, codes) in sorted_groups {
        out.push_str(&format!("{}:\n", name));
        let mut count = 0;
        for &code in codes {
            if let Some(ch) = char::from_u32(code) {
                out.push_str(&format!("{} ", ch));
                count += 1;
                if count >= 32 {
                    out.push('\n');
                    count = 0;
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
    for (name, codes) in sorted_groups {
        out.push_str(&format!("{}: {} glyphs\n", name, codes.len()));
    }
    out
}

pub fn print_block_bitmaps(store: &GlyphStore) -> String {
    let glyph_map: HashMap<char, &GlyphBitmap> =
        store.glyphs.iter().map(|(c, b)| (*c, b)).collect();
    let mut sorted_groups: Vec<_> = store.groups.keys().collect();
    sorted_groups.sort();

    let mut out = String::new();
    for name in sorted_groups {
        let chars = &store.groups[name];
        let selected_chars: Vec<char> = chars
            .iter()
            .copied()
            .filter(|c| store.selected.contains(c))
            .collect();

        if selected_chars.is_empty() {
            continue;
        }

        out.push_str(&format!("Block: {}\n", name));
        for &ch in &selected_chars {
            if let Some(bitmap) = glyph_map.get(&ch) {
                out.push_str(&format!("Character: {} ({})\n", ch, ch.escape_unicode()));
                if let Some(first_row) = bitmap.first() {
                    let width = first_row.len();
                    out.push('┌');
                    for _ in 0..width {
                        out.push_str("──");
                    }
                    out.push_str("┐\n");

                    for row in *bitmap {
                        out.push('│');
                        for &pixel in row {
                            if pixel > 0 {
                                out.push_str("██");
                            } else {
                                out.push_str("  ");
                            }
                        }
                        out.push_str("│\n");
                    }
                    out.push('└');
                    for _ in 0..width {
                        out.push_str("──");
                    }
                    out.push_str("┘\n");
                }
                out.push('\n');
            }
        }
        out.push_str(&format!("{}\n", "-".repeat(40)));
    }
    out
}
