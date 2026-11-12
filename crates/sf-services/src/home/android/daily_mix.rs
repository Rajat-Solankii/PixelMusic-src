//! Daily Mix section: merged with YT "Daily Discover".

use md3_model::{HomeShelf, LibraryEntry, TrackItem};
use serde_json::json;

use crate::home::android::{AndroidHomeSection, SectionAction};
use crate::youtube::{Shelf, YtItem};

/// Daily Mix / Daily Discover section.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DailyMixSection {
    pub title: String,
    pub tracks: Vec<TrackItem>,
    pub mix_uri: String,
    pub cover: String,
}

impl DailyMixSection {
    pub fn new(title: impl Into<String>, tracks: Vec<TrackItem>, mix_uri: impl Into<String>) -> Self {
        let title = title.into();
        let mix_uri = mix_uri.into();
        let cover = tracks.first().map(|t| t.cover.clone()).unwrap_or_default();
        
        Self {
            title,
            tracks,
            mix_uri,
            cover,
        }
    }

    /// Convert to AndroidHomeSection for QML rendering.
    pub fn to_android_section(&self) -> AndroidHomeSection {
        let items: Vec<serde_json::Value> = self.tracks.iter().map(|t| json!(t)).collect();
        
        AndroidHomeSection {
            key: crate::home::android::SECTION_DAILY_MIX.to_string(),
            title: self.title.clone(),
            kind: "dailyMix".to_string(),
            items,
            actions: vec![
                SectionAction {
                    id: "play".to_string(),
                    label: mf_i18n::t("Play"),
                    icon: "play_arrow".to_string(),
                },
                SectionAction {
                    id: "shuffle".to_string(),
                    label: mf_i18n::t("Shuffle"),
                    icon: "shuffle".to_string(),
                }
            ],
            ..Default::default()
        }
    }
}

/// Extract Daily Discover tracks from YouTube shelves.
pub fn extract_daily_discover(shelves: &[Shelf]) -> Option<Vec<TrackItem>> {
    for shelf in shelves {
        let title_lower = shelf.title.to_lowercase();
        if title_lower.contains("daily discover") || 
           (title_lower.contains("discover") && title_lower.contains("daily")) {
            let tracks: Vec<TrackItem> = shelf.items.iter().filter_map(|item| {
                item.clone().into_track()
            }).collect();
            
            if !tracks.is_empty() {
                return Some(tracks);
            }
        }
    }
    None
}

/// Build Daily Mix section from made_for_you and YouTube shelves.
pub fn build_daily_mix(
    made_for_you: &[LibraryEntry],
    yt_shelves: &[Shelf],
) -> Option<AndroidHomeSection> {
    // Try to find a Daily Mix playlist
    let daily_mix = made_for_you.iter().find(|e| {
        let title_lower = e.title.to_lowercase();
        (title_lower.contains("daily") && title_lower.contains("mix")) ||
        title_lower == "daylist"
    });

    // Try to extract from Daily Discover shelf
    let discover_tracks = extract_daily_discover(yt_shelves);

    if let Some(entry) = daily_mix {
        let mut section = AndroidHomeSection {
            key: crate::home::android::SECTION_DAILY_MIX.to_string(),
            title: entry.title.clone(),
            kind: "dailyMix".to_string(),
            items: Vec::new(),
            actions: vec![
                SectionAction {
                    id: "play".to_string(),
                    label: mf_i18n::t("Play"),
                    icon: "play_arrow".to_string(),
                },
                SectionAction {
                    id: "shuffle".to_string(),
                    label: mf_i18n::t("Shuffle"),
                    icon: "shuffle".to_string(),
                }
            ],
            ..Default::default()
        };

        // If we have discover tracks, add them as items
        if let Some(tracks) = discover_tracks {
            let items: Vec<serde_json::Value> = tracks.into_iter().map(|t| json!(t)).collect();
            section.items = items;
        }

        Some(section)
    } else if let Some(tracks) = discover_tracks {
        // Use Daily Discover as fallback
        let items: Vec<serde_json::Value> = tracks.into_iter().map(|t| json!(t)).collect();
        
        Some(AndroidHomeSection {
            key: crate::home::android::SECTION_DAILY_MIX.to_string(),
            title: mf_i18n::t("Daily Discover"),
            kind: "dailyMix".to_string(),
            items,
            actions: vec![
                SectionAction {
                    id: "play".to_string(),
                    label: mf_i18n::t("Play"),
                    icon: "play_arrow".to_string(),
                },
                SectionAction {
                    id: "shuffle".to_string(),
                    label: mf_i18n::t("Shuffle"),
                    icon: "shuffle".to_string(),
                }
            ],
            ..Default::default()
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daily_mix_section() {
        let tracks = vec![
            TrackItem {
                track_id: "1".to_string(),
                title: "Song 1".to_string(),
                cover: "cover1.jpg".to_string(),
                ..Default::default()
            },
        ];
        
        let section = DailyMixSection::new("Daily Mix 1", tracks, "spotify:playlist:dm1");
        let android = section.to_android_section();
        
        assert_eq!(android.key, "dailyMix");
        assert_eq!(android.title, "Daily Mix 1");
        assert_eq!(android.kind, "dailyMix");
        assert_eq!(android.actions.len(), 2);
    }
}
