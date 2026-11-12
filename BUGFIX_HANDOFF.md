# Bug-fix pass, 2026-11-12: handover

Everything below is uncommitted on `main`. `cargo build -p materialfy` succeeds; `cargo test -p sf-player -p sf-services
-p mf-platform -p mf-i18n -p sf-auth -p sf-api` is green (sf-player 366, sf-services 385 + 9 integration).
Build environment: see `PO_TOKEN_HANDOFF.md`.

## Done

1. **Taskbar player freeze** (`crates/materialfy/cpp/floating.cpp`). The UI Automation scan of the taskbar buttons ran
   on the GUI thread for every position except "By the tray". The widget is a child of `Shell_TrayWnd`, so it sits in
   the tree being scanned: UIA waits for Explorer while Explorer waits for our thread, and the shared input queue
   stalls the taskbar too. The scan now runs on its own MTA thread (`buttons_worker`, 2 s / 3 s UIA timeouts);
   `taskbar_buttons` only reads the last result and asks for a new one, and a changed result calls QML `place()`.
   "Left" again goes after Start and the apps when the taskbar icons are left-aligned.
2. **Seek goes silent** (`crates/sf-player/src/engine/youtube/player.rs`, `download.rs`).
   - The seek thread remembered `was_playing` after `playing` was cleared, so every seek ended paused. Seeks now
     carry a `Seeking { id, play, next }` state: Play/Pause during a seek are kept, the last of several seeks wins,
     Stop/Load cancel it.
   - The downloader was strictly sequential, so a forward seek in a 30 MB Monochrome FLAC waited for everything in
     between. It now keeps sparse pieces and jumps to where the reader waits (`State::want`), then fills the gap.
3. **YouTube "Sign in to confirm you're not a bot" while signed in** (`crates/sf-services/src/youtube/stream.rs`,
   `client.rs`, `mod.rs`). New tier `ANDROID_VR_USER`: the VR client with the account's cookie and SAPISIDHASH, asked
   after the anonymous VR and before the web tier. `set_account` now clears tier cooldowns and the stream cache
   (a bot check from before sign-in used to be replayed for 15 minutes).
4. **Search history** was half-wired: QML called `Catalog.searchHistory` / `addSearchQuery` / `removeSearchQuery` /
   `clearSearchHistory`, none of which existed, and the history block used unknown types and properties
   (`TextButton`, `Icon.icon`, numeric `IconButton.size`). Added `pub mod search_history`, the bridge members in
   `crates/materialfy/src/bridges/catalog.rs`, and rewrote the block in `SearchPage.qml`.
5. Smaller: `decode.rs` reports a refused download as such instead of "unsupported audio" (that test was failing);
   `QueueView.qml` section lookup back to `queue.find` (the loop built a map per queued track on every change);
   `connect.rs` test fixture order after the Materialfy → Pixel Music rename.

## Not verified / open

- **Taskbar positions on the real taskbar**: the debug build started and stayed responsive, but nobody clicked
  through Left / Before apps / After apps while a song played. That is the test that matters.
- **Signed-in Android VR against real YouTube**: covered by mock-server tests only. If YouTube rejects cookies for
  this client, the web tier still follows; set `RUST_LOG=debug` and look for `youtube streams:` lines.
- **Monochrome seek against the real server**: covered by a local range-server test only.
- **Search page**: not opened in the running app after the rewrite.
- The rename changed English source strings (`"…return to Pixel Music."`), so their RU/PT translations no longer match.
