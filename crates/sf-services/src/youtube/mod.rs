//! YouTube Music (port of the soundwave1 InnerTube engine): HTTP client, response parsers and the
//! `YouTubeService` facade. Items come back as the app's own md3-model types with `youtube:` URIs.
//!
//! The service is split by area: `library.rs` (the user's library), `playlist.rs` (pages and the guarded
//! full read), `search.rs`, `browse.rs` (home, explore, charts, moods), `catalog.rs` (album, artist, queue,
//! lyrics, account), `mutations.rs` (likes, subscriptions, playlist edits) and `import.rs`.

pub mod client;
pub mod duration_cache;
pub mod error;
pub mod import;
pub mod models;
pub mod parsers;

mod browse;
mod cipher;
mod jsworker;
mod potoken;
mod catalog;
mod library;
mod mutations;
mod playlist;
mod search;
mod stream;

#[cfg(test)]
mod client_tests;
#[cfg(test)]
mod parser_tests;

use std::sync::Mutex;
use std::time::Instant;

use serde_json::{Value, json};
use sf_auth::YouTubeAccount;

pub use client::{InnerTube, Login};
pub use error::{Result, YouTubeError};
pub use catalog::radio_playlist_id;
pub use import::{ImportResult, ImportSource};
pub use library::{HISTORY, LIBRARY_ARTISTS, LIKED_ALBUMS, LIKED_MUSIC, LIKED_PLAYLISTS, liked_music};
pub use models::*;
pub use mutations::Rating;
pub use stream::{COOLDOWN as STREAM_COOLDOWN, Ciphered, Refusal, choose as choose_stream, parse_player, valid_until};

/// The YouTube Music side of the app. Anonymous until `set_account`; browse and search work either way,
/// the library and every edit need an account (`YouTubeError::NotSignedIn`).
pub struct YouTubeService {
    client: std::sync::Arc<InnerTube>,
    /// stream resolution: cache, single-flight, tier cooldowns (`stream.rs`)
    streams: std::sync::Arc<stream::Streams>,
    /// the default home feed and when it was read (kept for `browse::HOME_TTL`)
    home_cache: Mutex<Option<(Instant, ShelvesPage)>>,
    /// one home request at a time, so concurrent callers share the result
    home_gate: tokio::sync::Mutex<()>,
    /// cache for resolved track durations
    duration_cache: duration_cache::DurationCache,
}

impl YouTubeService {
    pub fn new(http: sf_api::Http) -> Self {
        Self::with_client(InnerTube::new(http))
    }

    /// A service over a prepared client (tests point it at a local server).
    pub fn with_client(client: InnerTube) -> Self {
        Self { 
            client: std::sync::Arc::new(client), 
            streams: Default::default(), 
            home_cache: Mutex::new(None), 
            home_gate: tokio::sync::Mutex::new(()),
            duration_cache: duration_cache::DurationCache::new(),
        }
    }

    pub fn client(&self) -> &InnerTube {
        &self.client
    }

    /// Sign in (`Some`) or out (`None`). The cached home feed belonged to the previous identity.
    pub fn set_account(&self, account: Option<YouTubeAccount>) {
        self.client.set_account(account);
        self.streams.account_changed();
        self.invalidate_home();
        self.duration_cache.clear();
    }

    pub fn signed_in(&self) -> bool {
        self.client.signed_in()
    }

    /// `POST browse` for a page.
    async fn browse(&self, browse_id: &str, params: Option<&str>, login: Login) -> Result<Value> {
        let mut body = json!({ "browseId": browse_id });
        if let Some(params) = params {
            body["params"] = json!(params);
        }
        self.client.post(&client::WEB_REMIX, "browse", body, login).await
    }

    /// `POST browse` for the next page of a list: the token goes in the body, as the reference client sends it.
    async fn browse_continuation(&self, continuation: &str, login: Login) -> Result<Value> {
        self.client.post(&client::WEB_REMIX, "browse", json!({ "continuation": continuation }), login).await
    }
    
    /// Resolve duration for a track that might be missing it (e.g., from Quick Picks).
    /// Returns cached duration if available, or fetches it from YouTube Music.
    pub async fn resolve_track_duration(&self, video_id: &str) -> Option<i64> {
        // Check cache first
        if let Some(cached) = self.duration_cache.get(video_id) {
            return Some(cached);
        }
        
        // Fetch video info to get duration
        let response = self.client.get_video_info(video_id).await.ok()?;
        let duration_ms = InnerTube::extract_duration_ms(&response)?;
        
        // Cache the result
        self.duration_cache.set(video_id.to_owned(), duration_ms);
        
        Some(duration_ms)
    }
    
    /// Get cached duration if available.
    pub fn cached_duration(&self, video_id: &str) -> Option<i64> {
        self.duration_cache.get(video_id)
    }
}

/// Stops a paged read that would never end: a token seen before, too many pages, or a run of empty pages
/// (`Utils.kt` in the reference project).
pub(crate) struct PageGuard {
    seen: std::collections::HashSet<String>,
    pages: usize,
    empty_streak: usize,
}

impl PageGuard {
    pub(crate) const MAX_PAGES: usize = 50;
    const MAX_EMPTY: usize = 2;

    pub(crate) fn new() -> Self {
        Self { seen: Default::default(), pages: 0, empty_streak: 0 }
    }

    /// Whether to fetch the page behind `token`.
    pub(crate) fn admit(&mut self, token: &str) -> bool {
        if token.is_empty() || self.pages >= Self::MAX_PAGES || !self.seen.insert(token.to_owned()) {
            return false;
        }
        self.pages += 1;
        true
    }

    /// Record how many entries the fetched page had; false once enough empty pages came in a row.
    pub(crate) fn record(&mut self, entries: usize) -> bool {
        self.empty_streak = if entries == 0 { self.empty_streak + 1 } else { 0 };
        self.empty_streak < Self::MAX_EMPTY
    }
}

#[cfg(test)]
mod guard_tests {
    use super::PageGuard;

    #[test]
    fn the_guard_refuses_repeated_tokens_empty_runs_and_endless_lists() {
        let mut guard = PageGuard::new();
        assert!(guard.admit("a"));
        assert!(!guard.admit("a"));
        assert!(!guard.admit(""));
        assert!(guard.record(0));
        assert!(guard.record(3));
        assert!(guard.record(0));
        assert!(!guard.record(0));

        let mut guard = PageGuard::new();
        for page in 0..PageGuard::MAX_PAGES {
            assert!(guard.admit(&page.to_string()));
        }
        assert!(!guard.admit("one more"));
    }
}
