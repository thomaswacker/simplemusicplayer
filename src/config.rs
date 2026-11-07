//! Persistent settings, stored as JSON in `~/.config/simplemusicplayer/config.json`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Start directory; the browser never navigates above it.
    pub root_dir: PathBuf,
    /// Last browsed directory (restored on start if it lies inside `root_dir`).
    pub last_dir: Option<PathBuf>,
    /// Volume as shown to the user, 0.0..=1.0.
    pub volume: f32,
    pub muted: bool,
}

impl Default for Config {
    fn default() -> Self {
        let root_dir = dirs::audio_dir()
            .filter(|p| p.is_dir())
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("/"));
        Self {
            root_dir,
            last_dir: None,
            volume: 0.7,
            muted: false,
        }
    }
}

fn config_path() -> Option<PathBuf> {
    Some(
        dirs::config_dir()?
            .join("simplemusicplayer")
            .join("config.json"),
    )
}

impl Config {
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Config>(&text) {
                Ok(mut cfg) => {
                    cfg.volume = cfg.volume.clamp(0.0, 1.0);
                    if !cfg.root_dir.is_dir() {
                        cfg.root_dir = Self::default().root_dir;
                    }
                    cfg
                }
                Err(e) => {
                    eprintln!(
                        "simplemusicplayer: ignoring broken config {}: {e}",
                        path.display()
                    );
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    /// Writes the config atomically (temp file + rename).
    pub fn save(&self) {
        let Some(path) = config_path() else { return };
        if let Err(e) = write_atomic(&path, self) {
            eprintln!("simplemusicplayer: could not save config: {e}");
        }
    }
}

fn write_atomic(path: &Path, cfg: &Config) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(cfg).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}
