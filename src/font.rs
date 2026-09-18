#[cfg(not(target_arch = "wasm32"))]
use fontdb::{Database, Family, Query, Source};
#[cfg(not(target_arch = "wasm32"))]
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::process::Command;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex;
use toml::Value;
use unicode_width::UnicodeWidthChar;

use swash::scale::{Render, ScaleContext, Source as SwashSource};
use swash::FontRef;

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
    font_size: f32,
    main_ascent_px: f32,
    bounds_min_x: f32,
    bounds_max_x: f32,
    bounds_min_y: f32,
    bounds_max_y: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedBlock {
    glyphs: Vec<(char, GlyphBitmap, String)>,
}

pub struct SystemFont {
    pub bytes: Arc<[u8]>,
    pub face_index: usize,
    pub source_path: String,
}

impl SystemFont {
    fn new(bytes: Arc<[u8]>, face_index: usize, source_path: String) -> Option<Self> {
        FontRef::from_index(&bytes, face_index)?;
        Some(Self {
            bytes,
            face_index,
            source_path,
        })
    }

    fn font_ref(&self) -> FontRef<'_> {
        FontRef::from_index(&self.bytes, self.face_index).expect("validated font face")
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct BundledFont {
    name: &'static str,
    path: &'static str,
    data: &'static [u8],
    face: std::sync::OnceLock<Arc<SystemFont>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl BundledFont {
    fn face(&self) -> Arc<SystemFont> {
        Arc::clone(self.face.get_or_init(|| {
            Arc::new(
                SystemFont::new(Arc::from(self.data), 0, self.path.to_string())
                    .expect("valid bundled font"),
            )
        }))
    }
}

#[cfg(not(target_arch = "wasm32"))]
static BUNDLED_FONTS: [BundledFont; 4] = [
    BundledFont {
        name: "Cascadia Code",
        path: "bundled/CascadiaCode-Regular.ttf",
        data: include_bytes!("../static/CascadiaCode-Regular.ttf"),
        face: std::sync::OnceLock::new(),
    },
    BundledFont {
        name: "Iosevka Fixed",
        path: "bundled/IosevkaFixed-Regular.ttf",
        data: include_bytes!("../static/fonts/IosevkaFixed-Regular.ttf"),
        face: std::sync::OnceLock::new(),
    },
    BundledFont {
        name: "Unifont",
        path: "bundled/unifont-17.0.05.otf",
        data: include_bytes!("../static/fonts/unifont-17.0.05.otf"),
        face: std::sync::OnceLock::new(),
    },
    BundledFont {
        name: "Unifont Upper",
        path: "bundled/unifont_upper-17.0.05.otf",
        data: include_bytes!("../static/fonts/unifont_upper-17.0.05.otf"),
        face: std::sync::OnceLock::new(),
    },
];

#[cfg(not(target_arch = "wasm32"))]
pub fn bundled_font_names() -> impl Iterator<Item = &'static str> {
    BUNDLED_FONTS.iter().map(|font| font.name)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn bundled_font_licenses() -> &'static str {
    concat!(
        "Cascadia Code\n",
        include_str!("../static/fonts/LICENSE-Cascadia.md"),
        "\nIosevka Fixed\n",
        include_str!("../static/fonts/LICENSE-Iosevka.md"),
        "\nGNU Unifont and Unifont Upper\n",
        "Copyright © 1998-2026 Roman Czyborra, Paul Hardy, Qianqian Fang, Andrew Miller, Johnnie Weaver, David Corbett, Ælla Chiana Moskopp, Rebecca Bettencourt, Ho-Seok Ee, et al.\n",
        include_str!("../static/fonts/LICENSE-Unifont-OFL-1.1.txt"),
    )
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Serialize, Deserialize)]
struct FontLocation {
    path: String,
    face_index: usize,
}

struct FontContext {
    user_faces: HashMap<String, Arc<SystemFont>>,
    fallback_faces: HashMap<String, Vec<Arc<SystemFont>>>,
    font_paths: Vec<String>,
    missing_chars: HashSet<char>,
    font_size: f32,
}

impl FontContext {
    #[cfg(not(target_arch = "wasm32"))]
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
            if let Some(font) = BUNDLED_FONTS
                .iter()
                .find(|font| font.name.eq_ignore_ascii_case(font_name))
            {
                let face = font.face();
                let key = face.source_path.clone();
                font_paths.push(key.clone());
                user_faces.insert(key, face);
                continue;
            }
            if let Some(location) = find_font_location(font_name) {
                if let Ok(data) = fs::read(&location.path) {
                    let bytes = Arc::<[u8]>::from(data);
                    if let Some(sys_font) =
                        SystemFont::new(bytes, location.face_index, location.path.clone())
                    {
                        if is_monospace_font(&sys_font) {
                            let key = format!("{}#{}", location.path, location.face_index);
                            log::info!(
                                "Loaded font: {} (face {})",
                                location.path,
                                location.face_index
                            );
                            font_paths.push(key.clone());
                            user_faces.insert(key, Arc::new(sys_font));
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
            let face = BUNDLED_FONTS[0].face();
            let key = face.source_path.clone();
            font_paths.push(key.clone());
            user_faces.insert(key, face);
        }

        FontContext {
            user_faces,
            fallback_faces: HashMap::new(),
            font_paths,
            missing_chars: HashSet::new(),
            font_size,
        }
    }

    fn from_bytes(data: &[u8], font_size: f32) -> Result<Self, String> {
        let bytes = Arc::<[u8]>::from(data.to_vec());
        let mut selected = None;
        let mut face_index = 0usize;

        while FontRef::from_index(&bytes, face_index).is_some() {
            if let Some(font) = SystemFont::new(
                Arc::clone(&bytes),
                face_index,
                format!("browser-font#{face_index}"),
            ) {
                if is_monospace_font(&font) {
                    selected = Some((face_index, Arc::new(font)));
                    break;
                }
            }
            face_index += 1;
        }

        let (face_index, font) = selected.ok_or_else(|| {
            "font data does not contain a usable monospace OpenType/TrueType face".to_string()
        })?;
        let key = format!("memory-font#{face_index}");
        let mut user_faces = HashMap::new();
        user_faces.insert(key.clone(), font);

        Ok(Self {
            user_faces,
            fallback_faces: HashMap::new(),
            font_paths: vec![key],
            missing_chars: HashSet::new(),
            font_size,
        })
    }

    fn get_main_font(&self) -> Arc<SystemFont> {
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

pub fn validate_monospace_font_bytes(data: &[u8]) -> Result<(), String> {
    FontContext::from_bytes(data, 16.0).map(|_| ())
}

#[cfg(not(target_arch = "wasm32"))]
fn get_font_cache_path() -> Option<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    Some(Path::new(&home).join(".img2irc_font_cache.json"))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_cached_font_location(font_name: &str) -> Option<FontLocation> {
    let path = get_font_cache_path()?;
    if !path.is_file() {
        return None;
    }
    let content = fs::read_to_string(&path).ok()?;
    let cache: HashMap<String, FontLocation> = serde_json::from_str(&content).ok()?;
    let location = cache.get(font_name)?;
    if Path::new(&location.path).is_file() {
        Some(location.clone())
    } else {
        None
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_cached_font_location(font_name: &str, location: &FontLocation) {
    if let Some(path) = get_font_cache_path() {
        let mut cache = if path.is_file() {
            if let Ok(content) = fs::read_to_string(&path) {
                serde_json::from_str(&content).unwrap_or_default()
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };
        cache.insert(font_name.to_string(), location.clone());
        if let Ok(content) = serde_json::to_string_pretty(&cache) {
            let _ = fs::write(&path, content);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
static DB: Lazy<Mutex<Database>> = Lazy::new(|| {
    if atty::is(atty::Stream::Stderr) {
        eprintln!("Scanning system fonts (this may take a moment)...");
    }
    let mut db = Database::new();
    db.load_system_fonts();
    Mutex::new(db)
});

#[cfg(not(target_arch = "wasm32"))]
fn find_font_location(font_name: &str) -> Option<FontLocation> {
    let direct_path = Path::new(font_name);
    if direct_path.is_file() {
        return Some(FontLocation {
            path: direct_path.to_string_lossy().to_string(),
            face_index: 0,
        });
    }

    if let Some(location) = read_cached_font_location(font_name) {
        return Some(location);
    }

    let db = DB.lock().unwrap();
    let query = Query {
        families: &[Family::Name(font_name)],
        ..Default::default()
    };
    let resolved = db.query(&query).and_then(|id| {
        let (src, face_index) = db.face_source(id)?;
        if let Source::File(path) = src {
            Some(FontLocation {
                path: path.to_string_lossy().to_string(),
                face_index: face_index as usize,
            })
        } else {
            None
        }
    });

    if let Some(ref location) = resolved {
        write_cached_font_location(font_name, location);
    }
    resolved
}

#[cfg(not(target_arch = "wasm32"))]
static FALLBACK_FONT_CACHE: Lazy<Mutex<HashMap<char, Vec<String>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(target_arch = "wasm32")]
fn get_fallback_font_paths(_ch: char) -> Vec<String> {
    Vec::new()
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum HexOrInt {
    Hex(String),
    Int(u32),
}

#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
impl Serialize for HexOrInt {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u32(self.to_u32())
    }
}

#[cfg(not(target_arch = "wasm32"))]
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
    let is_misc_symbol = (0x2600..=0x26FF).contains(&cp) || (0x2700..=0x27BF).contains(&cp);
    // Miscellaneous Symbols and Dingbats contain many ordinary text glyphs
    // (for example U+266A ♪). Only treat their terminal-wide members as
    // graphical emoji; the explicit emoji list below handles known exceptions.
    (cp >= 0x1F300 && cp <= 0x1F64F) || // Misc Symbols and Pictographs, Emoticons
    (cp >= 0x1F680 && cp <= 0x1F6FF) || // Transport and Map
    (cp >= 0x1F700 && cp <= 0x1F77F) || // Alchemical Symbols
    (cp >= 0x1F900 && cp <= 0x1F9FF) || // Misc Symbols and Arrows
    (cp >= 0x1FA70 && cp <= 0x1FAFF) || // Symbols and Pictographs Extended-A
    (is_misc_symbol && ch.width().unwrap_or(1) > 1)
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

fn block_table_mut<'a>(
    root: &'a mut toml::map::Map<String, Value>,
    path: &str,
    create: bool,
) -> Option<&'a mut toml::map::Map<String, Value>> {
    // Preserve compatibility with quoted keys that literally contain dots.
    if root.contains_key(path) {
        return root.get_mut(path)?.as_table_mut();
    }

    let segments = path
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return None;
    }

    let mut table = root;
    for segment in segments {
        if !table.contains_key(segment) {
            if !create {
                return None;
            }
            table.insert(segment.to_string(), Value::Table(toml::map::Map::new()));
        }
        table = table.get_mut(segment)?.as_table_mut()?;
    }
    Some(table)
}

fn remove_block_path(root: &mut toml::map::Map<String, Value>, path: &str) -> bool {
    if root.remove(path).is_some() {
        return true;
    }

    fn remove_nested(table: &mut toml::map::Map<String, Value>, segments: &[&str]) -> bool {
        let Some((head, tail)) = segments.split_first() else {
            return false;
        };
        if tail.is_empty() {
            return table.remove(*head).is_some();
        }

        let (removed, child_empty) = {
            let Some(child) = table.get_mut(*head).and_then(Value::as_table_mut) else {
                return false;
            };
            let removed = remove_nested(child, tail);
            (removed, removed && child.is_empty())
        };
        if child_empty {
            table.remove(*head);
        }
        removed
    }

    let segments = path
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    remove_nested(root, &segments)
}

pub fn add_range_to_blocks_config(name: &str, start: u32, end: u32) {
    let mut toml_path = None;
    if let Ok(config_dir) = env::var("IMG2IRC_CONFIG_DIR") {
        let path = Path::new(&config_dir).join("glyphs.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }
    if toml_path.is_none() {
        let path = PathBuf::from("glyphs.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }
    if toml_path.is_none() {
        let config_home = env::var("XDG_CONFIG_HOME")
            .unwrap_or_else(|_| env::var("HOME").unwrap_or_else(|_| ".".to_string()) + "/.config");
        let path = Path::new(&config_home).join("img2irc").join("glyphs.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }
    if toml_path.is_none() {
        if let Ok(home) = env::var("HOME") {
            let path = Path::new(&home)
                .join(".config")
                .join("img2irc")
                .join("glyphs.toml");
            if path.exists() {
                toml_path = Some(path);
            }
        }
    }

    let toml_path = toml_path.expect("Failed to find glyphs.toml for adding range");
    let toml_str = fs::read_to_string(&toml_path).expect("Failed to read glyphs.toml");
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

    let block = block_table_mut(&mut raw_config, name, true)
        .expect("Invalid empty or conflicting glyph-set path");
    *block = entry;

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

    fs::write(toml_path, new_toml).expect("Failed to write glyphs.toml");
}

pub fn remove_range_from_blocks_config(name: &str) {
    let mut toml_path = None;
    if let Ok(config_dir) = env::var("IMG2IRC_CONFIG_DIR") {
        let path = Path::new(&config_dir).join("glyphs.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }
    if toml_path.is_none() {
        let path = PathBuf::from("glyphs.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }
    if toml_path.is_none() {
        let config_home = env::var("XDG_CONFIG_HOME")
            .unwrap_or_else(|_| env::var("HOME").unwrap_or_else(|_| ".".to_string()) + "/.config");
        let path = Path::new(&config_home).join("img2irc").join("glyphs.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }
    if toml_path.is_none() {
        if let Ok(home) = env::var("HOME") {
            let path = Path::new(&home)
                .join(".config")
                .join("img2irc")
                .join("glyphs.toml");
            if path.exists() {
                toml_path = Some(path);
            }
        }
    }

    let toml_path = toml_path.expect("Failed to find glyphs.toml for removing range");
    let toml_str = fs::read_to_string(&toml_path).expect("Failed to read glyphs.toml");
    let mut raw_config: toml::map::Map<String, Value> =
        toml::from_str(&toml_str).expect("Invalid TOML");

    if remove_block_path(&mut raw_config, name) {
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

        fs::write(toml_path, new_toml).expect("Failed to write glyphs.toml");
    }
}

pub fn save_blocks_config(block_name: &str, exclude: &[char]) {
    let mut toml_path = None;
    if let Ok(config_dir) = env::var("IMG2IRC_CONFIG_DIR") {
        let path = Path::new(&config_dir).join("glyphs.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }
    if toml_path.is_none() {
        let path = PathBuf::from("glyphs.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }
    if toml_path.is_none() {
        let config_home = env::var("XDG_CONFIG_HOME")
            .unwrap_or_else(|_| env::var("HOME").unwrap_or_else(|_| ".".to_string()) + "/.config");
        let path = Path::new(&config_home).join("img2irc").join("glyphs.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }
    if toml_path.is_none() {
        if let Ok(home) = env::var("HOME") {
            let path = Path::new(&home)
                .join(".config")
                .join("img2irc")
                .join("glyphs.toml");
            if path.exists() {
                toml_path = Some(path);
            }
        }
    }

    let toml_path = toml_path.expect("Failed to find glyphs.toml for saving");
    let toml_str = fs::read_to_string(&toml_path).expect("Failed to read glyphs.toml");
    let mut raw_config: toml::map::Map<String, Value> =
        toml::from_str(&toml_str).expect("Invalid TOML");

    // Load full config to know which characters belong to which block
    let (config_groups, _) = load_blocks_config_unfiltered();
    let block_codes = config_groups.get(block_name).expect("Block not found");

    // Exclusions belong to this block only.
    let block_exclude: Vec<Value> = exclude
        .iter()
        .filter(|&ch| block_codes.contains(&(*ch as u32)))
        .map(|&ch| Value::Integer(ch as u32 as i64))
        .collect();

    // Ensure the block entry exists and update its exclude list
    if let Some(table) = block_table_mut(&mut raw_config, block_name, false) {
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

    fs::write(toml_path, new_toml).expect("Failed to write glyphs.toml");
}

fn find_glyphs_toml_path() -> Option<PathBuf> {
    if let Ok(config_dir) = env::var("IMG2IRC_CONFIG_DIR") {
        let p = Path::new(&config_dir);
        if p.is_file() {
            if p.file_name().and_then(|n| n.to_str()) == Some("glyphs.toml") {
                return Some(p.to_path_buf());
            } else if let Some(parent) = p.parent() {
                let candidate = parent.join("glyphs.toml");
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        } else {
            let path = p.join("glyphs.toml");
            if path.exists() {
                return Some(path);
            }
        }
    }
    let path = PathBuf::from("glyphs.toml");
    if path.exists() {
        return Some(path);
    }
    let config_home = env::var("XDG_CONFIG_HOME")
        .unwrap_or_else(|_| env::var("HOME").unwrap_or_else(|_| ".".to_string()) + "/.config");
    let path = Path::new(&config_home).join("img2irc").join("glyphs.toml");
    if path.exists() {
        return Some(path);
    }
    if let Ok(home) = env::var("HOME") {
        let path = Path::new(&home)
            .join(".config")
            .join("img2irc")
            .join("glyphs.toml");
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn enabled_sets_from_config(root: &toml::map::Map<String, Value>) -> Option<Vec<String>> {
    let table = root.get("ui")?.as_table()?;
    let array = table.get("enabled_sets")?.as_array()?;
    let sets = array
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    (!sets.is_empty()).then_some(sets)
}

fn upsert_enabled_sets(root: &mut toml::map::Map<String, Value>, sets: &[String]) {
    let ui_table = root
        .entry("ui")
        .or_insert_with(|| Value::Table(toml::map::Map::new()));
    let Some(table) = ui_table.as_table_mut() else {
        return;
    };
    let mut sorted_sets = sets.to_vec();
    sorted_sets.sort();
    sorted_sets.dedup();
    table.insert(
        "enabled_sets".to_string(),
        Value::Array(sorted_sets.into_iter().map(Value::String).collect()),
    );
}

fn format_glyphs_toml(raw_config: toml::map::Map<String, Value>) -> String {
    let new_toml =
        toml::to_string_pretty(&Value::Table(raw_config)).expect("Failed to serialize TOML");
    let re = Regex::new(r"([ \[,]| = )(\d{3,9})").unwrap();
    re.replace_all(&new_toml, |caps: &regex::Captures| {
        let val: i64 = caps[2].parse().unwrap();
        if val > 127 {
            format!("{}0x{:04X}", &caps[1], val)
        } else {
            caps[0].to_string()
        }
    })
    .to_string()
}

/// Load the glyph-set selection persisted by the TUI, if any.
pub fn load_enabled_glyph_sets() -> Option<Vec<String>> {
    let toml_path = find_glyphs_toml_path()?;
    let toml_str = fs::read_to_string(&toml_path).ok()?;
    let root: toml::map::Map<String, Value> = toml::from_str(&toml_str).ok()?;
    enabled_sets_from_config(&root)
}

/// Persist the TUI glyph-set selection so it is restored on the next launch.
pub fn save_enabled_glyph_sets(sets: &[String]) {
    let Some(toml_path) = find_glyphs_toml_path() else {
        log::warn!("No glyphs.toml found; enabled glyph sets were not saved");
        return;
    };
    let Ok(toml_str) = fs::read_to_string(&toml_path) else {
        log::warn!("Failed to read glyphs.toml; enabled glyph sets were not saved");
        return;
    };
    let Ok(mut root) = toml::from_str::<toml::map::Map<String, Value>>(&toml_str) else {
        log::warn!("Invalid glyphs.toml; enabled glyph sets were not saved");
        return;
    };
    upsert_enabled_sets(&mut root, sets);
    match fs::write(&toml_path, format_glyphs_toml(root)) {
        Ok(()) => {}
        Err(error) => log::warn!("Failed to write glyphs.toml: {error}"),
    }
}

#[cfg(target_arch = "wasm32")]
fn load_blocks_config_impl(_apply_exclusions: bool) -> (HashMap<String, Vec<u32>>, HashSet<char>) {
    // Browser callers provide their selected characters explicitly from the
    // runtime glyph catalog. Keep the catalog out of the WASM binary so it can
    // be changed or replaced independently of the renderer.
    (HashMap::new(), HashSet::new())
}

#[cfg(not(target_arch = "wasm32"))]
fn load_blocks_config_impl(apply_exclusions: bool) -> (HashMap<String, Vec<u32>>, HashSet<char>) {
    let mut toml_path = None;
    if let Ok(config_dir) = env::var("IMG2IRC_CONFIG_DIR") {
        let path = Path::new(&config_dir).join("glyphs.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }
    if toml_path.is_none() {
        let path = PathBuf::from("glyphs.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }
    if toml_path.is_none() {
        let config_home = env::var("XDG_CONFIG_HOME")
            .unwrap_or_else(|_| env::var("HOME").unwrap_or_else(|_| ".".to_string()) + "/.config");
        let path = Path::new(&config_home).join("img2irc").join("glyphs.toml");
        if path.exists() {
            toml_path = Some(path);
        }
    }
    if toml_path.is_none() {
        if let Ok(home) = env::var("HOME") {
            let path = Path::new(&home)
                .join(".config")
                .join("img2irc")
                .join("glyphs.toml");
            if path.exists() {
                toml_path = Some(path);
            }
        }
    }

    let toml_str = if let Some(path) = toml_path {
        fs::read_to_string(path).expect("Failed to read glyphs.toml")
    } else {
        let default_toml = include_str!("../glyphs.toml");
        // Attempt to save default toml
        let mut save_path = None;
        if let Ok(config_home) = env::var("XDG_CONFIG_HOME") {
            save_path = Some(Path::new(&config_home).join("img2irc").join("glyphs.toml"));
        } else if let Ok(home) = env::var("HOME") {
            save_path = Some(
                Path::new(&home)
                    .join(".config")
                    .join("img2irc")
                    .join("glyphs.toml"),
            );
        }

        if let Some(p) = save_path {
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).ok();
            }
            if fs::write(&p, default_toml).is_err() {
                // If config home fails, try current dir
                fs::write("glyphs.toml", default_toml).ok();
            }
        } else {
            fs::write("glyphs.toml", default_toml).ok();
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
        apply_exclusions: bool,
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
                    glyphs.extend(resolve_block(
                        other,
                        blocks,
                        resolved,
                        visited,
                        apply_exclusions,
                    ));
                }
            }
            if apply_exclusions {
                if let Some(exc) = &entry.exclude {
                    let exc_set: HashSet<_> = exc.iter().map(|h| h.to_u32()).collect();
                    glyphs.retain(|g| !exc_set.contains(g));
                }
            }
        }

        glyphs.sort();
        glyphs.dedup();
        resolved.insert(name.to_string(), glyphs.clone());
        glyphs
    }

    for name in blocks.keys() {
        let mut visited = HashSet::new();
        let glyphs = resolve_block(
            name,
            &blocks,
            &mut resolved_groups,
            &mut visited,
            apply_exclusions,
        );
        for &code in &glyphs {
            if let Some(ch) = char::from_u32(code) {
                all_chars.insert(ch);
            }
        }
    }

    (resolved_groups, all_chars)
}

/// Resolve the glyphs currently enabled by each block's saved exclusions.
pub fn load_blocks_config() -> (HashMap<String, Vec<u32>>, HashSet<char>) {
    load_blocks_config_impl(true)
}

/// Resolve complete block membership for browsing and bitmap caching. Saved
/// exclusions remain members here so the UI can display and re-enable them.
pub fn load_blocks_config_unfiltered() -> (HashMap<String, Vec<u32>>, HashSet<char>) {
    load_blocks_config_impl(false)
}

#[allow(dead_code)]
fn procedural_legacy_diagonal_bitmap(ch: char, w: usize, h: usize) -> Option<GlyphBitmap> {
    let code = ch as u32;
    let mut cell = vec![vec![0u8; w]; h];

    // Triangular quarter blocks and their three-quarter complements.
    if (0x1FB68..=0x1FB6F).contains(&code) {
        for (y, row) in cell.iter_mut().enumerate() {
            let py = (y as f64 + 0.5) / h.max(1) as f64;
            for (x, pixel) in row.iter_mut().enumerate() {
                let px = (x as f64 + 0.5) / w.max(1) as f64;
                let left = px <= 0.5 - (py - 0.5).abs();
                let upper = py <= 0.5 - (px - 0.5).abs();
                let right = px >= 0.5 + (py - 0.5).abs();
                let lower = py >= 0.5 + (px - 0.5).abs();
                let filled = match code {
                    0x1FB68 => !left,
                    0x1FB69 => !upper,
                    0x1FB6A => !right,
                    0x1FB6B => !lower,
                    0x1FB6C => left,
                    0x1FB6D => upper,
                    0x1FB6E => right,
                    0x1FB6F => lower,
                    _ => unreachable!(),
                };
                *pixel = filled as u8;
            }
        }
        return Some(cell);
    }

    // Each remaining glyph is one half-plane bounded by the named diagonal.
    // Unicode's UPPER/LOWER MIDDLE edge positions use the three-row legacy
    // graphics grid: one-third and two-thirds of the cell height.
    type Point = (f64, f64);
    let lower_left: Point = (0.0, 1.0);
    let lower_right: Point = (1.0, 1.0);
    let upper_right: Point = (1.0, 0.0);
    let upper_left: Point = (0.0, 0.0);
    let (a, b, filled_anchor): (Point, Point, Point) = match code {
        0x1FB3C => ((0.0, 0.75), (0.5, 1.0), lower_left),
        0x1FB3D => ((0.0, 0.75), (1.0, 1.0), lower_left),
        0x1FB3E => ((0.0, 0.25), (0.5, 1.0), lower_left),
        0x1FB3F => ((0.0, 0.25), (1.0, 1.0), lower_left),
        0x1FB40 => ((0.0, 0.0), (0.5, 1.0), lower_left),
        0x1FB41 => ((0.0, 0.25), (0.5, 0.0), lower_right),
        0x1FB42 => ((0.0, 0.25), (1.0, 0.0), lower_right),
        0x1FB43 => ((0.0, 0.75), (0.5, 0.0), lower_right),
        0x1FB44 => ((0.0, 0.75), (1.0, 0.0), lower_right),
        0x1FB45 => ((0.0, 1.0), (0.5, 0.0), lower_right),
        0x1FB46 => ((0.0, 0.75), (1.0, 0.25), lower_right),
        0x1FB47 => ((0.5, 1.0), (1.0, 0.75), lower_right),
        0x1FB48 => ((0.0, 1.0), (1.0, 0.75), lower_right),
        0x1FB49 => ((0.5, 1.0), (1.0, 0.25), lower_right),
        0x1FB4A => ((0.0, 1.0), (1.0, 0.25), lower_right),
        0x1FB4B => ((0.5, 1.0), (1.0, 0.0), lower_right),
        0x1FB4C => ((0.5, 0.0), (1.0, 0.25), lower_left),
        0x1FB4D => ((0.0, 0.0), (1.0, 0.25), lower_left),
        0x1FB4E => ((0.5, 0.0), (1.0, 0.75), lower_left),
        0x1FB4F => ((0.0, 0.0), (1.0, 0.75), lower_left),
        0x1FB50 => ((0.5, 0.0), (1.0, 1.0), lower_left),
        0x1FB51 => ((0.0, 0.25), (1.0, 0.75), lower_left),
        0x1FB52 => ((0.0, 0.75), (0.5, 1.0), upper_right),
        0x1FB53 => ((0.0, 0.75), (1.0, 1.0), upper_right),
        0x1FB54 => ((0.0, 0.25), (0.5, 1.0), upper_right),
        0x1FB55 => ((0.0, 0.25), (1.0, 1.0), upper_right),
        0x1FB56 => ((0.0, 0.0), (0.5, 1.0), upper_right),
        0x1FB57 => ((0.0, 0.25), (0.5, 0.0), upper_left),
        0x1FB58 => ((0.0, 0.25), (1.0, 0.0), upper_left),
        0x1FB59 => ((0.0, 0.75), (0.5, 0.0), upper_left),
        0x1FB5A => ((0.0, 0.75), (1.0, 0.0), upper_left),
        0x1FB5B => ((0.0, 1.0), (0.5, 0.0), upper_left),
        0x1FB5C => ((0.0, 0.75), (1.0, 0.25), upper_left),
        0x1FB5D => ((0.5, 1.0), (1.0, 0.75), upper_left),
        0x1FB5E => ((0.0, 1.0), (1.0, 0.75), upper_left),
        0x1FB5F => ((0.5, 1.0), (1.0, 0.25), upper_left),
        0x1FB60 => ((0.0, 1.0), (1.0, 0.25), upper_left),
        0x1FB61 => ((0.5, 1.0), (1.0, 0.0), upper_left),
        0x1FB62 => ((0.5, 0.0), (1.0, 0.25), upper_right),
        0x1FB63 => ((0.0, 0.0), (1.0, 0.25), upper_right),
        0x1FB64 => ((0.5, 0.0), (1.0, 0.75), upper_right),
        0x1FB65 => ((0.0, 0.0), (1.0, 0.75), upper_right),
        0x1FB66 => ((0.5, 0.0), (1.0, 1.0), upper_right),
        0x1FB67 => ((0.0, 0.25), (1.0, 0.75), upper_right),
        _ => return None,
    };

    let legacy_y = |y: f64| {
        if (y - 0.25).abs() < f64::EPSILON {
            1.0 / 3.0
        } else if (y - 0.75).abs() < f64::EPSILON {
            2.0 / 3.0
        } else {
            y
        }
    };
    let a = (a.0, legacy_y(a.1));
    let b = (b.0, legacy_y(b.1));
    let cross = |point: Point| (b.0 - a.0) * (point.1 - a.1) - (b.1 - a.1) * (point.0 - a.0);
    let anchor_side = cross(filled_anchor);
    for (y, row) in cell.iter_mut().enumerate() {
        let py = (y as f64 + 0.5) / h.max(1) as f64;
        for (x, pixel) in row.iter_mut().enumerate() {
            let px = (x as f64 + 0.5) / w.max(1) as f64;
            *pixel = ((cross((px, py)) * anchor_side) >= 0.0) as u8;
        }
    }
    Some(cell)
}

/// Generate terminal block geometry procedurally. Font outlines for these
/// characters can use a fallback font with metrics unrelated to the main
/// terminal cell, causing clipping and size-dependent shape changes.
pub fn procedural_block_bitmap(ch: char, w: usize, h: usize) -> Option<GlyphBitmap> {
    let code = ch as u32;
    if code < 0x2580 || code > 0x259F {
        return None;
    }

    let mut cell = vec![vec![0u8; w]; h];

    // Helper: fill a rectangular region
    let fill = |cell: &mut Vec<Vec<u8>>, y0: usize, y1: usize, x0: usize, x1: usize| {
        for y in y0..y1.min(h) {
            for x in x0..x1.min(w) {
                cell[y][x] = 1;
            }
        }
    };

    match code {
        // ▀ UPPER HALF BLOCK
        0x2580 => fill(&mut cell, 0, h / 2, 0, w),
        // ▁ LOWER ONE EIGHTH BLOCK
        0x2581 => fill(&mut cell, h - h / 8, h, 0, w),
        // ▂ LOWER ONE QUARTER BLOCK
        0x2582 => fill(&mut cell, h - h / 4, h, 0, w),
        // ▃ LOWER THREE EIGHTHS BLOCK
        0x2583 => fill(&mut cell, h - (h * 3 / 8), h, 0, w),
        // ▄ LOWER HALF BLOCK
        0x2584 => fill(&mut cell, h / 2, h, 0, w),
        // ▅ LOWER FIVE EIGHTHS BLOCK
        0x2585 => fill(&mut cell, h - (h * 5 / 8), h, 0, w),
        // ▆ LOWER THREE QUARTERS BLOCK
        0x2586 => fill(&mut cell, h - (h * 3 / 4), h, 0, w),
        // ▇ LOWER SEVEN EIGHTHS BLOCK
        0x2587 => fill(&mut cell, h - (h * 7 / 8), h, 0, w),
        // █ FULL BLOCK
        0x2588 => fill(&mut cell, 0, h, 0, w),
        // ▉ LEFT SEVEN EIGHTHS BLOCK
        0x2589 => fill(&mut cell, 0, h, 0, w * 7 / 8),
        // ▊ LEFT THREE QUARTERS BLOCK
        0x258A => fill(&mut cell, 0, h, 0, w * 3 / 4),
        // ▋ LEFT FIVE EIGHTHS BLOCK
        0x258B => fill(&mut cell, 0, h, 0, w * 5 / 8),
        // ▌ LEFT HALF BLOCK
        0x258C => fill(&mut cell, 0, h, 0, w / 2),
        // ▍ LEFT THREE EIGHTHS BLOCK
        0x258D => fill(&mut cell, 0, h, 0, w * 3 / 8),
        // ▎ LEFT ONE QUARTER BLOCK
        0x258E => fill(&mut cell, 0, h, 0, w / 4),
        // ▏ LEFT ONE EIGHTH BLOCK
        0x258F => fill(&mut cell, 0, h, 0, w / 8),
        // ▐ RIGHT HALF BLOCK
        0x2590 => fill(&mut cell, 0, h, w / 2, w),
        // ░ LIGHT SHADE (25% coverage — checkerboard)
        0x2591 => {
            for y in 0..h {
                for x in 0..w {
                    if (x + y) % 4 == 0 {
                        cell[y][x] = 1;
                    }
                }
            }
        }
        // ▒ MEDIUM SHADE (50% coverage — checkerboard)
        0x2592 => {
            for y in 0..h {
                for x in 0..w {
                    if (x + y) % 2 == 0 {
                        cell[y][x] = 1;
                    }
                }
            }
        }
        // ▓ DARK SHADE (75% coverage — inverse checkerboard)
        0x2593 => {
            for y in 0..h {
                for x in 0..w {
                    if (x + y) % 4 != 0 {
                        cell[y][x] = 1;
                    }
                }
            }
        }
        // ▔ UPPER ONE EIGHTH BLOCK
        0x2594 => fill(&mut cell, 0, h / 8, 0, w),
        // ▕ RIGHT ONE EIGHTH BLOCK
        0x2595 => fill(&mut cell, 0, h, w - w / 8, w),
        // ▖ QUADRANT LOWER LEFT
        0x2596 => fill(&mut cell, h / 2, h, 0, w / 2),
        // ▗ QUADRANT LOWER RIGHT
        0x2597 => fill(&mut cell, h / 2, h, w / 2, w),
        // ▘ QUADRANT UPPER LEFT
        0x2598 => fill(&mut cell, 0, h / 2, 0, w / 2),
        // ▙ QUADRANT UPPER LEFT AND LOWER LEFT AND LOWER RIGHT
        0x2599 => {
            fill(&mut cell, 0, h / 2, 0, w / 2);
            fill(&mut cell, h / 2, h, 0, w);
        }
        // ▚ QUADRANT UPPER LEFT AND LOWER RIGHT
        0x259A => {
            fill(&mut cell, 0, h / 2, 0, w / 2);
            fill(&mut cell, h / 2, h, w / 2, w);
        }
        // ▛ QUADRANT UPPER LEFT AND UPPER RIGHT AND LOWER LEFT
        0x259B => {
            fill(&mut cell, 0, h / 2, 0, w);
            fill(&mut cell, h / 2, h, 0, w / 2);
        }
        // ▜ QUADRANT UPPER LEFT AND UPPER RIGHT AND LOWER RIGHT
        0x259C => {
            fill(&mut cell, 0, h / 2, 0, w);
            fill(&mut cell, h / 2, h, w / 2, w);
        }
        // ▝ QUADRANT UPPER RIGHT
        0x259D => fill(&mut cell, 0, h / 2, w / 2, w),
        // ▞ QUADRANT UPPER RIGHT AND LOWER LEFT
        0x259E => {
            fill(&mut cell, 0, h / 2, w / 2, w);
            fill(&mut cell, h / 2, h, 0, w / 2);
        }
        // ▟ QUADRANT UPPER RIGHT AND LOWER LEFT AND LOWER RIGHT
        0x259F => {
            fill(&mut cell, 0, h / 2, w / 2, w);
            fill(&mut cell, h / 2, h, 0, w);
        }
        _ => return None,
    }

    Some(cell)
}

thread_local! {
    static SCALE_CONTEXT: std::cell::RefCell<ScaleContext> =
        std::cell::RefCell::new(ScaleContext::new());
}

fn render_glyph_from_font(font: &SystemFont, ch: char, metrics: &CachedMetrics) -> GlyphBitmap {
    let cell_width = metrics.cell_width.max(1);
    let cell_height = metrics.cell_height.max(1);

    // Block elements (U+2580–U+259F): render procedurally, matching terminal behavior.
    if let Some(bitmap) = procedural_block_bitmap(ch, cell_width, cell_height) {
        return bitmap;
    }

    let mut cell = vec![vec![0u8; cell_width]; cell_height];

    let font_ref = font.font_ref();
    let gid = font_ref.charmap().map(ch);
    if gid == 0 {
        return cell;
    }

    SCALE_CONTEXT.with(|ctx_cell| {
        let mut ctx = ctx_cell.borrow_mut();
        let mut scaler = ctx
            .builder(font_ref)
            .size(metrics.font_size)
            .hint(false)
            .build();

        if let Some(image) = Render::new(&[SwashSource::Outline]).render(&mut scaler, gid) {
            let baseline_y = metrics.main_ascent_px.round() as i32;
            let start_y = baseline_y - image.placement.top;

            let font_metrics = font_ref.metrics(&[]);
            let units_per_em = font_metrics.units_per_em as f32;
            let scale_factor = metrics.font_size / units_per_em;

            let glyph_metrics = font_ref.glyph_metrics(&[]);
            let advance_px = glyph_metrics.advance_width(gid) * scale_factor;
            let x_origin = (cell_width as f32 - advance_px) / 2.0;
            let start_x = (x_origin.round() as i32) + image.placement.left;

            let w = image.placement.width as usize;
            let h = image.placement.height as usize;

            for y in 0..h {
                let cell_y = start_y + y as i32;
                if cell_y >= 0 && cell_y < cell_height as i32 {
                    for x in 0..w {
                        let cell_x = start_x + x as i32;
                        if cell_x >= 0 && cell_x < cell_width as i32 {
                            let idx = y * w + x;
                            if idx < image.data.len() {
                                if image.data[idx] > 127 {
                                    cell[cell_y as usize][cell_x as usize] = 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    });

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

fn measure_reference_advance_unscaled(font: &SystemFont) -> f32 {
    let font_ref = font.font_ref();
    let charmap = font_ref.charmap();
    let glyph_metrics = font_ref.glyph_metrics(&[]);

    let mut gid = charmap.map('X');
    let mut advance = glyph_metrics.advance_width(gid);
    if advance == 0.0 {
        gid = charmap.map('0');
        advance = glyph_metrics.advance_width(gid);
    }
    if advance == 0.0 {
        let metrics = font_ref.metrics(&[]);
        advance = metrics.units_per_em as f32 * 0.6;
    }
    advance.max(1.0)
}

pub fn is_monospace_font(font: &SystemFont) -> bool {
    let font_ref = font.font_ref();
    let charmap = font_ref.charmap();
    let glyph_metrics = font_ref.glyph_metrics(&[]);

    let chars_to_check = ['X', 'i', 'W', 'M', '.', '|'];
    let mut advances = Vec::new();
    for &ch in &chars_to_check {
        let gid = charmap.map(ch);
        if gid != 0 {
            let adv = glyph_metrics.advance_width(gid);
            if adv > 0.0 {
                advances.push(adv);
            }
        }
    }
    if advances.len() < 2 {
        return true;
    }
    let first = advances[0];
    advances.iter().all(|&a| (a - first).abs() < 1.0)
}

fn is_renderable_char(ch: char) -> bool {
    if ch.is_control() {
        return false;
    }
    ch.width() == Some(1)
}

fn font_supports_terminal_char(font: &SystemFont, ch: char) -> bool {
    if !is_renderable_char(ch) {
        return false;
    }

    let font_ref = font.font_ref();
    let gid = font_ref.charmap().map(ch);
    if gid == 0 {
        return false;
    }

    let glyph_advance = font_ref.glyph_metrics(&[]).advance_width(gid);
    let cell_advance = measure_reference_advance_unscaled(font);
    glyph_advance > 0.0 && (glyph_advance - cell_advance).abs() <= cell_advance * 0.1
}

fn load_monospace_faces(path: &str) -> Vec<Arc<SystemFont>> {
    let Ok(data) = fs::read(path) else {
        return Vec::new();
    };
    let bytes = Arc::<[u8]>::from(data);
    let mut faces = Vec::new();
    let mut face_index = 0usize;

    while FontRef::from_index(&bytes, face_index).is_some() {
        if let Some(font) = SystemFont::new(bytes.clone(), face_index, path.to_string()) {
            if is_monospace_font(&font) {
                faces.push(Arc::new(font));
            }
        }
        face_index += 1;
    }

    faces
}

fn resolve_font_for_char(ctx: &mut FontContext, ch: char) -> Option<(Arc<SystemFont>, String)> {
    if ctx.missing_chars.contains(&ch) {
        return None;
    }

    for path in &ctx.font_paths {
        if let Some(font) = ctx.user_faces.get(path) {
            if font_supports_terminal_char(font, ch) {
                let font_name = Path::new(&font.source_path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| font.source_path.clone());
                return Some((font.clone(), font_name));
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    for bundled in &BUNDLED_FONTS {
        let font = bundled.face();
        if font_supports_terminal_char(&font, ch) {
            return Some((font, bundled.name.to_string()));
        }
    }

    let paths = get_fallback_font_paths(ch);
    for path in paths {
        if !ctx.fallback_faces.contains_key(&path) {
            let faces = load_monospace_faces(&path);
            if !faces.is_empty() {
                log::info!("Adding fallback font: {} ({} faces)", path, faces.len());
            } else {
                log::debug!("Fallback font {} has no monospace faces, skipping", path);
            }
            ctx.fallback_faces.insert(path.clone(), faces);
        }
        if let Some(faces) = ctx.fallback_faces.get(&path) {
            if let Some(font) = faces
                .iter()
                .find(|font| font_supports_terminal_char(font, ch))
            {
                let font_name = Path::new(&path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
                return Some((Arc::clone(font), font_name));
            }
        }
    }

    log::info!("Could not resolve font for character U+{:04X}", ch as u32);
    ctx.missing_chars.insert(ch);
    None
}

fn metrics_hash_context(font_paths: &[String], font_size: f32, chars: &[char]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    "v17".hash(&mut hasher);
    for p in font_paths {
        p.hash(&mut hasher);
    }
    font_size.to_bits().hash(&mut hasher);
    for ch in chars {
        (*ch as u32).hash(&mut hasher);
    }
    format!("{:x}", hasher.finish())
}

fn block_hash_context(
    font_paths: &[String],
    font_size: f32,
    metrics: &CachedMetrics,
    chars: &[char],
) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    "v17".hash(&mut hasher);
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
    let mut sorted_chars = chars.to_vec();
    sorted_chars.sort_unstable();
    sorted_chars.dedup();
    for ch in sorted_chars {
        (ch as u32).hash(&mut hasher);
    }
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

    let main_font = ctx.get_main_font();
    let font_ref = main_font.font_ref();
    let metrics = font_ref.metrics(&[]);

    let units_per_em = metrics.units_per_em as f32;
    let font_size = ctx.font_size;
    let scale_factor = font_size / units_per_em;

    let ascent = metrics.ascent as f32 * scale_factor;
    let descent = metrics.descent as f32 * scale_factor;
    let leading = metrics.leading as f32 * scale_factor;

    let line_height = ascent + descent + leading;
    let cell_height = line_height.ceil() as usize;

    let advance_unscaled = measure_reference_advance_unscaled(&*main_font);
    let advance = advance_unscaled * scale_factor;
    let cell_width = advance.round().max(1.0) as usize;

    let main_ascent_px = ascent;

    let bounds_min_x = 0.0f32;
    let bounds_max_x = advance_unscaled;
    let bounds_min_y = -(metrics.descent as f32);
    let bounds_max_y = metrics.ascent as f32;

    let ascender = bounds_max_y.ceil() as i32;
    let descender = bounds_min_y.floor() as i32;

    log::info!(
        "Metrics (glyph box): Ascender={}, Descender={}, Height={}, Width={}",
        ascender,
        descender,
        cell_height,
        cell_width
    );

    let metrics = CachedMetrics {
        cell_width,
        cell_height,
        ascender,
        descender,
        float_width: advance,
        float_height: line_height,
        font_size: ctx.font_size,
        main_ascent_px,
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
    let hash = block_hash_context(&ctx.font_paths, ctx.font_size, metrics, chars);
    let path = cache_dir().join(format!("block_{}_{}.bin", block_name, hash));

    if path.exists() {
        if let Ok(data) = fs::read(&path) {
            if let Ok(block) = bincode::deserialize::<CachedBlock>(&data) {
                let dimensions_are_valid = block.glyphs.iter().all(|(_, bitmap, _)| {
                    bitmap.len() == metrics.cell_height
                        && bitmap.iter().all(|row| row.len() == metrics.cell_width)
                });
                if dimensions_are_valid {
                    return block.glyphs;
                }
            }
        }
    }

    log::info!("Rendering block '{}' ({} glyphs)", block_name, chars.len());
    let mut glyphs = Vec::new();

    for &ch in chars {
        if ch == ' ' {
            let bitmap = vec![vec![0u8; metrics.cell_width.max(1)]; metrics.cell_height.max(1)];
            glyphs.push((ch, bitmap, "Space".to_string()));
            continue;
        }

        let mut found = false;

        if ctx.missing_chars.contains(&ch) {
            log::info!("Skipping missing char: {}", ch);
            continue;
        }

        if !is_renderable_char(ch) {
            log::info!("Skipping non-renderable char: {}", ch);
            continue;
        }

        // Terminal block geometry must not depend on fallback-font metrics.
        if let Some(bitmap) =
            procedural_block_bitmap(ch, metrics.cell_width.max(1), metrics.cell_height.max(1))
        {
            glyphs.push((ch, bitmap, "Procedural terminal geometry".to_string()));
            found = true;
        // Otherwise resolve a font (handles user fonts and fallbacks).
        } else if let Some((font, font_name)) = resolve_font_for_char(ctx, ch) {
            let bitmap = render_glyph_from_font(&*font, ch, metrics);
            glyphs.push((ch, bitmap, font_name));
            found = true;
        } else {
            let main_font = ctx.get_main_font();
            if main_font.font_ref().charmap().map(ch) != 0 {
                let bitmap = render_glyph_from_font(&*main_font, ch, metrics);
                glyphs.push((ch, bitmap, main_font.source_path.clone()));
                found = true;
            }
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

#[cfg(not(target_arch = "wasm32"))]
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
    let ctx = FontContext::new(fonts, font_size);
    glyph_store_with_context(
        ctx,
        blocks,
        exclude_ranges,
        exclude_chars,
        include_ranges,
        include_chars,
        load_all,
        true,
        abort_flag,
    )
}

/// Build a glyph store from font bytes supplied by an embedding application.
///
/// This is the browser-safe equivalent of [`glyph_store`], which discovers
/// installed fonts through native operating-system APIs.
pub fn glyph_store_from_font_bytes(
    font_data: &[u8],
    blocks: &[String],
    exclude_ranges: &[String],
    exclude_chars: &[char],
    include_ranges: &[String],
    include_chars: Option<&String>,
    font_size: f32,
    load_all: bool,
) -> Result<GlyphStore, String> {
    let ctx = FontContext::from_bytes(font_data, font_size)?;
    Ok(glyph_store_with_context(
        ctx,
        blocks,
        exclude_ranges,
        exclude_chars,
        include_ranges,
        include_chars,
        load_all,
        false,
        None,
    ))
}

fn glyph_store_with_context(
    mut ctx: FontContext,
    blocks: &[String],
    exclude_ranges: &[String],
    exclude_chars: &[char],
    include_ranges: &[String],
    include_chars: Option<&String>,
    load_all: bool,
    select_all_when_blocks_empty: bool,
    abort_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> GlyphStore {
    let (config_groups, _) = load_blocks_config();
    let (all_config_groups, _) = load_blocks_config_unfiltered();

    // Determine which blocks to load
    let mut blocks_to_load = HashSet::new();
    let mut blocks_to_select = HashSet::new();

    // Always select specified blocks or prefixes
    for name in blocks {
        let name_lc = name.to_lowercase();
        let mut matched = false;
        for key in all_config_groups.keys() {
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

    if blocks.is_empty() && !load_all && select_all_when_blocks_empty {
        // If no blocks specified and not loading all, load ALL known blocks (legacy behavior?)
        // Wait, if blocks is empty, args.blocks is empty.
        // Old behavior: "If no blocks specified, load ALL known blocks from config"
        blocks_to_load.extend(all_config_groups.keys().cloned());
        blocks_to_select.extend(all_config_groups.keys().cloned());
    } else if load_all {
        // Load all, but only select specified
        blocks_to_load.extend(all_config_groups.keys().cloned());
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
        if let Some(codes) = all_config_groups.get(*block_name) {
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

        if let Some(codes) = all_config_groups.get(block_name) {
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
                let enabled_codes = config_groups.get(block_name);

                for (ch, bitmap, font_name) in glyphs {
                    all_glyphs.insert(ch, (bitmap, font_name));
                    if is_selected
                        && enabled_codes.is_some_and(|codes| codes.contains(&(ch as u32)))
                    {
                        selected_chars.insert(ch);
                    }
                }
            }
        } else {
            log::info!("Warning: Block '{}' not defined in glyphs.toml", block_name);
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

pub fn print_glyph_chars() -> String {
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

pub fn print_glyph_bitmaps(store: &GlyphStore) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_fonts_supply_default_and_explicit_family_without_system_lookup() {
        let default = FontContext::new(&[], 16.0);
        assert_eq!(default.get_main_font().source_path, BUNDLED_FONTS[0].path);

        let fixed = FontContext::new(&["iosevka fixed".to_string()], 16.0);
        assert_eq!(fixed.get_main_font().source_path, BUNDLED_FONTS[1].path);
        assert!(is_monospace_font(&fixed.get_main_font()));
        assert!(font_supports_terminal_char(&fixed.get_main_font(), 'A'));
    }

    #[test]
    fn bundled_fonts_cover_polygon_third_octant_and_large_glyphs() {
        let mut context = FontContext::new(&[], 16.0);
        for code in [0x1FB3C, 0x1FB02, 0x1FB0E, 0x1FB2D, 0x1FB39, 0x1CD20, 0x1CE1A] {
            let ch = char::from_u32(code).unwrap();
            let (font, _) = resolve_font_for_char(&mut context, ch)
                .unwrap_or_else(|| panic!("missing bundled glyph U+{code:X}"));
            assert!(font.source_path.starts_with("bundled/"));
        }
    }

    #[test]
    fn nested_block_paths_can_be_created_and_removed_at_any_depth() {
        let mut root = toml::map::Map::new();
        block_table_mut(&mut root, "blocks.eighth.vertical", true)
            .unwrap()
            .insert(
                "ranges".to_string(),
                Value::Array(vec![Value::Array(vec![
                    Value::Integer(0x2581),
                    Value::Integer(0x2587),
                ])]),
            );
        block_table_mut(&mut root, "blocks.eighth.horizontal", true)
            .unwrap()
            .insert(
                "glyphs".to_string(),
                Value::Array(vec![Value::Integer(0x258C)]),
            );

        assert!(block_table_mut(&mut root, "blocks.eighth.vertical", false).is_some());
        assert!(block_table_mut(&mut root, "blocks.eighth.horizontal", false).is_some());

        assert!(remove_block_path(&mut root, "blocks.eighth.vertical"));
        assert!(block_table_mut(&mut root, "blocks.eighth.vertical", false).is_none());
        assert!(block_table_mut(&mut root, "blocks.eighth.horizontal", false).is_some());

        assert!(remove_block_path(&mut root, "blocks.eighth.horizontal"));
        assert!(root.is_empty(), "empty parent folders should be pruned");
    }

    #[test]
    fn quoted_dotted_block_names_remain_addressable() {
        let mut root = toml::map::Map::new();
        root.insert(
            "legacy.one".to_string(),
            Value::Table(toml::map::Map::from_iter([(
                "glyphs".to_string(),
                Value::Array(vec![Value::Integer(0x2588)]),
            )])),
        );

        assert!(block_table_mut(&mut root, "legacy.one", false).is_some());
        assert!(remove_block_path(&mut root, "legacy.one"));
        assert!(root.is_empty());
    }

    #[test]
    fn only_single_cell_characters_are_font_render_candidates() {
        assert!(is_renderable_char('⇠'));
        assert!(!is_renderable_char('界'));
        assert!(!is_renderable_char('\u{0301}'));
    }

    #[test]
    fn smooth_group_composes_polygons_and_four_thirds() {
        let (groups, _) = load_blocks_config();
        let smooth = groups["smooth"].iter().copied().collect::<HashSet<_>>();
        let expected = ["poly", "thirds"]
            .into_iter()
            .flat_map(|name| groups[name].iter().copied())
            .collect::<HashSet<_>>();

        assert_eq!(smooth, expected);
        assert_eq!(groups["thirds"], [0x1FB02, 0x1FB0E, 0x1FB2D, 0x1FB39]);
    }

    #[test]
    fn iosevka_collection_rejects_the_double_advance_arrow_face() {
        let path = "/usr/share/fonts/TTF/Iosevka-Regular.ttc";
        if !Path::new(path).is_file() {
            return;
        }

        let faces = load_monospace_faces(path);
        assert!(faces.len() >= 2);
        assert!(!font_supports_terminal_char(&faces[0], '⇠'));
        assert!(faces
            .iter()
            .skip(1)
            .any(|face| font_supports_terminal_char(face, '⇠')));
    }

    #[test]
    fn enabled_sets_round_trip_through_the_ui_table() {
        let mut root = toml::map::Map::new();
        root.insert(
            "blocks.half".to_string(),
            Value::Table(toml::map::Map::from_iter([(
                "glyphs".to_string(),
                Value::Array(vec![Value::Integer(0x2580)]),
            )])),
        );

        upsert_enabled_sets(
            &mut root,
            &["blocks.half".to_string(), "default".to_string(), "blocks.half".to_string()],
        );
        assert_eq!(
            enabled_sets_from_config(&root),
            Some(vec!["blocks.half".to_string(), "default".to_string()])
        );

        upsert_enabled_sets(&mut root, &["geo".to_string()]);
        assert_eq!(
            enabled_sets_from_config(&root),
            Some(vec!["geo".to_string()])
        );

        let serialized = format_glyphs_toml(root.clone());
        let reloaded: toml::map::Map<String, Value> = toml::from_str(&serialized).unwrap();
        assert_eq!(
            enabled_sets_from_config(&reloaded),
            Some(vec!["geo".to_string()])
        );
        assert!(reloaded.get("blocks.half").is_some());
    }

    #[test]
    fn enabled_sets_are_absent_without_a_ui_table() {
        let mut root = toml::map::Map::new();
        root.insert(
            "half".to_string(),
            Value::Table(toml::map::Map::from_iter([(
                "glyphs".to_string(),
                Value::Array(vec![Value::Integer(0x2580)]),
            )])),
        );
        assert!(enabled_sets_from_config(&root).is_none());
    }
}
