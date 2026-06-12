//! The recorder: turns a stream of decode events into a tagged FLAC library.
//!
//! The decode thread emits record events (a song began, here are its samples, it
//! ended cleanly, or it was interrupted). A dedicated recorder thread accumulates
//! each song's PCM and, on a clean finish, encodes and writes it — so the
//! CPU-heavy encode never runs on the playback path.
//!
//! Correctness (see `tasks/recording-design.md`): a song is written only if it was
//! captured from its start *and* ended cleanly; writes are atomic (temp file then
//! rename) so the library never contains a partial file.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::flac::{encode_flac, Picture};
use crate::pcm::PcmChunk;

/// What the recorder does with completed songs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordMode {
    /// Not recording.
    #[default]
    Off,
    /// Keep a rolling cache of the most recent songs; promote with "keep".
    Cache,
    /// Write every completed song straight to the library.
    Session,
}

impl RecordMode {
    /// The next mode when cycling Off → Cache → Session → Off.
    pub fn next(self) -> RecordMode {
        match self {
            RecordMode::Off => RecordMode::Cache,
            RecordMode::Cache => RecordMode::Session,
            RecordMode::Session => RecordMode::Off,
        }
    }

    /// A short label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            RecordMode::Off => "off",
            RecordMode::Cache => "cache",
            RecordMode::Session => "session",
        }
    }
}

/// Recorder configuration.
#[derive(Debug, Clone)]
pub struct RecordingConfig {
    /// Starting mode.
    pub mode: RecordMode,
    /// Library root directory.
    pub root: PathBuf,
    /// How many songs the rolling cache retains.
    pub cache_size: usize,
    /// Whether to download and embed cover art.
    pub embed_artwork: bool,
    /// Skip writing a library file that already exists.
    pub deduplicate: bool,
}

/// In-band metadata for a song, read from the stream.
#[derive(Debug, Clone, Default)]
pub struct SongTags {
    /// Performing artist.
    pub artist: Option<String>,
    /// Album title.
    pub album: Option<String>,
    /// Track title.
    pub title: Option<String>,
}

/// An event from the decode path to the recorder.
pub(crate) enum RecordEvent {
    /// A new song started. `savable` is false for the song already in progress when
    /// recording (or the stream) began — we joined it mid-way, so it's incomplete.
    Begin { tags: SongTags, savable: bool },
    /// Decoded audio for the current song.
    Samples(PcmChunk),
    /// The current song ended cleanly at a boundary.
    Finish,
    /// The current song was interrupted; discard it.
    Abort,
    /// Promote the most recently cached song into the library.
    Keep,
    /// Change what completed songs are done with.
    SetMode(RecordMode),
}

/// Handle the decode path uses to feed the recorder. Cloneable and cheap.
///
/// This is internal plumbing that appears in [`OpusPcmSource::open`](crate::sources::OpusPcmSource::open);
/// it has no public constructor, so external callers only ever pass `None`.
#[doc(hidden)]
#[derive(Clone)]
pub struct RecordHandle {
    tx: Sender<RecordEvent>,
    active: Arc<AtomicBool>,
}

impl RecordHandle {
    /// Whether recording is currently on (so the source can skip cloning samples).
    pub(crate) fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    pub(crate) fn begin(&self, tags: SongTags, savable: bool) {
        let _ = self.tx.send(RecordEvent::Begin { tags, savable });
    }

    pub(crate) fn samples(&self, chunk: PcmChunk) {
        let _ = self.tx.send(RecordEvent::Samples(chunk));
    }

    pub(crate) fn finish(&self) {
        let _ = self.tx.send(RecordEvent::Finish);
    }

    pub(crate) fn abort(&self) {
        let _ = self.tx.send(RecordEvent::Abort);
    }
}

/// Owns the recorder thread and the controls the player exposes.
pub(crate) struct Recorder {
    tx: Sender<RecordEvent>,
    active: Arc<AtomicBool>,
    /// Current now-playing artwork URL, snapshotted per song when it begins.
    artwork: Arc<Mutex<Option<String>>>,
    mode: RecordMode,
    _thread: std::thread::JoinHandle<()>,
}

impl Recorder {
    /// Spawn the recorder thread.
    pub(crate) fn spawn(config: RecordingConfig) -> Self {
        let (tx, rx) = channel();
        let active = Arc::new(AtomicBool::new(config.mode != RecordMode::Off));
        let artwork = Arc::new(Mutex::new(None));
        let mode = config.mode;
        let art_for_thread = Arc::clone(&artwork);
        let thread = std::thread::Builder::new()
            .name("plaza-recorder".to_string())
            .spawn(move || recorder_loop(rx, config, art_for_thread))
            .expect("spawning a thread should not fail");
        Recorder {
            tx,
            active,
            artwork,
            mode,
            _thread: thread,
        }
    }

    /// A handle for the decode path to feed events through.
    pub(crate) fn handle(&self) -> RecordHandle {
        RecordHandle {
            tx: self.tx.clone(),
            active: Arc::clone(&self.active),
        }
    }

    pub(crate) fn mode(&self) -> RecordMode {
        self.mode
    }

    /// Change the recording mode, returning the new mode.
    pub(crate) fn set_mode(&mut self, mode: RecordMode) {
        self.mode = mode;
        self.active
            .store(mode != RecordMode::Off, Ordering::Relaxed);
        let _ = self.tx.send(RecordEvent::SetMode(mode));
    }

    /// Promote the most recently cached song into the library.
    pub(crate) fn keep(&self) {
        let _ = self.tx.send(RecordEvent::Keep);
    }

    /// Update the now-playing artwork URL used to tag newly started songs.
    pub(crate) fn set_artwork(&self, url: Option<String>) {
        if let Ok(mut g) = self.artwork.lock() {
            *g = url;
        }
    }
}

/// A song being accumulated in memory.
struct Current {
    tags: SongTags,
    savable: bool,
    artwork_url: Option<String>,
    samples: Vec<f32>,
    sample_rate: u32,
    channels: u16,
}

fn recorder_loop(
    rx: Receiver<RecordEvent>,
    config: RecordingConfig,
    artwork: Arc<Mutex<Option<String>>>,
) {
    let mut sink = RecordingSink::new(config);
    let mut current: Option<Current> = None;

    for event in rx {
        match event {
            RecordEvent::Begin { tags, savable } => {
                // A new song beginning means any unfinished one was interrupted.
                current = Some(Current {
                    tags,
                    savable,
                    artwork_url: artwork.lock().ok().and_then(|g| g.clone()),
                    samples: Vec::new(),
                    sample_rate: 0,
                    channels: 0,
                });
            }
            RecordEvent::Samples(chunk) => {
                if let Some(c) = &mut current {
                    c.sample_rate = chunk.sample_rate;
                    c.channels = chunk.channels;
                    c.samples.extend_from_slice(&chunk.samples);
                }
            }
            RecordEvent::Finish => {
                if let Some(c) = current.take() {
                    if c.savable && !c.samples.is_empty() {
                        if let Err(e) = sink.write_song(&c) {
                            tracing::warn!("recording: failed to save song: {e}");
                        }
                    }
                }
            }
            RecordEvent::Abort => current = None,
            RecordEvent::Keep => sink.keep_last(),
            RecordEvent::SetMode(mode) => {
                sink.mode = mode;
                if mode == RecordMode::Off {
                    current = None;
                }
            }
        }
    }
    // Channel closed: any in-progress song is incomplete and is dropped unsaved.
}

/// Writes finished songs to disk and manages the rolling cache.
struct RecordingSink {
    mode: RecordMode,
    root: PathBuf,
    cache_size: usize,
    embed_artwork: bool,
    deduplicate: bool,
    /// Cache files in eviction order (oldest first).
    cache: VecDeque<PathBuf>,
    /// The most recently written cache file and its tags, for "keep".
    last_cached: Option<(PathBuf, SongTags)>,
    http: reqwest::blocking::Client,
}

impl RecordingSink {
    fn new(config: RecordingConfig) -> Self {
        RecordingSink {
            mode: config.mode,
            root: config.root,
            cache_size: config.cache_size.max(1),
            embed_artwork: config.embed_artwork,
            deduplicate: config.deduplicate,
            cache: VecDeque::new(),
            last_cached: None,
            http: reqwest::blocking::Client::builder()
                .user_agent(concat!("plaza-tui/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_default(),
        }
    }

    fn write_song(&mut self, song: &Current) -> super::Result<()> {
        match self.mode {
            RecordMode::Off => Ok(()),
            RecordMode::Session => {
                let dest = library_path(&self.root, &song.tags);
                if self.deduplicate && dest.exists() {
                    return Ok(());
                }
                self.encode_to(song, &dest)
            }
            RecordMode::Cache => {
                let dest = cache_path(&self.root, &song.tags);
                self.encode_to(song, &dest)?;
                self.cache.retain(|p| p != &dest);
                self.cache.push_back(dest.clone());
                self.last_cached = Some((dest, song.tags.clone()));
                while self.cache.len() > self.cache_size {
                    if let Some(old) = self.cache.pop_front() {
                        let _ = std::fs::remove_file(old);
                    }
                }
                Ok(())
            }
        }
    }

    /// Promote the most recently cached song into the permanent library.
    fn keep_last(&mut self) {
        let Some((cache_path, tags)) = self.last_cached.clone() else {
            return;
        };
        let dest = library_path(&self.root, &tags);
        if let Some(parent) = dest.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        if std::fs::rename(&cache_path, &dest).is_ok() {
            self.cache.retain(|p| p != &cache_path);
            self.last_cached = None;
            tracing::info!("recording: kept {}", dest.display());
        }
    }

    /// Encode a song and write it atomically (temp file then rename).
    fn encode_to(&self, song: &Current, dest: &Path) -> super::Result<()> {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut comments: Vec<(&str, &str)> = Vec::new();
        if let Some(a) = &song.tags.artist {
            comments.push(("ARTIST", a));
        }
        if let Some(a) = &song.tags.album {
            comments.push(("ALBUM", a));
        }
        if let Some(t) = &song.tags.title {
            comments.push(("TITLE", t));
        }
        comments.push(("SOURCE", "Nightwave Plaza"));

        let cover = if self.embed_artwork {
            song.artwork_url
                .as_deref()
                .and_then(|u| self.fetch_cover(u))
        } else {
            None
        };

        let flac = encode_flac(
            &song.samples,
            song.channels,
            song.sample_rate,
            &comments,
            cover.as_ref(),
        )?;

        let tmp = dest.with_extension("flac.part");
        std::fs::write(&tmp, &flac)?;
        std::fs::rename(&tmp, dest)?;
        tracing::info!("recording: wrote {}", dest.display());
        Ok(())
    }

    fn fetch_cover(&self, url: &str) -> Option<Picture> {
        let resp = self.http.get(url).send().ok()?.error_for_status().ok()?;
        let mime = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
            .unwrap_or_else(|| "image/jpeg".to_string());
        let data = resp.bytes().ok()?.to_vec();
        if data.is_empty() {
            return None;
        }
        Some(Picture { mime, data })
    }
}

/// `<root>/<Artist>/<Album>/<Artist> - <Title>.flac`, each component sanitized.
fn library_path(root: &Path, tags: &SongTags) -> PathBuf {
    let artist = sanitize(tags.artist.as_deref().unwrap_or("Unknown Artist"));
    let album = sanitize(tags.album.as_deref().unwrap_or("Unknown Album"));
    let title = sanitize(tags.title.as_deref().unwrap_or("Untitled"));
    root.join(&artist)
        .join(&album)
        .join(format!("{artist} - {title}.flac"))
}

/// A flat cache location: `<root>/.cache/<Artist> - <Title>.flac`.
fn cache_path(root: &Path, tags: &SongTags) -> PathBuf {
    let artist = sanitize(tags.artist.as_deref().unwrap_or("Unknown Artist"));
    let title = sanitize(tags.title.as_deref().unwrap_or("Untitled"));
    root.join(".cache").join(format!("{artist} - {title}.flac"))
}

/// Make a string safe to use as a single path component.
fn sanitize(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    let capped: String = trimmed.chars().take(120).collect();
    let capped = capped.trim().to_string();
    if capped.is_empty() {
        "Unknown".to_string()
    } else {
        capped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(artist: &str, album: &str, title: &str) -> SongTags {
        SongTags {
            artist: Some(artist.into()),
            album: Some(album.into()),
            title: Some(title.into()),
        }
    }

    #[test]
    fn library_path_uses_artist_album_title_tree() {
        let p = library_path(
            Path::new("/m"),
            &tags("la trace", "LITE TOUCH", "Man Enough"),
        );
        assert_eq!(
            p,
            Path::new("/m/la trace/LITE TOUCH/la trace - Man Enough.flac")
        );
    }

    #[test]
    fn missing_tags_fall_back_to_placeholders() {
        let p = library_path(Path::new("/m"), &SongTags::default());
        assert_eq!(
            p,
            Path::new("/m/Unknown Artist/Unknown Album/Unknown Artist - Untitled.flac")
        );
    }

    #[test]
    fn sanitize_strips_path_separators_and_control_chars() {
        assert_eq!(sanitize("AC/DC"), "AC_DC");
        assert_eq!(sanitize("a\nb:c"), "a_b_c");
        assert_eq!(sanitize("  spaced.  "), "spaced");
        assert_eq!(sanitize(""), "Unknown");
        assert_eq!(sanitize("   "), "Unknown");
    }

    #[test]
    fn mode_cycles_off_cache_session() {
        assert_eq!(RecordMode::Off.next(), RecordMode::Cache);
        assert_eq!(RecordMode::Cache.next(), RecordMode::Session);
        assert_eq!(RecordMode::Session.next(), RecordMode::Off);
    }
}
