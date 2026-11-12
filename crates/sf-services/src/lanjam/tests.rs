//! Hermetic: loopback sockets only. Protocol cases mirror the Kotlin `JamProtocolTest.kt`.
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use md3_model::{PlayContext, QueueRow, QueueSection, TrackItem};
use serde_json::{Value, json};
use sf_player::{PlayerSnapshot, Repeat};

use super::guest::{Discovery, Follower, GuestEvent, LanGuest};
use super::host::{HostConfig, LanHost};
use super::protocol::*;
use super::{JamPlayer, LanJamService, NetConfig, http, now_ms};

fn rt(json: &Value) -> Value {
    serde_json::from_str(&json.to_string()).unwrap()
}

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

fn sorted(list: &[&str]) -> Vec<String> {
    let mut k: Vec<String> = list.iter().map(|s| s.to_string()).collect();
    k.sort();
    k
}

// ---------------- protocol (JamProtocolTest.kt) ----------------

#[test]
fn playback_round_trips_every_field_including_both_clock_stamps() {
    let p = JamPlayback {
        track_id: "saavn_abc".into(),
        title: "Tum Hi Ho — ✨".into(),
        artist: "Arijit".into(),
        artwork_url: "https://a/b.jpg".into(),
        source: "JIOSAAVN".into(),
        is_playing: true,
        position_ms: 123_456,
        server_timestamp: 1_794_000_000_123,
        host_clock_ms: 1_794_000_000_080,
        duration_ms: 262_000,
        youtube_id: None,
        content_uri: Some("https://aac.saavncdn.com/x.mp4".into()),
    };
    let json = playback_to_json(&p);
    assert_eq!(
        keys(&json),
        sorted(&["trackId", "title", "artist", "artworkUrl", "source", "isPlaying", "positionMs", "serverTimestamp", "hostClockMs", "durationMs", "youtubeId", "contentUri"])
    );
    // a null id goes out as "" (Kotlin `p.youtubeId ?: ""`)
    assert_eq!(json["youtubeId"], "");
    assert_eq!(playback_from_json(&rt(&json)), p);
}

#[test]
fn playback_reads_what_the_android_host_sends() {
    // org.json output of JamProtocol.playbackToJson for a YouTube song
    let text = r#"{"trackId":"youtube_dQw4w9WgXcQ","title":"Song","artist":"A","artworkUrl":"","source":"YOUTUBE","isPlaying":false,
        "positionMs":5000,"serverTimestamp":1794000000000,"hostClockMs":1794000000000,"durationMs":212000,"youtubeId":"dQw4w9WgXcQ","contentUri":""}"#;
    let p = playback_from_json(&serde_json::from_str(text).unwrap());
    assert_eq!(p.youtube_id.as_deref(), Some("dQw4w9WgXcQ"));
    assert_eq!(p.content_uri, None);
    assert_eq!(p.position_ms, 5000);
    assert!(!p.is_playing);
    // missing source → YOUTUBE, as optString("source", "YOUTUBE")
    assert_eq!(playback_from_json(&json!({})).source, "YOUTUBE");
}

#[test]
fn queue_item_keeps_content_uri_source_and_duration() {
    let q = JamQueueItem {
        queue_id: "3_saavn_x".into(),
        track_id: "saavn_x".into(),
        title: "T".into(),
        artist: "A".into(),
        artwork_url: String::new(),
        source: "JIOSAAVN".into(),
        duration_ms: 200_000,
        youtube_id: None,
        content_uri: Some("saavn://x".into()),
        added_by: "u1".into(),
        added_by_name: "Guest".into(),
        added_at: 42,
        is_play_next: true,
    };
    let json = queue_item_to_json(&q);
    assert_eq!(
        keys(&json),
        sorted(&["queueId", "trackId", "title", "artist", "artworkUrl", "source", "durationMs", "youtubeId", "contentUri", "addedBy", "addedByName", "addedAt", "isPlayNext"])
    );
    assert_eq!(queue_item_from_json(&rt(&json)), q);
}

#[test]
fn queue_payload_round_trips_items_and_index() {
    let items: Vec<JamQueueItem> = (0..5)
        .map(|i| JamQueueItem { queue_id: format!("{i}_id{i}"), track_id: format!("id{i}"), title: format!("T{i}"), ..Default::default() })
        .collect();
    let q = JamQueue { items: items.clone(), current_index: 3, repeat_mode: 1, shuffle_enabled: true, total_count: 5 };
    let json = rt(&queue_to_json(&q));
    assert_eq!(keys(&json), sorted(&["v", "items", "currentIndex", "repeatMode", "shuffleEnabled", "totalCount"]));
    let back = queue_from_json(&json);
    assert_eq!(back.items.iter().map(|i| i.queue_id.clone()).collect::<Vec<_>>(), items.iter().map(|i| i.queue_id.clone()).collect::<Vec<_>>());
    assert_eq!(json["currentIndex"], 3);
    assert_eq!(json["shuffleEnabled"], true);
    assert_eq!(back, q);
}

fn sample_room() -> JamRoom {
    JamRoom {
        room_id: "ABC234".into(),
        host_id: "host".into(),
        host_name: "Rajat".into(),
        created_at: 7,
        participants: vec![
            JamParticipant { user_id: "host".into(), name: "Rajat".into(), is_host: true, ..Default::default() },
            JamParticipant { user_id: "g1".into(), name: "Guest".into(), is_pending: true, device: "Pixel".into(), ..Default::default() },
        ],
        settings: JamRoomSettings { allow_guests_to_add_tracks: false, allow_guests_to_control_playback: true, require_host_approval_to_join: true },
        activity_log: vec![JamLogItem { id: "l1".into(), icon_type: "join".into(), message: "Guest joined".into(), timestamp: 9 }],
        lan_host_ip: Some("192.168.1.20".into()),
        lan_port: Some(40000),
        is_online: true,
    }
}

#[test]
fn room_metadata_round_trips_participants_settings_log_and_lan_info() {
    let room = sample_room();
    let json = rt(&room_to_json(&room));
    assert_eq!(
        keys(&json),
        sorted(&["v", "roomId", "hostId", "hostName", "createdAt", "settings", "participants", "activityLog", "lanHostIp", "lanPort", "isOnline"])
    );
    assert_eq!(keys(&json["participants"][0]), sorted(&["userId", "name", "isHost", "isPending", "joinedAt", "device"]));
    assert_eq!(keys(&json["activityLog"][0]), sorted(&["id", "iconType", "message", "timestamp"]));
    assert_eq!(keys(&json["settings"]), sorted(&["allowGuestsToAddTracks", "allowGuestsToControlPlayback", "requireHostApprovalToJoin"]));
    let back = apply_room_json(None, &json);
    assert_eq!(back.participants, room.participants);
    assert_eq!(back.settings, room.settings);
    assert_eq!(back.activity_log, room.activity_log);
    assert_eq!(back.lan_host_ip.as_deref(), Some("192.168.1.20"));
    assert_eq!(back.lan_port, Some(40000));
    assert!(back.is_online);
    assert_eq!(back, room);
}

#[test]
fn applying_room_metadata_keeps_fields_it_does_not_carry() {
    let base = JamRoom { room_id: "R".into(), host_name: "Old".into(), created_at: 5, ..Default::default() };
    let updated = apply_room_json(Some(&base), &json!({ "hostName": "New" }));
    assert_eq!(updated.host_name, "New");
    assert_eq!(updated.room_id, "R");
    assert_eq!(updated.created_at, 5);
    // Kotlin: isOnline defaults to true, settings to the defaults
    assert!(updated.is_online);
    assert_eq!(updated.settings, JamRoomSettings::default());
}

fn all_controls() -> Vec<JamControl> {
    vec![
        JamControl::Play,
        JamControl::Pause,
        JamControl::SkipNext,
        JamControl::SkipPrevious,
        JamControl::SeekTo(5_000),
        JamControl::SeekToIndex { index: 4, position_ms: 1_000 },
        JamControl::SeekToTrack { track_id: "youtube_x".into(), position_ms: 2_000 },
        JamControl::SetShuffle(true),
        JamControl::SetRepeat(2),
    ]
}

#[test]
fn every_control_action_round_trips() {
    for action in all_controls() {
        let json = rt(&control_to_json(&action, "u1", "Guest"));
        assert_eq!(json["senderId"], "u1");
        assert_eq!(json["guestName"], "Guest");
        assert_eq!(control_from_json(&json, 0), Some(action));
    }
    let types: Vec<String> = all_controls().iter().map(|a| opt_str(&control_to_json(a, "", ""), "type")).collect();
    assert_eq!(types, ["play", "pause", "skipNext", "skipPrevious", "seekTo", "seekToIndex", "seekToTrack", "setShuffle", "setRepeat"]);
}

#[test]
fn add_track_control_keeps_the_full_track() {
    let track = JamQueueItem {
        track_id: "saavn_x".into(),
        title: "Song".into(),
        artist: "A".into(),
        source: "JIOSAAVN".into(),
        content_uri: Some("saavn://x".into()),
        duration_ms: 9,
        ..Default::default()
    };
    let json = rt(&control_to_json(&JamControl::AddTrack { track: Box::new(track), is_play_next: true, play_now: false }, "u1", "Guest"));
    let Some(JamControl::AddTrack { track, is_play_next, play_now }) = control_from_json(&json, 77) else { panic!("not addTrack") };
    assert!(is_play_next && !play_now);
    assert_eq!(track.content_uri.as_deref(), Some("saavn://x"));
    assert_eq!(track.source, "JIOSAAVN");
    assert_eq!(track.duration_ms, 9);
    // the host restamps it, as Kotlin does
    assert_eq!(track.queue_id, "q-77");
    assert_eq!(track.added_at, 77);
    assert!(track.is_play_next);
}

#[test]
fn unknown_control_type_is_ignored() {
    assert_eq!(control_from_json(&json!({ "type": "selfDestruct" }), 0), None);
    assert_eq!(control_from_json(&json!({ "type": "addTrack" }), 0), None);
}

#[test]
fn lan_controls_survive_the_flat_query_both_ways() {
    let mut actions = all_controls();
    actions.push(JamControl::AddTrack {
        track: Box::new(JamQueueItem { track_id: "dQw4w9WgXcQ".into(), title: "Never & Gonna?".into(), artist: "Rick Astley".into(), youtube_id: Some("dQw4w9WgXcQ".into()), duration_ms: 212_000, ..Default::default() }),
        is_play_next: false,
        play_now: true,
    });
    for action in actions {
        let params = control_to_params(&control_to_json(&action, "u1", "Guest Name"));
        assert!(params.iter().all(|(k, _)| k != "track"));
        // through a real query string
        let query = http::build_query(&params);
        let back = control_from_json(&control_from_params(&http::parse_query(&query)), 5);
        match (&action, back) {
            (JamControl::AddTrack { track, play_now, .. }, Some(JamControl::AddTrack { track: got, play_now: got_now, .. })) => {
                assert_eq!(got.title, track.title);
                assert_eq!(got.youtube_id, track.youtube_id);
                assert_eq!(got.duration_ms, track.duration_ms);
                assert_eq!(got_now, *play_now);
            }
            (_, back) => assert_eq!(back.as_ref(), Some(&action)),
        }
    }
    // legacy `/queue`: bare track parameters
    let legacy = control_from_params(&[("title".into(), "Song".into()), ("youtubeId".into(), "abc".into())]);
    assert!(matches!(control_from_json(&legacy, 0), Some(JamControl::AddTrack { .. })));
    let legacy = control_from_params(&[("action".into(), "skipNext".into())]);
    assert_eq!(control_from_json(&legacy, 0), Some(JamControl::SkipNext));
}

#[test]
fn query_encoding_matches_java_url_decoder() {
    assert_eq!(http::decode("a+b%20c"), "a b c");
    assert_eq!(http::decode("%E2%9C%A8%"), "✨%");
    let text = "Tum Hi Ho — ✨ & = + ?";
    assert_eq!(http::decode(&http::encode(text)), text);
    assert!(!http::encode(text).contains(' '));
    assert_eq!(http::parse_query("uid=a%2Bb&name=Guest+One&flag"), vec![("uid".into(), "a+b".into()), ("name".into(), "Guest One".into()), ("flag".into(), String::new())]);
}

#[test]
fn beacon_matches_the_kotlin_format() {
    let b = Beacon { room_id: "ABC234".into(), host_name: "Rajat's PC".into(), port: 40000 };
    let bytes = beacon_to_bytes(&b);
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(keys(&json), sorted(&["app", "roomId", "hostName", "port"]));
    assert_eq!(json["app"], "soundwave_jam");
    assert_eq!(beacon_from_bytes(&bytes), Some(b));
    // Android/desktop beacon text as org.json writes it
    let android = br#"{"app":"soundwave_jam","roomId":"XYZ789","hostName":"Pixel","port":41234}"#;
    assert_eq!(beacon_from_bytes(android).unwrap().port, 41234);
    assert_eq!(beacon_from_bytes(br#"{"app":"other","roomId":"X","port":1}"#), None);
    assert_eq!(beacon_from_bytes(br#"{"app":"soundwave_jam","roomId":"X","port":0}"#), None);
    assert_eq!(beacon_from_bytes(b"soundwave_jam"), None);
    assert_eq!(BEACON_PORT, 58245);
}

#[test]
fn track_identity_follows_jam_track_source() {
    assert_eq!(youtube_id_of("x", None, Some("abc")).as_deref(), Some("abc"));
    assert_eq!(youtube_id_of("x", Some("youtube://abc?t=1"), None).as_deref(), Some("abc"));
    assert_eq!(youtube_id_of("youtube_abc", None, None).as_deref(), Some("abc"));
    assert_eq!(youtube_id_of("saavn_abcde", None, None), None);
    assert_eq!(classify("saavn_abcde", None, None), SOURCE_JIOSAAVN);
    assert_eq!(classify("123", Some("https://aac.saavncdn.com/x.mp4"), None), SOURCE_JIOSAAVN);
    assert_eq!(classify("123", None, None), SOURCE_LOCAL);

    assert_eq!(wire_track("youtube:track:abc"), ("abc".into(), SOURCE_YOUTUBE, Some("abc".into())));
    assert_eq!(wire_track("saavn:track:q1").0, "saavn_q1");
    assert_eq!(wire_track("local:track:f1").1, SOURCE_LOCAL);
    assert_eq!(wire_track("spotify:track:4u").1, SOURCE_SPOTIFY);

    let lane = |uri: &str| uri.starts_with("youtube:track:");
    assert_eq!(local_uri("abc", "YOUTUBE", Some("abc"), None, lane).as_deref(), Some("youtube:track:abc"));
    assert_eq!(local_uri("youtube_abc", "YOUTUBE", None, None, lane).as_deref(), Some("youtube:track:abc"));
    assert_eq!(local_uri("spotify:track:4u", "SPOTIFY", None, None, lane).as_deref(), Some("spotify:track:4u"));
    // JioSaavn plays only once the lane takes saavn:track:
    assert_eq!(local_uri("saavn_q1", "JIOSAAVN", None, None, lane), None);
    assert_eq!(local_uri("saavn_q1", "JIOSAAVN", None, None, |u: &str| u.starts_with("saavn:")).as_deref(), Some("saavn:track:q1"));
    assert_eq!(local_uri("1234", "LOCAL", None, Some("content://media/1234"), lane), None);
}

#[test]
fn join_input_takes_codes_and_invite_links() {
    assert_eq!(parse_join(" abc-234 "), Some(("ABC234".into(), None)));
    assert_eq!(parse_join("ab"), None);
    assert_eq!(
        parse_join("https://sound-wave-website-eta.vercel.app/jam/ABC234?localHost=192.168.1.20&localPort=40000"),
        Some(("ABC234".into(), Some(("192.168.1.20".into(), 40000))))
    );
    assert_eq!(parse_join("https://x/jam/abc234#t=token"), Some(("ABC234".into(), None)));
}

// ---------------- fake player ----------------

#[derive(Default)]
struct FakePlayer {
    snap: Mutex<PlayerSnapshot>,
    log: Mutex<Vec<String>>,
}

fn yt(id: &str) -> TrackItem {
    TrackItem { track_id: id.into(), uri: format!("youtube:track:{id}"), title: format!("Title {id}"), artists: "Artist".into(), duration_ms: 200_000, available: true, ..Default::default() }
}

fn rows(items: &[TrackItem], current: usize) -> Arc<Vec<QueueRow>> {
    Arc::new(
        items
            .iter()
            .enumerate()
            .skip(current)
            .map(|(i, t)| QueueRow { section: if i == current { QueueSection::Current } else { QueueSection::Context }, uid: i as u64 + 1, track: t.clone() })
            .collect(),
    )
}

impl FakePlayer {
    fn playing(items: &[TrackItem], current: usize, position_ms: i64) -> Arc<Self> {
        let p = Arc::new(FakePlayer::default());
        p.load(items, current, position_ms, true);
        p
    }

    fn load(&self, items: &[TrackItem], current: usize, position_ms: i64, playing: bool) {
        let mut s = self.snap.lock().unwrap();
        s.track = items.get(current).cloned();
        s.queue = rows(items, current);
        s.current_index = 0;
        s.position_ms = position_ms;
        s.duration_ms = 200_000;
        s.playing = playing;
    }

    fn record(&self, entry: String) {
        self.log.lock().unwrap().push(entry);
    }

    fn entries(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl JamPlayer for FakePlayer {
    fn snapshot(&self) -> PlayerSnapshot {
        self.snap.lock().unwrap().clone()
    }
    fn play_items(&self, items: Vec<TrackItem>, row: usize, _context: PlayContext) {
        self.record(format!("play_items {} @{row}", items.iter().map(|t| t.uri.as_str()).collect::<Vec<_>>().join(",")));
        self.load(&items, row, 0, true);
    }
    fn play(&self) {
        self.record("play".into());
        self.snap.lock().unwrap().playing = true;
    }
    fn pause(&self) {
        self.record("pause".into());
        self.snap.lock().unwrap().playing = false;
    }
    fn next(&self) {
        self.record("next".into());
    }
    fn previous(&self) {
        self.record("previous".into());
    }
    fn seek_ms(&self, ms: i64) {
        self.record(format!("seek {ms}"));
        self.snap.lock().unwrap().position_ms = ms;
    }
    fn play_queue_row(&self, uid: u64) {
        self.record(format!("row {uid}"));
    }
    fn set_shuffle(&self, value: bool) {
        self.record(format!("shuffle {value}"));
    }
    fn set_repeat(&self, repeat: Repeat) {
        self.record(format!("repeat {}", repeat.as_str()));
    }
    fn add_to_queue(&self, items: Vec<TrackItem>) {
        self.record(format!("add {}", items.iter().map(|t| t.uri.as_str()).collect::<Vec<_>>().join(",")));
    }
    fn insert_manual(&self, item: TrackItem, at: usize) {
        self.record(format!("insert {} @{at}", item.uri));
    }
    fn lane_plays(&self, uri: &str) -> bool {
        uri.starts_with("youtube:track:") || uri.starts_with("local:track:")
    }
}

const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

fn config(code: &str, beacon_targets: Vec<SocketAddr>) -> HostConfig {
    HostConfig {
        room_id: code.into(),
        host_id: "desktop-host".into(),
        host_name: "Host PC".into(),
        device: "Materialfy for Windows".into(),
        lan_ip: Some("127.0.0.1".into()),
        bind: LOOPBACK,
        beacon_targets,
        settings: JamRoomSettings::default(),
    }
}

async fn eventually(what: &str, limit: Duration, mut check: impl FnMut() -> bool) {
    let start = std::time::Instant::now();
    while !check() {
        assert!(start.elapsed() < limit, "timed out waiting for: {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

type Events = Arc<Mutex<Vec<String>>>;

fn recorder() -> (Events, super::guest::GuestCallback) {
    let events: Events = Arc::default();
    let sink = events.clone();
    let cb: super::guest::GuestCallback = Arc::new(move |e| {
        let text = match e {
            GuestEvent::Sync(body) => format!("sync {}", opt_str(body.get("playback").unwrap_or(&Value::Null), "trackId")),
            GuestEvent::Kicked => "kicked".into(),
            GuestEvent::HostLost => "lost".into(),
        };
        sink.lock().unwrap().push(text);
    });
    (events, cb)
}

// ---------------- host + guest over loopback ----------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_and_guest_talk_over_loopback() {
    let tracks = [yt("aaa"), yt("bbb"), yt("ccc")];
    let host_player = FakePlayer::playing(&tracks, 0, 30_000);
    let host = LanHost::start(config("ABC234", vec![]), host_player.clone()).await.unwrap();
    let addr = SocketAddr::new(LOOPBACK, host.port());

    // /time answers with this computer's clock
    let (code, text) = http::request(addr, "GET", "/time", Duration::from_secs(1), Duration::from_secs(1)).await.unwrap();
    assert_eq!(code, 200);
    assert!((opt_i64(&serde_json::from_str(&text).unwrap(), "serverTime") - now_ms()).abs() < 5_000);
    // unknown path; sync without a token
    assert_eq!(http::request(addr, "GET", "/nope", Duration::from_secs(1), Duration::from_secs(1)).await.unwrap().0, 404);
    assert_eq!(http::request(addr, "GET", "/sync?uid=x&token=y", Duration::from_secs(1), Duration::from_secs(1)).await.unwrap().0, 403);

    let (events, cb) = recorder();
    let (guest, first) = LanGuest::join(addr, "android-1", "Pixel Guest", "Pixel 8", cb).await.unwrap();
    assert_eq!(keys(&first), sorted(&["v", "version", "serverTime", "room", "playback", "queue", "you"]));
    assert_eq!(first["you"]["userId"], "android-1");
    assert_eq!(first["you"]["token"].as_str().unwrap().len(), 32);
    let room = apply_room_json(None, &first["room"]);
    assert_eq!(room.room_id, "ABC234");
    assert_eq!(room.lan_port, Some(host.port()));
    assert!(!room.is_online);
    assert_eq!(room.participants.iter().map(|p| (p.user_id.as_str(), p.is_host)).collect::<Vec<_>>(), [("desktop-host", true), ("android-1", false)]);
    let pb = playback_from_json(&first["playback"]);
    assert_eq!((pb.track_id.as_str(), pb.youtube_id.as_deref(), pb.source.as_str()), ("aaa", Some("aaa"), "YOUTUBE"));
    assert!(pb.is_playing && pb.position_ms == 30_000 && pb.host_clock_ms > 0 && pb.host_clock_ms == pb.server_timestamp);
    let queue = queue_from_json(&first["queue"]);
    assert_eq!(queue.items.iter().map(|q| q.queue_id.as_str()).collect::<Vec<_>>(), ["0_aaa", "1_bbb", "2_ccc"]);
    assert_eq!(queue.current_index, 0);
    assert!(guest.clock_offset().abs() < 1_000);

    // guest → host controls, flattened into the query
    guest.control(&control_to_json(&JamControl::Pause, guest.user_id(), "Pixel Guest")).await.unwrap();
    guest.control(&control_to_json(&JamControl::SeekTo(61_000), guest.user_id(), "Pixel Guest")).await.unwrap();
    guest.control(&control_to_json(&JamControl::SeekToTrack { track_id: "ccc".into(), position_ms: 0 }, guest.user_id(), "Pixel Guest")).await.unwrap();
    guest.control(&control_to_json(&JamControl::SetRepeat(1), guest.user_id(), "Pixel Guest")).await.unwrap();
    let add = JamQueueItem { track_id: "youtube_ddd".into(), title: "Added".into(), ..Default::default() };
    guest.control(&control_to_json(&JamControl::AddTrack { track: Box::new(add), is_play_next: true, play_now: false }, guest.user_id(), "Pixel Guest")).await.unwrap();
    assert_eq!(host_player.entries(), ["pause", "seek 61000", "row 3", "repeat one", "insert youtube:track:ddd @0"]);

    // a change on the host reaches the waiting long-poll
    host_player.load(&tracks, 1, 0, true);
    eventually("sync with the new track", Duration::from_secs(6), || events.lock().unwrap().iter().any(|e| e == "sync bbb")).await;

    // the host removes the guest: its next sync is refused and it is told so
    host.remove("android-1");
    eventually("kicked", Duration::from_secs(6), || events.lock().unwrap().iter().any(|e| e == "kicked")).await;
    assert!(!guest.connected());
    // banned: can't come back
    let (_, cb) = recorder();
    assert!(LanGuest::join(addr, "android-1", "Again", "", cb).await.is_err());
    assert_eq!(host.participants().len(), 1);
    host.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_gates_controls_by_approval_and_room_settings() {
    let host_player = FakePlayer::playing(&[yt("aaa")], 0, 0);
    let mut cfg = config("GATE22", vec![]);
    cfg.settings.require_host_approval_to_join = true;
    let host = LanHost::start(cfg, host_player.clone()).await.unwrap();
    let addr = SocketAddr::new(LOOPBACK, host.port());
    let (_, cb) = recorder();
    let (guest, first) = LanGuest::join(addr, "g1", "G", "", cb).await.unwrap();
    let mut follower = Follower::new("g1");
    follower.absorb(&first);
    assert!(follower.pending());

    let send = |c: JamControl| {
        let body = control_to_json(&c, "g1", "G");
        let guest = guest.clone();
        async move { guest.control(&body).await.unwrap() }
    };
    send(JamControl::SkipNext).await;
    assert!(host_player.entries().is_empty(), "a pending guest can't control");

    host.approve("g1");
    send(JamControl::SkipNext).await;
    assert_eq!(host_player.entries(), ["next"]);

    host.set_settings(JamRoomSettings { allow_guests_to_add_tracks: false, allow_guests_to_control_playback: false, require_host_approval_to_join: false });
    send(JamControl::SkipPrevious).await;
    send(JamControl::AddTrack { track: Box::new(JamQueueItem { youtube_id: Some("zzz".into()), ..Default::default() }), is_play_next: false, play_now: false }).await;
    assert_eq!(host_player.entries(), ["next"]);

    // leaving removes the guest at once
    guest.leave(true).await;
    eventually("guest gone", Duration::from_secs(2), || host.participants().len() == 1).await;
    host.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beacon_is_heard_on_loopback() {
    let discovery = Discovery::start(SocketAddr::new(LOOPBACK, 0), Some("OWN999".into())).await.unwrap();
    let player = FakePlayer::playing(&[], 0, 0);
    let host = LanHost::start(config("BEAC0N", vec![discovery.local_addr]), player.clone()).await.unwrap();
    let own = LanHost::start(config("OWN999", vec![discovery.local_addr]), player).await.unwrap();
    let found = discovery.find("beac0n", Duration::from_secs(4)).await.expect("beacon");
    assert_eq!((found.room_id.as_str(), found.host_name.as_str(), found.host_ip.as_str(), found.port), ("BEAC0N", "Host PC", "127.0.0.1", host.port()));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(discovery.nearby().iter().all(|n| n.room_id != "OWN999"), "this computer's own room isn't listed");
    host.stop();
    own.stop();
}

// ---------------- following the host ----------------

fn payload(track: &str, playing: bool, position_ms: i64, stamp: i64, queue: &[&str], current: i64, pending: bool) -> Value {
    let items = queue.iter().enumerate().map(|(i, id)| JamQueueItem { queue_id: format!("{i}_{id}"), track_id: id.to_string(), youtube_id: Some(id.to_string()), title: id.to_string(), ..Default::default() }).collect();
    json!({
        "room": room_to_json(&JamRoom {
            room_id: "R".into(), host_name: "Host".into(),
            participants: vec![JamParticipant { user_id: "me".into(), is_pending: pending, ..Default::default() }],
            ..Default::default()
        }),
        "playback": playback_to_json(&JamPlayback { track_id: track.into(), youtube_id: Some(track.into()), is_playing: playing, position_ms, host_clock_ms: stamp, server_timestamp: stamp, ..Default::default() }),
        "queue": queue_to_json(&JamQueue { items, current_index: current, ..Default::default() }),
    })
}

#[test]
fn target_position_adds_the_time_since_the_stamp_in_the_host_clock() {
    let pb = JamPlayback { is_playing: true, position_ms: 10_000, host_clock_ms: 1_000_000, ..Default::default() };
    // our clock is 500 ms behind the host's
    assert_eq!(Follower::target_position(&pb, 1_001_500, 500), 12_000);
    let paused = JamPlayback { is_playing: false, ..pb.clone() };
    assert_eq!(Follower::target_position(&paused, 1_001_500, 500), 10_000);
    // a stamp from the future never goes backwards
    assert_eq!(Follower::target_position(&pb, 900_000, 0), 10_000);
}

#[test]
fn follower_loads_the_hosts_track_seeks_and_corrects_drift() {
    let me = FakePlayer::default();
    let mut f = Follower::new("me");
    let now = 2_000_000;
    // pending: nothing plays
    f.absorb(&payload("bbb", true, 30_000, now - 1_000, &["aaa", "bbb", "ccc"], 1, true));
    f.apply(&me, now, 0, true);
    assert!(me.entries().is_empty());

    f.absorb(&payload("bbb", true, 30_000, now - 1_000, &["aaa", "bbb", "ccc"], 1, false));
    f.apply(&me, now, 0, true);
    assert_eq!(me.entries(), ["play_items youtube:track:aaa,youtube:track:bbb,youtube:track:ccc @1", "seek 31000"]);

    // in step: nothing to do
    f.apply(&me, now, 0, false);
    assert_eq!(me.entries().len(), 2);

    // the host paused: we pause
    f.absorb(&payload("bbb", false, 31_000, now, &["aaa", "bbb", "ccc"], 1, false));
    f.apply(&me, now, 0, true);
    assert_eq!(me.entries().last().unwrap(), "pause");

    // far off, but just after a hard seek: wait; the drift is fixed once 4 s have passed (simulated by a new follower)
    me.snap.lock().unwrap().position_ms = 90_000;
    f.apply(&me, now, 0, true);
    assert_eq!(me.entries().last().unwrap(), "pause");
    let mut later = Follower::new("me");
    later.absorb(&payload("bbb", false, 31_000, now, &["aaa", "bbb", "ccc"], 1, false));
    later.apply(&me, now, 0, false);
    assert_eq!(me.entries().last().unwrap(), "seek 31000");
}

#[test]
fn follower_skips_what_it_cant_play_and_plays_a_track_missing_from_the_queue() {
    let me = FakePlayer::default();
    let mut f = Follower::new("me");
    let mut body = payload("zzz", true, 0, 0, &["aaa"], 0, false);
    // a JioSaavn row without a lane for it is dropped
    body["queue"]["items"].as_array_mut().unwrap().push(queue_item_to_json(&JamQueueItem { track_id: "saavn_q".into(), source: "JIOSAAVN".into(), ..Default::default() }));
    f.absorb(&body);
    f.apply(&me, now_ms(), 0, true);
    assert_eq!(me.entries(), ["play_items youtube:track:zzz,youtube:track:aaa @0"]);
}

// ---------------- the service, host and guest in one process ----------------

fn loopback_net() -> NetConfig {
    NetConfig { bind: LOOPBACK, discovery_bind: SocketAddr::new(LOOPBACK, 0), lan_ip: Some(Ipv4Addr::LOCALHOST), beacon_targets: vec![] }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn service_hosts_and_joins_over_loopback() {
    let rt = tokio::runtime::Handle::current();
    let host_player = FakePlayer::playing(&[yt("aaa"), yt("bbb")], 0, 12_000);
    let guest_player = Arc::new(FakePlayer::default());
    let host = LanJamService::new(rt.clone(), host_player.clone(), loopback_net());
    let guest = LanJamService::new(rt, guest_player.clone(), loopback_net());

    host.host("Host PC");
    eventually("hosting", Duration::from_secs(3), || host.snapshot().status == "joined").await;
    let s = host.snapshot();
    assert!(s.is_host && s.room_code.len() == 6 && s.address.starts_with("127.0.0.1:"));
    assert!(s.invite_url.contains(&format!("/jam/{}?localHost=127.0.0.1&localPort=", s.room_code)));
    assert_eq!(s.members.len(), 1);
    assert!(s.members[0].is_you && s.members[0].is_host);

    // a bad code is refused before anything starts
    guest.join("?", "Guest");
    assert!(!guest.snapshot().error.is_empty());

    guest.join(&s.invite_url, "Guest");
    eventually("joined", Duration::from_secs(5), || guest.snapshot().status == "joined").await;
    eventually("guest plays the host's track", Duration::from_secs(3), || {
        guest_player.entries().first().is_some_and(|e| e == "play_items youtube:track:aaa,youtube:track:bbb @0")
    })
    .await;
    let g = guest.snapshot();
    assert_eq!(g.host_name, "Host PC");
    assert_eq!(g.room_code, s.room_code);
    assert!(g.members.iter().any(|m| m.is_you && !m.is_host) && g.members.iter().any(|m| m.is_host && m.name == "Host PC"));
    eventually("host sees the guest", Duration::from_secs(3), || host.snapshot().members.len() == 2).await;

    // a guest's play button asks the host
    guest.toggle_play();
    eventually("host paused", Duration::from_secs(3), || host_player.entries().contains(&"pause".to_string())).await;

    // permissions reach the guest and are enforced on its side too
    host.set_room_settings(true, false, false);
    eventually("guest sees the setting", Duration::from_secs(6), || !guest.snapshot().can_control).await;
    guest.control(JamControl::SkipNext);
    assert!(!guest.snapshot().error.is_empty());

    guest.leave();
    eventually("host sees the guest leave", Duration::from_secs(3), || host.snapshot().members.len() == 1).await;
    assert_eq!(guest.snapshot().status, "idle");
    host.leave();
    assert_eq!(host.snapshot().status, "idle");
}
