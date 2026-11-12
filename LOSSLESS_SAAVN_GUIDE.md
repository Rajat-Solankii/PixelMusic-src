# Lossless and JioSaavn sources: port guide

This guide covers porting soundwave1's lossless sources and JioSaavn into PixelMusicDesktop. The Kotlin project is at `C:\Users\rajat\AndroidStudioProjects\soundwave1`. It sits alongside `CLAUDE_YOUTUBE_IMPLEMENTATION_GUIDE.md` and `YOUTUBE_HANDOFF.md`.

**Status (2026-11-12):** the two sources that need no account are done end to end: Monochrome (FLAC) and JioSaavn (AAC). That covers the Rust back end, the Settings switches, the Monochrome check, the source in the quality chip, and JioSaavn songs in search. The UI part is not built or tried, because Qt is not installed here. The account-based sources (Tidal, Qobuz, Deezer, Apple Music) and the Source Pool are not ported. They are listed below as later steps.

## 1. What the Kotlin app has

### Android: `app/src/main/java/com/unshoo/pixelmusic/data/lossless/`

| File / package | What it does |
|---|---|
| `LosslessSources.kt` | Wiring at startup. It pushes the Apple Music tokens and the Deezer ARL into their providers, loads and refreshes the Source Pool, runs a Tidal instance health scan, restores the Qobuz probe track, and schedules `SourceRefreshWorker` every 6 hours. |
| `LosslessStreamResolver.kt` | The resolution chain, ported from ArchiveTune's `MusicService`. It has a per-media mutex (single-flight) and a direct-stream cache keyed `source:mediaId`. It checks a per-song source override, then walks the chain in the user's order (`AudioSourceConfig.DEFAULT_ORDER` = MONOCHROME, TIDAL, QOBUZ, QOBUZ_BACKUP, DEEZER, APPLE, JIOSAAVN, YOUTUBE). Every stream passes the `TitleMatch` gate. The first accepted stream wins. It skips everything when low-data mode is on. It owns the Monochrome cooldown (`monochromeDownUntilMs`, 5 minutes). |
| `SourceCheckService.kt` | Per-source health checks for Settings. Monochrome is checked with `GET /search/tracks?q=test&limit=1`: 2xx marks it up, 5xx or an exception marks it down. |
| `audiosource/AudioSourceConfig.kt` | `DirectStream`, the **`TitleMatch` gate** (Jaro-Winkler, version markers, 15 s duration gate, weighted score ≥ 0.78), the chain order/enable parsing, and the per-song overrides. |
| `audiosource/TrackMatching.kt` | Token-overlap scorer (min 60) used by the Tidal/Qobuz searches. |
| `audiosource/FlacStreamInfo.kt` | Reads STREAMINFO for the sample rate and bit depth. |
| `pool/` | **Source Pool.** `PoolAccountManager` downloads *shared Tidal/Qobuz/Deezer/Apple/Amazon accounts* from a remote server (`<url>/api/accounts`, key from `BuildConfig.SOURCE_PROVIDER_KEY` or Settings). It caches them encrypted (`PoolCacheCrypto`, `PoolCrypto`), rotates them with cooldowns, and reports dead ones back. |
| `tidal/` | Tidal: account login and refresh (`TidalAccountManager`), pool accounts, **and public proxy "instances"** that are health-ranked (`TidalInstanceHealthManager`, `TidalDns`). It answers `/trackManifests/` or `/track/` with a BTS JSON or DASH MPD manifest; `TidalProgressiveDashDataSource` stitches the DASH segments. |
| `qobuz/` | Qobuz: user tokens (JSON) or pool accounts, MD5-signed `track/getFileUrl`, and public instances. |
| `deezer/` | Deezer: ARL cookie (manual or pool). Streams are Blowfish-encrypted per track (`DeezerCrypto`, `DeezerDecryptingDataSource` behind a `deezer://` scheme). |
| `applemusic/` | Apple Music: developer token (scraped) plus Media-User-Token. Uses Widevine L3 (`AppleMusicDrm`) and a virtual stream for the decrypted fMP4. |
| `playback/LosslessSchemeRoutingDataSource.kt` | Routes the `monochrome://`, `deezer://` and Apple schemes to their data sources. |

### Monochrome, the free server: `tracks.monochrome.st`

- **Search:** `GET https://tracks.monochrome.st/search/tracks?q=<first artist> <title>&limit=5`, sent with a Chrome 128 User-Agent. The answer is `{"tracks":[{id|trackId, title, artistNames[], albumTitle, duration, playable}]}`. `duration` is in milliseconds, or in seconds when it is ≤ 10 000. Hits with `playable:false` are skipped.
- **Stream:** `GET https://tracks.monochrome.st/track/<id>` returns a plain FLAC file (16-bit / 44.1 kHz). Android also accepts `monochrome://<id>` and rewrites it to that URL.
- **Match:** each hit is checked with `TitleMatch.evaluate`; the best accepted score wins.
- **Enabled / down-until:** the setting `MonochromeEnabledKey` defaults to true, but it only matters when lossless streaming itself is enabled, which defaults to false. `markMonochromeDown(5 min)` is called on any 5xx and on any `IOException`. `markMonochromeUp()` is called on any 2xx. `isMonochromeUp()` is `now >= downUntil`.

### Desktop: `desktopApp/.../data/lossless/`

- **`LosslessProvider.kt`:** the `LosslessAudioSource` interface plus `TidalLosslessProvider` (Bearer token, `playbackinfopostpaywall`, base64 manifest as JSON urls[] or MPD BaseURL) and `QobuzLosslessProvider` (app_id, secret and user token; signature `md5("trackgetFileUrlformat_id{f}intentstreamtrack_id{id}{ts}{secret}")`; formats 27/7/6). `LosslessRouter` tries Tidal, then Qobuz, and only when the quality setting is not STANDARD. It also has its own `TrackMatching`, which prefers an ISRC match and otherwise uses token overlap. The desktop app has **no Monochrome**.
- **`LosslessKeyStore.kt`:** a local credentials store for the Tidal token and the Qobuz app id, secret and token.

### `SaavnService.kt`, in both apps (`https://www.jiosaavn.com/api.php`)

- **Common parameters:** `_format=json&_marker=0&api_version=4&ctx=android`.
  - Search uses `__call=search.getResults&q=..&p=1&n=30`.
  - Details use `__call=song.getDetails&pids=<id>`; the song is in `songs[0]` or under the id key.
- **Headers:** `Accept: application/json`, a Chrome 134 User-Agent, `Accept-Language: en-IN`, `Cookie: explicit_content=1`. Android also sends `X-Forwarded-For` / `X-Real-IP: 49.36.0.1`, which spoofs an Indian IP. The port leaves these out.
- **Media URL:** `more_info.encrypted_media_url` is base64 over **DES-ECB**, with key `38346591` and PKCS#5 padding. The URL's `_96.mp4` suffix picks the bitrate: `_96`, `_160` or `_320`. The stream is **AAC-LC in MP4**.
- **Android `resolveStream`:** cleans the title (removes "(Official Video)", "[Lyrics]" and "ft. …"). The primary query is `album title artists`; the fallback query is `title artists`. Candidates are sorted with an exact album match first, then the same explicit flag. The first candidate wins if it passes these checks:
  - it is not Pro-only (`rights.code == "1"` or reason contains "Pro Only");
  - the cleaned titles contain each other;
  - an artist is shared or contained;
  - the duration is within 12 s.

  The quality is 320 kbps, 160 kbps for the medium plan, and 96 kbps for the low plan. The whole step has a 3 s timeout.
- **Desktop app:** JioSaavn is used only for `saavn:<id>` tracks from search; it does no matching. It uses 320 kbps, or 96 kbps with the data saver, and upgrades `http://` to `https://`.

### How the apps choose a stream

- **Android** (`YoutubeHelper.getSongPlayerUrl*`): local file → `LosslessStreamResolver.resolve` (skipped on the LOW/MEDIUM plan) → JioSaavn when `isSaavnEnabled` (3 s timeout) → YouTube.
- **Desktop** (`StreamResolver.doResolve`): a Jam host's direct URL → `LosslessRouter` (if quality ≠ STANDARD and a provider is configured) → JioSaavn for `saavn:` ids → YouTube (signed-in WEB_REMIX / ANDROID_VR / NewPipe). `PlayerController` only displays `isLossless` / `isHiRes`.

### Accounts, keys and formats

| Source | Account / key | Audio |
|---|---|---|
| **Monochrome** | none | FLAC 16/44.1 |
| **JioSaavn** | none (India-centric catalogue) | AAC-LC in MP4: 96, 160 or 320 kbps |
| Tidal | user account (OAuth token), Source Pool account, or a third-party public instance | FLAC 16/44.1, up to 24/192 (BTS JSON or DASH) |
| Qobuz | app id + app secret + user auth token (paid subscription) | FLAC: 6 = 16/44.1, 7 = 24/96, 27 = 24/192 |
| Deezer | ARL cookie (premium for FLAC) | FLAC or MP3 320, Blowfish-encrypted per track |
| Apple Music | developer token + Media-User-Token (subscription) | ALAC / AAC, Widevine-protected |
| Amazon | (pool only) | CENC fMP4; no resolver even on Android |

## 2. What was ported (Rust)

All of this work is uncommitted on `main`.

- **`Cargo.toml`:** symphonia now has the `flac` feature. It also adds `des = "0.8.1"`, which uses cipher 0.4 like the existing `ecb`.
- **`crates/sf-player/src/engine/youtube/mod.rs`:**
  - `StreamRequest` holds the video id, title, artists, album, duration and explicit flag; `StreamRequest::from_track(&TrackItem)` builds one.
  - `StreamSource` gains `resolve_track` and `prefetch_track`. Their defaults fall back to `resolve` / `prefetch(video_id)`, so `YouTubeService` is unchanged.
  - `YtStream` gains `sample_rate`, `bit_depth` and `is_lossless()`. `codec_label()` returns "FLAC 16/44.1".
- **`engine/youtube/decode.rs`:** `Decoder::open(source, mime)` hints the probe with `flac` or `m4a` and the mime. `Decoder::new` is `open(.., "audio/mp4")`. `player.rs` opens with `stream.mime`.
- **`state/youtube.rs` and `state/actor.rs`:** the lane's `YtEffect::Resolve { seq, request }` and `Prefetch(StreamRequest)` carry the track metadata. The actor calls `resolve_track` and `prefetch_track`. A retry after a player error rebuilds the request from the current track.
- **`crates/sf-services/src/sources/`:**
  - `matching.rs`: a port of `TitleMatch` (normalize with NFD and accent stripping, Jaro-Winkler, segments, version markers, artist parts, and the weighted score).
  - `monochrome.rs`: search, parse, match, and the `/track/<id>` FLAC stream. Has a `check()`. Times out after 8 s.
  - `saavn.rs`: DES URL decryption, `quality_url`, `clean_title`, `parse_song` (HTML entities unescaped), `best_match` (the Android rules), primary and fallback queries, and the `song.getDetails` fallback. Times out after 3 s.
  - `mod.rs`:
    - `Cooldown`: a down-until state, with `REST` of 5 minutes. A 5xx or a network error rests the source; an answer marks it up; a parse error does not count. This is applied to JioSaavn too, which the Kotlin app does not do.
    - `SourceError`.
    - **`SourceChain`**, an implementation of `StreamSource`. It tries Monochrome (only when lossless is on and the data saver is off), then JioSaavn (96 kbps with the data saver, 320 otherwise), then YouTube.
    - A found stream is cached for 30 minutes. Concurrent resolves and prefetches share one search.
    - `invalidate(video)` on a stream from another catalog sends that track to YouTube for 30 minutes. `clear()` forgets the cache and that list.
- **`crates/sf-services/src/lib.rs`:** `Services` wraps `YouTubeService` in `SourceChain` before calling `player.enable_youtube`. `follow_youtube_data_saver` also applies the new settings and clears the chain on every change.
- **`crates/mf-platform/src/settings.rs`:** new keys `LosslessStreams` (`losslessStreams`) and `JioSaavnStreams` (`jioSaavnStreams`). Both are bool and default to false, matching Android's defaults. Both apply on the fly.
- **Tests (hermetic):**
  - `sources::matching::tests` (7) and `sources::tests` (24) use an in-process `TcpListener` server and the fixtures in `tests/fixtures/lossless/`:
    - `monochrome_search.json` and `saavn_*.json`, written by hand in the real API shapes;
    - `tone_44k_stereo.flac`, 1 s at 440 Hz, made with ffmpeg.
  - The DES vectors were produced by Java's `DES/ECB/PKCS5Padding`, the same cipher the Kotlin app uses.
  - An end-to-end test resolves through the chain, downloads with the player's ranged downloader and decodes the FLAC.
  - The `sf-player` decoder has tests for FLAC by mime, for seeking, and for the case where the mime is only a hint.
  - None of the tests call the real servers.

### Second batch: UI and JioSaavn as a catalogue

- **Settings** (`crates/materialfy/src/bridges/settings.rs`, `SettingsPage.qml` › Playback, under "YouTube Music data saver"):
  - Two switches: `Settings.losslessStreams` and `Settings.jioSaavnStreams`.
  - A "Monochrome server" row with a **Check** button, shown while lossless is on. The button calls `Settings.checkMonochrome()`, which runs `streams.monochrome().check()` and updates `Settings.monochromeCheck`: `""` (not checked), `"checking"`, `"ok"` or `"down"`.
  - All strings go through `I18n.t`.
- **Quality chip:**
  - The lane's `PlayerSnapshot.codec` is now `YtStream::quality_label()`. Monochrome and JioSaavn streams show their catalogue first, for example "Monochrome · FLAC 16/44.1" or "JioSaavn · AAC 320". YouTube streams still show "AAC 129".
  - `CodecBadge.qml` sizes itself to the text, so no QML change was needed there.
- **`saavn:track:<id>` URIs** (`sf-player` `engine/youtube/mod.rs`):
  - `SAAVN_TRACK`, `saavn_id` and `is_saavn_track` are new.
  - `lane_id` passes the whole URI for these, as it does for local files, so `is_lane_track` routes them to the YouTube lane.
  - `lane_track_id` gives a URI-only item its `track_id`. It replaces the two `video_id().or_else(local_id)` call sites.
- **Resolution:**
  - `SourceChain` answers a `saavn:track:` request from `Saavn::stream_of`, which uses `song.getDetails` at 320 kbps, or 96 kbps with the data saver. This happens whatever the settings say and ignores the cooldown, because the listener picked the song.
  - The stream is cached for 30 minutes. `invalidate` forgets it, and the retry asks again; these requests never fall back to YouTube.
  - Errors come back as translated messages.
  - `details` now accepts only the song id it asked for. Before, an answer missing that id fell back to its first song, which could play the wrong track; a test caught this.
- **Search:**
  - `Saavn::search_tracks` gives `TrackItem`s: `saavn:track:<id>`, a 150px cover and a 500px large cover, and Pro-only songs marked unavailable.
  - The catalog bridge's `search()` runs it alongside the Spotify search, for "all" and "tracks" while `JioSaavnStreams` is on.
  - `sources::with_saavn_tracks` appends its songs after the catalogue's tracks, or shows them alone if Spotify failed.
  - JioSaavn songs have no album, artist, like or radio pages.
- **`Services.streams`:** the `SourceChain` is now public for the bridges.
- **Tests:** 4 new `sources::tests` and 2 new `sf-player` tests.

## 3. Not done: next steps

1. **Try the UI in the app.** Qt is not installed here, so `materialfy` was never built. Check the cxx-qt property and invokable code, the two switches, the Check button and the search merge in a real build.
2. **JioSaavn polish:** a "JioSaavn" label on its rows in search, plus album and artist pages (`__call=content.getAlbumDetails` / `artist.getArtistPageDetails`). Library likes and history for `saavn:` items are not routed anywhere yet.
3. **Qobuz** (account): port the desktop `QobuzLosslessProvider`. It needs a key store, which fits in sf-auth credentials beside the YouTube account. Signing needs `md-5`, which is already in the workspace. The streams are FLAC, so decoding already works.
4. **Tidal** (account): start with the desktop provider (Bearer token, `playbackinfopostpaywall`, BTS JSON manifest). Port DASH segment stitching only if it is needed. Leave out the public "instances": they are unofficial third-party proxies.
5. **Deezer** (ARL): needs Blowfish CBC (`blowfish` crate) and a decrypting `MediaSource` in front of the downloader, with a 2048-byte stripe on every third chunk.
6. **Apple Music:** Widevine L3 is not feasible or appropriate to port; skip it.
7. **Desktop extras:** ISRC-first matching from the desktop `TrackMatching`, which would need an ISRC in `StreamRequest`, and per-song source overrides (`SongSourceOverride`).
8. **Deliberately left out:**
   - **The Source Pool** downloads other people's shared paid-service accounts from a remote server using a baked-in key, and reports accounts back to it. It is not ported, and the pool-only Amazon path goes with it.
   - **Telegram admin, telemetry, device tracking and remote block:** per instructions, none of these are ported. The lossless packages do not depend on any of them; no Telegram or bot code was found there.
