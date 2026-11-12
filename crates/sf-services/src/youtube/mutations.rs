//! Likes, subscriptions and playlist edits (`InnerTube.kt` `likeVideo` … `deletePlaylist`). Every call needs
//! a signed-in account.
use serde_json::{Value, json};

use super::client::{Login, WEB_REMIX};
use super::error::{Result, YouTubeError};
use super::{YouTubeService, parsers};

/// A track's rating.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rating {
    Like,
    Dislike,
    /// neither (removes a like or a dislike)
    None,
}

impl Rating {
    fn endpoint(self) -> &'static str {
        match self {
            Self::Like => "like/like",
            Self::Dislike => "like/dislike",
            Self::None => "like/removelike",
        }
    }
}

fn bare(playlist_id: &str) -> &str {
    playlist_id.strip_prefix("VL").unwrap_or(playlist_id)
}

/// The `browse/edit_playlist` body for some actions.
pub(crate) fn edit_body(playlist_id: &str, actions: Vec<Value>) -> Value {
    json!({ "playlistId": bare(playlist_id), "actions": actions })
}

impl YouTubeService {
    async fn signed_post(&self, endpoint: &str, body: Value) -> Result<Value> {
        self.client.post(&WEB_REMIX, endpoint, body, Login::Required).await
    }

    /// `browse/edit_playlist`; returns the set-video ids of added rows. A response that names a status other
    /// than success is an error (one without a status is taken as done, as the reference client does).
    async fn edit_playlist(&self, playlist_id: &str, actions: Vec<Value>) -> Result<Vec<String>> {
        let response = self.signed_post("browse/edit_playlist", edit_body(playlist_id, actions)).await?;
        let (succeeded, added) = parsers::edit_result(&response);
        if !succeeded && response["status"].is_string() {
            return Err(YouTubeError::Unexpected("playlist edit status"));
        }
        Ok(added)
    }

    pub async fn rate_track(&self, video_id: &str, rating: Rating) -> Result<()> {
        self.signed_post(rating.endpoint(), json!({ "target": { "videoId": video_id } })).await.map(drop)
    }

    /// Save (`true`) or remove (`false`) a playlist or album playlist in the library.
    pub async fn like_playlist(&self, playlist_id: &str, like: bool) -> Result<()> {
        let rating = if like { Rating::Like } else { Rating::None };
        self.signed_post(rating.endpoint(), json!({ "target": { "playlistId": bare(playlist_id) } })).await.map(drop)
    }

    pub async fn subscribe(&self, channel_id: &str, subscribe: bool) -> Result<()> {
        let endpoint = if subscribe { "subscription/subscribe" } else { "subscription/unsubscribe" };
        self.signed_post(endpoint, json!({ "channelIds": [channel_id] })).await.map(drop)
    }

    /// A new playlist (private unless `public`), optionally with songs; returns its id.
    pub async fn create_playlist(&self, title: &str, video_ids: &[String], public: bool) -> Result<String> {
        let mut body = json!({ "title": title, "privacyStatus": if public { "PUBLIC" } else { "PRIVATE" } });
        if !video_ids.is_empty() {
            body["videoIds"] = json!(video_ids);
        }
        let response = self.signed_post("playlist/create", body).await?;
        parsers::created_playlist_id(&response).ok_or(YouTubeError::Unexpected("created playlist id"))
    }

    pub async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        self.signed_post("playlist/delete", json!({ "playlistId": bare(playlist_id) })).await.map(drop)
    }

    pub async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()> {
        let action = json!({ "action": "ACTION_SET_PLAYLIST_NAME", "playlistName": name });
        self.edit_playlist(playlist_id, vec![action]).await.map(drop)
    }

    /// Append songs; returns the set-video ids of the new rows.
    pub async fn add_to_playlist(&self, playlist_id: &str, video_ids: &[&str]) -> Result<Vec<String>> {
        let actions = video_ids.iter().map(|id| json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": id })).collect();
        self.edit_playlist(playlist_id, actions).await
    }

    /// Append every song of another playlist.
    pub async fn add_playlist_to_playlist(&self, playlist_id: &str, source_playlist_id: &str) -> Result<Vec<String>> {
        let action = json!({ "action": "ACTION_ADD_PLAYLIST", "addedFullListId": bare(source_playlist_id) });
        self.edit_playlist(playlist_id, vec![action]).await
    }

    /// Remove rows, each named by its (video id, set-video id).
    pub async fn remove_from_playlist(&self, playlist_id: &str, rows: &[(&str, &str)]) -> Result<()> {
        let actions = rows
            .iter()
            .map(|(video_id, set_video_id)| {
                json!({ "action": "ACTION_REMOVE_VIDEO", "removedVideoId": video_id, "setVideoId": set_video_id })
            })
            .collect();
        self.edit_playlist(playlist_id, actions).await.map(drop)
    }

    /// Move a row before `successor` (another row's set-video id), or to the end when `None`.
    pub async fn move_in_playlist(&self, playlist_id: &str, set_video_id: &str, successor: Option<&str>) -> Result<()> {
        let mut action = json!({ "action": "ACTION_MOVE_VIDEO_BEFORE", "setVideoId": set_video_id });
        if let Some(successor) = successor {
            action["movedSetVideoIdSuccessor"] = json!(successor);
        }
        self.edit_playlist(playlist_id, vec![action]).await.map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_name_the_playlist_without_its_browse_prefix() {
        let body = edit_body("VLPLabc", vec![json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": "v" })]);
        assert_eq!(body, json!({ "playlistId": "PLabc", "actions": [{ "action": "ACTION_ADD_VIDEO", "addedVideoId": "v" }] }));
        assert_eq!(Rating::Like.endpoint(), "like/like");
        assert_eq!(Rating::Dislike.endpoint(), "like/dislike");
        assert_eq!(Rating::None.endpoint(), "like/removelike");
    }
}
