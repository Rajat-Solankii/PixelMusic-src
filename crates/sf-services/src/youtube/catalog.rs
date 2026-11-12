//! Albums, artists, the play queue (radio, up next), lyrics, transcripts and the account menu
//! (`YouTube.album` / `artist` / `next` / `lyrics` / `transcript` / `accountInfo`).
use base64::Engine;
use md3_model::TrackItem;
use serde_json::json;

use super::client::{Login, WEB, WEB_REMIX};
use super::error::{Result, YouTubeError};
use super::models::{AccountInfo, AlbumPage, ArtistPage, BrowseRef, NextPage};
use super::{YouTubeService, parsers};

/// The public key the web player sends with `get_transcript`.
const TRANSCRIPT_KEY: &str = "AIzaSyC9XL3ZjWddXya6X74dJoCTL-WEYFDNX3";

/// The radio playlist of a track.
pub fn radio_playlist_id(video_id: &str) -> String {
    format!("RDAMVM{video_id}")
}

/// `params` of `get_transcript`: a tiny protobuf (field 1, length 11) holding the video id.
pub(crate) fn transcript_params(video_id: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(format!("\n\u{b}{video_id}"))
}

/// Album details for tracks read from the album's playlist (those rows name no album, or a shorter one).
fn as_album_tracks(tracks: Vec<TrackItem>, page: &AlbumPage) -> Vec<TrackItem> {
    let album = &page.info.item;
    tracks
        .into_iter()
        .map(|mut track| {
            track.album = album.title.clone();
            track.album_id = album.album_id.clone();
            if track.cover.is_empty() {
                track.cover = album.cover.clone();
                track.cover_large = album.cover_large.clone();
            }
            if track.artist_refs.is_empty() {
                track.artist_refs = album.artist_refs.clone();
                track.artists = album.artists.clone();
            }
            track
        })
        .collect()
}

impl YouTubeService {
    /// An album with every song: read from the album's own playlist, falling back to the songs listed on the
    /// album page when that fails.
    pub async fn album(&self, browse_id: &str) -> Result<AlbumPage> {
        let response = self.browse(browse_id, None, Login::IfSignedIn).await?;
        let mut page = parsers::album_page(&response, browse_id)?;
        if page.playlist_id.is_empty() {
            return Ok(page);
        }
        match self.playlist_full(&page.playlist_id, |_| {}).await {
            Ok(full) if !full.tracks.is_empty() => {
                let tracks = full.tracks.into_iter().map(|row| row.track).collect();
                page.tracks = as_album_tracks(tracks, &page);
                page.info.duration_ms = page.tracks.iter().map(|track| track.duration_ms).sum();
            }
            Ok(_) => {}
            Err(error) if page.tracks.is_empty() => return Err(error),
            Err(error) => tracing::debug!("album playlist {} unavailable, using the page's songs: {error:?}", page.playlist_id),
        }
        Ok(page)
    }

    pub async fn artist(&self, browse_id: &str) -> Result<ArtistPage> {
        let response = self.browse(browse_id, None, Login::IfSignedIn).await?;
        parsers::artist_page(&response, browse_id)
    }

    /// The queue for a track and/or playlist (`next`). With `follow_automix`, an automix preview at the end
    /// of the panel is opened and its tracks appended (the continuation is then the automix one).
    pub async fn next(
        &self,
        video_id: Option<&str>,
        playlist_id: Option<&str>,
        params: Option<&str>,
        follow_automix: bool,
    ) -> Result<NextPage> {
        let (mut page, automix) = self.next_request(video_id, playlist_id, params, None).await?;
        if follow_automix && let Some((automix_id, automix_params)) = automix {
            let (more, _) = self.next_request(None, Some(&automix_id), automix_params.as_deref(), None).await?;
            page.tracks.extend(more.tracks);
            page.continuation = more.continuation;
        }
        Ok(page)
    }

    /// More of a queue: the same playlist and the token of the previous page.
    pub async fn next_continuation(&self, playlist_id: &str, continuation: &str) -> Result<NextPage> {
        Ok(self.next_request(None, Some(playlist_id), None, Some(continuation)).await?.0)
    }

    async fn next_request(
        &self,
        video_id: Option<&str>,
        playlist_id: Option<&str>,
        params: Option<&str>,
        continuation: Option<&str>,
    ) -> Result<(NextPage, Option<(String, Option<String>)>)> {
        let mut body = json!({});
        for (key, value) in [("videoId", video_id), ("playlistId", playlist_id), ("params", params), ("continuation", continuation)] {
            if let Some(value) = value {
                body[key] = json!(value);
            }
        }
        let response = self.client.post(&WEB_REMIX, "next", body, Login::IfSignedIn).await?;
        parsers::next_page(&response)
    }

    /// Radio of a track: the track first, then similar songs (`RDAMVM<videoId>`).
    pub async fn radio(&self, video_id: &str) -> Result<NextPage> {
        self.next(Some(video_id), Some(&radio_playlist_id(video_id)), None, true).await
    }

    /// Plain lyrics of a track, `None` when YouTube Music has none.
    pub async fn lyrics(&self, video_id: &str) -> Result<Option<String>> {
        let page = self.next(Some(video_id), None, None, false).await?;
        match page.lyrics {
            Some(target) => self.lyrics_at(&target).await,
            None => Ok(None),
        }
    }

    /// Lyrics behind a lyrics tab of the queue.
    pub async fn lyrics_at(&self, target: &BrowseRef) -> Result<Option<String>> {
        let response = self.browse(&target.browse_id, target.params.as_deref(), Login::IfSignedIn).await?;
        Ok(parsers::lyrics(&response))
    }

    /// Timed captions of a video as LRC lines, `None` when it has none.
    pub async fn transcript(&self, video_id: &str) -> Result<Option<String>> {
        let body = json!({ "params": transcript_params(video_id) });
        let query = [("key", TRANSCRIPT_KEY)];
        let response = self.client.post_query(&WEB, "get_transcript", body, Login::Never, &query).await?;
        Ok(parsers::transcript(&response))
    }

    /// Name, e-mail and avatar of the signed-in account.
    pub async fn account_info(&self) -> Result<AccountInfo> {
        let response = self.client.post(&WEB_REMIX, "account/account_menu", json!({}), Login::Required).await?;
        parsers::account_info(&response).ok_or(YouTubeError::Unexpected("account menu"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radio_and_transcript_requests_carry_the_video_id() {
        assert_eq!(radio_playlist_id("abc"), "RDAMVMabc");
        // "\n" (field 1, wire type 2), 11 (length), then the id
        assert_eq!(transcript_params("dQw4w9WgXcQ"), "CgtkUXc0dzlXZ1hjUQ==");
    }
}
