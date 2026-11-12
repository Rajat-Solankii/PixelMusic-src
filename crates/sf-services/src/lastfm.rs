//! Last.fm scrobbling — port of soundwave1 desktopApp data/LastFm.kt (`LastFm` + `LastFmScrobbler`).
//!
//! The user's own API key and shared secret (last.fm/api/account/create) and the session key live in the OS
//! credential store (sf-auth, `Slot::LastFm`). Sign-in is the web-token flow: auth.getToken → the approval page in
//! the browser → auth.getSession polled until approved (180 s). Then "now playing" when a track starts and a
//! scrobble once half of it (at most 3 minutes) has played; the played time pauses with playback, tracks of 30 s
//! or less are never scrobbled. Each call has its own toggle (SettingKey::LastFmScrobble / LastFmNowPlaying).
//! Error 9 (invalid session) forgets the session. Like the Kotlin code there is no offline queue: a scrobble that
//! fails is logged and dropped.
//!
//! Fed by PlayerEvent::TrackStarted / TrackEnded with `local` (Spotify on this PC and YouTube Music both), the same
//! way history.rs is; Spotify Connect playback on other devices is not scrobbled from here.
use std::fmt;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use md3_model::TrackItem;
use md5::{Digest, Md5};
use mf_platform::{SettingKey, SettingsStore};
use serde_json::Value;
use sf_auth::credentials::Grant as StoredGrant;
use sf_auth::{LastFmAccount, Slot, Store};
use sf_player::{PlayerEvent, PlayerHandle};
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tokio::time::Instant;

use crate::lock;

pub mod queue;
use queue::{Pending, Queue, batch_params};

pub const API_URL: &str = "https://ws.audioscrobbler.com/2.0/";
pub const AUTH_URL: &str = "https://www.last.fm/api/auth/";
/// Page where the user creates their own API key and shared secret.
pub const CREATE_KEY_URL: &str = "https://www.last.fm/api/account/create";
/// Tracks this long or shorter are never scrobbled.
pub const MIN_TRACK: Duration = Duration::from_secs(30);
/// A listen counts after half the track, but never later than this.
pub const SCROBBLE_CAP: Duration = Duration::from_secs(180);
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(180);
const SIGN_IN_POLL: Duration = Duration::from_millis(2500);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const USER_AGENT: &str = concat!("Materialfy/", env!("CARGO_PKG_VERSION"));
/// "Invalid session key": the user revoked access on last.fm.
const ERROR_INVALID_SESSION: i64 = 9;
/// "Unauthorized token": the user hasn't approved in the browser yet.
const ERROR_NOT_APPROVED: i64 = 14;
/// Waiting scrobbles are tried again this long after a network failure or "service offline".
const RETRY_DELAY: Duration = Duration::from_secs(5 * 60);

/// Worth trying again later: no connection, or Last.fm busy (8 operation failed, 11 offline, 16 temporarily
/// unavailable, 29 rate limit).
fn transient(error: &LastFmError) -> bool {
    matches!(error, LastFmError::Network | LastFmError::Unreadable | LastFmError::Api { code: 8 | 11 | 16 | 29, .. })
}

/// The batch can never go through as it is (6 invalid parameters, 7 invalid resource): dropped, not retried.
fn rejects_batch(error: &LastFmError) -> bool {
    matches!(error, LastFmError::Api { code: 6 | 7, .. })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LastFmError {
    NoKeys,
    NotConnected,
    Api { code: i64, message: String },
    Network,
    Unreadable,
    TimedOut,
    Store(String),
}

impl fmt::Display for LastFmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::NoKeys => mf_i18n::t("Enter your Last.fm API key and shared secret first."),
            Self::NotConnected => mf_i18n::t("Not connected to Last.fm."),
            Self::Api { code: 10 | 26, .. } => {
                mf_i18n::t("Last.fm rejected the API key. Check the key and shared secret.")
            }
            Self::Api { code: ERROR_INVALID_SESSION, .. } => {
                mf_i18n::t("Your Last.fm session has ended — connect again.")
            }
            // Last.fm's own text (English)
            Self::Api { message, .. } if !message.is_empty() => message.clone(),
            Self::Api { code, .. } => mf_i18n::tf("Last.fm error {0}", &[code]),
            Self::Network => mf_i18n::t("Couldn't reach Last.fm."),
            Self::Unreadable => mf_i18n::t("Last.fm sent an unreadable answer."),
            Self::TimedOut => mf_i18n::t("Timed out waiting for approval in the browser."),
            Self::Store(text) => text.clone(),
        };
        f.write_str(&text)
    }
}

impl std::error::Error for LastFmError {}

/// api_sig: md5 of every parameter sorted by name (name + value, concatenated) plus the secret.
/// `format` and `callback` are not signed (they are added after).
pub fn signature(params: &[(String, String)], secret: &str) -> String {
    let mut sorted: Vec<&(String, String)> = params.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Md5::new();
    for (name, value) in sorted {
        hasher.update(name.as_bytes());
        hasher.update(value.as_bytes());
    }
    hasher.update(secret.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// application/x-www-form-urlencoded (reqwest is built without its "form" feature).
pub fn form_encode(params: &[(String, String)]) -> String {
    params.iter().map(|(name, value)| format!("{}={}", url_encode(name), url_encode(value))).collect::<Vec<_>>().join("&")
}

fn url_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn pair(name: &str, value: impl Into<String>) -> (String, String) {
    (name.to_owned(), value.into())
}

/// The track as Last.fm wants it: the first credited artist matches its catalogue best.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Song {
    pub artist: String,
    pub title: String,
    /// "" — unknown
    pub album: String,
    pub duration: Option<Duration>,
}

impl Song {
    /// None for podcast episodes and tracks without an artist or title.
    pub fn from_track(track: &TrackItem) -> Option<Self> {
        if track.is_episode {
            return None;
        }
        let first = track.artist_refs.iter().map(|artist| artist.name.trim()).find(|name| !name.is_empty());
        let artist = first.or_else(|| track.artists.split(", ").next()).unwrap_or_default().trim();
        // YouTube auto-generated channels: "Artist - Topic"
        let artist = artist.strip_suffix(" - Topic").unwrap_or(artist).trim().to_owned();
        let title = track.title.trim().to_owned();
        if artist.is_empty() || title.is_empty() || artist.eq_ignore_ascii_case("unknown artist") {
            return None;
        }
        let duration = (track.duration_ms > 0).then(|| Duration::from_millis(track.duration_ms as u64));
        Some(Self { artist, title, album: track.album.trim().to_owned(), duration })
    }

    /// track.updateNowPlaying (`index` None) or track.scrobble (`artist[0]`…).
    fn params(&self, index: Option<usize>, timestamp: Option<i64>) -> Vec<(String, String)> {
        let name = |key: &str| index.map_or_else(|| key.to_owned(), |i| format!("{key}[{i}]"));
        let mut params = vec![(name("artist"), self.artist.clone()), (name("track"), self.title.clone())];
        if let Some(timestamp) = timestamp {
            params.push((name("timestamp"), timestamp.to_string()));
        }
        if !self.album.is_empty() {
            params.push((name("album"), self.album.clone()));
        }
        if let Some(duration) = self.duration {
            params.push((name("duration"), duration.as_secs().to_string()));
        }
        params
    }
}

/// How much has to play before a listen counts: half the track, at most 3 minutes; unknown length — 3 minutes;
/// 30 s or shorter — never (None).
pub fn scrobble_after(duration: Option<Duration>) -> Option<Duration> {
    match duration {
        None => Some(SCROBBLE_CAP),
        Some(duration) if duration <= MIN_TRACK => None,
        Some(duration) => Some((duration / 2).min(SCROBBLE_CAP)),
    }
}

/// One listen of one track (LastFmScrobbler's timer): counts played time only while playing.
/// All times come from the caller, so the rule is deterministic.
#[derive(Clone, Debug)]
pub struct Listen {
    pub song: Song,
    /// unix seconds of the first moment it played (the scrobble timestamp); None — not played yet
    pub started_at: Option<i64>,
    /// play time still needed; None — never scrobbled
    remaining: Option<Duration>,
    /// playing since
    since: Option<Instant>,
    done: bool,
}

impl Listen {
    pub fn new(song: Song) -> Self {
        let remaining = scrobble_after(song.duration);
        Self { song, started_at: None, remaining, since: None, done: false }
    }

    /// true on the first play of this listen (time for "now playing").
    pub fn set_playing(&mut self, playing: bool, now: Instant, unix: i64) -> bool {
        let first = playing && self.started_at.is_none();
        if first {
            self.started_at = Some(unix);
        }
        match (playing, self.since) {
            (true, None) => self.since = Some(now),
            (false, Some(since)) => {
                self.since = None;
                if let Some(remaining) = &mut self.remaining {
                    *remaining = remaining.saturating_sub(now.saturating_duration_since(since));
                }
            }
            _ => {}
        }
        first
    }

    /// When the scrobble becomes due if playback keeps going; None — paused, sent, or never.
    pub fn due_at(&self) -> Option<Instant> {
        if self.done {
            return None;
        }
        Some(self.since? + self.remaining?)
    }

    /// true once, when enough has played by `now`.
    pub fn take_due(&mut self, now: Instant) -> bool {
        if self.due_at().is_some_and(|at| at <= now) {
            self.done = true;
            true
        } else {
            false
        }
    }
}

#[derive(Clone)]
struct Api {
    http: sf_api::Http,
    url: String,
}

impl Api {
    async fn call(
        &self,
        method: &str,
        extra: Vec<(String, String)>,
        key: &str,
        secret: &str,
        session: Option<&str>,
    ) -> Result<Value, LastFmError> {
        let mut params = vec![pair("method", method), pair("api_key", key)];
        if let Some(session) = session {
            params.push(pair("sk", session));
        }
        params.extend(extra);
        let sig = signature(&params, secret);
        params.push(pair("api_sig", sig));
        params.push(pair("format", "json"));
        let client = self.http.client().map_err(|_| LastFmError::Network)?;
        let response = client
            .post(&self.url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(form_encode(&params))
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|error| {
                // the error names the URL only: the parameters (and the session key) are in the body
                tracing::warn!("Last.fm {method}: {error}");
                LastFmError::Network
            })?;
        // errors come with 4xx and a JSON body as well
        let text = response.text().await.map_err(|_| LastFmError::Network)?;
        let root: Value = serde_json::from_str(&text).map_err(|_| LastFmError::Unreadable)?;
        if let Some(code) = root.get("error").and_then(Value::as_i64) {
            let message = root.get("message").and_then(Value::as_str).unwrap_or_default().to_owned();
            return Err(LastFmError::Api { code, message });
        }
        if root.is_object() { Ok(root) } else { Err(LastFmError::Unreadable) }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LastFmSnapshot {
    /// the credential store has been read
    pub loaded: bool,
    pub has_keys: bool,
    pub connected: bool,
    pub username: String,
    /// shown back in the settings field (the secret never is)
    pub api_key: String,
    pub signing_in: bool,
    /// the approval page while signing in (to open it again if the browser didn't come up)
    pub auth_url: String,
    pub error_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LastFmEvent {
    Changed,
    /// signed in as this user
    Connected(String),
    NowPlaying { artist: String, title: String },
    Scrobbled { artist: String, title: String },
}

type OpenBrowser = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
type Clock<T> = Arc<dyn Fn() -> T + Send + Sync>;

/// Endpoints, waits and clocks — replaced in tests.
#[derive(Clone)]
pub struct LastFmOptions {
    pub api_url: String,
    pub auth_url: String,
    pub sign_in_poll: Duration,
    pub sign_in_timeout: Duration,
    pub open_browser: OpenBrowser,
    /// monotonic time for the scrobble rule
    pub now: Clock<Instant>,
    /// unix seconds (scrobble timestamps)
    pub unix_now: Clock<i64>,
    /// scrobbles not sent yet survive restarts here; None — in memory only
    pub queue_file: Option<std::path::PathBuf>,
    pub retry_delay: Duration,
}

impl Default for LastFmOptions {
    fn default() -> Self {
        Self {
            api_url: API_URL.to_owned(),
            auth_url: AUTH_URL.to_owned(),
            sign_in_poll: SIGN_IN_POLL,
            sign_in_timeout: SIGN_IN_TIMEOUT,
            open_browser: Arc::new(sf_auth::open_browser),
            now: Arc::new(Instant::now),
            unix_now: Arc::new(|| jiff::Timestamp::now().as_second()),
            queue_file: Some(mf_platform::dirs::app_data_dir().join("lastfm-queue.json")),
            retry_delay: RETRY_DELAY,
        }
    }
}

#[derive(Default)]
struct State {
    account: LastFmAccount,
    loaded: bool,
    signing_in: bool,
    /// a newer sign-in or a cancel makes an older poll stop
    sign_in: u64,
    auth_url: String,
    error_text: String,
    /// (track uri, listen)
    listen: Option<(String, Listen)>,
    /// the scrobble timer that is armed (a newer one replaces it)
    timer: u64,
    armed: Option<Instant>,
    /// scrobbles not sent yet (saved to options.queue_file)
    queue: Queue,
    /// one flush at a time; a request during it runs another after
    flushing: bool,
    flush_again: bool,
    retry_armed: bool,
}

pub struct LastFmService {
    rt: Handle,
    settings: Arc<SettingsStore>,
    store: Store,
    api: Api,
    options: LastFmOptions,
    events: broadcast::Sender<LastFmEvent>,
    state: Mutex<State>,
    me: Weak<LastFmService>,
}

impl LastFmService {
    /// Reads the saved account from `store`; call `attach_player` to start scrobbling.
    pub fn new(rt: Handle, settings: Arc<SettingsStore>, http: sf_api::Http, store: Store) -> Arc<Self> {
        Self::with_options(rt, settings, http, store, LastFmOptions::default())
    }

    pub fn with_options(
        rt: Handle,
        settings: Arc<SettingsStore>,
        http: sf_api::Http,
        store: Store,
        options: LastFmOptions,
    ) -> Arc<Self> {
        let queue = Queue::load(options.queue_file.clone());
        let service = Arc::new_cyclic(|me| Self {
            rt,
            settings,
            store,
            api: Api { http, url: options.api_url.clone() },
            options,
            events: broadcast::channel(64).0,
            state: Mutex::new(State { queue, ..Default::default() }),
            me: me.clone(),
        });
        service.load();
        service
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LastFmEvent> {
        self.events.subscribe()
    }

    pub fn snapshot(&self) -> LastFmSnapshot {
        let state = lock(&self.state);
        LastFmSnapshot {
            loaded: state.loaded,
            has_keys: state.account.has_keys(),
            connected: state.account.connected(),
            username: state.account.username.clone(),
            api_key: state.account.api_key.clone(),
            signing_in: state.signing_in,
            auth_url: state.auth_url.clone(),
            error_text: state.error_text.clone(),
        }
    }

    fn changed(&self) {
        let _ = self.events.send(LastFmEvent::Changed);
    }

    fn set_error(&self, error: impl ToString) {
        lock(&self.state).error_text = error.to_string();
        self.changed();
    }

    fn load(&self) {
        let lease = self.store.lease(Slot::LastFm);
        let me = self.me.clone();
        self.rt.spawn(async move {
            let loaded = lease.load().await;
            let Some(service) = me.upgrade() else { return };
            {
                let mut state = lock(&service.state);
                state.loaded = true;
                match loaded {
                    Ok(loaded) => {
                        if let Some(StoredGrant::LastFm(account)) = loaded.grant {
                            // set_keys during the read wins
                            if !state.account.has_keys() {
                                state.account = account;
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!("saved Last.fm account not read: {error}");
                        state.error_text = error.to_string();
                    }
                }
            }
            service.changed();
            // scrobbles left from the last run
            service.flush();
        });
    }

    /// Queues the write at once (writes keep their order) and reports a failure when it finishes.
    fn save(&self, account: LastFmAccount) {
        let pending = self.store.lease(Slot::LastFm).save(StoredGrant::LastFm(account));
        let me = self.me.clone();
        self.rt.spawn(async move {
            if let Err(error) = pending.await {
                tracing::warn!("Last.fm account not saved: {error}");
                if let Some(service) = me.upgrade() {
                    service.set_error(error);
                }
            }
        });
    }

    /// The user's own API key and shared secret. An empty `secret` keeps the saved one; other keys end the session.
    pub fn set_keys(&self, api_key: &str, secret: &str) {
        let account = {
            let mut state = lock(&self.state);
            let secret = if secret.trim().is_empty() { state.account.secret.clone() } else { secret.trim().to_owned() };
            let account = LastFmAccount::new(api_key, &secret);
            if account.api_key == state.account.api_key && account.secret == state.account.secret {
                return;
            }
            state.account = account.clone();
            state.error_text.clear();
            account
        };
        if account.has_keys() {
            self.save(account);
        }
        self.changed();
    }

    /// Step 1 and 2 of the sign-in: a token, the approval page in the browser, then waiting for the approval.
    pub fn begin_sign_in(&self) {
        let (account, generation) = {
            let mut state = lock(&self.state);
            if !state.account.has_keys() {
                drop(state);
                self.set_error(LastFmError::NoKeys);
                return;
            }
            state.sign_in += 1;
            state.signing_in = true;
            state.auth_url.clear();
            state.error_text.clear();
            (state.account.clone(), state.sign_in)
        };
        self.changed();
        let me = self.me.clone();
        self.rt.spawn(async move {
            let Some(service) = me.upgrade() else { return };
            let result = service.sign_in(account, generation).await;
            let connected = {
                let mut state = lock(&service.state);
                if state.sign_in != generation {
                    return;
                }
                state.signing_in = false;
                state.auth_url.clear();
                match result {
                    Ok(username) => Some(username),
                    Err(error) => {
                        state.error_text = error.to_string();
                        None
                    }
                }
            };
            let signed_in = connected.is_some();
            if let Some(username) = connected {
                let _ = service.events.send(LastFmEvent::Connected(username));
            }
            service.changed();
            if signed_in {
                service.flush();
            }
        });
    }

    async fn sign_in(&self, account: LastFmAccount, generation: u64) -> Result<String, LastFmError> {
        let (key, secret) = (account.api_key.as_str(), account.secret.as_str());
        let answer = self.api.call("auth.getToken", Vec::new(), key, secret, None).await?;
        let token = answer.get("token").and_then(Value::as_str).filter(|t| !t.is_empty()).ok_or(LastFmError::Unreadable)?;
        let url = format!("{}?api_key={}&token={}", self.options.auth_url, url_encode(key), url_encode(token));
        {
            let mut state = lock(&self.state);
            if state.sign_in != generation {
                return Err(LastFmError::NotConnected);
            }
            state.auth_url = url.clone();
        }
        if let Err(error) = (self.options.open_browser)(&url) {
            tracing::warn!("browser not opened: {error}");
            lock(&self.state).error_text = mf_i18n::t("Couldn't open the browser — open the approval link yourself.");
        }
        self.changed();
        let deadline = (self.options.now)() + self.options.sign_in_timeout;
        loop {
            tokio::time::sleep(self.options.sign_in_poll).await;
            if lock(&self.state).sign_in != generation {
                return Err(LastFmError::NotConnected);
            }
            match self.api.call("auth.getSession", vec![pair("token", token)], key, secret, None).await {
                Ok(answer) => {
                    let session = &answer["session"];
                    let session_key =
                        session.get("key").and_then(Value::as_str).filter(|k| !k.is_empty()).ok_or(LastFmError::Unreadable)?;
                    let username = session.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
                    let connected = LastFmAccount { session_key: session_key.to_owned(), username: username.clone(), ..account };
                    let mut state = lock(&self.state);
                    if state.sign_in != generation {
                        return Err(LastFmError::NotConnected);
                    }
                    // queued before anyone can see the new state, so a later read finds it
                    self.save(connected.clone());
                    state.account = connected;
                    return Ok(username);
                }
                Err(LastFmError::Api { code: ERROR_NOT_APPROVED, .. }) if (self.options.now)() < deadline => {}
                Err(LastFmError::Api { code: ERROR_NOT_APPROVED, .. }) => return Err(LastFmError::TimedOut),
                Err(error) => return Err(error),
            }
        }
    }

    pub fn cancel_sign_in(&self) {
        {
            let mut state = lock(&self.state);
            state.sign_in += 1;
            state.signing_in = false;
            state.auth_url.clear();
        }
        self.changed();
    }

    /// Forget the session (the keys stay, so Connect works again at once).
    pub fn disconnect(&self) {
        let account = {
            let mut state = lock(&self.state);
            state.sign_in += 1;
            state.signing_in = false;
            state.auth_url.clear();
            state.error_text.clear();
            state.account = state.account.disconnected();
            state.account.clone()
        };
        if account.has_keys() {
            self.save(account);
        }
        self.changed();
    }

    /// Remove the keys and the session from the credential store.
    pub fn forget(&self) {
        {
            let mut state = lock(&self.state);
            state.sign_in += 1;
            state.signing_in = false;
            state.auth_url.clear();
            state.account = LastFmAccount::default();
            state.error_text = self.store.revoke(Slot::LastFm).err().map(|e| e.to_string()).unwrap_or_default();
        }
        let pending = self.store.lease(Slot::LastFm).delete();
        self.rt.spawn(async move {
            if let Err(error) = pending.await {
                tracing::warn!("Last.fm account not deleted: {error}");
            }
        });
        self.changed();
    }

    /// Follow this player: local tracks (Spotify on this PC, YouTube Music) are reported.
    pub fn attach_player(&self, player: PlayerHandle) {
        let mut events = player.subscribe().events;
        let me = self.me.clone();
        self.rt.spawn(async move {
            loop {
                let event = match events.recv().await {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => PlayerEvent::StateChanged,
                    Err(broadcast::error::RecvError::Closed) => return,
                };
                let Some(service) = me.upgrade() else { return };
                match event {
                    PlayerEvent::TrackStarted(started) if started.local => {
                        let snapshot = player.snapshot();
                        let mut track = started.track;
                        if track.duration_ms <= 0 {
                            track.duration_ms = snapshot.duration_ms;
                        }
                        service.track_started(&track, snapshot.playing && !snapshot.buffering);
                    }
                    PlayerEvent::TrackEnded(ended) => service.track_ended(&ended.track),
                    PlayerEvent::StateChanged | PlayerEvent::TrackChanged => {
                        let snapshot = player.snapshot();
                        service.set_playing(snapshot.playing && !snapshot.buffering);
                    }
                    _ => {}
                }
            }
        });
    }

    /// A new listen (also of the same track again).
    pub fn track_started(&self, track: &TrackItem, playing: bool) {
        {
            let mut state = lock(&self.state);
            state.listen = Song::from_track(track).map(|song| (track.uri.clone(), Listen::new(song)));
            state.timer += 1;
            state.armed = None;
        }
        self.set_playing(playing);
    }

    pub fn track_ended(&self, track: &TrackItem) {
        let mut state = lock(&self.state);
        if state.listen.as_ref().is_some_and(|(uri, _)| *uri == track.uri) {
            state.listen = None;
            state.timer += 1;
            state.armed = None;
        }
    }

    /// Playback started or stopped (pause, buffering): the played-time timer follows.
    pub fn set_playing(&self, playing: bool) {
        let now = (self.options.now)();
        let (first, song, due, timer) = {
            let mut state = lock(&self.state);
            let Some((_, listen)) = state.listen.as_mut() else { return };
            let first = listen.set_playing(playing, now, (self.options.unix_now)());
            let (song, due) = (listen.song.clone(), listen.due_at());
            if due != state.armed {
                state.armed = due;
                state.timer += 1;
            }
            (first, song, due, state.timer)
        };
        if first {
            self.now_playing(song);
        }
        if let Some(at) = due {
            let me = self.me.clone();
            self.rt.spawn(async move {
                tokio::time::sleep_until(at).await;
                if let Some(service) = me.upgrade() {
                    service.fire(timer);
                }
            });
        }
    }

    /// Check the scrobble rule now (the armed timer does this by itself).
    pub fn poll(&self) {
        let timer = lock(&self.state).timer;
        self.fire(timer);
    }

    fn fire(&self, timer: u64) {
        let now = (self.options.now)();
        let (song, started_at) = {
            let mut state = lock(&self.state);
            if state.timer != timer {
                return;
            }
            let Some((_, listen)) = state.listen.as_mut() else { return };
            if !listen.take_due(now) {
                return;
            }
            let found = (listen.song.clone(), listen.started_at.unwrap_or_else(|| (self.options.unix_now)()));
            state.armed = None;
            found
        };
        if !self.settings.get_bool(SettingKey::LastFmScrobble) {
            return;
        }
        {
            let mut state = lock(&self.state);
            if !state.account.connected() {
                return;
            }
            // queued first (and saved): a failed send is retried later, also after a restart
            let pending = Pending {
                user: state.account.username.clone(),
                artist: song.artist,
                title: song.title,
                album: song.album,
                duration_s: song.duration.map_or(0, |duration| duration.as_secs()),
                timestamp: started_at,
            };
            state.queue.push(pending);
        }
        self.flush();
    }

    /// How many scrobbles wait to be sent.
    pub fn queued(&self) -> usize {
        lock(&self.state).queue.len()
    }

    /// Send the waiting scrobbles, 50 per call, oldest first. A network failure or a busy Last.fm keeps them
    /// and tries again after `retry_delay`; also on the next scrobble, sign-in and start.
    pub fn flush(&self) {
        {
            let mut state = lock(&self.state);
            if state.queue.is_empty() || !state.account.connected() {
                return;
            }
            if state.flushing {
                state.flush_again = true;
                return;
            }
            state.flushing = true;
        }
        let me = self.me.clone();
        self.rt.spawn(async move {
            let Some(service) = me.upgrade() else { return };
            service.send_queue().await;
            let again = {
                let mut state = lock(&service.state);
                state.flushing = false;
                std::mem::take(&mut state.flush_again)
            };
            if again {
                service.flush();
            }
        });
    }

    async fn send_queue(&self) {
        loop {
            let batch = {
                let mut state = lock(&self.state);
                let dropped = state.queue.prune((self.options.unix_now)());
                if dropped > 0 {
                    tracing::warn!("{dropped} Last.fm scrobbles were too old to send");
                }
                let user = state.account.username.clone();
                state.queue.batch(&user)
            };
            if batch.is_empty() {
                return;
            }
            match self.signed("track.scrobble", batch_params(&batch)).await {
                Ok(()) => {
                    lock(&self.state).queue.remove(&batch);
                    for item in batch {
                        let _ = self.events.send(LastFmEvent::Scrobbled { artist: item.artist, title: item.title });
                    }
                }
                Err(error) if rejects_batch(&error) => {
                    tracing::warn!("Last.fm refused {} scrobbles: {error}", batch.len());
                    lock(&self.state).queue.remove(&batch);
                }
                Err(error) => {
                    tracing::warn!("Last.fm scrobble failed, {} kept for later: {error}", batch.len());
                    if transient(&error) {
                        self.retry_later();
                    }
                    return;
                }
            }
        }
    }

    fn retry_later(&self) {
        {
            let mut state = lock(&self.state);
            if state.retry_armed {
                return;
            }
            state.retry_armed = true;
        }
        let me = self.me.clone();
        let delay = self.options.retry_delay;
        self.rt.spawn(async move {
            tokio::time::sleep(delay).await;
            let Some(service) = me.upgrade() else { return };
            lock(&service.state).retry_armed = false;
            service.flush();
        });
    }

    fn now_playing(&self, song: Song) {
        if !self.settings.get_bool(SettingKey::LastFmNowPlaying) || !lock(&self.state).account.connected() {
            return;
        }
        let me = self.me.clone();
        self.rt.spawn(async move {
            let Some(service) = me.upgrade() else { return };
            match service.signed("track.updateNowPlaying", song.params(None, None)).await {
                Ok(()) => {
                    let _ = service.events.send(LastFmEvent::NowPlaying { artist: song.artist, title: song.title });
                }
                Err(error) => tracing::warn!("Last.fm now playing failed: {error}"),
            }
        });
    }

    async fn signed(&self, method: &str, params: Vec<(String, String)>) -> Result<(), LastFmError> {
        let account = lock(&self.state).account.clone();
        if !account.connected() {
            return Err(LastFmError::NotConnected);
        }
        let result =
            self.api.call(method, params, &account.api_key, &account.secret, Some(&account.session_key)).await;
        if let Err(error @ LastFmError::Api { code: ERROR_INVALID_SESSION, .. }) = &result {
            // revoked on last.fm: forget it so Settings shows Connect again
            let forgotten = {
                let mut state = lock(&self.state);
                let current = state.account.session_key == account.session_key;
                if current {
                    self.save(account.disconnected());
                    state.account = account.disconnected();
                    state.error_text = error.to_string();
                }
                current
            };
            if forgotten {
                self.changed();
            }
        }
        result.map(|_| ())
    }
}

#[cfg(test)]
mod tests;
