use fontconfig::Fontconfig;
use freetype::{face::LoadFlag, Library};
use once_cell::sync::OnceCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;
use toml::Value;

pub type GlyphBitmap = Vec<Vec<u8>>;

#[derive(Clone)]
pub struct GlyphStore {
    pub glyphs: Vec<(char, GlyphBitmap)>,
    pub groups: HashMap<String, Vec<char>>,
    pub selected: HashSet<char>,
    pub metrics: (usize, usize),
}

struct GlyphBase {
    glyphs: Vec<(char, GlyphBitmap)>,
    groups: HashMap<String, Vec<char>>,
    metrics: (usize, usize),
}

static GLYPH_BASE: OnceCell<GlyphBase> = OnceCell::new();

fn find_font_path(font_name: &str) -> Option<String> {
    let fc = Fontconfig::new().ok()?;
    fc.find("monospace", Some(font_name))
        .map(|f| f.path.display().to_string())
}

fn load_blocks() -> (HashMap<String, Vec<std::ops::RangeInclusive<u32>>>, HashSet<char>) {
    let toml_str = fs::read_to_string("blocks.toml").expect("Failed to read blocks.toml");
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
                    cell[(cell_height - 1 - y as usize)][x as usize] = 1;
                }
            }
        }
    }

    cell
}

fn build_base() -> GlyphBase {
    let (group_ranges, mut chars) = load_blocks();

    const FONT_NAME: &str = "Cascadia Code";
    let font_path = find_font_path(FONT_NAME)
        .unwrap_or_else(|| panic!("Could not find font '{}' via fontconfig", FONT_NAME));

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

    GlyphBase {
        glyphs,
        groups,
        metrics: (cell_height, cell_width),
    }
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
