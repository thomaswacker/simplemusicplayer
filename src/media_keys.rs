//! MPRIS integration (D-Bus). KDE Plasma routes the media keys (Play/Pause,
//! Next, Previous, Stop) to MPRIS players, and shows the player in its media
//! controller widget / lock screen, including volume.

use crate::player::{Command, Status, TrackInfo};
use eframe::egui;
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use std::path::Path;
use std::sync::mpsc::Sender;
use std::time::Duration;

const DEFAULT_SEEK_MS: i64 = 10_000;

pub struct MediaKeys {
    controls: MediaControls,
}

impl MediaKeys {
    pub fn new(tx: Sender<Command>, ctx: egui::Context) -> Option<Self> {
        let config = PlatformConfig {
            display_name: "Simple Music Player",
            dbus_name: "simplemusicplayer",
            hwnd: None,
        };
        let mut controls = match MediaControls::new(config) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("simplemusicplayer: media keys unavailable: {e:?}");
                return None;
            }
        };
        let attached = controls.attach(move |event| {
            let cmd = match event {
                MediaControlEvent::Play => Command::Play,
                MediaControlEvent::Pause => Command::Pause,
                MediaControlEvent::Toggle => Command::TogglePause,
                MediaControlEvent::Next => Command::Next,
                MediaControlEvent::Previous => Command::Previous,
                MediaControlEvent::Stop => Command::Stop,
                MediaControlEvent::Seek(dir) => Command::SeekBy(signed(dir, DEFAULT_SEEK_MS)),
                MediaControlEvent::SeekBy(dir, d) => {
                    Command::SeekBy(signed(dir, d.as_millis().min(i64::MAX as u128) as i64))
                }
                MediaControlEvent::SetPosition(MediaPosition(pos)) => Command::Seek(pos),
                MediaControlEvent::SetVolume(v) => Command::SetVolume(v.clamp(0.0, 1.0) as f32),
                MediaControlEvent::Raise => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    ctx.request_repaint();
                    return;
                }
                MediaControlEvent::Quit => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    ctx.request_repaint();
                    return;
                }
                _ => return,
            };
            let _ = tx.send(cmd);
        });
        if let Err(e) = attached {
            eprintln!("simplemusicplayer: media keys unavailable: {e:?}");
            return None;
        }
        Some(Self { controls })
    }

    pub fn set_track(&mut self, track: &TrackInfo) {
        let cover = find_cover(&track.path);
        let _ = self.controls.set_metadata(MediaMetadata {
            title: Some(&track.title),
            artist: track.artist.as_deref(),
            album: track.album.as_deref(),
            cover_url: cover.as_deref(),
            duration: track.duration,
        });
    }

    pub fn set_status(&mut self, status: Status, position: Duration) {
        let progress = Some(MediaPosition(position));
        let playback = match status {
            Status::Stopped => MediaPlayback::Stopped,
            Status::Playing => MediaPlayback::Playing { progress },
            Status::Paused => MediaPlayback::Paused { progress },
        };
        let _ = self.controls.set_playback(playback);
    }

    pub fn set_volume(&mut self, volume: f32, muted: bool) {
        let v = if muted { 0.0 } else { volume as f64 };
        let _ = self.controls.set_volume(v);
    }
}

fn signed(dir: SeekDirection, ms: i64) -> i64 {
    match dir {
        SeekDirection::Forward => ms,
        SeekDirection::Backward => -ms,
    }
}

/// Looks for a typical cover image next to the track and returns it as `file://` URL.
fn find_cover(track: &Path) -> Option<String> {
    const NAMES: &[&str] = &["cover", "folder", "front", "album", "albumart"];
    const EXTS: &[&str] = &["jpg", "jpeg", "png", "webp"];
    let dir = track.parent()?;
    let entries = std::fs::read_dir(dir).ok()?;
    let mut found = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let (Some(stem), Some(ext)) = (
            path.file_stem().and_then(|s| s.to_str()),
            path.extension().and_then(|s| s.to_str()),
        ) else {
            continue;
        };
        let stem = stem.to_ascii_lowercase();
        let ext = ext.to_ascii_lowercase();
        if EXTS.contains(&ext.as_str()) && NAMES.contains(&stem.as_str()) {
            found = Some(path);
            break;
        }
    }
    found.and_then(|p| file_url(&p))
}

fn file_url(path: &Path) -> Option<String> {
    use std::os::unix::ffi::OsStrExt;
    let mut url = String::from("file://");
    for &b in path.canonicalize().ok()?.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    Some(url)
}
