//! Local music: songs from this computer, played straight from their files — nothing
//! is uploaded, and a FLAC plays as the FLAC it is (Library › Uploads › Local music).
//!
//! `local.json` in the data dir lists them in the order they were added. A file that
//! was moved or deleted keeps its place: like a download on a drive that's away, it
//! comes back when the file does, and until then playing it just fails.

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha1::Digest;

use crate::model::Track;
use crate::stream::{Format, Shared};

/// The id every local song plays under, with the file's digest after it.
pub const PREFIX: &str = "local:";

/// What rodio opens with a hint (see `Format::File`): the extensions local music takes.
/// WMA is missing on purpose — YouTube Music takes it for upload, nothing here decodes it.
const EXTS: &[&str] = &["mp3", "m4a", "mp4", "m4b", "aac", "flac", "ogg", "oga", "wav", "webm", "mka", "mkv"];

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    /// The song as its tags told it, for lists and the player.
    track: Track,
}

struct State {
    /// The songs in the order they were added, the way the page lists them.
    index: Vec<Entry>,
    /// Where the index is saved (the data dir's local.json).
    file: PathBuf,
}

static STATE: LazyLock<RwLock<State>> =
    LazyLock::new(|| RwLock::new(State { index: Vec::new(), file: PathBuf::new() }));

/// Bumped whenever the index changes, so the page can pick up files added on the
/// picker's thread without reading them every frame.
static REVISED: AtomicU64 = AtomicU64::new(0);

/// How many changes the index has had: a number the page hasn't seen means new files.
pub fn revision() -> u64 {
    REVISED.load(Ordering::Relaxed)
}

/// Load the index.
pub fn init(data_dir: &Path) {
    let file = data_dir.join("local.json");
    let index = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut st = STATE.write().unwrap();
    st.index = index;
    st.file = file;
}

/// Save the index whole or not at all: a crash mid-write doesn't lose it. Called after
/// a change, with `STATE` let go, so the file is written without holding anyone up.
fn write_index() {
    static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (json, file) = {
        let st = STATE.read().unwrap();
        (serde_json::to_vec(&st.index), st.file.clone())
    };
    let Ok(json) = json else { return };
    let tmp = file.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, json).and_then(|_| std::fs::rename(&tmp, &file)) {
        log::warn!("couldn't save the local music index: {e}");
    }
}

/// Whether `id` is one of this computer's songs (anything after the prefix is opaque).
pub fn is_local(id: &str) -> bool {
    id.starts_with(PREFIX)
}

/// The songs as the page lists them, in the order they were added.
pub fn tracks() -> Vec<Track> {
    STATE.read().unwrap().index.iter().map(|e| e.track.clone()).collect()
}

/// One song's file, for Show in folder.
pub fn path(id: &str) -> Option<PathBuf> {
    STATE.read().unwrap().index.iter().find(|e| e.track.id == id).map(|e| e.path.clone())
}

/// A song's audio, as a complete stream buffer for playback. Nothing is transcoded:
/// the bytes go in as they are on disk, and the decoder reads the format from them.
pub fn open(id: &str) -> Option<Arc<Shared>> {
    let entry = STATE.read().unwrap().index.iter().find(|e| e.track.id == id).cloned()?;
    let data = std::fs::read(&entry.path).ok()?;
    let duration_ms = entry.track.duration.map(|s| s as u64 * 1000).unwrap_or(0);
    let ext = entry.path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    log::info!("local {}: playing {}", entry.track.title, entry.path.display());
    Some(Shared::from_bytes_with_format(data, duration_ms, None, Format::File { hint: hint(&ext), mime: None }, None))
}

/// Take files in: each is read for its tags and duration (a header's worth of disk
/// work, so this runs off the UI thread), skipping what isn't audio. Returns the
/// number added; a file already in the list by the same path isn't added twice.
pub fn add(paths: Vec<PathBuf>) -> usize {
    let mut added = 0;
    {
        let mut st = STATE.write().unwrap();
        for path in paths {
            let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
            if !EXTS.contains(&ext.as_str()) || !path.is_file() {
                continue;
            }
            if st.index.iter().any(|e| e.path == path) {
                continue;
            }
            let (mut track, duration) = read_tags(&path, &ext);
            track.id = format!("{PREFIX}{:x}", sha1::Sha1::digest(path.to_string_lossy().as_bytes()));
            track.duration = duration;
            st.index.push(Entry { path, track });
            added += 1;
        }
    }
    if added > 0 {
        REVISED.fetch_add(1, Ordering::Relaxed);
        write_index();
    }
    added
}

/// Take a song out of the list. Its file is left alone.
pub fn remove(id: &str) {
    let changed = {
        let mut st = STATE.write().unwrap();
        let before = st.index.len();
        st.index.retain(|e| e.track.id != id);
        before != st.index.len()
    };
    if changed {
        REVISED.fetch_add(1, Ordering::Relaxed);
        write_index();
    }
}

/// One file's song: its tags where they say something, the file's name where they
/// don't, and its length in seconds (unknown when the header doesn't tell).
fn read_tags(path: &Path, ext: &str) -> (Track, Option<u32>) {
    let mut track = Track { id: String::new(), title: String::new(), artists: Vec::new(), album: None, duration: None, thumbs: Vec::new(), track_nr: None, plays: None, music_video: false, video: false, explicit: false };
    let mut duration = None;
    // The tags and the length both sit in the container's header: one open reads both.
    match std::fs::File::open(path) {
        Ok(file) => {
            let mss = symphonia::core::io::MediaSourceStream::new(Box::new(file), Default::default());
            let mut hint = symphonia::core::probe::Hint::new();
            hint.with_extension(ext);
            match symphonia::default::get_probe().format(&hint, mss, &Default::default(), &Default::default()) {
                Ok(mut probed) => {
                    let params = probed.format.tracks().iter().find(|t| t.codec_params.sample_rate.is_some()).map(|t| t.codec_params.clone());
                    // A timebase in ticks makes frames into seconds; without one (or a
                    // length), the header just doesn't say how long the song is.
                    if let Some(p) = params
                        && let (Some(frames), Some(base)) = (p.n_frames, p.time_base)
                    {
                        let t = base.calc_time(frames);
                        duration = Some(t.seconds.min(u32::MAX as u64) as u32);
                    }
                    // The container's own tags, or the ones the probe read ahead of it.
                    let tags = probed.format.metadata().current().map(|m| m.tags().to_vec());
                    let tags = tags.or_else(|| probed.metadata.get().and_then(|m| m.current().map(|r| r.tags().to_vec())));
                    if let Some(tags) = tags {
                        let tag = |key: symphonia::core::meta::StandardTagKey| {
                            tags.iter().find_map(|t| match (t.std_key == Some(key), &t.value) {
                                // A WAV's INFO tags can carry their NUL terminator; trim it.
                                (true, symphonia::core::meta::Value::String(v)) => Some(v.trim_matches(|c: char| c.is_whitespace() || c == '\0')).filter(|v| !v.is_empty()).map(str::to_owned),
                                _ => None,
                            })
                        };
                        track.title = tag(symphonia::core::meta::StandardTagKey::TrackTitle).unwrap_or_default();
                        if let Some(artist) = tag(symphonia::core::meta::StandardTagKey::Artist) {
                            track.artists.push(crate::model::Link { id: None, name: artist });
                        }
                        track.album = tag(symphonia::core::meta::StandardTagKey::Album).map(|name| crate::model::Link { id: None, name });
                    }
                }
                Err(e) => log::warn!("local {}: couldn't read the header: {e}", path.display()),
            }
        }
        Err(e) => log::warn!("local {}: couldn't open: {e}", path.display()),
    }
    // A tagless file still has its name: "Artist - Title" splits, the rest is the title.
    if track.title.is_empty() {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().trim();
        match stem.split_once(" - ") {
            Some((artist, title)) if !title.trim().is_empty() => {
                track.artists.push(crate::model::Link { id: None, name: artist.trim().to_owned() });
                track.title = title.trim().to_owned();
            }
            _ => track.title = stem.to_owned(),
        }
    }
    if track.title.is_empty() {
        track.title = path.file_name().and_then(|s| s.to_str()).unwrap_or("Local file").to_owned();
    }
    (track, duration)
}

/// The extension as rodio's hint wants it (a file name's worth of format names).
fn hint(ext: &str) -> &'static str {
    match ext {
        "m4a" | "m4b" | "mp4" => "m4a",
        "oga" => "ogg",
        "mka" | "mkv" => "webm",
        other => EXTS.iter().find(|e| **e == other).copied().unwrap_or("mp3"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_ids_are_marked() {
        assert!(is_local("local:1234"));
        assert!(!is_local("dQw4w9WgXcQ"));
    }

    #[test]
    fn hints_match_the_extensions_taken() {
        for ext in EXTS {
            assert!(EXTS.contains(&hint(ext)), "{ext} has a hint");
        }
        assert_eq!(hint("m4b"), "m4a");
        assert_eq!(hint("oga"), "ogg");
    }
}
