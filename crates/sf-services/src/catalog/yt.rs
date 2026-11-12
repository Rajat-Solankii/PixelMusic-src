//! YouTube Music side of `CatalogService`. `album`, `artist` and `track` route here when given a
//! `youtube:<kind>:<id>` URI (`uri_provider`); search has its own entry (`search_youtube`), since a query
//! names no service. Results come back in the same page types the Spotify catalog uses.
use md3_model::{AlbumItem, ArtistItem, PlaylistItem, TrackItem};

use super::{AlbumPage, ArtistPage, Cached, CatalogService, SearchKind, SearchResults};
use crate::ServiceError;
use crate::youtube::{self, SearchFilter, Shelf, YouTubeError, YtItem};

/// The id of a `youtube:<kind>:<id>` URI of that kind.
pub(super) fn youtube_id<'a>(uri: &'a str, kind: &str) -> Option<&'a str> {
    let rest = uri.strip_prefix("youtube:")?;
    rest.strip_prefix(kind)?.strip_prefix(':').filter(|id| !id.is_empty())
}

pub(crate) fn service_error(error: YouTubeError) -> ServiceError {
    ServiceError::Other(error.to_string())
}

/// Items of shelves sorted into the result lists.
#[derive(Default)]
struct Buckets {
    tracks: Vec<TrackItem>,
    albums: Vec<AlbumItem>,
    artists: Vec<ArtistItem>,
    playlists: Vec<PlaylistItem>,
}

impl Buckets {
    fn add(&mut self, item: YtItem) {
        match item {
            YtItem::Track(track) => self.tracks.push(track),
            YtItem::Album(album) => self.albums.push(album),
            YtItem::Artist(artist) => self.artists.push(artist),
            YtItem::Playlist(playlist) => self.playlists.push(playlist),
        }
    }

    fn of(items: impl IntoIterator<Item = YtItem>) -> Self {
        let mut buckets = Self::default();
        for item in items {
            buckets.add(item);
        }
        buckets
    }
}

/// Artist shelves → the artist page: songs first, albums vs. singles/videos/appearances by shelf title order
/// (the first album shelf is the discography, later ones go to "also"), artists = similar.
pub(super) fn artist_page(page: youtube::ArtistPage) -> ArtistPage {
    let mut result = ArtistPage { info: page.info, ..Default::default() };
    let mut first_albums = true;
    for Shelf { items, .. } in page.shelves {
        let buckets = Buckets::of(items);
        if result.popular.is_empty() {
            result.popular = buckets.tracks;
        }
        if !buckets.albums.is_empty() {
            if first_albums {
                result.albums = buckets.albums;
                first_albums = false;
            } else {
                result.also_albums.extend(buckets.albums);
            }
        }
        result.similar.extend(buckets.artists);
    }
    result
}

fn filter_of(kind: SearchKind) -> Option<SearchFilter> {
    match kind {
        SearchKind::Tracks => Some(SearchFilter::Songs),
        SearchKind::Albums => Some(SearchFilter::Albums),
        SearchKind::Artists => Some(SearchFilter::Artists),
        SearchKind::Playlists => Some(SearchFilter::FeaturedPlaylists),
        SearchKind::All | SearchKind::Shows | SearchKind::Episodes => None,
    }
}

impl CatalogService {
    fn youtube(&self) -> std::sync::Arc<youtube::YouTubeService> {
        self.auth.youtube()
    }

    pub(super) async fn yt_album(&self, browse_id: &str) -> Result<AlbumPage, ServiceError> {
        let key = format!("youtube:album:{browse_id}");
        if let Some(Cached::Album(page)) = self.cached(&key) {
            return Ok(page);
        }
        let album = self.youtube().album(browse_id).await.map_err(service_error)?;
        let page = AlbumPage { info: album.info, tracks: album.tracks };
        self.store(key, Cached::Album(page.clone()));
        Ok(page)
    }

    pub(super) async fn yt_artist(&self, browse_id: &str) -> Result<ArtistPage, ServiceError> {
        let key = format!("youtube:artist:{browse_id}");
        if let Some(Cached::Artist(page)) = self.cached(&key) {
            return Ok(page);
        }
        let page = artist_page(self.youtube().artist(browse_id).await.map_err(service_error)?);
        self.store(key, Cached::Artist(page.clone()));
        Ok(page)
    }

    /// The song as the `next` endpoint describes it (YouTube has no single-song metadata call).
    pub(super) async fn yt_track(&self, video_id: &str) -> Result<TrackItem, ServiceError> {
        let key = format!("youtube:track:{video_id}");
        if let Some(Cached::Track(item)) = self.cached(&key) {
            return Ok(item);
        }
        let page = self.youtube().next(Some(video_id), None, None, false).await.map_err(service_error)?;
        let uri = youtube::track_uri(video_id);
        let track = page
            .current_index
            .and_then(|at| page.tracks.get(at))
            .filter(|track| track.uri == uri)
            .or_else(|| page.tracks.iter().find(|track| track.uri == uri))
            .cloned()
            .ok_or_else(|| service_error(YouTubeError::Unexpected("track in next")))?;
        self.store(key, Cached::Track(track.clone()));
        Ok(track)
    }

    /// Search YouTube Music. `All`: the summary page (top result first); one kind: that filter's list.
    /// Shows and episodes: empty (YouTube Music has none).
    pub async fn search_youtube(&self, query: &str, kind: SearchKind) -> Result<SearchResults, ServiceError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(SearchResults::default());
        }
        let yt = self.youtube();
        let (buckets, top) = match filter_of(kind) {
            Some(filter) => (Buckets::of(yt.search(query, filter).await.map_err(service_error)?.items), None),
            None if kind == SearchKind::All => {
                let shelves = yt.search_summary(query).await.map_err(service_error)?;
                summary_results(shelves)
            }
            None => (Buckets::default(), None),
        };
        Ok(results_of(buckets, top))
    }

    /// Query completions and suggested items for the search box (anonymous).
    pub async fn youtube_suggestions(&self, input: &str) -> Result<youtube::Suggestions, ServiceError> {
        self.youtube().suggestions(input).await.map_err(service_error)
    }

    /// A playlist's header and first page (to show a YouTube playlist that is not in the library).
    pub async fn youtube_playlist(&self, uri: &str) -> Result<youtube::PlaylistPage, ServiceError> {
        let id = youtube_id(uri, "playlist").ok_or_else(|| service_error(YouTubeError::PlaylistUnavailable))?;
        self.youtube().playlist(id).await.map_err(service_error)
    }
}

/// The summary page's shelves → the result lists, and its top result ("Top result" shelf: the first item of
/// the first shelf).
fn summary_results(shelves: Vec<Shelf>) -> (Buckets, Option<YtItem>) {
    let top = shelves.first().and_then(|shelf| shelf.items.first()).cloned();
    (Buckets::of(shelves.into_iter().flat_map(|shelf| shelf.items)), top)
}

/// Result lists → `SearchResults`: totals are the counts YouTube returned (it gives no grand totals), the
/// best result is the top result, in the `{type, item}` form the Spotify search used.
fn results_of(buckets: Buckets, top: Option<YtItem>) -> SearchResults {
    let count = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
    let totals = md3_model::SearchTotals {
        tracks: count(buckets.tracks.len()),
        albums: count(buckets.albums.len()),
        artists: count(buckets.artists.len()),
        playlists: count(buckets.playlists.len()),
        ..Default::default()
    };
    SearchResults {
        tracks: buckets.tracks,
        albums: buckets.albums,
        artists: buckets.artists,
        playlists: buckets.playlists,
        totals,
        best: top.and_then(search_best),
        ..Default::default()
    }
}

fn search_best(item: YtItem) -> Option<md3_model::SearchBest> {
    let (kind, value) = match item {
        YtItem::Track(track) => ("track", serde_json::to_value(track)),
        YtItem::Album(album) => ("album", serde_json::to_value(album)),
        YtItem::Artist(artist) => ("artist", serde_json::to_value(artist)),
        YtItem::Playlist(playlist) => ("playlist", serde_json::to_value(playlist)),
    };
    match value {
        Ok(serde_json::Value::Object(item)) => Some(md3_model::SearchBest { kind: kind.to_owned(), item }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_top_result_becomes_the_best_and_counts_fill_the_totals() {
        let artist = YtItem::Artist(ArtistItem { uri: youtube::artist_uri("UCtop"), ..Default::default() });
        let (buckets, top) = summary_results(vec![shelf(vec![artist]), shelf(vec![track("a"), track("b"), album("c")])]);
        let results = results_of(buckets, top);
        let best = results.best.expect("top result");
        assert_eq!(best.kind, "artist");
        assert_eq!(best.item.get("uri").and_then(|v| v.as_str()), Some("youtube:artist:UCtop"));
        assert_eq!((results.totals.tracks, results.totals.albums, results.totals.artists), (2, 1, 1));
        assert_eq!(results.tracks.len(), 2);
        assert!(results_of(Buckets::default(), None).best.is_none());
    }

    #[test]
    fn youtube_ids_come_only_from_their_own_kind() {
        assert_eq!(youtube_id("youtube:album:MPREb_x", "album"), Some("MPREb_x"));
        assert_eq!(youtube_id("youtube:artist:UCabc", "album"), None);
        assert_eq!(youtube_id("youtube:album:", "album"), None);
        assert_eq!(youtube_id("spotify:album:abc", "album"), None);
        assert_eq!(youtube_id("abc", "album"), None);
    }

    fn track(id: &str) -> YtItem {
        YtItem::Track(TrackItem { uri: youtube::track_uri(id), ..Default::default() })
    }

    fn album(id: &str) -> YtItem {
        YtItem::Album(AlbumItem { uri: youtube::album_uri(id), ..Default::default() })
    }

    fn shelf(items: Vec<YtItem>) -> Shelf {
        Shelf { title: String::new(), items, more: None, continuation: None }
    }

    #[test]
    fn artist_shelves_fill_the_artist_page() {
        let page = youtube::ArtistPage {
            shelves: vec![
                shelf(vec![track("a"), track("b")]),
                shelf(vec![album("discography")]),
                shelf(vec![album("single")]),
                shelf(vec![YtItem::Artist(ArtistItem { uri: youtube::artist_uri("UCx"), ..Default::default() })]),
                shelf(vec![track("later")]),
            ],
            ..Default::default()
        };
        let page = artist_page(page);
        let uris = |items: &[TrackItem]| items.iter().map(|item| item.uri.clone()).collect::<Vec<_>>();
        assert_eq!(uris(&page.popular), ["youtube:track:a", "youtube:track:b"]);
        assert_eq!(page.albums.len(), 1);
        assert_eq!(page.albums[0].uri, "youtube:album:discography");
        assert_eq!(page.also_albums[0].uri, "youtube:album:single");
        assert_eq!(page.similar[0].uri, "youtube:artist:UCx");
    }
}
