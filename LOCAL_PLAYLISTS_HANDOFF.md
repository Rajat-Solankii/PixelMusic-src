# Handover: local playlists + "Sync local playlists to YouTube Music" (Pixel Music)

Read `LASTFM_YT_INTEGRATION_HANDOFF.md` at the repo root for the project rules (Qt path, qmllint baseline, screenshot
command, settings-key recipe, icons.json, no commits, ask before closing the user's running app).

## What the user asked
In Settings → Accounts → Last.fm card → Open Service → **Last.fm Integration** page, the user wants all four switches
real. They chose to build the very large item too: **local playlists** (desktop has none today — every library
playlist is a YouTube Music one, `crates/sf-services/src/library/yt.rs`) and **"Sync local playlists to YouTube
Music"** ("Sync local playlist edits back to your YouTube Music account").

## Already done (uncommitted, by the previous session)
- Setting `localPlaylistSync` (bool, default true) exists: `SettingKey::LocalPlaylistSync` in
  `crates/mf-platform/src/settings.rs`, `Settings.localPlaylistSync` in `crates/pixelmusic/src/bridges/settings.rs`.
- The switch row is already on the page: `src/pixelmusic/qml/app/LastFmSettings.qml` (SwitchRow key
  "localPlaylistSync", icon "public"). It currently has NO backend — hide it (`visible: false`) if this work is dropped.
- Other work of that session (all wired): YouTube card switches `youtubeLibrarySync`, `youtubeHistorySync`,
  `youtubePersonalHome`, `youtubePersonalRadio` (lib.rs `follow_youtube_account_use`, youtube/mod.rs
  `feed_login`/`radio_login`); Last.fm page; `lastfmExplore` (Home shelf `LastFmHomeShelf.qml`);
  `lastfmShowMixes` + `lastfmMixRetentionDays` (`library/smart_mixes.rs`); `lastfmSmartMix` (lib.rs
  `follow_lastfm_home_mixes`, `HomeService::set_lastfm_mixes`, `LastFmFeedService::home_mix`).

## Suggested design (to confirm with a quick look at the code)
- Store: `app_data_dir()/local_playlists.json` — `[{id, title, tracks:[TrackItem uri/title/artist/cover/duration],
  created, updated, youtube_id: Option<String>}]`. URI scheme `local:playlist:<uuid>` (check `PlaylistId` in
  `crates/md3-model` for how ids/providers are parsed; `contexts` for play contexts).
- LibraryService routes by id (like `yt.rs` does for YouTube ids): open page, add/remove/move tracks, rename,
  delete, create. Library list entries (`yt_entries` equivalent) with kind "playlist", `read_only: false`.
- "New playlist" in the UI should create a local playlist when YouTube is signed out, or always (ask the user).
- Sync: when `localPlaylistSync` is on and YouTube is signed in, each edit is mirrored to a linked YouTube playlist
  (create it on first sync with `YouTubeService::create_playlist`, then add/remove/move via the existing
  `yt_add_tracks`/`yt_remove_tracks`/`yt_move_tracks` mutations). Only `youtube:` tracks can be mirrored.
- Tests in sf-services for the store and the routing; run
  `cargo test -p sf-services -p mf-platform -p md3-model`.

## Done means
qmllint clean (baseline aside), cargo check/tests pass, the app builds and runs via `run_windows.bat`, a dark + light
screenshot of a local playlist in Library, and the switch on the Last.fm page really drives the sync.
