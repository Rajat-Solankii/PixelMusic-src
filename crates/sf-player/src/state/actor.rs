//! Актор плеера: mpsc команд → ядро → снимок в watch, затем события в broadcast (по одному на Qt-сигнал).
//! Сетевые команды управления идут по одной (порядок важен), чтения — параллельно.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(test)]
use md3_model::QueueRow;
use md3_model::{DeviceItem, PlayContext, QueueSection, TrackItem};
use tokio::sync::{broadcast, mpsc, watch};

use super::backend::{LocalEngine, WebBackend};
use super::core::{ApiCall, ApiReply, Core, Effect};
use super::diff;
use super::session::SessionData;
use super::youtube::{YtEffect, YtLane, YtLink, YtMsg};
use crate::engine::youtube::{YtCommand, is_lane_track};
use crate::engine::{EngineConfig, EngineEvent, LoadSpec, Playback};
#[cfg(test)]
use crate::QueueChange;
use crate::{LocalPlayback, PlayerEvent, PlayerSnapshot, Repeat, Target, codec_label};

/// Пока что-то движется — снимок пересчитывается так часто.
const FRAME: Duration = Duration::from_millis(250);
/// «Дыхание» обложки — ~30 Гц.
const LEVEL_FRAME: Duration = Duration::from_millis(33);
const IDLE: Duration = Duration::from_secs(5);
/// Шаг позиции в снимке во время игры, мс.
const POSITION_STEP_MS: i64 = 200;

pub(crate) enum LocalMsg {
    Connecting,
    Ready { engine: Arc<dyn LocalEngine>, device_id: String, name: String, bitrate: u16, autoplay: bool },
    Failed(String),
    Unavailable,
}

#[derive(Debug)]
pub(crate) enum Command {
    PlayItems { items: Vec<TrackItem>, row: usize, context: PlayContext },
    ShuffleFrom { items: Vec<TrackItem>, context: PlayContext },
    PlayContext { context: PlayContext, offset: Option<String>, shuffle: Option<bool> },
    PlayUris { uris: Vec<String>, index: usize, context: PlayContext },
    /// Строка для оптимистичной сверки: списки присылают только URI.
    Remember(Box<TrackItem>),
    PlayEpisode { uri: String, position_ms: i64 },
    /// more tracks for the playing YouTube context (a radio filling in after its first song)
    ExtendContext { context_uri: String, items: Vec<TrackItem> },
    /// Autoplay: the radio of `seed_uri` after the lane's list (ignored once another song plays).
    Autoplay { seed_uri: String, items: Vec<TrackItem> },
    PlayIndex(usize),
    PlayQueueRow(u64),
    AddToQueue(Vec<TrackItem>),
    AddUriToQueue(String),
    InsertManual { item: Box<TrackItem>, at: usize },
    MoveManual { from: usize, to: usize },
    /// reorder the rest of the playing context (indices within that section; only the YouTube lane)
    MoveContext { from: usize, to: usize },
    ClearManualQueue,
    /// stop and forget the current song and its queue ("Nothing playing")
    Clear,
    RefreshQueue,
    TogglePlay,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    Seek(f64),
    SeekMs(i64),
    SeekRelative(i64),
    CycleRepeat,
    SetRepeat(Repeat),
    SetShuffle(bool),
    SetVolume(f64),
    SetLevelsWanted(bool),
    SetAutoplay(bool),
    TransferHere(bool),
    TransferTo(String, bool),
    RefreshRemote,
    SetAppActive(bool),
    ResetAccount,
    ExpectLocal(bool),
    /// перезапуск движка с новыми настройками, с подхватом контекста и ручной очереди
    RestartEngine(Box<EngineConfig>),
}

pub(crate) enum Msg {
    Command(Command),
    Engine(Box<EngineEvent>),
    Api(ApiReply),
    Local(LocalMsg),
    Cover { url: String, result: Result<PathBuf, String> },
    Restore(Option<SessionData>),
    YouTube(YtMsg),
    Shutdown(std::sync::mpsc::Sender<()>),
}

/// Как пересоздать сессию librespot (громкость — для нового движка).
pub(crate) type Reconnect = Arc<dyn Fn(u16) + Send + Sync>;
/// Перезапуск движка с настройками; LoadSpec — что загрузить в новый (None — подхват движком).
pub(crate) type Restart = Arc<dyn Fn(EngineConfig, u16, Option<LoadSpec>) + Send + Sync>;

pub(crate) struct Actor {
    pub core: Core,
    rx: mpsc::UnboundedReceiver<Msg>,
    tx: mpsc::UnboundedSender<Msg>,
    snapshot: Arc<watch::Sender<PlayerSnapshot>>,
    events: broadcast::Sender<PlayerEvent>,
    backend: Arc<dyn WebBackend>,
    control: mpsc::UnboundedSender<ApiCall>,
    /// записи очереди (альбом — десятки POST) — своя упорядоченная очередь, транспорт за ними не ждёт
    queue_writes: mpsc::UnboundedSender<ApiCall>,
    engine: Option<Arc<dyn LocalEngine>>,
    reconnect: Reconnect,
    restart: Restart,
    session_file: Option<PathBuf>,
    published: PlayerSnapshot,
    cover_url: String,
    cover_file: String,
    level: f64,
    restored: bool,
    pending: Vec<PlayerEvent>,
    /// YouTube Music: its own queue and player beside `Core`
    youtube: YtLane,
    yt: Option<YtLink>,
    /// the librespot load that was current when YouTube took over: a new one means Spotify plays again
    yt_spotify_sequence: u64,
    /// the cover `Core` asked for last (shown again when the lane stops)
    core_cover: String,
    yt_was_active: bool,
    /// the song autoplay was last asked for (once per song)
    autoplay_asked: Option<String>,
    /// the song autoplay is asked for and how many of its answers added nothing (each is asked again, up to
    /// AUTOPLAY_RETRIES: the services move on along the radio)
    autoplay_empty: (String, u32),
}

/// Answers that add nothing before autoplay stops asking for one song.
const AUTOPLAY_RETRIES: u32 = 3;

impl Actor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        core: Core,
        rx: mpsc::UnboundedReceiver<Msg>,
        tx: mpsc::UnboundedSender<Msg>,
        snapshot: Arc<watch::Sender<PlayerSnapshot>>,
        events: broadcast::Sender<PlayerEvent>,
        backend: Arc<dyn WebBackend>,
        reconnect: Reconnect,
        restart: Restart,
        session_file: Option<PathBuf>,
    ) -> Self {
        let published = snapshot.borrow().clone();
        // управление — строго по порядку; записи очереди — по порядку между собой
        let control = ordered(backend.clone(), tx.clone());
        let queue_writes = ordered(backend.clone(), tx.clone());
        Self {
            core,
            rx,
            tx,
            snapshot,
            events,
            backend,
            control,
            queue_writes,
            engine: None,
            reconnect,
            restart,
            session_file,
            published,
            cover_url: String::new(),
            cover_file: String::new(),
            level: 0.0,
            restored: false,
            pending: Vec::new(),
            youtube: YtLane::default(),
            yt: None,
            yt_spotify_sequence: 0,
            core_cover: String::new(),
            yt_was_active: false,
            autoplay_asked: None,
            autoplay_empty: (String::new(), 0),
        }
    }

    pub async fn run(mut self) {
        loop {
            let wait = self.wait();
            match tokio::time::timeout(wait, self.rx.recv()).await.ok() {
                Some(None) => break,
                Some(Some(Msg::Shutdown(done))) => {
                    self.shutdown();
                    let _ = done.send(());
                    break;
                }
                Some(Some(message)) => self.handle(message),
                None => {}
            }
            // всё, что уже пришло, — одним снимком
            while let Ok(message) = self.rx.try_recv() {
                if let Msg::Shutdown(done) = message {
                    self.shutdown();
                    let _ = done.send(());
                    return;
                }
                self.handle(message);
            }
            self.step();
        }
    }

    /// Обработать всё, что уже пришло (для тестов).
    #[cfg(test)]
    pub fn drain(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            self.handle(message);
        }
    }

    fn wait(&self) -> Duration {
        if self.levels_active() {
            return LEVEL_FRAME;
        }
        if self.core.animating() || (self.youtube.is_active() && (self.youtube.playing() || self.youtube.loading())) {
            return FRAME;
        }
        self.core.next_deadline().saturating_duration_since(Instant::now()).clamp(Duration::from_millis(10), IDLE)
    }

    fn levels_active(&self) -> bool {
        if self.youtube.is_active() {
            return self.core.levels_wanted && self.yt.is_some() && self.published.playing;
        }
        self.core.levels_wanted && self.engine.is_some() && self.core.local.is_active() && self.published.playing
    }

    /// Таймеры ядра, эффекты, снимок, события.
    pub fn step(&mut self) {
        self.core.tick();
        self.run_effects();
        self.run_youtube_effects();
        self.level = if !self.levels_active() {
            0.0
        } else if self.youtube.is_active() {
            self.yt.as_ref().map_or(0.0, |yt| yt.engine.level())
        } else {
            self.engine.as_ref().map_or(0.0, |engine| engine.level())
        };
        self.publish();
    }

    pub fn handle(&mut self, message: Msg) {
        match message {
            Msg::Command(command) => self.command(command),
            Msg::Engine(event) => match *event {
                EngineEvent::State(state) => {
                    // Spotify started a new track here (a Connect command, autoplay): it takes over
                    if self.youtube.is_active()
                        && state.playback == Playback::Playing
                        && state.track_sequence != self.yt_spotify_sequence
                    {
                        self.youtube.stop();
                    }
                    self.core.handle_local(state);
                }
                EngineEvent::SessionEnded => {
                    let mut state = self.core.local.clone();
                    state.connected = false;
                    self.core.handle_local(state);
                    (self.reconnect)(self.core.volume);
                }
            },
            Msg::Api(reply) => self.core.handle_api(reply),
            Msg::Local(local) => self.local(local),
            Msg::Cover { url, result } => {
                if url == self.cover_url {
                    self.cover_file = match result {
                        Ok(path) => path.to_string_lossy().into_owned(),
                        Err(error) => {
                            tracing::debug!("cover not downloaded: {error:#}");
                            String::new()
                        }
                    };
                }
            }
            Msg::Restore(data) => {
                if !self.restored {
                    self.restored = true;
                    // the last song goes back to the lane (YouTube, local files). Spotify is no longer played: a
                    // Spotify song from an old session is not restored, Play would find nothing to play with it
                    if let Some(data) = data
                        && let Some(track) = data.track.filter(|track| is_lane_track(&track.uri))
                    {
                        self.youtube.restore(track, data.rows, u64::from(data.position_ms), data.context);
                    }
                }
                self.core.poll_remote_soon();
            }
            Msg::YouTube(message) => self.youtube_message(message),
            Msg::Shutdown(_) => {}
        }
    }

    fn local(&mut self, local: LocalMsg) {
        match local {
            LocalMsg::Connecting => self.core.handle_playback(LocalPlayback::Connecting),
            LocalMsg::Ready { engine, device_id, name, bitrate, autoplay } => {
                // движок мог подключиться уже после запроса уровней — синхронизируем гейт tap'а
                engine.set_levels_wanted(self.core.levels_wanted);
                self.engine = Some(engine);
                self.core.local_device_name = name;
                self.core.bitrate = bitrate;
                self.core.autoplay = autoplay;
                self.core.handle_playback(LocalPlayback::Ready { device_id });
                self.core.poll_remote_soon();
            }
            LocalMsg::Failed(text) => {
                self.engine = None;
                self.core.handle_playback(LocalPlayback::Failed(text));
            }
            LocalMsg::Unavailable => {
                self.engine = None;
                self.core.handle_playback(LocalPlayback::Unavailable);
            }
        }
    }

    fn command(&mut self, command: Command) {
        let Some(command) = self.youtube_command(command) else {
            return;
        };
        let core = &mut self.core;
        match command {
            Command::PlayItems { items, row, context } => core.play_items(items, row, context),
            Command::ShuffleFrom { items, context } => core.shuffle_from(items, context),
            Command::PlayContext { context, offset, shuffle } => core.play_context(context, offset, shuffle),
            Command::PlayUris { uris, index, context } => core.play_uris(uris, index, context),
            Command::Remember(item) => core.remember(&item),
            Command::PlayEpisode { uri, position_ms } => core.play_episode(uri, position_ms),
            // the YouTube lane takes these; Spotify contexts are not extended
            Command::ExtendContext { .. } | Command::Autoplay { .. } => {}
            Command::PlayIndex(index) => {
                // индекс строки Player.queue: 0 — текущая
                let rows = core.queue_rows();
                if let Some(row) = rows.get(index)
                    && row.section != QueueSection::Current
                {
                    let uid = row.uid;
                    if let Some((at, uri)) = core.find_row(uid) {
                        core.play_queue_item(at, uri);
                    }
                }
            }
            Command::PlayQueueRow(uid) => {
                if let Some((at, uri)) = core.find_row(uid) {
                    core.play_queue_item(at, uri);
                }
            }
            Command::AddToQueue(items) => match items.len() {
                0 => {}
                1 => {
                    let item = items.into_iter().next().expect("one item");
                    core.remember(&item);
                    let uri = item.play_uri();
                    core.add_to_queue(uri, item.title);
                }
                _ => core.queue_many(items),
            },
            Command::AddUriToQueue(uri) => core.add_to_queue(uri, String::new()),
            Command::InsertManual { item, at } => core.insert_manual(vec![*item], at),
            Command::MoveManual { from, to } => core.move_manual(from, to),
            Command::MoveContext { .. } => {}
            Command::ClearManualQueue => core.clear_queue(),
            // Spotify keeps its context on the device: here clearing can only stop it
            Command::Clear => core.pause(),
            Command::RefreshQueue => core.refresh_queue(true),
            Command::TogglePlay => core.toggle_play(),
            Command::Play => core.play(),
            Command::Pause | Command::Stop => core.pause(),
            Command::Next => core.next(),
            Command::Previous => core.previous(),
            Command::Seek(fraction) => {
                if let Some(now) = core.now_playing() {
                    let position = (fraction.clamp(0.0, 1.0) * f64::from(now.duration_ms)).round() as u32;
                    core.seek(position);
                }
            }
            Command::SeekMs(ms) => core.seek(ms.clamp(0, i64::from(u32::MAX)) as u32),
            Command::SeekRelative(delta) => core.seek_relative(delta),
            Command::CycleRepeat => {
                let mode = core.repeat().next();
                core.set_repeat(mode);
            }
            Command::SetRepeat(repeat) => core.set_repeat(repeat.to_engine()),
            Command::SetShuffle(value) => core.set_shuffle(value),
            Command::SetVolume(value) => core.set_volume(crate::volume_to_u16(value)),
            Command::SetLevelsWanted(value) => {
                core.levels_wanted = value;
                // движок узнаёт о гейте сразу: буфер tap'а не наполняется, пока уровень не нужен
                if let Some(engine) = &self.engine {
                    engine.set_levels_wanted(value);
                }
            }
            Command::SetAutoplay(value) => core.autoplay = value,
            Command::TransferHere(play) => core.transfer_here(play),
            Command::TransferTo(device, play) => core.transfer_to(device, play),
            Command::RefreshRemote => core.poll_remote(),
            Command::SetAppActive(active) => core.set_app_active(active),
            Command::ResetAccount => core.reset_account(),
            Command::ExpectLocal(expected) => core.expect_local(expected),
            Command::RestartEngine(config) => {
                let resume = core.prepare_restart();
                (self.restart)(*config, core.volume, resume);
            }
        }
    }

    fn run_effects(&mut self) {
        for effect in self.core.take_effects() {
            match effect {
                Effect::Engine(command) => match &self.engine {
                    Some(engine) => {
                        if let Err(error) = engine.command(command) {
                            tracing::warn!("engine command failed: {error:#}");
                        }
                    }
                    None => tracing::debug!("engine not ready, command {command:?} skipped"),
                },
                Effect::Api(call) => self.api(call),
                Effect::Notice(text) => self.pending.push(PlayerEvent::Notice(text)),
                Effect::Reconnect => (self.reconnect)(self.core.volume),
                Effect::Cover(url) => {
                    self.core_cover = url.clone();
                    if !self.youtube.is_active() {
                        self.cover(url);
                    }
                }
                Effect::Save(data) => {
                    let data = self.with_lane_session(*data);
                    if let Some(path) = self.session_file.clone() {
                        tokio::task::spawn_blocking(move || {
                            if let Err(error) = data.save(&path) {
                                tracing::warn!("player session not saved: {error:#}");
                            }
                        });
                    }
                }
                Effect::Started(started) => self.pending.push(PlayerEvent::TrackStarted((*started).into())),
                Effect::Ended(ended) => self.pending.push(PlayerEvent::TrackEnded((*ended).into())),
            }
        }
    }

    fn cover(&mut self, url: String) {
        if url.is_empty() {
            self.cover_url.clear();
            self.cover_file.clear();
            return;
        }
        if url == self.cover_url && !self.cover_file.is_empty() {
            return;
        }
        self.cover_url = url.clone();
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend.fetch_cover(url.clone()).await;
            let _ = tx.send(Msg::Cover { url, result });
        });
    }

    pub(crate) fn api(&mut self, call: ApiCall) {
        match call {
            // чтения — параллельно
            ApiCall::Playback { .. } | ApiCall::Queue { .. } | ApiCall::AlbumTracks { .. } | ApiCall::TrackInfo { .. } => {
                let backend = self.backend.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let reply = backend.call(call).await;
                    let _ = tx.send(Msg::Api(reply));
                });
            }
            call @ ApiCall::AddMany { .. } => {
                let _ = self.queue_writes.send(call);
            }
            call => {
                let _ = self.control.send(call);
            }
        }
    }

    pub fn build(&mut self) -> PlayerSnapshot {
        if self.youtube.is_active() {
            return self.build_youtube();
        }
        let core = &mut self.core;
        let now = core.now_playing();
        let rows = core.queue_rows();
        let queue = if rows.as_slice() == self.published.queue.as_slice() {
            self.published.queue.clone()
        } else {
            Arc::new(rows)
        };
        let target = core.target();
        let context_remaining = queue.iter().filter(|row| row.section == QueueSection::Context).count() as i32;
        let manual_count = queue.iter().filter(|row| row.section == QueueSection::Manual).count() as i32;
        let live = now.as_ref().filter(|now| !now.resuming);
        let is_local = live.is_some_and(|now| now.local) || (target == Target::Local && core.local.is_active());
        let volume = now.as_ref().map_or(core.volume, |now| if now.resuming { core.volume } else { now.volume });
        PlayerSnapshot {
            track: now.as_ref().map(|now| now.track.clone()),
            codec: if is_local && now.is_some() { codec_label(core.bitrate) } else { String::new() },
            cover_file: self.cover_file.clone(),
            playing: now.as_ref().is_some_and(|now| now.playing),
            buffering: now.as_ref().is_some_and(|now| now.loading),
            narrating: is_local && core.local.narrating,
            position_ms: now.as_ref().map_or(0, |now| i64::from(now.position_ms)),
            duration_ms: now.as_ref().map_or(0, |now| i64::from(now.duration_ms)),
            volume: f64::from(volume) / f64::from(u16::MAX),
            shuffle: core.shuffle_wanted,
            repeat: Repeat::from_engine(now.as_ref().map_or(core.local.repeat, |now| now.repeat)),
            current_index: if queue.first().is_some_and(|row| row.section == QueueSection::Current) { 0 } else { -1 },
            error_text: core.error_text.clone(),
            direction: core.direction,
            level: self.level,
            levels_wanted: core.levels_wanted,
            context: core.shown_context(),
            played_seconds: core.played_seconds(),
            device: live.and_then(|now| now.device.clone()),
            target,
            is_local,
            local: core.local_playback.clone(),
            can_control: live.is_none_or(|now| now.can_control),
            can_set_volume: live.is_none_or(|now| now.can_set_volume),
            transferring: core.transferring(),
            device_lost: core.device_lost,
            context_remaining,
            manual_count,
            can_clear_queue: core.can_clear_queue(),
            queue_reorderable: core.queue_locally_reorderable(),
            queue,
        }
    }

    /// Снимок — в watch, затем события (снимок обновляется до рассылки).
    pub fn publish(&mut self) {
        let mut next = self.build();
        // кадры «дыхания» идут ~30 Гц; позиция в QML — не чаще ~5 раз в секунду
        let old = &self.published;
        if old.track == next.track
            && old.playing
            && next.playing
            && old.duration_ms == next.duration_ms
            && (0..POSITION_STEP_MS).contains(&(next.position_ms - old.position_ms))
        {
            next.position_ms = old.position_ms;
        }
        let old = std::mem::replace(&mut self.published, next.clone());
        let mut events = Vec::new();
        if old.track != next.track || old.codec != next.codec {
            events.push(PlayerEvent::TrackChanged);
        }
        if old.playing != next.playing || old.buffering != next.buffering || old.narrating != next.narrating {
            events.push(PlayerEvent::StateChanged);
        }
        if old.position_ms != next.position_ms || old.duration_ms != next.duration_ms {
            events.push(PlayerEvent::PositionChanged);
        }
        if old.volume != next.volume {
            events.push(PlayerEvent::VolumeChanged);
        }
        if old.shuffle != next.shuffle || old.repeat != next.repeat {
            events.push(PlayerEvent::ModeChanged);
        }
        let queue_changed = !Arc::ptr_eq(&old.queue, &next.queue) && old.queue != next.queue;
        if queue_changed {
            for change in diff::queue_changes(&old.queue, &next.queue) {
                events.push(PlayerEvent::Queue(change));
            }
        }
        if queue_changed
            || old.current_index != next.current_index
            || old.context != next.context
            || old.context_remaining != next.context_remaining
            || old.manual_count != next.manual_count
            || old.can_clear_queue != next.can_clear_queue
            || old.queue_reorderable != next.queue_reorderable
        {
            events.push(PlayerEvent::QueueChanged);
        }
        if old.cover_file != next.cover_file {
            events.push(PlayerEvent::CoverFileChanged);
        }
        if old.error_text != next.error_text {
            events.push(PlayerEvent::ErrorChanged);
        }
        if old.level != next.level || old.levels_wanted != next.levels_wanted {
            events.push(PlayerEvent::LevelChanged);
        }
        if old.target != next.target
            || old.device != next.device
            || old.is_local != next.is_local
            || old.can_control != next.can_control
            || old.can_set_volume != next.can_set_volume
            || old.transferring != next.transferring
            || old.device_lost != next.device_lost
            || old.local != next.local
        {
            events.push(PlayerEvent::DeviceChanged);
        }
        if old.local != next.local {
            events.push(PlayerEvent::LocalChanged);
        }
        let changed = !events.is_empty() || old.played_seconds != next.played_seconds;
        if changed {
            self.snapshot.send_replace(next);
        }
        for event in events.into_iter().chain(self.pending.drain(..)) {
            let _ = self.events.send(event);
        }
    }

    fn shutdown(&mut self) {
        self.core.save_session();
        for effect in self.core.take_effects() {
            if let (Effect::Save(data), Some(path)) = (effect, self.session_file.as_ref())
                && let Err(error) = self.with_lane_session(*data).save(path)
            {
                tracing::warn!("player session not saved: {error:#}");
            }
        }
        self.youtube.stop();
        self.run_youtube_effects();
        self.yt = None;
        self.engine = None;
    }
}

/// The YouTube lane: commands for YouTube tracks go to it, transport goes to it while it is active, and any
/// Spotify play stops it.
impl Actor {
    fn youtube_message(&mut self, message: YtMsg) {
        match message {
            YtMsg::Enable(link) => {
                link.engine.send(YtCommand::Volume(self.core.volume));
                link.engine.set_levels_wanted(self.core.levels_wanted);
                self.yt = Some(link);
            }
            YtMsg::Resolved { seq, result } => self.youtube.resolved(seq, result),
            YtMsg::Event { seq, event } => self.youtube.event(seq, event),
        }
    }

    /// While the lane has a song, the saved session is the lane's (`Core` knows nothing of YouTube songs).
    pub(crate) fn with_lane_session(&self, data: SessionData) -> SessionData {
        if !self.youtube.is_active() {
            return data;
        }
        let Some((track, position_ms, rows, context)) = self.youtube.session() else {
            // the lane owns the player but has no song (cleared): nothing to restore next time
            return SessionData {
                context_uri: None,
                context: PlayContext::default(),
                track: None,
                position_ms: 0,
                manual: Vec::new(),
                rows: Vec::new(),
                ..data
            };
        };
        SessionData {
            version: SessionData::VERSION,
            context_uri: context.context_uri.clone(),
            context,
            track: Some(track),
            position_ms: u32::try_from(position_ms).unwrap_or(u32::MAX),
            manual: Vec::new(),
            rows,
            shuffle: data.shuffle,
        }
    }

    /// YouTube starts playing: Spotify pauses here or on the remote device.
    fn youtube_takes_over(&mut self) -> bool {
        if self.yt.is_none() {
            self.pending.push(PlayerEvent::Notice(mf_i18n::t("YouTube playback isn't available")));
            return false;
        }
        self.core.pause();
        self.yt_spotify_sequence = self.core.local.track_sequence;
        true
    }

    /// Spotify and YouTube songs never share a queue.
    fn mixed_queue_notice(&mut self) {
        self.pending.push(PlayerEvent::Notice(mf_i18n::t("Spotify and YouTube songs can't share a queue")));
    }

    /// Handles what belongs to the lane; returns the command for `Core` otherwise.
    fn youtube_command(&mut self, command: Command) -> Option<Command> {
        let active = self.youtube.is_active();
        // Spotify off (the app): the lane takes all transport and the queue even while it has nothing loaded
        let lane = active || !self.core.spotify;
        match command {
            Command::PlayItems { items, row, context } if items.get(row).is_some_and(|item| is_lane_track(&item.uri)) => {
                if self.youtube_takes_over() {
                    self.youtube.play_items(items, row, context);
                }
                None
            }
            Command::ShuffleFrom { items, context }
                if items.iter().find(|item| item.available).is_some_and(|item| is_lane_track(&item.uri)) =>
            {
                if self.youtube_takes_over() {
                    self.youtube.shuffle_from(items, context);
                }
                None
            }
            Command::PlayUris { uris, index, context } if uris.get(index).is_some_and(|uri| is_lane_track(uri)) => {
                if self.youtube_takes_over() {
                    self.youtube.play_uris(uris, index, context);
                }
                None
            }
            Command::PlayContext { context, .. }
                if context.context_uri.as_deref().is_some_and(|uri| uri.starts_with("youtube:")) =>
            {
                // YouTube contexts are expanded into tracks by the services (Phase 4)
                tracing::debug!("youtube context {:?} is not played by uri", context.context_uri);
                None
            }
            Command::Remember(item) if is_lane_track(&item.uri) => None,
            Command::ExtendContext { context_uri, items } => {
                self.youtube.extend_context(&context_uri, items);
                None
            }
            Command::Autoplay { seed_uri, items } => {
                let added = self.youtube.autoplay(&seed_uri, items);
                // nothing new (all already listed, or an empty page) while that song still plays: ask again
                let (song, empty) = &mut self.autoplay_empty;
                if added == 0 && *song == seed_uri && *empty < AUTOPLAY_RETRIES && self.autoplay_asked.as_deref() == Some(&seed_uri)
                {
                    *empty += 1;
                    self.autoplay_asked = None;
                }
                None
            }
            command @ (Command::PlayItems { .. }
            | Command::ShuffleFrom { .. }
            | Command::PlayContext { .. }
            | Command::PlayUris { .. }
            | Command::PlayEpisode { .. }
            | Command::TransferHere(_)
            | Command::TransferTo(..)) => {
                self.youtube.stop();
                Some(command)
            }
            Command::AddToQueue(items) => {
                let (youtube, spotify): (Vec<TrackItem>, Vec<TrackItem>) =
                    items.into_iter().partition(|item| is_lane_track(&item.uri));
                let (ours, theirs) = if lane { (youtube, spotify) } else { (spotify, youtube) };
                if !theirs.is_empty() {
                    self.mixed_queue_notice();
                }
                if lane {
                    self.youtube.add(ours);
                    None
                } else {
                    (!ours.is_empty()).then_some(Command::AddToQueue(ours))
                }
            }
            Command::AddUriToQueue(uri) if lane || is_lane_track(&uri) => {
                if lane && is_lane_track(&uri) {
                    let id = crate::engine::youtube::lane_track_id(&uri).unwrap_or_default().to_owned();
                    self.youtube.add(vec![TrackItem { track_id: id, uri, available: true, ..Default::default() }]);
                } else {
                    self.mixed_queue_notice();
                }
                None
            }
            Command::InsertManual { item, at } if lane || is_lane_track(&item.uri) => {
                if lane && is_lane_track(&item.uri) {
                    self.youtube.insert_manual(*item, at);
                } else {
                    self.mixed_queue_notice();
                }
                None
            }
            Command::SetVolume(value) => {
                if let Some(yt) = &self.yt {
                    yt.engine.send(YtCommand::Volume(crate::volume_to_u16(value)));
                }
                Some(Command::SetVolume(value))
            }
            Command::SetLevelsWanted(value) => {
                if let Some(yt) = &self.yt {
                    yt.engine.set_levels_wanted(value);
                }
                Some(Command::SetLevelsWanted(value))
            }
            command if !lane => Some(command),
            Command::TogglePlay | Command::Play if self.youtube.track().is_none() => {
                self.pending.push(PlayerEvent::Notice(mf_i18n::t("Nothing is playing. Choose something to play first")));
                None
            }
            Command::PlayIndex(index) => {
                self.youtube.play_index(index);
                None
            }
            Command::PlayQueueRow(uid) => {
                self.youtube.play_row(uid);
                None
            }
            Command::MoveManual { from, to } => {
                self.youtube.move_manual(from, to);
                None
            }
            Command::MoveContext { from, to } => {
                self.youtube.move_context(from, to);
                None
            }
            Command::ClearManualQueue => {
                self.youtube.clear_manual();
                None
            }
            Command::Clear => {
                self.youtube.clear();
                // the session file forgets the song now, not at the next save
                self.core.save_session();
                None
            }
            Command::RefreshQueue => None,
            Command::TogglePlay => {
                self.youtube.toggle();
                None
            }
            Command::Play => {
                self.youtube.play();
                None
            }
            Command::Pause | Command::Stop => {
                self.youtube.pause();
                None
            }
            Command::Next => {
                self.youtube.next();
                None
            }
            Command::Previous => {
                self.youtube.previous();
                None
            }
            Command::Seek(fraction) => {
                let position = (fraction.clamp(0.0, 1.0) * self.youtube.duration_ms() as f64).round() as u64;
                self.youtube.seek(position);
                None
            }
            Command::SeekMs(ms) => {
                self.youtube.seek(ms.max(0) as u64);
                None
            }
            Command::SeekRelative(delta) => {
                self.youtube.seek_relative(delta);
                None
            }
            Command::CycleRepeat => {
                let repeat = self.youtube.repeat().cycled();
                self.youtube.set_repeat(repeat);
                None
            }
            Command::SetRepeat(repeat) => {
                self.youtube.set_repeat(repeat);
                None
            }
            Command::SetShuffle(value) => {
                self.youtube.set_shuffle(value);
                None
            }
            command => Some(command),
        }
    }

    fn run_youtube_effects(&mut self) {
        for effect in self.youtube.take_effects() {
            match effect {
                YtEffect::Resolve { seq, request } => match &self.yt {
                    Some(yt) => {
                        let source = yt.source.clone();
                        let tx = self.tx.clone();
                        tokio::spawn(async move {
                            let result = source.resolve_track(request).await;
                            let _ = tx.send(Msg::YouTube(YtMsg::Resolved { seq, result: Box::new(result) }));
                        });
                    }
                    None => tracing::debug!("youtube is off, {} not resolved", request.video_id),
                },
                YtEffect::Prefetch(request) => {
                    if let Some(yt) = &self.yt {
                        tokio::spawn(yt.source.prefetch_track(request));
                    }
                }
                YtEffect::Invalidate(video_id) => {
                    if let Some(yt) = &self.yt {
                        yt.source.invalidate(&video_id);
                    }
                }
                YtEffect::Engine(command) => match &self.yt {
                    Some(yt) => yt.engine.send(command),
                    None => tracing::debug!("youtube is off, command {command:?} skipped"),
                },
                YtEffect::Cover(url) => {
                    if self.youtube.is_active() {
                        self.cover(url);
                    }
                }
                YtEffect::Started(started) => {
                    self.pending.push(PlayerEvent::TrackStarted((*started).into()));
                    // a new song: the session file follows it
                    self.core.save_session();
                }
                YtEffect::Ended(ended) => self.pending.push(PlayerEvent::TrackEnded((*ended).into())),
            }
        }
        // the list is about to run out: ask the services once per song for its radio
        let seed = if self.core.autoplay { self.youtube.autoplay_seed() } else { None };
        match seed {
            Some(track) if self.autoplay_asked.as_deref() != Some(track.uri.as_str()) => {
                self.autoplay_asked = Some(track.uri.clone());
                if self.autoplay_empty.0 != track.uri {
                    self.autoplay_empty = (track.uri.clone(), 0);
                }
                self.pending.push(PlayerEvent::AutoplayWanted(Box::new(track.clone())));
            }
            Some(_) => {}
            None => self.autoplay_asked = None,
        }
        // the lane stopped: the Spotify track's cover again
        let active = self.youtube.is_active();
        if self.yt_was_active && !active {
            self.cover(self.core_cover.clone());
        }
        self.yt_was_active = active;
    }

    /// The snapshot while the lane plays: everything from the lane, the device is this computer.
    fn build_youtube(&mut self) -> PlayerSnapshot {
        let lane = &self.youtube;
        let core = &self.core;
        let rows = lane.rows();
        let queue = if rows.as_slice() == self.published.queue.as_slice() {
            self.published.queue.clone()
        } else {
            Arc::new(rows)
        };
        let context_remaining = queue.iter().filter(|row| row.section == QueueSection::Context).count() as i32;
        let manual_count = lane.manual_count() as i32;
        let volume = crate::volume_from_u16(core.volume);
        let name = if core.local_device_name.is_empty() {
            mf_i18n::t("This computer")
        } else {
            core.local_device_name.clone()
        };
        let device = DeviceItem {
            id: core.local_device_id.clone().unwrap_or_default(),
            name,
            kind: "computer".into(),
            is_active: true,
            is_this_computer: true,
            is_restricted: false,
            supports_volume: true,
            volume: (volume * 100.0).round() as i32,
            source: "youtube".into(),
        };
        PlayerSnapshot {
            track: lane.track().cloned(),
            codec: lane.stream().map(|stream| stream.quality_label()).unwrap_or_default(),
            cover_file: self.cover_file.clone(),
            playing: lane.playing(),
            buffering: lane.loading(),
            narrating: false,
            position_ms: lane.position_ms() as i64,
            duration_ms: lane.duration_ms() as i64,
            volume,
            shuffle: lane.shuffle(),
            repeat: lane.repeat(),
            current_index: if queue.first().is_some_and(|row| row.section == QueueSection::Current) { 0 } else { -1 },
            error_text: lane.error().to_owned(),
            direction: lane.direction(),
            level: self.level,
            levels_wanted: core.levels_wanted,
            context: lane.context().clone(),
            played_seconds: lane.played_seconds(),
            device: Some(device),
            target: Target::Local,
            is_local: true,
            local: core.local_playback.clone(),
            can_control: true,
            can_set_volume: true,
            transferring: false,
            device_lost: false,
            context_remaining,
            manual_count,
            can_clear_queue: manual_count > 0,
            queue_reorderable: true,
            queue,
        }
    }
}

/// Задача, выполняющая вызовы строго по одному в порядке отправки.
fn ordered(backend: Arc<dyn WebBackend>, tx: mpsc::UnboundedSender<Msg>) -> mpsc::UnboundedSender<ApiCall> {
    let (sender, mut calls) = mpsc::unbounded_channel::<ApiCall>();
    tokio::spawn(async move {
        while let Some(call) = calls.recv().await {
            let reply = backend.call(call).await;
            if tx.send(Msg::Api(reply)).is_err() {
                break;
            }
        }
    });
    sender
}

/// Порядок строк (для тестов).
#[cfg(test)]
pub(crate) fn uris(rows: &[QueueRow]) -> Vec<(QueueSection, String)> {
    rows.iter().map(|row| (row.section, row.track.uri.clone())).collect()
}

/// Применить изменения очереди, как это делает мост (для тестов).
#[cfg(test)]
pub(crate) fn apply_changes(rows: &mut Vec<QueueRow>, changes: &[QueueChange]) {
    diff::apply(rows, changes);
}
