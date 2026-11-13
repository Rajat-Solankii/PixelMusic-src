//! Results of the YouTube Music client. Items are the app's own flat QML models (md3-model) with
//! `youtube:<kind>:<id>` URIs; the structs here only add what those models have no field for.
use md3_model::{AlbumInfo, AlbumItem, ArtistInfo, ArtistItem, PlaylistItem, TrackItem};

pub fn track_uri(video_id: &str) -> String {
    format!("youtube:track:{video_id}")
}

pub fn playlist_uri(playlist_id: &str) -> String {
    format!("youtube:playlist:{playlist_id}")
}

pub fn album_uri(browse_id: &str) -> String {
    format!("youtube:album:{browse_id}")
}

pub fn artist_uri(browse_id: &str) -> String {
    format!("youtube:artist:{browse_id}")
}

/// Anything a shelf, a search or the library can list.
#[derive(Clone, Debug, PartialEq)]
pub enum YtItem {
    Track(TrackItem),
    Album(AlbumItem),
    Artist(ArtistItem),
    Playlist(PlaylistItem),
}

impl YtItem {
    pub fn uri(&self) -> &str {
        match self {
            Self::Track(track) => &track.uri,
            Self::Album(album) => &album.uri,
            Self::Artist(artist) => &artist.uri,
            Self::Playlist(playlist) => &playlist.uri,
        }
    }

    pub fn into_track(self) -> Option<TrackItem> {
        match self {
            Self::Track(track) => Some(track),
            _ => None,
        }
    }
}

/// A browse target: "see all" of a shelf, a mood, lyrics of a track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowseRef {
    pub browse_id: String,
    pub params: Option<String>,
}

/// A titled row of items (home, explore, artist page, search summary).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shelf {
    pub title: String,
    pub items: Vec<YtItem>,
    /// "see all"
    pub more: Option<BrowseRef>,
    pub continuation: Option<String>,
    /// the small line above the title ("Similar to" over an artist's name, "Mixed for you"), "" when none
    pub strapline: String,
    /// the header's own picture (the artist of a "Similar to" shelf), "" when none
    pub thumbnail: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShelvesPage {
    pub shelves: Vec<Shelf>,
    pub continuation: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemsPage {
    pub items: Vec<YtItem>,
    pub continuation: Option<String>,
}

/// A playlist row: the track plus the id of this occurrence, which removing and reordering need.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaylistTrack {
    pub track: TrackItem,
    /// "" when the row has none (search results, album tracks)
    pub set_video_id: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TracksPage {
    pub tracks: Vec<PlaylistTrack>,
    pub continuation: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaylistPage {
    pub info: PlaylistItem,
    pub tracks: Vec<PlaylistTrack>,
    pub continuation: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Suggestions {
    pub queries: Vec<String>,
    pub items: Vec<YtItem>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AlbumPage {
    pub info: AlbumInfo,
    /// the album's own playlist (`OLAK5uy_…`), "" when unknown
    pub playlist_id: String,
    pub description: String,
    pub tracks: Vec<TrackItem>,
    pub other_versions: Vec<AlbumItem>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArtistPage {
    pub info: ArtistInfo,
    /// channel to subscribe to, "" when the page has no subscribe button
    pub channel_id: String,
    pub shelves: Vec<Shelf>,
}

/// The queue the `next` endpoint returns (radio, up next).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NextPage {
    pub title: String,
    pub tracks: Vec<TrackItem>,
    pub current_index: Option<usize>,
    pub continuation: Option<String>,
    pub lyrics: Option<BrowseRef>,
    pub related: Option<BrowseRef>,
}

/// A mood or genre tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mood {
    /// the section it sits in ("Moods & moments", "Genres")
    pub group: String,
    pub title: String,
    pub browse: BrowseRef,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountInfo {
    pub name: String,
    pub email: String,
    pub avatar: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchFilter {
    Songs,
    Videos,
    Albums,
    Artists,
    FeaturedPlaylists,
    CommunityPlaylists,
}

impl SearchFilter {
    /// The `params` value of the search request (as the reference client sends it).
    pub fn params(self) -> &'static str {
        match self {
            Self::Songs => "EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D",
            Self::Videos => "EgWKAQIQAWoKEAkQChAFEAMQBA%3D%3D",
            Self::Albums => "EgWKAQIYAWoKEAkQChAFEAMQBA%3D%3D",
            Self::Artists => "EgWKAQIgAWoKEAkQChAFEAMQBA%3D%3D",
            Self::FeaturedPlaylists => "EgeKAQQoADgBagwQDhAKEAMQBRAJEAQ%3D",
            Self::CommunityPlaylists => "EgeKAQQoAEABagoQAxAEEAoQCRAF",
        }
    }
}
