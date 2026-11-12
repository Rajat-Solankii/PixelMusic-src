# YouTube Music port — handoff notes

Working notes for whoever continues the port described in `CLAUDE_YOUTUBE_IMPLEMENTATION_GUIDE.md`
(reference project: `C:\Users\rajat\AndroidStudioProjects\soundwave1`). Read the guide first; this file
records what is done, what was decided, and what is left. Nothing is committed yet (all changes are in the
working tree on `main`; branch before committing).

## Environment

- Rust 1.99.1 (stable MSVC) is installed at `%USERPROFILE%\.cargo\bin`. A shell opened before the install
  does not have it on PATH; in PowerShell prefix commands with
  `$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path";`.
- Visual Studio Community 2026 has the "Desktop development with C++" workload and the Windows 11 SDK.
- Qt 6.12 is **not** installed, so the `materialfy` binary crate cannot be built here. Verify with the
  pure-Rust crates: `cargo test -p md3-model -p sf-auth -p sf-api -p sf-services -p sf-player`.
- A first build of `sf-services` takes several minutes (librespot).

## Phase 1 — done and verified

`cargo test -p sf-auth`: 48 passed, 0 failed. `sf-api`, `sf-player`, `sf-services` compile against it.

- `Cargo.toml`: `tokio-tungstenite = "0.28"` in `[workspace.dependencies]`.
- `crates/sf-auth/Cargo.toml`: `sha1`, `futures`, `tokio-tungstenite`.
- `crates/sf-auth/src/credentials.rs`: `Slot::YouTube` (serde name `youtube`), `Slot::COUNT = 5`,
  `Grant::YouTube(YouTubeAccount)`; `Slot::SPOTIFY` unchanged, so a Spotify sign-out keeps the YouTube
  session and the reverse.
- `crates/sf-auth/src/youtube.rs` (new): `YouTubeAccount` (camelCase serde, redacting `Debug`),
  `parse_cookies`, `compute_sapisid_hash`, `authorization_header`, `YouTubeAccount::authorization(origin)`,
  `normalize_data_sync_id`, `browser_path`, `sign_in(profile_dir)` (launches Edge/Chrome, polls the DevTools
  port, reads cookies + `DATASYNC_ID` + `VISITOR_DATA`, kills the browser tree on drop),
  `YouTubeAccount::from_cookie_string` (manual paste), `remove_profile`, `YouTubeAuthError`.

Decisions and open points:

- `sign_in` does **not** fill `name` / `email` / `avatar`; call `account/account_menu` through the InnerTube
  client afterwards (Phase 2 `YouTubeService::account_info`).
- `DATASYNC_ID` keeps the part **before** `||` (desktop `YouTubeAuth.kt:189`). Android's
  `PlaybackAuthState.kt:111-116` takes the part after when the value does not end in `||`. Revisit with a
  brand account if authenticated calls fail.
- `sign_in` has never been run against a real browser (nothing calls it until Phase 4).
- No `ru` / `pt` translations yet for the new strings (missing translations fall back to English).

## Phase 2 — done and verified

`cargo test -p sf-auth -p sf-api -p sf-services`: sf-auth 48 passed (1 ignored, as before), sf-api 79 passed,
sf-services 224 passed (38 of them YouTube) + 5 offline; `cargo check -p sf-player` clean; no warnings.
Nothing has been run against the real YouTube Music or Spotify servers.

Files (all in `crates/sf-services/src/youtube/` unless noted):

- `error.rs`, `client.rs`, `models.rs`, `parsers.rs` — as described before; they compiled after one fix
  (`AlbumItem.version` was missing in `album_page`). Parsers are pure functions over `serde_json::Value`.
- `mod.rs` — `YouTubeService { client: InnerTube, home cache }`, `new(sf_api::Http)`, `with_client(InnerTube)`,
  `client()`, `set_account(Option<YouTubeAccount>)` (also drops the home cache), `signed_in()`; private
  `browse` / `browse_continuation` (continuation token in the **body**); `PageGuard` (seen tokens, 50 pages,
  2 empty pages in a row). Re-exports the models, `InnerTube`, `Login`, `YouTubeError`, `Rating`,
  `ImportResult`, `ImportSource`, the library browse ids and `radio_playlist_id`.
- `library.rs` — `library(browse_id)`, `library_continuation`, `library_all` (guarded), `user_playlists`
  (synthetic "Liked Music" `LM` first, listed `LM` / `SE` dropped, duplicates dropped), `history()`;
  constants `LIKED_PLAYLISTS`, `LIKED_ALBUMS`, `LIBRARY_ARTISTS`, `HISTORY`, `LIKED_MUSIC`.
- `playlist.rs` — `playlist(id)` (adds `VL`; `LM` needs an account, others are `IfSignedIn`),
  `playlist_continuation`, `playlist_full(id, progress)` (guarded; keeps the continuation when the guard
  stopped early).
- `search.rs` — `search(query, filter)`, `search_continuation` (token in the **query**: `continuation` +
  `ctoken`), `search_summary`, `suggestions` (≤ 8 queries, anonymous), `search_songs`.
- `browse.rs` — `home(force_refresh)` (60 s cache, single-flight), `home_continuation`, `explore`,
  `new_releases`, `charts(country)` (anonymous, `formData.selectedValues`, fallback `params`), `moods`,
  `browse_shelves(&BrowseRef)`, `shelves_continuation`.
- `catalog.rs` — `album` (page, then the full album playlist; falls back to the page's songs), `artist`,
  `next(video, playlist, params, follow_automix)`, `next_continuation`, `radio` (`RDAMVM<id>`), `lyrics`,
  `lyrics_at`, `transcript` (WEB client + key), `account_info`.
- `mutations.rs` — `rate_track(Rating::{Like, Dislike, None})`, `like_playlist`, `subscribe`,
  `create_playlist(title, video_ids, public)`, `delete_playlist`, `rename_playlist`, `add_to_playlist`
  (several videos in one request; returns new set-video ids), `add_playlist_to_playlist`,
  `remove_from_playlist(&[(video_id, set_video_id)])`, `move_in_playlist`. `VL` is stripped everywhere
  (the Kotlin `moveSongPlaylist` did not). An edit answer with a `status` other than `STATUS_SUCCEEDED` is
  `Unexpected`; one without a status counts as done.
- `import.rs` — `detect`, `youtube_playlist_id`, `spotify_playlist_id`, `normalize`, `parse_m3u`,
  `YouTubeService::import_playlist(input, progress(done, total))` → `ImportResult { title, tracks, total,
  missed }`. Spotify: embed `__NEXT_DATA__`, then Web API pages with the embed's anonymous token past 100
  songs; `spotify.link` short links are followed. Matching: `title*2 + artist*1.5 + length >= 2.2`, 4 at a
  time, order kept, results de-duplicated. Error texts go through `mf_i18n::t`.
- Tests: `parser_tests.rs` (fixtures + inline shapes), `client_tests.rs` (in-process `TcpListener` server:
  signed-in headers and context, anonymous requests and the `sw.js_data` visitor id, ANDROID_VR never
  signed, 429/500 retried, 4-attempt limit, 401 → `SessionExpired` without retry, `NotSignedIn` before any
  request, multi-page playlist, repeated-token guard, search continuation query, edit bodies, home cache,
  YouTube-link and M3U imports), plus unit tests in `library`, `playlist`, `catalog`, `mutations`, `import`
  and `mod.rs`.
- `crates/sf-api/src/util.rs`: `uri_provider(uri)` → `Some("youtube")` / `Some("spotify")` / `None`, tested.
- `crates/sf-services/src/lib.rs`: `pub mod youtube;`, `pub use youtube::{YouTubeError, YouTubeService}`,
  `Services.youtube: Arc<YouTubeService>` built from the shared `sf_api::Http` (same proxy, live changes).
  Nothing calls it yet and no account is restored into it at startup — that is Phase 4.

Decisions and open points:

- Import differences from the Kotlin: local-file M3U/JSON entries are looked up by name instead of played
  (no local playback here); an M3U location that is a YouTube link (`watch?v=`, `youtu.be/`, `shorts/`) is
  used directly; JSON `duration` is read as ms when > 10 000, else seconds (the Kotlin code mixes both).
- `charts(country)` does not change the request's `gl` as the Kotlin code does (the context comes from the
  client's locale); only `formData` carries the country.
- `next` bodies carry only the fields given (`videoId`, `playlistId`, `params`, `continuation`), as
  `NextBody.kt` with null fields dropped; `playlistSetVideoId` / `index` are not supported yet.
- Not done in Phase 2 (as planned): chart playlist hydration, `registerPlayback`, `getMediaInfo`,
  `get_queue`, artist "items" continuation pages (use `browse_shelves` / `shelves_continuation`).
- Not verified: any real InnerTube or Spotify response. The parsers for album, artist, `next`, moods,
  home/explore shelves and lyrics have **no tests at all** (no fixtures exist for them); the account menu,
  transcript, edit result and Spotify embed parsers are tested only on hand-written shapes. Recording real
  responses into `tests/fixtures/youtube/` is the first thing to do before relying on them.
- Fixtures: `fixedColumns` in `playlist_tracks_page1/2.json` and `search_response.json` use the real
  `musicResponsiveListItemFixedColumnRenderer`; the parser still accepts the old spelling (tested).
- No `ru` / `pt` translations for the new strings yet.

## Phase 3 — done (steps 1–7)

Started and finished 2026-11-12. `cargo test -p sf-player -p md3-model -p sf-services -p sf-api -p sf-auth` all
green: sf-player 334, sf-services 235 + 5 offline, sf-api 79, sf-auth 48 (1 ignored), md3-model 5 + 13.
`cargo check` of those crates with `--all-targets` gives **no warnings**. Everything runs against fakes. Nothing
has been played on a real audio device or from real YouTube URLs, and `materialfy` has not been built (no Qt).

### Done so far

- Root `Cargo.toml`: `symphonia = { version = "0.5.5", default-features = false, features = ["aac", "isomp4"] }`;
  `crates/sf-player/Cargo.toml`: `symphonia.workspace = true`.
- Fixtures (ffmpeg): `tests/fixtures/youtube/tone_44k_stereo.m4a` (2 s, 440 Hz, 44.1 kHz stereo AAC) and
  `tone_48k_mono.m4a` (1 s, 1 kHz, 48 kHz mono AAC).
- `engine/mod.rs` declares `pub mod youtube;`.
- `engine/youtube/mod.rs`: `YtStream` (+ `codec_label()`), trait `StreamSource { resolve, prefetch, invalidate }`,
  `YtOutput { audio_device, buffer_ms, eq, normalisation }`, `video_id(uri)`, `is_youtube_track(uri)`, `BoxFuture`.
- `engine/youtube/download.rs` (step 1): `start(reqwest::Client, &Handle, DownloadSpec) -> StreamReader`;
  `DownloadSpec::new(url, user_agent, content_length)` (4 MiB `chunk_bytes`). Ranged chunks with the stream's
  User-Agent; total from `content_length` / `Content-Range` / 200 `Content-Length`; 416 past the start = end of file;
  403/410 -> `DownloadError::Expired` (no retry; reader gets `io::ErrorKind::PermissionDenied`); other failures
  retried 3 times (800 ms, 1.6 s, 3.2 s). `StreamReader`: `Read + Seek + MediaSource`, 30 s read timeout,
  `status() -> DownloadStatus { error, downloaded, total, finished }`; dropping the last reader aborts the task.
  4 tests against an in-process Range server (`download::tests::serve`, reusable via `pub(crate)`).
- `engine/youtube/decode.rs` (step 2): `Decoder::new(Box<dyn MediaSource>)`, `next_chunk() -> Result<Option<Vec<f64>>,
  String>` (stereo 44.1 kHz; mono doubled, >2 ch first two, other rates through `Resampler`), `seek(ms)` (accurate:
  drops frames before `required_ts`, resets codec + resampler), `position_ms()`, `duration_ms()`; up to 32 bad
  packets skipped. Note: symphonia's MP4 reader keeps the AAC priming/padding (~2k extra frames per file).
  4 tests (both fixtures, Goertzel tone check, seek accuracy within 30 ms).
- `engine/youtube/player.rs` (step 3): `YtPlayer::spawn(YtOutput, Opener, Notify)` builds `RodioSink` behind
  `Tapped` (volume via `MappedVolume` = librespot Cubic curve over an `AtomicU16`); `YtPlayer::with_sink(factory,
  normalise, opener, notify)` for tests. `send(YtCommand)`, `tap()` (level for cover breathing). Commands:
  `Load { seq, stream, position_ms, play }`, `Play`, `Pause`, `Seek(ms)`, `Stop`, `Volume(u16)`, `Shutdown`.
  `Notify = Arc<dyn Fn(u64 /*seq*/, YtEvent)>`; events `Loading`, `Playing{position_ms}`, `Paused{position_ms}`,
  `Position(ms)` (every 500 ms), `Ended`, `Error(String)`. **Every event carries the load's `seq`**: the lane must
  drop events whose seq is not the current load. Opening (download + probe + initial seek) runs on a helper
  thread; a superseded open is dropped. Skips/seeks call `AudioControl::interrupt()` + `track_changed()`.
  Normalisation: `normalisation_factor(loudness_db) = min(1, 10^(-db/20))` applied by the player and stored in
  the `Tapped` atomic. `http_opener(sf_api::Http, Handle) -> Opener` for production. 6 tests with a recording sink.
- sf-services `youtube/stream.rs` (step 4): `parse_player(&Value, &ClientPreset, data_saver) -> Result<YtStream,
  Refusal>` (pure; direct `audio/mp4` only; highest bitrate or nearest 64 kbps), `Refusal { status, reason }`
  + `is_bot_check()`, `valid_until(url, now)`. `YouTubeService::{resolve_stream, preheat_stream,
  invalidate_stream, set_data_saver}`; tiers ANDROID_VR (`Login::Never`) then WEB_REMIX (`Login::Required`,
  only when signed in); 15-min cooldowns (VR after a bot check / LOGIN_REQUIRED, WEB_REMIX after 2 failures);
  cache 200 entries until `expire - 600 s` (else 1 h); single-flight via `futures::Shared`; preheat
  `Range: bytes=0-1024`, 403/410 invalidates. `impl StreamSource for YouTubeService`. `YouTubeService.client`
  is now `Arc<InnerTube>` and it has `streams: Arc<stream::Streams>`. `YouTubeError::Unplayable(String)` added
  (Display: YouTube's reason, or `t("This song can't be played.")`). Tests: 6 unit (fixtures) + 5 mock-server
  tests at the end of `client_tests.rs` (single-flight + cache, bot-check cooldown, WEB_REMIX fallback,
  WEB_REMIX rest after 2 failures, preheat 403).

### What was learned about sf-player (why this design)

- librespot's `Spirc` owns the Spotify queue, and `state/core.rs` (3300 lines) merges local librespot state
  with Spotify Connect / Web API polling. Routing YouTube through `Core` would fight that logic, so YouTube
  gets **its own lane in the actor** and `Core` stays untouched.
- The output chain is reusable without librespot: `RodioSink::new(device, on_error, volume: Box<dyn
  VolumeGetter + Send>, buffer_ms, AudioControl::new(buffer_ms))` wrapped in `Tapped::new(sink, control,
  tap, volume, false, eq, normalisation: Arc<AtomicU64>)` (`engine/sink.rs`, `engine/vis.rs`). Both
  implement librespot's `Sink { start, stop, write(AudioPacket::Samples(Vec<f64>), &mut Converter) }`.
  Samples must be **interleaved stereo f64 at 44 100 Hz** (`librespot_playback::{SAMPLE_RATE,
  NUM_CHANNELS}`); `Converter::new(None)`. `write` blocks while the rodio queue is full, which paces the
  decoder.
- Volume: librespot uses `VolumeCtrl::Cubic(VolumeCtrl::DEFAULT_DB_RANGE)`; map with
  `librespot_playback::mixer::mappings::MappedCtrl::to_mapped(u16)` inside a small `VolumeGetter` over an
  `Arc<AtomicU16>`.
- `Tapped`'s `normalisation` atomic holds the factor (f64 bits) already applied to the samples so the level
  tap can undo it: apply `min(1, 10^(-loudness_db / 20))` yourself when normalisation is on and store it there.
- `engine/resample.rs`: `Resampler::new(from_hz, to_hz, channels) -> Option<Self>` (None if the rates are
  equal), stateful `process(&[f32]) -> Vec<f32>`; use it for 48 kHz sources.
- Level for the cover "breathing": the YouTube player owns its own `AudioTap::new()`; the actor reads
  `tap.level()` while the lane is active.
- Actor (`state/actor.rs`): `Msg::{Command, Engine, Api, Local, Cover, Restore, Shutdown}`; commands in
  `Command` (PlayItems, PlayUris, AddToQueue, TogglePlay, Next, Seek, SetVolume, ...); `build()` makes the
  `PlayerSnapshot`, `publish()` diffs it and emits `PlayerEvent`s; `Effect::Cover(url)` downloads the cover;
  `Effect::Started` / `Effect::Ended` feed history. Actor tests live in `state/tests.rs`.
- Kotlin reference for resolution: `soundwave1/desktopApp/.../data/StreamResolver.kt`; player request body:
  `{ context, videoId, playlistId? }` (`PlayerBody.kt`).

### Steps 5–7 (done 2026-11-12)

- `state/youtube.rs` (step 5): `YtLane`. It holds the context list (`items`), the play `order` (shuffled or not),
  `pos` (the position in `order` of the last context track played, so the context goes on at `pos + 1` even
  after "Playing next" songs), the manual queue, the current track and its load: `seq`, `stream`, playing,
  loading, position (extrapolated with an `Instant`), duration, start position, error, retried, started, and
  played time. Its methods change state only and queue `YtEffect`s: `Resolve { seq, video_id }`, `Prefetch`,
  `Invalidate`, `Engine(YtCommand)`, `Cover`, `Started` and `Ended`. It reuses Core's `TrackStartedLite` /
  `TrackEndedLite`.
  - Start a list with `play_items` (non-YouTube and unavailable rows are dropped), `shuffle_from` or `play_uris`.
    URI-only tracks get their title, artist, duration and cover from the `YtStream` (`fill_from`).
  - Transport: `toggle`, `play` (reloads after an end, a failure or a stop), `pause`, `seek`, `seek_relative`,
    `next` (manual songs first; at the end, repeat-all wraps and reshuffles a new round, otherwise the last
    track stays shown, stopped), `previous` (3 s restart rule), `play_row(uid)` / `play_index`, `set_repeat`,
    `set_shuffle` (the current track stays first), `add`, `insert_manual`, `move_manual`, `clear_manual`,
    `stop` (another source takes over).
  - `resolved(seq, result)` and `event(seq, YtEvent)` drop anything whose `seq` is not the current load, and
    anything that arrives while the lane is inactive.
  - The first `Playing` event of a load emits `Started`; leaving a started track emits `Ended`, which is
    natural on `YtEvent::Ended`. Repeat-one reloads the track at 0.
  - A player `Error` invalidates the URL and resolves again once, at the same position. A second error, or
    a resolve error, sets `error` and stops; it does **not** skip to the next track.
  - Row uids start at `1 << 48`, so they never collide with Core's.
  - `YtEngine` (`send`, `level`, `set_levels_wanted`) is the seam over `YtPlayer`. `YtLink { source, engine }`
    and `YtMsg { Enable, Resolved, Event }` are defined there too. 12 unit tests.
- `state/actor.rs` (step 6): `Msg::YouTube(YtMsg)`. `youtube_command` runs before Core:
  - `PlayItems`, `ShuffleFrom` or `PlayUris` whose chosen or first available item is `youtube:track:` → the lane,
    after `core.pause()`. Without `enable_youtube` the user gets a notice (`t("YouTube playback isn't available")`)
    instead.
  - `PlayContext` with a `youtube:` context URI is ignored with a debug log, because the services must expand it
    into tracks (Phase 4). `Remember` of a YouTube item is dropped.
  - Any other play, `PlayEpisode`, `TransferHere` or `TransferTo` stops the lane and then goes to Core.
  - While the lane is active, transport, queue, repeat and shuffle commands go to it, and `RefreshQueue` is a no-op.
  - Queues never mix. YouTube items are added only while the lane is active, Spotify items only while it is not;
    anything else gets `t("Spotify and YouTube songs can't share a queue")`.
  - `SetVolume` and `SetLevelsWanted` also reach the YouTube player (and Core, as before).
  - A librespot `State` with `Playing` and a **new** `track_sequence` (Connect or autoplay starting something
    here) stops the lane. A paused Spotify track that reports again does not.
  - Core's `Effect::Cover` is remembered in `core_cover` and shown only while the lane is inactive. The cover is
    restored when the lane stops. `cover()` was extracted from `run_effects`.
  - `build()` → `build_youtube()` while the lane is active. Track, playing, buffering, position, duration,
    queue, shuffle, repeat, context, codec (`YtStream::codec_label`), error, direction, played seconds and
    level come from the lane. Target is `Local` and `is_local` is true. The device is this computer
    (`local_device_name`, or `t("This computer")` when librespot is not up). Volume is `core.volume`.
  - `wait()` ticks at `FRAME` while the lane plays or loads. Shutdown stops the lane and drops the player.
- `handle.rs`: `PlayerHandle::enable_youtube(source: Arc<dyn StreamSource>, output: YtOutput, http: sf_api::Http)`.
  **Deviation from the plan:** it takes a third argument, `http`. `PlayerConfig.http` is the covers client and
  may bypass the proxy, so it must not carry the audio downloads. The method spawns `YtPlayer` with
  `http_opener(http, rt)` and a `Notify` that posts `Msg::YouTube(YtMsg::Event)`. On `Enable` the actor sends the
  current volume and the level gate.
- `sf-services/src/lib.rs`: `Services::start` calls `player.enable_youtube(youtube.clone(), output, http.clone())`
  right after `let youtube = auth.youtube();`. The output is `AudioDevice` + `Normalisation` from settings,
  read **once**. The default buffer and a fresh EQ are used. (Another session changed this function at the
  same time so that `auth` owns the YouTube service; the call sits after that line.)
- `state/tests.rs` (step 7): 6 actor tests in `mod actor` with `FakeSource` (a `StreamSource` where `"broken"`
  fails) and `FakeYt` (records `YtCommand`s). They cover: lane start pauses Spotify and resolve → load →
  `Playing` gives the snapshot, codec, device, queue rows, bridge model and `TrackStarted`; transport and volume
  reach the lane; a stale `Ended` is ignored; end of list; a Spotify play stops the lane with `TrackEnded`; a new
  librespot track stops the lane while the same one does not; notices for no player and for mixed queues;
  resolve-error text; retry after a player error with invalidate.

### Not verified / open after Phase 3

- No real playback: `YtPlayer::spawn` over `RodioSink`, `http_opener` downloads and the whole chain on a real
  device were never run. The actor tests use `FakeYt`, not `YtPlayer::with_sink`.
- Settings are not followed live: changing the output device or normalisation needs a restart for YouTube.
  The data-saver setting is still not wired (`set_data_saver` exists). The EQ is not shared with librespot's.
- A track that fails to resolve stops playback; it does not skip ahead (deliberately, so being offline does not
  run through the whole list).
- Shuffle only reorders the lane's own list. Turning shuffle off goes back to list order after the current track.
- Restoring a YouTube session at startup (Core's session file) is not done: the lane is not saved.
- Media keys and SMTC work through the snapshot, but were not checked with a YouTube track.
- No `ru` / `pt` translations for the new strings "YouTube playback isn't available", "Spotify and YouTube
  songs can't share a queue", "This computer" and "This song can't be played.".

## Phase 4 (services) — done and verified (Rust half; bridges not started)

Done 2026-11-12. `cargo test -p sf-services -p sf-api -p sf-auth -p md3-model`: sf-services 240 + 5 offline
+ 3 new (`tests/youtube_services.rs`), sf-api 79, sf-auth 48 (1 ignored), md3-model 5 + 13; `cargo test -p sf-player`
334; `cargo check -p sf-services --all-targets` has no warnings. All tests use an in-process HTTP server and
`Store::in_memory`; nothing was run against real YouTube, Spotify or the OS keyring, and `sign_in` (browser) has
still never been run.

Design: the app services reach YouTube through `AuthService`, which owns the one `Arc<YouTubeService>`
(`Services.youtube` is the same Arc). Each service has a `yt.rs` submodule (`auth/youtube.rs`, `library/yt.rs`,
`catalog/yt.rs`, `home/yt.rs`, `wave/yt.rs`); the existing public methods route there when the id or URI is a
YouTube one, and **no variant was added to `AuthEvent` or `LibraryEvent`** (the materialfy bridges match them
exhaustively). Spotify paths are unchanged.

### What the bridges should call

**`md3_model::PlaylistId`** — `from_uri_or_id("youtube:playlist:<id>")` now keeps the whole URI as the id;
`is_youtube()`, `youtube_id()` (bare `LM` / `PL…`), `uri()` returns it as is. **QML must pass the playlist URI
(or the `LibraryEntry.id`, which is the URI for YouTube rows), not the bare `playlistId`**, or a YouTube playlist
is taken for a Spotify one.

**Auth** (`services.auth`, separate snapshot and events from Spotify):
- `youtube_snapshot() -> YouTubeAuthSnapshot { state, account: Option<AccountInfo{name,email,avatar}>, error_text }`,
  `watch_youtube()`, `subscribe_youtube() -> YouTubeAuthEvent::{SignedIn(AccountInfo), SignedOut}`,
  `is_youtube_signed_in()`. `YouTubeAuthState::as_str()`: `signedOut | checking | awaitingBrowser | verifying |
  signedIn`.
- `start_youtube_login()` (Edge/Chrome window, profile `state_dir/youtube-signin-profile`),
  `cancel_youtube_login()`, `youtube_sign_in_with_cookies(&str) -> Result<(), String>` (manual paste; a header
  without SAPISID is refused at once), `youtube_sign_out()` (revokes `Slot::YouTube`, removes the browser
  profile; Spotify stays), `refresh_youtube_account()`, `restore_youtube()` (called by `start()`).
- A fresh sign-in is checked with `account_menu` before it is saved (name/email/avatar are saved with the
  cookies). A restored account is signed in at once from the saved card, then re-checked: 401 → signed out with
  the "session expired" text; network errors keep it signed in.
- `AuthService::with_youtube(.., youtube, profile_dir)` is the test constructor; `new` is unchanged.

**Library** (`services.library`):
- Routed by `PlaylistId::is_youtube()`: `load_playlist`, `refresh_playlist`, `cancel_playlist`, `replay_playlist`,
  `playlist_tracks`, `playlist_info`, `can_edit`, `add_to_playlist`, `add_tracks`, `insert_tracks` (YouTube
  appends only), `remove_tracks`, `move_tracks`, `rename_playlist`, `follow_playlist`, `unfollow_playlist`
  (someone else's: unlike; **own: `delete_playlist`**, as unfollowing an own playlist deletes it on Spotify);
  `set_description` / `set_public` / `upload_cover` give a notice ("can't be changed").
- Routed by `uri_provider == youtube`: `is_liked`, `toggle_like` (`like/like` / `like/removelike`, optimistic,
  reverted on failure), `dislike`, `is_saved`, `toggle_saved` (album → like its `OLAK5uy_` playlist, artist →
  subscribe, playlist → like). `create_playlist(title, Some(youtube track))` and `create_playlist_from(title,
  all-YouTube uris)` create a private YouTube playlist; `create_youtube_playlist(title, uris)` directly.
- Playlist pages emit the usual `PlaylistTracks { Loading(true), Reset(first page), Append(..)…, Loading(false) }`
  and `PlaylistInfo(id)`. Liked Music (`youtube:playlist:LM`): removing rows unlikes, adding likes.
- `entries(kind, owner, sort)` now also lists YouTube playlists (Liked Music first in YouTube's order), saved
  albums and artists (`id` = `uri` = `youtube:…`, subtitle "YouTube Music · N tracks"); the list changes come as
  `EntriesChanged` / `LikesChanged` / `SavedChanged`. Raw lists: `youtube_playlists()`, `youtube_albums()`,
  `youtube_artists()`, `youtube_loading()`, `refresh_youtube()` (also called by `refresh()` and on YouTube
  sign-in).
- Import: `import_playlist(input, save)` (YouTube link/id, Spotify link, M3U or JSON text; `save` also creates a
  private YouTube playlist, needs the account), `cancel_import()`, `import_progress()` / `watch_import() ->
  ImportProgress { running, done, total, title, tracks, missed, playlist_uri, error_text }`, plus a notice
  "Found N of M songs".

**Catalog** (`services.catalog`): `album("youtube:album:<id>")`, `artist("youtube:artist:<id>")` (shelves mapped
to popular / albums / also_albums / similar), `track("youtube:track:<id>")` (from the `next` endpoint) — they take
the **URI**; a bare id goes to Spotify. `track_details` on a YouTube URI returns an error text. New:
`search_youtube(query, SearchKind)` (All = the summary page; Tracks/Albums/Artists/Playlists = filters; Shows /
Episodes empty), `youtube_suggestions(input)`, `youtube_playlist(uri)` (header + first page, for a playlist
outside the library). `search()` is unchanged (Spotify only).

**Home** (`services.home`): `refresh_youtube(force)`, `more_youtube()` (continuation), `youtube_snapshot()` /
`watch_youtube() -> YouTubeHomeSnapshot { loading, shelves: Arc<Vec<HomeShelf>>, has_more, error_text }`.
Shelves: only songs → kind `tracks`, only artists → `artists`, else `tiles` (`LibraryEntry` with kind
track/album/artist/playlist); keys `youtube:<n>:<title>`. Reloaded on YouTube sign-in/out once it was shown.
Async: `youtube_explore()`, `youtube_charts(country)`, `youtube_moods() -> Vec<Mood>`, `youtube_browse(&BrowseRef)`.

**Releases**: `youtube_new_releases() -> Vec<AlbumItem>` (async). **History**: `youtube_history() ->
Vec<HomeShelf>` (async, `FEmusic_history` groups). Local history already records YouTube plays (Phase 3 effects).

**Radio** (`services.wave`): `play_station("youtube:track:<id>", title)`, `open(..)` and `refresh()` use the song's
radio (`RDAMVM<id>`); `station_uri` is `youtube:playlist:RDAMVM<id>`; playing goes through `player.play_items`
with the rows (context `radio`). `save()` makes a YouTube playlist (through `create_playlist_from`).

**Lyrics**: for a `youtube:track:` song the synced sources of mf-lyrics are tried by title/artist (no Spotify
call), then YouTube Music's own lyrics: `source` "YouTube Music", **`kind` "plain"** (untimed lines, `t` = `e` = 0)
— QML must show these as static text, not karaoke. `LyricsService::set_youtube` is called in `Services::start`.

### Files

`md3-model/src/ids.rs` (+ test); `sf-services/src/auth/youtube.rs` (new) + `auth.rs` (field, `with_youtube`,
`start`); `library/yt.rs` (new) + `library.rs` (fields, routing guards, `entries`); `catalog/yt.rs` (new) +
`catalog.rs`; `home/yt.rs` (new) + `home.rs`; `wave/yt.rs` (new) + `wave.rs`; `lyrics.rs`; `history.rs`,
`releases.rs` (one `impl` block each at the end); `lib.rs` (exports, `youtube = auth.youtube()`,
`lyrics.set_youtube`); `tests/youtube_services.rs` (new).

### Not done / open

- Bridges and QML (Phase 4 second bullet, Phase 5). New user-facing strings have no `ru` / `pt` translations:
  "YouTube Music", "YouTube Music · {0}", "Only YouTube Music songs can be added to a YouTube Music playlist.",
  "This can't be changed for a YouTube Music playlist.", "Playlist “{0}” created", "Imported playlist",
  "Found {0} of {1} songs", "Song details aren't available for YouTube Music songs.", "{0} Radio",
  "YouTube Music returned no songs for this radio.".
- Playback telemetry (`registerPlayback`, watchtime) is still not implemented; the YouTube history only reads.
- Search paging (`search_continuation`) and artist "see all" pages are not wrapped by the services (call
  `services.youtube` / `home.youtube_browse` directly).
- `WaveSnapshot.active` is set only for Spotify stations; a playing YouTube radio is not marked active yet.
- Removing rows needs the rows' `setVideoId`; the recorded fixtures have none, so remove/move were not tested
  end to end (they are refused when the id is missing).
- The parsers for album, artist, home, moods and the library lists still have no real-response fixtures (see
  Phase 2); the services were tested only with the playlist fixtures and hand-written account/edit answers.

## Phase 4 (bridges) — written, NOT compiled

Done 2026-11-12. **Qt is not installed, so `materialfy` (all of `crates/materialfy/`) has never been compiled
with these changes.** Everything in that crate below is unverified: expect a first `cargo build -p materialfy`
to need small fixes (cxx-qt attribute details, borrow of `Pin<&mut …Rust>`, `QStringList` from an iterator).
The pure-Rust parts it relies on were added to `sf-api` / `mf-platform` / `sf-services` and are tested.
No new QObject type was added, so **`cpp/register.cpp` is unchanged** (only new properties/signals/invokables on
existing singletons).

Verification at hand-over: `cargo test -p sf-player -p md3-model -p sf-api -p sf-auth -p mf-platform` all green, no
warnings (sf-player 350, sf-api 82, sf-auth 50 (1 ignored), mf-platform 27 (1 ignored), md3-model 5 + 13).
`cargo check -p mf-platform -p sf-services --all-targets` was clean right after the data-saver change, but at
hand-over **sf-services did not build because of other sessions' work in progress** (`mod transfer;` in `lib.rs`
without its file, and `src/local/tags.rs` using `symphonia` that is not a dependency of sf-services) — not from this
phase. Re-run `cargo test -p sf-services` once those land.

Pure-Rust (compiled and tested):
- `crates/sf-api/src/link.rs`: `parse_youtube(text) -> Option<String>` (`youtube:` URIs; music./www./m.youtube.com
  `watch?v=` (also with `&list=`), `shorts/`, `playlist?list=`, `browse/MPREb_…` album, `browse/VL…` playlist,
  `browse/UC…` + `channel/UC…` artist; `youtu.be/ID`; look-alike hosts refused), `youtube_page_uri(kind, id)`
  (bare `MPREb_…`/`OLAK5uy_…` album ids and 24-char `UC…` / `MPLA…` artist ids → `youtube:` URI; 22-char Spotify ids
  are left alone), `youtube_url(uri)` (music.youtube.com address for share/copy link). 3 new tests.
- `crates/mf-platform/src/settings.rs`: `SettingKey::YouTubeDataSaver` (`"youtubeDataSaver"`, default `false`);
  `ALL` is now 91.
- `crates/sf-services/src/lib.rs`: `follow_youtube_data_saver` applies the setting at start and on every change.
  (Another session has since extended this function with the source chain — lossless / JioSaavn settings.)

Bridges (`crates/materialfy/src/bridges/`, unverified):
- `auth.rs`: properties `youtubeState`, `youtubeSignedIn`, `youtubeError`, `youtubeAccount` ({name, email, avatar})
  with one NOTIFY `youtubeChanged`; signals `youtubeConnected` / `youtubeDisconnected` (from `subscribe_youtube`);
  invokables `startYouTubeLogin()`, `cancelYouTubeLogin()`, `youtubeSignInWithCookies(text) -> QString` (error text,
  "" when accepted), `youtubeSignOut()`, `refreshYouTubeAccount()`. Fed by `watch_youtube` (`apply_youtube`).
- `library.rs`: import — `importPlaylist(input, save)`, `cancelImport()`; properties `importRunning`, `importDone`,
  `importTotal`, `importTitle`, `importMissed` (QStringList), `importPlaylistUri`, `importError` (NOTIFY
  `importChanged`) and the CONSTANT model `importTracks` (Track rows, context `tracks`), fed by `watch_import`.
- `catalog.rs`: `openUri` tries `sf_api::link::parse_youtube` first → artist / album page, playlist page
  (`playlistId` = the `youtube:playlist:` URI), or a song: its album page with the song highlighted, or (no album)
  the song is played. `openAlbum` / `openArtist` / `album()` / `artist()` turn bare YouTube ids into `youtube:` URIs
  (`page_id`), so QML calls with `track.albumId` / artist ref ids work unchanged. YouTube pages load **without the
  Spotify sign-in** (`can_load`). `trackUrl` / `artistUrl` / `uriUrl` give music.youtube.com links for YouTube.
- `home.rs`: `youtubeShelves` (same shelf shape as `shelves`, NOTIFY `youtubeShelvesChanged`), `youtubeLoading`,
  `youtubeHasMore`, `youtubeError` (NOTIFY `youtubeChanged`), `refreshYouTube(force)`, `moreYouTube()`.
- `player.rs`: `playContext` / `shuffleContext` / `addUriToQueue` with a `youtube:` URI no longer reach the actor
  (which ignores YouTube contexts): `youtube_context` reads the songs (album → `catalog.album`; playlist → the
  library's cached rows, else `catalog.youtube_playlist` = **first page only**; track → `catalog.track`) and calls
  `play_items` (offset found by URI) / `shuffle_from` / `add_to_queue`.
- `settings.rs`: `Settings.youtubeDataSaver` (READ/WRITE/NOTIFY through the `settings!` macro).
- `radio.rs`, `lyrics.rs`, `releases.rs`: unchanged (radio already passes `youtube:track:` seeds through; lyrics
  pass `kind` through; YouTube new releases are not exposed — `releases.youtube_new_releases()` is unused).

## Phase 5 (QML) — written, NOT run

Nothing here has been loaded by a QML engine (no Qt): unverified, including syntax. All new text goes through
`I18n.t`; no `ru` / `pt` translations were added. Layout geometry, tokens, fonts, shaders and animations are
unchanged; only existing components are used. Icons are limited to names already in the subset font
(`assets/fonts/icons.json`): `library_music`, `edit`, `download`, `link`, `logout`, `open_in_new`, `refresh`, `cookie`.

- `app/pages/SettingsPage.qml`: new `youtubeCard` (SettingsCard right after the Spotify account card, same row
  layout): avatar / name / email, Sign in (browser) / Cancel / Sign out, error `SettingsBanner` with Retry, and a
  "Sign in with cookies" `SettingRow` that unfolds a password-style `TextField` (same unfold animation as the
  personal Client ID form) calling `Auth.youtubeSignInWithCookies`. Playback card: "YouTube Music data saver" switch.
  No new entry in `page.sections` (the card scrolls with the "account" section).
- `app/ImportPlaylistDialog.qml` (new, the `NewPlaylistDialog` pattern): link field, "Save as a YouTube Music
  playlist" switch (needs the YouTube account), `WavyProgress` + status ("Matching songs: {0} of {1}", "Found {0}
  of {1} songs", or the error), then Open (saved playlist) or Play (`Player.playFrom(Library.importTracks, 0)`).
  Only single-line input: **M3U / JSON text import is not reachable from the UI** (the service supports it).
- `Main.qml`: `ImportPlaylistDialog` instance + `win.openImportDialog()`; `win.uriOf(track, kind, id)` keeps a
  track's scheme for its album/artist; track radio uses `track.uri`, artist radio / "save album" use `uriOf`.
- `app/pages/LibraryPage.qml`: "Import" text button in the toolbar; the page is open with Spotify **or** YouTube
  signed in; the "Liked Songs" row only with Spotify.
- `app/Sidebar.qml`: `signedIn` = Spotify or YouTube (`spotifySignedIn` kept for the "+" new-playlist button); the
  account card shows the YouTube account when Spotify is signed out.
- `app/pages/HomePage.qml`: YouTube shelves appended after the Spotify ones (or alone) while YouTube is signed in;
  `Home.refreshYouTube(false)` on show / sign-in.
- `app/pages/AccountPage.qml`: "Use YouTube Music" / "YouTube Music connected" buttons → Settings, account section.
- `app/MiniPlayer.qml` `_dropToQueue`: also `youtube:` URIs, `watch?v=`, `shorts/`, `youtu.be/` and playlist links.
- `app/TrackMenu.qml`: "save album" checks the right scheme; no "Artist radio" for YouTube songs.
- `app/pages/PlaylistPage.qml`: a `youtube:` `playlistId` is used as the URI (no `spotify:playlist:` prefix).
- `app/LyricsView.qml`: `kind "plain"` (YouTube Music lyrics, untimed) → no active line, no auto-scroll, label
  "{0} · not synced".

### Not done / open after Phases 4–5

- Compile `materialfy` and load the QML (Phase 6) — the first real check of everything above.
- YouTube search in the UI (`catalog.search_youtube` / `youtube_suggestions` are not bridged; SearchPage is still
  Spotify-only and says "Sign in to search" to a YouTube-only user). Same for explore / charts / moods / YouTube
  history / new releases.
- `LikedPage`, `ReleasesPage`, `MiniMode` radio gating and Radio remain Spotify-gated (YouTube Liked Music is the
  `youtube:playlist:LM` row in the library). Sidebar enables all sections for a YouTube-only user; those pages show
  their own "sign in" states.
- Playing a YouTube playlist from a tile plays its first page only unless the library already holds the rows.
- New strings without `ru` / `pt` translations (bridges + QML): "YouTube Music", "Import", "Import playlist",
  "Playlist link", "Matching songs: {0} of {1}", "Reading the playlist…", "Save as a YouTube Music playlist",
  "Sign in to YouTube Music to save the result as a playlist", "Use YouTube Music", "YouTube Music connected",
  "Sign in with cookies", "Paste", "Couldn’t sign in to YouTube Music", "YouTube Music data saver", "{0} · not synced",
  and the others in the files above.
- Working-tree line endings: files edited in this phase are LF in the working copy (the index is LF anyway; git
  diffs are clean).
