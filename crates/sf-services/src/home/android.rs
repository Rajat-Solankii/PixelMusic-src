//! Android-style Home sections - core types and utilities.

pub mod daily_mix;
pub mod greeting;
pub mod new_releases;
pub mod quick_picks;
pub mod recently_played;
pub mod your_mix;

pub use daily_mix::{DailyMixSection, build_daily_mix};
pub use greeting::{GreetingSection, greeting_with_name, localized_greeting};
pub use new_releases::{NewReleasesSection, build_new_releases};
pub use quick_picks::{QuickPicksSection, build_quick_picks};
pub use recently_played::{build_recently_played, RECENTLY_PLAYED_COUNT};
pub use your_mix::{YourMixSection, build_your_mix};

use md3_model::{HomeShelf, TrackItem};
use serde::Serialize;

/// Section key constants for Android-style home.
pub const SECTION_GREETING: &str = "greeting";
pub const SECTION_QUICK_PICKS: &str = "quickPicks";
pub const SECTION_YOUR_MIX: &str = "yourMix";
pub const SECTION_DAILY_MIX: &str = "dailyMix";
pub const SECTION_RECENTLY_PLAYED: &str = "recentlyPlayed";
pub const SECTION_NEW_RELEASES: &str = "newReleasesFromFavorites";
pub const SECTION_LISTENING_STATS: &str = "listeningStats";

/// A specialized home section with custom rendering needs.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidHomeSection {
    pub key: String,
    pub title: String,
    pub kind: String,
    pub items: Vec<serde_json::Value>,
    /// For sections that need custom actions (shuffle play, etc.)
    pub actions: Vec<SectionAction>,
    /// Optional collage covers for Your Mix
    pub collage_covers: Vec<String>,
    /// Greeting text (for greeting section)
    pub greeting_text: Option<String>,
    /// User name (for greeting section)
    pub user_name: Option<String>,
}

/// Action available on a section header.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionAction {
    pub id: String,
    pub label: String,
    pub icon: String,
}

/// Convert Android sections to standard HomeShelf for compatibility.
impl From<AndroidHomeSection> for HomeShelf {
    fn from(section: AndroidHomeSection) -> Self {
        HomeShelf {
            key: section.key,
            title: section.title,
            kind: section.kind,
            items: section.items,
        }
    }
}

/// Deduplicate tracks by URI, keeping the first occurrence.
pub fn dedupe_tracks(tracks: Vec<TrackItem>) -> Vec<TrackItem> {
    let mut seen = std::collections::HashSet::new();
    tracks
        .into_iter()
        .filter(|t| !t.uri.is_empty() && seen.insert(t.uri.clone()))
        .collect()
}

/// Diversify tracks: max N tracks per artist.
pub fn diversify_by_artist(tracks: Vec<TrackItem>, max_per_artist: usize) -> Vec<TrackItem> {
    let mut artist_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    tracks
        .into_iter()
        .filter(|t| {
            let artist_key = t.artists.clone();
            let count = artist_counts.entry(artist_key.clone()).or_insert(0);
            if *count < max_per_artist {
                *count += 1;
                true
            } else {
                false
            }
        })
        .collect()
}

/// Extract video ID from YouTube track URI.
pub fn video_id_from_uri(uri: &str) -> Option<&str> {
    uri.strip_prefix("youtube:track:").filter(|id| !id.is_empty())
}

/// Build a radio station URI from a video ID.
pub fn radio_station_uri(video_id: &str) -> String {
    format!("youtube:playlist:RDAMVM{}", video_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use md3_model::TrackItem;

    #[test]
    fn test_diversify_by_artist() {
        let tracks = vec![
            TrackItem { uri: "youtube:track:1".into(), artists: "Artist A".into(), ..Default::default() },
            TrackItem { uri: "youtube:track:2".into(), artists: "Artist A".into(), ..Default::default() },
            TrackItem { uri: "youtube:track:3".into(), artists: "Artist A".into(), ..Default::default() },
            TrackItem { uri: "youtube:track:4".into(), artists: "Artist B".into(), ..Default::default() },
            TrackItem { uri: "youtube:track:5".into(), artists: "Artist C".into(), ..Default::default() },
        ];
        let diversified = diversify_by_artist(tracks, 2);
        assert_eq!(diversified.len(), 4);
        assert_eq!(diversified[0].uri, "youtube:track:1");
        assert_eq!(diversified[1].uri, "youtube:track:2");
        assert_eq!(diversified[2].uri, "youtube:track:4");
        assert_eq!(diversified[3].uri, "youtube:track:5");
    }

    #[test]
    fn test_video_id_from_uri() {
        assert_eq!(video_id_from_uri("youtube:track:abc123"), Some("abc123"));
        assert_eq!(video_id_from_uri("spotify:track:abc"), None);
        assert_eq!(video_id_from_uri("youtube:track:"), None);
    }
}
