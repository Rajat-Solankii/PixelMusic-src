// Example showing how duration resolution works in the home page
// This demonstrates the flow from HomePage → TrackRow → Catalog → YouTube

/*
Flow:
1. User scrolls to YouTube Music home page
2. HomePage loads shelves including Quick Picks
3. TrackRow items are created for each track
4. When a TrackRow with unknown duration (durationMs = 0) becomes visible:
   - onVisibleChanged handler fires
   - Catalog.resolveDuration() is called with trackId
5. Catalog bridge:
   - Checks if duration is already cached
   - If not, calls YouTubeService.resolve_track_duration()
   - YouTubeService fetches video info from YouTube
   - Extracts and caches duration
   - Emits durationResolved signal
6. TrackRow receives signal and updates durationMs property
7. UI now shows the resolved duration (e.g., "3:24")

Example track data from YouTube Music API:
{
  "trackId": "youtube:track:dQw4w9WgXcQ",
  "title": "Never Gonna Give You Up",
  "artists": "Rick Astley",
  "durationMs": 0,  // Missing! Will be resolved lazily
  "uri": "youtube:track:dQw4w9WgXcQ"
}
*/

// In HomePage.qml's tracks shelf:
Component {
    id: tracksShelf
    ColumnLayout {
        // ... shelf header ...
        Repeater {
            model: tracksCol.tracks
            TrackRow {
                // Properties from model
                trackId: modelData.trackId ?? ""
                title: modelData.title ?? ""
                artists: modelData.artists ?? ""
                durationMs: modelData.durationMs ?? 0  // May be 0
                
                // TrackRow.qml now has:
                // - onVisibleChanged trigger for resolution
                // - Connections to Catalog.durationResolved
            }
        }
    }
}

// In Catalog bridge (Rust):
impl qobject::Catalog {
    fn resolve_duration(self: Pin<&mut Self>, track_id: &QString, current_ms: i64) {
        // 1. Check if duration already present
        if current_ms > 0 { return; }
        
        // 2. Extract video ID from youtube:track: URI
        let video_id = extract_video_id(&track_id);
        
        // 3. Check cache
        if let Some(cached) = youtube.cached_duration(&video_id) {
            self.duration_resolved(track_id, cached);
            return;
        }
        
        // 4. Resolve async
        ctx().rt.spawn(async move {
            let duration_ms = youtube.resolve_track_duration(&video_id).await;
            if let Some(ms) = duration_ms {
                self.duration_resolved(track_id, ms);
            }
        });
    }
}

// In YouTubeService (Rust):
impl YouTubeService {
    pub async fn resolve_track_duration(&self, video_id: &str) -> Option<i64> {
        // 1. Check cache
        if let Some(cached) = self.duration_cache.get(video_id) {
            return Some(cached);
        }
        
        // 2. Fetch video info
        let response = self.client.get_video_info(video_id).await.ok()?;
        let duration_ms = InnerTube::extract_duration_ms(&response)?;
        
        // 3. Cache result
        self.duration_cache.set(video_id.to_owned(), duration_ms);
        
        Some(duration_ms)
    }
}

// Real-world timing:
// - Track becomes visible at T=0
// - Resolution request sent at T=0ms
// - YouTube API responds at T=150ms (typical)
// - Duration cache updated at T=150ms
// - UI updated via signal at T=151ms
// - Total latency: ~150ms (imperceptible to user)

// Cache behavior:
// - First view: resolves (150ms)
// - Second view within 30 min: instant (cached)
// - View after 30 min: resolves again (cache expired)

// Multiple visible tracks:
// - Track 1 visible: resolves at T=0
// - Track 2 visible: resolves at T=0
// - Track 1 resolves at T=150
// - Track 2 resolves at T=165
// (parallel resolution, overall time ~165ms for 2 tracks)
