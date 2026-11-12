# FLAC seek, fetching: handover (2026-11-13, ~02:30)

## Done in this session (items 5 and 6, uncommitted)
5. **Monochrome seek → silence: fixed** (`sf-player/src/engine/youtube/{download,decode}.rs`). Findings, live:
   `tracks.monochrome.st/track/<id>` honours ranges (206 + total), but it is erratic: 0.5–1.4 s to the first
   byte, 4 MiB in 7–10 s, one 256 KiB answer took 5.5 s (slower than the FLAC plays) and one far range came back
   206 with 0 bytes after 10.7 s. The decoder/symphonia seek itself is fine (live: lands in ~1.8 s and plays).
   Causes and fixes:
   - one 30 s whole-request timeout: a stalled answer kept the song silent up to 30 s (the reader's own timeout
     is 30 s too), and a slow answer that timed out counted as a failure (4 in a row killed the download). Now
     `IDLE_TIMEOUT` (8 s, `DownloadSpec::idle_timeout`) for the head and for each next piece of the body; an
     answer that stops after some bytes keeps them and the next request goes on from there (not a failure).
   - after a FLAC seek (binary search probes the file) the download kept going from the last probe, not after
     where playback landed → a second stall ~2 s later (bench log: "audio queue ran dry ... 2308 ms"). Now
     `StreamReader::read` reading a stretch the download is not extending sets `want` to the gap right after it,
     but only once the reader has read `chunk/64` (64 KiB) on from its last seek (`since`, `PLAYING_PART`): a first
     version that did it on every read redirected the download on each probe and made seeks ~3 s slower.
   - Live after the fix: no "ran dry" stalls; a 50 % seek landed in ~3 s (each probe is a 0.5–1 s round trip to
     this server). Then the server began answering **HTTP 429** (rate limit, most likely from this session's many
     probes): an 80 % seek hung on the 429 retries (backoff 0.8/1.6/3.2 s) and a 97 % seek failed and recovered
     through the lane's retry (re-resolve, plays on). Seek timing against the live server NOT re-measured cleanly.
     Ideas if seeks are still slow: honour `Retry-After` on 429; fewer probes (bigger first request after a jump so
     later probes land in fetched bytes).
   - a seek past the FLAC's end (the lane's length is YouTube's; Monochrome's metadata can be far off: 381 s
     listed for a 343 s file) failed with OutOfRange → now `Decoder::seek` clamps to the last moment, the song ends.
   - Tests: `decode.rs::a_downloading_flac_keeps_playing_after_a_far_seek` (new fixture
     `tests/fixtures/lossless/sweep_20s_noseektable.flac`, ffmpeg, no SEEKTABLE; server `Mode::Stalls`: failed
     before the fix with "no audio data arrived in time"), `download.rs::reading_an_earlier_stretch_brings_the_download_back_after_it`
     (`Mode::Slow`; checked it fails without the read-ahead), `an_answer_that_stops_sending_is_asked_again_from_where_it_stopped`,
     `flac_seeks` (past the end).
   - Live bench in the app before the fix (lossless on): FLAC played, seeks to 50/80/97 % resumed but with the stall
     above. NOTE `Player.playUris` gives no title/artist, so Monochrome is never tried: use `Player.playTracks` with
     maps (`uri,title,artists,durationMs`); bench at scratch `FlacBench.qml` (turns lossless on, JioSaavn off, and
     restores both at the end; the user's settings: losslessStreams=false, jioSaavnStreams=true).
6. **Fetching**: prefetch of the next song already existed (`YtEffect::Prefetch` on resolve → `preheat`: resolves and
   opens 1 KB). Added: `YouTubeService::warm_streams()` (stream.rs `warm`, `PoTokens::warm`) loads the player script
   and the BotGuard minter once the saved identity is restored (`auth/youtube.rs` restore, both branches), so the
   first web-tier song skips ~1.5–2 s; test `warming_up_loads_the_player_script_and_the_minter_before_the_first_song`.
   ANDROID_VR / ANDROID_VR_USER rest 15 → 30 → 60 → 120 min after bot checks in a row (`rest_after`,
   `MAX_REST_DOUBLINGS`; a stream resets; sign in/out resets), test `bot_checks_in_a_row_rest_a_tier_longer_up_to_two_hours`.
   Measured before: first song ~7 s from play to sound (debug build, signed in: VR anon + VR user round trips, script
   0.5 s, minter 0.8 s, WEB_REMIX). Not done (bigger, ask the user): start downloading the next song's first chunk
   during the current one so a track change does not wait for the server's first byte.

7. **Final**: `cargo test -p sf-services -p sf-player -p mf-platform -p mf-i18n -p sf-auth -p sf-api` green (no
   warnings), `cargo build -p materialfy` OK, PlayBench2 signed in: warm-up done in 2.1 s before play, first song
   sound ~2 s after play (was ~7 s), next + seeks 50/85 % kept playing. Release rebuilt 01:34
   (`target\dist\PixelMusic-0.5.1-Setup.exe` + portable zip). `build_windows.ps1` needs PowerShell 7 (`pwsh`), not 5.1.
   Nothing committed.

## (previous) NEXT SESSION list
5. **Monochrome (FLAC) seek forward → silence**: DONE above. `sf-player/src/engine/youtube/{player,decode,download}.rs`.
   Read so far: `Decoder::seek` (decode.rs ~209) does `SeekMode::Accurate`, `codec.reset()`, drops the resampler,
   `skip_until = required_ts`. symphonia-bundle-flac 0.5.5 `seek` (registry `src/demuxer.rs` 192–318): needs
   `byte_len()` (else `Unseekable`), binary-searches the byte range with `reader.seek(mid)` + `parser.resync` (each
   probe a far seek → the ranged download moves there, download.rs ~79/475), then a linear `read_byte` scan.
   Suspects: probes leave the download chasing the wrong offset / the reader blocks; `skip_until` vs FLAC packet ts;
   the player's `Seeking` helper (player.rs ~206–320) never resuming output. Repro: a test in download.rs/decode.rs
   that serves `tone_44k_stereo.flac` (or a longer generated FLAC) through `StreamReader` from a mock range server,
   seeks to 80%, and checks chunks keep coming. Live: Settings → source Monochrome (lossless) needs to be on.
6. **"Optimize the fetching system"**: not started. Ideas: when ANDROID_VR is resting, skip straight to the web
   tier (already); when signed in ANDROID_VR_USER is a wasted round trip after the first bot check (it rests 15 min);
   preheat/prefetch the next song (`StreamSource::prefetch` → `preheat`); load the player script + BotGuard minter
   at start instead of on the first song (~1.5 s saved on the first web-tier song). Ask the user if unclear.
7. Final: `cargo build -p materialfy`; `cargo test -p sf-services -p sf-player -p mf-platform -p mf-i18n -p sf-auth -p sf-api`;
   run `target\debug\pixelmusic.exe` (`QT_FORCE_STDERR_LOGGING=1`, `RUST_LOG=warn,sf_services=debug,sf_player=info`
   does print `log::debug!` from stream.rs, shown as `materialfy: log: ...`); bench:
   `--qml C:\Users\rajat\AppData\Local\Claude-Data\claude_pzs8sxrjxfjjc\side-session-notes\local_752dd74b-f1ab-4d32-a61a-77d9299a6260\PlayBench2.qml`
   (plays dQw4w9WgXcQ, fJ9rUzIMcZQ, kJQP7kiw5Fk via `Player.playUris`, next at 8 s, seeks to 50% and 85%). Then `.\build_windows.ps1 -QtDir C:\Qt\6.12.0\msvc2022_64 -Portable`.

## Done this session (uncommitted)
- **Signed-in "Sign in to confirm you're not a bot" — fixed and verified live (signed in, saved account).** Three causes:
  1. The BotGuard API key in `client.rs` was a placeholder (`REPLACE_WITH_YOUR_API_KEY`, from commit 2e794f3), so
     `Create` got HTTP 400 and the web tier never had a PO token. Now `cipher::botguard_key(js)` reads the public key
     from base.js (`{["X-Goog-Api-Key"]:"AIza..."}` next to `jnn-pa.googleapis.com`), `Cipher::load` returns
     `Script { sts, botguard_key }`, `InnerTube::botguard(method, body, key)`; env `BOTGUARD_API_KEY` still overrides
     (`potoken::KEY_ENV`). No key is in the source.
  2. Anonymous requests (ANDROID_VR `Login::Never`) carried the *account's* visitor id. `InnerTube::visitor_data(as_user)`
     now gives anonymous requests the anonymous id only.
  3. ANDROID_VR_USER sent web cookies + SAPISIDHASH → HTTP 400 (the client takes no cookies); with
     `onBehalfOfUser` and no cookie → 401. Now `ANDROID_VR_USER = ANDROID_VR` asked with `Login::Required`: no
     cookie, no onBehalfOfUser, the account's visitor id (`client.rs::post_query`: `user` vs cookie `account`). From
     this IP it gets an ordinary bot check and rests 15 min; it also rests on HTTP 4xx / 401.
  - Web tier: `web_tier(.., login)`; signed in it goes as the user (verified: `web stream itag 140 (as the user: true)`,
    played, seeks to 50%/85% of Bohemian Rhapsody kept playing → the video-bound `pot` works signed in). If the
    account's WEB_REMIX request is bot-checked, it retries once anonymously (`web_attempt`, `WebError::Refused(_, bot)`).
- **Sign-out/sign-in forgets the identity's tokens**: `InnerTube::set_account` returns whether the identity (cookie)
  changed and then drops the anonymous visitor id (generation counter so a late fetch is not kept);
  `YouTubeService::set_account` only on a change: `Streams::account_changed` (tiers, cache, flights, generation so
  older resolutions are not cached, `PoTokens::reset` = token cache + minter marked stale), home, durations.
- **Autoplay never runs dry**: `wave/yt.rs::AutoplayRadio` (in `WaveService.yt_autoplay`, tokio Mutex) follows the
  radio page by page (`YouTubeService::radio_page(video_id, continuation)` in catalog.rs, first page without automix
  so the continuation is RDAMVM's), never sends a song twice, and when a radio has no more pages follows the radio
  of its newest unsent-as-seed song; batches of 20, ≤4 requests per ask; a seed it did not send (user's own list)
  starts over. Actor (`actor.rs`): an `Autoplay` answer that adds 0 while that song plays clears `autoplay_asked`
  so it asks again, up to `AUTOPLAY_RETRIES` (3) per song.
- `materialfy.iss`: VersionInfoDescription "music player for YouTube Music, Monochrome and JioSaavn",
  `CloseApplicationsFilter=*pixelmusic.exe,*materialfy.exe` (old installs ran materialfy.exe), header comment.
- Tests (all green: `cargo test -p sf-services -p sf-player`): cipher key extraction; client_tests
  `the_signed_in_web_tier_follows_a_refusal` (rewritten), `signed_in_android_vr_goes_with_the_accounts_visitor_id_and_no_cookies`,
  `a_bot_check_or_a_refused_request_rests_the_accounts_tier_too`, `a_bot_checked_account_gets_the_web_stream_anonymously`,
  `signing_in_or_out_forgets_the_tokens_the_minter_and_the_streams`; wave/yt.rs three `AutoplayRadio` tests;
  state/tests.rs `autoplay_asks_again_while_its_answers_add_nothing`.

## Not verified
- Signed-out playback live (no way to sign out without losing the user's saved account; covered by mock tests).
- Autoplay live over many batches (unit/actor tests only).
- `build_windows.ps1` doc comment still says `materialfy.exe` in its synopsis lines 9/29 (cosmetic).

---

# Queue, durations, playback routing, branding, sign-in: handover (2026-11-13, ~00:30)

## (earlier list) the user's requests: 1-4 done (see top), 5-7 carried over to the top
1. **Signed in, songs fail with "Sign in to confirm you're not a bot"** (user screenshot: Kabhi Kabhi, 0:00/3:36).
   Signed out it works. `youtube/stream.rs::fetch`: ANDROID_VR anon → ANDROID_VR_USER (signed in) → `web_tier`
   (WEB_REMIX, `Login::IfSignedIn`, PO token bound to the video id). Likely the signed-in WEB_REMIX player request is
   bot-checked (PO_TOKEN_HANDOFF: "Not yet verified: whether the video-bound token also works when signed in";
   yt-dlp binds the GVS token to the DataSync id when logged in). Plan: make `WebError::Refused` carry whether it was
   a bot check; when signed in and refused by a bot check, retry `web_tier` with `Login::Never` (anonymous works);
   and/or mint the token for the account (`datasyncId` from `responseContext`/`ytcfg`). Reproduce first: the bench
   `--qml <scratch>/bench/QueueBench.qml --screenshot x.png --delay 25000` (screenshot mode does not save the
   player session) uses the saved account; `RUST_LOG=sf_services=debug` did NOT print `log::debug!` lines from
   stream.rs, find how to enable them (tracing-log bridge / filter target `sf_services::youtube::stream`).
2. **"Implement the Android VR client too"**: ANDROID_VR (anon) and ANDROID_VR_USER (signed in) already exist in
   `youtube/client.rs`; check they are really used signed in, why ANDROID_VR_USER fails (bot check → 15 min rest),
   and compare with the SoundWave/PixelMusic Android app's client set.
3. **Tokens removed on sign-out**: on YouTube sign-out forget the PO token minter/visitor data
   (`PoTokens::forget`, `Streams::account_changed` in stream.rs, `set_account(None)` in `YouTubeService`), and mint
   per identity on sign-in. Check what `account_changed` clears today.
4. **Autoplay "only some fixed tracks, then nothing"**: `YtLane::autoplay_seed/autoplay` + actor
   `autoplay_asked` (actor.rs) + `WaveService::yt_autoplay` (wave/yt.rs). Suspects: the radio adds 0 new songs (all
   already listed) so the same seed is never asked again; or playback errors (item 1) stop advancing. Make it keep
   generating: on 0 added, ask with the radio's continuation or the next unplayed seed; maybe also fill "Next up"
   at once like SoundWave. Add tests.
5. **Monochrome (FLAC) seek forward → audio goes silent**: `sf-player/src/engine/youtube/{player,decode,download}.rs`
   (2e794f3 changed download.rs/decode.rs/player.rs heavily, unreviewed). Seek in a FLAC stream fetched by
   ranges; check that seek past the downloaded part re-requests the right range and the decoder resets.
6. **"Optimize the fetching system"**: vague: stream resolution latency (tiers tried in sequence, each ANDROID_VR
   refusal costs a round-trip; cache; prefetch next song), Home/home feed. Ask the user if unclear.
7. Then rebuild the release: `.\build_windows.ps1 -QtDir C:\Qt\6.12.0\msvc2022_64 -Portable` (writes
   `target\dist\PixelMusic-0.5.1-Setup.exe` + portable zip; close running installers first).

## Done in the previous session after the list below
- Release 0.5.1 built (installer + portable zip in target\dist). Packaging scripts now copy `assets\logo`
  (build_windows.ps1, .github/scripts/package_{linux,macos}.sh).
- Icons: setup exe, release exe and the installed exe all carry the new icon (checked by extraction); the old "M"
  the user saw was Windows' icon cache, refreshed with `ie4uinit.exe -show`. If the desktop shortcut still shows the
  old icon, check the .lnk's IconLocation.
- `materialfy.iss` line 32 still says "Spotify client" in VersionInfoDescription; `CloseApplicationsFilter=*materialfy.exe`
  should be `*pixelmusic.exe` (fix both).

---

# (earlier) Queue, durations, playback routing, branding, sign-in: handover (2026-11-12, night)

Everything below is uncommitted on `main` (the user commits). Build environment: see `PO_TOKEN_HANDOFF.md`.
`stash@{0}` "WIP changes" is NOT fully superseded: its search history is replaced by HEAD's, but it also has YouTube
search suggestions (`Catalog.requestYouTubeSuggestions` / `youtubeSuggestions`) that HEAD lacks. Ask before dropping.

## Done this session
1. **Empty "Next up" rows**: `Player.playTracks(list, index)` (`bridges/player.rs`, `bridges/mod.rs::tracks_from_list`)
   queues whole track maps; Home's Quick picks / track shelves use it instead of `playUris`.
2. **Durations**: `YouTubeService::fill_durations` (`youtube/duration_cache.rs`) fills songs without a length in
   `home()` / `home_continuation()` with one WEB_REMIX `music/get_queue` request per 50 ids (cached 6 h);
   parser `parsers::queue_lengths` (plain + `playlistPanelVideoWrapperRenderer`). Old per-row ANDROID_VR lookup,
   `Catalog.resolveDuration`/`durationResolved` and the QML `onVisibleChanged` hooks removed. Unknown length shows
   nothing. Tests: `get_queue_gives_each_songs_length`, `songs_without_a_length_get_it_from_one_get_queue_request`.
3. **"Nothing is playing" on Play/seek**: cause — the session file was written/restored only by the Spotify `Core`,
   so a YouTube song from the last session was shown by Core and Play went down the Spotify path. Now
   `YtLane::restore/session` + actor `with_lane_session`: the lane saves (on each new song and on Core's saves) and
   restores its song paused at its position; Spotify sessions are no longer restored. Test:
   `a_restored_youtube_song_plays_in_the_lane_not_through_spotify` (+ lane test).
4. **Autoplay**: with Settings → Autoplay on, repeat off and nothing in "Playing next", when ≤1 song is left the actor
   emits `PlayerEvent::AutoplayWanted(track)`; `WaveService::yt_autoplay` fetches that song's YouTube Music radio
   and sends `PlayerHandle::autoplay` (`Command::Autoplay`), appended only if that song still plays. Test:
   `autoplay_asks_for_the_radio_near_the_end_and_appends_it_while_the_song_plays`.
5. **Review of 2e794f3**: reverted `cpp/mf_shell.cpp` (a hand-made `NoCacheReply` wrapper dropped the real reply's
   error/status/redirects → covers failing); removed a garbled Chinese comment in `Main.qml`; Quick picks
   "Show more" pushed a non-existent `quickPicks` route → now expands in place; search history no longer records
   every debounced partial query (only Enter / history clicks).
6. **Spotify cut from the app** (user chose "cut from the app", code/crates kept): DJ voice, Spotify bitrate,
   engine-restart banner, gapless, crossfade and speed (Spotify sink only) removed from Settings; Devices and
   Friends tabs, device buttons and DJ badge removed; Smart Mix finds songs on YouTube Music only; About text.
7. **Branding**: ran `tools/pixelmusic_logo.py` (needs Pillow) → real Pixel Music icon from the Android drawables in
   `materialfy.ico` (exe), `materialfy.png`, `assets/materialfy.svg` (window), tray PNGs, `assets/logo/*.svg`.
   Sidebar shows that logo + "Pixel Music"; Settings colour picker shows the real logos; window icon follows the
   colour; exe VERSIONINFO "Pixel Music"/`pixelmusic.exe`; `setApplicationDisplayName("Pixel Music")`
   (applicationName stays "Materialfy": it names the data/settings folders). Deleted the AI-made
   `assets/logos`, `assets/icons`, `crates/icon-gen`, `LogoColorwayPicker.qml`, `tests/qml/test_duration_*`,
   `test_logo_colorway_integration.qml`.
8. **YouTube sign-in not kept**: Windows Credential Manager refuses secrets > 2560 bytes and the YouTube cookie set
   is bigger, so the save failed every time. `credentials.rs`: `Chunked` store splits secrets into 2048-byte
   entries (`<key>#n`, head `mf-chunks:<n>`). Test: `a_secret_bigger_than_a_windows_credential_is_split_...`.

## Verified
- `cargo build -p materialfy` (no warnings), `cargo test -p sf-services -p sf-player -p mf-platform -p mf-i18n -p sf-auth
  -p sf-api` green. Binary is `target\debug\pixelmusic.exe`.
- App run with `QT_FORCE_STDERR_LOGGING=1`, no QML errors: Home (logo, durations filled live), Settings (logo picker),
  and a `--qml` bench with NowPlaying + QueueView: `Player.playTracks` played a YouTube song, "Next up" rows full, and
  autoplay appended the song's radio live.

## Not verified / left
- Saving a real sign-in after the chunking fix (the user must sign in once more; the old failed saves left nothing).
- Restoring a YouTube song across a real app restart (unit/actor tests only).
- The Settings → Playback section was not in the screenshot.
- `win.autoPlay` (`--set autoPlay=`) dev hook waits for the Spotify engine and no longer fires.
