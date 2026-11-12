//! Jam wire format, field for field the Kotlin apps' `JamProtocol` (Android `data/jam/model/JamProtocol.kt`,
//! desktop `JamSession.kt`): playback, queue, room, guest controls, and the beacon. Readers follow `org.json`'s
//! `opt*` rules: a missing or mistyped field gets its default, and numbers and booleans also come as strings
//! (LAN controls arrive as query parameters).
use serde_json::{Map, Value, json};

pub const VERSION: i64 = 1;
/// UDP port the host's beacon goes to and guests listen on.
pub const BEACON_PORT: u16 = 58245;
pub const BEACON_MAGIC: &str = "soundwave_jam";
/// A `/sync` with an unchanged `since` is held this long.
pub const LONG_POLL_MS: u64 = 4_000;
/// A guest that can't reach its host for this long gives up.
pub const HOST_LOST_MS: u64 = 20_000;
pub const BEACON_INTERVAL_MS: u64 = 3_000;
/// Room codes: no 0/O, 1/I.
pub const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

pub const SOURCE_YOUTUBE: &str = "YOUTUBE";
pub const SOURCE_JIOSAAVN: &str = "JIOSAAVN";
pub const SOURCE_LOCAL: &str = "LOCAL";
/// Not in the Kotlin apps: a Spotify track from a Materialfy host (Kotlin guests treat it as unplayable).
pub const SOURCE_SPOTIFY: &str = "SPOTIFY";

// ---- lenient readers (org.json opt*) ----

pub fn opt_str(j: &Value, key: &str) -> String {
    match j.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn opt_str_or(j: &Value, key: &str, default: &str) -> String {
    match j.get(key) {
        Some(Value::Null) | None => default.to_owned(),
        _ => opt_str(j, key),
    }
}

pub fn opt_i64(j: &Value, key: &str) -> i64 {
    opt_i64_or(j, key, 0)
}

fn opt_i64_or(j: &Value, key: &str, default: i64) -> i64 {
    match j.get(key) {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(default),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok().or_else(|| s.trim().parse::<f64>().ok().map(|f| f as i64)).unwrap_or(default),
        _ => default,
    }
}

pub fn opt_bool(j: &Value, key: &str) -> bool {
    opt_bool_or(j, key, false)
}

fn opt_bool_or(j: &Value, key: &str, default: bool) -> bool {
    match j.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) if s.eq_ignore_ascii_case("true") => true,
        Some(Value::String(s)) if s.eq_ignore_ascii_case("false") => false,
        _ => default,
    }
}

fn non_blank(s: String) -> Option<String> {
    (!s.trim().is_empty()).then_some(s)
}

fn objects(j: &Value, key: &str) -> Vec<Value> {
    j.get(key).and_then(Value::as_array).map(|a| a.iter().filter(|v| v.is_object()).cloned().collect()).unwrap_or_default()
}

// ---- playback ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JamPlayback {
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub artwork_url: String,
    pub source: String,
    pub is_playing: bool,
    pub position_ms: i64,
    /// Ably server-time domain (on a LAN-only host: the host's wall clock)
    pub server_timestamp: i64,
    /// the host's wall clock at capture; LAN guests pair it with their measured clock offset
    pub host_clock_ms: i64,
    pub duration_ms: i64,
    pub youtube_id: Option<String>,
    pub content_uri: Option<String>,
}

impl Default for JamPlayback {
    fn default() -> Self {
        Self {
            track_id: String::new(),
            title: String::new(),
            artist: String::new(),
            artwork_url: String::new(),
            source: SOURCE_YOUTUBE.to_owned(),
            is_playing: false,
            position_ms: 0,
            server_timestamp: 0,
            host_clock_ms: 0,
            duration_ms: 0,
            youtube_id: None,
            content_uri: None,
        }
    }
}

pub fn playback_to_json(p: &JamPlayback) -> Value {
    json!({
        "trackId": p.track_id, "title": p.title, "artist": p.artist, "artworkUrl": p.artwork_url, "source": p.source,
        "isPlaying": p.is_playing, "positionMs": p.position_ms, "serverTimestamp": p.server_timestamp,
        "hostClockMs": p.host_clock_ms, "durationMs": p.duration_ms,
        "youtubeId": p.youtube_id.clone().unwrap_or_default(), "contentUri": p.content_uri.clone().unwrap_or_default(),
    })
}

pub fn playback_from_json(j: &Value) -> JamPlayback {
    JamPlayback {
        track_id: opt_str(j, "trackId"),
        title: opt_str(j, "title"),
        artist: opt_str(j, "artist"),
        artwork_url: opt_str(j, "artworkUrl"),
        source: opt_str_or(j, "source", SOURCE_YOUTUBE),
        is_playing: opt_bool(j, "isPlaying"),
        position_ms: opt_i64(j, "positionMs"),
        server_timestamp: opt_i64(j, "serverTimestamp"),
        host_clock_ms: opt_i64(j, "hostClockMs"),
        duration_ms: opt_i64(j, "durationMs"),
        youtube_id: non_blank(opt_str(j, "youtubeId")),
        content_uri: non_blank(opt_str(j, "contentUri")),
    }
}

// ---- queue ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JamQueueItem {
    pub queue_id: String,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub artwork_url: String,
    pub source: String,
    pub duration_ms: i64,
    pub youtube_id: Option<String>,
    pub content_uri: Option<String>,
    pub added_by: String,
    pub added_by_name: String,
    pub added_at: i64,
    pub is_play_next: bool,
}

impl Default for JamQueueItem {
    fn default() -> Self {
        Self {
            queue_id: String::new(),
            track_id: String::new(),
            title: String::new(),
            artist: String::new(),
            artwork_url: String::new(),
            source: SOURCE_YOUTUBE.to_owned(),
            duration_ms: 0,
            youtube_id: None,
            content_uri: None,
            added_by: String::new(),
            added_by_name: String::new(),
            added_at: 0,
            is_play_next: false,
        }
    }
}

pub fn queue_item_to_json(q: &JamQueueItem) -> Value {
    json!({
        "queueId": q.queue_id, "trackId": q.track_id, "title": q.title, "artist": q.artist, "artworkUrl": q.artwork_url,
        "source": q.source, "durationMs": q.duration_ms,
        "youtubeId": q.youtube_id.clone().unwrap_or_default(), "contentUri": q.content_uri.clone().unwrap_or_default(),
        "addedBy": q.added_by, "addedByName": q.added_by_name, "addedAt": q.added_at, "isPlayNext": q.is_play_next,
    })
}

pub fn queue_item_from_json(j: &Value) -> JamQueueItem {
    JamQueueItem {
        queue_id: opt_str(j, "queueId"),
        track_id: opt_str(j, "trackId"),
        title: opt_str(j, "title"),
        artist: opt_str(j, "artist"),
        artwork_url: opt_str(j, "artworkUrl"),
        source: opt_str_or(j, "source", SOURCE_YOUTUBE),
        duration_ms: opt_i64(j, "durationMs"),
        youtube_id: non_blank(opt_str(j, "youtubeId")),
        content_uri: non_blank(opt_str(j, "contentUri")),
        added_by: opt_str(j, "addedBy"),
        added_by_name: opt_str(j, "addedByName"),
        added_at: opt_i64(j, "addedAt"),
        is_play_next: opt_bool(j, "isPlayNext"),
    }
}

/// `items` may be a window of the full queue; `current_index` is relative to `items`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JamQueue {
    pub items: Vec<JamQueueItem>,
    pub current_index: i64,
    /// 0 off, 1 one, 2 all
    pub repeat_mode: i64,
    pub shuffle_enabled: bool,
    pub total_count: i64,
}

pub fn queue_to_json(q: &JamQueue) -> Value {
    json!({
        "v": VERSION, "items": q.items.iter().map(queue_item_to_json).collect::<Vec<_>>(), "currentIndex": q.current_index,
        "repeatMode": q.repeat_mode, "shuffleEnabled": q.shuffle_enabled, "totalCount": q.total_count,
    })
}

pub fn queue_from_json(j: &Value) -> JamQueue {
    JamQueue {
        items: objects(j, "items").iter().map(queue_item_from_json).collect(),
        current_index: opt_i64(j, "currentIndex"),
        repeat_mode: opt_i64(j, "repeatMode"),
        shuffle_enabled: opt_bool(j, "shuffleEnabled"),
        total_count: opt_i64(j, "totalCount"),
    }
}

// ---- room ----

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JamParticipant {
    pub user_id: String,
    pub name: String,
    pub is_host: bool,
    pub is_pending: bool,
    pub joined_at: i64,
    pub device: String,
}

pub fn participant_to_json(p: &JamParticipant) -> Value {
    json!({ "userId": p.user_id, "name": p.name, "isHost": p.is_host, "isPending": p.is_pending, "joinedAt": p.joined_at, "device": p.device })
}

pub fn participant_from_json(j: &Value) -> JamParticipant {
    JamParticipant {
        user_id: opt_str(j, "userId"),
        name: opt_str_or(j, "name", "Guest"),
        is_host: opt_bool(j, "isHost"),
        is_pending: opt_bool(j, "isPending"),
        joined_at: opt_i64(j, "joinedAt"),
        device: opt_str(j, "device"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JamRoomSettings {
    pub allow_guests_to_add_tracks: bool,
    pub allow_guests_to_control_playback: bool,
    pub require_host_approval_to_join: bool,
}

impl Default for JamRoomSettings {
    fn default() -> Self {
        Self { allow_guests_to_add_tracks: true, allow_guests_to_control_playback: true, require_host_approval_to_join: false }
    }
}

pub fn settings_to_json(s: &JamRoomSettings) -> Value {
    json!({
        "allowGuestsToAddTracks": s.allow_guests_to_add_tracks,
        "allowGuestsToControlPlayback": s.allow_guests_to_control_playback,
        "requireHostApprovalToJoin": s.require_host_approval_to_join,
    })
}

pub fn settings_from_json(j: Option<&Value>) -> JamRoomSettings {
    let Some(j) = j.filter(|j| j.is_object()) else { return JamRoomSettings::default() };
    JamRoomSettings {
        allow_guests_to_add_tracks: opt_bool_or(j, "allowGuestsToAddTracks", true),
        allow_guests_to_control_playback: opt_bool_or(j, "allowGuestsToControlPlayback", true),
        require_host_approval_to_join: opt_bool_or(j, "requireHostApprovalToJoin", false),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JamLogItem {
    pub id: String,
    /// "play", "pause", "skip", "join", "leave", "queue", "info", …
    pub icon_type: String,
    pub message: String,
    pub timestamp: i64,
}

pub fn log_item_to_json(l: &JamLogItem) -> Value {
    json!({ "id": l.id, "iconType": l.icon_type, "message": l.message, "timestamp": l.timestamp })
}

pub fn log_item_from_json(j: &Value) -> JamLogItem {
    JamLogItem {
        id: opt_str(j, "id"),
        icon_type: opt_str_or(j, "iconType", "info"),
        message: opt_str(j, "message"),
        timestamp: opt_i64(j, "timestamp"),
    }
}

/// Room metadata (everything but playback and queue).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JamRoom {
    pub room_id: String,
    pub host_id: String,
    pub host_name: String,
    pub created_at: i64,
    pub settings: JamRoomSettings,
    pub participants: Vec<JamParticipant>,
    pub activity_log: Vec<JamLogItem>,
    pub lan_host_ip: Option<String>,
    pub lan_port: Option<u16>,
    pub is_online: bool,
}

pub fn room_to_json(r: &JamRoom) -> Value {
    json!({
        "v": VERSION, "roomId": r.room_id, "hostId": r.host_id, "hostName": r.host_name, "createdAt": r.created_at,
        "settings": settings_to_json(&r.settings),
        "participants": r.participants.iter().map(participant_to_json).collect::<Vec<_>>(),
        "activityLog": r.activity_log.iter().map(log_item_to_json).collect::<Vec<_>>(),
        "lanHostIp": r.lan_host_ip.clone().unwrap_or_default(), "lanPort": r.lan_port.unwrap_or(0), "isOnline": r.is_online,
    })
}

/// Like `JamProtocol.applyRoomJson`: fields missing from `j` keep `base`'s value (participants and log are
/// replaced; `isOnline` defaults to true, as in Kotlin).
pub fn apply_room_json(base: Option<&JamRoom>, j: &Value) -> JamRoom {
    let from = base.cloned().unwrap_or_default();
    let has = |k: &str| j.get(k).is_some_and(|v| !v.is_null());
    let mut participants: Vec<JamParticipant> = Vec::new();
    for p in objects(j, "participants").iter().map(participant_from_json) {
        // associateBy: a later entry with the same id replaces the earlier one
        match participants.iter_mut().find(|q| q.user_id == p.user_id) {
            Some(slot) => *slot = p,
            None => participants.push(p),
        }
    }
    JamRoom {
        room_id: if has("roomId") { opt_str(j, "roomId") } else { from.room_id },
        host_id: if has("hostId") { opt_str(j, "hostId") } else { from.host_id },
        host_name: if has("hostName") { opt_str(j, "hostName") } else { from.host_name },
        created_at: opt_i64_or(j, "createdAt", from.created_at),
        settings: settings_from_json(j.get("settings")),
        participants,
        activity_log: objects(j, "activityLog").iter().map(log_item_from_json).collect(),
        lan_host_ip: non_blank(opt_str(j, "lanHostIp")),
        lan_port: u16::try_from(opt_i64(j, "lanPort")).ok().filter(|p| *p > 0),
        is_online: opt_bool_or(j, "isOnline", true),
    }
}

// ---- guest controls ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JamControl {
    Play,
    Pause,
    SkipNext,
    SkipPrevious,
    SeekTo(i64),
    SeekToIndex { index: i64, position_ms: i64 },
    SeekToTrack { track_id: String, position_ms: i64 },
    SetShuffle(bool),
    /// 0 off, 1 one, 2 all
    SetRepeat(i64),
    /// `play_now`: a guest tapped the song to play it: insert next and jump to it.
    AddTrack { track: Box<JamQueueItem>, is_play_next: bool, play_now: bool },
}

pub fn control_to_json(action: &JamControl, sender_id: &str, sender_name: &str) -> Value {
    let mut j = Map::new();
    j.insert("senderId".into(), sender_id.into());
    j.insert("guestName".into(), sender_name.into());
    let mut put = |k: &str, v: Value| {
        j.insert(k.to_owned(), v);
    };
    match action {
        JamControl::Play => put("type", "play".into()),
        JamControl::Pause => put("type", "pause".into()),
        JamControl::SkipNext => put("type", "skipNext".into()),
        JamControl::SkipPrevious => put("type", "skipPrevious".into()),
        JamControl::SeekTo(ms) => {
            put("type", "seekTo".into());
            put("positionMs", (*ms).into());
        }
        JamControl::SeekToIndex { index, position_ms } => {
            put("type", "seekToIndex".into());
            put("index", (*index).into());
            put("positionMs", (*position_ms).into());
        }
        JamControl::SeekToTrack { track_id, position_ms } => {
            put("type", "seekToTrack".into());
            put("trackId", track_id.as_str().into());
            put("positionMs", (*position_ms).into());
        }
        JamControl::SetShuffle(enabled) => {
            put("type", "setShuffle".into());
            put("enabled", (*enabled).into());
        }
        JamControl::SetRepeat(mode) => {
            put("type", "setRepeat".into());
            put("mode", (*mode).into());
        }
        JamControl::AddTrack { track, is_play_next, play_now } => {
            put("type", "addTrack".into());
            put("isPlayNext", (*is_play_next).into());
            put("playNow", (*play_now).into());
            put("track", queue_item_to_json(track));
        }
    }
    Value::Object(j)
}

/// `now_ms` stamps an added track's new `queueId` (`q-<now>`) and `addedAt`, as the Kotlin host does.
pub fn control_from_json(j: &Value, now_ms: i64) -> Option<JamControl> {
    Some(match opt_str(j, "type").as_str() {
        "play" => JamControl::Play,
        "pause" => JamControl::Pause,
        "skipNext" => JamControl::SkipNext,
        "skipPrevious" => JamControl::SkipPrevious,
        "seekTo" => JamControl::SeekTo(opt_i64(j, "positionMs")),
        "seekToIndex" => JamControl::SeekToIndex { index: opt_i64(j, "index"), position_ms: opt_i64(j, "positionMs") },
        "seekToTrack" => JamControl::SeekToTrack { track_id: opt_str(j, "trackId"), position_ms: opt_i64(j, "positionMs") },
        "setShuffle" => JamControl::SetShuffle(opt_bool(j, "enabled")),
        "setRepeat" => JamControl::SetRepeat(opt_i64(j, "mode")),
        "addTrack" => {
            let track = j.get("track").filter(|t| t.is_object())?;
            let is_play_next = opt_bool(j, "isPlayNext");
            let track = JamQueueItem {
                queue_id: format!("q-{now_ms}"),
                added_at: now_ms,
                is_play_next,
                ..queue_item_from_json(track)
            };
            JamControl::AddTrack { track: Box::new(track), is_play_next, play_now: opt_bool(j, "playNow") }
        }
        _ => return None,
    })
}

/// Guest side of a LAN control: the control's fields, then a nested `track`'s, as flat `key=value` pairs (what
/// both Kotlin guests send to `/control`).
pub fn control_to_params(control: &Value) -> Vec<(String, String)> {
    let text = |v: &Value| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let mut out = Vec::new();
    if let Some(map) = control.as_object() {
        for (k, v) in map.iter().filter(|(k, _)| k.as_str() != "track") {
            out.push((k.clone(), text(v)));
        }
        if let Some(track) = map.get("track").and_then(Value::as_object) {
            for (k, v) in track {
                out.push((k.clone(), text(v)));
            }
        }
    }
    out
}

/// Host side of a LAN control (Android `LocalWifiJamEngine.parseAction`): flat parameters back into a control
/// object. `/queue` and older clients send `action=` or bare track parameters.
pub fn control_from_params(params: &[(String, String)]) -> Value {
    let mut j = Map::new();
    // a key that comes twice (the control's `isPlayNext` and the track's): the first wins, as Android's
    // `Uri.getQueryParameter`
    for (k, v) in params {
        j.entry(k.clone()).or_insert_with(|| Value::String(v.clone()));
    }
    let get = |k: &str| j.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let mut kind = get("type");
    if kind.trim().is_empty() {
        kind = get("action");
        if kind.trim().is_empty() && j.contains_key("title") {
            kind = "addTrack".into();
        }
    }
    if kind == "addTrack" && !j.contains_key("track") {
        let track = json!({
            "trackId": get("trackId"), "title": get("title"), "artist": get("artist"), "artworkUrl": get("artworkUrl"),
            "source": if j.contains_key("source") { get("source") } else { SOURCE_YOUTUBE.to_owned() },
            "durationMs": get("durationMs").parse::<i64>().unwrap_or(0),
            "youtubeId": get("youtubeId"), "contentUri": get("contentUri"),
        });
        j.insert("track".into(), track);
    }
    j.insert("type".into(), kind.into());
    Value::Object(j)
}

// ---- beacon ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Beacon {
    pub room_id: String,
    pub host_name: String,
    pub port: u16,
}

pub fn beacon_to_bytes(b: &Beacon) -> Vec<u8> {
    json!({ "app": BEACON_MAGIC, "roomId": b.room_id, "hostName": b.host_name, "port": b.port }).to_string().into_bytes()
}

pub fn beacon_from_bytes(bytes: &[u8]) -> Option<Beacon> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if !text.starts_with('{') || !text.contains(BEACON_MAGIC) {
        return None;
    }
    let j: Value = serde_json::from_str(text).ok()?;
    if opt_str(&j, "app") != BEACON_MAGIC {
        return None;
    }
    let room_id = opt_str(&j, "roomId");
    let port = u16::try_from(opt_i64(&j, "port")).ok().filter(|p| *p > 0)?;
    if room_id.trim().is_empty() {
        return None;
    }
    Some(Beacon { room_id, host_name: opt_str_or(&j, "hostName", "Nearby friend"), port })
}

// ---- track identity ----

/// Android `JamTrackSource.youtubeIdOf`: the explicit id, a `youtube:` content URI, or a `youtube_` track id.
pub fn youtube_id_of(track_id: &str, content_uri: Option<&str>, explicit: Option<&str>) -> Option<String> {
    if let Some(id) = explicit.filter(|s| !s.trim().is_empty()) {
        return Some(id.to_owned());
    }
    if let Some(uri) = content_uri.filter(|u| u.starts_with("youtube:")) {
        let rest = uri.strip_prefix("youtube://").or_else(|| uri.strip_prefix("youtube:")).unwrap_or(uri);
        let id = rest.split('?').next().unwrap_or_default();
        if !id.trim().is_empty() {
            return Some(id.to_owned());
        }
    }
    track_id.strip_prefix("youtube_").filter(|id| !id.trim().is_empty()).map(str::to_owned)
}

/// Android `JamTrackSource.classify`.
pub fn classify(track_id: &str, content_uri: Option<&str>, explicit: Option<&str>) -> &'static str {
    if youtube_id_of(track_id, content_uri, explicit).is_some() {
        SOURCE_YOUTUBE
    } else if track_id.starts_with("saavn_")
        || content_uri.is_some_and(|u| u.starts_with("saavn:") || u.contains("saavncdn.com"))
    {
        SOURCE_JIOSAAVN
    } else {
        SOURCE_LOCAL
    }
}

/// What a guest sees for one of this computer's player URIs: (trackId, source, youtubeId).
/// `youtube:track:X` → `X` (as the Kotlin desktop host sends it), `saavn:track:X` → `saavn_X`, a local file →
/// `local_X`, a Spotify track → its URI.
pub fn wire_track(uri: &str) -> (String, &'static str, Option<String>) {
    if let Some(id) = uri.strip_prefix("youtube:track:") {
        (id.to_owned(), SOURCE_YOUTUBE, Some(id.to_owned()))
    } else if let Some(id) = uri.strip_prefix("saavn:track:") {
        (format!("saavn_{id}"), SOURCE_JIOSAAVN, None)
    } else if let Some(id) = uri.strip_prefix("local:track:") {
        (format!("local_{id}"), SOURCE_LOCAL, None)
    } else if uri.starts_with("spotify:") {
        (uri.to_owned(), SOURCE_SPOTIFY, None)
    } else {
        (uri.to_owned(), SOURCE_LOCAL, None)
    }
}

/// The player URI a guest plays for a host's track, or None when this computer can't fetch it.
/// `lane_plays` says whether the player's lane takes a URI (JioSaavn works once the lane accepts `saavn:track:`).
pub fn local_uri(
    track_id: &str,
    source: &str,
    youtube_id: Option<&str>,
    content_uri: Option<&str>,
    lane_plays: impl Fn(&str) -> bool,
) -> Option<String> {
    if let Some(saavn) = youtube_id.and_then(|y| y.strip_prefix("saavn_").or_else(|| y.strip_prefix("saavn:"))) {
        let uri = format!("saavn:track:{saavn}");
        return lane_plays(&uri).then_some(uri);
    }
    if let Some(id) = youtube_id_of(track_id, content_uri, youtube_id) {
        return Some(format!("youtube:track:{id}"));
    }
    if source == SOURCE_SPOTIFY && track_id.starts_with("spotify:track:") {
        return Some(track_id.to_owned());
    }
    if let Some(id) = track_id.strip_prefix("saavn_").filter(|id| !id.is_empty()) {
        let uri = format!("saavn:track:{id}");
        return lane_plays(&uri).then_some(uri);
    }
    None
}

/// Room code typed by a person: dashes and spaces dropped, upper case, 4–10 of [A-Z0-9].
pub fn normalize_code(input: &str) -> Option<String> {
    let code: String = input.chars().filter(|c| *c != '-' && !c.is_whitespace()).collect::<String>().to_uppercase();
    (code.len() >= 4 && code.len() <= 10 && code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())).then_some(code)
}

/// What a guest typed: a room code, or an invite link `…/jam/CODE?localHost=IP&localPort=PORT` (the Kotlin
/// hosts' Wi-Fi-only link). Returns (code, direct address).
pub fn parse_join(input: &str) -> Option<(String, Option<(String, u16)>)> {
    let text = input.trim();
    if !(text.starts_with("http://") || text.starts_with("https://") || text.starts_with("soundwave://")) {
        return normalize_code(text).map(|c| (c, None));
    }
    let after_scheme = text.split_once("://").map(|(_, r)| r).unwrap_or(text);
    let without_fragment = after_scheme.split('#').next().unwrap_or_default();
    let (path, query) = without_fragment.split_once('?').unwrap_or((without_fragment, ""));
    let code = path.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_uppercase();
    let code = normalize_code(&code)?;
    let params = super::http::parse_query(query);
    let get = |k: &str| params.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    let direct = match (get("localHost").filter(|h| !h.is_empty()), get("localPort").and_then(|p| p.parse::<u16>().ok())) {
        (Some(host), Some(port)) if port > 0 => Some((host, port)),
        _ => None,
    };
    Some((code, direct))
}
