//! The egui user interface.

use crate::config::Config;
use crate::icons::{self, Icon};
use crate::library::{self, Listing};
use crate::player::{Command, PlayerHandle, Snapshot, Status, VOLUME_STEP};
use eframe::egui::{
    self, Align, Color32, CornerRadius, FontId, Frame, Key, Margin, Rect, RichText, Sense, Stroke,
    Vec2,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

pub const ACCENT: Color32 = Color32::from_rgb(0x8B, 0x7C, 0xFF);
const BG: Color32 = Color32::from_rgb(0x17, 0x17, 0x1C);
const PANEL: Color32 = Color32::from_rgb(0x1F, 0x1F, 0x26);
const ROW_HOVER: Color32 = Color32::from_rgb(0x2A, 0x2A, 0x34);
const ROW_ACTIVE: Color32 = Color32::from_rgb(0x2C, 0x28, 0x4A);
const TEXT: Color32 = Color32::from_rgb(0xE8, 0xE8, 0xEE);
const TEXT_DIM: Color32 = Color32::from_rgb(0x9A, 0x9A, 0xA8);
const FOLDER: Color32 = Color32::from_rgb(0xC9, 0xB8, 0x7A);
const ERROR: Color32 = Color32::from_rgb(0xFF, 0x8A, 0x80);

const SEEK_STEP_MS: i64 = 10_000;

pub struct MusicApp {
    config: Config,
    player: PlayerHandle,
    current_dir: PathBuf,
    listing: Listing,
    listing_error: Option<String>,
    /// Seek slider value while the user drags it (seconds).
    seek_drag: Option<f32>,
    folder_dialog: Option<Receiver<Option<PathBuf>>>,
    window_title: String,
}

impl MusicApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        mut config: Config,
        root_override: Option<PathBuf>,
    ) -> Self {
        setup_style(&cc.egui_ctx);
        if let Some(root) = root_override {
            config.root_dir = root;
            config.last_dir = None;
            config.save();
        }
        let player = PlayerHandle::spawn(cc.egui_ctx.clone(), config.volume, config.muted);
        let start = config
            .last_dir
            .clone()
            .filter(|d| d.starts_with(&config.root_dir) && d.is_dir())
            .unwrap_or_else(|| config.root_dir.clone());
        let mut app = Self {
            config,
            player,
            current_dir: start.clone(),
            listing: Listing::default(),
            listing_error: None,
            seek_drag: None,
            folder_dialog: None,
            window_title: String::new(),
        };
        app.navigate(start);
        app
    }

    fn navigate(&mut self, dir: PathBuf) {
        match library::list_dir(&dir) {
            Ok(listing) => {
                self.listing = listing;
                self.listing_error = None;
            }
            Err(e) => {
                self.listing = Listing::default();
                self.listing_error = Some(format!("Ordner kann nicht gelesen werden: {e}"));
            }
        }
        self.current_dir = dir;
        let tracks = self.listing.tracks.iter().map(|t| t.path.clone()).collect();
        self.player.send(Command::Browse(tracks));
        if self.config.last_dir.as_ref() != Some(&self.current_dir) {
            self.config.last_dir = Some(self.current_dir.clone());
            self.config.save();
        }
    }

    fn refresh(&mut self) {
        self.navigate(self.current_dir.clone());
    }

    fn go_up(&mut self) {
        if self.current_dir != self.config.root_dir
            && let Some(parent) = self.current_dir.parent()
            && parent.starts_with(&self.config.root_dir)
        {
            self.navigate(parent.to_path_buf());
        }
    }

    fn set_root(&mut self, root: PathBuf) {
        self.config.root_dir = root.clone();
        self.navigate(root);
    }

    fn play_index(&self, index: usize) {
        let tracks = self.listing.tracks.iter().map(|t| t.path.clone()).collect();
        self.player.send(Command::PlayList { tracks, index });
    }

    /// Play/Pause button. With nothing loaded, the player starts the shown folder.
    fn toggle_play(&self) {
        self.player.send(Command::TogglePause);
    }

    fn open_folder_dialog(&mut self, ctx: &egui::Context) {
        if self.folder_dialog.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let start = self.config.root_dir.clone();
        let ctx = ctx.clone();
        // The portal dialog blocks, so keep it off the UI thread.
        std::thread::spawn(move || {
            let picked = rfd::FileDialog::new()
                .set_title("Startordner wählen")
                .set_directory(&start)
                .pick_folder();
            let _ = tx.send(picked);
            ctx.request_repaint();
        });
        self.folder_dialog = Some(rx);
    }

    fn poll_folder_dialog(&mut self) {
        let Some(rx) = &self.folder_dialog else {
            return;
        };
        match rx.try_recv() {
            Ok(picked) => {
                self.folder_dialog = None;
                if let Some(dir) = picked.filter(|d| d.is_dir()) {
                    self.set_root(dir);
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => self.folder_dialog = None,
        }
    }

    /// Keyboard shortcuts. Keys are consumed here, before any widget sees them,
    /// so e.g. Space never also "clicks" a focused button.
    fn handle_keys(&mut self, ctx: &egui::Context, snap: &Snapshot) {
        use egui::Modifiers as M;
        let mut actions: Vec<Command> = Vec::new();
        let mut go_up = false;
        let mut refresh = false;
        let mpris = snap.media_keys_active;
        ctx.input_mut(|i| {
            let mut key = |mods: M, k: Key| i.consume_key(mods, k);
            if key(M::NONE, Key::Space) {
                actions.push(Command::TogglePause);
            }
            if key(M::NONE, Key::N) || key(M::COMMAND, Key::ArrowRight) {
                actions.push(Command::Next);
            }
            if key(M::NONE, Key::P) || key(M::COMMAND, Key::ArrowLeft) {
                actions.push(Command::Previous);
            }
            if key(M::NONE, Key::ArrowRight) {
                actions.push(Command::SeekBy(SEEK_STEP_MS));
            }
            if key(M::NONE, Key::ArrowLeft) {
                actions.push(Command::SeekBy(-SEEK_STEP_MS));
            }
            if key(M::NONE, Key::Plus) || key(M::NONE, Key::Equals) || key(M::NONE, Key::ArrowUp) {
                actions.push(Command::VolumeStep(VOLUME_STEP));
            }
            if key(M::NONE, Key::Minus) || key(M::NONE, Key::ArrowDown) {
                actions.push(Command::VolumeStep(-VOLUME_STEP));
            }
            if key(M::NONE, Key::M) {
                actions.push(Command::ToggleMute);
            }
            go_up = key(M::NONE, Key::Backspace);
            refresh = key(M::NONE, Key::F5);
            // Hardware media keys normally reach us via MPRIS (works without focus).
            // Only handle them here if MPRIS is unavailable, to avoid double triggers.
            if !mpris {
                let media = [
                    (Key::MediaPlayPause, Command::TogglePause),
                    (Key::MediaTrackNext, Command::Next),
                    (Key::MediaTrackPrevious, Command::Previous),
                    (Key::MediaStop, Command::Stop),
                    (Key::AudioVolumeUp, Command::VolumeStep(VOLUME_STEP)),
                    (Key::AudioVolumeDown, Command::VolumeStep(-VOLUME_STEP)),
                    (Key::AudioVolumeMute, Command::ToggleMute),
                ];
                for (k, cmd) in media {
                    if key(M::NONE, k) {
                        actions.push(cmd);
                    }
                }
            }
        });
        for a in actions {
            self.player.send(a);
        }
        if go_up {
            self.go_up();
        }
        if refresh {
            self.refresh();
        }
    }

    fn sync_config(&mut self, snap: &Snapshot, ctx: &egui::Context) {
        let dragging = ctx.input(|i| i.pointer.any_down());
        if !dragging && (self.config.volume != snap.volume || self.config.muted != snap.muted) {
            self.config.volume = snap.volume;
            self.config.muted = snap.muted;
            self.config.save();
        }
    }

    fn update_title(&mut self, snap: &Snapshot, ctx: &egui::Context) {
        let title = match &snap.track {
            Some(t) if snap.status != Status::Stopped => {
                format!("{} – Simple Music Player", t.title)
            }
            _ => "Simple Music Player".to_owned(),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    // ---------------------------------------------------------------- UI parts

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let at_root = self.current_dir == self.config.root_dir;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if icon_button(ui, Icon::Home, 30.0, !at_root, "Zum Startordner").clicked() {
                let root = self.config.root_dir.clone();
                self.navigate(root);
            }
            if icon_button(
                ui,
                Icon::Up,
                30.0,
                !at_root,
                "Übergeordneter Ordner (Rücktaste)",
            )
            .clicked()
            {
                self.go_up();
            }
            ui.add_space(6.0);

            // Right side first so the breadcrumb can use the remaining width.
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let busy = self.folder_dialog.is_some();
                if icon_button(ui, Icon::FolderOpen, 30.0, !busy, "Startordner wählen …").clicked()
                {
                    let ctx = ui.ctx().clone();
                    self.open_folder_dialog(&ctx);
                }
                if icon_button(ui, Icon::Refresh, 30.0, true, "Neu einlesen (F5)").clicked() {
                    self.refresh();
                }
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                    self.breadcrumb(ui);
                });
            });
        });
    }

    fn breadcrumb(&mut self, ui: &mut egui::Ui) {
        let root = self.config.root_dir.clone();
        let mut crumbs: Vec<(String, PathBuf)> = vec![(dir_name(&root), root.clone())];
        if let Ok(rel) = self.current_dir.strip_prefix(&root) {
            let mut p = root.clone();
            for c in rel.components() {
                p.push(c);
                crumbs.push((c.as_os_str().to_string_lossy().into_owned(), p.clone()));
            }
        }
        // Keep it short: root › … › last two.
        let shown: Vec<Option<&(String, PathBuf)>> = if crumbs.len() > 3 {
            vec![
                Some(&crumbs[0]),
                None,
                Some(&crumbs[crumbs.len() - 2]),
                Some(&crumbs[crumbs.len() - 1]),
            ]
        } else {
            crumbs.iter().map(Some).collect()
        };
        let mut target = None;
        ui.spacing_mut().item_spacing.x = 2.0;
        let last = shown.len() - 1;
        for (i, crumb) in shown.into_iter().enumerate() {
            if i > 0 {
                ui.label(RichText::new("›").color(TEXT_DIM));
            }
            match crumb {
                None => {
                    ui.label(RichText::new("…").color(TEXT_DIM));
                }
                Some((name, path)) => {
                    let text = RichText::new(name.as_str()).size(14.0);
                    if i == last {
                        ui.add(
                            egui::Label::new(text.color(TEXT).font(heading_font(14.0))).truncate(),
                        );
                    } else {
                        let r = ui.add(
                            egui::Label::new(text.color(TEXT_DIM))
                                .sense(Sense::click())
                                .truncate(),
                        );
                        if r.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if r.clicked() {
                            target = Some(path.clone());
                        }
                    }
                }
            }
        }
        if let Some(t) = target {
            self.navigate(t);
        }
    }

    fn file_list(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        if let Some(err) = &self.listing_error {
            let err = err.clone();
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(err).color(ERROR));
                ui.add_space(8.0);
                if ui.button("Zum Startordner").clicked() {
                    let root = self.config.root_dir.clone();
                    self.navigate(root);
                }
            });
            return;
        }
        if self.listing.dirs.is_empty() && self.listing.tracks.is_empty() {
            ui.add_space(48.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("Keine Musik in diesem Ordner")
                        .color(TEXT_DIM)
                        .size(15.0),
                );
            });
            return;
        }

        let playing_path = snap.track.as_ref().map(|t| t.path.as_path());
        let mut open_dir = None;
        let mut play = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for d in &self.listing.dirs {
                    if list_row(ui, Icon::Folder, FOLDER, &d.name, false, snap.status).clicked() {
                        open_dir = Some(d.path.clone());
                    }
                }
                for (i, t) in self.listing.tracks.iter().enumerate() {
                    let active = playing_path == Some(t.path.as_path());
                    let name = library::display_stem(&t.path);
                    if list_row(ui, Icon::Note, TEXT_DIM, &name, active, snap.status).clicked() {
                        play = Some(i);
                    }
                }
                ui.add_space(6.0);
            });
        if let Some(d) = open_dir {
            self.navigate(d);
        }
        if let Some(i) = play {
            self.play_index(i);
        }
    }

    fn player_bar(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        // --- Now playing
        match &snap.track {
            Some(t) => {
                ui.add(
                    egui::Label::new(RichText::new(&t.title).font(heading_font(17.0)).color(TEXT))
                        .truncate(),
                );
                let sub = match (&t.artist, &t.album) {
                    (Some(a), Some(b)) => format!("{a} — {b}"),
                    (Some(a), None) => a.clone(),
                    (None, Some(b)) => b.clone(),
                    (None, None) => dir_name(t.path.parent().unwrap_or(Path::new(""))),
                };
                ui.add(egui::Label::new(RichText::new(sub).size(13.0).color(TEXT_DIM)).truncate());
            }
            None => {
                ui.label(
                    RichText::new("Nichts ausgewählt")
                        .size(17.0)
                        .color(TEXT_DIM),
                );
                ui.label(
                    RichText::new("Titel anklicken zum Abspielen")
                        .size(13.0)
                        .color(TEXT_DIM),
                );
            }
        }
        ui.add_space(6.0);

        // --- Seek bar
        let duration = snap
            .track
            .as_ref()
            .and_then(|t| t.duration)
            .unwrap_or_default();
        let total = duration.as_secs_f32();
        let can_seek = snap.status != Status::Stopped && total > 0.0;
        let pos = self
            .seek_drag
            .unwrap_or(snap.position.as_secs_f32())
            .min(total.max(0.0));
        ui.horizontal(|ui| {
            let time_w = 44.0;
            ui.add_sized([time_w, 18.0], egui::Label::new(time_text(pos)));
            let slider_w = (ui.available_width() - time_w - ui.spacing().item_spacing.x).max(40.0);
            ui.spacing_mut().slider_width = slider_w;
            let mut value = pos;
            let resp = ui.add_enabled(
                can_seek,
                egui::Slider::new(&mut value, 0.0..=total.max(0.001))
                    .show_value(false)
                    .trailing_fill(true),
            );
            if resp.dragged() {
                self.seek_drag = Some(value);
            }
            if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                self.player
                    .send(Command::Seek(Duration::from_secs_f32(value.max(0.0))));
                self.seek_drag = None;
            }
            let right = if total > 0.0 {
                time_text(total)
            } else {
                "–:––".into()
            };
            ui.add_sized([time_w, 18.0], egui::Label::new(right));
        });
        ui.add_space(4.0);

        // --- Transport + volume
        let (row, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 52.0), Sense::hover());
        let c = row.center();
        let small = 40.0;
        let big = 52.0;
        let gap = 14.0;
        let prev_rect = Rect::from_center_size(
            c - Vec2::new(big / 2.0 + gap + small / 2.0, 0.0),
            Vec2::splat(small),
        );
        let play_rect = Rect::from_center_size(c, Vec2::splat(big));
        let next_rect = Rect::from_center_size(
            c + Vec2::new(big / 2.0 + gap + small / 2.0, 0.0),
            Vec2::splat(small),
        );

        let has_track = snap.track.is_some();
        if icon_button_at(
            ui,
            prev_rect,
            "prev",
            Icon::Previous,
            has_track,
            "Vorheriger Titel (P)",
        )
        .clicked()
        {
            self.player.send(Command::Previous);
        }
        let playing = snap.status == Status::Playing;
        let can_play = has_track || !self.listing.tracks.is_empty();
        let tip = if playing {
            "Pause (Leertaste)"
        } else {
            "Abspielen (Leertaste)"
        };
        if play_button(ui, play_rect, playing, can_play, tip).clicked() {
            self.toggle_play();
        }
        if icon_button_at(
            ui,
            next_rect,
            "next",
            Icon::Next,
            snap.has_next,
            "Nächster Titel (N)",
        )
        .clicked()
        {
            self.player.send(Command::Next);
        }

        // Volume on the right, if there is room next to the transport buttons.
        let vol_slider_w = 96.0;
        let vol_w = 34.0 + 6.0 + vol_slider_w;
        if row.right() - vol_w > next_rect.right() + 12.0 {
            let mute_rect = Rect::from_center_size(
                egui::pos2(row.right() - vol_w + 17.0, c.y),
                Vec2::splat(34.0),
            );
            let icon = if snap.muted || snap.volume <= 0.0 {
                Icon::VolumeMuted
            } else if snap.volume < 0.5 {
                Icon::VolumeLow
            } else {
                Icon::VolumeHigh
            };
            let tip = if snap.muted {
                "Stummschaltung aufheben (M)"
            } else {
                "Stummschalten (M)"
            };
            if icon_button_at(ui, mute_rect, "mute", icon, true, tip).clicked() {
                self.player.send(Command::ToggleMute);
            }
            let slider_rect = Rect::from_min_size(
                egui::pos2(row.right() - vol_slider_w, c.y - 10.0),
                Vec2::new(vol_slider_w, 20.0),
            );
            ui.spacing_mut().slider_width = vol_slider_w;
            let mut vol = if snap.muted { 0.0 } else { snap.volume };
            let resp = ui
                .put(
                    slider_rect,
                    egui::Slider::new(&mut vol, 0.0..=1.0)
                        .show_value(false)
                        .trailing_fill(true),
                )
                .on_hover_text(format!(
                    "Lautstärke {} % (+/−)",
                    (snap.volume * 100.0).round()
                ));
            if resp.changed() {
                self.player.send(Command::SetVolume(vol));
            }
        }

        if let Some(err) = &snap.error {
            ui.add_space(2.0);
            ui.add(egui::Label::new(RichText::new(err).size(12.0).color(ERROR)).truncate());
        }
    }
}

impl eframe::App for MusicApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let snap = self.player.snapshot();
        self.poll_folder_dialog();
        self.handle_keys(&ctx, &snap);
        self.sync_config(&snap, &ctx);
        self.update_title(&snap, &ctx);

        egui::Panel::top("top")
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(10, 8)),
            )
            .show(ui, |ui| self.top_bar(ui));

        egui::Panel::bottom("player")
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin {
                        left: 18,
                        right: 18,
                        top: 14,
                        bottom: 12,
                    })
                    .stroke(Stroke::new(1.0, Color32::from_rgb(0x2C, 0x2C, 0x36))),
            )
            .show(ui, |ui| self.player_bar(ui, &snap));

        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(8, 8)))
            .show(ui, |ui| self.file_list(ui, &snap));

        // Smooth progress while playing; the playback thread also wakes us on changes.
        if snap.status == Status::Playing {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self) {
        let snap = self.player.snapshot();
        self.config.volume = snap.volume;
        self.config.muted = snap.muted;
        self.config.save();
    }
}

// -------------------------------------------------------------------- widgets

fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    size: f32,
    enabled: bool,
    tip: &str,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::splat(size),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    paint_icon_button(ui, rect, &resp, icon, enabled);
    resp.on_hover_text(tip)
}

fn icon_button_at(
    ui: &mut egui::Ui,
    rect: Rect,
    id: &str,
    icon: Icon,
    enabled: bool,
    tip: &str,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let resp = ui.interact(rect, ui.scope_id().with(id), sense);
    paint_icon_button(ui, rect, &resp, icon, enabled);
    resp.on_hover_text(tip)
}

fn paint_icon_button(ui: &egui::Ui, rect: Rect, resp: &egui::Response, icon: Icon, enabled: bool) {
    let painter = ui.painter();
    if enabled && resp.hovered() {
        let fill = if resp.is_pointer_button_down_on() {
            ROW_ACTIVE
        } else {
            ROW_HOVER
        };
        painter.circle_filled(rect.center(), rect.width() / 2.0, fill);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let color = if enabled {
        TEXT
    } else {
        Color32::from_rgb(0x55, 0x55, 0x60)
    };
    icons::paint(painter, rect.shrink(rect.width() * 0.22), icon, color);
}

fn play_button(
    ui: &mut egui::Ui,
    rect: Rect,
    playing: bool,
    enabled: bool,
    tip: &str,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let resp = ui.interact(rect, ui.scope_id().with("play"), sense);
    let painter = ui.painter();
    let fill = if !enabled {
        Color32::from_rgb(0x3A, 0x3A, 0x46)
    } else if resp.is_pointer_button_down_on() {
        Color32::from_rgb(0x74, 0x64, 0xF0)
    } else if resp.hovered() {
        Color32::from_rgb(0x9D, 0x90, 0xFF)
    } else {
        ACCENT
    };
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    painter.circle_filled(rect.center(), rect.width() / 2.0, fill);
    let icon = if playing { Icon::Pause } else { Icon::Play };
    icons::paint(
        painter,
        rect.shrink(rect.width() * 0.3),
        icon,
        Color32::WHITE,
    );
    resp.on_hover_text(tip)
}

fn list_row(
    ui: &mut egui::Ui,
    icon: Icon,
    icon_color: Color32,
    name: &str,
    active: bool,
    status: Status,
) -> egui::Response {
    let height = 34.0;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let painter = ui.painter();
    let bg = if active {
        Some(ROW_ACTIVE)
    } else if resp.hovered() {
        Some(ROW_HOVER)
    } else {
        None
    };
    if let Some(bg) = bg {
        painter.rect_filled(rect, CornerRadius::same(6), bg);
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let icon_rect = Rect::from_center_size(
        egui::pos2(rect.left() + 20.0, rect.center().y),
        Vec2::splat(18.0),
    );
    let (icon, icon_color) = if active {
        let i = match status {
            Status::Playing => Icon::VolumeHigh,
            _ => Icon::Pause,
        };
        (i, ACCENT)
    } else {
        (icon, icon_color)
    };
    icons::paint(painter, icon_rect, icon, icon_color);

    let text_color = if active { Color32::WHITE } else { TEXT };
    let font = FontId::proportional(14.5);
    let max_w = rect.width() - 46.0;
    let galley = ui.fonts_mut(|f| {
        let mut job = egui::text::LayoutJob::simple_singleline(name.to_owned(), font, text_color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
        f.layout_job(job)
    });
    let pos = egui::pos2(rect.left() + 38.0, rect.center().y - galley.size().y / 2.0);
    painter.galley(pos, galley, text_color);
    resp
}

fn time_text(secs: f32) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn dir_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------- style

fn setup_style(ctx: &egui::Context) {
    load_system_font(ctx);
    ctx.set_theme(egui::Theme::Dark);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = ACCENT;
    visuals.slider_trailing_fill = true;
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(0x3A, 0x3A, 0x46);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0xB0, 0xA6, 0xFF);
    visuals.widgets.active.bg_fill = Color32::WHITE;
    ctx.set_visuals_of(egui::Theme::Dark, visuals);
    ctx.all_styles_mut(|style| {
        style.spacing.slider_rail_height = 5.0;
        style.spacing.interact_size.y = 20.0;
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(14.0));
    });
}

/// Use KDE's default UI font (Noto Sans) if installed, for a native look.
/// A semibold cut is registered as the `semibold` family for headings.
fn load_system_font(ctx: &egui::Context) {
    const DIRS: &[&str] = &["/usr/share/fonts/truetype/noto", "/usr/share/fonts/noto"];
    let read = |file: &str| {
        DIRS.iter()
            .find_map(|d| std::fs::read(Path::new(d).join(file)).ok())
    };

    let mut fonts = egui::FontDefinitions::default();
    let fallback = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    if let Some(bytes) = read("NotoSans-Regular.ttf") {
        fonts.font_data.insert(
            "system".to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            family.insert(0, "system".to_owned());
        }
    }
    let mut semibold = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or(fallback);
    if let Some(bytes) = read("NotoSans-SemiBold.ttf") {
        fonts.font_data.insert(
            "system-semibold".to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        semibold.insert(0, "system-semibold".to_owned());
    }
    fonts
        .families
        .insert(egui::FontFamily::Name(SEMIBOLD.into()), semibold);
    ctx.set_fonts(fonts);
}

const SEMIBOLD: &str = "semibold";

fn heading_font(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name(SEMIBOLD.into()))
}
