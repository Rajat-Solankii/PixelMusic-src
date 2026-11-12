//! Your Mix section: header with shuffle-play button and album-art collage.

use md3_model::{HomeShelf, LibraryEntry, TrackItem};
use serde_json::json;

use crate::home::android::{AndroidHomeSection, SectionAction};

pub const COLLAGE_SIZE: usize = 4;

/// Your Mix section with shuffle play and collage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct YourMixSection {
    pub title: String,
    pub subtitle: String,
    pub tracks: Vec<TrackItem>,
    pub collage_covers: Vec<String>,
    pub mix_uri: String,
}

impl YourMixSection {
    pub fn new(title: impl Into<String>, tracks: Vec<TrackItem>, mix_uri: impl Into<String>) -> Self {
        let title = title.into();
        let mix_uri = mix_uri.into();
        let collage_covers = Self::build_collage(&tracks);
        
        Self {
            title: title.clone(),
            subtitle: mf_i18n::t("Made for you"),
            tracks,
            collage_covers,
            mix_uri,
        }
    }

    /// Build a collage of up to 4 unique album covers.
    fn build_collage(tracks: &[TrackItem]) -> Vec<String> {
        let mut covers: Vec<String> = tracks
            .iter()
            .filter_map(|t| {
                if t.cover.is_empty() {
                    None
                } else {
                    Some(t.cover.clone())
                }
            })
            .collect();
        
        // Deduplicate while preserving order
        let mut seen = std::collections::HashSet::new();
        covers.retain(|c| seen.insert(c.clone()));
        covers.truncate(COLLAGE_SIZE);
        covers
    }

    /// Convert to AndroidHomeSection for QML rendering.
    pub fn to_android_section(&self) -> AndroidHomeSection {
        let items: Vec<serde_json::Value> = self.tracks.iter().map(|t| json!(t)).collect();
        
        AndroidHomeSection {
            key: crate::home::android::SECTION_YOUR_MIX.to_string(),
            title: self.title.clone(),
            kind: "yourMix".to_string(),
            items,
            actions: vec![
                SectionAction {
                    id: "shufflePlay".to_string(),
                    label: mf_i18n::t("Shuffle play"),
                    icon: "shuffle".to_string(),
                }
            ],
            collage_covers: self.collage_covers.clone(),
            ..Default::default()
        }
    }

    /// Build from a "Made for you" playlist entry.
    pub fn from_library_entry(entry: &LibraryEntry) -> Option<Self> {
        if entry.uri.is_empty() {
            return None;
        }
        
        Some(Self {
            title: entry.title.clone(),
            subtitle: entry.subtitle.clone(),
            tracks: Vec::new(),
            collage_covers: if entry.cover.is_empty() {
                entry.covers.clone()
            } else {
                vec![entry.cover.clone()]
            },
            mix_uri: entry.uri.clone(),
        })
    }
}

/// Builder for Your Mix section from existing data.
pub fn build_your_mix(
    made_for_you: &[LibraryEntry],
) -> Option<AndroidHomeSection> {
    // Find a "Your Mix" playlist from made_for_you entries
    let mix = made_for_you.iter().find(|e| {
        let title_lower = e.title.to_lowercase();
        title_lower.contains("your mix") || 
        title_lower.contains("mixtape") ||
        (e.kind == "playlist" && title_lower.contains("mix"))
    });

    if let Some(entry) = mix {
        let mut section = YourMixSection::from_library_entry(entry)?.to_android_section();
        // Get more covers from other playlists for the collage
        let additional_covers: Vec<String> = made_for_you
            .iter()
            .filter(|e| e.uri != entry.uri && !e.cover.is_empty())
            .take(3)
            .map(|e| e.cover.clone())
            .collect();
        
        section.collage_covers.extend(additional_covers);
        section.collage_covers.truncate(COLLAGE_SIZE);
        
        Some(section)
    } else {
        // Create a default Your Mix section
        let collage_covers: Vec<String> = made_for_you
            .iter()
            .filter(|e| !e.cover.is_empty())
            .take(COLLAGE_SIZE)
            .map(|e| e.cover.clone())
            .collect();
        
        if collage_covers.is_empty() {
            return None;
        }

        Some(AndroidHomeSection {
            key: crate::home::android::SECTION_YOUR_MIX.to_string(),
            title: mf_i18n::t("Your Mix"),
            kind: "yourMix".to_string(),
            items: Vec::new(),
            actions: vec![
                SectionAction {
                    id: "shufflePlay".to_string(),
                    label: mf_i18n::t("Shuffle play"),
                    icon: "shuffle".to_string(),
                }
            ],
            collage_covers,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use md3_model::{ArtistRef, TrackItem};

    #[test]
    fn test_build_collage() {
        let tracks = vec![
            TrackItem { cover: "cover1.jpg".to_string(), ..Default::default() },
            TrackItem { cover: "cover2.jpg".to_string(), ..Default::default() },
            TrackItem { cover: "cover1.jpg".to_string(), ..Default::default() }, // duplicate
            TrackItem { cover: "cover3.jpg".to_string(), ..Default::default() },
            TrackItem { cover: "cover4.jpg".to_string(), ..Default::default() },
            TrackItem { cover: "cover5.jpg".to_string(), ..Default::default() },
        ];
        
        let collage = YourMixSection::build_collage(&tracks);
        assert_eq!(collage.len(), 4);
        assert!(collage.contains(&"cover1.jpg".to_string()));
        assert!(!collage.contains(&"cover5.jpg".to_string())); // 5th is excluded
    }

    #[test]
    fn test_from_library_entry() {
        let entry = LibraryEntry {
            title: "Your Mix".to_string(),
            subtitle: "Made for you".to_string(),
            cover: "mix_cover.jpg".to_string(),
            uri: "youtube:playlist:mix123".to_string(),
            kind: "playlist".to_string(),
            ..Default::default()
        };
        
        let section = YourMixSection::from_library_entry(&entry).unwrap();
        assert_eq!(section.title, "Your Mix");
        assert_eq!(section.mix_uri, "youtube:playlist:mix123");
    }
}
