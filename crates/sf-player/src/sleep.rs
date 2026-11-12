//! Sleep timer, ported from soundwave1 (desktop `PlayerController.setSleepTimer` / `sleepAfterCurrentTrack`,
//! Android `SleepTimerStateHolder`).
//!
//! Two modes: pause after a number of minutes, or pause when the track that is playing now ends. Before it pauses
//! the timer fades the volume out over [`FADE_MS`], then pauses and puts the volume back, so the next play starts
//! at the level the user set. Pausing goes through [`PlayerHandle::pause`], which stops whichever lane is playing:
//! the local librespot engine, a Connect device, or the YouTube lane.
//!
//! [`SleepTimer`] is a pure state machine driven by a millisecond clock, so it is tested on a fake clock.
//! [`SleepTimerHandle`] runs it on the tokio runtime against a [`SleepTarget`] (the player handle in the app).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::{PlayerEvent, PlayerHandle};

/// How long the volume takes to fall to silence before the pause.
pub const FADE_MS: u64 = 10_000;
/// How often the runner looks at the clock.
pub const TICK_MS: u64 = 250;
/// The durations offered in the menu, in minutes (soundwave1: 15, 30, 45, 60; the Android app adds the long ones).
pub const PRESET_MINUTES: [u32; 6] = [5, 15, 30, 45, 60, 90];

/// What the timer is waiting for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SleepMode {
    #[default]
    Off,
    /// Pause when the clock reaches this millisecond.
    At { deadline_ms: u64 },
    /// Pause when this track ends (its URI; empty when nothing was playing when it was set).
    EndOfTrack { uri: String },
}

/// One step the timer asks the player to take.
#[derive(Clone, Debug, PartialEq)]
pub enum SleepAction {
    /// Set the volume (0..1) while fading.
    Volume(f64),
    /// Pause, then put the volume back to this level.
    Pause { restore_volume: f64 },
}

/// What the player looks like at a tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerView {
    pub playing: bool,
    /// 0..1, as the user set it (the timer remembers it before fading).
    pub volume: f64,
    pub uri: String,
    pub position_ms: i64,
    pub duration_ms: i64,
    /// Whether this computer plays the sound; a Connect device is paused without fading its volume.
    pub fade: bool,
}

#[derive(Debug, Default)]
pub struct SleepTimer {
    mode: SleepMode,
    /// The volume before the fade began; Some while fading.
    fading_from: Option<f64>,
    /// The last volume the fade set, to tell the user's own volume change from ours.
    last_set: Option<f64>,
}

impl SleepTimer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mode(&self) -> &SleepMode {
        &self.mode
    }

    pub fn is_active(&self) -> bool {
        self.mode != SleepMode::Off
    }

    /// Pause `minutes` from `now_ms`. Replaces any timer already set.
    pub fn start_minutes(&mut self, now_ms: u64, minutes: u32) -> Option<SleepAction> {
        let restore = self.cancel();
        self.mode = SleepMode::At { deadline_ms: now_ms + u64::from(minutes) * 60_000 };
        restore
    }

    /// Pause when `uri` ends.
    pub fn start_end_of_track(&mut self, uri: &str) -> Option<SleepAction> {
        let restore = self.cancel();
        self.mode = SleepMode::EndOfTrack { uri: uri.to_owned() };
        restore
    }

    /// Turn the timer off. If it was fading, the returned action puts the volume back.
    pub fn cancel(&mut self) -> Option<SleepAction> {
        self.mode = SleepMode::Off;
        self.last_set = None;
        self.fading_from.take().map(SleepAction::Volume)
    }

    /// Milliseconds until the pause in the timed mode, None otherwise.
    pub fn remaining_ms(&self, now_ms: u64) -> Option<u64> {
        match self.mode {
            SleepMode::At { deadline_ms } => Some(deadline_ms.saturating_sub(now_ms)),
            _ => None,
        }
    }

    /// The deadline in the timed mode.
    pub fn deadline_ms(&self) -> Option<u64> {
        match self.mode {
            SleepMode::At { deadline_ms } => Some(deadline_ms),
            _ => None,
        }
    }

    /// The track ended (on its own or skipped) or another one started.
    pub fn track_ended(&mut self, ended_uri: &str, view: &PlayerView) -> Option<SleepAction> {
        match &self.mode {
            SleepMode::EndOfTrack { uri } if uri.is_empty() || uri == ended_uri => Some(self.fire(view)),
            _ => None,
        }
    }

    /// Look at the clock. Call every [`TICK_MS`] or so.
    pub fn tick(&mut self, now_ms: u64, view: &PlayerView) -> Option<SleepAction> {
        // A track change the event missed still ends an end-of-track timer.
        if let SleepMode::EndOfTrack { uri } = &self.mode {
            if uri.is_empty() && !view.uri.is_empty() {
                self.mode = SleepMode::EndOfTrack { uri: view.uri.clone() };
            } else if !uri.is_empty() && !view.uri.is_empty() && *uri != view.uri {
                return Some(self.fire(view));
            }
        }
        let left = match &self.mode {
            SleepMode::Off => return None,
            SleepMode::At { deadline_ms } => deadline_ms.saturating_sub(now_ms),
            SleepMode::EndOfTrack { .. } => {
                if view.duration_ms <= 0 {
                    return None;
                }
                (view.duration_ms - view.position_ms).max(0) as u64
            }
        };
        if left == 0 {
            return match self.mode {
                SleepMode::At { .. } => Some(self.fire(view)),
                // The end-of-track timer pauses on the track's end event, not on the last tick of it.
                _ => self.fade_step(0, view),
            };
        }
        if left > FADE_MS {
            return self.unfade();
        }
        self.fade_step(left, view)
    }

    fn fade_step(&mut self, left: u64, view: &PlayerView) -> Option<SleepAction> {
        if !view.fade || !view.playing {
            return None;
        }
        // The user moved the volume during the fade: start the fade again from their level.
        if let (Some(_), Some(set)) = (self.fading_from, self.last_set) {
            if (view.volume - set).abs() > 0.02 {
                self.fading_from = Some(view.volume);
            }
        }
        let from = *self.fading_from.get_or_insert(view.volume);
        let level = (from * left as f64 / FADE_MS as f64).clamp(0.0, from);
        if self.last_set.is_some_and(|set| (set - level).abs() < 0.005) {
            return None;
        }
        self.last_set = Some(level);
        Some(SleepAction::Volume(level))
    }

    /// The deadline moved away (a seek back, a longer track): undo the fade.
    fn unfade(&mut self) -> Option<SleepAction> {
        self.last_set = None;
        self.fading_from.take().map(SleepAction::Volume)
    }

    fn fire(&mut self, view: &PlayerView) -> SleepAction {
        let restore_volume = self.fading_from.take().unwrap_or(view.volume);
        self.mode = SleepMode::Off;
        self.last_set = None;
        SleepAction::Pause { restore_volume }
    }
}

/// Milliseconds since some fixed point; a fake one in tests.
pub trait Clock: Send + Sync + 'static {
    fn now_ms(&self) -> u64;
}

/// The monotonic clock.
pub struct SystemClock(Instant);

impl Default for SystemClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        self.0.elapsed().as_millis() as u64
    }
}

/// What the timer pauses.
pub trait SleepTarget: Send + Sync + 'static {
    fn view(&self) -> PlayerView;
    fn set_volume(&self, value: f64);
    fn pause(&self);
}

impl SleepTarget for PlayerHandle {
    fn view(&self) -> PlayerView {
        let snapshot = self.snapshot();
        PlayerView {
            playing: snapshot.playing,
            volume: snapshot.volume,
            uri: snapshot.track.as_ref().map(|t| t.uri.clone()).unwrap_or_default(),
            position_ms: snapshot.position_ms,
            duration_ms: snapshot.duration_ms,
            fade: snapshot.is_local || snapshot.track.as_ref().is_some_and(|t| t.uri.starts_with("youtube:")),
        }
    }

    fn set_volume(&self, value: f64) {
        PlayerHandle::set_volume(self, value);
    }

    fn pause(&self) {
        PlayerHandle::pause(self);
    }
}

/// What the UI shows about the timer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SleepStatus {
    pub active: bool,
    pub end_of_track: bool,
    /// Milliseconds left in the timed mode, 0 otherwise.
    pub remaining_ms: u64,
    /// The timer fired since the last time it was set (for a "playback paused" notice).
    pub fired: bool,
}

struct Shared {
    timer: SleepTimer,
    clock: Arc<dyn Clock>,
    target: Arc<dyn SleepTarget>,
    status: watch::Sender<SleepStatus>,
}

impl Shared {
    fn apply(&mut self, action: Option<SleepAction>) {
        match action {
            None => {}
            Some(SleepAction::Volume(value)) => self.target.set_volume(value),
            Some(SleepAction::Pause { restore_volume }) => {
                self.target.pause();
                self.target.set_volume(restore_volume);
                tracing::info!("sleep timer: playback paused");
            }
        }
    }

    fn publish(&self, fired: bool) {
        let now = self.clock.now_ms();
        let status = SleepStatus {
            active: self.timer.is_active(),
            end_of_track: matches!(self.timer.mode(), SleepMode::EndOfTrack { .. }),
            remaining_ms: self.timer.remaining_ms(now).unwrap_or(0),
            fired,
        };
        self.status.send_if_modified(|old| {
            let changed = *old != status;
            *old = status;
            changed
        });
    }

    fn tick(&mut self) {
        let was_active = self.timer.is_active();
        let view = self.target.view();
        let action = self.timer.tick(self.clock.now_ms(), &view);
        self.apply(action);
        self.publish(was_active && !self.timer.is_active());
    }

    fn track_ended(&mut self, uri: &str) {
        let was_active = self.timer.is_active();
        let view = self.target.view();
        let action = self.timer.track_ended(uri, &view);
        self.apply(action);
        self.publish(was_active && !self.timer.is_active());
    }
}

/// The running timer. Cheap to clone; dropping every clone stops the task.
#[derive(Clone)]
pub struct SleepTimerHandle {
    shared: Arc<Mutex<Shared>>,
    status: watch::Receiver<SleepStatus>,
}

impl SleepTimerHandle {
    /// Run against the player on its runtime, waking on track ends and every [`TICK_MS`].
    pub fn spawn(rt: &tokio::runtime::Handle, player: PlayerHandle) -> Self {
        let mut events = player.subscribe().events;
        let this = Self::new(Arc::new(player), Arc::new(SystemClock::default()));
        let weak = Arc::downgrade(&this.shared);
        rt.spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(TICK_MS));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                let ended = tokio::select! {
                    _ = tick.tick() => None,
                    event = events.recv() => match event {
                        Ok(PlayerEvent::TrackEnded(ended)) => Some(ended.track.uri),
                        Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                };
                let Some(shared) = weak.upgrade() else { break };
                let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
                match ended {
                    Some(uri) => shared.track_ended(&uri),
                    None if shared.timer.is_active() => shared.tick(),
                    None => {}
                }
            }
        });
        this
    }

    /// A timer driven by hand ([`Self::tick`], [`Self::track_ended`]); the app uses [`Self::spawn`].
    pub fn new(target: Arc<dyn SleepTarget>, clock: Arc<dyn Clock>) -> Self {
        let (tx, status) = watch::channel(SleepStatus::default());
        let shared = Shared { timer: SleepTimer::new(), clock, target, status: tx };
        Self { shared: Arc::new(Mutex::new(shared)), status }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Shared) -> R) -> R {
        f(&mut self.shared.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Pause in `minutes`; 0 turns the timer off.
    pub fn set_minutes(&self, minutes: u32) {
        self.with(|shared| {
            let action = if minutes == 0 {
                shared.timer.cancel()
            } else {
                let now = shared.clock.now_ms();
                shared.timer.start_minutes(now, minutes)
            };
            shared.apply(action);
            shared.publish(false);
        });
    }

    /// Pause when the track playing now ends.
    pub fn set_end_of_track(&self) {
        self.with(|shared| {
            let uri = shared.target.view().uri;
            let action = shared.timer.start_end_of_track(&uri);
            shared.apply(action);
            shared.publish(false);
        });
    }

    pub fn cancel(&self) {
        self.set_minutes(0);
    }

    pub fn status(&self) -> SleepStatus {
        self.status.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<SleepStatus> {
        self.status.clone()
    }

    pub fn tick(&self) {
        self.with(Shared::tick);
    }

    pub fn track_ended(&self, uri: &str) {
        self.with(|shared| shared.track_ended(uri));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn playing(volume: f64) -> PlayerView {
        PlayerView {
            playing: true,
            volume,
            uri: "spotify:track:a".into(),
            position_ms: 0,
            duration_ms: 200_000,
            fade: true,
        }
    }

    #[test]
    fn timed_mode_waits_then_fades_then_pauses_and_restores() {
        let mut timer = SleepTimer::new();
        let view = playing(0.8);
        assert_eq!(timer.start_minutes(1_000, 1), None);
        assert_eq!(timer.remaining_ms(31_000), Some(30_000));
        assert_eq!(timer.tick(31_000, &view), None, "nothing before the fade window");
        // halfway through the fade
        let Some(SleepAction::Volume(half)) = timer.tick(61_000 - FADE_MS / 2, &view) else { panic!() };
        assert!((half - 0.4).abs() < 1e-9);
        // the player now reports the faded volume; the fade keeps counting from 0.8
        let faded = PlayerView { volume: half, ..view.clone() };
        let Some(SleepAction::Volume(low)) = timer.tick(61_000 - FADE_MS / 10, &faded) else { panic!() };
        assert!((low - 0.08).abs() < 1e-9);
        let low_view = PlayerView { volume: low, ..view };
        assert_eq!(timer.tick(61_000, &low_view), Some(SleepAction::Pause { restore_volume: 0.8 }));
        assert!(!timer.is_active());
        assert_eq!(timer.tick(70_000, &low_view), None);
    }

    #[test]
    fn cancelling_mid_fade_puts_the_volume_back() {
        let mut timer = SleepTimer::new();
        timer.start_minutes(0, 1);
        assert!(matches!(timer.tick(55_000, &playing(0.5)), Some(SleepAction::Volume(_))));
        assert_eq!(timer.cancel(), Some(SleepAction::Volume(0.5)));
        assert_eq!(timer.cancel(), None);
    }

    #[test]
    fn a_user_volume_change_during_the_fade_becomes_the_new_level() {
        let mut timer = SleepTimer::new();
        timer.start_minutes(0, 1);
        let Some(SleepAction::Volume(v)) = timer.tick(55_000, &playing(1.0)) else { panic!() };
        assert!((v - 0.5).abs() < 1e-9);
        // user drags to 0.3
        let Some(SleepAction::Volume(v)) = timer.tick(57_500, &playing(0.3)) else { panic!() };
        assert!((v - 0.075).abs() < 1e-9);
        assert_eq!(timer.tick(60_000, &playing(0.075)), Some(SleepAction::Pause { restore_volume: 0.3 }));
    }

    #[test]
    fn a_connect_device_is_paused_without_a_fade() {
        let mut timer = SleepTimer::new();
        timer.start_minutes(0, 1);
        let remote = PlayerView { fade: false, ..playing(0.7) };
        assert_eq!(timer.tick(55_000, &remote), None);
        assert_eq!(timer.tick(60_000, &remote), Some(SleepAction::Pause { restore_volume: 0.7 }));
    }

    #[test]
    fn end_of_track_fades_near_the_end_and_pauses_on_the_end_event() {
        let mut timer = SleepTimer::new();
        timer.start_end_of_track("spotify:track:a");
        let mut view = playing(0.6);
        view.position_ms = 100_000;
        assert_eq!(timer.tick(0, &view), None);
        view.position_ms = 195_000;
        let Some(SleepAction::Volume(v)) = timer.tick(0, &view) else { panic!() };
        assert!((v - 0.3).abs() < 1e-9);
        assert_eq!(timer.track_ended("spotify:track:other", &view), None, "another track's end is not ours");
        assert_eq!(timer.track_ended("spotify:track:a", &view), Some(SleepAction::Pause { restore_volume: 0.6 }));
        assert!(!timer.is_active());
    }

    #[test]
    fn end_of_track_fires_when_the_next_track_shows_up_without_an_event() {
        let mut timer = SleepTimer::new();
        timer.start_end_of_track("youtube:track:abc");
        let mut view = playing(0.5);
        view.uri = "youtube:track:def".into();
        assert_eq!(timer.tick(0, &view), Some(SleepAction::Pause { restore_volume: 0.5 }));
    }

    #[test]
    fn end_of_track_set_while_idle_adopts_the_next_track() {
        let mut timer = SleepTimer::new();
        timer.start_end_of_track("");
        let view = playing(0.5);
        assert_eq!(timer.tick(0, &view), None);
        assert_eq!(timer.mode(), &SleepMode::EndOfTrack { uri: "spotify:track:a".into() });
    }

    #[test]
    fn seeking_back_out_of_the_fade_window_restores_the_volume() {
        let mut timer = SleepTimer::new();
        timer.start_end_of_track("spotify:track:a");
        let mut view = playing(0.9);
        view.position_ms = 195_000;
        assert!(matches!(timer.tick(0, &view), Some(SleepAction::Volume(_))));
        view.position_ms = 10_000;
        assert_eq!(timer.tick(0, &view), Some(SleepAction::Volume(0.9)));
        assert!(timer.is_active());
    }

    struct FakeClock(AtomicU64);
    impl Clock for FakeClock {
        fn now_ms(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    #[derive(Default)]
    struct FakePlayer {
        view: Mutex<PlayerView>,
        log: Mutex<Vec<String>>,
    }
    impl SleepTarget for FakePlayer {
        fn view(&self) -> PlayerView {
            self.view.lock().unwrap().clone()
        }
        fn set_volume(&self, value: f64) {
            self.view.lock().unwrap().volume = value;
            self.log.lock().unwrap().push(format!("volume {value:.2}"));
        }
        fn pause(&self) {
            self.view.lock().unwrap().playing = false;
            self.log.lock().unwrap().push("pause".into());
        }
    }

    #[test]
    fn handle_drives_a_target_on_a_fake_clock() {
        let clock = Arc::new(FakeClock(AtomicU64::new(0)));
        let player = Arc::new(FakePlayer::default());
        *player.view.lock().unwrap() = playing(1.0);
        let timer = SleepTimerHandle::new(player.clone(), clock.clone());
        timer.set_minutes(15);
        assert_eq!(timer.status(), SleepStatus { active: true, remaining_ms: 900_000, ..Default::default() });
        let mut ms = 0;
        while ms < 900_000 {
            ms += TICK_MS;
            clock.0.store(ms, Ordering::SeqCst);
            timer.tick();
        }
        let log = player.log.lock().unwrap().clone();
        assert_eq!(log.iter().filter(|l| *l == "pause").count(), 1);
        assert_eq!(log.last().map(String::as_str), Some("volume 1.00"), "volume restored after the pause");
        assert!(log.len() > 10, "the fade took many steps: {log:?}");
        assert!(!player.view().playing);
        assert_eq!(timer.status(), SleepStatus { fired: true, ..Default::default() });
    }

    #[test]
    fn handle_end_of_track_pauses_on_the_track_end() {
        let clock = Arc::new(FakeClock(AtomicU64::new(0)));
        let player = Arc::new(FakePlayer::default());
        *player.view.lock().unwrap() = playing(0.4);
        let timer = SleepTimerHandle::new(player.clone(), clock);
        timer.set_end_of_track();
        assert!(timer.status().end_of_track);
        timer.track_ended("spotify:track:a");
        assert_eq!(*player.log.lock().unwrap(), vec!["pause".to_owned(), "volume 0.40".to_owned()]);
        timer.set_minutes(30);
        timer.cancel();
        assert!(!timer.status().active);
    }
}
