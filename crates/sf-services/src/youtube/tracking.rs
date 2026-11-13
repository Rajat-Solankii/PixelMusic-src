//! Reporting plays to the account's YouTube Music history, as the web player does (InnerTune / OuterTune
//! `registerPlayback`): the signed-in `player` response carries `playbackTracking.videostatsPlaybackUrl`, a
//! `s.youtube.com/api/stats/playback?…` address; a `GET` of it on music.youtube.com with `ver=2`, `c=WEB_REMIX`
//! and a fresh client playback nonce (`cpn`) puts the song in "History". Reading the history back is
//! `library.rs::history` (`FEmusic_history`).
use serde_json::{Value, json};

use super::YouTubeService;
use super::client::{Login, WEB_REMIX};
use super::error::{Result, YouTubeError};

/// The alphabet of a client playback nonce (16 characters, as the web player makes them).
const CPN_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";

/// Percent-encoding of a query value (everything but the unreserved characters).
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A new client playback nonce.
pub fn new_cpn() -> String {
    (0..16).map(|_| CPN_ALPHABET[fastrand::usize(..CPN_ALPHABET.len())] as char).collect()
}

/// `playbackTracking.videostatsPlaybackUrl.baseUrl` of a `player` response.
pub fn playback_url(player: &Value) -> Option<&str> {
    player
        .pointer("/playbackTracking/videostatsPlaybackUrl/baseUrl")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
}

/// The address to report the play at: the tracking URL's path and query on `site_base` (music.youtube.com/,
/// a test server) plus `ver=2&c=WEB_REMIX&cpn=…` and, when the song plays from a playlist, `list` + `referrer`.
pub fn report_url(tracking: &str, site_base: &str, cpn: &str, playlist_id: Option<&str>) -> Option<String> {
    let rest = tracking.split_once("://").map_or(tracking, |(_, rest)| rest);
    let path = &rest[rest.find('/')?..];
    let mut url = format!("{}/{}", site_base.trim_end_matches('/'), path.trim_start_matches('/'));
    url.push(if url.contains('?') { '&' } else { '?' });
    url.push_str("ver=2&c=WEB_REMIX&cpn=");
    url.push_str(cpn);
    if let Some(list) = playlist_id.filter(|id| !id.is_empty()) {
        let referrer = format!("https://music.youtube.com/playlist?list={list}");
        url.push_str("&list=");
        url.push_str(&encode(list));
        url.push_str("&referrer=");
        url.push_str(&encode(&referrer));
    }
    Some(url)
}

impl YouTubeService {
    /// Put the song in the account's YouTube Music history (`playlist_id`: the playlist it played from, if any).
    /// Needs an account; a `player` response without a tracking address is `Unexpected`.
    pub async fn report_play(&self, video_id: &str, playlist_id: Option<&str>) -> Result<()> {
        if !self.signed_in() {
            return Err(YouTubeError::NotSignedIn);
        }
        let mut body = json!({ "videoId": video_id });
        if let Some(list) = playlist_id.filter(|id| !id.is_empty()) {
            body["playlistId"] = json!(list);
        }
        // the web player's own request has the script's signatureTimestamp; use it when a stream already loaded it
        if let Some(sts) = self.streams.loaded_sts() {
            body["playbackContext"] = json!({ "contentPlaybackContext": { "signatureTimestamp": sts } });
        }
        let player = self.client.post(&WEB_REMIX, "player", body, Login::Required).await?;
        let tracking = playback_url(&player).ok_or(YouTubeError::Unexpected("the player response has no playback tracking"))?;
        let url = report_url(tracking, self.client.site_base(), &new_cpn(), playlist_id)
            .ok_or(YouTubeError::Unexpected("the playback tracking address has no path"))?;
        self.client.get_signed(&url).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tracking_address_moves_to_music_youtube_com_with_the_web_remix_parameters() {
        let player = json!({ "playbackTracking": { "videostatsPlaybackUrl": {
            "baseUrl": "https://s.youtube.com/api/stats/playback?cl=1&docid=abc&ei=XyZ"
        } } });
        let tracking = playback_url(&player).expect("tracking url");
        let url = report_url(tracking, "https://music.youtube.com/", "CPN0123456789abc", Some("PL a&b")).unwrap();
        assert_eq!(
            url,
            "https://music.youtube.com/api/stats/playback?cl=1&docid=abc&ei=XyZ&ver=2&c=WEB_REMIX&cpn=CPN0123456789abc\
             &list=PL%20a%26b&referrer=https%3A%2F%2Fmusic.youtube.com%2Fplaylist%3Flist%3DPL%20a%26b"
        );
        assert_eq!(report_url("https://s.youtube.com/api/stats/playback", "http://127.0.0.1:9", "c", None).unwrap(),
                   "http://127.0.0.1:9/api/stats/playback?ver=2&c=WEB_REMIX&cpn=c");
        assert!(playback_url(&json!({ "playbackTracking": {} })).is_none());
    }

    #[test]
    fn a_nonce_is_sixteen_url_safe_characters() {
        let cpn = new_cpn();
        assert_eq!(cpn.len(), 16);
        assert!(cpn.bytes().all(|b| CPN_ALPHABET.contains(&b)));
        assert_ne!(cpn, new_cpn());
    }
}
