//! Music files on this computer (soundwave1's `LocalFiles`): folders the user picks are scanned for music,
//! tags and covers are read with symphonia (`tags`), the result is kept as an index on disk (`index`) and
//! shown as ordinary track rows with `local:track:<id>` URIs. They play in-process through the player's
//! YouTube lane (symphonia, the same output chain): [`LaneSource`] answers their streams with the file.
pub mod artists;
pub mod index;
pub mod source;
pub mod tags;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use md3_model::{ArtistRef, TrackItem};
use mf_platform::{SettingKey, SettingsStore};
use tokio::runtime::Handle;
use tokio::sync::{broadcast, watch};

pub use index::{LocalIndex, LocalTrack};
pub use sf_player::engine::youtube::{LOCAL_TRACK, is_local_track, local_id};
pub use source::LaneSource;

/// What the Local files page shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalSnapshot {
    /// the folders being shown (or scanned)
    pub folders: Vec<String>,
    pub tracks: Arc<Vec<LocalTrack>>,
    /// the saved index was read (until then the page is "loading")
    pub loaded: bool,
    pub scanning: bool,
    /// files found by the running scan
    pub found: usize,
    /// of those, files read so far
    pub read: usize,
    /// the last scan's failure ("" — none)
    pub error: String,
}

/// A folder of the library with what is in it, for a folder view (soundwave1's `FolderTreeBuilder`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FolderNode {
    pub path: String,
    pub name: String,
    /// tracks directly in this folder
    pub tracks: usize,
    /// tracks here and below
    pub total: usize,
    pub children: Vec<FolderNode>,
}

pub struct LocalFilesService {
    rt: Handle,
    index_file: PathBuf,
    cover_dir: PathBuf,
    snapshot: watch::Sender<LocalSnapshot>,
    by_id: Mutex<HashMap<String, usize>>,
    /// the saved index of other folders, for the next scan to reuse
    seed: Mutex<Vec<LocalTrack>>,
    /// the running scan's stop flag
    cancel: Mutex<Arc<AtomicBool>>,
    /// scans started; a scan only publishes while it is the newest
    generation: AtomicU64,
    /// one scan at a time (a newer one waits for the cancelled one to let go)
    scan_lock: tokio::sync::Mutex<()>,
}

impl LocalFilesService {
    /// The service over the app's own index (app data) and cover cache.
    pub fn new(rt: Handle) -> Arc<Self> {
        let index_file = mf_platform::dirs::app_data_dir().join("local_files.json");
        Self::with_paths(rt, index_file, mf_platform::dirs::cache_dir().join("local_covers"))
    }

    pub fn with_paths(rt: Handle, index_file: PathBuf, cover_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            rt,
            index_file,
            cover_dir,
            snapshot: watch::Sender::new(LocalSnapshot::default()),
            by_id: Mutex::new(HashMap::new()),
            seed: Mutex::new(Vec::new()),
            cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
            generation: AtomicU64::new(0),
            scan_lock: tokio::sync::Mutex::new(()),
        })
    }

    pub fn subscribe(&self) -> watch::Receiver<LocalSnapshot> {
        self.snapshot.subscribe()
    }

    pub fn snapshot(&self) -> LocalSnapshot {
        self.snapshot.borrow().clone()
    }

    /// Shows the saved index at once, then looks for changes in `folders` (the setting).
    pub fn start(self: &Arc<Self>, folders: Vec<String>) {
        let this = self.clone();
        self.rt.spawn(async move {
            let file = this.index_file.clone();
            let saved = tokio::task::spawn_blocking(move || LocalIndex::load(&file)).await.unwrap_or_default();
            let folders = normalise(folders);
            if saved.folders == folders {
                this.publish_index(saved.tracks, &folders);
            } else {
                // other folders now: not shown, but the files still in them aren't read again
                *this.seed.lock().expect("seed") = saved.tracks;
            }
            this.snapshot.send_modify(|snapshot| {
                snapshot.loaded = true;
                snapshot.folders = folders.clone();
            });
            if let Err(error) = this.scan(folders, false).await {
                log::warn!("local files: {error}");
            }
        });
    }

    /// The folders changed (Settings): scan them in the background.
    pub fn set_folders(self: &Arc<Self>, folders: Vec<String>) {
        let this = self.clone();
        self.rt.spawn(async move {
            if let Err(error) = this.scan(folders, false).await {
                log::warn!("local files: {error}");
            }
        });
    }

    /// Reads every file again ("Rescan").
    pub fn rescan(self: &Arc<Self>) {
        let this = self.clone();
        let folders = self.snapshot.borrow().folders.clone();
        self.rt.spawn(async move {
            if let Err(error) = this.scan(folders, true).await {
                log::warn!("local files: {error}");
            }
        });
    }

    /// Scans `folders` (reusing unchanged files unless `force`), publishes and saves the result; the number
    /// of tracks found. A newer scan cancels this one (`Err`).
    pub async fn scan(self: &Arc<Self>, folders: Vec<String>, force: bool) -> Result<usize, String> {
        let folders = normalise(folders);
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let cancel = {
            let mut running = self.cancel.lock().expect("cancel");
            running.store(true, Ordering::Relaxed);
            *running = Arc::new(AtomicBool::new(false));
            running.clone()
        };
        let _one = self.scan_lock.lock().await;
        if self.generation.load(Ordering::SeqCst) != generation {
            return Err(cancelled());
        }
        self.snapshot.send_modify(|snapshot| {
            snapshot.folders = folders.clone();
            snapshot.scanning = true;
            snapshot.found = 0;
            snapshot.read = 0;
            snapshot.error.clear();
        });
        let seed = std::mem::take(&mut *self.seed.lock().expect("seed"));
        let previous = if force {
            LocalIndex::default()
        } else {
            let mut tracks = self.snapshot.borrow().tracks.as_ref().clone();
            tracks.extend(seed);
            LocalIndex { tracks, ..Default::default() }
        };
        let (index_file, cover_dir, sender) = (self.index_file.clone(), self.cover_dir.clone(), self.snapshot.clone());
        let scan_folders = folders.clone();
        let result = tokio::task::spawn_blocking(move || {
            let progress = SnapshotProgress(sender);
            let index = index::scan(&scan_folders, &previous, &cover_dir, &cancel, &progress)?;
            if let Err(error) = index.save(&index_file) {
                log::warn!("local files: couldn't save the index {}: {error}", index_file.display());
            }
            Some(index)
        })
        .await
        .map_err(|error| error.to_string());
        let current = self.generation.load(Ordering::SeqCst) == generation;
        match result {
            Ok(Some(index)) if current => {
                let count = index.tracks.len();
                self.publish_index(index.tracks, &folders);
                self.snapshot.send_modify(|snapshot| snapshot.scanning = false);
                Ok(count)
            }
            Ok(_) => Err(cancelled()),
            Err(error) => {
                if current {
                    self.snapshot.send_modify(|snapshot| {
                        snapshot.scanning = false;
                        snapshot.error = error.clone();
                    });
                }
                Err(error)
            }
        }
    }

    fn publish_index(&self, tracks: Vec<LocalTrack>, folders: &[String]) {
        let by_id: HashMap<String, usize> =
            tracks.iter().enumerate().map(|(row, track)| (track.id.clone(), row)).collect();
        *self.by_id.lock().expect("by_id") = by_id;
        let tracks = Arc::new(tracks);
        self.snapshot.send_modify(|snapshot| {
            snapshot.tracks = tracks;
            snapshot.folders = folders.to_vec();
        });
    }

    /// The scanned file of an id (or of a `local:track:` URI).
    pub fn track(&self, id_or_uri: &str) -> Option<LocalTrack> {
        let id = local_id(id_or_uri).unwrap_or(id_or_uri);
        let row = *self.by_id.lock().expect("by_id").get(id)?;
        self.snapshot.borrow().tracks.get(row).cloned()
    }

    /// Every track as rows for the track lists.
    pub fn items(&self) -> Vec<TrackItem> {
        self.snapshot.borrow().tracks.iter().map(to_item).collect()
    }

    /// The folders with their tracks, under the chosen folders.
    pub fn folder_tree(&self) -> Vec<FolderNode> {
        let snapshot = self.snapshot.borrow();
        folder_tree(&snapshot.folders, &snapshot.tracks)
    }
}

/// Starts the service with the LocalFolders setting and scans again whenever it changes.
pub fn follow_folders_setting(rt: &Handle, settings: &Arc<SettingsStore>, local: Arc<LocalFilesService>) {
    local.start(folders_from_setting(&settings.get_str(SettingKey::LocalFolders)));
    let settings = settings.clone();
    let mut changes = settings.subscribe();
    rt.spawn(async move {
        loop {
            match changes.recv().await {
                Ok(SettingKey::LocalFolders) | Err(broadcast::error::RecvError::Lagged(_)) => {
                    let folders = folders_from_setting(&settings.get_str(SettingKey::LocalFolders));
                    if folders != local.snapshot().folders {
                        local.set_folders(folders);
                    }
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn cancelled() -> String {
    "scan replaced by a newer one".into()
}

struct SnapshotProgress(watch::Sender<LocalSnapshot>);

impl index::Progress for SnapshotProgress {
    fn found(&self, files: usize) {
        self.0.send_modify(|snapshot| snapshot.found = files);
    }

    fn read(&self, done: usize, _total: usize) {
        self.0.send_modify(|snapshot| snapshot.read = done);
    }
}

/// Trimmed, without empties or repeats, in the order given.
pub fn normalise(folders: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for folder in folders {
        let mut folder = folder.trim().trim_end_matches(['/', '\\']).to_owned();
        if folder.ends_with(':') {
            // "C:\" stays a drive root ("C:" alone is the drive's current folder)
            folder.push(std::path::MAIN_SEPARATOR);
        }
        if !folder.is_empty() && !out.contains(&folder) {
            out.push(folder);
        }
    }
    out
}

/// The folders setting (one path per line) as a list, and back.
pub fn folders_from_setting(value: &str) -> Vec<String> {
    normalise(value.lines().map(str::to_owned).collect())
}

pub fn folders_to_setting(folders: &[String]) -> String {
    folders.join("\n")
}

/// A path as a `file:` URL for QML images.
pub fn file_url(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let mut slashed = path.replace('\\', "/");
    if !slashed.starts_with('/') {
        slashed.insert(0, '/');
    }
    let mut out = String::from("file://");
    for c in slashed.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            '%' => out.push_str("%25"),
            _ => out.push(c),
        }
    }
    out
}

/// A scanned file as a track row: `local:track:<id>`, no Spotify album or artist pages behind it.
pub fn to_item(track: &LocalTrack) -> TrackItem {
    let artists = if track.artists.is_empty() { vec![mf_i18n::t("Unknown artist")] } else { track.artists.clone() };
    let cover = file_url(&track.cover);
    TrackItem {
        track_id: track.id.clone(),
        title: track.title.clone(),
        artists: artists.join(", "),
        artist_refs: artists.into_iter().map(|name| ArtistRef { id: String::new(), name }).collect(),
        album: track.album.clone(),
        cover_large: cover.clone(),
        cover,
        duration_ms: track.duration_ms,
        available: true,
        uri: track.uri(),
        ..Default::default()
    }
}

pub fn folder_tree(roots: &[String], tracks: &[LocalTrack]) -> Vec<FolderNode> {
    let mut direct: BTreeMap<PathBuf, usize> = BTreeMap::new();
    for track in tracks {
        *direct.entry(PathBuf::from(track.folder())).or_default() += 1;
    }
    roots.iter().map(|root| node(Path::new(root), &direct)).filter(|node| node.total > 0).collect()
}

fn node(dir: &Path, direct: &BTreeMap<PathBuf, usize>) -> FolderNode {
    let tracks = direct.get(dir).copied().unwrap_or(0);
    // the children are the next path component of every folder below this one
    let mut children: BTreeMap<PathBuf, ()> = BTreeMap::new();
    for folder in direct.keys() {
        if let Ok(rest) = folder.strip_prefix(dir)
            && let Some(first) = rest.components().next()
        {
            children.insert(dir.join(first), ());
        }
    }
    let children: Vec<FolderNode> = children.into_keys().map(|child| node(&child, direct)).collect();
    let total = tracks + children.iter().map(|child| child.total).sum::<usize>();
    FolderNode {
        path: dir.to_string_lossy().into_owned(),
        name: dir.file_name().and_then(|name| name.to_str()).unwrap_or(&dir.to_string_lossy()).to_owned(),
        tracks,
        total,
        children,
    }
}
