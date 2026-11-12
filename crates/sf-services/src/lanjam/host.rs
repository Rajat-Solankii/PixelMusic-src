//! Host side of the same-Wi-Fi Jam: the HTTP server guests talk to (`/join`, `/sync` long-poll, `/control`,
//! `/queue`, `/leave`, `/time`) and the UDP beacon that lets them find the room by its code. The local player is
//! the source of truth: guests get its track, position and queue, and their controls are applied to it.
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use md3_model::{PlayContext, QueueRow, QueueSection, TrackItem};
use serde_json::{Value, json};
use sf_player::{PlayerSnapshot, Repeat};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::http::{self, Request};
use super::protocol::{self as p, JamControl, JamLogItem, JamParticipant, JamPlayback, JamQueue, JamQueueItem, JamRoom, JamRoomSettings};
use super::{JamPlayer, now_ms};

/// Guests that sent nothing for this long are dropped (app closed, network gone).
const GUEST_TIMEOUT: Duration = Duration::from_secs(90);
/// How often the host looks at its player for changes guests must hear about.
const PLAYER_POLL: Duration = Duration::from_millis(300);
/// A position this far from where playback should be is a seek.
const SEEK_JUMP_MS: i64 = 1_500;
const MAX_LOG: usize = 40;

#[derive(Clone, Debug)]
pub struct HostConfig {
    pub room_id: String,
    pub host_id: String,
    pub host_name: String,
    pub device: String,
    /// this computer's LAN address, shown to guests in the room and the invite link
    pub lan_ip: Option<String>,
    /// address the server listens on: unspecified in the app, loopback in tests
    pub bind: IpAddr,
    /// where the beacon goes every 3 s (broadcast addresses in the app)
    pub beacon_targets: Vec<SocketAddr>,
    pub settings: JamRoomSettings,
}

struct Guests {
    /// in join order
    list: Vec<JamParticipant>,
    tokens: HashMap<String, String>,
    banned: HashSet<String>,
    last_seen: HashMap<String, Instant>,
    log: Vec<JamLogItem>,
    settings: JamRoomSettings,
    host_name: String,
}

struct Inner {
    config: HostConfig,
    created_at: i64,
    port: u16,
    guests: Mutex<Guests>,
    /// bumped on every change guests must see; `/sync?since=` waits on it
    version: watch::Sender<u64>,
    player: Arc<dyn JamPlayer>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

/// A running host. Dropping it does not stop it; call `stop`.
#[derive(Clone)]
pub struct LanHost {
    inner: Arc<Inner>,
}

fn token() -> String {
    rand::random::<[u8; 16]>().iter().map(|b| format!("{b:02x}")).collect()
}

fn error(message: &str) -> Value {
    json!({ "error": message })
}

impl LanHost {
    /// Binds the server on a free port and starts it, the beacon, the player watcher and the guest timeout.
    pub async fn start(config: HostConfig, player: Arc<dyn JamPlayer>) -> std::io::Result<LanHost> {
        let listener = TcpListener::bind((config.bind, 0)).await?;
        let port = listener.local_addr()?.port();
        let created_at = now_ms();
        let guests = Guests {
            list: Vec::new(),
            tokens: HashMap::new(),
            banned: HashSet::new(),
            last_seen: HashMap::new(),
            log: Vec::new(),
            settings: config.settings.clone(),
            host_name: config.host_name.clone(),
        };
        let (version, _) = watch::channel(0u64);
        let host = LanHost {
            inner: Arc::new(Inner { config, created_at, port, guests: Mutex::new(guests), version, player, tasks: Mutex::new(Vec::new()) }),
        };
        let name = host.inner.config.host_name.clone();
        host.log("fire", mf_i18n::tf("Session started by {0}", &[&name]));
        let tasks = vec![
            tokio::spawn(accept_loop(host.clone(), listener)),
            tokio::spawn(beacon_loop(host.clone())),
            tokio::spawn(player_watch(host.clone())),
            tokio::spawn(timeout_loop(host.clone())),
        ];
        *host.inner.tasks.lock().unwrap() = tasks;
        Ok(host)
    }

    pub fn stop(&self) {
        for task in self.inner.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }

    pub fn port(&self) -> u16 {
        self.inner.port
    }

    pub fn room_id(&self) -> &str {
        &self.inner.config.room_id
    }

    /// Changes to the room (guests, settings) and to playback; the service redraws from `participants`.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.inner.version.subscribe()
    }

    pub fn version(&self) -> u64 {
        *self.inner.version.borrow()
    }

    /// Wakes every waiting long-poll.
    pub fn notify_changed(&self) {
        self.inner.version.send_modify(|v| *v += 1);
    }

    /// The host first, then guests in join order.
    pub fn participants(&self) -> Vec<JamParticipant> {
        let c = &self.inner.config;
        let g = self.inner.guests.lock().unwrap();
        let mut out = vec![JamParticipant {
            user_id: c.host_id.clone(),
            name: g.host_name.clone(),
            is_host: true,
            is_pending: false,
            joined_at: self.inner.created_at,
            device: c.device.clone(),
        }];
        out.extend(g.list.iter().cloned());
        out
    }

    pub fn settings(&self) -> JamRoomSettings {
        self.inner.guests.lock().unwrap().settings.clone()
    }

    pub fn set_settings(&self, settings: JamRoomSettings) {
        self.inner.guests.lock().unwrap().settings = settings;
        self.notify_changed();
    }

    pub fn approve(&self, user_id: &str) {
        let name = {
            let mut g = self.inner.guests.lock().unwrap();
            let Some(guest) = g.list.iter_mut().find(|p| p.user_id == user_id) else { return };
            guest.is_pending = false;
            guest.name.clone()
        };
        self.log("check", mf_i18n::tf("{0} was approved", &[&name]));
        self.notify_changed();
    }

    /// Removes and bans a guest for the rest of this Jam: its token stops working at once.
    pub fn remove(&self, user_id: &str) {
        let removed = {
            let mut g = self.inner.guests.lock().unwrap();
            g.banned.insert(user_id.to_owned());
            g.tokens.remove(user_id);
            g.last_seen.remove(user_id);
            let at = g.list.iter().position(|p| p.user_id == user_id);
            at.map(|i| g.list.remove(i))
        };
        if let Some(guest) = removed {
            self.log("close", mf_i18n::tf("{0} was removed", &[&guest.name]));
        }
        self.notify_changed();
    }

    fn log(&self, icon: &str, message: String) {
        let mut g = self.inner.guests.lock().unwrap();
        let id = format!("{:016x}", rand::random::<u64>());
        g.log.push(JamLogItem { id, icon_type: icon.to_owned(), message, timestamp: now_ms() });
        let excess = g.log.len().saturating_sub(MAX_LOG);
        g.log.drain(..excess);
    }

    fn room(&self) -> JamRoom {
        let c = &self.inner.config;
        let participants = self.participants();
        let g = self.inner.guests.lock().unwrap();
        JamRoom {
            room_id: c.room_id.clone(),
            host_id: c.host_id.clone(),
            host_name: g.host_name.clone(),
            created_at: self.inner.created_at,
            settings: g.settings.clone(),
            participants,
            activity_log: g.log.clone(),
            lan_host_ip: c.lan_ip.clone(),
            lan_port: Some(self.inner.port),
            is_online: false,
        }
    }

    /// What `/join` and `/sync` return: `{v, version, serverTime, room, playback?, queue}`.
    pub fn payload(&self) -> Value {
        let snapshot = self.inner.player.snapshot();
        let mut body = json!({
            "v": p::VERSION,
            "version": self.version(),
            "serverTime": now_ms(),
            "room": p::room_to_json(&self.room()),
            "queue": p::queue_to_json(&queue_of(&snapshot, self.inner.created_at)),
        });
        if let Some(playback) = playback_of(&snapshot, now_ms()) {
            body["playback"] = p::playback_to_json(&playback);
        }
        body
    }

    /// A guest with a valid token that is still in the room (not banned).
    fn authorized(&self, req: &Request) -> Option<String> {
        let uid = req.param("uid").filter(|u| !u.trim().is_empty())?;
        let token = req.param("token")?;
        let g = self.inner.guests.lock().unwrap();
        let ok = !g.banned.contains(uid) && g.tokens.get(uid).is_some_and(|t| t == token) && g.list.iter().any(|p| p.user_id == uid);
        ok.then(|| uid.to_owned())
    }

    fn seen(&self, uid: &str) {
        self.inner.guests.lock().unwrap().last_seen.insert(uid.to_owned(), Instant::now());
    }

    fn join(&self, req: &Request) -> (u16, Value) {
        let Some(uid) = req.param("uid").filter(|u| !u.trim().is_empty()).map(str::to_owned) else {
            return (403, error("missing uid"));
        };
        let name: String = req.param("name").filter(|n| !n.trim().is_empty()).unwrap_or("Guest").chars().take(40).collect();
        let device = req.param("device").unwrap_or_default().to_owned();
        let (token, joined) = {
            let mut g = self.inner.guests.lock().unwrap();
            if g.banned.contains(&uid) {
                return (403, error("banned"));
            }
            let pending_default = g.settings.require_host_approval_to_join;
            let existing = g.list.iter().position(|p| p.user_id == uid);
            let joined = match existing {
                Some(i) => {
                    let guest = &mut g.list[i];
                    guest.name = name.clone();
                    guest.device = device;
                    None
                }
                None => {
                    let guest = JamParticipant {
                        user_id: uid.clone(),
                        name: name.clone(),
                        is_host: false,
                        is_pending: pending_default,
                        joined_at: now_ms(),
                        device,
                    };
                    g.list.push(guest.clone());
                    Some(guest)
                }
            };
            g.last_seen.insert(uid.clone(), Instant::now());
            let token = g.tokens.entry(uid.clone()).or_insert_with(token).clone();
            (token, joined)
        };
        if let Some(guest) = joined {
            if guest.is_pending {
                self.log("person", mf_i18n::tf("{0} requested to join", &[&guest.name]));
            } else {
                self.log("join", mf_i18n::tf("{0} joined the Jam", &[&guest.name]));
            }
        }
        self.notify_changed();
        let mut body = self.payload();
        body["you"] = json!({ "userId": uid, "token": token });
        (200, body)
    }

    async fn sync(&self, req: &Request) -> (u16, Value) {
        let Some(uid) = self.authorized(req) else { return (403, error("not a member")) };
        self.seen(&uid);
        let since = req.param("since").and_then(|s| s.parse::<i64>().ok()).unwrap_or(-1);
        let mut rx = self.inner.version.subscribe();
        if since >= 0 && *rx.borrow_and_update() == since as u64 {
            let _ = tokio::time::timeout(Duration::from_millis(p::LONG_POLL_MS), rx.changed()).await;
        }
        // kicked while waiting
        if self.authorized(req).is_none() {
            return (403, error("not a member"));
        }
        (200, self.payload())
    }

    fn control(&self, req: &Request) -> (u16, Value) {
        let Some(uid) = self.authorized(req) else { return (403, error("not a member")) };
        self.seen(&uid);
        let control = p::control_from_json(&p::control_from_params(&req.query), now_ms());
        let ok = control.is_some();
        if let Some(control) = control {
            self.apply_control(control, &uid);
        }
        (200, json!({ "ok": ok }))
    }

    fn leave(&self, req: &Request) -> (u16, Value) {
        if let Some(uid) = self.authorized(req) {
            let removed = {
                let mut g = self.inner.guests.lock().unwrap();
                g.tokens.remove(&uid);
                g.last_seen.remove(&uid);
                let at = g.list.iter().position(|p| p.user_id == uid);
                at.map(|i| g.list.remove(i))
            };
            if let Some(guest) = removed {
                self.log("leave", mf_i18n::tf("{0} left the Jam", &[&guest.name]));
            }
            self.notify_changed();
        }
        (200, json!({ "ok": true }))
    }

    /// Host-side gate and effect of a guest control: unknown, banned and unapproved senders, and actions the room
    /// settings forbid, are dropped.
    pub fn apply_control(&self, control: JamControl, sender: &str) {
        let (name, settings) = {
            let g = self.inner.guests.lock().unwrap();
            if g.banned.contains(sender) {
                return;
            }
            let Some(guest) = g.list.iter().find(|p| p.user_id == sender && !p.is_pending) else { return };
            (guest.name.clone(), g.settings.clone())
        };
        let adding = matches!(control, JamControl::AddTrack { .. });
        if !(if adding { settings.allow_guests_to_add_tracks } else { settings.allow_guests_to_control_playback }) {
            return;
        }
        let player = &self.inner.player;
        let s = player.snapshot();
        match control {
            JamControl::Play => {
                if !s.playing {
                    player.play();
                    self.log("play", mf_i18n::tf("{0} resumed playback", &[&name]));
                }
            }
            JamControl::Pause => {
                if s.playing {
                    player.pause();
                    self.log("pause", mf_i18n::tf("{0} paused playback", &[&name]));
                }
            }
            JamControl::SkipNext => {
                player.next();
                self.log("skip", mf_i18n::tf("{0} skipped to next track", &[&name]));
            }
            JamControl::SkipPrevious => {
                player.previous();
                self.log("skip", mf_i18n::tf("{0} skipped to previous track", &[&name]));
            }
            JamControl::SeekTo(ms) => player.seek_ms(ms.max(0)),
            JamControl::SeekToIndex { index, position_ms } => {
                if let Some(row) = usize::try_from(index).ok().and_then(|i| s.queue.get(i)) {
                    if row.section != QueueSection::Current {
                        player.play_queue_row(row.uid);
                    }
                    if position_ms > 0 {
                        player.seek_ms(position_ms);
                    }
                }
            }
            JamControl::SeekToTrack { track_id, position_ms } => {
                let row = s.queue.iter().find(|r| r.section != QueueSection::Current && p::wire_track(&r.track.play_uri()).0 == track_id);
                if let Some(row) = row {
                    player.play_queue_row(row.uid);
                    if position_ms > 0 {
                        player.seek_ms(position_ms);
                    }
                }
            }
            JamControl::SetShuffle(on) => player.set_shuffle(on),
            JamControl::SetRepeat(mode) => player.set_repeat(match mode {
                1 => Repeat::One,
                2 => Repeat::All,
                _ => Repeat::Off,
            }),
            JamControl::AddTrack { track, is_play_next, play_now } => {
                let Some(item) = track_item(&track, &|uri| player.lane_plays(uri)) else {
                    tracing::debug!(track = %track.track_id, "lanjam: a guest's track can't play here");
                    return;
                };
                let title = item.title.clone();
                if s.track.is_none() {
                    player.play_items(vec![item], 0, PlayContext::new("jam"));
                } else if play_now {
                    player.insert_manual(item, 0);
                    player.next();
                } else if is_play_next {
                    player.insert_manual(item, 0);
                } else {
                    player.add_to_queue(vec![item]);
                }
                let what = if play_now {
                    mf_i18n::tf("{0} played \"{1}\"", &[&name, &title])
                } else if is_play_next {
                    mf_i18n::tf("{0} added \"{1}\" to play next", &[&name, &title])
                } else {
                    mf_i18n::tf("{0} added \"{1}\" to the queue", &[&name, &title])
                };
                self.log("queue", what);
            }
        }
        self.notify_changed();
    }
}

/// A host's (or guest's) wire track as a player row; None when this computer can't play it.
pub fn track_item(t: &JamQueueItem, lane_plays: &dyn Fn(&str) -> bool) -> Option<TrackItem> {
    let uri = p::local_uri(&t.track_id, &t.source, t.youtube_id.as_deref(), t.content_uri.as_deref(), lane_plays)?;
    let cover = if t.artwork_url.starts_with("http") {
        t.artwork_url.clone()
    } else if let Some(id) = uri.strip_prefix("youtube:track:") {
        format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")
    } else {
        String::new()
    };
    let track_id = uri.rsplit(':').next().unwrap_or_default().to_owned();
    Some(TrackItem {
        track_id,
        title: if t.title.trim().is_empty() { mf_i18n::t("Unknown") } else { t.title.clone() },
        artists: t.artist.clone(),
        cover: cover.clone(),
        cover_large: cover,
        duration_ms: t.duration_ms.max(0),
        available: true,
        uri,
        ..Default::default()
    })
}

fn wire_item(track: &TrackItem) -> (String, &'static str, Option<String>, String) {
    let (id, source, yt) = p::wire_track(&track.play_uri());
    let art = [&track.cover_large, &track.cover].into_iter().find(|c| c.starts_with("http")).cloned().unwrap_or_default();
    (id, source, yt, art)
}

/// The player's queue as guests see it: every row (current, manual, context) with position-based queue ids,
/// since the Android UI keys its list by `queueId`.
pub fn queue_of(s: &PlayerSnapshot, created_at: i64) -> JamQueue {
    let rows: &[QueueRow] = &s.queue;
    let items = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let (id, source, yt, art) = wire_item(&row.track);
            JamQueueItem {
                queue_id: format!("{i}_{id}"),
                track_id: id,
                title: row.track.title.clone(),
                artist: row.track.artists.clone(),
                artwork_url: art,
                source: source.to_owned(),
                duration_ms: row.track.duration_ms.max(0),
                youtube_id: yt,
                content_uri: None,
                added_by: String::new(),
                added_by_name: String::new(),
                added_at: created_at,
                is_play_next: row.section == QueueSection::Manual,
            }
        })
        .collect::<Vec<_>>();
    let current = rows.iter().position(|r| r.section == QueueSection::Current).unwrap_or(0) as i64;
    JamQueue {
        total_count: items.len() as i64,
        items,
        current_index: current,
        repeat_mode: match s.repeat {
            Repeat::Off => 0,
            Repeat::One => 1,
            Repeat::All => 2,
        },
        shuffle_enabled: s.shuffle,
    }
}

/// Both clock stamps are this computer's wall clock: a LAN-only host has no Ably server time.
/// While the next track loads guests are told "playing", so they start loading it too.
pub fn playback_of(s: &PlayerSnapshot, now: i64) -> Option<JamPlayback> {
    let track = s.track.as_ref()?;
    let (id, source, yt, art) = wire_item(track);
    Some(JamPlayback {
        track_id: id,
        title: track.title.clone(),
        artist: track.artists.clone(),
        artwork_url: art,
        source: source.to_owned(),
        is_playing: s.playing || s.buffering,
        position_ms: s.position_ms.max(0),
        server_timestamp: now,
        host_clock_ms: now,
        duration_ms: s.duration_ms.max(track.duration_ms).max(0),
        youtube_id: yt,
        content_uri: None,
    })
}

async fn accept_loop(host: LanHost, listener: TcpListener) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        let host = host.clone();
        tokio::spawn(async move { handle(host, stream).await });
    }
}

async fn handle(host: LanHost, mut stream: TcpStream) {
    let _ = stream.set_nodelay(true);
    let Some(req) = http::read_request(&mut stream, Duration::from_secs(5)).await else { return };
    let post = req.method.eq_ignore_ascii_case("POST");
    let (status, body) = match req.path.as_str() {
        "/time" => (200, json!({ "serverTime": now_ms() })),
        "/join" if post => host.join(&req),
        "/sync" => host.sync(&req).await,
        "/control" | "/queue" if post => host.control(&req),
        "/leave" if post => host.leave(&req),
        _ => (404, error("not found")),
    };
    http::write_response(&mut stream, status, &body.to_string()).await;
}

async fn beacon_loop(host: LanHost) {
    let c = &host.inner.config;
    if c.beacon_targets.is_empty() {
        return;
    }
    let bind: SocketAddr = if c.bind.is_loopback() { (c.bind, 0).into() } else { ([0, 0, 0, 0], 0).into() };
    let socket = match UdpSocket::bind(bind).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("lanjam: no beacon socket: {e}");
            return;
        }
    };
    let _ = socket.set_broadcast(true);
    let mut warned = HashSet::new();
    loop {
        let name = host.inner.guests.lock().unwrap().host_name.clone();
        let bytes = p::beacon_to_bytes(&p::Beacon { room_id: c.room_id.clone(), host_name: name, port: host.inner.port });
        // Some targets are refused (255.255.255.255 on a VPN adapter); the others still go out.
        for target in &c.beacon_targets {
            if let Err(e) = socket.send_to(&bytes, target).await
                && warned.insert(*target)
            {
                tracing::debug!("lanjam: beacon to {target} refused: {e}");
            }
        }
        tokio::time::sleep(Duration::from_millis(p::BEACON_INTERVAL_MS)).await;
    }
}

/// What guests must hear about at once: another track, play/pause, loading, the queue, repeat, shuffle, a seek.
#[derive(PartialEq)]
struct Signature {
    track: String,
    playing: bool,
    buffering: bool,
    queue: Vec<String>,
    current: i32,
    repeat: Repeat,
    shuffle: bool,
}

fn signature(s: &PlayerSnapshot) -> Signature {
    Signature {
        track: s.track.as_ref().map(TrackItem::play_uri).unwrap_or_default(),
        playing: s.playing || s.buffering,
        buffering: s.buffering,
        queue: s.queue.iter().map(|r| r.track.play_uri()).collect(),
        current: s.current_index,
        repeat: s.repeat,
        shuffle: s.shuffle,
    }
}

async fn player_watch(host: LanHost) {
    let mut last = signature(&host.inner.player.snapshot());
    let mut last_pos = (host.inner.player.snapshot().position_ms, Instant::now());
    loop {
        tokio::time::sleep(PLAYER_POLL).await;
        let s = host.inner.player.snapshot();
        let next = signature(&s);
        let expected = if s.playing { last_pos.0 + last_pos.1.elapsed().as_millis() as i64 } else { last_pos.0 };
        let seeked = next.track == last.track && !s.buffering && (s.position_ms - expected).abs() > SEEK_JUMP_MS;
        last_pos = (s.position_ms, Instant::now());
        if next != last || seeked {
            last = next;
            host.notify_changed();
        }
    }
}

async fn timeout_loop(host: LanHost) {
    loop {
        tokio::time::sleep(Duration::from_secs(15)).await;
        let gone: Vec<JamParticipant> = {
            let mut g = host.inner.guests.lock().unwrap();
            let stale: Vec<String> = g.last_seen.iter().filter(|(_, at)| at.elapsed() > GUEST_TIMEOUT).map(|(k, _)| k.clone()).collect();
            let mut gone = Vec::new();
            for uid in stale {
                g.last_seen.remove(&uid);
                g.tokens.remove(&uid);
                if let Some(i) = g.list.iter().position(|p| p.user_id == uid) {
                    gone.push(g.list.remove(i));
                }
            }
            gone
        };
        if !gone.is_empty() {
            for guest in gone {
                host.log("leave", mf_i18n::tf("{0} left the Jam", &[&guest.name]));
            }
            host.notify_changed();
        }
    }
}
