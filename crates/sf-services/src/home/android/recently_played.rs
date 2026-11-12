//! Recently Played section: local listening history.

use md3_model::{HomeShelf, LibraryEntry, PlayContext, TrackItem};
use serde_json::json;

use crate::home::android::{AndroidHomeSection, SectionAction};
use crate::history::PlayRecord;

pub const RECENTLY_PLAYED_COUNT: usize = 16;

/// Recently Played section item (context-based, like the current implementation).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentlyPlayedItem {
    #[serde(flatten)]
    pub entry: LibraryEntry,
    pub played_at: String,
    pub context_uri: Option<String>,
    pub last_track: Option<TrackItem>,
}

/// Build recently played section from local history.
pub fn build_recently_played(
    plays: Vec<PlayRecord>,
    playlists: &[md3_model::PlaylistItem],
) -> AndroidHomeSection {
    let items: Vec<serde_json::Value> = plays
        .into_iter()
        .filter_map(|play| recent_item(&play, playlists))
        .take(RECENTLY_PLAYED_COUNT)
        .map(|item| json!(item))
        .collect();

    AndroidHomeSection {
        key: crate::home::android::SECTION_RECENTLY_PLAYED.to_string(),
        title: mf_i18n::t("Recently played"),
        kind: "recentTiles".to_string(),
        items,
        actions: vec![
            SectionAction {
                id: "seeAll".to_string(),
                label: mf_i18n::t("See all"),
                icon: "history".to_string(),
            }
        ],
        ..Default::default()
    }
}

/// Convert a play record to a recently played item.
fn recent_item(
    play: &PlayRecord,
    playlists: &[md3_model::PlaylistItem],
) -> Option<RecentlyPlayedItem> {
    let track = &play.track;
    let context = &play.context;
    
    // Build the library entry based on context
    let entry = build_entry(track, context, playlists)?;
    
    Some(RecentlyPlayedItem {
        entry,
        played_at: play.played_at.clone(),
        context_uri: context.context_uri.clone(),
        last_track: Some(track.clone()),
    })
}

/// Build library entry from play context.
fn build_entry(
    track: &TrackItem,
    context: &PlayContext,
    playlists: &[md3_model::PlaylistItem],
) -> Option<LibraryEntry> {
    use sf_api::util::{uri_kind, uri_id};
    
    let context_uri = context.context_uri.as_deref().unwrap_or("");
    let context_title = context.title.as_str();
    let context_cover = if context.cover.is_empty() {
        &track.cover
    } else {
        &context.cover
    };

    // Helper to create base entry
    let make_entry = |kind: &str, id: &str, uri: &str, title: &str, subtitle: String, cover: &str| {
        LibraryEntry {
            kind: kind.to_string(),
            id: id.to_string(),
            uri: uri.to_string(),
            title: title.to_string(),
            subtitle,
            cover: cover.to_string(),
            covers: if cover.is_empty() { Vec::new() } else { vec![cover.to_string()] },
            read_only: true,
            played_at: String::new(),
            ..Default::default()
        }
    };

    // Check for radio/station
    if let Some(seed) = sf_api::util::station_seed(context_uri) {
        let title = if context_title.is_empty() {
            format!("{} Radio", track.title)
        } else {
            context_title.to_string()
        };
        return Some(make_entry("radio", &seed, context_uri, &title, mf_i18n::t("Radio"), context_cover));
    }

    // Check for liked songs
    if context_uri.ends_with(":collection") {
        return Some(make_entry(
            "liked",
            "liked",
            context_uri,
            &mf_i18n::t("Liked Songs"),
            mf_i18n::t("Playlist"),
            ""
        ));
    }

    // Check by URI kind
    if let Some((kind, id)) = uri_kind(context_uri).zip(uri_id(context_uri)) {
        match kind {
            "playlist" => {
                // Try to find in known playlists
                if let Some(p) = playlists.iter().find(|p| p.playlist_id == id) {
                    let mut entry = LibraryEntry {
                        kind: "playlist".to_string(),
                        id: p.playlist_id.clone(),
                        uri: p.uri.clone(),
                        title: p.title.clone(),
                        subtitle: mf_i18n::tf("Playlist · {0}", &[&p.owner]),
                        cover: p.cover.clone(),
                        covers: if p.cover.is_empty() { Vec::new() } else { vec![p.cover.clone()] },
                        owner: p.owner.clone(),
                        owner_id: p.owner_id.clone(),
                        can_edit: p.can_edit,
                        read_only: !p.can_edit,
                        track_count: p.track_count,
                        ..Default::default()
                    };
                    entry.played_at = String::new();
                    return Some(entry);
                }
                let title = if context_title.is_empty() {
                    mf_i18n::t("Playlist")
                } else {
                    context_title.to_string()
                };
                return Some(make_entry("playlist", id, context_uri, &title, mf_i18n::t("Playlist"), context_cover));
            }
            "album" => {
                let title = if context_title.is_empty() {
                    &track.album
                } else {
                    context_title
                };
                let subtitle = if track.artists.is_empty() {
                    mf_i18n::t("Album")
                } else {
                    mf_i18n::tf("Album · {0}", &[&track.artists])
                };
                return Some(make_entry("album", id, context_uri, title, subtitle, context_cover));
            }
            "artist" => {
                let name = track.artist_refs.iter().find(|a| a.id == id).map(|a| a.name.as_str()).unwrap_or("");
                let title = if context_title.is_empty() { name } else { context_title };
                if title.is_empty() {
                    return None;
                }
                return Some(make_entry("artist", id, context_uri, title, mf_i18n::t("Artist"), context_cover));
            }
            "show" => {
                let title = if context_title.is_empty() { &track.album } else { context_title };
                return Some(make_entry("show", id, context_uri, title, mf_i18n::t("Podcast"), context_cover));
            }
            _ => {}
        }
    }

    // Episode without context
    if track.is_episode {
        if track.show_id.is_empty() {
            return None;
        }
        let uri = format!("spotify:show:{}", track.show_id);
        return Some(make_entry("show", &track.show_id, &uri, &track.album, mf_i18n::t("Podcast"), &track.cover));
    }

    // Default to album
    if track.album_id.is_empty() {
        return None;
    }
    let uri = format!("spotify:album:{}", track.album_id);
    let subtitle = if track.artists.is_empty() {
        mf_i18n::t("Album")
    } else {
        mf_i18n::tf("Album · {0}", &[&track.artists])
    };
    Some(make_entry("album", &track.album_id, &uri, &track.album, subtitle, &track.cover))
}

#[cfg(test)]
mod tests {
    use super::*;
    use md3_model::{ArtistRef, TrackItem};

    fn test_track(id: &str) -> TrackItem {
        TrackItem {
            track_id: id.to_string(),
            uri: format!("spotify:track:{}", id),
            title: format!("Song {}", id),
            album: format!("Album {}", id),
            album_id: format!("al{}", id),
            artists: "Test Artist".to_string(),
            artist_refs: vec![ArtistRef { id: "artist1".to_string(), name: "Test Artist".to_string() }],
            cover: "cover.jpg".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_build_entry_album() {
        let track = test_track("1");
        let context = PlayContext::new("album").with_uri("spotify:album:al1");
        let entry = build_entry(&track, &context, &[]).unwrap();
        
        assert_eq!(entry.kind, "album");
        assert_eq!(entry.title, "Album 1");
    }

    #[test]
    fn test_build_entry_artist() {
        let track = test_track("1");
        let context = PlayContext::new("artist").with_uri("spotify:artist:artist1");
        let entry = build_entry(&track, &context, &[]).unwrap();
        
        assert_eq!(entry.kind, "artist");
        assert_eq!(entry.title, "Test Artist");
    }
}
