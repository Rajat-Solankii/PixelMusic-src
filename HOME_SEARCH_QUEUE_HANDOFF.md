# Handover: Android-style Home, search history, queue panel bug (2026-11-12)

Repo `D:\Desktop\PixelMusicDesktop` (Materialfy, Rust 2024 + Qt 6.12/QML). All work is uncommitted on `main`.
Don't commit, push, stash or switch branches.

## State as of this handover
- **Spotify sign-in removed.** `sf-services/src/auth.rs` is a stub, and the built-in client IDs are deleted. `Auth.state` is always
  signed out. The QML pages for it are gone (AccountPage, PlaybackSetupPage, RadioOrbit, and Spotify cards in Settings).
- **Search uses YouTube Music.**
  - The bridge `crates/materialfy/src/bridges/catalog.rs::search` calls `CatalogService::search_youtube`
    (`sf-services/src/catalog/yt.rs`, which fills best and totals). Works signed out.
  - A track click in `SearchPage.qml` calls `Player.playRadioFrom(model,row)`, which calls `WaveService::play_track_radio`
    (`sf-services/src/wave/yt.rs::yt_play_track`).
  - The song plays now as a one-item list with context `youtube:playlist:RDAMVM<id>` (radio). When the radio arrives,
    `PlayerHandle::extend_context`, which is `Command::ExtendContext` and then `YtLane::extend_context` (`sf-player/src/state/youtube.rs`),
    appends the songs without restarting.
- **Home.** `HomePage.qml` always loads `Home.youtubeShelves` (anonymous YouTube Music home feed); `youtubeHome: true`.
  The shelf kinds rendered are `quick` (grid), `tracks`, and the row shelf; see `sf-services/src/home/yt.rs`.
- **YouTube playback.** Works signed out via the PO token, decipher and QuickJS code (`sf-services/src/youtube/{potoken,cipher,jsworker}.rs`;
  `PO_TOKEN_HANDOFF.md`).
- Local listening log: `sf-services/src/history.rs` (journal). Stats: `ListeningStatsCard.qml`.

## Task A (bug, do first): queue panel
The user's screenshot of the queue side panel while a search radio is playing ("Playing radio · Tu hai kahan Radio"):
- The "Now playing" section is EMPTY.
- The current song's row ("Haseen", Talwiinder…) is drawn on top of the dashed "Drop a track here to play it next" zone
  (`app/QueueDropZone.qml`) instead of under "Now playing".
- **Drag and drop doesn't work.**

"Then the song goes over here when played by clicking": a song clicked in the queue ends up drawn over the drop zone.
Likely causes:
- The queue view's section and row layout breaks when the current row changes or a row is removed while delegates animate.
- The YouTube lane's `rows()` / uid changes after `play_row` or `extend_context`.

Find the queue panel QML (grep `QueueDropZone`, `Next from`, `Now playing`) and the player bridge queue model
(`crates/materialfy/src/bridges/player.rs`, `YtLane::rows`).

Fix the layout and dragging, both drag inside the queue and dragging a track onto "play it next". Verify with a screenshot of the
panel after clicking a queue row. The panel can be opened in a QML probe or from Main.

## Task B: Android-style Home sections
The Android app is `C:\Users\rajat\AndroidStudioProjects\soundwave1\app\src\main\java\com\unshoo\pixelmusic`.
- Screen: `presentation/screens/HomeScreen.kt` (around lines 540–900).
- Logic: `presentation/viewmodel/QuickPicksViewModel.kt` (around 330–520), `data/DailyMixManager.kt`, `presentation/viewmodel/DailyMixStateHolder.kt`.

Android Home, in order:
1. Greeting header ("Good evening, <name>").
2. **Quick Picks**, about 20 songs:
   - Primary: the YT home shelf whose title contains "quick", plus the "discover/daily/your mix" shelf.
   - Secondary: `RDAMVM` radios of up to 3 recently played songs, and radios of 2 seed artists.
   - Diversified, with a maximum of 2 songs per artist. Shown as a song grid, with "See all".
3. **Your Mix**: a header with a shuffle-play button and an album-art collage.
4. **Daily Mix**, merged with YT "Daily Discover", opening a Daily Mix page.
5. **Recently played** (local history).
6. **New releases from favorite artists** (YouTube artist pages).
7. A listening stats preview.

Then the remaining YouTube home shelves.

Desktop gaps to fill:
- Compute these in `sf-services` (YouTube service `radio`, `artist`, `home`, plus local history).
- Expose them through the `Home` bridge (`crates/materialfy/src/bridges/home.rs`).
- Render them in `HomePage.qml`, keeping its MD3 style.

## Task C: search history ("search history is not stored")
- The desktop has none. Android uses `SearchHistoryDao` / `SearchHistoryEntity` (under `data/database`).
- Store recent queries locally (in the app data dir, as other services persist JSON), with a cap of about 20, deduplicated, newest first.
  Record a query when a search is run or a result is opened.
- Show them in `SearchPage.qml` when the box is empty, with a click to search again, remove one, and clear all.
- Also wire YouTube typing suggestions: `CatalogService::youtube_suggestions` exists but has no bridge.

## Rules
- Tests use mocks, fixtures and `Store::in_memory` only. Never call real YouTube from tests.
- User-facing strings go through `mf_i18n::t` / `I18n.t`.
- Existing style: not rustfmt-formatted, about 120 columns.
- Never port the Android `TelegramAdminManager` or any telemetry.

## Build and check
- Build, in bash:
  ```bash
  export PATH="$USERPROFILE/.cargo/bin:/c/Qt/6.12.0/msvc2022_64/bin:$PATH" QMAKE='C:\Qt\6.12.0\msvc2022_64\bin\qmake6.exe' QTDIR='C:\Qt\6.12.0\msvc2022_64'
  cargo build -p materialfy
  ```
  Tests: `cargo test -p sf-player -p sf-services`.
- QML errors are NOT visible on stderr from a tool shell. Write a probe `.qml` that calls
  `Qt.createComponent("file:///D:/.../X.qml").errorString()` (or reads bridge properties) into a Text, and run it:
  `target\debug\materialfy.exe --new-instance --config-dir <scratch> --qml probe.qml --screenshot out.png --delay 4000`.
  Then read the PNG. Qt's DLLs must be on PATH (`C:\Qt\6.12.0\msvc2022_64\bin`).
- Home screenshot: `materialfy.exe --new-instance --config-dir <scratch> --screenshot home.png --delay 12000 --size 1400x900`.
