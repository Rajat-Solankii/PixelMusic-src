//! Duration resolution for YouTube Music tracks.
//! 
//! YouTube Music home feed items (Quick Picks, shelf tracks) often lack duration information.
//! This module provides lazy resolution and caching for missing durations.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::json;

use super::client::{InnerTube, Login};
use crate::lock;

/// How long cached durations are kept in memory.
const DURATION_TTL: Duration = Duration::from_secs(30 * 60); // 30 minutes
/// Maximum number of cached durations to keep.
const MAX_CACHE_SIZE: usize = 1000;

/// Duration cache for YouTube Music tracks.
pub struct DurationCache {
    cache: Mutex<HashMap<String, (Instant, i64)>>,
}

impl DurationCache {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Get a cached duration if it exists and hasn't expired.
    pub fn get(&self, video_id: &str) -> Option<i64> {
        let cache = lock(&self.cache);
        cache.get(video_id).filter(|(at, _)| at.elapsed() < DURATION_TTL).map(|(_, ms)| *ms)
    }

    /// Store a resolved duration in the cache.
    pub fn set(&self, video_id: String, duration_ms: i64) {
        let mut cache = lock(&self.cache);
        
        // Evict old entries if cache is full
        if cache.len() >= MAX_CACHE_SIZE {
            cache.retain(|_, (at, _)| at.elapsed() < DURATION_TTL);
            if cache.len() >= MAX_CACHE_SIZE {
                // Remove oldest 25% of entries
                let mut entries: Vec<_> = cache.iter().map(|(id, (at, _))| (id.clone(), *at)).collect();
                entries.sort_by_key(|(_, at)| *at);
                let to_remove: Vec<String> = entries.iter().take(cache.len() / 4).map(|(id, _)| id.clone()).collect();
                // Drop the immutable borrow before modifying
                drop(entries);
                for id in to_remove {
                    cache.remove(&id);
                }
            }
        }
        
        cache.insert(video_id, (Instant::now(), duration_ms));
    }

    /// Check if a duration needs to be resolved (not in cache or expired).
    pub fn needs_resolution(&self, video_id: &str) -> bool {
        self.get(video_id).is_none()
    }

    /// Clear the entire cache.
    pub fn clear(&self) {
        lock(&self.cache).clear();
    }
}

impl Default for DurationCache {
    fn default() -> Self {
        Self::new()
    }
}

impl InnerTube {
    /// Fetch basic video info to extract duration.
    /// This is a lightweight player request that only needs the video metadata.
    pub async fn get_video_info(&self, video_id: &str) -> super::Result<serde_json::Value> {
        // Use ANDROID_VR client for unauthenticated requests (no PoToken needed)
        self.post(&super::client::ANDROID_VR, "player", json!({ "videoId": video_id }), Login::Never).await
    }
    
    /// Extract duration in milliseconds from video info response.
    pub fn extract_duration_ms(response: &serde_json::Value) -> Option<i64> {
        // Extract duration from videoDetails.lengthSeconds
        let duration_text = response
            .get("videoDetails")?
            .get("lengthSeconds")?
            .as_str()?;
        
        let seconds: i64 = duration_text.parse().ok()?;
        Some(seconds * 1000)
    }
}

