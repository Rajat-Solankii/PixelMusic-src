//! New Releases from Favorite Artists section.

use std::collections::HashSet;

use md3_model::{AlbumItem, ArtistItem, HomeShelf, LibraryEntry};
use serde_json::json;

use crate::home::android::{AndroidHomeSection, SectionAction};

pub const NEW_RELEASES_COUNT: usize = 16;

/// New Releases section builder.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewReleasesSection {
    pub title: String,
    pub albums: Vec<AlbumItem>,
}

impl NewReleasesSection {
    pub fn new(albums: Vec<AlbumItem>) -> Self {
        Self {
            title: mf_i18n::t("New releases from favorite artists"),
            albums,
        }
    }

    /// Convert to AndroidHomeSection for QML.
    pub fn to_android_section(&self) -> AndroidHomeSection {
        let items: Vec<serde_json::Value> = self.albums.iter().map(|a| json!(a)).collect();
        
        AndroidHomeSection {
            key: crate::home::android::SECTION_NEW_RELEASES.to_string(),
            title: self.title.clone(),
            kind: "newReleases".to_string(),
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

/// Build new releases section from favorite artists and YouTube releases.
pub fn build_new_releases(
    favorite_artists: &[ArtistItem],
    existing_releases: &[crate::releases::NewRelease],
) -> Option<AndroidHomeSection> {
    if favorite_artists.is_empty() && existing_releases.is_empty() {
        return None;
    }

    // Convert existing releases to albums
    let mut albums: Vec<AlbumItem> = existing_releases
        .iter()
        .map(|r| r.album.clone())
        .collect();

    // Limit and deduplicate
    let mut seen: HashSet<String> = HashSet::new();
    albums.retain(|a| !a.album_id.is_empty() && seen.insert(a.album_id.clone()));
    albums.truncate(NEW_RELEASES_COUNT);

    if albums.is_empty() {
        return None;
    }

    Some(NewReleasesSection::new(albums).to_android_section())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_new_releases_empty() {
        let result = build_new_releases(&[], &[]);
        assert!(result.is_none());
    }

    #[test]
    fn test_new_releases_section() {
        let albums = vec![
            AlbumItem {
                album_id: "al1".to_string(),
                title: "New Album".to_string(),
                artists: "Artist".to_string(),
                cover: "cover.jpg".to_string(),
                ..Default::default()
            },
        ];
        
        let section = NewReleasesSection::new(albums);
        let android = section.to_android_section();
        
        assert_eq!(android.key, "newReleasesFromFavorites");
        assert_eq!(android.kind, "newReleases");
        assert_eq!(android.items.len(), 1);
    }
}
