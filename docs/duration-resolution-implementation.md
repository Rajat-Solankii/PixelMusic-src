# Lazy Duration Resolution Implementation

## Overview
This document describes the implementation of lazy duration resolution for YouTube Music tracks that lack duration information in their initial metadata.

## Problem
YouTube Music home feed items (Quick Picks, shelf tracks) often lack duration information when returned from the API. This causes tracks to display with "0:00" duration, which is hidden in the UI but provides a poor user experience.

## Solution
We implemented a lazy resolution system that:
1. Detects when tracks with unknown duration become visible
2. Triggers background resolution only for visible tracks
3. Caches results to avoid redundant API calls
4. Updates the UI when durations are resolved

## Implementation Details

### 1. Duration Cache Infrastructure (`crates/sf-services/src/youtube/duration_cache.rs`)
- **DurationCache**: Thread-safe cache with TTL (30 minutes)
- **get/set operations**: Fast lookup and storage
- **Automatic eviction**: Removes old entries when full (max 1000 entries)

### 2. YouTube Service Method (`crates/sf-services/src/youtube/mod.rs`)
```rust
pub async fn resolve_track_duration(&self, video_id: &str) -> Option<i64>
```
- Checks cache first
- Fetches video info via InnerTube API if not cached
- Extracts duration from video details
- Caches the result
- Returns duration in milliseconds

### 3. Catalog Bridge Method (`crates/materialfy/src/bridges/catalog.rs`)
```rust
#[qinvokable]
fn resolve_duration(self: Pin<&mut Self>, track_id: &QString, current_ms: i64)
```
- QML-callable method
- Only resolves if `current_ms <= 0` (missing duration)
- Extracts video ID from `youtube:track:` URI
- Checks for cached duration first
- Spawns async resolution task
- Emits `durationResolved` signal when complete

### 4. TrackRow.qml Trigger
```qml
property bool _durationResolved: false
onVisibleChanged: {
    if (visible && durationMs <= 0 && trackId !== "" && !_durationResolved) {
        _durationResolved = true
        Catalog.resolveDuration(trackId, durationMs)
    }
}
Connections {
    target: Catalog
    function onDurationResolved(resolvedTrackId, resolvedMs) {
        if (resolvedTrackId === trackId && durationMs <= 0 && resolvedMs > 0) {
            durationMs = resolvedMs
        }
    }
}
```

## Key Features

### Per-Track Resolution
- Each track row independently triggers resolution when it becomes visible
- Prevents resolving all tracks at once (performance benefit)
- Only visible tracks get resolved

### Deduplication
- `_durationResolved` flag prevents multiple resolution calls for the same track
- Cache prevents redundant API calls across the app

### Async & Non-Blocking
- Resolution happens in background thread
- UI remains responsive during resolution
- Results delivered via Qt signal/slot mechanism

### Smart Caching
- Results cached for 30 minutes
- Shared across all track rows
- Automatic eviction prevents memory bloat

## Testing

### Unit Test File: `tests/qml/test_duration_resolution.qml`
Tests cover:
- No resolution when duration already present
- Resolution triggered on visibility
- No duplicate resolution calls
- Duration updates from resolved signal
- Multiple tracks resolving independently

### Manual Testing
1. Open YouTube Music home page
2. Navigate to Quick Picks or shelf with tracks
3. Observe tracks initially show no duration
4. After brief moment, durations appear one by one
5. Subsequent views show cached durations immediately

## Performance Considerations

### Benefits
- **Lazy**: Only resolves visible tracks
- **Cached**: No redundant API calls
- **Async**: Non-blocking UI
- **Efficient**: Uses lightweight player endpoint

### Network Impact
- Each resolution = 1 API call per track
- Bounded by visibility (not all tracks at once)
- Cached results reduce overall calls

### Memory Impact
- Max 1000 cached durations (~16KB)
- Automatic eviction of old entries
- TTL ensures stale data is refreshed

## Future Enhancements

### Potential Improvements
1. **Batch Resolution**: Resolve multiple visible tracks at once
2. **Prefetch**: Predict which tracks will be visible and prefetch
3. **Smart Loading**: Prioritize resolution for likely-to-play tracks
4. **Metrics**: Track resolution success rate and timing

### Alternative Approaches Considered
1. **Batch Resolution** (Option B from task)
   - Would require changes to shelf views
   - More complex coordination
   - Harder to test
   - Current per-track approach is simpler and sufficient

## Files Modified
- `crates/materialfy/src/bridges/catalog.rs`: Added `resolve_duration` method
- `src/materialfy/qml/app/TrackRow.qml`: Added visibility trigger
- `tests/qml/test_duration_resolution.qml`: Added unit tests
- `docs/duration-resolution-implementation.md`: This documentation

## Success Criteria
✅ Duration resolution triggered when tracks become visible
✅ No redundant resolution calls
✅ Results cached for performance
✅ UI updated when resolution completes
✅ Async and non-blocking
✅ Unit tests demonstrate correctness

## Production Task 3: Complete
The implementation successfully adds QML trigger for lazy duration resolution, meeting all requirements:
- Minimum viable implementation
- Visibility-based triggering
- Proper caching
- Async resolution
- UI updates
