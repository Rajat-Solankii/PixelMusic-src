//! Строковые id Spotify (base62). Сериализуются строкой.
use serde::{Deserialize, Serialize};

/// id аккаунта Spotify (`User.id`), не числовой.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountId(pub String);

impl AccountId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const YOUTUBE_PLAYLIST: &str = "youtube:playlist:";
/// A playlist kept on this computer (`local:playlist:<id>`), optionally mirrored to YouTube Music.
pub const LOCAL_PLAYLIST: &str = "local:playlist:";

/// id плейлиста (base62, без префикса `spotify:playlist:`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlaylistId(pub String);

impl PlaylistId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `spotify:playlist:<id>`; a YouTube playlist id is already its URI (`youtube:playlist:<id>`).
    pub fn uri(&self) -> String {
        if self.is_youtube() || self.is_local() {
            return self.0.clone();
        }
        format!("spotify:playlist:{}", self.0)
    }

    /// A YouTube Music playlist: the id is kept as the whole `youtube:playlist:<id>` URI.
    pub fn is_youtube(&self) -> bool {
        self.0.starts_with(YOUTUBE_PLAYLIST)
    }

    /// A local playlist: the id is kept as the whole `local:playlist:<id>` URI.
    pub fn is_local(&self) -> bool {
        self.0.starts_with(LOCAL_PLAYLIST)
    }

    /// The local playlist id without the URI prefix.
    pub fn local_id(&self) -> Option<&str> {
        self.0.strip_prefix(LOCAL_PLAYLIST).filter(|id| !id.is_empty())
    }

    /// The YouTube playlist id (`LM`, `PL…`, `OLAK5uy_…`) without the URI prefix.
    pub fn youtube_id(&self) -> Option<&str> {
        self.0.strip_prefix(YOUTUBE_PLAYLIST).filter(|id| !id.is_empty())
    }

    /// Из id, URI `spotify:playlist:<id>` или ссылки `https://open.spotify.com/playlist/<id>?…`.
    /// A `youtube:playlist:<id>` URI stays whole, so services can tell the two apart.
    pub fn from_uri_or_id(text: &str) -> Self {
        let text = text.trim();
        if text.starts_with(YOUTUBE_PLAYLIST) || text.starts_with(LOCAL_PLAYLIST) {
            return Self(text.to_owned());
        }
        if let Some((_, rest)) = text.split_once("/playlist/") {
            let id = rest.split(['?', '#', '/']).next().unwrap_or(rest);
            return Self(id.to_owned());
        }
        Self(text.rsplit(':').next().unwrap_or(text).to_owned())
    }
}

impl std::borrow::Borrow<str> for PlaylistId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PlaylistId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::PlaylistId;

    #[test]
    fn youtube_playlist_ids_keep_their_provider() {
        let liked = PlaylistId::from_uri_or_id("youtube:playlist:LM");
        assert!(liked.is_youtube());
        assert_eq!((liked.as_str(), liked.youtube_id(), liked.uri().as_str()), ("youtube:playlist:LM", Some("LM"), "youtube:playlist:LM"));
        let spotify = PlaylistId::from_uri_or_id("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M");
        assert!(!spotify.is_youtube());
        assert_eq!((spotify.as_str(), spotify.youtube_id()), ("37i9dQZF1DXcBWIGoYBM5M", None));
        assert_eq!(spotify.uri(), "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M");
        assert_eq!(PlaylistId::from_uri_or_id("https://open.spotify.com/playlist/abc?si=1").as_str(), "abc");
    }

    #[test]
    fn local_playlist_ids_stay_whole() {
        let local = PlaylistId::from_uri_or_id("local:playlist:00ff");
        assert!(local.is_local() && !local.is_youtube());
        assert_eq!((local.as_str(), local.local_id(), local.uri().as_str()), ("local:playlist:00ff", Some("00ff"), "local:playlist:00ff"));
        assert_eq!(PlaylistId::from_uri_or_id("local:playlist:").local_id(), None);
    }
}
