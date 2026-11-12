//! Quick Picks section: ~20 diversified songs from YT home + radio songs.

use std::collections::HashSet;

use md3_model::{HomeShelf, LibraryEntry, TrackItem};
use serde_json::json;

use crate::home::android::{AndroidHomeSection, SectionAction, dedupe_tracks, diversify_by_artist, video_id_from_uri};

pub const QUICK_PICKS_COUNT: usize = 20;
pub const MAX_PER_ARTIST: usize = 2;
pub const RECENT_RADIO_SEEDS: usize = 3;
pub const ARTIST_RADIO_SEEDS: usize = 2;

/// Section builder for Quick Picks.
pub struct QuickPicksSection;

impl QuickPicksSection {
    pub fn new() -> Self {
        Self
    }

    /// Build from pre-computed tracks (simplified version).
    pub fn from_tracks(tracks: Vec<TrackItem>) -> AndroidHomeSection {
        let tracks = diversify_by_artist(dedupe_tracks(tracks), MAX_PER_ARTIST);
        let tracks: Vec<_> = tracks.into_iter().take(QUICK_PICKS_COUNT).collect();
        
        let items: Vec<serde_json::Value> = tracks
            .iter()
            .map(|t| {
                json!({
                    "uri": t.uri,
                    "trackId": t.track_id,
                    "title": t.title,
                    "artists": t.artists,
                    "cover": t.cover,
                    "durationMs": t.duration_ms,
                    "explicit": t.explicit,
                })
            })
            .collect();

        AndroidHomeSection {
            key: crate::home::android::SECTION_QUICK_PICKS.to_string(),
            title: mf_i18n::t("Quick Picks"),
            kind: "trackGrid".to_string(),
            items,
            actions: vec![
                SectionAction {
                    id: "seeAll".to_string(),
                    label: mf_i18n::t("See all"),
                    icon: "arrow_forward".to_string(),
                }
            ],
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use md3_model::{ArtistRef, TrackItem};

    fn test_track(id: &str, artist: &str) -> TrackItem {
        TrackItem {
            track_id: id.to_string(),
            uri: format!("youtube:track:{}", id),
            title: format!("Song {}", id),
            artists: artist.to_string(),
            artist_refs: vec![ArtistRef { id: artist.to_string(), name: artist.to_string() }],
            cover: "cover.jpg".to_string(),
            duration_ms: 180000,
            ..Default::default()
        }
    }

    #[test]
    fn test_quick_picks_from_tracks() {
        let tracks = vec![
            test_track("1", "Artist A"),
            test_track("2", "Artist A"),
            test_track("3", "Artist A"),
            test_track("4", "Artist B"),
        ];
        
        let section = QuickPicksSection::from_tracks(tracks);
        assert_eq!(section.key, "quickPicks");
        assert_eq!(section.kind, "trackGrid");
        // Should be diversified to max 2 per artist
        assert!(section.items.len() <= 3);
    }
}
