# soundwave1 feature port notes

Each porting session keeps its own section; edit only yours.

## Desktop extras

Sleep timer, equalizer settings, listening stats, settings backup/restore, M3U export. Ported from
soundwave1 `desktopApp` (`PlayerController.setSleepTimer`, `EqualizerPanel`, `data/ListeningStats.kt`,
`BackupPanel`) and the Android app (`SleepTimerStateHolder`, `M3uManager`). Nothing from `TelegramAdminManager`
was ported: no telemetry, device tracking or remote block.

### Rust back end (done, tested)

- **Sleep timer**: `crates/sf-player/src/sleep.rs`.
  - `SleepTimer` is a pure state machine on a millisecond clock. It has two modes: N minutes, or end of the
    current track.
  - It fades the volume out over the last 10 s (`FADE_MS`), then pauses and puts the volume back.
  - If the user moves the volume during the fade, the fade restarts from their level. Seeking back out of the
    fade window undoes the fade.
  - A Connect device is paused without a fade (`PlayerView.fade = false`).
  - `SleepTimerHandle::spawn(rt, player)` runs it every 250 ms and also wakes on `PlayerEvent::TrackEnded`.
  - Pausing goes through `PlayerHandle::pause` / `set_volume`. The actor routes those to the YouTube lane when
    it is active, so Spotify and YouTube are both covered.
  - Tests use a fake clock and a fake target (`SleepTarget`).
- **Equalizer settings**: `crates/mf-platform/src/settings.rs`.
  - New keys: `eqEnabled`, `eqPreamp` (±12 dB, clamped), `eqBands` (10 comma-separated dB values; empty means
    flat), `eqPreset` (preset name; empty means custom), `audioBalance` (−1..1), `monoAudio`.
  - New helpers: `eq_bands_from_str` / `eq_bands_to_string`.
  - `sf_player::engine::eq::{preset(name), EqSettings::from_stored(..)}` map the stored settings onto the engine.
  - `SettingKey::ALL` is now 101 entries. The `schema` test had a stale 93 and was updated to 101; other
    sessions adding keys must bump both.
- **Listening stats**: `crates/sf-services/src/stats.rs`.
  - Own compact log `app_data_dir()/stats.json`, up to 20,000 `ListenEvent`s holding the seconds actually heard.
  - Fed by `PlayerEvent::TrackEnded { local: true }` once at least 10 s were heard. YouTube lane plays also end
    with `local: true`, so they are counted.
  - On first open, the log is seeded from `history.json` (the journal). Those plays count as the track's length,
    capped at 10 min, because the journal keeps no listening time.
  - `summary(plays, range, today, tz)` is pure. It returns totals, average per day, Spotify vs YouTube seconds,
    top 10 tracks (by plays) and top 6 artists (by time; YouTube artists without ids merge by name).
  - Buckets: Monday..Sunday for a week, days for a month, months for a year.
  - Ranges are `week` | `month` | `year` | `all`.
- **Backup/restore**: `crates/sf-services/src/transfer/backup.rs`.
  - One JSON file `{format: "materialfy-backup", version: 1, settings, history, stats}`.
  - Sign-ins, cookies, tokens and keys are never read: they live in the sf-auth credential store.
  - `is_private` drops these settings: `playbackAuthorized`, `webClientId`, `proxyUsername`, window geometry,
    and any key whose name contains token/cookie/secret/password/apikey/clientid/session/credential/auth.
  - Import ignores private and unknown names.
  - `HistoryService::{local_plays, replace_local}` and `Journal::replace` were added for the history part.
- **M3U export**: `crates/sf-services/src/transfer/m3u.rs`.
  - `export_m3u(name, tracks, locate)` writes `#EXTM3U`, `#PLAYLIST`, then `#EXTINF:<s>,<artists> - <title>`
    and a location for each track.
  - Locations: Spotify tracks become open.spotify.com links and YouTube tracks music.youtube.com links. Local
    files use the path from the caller's `locate`.
  - Tracks without a location are skipped and counted.
  - A round-trip test reads the output back with `youtube::import::parse_m3u`.
  - `file_name()` makes a safe `.m3u8` file name.

### Services wiring (done, compiles)

- `glue::shared_eq()` is one process-wide `SharedEq`. It is used by `engine_config` (every librespot restart)
  and by the YouTube `YtOutput.eq` in `Services::start`.
- `glue::spawn_settings_glue` applies `eq_settings(..)` at startup and on every eq/balance/mono setting change.
  The bridge only writes settings.
- `Services` has two new fields: `stats` (`StatsService::new`) and `sleep` (`SleepTimerHandle::spawn`).
  `shutdown` flushes the stats.

### Bridges and QML (written, not built)

- `crates/materialfy/src/bridges/extras.rs`: QML singleton `Extras`, registered in `cpp/register.cpp` and
  `bridges/mod.rs`.
  - Sleep: `sleepActive`, `sleepEndOfTrack`, `sleepRemaining` (s), `setSleepMinutes`, `sleepAtEndOfTrack`,
    `cancelSleep`, `sleepFired`.
  - EQ: `eqEnabled`, `eqPreamp`, `eqBands`, `eqPreset`, `audioBalance`, `monoAudio`, `eqPresetNames`,
    `eqFrequencies`, `setBand`, `applyPreset`, `resetEq`.
  - Stats: `stats(range)`, `formatTime`, `clearStats`, `statsChanged`.
  - Files: `exportBackup`, `importBackup`, `backupFileName`, `exportPlaylistM3u(id, title, file)`, `m3uFileName`,
    `busy`, `transferFinished(ok, message)`.
- The Settings bridge (`settings.rs`) was not touched.
- New QML: `app/EqualizerCard.qml`, `app/SleepTimerCard.qml`, `app/ListeningStatsCard.qml`, `app/BackupCard.qml`.
- Edits to shared QML:
  - `SettingsPage.qml`: nav sections `equalizer`, `stats`, `backup`; `cardFor` cases; EQ and sleep cards after
    Playback; stats and backup cards after Storage.
  - `PlaylistPage.qml`: ⋯ → "Export as M3U" with a save `FileDialog`.
  - `Main.qml`: a `Connections` to `Extras` for the sleep and transfer toasts.
- Icons are limited to names in `assets/fonts/icons.json`: insights, settings_backup_restore, upload and
  file_save are missing, so history, restore and save are used instead.
- New strings go through `I18n.t` / `mf_i18n::t`/`tf` but were not added to the `i18n/` dictionaries, so ru/pt
  show English.
- Stats are a Settings card, not a separate page like soundwave1's StatsScreen.
- The sleep timer lives in Settings, not in the player bar like soundwave1.

### Unverified

- Nothing in `materialfy` or QML was built: Qt is not installed, and materialfy must not be built here.
  Possible compile risks, all following patterns used elsewhere in the bridges except where noted:
  - the `QList_QVariant` property `eqBands`: no other bridge has a QVariantList property, only invokables
    return one;
  - the custom WRITE setters;
  - `self.rust()` reads.
- M3U export uses `LibraryService::playlist_tracks`, which only has rows for a playlist whose page was opened
  (the menu item is disabled until the model has rows).
- The fade uses `set_volume`, so the bridge's volume-saving path sees the faded values for up to 10 s. The
  timer restores the level after pausing, so the saved value ends up right, but it was not checked in the app.
- Restoring a backup only puts settings back through `SettingsStore::set`. Settings that the app reads only at
  startup (bitrate, UI scale, GPU) take effect after a restart.

## Last.fm

Ported from soundwave1 `desktopApp/.../data/LastFm.kt` (`LastFm`, `LastFmScrobbler`) and the `LastFmPanel` in
`ui/screens/Settings.kt`.

### What was added

- `crates/sf-auth/src/lastfm.rs`: `LastFmAccount` (API key, shared secret, session key, user name; no `Debug`).
  It lives in a new credential slot, `Slot::LastFm` (key `lastfm`), in `credentials.rs`. A record with only the
  keys is valid, so the keys are saved before sign-in. Signing out of Spotify or YouTube leaves this slot alone.
- `crates/mf-platform/src/settings.rs`: `LastFmScrobble` (`lastfmScrobble`) and `LastFmNowPlaying`
  (`lastfmNowPlaying`). Both default to on, and both are read each time a call would go out.
- `crates/sf-services/src/lastfm.rs` (tests in `lastfm/tests.rs`) adds `LastFmService`:
  - Calls go to `ws.audioscrobbler.com/2.0`, sent as POST form data. `api_sig` is the md5 of the sorted
    parameters plus the secret.
  - Sign-in uses the web-token flow: `auth.getToken`, then the approval page opens in the browser, then
    `auth.getSession` is polled every 2.5 s. Error 14 means "not approved yet". It gives up after 180 s.
  - Error 9 forgets the session and keeps the keys. Errors 10 and 26 get a "check the key" message.
  - It sends `track.updateNowPlaying` on the first play of a listen. It sends `track.scrobble` once, after
    min(half the length, 180 s) of played time. The timestamp is the moment of first play. An unknown length
    means 180 s. Tracks of 30 s or less are never scrobbled.
  - The played-time timer pauses with playback and with buffering.
  - The rule is the pure `Listen` struct. The clocks are injected through `LastFmOptions`.
  - It is fed by `PlayerEvent::TrackStarted` / `TrackEnded` with `local == true`, the same as `history.rs`.
    That covers Spotify on this PC and YouTube Music. It also reads `StateChanged` / `TrackChanged` for
    playing or paused.
  - Podcast episodes and "Unknown artist" are skipped. YouTube "Artist - Topic" becomes "Artist".
  - It is wired in `Services::start`, which shares the auth `Store` clone, and exposed as `Services::lastfm`.
- `md-5` (already in the workspace) was added to the sf-services dependencies.
- Bridge `crates/materialfy/src/bridges/lastfm.rs` is the QML singleton `LastFm`.
  - Properties: `loaded`, `hasKeys`, `connected`, `username`, `apiKey`, `signingIn`, `authUrl`, `errorText`,
    `createKeyUrl`.
  - Methods: `setKeys`, `signIn`, `cancelSignIn`, `disconnect`, `forget`. Signal: `connectedAs`.
  - Registered in `bridges/mod.rs` and `cpp/register.cpp`.
- The Settings bridge gained the `lastfmScrobble` / `lastfmNowPlaying` properties.
- The "Last.fm" card in `SettingsPage.qml` sits after the YouTube Music card. It uses `SettingsCard`,
  `SettingRow`, `SettingsBanner`, `TextField`, `Button` and `Switch`, all already in the app.

### Differences from the Kotlin code

- There is no offline retry queue, because neither the desktop nor the Android Kotlin code has one. A failed
  scrobble is logged and dropped.
- There is no built-in API key. The user always enters their own. The secret is never shown back: an empty
  secret field keeps the saved one.
- Spotify Connect playback on other devices (`local == false`) is not scrobbled. Spotify's own Last.fm link
  covers that.

### Unverified

- `materialfy` and the QML were not built or run, because Qt is not installed. Check these in the app:
  - the bridge compiles (cxx-qt property and signal names);
  - the card lays out correctly;
  - `Qt.openUrlExternally` works for "Open page" and "Create".
- Not tested against the real Last.fm. Request shapes match the Kotlin code and Last.fm's docs, and they were
  checked against a local mock server only.
- The card has no section chip in `cardFor()`, so the section navigation does not scroll to it.
- `connectedAs` is not hooked to a toast; the card's header shows the connected name.
- Repeat-one counts as a new listen only if the player emits a new `TrackStarted` when the track restarts.
  The Kotlin code checked for this explicitly; it was not checked here.

### Last.fm feed and Smart Mix

Ported from soundwave1 `data/feed/FeedLastFmRepository.kt` and `presentation/viewmodel/SmartMixViewModel.kt`
(the mode fetchers, `fetchMix`, `fetchRecommendations` with `scoreTrack`, `resolveTrackToYoutubeSong` and
`generatePlaylistName`), with the modes of `presentation/screens/SmartMixScreen.kt`.

- `crates/sf-services/src/lastfm_feed.rs` (tests in `lastfm_feed/tests.rs`):
  - `FeedApi` makes unsigned read calls: GET `ws.audioscrobbler.com/2.0/?method&api_key&format=json&…`. It uses
    the API key and username of the account connected in `LastFmService`, with no session key and no signature.
  - The feed: `user.getrecenttracks` (30, with the "now playing" row dropped), `user.getfriends` (20; real name,
    else username; the last real avatar) and `user.gettopalbums` (20, overall; "Unknown artist" skipped). The
    parsers accept both a single object and an array, an artist as a string or `{name|#text}`, an album as
    `{#text|title}`, and they drop Last.fm's no-art placeholder image. "Guest User" asks nothing.
  - Smart Mix modes `top`, `library`, `recent`, `similar-tracks`, `similar-artists`, `tag`, `mix` and
    `recommendations` use the Kotlin limits and random pages:
    - The blend's buckets have weights 4 (top tracks over 3 random periods), 3 (recent plays and
      `track.getsimilar` of 5 of them), 2 (`user.gettopartists`, then `artist.getsimilar`, then
      `artist.gettoptracks`) and 1 (`user.gettoptags`, then `tag.gettoptracks`). The highest weight wins in the
      dedupe; then each tier is shuffled, and there are at most 2 tracks per artist.
    - Recommendations use `scoreTrack`: a recent play is rejected, and the score bands are 30, then 10, then the
      rest. The split is 60 % familiar to 40 % discovery, with at most 2 tracks per artist.
  - All randomness goes through one `fastrand::Rng`. Each bucket gets its own fork, so one seed gives one mix
    (`LastFmFeedOptions::seed`).
  - Resolving: the search is "name artist" on YouTube Music (`search_songs`) or Spotify (`CatalogService::search`,
    tracks), through the `TrackSearch` trait. Each result passes the gate in `sources/matching.rs`, which this
    port only reads. Three searches run at once, in order. Results are deduped by URI, and it stops once it has
    the requested count.
  - Names follow `generatePlaylistName` (`{Artist}Era`, `{Track}Echo`, `{Tag}Hood`…). `unique_name` adds " #2",
    " #3" and so on against the library's playlist titles.
  - `LastFmFeedService` (`Services::lastfm_feed`) publishes a `watch` snapshot. Its methods are `refresh_feed`,
    `generate(request, target, existing)`, `cancel` and `play_track` (a feed row: resolve it, then play it).
  - Tests: a GET mock server, a fake catalog search and fixed seeds. They cover the parsers, naming, weighting,
    scoring, each mode's request parameters, the blend's reproducibility, resolving, and the service end to end.
- Bridge `crates/materialfy/src/bridges/smart_mix.rs` is the QML singleton `SmartMix`.
  - Properties: `feedLoading`, `recent`, `friends`, `topAlbums`, `feedError`, `generating`, `progress`,
    `mixTitle`, `mixMode`, `mixCover`, `trackCount`, `mixError`, and `tracks` (a track `DictListModel`).
  - Methods: `refreshFeed`, `generate`, `cancel`, `save` (`LibraryService::create_playlist_from`, so a YouTube
    mix becomes a YouTube Music playlist) and `playTrack`.
  - Registered in `bridges/mod.rs` and `cpp/register.cpp`.
- `src/materialfy/qml/app/pages/SmartMixPage.qml` is built on `TrackListPage`, like `RadioPage`.
  - The hero note holds the mode `Chip`s and the `MenuField`s for period, count and catalog. The seed
    `TextField`s show only for the modes that need them.
  - The footer holds the feed as `MediaTile` flows: recent scrobbles (click to play), top albums and friends.
  - The page is routed as `smartmix` in `Main.qml`. A "Smart Mix" sidebar entry stays enabled without Spotify
    sign-in.
  - Icons come from the subset font only (`star`, `equalizer`, `label`, `schedule`, `format_list_bulleted`).

Differences from the Kotlin code:
- A catalog result must pass the matching gate. Kotlin took the first YouTube result, preferring ATV. As a
  result, fewer tracks may resolve, but wrong songs are not added. The catalog's own title and artist are kept;
  Kotlin overwrote them with Last.fm's.
- A mix is not saved automatically: the Save button makes the playlist. Kotlin wrote the songs into Room and
  created a playlist on every run.
- Resolving stops once it has the requested count; Kotlin resolved every candidate.
- The supplement step in `fetchRecommendations` had a key typo (`"…|it.first.artist"`). The port uses the
  real key.
- The seed search helpers (`track.search`, `artist.search` and the top-seed pickers) are not ported. Seeds are
  typed in. `parse_tracks` and `parse_artists` already read the search answers.

Unverified:
- The bridge and the page were not built or run, because Qt is not installed. Check that the cxx-qt names
  compile, that the hero-note controls fit in `TrackListPage`'s header, and that the feed `ColumnLayout` sizes
  correctly as `footerExtra`.
- Not tested against the real Last.fm, YouTube Music or Spotify search. Request shapes match the Kotlin code
  and were checked only against the mock. The matching gate's acceptance rate on real searches without
  durations is unknown.
- The page is not in the router's `keepAlive`, so the chosen mode and inputs reset when you navigate away. The
  generated mix stays in the service.
- Top album and friend tiles do nothing when clicked. There is no Last.fm → catalog album lookup or profile
  page.

Covers for feed tiles (added after a user report that most recent-track and top-album tiles showed placeholders):
- Last.fm often sends an empty image URL or its grey star (`2a96cbd8b46e442fc41c2b86b821562f`); both count as no
  art (`is_real_image`, tested).
- A tile without art calls `SmartMix.requestCover(kind, title, artist)` when it is created, so only the shown
  tiles ask. `LastFmFeedService::request_cover` looks each `cover_key` ("track|artist|title" / "album|…", lower
  case) up once per run, at most 3 at a time, through `CoverSearch`; the YouTube implementation uses
  `search_songs` (track) or `search(…, Albums)` (album) and the same matching gate, so a wrong song's art is not
  used. Found covers are published as `covers` (bridge `QVariantMap`, `coversChanged`), and the tiles bind
  `page.coverOf(kind, item)`: Last.fm art, else the found cover.
- A generated mix without Last.fm art takes the first resolved track's cover for the hero.
- Misses are cached too (not asked again until restart); the cache is in memory only.

## Local files

Port of soundwave1's desktop `LocalFiles.kt`, plus artist splitting from Android `Extensions.kt` and a folder
tree like `FolderTreeBuilder.kt`. Nothing is committed.

**What it does**
- `crates/sf-services/src/local/`:
  - `tags.rs` reads tags, the embedded cover, duration, rate, bit depth and codec with **symphonia**. That is
    the decoder the player already uses, so there is no new crate (no lofty). It covers ID3v2, Vorbis
    comments, MP4 atoms and RIFF INFO. An MP3 with no frame count gets its duration by summing packets,
    without decoding.
  - `index.rs` scans folders up to 8 levels deep and stops at 20 000 files (soundwave1's limits). It skips
    hidden folders and keeps the index at `app_data_dir()/local_files.json`. A rescan re-reads only files
    whose size or mtime changed.
  - Covers are written once per album to `cache_dir()/local_covers`. If a file has no embedded picture,
    `cover|folder|front|album|albumart.(jpg|png|webp)` next to it is used instead.
  - Ids are FNV-1a of the path, giving `local:track:<16 hex>`.
  - `artists.rs` splits artists on `/ ; , + &` and on the words feat./ft./vs/with/prod (only between spaces).
    A backslash escapes a delimiter (`AC\/DC`). Artists named in the title as "(feat. X)" are extracted.
  - `mod.rs` has `LocalFilesService`: a watch snapshot, a cancellable scan that drops superseded runs, and
    `to_item` mapping to `TrackItem` with a `file:///` cover URL and empty album/artist ids, so there are
    no Spotify pages behind the rows. It also has `folder_tree` and `follow_folders_setting`.
  - `source.rs` has `LaneSource`, the `StreamSource` the player gets. It answers `local:track:` with a
    `YtStream` whose url is `file:<path>` and sends everything else to the YouTube `SourceChain`.
- Setting `LocalFolders` (`localFolders`, mf-platform): one path per line. Changing it rescans.
- `Services.local` exists, and `enable_youtube` now gets `LaneSource` wrapping the old `SourceChain`
  (`sf-services/src/lib.rs`).
- sf-player changes:
  - `engine/youtube/mod.rs` adds `LOCAL_TRACK`, `LOCAL_FILE_URL`, `local_id`, `is_local_track`, `lane_id`
    and `is_lane_track`. `StreamRequest::from_track` now uses `lane_id`.
  - `player.rs` adds `open_local`, which `http_opener` calls first, so `file:` URLs are read from disk.
  - `state/youtube.rs` and `state/actor.rs` use `is_lane_track` and `lane_id` instead of
    `is_youtube_track` and `video_id`, so local URIs take the lane. The lane hands the source the whole
    `local:track:<id>` and hands YouTube tracks the bare video id.
- UI (not built: Qt isn't installed):
  - Bridge `crates/materialfy/src/bridges/local_files.rs` is the singleton `LocalFiles`, registered in
    `cpp/register.cpp`. It exposes `tracks` (a Track DictListModel, context "local"), `folders`,
    `scanning/found/read/errorText`, `addFolder(url|path)`, `removeFolder` and `rescan`.
  - `qml/app/pages/LocalFilesPage.qml` is a `TrackListPage`. The header has add-folder and rescan buttons,
    and the empty state has an "Add folder" action. It uses `FolderDialog`.
  - Route `local` in `Main.qml`. Sidebar entry "Local files", which is enabled without signing in.
  - Settings has a "Local files" card after YouTube Music: the folder list with delete buttons, plus
    Add folder and Rescan.
- Fixtures: `tests/fixtures/local/` holds tagged.flac (with a cover), tagged.mp3, tagged.ogg and
  untagged.wav, made with ffmpeg. Tests: `sf-services` `local::*` and the sf-player lane/player tests.

**Unverified / open**
- Symphonia features: the lossless session added `"alac", "wav", "pcm"`. mp3, ogg and vorbis already come
  in through librespot's symphonia features (feature unification). No ALAC fixture was tested.
- `decode.rs` is unchanged. FLAC, MP3, Ogg and WAV files decode through `Decoder::open` with their mime
  (test `local_formats_decode_by_their_mime`).
- The bridge and QML were never compiled or run:
  - `I18n.t` placeholders.
  - The `FolderDialog` import.
  - `IconButton`/`Button` props, copied from other pages.
  - The `folder_open`, `add` and `delete` icons are in `icons.json`, but the subset font was not rebuilt
    (`create_new_folder` isn't in it, so it wasn't used).
- New i18n strings have no translations yet. Among them: "Local files", "Unknown artist", "Add folder",
  "Rescan", "Scanning… {0} of {1}", "Music folders" and the delete/empty-state texts.
- The quality label ("FLAC 16/44.1") is stored but not shown, to avoid design changes. The player's codec
  label uses `YtStream::codec_label`, which shows MP3/OGG as "AUDIO <kbps>".
- History, Last.fm and lyrics may see `local:track:` URIs. They were not checked for how they handle them.
  The actor already skips Spotify `Remember` for lane tracks.
- Race: if the folders setting changes within milliseconds of startup, `start()` may rescan the old
  folders after the new ones.
- Opus files are not listed, because symphonia has no Opus decoder. APE, WMA and AIFF are not listed either.
- No folder-tree view in QML yet. `LocalFilesService::folder_tree` is ready for one.

## Downloads

Port of soundwave1's `SongDownloadWorker` / `PlaylistDownloadWorker` / `DownloadRepository`, with a storage
limit and a clean-up after `StorageSenseRepository`. Nothing is committed.

**What it does**
- `crates/sf-services/src/downloads/`:
  - `mod.rs` has `DownloadsService`. It keeps a queue and runs `WORKERS` = 2 downloads at once. It has a watch
    snapshot (`DownloadsSnapshot`: tracks, playlists, jobs with progress, bytes, limit, folder), plus `download`,
    `download_playlist`, `cancel`, `cancel_all`, `retry`, `retry_failed`, `remove`, `remove_playlist`,
    `remove_all`, `forgotten(days)` and `remove_forgotten(days)`.
  - Songs are resolved through the lane's `SourceChain` (`resolve_track`), so a Monochrome or JioSaavn copy is
    saved when Settings choose one. The file is `<folder>/<videoId>.<ext>` (m4a, flac, mp3…, from the mime) and
    the cover is `<folder>/covers/<videoId>.jpg`.
  - A network failure is retried up to 4 tries in all, with a doubling delay, and goes on from the end of the
    `.part` file. A 403 or 410 invalidates the address and resolves again (3 times at most). A failed job stays
    in the list until `retry`.
  - The index is `app_data_dir()/downloads.json`, holding the `TrackItem`, file, cover, format, source, download
    time, last play, the playlists and whether it was downloaded on its own. Entries whose file is gone are
    dropped at start.
  - Removing a playlist deletes only the songs that were not downloaded on their own or with another playlist,
    as in the Kotlin code.
  - `fetch.rs` saves a URL with 4 MiB `Range` requests and the stream's User-Agent. It is the same approach as
    `engine/youtube/download.rs`, which keeps the whole file in memory for a reader and so wasn't reused.
  - `source.rs` has `OfflineSource`, which wraps the lane's source (`LaneSource`). A downloaded video is answered
    with `file:<path>`, which the player's existing `open_local` reads from disk, so it needs no network. A play
    sets `last_played`. Prefetch is skipped for downloaded songs.
- Settings (mf-platform): `DownloadsFolder` (`downloadsFolder`, "" means `app_data_dir()/downloads`) and
  `DownloadsLimitMb` (`downloadsLimitMb`, 0 means no limit). Both are followed live. A new folder applies to the
  next downloads, and the files already saved stay where they are. When the limit is reached, new downloads fail
  with "Downloads have reached their storage limit"; nothing is deleted on its own.
- `sf-services/src/lib.rs` adds `Services.downloads` and puts `OfflineSource` in front of `LaneSource` before
  `enable_youtube`. sf-player was not changed.
- UI (not built: Qt isn't installed):
  - Bridge `crates/materialfy/src/bridges/downloads.rs` is the singleton `Downloads`, registered in
    `cpp/register.cpp`. It exposes `tracks` (a Track model, context "downloads", rows with the saved cover as
    `cover`), `revision`, `queued`, `running`, `failed`, `progress`, `usage`, `folder` and `errorText`.
  - Its invokables are `download(track)`, `downloadPlaylist(uri)` (the library's loaded rows),
    `removePlaylist`, `remove`, `cancel`, `cancelAll`, `retry`, `retryFailed`, `removeAll`,
    `removeForgotten(days)`, `setFolder`, `isDownloaded`, `isPending`, `isFailed`, `progressOf`, `hasPlaylist`,
    `playlistStatus` and `canDownload`.
  - `TrackMenu.qml` has "Download", "Cancel download" and "Remove download" for `youtube:track:` rows, handled
    inside the menu.
  - `PlaylistPage.qml` has a download / remove-download toggle button for `youtube:playlist:` pages.
  - `TrackRow.qml` shows a `download_done` badge (or `downloading` while queued) after the title.
  - `pages/DownloadsPage.qml` is a `TrackListPage` with retry, cancel, clean-up (songs not played for 30 days)
    and folder buttons. Route `downloads` in `Main.qml`; the Sidebar entry "Downloads" works without signing in.
- Tests: `sf-services` `downloads::tests` (12). They use an in-process Range server, temp folders, a fake
  stream source and `tests/fixtures/youtube/tone_44k_stereo.m4a`. They cover:
  - ranged pieces and the User-Agent, the cover, the index across a restart, and offline resolve with no
    network call, then decoding the file through `open_local` + `Decoder`;
  - re-resolve after a 403;
  - resume after 500s without starting at 0 again;
  - give up, then retry;
  - 2 at a time, a playlist kept in order, and playlist removal that keeps a single;
  - cancel leaving no `.part` file;
  - the storage limit;
  - the clean-up of songs not played for long.

**Unverified / open**
- No real YouTube download was run. URL lifetime, throttling and the WEB_REMIX User-Agent were only checked
  against the fake server.
- The bridge and QML were never compiled or run:
  - cxx-qt details (the `QMap_QString_QVariant` argument, `tf` with mixed `&dyn Display`);
  - `IconButton` props copied from PlaylistPage;
  - `FolderDialog`;
  - the `download_done`, `downloading`, `delete_sweep` icons, which are in `icons.json`.
- Offline cover: the player's cover fetch only takes http(s) URLs, so rows keep the network address in
  `coverLarge`. Offline, the now-playing cover shows only if the player cached it before. List thumbnails use
  the saved `file://` cover.
- `downloadPlaylist` takes the rows the library has loaded. For a playlist opened outside the library, or one
  still loading, only the first page may be saved. Songs added to the playlist later are not downloaded on
  their own.
- Lyrics, history, Last.fm and the player's "AAC/FLAC" label for downloaded songs were not checked. The stream
  `source` is "DOWNLOAD".
- No Settings card for the folder or the limit; only the page's folder button. `DownloadsLimitMb` has no UI.
- Downloads keep running while the app is open only. There is no resume of the queue after a restart: jobs are
  not saved, and a leftover `.part` file is resumed only if the same song is downloaded again into the same
  folder.
- New strings have no `ru` / `pt` translations: "Downloads", "Download", "Cancel download", "Remove download",
  "Downloaded", "Downloading", "Downloading…", "Downloading {0} songs… {1}%", "{0} downloads failed",
  "No downloads yet", "Songs saved on this computer", the empty-state text, "Retry failed downloads",
  "Cancel downloads", "Remove songs not played for 30 days", "Downloads folder", "Choose the downloads folder",
  "New downloads go to the chosen folder", "Downloads have reached their storage limit",
  "The download address keeps expiring", "No songs to clean up", "Removed {0} songs, freed {1}",
  "{0} of {1} downloaded", "{0} of {1}", "Removed the download of “{0}”" and "Downloading “{0}”".

## LAN Jam

Same-Wi-Fi Jam, no account. Ported from soundwave1's `desktopApp/.../jam/LanJam.kt` and `JamSession.kt`
(LAN parts only) and Android's `data/jam/wifi/LocalWifiJamEngine.kt` and `data/jam/model/JamProtocol.kt`.
Spotify's own Jam (`social.rs`, `JamCard.qml`) is unchanged.

### What was added

- `crates/sf-services/src/lanjam/` (new):
  - `protocol.rs`: the JamProtocol codecs (playback, queue item, queue, participant, settings, log, room,
    `applyRoomJson`, controls). It also has the flat-query form of a control, both ways, including Android's
    legacy `action=` and bare-track `/queue` form. When a key appears twice, the first value wins, as with
    Android's `getQueryParameter`. Also here: the beacon (`{"app":"soundwave_jam",roomId,hostName,port}` on UDP
    58245), `JamTrackSource.youtubeIdOf`/`classify`, room codes, and invite-link parsing
    (`…/jam/CODE?localHost=&localPort=`). Readers follow `org.json` `opt*` rules: missing or mistyped fields
    get defaults, and numbers and booleans may arrive as strings.
  - `http.rs`: a minimal HTTP/1.1 server and client over tokio (one request per connection,
    `Connection: close`, JSON body). Query values are percent-encoded; on decode `+` becomes a space, as in
    Java's `URLDecoder`.
  - `host.rs`: `LanHost`. It serves `/time`, `/join`, `/sync?since=` (long-poll held 4 s on a `watch` version),
    `/control` and `/queue`, and `/leave`. Guests get 16-byte random tokens; a removed guest is banned. Guests
    with pending approval or blocked by room settings are refused. The beacon goes out every 3 s. A 300 ms
    player watch bumps the version on track, play/pause, loading, queue, repeat or shuffle changes, and on a
    seek jump of more than 1.5 s. Guests not seen for 90 s are dropped. Controls map onto `PlayerHandle`:
    play/pause, next/previous, `seek_ms`, `seekToIndex`/`seekToTrack` → `play_queue_row`, shuffle, repeat
    (0 off, 1 one, 2 all), and `addTrack` → `add_to_queue`, `insert_manual(0)`, or `insert_manual(0)` + `next`
    for play-now.
  - `guest.rs`: `LanGuest` (join, sync loop, host lost after 20 s, 403 → kicked; `/time` clock offset from 5
    samples, keeping the lowest-RTT one, re-measured at 15 s, 15 s, then every 60 s; controls; leave).
    `Discovery` is the beacon listener: it lists rooms heard in the last 8 s and leaves out this computer's own
    room. `Follower` makes the local player follow the host. On a new track it calls
    `play_items(the host's playable queue, current row)`, then `seek_ms(target)`. Otherwise it matches
    play/pause and corrects drift above 2.5 s, or above 0.8 s right after a host change. After a hard seek it
    waits 4 s before correcting again. Target = `positionMs + (now + offset − hostClockMs)` while playing.
  - `mod.rs`: `JamPlayer` trait (implemented for `PlayerHandle`; tests use a fake), `NetConfig`, and
    `LanJamService`. The service's snapshot is serialised for QML. It covers host, join, leave, approve,
    remove, room settings, guest controls (`toggle_play`, `control`, `add_track`), and start/stop discovery.
  - `tests.rs`: 21 hermetic tests using loopback sockets only:
    - every `JamProtocolTest.kt` case, plus exact key sets and Android-shaped JSON;
    - controls through a real query string, and the beacon format;
    - track identity and invite links;
    - a host and a guest over loopback: join payload, controls, a long-poll wake, kick, and ban;
    - approval and settings gates;
    - the beacon heard by discovery;
    - the follower: load, seek, pause and drift;
    - two `LanJamService`s (one hosting, one joining) in one process.
- Track identity on the wire matches the Kotlin desktop host:
  - `youtube:track:X` → `trackId=X`, `youtubeId=X`, `YOUTUBE`
  - `saavn:track:X` → `saavn_X`, `JIOSAAVN`
  - `local:track:X` → `local_X`, `LOCAL`
  - `spotify:track:X` → its URI with source `SPOTIFY`. This is a Materialfy extension; Kotlin apps treat it as
    unplayable.

  Guests play YouTube tracks in the YouTube lane, and a Materialfy host's Spotify tracks through Spotify.
  `saavn_X` plays only once the lane accepts `saavn:track:`, checked at runtime through `is_lane_track`.
  Anything else is skipped.
- `sf-services/src/lib.rs`: `pub mod lanjam`, `Services.lanjam` (built with `NetConfig::system()`), and
  `lanjam.shutdown()` in `shutdown`.
- Bridge `crates/materialfy/src/bridges/lanjam.rs`: the `LanJam` singleton. It has `state` and `active`, plus
  `host`, `join`, `leave`, `approve`, `remove`, `setRoomSettings`, `startDiscovery`, `stopDiscovery`,
  `togglePlay`, `next`, `previous` and `copyInvite`. It is registered in `bridges/mod.rs` and `register.cpp`.
- QML: `app/LanJamCard.qml` (new), the "Same Wi-Fi Jam" card under `JamCard` in `FriendsPanel.qml`. It is
  shown even when signed out and reuses only existing MD3 components and subset icons.

### Unverified

- **Android interop has not been tested on real devices**, in either direction (desktop host + Android guest,
  Android host + desktop guest). Compatibility rests on reading the Kotlin code and on JSON-shape tests only.
  The Kotlin desktop app has not been tried against it either.
- `materialfy` was not built (no Qt), so the bridge and `LanJamCard.qml` were never compiled or shown.
- Real networks were never used:
  - The beacon goes to 255.255.255.255 and to the computer's address with `.255` in place of the last part,
    which assumes a /24 network: std can't list interfaces or their masks.
  - The port-58245 listener binds without `SO_REUSEADDR`, so it fails if another app (e.g. SoundWave desktop)
    holds the port.
  - Windows Firewall prompts for the server and the UDP port have not been seen.
- `local_ipv4()` (a UDP "connect" to 8.8.8.8 with no packet sent) is used in the app only, never in tests.
- The guest's own player buttons still act locally and are then corrected by the host sync. Only the card's
  three buttons are routed to the host. Nothing is wired yet for adding a song to the Jam from track menus
  (`LanJamService::add_track` exists).
- The host's display name is the computer name (`COMPUTERNAME`/`HOSTNAME`); there is no field to change it.
- Room-setting switches (add tracks, control, approval) are in the service and the bridge, but not in the card.
- The activity log is sent to guests but not shown here. New log strings and card strings are English only
  (no dictionary entries).

### Later

- Online Jam over Ably (`AblyHostClient.kt`, `AblyGuestClient.kt`) was left out. It needs the host's own Ably
  API key in Settings, plus invite tokens.
