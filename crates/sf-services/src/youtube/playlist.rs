//! Playlist pages (`YouTube.playlist` / `playlistContinuation` / `YouTubeRepository.fullPlaylist`).
use super::client::Login;
use super::error::Result;
use super::library::LIKED_MUSIC;
use super::models::{PlaylistPage, TracksPage};
use super::{PageGuard, YouTubeService, parsers};

/// The browse id of a playlist: its id with the `VL` prefix.
pub(crate) fn playlist_browse_id(playlist_id: &str) -> String {
    if playlist_id.starts_with("VL") { playlist_id.to_owned() } else { format!("VL{playlist_id}") }
}

/// "Liked Music" only exists for an account; other playlists can be public.
fn login_for(playlist_id: &str) -> Login {
    if playlist_id.trim_start_matches("VL") == LIKED_MUSIC { Login::Required } else { Login::IfSignedIn }
}

impl YouTubeService {
    /// Header and first page of tracks.
    pub async fn playlist(&self, playlist_id: &str) -> Result<PlaylistPage> {
        let response = self.browse(&playlist_browse_id(playlist_id), None, login_for(playlist_id)).await?;
        parsers::playlist_page(&response, playlist_id)
    }

    /// The tracks after `continuation`.
    pub async fn playlist_continuation(&self, continuation: &str) -> Result<TracksPage> {
        let response = self.browse_continuation(continuation, Login::IfSignedIn).await?;
        Ok(parsers::playlist_continuation(&response))
    }

    /// The whole playlist: every page under the page guard. `progress` gets the number of tracks read so far.
    /// The returned page has no continuation unless the guard stopped early.
    pub async fn playlist_full(&self, playlist_id: &str, progress: impl Fn(usize)) -> Result<PlaylistPage> {
        let mut page = self.playlist(playlist_id).await?;
        progress(page.tracks.len());
        let mut guard = PageGuard::new();
        while let Some(token) = page.continuation.take() {
            if !guard.admit(&token) {
                page.continuation = Some(token);
                break;
            }
            let next = self.playlist_continuation(&token).await?;
            let more = guard.record(next.tracks.len());
            page.tracks.extend(next.tracks);
            progress(page.tracks.len());
            page.continuation = next.continuation;
            if !more {
                break;
            }
        }
        if page.continuation.is_some() {
            tracing::debug!("stopped reading YouTube playlist {playlist_id} after {} tracks", page.tracks.len());
        }
        Ok(page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_ids_get_one_vl_prefix() {
        assert_eq!(playlist_browse_id("PLabc"), "VLPLabc");
        assert_eq!(playlist_browse_id("VLPLabc"), "VLPLabc");
        assert_eq!(playlist_browse_id("LM"), "VLLM");
        assert_eq!(login_for("LM"), Login::Required);
        assert_eq!(login_for("VLLM"), Login::Required);
        assert_eq!(login_for("PLabc"), Login::IfSignedIn);
    }
}
