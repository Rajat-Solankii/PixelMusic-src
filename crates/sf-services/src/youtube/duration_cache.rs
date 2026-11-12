//! Song lengths the home feed leaves out. Quick picks and "Listen again" rows come without a length, so the
//! service asks for all of them in one WEB_REMIX `music/get_queue` request (as ytmusicapi's `get_queue` does) and
//! remembers them per video id.
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::json;

use super::client::{Login, WEB_REMIX};
use super::models::{ShelvesPage, YtItem};
use super::{YouTubeService, parsers};
use crate::lock;

/// A song's length does not change; the limit only keeps the map from growing for ever.
const DURATION_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const MAX_CACHE_SIZE: usize = 2000;
/// Video ids per `get_queue` request.
const BATCH: usize = 50;

/// Lengths in milliseconds by video id.
#[derive(Default)]
pub struct DurationCache {
    cache: Mutex<HashMap<String, (Instant, i64)>>,
}

impl DurationCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, video_id: &str) -> Option<i64> {
        lock(&self.cache).get(video_id).filter(|(at, _)| at.elapsed() < DURATION_TTL).map(|(_, ms)| *ms)
    }

    pub fn set(&self, video_id: String, duration_ms: i64) {
        let mut cache = lock(&self.cache);
        if cache.len() >= MAX_CACHE_SIZE {
            cache.retain(|_, (at, _)| at.elapsed() < DURATION_TTL);
            if cache.len() >= MAX_CACHE_SIZE {
                cache.clear();
            }
        }
        cache.insert(video_id, (Instant::now(), duration_ms));
    }

    pub fn clear(&self) {
        lock(&self.cache).clear();
    }
}

fn video_id(uri: &str) -> Option<&str> {
    uri.strip_prefix("youtube:track:").filter(|id| !id.is_empty())
}

impl YouTubeService {
    /// Gives every song of `page` that came without a length its length: from the cache, else from one
    /// `music/get_queue` request per 50 songs. A failed request leaves those songs at 0 (the UI shows nothing).
    pub(crate) async fn fill_durations(&self, page: &mut ShelvesPage) {
        let mut seen = HashSet::new();
        let missing: Vec<String> = (page.shelves.iter().flat_map(|shelf| &shelf.items))
            .filter_map(|item| match item {
                YtItem::Track(track) if track.duration_ms <= 0 => video_id(&track.uri),
                _ => None,
            })
            .filter(|id| self.duration_cache.get(id).is_none() && seen.insert(*id))
            .map(str::to_owned)
            .collect();
        for batch in missing.chunks(BATCH) {
            match self.client.post(&WEB_REMIX, "music/get_queue", json!({ "videoIds": batch }), Login::IfSignedIn).await {
                Ok(response) => {
                    for (id, seconds) in parsers::queue_lengths(&response) {
                        self.duration_cache.set(id, seconds * 1000);
                    }
                }
                Err(error) => log::debug!("youtube: song lengths of the home feed: {error}"),
            }
        }
        for item in page.shelves.iter_mut().flat_map(|shelf| shelf.items.iter_mut()) {
            if let YtItem::Track(track) = item
                && track.duration_ms <= 0
                && let Some(ms) = video_id(&track.uri).and_then(|id| self.duration_cache.get(id))
            {
                track.duration_ms = ms;
            }
        }
    }
}
