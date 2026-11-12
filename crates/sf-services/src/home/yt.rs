//! YouTube Music home on `HomeService`: the home feed as `HomeShelf`s in its own snapshot (`watch_youtube`),
//! more shelves through the continuation, and explore / charts / moods / "see all" pages on request. Works
//! anonymously; signing in or out of YouTube Music reloads it (the feed is personal when signed in).
use std::sync::Arc;
use std::sync::atomic::Ordering;

use md3_model::{HomeShelf, LibraryEntry};
use tokio::sync::{broadcast, watch};

use super::HomeService;
use crate::ServiceError;
use crate::auth::YouTubeAuthEvent;
use crate::catalog::yt_error;
use crate::youtube::{BrowseRef, Mood, Shelf, ShelvesPage, YtItem};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct YouTubeHomeSnapshot {
    pub loading: bool,
    pub shelves: Arc<Vec<HomeShelf>>,
    /// more shelves can be loaded (`more_youtube`)
    pub has_more: bool,
    pub error_text: String,
}

#[derive(Default)]
pub(super) struct YtHome {
    continuation: Option<String>,
}

fn to_value(value: impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or_default()
}

/// A tile for anything a shelf can hold (`HomeShelf` kind "tiles").
pub(crate) fn tile(item: &YtItem) -> LibraryEntry {
    match item {
        YtItem::Track(track) => LibraryEntry {
            kind: "track".to_owned(),
            id: track.uri.clone(),
            uri: track.uri.clone(),
            title: track.title.clone(),
            subtitle: track.artists.clone(),
            cover: track.cover.clone(),
            read_only: true,
            ..Default::default()
        },
        YtItem::Album(album) => LibraryEntry {
            kind: "album".to_owned(),
            id: album.uri.clone(),
            uri: album.uri.clone(),
            title: album.title.clone(),
            subtitle: album.artists.clone(),
            cover: album.cover.clone(),
            read_only: true,
            ..Default::default()
        },
        YtItem::Artist(artist) => LibraryEntry {
            kind: "artist".to_owned(),
            id: artist.uri.clone(),
            uri: artist.uri.clone(),
            title: artist.name.clone(),
            cover: artist.cover.clone(),
            read_only: true,
            ..Default::default()
        },
        YtItem::Playlist(playlist) => LibraryEntry {
            kind: "playlist".to_owned(),
            id: playlist.uri.clone(),
            uri: playlist.uri.clone(),
            title: playlist.title.clone(),
            subtitle: playlist.owner.clone(),
            cover: playlist.cover.clone(),
            owner: playlist.owner.clone(),
            read_only: !playlist.can_edit,
            track_count: playlist.track_count,
            ..Default::default()
        },
    }
}

/// A YouTube shelf as a home shelf: only songs → "tracks", only artists → "artists", anything else → "tiles".
/// `key` is "youtube:<index>:<title>" (stable while the feed is unchanged).
pub(crate) fn home_shelf(index: usize, shelf: &Shelf) -> HomeShelf {
    let all = |test: fn(&YtItem) -> bool| !shelf.items.is_empty() && shelf.items.iter().all(test);
    let (kind, items) = if all(|item| matches!(item, YtItem::Track(_))) {
        ("tracks", shelf.items.iter().filter_map(|item| item.clone().into_track()).map(to_value).collect())
    } else if all(|item| matches!(item, YtItem::Artist(_))) {
        let artists = shelf.items.iter().filter_map(|item| match item {
            YtItem::Artist(artist) => Some(to_value(artist)),
            _ => None,
        });
        ("artists", artists.collect())
    } else {
        ("tiles", shelf.items.iter().map(|item| to_value(tile(item))).collect())
    };
    HomeShelf { key: format!("youtube:{index}:{}", shelf.title), title: shelf.title.clone(), kind: kind.to_owned(), items }
}

fn home_shelves(offset: usize, page: &ShelvesPage) -> Vec<HomeShelf> {
    page.shelves
        .iter()
        .filter(|shelf| !shelf.items.is_empty())
        .enumerate()
        .map(|(index, shelf)| home_shelf(offset + index, shelf))
        .collect()
}

impl HomeService {
    pub(super) fn spawn_yt_listener(&self) {
        let mut events = self.auth.subscribe_youtube();
        let me = self.me.clone();
        self.rt.spawn(async move {
            loop {
                let event = events.recv().await;
                let Some(service) = me.upgrade() else { return };
                match event {
                    Ok(YouTubeAuthEvent::SignedIn(_) | YouTubeAuthEvent::SignedOut)
                    | Err(broadcast::error::RecvError::Lagged(_)) => {
                        // only a feed someone has looked at is reloaded
                        let shown = !service.yt_home.borrow().shelves.is_empty() || service.yt_home.borrow().loading;
                        if shown {
                            service.refresh_youtube(true);
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
    }

    pub fn youtube_snapshot(&self) -> YouTubeHomeSnapshot {
        self.yt_home.borrow().clone()
    }

    pub fn watch_youtube(&self) -> watch::Receiver<YouTubeHomeSnapshot> {
        self.yt_home.subscribe()
    }

    /// Load the YouTube Music home feed (`force`: past the 60 s cache of `YouTubeService::home`).
    pub fn refresh_youtube(&self, force: bool) {
        let generation = self.yt_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.yt_home.send_modify(|s| {
            s.loading = true;
            s.error_text.clear();
        });
        let me = self.me.clone();
        self.rt.spawn(async move {
            let Some(service) = me.upgrade() else { return };
            let page = service.auth.youtube().home(force).await;
            if service.yt_generation.load(Ordering::SeqCst) != generation {
                return;
            }
            match page {
                Ok(page) => {
                    crate::lock(&service.yt_state).continuation = page.continuation.clone();
                    service.yt_home.send_replace(YouTubeHomeSnapshot {
                        loading: false,
                        shelves: Arc::new(home_shelves(0, &page)),
                        has_more: page.continuation.is_some(),
                        error_text: String::new(),
                    });
                }
                Err(error) => service.yt_home.send_modify(|s| {
                    s.loading = false;
                    s.error_text = error.to_string();
                }),
            }
        });
    }

    /// Append the next shelves of the YouTube home feed.
    pub fn more_youtube(&self) {
        let Some(token) = crate::lock(&self.yt_state).continuation.take() else { return };
        let generation = self.yt_generation.load(Ordering::SeqCst);
        self.yt_home.send_modify(|s| s.loading = true);
        let me = self.me.clone();
        self.rt.spawn(async move {
            let Some(service) = me.upgrade() else { return };
            let page = service.auth.youtube().home_continuation(&token).await;
            if service.yt_generation.load(Ordering::SeqCst) != generation {
                return;
            }
            match page {
                Ok(page) => {
                    crate::lock(&service.yt_state).continuation = page.continuation.clone();
                    service.yt_home.send_modify(|s| {
                        let mut shelves = (*s.shelves).clone();
                        shelves.extend(home_shelves(shelves.len(), &page));
                        s.shelves = Arc::new(shelves);
                        s.has_more = page.continuation.is_some();
                        s.loading = false;
                    });
                }
                Err(error) => {
                    // the token stays usable: try again later
                    crate::lock(&service.yt_state).continuation = Some(token);
                    service.yt_home.send_modify(|s| {
                        s.loading = false;
                        s.error_text = error.to_string();
                    });
                }
            }
        });
    }

    /// YouTube Music "Explore" as home shelves.
    pub async fn youtube_explore(&self) -> Result<Vec<HomeShelf>, ServiceError> {
        let page = self.auth.youtube().explore().await.map_err(yt_error)?;
        Ok(home_shelves(0, &page))
    }

    /// Charts (`country`: ISO code, `None` = the account's region).
    pub async fn youtube_charts(&self, country: Option<&str>) -> Result<Vec<HomeShelf>, ServiceError> {
        let page = self.auth.youtube().charts(country).await.map_err(yt_error)?;
        Ok(home_shelves(0, &page))
    }

    /// Mood and genre tiles; open one with `youtube_browse(&mood.browse)`.
    pub async fn youtube_moods(&self) -> Result<Vec<Mood>, ServiceError> {
        self.auth.youtube().moods().await.map_err(yt_error)
    }

    /// A "see all" / mood / genre page as home shelves.
    pub async fn youtube_browse(&self, target: &BrowseRef) -> Result<Vec<HomeShelf>, ServiceError> {
        let page = self.auth.youtube().browse_shelves(target).await.map_err(yt_error)?;
        Ok(home_shelves(0, &page))
    }
}

#[cfg(test)]
mod tests {
    use md3_model::{ArtistItem, TrackItem};

    use super::*;

    fn shelf(items: Vec<YtItem>) -> Shelf {
        Shelf { title: "Quick picks".into(), items, more: None, continuation: None }
    }

    #[test]
    fn shelves_take_the_kind_of_their_items() {
        let track = YtItem::Track(TrackItem { uri: "youtube:track:v".into(), title: "Song".into(), ..Default::default() });
        let artist = YtItem::Artist(ArtistItem { uri: "youtube:artist:UC".into(), ..Default::default() });
        let tracks = home_shelf(0, &shelf(vec![track.clone()]));
        assert_eq!((tracks.kind.as_str(), tracks.key.as_str()), ("tracks", "youtube:0:Quick picks"));
        assert_eq!(tracks.items[0]["uri"], "youtube:track:v");
        assert_eq!(home_shelf(1, &shelf(vec![artist.clone()])).kind, "artists");
        let mixed = home_shelf(2, &shelf(vec![track, artist]));
        assert_eq!(mixed.kind, "tiles");
        assert_eq!(mixed.items[0]["kind"], "track");
        assert_eq!(mixed.items[1]["uri"], "youtube:artist:UC");
    }
}
