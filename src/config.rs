use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Default, Debug)]
pub struct Config {
    pub font: Option<String>,
    pub blocks: Option<Vec<String>>,
    pub ocr_figlet_fonts: Option<BTreeMap<String, Vec<String>>>,
}

#[derive(Clone, Deserialize, Default, Debug, PartialEq, Eq)]
pub struct UiLayout {
    #[serde(default)]
    pub general: ItemLayout,
    #[serde(default)]
    pub pipeline: ItemLayout,
    #[serde(default)]
    pub panes: PaneLayout,
}

#[derive(Clone, Deserialize, Default, Debug, PartialEq, Eq)]
pub struct ItemLayout {
    /// Stable item IDs to place first, in the requested order. Items omitted
    /// here are appended in their default order so new releases remain usable.
    #[serde(default)]
    pub order: Vec<String>,
    /// Stable item IDs that should not be shown.
    #[serde(default)]
    pub hidden: Vec<String>,
}

#[derive(Clone, Deserialize, Default, Debug, PartialEq, Eq)]
pub struct PaneLayout {
    pub general_width: Option<u16>,
    pub pipeline_width: Option<u16>,
    pub glyphs_width: Option<u16>,
    pub text_width: Option<u16>,
    pub glyph_tree_height: Option<u16>,
    pub glyph_list_height: Option<u16>,
}

impl Config {
    pub fn load() -> Option<Self> {
        load_toml("config.toml")
    }
}

impl UiLayout {
    /// Load the optional TUI layout file using the same search order as the
    /// main config: IMG2IRC_CONFIG_DIR, the current directory, then XDG config.
    pub fn load() -> Option<Self> {
        load_toml("layout.toml")
    }
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum FigletConfigFile {
    Direct(BTreeMap<String, Vec<String>>),
    NestedFonts { fonts: BTreeMap<String, Vec<String>> },
    NestedOcr { ocr_figlet_fonts: BTreeMap<String, Vec<String>> },
}

pub fn load_figlet_config() -> Option<BTreeMap<String, Vec<String>>> {
    load_toml::<FigletConfigFile>("figlet.toml").map(|cfg| match cfg {
        FigletConfigFile::Direct(m) => m,
        FigletConfigFile::NestedFonts { fonts } => fonts,
        FigletConfigFile::NestedOcr { ocr_figlet_fonts } => ocr_figlet_fonts,
    })
}

fn load_toml<T: DeserializeOwned>(filename: &str) -> Option<T> {
    find_config_file(filename)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|content| toml::from_str(&content).ok())
}

fn find_config_file(filename: &str) -> Option<PathBuf> {
    let mut toml_path = None;

    // 1. Check IMG2IRC_CONFIG_DIR
    if let Ok(config_dir) = env::var("IMG2IRC_CONFIG_DIR") {
        let p = Path::new(&config_dir);
        if p.is_file() {
            if filename == "config.toml"
                || p.file_name().and_then(|n| n.to_str()) == Some(filename)
            {
                toml_path = Some(p.to_path_buf());
            } else if let Some(parent) = p.parent() {
                let candidate = parent.join(filename);
                if candidate.exists() {
                    toml_path = Some(candidate);
                }
            }
        } else {
            let path = p.join(filename);
            if path.exists() {
                toml_path = Some(path);
            }
        }
    }

    // 2. Check local directory
    if toml_path.is_none() {
        let path = PathBuf::from(filename);
        if path.exists() {
            toml_path = Some(path);
        }
    }

    // 3. Check XDG_CONFIG_HOME or ~/.config/img2irc/
    if toml_path.is_none() {
        let config_home = env::var("XDG_CONFIG_HOME")
            .unwrap_or_else(|_| env::var("HOME").unwrap_or_else(|_| ".".to_string()) + "/.config");
        let path = Path::new(&config_home).join("img2irc").join(filename);
        if path.exists() {
            toml_path = Some(path);
        }
    }

    // 4. Check ~/.config/img2irc directly just in case logic above varies
    if toml_path.is_none() {
        if let Ok(home) = env::var("HOME") {
            let path = Path::new(&home)
                .join(".config")
                .join("img2irc")
                .join(filename);
            if path.exists() {
                toml_path = Some(path);
            }
        }
    }

    toml_path
}

#[cfg(test)]
mod tests {
    use super::{Config, UiLayout};

    #[test]
    fn parses_ocr_figlet_fonts_by_line_height() {
        let config: Config = toml::from_str(
            r#"
                [ocr_figlet_fonts]
                1 = ["plain"]
                2 = ["phm-minecraft", "phm-lcdmatrix"]
                3 = ["phm-largetype"]
            "#,
        )
        .unwrap();

        let fonts = config.ocr_figlet_fonts.unwrap();
        assert_eq!(fonts["2"], ["phm-minecraft", "phm-lcdmatrix"]);
    }

    #[test]
    fn parses_partial_ui_layout() {
        let layout: UiLayout = toml::from_str(
            r#"
                [general]
                order = ["width", "mode", "ocr"]
                hidden = ["encoding"]

                [pipeline]
                order = ["geometry", "rotate", "brightness"]

                [panes]
                general_width = 48
                pipeline_width = 72
            "#,
        )
        .unwrap();

        assert_eq!(layout.general.order, ["width", "mode", "ocr"]);
        assert_eq!(layout.general.hidden, ["encoding"]);
        assert_eq!(layout.pipeline.order[0], "geometry");
        assert_eq!(layout.panes.general_width, Some(48));
        assert_eq!(layout.panes.pipeline_width, Some(72));
    }
}
