//! The signed-in user's library (`YouTube.library` / `YouTubeRepository.userPlaylists`).
use md3_model::PlaylistItem;

use super::client::Login;
use super::error::Result;
use super::models::{ItemsPage, ShelvesPage, YtItem, playlist_uri};
use super::{PageGuard, YouTubeService, parsers};

/// Playlists the user made or saved.
pub const LIKED_PLAYLISTS: &str = "FEmusic_liked_playlists";
/// Albums the user saved.
pub const LIKED_ALBUMS: &str = "FEmusic_liked_albums";
/// Artists of the songs in the library.
pub const LIBRARY_ARTISTS: &str = "FEmusic_library_corpus_track_artists";
/// Listening history.
pub const HISTORY: &str = "FEmusic_history";

/// The id of "Liked Music" (the songs the user liked), which the playlist grid does not list reliably.
pub const LIKED_MUSIC: &str = "LM";
/// "Episodes for later"; not a music playlist.
const EPISODES_FOR_LATER: &str = "SE";

/// The synthetic "Liked Music" entry shown first in the playlist list.
pub fn liked_music() -> PlaylistItem {
    PlaylistItem {
        playlist_id: LIKED_MUSIC.to_owned(),
        uri: playlist_uri(LIKED_MUSIC),
        title: mf_i18n::t("Liked Music"),
        description: mf_i18n::t("Your YouTube Music likes"),
        ..Default::default()
    }
}

/// "Liked Music" first, then every other playlist once, without "Liked Music" / "Episodes for later" again.
pub(crate) fn with_liked_music(items: Vec<YtItem>) -> Vec<PlaylistItem> {
    let mut seen = std::collections::HashSet::new();
    std::iter::once(liked_music())
        .chain(items.into_iter().filter_map(|item| match item {
            YtItem::Playlist(playlist) => Some(playlist),
            _ => None,
        }))
        .filter(|playlist| {
            // the synthetic entry is first, so a listed "LM" is a repeat
            playlist.playlist_id != EPISODES_FOR_LATER && seen.insert(playlist.playlist_id.clone())
        })
        .collect()
}

impl YouTubeService {
    /// First page of a library tab (`LIKED_PLAYLISTS`, `LIKED_ALBUMS`, `LIBRARY_ARTISTS`).
    pub async fn library(&self, browse_id: &str) -> Result<ItemsPage> {
        let response = self.browse(browse_id, None, Login::Required).await?;
        Ok(parsers::library_page(&response))
    }

    /// Next page of a library tab; an empty page has no continuation.
    pub async fn library_continuation(&self, continuation: &str) -> Result<ItemsPage> {
        let response = self.browse_continuation(continuation, Login::Required).await?;
        Ok(parsers::library_continuation(&response))
    }

    /// Every item of a library tab, following continuations under the page guard.
    pub async fn library_all(&self, browse_id: &str) -> Result<Vec<YtItem>> {
        let first = self.library(browse_id).await?;
        let mut items = first.items;
        let mut continuation = first.continuation;
        let mut guard = PageGuard::new();
        while let Some(token) = continuation.take() {
            if !guard.admit(&token) {
                break;
            }
            let page = self.library_continuation(&token).await?;
            let more = guard.record(page.items.len());
            items.extend(page.items);
            if !more {
                break;
            }
            continuation = page.continuation;
        }
        Ok(items)
    }

    /// The user's playlists: "Liked Music" first, then every playlist of `FEmusic_liked_playlists`.
    pub async fn user_playlists(&self) -> Result<Vec<PlaylistItem>> {
        Ok(with_liked_music(self.library_all(LIKED_PLAYLISTS).await?))
    }

    /// Listening history, grouped by day ("Today", "Yesterday", ...).
    pub async fn history(&self) -> Result<ShelvesPage> {
        let response = self.browse(HISTORY, None, Login::Required).await?;
        Ok(parsers::shelves_page(&response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playlist(id: &str) -> YtItem {
        YtItem::Playlist(PlaylistItem { playlist_id: id.to_owned(), uri: playlist_uri(id), ..Default::default() })
    }

    #[test]
    fn liked_music_comes_first_once_and_episodes_for_later_is_dropped() {
        let ids: Vec<String> = with_liked_music(vec![playlist("PLa"), playlist("LM"), playlist("SE"), playlist("PLa"), playlist("PLb")])
            .into_iter()
            .map(|playlist| playlist.playlist_id)
            .collect();
        assert_eq!(ids, ["LM", "PLa", "PLb"]);
        let first = liked_music();
        assert_eq!(first.uri, "youtube:playlist:LM");
        assert_eq!(first.title, "Liked Music");
        assert!(!first.can_edit);
    }
}
