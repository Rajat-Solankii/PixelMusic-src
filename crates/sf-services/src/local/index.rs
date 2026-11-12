//! The scan: walk the chosen folders, read each music file's tags (`tags`), save the covers, and keep the
//! result as an index on disk so the next start shows the library at once and only reads files that changed
//! (size or modification time). Limits follow soundwave1: 8 folders deep, 20 000 files.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::artists::{artists_from_title, split_artists};
use super::tags::{self, FileTags};

pub const MAX_DEPTH: usize = 8;
pub const MAX_FILES: usize = 20_000;
/// bumped when what a scan stores changes, so an old index is read again from the files
const INDEX_VERSION: u32 = 1;
/// pictures beside the files when the file has none inside it
const FOLDER_COVERS: &[&str] = &["cover", "folder", "front", "album", "albumart"];

/// One scanned file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalTrack {
    /// 16 hex digits from the path; the URI is `local:track:<id>`
    pub id: String,
    pub path: String,
    pub size: u64,
    /// seconds since the epoch
    pub modified: u64,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub album_artist: String,
    pub track_no: u32,
    pub disc_no: u32,
    pub year: String,
    pub genre: String,
    pub duration_ms: i64,
    pub sample_rate: u32,
    pub bit_depth: u32,
    pub channels: u32,
    pub codec: String,
    pub lossless: bool,
    /// "FLAC 16/44.1", "MP3 192"
    pub quality: String,
    /// the saved cover file ("" — none)
    pub cover: String,
}

impl LocalTrack {
    pub fn uri(&self) -> String {
        format!("{}{}", super::LOCAL_TRACK, self.id)
    }

    /// The folder the file is in.
    pub fn folder(&self) -> &str {
        Path::new(&self.path).parent().and_then(Path::to_str).unwrap_or_default()
    }
}

/// The index file: the folders it was made from and what they held.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalIndex {
    pub version: u32,
    pub folders: Vec<String>,
    pub tracks: Vec<LocalTrack>,
}

impl LocalIndex {
    /// The saved index; empty when it is missing, unreadable or from another version.
    pub fn load(file: &Path) -> Self {
        let Ok(bytes) = std::fs::read(file) else { return Self::default() };
        match serde_json::from_slice::<Self>(&bytes) {
            Ok(index) if index.version == INDEX_VERSION => index,
            Ok(_) => Self::default(),
            Err(error) => {
                log::warn!("local files: unreadable index {}: {error}", file.display());
                Self::default()
            }
        }
    }

    /// Writes the index next to itself first, then moves it into place.
    pub fn save(&self, file: &Path) -> Result<(), String> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        }
        let bytes = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        let temp = file.with_extension("json.tmp");
        std::fs::write(&temp, bytes).map_err(|error| error.to_string())?;
        std::fs::rename(&temp, file).map_err(|error| error.to_string())
    }
}

/// What a scan reports while it runs.
pub trait Progress: Sync {
    fn found(&self, files: usize);
    fn read(&self, done: usize, total: usize);
}

impl Progress for () {
    fn found(&self, _: usize) {}
    fn read(&self, _: usize, _: usize) {}
}

/// Scans `folders`, reusing `previous` entries whose file did not change. `cancel` stops it (returns `None`).
pub fn scan(
    folders: &[String],
    previous: &LocalIndex,
    cover_dir: &Path,
    cancel: &AtomicBool,
    progress: &dyn Progress,
) -> Option<LocalIndex> {
    let files = collect(folders, cancel)?;
    progress.found(files.len());
    let known: HashMap<&str, &LocalTrack> = previous.tracks.iter().map(|track| (track.path.as_str(), track)).collect();
    let mut tracks = Vec::with_capacity(files.len());
    for (done, (path, size, modified)) in files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let key = path.to_string_lossy();
        let cover_alive = |track: &LocalTrack| track.cover.is_empty() || Path::new(&track.cover).is_file();
        match known.get(key.as_ref()) {
            Some(track) if track.size == *size && track.modified == *modified && cover_alive(track) => {
                tracks.push((*track).clone());
            }
            _ => match tags::read(path) {
                Ok(tags) => tracks.push(entry(path, *size, *modified, tags, cover_dir)),
                Err(error) => log::debug!("local files: skipped {}: {error}", path.display()),
            },
        }
        if done % 25 == 0 {
            progress.read(done, files.len());
        }
    }
    progress.read(files.len(), files.len());
    sort(&mut tracks);
    Some(LocalIndex { version: INDEX_VERSION, folders: folders.to_vec(), tracks })
}

/// Every music file under the folders: path, size, modification time; a file reached twice counts once.
fn collect(folders: &[String], cancel: &AtomicBool) -> Option<Vec<(PathBuf, u64, u64)>> {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out = Vec::new();
    for folder in folders {
        let mut stack: Vec<(PathBuf, usize)> = vec![(PathBuf::from(folder), 0)];
        while let Some((dir, depth)) = stack.pop() {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            let mut subdirs = Vec::new();
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = entry.metadata() else { continue };
                if meta.is_dir() {
                    let hidden = entry.file_name().to_str().is_some_and(|name| name.starts_with('.'));
                    if depth < MAX_DEPTH && !hidden {
                        subdirs.push(path);
                    }
                } else if meta.is_file() && tags::is_audio(&path) {
                    let real = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                    if seen.insert(real) {
                        let modified = meta.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok());
                        out.push((path, meta.len(), modified.map_or(0, |time| time.as_secs())));
                        if out.len() >= MAX_FILES {
                            return Some(out);
                        }
                    }
                }
            }
            // depth-first in name order, so the cap keeps whole folders
            subdirs.sort();
            stack.extend(subdirs.into_iter().rev().map(|sub| (sub, depth + 1)));
        }
    }
    Some(out)
}

fn entry(path: &Path, size: u64, modified: u64, tags: FileTags, cover_dir: &Path) -> LocalTrack {
    let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
    let folder_name = path.parent().and_then(Path::file_name).and_then(|name| name.to_str()).unwrap_or_default();
    let (title, featured) = artists_from_title(if tags.title.is_empty() { stem } else { &tags.title });
    let mut artists = split_artists(&tags.artist);
    for name in featured {
        if !artists.iter().any(|seen| seen.eq_ignore_ascii_case(&name)) {
            artists.push(name);
        }
    }
    let album = if tags.album.is_empty() { folder_name.to_owned() } else { tags.album.clone() };
    let bitrate_kbps = if tags.duration_ms > 0 { (size * 8 / tags.duration_ms as u64) as u32 } else { 0 };
    let quality = tags::quality_label(&tags.codec, tags.lossless, tags.bit_depth, tags.sample_rate, bitrate_kbps);
    let id = track_id(path);
    let album_key = format!("{}\u{1}{}\u{1}{}", path.parent().map(Path::to_string_lossy).unwrap_or_default(), album, tags.album_artist);
    let cover = match &tags.cover {
        Some((mime, bytes)) => save_cover(cover_dir, &hash_hex(album_key.as_bytes()), mime, bytes),
        None => folder_cover(path),
    };
    LocalTrack {
        id,
        path: path.to_string_lossy().into_owned(),
        size,
        modified,
        title: if title.is_empty() { stem.to_owned() } else { title },
        artists,
        album,
        album_artist: tags.album_artist,
        track_no: tags.track_no,
        disc_no: tags.disc_no,
        year: tags.year,
        genre: tags.genre,
        duration_ms: tags.duration_ms,
        sample_rate: tags.sample_rate,
        bit_depth: tags.bit_depth,
        channels: tags.channels,
        codec: tags.codec,
        lossless: tags.lossless,
        quality,
        cover,
    }
}

/// Saves an embedded cover once per album; the path, or "" when it can't be written.
fn save_cover(dir: &Path, key: &str, mime: &str, bytes: &[u8]) -> String {
    let ext = match mime {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/gif" => "gif",
        _ => "jpg",
    };
    let file = dir.join(format!("{key}.{ext}"));
    if file.metadata().is_ok_and(|meta| meta.len() == bytes.len() as u64) {
        return file.to_string_lossy().into_owned();
    }
    let written = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&file, bytes));
    match written {
        Ok(()) => file.to_string_lossy().into_owned(),
        Err(error) => {
            log::warn!("local files: couldn't save a cover to {}: {error}", file.display());
            String::new()
        }
    }
}

/// cover.jpg / folder.png … beside the file.
fn folder_cover(path: &Path) -> String {
    let Some(dir) = path.parent() else { return String::new() };
    let Ok(entries) = std::fs::read_dir(dir) else { return String::new() };
    let mut pictures: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|file| {
            let stem = file.file_stem().and_then(|stem| stem.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
            let ext = tags::extension(file).unwrap_or_default();
            FOLDER_COVERS.contains(&stem.as_str()) && matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp")
        })
        .collect();
    pictures.sort_by_key(|file| {
        let stem = file.file_stem().and_then(|stem| stem.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        FOLDER_COVERS.iter().position(|name| *name == stem).unwrap_or(usize::MAX)
    });
    pictures.first().map(|file| file.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Artist, album, disc, track number, title (soundwave1 sorted artist, album, title).
pub fn sort(tracks: &mut [LocalTrack]) {
    tracks.sort_by_cached_key(|track| {
        let artist = if track.album_artist.is_empty() { track.artists.first().cloned().unwrap_or_default() } else { track.album_artist.clone() };
        (artist.to_lowercase(), track.album.to_lowercase(), track.disc_no, track.track_no, track.title.to_lowercase())
    });
}

/// The stable id of a file: FNV-1a over its path (the same path always gives the same id).
pub fn track_id(path: &Path) -> String {
    hash_hex(path.to_string_lossy().as_bytes())
}

fn hash_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}
