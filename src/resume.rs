//! Remembers the current track and position across restarts
//! (`~/.local/state/simplemusicplayer/resume.json`).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResumeState {
    pub track: PathBuf,
    pub position_ms: u64,
}

impl ResumeState {
    pub fn new(track: PathBuf, position: Duration) -> Self {
        Self {
            track,
            position_ms: position.as_millis().min(u64::MAX as u128) as u64,
        }
    }

    pub fn position(&self) -> Duration {
        Duration::from_millis(self.position_ms)
    }
}

fn path() -> Option<PathBuf> {
    let base = dirs::state_dir().or_else(dirs::data_local_dir)?;
    Some(base.join("simplemusicplayer").join("resume.json"))
}

/// The saved state, if any and if the track still exists.
pub fn load() -> Option<ResumeState> {
    let text = std::fs::read_to_string(path()?).ok()?;
    let state: ResumeState = serde_json::from_str(&text).ok()?;
    state.track.is_file().then_some(state)
}

/// Saves `state`, or removes the saved state for `None`.
pub fn store(state: Option<&ResumeState>) {
    let Some(path) = path() else { return };
    let result = match state {
        Some(s) => (|| {
            std::fs::create_dir_all(path.parent().expect("has parent"))?;
            let tmp = path.with_extension("json.tmp");
            std::fs::write(
                &tmp,
                serde_json::to_string(s).map_err(std::io::Error::other)?,
            )?;
            std::fs::rename(&tmp, &path)
        })(),
        None => match std::fs::remove_file(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        },
    };
    if let Err(e) = result {
        eprintln!("simplemusicplayer: could not save playback position: {e}");
    }
}
