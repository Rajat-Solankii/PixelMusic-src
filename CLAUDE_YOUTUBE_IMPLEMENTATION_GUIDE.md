# Claude Task Implementation Guide: YouTube & YouTube Music Integration in PixelMusicDesktop

**Objective**: Implement full-featured YouTube & YouTube Music support (authentication, playlist fetching & editing, search, home/explore/charts, albums/artists, stream resolution & playback, radio/autoplay, lyrics/transcripts, and playlist import) in `D:\Desktop\PixelMusicDesktop` by porting the YouTube engine from `C:\Users\rajat\AndroidStudioProjects\soundwave1` (`desktopApp` + `app`), **while strictly preserving PixelMusicDesktop's existing Material 3 Expressive Qt Quick UI design**.

---

## 1. Critical Architectural Context & Current State

### 1.1 Target Stack vs. Reference Stack
Do **not** look for Gradle, Kotlin, or Compose in `PixelMusicDesktop`:
- **Reference (`C:\Users\rajat\AndroidStudioProjects\soundwave1`)**: Kotlin Multiplatform / Compose Multiplatform Desktop (`:desktopApp`) + Android (`:app`) sharing an InnerTube Kotlin client (`app/src/main/java/unshoo/ianshulyadav/pixelmusic/innertube/` synced via Gradle `syncInnertube` in `desktopApp/build.gradle.kts`).
- **Target (`D:\Desktop\PixelMusicDesktop`)**: **Materialfy v0.5.1** — **Rust 2024 (`rust-version = "1.98"`)**, **Tokio multi-threaded runtime**, **Qt 6.12 Quick/QML**, and **CXX-Qt 0.10.0** bridges.
  - Workspace crates (`Cargo.toml`):
    - `crates/md3-model`: Flat camelCase serde models (`TrackItem`, `PlaylistItem`, `AlbumItem`, `ArtistItem`, `LibraryEntry`, `Account`, `PlayContext`).
    - `crates/sf-auth`: OS keyring credential storage (`keyring-core`, Windows Credential Manager / macOS Keychain / Linux Secret Service) + OAuth PKCE + proxy config.
    - `crates/sf-api`: Spotify Web API client, URI helpers (`util.rs`), atomic file replacement (`replace_file`).
    - `crates/sf-player`: Playback actor (`PlayerHandle`, `state::core::Core`, `state::actor::Actor`), `librespot` engine, `rodio` + `fastframe-audio` (`cpal`) sink with EQ/limiter/resampler/visualizer tap (`AudioTap`).
    - `crates/sf-services`: Async services (`Services`, `AuthService`, `LibraryService`, `CatalogService`, `HomeService`, `ReleasesService`, `WaveService`, `HistoryService`, `LyricsService`, `DevicesService`, `SocialService`).
    - `crates/mf-theme`: `material-colors` OKLab dynamic theming.
    - `crates/mf-lyrics`: LRC/YRC lyrics fetcher (LRCLIB, NetEase, Musixmatch).
    - `crates/mf-platform`: OS dirs, `SettingsStore` (`SettingKey` registry/INI), MPRIS/SMTC.
    - `crates/mf-i18n`: Localization (`mf_i18n::t(...)` / `I18n.t(...)`).
    - `crates/materialfy`: Binary crate (`[[bin]] name = "materialfy"`), CXX-Qt bridges in `src/bridges/`, QML registration in `cpp/register.cpp`, and QML UI in `src/materialfy/qml/`.

### 1.2 Current Repository State
- `git status`: `main` is unmodified except untracked `tests/fixtures/youtube/` containing 8 valid JSON mock files:
  - `auth_session.json`, `browse_library.json`, `error_responses.json`, `player_response_android_vr.json`, `player_response_web_remix.json`, `playlist_tracks_page1.json`, `playlist_tracks_page2.json`, `search_response.json`.
- **Important Fixture Bug Already Spotted**:
  - `tests/fixtures/youtube/playlist_tracks_page1.json` (line 61), `playlist_tracks_page2.json` (line 27), and `search_response.json` (line 38) currently use `"musicResponsiveListItemFlexColumnRenderer"` inside `"fixedColumns"` instead of real InnerTube's `"musicResponsiveListItemFixedColumnRenderer"` (see `soundwave1/.../PlaylistPage.kt:42`).
  - Fix those three fixture files (or accept both in the parser as a defensive fallback) before writing parser assertions.
- **Pre-existing Dependencies in `Cargo.lock` / `Cargo.toml`**:
  - Already in root `[workspace.dependencies]`: `tokio`, `reqwest`, `serde`, `serde_json`, `sha1 = "0.10.6"`, `sha2`, `hex = "0.4.3"`, `base64`, `keyring-core`, `thiserror`, `anyhow`, `tracing`, `chrono`, `jiff`, `regex`, `urlencoding`, `rodio = { version = "0.21", default-features = false }`, `fastframe-audio`.
  - Already resolved in `Cargo.lock`: `tokio-tungstenite = "0.28.0"` (and `tungstenite = "0.28.0"`), `symphonia = "0.5.5"` (currently with `vorbis`, `ogg`, `flac`, `mp3` subcrates — see Section 4.4 for AAC/Opus audio decoding!).

---

## 2. Complete YouTube Feature Parity Matrix (`soundwave1` -> `PixelMusicDesktop`)

Every YouTube capability present across `soundwave1/desktopApp` and `soundwave1/app/innertube`:

| ID | Capability | `soundwave1` Reference Files | Target Location in `PixelMusicDesktop` |
|---|---|---|---|
| **F1** | **YouTube Browser CDP Sign-In & Keyring Storage** | `desktopApp/.../data/YouTubeAuth.kt` | `crates/sf-auth/src/youtube.rs`, `credentials.rs` |
| **F2** | **InnerTube HTTP Client & `SAPISIDHASH` Signing** | `app/.../innertube/InnerTube.kt`, `YouTubeClient.kt`, `PlaybackAuthState.kt` | `crates/sf-services/src/youtube/client.rs` |
| **F3** | **Account Info & Anonymous `visitorData`** | `app/.../innertube/YouTube.kt` (`accountInfo`, `visitorData`) | `crates/sf-services/src/youtube/` |
| **F4** | **User Library & Playlists (`FEmusic_liked_playlists` + `"LM"`)** | `desktopApp/.../AccountLibrary.kt`, `YouTubeRepository.kt`, `YouTube.kt:1061-1188` | `crates/sf-services/src/youtube/library.rs`, `LibraryService` |
| **F5** | **Playlist Tracks, Dual-Layout Parsing & Continuations** | `YouTube.kt:627-745, 1925-1993`, `YouTubeRepository.kt:172-194`, `PlaylistPage.kt` | `crates/sf-services/src/youtube/playlist.rs` |
| **F6** | **Playlist & Library Mutations** (`create`, `rename`, `delete`, `add`, `remove`, `move`, `like`, `dislike`, `subscribe`) | `YouTube.kt:1580-1674`, `EditPlaylistBody.kt` | `crates/sf-services/src/youtube/mutations.rs`, `LibraryService` |
| **F7** | **Multi-Category Search, Summary, Suggestions & Continuations** | `YouTubeRepository.kt:37-67`, `YouTube.kt:230-385`, `SearchPage.kt`, `SearchSummaryPage.kt` | `crates/sf-services/src/youtube/search.rs`, `CatalogService` |
| **F8** | **Stream Resolution (`ANDROID_VR` + `WEB_REMIX` + fallback), Single-Flighting, TTL Cache & Playback** | `desktopApp/.../StreamResolver.kt`, `StreamClientUtils.kt`, `PoTokenGenerator.kt` | `crates/sf-services/src/youtube/stream.rs`, `crates/sf-player/` |
| **F9** | **Home Feed, Explore, New Releases, Charts, Moods & Genres** | `YouTube.kt:758-1485`, `YouTubeRepository.kt:69-88` | `crates/sf-services/src/youtube/browse.rs`, `HomeService`, `ReleasesService` |
| **F10** | **Album, Artist, Song & Media Info Pages** | `YouTube.kt:387-625, 830-899, 1852-1860` | `crates/sf-services/src/youtube/catalog.rs`, `CatalogService` |
| **F11** | **Radio / Up-Next Autoplay (`RDAMVM...`) & Related** | `YouTube.kt:1752-1850`, `YouTubeRepository.kt:90-102`, `PlayerController.kt:615-646` | `WaveService`, `Player` autoplay / `Core` |
| **F12** | **Watchtime / Playback Telemetry & Music History (`FEmusic_history`)** | `YouTube.kt:1486-1578, 1714-1750` | `HistoryService`, `crates/sf-services/src/youtube/` |
| **F13** | **YouTube Lyrics & Transcripts** | `YouTube.kt:1809-1812, 1862-1871` | `LyricsService` / `mf-lyrics` |
| **F14** | **Playlist Import (YouTube URL/ID, Spotify->YT Match, M3U, JSON)** | `desktopApp/.../PlaylistImport.kt` | `crates/sf-services/src/youtube/import.rs`, `Library` bridge |
| **F15** | **UI Integration & Design Preservation** | `src/materialfy/qml/` (`Main.qml`, `Sidebar.qml`, `SettingsPage.qml`, `AccountPage.qml`, etc.) | `crates/materialfy/src/bridges/`, QML pages |

---

## 3. Exact Protocol & Implementation Specifications (From `soundwave1`)

### 3.1 Authentication: Browser CDP + Keyring Persistence (`crates/sf-auth/src/youtube.rs`)
Google blocks embedded WebViews and standard desktop OAuth for YouTube Music. Port `YouTubeAuth.kt` exactly:
1. **Browser Discovery (`browser_path()`)**: Search in order on Windows:
   - `%ProgramFiles(x86)%\Microsoft\Edge\Application\msedge.exe`
   - `%ProgramFiles%\Microsoft\Edge\Application\msedge.exe`
   - `%ProgramFiles%\Google\Chrome\Application\chrome.exe`
   - `%ProgramFiles(x86)%\Google\Chrome\Application\chrome.exe`
   - `%LOCALAPPDATA%\Google\Chrome\Application\chrome.exe`
   *(On Linux/macOS, check `google-chrome`, `chromium`, `microsoft-edge` / `/Applications/...`.)*
2. **Process Launch**:
   - Bind an ephemeral TCP port (`TcpListener::bind("127.0.0.1:0")` -> get port, drop listener immediately before spawning browser).
   - Use dedicated profile dir: `mf_platform::dirs::state_dir().join("youtube-signin-profile")`.
   - Launch flags:
     - `--remote-debugging-port=<port>`
     - `--remote-allow-origins=http://127.0.0.1:<port>`
     - `--user-data-dir=<profile_dir>`
     - `--no-first-run`
     - `--no-default-browser-check`
     - `--window-size=520,760`
     - `--app=https://accounts.google.com/ServiceLogin?service=youtube&passive=true&continue=https://music.youtube.com/`
3. **CDP Polling & Extraction**:
   - Poll `GET http://127.0.0.1:<port>/json` every 1200ms (10-minute overall timeout; abort immediately if process exits after `seen_browser == true`).
   - Find first JSON object where `type == "page"` and `url.starts_with("https://music.youtube.com")`, read `webSocketDebuggerUrl`.
   - Connect via `tokio-tungstenite` (`0.28.0` is already in `Cargo.lock`; expose it in root `Cargo.toml` `[workspace.dependencies]` and `crates/sf-auth/Cargo.toml`):
     - Call `Network.getCookies` with `{"urls": ["https://music.youtube.com", "https://www.youtube.com"]}`.
     - Deduplicate cookies by name, format as `"name=value; ..."`.
     - Require `SAPISID` or `__Secure-3PAPISID` or `__Secure-1PAPISID`.
     - Call `Runtime.evaluate` (`returnByValue: true`) for:
       - `(window.yt&&yt.config_&&yt.config_.DATASYNC_ID)||''` -> normalize per `PlaybackAuthState.kt:111-116` (if `"||"` present: if ends with `"||"` take `substring_before("||")`, else `substring_after("||")` — wait: note `YouTubeAuth.kt:189` uses `substringBefore("||")`; check live behavior carefully, keeping `YouTubeAuth.kt`'s `substringBefore("||")` as primary when trailing or testing both).
       - `(window.yt&&yt.config_&&yt.config_.VISITOR_DATA)||''`.
   - Kill browser process (and process tree on Windows) in a `finally`/Drop guard!
   - Call `accountInfo()` (`POST account/account_menu`) to enrich `name`, `email`, `avatar`.
4. **Manual Cookie Fallback**:
   - Provide `YouTubeAccount::from_cookie_string(...)` so users without Edge/Chrome (or in restricted environments) can paste a `Cookie:` header string extracted from DevTools.
5. **Keyring Storage (`crates/sf-auth/src/credentials.rs`)**:
   - Add `Slot::YouTube` (`"youtube"`) and `Grant::YouTube(YouTubeAccount)`.
   - **GOTCHA**: In `crates/sf-auth/src/credentials.rs:243`, `generations: [AtomicU64; 4]` is hardcoded to length `4`! Update it to `[AtomicU64; 5]` (or `Slot::COUNT`) and update `Slot::ALL` + the `credentials.rs` unit tests (`Slot::ALL.into_iter().zip(...)`) while keeping `Slot::SPOTIFY` untouched.
   - Use `Store::in_memory(...)` for all unit/integration tests so Windows Credential Manager is never touched in CI.

### 3.2 InnerTube Client & `SAPISIDHASH` Signing (`crates/sf-services/src/youtube/client.rs`)
1. **Base & Dependency Setup**:
   - Add `sha1.workspace = true` to `crates/sf-auth/Cargo.toml` and `crates/sf-services/Cargo.toml`.
   - Default base URL: `https://music.youtube.com/youtubei/v1/`, configurable via `with_base_url(...)` for hermetic mock testing.
   - Reuse workspace proxy settings (`sf_api::Http` or build `reqwest::Client` honoring `ProxyConfig`).
2. **Client Presets (`YouTubeClient.kt`)**:
   - `WEB_REMIX` (Primary authenticated browse/library/search/mutations/next):
     - `clientName = "WEB_REMIX"`, `clientVersion = "1.20260114.01.00"`, `clientId = "67"`
     - `User-Agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36"`
     - `X-Origin / Origin = "https://music.youtube.com"`, `Referer = "https://music.youtube.com/"`
     - `X-Goog-Api-Format-Version = "1"`, query `?prettyPrint=false`
   - `ANDROID_VR_1_61_48` (Primary unauthenticated stream resolution — bypasses PoToken/BotGuard):
     - `clientName = "ANDROID_VR"`, `clientVersion = "1.61.48"`, `clientId = "28"`
     - `User-Agent = "com.google.android.apps.youtube.vr.oculus/1.61.48 (Linux; U; Android 12; en_US; Quest 3; Build/SQ3A.220605.009.A1; Cronet/132.0.6808.3)"`
     - Context client fields: `osName = "Android"`, `osVersion = "12"`, `deviceMake = "Oculus"`, `deviceModel = "Quest 3"`, `androidSdkVersion = "32"`
     - `loginSupported = false` (do NOT send cookies / `onBehalfOfUser`)
   - `WEB` (`clientName = "WEB"`, `clientVersion = "2.20260114.00.00"`, `clientId = "1"`) for `get_transcript` (`?key=AIzaSyC9XL3ZjWddXya6X74dJoCTL-WEYFDNX3`) and `getMediaInfo`.
3. **Exact `SAPISIDHASH` Signing (`InnerTube.kt:240-250`)**:
   - Lookup key in parsed cookies: `SAPISID`, fallback `__Secure-3PAPISID`, fallback `__Secure-1PAPISID`.
   - `timestamp = unix_epoch_secs` (u64).
   - `sha1_hex = hex(sha1(format!("{timestamp} {sapisid} {request_origin}")))` where `request_origin = "https://music.youtube.com"`.
   - Header value:
     `SAPISIDHASH {timestamp}_{sha1_hex} SAPISID1PHASH {timestamp}_{sha1_hex} SAPISID3PHASH {timestamp}_{sha1_hex}`
   - Also send `cookie: <full_cookie_string>`, `X-Goog-Visitor-Id: <visitorData>`, and `context.user.onBehalfOfUser: <dataSyncId>` when `set_login && client.login_supported`.
4. **Retries & Auth Errors**:
   - Retry transient I/O errors, HTTP 5xx, and HTTP 429 with exponential backoff (up to 4 attempts: 800ms * 2^n, `InnerTube.kt:262-316`).
   - On HTTP 401 (`UNAUTHENTICATED`), emit a clean `YouTubeError::SessionExpired` / auth state reset without crashing.
   - Fetch anonymous `visitorData` on first unauthenticated call from `GET https://music.youtube.com/sw.js_data` (strip 5 chars `)]}'\n`, parse JSON `[0][2]`, pick first string matching `^Cg[ts]`).

### 3.3 InnerTube Endpoints & Parsers
- **Account Info**: `POST account/account_menu` -> `actions[0].openPopupAction.popup.multiPageMenuRenderer.header.activeAccountHeaderRenderer` -> `accountName.runs[0].text`, `email.runs[0].text`, `avatar.thumbnails`.
- **User Playlists (`FEmusic_liked_playlists`)**:
  - `POST browse` with `{"context": ..., "browseId": "FEmusic_liked_playlists"}`.
  - Prepend synthetic Liked Music card: `playlist_id = "LM"`, `uri = "youtube:playlist:LM"`, `title = "Liked Music"`, `description = "Your YouTube Music likes"`.
  - Parse `contents.singleColumnBrowseResultsRenderer.tabs[0].tabRenderer.content.sectionListRenderer.contents[0]` (`gridRenderer.items` -> `musicTwoRowItemRenderer` or `musicShelfRenderer.contents` -> `musicResponsiveListItemRenderer`), stripping `"VL"` prefix from `browseId`, filtering out duplicate `"LM"` and `"SE"` (`YouTubeRepository.kt:110`).
  - Also support `FEmusic_liked_albums`, `FEmusic_library_corpus_track_artists`, `FEmusic_history`, and `libraryContinuation`.
- **Playlist Tracks & Continuations (`YouTube.kt:627-745, 1925-1993`)**:
  - `POST browse` with `browseId = "VL" + id` (if not already prefixed `"VL"`).
  - Support **both** layouts:
    1. `contents.twoColumnBrowseResultsRenderer`: header in `tabs[0]...musicResponsiveHeaderRenderer` or `musicEditablePlaylistDetailHeaderRenderer` (`editable = true` -> `PlaylistItem.can_edit = true`); tracks in `secondaryContents.sectionListRenderer.contents[0].musicPlaylistShelfRenderer.contents`.
    2. `header.musicDetailHeaderRenderer` (used by `"LM"` Liked Music!) + tracks in `contents.singleColumnBrowseResultsRenderer.tabs[0]...musicPlaylistShelfRenderer.contents` (or `musicShelfRenderer.contents`).
  - **Continuation (`POST browse?continuation=<tok>&ctoken=<tok>`)**:
    - Check all 4 candidate structures from `playlistContinuationPageFromResponse`:
      1. `continuationContents.sectionListContinuation.contents[*].musicPlaylistShelfRenderer.contents` (+ `appendContinuationItemsAction`)
      2. `continuationContents.musicPlaylistShelfContinuation.contents`
      3. `continuationContents.musicShelfContinuation.contents`
      4. `onResponseReceivedActions[0].appendContinuationItemsAction.continuationItems`
    - Guardrails (`Utils.kt:27-64`): `seen_continuations: HashSet<String>`, `MAX_PAGES = 50`, break on `>= 2` consecutive empty pages.
  - Extract `playlistItemData.playlistSetVideoId` into a side map / extended track metadata so playlist removal & reordering mutations work!
- **Playlist & Library Mutations (`YouTube.kt:1580-1674`)**:
  - `like/like`, `like/dislike`, `like/removelike` (video or playlist target)
  - `subscription/subscribe`, `subscription/unsubscribe` (`channelIds`)
  - `playlist/create`, `playlist/delete`
  - `browse/edit_playlist` actions: `ACTION_ADD_VIDEO`, `ACTION_ADD_PLAYLIST`, `ACTION_REMOVE_VIDEO` (needs `videoId` + `setVideoId`), `ACTION_MOVE_VIDEO_BEFORE` (`setVideoId` + `movedSetVideoIdSuccessor`), `ACTION_SET_PLAYLIST_NAME`.
- **Search & Suggestions**:
  - `POST music/get_search_suggestions` -> up to 8 suggestion strings + recommended items.
  - `POST search` with filter params:
    - Songs: `EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D`
    - Videos: `EgWKAQIQAWoKEAkQChAFEAMQBA%3D%3D`
    - Albums: `EgWKAQIYAWoKEAkQChAFEAMQBA%3D%3D`
    - Artists: `EgWKAQIgAWoKEAkQChAFEAMQBA%3D%3D`
    - Featured Playlists: `EgeKAQQoADgBagwQDhAKEAMQBRAJEAQ%3D`
    - Community Playlists: `EgeKAQQoAEABagoQAxAEEAoQCRAF`
  - Plus unfiltered `searchSummary` and `searchContinuation`.
- **Home, Explore, New Releases, Charts, Moods & Genres, Album, Artist, Next/Radio**:
  - `FEmusic_home` (+ `homeContinuation`, 60s cache), `FEmusic_explore`, `FEmusic_new_releases_albums`, `FEmusic_charts` (with fallback `params = "ggMGCgQIgAQ="` and primary chart playlist hydration), `FEmusic_moods_and_genres`.
  - `album(browseId)` -> `albumSongs("VL" + playlistId)` (or inline songs fallback).
  - `artist(browseId)` -> header (`musicImmersiveHeaderRenderer` / `musicVisualHeaderRenderer` / `musicHeaderRenderer`) + sections + `artistItems` / `artistItemsContinuation`.
  - `next` with `WatchEndpoint { video_id, playlist_id: Some("RDAMVM{video_id}") }` for Radio / Up Next + `followAutomixPreview`.
  - `registerPlayback` (`playbackTracking` URL, replace `https://s.youtube.com` -> `https://music.youtube.com`, send `Cookie` + `SAPISIDHASH` + `cpn` + `ver=2` + `c=WEB_REMIX`) to update YouTube Music history!
- **Playlist Import (`PlaylistImport.kt`)**:
  - URL/ID detection for YouTube (`?list=`, `/browse/VL...`, bare IDs `PL|RD|OLAK5uy_|UU|LL|LM`), Spotify (`open.spotify.com/playlist/...`, `spotify.link/...` via embed `__NEXT_DATA__` + optional Spotify REST pagination), `.m3u`/`.m3u8`, and `.json`.
  - Fuzzy track matching (`normalize()`, `title_score * 2.0 + artist_score * 1.5 + duration_score >= 2.2`, semaphore concurrency = 4).

---

## 4. Target Crate Integration Plan

### 4.1 Model & URI Contract (`crates/md3-model`, `crates/sf-api`)
- Keep all QML-facing serde camelCase field names in `crates/md3-model/src/items.rs` unchanged!
- Standardize YouTube URIs:
  - `youtube:track:<videoId>`
  - `youtube:playlist:<playlistId>` (e.g. `youtube:playlist:LM`)
  - `youtube:album:<browseId>`
  - `youtube:artist:<browseId>`
- Update `TrackItem::play_uri()` / `sf_api::util::{uri_id, uri_kind}` to recognize `youtube:` URIs cleanly (note `uri_id` already splits on last `:`, and `uri_kind` returns the second `:` segment `"track"`/`"playlist"`/`"album"`/`"artist"`!). Add a helper `uri_provider(uri: &str)` (`"youtube"` vs `"spotify"`).
- Add `PlaylistItem::source` or detect `"youtube:"` prefix on `playlist.uri` so `LibraryService` knows whether mutations target Spotify or YouTube!

### 4.2 `crates/sf-auth`
- Update `crates/sf-auth/Cargo.toml`: add `sha1.workspace = true`, `tokio-tungstenite = "0.28"` (expose in root `[workspace.dependencies]`).
- Update `crates/sf-auth/src/credentials.rs`: add `Slot::YouTube`, `Grant::YouTube(YouTubeAccount)`, update `generations: [AtomicU64; 5]`, update `Slot::ALL` and tests.
- Create `crates/sf-auth/src/youtube.rs`: `YouTubeAccount`, cookie parsing, `compute_sapisid_hash`, browser discovery, CDP WebSocket login (`sign_in`), and `sign_out`.

### 4.3 `crates/sf-services`
- Create `crates/sf-services/src/youtube/` (`mod.rs`, `error.rs`, `client.rs`, `models.rs`, `parsers.rs`, `stream.rs`, `import.rs`).
- Expose `pub youtube: Arc<YouTubeService>` on `Services` (`crates/sf-services/src/lib.rs`).
- Wire `YouTubeService` into:
  - `AuthService` (or unified auth facade): restore `Slot::YouTube` at startup, emit auth events when YouTube signs in/out so Library/Home refresh automatically.
  - `LibraryService`: merge or switch/aggregate YouTube playlists (`"LM"` Liked Music + `FEmusic_liked_playlists`) and saved albums/artists (`LibraryEntry` with `uri = "youtube:..."`), route `playlist_tracks`, `toggle_like`, `toggle_saved`, `create_playlist`, `rename_playlist`, `add_to_playlist`, `remove_tracks`, `move_tracks` to YouTube when ID/URI is YouTube!
  - `CatalogService`: route `youtube:` / YouTube IDs (or active provider / merged search) for `search`, `album`, `artist`, `track_details`.
  - `HomeService`, `ReleasesService`, `WaveService`, `HistoryService`, `LyricsService`: integrate YouTube home shelves, new releases/charts, `RDAMVM` radio, YouTube telemetry + `FEmusic_history`, and YouTube lyrics/transcripts fallback.

### 4.4 `crates/sf-player` — Audio Stream Resolution & Playback (**Read Carefully!**)
`PixelMusicDesktop`'s `sf-player` currently plays Spotify via `librespot` feeding `RodioSink` (`fastframe-audio`). For YouTube streams:
1. **Stream Resolution (`StreamResolver`)**:
   - Tier 1: `ANDROID_VR_1_61_48` unauthenticated `/player` (direct `adaptiveFormats[].url`, 15-min bot-challenge cooldown).
   - Tier 2: Authenticated `WEB_REMIX` `/player` (if signed in and URL is direct; 15-min cooldown on failure/cipher-only).
   - Tier 3 (optional): external `yt-dlp -f bestaudio -g` fallback if installed on PATH.
   - Single-flight deduplication + bounded TTL cache (max 200 entries, `expire - 600s` margin, 1h fallback).
   - Socket preheat (`Range: bytes=0-1024`) for next track lookahead.
2. **CRITICAL PURE-RUST AUDIO DECODING DETAIL**:
   - Verified in `Cargo.lock`: `symphonia = "0.5.5"` is already resolved, **but** only with `vorbis`, `ogg`, `flac`, `mp3` (brought in by `rodio`/`librespot`).
   - **YouTube `ANDROID_VR` serves two main audio formats**:
     - `itag 140`: AAC-LC inside MP4/M4A (`audio/mp4; codecs="mp4a.40.2"`), ~128 kbps.
     - `itag 251`: Opus inside WebM/Matroska (`audio/webm; codecs="opus"`), ~160 kbps.
   - **Gotcha**: `symphonia 0.5.5` natively supports `isomp4` + `aac` (so **itag 140 MP4/AAC plays 100% in pure Rust** if you enable `symphonia` features `["aac", "isomp4"]`!), whereas `symphonia 0.5.5` does **not** have a built-in WebM/Matroska demuxer or Opus decoder!
   - **Recommended Playback Path in `crates/sf-player`**:
     - Option A (Pure Rust, zero external binaries): Enable `symphonia = { version = "0.5.5", features = ["aac", "isomp4"] }` for `itag 140` (or add `matroska`/`webm` + `opus`/`audiopus` if Opus `itag 251` is desired), decode frames into `i16`/`f32` PCM via a `reqwest` byte-range streaming `Read + Seek` source (or chunked buffered reader), and push samples into the existing `fastframe-audio` / `RodioSink` pipeline (retaining `AudioTap` visualizer levels, `SharedEq`, limiter, volume ramp, speed, and crossfade!).
     - Option B: Pipe HTTP/stream via `ffmpeg` if installed as fallback for Opus/WebM.
     - Clearly prefer `audio/mp4` (`itag 140`) when decoding with `symphonia` `isomp4 + aac`, or implement WebM+Opus explicitly so `itag 251` is never handed to a decoder that cannot parse WebM!
3. **Hybrid `Core` / `Actor` Routing (`crates/sf-player/src/state/`)**:
   - Update `Actor` / `Core` so `PlayItems`, `PlayUris`, `PlayContext`, and `AddToQueue` containing `youtube:track:` URIs do **not** require a `librespot` session or Spotify Premium; route them to the local YouTube HTTP audio engine while keeping Spotify tracks on `librespot`, and preserve unified `PlayerSnapshot` + `Player` QML bridge properties (`hasTrack`, `playing`, `buffering`, `positionMs`, `durationMs`, `level`, `codec`, `queue`, `coverFile`).

---

## 5. UI Design Preservation Rules & Exact QML Touchpoints

### 5.1 Design Rules (Non-Negotiable)
- **Preserve Material 3 Expressive (`src/materialfy/qml/Md3/`)**:
  - Do **not** alter `Main.qml` layout geometry, `Sidebar.qml` rail/expanded widths (`80px` / `264px`), `Theme.qml` OKLab dynamic palette tokens, `GoogleSans.ttf`, `MaterialSymbolsRounded.ttf`, GLSL shaders, `MiniPlayer`, `NowPlaying`, `FloatingIsland`, or `TaskbarPlayer`.
  - Reuse `TrackListPage.qml`, `TrackRow.qml`, `LibraryRow.qml`, `MediaTile.qml`, `SettingsCard.qml`, `SettingRow.qml`, `SettingsBanner.qml`, `Snackbar`, `Dialog`/`Popup` styles (`NewPlaylistDialog.qml`, `TrackInfoDialog.qml`).
  - Wrap all new user-facing text in `I18n.t(...)` / `mf_i18n::t(...)`.

### 5.2 Exact QML Touchpoints Requiring Updates
Direct inspection found these exact QML lines that currently assume Spotify-only auth or `spotify:` URIs:

1. **Auth / Signed-In Gating (`Auth.state === "signedIn"` checks)**:
   - `HomePage.qml:15` (`readonly property bool signedIn: Auth.state === "signedIn"`)
   - `SearchPage.qml:23` (`readonly property bool signedIn: Auth.state === "signedIn"`)
   - `LibraryPage.qml:25` (`readonly property bool signedIn: Auth.state === "signedIn"`)
   - `LikedPage.qml:11,14,28`
   - `Sidebar.qml:16` (`readonly property bool signedIn: Auth.state === "signedIn"`)
   - `ReleasesPage.qml:94`
   - `MiniMode.qml:146,215`
   - **Fix**: Add QML properties on `Auth` bridge (e.g., `youtubeState`, `youtubeAccount`, `hasActiveSource`, or allow unauthenticated YouTube Search/Home/Playback while gating private Library/Playlists behind `signedIn || youtubeSignedIn`).

2. **Hardcoded `spotify:` URI Construction**:
   - `Main.qml:644` (`"spotify:track:" + track.trackId`) -> use `track.uri || ("spotify:track:" + track.trackId)`
   - `Main.qml:648` (`"spotify:artist:" + artistId`) -> use `ref.uri` or prefix matching `track.uri` (`youtube:artist:` vs `spotify:artist:`)
   - `Main.qml:657` (`"spotify:album:" + (track.albumId ?? "")`) -> derive prefix from `track.uri`
   - `TrackMenu.qml:127` (`"spotify:album:" + track.albumId`) -> derive scheme from `track.uri`
   - `ArtistPage.qml:305,339` -> ensure `slot.model.uri` is always set for YouTube albums (`youtube:album:...`)
   - `PlaylistPage.qml:34` -> ensure `info.uri` is set (`youtube:playlist:...`), avoiding `"spotify:playlist:"` fallback for YT IDs
   - `MiniPlayer.qml:380-382` (`_dropToQueue`) -> extend regex to also match `youtube.com` / `music.youtube.com` / `youtu.be` / `youtube:` URLs/URIs!
   - `Catalog.openUri` (`crates/materialfy/src/bridges/catalog.rs`): support `youtube:` URIs and `https://music.youtube.com/...` / `https://www.youtube.com/...` links alongside Spotify links!

3. **YouTube Account, Import & Provider UI Additions (Styled with Existing MD3 Components)**:
   - `SettingsPage.qml` (`accountCard` / `Account` section): add a YouTube Music `SettingsCard` matching the existing Spotify sign-in rows (`SettingRow`, `Button`, `SettingsBanner`):
     - Sign in with browser (opens Edge/Chrome CDP login, shows waiting/error state cleanly)
     - Optional manual cookie paste dialog (fallback when Chrome/Edge isn't present)
     - Connected user display (`name`, `email`, avatar via `Avatar.qml`) + `Sign out` button
     - YouTube audio quality / data saver toggle in `Playback` section
   - `AccountPage.qml`: support showing YouTube Music connection alongside Spotify or signing in when Spotify isn't used.
   - `LibraryPage.qml` / `Sidebar.qml`: add an unobtrusive `"Import playlist"` action (using `NewPlaylistDialog.qml` MD3 modal pattern) wired to `PlaylistImport` (YouTube URL, Spotify public URL -> YT match, M3U, JSON) with progress feedback!

---

## 6. Recommended Step-by-Step Execution Order for Claude

1. **Phase 1 — Auth & Credentials (`sf-auth`)**:
   - Add `sha1` & `tokio-tungstenite` (`0.28.0`) to `crates/sf-auth/Cargo.toml` and root `[workspace.dependencies]`.
   - Extend `Slot` / `Grant` / `generations` in `crates/sf-auth/src/credentials.rs`.
   - Implement `crates/sf-auth/src/youtube.rs` (`YouTubeAccount`, cookie parser, `compute_sapisid_hash`, CDP browser login, manual cookie fallback) + unit tests (`cargo test -p sf-auth`).
2. **Phase 2 — InnerTube Core Client, Parsers & Fixtures (`sf-services`)**:
   - Fix `fixedColumns` renderer names in `tests/fixtures/youtube/{playlist_tracks_page1,playlist_tracks_page2,search_response}.json` (and support both in parsers defensively).
   - Implement `crates/sf-services/src/youtube/` (`client.rs`, `models.rs`, `parsers.rs`, `error.rs`, `library`, `playlist`, `search`, `browse`, `catalog`, `mutations`, `import`).
   - Write comprehensive hermetic unit & integration tests (against `tests/fixtures/youtube/` + in-process `tokio::net::TcpListener` mock HTTP server) and run `cargo test -p sf-services`.
3. **Phase 3 — Stream Resolution & Audio Playback (`sf-services` + `sf-player`)**:
   - Implement `StreamResolver` (`ANDROID_VR_1_61_48` + `WEB_REMIX` + TTL cache + single-flighting + cooldowns + preheat).
   - Add `symphonia` (`isomp4`, `aac`, plus WebM/Opus or fallback) HTTP audio decoder in `crates/sf-player` feeding `RodioSink` / `fastframe-audio`.
   - Update `sf-player::state::{core, actor}` to route `youtube:track:` URIs locally alongside `spotify:` tracks.
   - Run `cargo test -p sf-player -p md3-model`.
4. **Phase 4 — Service Wiring & CXX-Qt Bridges (`sf-services` + `materialfy`)**:
   - Wire `YouTubeService` into `Services`, `AuthService`, `LibraryService`, `CatalogService`, `HomeService`, `ReleasesService`, `WaveService`, `HistoryService`, `LyricsService`.
   - Update `crates/materialfy/src/bridges/{auth,library,catalog,home,player,radio,releases,lyrics}.rs` (and `cpp/register.cpp` if adding new properties/signals).
5. **Phase 5 — QML Integration & UI Preservation (`src/materialfy/qml/`)**:
   - Update `Auth.state` guards, hardcoded `spotify:` URI builders, `MiniPlayer.qml` drop handler, `SettingsPage.qml`, `AccountPage.qml`, `Sidebar.qml`, `LibraryPage.qml`, `PlaylistPage.qml`, `SearchPage.qml`, `HomePage.qml`.
   - Keep all MD3 visual tokens, layouts, animations, and i18n intact.
6. **Phase 6 — Full Build & Verification**:
   - Pure-Rust crates (works even without Qt on PATH):
     ```powershell
     cargo test -p md3-model -p sf-auth -p sf-services -p sf-player
     ```
   - Full workspace + GUI binary (requires Qt 6.12 MSVC on PATH, e.g. `C:\Qt\6.12.0\msvc2022_64`, see `.github/scripts/install_qt.py` and `build_windows.ps1`):
     ```powershell
     cargo test --locked --workspace
     cargo build --locked --release -p materialfy
     qmltestrunner -input tests/qml/tst_imagegate.qml
     ```
   - Manual interactive smoke check: launch built `materialfy.exe`, test YouTube CDP sign-in (or manual cookie import), browse Liked Music (`"LM"`) & playlists, paginate a >100-track playlist, edit a playlist, search, play a track (`mini` + `full` + `NowPlaying` + OKLab theme extraction), trigger radio autoplay, and import a playlist URL.

