//! Playback engine.
//!
//! All audio state lives on a dedicated controller thread. The UI and the MPRIS
//! (media key) integration only send [`Command`]s and read a [`Snapshot`]. This
//! keeps playback (including auto-advance and media keys) working even while the
//! window is minimized and the UI loop is not running.

use crate::library::display_stem;
use crate::media_keys::MediaKeys;
use crate::opus::OpusSource;
use crate::resume::{self, ResumeState};
use eframe::egui;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// "Previous" restarts the current track if more than this has been played.
const RESTART_THRESHOLD: Duration = Duration::from_secs(3);
/// While playing, the position is saved this often (in case the app gets killed).
const RESUME_SAVE_INTERVAL: Duration = Duration::from_secs(5);
pub const VOLUME_STEP: f32 = 0.05;

#[derive(Debug, Clone)]
pub enum Command {
    /// The folder currently shown in the UI; "Play" starts it if nothing is loaded yet.
    Browse(Vec<PathBuf>),
    /// Replace the playlist and start playing `index`.
    PlayList {
        tracks: Vec<PathBuf>,
        index: usize,
    },
    TogglePause,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    Seek(Duration),
    /// Relative seek in milliseconds (negative = backwards).
    SeekBy(i64),
    /// User volume 0.0..=1.0 (a perceptual curve is applied internally). Unmutes.
    SetVolume(f32),
    VolumeStep(f32),
    ToggleMute,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrackInfo {
    pub path: PathBuf,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<Duration>,
}

/// What the UI needs to render.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub status: Status,
    pub track: Option<TrackInfo>,
    pub position: Duration,
    pub has_next: bool,
    pub has_previous: bool,
    pub volume: f32,
    pub muted: bool,
    pub error: Option<String>,
    pub media_keys_active: bool,
}

pub struct PlayerHandle {
    tx: Sender<Command>,
    shared: Arc<Mutex<Snapshot>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PlayerHandle {
    pub fn spawn(ctx: egui::Context, volume: f32, muted: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Snapshot {
            volume,
            muted,
            ..Default::default()
        }));
        let thread_shared = shared.clone();
        let thread_tx = tx.clone();
        let thread = std::thread::Builder::new()
            .name("playback".into())
            .spawn(move || {
                Controller::new(ctx, rx, thread_tx, thread_shared, volume, muted).run();
            })
            .expect("failed to spawn playback thread");
        Self {
            tx,
            shared,
            thread: Some(thread),
        }
    }

    pub fn send(&self, cmd: Command) {
        // Only fails if the controller thread is gone; nothing sensible to do then.
        let _ = self.tx.send(cmd);
    }

    pub fn snapshot(&self) -> Snapshot {
        lock(&self.shared).clone()
    }
}

impl PlayerHandle {
    /// Stops playback and waits until the playback thread has saved its state.
    pub fn shutdown(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for PlayerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Locks a mutex, recovering from poisoning (a panic elsewhere must not take the UI down).
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct Controller {
    ctx: egui::Context,
    rx: Receiver<Command>,
    shared: Arc<Mutex<Snapshot>>,

    sink: Option<MixerDeviceSink>,
    device_lost: Arc<AtomicBool>,
    player: Option<Player>,

    playlist: Vec<PathBuf>,
    index: Option<usize>,
    status: Status,
    track: Option<TrackInfo>,
    volume: f32,
    muted: bool,
    error: Option<String>,

    media_keys: Option<MediaKeys>,
    last_published: Snapshot,
    /// Last state written to disk, and when.
    saved_resume: Option<ResumeState>,
    last_resume_save: Instant,
}

impl Controller {
    fn new(
        ctx: egui::Context,
        rx: Receiver<Command>,
        tx: Sender<Command>,
        shared: Arc<Mutex<Snapshot>>,
        volume: f32,
        muted: bool,
    ) -> Self {
        let media_keys = MediaKeys::new(tx, ctx.clone());
        let mut c = Self {
            ctx,
            rx,
            shared,
            sink: None,
            device_lost: Arc::new(AtomicBool::new(false)),
            player: None,
            playlist: Vec::new(),
            index: None,
            status: Status::Stopped,
            track: None,
            volume: volume.clamp(0.0, 1.0),
            muted,
            error: None,
            media_keys,
            last_published: Snapshot::default(),
            saved_resume: None,
            last_resume_save: Instant::now(),
        };
        if let Some(mk) = &mut c.media_keys {
            mk.set_volume(c.volume, c.muted);
        }
        c.restore();
        c.publish();
        c
    }

    fn run(mut self) {
        loop {
            let timeout = if self.status == Status::Playing {
                Duration::from_millis(200)
            } else {
                Duration::from_secs(1)
            };
            match self.rx.recv_timeout(timeout) {
                Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(cmd) => self.handle(cmd),
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.tick();
            self.save_resume(false);
            self.publish();
        }
        self.save_resume(true);
        // Stop audio before the sink goes away.
        self.player = None;
        self.sink = None;
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::PlayList { tracks, index } => {
                self.playlist = tracks;
                self.play_from(index, true);
            }
            Command::Browse(tracks) => {
                if self.index.is_none() {
                    self.playlist = tracks;
                }
            }
            Command::TogglePause => match self.status {
                Status::Playing => self.pause(),
                _ => self.play(),
            },
            Command::Play => self.play(),
            Command::Pause => self.pause(),
            Command::Stop => self.stop(),
            Command::Next => {
                if let Some(i) = self.index
                    && i + 1 < self.playlist.len()
                {
                    self.play_from(i + 1, true);
                }
            }
            Command::Previous => self.previous(),
            Command::Seek(pos) => self.seek(pos),
            Command::SetVolume(v) => self.set_volume(v, false),
            Command::SeekBy(delta_ms) => {
                let pos = self.position().as_millis() as i64 + delta_ms;
                self.seek(Duration::from_millis(pos.max(0) as u64));
            }
            Command::VolumeStep(d) => self.set_volume(self.volume + d, false),
            Command::ToggleMute => self.set_volume(self.volume, !self.muted),
            Command::Shutdown => {}
        }
    }

    /// Periodic housekeeping: auto-advance and audio device recovery.
    fn tick(&mut self) {
        if self.device_lost.swap(false, Ordering::SeqCst) {
            self.recover_device();
        }
        if self.status == Status::Playing && self.player.as_ref().is_some_and(|p| p.empty()) {
            match self.index {
                Some(i) if i + 1 < self.playlist.len() => self.play_from(i + 1, true),
                _ => self.stop(),
            }
        }
    }

    fn ensure_sink(&mut self) -> Result<(), String> {
        if self.sink.is_none() {
            let flag = self.device_lost.clone();
            let mut sink = DeviceSinkBuilder::from_default_device()
                .and_then(|b| {
                    b.with_error_callback(move |err: rodio::cpal::Error| {
                        use rodio::cpal::ErrorKind as K;
                        eprintln!("simplemusicplayer: audio stream error: {err}");
                        if matches!(
                            err.kind(),
                            K::DeviceNotAvailable | K::HostUnavailable | K::StreamInvalidated
                        ) {
                            flag.store(true, Ordering::SeqCst);
                        }
                    })
                    .open_sink_or_fallback()
                })
                .map_err(|e| format!("Audioausgabe nicht verfügbar: {e}"))?;
            sink.log_on_drop(false);
            self.sink = Some(sink);
        }
        Ok(())
    }

    /// Starts the track at `index`. On failure, tries following tracks (or
    /// earlier ones if `forward` is false) so a single broken file does not stop playback.
    fn play_from(&mut self, index: usize, forward: bool) {
        let mut skipped: Vec<String> = Vec::new();
        let mut i = index;
        while i < self.playlist.len() {
            match self.start_track(i) {
                Ok(()) => {
                    self.error = (!skipped.is_empty())
                        .then(|| format!("Nicht abspielbar, übersprungen: {}", skipped.join(", ")));
                    return;
                }
                Err(e) => {
                    let name = display_stem(&self.playlist[i]);
                    eprintln!("simplemusicplayer: {name}: {e}");
                    // A missing audio device affects every track; do not loop over all of them.
                    if self.sink.is_none() {
                        self.halt();
                        self.error = Some(e);
                        return;
                    }
                    skipped.push(name);
                }
            }
            if forward {
                i += 1;
            } else if i == 0 {
                break;
            } else {
                i -= 1;
            }
        }
        self.halt();
        self.error = Some(format!("Nicht abspielbar: {}", skipped.join(", ")));
    }

    fn start_track(&mut self, index: usize) -> Result<(), String> {
        self.load_track(index, false)
    }

    /// Loads the track at `index`; with `paused` it is ready but silent (no audible blip).
    fn load_track(&mut self, index: usize, paused: bool) -> Result<(), String> {
        let path = self.playlist[index].clone();
        // Stop the old track first so they never overlap.
        self.player = None;
        self.ensure_sink()?;

        let source = open_source(&path)?;
        let decoder_duration = source.total_duration();
        let sink = self.sink.as_ref().expect("ensure_sink succeeded");
        let player = Player::connect_new(sink.mixer());
        let amp = self.amplitude();
        player.set_volume(amp);
        if paused {
            player.pause();
        }
        player.append(source);

        self.player = Some(player);
        self.index = Some(index);
        self.status = if paused {
            Status::Paused
        } else {
            Status::Playing
        };
        self.track = Some(read_track_info(&path, decoder_duration));
        self.update_media_keys_metadata();
        Ok(())
    }

    fn play(&mut self) {
        match self.status {
            Status::Playing => {}
            Status::Paused => {
                if let Some(p) = &self.player {
                    p.play();
                    self.status = Status::Playing;
                }
            }
            Status::Stopped => {
                let i = self.index.unwrap_or(0);
                if i < self.playlist.len() {
                    self.play_from(i, true);
                }
            }
        }
    }

    fn pause(&mut self) {
        if self.status == Status::Playing
            && let Some(p) = &self.player
        {
            p.pause();
            self.status = Status::Paused;
        }
    }

    /// Stops playback but keeps the current track selected (Play restarts it).
    fn stop(&mut self) {
        self.player = None;
        self.status = Status::Stopped;
    }

    /// Stops after an error; the audio device is released so it can be reopened later.
    fn halt(&mut self) {
        self.stop();
        self.sink = None;
    }

    fn previous(&mut self) {
        let pos = self.position();
        match self.index {
            Some(i) if i > 0 && pos < RESTART_THRESHOLD => self.play_from(i - 1, false),
            Some(_) if self.status == Status::Stopped => self.play(),
            Some(_) => self.seek(Duration::ZERO),
            None => {}
        }
    }

    fn seek(&mut self, pos: Duration) {
        if let Some(p) = &self.player {
            if let Err(e) = p.try_seek(pos) {
                self.error = Some(format!("Spulen nicht möglich: {e}"));
            }
            if let Some(mk) = &mut self.media_keys {
                mk.set_status(self.status, pos);
            }
        }
    }

    fn set_volume(&mut self, volume: f32, muted: bool) {
        self.volume = volume.clamp(0.0, 1.0);
        self.muted = muted;
        let amp = self.amplitude();
        if let Some(p) = &self.player {
            p.set_volume(amp);
        }
        if let Some(mk) = &mut self.media_keys {
            mk.set_volume(self.volume, self.muted);
        }
    }

    /// Perceptual volume curve: the slider feels linear to the ear.
    fn amplitude(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.volume * self.volume
        }
    }

    fn position(&self) -> Duration {
        self.player
            .as_ref()
            .map(|p| p.get_pos())
            .unwrap_or_default()
    }

    /// The audio device vanished (e.g. sound server restart): reopen and resume.
    fn recover_device(&mut self) {
        eprintln!("simplemusicplayer: audio device lost, reopening");
        let pos = self.position();
        let was_paused = self.status == Status::Paused;
        let had_track = self.player.is_some();
        self.player = None;
        self.sink = None;
        if !had_track {
            return;
        }
        let Some(i) = self.index else { return };
        match self.load_track(i, true) {
            Ok(()) => {
                self.seek(pos);
                if !was_paused {
                    self.play();
                }
            }
            Err(e) => {
                self.halt();
                self.error = Some(e);
            }
        }
    }

    /// Reopens the track that was loaded when the app was last closed, paused at
    /// the saved position. The rest of its folder becomes the playlist.
    fn restore(&mut self) {
        let Some(state) = resume::load() else { return };
        let Some(dir) = state.track.parent() else {
            return;
        };
        let Ok(listing) = crate::library::list_dir(dir) else {
            return;
        };
        self.playlist = listing.tracks.into_iter().map(|t| t.path).collect();
        let Some(i) = self.playlist.iter().position(|p| *p == state.track) else {
            self.playlist.clear();
            return;
        };
        match self.load_track(i, true) {
            Ok(()) => {
                self.seek(state.position());
                self.saved_resume = Some(state);
            }
            Err(e) => {
                eprintln!(
                    "simplemusicplayer: cannot restore {}: {e}",
                    state.track.display()
                );
                self.halt();
                self.playlist.clear();
                self.index = None;
                self.track = None;
            }
        }
    }

    /// Persists track + position whenever it changed meaningfully. While playing,
    /// writes are throttled unless `force` is set.
    fn save_resume(&mut self, force: bool) {
        let current = match (&self.player, &self.track) {
            (Some(_), Some(t)) if self.status != Status::Stopped => {
                Some(ResumeState::new(t.path.clone(), self.position()))
            }
            _ => None,
        };
        if current == self.saved_resume {
            return;
        }
        let same_track =
            current.as_ref().map(|c| &c.track) == self.saved_resume.as_ref().map(|s| &s.track);
        let throttled = self.status == Status::Playing
            && same_track
            && self.last_resume_save.elapsed() < RESUME_SAVE_INTERVAL;
        if throttled && !force {
            return;
        }
        resume::store(current.as_ref());
        self.saved_resume = current;
        self.last_resume_save = Instant::now();
    }

    fn update_media_keys_metadata(&mut self) {
        if let (Some(mk), Some(t)) = (&mut self.media_keys, &self.track) {
            mk.set_track(t);
        }
    }

    fn publish(&mut self) {
        let snap = Snapshot {
            status: self.status,
            track: self.track.clone(),
            position: self.position(),
            has_next: self.index.is_some_and(|i| i + 1 < self.playlist.len()),
            has_previous: self.index.is_some(),
            volume: self.volume,
            muted: self.muted,
            error: self.error.clone(),
            media_keys_active: self.media_keys.is_some(),
        };
        if snap == self.last_published {
            return;
        }
        if snap.status != self.last_published.status
            && let Some(mk) = &mut self.media_keys
        {
            mk.set_status(snap.status, snap.position);
        }
        *lock(&self.shared) = snap.clone();
        self.last_published = snap;
        self.ctx.request_repaint();
    }
}

/// Opens an audio file: rodio's decoder for most formats, libopus for Ogg/Opus.
fn open_source(path: &Path) -> Result<Box<dyn Source + Send>, String> {
    let is_opus = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("opus"));
    if is_opus {
        return Ok(Box::new(OpusSource::open(path)?));
    }
    let file = std::fs::File::open(path).map_err(|e| format!("Datei nicht lesbar: {e}"))?;
    match Decoder::try_from(file) {
        Ok(d) => Ok(Box::new(d)),
        // An ".ogg" file may contain Opus instead of Vorbis.
        Err(e) => OpusSource::open(path)
            .map(|s| Box::new(s) as Box<dyn Source + Send>)
            .map_err(|_| format!("Format nicht unterstützt: {e}")),
    }
}

fn read_track_info(path: &Path, decoder_duration: Option<Duration>) -> TrackInfo {
    use lofty::prelude::*;

    let mut info = TrackInfo {
        path: path.to_path_buf(),
        title: display_stem(path),
        artist: None,
        album: None,
        duration: decoder_duration,
    };
    // Tags are optional; a file without (or with broken) tags still plays.
    if let Ok(tagged) = lofty::read_from_path(path) {
        let duration = tagged.properties().duration();
        if !duration.is_zero() {
            info.duration = Some(duration);
        }
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            let clean = |s: Option<std::borrow::Cow<'_, str>>| {
                s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
            };
            if let Some(title) = clean(tag.title()) {
                info.title = title;
            }
            info.artist = clean(tag.artist());
            info.album = clean(tag.album());
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decodes real files given in `SMP_TEST_FILES` (`:`-separated). Run with
    /// `SMP_TEST_FILES=a.mp3:b.opus cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn decode_real_files() {
        let files = std::env::var("SMP_TEST_FILES").unwrap_or_default();
        for f in files.split(':').filter(|f| !f.is_empty()) {
            let path = Path::new(f);
            let mut src = open_source(path).unwrap_or_else(|e| panic!("{f}: {e}"));
            let n = src.sample_rate().get() as usize * src.channels().get() as usize * 2;
            let got = src.by_ref().take(n).count();
            assert_eq!(got, n, "{f}: too few samples");
            src.try_seek(Duration::from_secs(20))
                .unwrap_or_else(|e| panic!("{f}: seek: {e}"));
            assert!(src.next().is_some(), "{f}: no samples after seek");
            let info = read_track_info(path, src.total_duration());
            println!(
                "{f}: {:?} / {:?} / {:?}",
                info.title, info.artist, info.duration
            );
        }
    }

    fn wait_for(h: &PlayerHandle, what: &str, f: impl Fn(&Snapshot) -> bool) -> Snapshot {
        let start = std::time::Instant::now();
        loop {
            let s = h.snapshot();
            if f(&s) {
                return s;
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "timeout waiting for {what}: {s:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Drives the real playback thread (muted) over a folder of short test tracks.
    /// `SMP_TEST_DIR=dir cargo test -- --ignored playback_flow`
    #[test]
    #[ignore]
    fn playback_flow() {
        let dir = std::env::var("SMP_TEST_DIR").expect("SMP_TEST_DIR");
        let listing = crate::library::list_dir(Path::new(&dir)).unwrap();
        let tracks: Vec<PathBuf> = listing.tracks.iter().map(|t| t.path.clone()).collect();
        let n = tracks.len();
        let name = |s: &Snapshot| {
            s.track
                .as_ref()
                .map(|t| display_stem(&t.path))
                .unwrap_or_default()
        };
        let h = PlayerHandle::spawn(egui::Context::default(), 0.5, true);

        h.send(Command::PlayList {
            tracks: tracks.clone(),
            index: 0,
        });
        let s = wait_for(&h, "playing", |s| s.status == Status::Playing);
        println!("start: {}", name(&s));
        wait_for(&h, "position advances", |s| {
            s.position > Duration::from_millis(300)
        });

        h.send(Command::Pause);
        let p1 = wait_for(&h, "paused", |s| s.status == Status::Paused).position;
        std::thread::sleep(Duration::from_millis(400));
        let p2 = h.snapshot().position;
        assert!(
            p2.saturating_sub(p1) < Duration::from_millis(100),
            "position moved while paused"
        );
        h.send(Command::TogglePause);
        wait_for(&h, "resumed", |s| s.status == Status::Playing);

        h.send(Command::Next);
        let s = wait_for(&h, "next", |s| name(s) == display_stem(&tracks[1]));
        println!("next: {}", name(&s));
        h.send(Command::Previous);
        wait_for(&h, "previous", |s| name(s) == display_stem(&tracks[0]));

        h.send(Command::Seek(Duration::from_millis(1000)));
        let s = wait_for(&h, "seek", |s| s.position >= Duration::from_millis(1000));
        println!("after seek: {:?}", s.position);

        // Auto-advance through the remaining tracks (one is broken and must be skipped).
        let end = wait_for(&h, "end of playlist", |s| s.status == Status::Stopped);
        println!(
            "ended on: {} (error: {:?}), {n} tracks",
            name(&end),
            end.error
        );
        assert_eq!(name(&end), display_stem(&tracks[n - 1]));

        // Changing the volume unmutes; mute toggles back.
        h.send(Command::VolumeStep(0.1));
        let s = wait_for(&h, "unmuted", |s| !s.muted);
        assert!((s.volume - 0.6).abs() < 1e-4);
        h.send(Command::ToggleMute);
        wait_for(&h, "muted", |s| s.muted);
    }

    /// Pause, quit, restart: the track must come back paused at the same position.
    /// `SMP_TEST_LONG_DIR=dir cargo test -- --ignored resume_flow` (tracks of several seconds).
    #[test]
    #[ignore]
    fn resume_flow() {
        let dir = std::env::var("SMP_TEST_LONG_DIR").expect("SMP_TEST_LONG_DIR");
        let state = std::env::temp_dir().join(format!("smp-resume-test-{}", std::process::id()));
        // SAFETY: this test runs alone (ignored test, single-threaded setup before spawning).
        unsafe { std::env::set_var("XDG_STATE_HOME", &state) };
        let listing = crate::library::list_dir(Path::new(&dir)).unwrap();
        let tracks: Vec<PathBuf> = listing.tracks.iter().map(|t| t.path.clone()).collect();
        assert!(tracks.len() >= 2);
        for idx in [0, 1] {
            let mut h = PlayerHandle::spawn(egui::Context::default(), 0.5, true);
            h.send(Command::PlayList {
                tracks: tracks.clone(),
                index: idx,
            });
            wait_for(&h, "position", |s| s.position > Duration::from_millis(1500));
            h.send(Command::Pause);
            let paused = wait_for(&h, "paused", |s| s.status == Status::Paused);
            h.shutdown();

            let h = PlayerHandle::spawn(egui::Context::default(), 0.5, true);
            let s = wait_for(&h, "restored", |s| s.track.is_some());
            println!(
                "restored: {:?} at {:?} (was {:?})",
                s.track.as_ref().map(|t| &t.path),
                s.position,
                paused.position
            );
            assert_eq!(s.status, Status::Paused);
            assert_eq!(
                s.track.as_ref().map(|t| t.path.clone()),
                Some(tracks[idx].clone())
            );
            let diff = s.position.abs_diff(paused.position);
            assert!(
                diff < Duration::from_millis(300),
                "position off by {diff:?}"
            );
            assert!(s.has_next == (idx + 1 < tracks.len()));
            h.send(Command::TogglePause);
            wait_for(&h, "playing again", |s| {
                s.status == Status::Playing && s.position > paused.position
            });
            h.send(Command::Stop);
            wait_for(&h, "stopped", |s| s.status == Status::Stopped);
        }
        // Stopped at exit: nothing to restore.
        let h = PlayerHandle::spawn(egui::Context::default(), 0.5, true);
        std::thread::sleep(Duration::from_millis(500));
        assert!(h.snapshot().track.is_none());
        drop(h);
        let _ = std::fs::remove_dir_all(&state);
    }
}
