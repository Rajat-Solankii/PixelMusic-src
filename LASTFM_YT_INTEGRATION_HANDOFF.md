# Last.fm Integration page + real YouTube sync toggles — handoff

Pixel Music: Rust + Qt 6.12 QML desktop music player (YouTube Music, Last.fm), own Md3 component library in
`src/pixelmusic/qml/Md3`. The user ports the look of their Android app (PixelPlayer/SoundWave) to desktop in
Material 3 Expressive, colours from the dynamic `Theme.*` roles, dark + light mode.
Qt: `C:\Qt\6.12.0\msvc2022_64`. Run from source: `run_windows.bat`. qmllint:
`C:/Qt/6.12.0/msvc2022_64/bin/qmllint.exe -I src/pixelmusic/qml <file>` (baseline noise: `[missing-property]` on
Theme.motion/shape, `[unqualified]`, `[import]` of PixelMusic.Core — only other categories matter).
Runtime QML errors are only visible with `QT_FORCE_STDERR_LOGGING=1` (qmllint misses e.g. a `Behavior` on a
`readonly` property — that blanked the whole Settings page once).
Screenshot a page: `target/release/pixelmusic.exe --new-instance [--light] --size 1280x2600
--set startPage=settings:accounts --delay 6000 --screenshot out.png`.
Nothing is committed; don't commit unless asked. Many uncommitted edits in the tree belong to earlier work.

## State before this task (done, uncommitted)
- The app was renamed from Materialfy: crate `crates/pixelmusic`, QML in `src/pixelmusic/qml`, QML module
  `PixelMusic.Core`, env vars `PIXELMUSIC_*`. Storage migrates once on start (`mf_platform::dirs::migrate_legacy`,
  registry copy in `settings.rs` `registry::migrate_legacy`, credentials moved on read in
  `sf-auth/src/credentials.rs`, and `keep_credential_profile`). Verified on the user's machine: settings, data,
  cache and both logins (YouTube Music, Last.fm) carried over.
- Settings → Accounts was rebuilt in `src/pixelmusic/qml/app/AccountsSettings.qml` (loaded by `accountsLoader` in
  `app/pages/SettingsPage.qml`): Connected Accounts tiles, red YouTube card (Linked/Options expander; today only
  "Sync Listening Activity" = `Settings.youtubeHistorySync`), Sync Library Now (`Library.refresh()`), Disconnect,
  Quick Import (Spotify/YouTube → `Library.importPlaylist`), Last.fm card (Connected chip, Open Service, Log out),
  Last.fm Options rows + Scrobbling Configuration sliders, Available Services.
- Scrobble thresholds are real settings: `Settings.lastfmMinTrackS` (30), `lastfmThresholdPct` (50),
  `lastfmMaxThresholdS` (180) → `ScrobbleRule` in `crates/sf-services/src/lastfm.rs`.
- New settings keys go in `crates/mf-platform/src/settings.rs` (enum, `ALL` array + its length, `name()`,
  `default_value()`, `normalize()` for ranges) and `crates/pixelmusic/src/bridges/settings.rs` (qproperty,
  setter decl, `#[qsignal]` with `cxx_name`, and the `settings!` macro row).

## What the user asked for now (reference screenshots: their Android app)
1. **"Open Service" opens an in-app "Last.fm Integration" page** (today it opens last.fm in the browser). Make it a
   routed page or sub-category (e.g. `settings:lastfm` via the router, with a back arrow), containing:
   - **Accounts**: row "Logged in as <username> · Click to Log Out" with chevron (→ `LastFm.disconnect()`, confirm).
     Keep a way to open the profile on last.fm (e.g. a link/button) since Open Service no longer does that.
   - **Options**: Enable Scrobbling (`Settings.lastfmScrobble`), Update Now Playing (`Settings.lastfmNowPlaying`),
     **Explore recommendations with Last.fm** ("Use your Last.fm listening history, top albums, and friends to
     personalize the Explore page").
   - **Smart Mix Playlists**: **Last.fm Smart Mix** ("Use Last.fm recommendations for Smart Mixes instead of
     YouTube Music"), **Show Smart Mix playlists in Library**, **Sync local playlists to YouTube Music** ("Sync local
     playlist edits back to your YouTube Music account"), **Generated Playlists Retention** (choice chip, e.g.
     "Permanently Store" / time-limited).
   - **Scrobbling Configuration**: the three sliders (move them here from the Accounts page).
   Grouped pill rows with round icons + switches, PixelPlayer style (see `SettingsCategoryRow` with `trailingIcon:""`
   and a `Switch` child, and `SliderRow` in `AccountsSettings.qml`). The Accounts page keeps only the Last.fm card.
2. **"Add these YouTube integrations properly"** — the YouTube card's Options should show, all wired for real:
   - **Sync Playlists & Liked Songs** — "Sync YouTube Music playlists and likes with your library" (library sync
     from the account on/off; today it is always on: `sf-services/src/library.rs`, `catalog/yt.rs`).
   - **Sync Listening Activity** — exists (`Settings.youtubeHistorySync`).
   - **Personalized Explore Feed** — "When off, loads the explore page anonymously without account history"
     (home/explore requests without the account cookies: `sf-services/src/home/yt.rs`, `home/android.rs`,
     `extra/feed.rs`, `extra/shelves.rs`).
   - **Personalized Queue & Radios** — "When off, builds auto-play recommendations purely based on current song"
     (radio/autoplay/next requests without the account: look at `radio`, autoplay in `sf-player`/`sf-services`).

## Rules from the user / previous sessions
- Only show a toggle that has a real backend. Any toggle needing a big new subsystem (e.g. local playlists that
  sync to YouTube, generated-playlist retention if Smart Mix playlists aren't stored at all) — investigate first
  (`crates/pixelmusic/src/bridges/smart_mix.rs`, `sf-services/src/lastfm_feed.rs`), then **ask the user** before
  building it rather than showing a dead switch.
- Every icon name must exist in `src/pixelmusic/assets/fonts/icons.json` (subset font; e.g. `cloud`, `expand_less`,
  `trending_up` are missing).
- Done = qmllint clean (beyond baseline), `cargo check`/tests pass for changed crates, app builds and runs via
  `run_windows.bat`, screenshots of the new page in dark + light.
- The user's standing rule: at ~50–55 % context, write an md handover like this one and continue in a new session.
