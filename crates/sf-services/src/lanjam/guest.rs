//! Guest side: joins a host's LAN server, long-polls `/sync`, measures the host's clock with `/time`, and sends
//! controls. `Follower` makes the local player match what the host plays.
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use md3_model::{PlayContext, TrackItem};
use serde_json::Value;
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::host::track_item;
use super::http::{self, build_query};
use super::protocol::{self as p, Beacon, JamPlayback, JamQueue, JamRoom};
use super::{JamPlayer, now_ms};

pub enum GuestEvent {
    /// a `/sync` reply (`{room, playback?, queue}`)
    Sync(Value),
    /// the host answered 403: removed or banned
    Kicked,
    /// no answer for `HOST_LOST_MS`
    HostLost,
}

pub type GuestCallback = Arc<dyn Fn(GuestEvent) + Send + Sync>;

struct Inner {
    addr: SocketAddr,
    user_id: String,
    token: String,
    version: AtomicI64,
    /// host clock minus ours, ms
    clock_offset: AtomicI64,
    connected: AtomicBool,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

#[derive(Clone)]
pub struct LanGuest {
    inner: Arc<Inner>,
}

fn auth(user_id: &str, token: &str) -> String {
    build_query(&[("uid".into(), user_id.into()), ("token".into(), token.into())])
}

impl LanGuest {
    /// `/join`, then the clock, then the sync and clock loops. Returns the join payload as the first state.
    pub async fn join(
        addr: SocketAddr,
        user_id: &str,
        name: &str,
        device: &str,
        on_event: GuestCallback,
    ) -> Result<(LanGuest, Value), String> {
        let query = build_query(&[("uid".into(), user_id.into()), ("name".into(), name.into()), ("device".into(), device.into())]);
        let (code, text) = http::request(addr, "POST", &format!("/join?{query}"), Duration::from_secs(3), Duration::from_secs(3))
            .await
            .map_err(|e| {
                tracing::debug!("lanjam: join {addr}: {e}");
                mf_i18n::t("Couldn't reach the host on this Wi-Fi.")
            })?;
        if code == 403 {
            return Err(mf_i18n::t("You were removed from this Jam by the host."));
        }
        if code != 200 {
            return Err(mf_i18n::tf("The host answered HTTP {0}", &[&code]));
        }
        let body: Value = serde_json::from_str(&text).map_err(|_| mf_i18n::t("The host sent something this app doesn't understand."))?;
        let you = body.get("you").cloned().unwrap_or(Value::Null);
        let uid = Some(p::opt_str(&you, "userId")).filter(|u| !u.trim().is_empty()).unwrap_or_else(|| user_id.to_owned());
        let inner = Arc::new(Inner {
            addr,
            user_id: uid,
            token: p::opt_str(&you, "token"),
            version: AtomicI64::new(body.get("version").map(|_| p::opt_i64(&body, "version")).unwrap_or(-1)),
            clock_offset: AtomicI64::new(0),
            connected: AtomicBool::new(true),
            tasks: Mutex::new(Vec::new()),
        });
        let guest = LanGuest { inner };
        if let Some(offset) = guest.measure_clock().await {
            guest.inner.clock_offset.store(offset, Ordering::Relaxed);
        }
        let tasks = vec![tokio::spawn(sync_loop(guest.clone(), on_event)), tokio::spawn(clock_loop(guest.clone()))];
        *guest.inner.tasks.lock().unwrap() = tasks;
        Ok((guest, body))
    }

    pub fn user_id(&self) -> &str {
        &self.inner.user_id
    }

    pub fn clock_offset(&self) -> i64 {
        self.inner.clock_offset.load(Ordering::Relaxed)
    }

    pub fn connected(&self) -> bool {
        self.inner.connected.load(Ordering::Relaxed)
    }

    /// NTP-style offset from 5 quick `/time` requests, keeping the minimum-RTT sample.
    async fn measure_clock(&self) -> Option<i64> {
        let mut best: Option<(i64, i64)> = None;
        for _ in 0..5 {
            let t0 = now_ms();
            let Ok((200, text)) = http::request(self.inner.addr, "GET", "/time", Duration::from_secs(1), Duration::from_secs(1)).await
            else {
                continue;
            };
            let t1 = now_ms();
            let Ok(j) = serde_json::from_str::<Value>(&text) else { continue };
            let server = p::opt_i64(&j, "serverTime");
            if server <= 0 {
                continue;
            }
            let rtt = t1 - t0;
            if best.is_none_or(|(r, _)| rtt < r) {
                best = Some((rtt, server + rtt / 2 - t1));
            }
        }
        best.map(|(_, offset)| offset)
    }

    /// `control` is a JamProtocol control object; a nested `track` goes flat into the query.
    pub async fn control(&self, control: &Value) -> Result<(), String> {
        let mut params = p::control_to_params(control);
        params.push(("uid".into(), self.inner.user_id.clone()));
        params.push(("token".into(), self.inner.token.clone()));
        let path = format!("/control?{}", build_query(&params));
        match http::request(self.inner.addr, "POST", &path, Duration::from_secs(2), Duration::from_secs(2)).await {
            Ok((200, _)) => Ok(()),
            Ok((403, _)) => {
                self.inner.connected.store(false, Ordering::Relaxed);
                Err(mf_i18n::t("You were removed from this Jam by the host."))
            }
            _ => Err(mf_i18n::t("Couldn't reach the host. Check your connection.")),
        }
    }

    /// Stops the loops; tells the host when `notify_host` (best effort, 1.5 s).
    pub async fn leave(&self, notify_host: bool) {
        let was = self.inner.connected.swap(false, Ordering::Relaxed);
        for task in self.inner.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
        if notify_host && was && !self.inner.token.is_empty() {
            let path = format!("/leave?{}", auth(&self.inner.user_id, &self.inner.token));
            let _ = http::request(self.inner.addr, "POST", &path, Duration::from_millis(1500), Duration::from_millis(1500)).await;
        }
    }
}

async fn sync_loop(guest: LanGuest, on_event: GuestCallback) {
    let inner = &guest.inner;
    let mut last_ok = Instant::now();
    while inner.connected.load(Ordering::Relaxed) {
        let path = format!("/sync?since={}&{}", inner.version.load(Ordering::Relaxed), auth(&inner.user_id, &inner.token));
        let read = Duration::from_millis(p::LONG_POLL_MS + 4_000);
        match http::request(inner.addr, "GET", &path, Duration::from_secs(3), read).await {
            Ok((200, text)) => {
                last_ok = Instant::now();
                if let Ok(body) = serde_json::from_str::<Value>(&text) {
                    if body.get("version").is_some() {
                        inner.version.store(p::opt_i64(&body, "version"), Ordering::Relaxed);
                    }
                    on_event(GuestEvent::Sync(body));
                }
            }
            Ok((403, _)) => {
                inner.connected.store(false, Ordering::Relaxed);
                on_event(GuestEvent::Kicked);
                return;
            }
            _ => {
                if last_ok.elapsed() > Duration::from_millis(p::HOST_LOST_MS) {
                    inner.connected.store(false, Ordering::Relaxed);
                    on_event(GuestEvent::HostLost);
                    return;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

async fn clock_loop(guest: LanGuest) {
    let mut rounds = 0u32;
    while guest.connected() {
        // converge quickly, then back off: clocks drift by milliseconds per hour
        rounds += 1;
        tokio::time::sleep(Duration::from_secs(if rounds < 3 { 15 } else { 60 })).await;
        if let Some(offset) = guest.measure_clock().await {
            guest.inner.clock_offset.store(offset, Ordering::Relaxed);
        }
    }
}

// ---- discovery ----

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NearbyJam {
    pub room_id: String,
    pub host_name: String,
    pub host_ip: String,
    pub port: u16,
    #[serde(skip)]
    pub seen_at: Option<Instant>,
}

/// Listens for host beacons; lists rooms heard in the last 8 s.
pub struct Discovery {
    nearby: watch::Sender<Vec<NearbyJam>>,
    task: JoinHandle<()>,
    pub local_addr: SocketAddr,
}

impl Discovery {
    /// `bind`: `0.0.0.0:58245` in the app, a loopback port in tests. `own_room` is not listed.
    pub async fn start(bind: SocketAddr, own_room: Option<String>) -> std::io::Result<Discovery> {
        let socket = UdpSocket::bind(bind).await?;
        let _ = socket.set_broadcast(true);
        let local_addr = socket.local_addr()?;
        let (nearby, _) = watch::channel(Vec::<NearbyJam>::new());
        let tx = nearby.clone();
        let task = tokio::spawn(async move {
            let mut buf = vec![0u8; 2048];
            loop {
                let received = tokio::time::timeout(Duration::from_millis(2_500), socket.recv_from(&mut buf)).await;
                let now = Instant::now();
                let fresh = |list: &mut Vec<NearbyJam>| list.retain(|n| n.seen_at.is_some_and(|at| now - at < Duration::from_secs(8)));
                match received {
                    Ok(Ok((n, from))) => {
                        let Some(Beacon { room_id, host_name, port }) = p::beacon_from_bytes(&buf[..n]) else { continue };
                        if own_room.as_deref().is_some_and(|own| own.eq_ignore_ascii_case(&room_id)) {
                            continue;
                        }
                        let info = NearbyJam { room_id, host_name, host_ip: from.ip().to_string(), port, seen_at: Some(now) };
                        tx.send_modify(|list| {
                            list.retain(|n| n.room_id != info.room_id);
                            list.push(info);
                            fresh(list);
                        });
                    }
                    Ok(Err(_)) => tokio::time::sleep(Duration::from_millis(200)).await,
                    Err(_) => {
                        tx.send_if_modified(|list| {
                            let before = list.len();
                            fresh(list);
                            list.len() != before
                        });
                    }
                }
            }
        });
        Ok(Discovery { nearby, task, local_addr })
    }

    pub fn subscribe(&self) -> watch::Receiver<Vec<NearbyJam>> {
        self.nearby.subscribe()
    }

    pub fn nearby(&self) -> Vec<NearbyJam> {
        self.nearby.borrow().clone()
    }

    /// Waits up to `limit` for the beacon of room `code` (any case).
    pub async fn find(&self, code: &str, limit: Duration) -> Option<NearbyJam> {
        let mut rx = self.nearby.subscribe();
        let wanted = |list: &Vec<NearbyJam>| list.iter().find(|n| n.room_id.eq_ignore_ascii_case(code)).cloned();
        tokio::time::timeout(limit, async {
            loop {
                if let Some(found) = wanted(&rx.borrow_and_update()) {
                    return Some(found);
                }
                if rx.changed().await.is_err() {
                    return None;
                }
            }
        })
        .await
        .ok()
        .flatten()
    }

    pub fn stop(&self) {
        self.task.abort();
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.task.abort();
    }
}

// ---- following the host ----

/// What the guest knows of the host, and the drift-correction clock.
#[derive(Default)]
pub struct Follower {
    pub room: Option<JamRoom>,
    pub queue: JamQueue,
    pub playback: Option<JamPlayback>,
    pub my_id: String,
    last_hard_seek: Option<Instant>,
}

impl Follower {
    pub fn new(my_id: &str) -> Self {
        Self { my_id: my_id.to_owned(), ..Default::default() }
    }

    pub fn absorb(&mut self, payload: &Value) {
        if let Some(room) = payload.get("room").filter(|r| r.is_object()) {
            self.room = Some(p::apply_room_json(self.room.as_ref(), room));
        }
        if let Some(queue) = payload.get("queue").filter(|q| q.is_object()) {
            self.queue = p::queue_from_json(queue);
        }
        // a host with nothing loaded sends no playback
        self.playback = payload.get("playback").filter(|p| p.is_object()).map(p::playback_from_json);
    }

    pub fn pending(&self) -> bool {
        self.room.as_ref().and_then(|r| r.participants.iter().find(|p| p.user_id == self.my_id)).is_some_and(|me| me.is_pending)
    }

    /// Where the host is now: its position plus the time since it was stamped (host clock = ours + offset).
    pub fn target_position(pb: &JamPlayback, now: i64, clock_offset: i64) -> i64 {
        let stamp = if pb.host_clock_ms > 0 { pb.host_clock_ms } else { pb.server_timestamp };
        let elapsed = if pb.is_playing && stamp > 0 { (now + clock_offset - stamp).max(0) } else { 0 };
        (pb.position_ms + elapsed).max(0)
    }

    /// The host's queue as player rows this computer can play, of the same kind as the host's current track
    /// (the player doesn't mix Spotify and lane tracks), with the index of the current one.
    fn playable_queue(&self, player: &dyn JamPlayer, current: &TrackItem) -> (Vec<TrackItem>, usize) {
        let lane = |uri: &str| player.lane_plays(uri);
        let spotify = current.uri.starts_with("spotify:");
        let mut items: Vec<TrackItem> = Vec::new();
        // (index in `items`, the row is at or after the host's current index)
        let mut matches = Vec::new();
        for (i, q) in self.queue.items.iter().enumerate() {
            let Some(item) = track_item(q, &lane) else { continue };
            if item.uri.starts_with("spotify:") != spotify {
                continue;
            }
            if item.uri == current.uri {
                matches.push((items.len(), i as i64 >= self.queue.current_index));
            }
            items.push(item);
        }
        let index = matches.iter().find(|(_, after)| *after).or(matches.first()).map(|(i, _)| *i);
        match index {
            Some(i) => (items, i),
            None => {
                // the current track isn't in the queue window: play it alone first
                items.insert(0, current.clone());
                (items, 0)
            }
        }
    }

    /// Makes the player match the host: the right track, play/pause, and the position. `force` corrects even
    /// small drift (after a pause or a seek on the host).
    pub fn apply(&mut self, player: &dyn JamPlayer, now: i64, clock_offset: i64, force: bool) {
        let Some(pb) = self.playback.clone() else { return };
        if self.pending() || pb.track_id.is_empty() {
            return;
        }
        let lane = |uri: &str| player.lane_plays(uri);
        let as_item = p::JamQueueItem {
            track_id: pb.track_id.clone(),
            title: pb.title.clone(),
            artist: pb.artist.clone(),
            artwork_url: pb.artwork_url.clone(),
            source: pb.source.clone(),
            duration_ms: pb.duration_ms,
            youtube_id: pb.youtube_id.clone(),
            content_uri: pb.content_uri.clone(),
            ..Default::default()
        };
        let Some(current) = track_item(&as_item, &lane) else { return };
        let s = player.snapshot();
        let target = Self::target_position(&pb, now, clock_offset);
        let here = s.track.as_ref().map(TrackItem::play_uri).unwrap_or_default();
        if here != current.uri {
            let (items, row) = self.playable_queue(player, &current);
            let host = self.room.as_ref().map(|r| r.host_name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| mf_i18n::t("host"));
            let mut context = PlayContext::new("jam");
            context.title = mf_i18n::tf("Jam with {0}", &[&host]);
            player.play_items(items, row, context);
            if target > 1_000 {
                player.seek_ms(target);
            }
            if !pb.is_playing {
                player.pause();
            }
            self.last_hard_seek = Some(Instant::now());
            return;
        }
        if pb.is_playing && !s.playing && !s.buffering {
            player.play();
        } else if !pb.is_playing && s.playing {
            player.pause();
        }
        if s.buffering {
            return;
        }
        let drift = (s.position_ms - target).abs();
        let recent = self.last_hard_seek.is_some_and(|at| at.elapsed() < Duration::from_secs(4));
        if (drift > 2_500 || (force && drift > 800)) && !recent {
            player.seek_ms(target);
            self.last_hard_seek = Some(Instant::now());
        }
    }
}
