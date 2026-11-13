//! YouTube Music side of `LibraryService`: the account's playlists (Liked Music `LM` first), saved albums and
//! artists, playlist pages with their set-video ids, likes, playlist edits and playlist import. The public
//! `LibraryService` methods route here when the id or URI is a YouTube one (`PlaylistId::is_youtube`,
//! `uri_provider`), and the same `LibraryEvent`s go out, so the bridges need no new event kinds.
//! Lock order: `state` before `yt`; nothing here takes `state`.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use md3_model::{AlbumItem, ArtistItem, LibraryEntry, ListChange, PlaylistId, PlaylistItem, TrackItem};
use tokio::sync::{broadcast, watch};

use super::{FeedbackKind, LibraryEvent, LibraryKind, LibraryService, PlaylistInfo};
use crate::auth::YouTubeAuthEvent;
use crate::lock;
use crate::youtube::{
    LIBRARY_ARTISTS, LIKED_ALBUMS, LIKED_MUSIC, PageGuard, PlaylistTrack, Rating, YouTubeError, YouTubeService, YtItem,
    playlist_uri,
};

/// Progress of `LibraryService::import_playlist` (`watch_import`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportProgress {
    pub running: bool,
    /// songs looked up so far / songs in the source (0 until the source is read)
    pub done: usize,
    pub total: usize,
    /// the source's name
    pub title: String,
    /// matched songs in source order (also when nothing was saved)
    pub tracks: Vec<TrackItem>,
    /// "title · artist" of every song without a match
    pub missed: Vec<String>,
    /// `youtube:playlist:<id>` of the playlist the import was saved to, "" when not saved
    pub playlist_uri: String,
    /// translated failure, "" when none
    pub error_text: String,
}

#[derive(Default)]
pub(super) struct YtState {
    generation: u64,
    pub(super) signed_in: bool,
    loading: bool,
    playlists: Vec<PlaylistItem>,
    albums: Vec<AlbumItem>,
    artists: Vec<ArtistItem>,
    /// URIs of the songs in Liked Music
    liked: HashSet<String>,
    pages: HashMap<PlaylistId, YtPage>,
    import: Option<tokio::task::AbortHandle>,
}

#[derive(Default)]
struct YtPage {
    generation: u64,
    info: PlaylistItem,
    rows: Vec<PlaylistTrack>,
    loading: bool,
    complete: bool,
    read: Option<tokio::task::AbortHandle>,
}

pub(super) fn is_youtube(uri: &str) -> bool {
    sf_api::util::uri_provider(uri) == Some("youtube")
}

fn video_id(uri: &str) -> Option<&str> {
    uri.strip_prefix("youtube:track:").filter(|id| !id.is_empty())
}

fn liked_id() -> PlaylistId {
    PlaylistId::from_uri_or_id(&playlist_uri(LIKED_MUSIC))
}

fn tracks(rows: &[PlaylistTrack]) -> Vec<TrackItem> {
    rows.iter().map(|row| row.track.clone()).collect()
}

fn playlist_entry(item: &PlaylistItem, pins: &[String]) -> LibraryEntry {
    let count = mf_i18n::n(item.track_count, "{0} track", "{0} tracks");
    let subtitle = if item.track_count > 0 {
        mf_i18n::tf("YouTube Music · {0}", &[&count])
    } else {
        mf_i18n::t("YouTube Music")
    };
    LibraryEntry {
        kind: "playlist".to_owned(),
        // the whole URI: `PlaylistId::from_uri_or_id` keeps it and the services route on it
        id: item.uri.clone(),
        uri: item.uri.clone(),
        title: item.title.clone(),
        subtitle,
        cover: item.cover.clone(),
        covers: vec![item.cover.clone()],
        owner: item.owner.clone(),
        owner_id: item.owner_id.clone(),
        can_edit: item.can_edit,
        read_only: !item.can_edit,
        pinned: pins.contains(&item.uri),
        track_count: item.track_count,
        ..Default::default()
    }
}

impl LibraryService {
    pub(super) fn yt(&self) -> Arc<YouTubeService> {
        self.auth.youtube()
    }

    /// (signed in, the account's playlists are read, their URIs) — for the Smart Mix retention sweep.
    pub(super) fn yt_playlist_uris(&self) -> (bool, bool, HashSet<String>) {
        let state = lock(&self.yt);
        let uris = state.playlists.iter().map(|playlist| playlist.uri.clone()).collect();
        (state.signed_in, !state.loading && !state.playlists.is_empty(), uris)
    }

    pub(super) fn spawn_yt_listener(&self) {
        let mut events = self.auth.subscribe_youtube();
        let me = self.me.clone();
        self.rt.spawn(async move {
            loop {
                let event = events.recv().await;
                let Some(service) = me.upgrade() else { return };
                match event {
                    Ok(YouTubeAuthEvent::SignedIn(_)) => service.yt_switch(true),
                    Ok(YouTubeAuthEvent::SignedOut) => service.yt_switch(false),
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let signed_in = service.auth.is_youtube_signed_in();
                        service.yt_switch(signed_in);
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
    }

    /// Another YouTube identity: drop the account's lists and Liked Music; public pages stay.
    fn yt_switch(&self, signed_in: bool) {
        let liked = liked_id();
        let dropped = {
            let mut state = lock(&self.yt);
            state.generation += 1;
            state.signed_in = signed_in;
            state.loading = false;
            state.playlists.clear();
            state.albums.clear();
            state.artists.clear();
            state.liked.clear();
            let page = state.pages.remove(&liked);
            if let Some(read) = page.as_ref().and_then(|page| page.read.as_ref()) {
                read.abort();
            }
            page.is_some()
        };
        let mut events = vec![LibraryEvent::EntriesChanged, LibraryEvent::LikesChanged, LibraryEvent::SavedChanged];
        if dropped {
            events.push(LibraryEvent::PlaylistTracks { id: liked, change: ListChange::Reset(Vec::new()) });
        }
        self.send(events);
        if signed_in {
            self.refresh_youtube();
            self.sync_local_playlists();
        }
    }

    /// Setting "Sync Playlists & Liked Songs": off — the account's lists leave the library; on — read again.
    fn yt_sync_on(&self) -> bool {
        self.settings.get_bool(mf_platform::settings::SettingKey::YouTubeLibrarySync)
    }

    pub fn youtube_sync_changed(&self) {
        let signed_in = lock(&self.yt).signed_in;
        self.yt_switch(signed_in);
    }

    /// Re-read the YouTube Music library: playlists, saved albums and artists, Liked Music.
    pub fn refresh_youtube(&self) {
        let generation = {
            let mut state = lock(&self.yt);
            if !state.signed_in || !self.yt_sync_on() {
                return;
            }
            state.loading = true;
            state.generation
        };
        self.send([LibraryEvent::EntriesChanged]);
        self.spawn(move |service| async move {
            let yt = service.yt();
            let (playlists, albums, artists, liked) = tokio::join!(
                yt.user_playlists(),
                yt.library_all(LIKED_ALBUMS),
                yt.library_all(LIBRARY_ARTISTS),
                yt.playlist_full(LIKED_MUSIC, |_| {}),
            );
            let mut failure = None;
            {
                let mut state = lock(&service.yt);
                if state.generation != generation {
                    return;
                }
                state.loading = false;
                match playlists {
                    Ok(list) => state.playlists = list,
                    Err(error) => failure = Some(error),
                }
                match albums {
                    Ok(items) => {
                        state.albums = items
                            .into_iter()
                            .filter_map(|item| match item {
                                YtItem::Album(album) => Some(album),
                                _ => None,
                            })
                            .collect()
                    }
                    Err(error) => failure = failure.or(Some(error)),
                }
                match artists {
                    Ok(items) => {
                        state.artists = items
                            .into_iter()
                            .filter_map(|item| match item {
                                YtItem::Artist(artist) => Some(artist),
                                _ => None,
                            })
                            .collect()
                    }
                    Err(error) => failure = failure.or(Some(error)),
                }
                match liked {
                    Ok(page) => state.liked = page.tracks.iter().map(|row| row.track.uri.clone()).collect(),
                    Err(error) => failure = failure.or(Some(error)),
                }
            }
            service.send([LibraryEvent::EntriesChanged, LibraryEvent::LikesChanged, LibraryEvent::SavedChanged]);
            match failure {
                Some(error) => service.yt_failed(&error),
                None => {
                    service.sweep_smart_mixes();
                    service.sync_local_playlists();
                }
            }
        });
    }

    fn yt_failed(&self, error: &YouTubeError) {
        tracing::warn!("YouTube library: {error:?}");
        self.notice(error.to_string());
    }

    fn yt_signed_in_or_notice(&self) -> bool {
        let signed_in = lock(&self.yt).signed_in;
        if !signed_in {
            self.notice(YouTubeError::NotSignedIn.to_string());
        }
        signed_in
    }

    /// The account's playlists (Liked Music first).
    pub fn youtube_playlists(&self) -> Vec<PlaylistItem> {
        lock(&self.yt).playlists.clone()
    }

    pub fn youtube_albums(&self) -> Vec<AlbumItem> {
        lock(&self.yt).albums.clone()
    }

    pub fn youtube_artists(&self) -> Vec<ArtistItem> {
        lock(&self.yt).artists.clone()
    }

    pub fn youtube_loading(&self) -> bool {
        lock(&self.yt).loading
    }

    pub(super) fn yt_is_liked(&self, uri: &str) -> bool {
        lock(&self.yt).liked.contains(uri)
    }

    pub(super) fn yt_is_saved(&self, uri: &str) -> bool {
        let state = lock(&self.yt);
        state.albums.iter().any(|album| album.uri == uri)
            || state.artists.iter().any(|artist| artist.uri == uri)
            || state.playlists.iter().any(|playlist| playlist.uri == uri)
    }

    /// YouTube rows of the Library screen (`entries`), before sorting.
    pub(super) fn yt_entries(&self, kind: LibraryKind, owner: &str, pins: &[String], entries: &mut Vec<LibraryEntry>) {
        let mut hidden = if kind == LibraryKind::Playlists { self.hidden_mix_uris() } else { Default::default() };
        if kind == LibraryKind::Playlists {
            // a local playlist's YouTube Music mirror: the local playlist stands for it
            hidden.extend(self.local_mirror_uris());
        }
        let state = lock(&self.yt);
        match kind {
            LibraryKind::Playlists => entries.extend(
                state
                    .playlists
                    .iter()
                    .filter(|item| !hidden.contains(&item.uri))
                    .filter(|item| match owner {
                        "mine" => item.can_edit || item.playlist_id == LIKED_MUSIC,
                        "followed" => !item.can_edit && item.playlist_id != LIKED_MUSIC,
                        "collab" => false,
                        _ => true,
                    })
                    .map(|item| playlist_entry(item, pins)),
            ),
            LibraryKind::Albums => entries.extend(state.albums.iter().map(|album| LibraryEntry {
                kind: "album".to_owned(),
                id: album.uri.clone(),
                uri: album.uri.clone(),
                title: album.title.clone(),
                subtitle: album.artists.clone(),
                cover: album.cover.clone(),
                covers: vec![album.cover.clone()],
                read_only: true,
                pinned: pins.contains(&album.uri),
                ..Default::default()
            })),
            LibraryKind::Artists => entries.extend(state.artists.iter().map(|artist| LibraryEntry {
                kind: "artist".to_owned(),
                id: artist.uri.clone(),
                uri: artist.uri.clone(),
                title: artist.name.clone(),
                subtitle: mf_i18n::t("YouTube Music"),
                cover: artist.cover.clone(),
                covers: vec![artist.cover.clone()],
                read_only: true,
                pinned: pins.contains(&artist.uri),
                ..Default::default()
            })),
            LibraryKind::Shows => {}
        }
    }

    // --- playlist pages ----------------------------------------------------------------------

    /// Open a YouTube playlist page: first page → Reset, the rest → Append (guarded paging), then Loading(false).
    pub(super) fn yt_open_playlist(&self, id: PlaylistId, force: bool) {
        let Some(playlist_id) = id.youtube_id().map(str::to_owned) else { return };
        let (generation, cached) = {
            let mut state = lock(&self.yt);
            let page = state.pages.entry(id.clone()).or_default();
            if !force && (page.loading || page.complete) {
                (None, Some((tracks(&page.rows), page.loading)))
            } else {
                if let Some(read) = page.read.take() {
                    read.abort();
                }
                page.generation += 1;
                page.loading = true;
                (Some(page.generation), None)
            }
        };
        if let Some((rows, loading)) = cached {
            self.send([
                LibraryEvent::PlaylistTracks { id: id.clone(), change: ListChange::Reset(rows) },
                LibraryEvent::PlaylistTracks { id, change: ListChange::Loading(loading) },
            ]);
            return;
        }
        let Some(generation) = generation else { return };
        self.send([LibraryEvent::PlaylistTracks { id: id.clone(), change: ListChange::Loading(true) }]);
        let me = self.me.clone();
        let task_id = id.clone();
        let read = self.rt.spawn(async move {
            if let Some(service) = me.upgrade() {
                service.yt_fetch_playlist(task_id, playlist_id, generation).await;
            }
        });
        let mut state = lock(&self.yt);
        match state.pages.get_mut(&id) {
            Some(page) if page.generation == generation && page.loading => page.read = Some(read.abort_handle()),
            _ => {}
        }
    }

    /// Apply `change` to the page if this read is still the current one.
    fn yt_page(&self, id: &PlaylistId, generation: u64, change: impl FnOnce(&mut YtPage)) -> bool {
        let mut state = lock(&self.yt);
        match state.pages.get_mut(id) {
            Some(page) if page.generation == generation => {
                change(page);
                true
            }
            _ => false,
        }
    }

    async fn yt_fetch_playlist(self: Arc<Self>, id: PlaylistId, playlist_id: String, generation: u64) {
        let yt = self.yt();
        let first = match yt.playlist(&playlist_id).await {
            Ok(page) => page,
            Err(error) => return self.yt_read_failed(&id, generation, &error),
        };
        let rows = tracks(&first.tracks);
        if !self.yt_page(&id, generation, |page| {
            page.info = first.info.clone();
            page.rows = first.tracks.clone();
        }) {
            return;
        }
        self.send([
            LibraryEvent::PlaylistInfo(id.clone()),
            LibraryEvent::PlaylistTracks { id: id.clone(), change: ListChange::Reset(rows) },
        ]);
        let mut continuation = first.continuation;
        let mut guard = PageGuard::new();
        while let Some(token) = continuation.take() {
            if !guard.admit(&token) {
                break;
            }
            let page = match yt.playlist_continuation(&token).await {
                Ok(page) => page,
                Err(error) => return self.yt_read_failed(&id, generation, &error),
            };
            let more = guard.record(page.tracks.len());
            let rows = tracks(&page.tracks);
            if !self.yt_page(&id, generation, |current| current.rows.extend(page.tracks.iter().cloned())) {
                return;
            }
            if !rows.is_empty() {
                self.send([LibraryEvent::PlaylistTracks { id: id.clone(), change: ListChange::Append(rows) }]);
            }
            if more {
                continuation = page.continuation;
            }
        }
        let mut liked = None;
        if !self.yt_page(&id, generation, |page| {
            page.loading = false;
            page.complete = true;
            page.read = None;
            liked = (id == liked_id()).then(|| page.rows.iter().map(|row| row.track.uri.clone()).collect());
        }) {
            return;
        }
        if let Some(liked) = liked {
            lock(&self.yt).liked = liked;
            self.send([LibraryEvent::LikesChanged]);
        }
        self.send([LibraryEvent::PlaylistTracks { id, change: ListChange::Loading(false) }]);
    }

    fn yt_read_failed(&self, id: &PlaylistId, generation: u64, error: &YouTubeError) {
        if self.yt_page(id, generation, |page| {
            page.loading = false;
            page.read = None;
        }) {
            self.send([LibraryEvent::PlaylistTracks { id: id.clone(), change: ListChange::Loading(false) }]);
            self.yt_failed(error);
        }
    }

    pub(super) fn yt_cancel_playlist(&self, id: &PlaylistId) {
        let mut state = lock(&self.yt);
        if let Some(page) = state.pages.get_mut(id)
            && let Some(read) = page.read.take()
        {
            read.abort();
            page.loading = false;
            page.generation += 1;
        }
    }

    pub(super) fn yt_replay_playlist(&self, id: &PlaylistId) {
        let rows = lock(&self.yt).pages.get(id).map(|page| tracks(&page.rows)).unwrap_or_default();
        if !rows.is_empty() {
            self.send([LibraryEvent::PlaylistTracks { id: id.clone(), change: ListChange::Reset(rows) }]);
        }
    }

    pub(super) fn yt_playlist_tracks(&self, id: &PlaylistId) -> Option<Vec<TrackItem>> {
        lock(&self.yt).pages.get(id).map(|page| tracks(&page.rows))
    }

    pub(super) fn yt_playlist_info(&self, id: &PlaylistId) -> Option<PlaylistInfo> {
        let state = lock(&self.yt);
        let listed = state.playlists.iter().find(|item| item.uri == id.as_str());
        let page = state.pages.get(id);
        let mut item = match (page, listed) {
            (Some(page), _) if !page.info.uri.is_empty() => page.info.clone(),
            (_, Some(listed)) => listed.clone(),
            _ => return None,
        };
        if let Some(listed) = listed {
            item.can_edit |= listed.can_edit;
        }
        let total_duration_ms = page.map_or(0, |page| page.rows.iter().map(|row| row.track.duration_ms.max(0)).sum());
        Some(PlaylistInfo { item, total_duration_ms, followed: listed.is_some() })
    }

    /// Own playlists only; Liked Music changes through likes, not edits.
    pub(super) fn yt_can_edit(&self, id: &PlaylistId) -> bool {
        id.youtube_id() != Some(LIKED_MUSIC) && self.yt_playlist_info(id).is_some_and(|info| info.item.can_edit)
    }

    // --- likes and saved items ---------------------------------------------------------------

    pub(super) fn yt_toggle_like(&self, track: TrackItem) {
        let Some(video) = video_id(&track.uri).map(str::to_owned) else { return };
        if !self.yt_signed_in_or_notice() {
            return;
        }
        let like = !self.yt_is_liked(&track.uri);
        self.yt_set_liked(&track, like);
        let kind = if like { FeedbackKind::Like } else { FeedbackKind::Unlike };
        self.send([LibraryEvent::Feedback { kind, track: track.clone() }]);
        self.spawn(move |service| async move {
            let rating = if like { Rating::Like } else { Rating::None };
            if let Err(error) = service.yt().rate_track(&video, rating).await {
                service.yt_set_liked(&track, !like);
                service.yt_failed(&error);
            }
        });
    }

    pub(super) fn yt_dislike(&self, track: TrackItem) {
        let Some(video) = video_id(&track.uri).map(str::to_owned) else { return };
        if !self.yt_signed_in_or_notice() {
            return;
        }
        if self.yt_is_liked(&track.uri) {
            self.yt_set_liked(&track, false);
        }
        self.send([LibraryEvent::Feedback { kind: FeedbackKind::Dislike, track }]);
        self.spawn(move |service| async move {
            if let Err(error) = service.yt().rate_track(&video, Rating::Dislike).await {
                service.yt_failed(&error);
            }
        });
    }

    /// Liked set + an open Liked Music page, then LikesChanged.
    fn yt_set_liked(&self, track: &TrackItem, like: bool) {
        let id = liked_id();
        let change = {
            let mut state = lock(&self.yt);
            if like {
                state.liked.insert(track.uri.clone());
            } else {
                state.liked.remove(&track.uri);
            }
            match state.pages.get_mut(&id) {
                Some(page) if page.complete => {
                    let at = page.rows.iter().position(|row| row.track.uri == track.uri);
                    match (like, at) {
                        (true, None) => {
                            page.rows.insert(0, PlaylistTrack { track: track.clone(), set_video_id: String::new() });
                            Some(ListChange::Insert { at: 0, items: vec![track.clone()] })
                        }
                        (false, Some(at)) => {
                            page.rows.remove(at);
                            Some(ListChange::Remove { at, count: 1 })
                        }
                        _ => None,
                    }
                }
                _ => None,
            }
        };
        let mut events = vec![LibraryEvent::LikesChanged];
        if let Some(change) = change {
            events.push(LibraryEvent::PlaylistTracks { id, change });
        }
        self.send(events);
    }

    /// Album: like its playlist; artist: subscribe to the channel; playlist: like (save) it.
    pub(super) fn yt_toggle_saved(&self, uri: String) {
        if !self.yt_signed_in_or_notice() {
            return;
        }
        let save = !self.yt_is_saved(&uri);
        let kind = sf_api::util::uri_kind(&uri).unwrap_or_default().to_owned();
        let Some(id) = sf_api::util::uri_id(&uri).map(str::to_owned) else { return };
        self.spawn(move |service| async move {
            let yt = service.yt();
            let done = match kind.as_str() {
                "album" => match yt.album(&id).await {
                    Ok(page) if !page.playlist_id.is_empty() => yt.like_playlist(&page.playlist_id, save).await,
                    Ok(_) => Err(YouTubeError::Unexpected("album playlist id")),
                    Err(error) => Err(error),
                },
                "artist" => yt.subscribe(&id, save).await,
                "playlist" => yt.like_playlist(&id, save).await,
                _ => return,
            };
            match done {
                Ok(()) => {
                    if save {
                        service.notice(mf_i18n::t("Added to your library"));
                    }
                    service.refresh_youtube();
                }
                Err(error) => service.yt_failed(&error),
            }
        });
    }

    // --- playlist edits ----------------------------------------------------------------------

    /// YouTube songs among `uris`; a notice when some were left out (Spotify songs can't go to YouTube).
    fn yt_videos(&self, uris: &[String]) -> Vec<String> {
        let videos: Vec<String> = uris.iter().filter_map(|uri| video_id(uri)).map(str::to_owned).collect();
        if videos.len() < uris.len() {
            self.notice(mf_i18n::t("Only YouTube Music songs can be added to a YouTube Music playlist."));
        }
        videos
    }

    /// YouTube adds at the end only; the page is re-read afterwards.
    pub(super) fn yt_add_tracks(&self, id: PlaylistId, uris: Vec<String>) {
        let Some(playlist_id) = id.youtube_id().map(str::to_owned) else { return };
        if !self.yt_signed_in_or_notice() {
            return;
        }
        let videos = self.yt_videos(&uris);
        if videos.is_empty() {
            return;
        }
        if playlist_id == LIKED_MUSIC {
            for uri in uris.iter().filter(|uri| !self.yt_is_liked(uri)) {
                self.yt_toggle_like(TrackItem { uri: uri.clone(), ..Default::default() });
            }
            return;
        }
        let title = self.yt_playlist_info(&id).map(|info| info.item.title).unwrap_or_default();
        self.spawn(move |service| async move {
            let refs: Vec<&str> = videos.iter().map(String::as_str).collect();
            match service.yt().add_to_playlist(&playlist_id, &refs).await {
                Ok(_) => {
                    service.notice(if title.is_empty() {
                        mf_i18n::t("Added to playlist")
                    } else {
                        mf_i18n::tf("Added to “{0}”", &[&title])
                    });
                    service.yt_after_edit(id);
                }
                Err(error) => service.yt_failed(&error),
            }
        });
    }

    /// Re-read an open page and the playlist list (counts) after an edit.
    fn yt_after_edit(&self, id: PlaylistId) {
        if lock(&self.yt).pages.contains_key(&id) {
            self.yt_open_playlist(id, true);
        }
        self.refresh_youtube();
    }

    /// (uri, row) pairs. Liked Music: the songs are unliked.
    pub(super) fn yt_remove_tracks(&self, id: PlaylistId, rows: Vec<(String, usize)>) {
        let Some(playlist_id) = id.youtube_id().map(str::to_owned) else { return };
        if !self.yt_signed_in_or_notice() {
            return;
        }
        if playlist_id == LIKED_MUSIC {
            for (uri, _) in rows {
                if self.yt_is_liked(&uri) {
                    self.yt_toggle_like(TrackItem { uri, ..Default::default() });
                }
            }
            return;
        }
        let (removed, events) = {
            let mut state = lock(&self.yt);
            let Some(page) = state.pages.get_mut(&id) else { return };
            let mut at: Vec<usize> = rows
                .iter()
                .filter(|(uri, row)| {
                    // YouTube removes a row by its set-video id: without one it can't be removed
                    page.rows.get(*row).is_some_and(|current| current.track.uri == *uri && !current.set_video_id.is_empty())
                })
                .map(|(_, row)| *row)
                .collect();
            at.sort_unstable();
            at.dedup();
            let mut removed = Vec::new();
            let mut events = Vec::new();
            for row in at.into_iter().rev() {
                let gone = page.rows.remove(row);
                if let Some(video) = video_id(&gone.track.uri) {
                    removed.push((video.to_owned(), gone.set_video_id.clone()));
                }
                events.push(LibraryEvent::PlaylistTracks { id: id.clone(), change: ListChange::Remove { at: row, count: 1 } });
            }
            (removed, events)
        };
        if removed.is_empty() {
            return;
        }
        self.send(events);
        self.spawn(move |service| async move {
            let pairs: Vec<(&str, &str)> = removed.iter().map(|(video, set)| (video.as_str(), set.as_str())).collect();
            if let Err(error) = service.yt().remove_from_playlist(&playlist_id, &pairs).await {
                service.yt_failed(&error);
            }
            service.yt_after_edit(id);
        });
    }

    /// Move one row: `to` is its index after the move (as `move_tracks`).
    pub(super) fn yt_move_tracks(&self, id: PlaylistId, from: usize, to: usize) {
        let Some(playlist_id) = id.youtube_id().map(str::to_owned) else { return };
        if !self.yt_signed_in_or_notice() {
            return;
        }
        let (set_video_id, successor) = {
            let mut state = lock(&self.yt);
            let Some(page) = state.pages.get_mut(&id) else { return };
            if from == to || from >= page.rows.len() || to >= page.rows.len() || page.rows[from].set_video_id.is_empty() {
                return;
            }
            let row = page.rows.remove(from);
            let set_video_id = row.set_video_id.clone();
            page.rows.insert(to, row);
            let successor = page.rows.get(to + 1).map(|next| next.set_video_id.clone()).filter(|id| !id.is_empty());
            (set_video_id, successor)
        };
        self.send([LibraryEvent::PlaylistTracks { id: id.clone(), change: ListChange::Move { from, to } }]);
        self.spawn(move |service| async move {
            let moved = service.yt().move_in_playlist(&playlist_id, &set_video_id, successor.as_deref()).await;
            if let Err(error) = moved {
                service.yt_failed(&error);
                service.yt_open_playlist(id, true);
            }
        });
    }

    /// A new private YouTube Music playlist with these songs (YouTube ones only).
    pub fn create_youtube_playlist(&self, title: String, uris: Vec<String>) {
        self.create_youtube_playlist_then(title, uris, |_, _| {});
    }

    /// `create_youtube_playlist`, then `created` with the new playlist's id (before the library is read again).
    pub(super) fn create_youtube_playlist_then(
        &self,
        title: String,
        uris: Vec<String>,
        created: impl FnOnce(&LibraryService, String) + Send + 'static,
    ) {
        if !self.yt_signed_in_or_notice() {
            return;
        }
        let videos = self.yt_videos(&uris);
        self.spawn(move |service| async move {
            match service.yt().create_playlist(&title, &videos, false).await {
                Ok(playlist_id) => {
                    created(&service, playlist_id);
                    service.notice(mf_i18n::tf("Playlist “{0}” created", &[&title]));
                    service.refresh_youtube();
                }
                Err(error) => service.yt_failed(&error),
            }
        });
    }

    pub(super) fn yt_rename_playlist(&self, id: PlaylistId, title: String) {
        let Some(playlist_id) = id.youtube_id().map(str::to_owned) else { return };
        if !self.yt_signed_in_or_notice() {
            return;
        }
        self.spawn(move |service| async move {
            match service.yt().rename_playlist(&playlist_id, &title).await {
                Ok(()) => {
                    {
                        let mut state = lock(&service.yt);
                        if let Some(page) = state.pages.get_mut(&id) {
                            page.info.title = title.clone();
                        }
                        if let Some(item) = state.playlists.iter_mut().find(|item| item.uri == id.as_str()) {
                            item.title = title.clone();
                        }
                    }
                    service.send([LibraryEvent::PlaylistInfo(id), LibraryEvent::EntriesChanged]);
                }
                Err(error) => service.yt_failed(&error),
            }
        });
    }

    /// Save someone else's playlist to the library.
    pub(super) fn yt_follow_playlist(&self, id: PlaylistId) {
        if let Some(uri) = id.youtube_id().map(playlist_uri)
            && !self.yt_is_saved(&uri)
        {
            self.yt_toggle_saved(uri);
        }
    }

    /// Remove from the library: someone else's playlist is unliked; an own playlist is deleted (as
    /// "unfollow" deletes an own playlist on Spotify).
    pub(super) fn yt_unfollow_playlist(&self, id: PlaylistId) {
        let Some(playlist_id) = id.youtube_id().map(str::to_owned) else { return };
        if playlist_id == LIKED_MUSIC || !self.yt_signed_in_or_notice() {
            return;
        }
        let own = self.yt_can_edit(&id);
        self.spawn(move |service| async move {
            let yt = service.yt();
            let done = if own { yt.delete_playlist(&playlist_id).await } else { yt.like_playlist(&playlist_id, false).await };
            match done {
                Ok(()) => service.refresh_youtube(),
                Err(error) => service.yt_failed(&error),
            }
        });
    }

    // --- import ------------------------------------------------------------------------------

    pub fn import_progress(&self) -> ImportProgress {
        self.yt_import.borrow().clone()
    }

    pub fn watch_import(&self) -> watch::Receiver<ImportProgress> {
        self.yt_import.subscribe()
    }

    /// Import a playlist (YouTube link or id, Spotify link, M3U or JSON text) into YouTube Music songs.
    /// Progress in `watch_import`. `save`: also create a private YouTube Music playlist from the matches
    /// (needs the account); otherwise the matched songs are only in the progress (to play or add).
    pub fn import_playlist(&self, input: String, save: bool) {
        if save && !self.yt_signed_in_or_notice() {
            return;
        }
        self.cancel_import();
        self.yt_import.send_replace(ImportProgress { running: true, ..Default::default() });
        let me = self.me.clone();
        let task = self.rt.spawn(async move {
            let Some(service) = me.upgrade() else { return };
            let yt = service.yt();
            let progress = service.yt_import.clone();
            let result = yt
                .import_playlist(&input, move |done, total| {
                    progress.send_modify(|state| {
                        state.done = done;
                        state.total = total;
                    })
                })
                .await;
            let found = match result {
                Ok(found) => found,
                Err(error) => {
                    service.yt_import.send_modify(|state| {
                        state.running = false;
                        state.error_text = error.to_string();
                    });
                    return;
                }
            };
            let mut saved = String::new();
            let mut error_text = String::new();
            if save && !found.tracks.is_empty() {
                let videos: Vec<String> =
                    found.tracks.iter().filter_map(|track| video_id(&track.uri)).map(str::to_owned).collect();
                let title = if found.title.is_empty() { mf_i18n::t("Imported playlist") } else { found.title.clone() };
                match yt.create_playlist(&title, &videos, false).await {
                    Ok(id) => saved = playlist_uri(&id),
                    Err(error) => error_text = error.to_string(),
                }
            }
            let matched = found.tracks.len();
            service.yt_import.send_modify(|state| {
                state.running = false;
                state.done = found.total;
                state.total = found.total;
                state.title = found.title;
                state.tracks = found.tracks;
                state.missed = found.missed;
                state.playlist_uri = saved.clone();
                state.error_text = error_text;
            });
            service.notice(mf_i18n::tf("Found {0} of {1} songs", &[&matched, &found.total]));
            if !saved.is_empty() {
                service.refresh_youtube();
            }
        });
        lock(&self.yt).import = Some(task.abort_handle());
    }

    pub fn cancel_import(&self) {
        if let Some(task) = lock(&self.yt).import.take() {
            task.abort();
        }
        self.yt_import.send_if_modified(|state| std::mem::replace(&mut state.running, false));
    }
}
