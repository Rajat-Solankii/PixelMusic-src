# Android-Style Home Sections Implementation Summary

## Overview
Implemented Android-style Home sections for the Materialfy desktop music player, organized in the same order as the Android app:

1. Greeting header ("Good evening, <name>")
2. Quick Picks (~20 diversified songs)
3. Your Mix (shuffle-play button + collage)
4. Daily Mix (merged with YT "Daily Discover")
5. Recently played (local history)
6. New releases from favorite artists
7. Listening stats preview

## Files Created

### Service Layer (Rust)
- `crates/sf-services/src/home/android.rs` - Core types and utilities
- `crates/sf-services/src/home/android/greeting.rs` - Greeting header section
- `crates/sf-services/src/home/android/quick_picks.rs` - Quick Picks computation
- `crates/sf-services/src/home/android/your_mix.rs` - Your Mix section
- `crates/sf-services/src/home/android/daily_mix.rs` - Daily Mix section
- `crates/sf-services/src/home/android/recently_played.rs` - Recently played section
- `crates/sf-services/src/home/android/new_releases.rs` - New releases section

### Bridge Layer (Rust)
- `crates/materialfy/src/bridges/home.rs` - Modified to expose `androidSections` property

### Existing File Modifications
- `crates/sf-services/src/home.rs` - Added Android sections computation and wiring
- `crates/sf-services/src/lib.rs` - Already had `pub mod home;`

## Architecture

### Core Types (`AndroidHomeSection`)
```rust
pub struct AndroidHomeSection {
    pub key: String,              // Section identifier
    pub title: String,            // Display title
    pub kind: String,             // Render type (greeting, trackGrid, yourMix, etc.)
    pub items: Vec<Value>,        // Section data
    pub actions: Vec<SectionAction>, // Header actions (shuffle play, etc.)
    pub collage_covers: Vec<String>, // For Your Mix collage
    pub greeting_text: Option<String>, // For greeting section
    pub user_name: Option<String>,     // For greeting section
}
```

### Section Constants
- `SECTION_GREETING`
- `SECTION_QUICK_PICKS`
- `SECTION_YOUR_MIX`
- `SECTION_DAILY_MIX`
- `SECTION_RECENTLY_PLAYED`
- `SECTION_NEW_RELEASES`
- `SECTION_LISTENING_STATS`

## Key Features Implemented

### 1. Greeting Section
- Time-appropriate greetings (morning/afternoon/evening/night)
- Personalized with user name when signed in
- Fallback to anonymous greeting when not signed in

### 2. Quick Picks
- Extracts tracks from YouTube home shelves with "quick" or similar titles
- Diversifies to max 2 songs per artist
- Deduplicates by URI
- Shows "See all" action
- Target: ~20 songs

### 3. Your Mix
- Finds "Your Mix" or similar playlists from "Made for you"
- Builds album-art collage from up to 4 unique covers
- Shuffle-play button on header

### 4. Daily Mix
- Merges Spotify Daily Mix with YouTube "Daily Discover" shelf
- Play and Shuffle actions

### 5. Recently Played
- Shows local listening history
- Context-aware (shows playlist/album/artist when available)
- Shows "See all" action
- Limit: 16 items

### 6. New Releases from Favorite Artists
- Shows new releases from favorite/top artists
- Uses existing `ReleasesService`
- Limit: 16 albums
- Shows "See all" action

### 7. Listening Stats Preview
- Placeholder section with "View stats" action
- Links to full stats page

## QML Integration

The `Home` bridge now exposes:
- `androidSections: QList<QVariant>` - The Android-style sections in order
- Signal: `androidSectionsChanged()`

## Data Flow

1. `HomeService::publish()` builds Android sections alongside regular shelves
2. `HomeService::build_android_sections()` constructs each section in order
3. Changes are pushed to `HomeSnapshot.android_sections`
4. Bridge's `apply()` detects changes and emits signal
5. QML's `HomePage` can display sections from `Home.androidSections`

## Testing

Unit tests included in each module:
- `diversify_by_artist()` - Ensures max N songs per artist
- `video_id_from_uri()` - Extracts YouTube video IDs
- Section building tests
- Collage building tests
- Greeting time-of-day logic

## Notes

- The implementation integrates with existing services: `HomeService`, `HistoryService`, `ReleasesService`
- YouTube radio integration is prepared but simplified (would need actual async fetching)
- All sections are optional and will only appear when data is available
- The sections follow the Material Design 3 styling of the existing UI

## Next Steps (for full implementation)

1. Update `HomePage.qml` to render `androidSections` with appropriate components
2. Add UI components for each section kind (trackGrid, yourMix, dailyMix, etc.)
3. Wire up section actions (shuffle play, see all)
4. Add actual YouTube radio fetching for Quick Picks
5. Test with real data
