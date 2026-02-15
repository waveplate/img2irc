use serde::Deserialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Default, Debug)]
pub struct Config {
    pub font: Option<String>,
    pub blocks: Option<Vec<String>>,
}

impl Config {
    pub fn load() -> Option<Self> {
        let mut toml_path = None;

        // 1. Check IMG2IRC_CONFIG_DIR
        if let Ok(config_dir) = env::var("IMG2IRC_CONFIG_DIR") {
            let path = Path::new(&config_dir).join("config.toml");
            if path.exists() {
                toml_path = Some(path);
            }
        }

        // 2. Check local directory
        if toml_path.is_none() {
            let path = PathBuf::from("config.toml");
            if path.exists() {
                toml_path = Some(path);
            }
        }

        // 3. Check XDG_CONFIG_HOME or ~/.config/img2irc/
        if toml_path.is_none() {
            let config_home = env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
                env::var("HOME").unwrap_or_else(|_| ".".to_string()) + "/.config"
            });
            let path = Path::new(&config_home).join("img2irc").join("config.toml");
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
                    .join("config.toml");
                if path.exists() {
                    toml_path = Some(path);
                }
            }
        }

        if let Some(path) = toml_path {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(config) = toml::from_str(&content) {
                    return Some(config);
                }
            }
        }

        None
    }
}
