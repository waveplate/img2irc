use fontconfig::Fontconfig;
use freetype::{face::LoadFlag, Library};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
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
struct GlyphBase {
    glyphs: Vec<(char, GlyphBitmap)>,
    groups: HashMap<String, Vec<char>>,
    metrics: (usize, usize),
}

static GLYPH_BASE: OnceCell<GlyphBase> = OnceCell::new();

fn find_font_path(font_name: &str) -> Option<String> {
    let fc = Fontconfig::new()?;
    fc.find("monospace", Some(font_name))
        .map(|f| f.path.display().to_string())
}

fn load_blocks() -> (HashMap<String, Vec<std::ops::RangeInclusive<u32>>>, HashSet<char>) {
    let mut toml_path = None;

    // Check IMG2IRC_CONFIG_DIR first (highest precedence)
    if let Ok(config_dir) = env::var("IMG2IRC_CONFIG_DIR") {
        let path = Path::new(&config_dir).join("blocks.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }

    // If not found, check current directory
    if toml_path.is_none() {
        let path = PathBuf::from("blocks.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }

    // If still not found, check XDG_CONFIG_HOME or ~/.config
    if toml_path.is_none() {
        let config_home = env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
            env::var("HOME").unwrap_or_else(|_| ".".to_string()) + "/.config"
        });
        let path = Path::new(&config_home).join("img2irc").join("blocks.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }

    // If still not found, check ~/.config/img2irc/
    if toml_path.is_none() {
        if let Ok(home) = env::var("HOME") {
            let path = Path::new(&home).join(".config").join("img2irc").join("blocks.toml");
            if path.exists() {
                toml_path = Some(path);
            }
        }
    }

    let toml_path = toml_path.expect("Failed to find blocks.toml in any expected location");
    let toml_str = fs::read_to_string(toml_path).expect("Failed to read blocks.toml");
    let value: Value = toml::from_str(&toml_str).expect("Invalid TOML in blocks.toml");

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

fn render_glyph(face: &mut freetype::Face, ch: char, cell_width: usize, cell_height: usize) -> GlyphBitmap {
    face.load_char(ch as usize, LoadFlag::RENDER | LoadFlag::TARGET_MONO)
        .unwrap_or_else(|_| panic!("Failed to load glyph {}", ch));

    let glyph = face.glyph();
    let bitmap = glyph.bitmap();
    let rows = bitmap.rows() as usize;
    let width = bitmap.width() as usize;
    let pitch = bitmap.pitch().abs() as usize;

    let mut cell = vec![vec![0u8; cell_width]; cell_height];
    let ascender = glyph.bitmap_top();
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
                let y = ascender - row as i32 - 1;
                if x >= 0 && x < cell_width as i32 && y >= 0 && y < cell_height as i32 {
                    cell[cell_height - 1 - y as usize][x as usize] = 1;
                }
            }
        }
    }

    cell
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

fn cache_path(font_name: &str) -> PathBuf {
    cache_dir().join(format!("{}.toml", font_name))
}

fn load_cached_glyphs(font_name: &str) -> Option<GlyphBase> {
    let path = cache_path(font_name);
    if path.exists() {
        fs::read_to_string(&path)
            .ok()
            .and_then(|content| toml::from_str(&content).ok())
    } else {
        None
    }
}

fn save_cached_glyphs(font_name: &str, base: &GlyphBase) {
    let path = cache_path(font_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    if let Ok(content) = toml::to_string(base) {
        fs::write(path, content).ok();
    }
}

fn get_font_name() -> String {
    env::var("IMG2IRC_FONT").unwrap_or_else(|_| "Cascadia Code".to_string())
}

fn build_base() -> GlyphBase {
    let font_name = get_font_name();
    if let Some(cached) = load_cached_glyphs(&font_name) {
        return cached;
    }

    let (group_ranges, mut chars) = load_blocks();

    let font_path = find_font_path(&font_name)
        .unwrap_or_else(|| panic!("Could not find font '{}' via fontconfig", font_name));

    let library = Library::init().expect("Failed to initialise FreeType");
    let mut face = library
        .new_face(Path::new(&font_path), 0)
        .expect("Failed to load font face");

    let size_px = 24;
    face.set_pixel_sizes(0, size_px)
        .expect("Failed to set pixel size");

    let metrics = face.size_metrics().expect("No size metrics available");
    let cell_width = ((metrics.max_advance >> 6) as i32).max(1) as usize;
    let ascender = (metrics.ascender >> 6) as i32;
    let descender = (metrics.descender >> 6) as i32;
    let cell_height = (ascender - descender).max(1) as usize;

    let mut groups: HashMap<String, Vec<char>> = HashMap::new();
    for (name, ranges) in group_ranges {
        let mut members = Vec::new();
        for range in ranges {
            for code in range {
                if let Some(ch) = char::from_u32(code) {
                    members.push(ch);
                    chars.insert(ch);
                }
            }
        }
        if !members.is_empty() {
            groups.insert(name, members);
        }
    }

    let mut glyphs_map: BTreeMap<char, GlyphBitmap> = BTreeMap::new();
    for ch in chars.iter().copied() {
        let bitmap = render_glyph(&mut face, ch, cell_width, cell_height);
        glyphs_map.insert(ch, bitmap);
    }

    let glyphs = glyphs_map.into_iter().collect();

    let base = GlyphBase {
        glyphs,
        groups,
        metrics: (cell_height, cell_width),
    };
    save_cached_glyphs(&font_name, &base);
    base
}

fn base_store() -> &'static GlyphBase {
    GLYPH_BASE.get_or_init(build_base)
}

pub fn glyph_store(blocks: &[String]) -> GlyphStore {
    let base = base_store();

    let mut selected = HashSet::new();
    if blocks.is_empty() {
        selected.extend(base.glyphs.iter().map(|(ch, _)| *ch));
    } else {
        for name in blocks {
            if let Some(chars) = base.groups.get(&name.to_lowercase()) {
                selected.extend(chars.iter().copied());
            }
        }
        if selected.is_empty() {
            selected.extend(base.glyphs.iter().map(|(ch, _)| *ch));
        }
    }
    selected.insert(' ');

    GlyphStore {
        glyphs: base.glyphs.clone(),
        groups: base.groups.clone(),
        selected,
        metrics: base.metrics,
    }
}

pub fn print_blocks_config() {
    let base = base_store();
    let mut sorted_groups: Vec<_> = base.groups.iter().collect();
    sorted_groups.sort_by_key(|(k, _)| *k);
    for (name, chars) in sorted_groups {
        println!("{}:", name);
        for &ch in chars {
            print!("{} ", ch);
        }
        println!();
    }
}


