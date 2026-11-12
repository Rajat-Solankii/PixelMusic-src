# Fix for `0:00` Song Durations on Quick Picks and Home Shelves

## Problem
YouTube Music home feed items (Quick Picks, shelf tracks) often display `0:00` as their duration because the InnerTube API responses for these items don't always include duration information in the standard location (`fixedColumns` or subtitle runs).

## Root Cause Analysis

1. **Feed Structure**: YouTube Music home shelves use `musicTwoRowItemRenderer` (tiles) for Quick Picks, not `musicResponsiveListItemRenderer` (list rows).

2. **Parsing Logic**: The `card_item` function (parsers.rs:377) attempts to extract duration from subtitle runs using `last_time(rest)`, but Quick Picks often don't include duration in their subtitle.

3. **Display Issue**: The QML TrackRow component unconditionally displays duration as `0:00` when `durationMs` is 0.

## Solution

The fix implements a multi-layered approach:

### 1. QML Layer: Hide Unknown Durations
**File**: `src/materialfy/qml/app/TrackRow.qml`

```qml
Label {
    visible: root.durationMs > 0
    Layout.preferredWidth: root.durationMs > 0 ? 44 : 0
    text: root._time(root.durationMs)
    type: "bodyMedium"
    color: Theme.fgSurfaceVariant
    horizontalAlignment: Text.AlignRight
}
```

**Impact**: Tracks with unknown duration no longer display "0:00" - the label is hidden when `durationMs <= 0`.

### 2. Duration Resolution Service
**File**: `crates/sf-services/src/youtube/duration_cache.rs`

A new module that provides:
- **DurationCache**: In-memory LRU cache with 30-minute TTL
- **Lazy Resolution**: Fetches duration from YouTube Music's player endpoint only when needed
- **Single-flight Requests**: Avoids duplicate network calls for the same video ID

Key features:
- Maximum 1000 cached entries
- Automatic eviction of expired entries
- Cache cleared on account changes

### 3. YouTube Service Integration
**File**: `crates/sf-services/src/youtube/mod.rs`

Added methods to `YouTubeService`:
- `resolve_track_duration(video_id)`: Returns cached duration or fetches from YouTube Music
- `cached_duration(video_id)`: Returns cached duration if available

The resolution uses the `ANDROID_VR` client (anonymous, no PoToken required) which returns video metadata including `lengthSeconds` in `videoDetails`.

## Usage

### From Rust Backend
```rust
// Resolve duration for a track
if let Some(duration_ms) = youtube_service.resolve_track_duration(video_id).await {
    track.duration_ms = duration_ms;
}
```

### From QML (Future Enhancement)
The infrastructure is in place for QML to request duration resolution on-demand when a track with `durationMs = 0` becomes visible. This would require:
1. Adding a QML-callable method to HomeService or CatalogService
2. Updating track models dynamically when durations are resolved
3. Triggering UI refresh for affected tracks

## Performance Considerations

1. **Minimal Impact**: Duration resolution only happens for tracks with `duration_ms = 0`
2. **Cached Results**: 30-minute TTL prevents repeated lookups
3. **Lightweight Request**: Uses anonymous `ANDROID_VR` client (no authentication, no PoToken)
4. **Async Resolution**: Doesn't block UI thread

## Testing

The implementation includes:
- Compile-time checks (cargo check)
- No runtime errors for missing durations
- Graceful fallback (hide the label) when resolution fails

## Future Enhancements

1. **Proactive Resolution**: When home feed loads, proactively resolve durations for all tracks with `duration_ms = 0` in the background
2. **QML Integration**: Add QML-callable method to request duration resolution
3. **Batch Resolution**: Resolve multiple track durations in one API call
4. **Persistent Cache**: Store resolved durations in local database for offline access

## Files Modified

1. `src/materialfy/qml/app/TrackRow.qml` - Hide duration label when durationMs <= 0
2. `crates/sf-services/src/youtube/duration_cache.rs` - New module (created)
3. `crates/sf-services/src/youtube/mod.rs` - Add duration cache and resolution methods
4. `crates/sf-services/src/youtube/parsers.rs` - Cleaned up (removed unused function)

## Alternative Approaches Considered

1. **Parse from Subtitle**: Already implemented in `last_time()`, but YouTube Music doesn't always include duration there
2. **Fetch from Album Page**: Would require additional API calls for each track's album
3. **Poll Queue Endpoint**: Queue endpoint returns durations, but requires the track to be in player context
4. **Hardcode Edits**: Not maintainable and doesn't scale

The chosen approach (lazy resolution with caching) provides the best balance of accuracy, performance, and maintainability.
