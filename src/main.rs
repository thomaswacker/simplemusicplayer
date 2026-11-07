//! Simple Music Player – a small, robust folder-based music player.

mod app;
mod config;
mod icons;
mod library;
mod media_keys;
mod opus;
mod player;
mod resume;

use eframe::egui;
use std::path::PathBuf;

const APP_ID: &str = "simplemusicplayer";

fn main() -> eframe::Result {
    let root_override = std::env::args_os().nth(1).map(PathBuf::from).and_then(|p| {
        if p.is_dir() {
            p.canonicalize().ok()
        } else {
            eprintln!(
                "simplemusicplayer: {} ist kein Ordner, ignoriert",
                p.display()
            );
            None
        }
    });
    let config = config::Config::load();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Simple Music Player")
        .with_app_id(APP_ID)
        .with_inner_size([560.0, 720.0])
        .with_min_inner_size([400.0, 420.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        APP_ID,
        options,
        Box::new(move |cc| Ok(Box::new(app::MusicApp::new(cc, config, root_override)))),
    )
}
