//! Local playlists: kept on this computer in `app_data_dir()/local_playlists.json` (or
//! `$PIXELMUSIC_LOCAL_PLAYLISTS_FILE`), ids `local:playlist:<16 hex>`. Any song can go in (YouTube Music,
//! local files). The public `LibraryService` methods route here on `PlaylistId::is_local`, and the same
//! `LibraryEvent`s go out, so the bridges and QML treat them like any other playlist.
//!
//! "Sync local playlists to YouTube Music" (`localPlaylistSync`): while it is on and YouTube Music is signed in,
//! every edit marks the playlist dirty and, after a short pause, the playlist is mirrored to a private YouTube
//! Music playlist linked to it (created on the first sync): its YouTube songs, in the same order, and its name.
//! The mirror is reconciled against what the account really holds (removes, adds, then moves), so edits made
//! while offline or signed out are caught up on the next sign-in or when the setting is switched on. Deleting
//! a local playlist deletes its mirror. The linked YouTube playlist is left out of the Library list (the local
//! one stands for it).
//! Lock order: `state` before `yt` before `local`; nothing here holds `local` across another lock.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use md3_model::{LibraryEntry, ListChange, PlaylistId, PlaylistItem, TrackItem, ids::LOCAL_PLAYLIST};
use mf_platform::settings::SettingKey;
use serde::{Deserialize, Serialize};

use super::{LibraryEvent, LibraryKind, LibraryService, PlaylistInfo};
use crate::data::known;
use crate::lock;
use crate::youtube::playlist_uri;

/// Pause after the last edit before the mirror is written (a burst of edits becomes one sync).
const SYNC_DELAY: std::time::Duration = std::time::Duration::from_millis(1500);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalPlaylist {
    /// 16 hex digits; the URI is `local:playlist:<id>`
    pub id: String,
    pub title: String,
    pub description: String,
    pub tracks: Vec<TrackItem>,
    /// unix seconds
    pub created: i64,
    pub updated: i64,
    /// the linked YouTube Music playlist (bare `PL…` id), None until the first sync
    pub youtube_id: Option<String>,
    /// the name last written to the linked playlist
    pub synced_title: String,
    /// edited since the last successful sync
    pub dirty: bool,
}

impl LocalPlaylist {
    pub fn uri(&self) -> String {
        format!("{LOCAL_PLAYLIST}{}", self.id)
    }

    /// Video ids of the YouTube songs, in order (what the mirror should hold).
    pub fn videos(&self) -> Vec<String> {
        self.tracks.iter().filter_map(|track| video_id(&track.uri)).map(str::to_owned).collect()
    }

    fn item(&self) -> PlaylistItem {
        PlaylistItem {
            playlist_id: self.uri(),
            uri: self.uri(),
            title: self.title.clone(),
            description: self.description.clone(),
            track_count: self.tracks.len() as i64,
            cover: self.tracks.iter().map(|track| track.cover.as_str()).find(|cover| !cover.is_empty()).unwrap_or_default().to_owned(),
            owner: mf_i18n::t("Local playlist"),
            can_edit: true,
            ..Default::default()
        }
    }
}

fn video_id(uri: &str) -> Option<&str> {
    uri.strip_prefix("youtube:track:").filter(|id| !id.is_empty())
}

#[derive(Default)]
pub(super) struct LocalState {
    path: Option<PathBuf>,
    playlists: Vec<LocalPlaylist>,
    /// bumped per playlist on every edit: a pending sync runs only if no later edit came
    edits: HashMap<String, u64>,
    /// the last deleted playlist, for "Undo" (`follow_playlist`)
    deleted: Option<LocalPlaylist>,
}

pub fn load(path: &Path) -> Vec<LocalPlaylist> {
    std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

pub fn save(path: &Path, playlists: &[LocalPlaylist]) {
    let text = match serde_json::to_vec_pretty(playlists) {
        Ok(text) => text,
        Err(error) => return tracing::warn!("local playlists: {error}"),
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = sf_api::util::replace_file(path, &text) {
        tracing::warn!("local playlists: writing {}: {error}", path.display());
    }
}

fn store_path() -> PathBuf {
    match std::env::var_os("PIXELMUSIC_LOCAL_PLAYLISTS_FILE") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => mf_platform::dirs::app_data_dir().join("local_playlists.json"),
    }
}

fn new_id(taken: &[LocalPlaylist]) -> String {
    loop {
        let id = format!("{:016x}", rand::random::<u64>());
        if !taken.iter().any(|playlist| playlist.id == id) {
            return id;
        }
    }
}

// --- edits on the rows (pure) --------------------------------------------------------------------

/// Insert `rows` at `position` (past the end — append); returns where they went.
pub fn insert_rows(tracks: &mut Vec<TrackItem>, rows: Vec<TrackItem>, position: Option<usize>) -> usize {
    let at = position.unwrap_or(tracks.len()).min(tracks.len());
    tracks.splice(at..at, rows);
    at
}

/// Remove the (uri, row) pairs whose row still holds that uri; returns the removed rows, descending.
pub fn remove_rows(tracks: &mut Vec<TrackItem>, rows: &[(String, usize)]) -> Vec<usize> {
    let mut at: Vec<usize> =
        rows.iter().filter(|(uri, row)| tracks.get(*row).is_some_and(|track| track.uri == *uri)).map(|(_, row)| *row).collect();
    at.sort_unstable_by(|a, b| b.cmp(a));
    at.dedup();
    for row in &at {
        tracks.remove(*row);
    }
    at
}

/// Move one row; `to` is its index after the move. False when out of range or a no-op.
pub fn move_row(tracks: &mut Vec<TrackItem>, from: usize, to: usize) -> bool {
    if from == to || from >= tracks.len() || to >= tracks.len() {
        return false;
    }
    let row = tracks.remove(from);
    tracks.insert(to, row);
    true
}

// --- reconcile with the mirror (pure) ------------------------------------------------------------

/// Remote rows to remove (video, setVideoId) and videos to add (at the end), so that the remote playlist holds
/// the same songs as `desired`, counting repeats. Rows are kept first-come.
pub fn plan_contents(desired: &[String], remote: &[(String, String)]) -> (Vec<(String, String)>, Vec<String>) {
    let mut wanted: HashMap<&str, usize> = HashMap::new();
    for video in desired {
        *wanted.entry(video.as_str()).or_default() += 1;
    }
    let mut remove = Vec::new();
    for (video, set) in remote {
        match wanted.get_mut(video.as_str()) {
            Some(count) if *count > 0 => *count -= 1,
            _ => remove.push((video.clone(), set.clone())),
        }
    }
    let mut add = Vec::new();
    for video in desired.iter().rev() {
        if let Some(count) = wanted.get_mut(video.as_str())
            && *count > 0
        {
            *count -= 1;
            add.push(video.clone());
        }
    }
    add.reverse();
    (remove, add)
}

/// Moves (setVideoId, successor setVideoId — None: to the end) that put `remote` in the order of `desired`.
/// `remote` must hold the same songs as `desired` (after `plan_contents` was applied).
pub fn plan_moves(desired: &[String], remote: &[(String, String)]) -> Vec<(String, Option<String>)> {
    let mut current: Vec<(String, String)> = remote.to_vec();
    let mut moves = Vec::new();
    for (i, video) in desired.iter().enumerate() {
        if current.get(i).is_some_and(|(have, _)| have == video) {
            continue;
        }
        let Some(j) = (i + 1..current.len()).find(|&j| current[j].0 == *video) else { continue };
        let row = current.remove(j);
        let successor = current.get(i).map(|(_, set)| set.clone());
        moves.push((row.1.clone(), successor));
        current.insert(i, row);
    }
    moves
}

impl LibraryService {
    fn local(&self) -> std::sync::MutexGuard<'_, LocalState> {
        let mut state = lock(&self.local);
        if state.path.is_none() {
            let path = store_path();
            state.playlists = load(&path);
            state.path = Some(path);
        }
        state
    }

    /// Write the store; call with the guard still held so writes keep their order.
    fn local_save(state: &LocalState) {
        if let Some(path) = &state.path {
            save(path, &state.playlists);
        }
    }

    fn local_title(&self, id: &PlaylistId) -> String {
        let key = id.local_id().unwrap_or_default();
        self.local().playlists.iter().find(|playlist| playlist.id == key).map(|playlist| playlist.title.clone()).unwrap_or_default()
    }

    /// Apply `edit` to the playlist, mark it edited, save; then PlaylistInfo + EntriesChanged + the edit's
    /// own events and a sync. None when there is no such playlist.
    fn local_edit<R>(&self, id: &PlaylistId, edit: impl FnOnce(&mut LocalPlaylist) -> R) -> Option<R> {
        let key = id.local_id()?.to_owned();
        let result = {
            let mut state = self.local();
            let playlist = state.playlists.iter_mut().find(|playlist| playlist.id == key)?;
            let result = edit(playlist);
            playlist.updated = super::now_seconds();
            playlist.dirty = true;
            *state.edits.entry(key.clone()).or_default() += 1;
            Self::local_save(&state);
            result
        };
        self.send([LibraryEvent::PlaylistInfo(id.clone()), LibraryEvent::EntriesChanged]);
        self.schedule_local_sync(key);
        Some(result)
    }

    /// Keep the local playlists in `path` instead of the app data folder (tests, screenshots).
    pub fn use_local_playlists_file(&self, path: PathBuf) {
        let mut state = lock(&self.local);
        state.playlists = load(&path);
        state.path = Some(path);
        state.edits.clear();
    }

    pub fn local_playlists(&self) -> Vec<LocalPlaylist> {
        self.local().playlists.clone()
    }

    /// `youtube:playlist:<id>` of the mirrors: the Library list shows the local playlist instead.
    pub(super) fn local_mirror_uris(&self) -> HashSet<String> {
        self.local().playlists.iter().filter_map(|playlist| playlist.youtube_id.as_deref()).map(playlist_uri).collect()
    }

    /// Local rows of the Library screen (`entries`), before sorting.
    pub(super) fn local_entries(
        &self,
        kind: LibraryKind,
        owner: &str,
        pins: &[String],
        times: &HashMap<String, String>,
        entries: &mut Vec<LibraryEntry>,
    ) {
        if kind != LibraryKind::Playlists || !matches!(owner, "all" | "mine" | "") {
            return;
        }
        let synced = self.local_sync_active();
        let state = self.local();
        entries.extend(state.playlists.iter().map(|playlist| {
            let uri = playlist.uri();
            let count = mf_i18n::n(playlist.tracks.len() as i64, "{0} track", "{0} tracks");
            let source = if synced && playlist.youtube_id.is_some() {
                mf_i18n::t("Local · synced to YouTube Music")
            } else {
                mf_i18n::t("Local playlist")
            };
            let mut covers: Vec<String> = Vec::new();
            for track in &playlist.tracks {
                if !track.cover.is_empty() && !covers.contains(&track.cover) {
                    covers.push(track.cover.clone());
                    if covers.len() == 4 {
                        break;
                    }
                }
            }
            LibraryEntry {
                kind: "playlist".to_owned(),
                id: uri.clone(),
                uri: uri.clone(),
                title: playlist.title.clone(),
                subtitle: if playlist.tracks.is_empty() { source } else { format!("{source} · {count}") },
                cover: covers.first().cloned().unwrap_or_default(),
                covers,
                owner: mf_i18n::t("Local playlist"),
                can_edit: true,
                read_only: false,
                pinned: pins.contains(&uri),
                track_count: playlist.tracks.len() as i64,
                added_at: jiff::Timestamp::from_second(playlist.created).map(|at| at.to_string()).unwrap_or_default(),
                played_at: times.get(&uri).cloned().unwrap_or_default(),
                ..Default::default()
            }
        }));
    }

    // --- pages -------------------------------------------------------------------------------

    /// Everything is on disk: the rows go out at once.
    pub(super) fn local_open(&self, id: PlaylistId) {
        let rows = self.local_tracks(&id).unwrap_or_default();
        self.send([
            LibraryEvent::PlaylistInfo(id.clone()),
            LibraryEvent::PlaylistTracks { id: id.clone(), change: ListChange::Reset(rows) },
            LibraryEvent::PlaylistTracks { id, change: ListChange::Loading(false) },
        ]);
    }

    pub(super) fn local_tracks(&self, id: &PlaylistId) -> Option<Vec<TrackItem>> {
        let key = id.local_id()?;
        self.local().playlists.iter().find(|playlist| playlist.id == key).map(|playlist| playlist.tracks.clone())
    }

    pub(super) fn local_info(&self, id: &PlaylistId) -> Option<PlaylistInfo> {
        let key = id.local_id()?;
        let state = self.local();
        let playlist = state.playlists.iter().find(|playlist| playlist.id == key)?;
        let total_duration_ms = playlist.tracks.iter().map(|track| track.duration_ms.max(0)).sum();
        Some(PlaylistInfo { item: playlist.item(), total_duration_ms, followed: true })
    }

    // --- edits -------------------------------------------------------------------------------

    /// Rows for `uris`: the hint, the shared catalog cache, else a bare row (filled in when it plays).
    fn local_rows(uris: &[String], hint: Option<&TrackItem>) -> Vec<TrackItem> {
        uris.iter()
            .filter(|uri| !uri.is_empty())
            .map(|uri| match hint.filter(|hint| hint.play_uri() == *uri || hint.uri == *uri) {
                Some(hint) => hint.clone(),
                None => known::get(uri).unwrap_or_else(|| TrackItem { uri: uri.clone(), available: true, ..Default::default() }),
            })
            .collect()
    }

    pub(super) fn local_add(&self, id: PlaylistId, uris: Vec<String>, position: Option<usize>, hint: Option<TrackItem>) {
        let rows = Self::local_rows(&uris, hint.as_ref());
        if rows.is_empty() {
            return;
        }
        let count = rows.len();
        let inserted = rows.clone();
        let Some(at) = self.local_edit(&id, |playlist| insert_rows(&mut playlist.tracks, rows, position)) else { return };
        self.send([LibraryEvent::PlaylistTracks { id: id.clone(), change: ListChange::Insert { at, items: inserted } }]);
        if position.is_none() {
            let title = self.local_title(&id);
            self.notice(if count == 1 {
                mf_i18n::tf("Added to “{0}”", &[&title])
            } else {
                mf_i18n::tf("Added {0} songs to “{1}”", &[&count, &title])
            });
        }
    }

    pub(super) fn local_remove(&self, id: PlaylistId, rows: Vec<(String, usize)>) {
        let Some(removed) = self.local_edit(&id, |playlist| remove_rows(&mut playlist.tracks, &rows)) else { return };
        self.send(
            removed
                .into_iter()
                .map(|at| LibraryEvent::PlaylistTracks { id: id.clone(), change: ListChange::Remove { at, count: 1 } }),
        );
    }

    pub(super) fn local_move(&self, id: PlaylistId, from: usize, to: usize) {
        if self.local_edit(&id, |playlist| move_row(&mut playlist.tracks, from, to)) == Some(true) {
            self.send([LibraryEvent::PlaylistTracks { id, change: ListChange::Move { from, to } }]);
        }
    }

    pub(super) fn local_rename(&self, id: PlaylistId, title: String) {
        let title = title.trim().to_owned();
        if title.is_empty() {
            return self.notice(mf_i18n::t("The name can’t be empty"));
        }
        self.local_edit(&id, |playlist| playlist.title = title);
    }

    pub(super) fn local_set_description(&self, id: PlaylistId, text: String) {
        let text = text.trim().to_owned();
        self.local_edit(&id, |playlist| playlist.description = text);
    }

    /// A new local playlist with these songs; returns its id (already saved and listed).
    pub fn create_local_playlist(&self, title: String, uris: Vec<String>, hint: Option<TrackItem>) -> PlaylistId {
        let title = if title.trim().is_empty() { mf_i18n::t("New playlist") } else { title.trim().to_owned() };
        let tracks = Self::local_rows(&uris, hint.as_ref());
        let now = super::now_seconds();
        let id = {
            let mut state = self.local();
            let id = new_id(&state.playlists);
            state.playlists.insert(
                0,
                LocalPlaylist { id: id.clone(), title: title.clone(), tracks, created: now, updated: now, dirty: true, ..Default::default() },
            );
            Self::local_save(&state);
            id
        };
        self.send([LibraryEvent::EntriesChanged]);
        self.notice(mf_i18n::tf("Playlist “{0}” created", &[&title]));
        self.schedule_local_sync(id.clone());
        PlaylistId::new(format!("{LOCAL_PLAYLIST}{id}"))
    }

    /// Delete a local playlist (and, while syncing, its YouTube Music mirror).
    pub(super) fn local_delete(&self, id: PlaylistId) {
        let Some(key) = id.local_id().map(str::to_owned) else { return };
        let mirror = {
            let syncing = self.local_sync_active();
            let mut state = self.local();
            let Some(at) = state.playlists.iter().position(|playlist| playlist.id == key) else { return };
            let mut removed = state.playlists.remove(at);
            state.edits.remove(&key);
            Self::local_save(&state);
            let mirror = removed.youtube_id.clone().filter(|_| syncing);
            if mirror.is_some() {
                // restored, it gets a new mirror
                removed.youtube_id = None;
                removed.synced_title.clear();
                removed.dirty = true;
            }
            state.deleted = Some(removed);
            mirror
        };
        self.send([LibraryEvent::EntriesChanged]);
        if let Some(mirror) = mirror {
            self.spawn(move |service| async move {
                let _sync = service.local_sync_lock.lock().await;
                match service.yt().delete_playlist(&mirror).await {
                    Ok(()) => service.refresh_youtube(),
                    Err(error) => tracing::warn!("local playlist mirror {mirror}: delete: {error:?}"),
                }
            });
        }
    }

    /// "Undo" after `local_delete`: the playlist comes back where it was listed (first).
    pub(super) fn local_restore(&self, id: PlaylistId) {
        let Some(key) = id.local_id().map(str::to_owned) else { return };
        let restored = {
            let mut state = self.local();
            match state.deleted.take() {
                Some(playlist) if playlist.id == key => {
                    let dirty = playlist.dirty;
                    state.playlists.insert(0, playlist);
                    Self::local_save(&state);
                    dirty
                }
                other => {
                    state.deleted = other;
                    return;
                }
            }
        };
        self.send([LibraryEvent::EntriesChanged, LibraryEvent::PlaylistInfo(id)]);
        if restored {
            self.schedule_local_sync(key);
        }
    }

    // --- sync to YouTube Music ---------------------------------------------------------------

    /// The setting is on and YouTube Music is signed in.
    pub(super) fn local_sync_active(&self) -> bool {
        self.settings.get_bool(SettingKey::LocalPlaylistSync) && lock(&self.yt).signed_in
    }

    /// Sync one playlist after `SYNC_DELAY`, unless another edit comes first (that one schedules its own).
    fn schedule_local_sync(&self, key: String) {
        if !self.local_sync_active() {
            return;
        }
        let edit = self.local().edits.get(&key).copied().unwrap_or_default();
        self.spawn(move |service| async move {
            tokio::time::sleep(SYNC_DELAY).await;
            if service.local().edits.get(&key).copied().unwrap_or_default() != edit {
                return;
            }
            service.sync_local(&key).await;
        });
    }

    /// Setting "Sync local playlists to YouTube Music" changed: subtitles change; on — catch up.
    pub fn local_sync_changed(&self) {
        self.send([LibraryEvent::EntriesChanged]);
        self.sync_local_playlists();
    }

    /// Setting switched, signed in, or the library read again: sync every playlist edited since its last sync.
    pub fn sync_local_playlists(&self) {
        if !self.local_sync_active() {
            return;
        }
        let dirty: Vec<String> =
            self.local().playlists.iter().filter(|playlist| playlist.dirty).map(|playlist| playlist.id.clone()).collect();
        if dirty.is_empty() {
            return;
        }
        self.spawn(move |service| async move {
            for key in dirty {
                service.sync_local(&key).await;
            }
        });
    }

    /// Mirror one local playlist: create the linked playlist, or bring its songs, order and name in line.
    async fn sync_local(self: &Arc<Self>, key: &str) {
        let _sync = self.local_sync_lock.lock().await;
        if !self.local_sync_active() {
            return;
        }
        let Some((snapshot, edit)) = ({
            let state = self.local();
            let edit = state.edits.get(key).copied().unwrap_or_default();
            state.playlists.iter().find(|playlist| playlist.id == key && playlist.dirty).cloned().map(|playlist| (playlist, edit))
        }) else {
            return;
        };
        let videos = snapshot.videos();
        let yt = self.yt();
        let result: Result<String, crate::youtube::YouTubeError> = async {
            let create = || async {
                let created = yt.create_playlist(&snapshot.title, &videos, false).await?;
                tracing::info!("local playlist “{}”: mirrored to YouTube Music {created}", snapshot.title);
                Ok(created)
            };
            let Some(mirror) = snapshot.youtube_id.clone() else { return create().await };
            let read = |page: crate::youtube::models::PlaylistPage| -> Vec<(String, String)> {
                page.tracks
                    .into_iter()
                    .filter_map(|row| Some((video_id(&row.track.uri)?.to_owned(), row.set_video_id)))
                    .filter(|(_, set)| !set.is_empty())
                    .collect()
            };
            let remote = match yt.playlist_full(&mirror, |_| {}).await {
                Ok(page) => read(page),
                Err(error) => {
                    // the mirror can't be read and the account's playlists (read) don't have it: it was
                    // deleted elsewhere, so a new one is linked
                    let (_, loaded, present) = self.yt_playlist_uris();
                    if loaded && !present.contains(&playlist_uri(&mirror)) {
                        tracing::info!("local playlist “{}”: mirror {mirror} is gone ({error:?})", snapshot.title);
                        return create().await;
                    }
                    return Err(error);
                }
            };
            let (remove, add) = plan_contents(&videos, &remote);
            if !remove.is_empty() {
                let pairs: Vec<(&str, &str)> = remove.iter().map(|(video, set)| (video.as_str(), set.as_str())).collect();
                yt.remove_from_playlist(&mirror, &pairs).await?;
            }
            if !add.is_empty() {
                let refs: Vec<&str> = add.iter().map(String::as_str).collect();
                yt.add_to_playlist(&mirror, &refs).await?;
            }
            let remote = if remove.is_empty() && add.is_empty() { remote } else { read(yt.playlist_full(&mirror, |_| {}).await?) };
            for (set, successor) in plan_moves(&videos, &remote) {
                yt.move_in_playlist(&mirror, &set, successor.as_deref()).await?;
            }
            if snapshot.title != snapshot.synced_title {
                yt.rename_playlist(&mirror, &snapshot.title).await?;
            }
            Ok(mirror)
        }
        .await;
        match result {
            Ok(mirror) => {
                let created = snapshot.youtube_id.as_deref() != Some(mirror.as_str());
                let kept = {
                    let mut state = self.local();
                    let still = state.edits.get(key).copied().unwrap_or_default() == edit;
                    match state.playlists.iter_mut().find(|playlist| playlist.id == key) {
                        Some(playlist) => {
                            playlist.youtube_id = Some(mirror.clone());
                            playlist.synced_title = snapshot.title.clone();
                            playlist.dirty = !still;
                            Self::local_save(&state);
                            true
                        }
                        None => false,
                    }
                };
                if !kept {
                    // deleted while the mirror was being written: the mirror goes too
                    let _ = yt.delete_playlist(&mirror).await;
                    return;
                }
                self.send([LibraryEvent::EntriesChanged]);
                if created {
                    self.refresh_youtube();
                }
            }
            Err(error) => {
                tracing::warn!("local playlist “{}”: sync to YouTube Music: {error:?}", snapshot.title);
                self.notice(mf_i18n::tf("Couldn’t sync “{0}” to YouTube Music: {1}", &[&snapshot.title, &error.to_string()]));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(uri: &str) -> TrackItem {
        TrackItem { uri: uri.into(), title: uri.to_uppercase(), ..Default::default() }
    }

    fn uris(tracks: &[TrackItem]) -> Vec<&str> {
        tracks.iter().map(|track| track.uri.as_str()).collect()
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    fn remote(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items.iter().map(|(video, set)| (video.to_string(), set.to_string())).collect()
    }

    /// Apply remove + add + moves to a simulated remote playlist.
    fn simulate(desired: &[String], start: Vec<(String, String)>) -> Vec<String> {
        let (remove, add) = plan_contents(desired, &start);
        let mut rows: Vec<(String, String)> = start.into_iter().filter(|row| !remove.contains(row)).collect();
        for (n, video) in add.into_iter().enumerate() {
            rows.push((video, format!("new{n}")));
        }
        for (set, successor) in plan_moves(desired, &rows) {
            let from = rows.iter().position(|row| row.1 == set).unwrap();
            let row = rows.remove(from);
            let to = successor.map_or(rows.len(), |successor| rows.iter().position(|row| row.1 == successor).unwrap());
            rows.insert(to, row);
        }
        rows.into_iter().map(|(video, _)| video).collect()
    }

    #[test]
    fn store_round_trips_and_tolerates_garbage() {
        let path = std::env::temp_dir().join(format!("sf-local-playlists-{:016x}.json", rand::random::<u64>()));
        assert!(load(&path).is_empty(), "missing file");
        let playlists = vec![LocalPlaylist {
            id: "00ff".into(),
            title: "Mine".into(),
            tracks: vec![track("youtube:track:a"), track("local:track:01")],
            youtube_id: Some("PLx".into()),
            dirty: true,
            ..Default::default()
        }];
        save(&path, &playlists);
        assert_eq!(load(&path), playlists);
        std::fs::write(&path, "not json").unwrap();
        assert!(load(&path).is_empty());
        let _ = std::fs::remove_file(&path);
        assert_eq!(playlists[0].uri(), "local:playlist:00ff");
        assert_eq!(playlists[0].videos(), strings(&["a"]), "only YouTube songs are mirrored");
    }

    #[test]
    fn row_edits() {
        let mut tracks = vec![track("a"), track("b"), track("c")];
        assert_eq!(insert_rows(&mut tracks, vec![track("x")], Some(1)), 1);
        assert_eq!(insert_rows(&mut tracks, vec![track("y")], Some(99)), 4);
        assert_eq!(insert_rows(&mut tracks, vec![track("z")], None), 5);
        assert_eq!(uris(&tracks), ["a", "x", "b", "c", "y", "z"]);
        // a stale row (uri moved) is not removed
        assert_eq!(remove_rows(&mut tracks, &[("x".into(), 1), ("c".into(), 0), ("z".into(), 5)]), vec![5, 1]);
        assert_eq!(uris(&tracks), ["a", "b", "c", "y"]);
        assert!(move_row(&mut tracks, 0, 3));
        assert_eq!(uris(&tracks), ["b", "c", "y", "a"]);
        assert!(!move_row(&mut tracks, 1, 1) && !move_row(&mut tracks, 0, 9));
    }

    #[test]
    fn contents_plan_counts_repeats() {
        let desired = strings(&["a", "b", "a", "c"]);
        let (remove, add) = plan_contents(&desired, &remote(&[("a", "1"), ("d", "2"), ("a", "3"), ("a", "4")]));
        assert_eq!(remove, remote(&[("d", "2"), ("a", "4")]));
        assert_eq!(add, strings(&["b", "c"]));
        let (remove, add) = plan_contents(&desired, &[]);
        assert!(remove.is_empty());
        assert_eq!(add, desired);
    }

    #[test]
    fn reconcile_reaches_the_local_order() {
        let cases: [(&[&str], &[(&str, &str)]); 5] = [
            (&["a", "b", "c"], &[("c", "1"), ("b", "2"), ("a", "3")]),
            (&["a", "b", "a", "c"], &[("a", "1"), ("d", "2"), ("a", "3"), ("a", "4")]),
            (&["x", "y"], &[]),
            (&[], &[("a", "1")]),
            (&["b", "a", "b"], &[("b", "1"), ("b", "2"), ("a", "3")]),
        ];
        for (desired, start) in cases {
            let desired = strings(desired);
            assert_eq!(simulate(&desired, remote(start)), desired, "from {start:?}");
        }
        let same = remote(&[("a", "1"), ("b", "2")]);
        assert!(plan_moves(&strings(&["a", "b"]), &same).is_empty(), "nothing to move");
    }
}
