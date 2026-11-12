//! Downloaded YouTube Music songs and playlists, for playing without the network (port of soundwave1's
//! `SongDownloadWorker` / `PlaylistDownloadWorker` / `DownloadRepository`, storage limit and clean-up after
//! `StorageSenseRepository`).
//!
//! A queue runs [`WORKERS`] downloads at a time. Each one resolves the song through the lane's stream source (the
//! YouTube `SourceChain`, so a lossless or JioSaavn copy is saved when Settings pick one), saves the whole file
//! into the downloads folder (`fetch`, ranged like the player), and its cover beside it in `covers/`. A network
//! failure is tried again from where the file ends; an expired address (403 / 410) is resolved again. The index
//! (`downloads.json`) keeps each song's metadata, file and cover. [`OfflineSource`] puts the files in front of the
//! lane's source, so a downloaded song plays from disk.
pub mod fetch;
mod source;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use md3_model::TrackItem;
use mf_platform::settings::{SettingKey, SettingsStore};
use serde::{Deserialize, Serialize};
use sf_player::engine::youtube::{LOCAL_FILE_URL, StreamRequest, StreamSource, YtStream, video_id};
use tokio::runtime::Handle;
use tokio::sync::{broadcast, watch};

pub use source::OfflineSource;

/// Downloads running at the same time.
pub const WORKERS: usize = 2;
/// Tries of one download after network failures (the first one included).
const ATTEMPTS: u32 = 4;
/// Fresh addresses asked for after the old one expired.
const RESOLVES: u32 = 3;
const INDEX_VERSION: u32 = 1;
const COVERS: &str = "covers";

/// One downloaded song.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DownloadedTrack {
    /// the row as it was downloaded (`youtube:track:` URI, original cover URLs)
    pub track: TrackItem,
    pub video_id: String,
    /// the audio file
    pub path: String,
    /// the saved cover; "" — none
    pub cover_path: String,
    /// bytes
    pub size: u64,
    pub mime: String,
    pub codecs: String,
    pub bitrate: i64,
    pub sample_rate: u32,
    pub bit_depth: u32,
    pub loudness_db: Option<f64>,
    /// the stream's source (`ANDROID_VR`, `MONOCHROME`, `SAAVN`…)
    pub source: String,
    /// unix seconds
    pub downloaded_at: i64,
    /// unix seconds of the last play from the file; 0 — never
    pub last_played: i64,
    /// downloaded on its own (not only with a playlist)
    pub single: bool,
    /// the playlists it was downloaded with (URIs)
    pub playlists: Vec<String>,
}

impl DownloadedTrack {
    /// The row for lists: the saved cover as the thumbnail. `cover_large` keeps the network address when there is
    /// one, because the player's cover cache only takes http(s) addresses (a cover it showed before is on disk).
    pub fn item(&self) -> TrackItem {
        let mut item = self.track.clone();
        if !self.cover_path.is_empty() {
            let cover = crate::local::file_url(&self.cover_path);
            if item.cover_large.is_empty() {
                item.cover_large = item.cover.clone();
            }
            item.cover = cover;
        }
        item
    }

    /// The stream the player opens: the file.
    pub fn stream(&self) -> YtStream {
        YtStream {
            video_id: self.video_id.clone(),
            url: format!("{LOCAL_FILE_URL}{}", self.path),
            mime: self.mime.clone(),
            codecs: self.codecs.clone(),
            bitrate: self.bitrate,
            content_length: Some(self.size),
            duration_ms: self.track.duration_ms,
            loudness_db: self.loudness_db,
            source: "DOWNLOAD".into(),
            title: self.track.title.clone(),
            author: self.track.artists.clone(),
            thumbnail: crate::local::file_url(&self.cover_path),
            sample_rate: self.sample_rate,
            bit_depth: self.bit_depth,
            ..Default::default()
        }
    }
}

/// A playlist downloaded as a whole.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DownloadedPlaylist {
    pub uri: String,
    pub title: String,
    pub cover: String,
    /// its songs in order (only YouTube ones)
    pub track_uris: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Index {
    version: u32,
    tracks: Vec<DownloadedTrack>,
    playlists: Vec<DownloadedPlaylist>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobState {
    Queued,
    Running,
    /// gave up after the retries; `retry` puts it back in the queue
    Failed(String),
}

/// A song waiting, downloading or failed.
#[derive(Clone, Debug, PartialEq)]
pub struct DownloadJob {
    pub uri: String,
    pub title: String,
    pub artists: String,
    pub state: JobState,
    pub downloaded: u64,
    pub total: Option<u64>,
    /// 1 — the first try
    pub attempt: u32,
}

impl DownloadJob {
    /// 0..1; 0 while the size is unknown.
    pub fn progress(&self) -> f64 {
        match self.total {
            Some(total) if total > 0 => (self.downloaded as f64 / total as f64).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DownloadsSnapshot {
    /// newest first
    pub tracks: Arc<Vec<DownloadedTrack>>,
    pub playlists: Arc<Vec<DownloadedPlaylist>>,
    /// in queue order
    pub jobs: Arc<Vec<DownloadJob>>,
    /// space the files take
    pub bytes: u64,
    /// 0 — no limit
    pub limit_bytes: u64,
    pub folder: String,
}

impl DownloadsSnapshot {
    pub fn is_downloaded(&self, uri: &str) -> bool {
        self.tracks.iter().any(|track| track.track.uri == uri)
    }

    /// Queued or downloading.
    pub fn is_pending(&self, uri: &str) -> bool {
        self.jobs.iter().any(|job| job.uri == uri && !matches!(job.state, JobState::Failed(_)))
    }

    pub fn job(&self, uri: &str) -> Option<&DownloadJob> {
        self.jobs.iter().find(|job| job.uri == uri)
    }

    pub fn has_playlist(&self, uri: &str) -> bool {
        self.playlists.iter().any(|playlist| playlist.uri == uri)
    }

    /// (downloaded, all) songs of a downloaded playlist.
    pub fn playlist_progress(&self, uri: &str) -> (usize, usize) {
        let Some(playlist) = self.playlists.iter().find(|playlist| playlist.uri == uri) else { return (0, 0) };
        let done = playlist.track_uris.iter().filter(|uri| self.is_downloaded(uri)).count();
        (done, playlist.track_uris.len())
    }
}

/// Where and how much.
#[derive(Clone, Debug)]
pub struct DownloadsConfig {
    pub folder: PathBuf,
    /// 0 — no limit
    pub limit_bytes: u64,
}

/// Timings (shorter in tests).
#[derive(Clone, Debug)]
pub struct Tuning {
    /// before the second try; doubles after each failure
    pub retry_delay: Duration,
    pub chunk_bytes: u64,
}

impl Default for Tuning {
    fn default() -> Self {
        Self { retry_delay: Duration::from_secs(2), chunk_bytes: fetch::CHUNK_BYTES }
    }
}

struct Job {
    track: TrackItem,
    video_id: String,
    /// the playlist it was asked for with
    playlist: Option<String>,
    state: JobState,
    downloaded: u64,
    total: Option<u64>,
    attempt: u32,
    task: Option<tokio::task::AbortHandle>,
    /// the `.part` file while it downloads
    part: Option<PathBuf>,
    /// bumped on every start, so a cancelled run does not report into a new one
    run: u64,
}

#[derive(Default)]
struct Inner {
    /// newest first
    tracks: Vec<DownloadedTrack>,
    playlists: Vec<DownloadedPlaylist>,
    jobs: VecDeque<Job>,
    runs: u64,
}

pub struct DownloadsService {
    me: Weak<DownloadsService>,
    rt: Handle,
    http: sf_api::Http,
    /// where songs are resolved (not the offline source: that would answer with the file)
    source: Arc<dyn StreamSource>,
    index_file: PathBuf,
    config: Mutex<DownloadsConfig>,
    tuning: Tuning,
    inner: Mutex<Inner>,
    snapshot: watch::Sender<DownloadsSnapshot>,
}

fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

/// The folder from the setting: "" — `app_data_dir()/downloads`.
pub fn folder_from_setting(value: &str) -> PathBuf {
    let value = value.trim();
    if value.is_empty() { mf_platform::dirs::app_data_dir().join("downloads") } else { PathBuf::from(value) }
}

pub fn limit_from_setting(mb: i64) -> u64 {
    (mb.max(0) as u64) * 1024 * 1024
}

/// The file name's extension for a stream.
fn extension(stream: &YtStream) -> &'static str {
    let mime = stream.mime.to_ascii_lowercase();
    match mime.split(';').next().unwrap_or_default().trim() {
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/webm" => "webm",
        "audio/ogg" => "ogg",
        "audio/wav" | "audio/x-wav" => "wav",
        _ => "m4a",
    }
}

/// A video id as a file name (they are `[A-Za-z0-9_-]`; anything else is replaced).
fn file_stem(video_id: &str) -> String {
    video_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

fn remove_file(path: &str) {
    if !path.is_empty() {
        let _ = std::fs::remove_file(path);
    }
}

impl DownloadsService {
    /// `source` resolves songs; the index is read from `index_file` (missing files are dropped from it).
    pub fn new(rt: Handle, http: sf_api::Http, source: Arc<dyn StreamSource>, index_file: PathBuf, config: DownloadsConfig) -> Arc<Self> {
        Self::with_tuning(rt, http, source, index_file, config, Tuning::default())
    }

    pub fn with_tuning(
        rt: Handle,
        http: sf_api::Http,
        source: Arc<dyn StreamSource>,
        index_file: PathBuf,
        config: DownloadsConfig,
        tuning: Tuning,
    ) -> Arc<Self> {
        let index: Index = std::fs::read(&index_file).ok().and_then(|data| serde_json::from_slice(&data).ok()).unwrap_or_default();
        let mut tracks = index.tracks;
        let before = tracks.len();
        tracks.retain(|track| Path::new(&track.path).is_file());
        let inner = Inner { tracks, playlists: index.playlists, ..Default::default() };
        let pruned = inner.tracks.len() != before;
        let service = Arc::new_cyclic(|me| Self {
            me: me.clone(),
            rt,
            http,
            source,
            index_file,
            config: Mutex::new(config),
            tuning,
            inner: Mutex::new(inner),
            snapshot: watch::Sender::new(DownloadsSnapshot::default()),
        });
        if pruned {
            service.save(&service.lock());
        }
        service.publish(&service.lock());
        service
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn config(&self) -> DownloadsConfig {
        self.config.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    pub fn snapshot(&self) -> DownloadsSnapshot {
        self.snapshot.borrow().clone()
    }

    pub fn watch(&self) -> watch::Receiver<DownloadsSnapshot> {
        self.snapshot.subscribe()
    }

    pub fn set_folder(&self, folder: PathBuf) {
        self.config.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).folder = folder;
        self.publish(&self.lock());
    }

    pub fn set_limit(&self, limit_bytes: u64) {
        self.config.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).limit_bytes = limit_bytes;
        self.publish(&self.lock());
    }

    fn publish(&self, inner: &Inner) {
        let config = self.config();
        let jobs = inner
            .jobs
            .iter()
            .map(|job| DownloadJob {
                uri: job.track.uri.clone(),
                title: job.track.title.clone(),
                artists: job.track.artists.clone(),
                state: job.state.clone(),
                downloaded: job.downloaded,
                total: job.total,
                attempt: job.attempt,
            })
            .collect();
        let snapshot = DownloadsSnapshot {
            tracks: Arc::new(inner.tracks.clone()),
            playlists: Arc::new(inner.playlists.clone()),
            jobs: Arc::new(jobs),
            bytes: inner.tracks.iter().map(|track| track.size).sum(),
            limit_bytes: config.limit_bytes,
            folder: config.folder.display().to_string(),
        };
        self.snapshot.send_if_modified(|current| {
            let changed = *current != snapshot;
            *current = snapshot;
            changed
        });
    }

    fn save(&self, inner: &Inner) {
        let index = Index { version: INDEX_VERSION, tracks: inner.tracks.clone(), playlists: inner.playlists.clone() };
        let Ok(data) = serde_json::to_vec_pretty(&index) else { return };
        if let Some(parent) = self.index_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let temp = self.index_file.with_extension("json.tmp");
        if let Err(error) = std::fs::write(&temp, data).and_then(|()| std::fs::rename(&temp, &self.index_file)) {
            tracing::warn!("downloads: can't save {}: {error}", self.index_file.display());
        }
    }

    /// The downloaded song of a video, when its file is still there.
    pub fn downloaded(&self, video_id: &str) -> Option<DownloadedTrack> {
        let inner = self.lock();
        inner.tracks.iter().find(|track| track.video_id == video_id).filter(|track| Path::new(&track.path).is_file()).cloned()
    }

    /// The file's stream for the player; `None` — not downloaded.
    pub fn offline_stream(&self, video_id: &str) -> Option<YtStream> {
        self.downloaded(video_id).map(|track| track.stream())
    }

    pub fn is_downloaded(&self, uri: &str) -> bool {
        video_id(uri).is_some_and(|id| self.downloaded(id).is_some())
    }

    /// Rows of the downloads list (newest first).
    pub fn items(&self) -> Vec<TrackItem> {
        self.lock().tracks.iter().map(DownloadedTrack::item).collect()
    }

    /// Rows of a downloaded playlist, in its order (only the songs already downloaded).
    pub fn playlist_items(&self, uri: &str) -> Vec<TrackItem> {
        let inner = self.lock();
        let Some(playlist) = inner.playlists.iter().find(|playlist| playlist.uri == uri) else { return Vec::new() };
        playlist
            .track_uris
            .iter()
            .filter_map(|uri| inner.tracks.iter().find(|track| &track.track.uri == uri))
            .map(DownloadedTrack::item)
            .collect()
    }

    /// Remember a play from the file (for the clean-up of songs not played for long).
    pub fn mark_played(&self, video_id: &str) {
        let mut inner = self.lock();
        let Some(track) = inner.tracks.iter_mut().find(|track| track.video_id == video_id) else { return };
        track.last_played = now();
        self.save(&inner);
        self.publish(&inner);
    }

    /// Download one song (YouTube only; anything else is ignored). Already downloaded — it is kept as a single.
    pub fn download(&self, track: TrackItem) {
        let mut inner = self.lock();
        if self.enqueue(&mut inner, track, None) {
            self.pump(&mut inner);
        }
        self.publish(&inner);
    }

    /// Download a playlist's YouTube songs (the others are skipped) and remember the playlist.
    pub fn download_playlist(&self, uri: &str, title: &str, cover: &str, tracks: Vec<TrackItem>) {
        let mut inner = self.lock();
        let mut uris: Vec<String> = Vec::new();
        for track in &tracks {
            if video_id(&track.uri).is_some() && !uris.contains(&track.uri) {
                uris.push(track.uri.clone());
            }
        }
        let playlist = DownloadedPlaylist { uri: uri.to_owned(), title: title.to_owned(), cover: cover.to_owned(), track_uris: uris };
        match inner.playlists.iter_mut().find(|known| known.uri == uri) {
            Some(known) => *known = playlist,
            None => inner.playlists.insert(0, playlist),
        }
        for track in tracks {
            self.enqueue(&mut inner, track, Some(uri.to_owned()));
        }
        self.save(&inner);
        self.pump(&mut inner);
        self.publish(&inner);
    }

    /// Adds the song to the queue; false — nothing to download.
    fn enqueue(&self, inner: &mut Inner, track: TrackItem, playlist: Option<String>) -> bool {
        let Some(id) = video_id(&track.uri).map(str::to_owned) else { return false };
        if let Some(known) = inner.tracks.iter_mut().find(|known| known.video_id == id) {
            match &playlist {
                Some(uri) if !known.playlists.contains(uri) => known.playlists.push(uri.clone()),
                Some(_) => {}
                None => known.single = true,
            }
            self.save(inner);
            return false;
        }
        if let Some(job) = inner.jobs.iter_mut().find(|job| job.video_id == id) {
            if matches!(job.state, JobState::Failed(_)) {
                job.state = JobState::Queued;
                job.attempt = 0;
            }
            if job.playlist.is_some() && playlist.is_none() {
                job.playlist = None;
            }
            return true;
        }
        inner.jobs.push_back(Job {
            track,
            video_id: id,
            playlist,
            state: JobState::Queued,
            downloaded: 0,
            total: None,
            attempt: 0,
            task: None,
            part: None,
            run: 0,
        });
        true
    }

    /// Start queued jobs while fewer than [`WORKERS`] run.
    fn pump(&self, inner: &mut Inner) {
        let Some(me) = self.me.upgrade() else { return };
        loop {
            let running = inner.jobs.iter().filter(|job| job.state == JobState::Running).count();
            if running >= WORKERS {
                return;
            }
            let Some(index) = inner.jobs.iter().position(|job| job.state == JobState::Queued) else { return };
            inner.runs += 1;
            let run = inner.runs;
            let job = &mut inner.jobs[index];
            job.state = JobState::Running;
            job.run = run;
            job.attempt = 1;
            let (track, id) = (job.track.clone(), job.video_id.clone());
            let me = me.clone();
            let handle = self.rt.spawn(async move {
                let result = me.fetch_song(run, &track, &id).await;
                me.finish(run, &id, result);
            });
            inner.jobs[index].task = Some(handle.abort_handle());
        }
    }

    /// Progress of the run `run` (ignored once it was cancelled).
    fn report(&self, run: u64, update: impl FnOnce(&mut Job)) {
        let mut inner = self.lock();
        let Some(job) = inner.jobs.iter_mut().find(|job| job.run == run && job.state == JobState::Running) else { return };
        update(job);
        self.publish(&inner);
    }

    fn finish(&self, run: u64, video_id: &str, result: Result<DownloadedTrack, String>) {
        let mut inner = self.lock();
        let Some(index) = inner.jobs.iter().position(|job| job.run == run && job.video_id == video_id) else {
            // cancelled while it finished: drop what it saved
            if let Ok(track) = result {
                remove_file(&track.path);
                remove_file(&track.cover_path);
            }
            return;
        };
        match result {
            Ok(mut track) => {
                let job = inner.jobs.remove(index).expect("the job");
                match job.playlist {
                    Some(uri) => track.playlists.push(uri),
                    None => track.single = true,
                }
                // a playlist asked for while it was downloading
                let wanted: Vec<String> = inner
                    .playlists
                    .iter()
                    .filter(|playlist| playlist.track_uris.contains(&track.track.uri))
                    .map(|playlist| playlist.uri.clone())
                    .collect();
                for uri in wanted {
                    if !track.playlists.contains(&uri) {
                        track.playlists.push(uri);
                    }
                }
                inner.tracks.insert(0, track);
                self.save(&inner);
            }
            Err(error) => {
                tracing::warn!("downloads: {video_id}: {error}");
                let job = &mut inner.jobs[index];
                job.state = JobState::Failed(error);
                job.task = None;
                job.part = None;
            }
        }
        self.pump(&mut inner);
        self.publish(&inner);
    }

    /// Stop and forget a queued, running or failed download (its partial file is deleted).
    pub fn cancel(&self, uri: &str) {
        let mut inner = self.lock();
        if let Some(index) = inner.jobs.iter().position(|job| job.track.uri == uri) {
            let job = inner.jobs.remove(index).expect("the job");
            drop_job(job);
        }
        self.pump(&mut inner);
        self.publish(&inner);
    }

    pub fn cancel_all(&self) {
        let mut inner = self.lock();
        for job in std::mem::take(&mut inner.jobs) {
            drop_job(job);
        }
        self.publish(&inner);
    }

    /// Put a failed download back in the queue.
    pub fn retry(&self, uri: &str) {
        let mut inner = self.lock();
        for job in inner.jobs.iter_mut().filter(|job| job.track.uri == uri && matches!(job.state, JobState::Failed(_))) {
            job.state = JobState::Queued;
            job.attempt = 0;
        }
        self.pump(&mut inner);
        self.publish(&inner);
    }

    pub fn retry_failed(&self) {
        let mut inner = self.lock();
        for job in inner.jobs.iter_mut().filter(|job| matches!(job.state, JobState::Failed(_))) {
            job.state = JobState::Queued;
            job.attempt = 0;
        }
        self.pump(&mut inner);
        self.publish(&inner);
    }

    /// Delete a downloaded song (or cancel it while it downloads).
    pub fn remove(&self, uri: &str) {
        self.cancel(uri);
        let mut inner = self.lock();
        let Some(index) = inner.tracks.iter().position(|track| track.track.uri == uri) else { return };
        let track = inner.tracks.remove(index);
        remove_file(&track.path);
        remove_file(&track.cover_path);
        self.save(&inner);
        self.publish(&inner);
    }

    /// Forget a downloaded playlist: its queued songs are cancelled and its songs deleted, except those
    /// downloaded on their own or with another playlist.
    pub fn remove_playlist(&self, uri: &str) {
        let mut inner = self.lock();
        let Some(index) = inner.playlists.iter().position(|playlist| playlist.uri == uri) else { return };
        inner.playlists.remove(index);
        let mut kept = VecDeque::new();
        for job in std::mem::take(&mut inner.jobs) {
            if job.playlist.as_deref() == Some(uri) { drop_job(job) } else { kept.push_back(job) }
        }
        inner.jobs = kept;
        let mut removed = Vec::new();
        inner.tracks.retain_mut(|track| {
            track.playlists.retain(|known| known != uri);
            let keep = track.single || !track.playlists.is_empty();
            if !keep {
                removed.push((track.path.clone(), track.cover_path.clone()));
            }
            keep
        });
        for (path, cover) in removed {
            remove_file(&path);
            remove_file(&cover);
        }
        self.save(&inner);
        self.pump(&mut inner);
        self.publish(&inner);
    }

    /// Songs not played from the file for `days` days (never played: counted from the download), oldest first.
    pub fn forgotten(&self, days: u32) -> Vec<DownloadedTrack> {
        let cutoff = now() - i64::from(days) * 86_400;
        let mut found: Vec<DownloadedTrack> = self
            .lock()
            .tracks
            .iter()
            .filter(|track| track.last_played.max(track.downloaded_at) < cutoff)
            .cloned()
            .collect();
        found.sort_by_key(|track| track.last_played.max(track.downloaded_at));
        found
    }

    /// Delete [`DownloadsService::forgotten`] songs; returns (songs, bytes) freed.
    pub fn remove_forgotten(&self, days: u32) -> (usize, u64) {
        let forgotten = self.forgotten(days);
        let freed = forgotten.iter().map(|track| track.size).sum();
        for track in &forgotten {
            self.remove(&track.track.uri);
        }
        (forgotten.len(), freed)
    }

    /// Delete every downloaded song and playlist.
    pub fn remove_all(&self) {
        self.cancel_all();
        let mut inner = self.lock();
        for track in std::mem::take(&mut inner.tracks) {
            remove_file(&track.path);
            remove_file(&track.cover_path);
        }
        inner.playlists.clear();
        self.save(&inner);
        self.publish(&inner);
    }

    fn used_bytes(&self) -> u64 {
        self.lock().tracks.iter().map(|track| track.size).sum()
    }

    fn check_room(&self, more: u64) -> Result<(), String> {
        let limit = self.config().limit_bytes;
        if limit > 0 && self.used_bytes() + more > limit {
            return Err(mf_i18n::t("Downloads have reached their storage limit"));
        }
        Ok(())
    }

    async fn resolve(&self, request: &StreamRequest) -> Result<YtStream, String> {
        self.source.resolve_track(request.clone()).await
    }

    /// One song: resolve, fetch with retries, save the cover.
    async fn fetch_song(&self, run: u64, track: &TrackItem, id: &str) -> Result<DownloadedTrack, String> {
        self.check_room(0)?;
        let folder = self.config().folder;
        std::fs::create_dir_all(&folder).map_err(|error| format!("{}: {error}", folder.display()))?;
        let client = self.http.client()?;
        let request = StreamRequest::from_track(track);
        let mut failures = 0u32;
        let mut resolves = 0u32;
        let mut stream: Option<YtStream> = None;
        let mut part: Option<PathBuf> = None;
        let result = loop {
            let attempt = failures + resolves + 1;
            self.report(run, |job| job.attempt = attempt);
            let current = match &stream {
                Some(stream) => stream.clone(),
                None => match self.resolve(&request).await {
                    Ok(resolved) => {
                        stream = Some(resolved.clone());
                        resolved
                    }
                    Err(error) => {
                        failures += 1;
                        if failures >= ATTEMPTS {
                            break Err(error);
                        }
                        self.back_off(failures).await;
                        continue;
                    }
                },
            };
            if let Err(error) = self.check_room(current.content_length.unwrap_or(0)) {
                break Err(error);
            }
            let path = folder.join(format!("{}.{}.part", file_stem(id), extension(&current)));
            if part.as_ref().is_some_and(|old| *old != path) {
                let _ = std::fs::remove_file(part.as_ref().expect("old part"));
            }
            part = Some(path.clone());
            self.report(run, |job| job.part = Some(path.clone()));
            let target = fetch::Target {
                url: &current.url,
                user_agent: &current.user_agent,
                content_length: current.content_length,
                chunk_bytes: self.tuning.chunk_bytes,
            };
            let progress = |downloaded: u64, total: Option<u64>| self.report(run, |job| (job.downloaded, job.total) = (downloaded, total));
            match fetch::to_file(&client, &target, &path, &progress).await {
                Ok(size) => break Ok((current, path, size)),
                Err(fetch::FetchError::Expired) => {
                    resolves += 1;
                    self.source.invalidate(id);
                    let _ = std::fs::remove_file(&path);
                    if resolves > RESOLVES {
                        break Err(mf_i18n::t("The download address keeps expiring"));
                    }
                    stream = None;
                }
                Err(fetch::FetchError::Failed(error)) => {
                    failures += 1;
                    if failures >= ATTEMPTS {
                        break Err(error);
                    }
                    self.back_off(failures).await;
                }
            }
        };
        let (stream, part, size) = match result {
            Ok(done) => done,
            Err(error) => {
                if let Some(part) = part {
                    let _ = std::fs::remove_file(part);
                }
                return Err(error);
            }
        };
        let path = part.with_extension("");
        std::fs::rename(&part, &path).map_err(|error| format!("{}: {error}", path.display()))?;
        let cover_path = self.save_cover(&client, &folder, id, track, &stream).await;
        Ok(DownloadedTrack {
            track: filled(track, &stream),
            video_id: id.to_owned(),
            path: path.display().to_string(),
            cover_path,
            size,
            mime: stream.mime.clone(),
            codecs: stream.codecs.clone(),
            bitrate: stream.bitrate,
            sample_rate: stream.sample_rate,
            bit_depth: stream.bit_depth,
            loudness_db: stream.loudness_db,
            source: stream.source.clone(),
            downloaded_at: now(),
            ..Default::default()
        })
    }

    async fn back_off(&self, failures: u32) {
        tokio::time::sleep(self.tuning.retry_delay * 2u32.pow(failures.saturating_sub(1).min(5))).await;
    }

    /// The cover next to the songs (`covers/<id>.jpg`); "" — none could be saved (the row keeps its URL).
    async fn save_cover(&self, client: &reqwest::Client, folder: &Path, id: &str, track: &TrackItem, stream: &YtStream) -> String {
        let url = [&track.cover_large, &track.cover, &stream.thumbnail]
            .into_iter()
            .find(|url| url.starts_with("http://") || url.starts_with("https://"))
            .cloned();
        let Some(url) = url else { return String::new() };
        let fetched = async {
            let response = client.get(&url).send().await.ok()?.error_for_status().ok()?;
            response.bytes().await.ok()
        };
        let Some(bytes) = fetched.await.filter(|bytes| !bytes.is_empty()) else { return String::new() };
        let dir = folder.join(COVERS);
        let path = dir.join(format!("{}.jpg", file_stem(id)));
        match std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, &bytes)) {
            Ok(()) => path.display().to_string(),
            Err(error) => {
                tracing::warn!("downloads: cover {}: {error}", path.display());
                String::new()
            }
        }
    }
}

/// Abort a job's task and delete its partial file.
fn drop_job(job: Job) {
    if let Some(task) = job.task {
        task.abort();
    }
    if let Some(part) = job.part {
        let _ = std::fs::remove_file(part);
    }
}

/// A URI-only row gets its title, artist, length and cover from the stream.
fn filled(track: &TrackItem, stream: &YtStream) -> TrackItem {
    let mut track = track.clone();
    if track.title.is_empty() {
        track.title = stream.title.clone();
    }
    if track.artists.is_empty() {
        track.artists = stream.author.clone();
    }
    if track.duration_ms <= 0 {
        track.duration_ms = stream.duration_ms;
    }
    if track.cover.is_empty() {
        track.cover = stream.thumbnail.clone();
    }
    track.available = true;
    track
}

/// Folder and limit follow Settings (a new folder applies to the next downloads; the files already saved stay
/// where they are).
pub fn follow_settings(rt: &Handle, settings: &Arc<SettingsStore>, downloads: Arc<DownloadsService>) {
    let settings = settings.clone();
    let mut changes = settings.subscribe();
    rt.spawn(async move {
        loop {
            match changes.recv().await {
                Ok(SettingKey::DownloadsFolder) | Err(broadcast::error::RecvError::Lagged(_)) => {
                    downloads.set_folder(folder_from_setting(&settings.get_str(SettingKey::DownloadsFolder)));
                    downloads.set_limit(limit_from_setting(settings.get_int(SettingKey::DownloadsLimitMb)));
                }
                Ok(SettingKey::DownloadsLimitMb) => {
                    downloads.set_limit(limit_from_setting(settings.get_int(SettingKey::DownloadsLimitMb)));
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// The config Settings give at start.
pub fn config_from_settings(settings: &SettingsStore) -> DownloadsConfig {
    DownloadsConfig {
        folder: folder_from_setting(&settings.get_str(SettingKey::DownloadsFolder)),
        limit_bytes: limit_from_setting(settings.get_int(SettingKey::DownloadsLimitMb)),
    }
}
