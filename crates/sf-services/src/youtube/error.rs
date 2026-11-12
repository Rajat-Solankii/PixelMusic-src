//! Errors of the YouTube Music client. Display is interface text through mf_i18n.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum YouTubeError {
    /// The call needs a signed-in YouTube Music account.
    NotSignedIn,
    /// HTTP 401: the saved cookies no longer work.
    SessionExpired,
    /// HTTP 429 after every retry.
    RateLimited,
    /// Any other HTTP failure (status code).
    Http(u16),
    /// The request never got an answer (text for the log, not for the interface).
    Network(String),
    /// The shared HTTP client is blocked (invalid proxy settings); text from sf-api.
    Unavailable(String),
    /// The playlist is private, deleted or has no header we recognise.
    PlaylistUnavailable,
    /// The response had a shape the parser does not know (what was missing).
    Unexpected(&'static str),
    /// Playlist import failed; the text is already localized.
    Import(String),
    /// No playable stream for the video: YouTube's reason (already in the request's language), or empty.
    Unplayable(String),
}

impl std::fmt::Display for YouTubeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::NotSignedIn => mf_i18n::t("Sign in to YouTube Music"),
            Self::SessionExpired => mf_i18n::t("Your YouTube Music session has expired — sign in again."),
            Self::RateLimited => mf_i18n::t("YouTube Music is limiting requests — try again in a moment."),
            Self::Http(status) => mf_i18n::tf("YouTube Music answered HTTP {0}", &[status]),
            Self::Network(_) => mf_i18n::t("Couldn't reach YouTube Music — check your connection."),
            Self::Unavailable(reason) | Self::Import(reason) => reason.clone(),
            Self::PlaylistUnavailable => mf_i18n::t("This playlist is private or no longer exists."),
            Self::Unplayable(reason) if !reason.is_empty() => reason.clone(),
            Self::Unplayable(_) => mf_i18n::t("This song can't be played."),
            Self::Unexpected(_) => mf_i18n::t("YouTube Music sent a response this app doesn't understand."),
        };
        f.write_str(&text)
    }
}

impl std::error::Error for YouTubeError {}

pub type Result<T> = std::result::Result<T, YouTubeError>;
