use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use fontdb::{Database, Family, Query, Source};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use toml::Value;
use unicode_width::UnicodeWidthChar;

pub type GlyphBitmap = Vec<Vec<u8>>;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GlyphStore {
    pub glyphs: Vec<(char, GlyphBitmap, String)>,
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
    bounds_min_x: f32,
    bounds_max_x: f32,
    bounds_min_y: f32,
    bounds_max_y: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedBlock {
    glyphs: Vec<(char, GlyphBitmap, String)>,
}

struct FontContext {
    user_faces: HashMap<String, Arc<FontVec>>,
    fallback_faces: HashMap<String, Arc<FontVec>>,
    font_paths: Vec<String>,
    missing_chars: HashSet<char>,
    font_size: f32,
}

impl FontContext {
    fn new(font_names: &[String], font_size: f32) -> Self {
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
                if let Ok(data) = fs::read(&path) {
                    if let Ok(font) = FontVec::try_from_vec(data) {
                        if is_monospace_font(&font) {
                            log::info!("Loaded font: {}", path);
                            font_paths.push(path.clone());
                            user_faces.insert(path, Arc::new(font));
                        } else {
                            log::info!("Warning: Font '{}' is not monospace, skipping.", font_name);
                        }
                    }
                }
            } else {
                log::info!("Warning: Font '{}' not found.", font_name);
            }
        }

        if user_faces.is_empty() {
            panic!("No usable fonts found. Please install Cascadia Code or specify a valid font with --font.");
        }

        FontContext {
            user_faces,
            fallback_faces: HashMap::new(),
            font_paths,
            missing_chars: HashSet::new(),
            font_size,
        }
    }

    fn get_main_font(&self) -> Arc<FontVec> {
        // Return the first successfully loaded user face
        // We iterate font_paths to preserve priority order
        for path in &self.font_paths {
            if let Some(font) = self.user_faces.get(path) {
                return font.clone();
            }
        }
        // Should be unreachable due to check in new()
        self.user_faces.values().next().unwrap().clone()
    }
}

static DB: Lazy<Mutex<Database>> = Lazy::new(|| {
    let mut db = Database::new();
    db.load_system_fonts();
    Mutex::new(db)
});

fn find_font_path(font_name: &str) -> Option<String> {
    let direct_path = Path::new(font_name);
    if direct_path.is_file() {
        return Some(direct_path.to_string_lossy().to_string());
    }

    let db = DB.lock().unwrap();
    let query = Query {
        families: &[Family::Name(font_name)],
        ..Default::default()
    };
    db.query(&query).and_then(|id| {
        let (src, _) = db.face_source(id)?;
        if let Source::File(path) = src {
            Some(path.to_string_lossy().to_string())
        } else {
            None
        }
    })
}

static FALLBACK_FONT_CACHE: Lazy<Mutex<HashMap<char, Vec<String>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn get_fallback_font_paths(ch: char) -> Vec<String> {
    if let Some(paths) = FALLBACK_FONT_CACHE.lock().unwrap().get(&ch) {
        return paths.clone();
    }

    // Use -s to get a sorted list of all matching fonts.
    let output = Command::new("fc-match")
        .arg("-s")
        .arg("-f")
        .arg("%{file}\n")
        .arg(format!(":charset={:X}", ch as u32))
        .output()
        .ok();

    let mut results = Vec::new();
    if let Some(output) = output {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let path = line.trim().to_string();
                if !path.is_empty() && !results.contains(&path) {
                    results.push(path);
                }
                if results.len() >= 8 {
                    // Increased limit to find more potential matches
                    break;
                }
            }
        }
    }

    FALLBACK_FONT_CACHE
        .lock()
        .unwrap()
        .insert(ch, results.clone());
    results
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum HexOrInt {
    Hex(String),
    Int(u32),
}

impl HexOrInt {
    fn to_u32(&self) -> u32 {
        match self {
            HexOrInt::Hex(s) => {
                let s = s.trim_start_matches("0x");
                u32::from_str_radix(s, 16).unwrap_or(0)
            }
            HexOrInt::Int(i) => *i,
        }
    }
}

// Custom serialization to write as integer (post-processed to hex)
impl Serialize for HexOrInt {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u32(self.to_u32())
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct BlockEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    glyphs: Option<Vec<HexOrInt>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ranges: Option<Vec<[HexOrInt; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    include: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exclude: Option<Vec<HexOrInt>>,
}

fn is_graphical_emoji(ch: char) -> bool {
    let cp = ch as u32;
    // Common emoji ranges
    (cp >= 0x1F300 && cp <= 0x1F64F) || // Misc Symbols and Pictographs, Emoticons
    (cp >= 0x1F680 && cp <= 0x1F6FF) || // Transport and Map
    (cp >= 0x1F700 && cp <= 0x1F77F) || // Alchemical Symbols
    (cp >= 0x1F900 && cp <= 0x1F9FF) || // Misc Symbols and Arrows
    (cp >= 0x1FA70 && cp <= 0x1FAFF) || // Symbols and Pictographs Extended-A
    (cp >= 0x2600 && cp <= 0x26FF) ||   // Misc Symbols (some are emojis)
    (cp >= 0x2700 && cp <= 0x27BF) // Dingbats
}

pub fn is_emoji_render_candidate(ch: char) -> bool {
    if is_graphical_emoji(ch) {
        return true;
    }

    let cp = ch as u32;
    matches!(
        cp,
        0x00A9
            | 0x00AE
            | 0x203C
            | 0x2049
            | 0x2122
            | 0x2139
            | 0x2328
            | 0x23CF
            | 0x24C2
            | 0x25B6
            | 0x25C0
            | 0x3030
            | 0x303D
            | 0x3297
            | 0x3299
    ) || (0x2194..=0x21AA).contains(&cp)
        || (0x231A..=0x231B).contains(&cp)
        || (0x23E9..=0x23F3).contains(&cp)
        || (0x23F8..=0x23FA).contains(&cp)
        || (0x25AA..=0x25AB).contains(&cp)
        || (0x25FB..=0x25FE).contains(&cp)
        || (0x2934..=0x2935).contains(&cp)
        || (0x2B05..=0x2B07).contains(&cp)
        || (0x2B1B..=0x2B1C).contains(&cp)
        || cp == 0x2B50
        || cp == 0x2B55
        || (0x1F000..=0x1FAFF).contains(&cp)
}

pub fn add_range_to_blocks_config(name: &str, start: u32, end: u32) {
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

    let toml_path = toml_path.expect("Failed to find blocks.toml for adding range");
    let toml_str = fs::read_to_string(&toml_path).expect("Failed to read blocks.toml");
    let mut raw_config: toml::map::Map<String, Value> =
        toml::from_str(&toml_str).expect("Invalid TOML");

    let mut entry = toml::map::Map::new();
    entry.insert(
        "ranges".to_string(),
        Value::Array(vec![Value::Array(vec![
            Value::Integer(start as i64),
            Value::Integer(end as i64),
        ])]),
    );

    // Automatically exclude graphical emojis
    let mut exclude = Vec::new();
    for cp in start..=end {
        if let Some(ch) = char::from_u32(cp) {
            if is_graphical_emoji(ch) {
                exclude.push(Value::Integer(cp as i64));
            }
        }
    }
    if !exclude.is_empty() {
        entry.insert("exclude".to_string(), Value::Array(exclude));
    }

    raw_config.insert(name.to_string(), Value::Table(entry));

    let mut new_toml =
        toml::to_string_pretty(&Value::Table(raw_config)).expect("Failed to serialize TOML");

    // Post-process to ensure hex format without quotes where it looks like a Unicode point
    // This is a heuristic: replace large integers or specific ranges with 0xHEX
    // Actually, TOML doesn't distinguish hex/dec in the Value enum.
    // We'll use a regex to replace integers that are likely Unicode points or specifically requested.
    let re = Regex::new(r"([ \[,]| = )(\d{3,9})").unwrap();
    new_toml = re
        .replace_all(&new_toml, |caps: &regex::Captures| {
            let val: i64 = caps[2].parse().unwrap();
            if val > 127 {
                format!("{}0x{:04X}", &caps[1], val)
            } else {
                caps[0].to_string()
            }
        })
        .to_string();

    fs::write(toml_path, new_toml).expect("Failed to write blocks.toml");
}

pub fn remove_range_from_blocks_config(name: &str) {
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

    let toml_path = toml_path.expect("Failed to find blocks.toml for removing range");
    let toml_str = fs::read_to_string(&toml_path).expect("Failed to read blocks.toml");
    let mut raw_config: toml::map::Map<String, Value> =
        toml::from_str(&toml_str).expect("Invalid TOML");

    if raw_config.remove(name).is_some() {
        let mut new_toml =
            toml::to_string_pretty(&Value::Table(raw_config)).expect("Failed to serialize TOML");

        let re = Regex::new(r"([ \[,]| = )(\d{3,9})").unwrap();
        new_toml = re
            .replace_all(&new_toml, |caps: &regex::Captures| {
                let val: i64 = caps[2].parse().unwrap();
                if val > 127 {
                    format!("{}0x{:04X}", &caps[1], val)
                } else {
                    caps[0].to_string()
                }
            })
            .to_string();

        fs::write(toml_path, new_toml).expect("Failed to write blocks.toml");
    }
}

pub fn save_blocks_config(block_name: &str, exclude: &[char]) {
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

    let toml_path = toml_path.expect("Failed to find blocks.toml for saving");
    let toml_str = fs::read_to_string(&toml_path).expect("Failed to read blocks.toml");
    let mut raw_config: toml::map::Map<String, Value> =
        toml::from_str(&toml_str).expect("Invalid TOML");

    // Load full config to know which characters belong to which block
    let (config_groups, _) = load_blocks_config();
    let block_codes = config_groups.get(block_name).expect("Block not found");

    // Filter global exclude list to only those in the current block
    let block_exclude: Vec<Value> = exclude
        .iter()
        .filter(|&ch| block_codes.contains(&(*ch as u32)))
        .map(|&ch| Value::Integer(ch as u32 as i64))
        .collect();

    // Ensure the block entry exists and update its exclude list
    let block_entry = raw_config
        .entry(block_name.to_string())
        .or_insert_with(|| Value::Table(toml::map::Map::new()));

    if let Some(table) = block_entry.as_table_mut() {
        if block_exclude.is_empty() {
            table.remove("exclude");
        } else {
            table.insert("exclude".to_string(), Value::Array(block_exclude));
        }
    }

    let mut new_toml =
        toml::to_string_pretty(&Value::Table(raw_config)).expect("Failed to serialize TOML");

    let re = Regex::new(r"([ \[,]| = )(\d{3,9})").unwrap();
    new_toml = re
        .replace_all(&new_toml, |caps: &regex::Captures| {
            let val: i64 = caps[2].parse().unwrap();
            if val > 127 {
                format!("{}0x{:04X}", &caps[1], val)
            } else {
                caps[0].to_string()
            }
        })
        .to_string();

    fs::write(toml_path, new_toml).expect("Failed to write blocks.toml");
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

    let toml_str = if let Some(path) = toml_path {
        fs::read_to_string(path).expect("Failed to read blocks.toml")
    } else {
        let default_toml = r#"[default]
include = [
    "polygons",
    "eighth.v",
    "eighth.h",
    "space",
    "half",
    "full",
]

[eighth.h]
ranges = [[
    0x2589,
    0x258F,
]]

[eighth.v]
ranges = [[
    0x2581,
    0x2587,
]]

[quarter]
ranges = [[
    0x2596,
    0x259F,
]]

[half]
glyphs = [0x2580]

[full]
glyphs = [0x2588]

[largetype]
ranges = [[
    0x1CE1A,
    0x1CE50,
]]

[sextants]
ranges = [[
    0x1FB00,
    0x1FB3B,
]]

[octants]
ranges = [[
    0x1CD20,
    0x1CDE5,
]]

[polygons]
exclude = [
    0x1FB69,
    0x1FB6B,
    0x1FB6D,
    0x1FB6F,
]
ranges = [[
    0x1FB3C,
    0x1FB6F,
]]

[space]
glyphs = [32]
"#;
        // Attempt to save default toml
        let mut save_path = None;
        if let Ok(config_home) = env::var("XDG_CONFIG_HOME") {
            save_path = Some(Path::new(&config_home).join("img2irc").join("blocks.toml"));
        } else if let Ok(home) = env::var("HOME") {
            save_path = Some(
                Path::new(&home)
                    .join(".config")
                    .join("img2irc")
                    .join("blocks.toml"),
            );
        }

        if let Some(p) = save_path {
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).ok();
            }
            if fs::write(&p, default_toml).is_err() {
                // If config home fails, try current dir
                fs::write("blocks.toml", default_toml).ok();
            }
        } else {
            fs::write("blocks.toml", default_toml).ok();
        }

        default_toml.to_string()
    };

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
                glyphs.extend(g.iter().map(|h| h.to_u32()));
            }
            if let Some(r) = &entry.ranges {
                for range in r {
                    for code in range[0].to_u32()..=range[1].to_u32() {
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
                let exc_set: HashSet<_> = exc.iter().map(|h| h.to_u32()).collect();
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

    (resolved_groups, all_chars)
}

fn render_glyph_from_font(font: &FontVec, ch: char, metrics: &CachedMetrics) -> GlyphBitmap {
    let cell_width = metrics.cell_width.max(1);
    let cell_height = metrics.cell_height.max(1);
    let mut cell = vec![vec![0u8; cell_width]; cell_height];

    let height_unscaled = font.height_unscaled().max(1.0);
    let glyph_box_width = (metrics.bounds_max_x - metrics.bounds_min_x).max(1.0);
    let glyph_box_height = (metrics.bounds_max_y - metrics.bounds_min_y).max(1.0);
    let scale = PxScale {
        x: cell_width as f32 * height_unscaled / glyph_box_width,
        y: cell_height as f32 * height_unscaled / glyph_box_height,
    };
    let scaled_font = font.as_scaled(scale);
    let x_offset = -metrics.bounds_min_x * scaled_font.h_scale_factor();
    let y_offset = metrics.bounds_max_y * scaled_font.v_scale_factor();
    let glyph = font
        .glyph_id(ch)
        .with_scale_and_position(scale, ab_glyph::point(x_offset, y_offset));

    if let Some(outlined) = font.outline_glyph(glyph) {
        let bounds = outlined.px_bounds();
        let base_x = bounds.min.x.floor() as i32;
        let base_y = bounds.min.y.floor() as i32;
        outlined.draw(|x, y, coverage| {
            if coverage > 0.5 {
                let px = base_x + x as i32;
                let py = base_y + y as i32;
                if px >= 0 && px < cell_width as i32 && py >= 0 && py < cell_height as i32 {
                    cell[py as usize][px as usize] = 1;
                }
            }
        });
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

fn measure_reference_advance_unscaled(font: &FontVec) -> f32 {
    let glyph_id = font.glyph_id('X');
    let mut advance = font.h_advance_unscaled(glyph_id);
    if advance == 0.0 {
        let glyph_id = font.glyph_id('0');
        advance = font.h_advance_unscaled(glyph_id);
    }
    if advance == 0.0 {
        advance = font.height_unscaled() * 0.6;
    }
    advance.max(1.0)
}

pub fn is_monospace_font(font: &FontVec) -> bool {
    let scale = PxScale::from(24.0); // Use a fixed scale for checking
    let scaled = font.as_scaled(scale);

    let chars_to_check = ['X', 'i', 'W', 'M', '.', '|'];
    let mut advances: Vec<f32> = Vec::new();
    for ch in &chars_to_check {
        let gid = font.glyph_id(*ch);
        if gid.0 != 0 {
            let adv = scaled.h_advance(gid);
            if adv > 0.0 {
                advances.push(adv);
            }
        }
    }
    if advances.len() < 2 {
        // If we can't find enough ASCII characters to check (e.g. symbol-only font),
        // we assume it is monospaced or at least usable.
        return true;
    }
    // All advances should be the same for monospace
    let first = advances[0];
    // Relaxed threshold to 1.0 to allow for slight variations in "monospaced" fonts
    advances.iter().all(|a| (a - first).abs() < 1.0)
}

fn is_renderable_char(ch: char) -> bool {
    if ch.is_control() {
        return false;
    }
    // Allow width 1 and 2. Some symbols might be reported as 2 but we still want to try rendering them.
    // Also some chars might return None for width but are still renderable.
    let w = ch.width().unwrap_or(1);
    w >= 1 && w <= 2
}

fn resolve_font_for_char(ctx: &mut FontContext, ch: char) -> Option<(Arc<FontVec>, String)> {
    if ctx.missing_chars.contains(&ch) {
        return None;
    }

    for path in &ctx.font_paths {
        if let Some(font) = ctx.user_faces.get(path) {
            if font.glyph_id(ch).0 != 0 {
                let font_name = Path::new(path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
                return Some((font.clone(), font_name));
            }
        }
    }

    let paths = get_fallback_font_paths(ch);
    for path in paths {
        if !ctx.fallback_faces.contains_key(&path) {
            if let Ok(data) = fs::read(&path) {
                match FontVec::try_from_vec(data) {
                    Ok(font) => {
                        if is_monospace_font(&font) {
                            log::info!("Adding fallback font: {}", path);
                            ctx.fallback_faces.insert(path.clone(), Arc::new(font));
                        } else {
                            log::debug!("Fallback font {} is not monospace, skipping", path);
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to load font {}: {:?}", path, e);
                    }
                }
            }
        }
        if let Some(font) = ctx.fallback_faces.get(&path) {
            if font.glyph_id(ch).0 != 0 {
                let font_name = Path::new(&path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
                return Some((font.clone(), font_name));
            }
        }
    }

    log::info!("Could not resolve font for character U+{:04X}", ch as u32);
    ctx.missing_chars.insert(ch);
    None
}

fn metrics_hash_context(font_paths: &[String], font_size: f32, chars: &[char]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    "v7".hash(&mut hasher);
    for p in font_paths {
        p.hash(&mut hasher);
    }
    font_size.to_bits().hash(&mut hasher);
    for ch in chars {
        (*ch as u32).hash(&mut hasher);
    }
    format!("{:x}", hasher.finish())
}

fn block_hash_context(font_paths: &[String], font_size: f32, metrics: &CachedMetrics) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    "v7".hash(&mut hasher);
    for p in font_paths {
        p.hash(&mut hasher);
    }
    font_size.to_bits().hash(&mut hasher);
    metrics.cell_width.hash(&mut hasher);
    metrics.cell_height.hash(&mut hasher);
    metrics.ascender.hash(&mut hasher);
    metrics.descender.hash(&mut hasher);
    metrics.bounds_min_x.to_bits().hash(&mut hasher);
    metrics.bounds_max_x.to_bits().hash(&mut hasher);
    metrics.bounds_min_y.to_bits().hash(&mut hasher);
    metrics.bounds_max_y.to_bits().hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn get_metrics(ctx: &mut FontContext, chars: &[char]) -> CachedMetrics {
    let mut metric_chars = chars.to_vec();
    metric_chars.sort_unstable();
    metric_chars.dedup();

    let hash = metrics_hash_context(&ctx.font_paths, ctx.font_size, &metric_chars);
    let path = cache_dir().join(format!("metrics_{}.bin", hash));

    if path.exists() {
        if let Ok(data) = fs::read(&path) {
            if let Ok(metrics) = bincode::deserialize(&data) {
                return metrics;
            }
        }
    }

    let (
        cell_width,
        cell_height,
        advance,
        line_height,
        mut bounds_min_x,
        mut bounds_max_x,
        mut bounds_min_y,
        mut bounds_max_y,
    ) = {
        let main_font = ctx.get_main_font();
        let scale = PxScale::from(ctx.font_size);
        let scaled_font = main_font.as_scaled(scale);
        let line_height = (scaled_font.height() + scaled_font.line_gap()).max(1.0);
        let cell_height = line_height.ceil() as usize;
        let advance_unscaled = measure_reference_advance_unscaled(&*main_font);
        let advance = (ctx.font_size * advance_unscaled / main_font.height_unscaled()).max(1.0);
        let cell_width = advance.round().max(1.0) as usize;

        (
            cell_width,
            cell_height,
            advance,
            line_height,
            0.0f32,
            advance_unscaled,
            main_font.descent_unscaled(),
            main_font.ascent_unscaled(),
        )
    };

    for &ch in &metric_chars {
        if !is_renderable_char(ch) {
            continue;
        }
        if let Some((font, _)) = resolve_font_for_char(ctx, ch) {
            if let Some(outline) = font.outline(font.glyph_id(ch)) {
                let glyph_min_x = outline.bounds.min.x.min(outline.bounds.max.x);
                let glyph_max_x = outline.bounds.min.x.max(outline.bounds.max.x);
                let glyph_min_y = outline.bounds.min.y.min(outline.bounds.max.y);
                let glyph_max_y = outline.bounds.min.y.max(outline.bounds.max.y);
                bounds_min_x = bounds_min_x.min(glyph_min_x);
                bounds_max_x = bounds_max_x.max(glyph_max_x);
                bounds_min_y = bounds_min_y.min(glyph_min_y);
                bounds_max_y = bounds_max_y.max(glyph_max_y);
            }
        }
    }

    let bounds_width = (bounds_max_x - bounds_min_x).max(1.0);
    let bounds_height = (bounds_max_y - bounds_min_y).max(1.0);
    let ascender = bounds_max_y.ceil() as i32;
    let descender = bounds_min_y.floor() as i32;

    log::info!(
        "Metrics (glyph box): Ascender={}, Descender={}, Height={}, Width={}, BoundsW={:.2}, BoundsH={:.2}",
        ascender,
        descender,
        cell_height,
        cell_width,
        bounds_width,
        bounds_height
    );

    let metrics = CachedMetrics {
        cell_width,
        cell_height,
        ascender,
        descender,
        float_width: advance,
        float_height: line_height,
        bounds_min_x,
        bounds_max_x,
        bounds_min_y,
        bounds_max_y,
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
) -> Vec<(char, GlyphBitmap, String)> {
    let hash = block_hash_context(&ctx.font_paths, ctx.font_size, metrics);
    let path = cache_dir().join(format!("block_{}_{}.bin", block_name, hash));

    if path.exists() {
        if let Ok(data) = fs::read(&path) {
            if let Ok(block) = bincode::deserialize::<CachedBlock>(&data) {
                return block.glyphs;
            }
        }
    }

    log::info!("Rendering block '{}' ({} glyphs)", block_name, chars.len());
    let mut glyphs = Vec::new();

    for &ch in chars {
        let mut found = false;

        if ctx.missing_chars.contains(&ch) {
            log::info!("Skipping missing char: {}", ch);
            continue;
        }

        if !is_renderable_char(ch) {
            log::info!("Skipping non-renderable char: {}", ch);
            continue;
        }

        // 1. Try resolving font (handles user fonts and fallbacks)
        if let Some((font, font_name)) = resolve_font_for_char(ctx, ch) {
            let bitmap = render_glyph_from_font(&*font, ch, metrics);
            glyphs.push((ch, bitmap, font_name));
            found = true;
        }

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
    font_size: f32,
    load_all: bool,
    abort_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> GlyphStore {
    let mut ctx = FontContext::new(fonts, font_size);
    let (config_groups, _) = load_blocks_config();

    // Determine which blocks to load
    let mut blocks_to_load = HashSet::new();
    let mut blocks_to_select = HashSet::new();

    // Always select specified blocks or prefixes
    for name in blocks {
        let name_lc = name.to_lowercase();
        let mut matched = false;
        for key in config_groups.keys() {
            let key_lc = key.to_lowercase();
            if key_lc == name_lc || key_lc.starts_with(&format!("{}.", name_lc)) {
                blocks_to_select.insert(key.clone());
                blocks_to_load.insert(key.clone());
                matched = true;
            }
        }
        if !matched {
            blocks_to_select.insert(name_lc.clone());
            blocks_to_load.insert(name_lc);
        }
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

    // ASCII 32-126 are always loaded for text overlay rendering, but only
    // selected for glyph matching when no specific blocks were requested.
    let mut overlay_ascii_chars: Vec<char> = Vec::new();
    for ch in 32..=126u32 {
        if let Some(c) = char::from_u32(ch) {
            overlay_ascii_chars.push(c);
        }
    }

    let mut metric_chars = Vec::new();
    for block_name in &sorted_blocks {
        if let Some(codes) = config_groups.get(*block_name) {
            for &code in codes {
                if let Some(ch) = char::from_u32(code) {
                    if !is_emoji_render_candidate(ch) {
                        metric_chars.push(ch);
                    }
                }
            }
        }
    }
    metric_chars.extend(explicit_chars.iter().copied());
    metric_chars.extend(overlay_ascii_chars.iter().copied());

    let metrics = get_metrics(&mut ctx, &metric_chars);
    let target_cell_width = metrics.cell_width.max(1);
    let target_cell_height = metrics.cell_height.max(1);

    let mut all_glyphs = BTreeMap::new();
    let mut selected_chars = HashSet::new();
    let mut groups_out = HashMap::new();

    // Load requested blocks
    for block_name in sorted_blocks {
        if let Some(flag) = &abort_flag {
            if flag.load(std::sync::atomic::Ordering::Relaxed) {
                return GlyphStore {
                    glyphs: Vec::new(),
                    groups: HashMap::new(),
                    metrics: (0, 0),
                    float_metrics: (0.0, 0.0),
                    selected: HashSet::new(),
                };
            }
        }

        if let Some(codes) = config_groups.get(block_name) {
            let mut block_chars = Vec::new();
            for &code in codes {
                if let Some(ch) = char::from_u32(code) {
                    if !is_emoji_render_candidate(ch) {
                        block_chars.push(ch);
                    }
                }
            }
            if !block_chars.is_empty() {
                let glyphs = get_block_glyphs(&mut ctx, block_name, &block_chars, &metrics);
                groups_out.insert(block_name.clone(), block_chars.clone());

                let is_selected = blocks_to_select.contains(block_name);

                for (ch, bitmap, font_name) in glyphs {
                    all_glyphs.insert(ch, (bitmap, font_name));
                    if is_selected {
                        selected_chars.insert(ch);
                    }
                }
            }
        } else {
            log::info!("Warning: Block '{}' not defined in blocks.toml", block_name);
        }
    }

    // Load user-specified explicit includes (--include, --include-range) and select them
    if !explicit_chars.is_empty() {
        let glyphs = get_block_glyphs(&mut ctx, "explicit_includes", &explicit_chars, &metrics);
        groups_out.insert("explicit_includes".to_string(), explicit_chars.clone());
        for (ch, bitmap, font_name) in glyphs {
            all_glyphs.insert(ch, (bitmap, font_name));
            selected_chars.insert(ch);
        }
    }

    // Load ASCII overlay characters (always available for text overlays).
    // Only select them for glyph matching when no specific blocks were requested.
    let is_filtering = !blocks.is_empty() || include_chars.is_some() || !include_ranges.is_empty();
    if !overlay_ascii_chars.is_empty() {
        let glyphs = get_block_glyphs(&mut ctx, "overlay_ascii", &overlay_ascii_chars, &metrics);
        groups_out.insert("overlay_ascii".to_string(), overlay_ascii_chars.clone());
        for (ch, bitmap, font_name) in glyphs {
            all_glyphs.insert(ch, (bitmap, font_name));
            if !is_filtering {
                selected_chars.insert(ch);
            }
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
    selected_chars.retain(|&ch| !is_emoji_render_candidate(ch));

    GlyphStore {
        glyphs: all_glyphs
            .into_iter()
            .map(|(ch, (bitmap, font_name))| (ch, bitmap, font_name))
            .collect(),
        groups: groups_out,
        selected: selected_chars,
        metrics: (target_cell_width, target_cell_height),
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
    let glyph_map: HashMap<char, (&GlyphBitmap, &String)> =
        store.glyphs.iter().map(|(c, b, f)| (*c, (b, f))).collect();
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
            if let Some((bitmap, font_name)) = glyph_map.get(&ch) {
                out.push_str(&format!(
                    "Character: {} ({}) [Font: {}]\n",
                    ch,
                    ch.escape_unicode(),
                    font_name
                ));
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
