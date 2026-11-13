# Accounts settings redesign — handoff

Goal: rebuild **Settings → Accounts** (`src/pixelmusic/qml/app/pages/SettingsPage.qml`, the `accountsLoader`
Loader, ~lines 334–674) to match the user's PixelPlayer/SoundWave Android reference screenshots, in Material 3
Expressive, coloured from the app's dynamic theme (`Theme.*` roles), dark and light mode.

## Already done this session (uncommitted, in the working tree)
- Lyrics toolbar buttons M3E (`LyricsTools.qml`, `Md3/ButtonGroup.qml` got `selectedColor/selectedContentColor/selectedGrow`, `NowPlaying.qml`).
- Settings category icons: per-category harmonized hues (`SettingsCategoryRow.qml` `hue`, `SettingsPage.qml` categories).
- Search page: Browse & Discover tiles + pill Recent searches (`SearchPage.qml`); Moods & genres pills in
  `ShelfPage.qml` (kind "moods", items `{kind:"browse", title, browseId, browseParams}`); Rust
  `sf-services/src/home/yt.rs` `youtube_browse` routes FEmusic_charts / FEmusic_moods_and_genres.
- Recently played pills: grey covers now use surfaceContainerHighest/onSurface (`mf-theme/src/lib.rs` `cover_tones`).
- Release build was produced once (`build_windows.ps1 -QtDir C:\Qt\6.12.0\msvc2022_64 -Portable` → `target\dist`).
  Run from source: `run_windows.bat`.

## Target layout (top to bottom)
1. **Connected Accounts** summary card (surfaceContainerHigh, extraLarge radius): title, one-line description,
   two stat tiles "Active N" (YouTube signed in + Last.fm connected) and "Available N" (services offered).
2. **YouTube Music card** — a branded gradient card (deep red → red; in light mode a lighter tint; keep text
   contrast ≥ 4.5:1): YouTube logo tile, account name (`Auth.youtubeAccount.name`), "Connected · Tap to configure
   sync", a **"Linked ▾ / Options ▴"** pill that expands a "Synchronization & Personalization" section with switches.
   Below a divider: filled white-ish pill **"Sync Library Now"** and outlined pill **"Disconnect"** (`Auth.youtubeSignOut()`).
   Signed-out state keeps today's Sign in / cookie paste flow (keep all of it: `Auth.startYouTubeLogin`,
   `Auth.youtubeSignInWithCookies`, `Auth.cancelYouTubeLogin`, error banner).
   Switches: only wire ones with a backend. Exists: `Settings.youtubeHistorySync` ("Sync Listening Activity").
   Check for others (grep `crates/pixelmusic/src/bridges/settings.rs`, `library.rs`) before adding; do not show
   fake toggles. "Sync Library Now": find a library refresh/reload invokable in `bridges/library.rs`; if none, add one
   that re-reads YouTube playlists + likes.
3. **Quick Import** card: purple round cloud icon, "Sync playlists & songs instantly", a 2-segment selector
   (Spotify green when selected / YouTube), helper text, a pill URL field with a round → button.
   Backend: `Library.importPlaylist(input, save)` + `importRunning/importDone/importTotal/importError` (YouTube Music).
   Check whether it accepts Spotify links; if not, either add Spotify public-playlist import or show the Spotify tab
   only if supported (ask the user if it needs new scraping).
4. **Linked Services** header + **Last.fm card**: round icon, username (`LastFm.username`), "Connected" chip,
   inner pill "Scrobbling enabled" status, primary-container **Open Service** (open `https://www.last.fm/user/<name>`),
   outlined **Log out** (`LastFm.disconnect()`). Not connected: keep the existing API key / secret / Connect flow.
   Options as grouped pill rows (large outer, small inner corners like SettingsCategoryRow) with round icons + switches:
   Enable Scrobbling (`Settings.lastfmScrobble`), Update Now Playing (`Settings.lastfmNowPlaying`).
   **Scrobbling Configuration** sliders with value chips: Minimum track duration (default 30 s), Scrobble threshold
   percentage (50 %), Max threshold duration (180 s). These are hardcoded today in
   `crates/sf-services/src/lastfm.rs` (`SCROBBLE_CAP`, the 30 s minimum, half-track rule ~line 178) → add three
   settings (mf-platform `settings.rs` keys, `bridges/settings.rs` qproperties like `lastfm_scrobble`) and read them there.
5. **Available Services**: Telegram / Google Drive have no backend — show as disabled "Coming soon" pills or omit (ask).

## Also from the same request
- Smart-mix related Last.fm toggles in the reference (Last.fm Smart Mix, show Smart Mix playlists in Library,
  sync local playlists to YouTube Music, generated playlist retention) — only add those with a real backend;
  list the rest to the user as follow-ups.

## Checks
- `C:/Qt/6.12.0/msvc2022_64/bin/qmllint.exe -I src/pixelmusic/qml <file>` for syntax.
- Icons must exist in `src/pixelmusic/assets/fonts/icons.json` (subset font) — e.g. `trending_up` is missing.
- `cargo check -p sf-services`, `cargo test -p mf-theme`; build + run with `run_windows.bat`.
- Nothing is committed; don't commit unless asked.
