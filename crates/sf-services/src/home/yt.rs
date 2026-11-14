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

/// The release type behind a tile's kind label ("Single", "EP", "Album"), for the New releases Play / Shuffle.
fn album_type(label: &str) -> &'static str {
    if label == "EP" {
        "ep"
    } else if label == mf_i18n::t("Single") || label.eq_ignore_ascii_case("single") {
        "single"
    } else {
        "album"
    }
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
            album_type: album_type(&album.kind).to_owned(),
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
    let more = shelf.more.as_ref();
    HomeShelf {
        key: format!("youtube:{index}:{}", shelf.title),
        title: shelf.title.clone(),
        kind: kind.to_owned(),
        items,
        strapline: shelf.strapline.clone(),
        thumbnail: shelf.thumbnail.clone(),
        browse_id: more.map(|more| more.browse_id.clone()).unwrap_or_default(),
        browse_params: more.and_then(|more| more.params.clone()).unwrap_or_default(),
    }
}

/// Mood and genre buttons grouped by their section, in the page's order.
fn mood_shelves(moods: &[Mood]) -> Vec<HomeShelf> {
    let mut out: Vec<HomeShelf> = Vec::new();
    for mood in moods {
        let item = serde_json::json!({
            "kind": "browse",
            "title": mood.title,
            "browseId": mood.browse.browse_id,
            "browseParams": mood.browse.params.clone().unwrap_or_default(),
            "color": mood.color,
        });
        match out.iter_mut().find(|shelf| shelf.title == mood.group) {
            Some(shelf) => shelf.items.push(item),
            None => out.push(HomeShelf {
                key: format!("youtube:moods:{}", mood.group),
                title: mood.group.clone(),
                kind: "moods".to_owned(),
                items: vec![item],
                ..Default::default()
            }),
        }
    }
    out
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
                    | Err(broadcast::error::RecvError::Lagged(_)) => service.reload_youtube_if_shown(),
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
    }

    /// Read the feed again (another identity, or personalised ↔ anonymous) — only a feed someone has looked at.
    pub fn reload_youtube_if_shown(&self) {
        let shown = !self.yt_home.borrow().shelves.is_empty() || self.yt_home.borrow().loading;
        if shown {
            self.refresh_youtube(true);
        }
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
        // Home is shown: on a new day the mixes are built again
        self.publish_mixes();
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
                    // Your Mix also draws from the feed's songs
                    service.publish_mixes();
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
    /// Charts and Moods & genres (the Search page's Browse tiles) go through their own readers: charts retry with
    /// the fallback params, and the mood/genre buttons become "moods" shelves (one per group) whose items open
    /// their own browse page ({kind: "browse", title, browseId, browseParams}).
    pub async fn youtube_browse(&self, target: &BrowseRef) -> Result<Vec<HomeShelf>, ServiceError> {
        match target.browse_id.as_str() {
            "FEmusic_charts" if target.params.is_none() => return self.youtube_charts(None).await,
            "FEmusic_moods_and_genres" => return Ok(mood_shelves(&self.youtube_moods().await?)),
            _ => {}
        }
        let page = self.auth.youtube().browse_shelves(target).await.map_err(yt_error)?;
        Ok(home_shelves(0, &page))
    }
}

#[cfg(test)]
mod tests {
    use md3_model::{ArtistItem, TrackItem};

    use super::*;

    fn shelf(items: Vec<YtItem>) -> Shelf {
        Shelf { title: "Quick picks".into(), items, more: None, continuation: None, ..Default::default() }
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

    #[test]
    fn a_similar_to_shelf_keeps_its_strapline_picture_and_full_page() {
        let album = YtItem::Album(md3_model::AlbumItem { uri: "youtube:album:MPREb".into(), ..Default::default() });
        let similar = Shelf {
            title: "Daft Punk".into(),
            items: vec![album],
            more: Some(BrowseRef { browse_id: "UCdaft".into(), params: Some("ggMIegYIARoCAQI=".into()) }),
            strapline: "Similar to".into(),
            thumbnail: "https://lh3.googleusercontent.com/a=w240-h240-l90-rj".into(),
            ..Default::default()
        };
        let shelf = home_shelf(3, &similar);
        assert_eq!((shelf.strapline.as_str(), shelf.title.as_str()), ("Similar to", "Daft Punk"));
        assert_eq!(shelf.browse_id, "UCdaft");
        assert_eq!(shelf.browse_params, "ggMIegYIARoCAQI=");
        let json = serde_json::to_value(&shelf).unwrap();
        assert_eq!(json["browseId"], "UCdaft");
        assert_eq!(json["thumbnail"], "https://lh3.googleusercontent.com/a=w240-h240-l90-rj");
        // a shelf with no full page leaves the keys out
        let plain = serde_json::to_value(home_shelf(0, &shelf_of_one())).unwrap();
        assert!(plain.get("browseId").is_none() && plain.get("strapline").is_none());
    }

    fn shelf_of_one() -> Shelf {
        shelf(vec![YtItem::Track(TrackItem { uri: "youtube:track:v".into(), ..Default::default() })])
    }
}
