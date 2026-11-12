//! Same-Wi-Fi Jam: listen together with people on the same network, no account needed. Wire-compatible with the
//! Kotlin apps' LAN Jam (Android `LocalWifiJamEngine`, desktop `LanJam.kt`): the host runs a small HTTP server and
//! broadcasts a UDP beacon on port 58245; guests find the room by its code, long-poll `/sync` and follow the
//! host's player. Online Jam over Ably is not here (it needs the host's own Ably key).
//!
//! Spotify's own Jam (`social`) is separate and unchanged.
pub mod guest;
pub mod host;
pub mod http;
pub mod protocol;
#[cfg(test)]
mod tests;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use md3_model::{PlayContext, TrackItem};
use serde::Serialize;
use sf_player::{PlayerHandle, PlayerSnapshot, Repeat};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use guest::{Discovery, Follower, GuestEvent, LanGuest, NearbyJam};
use host::{HostConfig, LanHost};
use protocol::{self as p, JamControl, JamRoomSettings};

/// The Kotlin hosts' invite page; Android opens `…/jam/CODE?localHost=…&localPort=…` links into the room.
pub const INVITE_BASE_URL: &str = "https://sound-wave-website-eta.vercel.app/jam";

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The player commands a Jam needs: `PlayerHandle` in the app, a recording fake in tests.
pub trait JamPlayer: Send + Sync + 'static {
    fn snapshot(&self) -> PlayerSnapshot;
    fn play_items(&self, items: Vec<TrackItem>, row: usize, context: PlayContext);
    fn play(&self);
    fn pause(&self);
    fn next(&self);
    fn previous(&self);
    fn seek_ms(&self, ms: i64);
    fn play_queue_row(&self, uid: u64);
    fn set_shuffle(&self, value: bool);
    fn set_repeat(&self, repeat: Repeat);
    fn add_to_queue(&self, items: Vec<TrackItem>);
    fn insert_manual(&self, item: TrackItem, at: usize);
    /// The player's YouTube lane plays this URI (YouTube now; JioSaavn once the lane takes `saavn:track:`).
    fn lane_plays(&self, uri: &str) -> bool {
        sf_player::engine::youtube::is_lane_track(uri)
    }
}

impl JamPlayer for PlayerHandle {
    fn snapshot(&self) -> PlayerSnapshot {
        PlayerHandle::snapshot(self)
    }
    fn play_items(&self, items: Vec<TrackItem>, row: usize, context: PlayContext) {
        PlayerHandle::play_items(self, items, row, context)
    }
    fn play(&self) {
        PlayerHandle::play(self)
    }
    fn pause(&self) {
        PlayerHandle::pause(self)
    }
    fn next(&self) {
        PlayerHandle::next(self)
    }
    fn previous(&self) {
        PlayerHandle::previous(self)
    }
    fn seek_ms(&self, ms: i64) {
        PlayerHandle::seek_ms(self, ms)
    }
    fn play_queue_row(&self, uid: u64) {
        PlayerHandle::play_queue_row(self, uid)
    }
    fn set_shuffle(&self, value: bool) {
        PlayerHandle::set_shuffle(self, value)
    }
    fn set_repeat(&self, repeat: Repeat) {
        PlayerHandle::set_repeat(self, repeat)
    }
    fn add_to_queue(&self, items: Vec<TrackItem>) {
        PlayerHandle::add_to_queue(self, items)
    }
    fn insert_manual(&self, item: TrackItem, at: usize) {
        PlayerHandle::insert_manual(self, item, at)
    }
}

/// Where the sockets go: the real network in the app, loopback in tests.
#[derive(Clone, Debug)]
pub struct NetConfig {
    /// the host's HTTP server
    pub bind: IpAddr,
    /// the beacon listener (port 58245 in the app)
    pub discovery_bind: SocketAddr,
    /// this computer's LAN address (None: unknown; the beacon still goes to 255.255.255.255)
    pub lan_ip: Option<Ipv4Addr>,
    /// beacon destinations
    pub beacon_targets: Vec<SocketAddr>,
}

impl NetConfig {
    /// The app's settings: listen on every interface, beacon to the global and the subnet broadcast.
    pub fn system() -> Self {
        let lan_ip = local_ipv4();
        let mut beacon_targets = vec![SocketAddr::from((Ipv4Addr::BROADCAST, p::BEACON_PORT))];
        if let Some(ip) = lan_ip {
            // std can't list interfaces and their masks; a /24 is what home routers hand out
            let [a, b, c, _] = ip.octets();
            beacon_targets.push(SocketAddr::from((Ipv4Addr::new(a, b, c, 255), p::BEACON_PORT)));
        }
        Self {
            bind: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            discovery_bind: SocketAddr::from((Ipv4Addr::UNSPECIFIED, p::BEACON_PORT)),
            lan_ip,
            beacon_targets,
        }
    }
}

/// IPv4 address of this computer on the local network: the one a default route would use. Connecting a UDP
/// socket sends nothing; it only asks the OS which interface it would pick.
pub fn local_ipv4() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(8, 8, 8, 8), 53)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if ip.is_private() => Some(ip),
        _ => None,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanJamMember {
    pub user_id: String,
    pub name: String,
    pub is_host: bool,
    pub is_pending: bool,
    pub device: String,
    pub is_you: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanJamSnapshot {
    /// "idle" | "connecting" | "joined"
    pub status: String,
    pub is_host: bool,
    pub room_code: String,
    pub host_name: String,
    pub members: Vec<LanJamMember>,
    /// guest: waiting for the host to let us in
    pub pending_approval: bool,
    pub can_control: bool,
    pub can_add_tracks: bool,
    pub require_approval: bool,
    /// host: "192.168.1.20:40000" ("" — address unknown)
    pub address: String,
    /// host: the link that also works where the beacon is blocked
    pub invite_url: String,
    pub error: String,
    pub busy: bool,
    /// rooms heard on this network while discovery runs
    pub nearby: Vec<NearbyJam>,
    pub discovering: bool,
}

enum Session {
    Idle,
    Host { host: LanHost, watcher: JoinHandle<()> },
    Guest { guest: LanGuest, follower: Arc<Mutex<Follower>>, task: JoinHandle<()> },
}

struct State {
    /// bumped by every host/join/leave; work of an older session is dropped
    epoch: u64,
    session: Session,
    discovery: Option<(Discovery, JoinHandle<()>)>,
    name: String,
}

pub struct LanJamService {
    rt: Handle,
    player: Arc<dyn JamPlayer>,
    net: NetConfig,
    user_id: String,
    device: String,
    state: Mutex<State>,
    snapshot: watch::Sender<LanJamSnapshot>,
}

fn idle() -> LanJamSnapshot {
    LanJamSnapshot { status: "idle".into(), can_control: true, can_add_tracks: true, ..Default::default() }
}

fn default_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Materialfy".into())
}

fn room_code() -> String {
    (0..6).map(|_| p::CODE_ALPHABET[fastrand::usize(..p::CODE_ALPHABET.len())] as char).collect()
}

impl LanJamService {
    pub fn new(rt: Handle, player: Arc<dyn JamPlayer>, net: NetConfig) -> Arc<Self> {
        let os = if cfg!(windows) {
            "Windows"
        } else if cfg!(target_os = "macos") {
            "macOS"
        } else {
            "Linux"
        };
        let user_id = format!("desktop-{:012x}", rand::random::<u64>() & 0xffff_ffff_ffff);
        Arc::new(Self {
            rt,
            player,
            net,
            user_id,
            device: format!("Materialfy for {os}"),
            state: Mutex::new(State { epoch: 0, session: Session::Idle, discovery: None, name: default_name() }),
            snapshot: watch::channel(idle()).0,
        })
    }

    pub fn watch(&self) -> watch::Receiver<LanJamSnapshot> {
        self.snapshot.subscribe()
    }

    pub fn snapshot(&self) -> LanJamSnapshot {
        self.snapshot.borrow().clone()
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    fn set(&self, epoch: u64, f: impl FnOnce(&mut LanJamSnapshot)) {
        if self.state.lock().unwrap().epoch != epoch {
            return;
        }
        self.snapshot.send_if_modified(|s| {
            let before = s.clone();
            f(s);
            *s != before
        });
    }

    /// Ends whatever runs and starts a new epoch.
    fn reset(&self, name: &str) -> u64 {
        let old = {
            let mut st = self.state.lock().unwrap();
            st.epoch += 1;
            if !name.trim().is_empty() {
                st.name = name.trim().chars().take(40).collect();
            }
            std::mem::replace(&mut st.session, Session::Idle)
        };
        match old {
            Session::Idle => {}
            Session::Host { host, watcher } => {
                watcher.abort();
                host.stop();
            }
            Session::Guest { guest, task, .. } => {
                task.abort();
                self.rt.spawn(async move { guest.leave(true).await });
            }
        }
        self.state.lock().unwrap().epoch
    }

    /// Host a Jam on this computer.
    pub fn host(self: &Arc<Self>, name: &str) {
        let epoch = self.reset(name);
        let this = self.clone();
        let nearby = self.snapshot.borrow().nearby.clone();
        let discovering = self.snapshot.borrow().discovering;
        self.snapshot.send_replace(LanJamSnapshot { status: "connecting".into(), is_host: true, busy: true, nearby, discovering, ..idle() });
        self.rt.spawn(async move {
            let name = this.state.lock().unwrap().name.clone();
            let code = room_code();
            let config = HostConfig {
                room_id: code.clone(),
                host_id: this.user_id.clone(),
                host_name: name.clone(),
                device: this.device.clone(),
                lan_ip: this.net.lan_ip.map(|ip| ip.to_string()),
                bind: this.net.bind,
                beacon_targets: this.net.beacon_targets.clone(),
                settings: JamRoomSettings::default(),
            };
            let host = match LanHost::start(config, this.player.clone()).await {
                Ok(host) => host,
                Err(e) => {
                    tracing::warn!("lanjam: server didn't start: {e}");
                    this.set(epoch, |s| {
                        *s = LanJamSnapshot { error: mf_i18n::t("Couldn't start the Wi-Fi Jam on this computer."), nearby: s.nearby.clone(), ..idle() }
                    });
                    return;
                }
            };
            let (address, invite) = match this.net.lan_ip {
                Some(ip) => (format!("{ip}:{}", host.port()), format!("{INVITE_BASE_URL}/{code}?localHost={ip}&localPort={}", host.port())),
                None => (String::new(), format!("{INVITE_BASE_URL}/{code}")),
            };
            let watcher = {
                let this = this.clone();
                let host = host.clone();
                let mut changes = host.subscribe();
                tokio::spawn(async move {
                    loop {
                        this.redraw_host(epoch, &host);
                        if changes.changed().await.is_err() {
                            return;
                        }
                    }
                })
            };
            {
                let mut st = this.state.lock().unwrap();
                if st.epoch != epoch {
                    watcher.abort();
                    host.stop();
                    return;
                }
                st.session = Session::Host { host: host.clone(), watcher };
            }
            this.set(epoch, |s| {
                s.status = "joined".into();
                s.room_code = code;
                s.host_name = name;
                s.address = address;
                s.invite_url = invite;
                s.busy = false;
            });
            this.redraw_host(epoch, &host);
        });
    }

    fn redraw_host(&self, epoch: u64, host: &LanHost) {
        let settings = host.settings();
        let members = host
            .participants()
            .into_iter()
            .map(|p| LanJamMember { is_you: p.user_id == self.user_id, user_id: p.user_id, name: p.name, is_host: p.is_host, is_pending: p.is_pending, device: p.device })
            .collect();
        self.set(epoch, |s| {
            s.members = members;
            s.can_add_tracks = settings.allow_guests_to_add_tracks;
            s.can_control = settings.allow_guests_to_control_playback;
            s.require_approval = settings.require_host_approval_to_join;
        });
    }

    /// Join by room code (found through the beacon) or by the host's invite link.
    pub fn join(self: &Arc<Self>, input: &str, name: &str) {
        let Some((code, direct)) = p::parse_join(input) else {
            self.snapshot.send_modify(|s| s.error = mf_i18n::t("Enter the room code shown on the host's screen."));
            return;
        };
        let epoch = self.reset(name);
        let this = self.clone();
        let nearby = self.snapshot.borrow().nearby.clone();
        let discovering = self.snapshot.borrow().discovering;
        self.snapshot.send_replace(LanJamSnapshot { status: "connecting".into(), room_code: code.clone(), busy: true, nearby, discovering, ..idle() });
        self.rt.spawn(async move {
            let fail = |message: String| {
                this.set(epoch, |s| *s = LanJamSnapshot { error: message, nearby: s.nearby.clone(), discovering: s.discovering, ..idle() });
            };
            let addr = match direct {
                Some((host, port)) => match host.parse::<IpAddr>() {
                    Ok(ip) => Some(SocketAddr::new(ip, port)),
                    Err(_) => None,
                },
                None => this.find(&code).await,
            };
            let Some(addr) = addr else {
                fail(mf_i18n::tf("Jam #{0} isn't on this Wi-Fi. Check that you're on the same network as the host.", &[&code]));
                return;
            };
            let (tx, rx) = mpsc::unbounded_channel();
            let callback: guest::GuestCallback = Arc::new(move |event| {
                let _ = tx.send(event);
            });
            let name = this.state.lock().unwrap().name.clone();
            // one retry: the first connect to a host that has just woken up is sometimes refused
            let mut joined = LanGuest::join(addr, &this.user_id, &name, &this.device, callback.clone()).await;
            if joined.is_err() {
                tokio::time::sleep(Duration::from_millis(600)).await;
                joined = LanGuest::join(addr, &this.user_id, &name, &this.device, callback).await;
            }
            let (guest, first) = match joined {
                Ok(ok) => ok,
                Err(message) => {
                    fail(message);
                    return;
                }
            };
            let follower = Arc::new(Mutex::new(Follower::new(guest.user_id())));
            follower.lock().unwrap().absorb(&first);
            let task = tokio::spawn(this.clone().follow(epoch, guest.clone(), follower.clone(), rx));
            {
                let mut st = this.state.lock().unwrap();
                if st.epoch != epoch {
                    task.abort();
                    tokio::spawn(async move { guest.leave(true).await });
                    return;
                }
                st.session = Session::Guest { guest: guest.clone(), follower: follower.clone(), task };
            }
            this.set(epoch, |s| {
                s.status = "joined".into();
                s.busy = false;
                s.error.clear();
            });
            this.redraw_guest(epoch, &follower);
            follower.lock().unwrap().apply(this.player.as_ref(), now_ms(), guest.clock_offset(), true);
        });
    }

    /// The host's address for room `code`: from the running discovery, else a short listen of its own.
    async fn find(&self, code: &str) -> Option<SocketAddr> {
        let known = self.snapshot.borrow().nearby.iter().find(|n| n.room_id.eq_ignore_ascii_case(code)).cloned();
        let found = match known {
            Some(n) => Some(n),
            None => {
                let running = self.state.lock().unwrap().discovery.as_ref().map(|(d, _)| d.subscribe());
                match running {
                    Some(mut rx) => tokio::time::timeout(Duration::from_secs(4), async {
                        loop {
                            if let Some(n) = rx.borrow_and_update().iter().find(|n| n.room_id.eq_ignore_ascii_case(code)) {
                                return Some(n.clone());
                            }
                            rx.changed().await.ok()?;
                        }
                    })
                    .await
                    .ok()
                    .flatten(),
                    None => match Discovery::start(self.net.discovery_bind, None).await {
                        Ok(d) => d.find(code, Duration::from_secs(4)).await,
                        Err(e) => {
                            tracing::warn!("lanjam: can't listen for nearby Jams: {e}");
                            None
                        }
                    },
                }
            }
        };
        found.and_then(|n| n.host_ip.parse::<IpAddr>().ok().map(|ip| SocketAddr::new(ip, n.port)))
    }

    async fn follow(self: Arc<Self>, epoch: u64, guest: LanGuest, follower: Arc<Mutex<Follower>>, mut rx: mpsc::UnboundedReceiver<GuestEvent>) {
        let mut tick = tokio::time::interval(Duration::from_secs(3));
        loop {
            tokio::select! {
                event = rx.recv() => match event {
                    Some(GuestEvent::Sync(body)) => {
                        let mut f = follower.lock().unwrap();
                        let was = f.playback.as_ref().map(|pb| (pb.track_id.clone(), pb.is_playing, pb.position_ms));
                        f.absorb(&body);
                        let now = f.playback.as_ref().map(|pb| (pb.track_id.clone(), pb.is_playing, pb.position_ms));
                        // a pause, a play or a seek on the host: line up closely
                        let force = was != now;
                        f.apply(self.player.as_ref(), now_ms(), guest.clock_offset(), force);
                        drop(f);
                        self.redraw_guest(epoch, &follower);
                    }
                    Some(GuestEvent::Kicked) => return self.ended(epoch, mf_i18n::t("You were removed from the Jam by the host.")),
                    Some(GuestEvent::HostLost) => return self.ended(epoch, mf_i18n::t("Lost connection to the host. The Jam has ended.")),
                    None => return,
                },
                _ = tick.tick() => {
                    let mut f = follower.lock().unwrap();
                    if f.playback.as_ref().is_some_and(|pb| pb.is_playing) {
                        f.apply(self.player.as_ref(), now_ms(), guest.clock_offset(), false);
                    }
                }
            }
        }
    }

    fn ended(&self, epoch: u64, message: String) {
        {
            let mut st = self.state.lock().unwrap();
            if st.epoch != epoch {
                return;
            }
            st.epoch += 1;
            st.session = Session::Idle;
        }
        self.snapshot.send_modify(|s| *s = LanJamSnapshot { error: message, nearby: s.nearby.clone(), discovering: s.discovering, ..idle() });
    }

    fn redraw_guest(&self, epoch: u64, follower: &Arc<Mutex<Follower>>) {
        let f = follower.lock().unwrap();
        let Some(room) = f.room.clone() else { return };
        let pending = f.pending();
        let me = f.my_id.clone();
        drop(f);
        self.set(epoch, |s| {
            s.host_name = room.host_name.clone();
            s.room_code = if room.room_id.is_empty() { s.room_code.clone() } else { room.room_id.clone() };
            s.members = room
                .participants
                .iter()
                .map(|p| LanJamMember {
                    user_id: p.user_id.clone(),
                    name: p.name.clone(),
                    is_host: p.is_host,
                    is_pending: p.is_pending,
                    device: p.device.clone(),
                    is_you: p.user_id == me,
                })
                .collect();
            s.pending_approval = pending;
            s.can_control = room.settings.allow_guests_to_control_playback;
            s.can_add_tracks = room.settings.allow_guests_to_add_tracks;
            s.require_approval = room.settings.require_host_approval_to_join;
        });
    }

    /// Leave the Jam you joined, or end the one you host.
    pub fn leave(&self) {
        self.reset("");
        self.snapshot.send_modify(|s| *s = LanJamSnapshot { nearby: s.nearby.clone(), discovering: s.discovering, ..idle() });
    }

    // ---- host ----

    fn host_session(&self) -> Option<LanHost> {
        match &self.state.lock().unwrap().session {
            Session::Host { host, .. } => Some(host.clone()),
            _ => None,
        }
    }

    pub fn approve(&self, user_id: &str) {
        if let Some(host) = self.host_session() {
            host.approve(user_id);
        }
    }

    /// Removes a guest and bans them for the rest of this Jam.
    pub fn remove(&self, user_id: &str) {
        if let Some(host) = self.host_session() {
            host.remove(user_id);
        }
    }

    pub fn set_room_settings(&self, can_add_tracks: bool, can_control: bool, require_approval: bool) {
        if let Some(host) = self.host_session() {
            host.set_settings(JamRoomSettings {
                allow_guests_to_add_tracks: can_add_tracks,
                allow_guests_to_control_playback: can_control,
                require_host_approval_to_join: require_approval,
            });
        }
    }

    // ---- guest → host ----

    /// Ask the host to play, pause, skip, seek or add a track. Refused here when the room doesn't allow it.
    pub fn control(self: &Arc<Self>, action: JamControl) {
        let guest = match &self.state.lock().unwrap().session {
            Session::Guest { guest, .. } => guest.clone(),
            _ => return,
        };
        let s = self.snapshot();
        let adding = matches!(action, JamControl::AddTrack { .. });
        let blocked = if s.pending_approval {
            Some(mf_i18n::t("Waiting for the host to let you in"))
        } else if adding && !s.can_add_tracks {
            Some(mf_i18n::t("The host has turned off adding tracks"))
        } else if !adding && !s.can_control {
            Some(mf_i18n::t("The host has turned off guest playback controls"))
        } else {
            None
        };
        if let Some(message) = blocked {
            self.snapshot.send_modify(|s| s.error = message);
            return;
        }
        let name = self.state.lock().unwrap().name.clone();
        let body = p::control_to_json(&action, guest.user_id(), &name);
        let this = self.clone();
        let epoch = self.state.lock().unwrap().epoch;
        self.rt.spawn(async move {
            if let Err(message) = guest.control(&body).await {
                this.set(epoch, |s| s.error = message);
            }
        });
    }

    /// Play/pause toward the host's state (what a guest's play button means).
    pub fn toggle_play(self: &Arc<Self>) {
        let playing = match &self.state.lock().unwrap().session {
            Session::Guest { follower, .. } => follower.lock().unwrap().playback.as_ref().is_some_and(|pb| pb.is_playing),
            _ => return,
        };
        self.control(if playing { JamControl::Pause } else { JamControl::Play });
    }

    /// A lane or Spotify track as a Jam queue item (what a guest adds).
    pub fn add_track(self: &Arc<Self>, track: &TrackItem, play_next: bool, play_now: bool) {
        let (track_id, source, youtube_id) = p::wire_track(&track.play_uri());
        let item = p::JamQueueItem {
            queue_id: track_id.clone(),
            track_id,
            title: track.title.clone(),
            artist: track.artists.clone(),
            artwork_url: [&track.cover_large, &track.cover].into_iter().find(|c| c.starts_with("http")).cloned().unwrap_or_default(),
            source: source.to_owned(),
            duration_ms: track.duration_ms,
            youtube_id,
            content_uri: None,
            added_by: self.user_id.clone(),
            added_by_name: self.state.lock().unwrap().name.clone(),
            added_at: now_ms(),
            is_play_next: play_next,
        };
        self.control(JamControl::AddTrack { track: item, is_play_next: play_next, play_now });
    }

    // ---- discovery for the card's "nearby" list ----

    pub fn start_discovery(self: &Arc<Self>) {
        if self.state.lock().unwrap().discovery.is_some() {
            return;
        }
        let this = self.clone();
        self.rt.spawn(async move {
            let own = match &this.state.lock().unwrap().session {
                Session::Host { host, .. } => Some(host.room_id().to_owned()),
                _ => None,
            };
            let discovery = match Discovery::start(this.net.discovery_bind, own).await {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!("lanjam: can't listen for nearby Jams: {e}");
                    return;
                }
            };
            let mut rx = discovery.subscribe();
            let forward = {
                let this = this.clone();
                tokio::spawn(async move {
                    loop {
                        let list = rx.borrow_and_update().clone();
                        this.snapshot.send_if_modified(|s| {
                            let changed = s.nearby != list || !s.discovering;
                            s.nearby = list;
                            s.discovering = true;
                            changed
                        });
                        if rx.changed().await.is_err() {
                            return;
                        }
                    }
                })
            };
            let mut st = this.state.lock().unwrap();
            if st.discovery.is_some() {
                forward.abort();
                return;
            }
            st.discovery = Some((discovery, forward));
        });
    }

    pub fn stop_discovery(&self) {
        if let Some((discovery, forward)) = self.state.lock().unwrap().discovery.take() {
            forward.abort();
            discovery.stop();
        }
        self.snapshot.send_if_modified(|s| {
            let changed = s.discovering || !s.nearby.is_empty();
            s.discovering = false;
            s.nearby.clear();
            changed
        });
    }

    pub fn shutdown(&self) {
        self.stop_discovery();
        self.reset("");
    }
}
