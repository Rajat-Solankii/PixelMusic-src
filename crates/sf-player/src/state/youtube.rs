//! The YouTube lane of the player actor: its own queue (context list, "Playing next", shuffle order, repeat) and
//! what the YouTube player reports. `Core` (librespot + Connect) never sees YouTube tracks; while the lane is
//! active the actor builds the snapshot from it.
//!
//! Methods are pure state changes that queue [`YtEffect`]s; the actor runs them (resolve a stream, send a
//! command to the player, download a cover, history events). Every load has a sequence number: resolve results
//! and player events of an older load are dropped.
use std::sync::Arc;
use std::time::{Duration, Instant};

use md3_model::{PlayContext, QueueRow, QueueSection, TrackItem};

use super::core::{TrackEndedLite, TrackStartedLite};
use crate::engine::youtube::{StreamRequest, StreamSource, YtCommand, YtEvent, YtPlayer, YtStream, is_lane_track, lane_id, lane_track_id};
use crate::{RESTART_BEFORE_PREVIOUS_MS, Repeat};

/// Queue row ids of the lane start high so they never meet the ids `Core` hands out.
const FIRST_UID: u64 = 1 << 48;

/// What the actor does for the lane.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum YtEffect {
    /// resolve the track's stream for load `seq`
    Resolve { seq: u64, request: StreamRequest },
    /// warm the next track's stream
    Prefetch(StreamRequest),
    /// the stream URL stopped working
    Invalidate(String),
    Engine(YtCommand),
    /// download the cover ("" — none)
    Cover(String),
    Started(Box<TrackStartedLite>),
    Ended(Box<TrackEndedLite>),
}

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    uid: u64,
    track: TrackItem,
}

/// The current track and where it came from.
#[derive(Clone, Debug, PartialEq)]
struct Current {
    entry: Entry,
    /// from "Playing next" (the context position did not move)
    manual: bool,
}

#[derive(Debug)]
pub(crate) struct YtLane {
    active: bool,
    /// the context list as given
    items: Vec<Entry>,
    /// play order: indices into `items` (shuffled or not)
    order: Vec<usize>,
    /// position in `order` of the last context track played (or playing); the context goes on after it
    pos: usize,
    manual: Vec<Entry>,
    current: Option<Current>,
    context: PlayContext,
    shuffle: bool,
    repeat: Repeat,
    next_uid: u64,
    /// +1 forward, -1 back (track change animation)
    direction: i32,

    // --- the load of the current track ---
    seq: u64,
    stream: Option<YtStream>,
    /// the user wants it playing
    playing: bool,
    loading: bool,
    position_ms: u64,
    /// while playing: when `position_ms` was reported
    position_at: Option<Instant>,
    duration_ms: u64,
    /// where to start once the stream is resolved
    start_ms: u64,
    error: String,
    /// a failed load was retried once with a fresh URL
    retried: bool,
    /// the player has played this load (history started)
    started: bool,
    played: Duration,
    played_since: Option<Instant>,

    effects: Vec<YtEffect>,
}

impl Default for YtLane {
    fn default() -> Self {
        Self {
            active: false,
            items: Vec::new(),
            order: Vec::new(),
            pos: 0,
            manual: Vec::new(),
            current: None,
            context: PlayContext::default(),
            shuffle: false,
            repeat: Repeat::Off,
            next_uid: FIRST_UID,
            direction: 1,
            seq: 0,
            stream: None,
            playing: false,
            loading: false,
            position_ms: 0,
            position_at: None,
            duration_ms: 0,
            start_ms: 0,
            error: String::new(),
            retried: false,
            started: false,
            played: Duration::ZERO,
            played_since: None,
            effects: Vec::new(),
        }
    }
}

impl YtLane {
    pub fn take_effects(&mut self) -> Vec<YtEffect> {
        std::mem::take(&mut self.effects)
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// The current load's sequence number.
    #[cfg(test)]
    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn track(&self) -> Option<&TrackItem> {
        self.current.as_ref().map(|current| &current.entry.track)
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn loading(&self) -> bool {
        self.loading
    }

    /// Extrapolated between the player's reports.
    pub fn position_ms(&self) -> u64 {
        let position = match self.position_at {
            Some(at) => self.position_ms + at.elapsed().as_millis() as u64,
            None => self.position_ms,
        };
        if self.duration_ms > 0 { position.min(self.duration_ms) } else { position }
    }

    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat(&self) -> Repeat {
        self.repeat
    }

    pub fn context(&self) -> &PlayContext {
        &self.context
    }

    pub fn error(&self) -> &str {
        &self.error
    }

    pub fn direction(&self) -> i32 {
        self.direction
    }

    pub fn stream(&self) -> Option<&YtStream> {
        self.stream.as_ref()
    }

    pub fn played_seconds(&self) -> f64 {
        let running = self.played_since.map_or(Duration::ZERO, |since| since.elapsed());
        (self.played + running).as_secs_f64()
    }

    pub fn manual_count(&self) -> usize {
        self.manual.len()
    }

    /// Current, "Playing next", then the rest of the context in play order.
    pub fn rows(&self) -> Vec<QueueRow> {
        let mut rows = Vec::new();
        if let Some(current) = &self.current {
            rows.push(row(QueueSection::Current, &current.entry));
        }
        rows.extend(self.manual.iter().map(|entry| row(QueueSection::Manual, entry)));
        rows.extend(self.order.iter().skip(self.pos + 1).map(|&index| row(QueueSection::Context, &self.items[index])));
        rows
    }

    // --- starting ----------------------------------------------------------------------------

    /// A list from the UI, starting at `row` (rows that are not YouTube tracks or unavailable are left out).
    pub fn play_items(&mut self, items: Vec<TrackItem>, row: usize, context: PlayContext) {
        let chosen = items.get(row).map(|item| item.uri.clone());
        let mut start = 0;
        let mut kept = Vec::new();
        for (index, item) in items.into_iter().enumerate() {
            if !is_lane_track(&item.uri) || !item.available {
                continue;
            }
            if index == row {
                start = kept.len();
            }
            kept.push(item);
        }
        if kept.is_empty() || chosen.is_none_or(|uri| !is_lane_track(&uri)) {
            return;
        }
        self.start_list(kept, start, context);
    }

    /// The previous session's song, shown paused at `position_ms` with the rows that followed it. Nothing is
    /// resolved until Play, so the app starts quietly; transport then belongs to the lane, not to Spotify.
    pub fn restore(&mut self, track: TrackItem, rows: Vec<TrackItem>, position_ms: u64, context: PlayContext) {
        if !is_lane_track(&track.uri) {
            return;
        }
        let items: Vec<TrackItem> = std::iter::once(track)
            .chain(rows.into_iter().filter(|row| is_lane_track(&row.uri) && row.available))
            .collect();
        self.leave(false);
        self.active = true;
        self.items = items.into_iter().map(|track| self.entry(track)).collect();
        self.order = (0..self.items.len()).collect();
        self.pos = 0;
        self.manual.clear();
        self.context = context;
        self.direction = 1;
        let entry = self.items[0].clone();
        self.duration_ms = entry.track.duration_ms.max(0) as u64;
        let cover = if entry.track.cover_large.is_empty() { entry.track.cover.clone() } else { entry.track.cover_large.clone() };
        self.current = Some(Current { entry, manual: false });
        self.seq += 1;
        self.stream = None;
        self.playing = false;
        self.loading = false;
        self.position_ms = if self.duration_ms > 0 { position_ms.min(self.duration_ms) } else { position_ms };
        self.position_at = None;
        self.start_ms = self.position_ms;
        self.error.clear();
        self.effects.push(YtEffect::Cover(cover));
    }

    /// What `restore` needs next time: the current song, where it is, and the rows after it.
    pub fn session(&self) -> Option<(TrackItem, u64, Vec<TrackItem>, PlayContext)> {
        let current = self.current.as_ref()?;
        let rows = self.manual.iter().chain(self.order.iter().skip(self.pos + 1).map(|&index| &self.items[index]));
        let rows = rows.take(30).map(|entry| entry.track.clone()).collect();
        Some((current.entry.track.clone(), self.position_ms(), rows, self.context.clone()))
    }

    /// Shuffle on and start from a random track.
    pub fn shuffle_from(&mut self, items: Vec<TrackItem>, context: PlayContext) {
        let items: Vec<TrackItem> =
            items.into_iter().filter(|item| is_lane_track(&item.uri) && item.available).collect();
        if items.is_empty() {
            return;
        }
        self.shuffle = true;
        let start = fastrand::usize(..items.len());
        self.start_list(items, start, context);
    }

    /// URIs only: titles, artists and covers come from the resolved stream.
    pub fn play_uris(&mut self, uris: Vec<String>, index: usize, context: PlayContext) {
        let chosen = uris.get(index).cloned().unwrap_or_default();
        let items: Vec<TrackItem> = uris
            .iter()
            .filter(|uri| is_lane_track(uri))
            .map(|uri| TrackItem {
                track_id: lane_track_id(uri).unwrap_or_default().to_owned(),
                uri: uri.clone(),
                available: true,
                ..Default::default()
            })
            .collect();
        let Some(start) = items.iter().position(|item| item.uri == chosen) else {
            return;
        };
        self.start_list(items, start, context);
    }

    fn start_list(&mut self, items: Vec<TrackItem>, start: usize, context: PlayContext) {
        self.leave(false);
        self.active = true;
        self.items = items.into_iter().map(|track| self.entry(track)).collect();
        self.context = context;
        self.order = (0..self.items.len()).collect();
        self.pos = start;
        if self.shuffle {
            self.reshuffle();
        }
        self.direction = 1;
        let entry = self.items[self.order[self.pos]].clone();
        self.current = Some(Current { entry, manual: false });
        self.load(0, true);
    }

    fn entry(&mut self, track: TrackItem) -> Entry {
        let uid = self.next_uid;
        self.next_uid += 1;
        Entry { uid, track }
    }

    /// Put the track at `pos` first and shuffle the rest after it.
    fn reshuffle(&mut self) {
        let Some(&first) = self.order.get(self.pos) else {
            return;
        };
        let mut rest: Vec<usize> = (0..self.items.len()).filter(|&index| index != first).collect();
        fastrand::shuffle(&mut rest);
        self.order = std::iter::once(first).chain(rest).collect();
        self.pos = 0;
    }

    /// Start loading the current track: resolve, then the player opens it.
    fn load(&mut self, position_ms: u64, play: bool) {
        let Some(current) = &self.current else {
            return;
        };
        let track = current.entry.track.clone();
        if self.stream.is_some() || self.loading {
            self.effects.push(YtEffect::Engine(YtCommand::Stop));
        }
        self.seq += 1;
        self.stream = None;
        self.playing = play;
        self.loading = true;
        self.position_ms = position_ms;
        self.position_at = None;
        self.start_ms = position_ms;
        self.duration_ms = track.duration_ms.max(0) as u64;
        self.error.clear();
        self.retried = false;
        self.started = false;
        self.played = Duration::ZERO;
        self.played_since = None;
        let cover = if track.cover_large.is_empty() { track.cover.clone() } else { track.cover_large.clone() };
        self.effects.push(YtEffect::Cover(cover));
        self.effects.push(YtEffect::Resolve { seq: self.seq, request: StreamRequest::from_track(&track) });
    }

    /// Leaving the current track: history gets its end.
    fn leave(&mut self, natural: bool) {
        self.pause_clock();
        if !self.started {
            return;
        }
        self.started = false;
        let Some(current) = &self.current else {
            return;
        };
        let track = current.entry.track.clone();
        self.effects.push(YtEffect::Ended(Box::new(TrackEndedLite {
            end: track.duration_ms.max(0) as f64 / 1000.0,
            track,
            context: self.context.clone(),
            played: self.played.as_secs_f64(),
            natural,
            local: true,
        })));
    }

    fn pause_clock(&mut self) {
        self.position_ms = self.position_ms();
        self.position_at = None;
        if let Some(since) = self.played_since.take() {
            self.played += since.elapsed();
        }
    }

    /// Another source takes over: stop the player and forget the lane's load.
    pub fn stop(&mut self) {
        if !self.active {
            return;
        }
        self.leave(false);
        self.effects.push(YtEffect::Engine(YtCommand::Stop));
        self.active = false;
        self.seq += 1;
        self.stream = None;
        self.playing = false;
        self.loading = false;
        self.error.clear();
    }

    // --- transport ---------------------------------------------------------------------------

    pub fn toggle(&mut self) {
        if self.playing { self.pause() } else { self.play() }
    }

    pub fn play(&mut self) {
        if self.current.is_none() || self.playing {
            return;
        }
        if self.stream.is_none() && !self.loading {
            // ended, failed or stopped: load again where it was
            let at = if self.duration_ms > 0 && self.position_ms >= self.duration_ms { 0 } else { self.position_ms };
            self.load(at, true);
            return;
        }
        self.playing = true;
        if self.stream.is_some() {
            self.effects.push(YtEffect::Engine(YtCommand::Play));
        }
    }

    pub fn pause(&mut self) {
        if !self.playing {
            return;
        }
        self.playing = false;
        self.pause_clock();
        if self.stream.is_some() {
            self.effects.push(YtEffect::Engine(YtCommand::Pause));
        }
    }

    pub fn seek(&mut self, position_ms: u64) {
        if self.current.is_none() {
            return;
        }
        let position = if self.duration_ms > 0 { position_ms.min(self.duration_ms) } else { position_ms };
        self.position_ms = position;
        self.position_at = None;
        if self.stream.is_some() {
            self.effects.push(YtEffect::Engine(YtCommand::Seek(position)));
        } else {
            self.start_ms = position;
        }
    }

    pub fn seek_relative(&mut self, delta_ms: i64) {
        let target = (self.position_ms() as i64 + delta_ms).max(0) as u64;
        self.seek(target);
    }

    /// The next track: "Playing next" first, then the context; at the end — repeat all wraps, otherwise stop.
    pub fn next(&mut self) {
        self.advance(false);
    }

    fn advance(&mut self, natural: bool) {
        if self.current.is_none() {
            return;
        }
        self.leave(natural);
        self.direction = 1;
        if !self.manual.is_empty() {
            let entry = self.manual.remove(0);
            self.current = Some(Current { entry, manual: true });
            self.load(0, true);
            return;
        }
        let after = self.pos + 1;
        let index = if after < self.order.len() {
            Some(after)
        } else if self.repeat == Repeat::All && !self.order.is_empty() {
            if self.shuffle {
                // a new round in a new order
                let mut order: Vec<usize> = (0..self.items.len()).collect();
                fastrand::shuffle(&mut order);
                self.order = order;
            }
            Some(0)
        } else {
            None
        };
        match index {
            Some(index) => {
                self.pos = index;
                let entry = self.items[self.order[index]].clone();
                self.current = Some(Current { entry, manual: false });
                self.load(0, true);
            }
            None => self.finish(),
        }
    }

    /// The end of the list: the last track stays shown, stopped at the start.
    fn finish(&mut self) {
        if self.stream.is_some() || self.loading {
            self.effects.push(YtEffect::Engine(YtCommand::Stop));
        }
        self.seq += 1;
        self.stream = None;
        self.playing = false;
        self.loading = false;
        self.position_ms = 0;
        self.position_at = None;
    }

    /// After 3 s — back to the start of the track, otherwise the previous track.
    pub fn previous(&mut self) {
        let Some(current) = &self.current else {
            return;
        };
        if self.position_ms() as i64 > RESTART_BEFORE_PREVIOUS_MS {
            self.seek(0);
            return;
        }
        let index = if current.manual {
            // the context track before the queued one
            Some(self.pos).filter(|&pos| pos < self.order.len())
        } else if self.pos > 0 {
            Some(self.pos - 1)
        } else if self.repeat == Repeat::All && !self.order.is_empty() {
            Some(self.order.len() - 1)
        } else {
            None
        };
        let Some(index) = index else {
            self.seek(0);
            return;
        };
        self.leave(false);
        self.direction = -1;
        self.pos = index;
        let entry = self.items[self.order[index]].clone();
        self.current = Some(Current { entry, manual: false });
        self.load(0, true);
    }

    /// A click on a queue row: rows above it are dropped (as N × Next), the context goes on from there.
    pub fn play_row(&mut self, uid: u64) -> bool {
        if let Some(at) = self.manual.iter().position(|entry| entry.uid == uid) {
            self.leave(false);
            self.direction = 1;
            let entry = self.manual.remove(at);
            self.manual.drain(..at);
            self.current = Some(Current { entry, manual: true });
            self.load(0, true);
            return true;
        }
        let from = self.pos + 1;
        let Some(offset) = self.order.iter().skip(from).position(|&index| self.items[index].uid == uid) else {
            return false;
        };
        self.leave(false);
        self.direction = 1;
        self.manual.clear();
        self.pos = from + offset;
        let entry = self.items[self.order[self.pos]].clone();
        self.current = Some(Current { entry, manual: false });
        self.load(0, true);
        true
    }

    /// The row at `index` of `rows()` (0 — current).
    pub fn play_index(&mut self, index: usize) {
        if index == 0 {
            return;
        }
        if let Some(row) = self.rows().get(index) {
            self.play_row(row.uid);
        }
    }

    pub fn set_repeat(&mut self, repeat: Repeat) {
        self.repeat = repeat;
    }

    pub fn set_shuffle(&mut self, shuffle: bool) {
        if self.shuffle == shuffle {
            return;
        }
        self.shuffle = shuffle;
        if self.order.is_empty() {
            return;
        }
        let at = self.order[self.pos.min(self.order.len() - 1)];
        if shuffle {
            self.pos = self.pos.min(self.order.len() - 1);
            self.reshuffle();
        } else {
            self.order = (0..self.items.len()).collect();
            self.pos = at;
        }
    }

    /// More tracks for the playing context (a radio filling in after its first song started): appended to the
    /// list only while that context still plays; tracks already in it are skipped. Returns how many were added.
    pub fn extend_context(&mut self, context_uri: &str, items: Vec<TrackItem>) -> usize {
        if !self.active || self.context.context_uri.as_deref() != Some(context_uri) {
            return 0;
        }
        self.append(items)
    }

    /// Autoplay: the song whose radio should follow once the list runs out — the current YouTube song while at
    /// most one song of the list is left after it, nothing is in "Playing next" and repeat is off.
    pub fn autoplay_seed(&self) -> Option<&TrackItem> {
        let current = self.current.as_ref()?;
        let left = self.order.len().saturating_sub(self.pos + 1);
        (self.active && self.repeat == Repeat::Off && self.manual.is_empty() && left <= 1)
            .then_some(&current.entry.track)
            .filter(|track| track.uri.starts_with("youtube:track:"))
    }

    /// Autoplay: `items` (the radio of `seed_uri`) go after the list, if that song still plays.
    pub fn autoplay(&mut self, seed_uri: &str, items: Vec<TrackItem>) -> usize {
        if !self.active || self.current.as_ref().is_none_or(|current| current.entry.track.uri != seed_uri) {
            return 0;
        }
        self.append(items)
    }

    fn append(&mut self, items: Vec<TrackItem>) -> usize {
        let first = self.items.len();
        for track in items {
            if !is_lane_track(&track.uri) || !track.available || self.items.iter().any(|entry| entry.track.uri == track.uri)
            {
                continue;
            }
            let entry = self.entry(track);
            self.items.push(entry);
        }
        let mut added: Vec<usize> = (first..self.items.len()).collect();
        if self.shuffle {
            fastrand::shuffle(&mut added);
        }
        self.order.extend(&added);
        added.len()
    }

    // --- "Playing next" ----------------------------------------------------------------------

    /// Adds YouTube tracks to "Playing next"; returns how many were added.
    pub fn add(&mut self, items: Vec<TrackItem>) -> usize {
        let before = self.manual.len();
        for track in items.into_iter().filter(|item| is_lane_track(&item.uri)) {
            let entry = self.entry(track);
            self.manual.push(entry);
        }
        self.manual.len() - before
    }

    pub fn insert_manual(&mut self, item: TrackItem, at: usize) {
        if !is_lane_track(&item.uri) {
            return;
        }
        let entry = self.entry(item);
        let at = at.min(self.manual.len());
        self.manual.insert(at, entry);
    }

    pub fn move_manual(&mut self, from: usize, to: usize) {
        if from >= self.manual.len() {
            return;
        }
        let entry = self.manual.remove(from);
        let to = to.min(self.manual.len());
        self.manual.insert(to, entry);
    }

    /// Moves a row of the rest of the context: `from`, `to` count from the first row after the current one.
    pub fn move_context(&mut self, from: usize, to: usize) {
        let start = self.pos + 1;
        if start + from >= self.order.len() {
            return;
        }
        let index = self.order.remove(start + from);
        let to = (start + to).min(self.order.len());
        self.order.insert(to, index);
    }

    pub fn clear_manual(&mut self) {
        self.manual.clear();
    }

    // --- what comes back ---------------------------------------------------------------------

    /// The stream of load `seq` is resolved (or not).
    pub fn resolved(&mut self, seq: u64, result: Box<Result<YtStream, String>>) {
        if seq != self.seq || !self.active {
            return;
        }
        match *result {
            Ok(stream) => {
                self.fill_from(&stream);
                if self.duration_ms == 0 && stream.duration_ms > 0 {
                    self.duration_ms = stream.duration_ms as u64;
                }
                self.effects.push(YtEffect::Engine(YtCommand::Load {
                    seq,
                    stream: Box::new(stream.clone()),
                    position_ms: self.start_ms,
                    play: self.playing,
                }));
                self.stream = Some(stream);
                if let Some(next) = self.peek_next() {
                    self.effects.push(YtEffect::Prefetch(next));
                }
            }
            Err(error) => self.failed(error),
        }
    }

    /// A URI-only track learns its title, artist and cover from the player response.
    fn fill_from(&mut self, stream: &YtStream) {
        let Some(current) = &mut self.current else {
            return;
        };
        let uid = current.entry.uid;
        let track = &mut current.entry.track;
        let mut changed = false;
        if track.title.is_empty() && !stream.title.is_empty() {
            track.title = stream.title.clone();
            changed = true;
        }
        if track.artists.is_empty() && !stream.author.is_empty() {
            track.artists = stream.author.clone();
            changed = true;
        }
        if track.duration_ms <= 0 && stream.duration_ms > 0 {
            track.duration_ms = stream.duration_ms;
            changed = true;
        }
        let had_cover = !track.cover.is_empty() || !track.cover_large.is_empty();
        if !had_cover && !stream.thumbnail.is_empty() {
            track.cover = stream.thumbnail.clone();
            track.cover_large = stream.thumbnail.clone();
            changed = true;
            self.effects.push(YtEffect::Cover(stream.thumbnail.clone()));
        }
        if changed {
            let track = track.clone();
            // the same row further down the list (repeat all) shows it too
            for entry in self.items.iter_mut().chain(self.manual.iter_mut()) {
                if entry.uid == uid {
                    entry.track = track.clone();
                }
            }
        }
    }

    fn peek_next(&self) -> Option<StreamRequest> {
        let entry = match self.manual.first() {
            Some(entry) => entry,
            None => {
                let index = match self.order.get(self.pos + 1) {
                    Some(&index) => index,
                    None if self.repeat == Repeat::All && !self.shuffle => *self.order.first()?,
                    None => return None,
                };
                &self.items[index]
            }
        };
        lane_id(&entry.track.uri).map(|_| StreamRequest::from_track(&entry.track))
    }

    /// What the player reports about load `seq`.
    pub fn event(&mut self, seq: u64, event: YtEvent) {
        if seq != self.seq || !self.active {
            return;
        }
        match event {
            YtEvent::Loading => self.loading = true,
            YtEvent::Playing { position_ms } => {
                self.loading = false;
                self.playing = true;
                self.position_ms = position_ms;
                self.position_at = Some(Instant::now());
                if self.played_since.is_none() {
                    self.played_since = Some(Instant::now());
                }
                if !self.started {
                    self.started = true;
                    if let Some(current) = &self.current {
                        self.effects.push(YtEffect::Started(Box::new(TrackStartedLite {
                            track: current.entry.track.clone(),
                            context: self.context.clone(),
                            local: true,
                        })));
                    }
                }
            }
            YtEvent::Paused { position_ms } => {
                self.loading = false;
                self.pause_clock();
                self.position_ms = position_ms;
            }
            YtEvent::Position(position_ms) => {
                self.position_ms = position_ms;
                if self.playing && !self.loading {
                    self.position_at = Some(Instant::now());
                }
            }
            YtEvent::Ended => {
                if let Some(current) = &self.current {
                    self.position_ms = current.entry.track.duration_ms.max(0) as u64;
                }
                self.position_at = None;
                if self.repeat == Repeat::One {
                    self.leave(true);
                    self.load(0, true);
                } else {
                    self.advance(true);
                }
            }
            YtEvent::Error(error) => {
                if !self.retried && self.stream.is_some() {
                    // most often an expired URL: forget it and resolve again, once
                    let position = self.position_ms();
                    let retried_load = self.started;
                    let video = self.stream.as_ref().map(|stream| stream.video_id.clone()).unwrap_or_default();
                    self.effects.push(YtEffect::Invalidate(video.clone()));
                    self.pause_clock();
                    self.seq += 1;
                    self.stream = None;
                    self.loading = true;
                    self.start_ms = position;
                    self.retried = true;
                    // the listening goes on: history keeps its start
                    self.started = retried_load;
                    let mut request = self
                        .current
                        .as_ref()
                        .map_or_else(StreamRequest::default, |current| StreamRequest::from_track(&current.entry.track));
                    request.video_id = video;
                    self.effects.push(YtEffect::Resolve { seq: self.seq, request });
                } else {
                    self.failed(error);
                }
            }
        }
    }

    fn failed(&mut self, error: String) {
        tracing::warn!("youtube playback failed: {error}");
        self.pause_clock();
        self.error = error;
        self.stream = None;
        self.loading = false;
        self.playing = false;
    }
}

fn row(section: QueueSection, entry: &Entry) -> QueueRow {
    QueueRow { section, uid: entry.uid, track: entry.track.clone() }
}

/// The YouTube player as the actor sees it (tests record the commands instead).
pub(crate) trait YtEngine: Send + Sync {
    fn send(&self, command: YtCommand);
    /// sound level 0..1 for the cover's breathing
    fn level(&self) -> f64;
    fn set_levels_wanted(&self, _wanted: bool) {}
}

impl YtEngine for YtPlayer {
    fn send(&self, command: YtCommand) {
        YtPlayer::send(self, command);
    }

    fn level(&self) -> f64 {
        self.tap().level()
    }

    fn set_levels_wanted(&self, wanted: bool) {
        self.tap().set_wanted(wanted);
    }
}

/// Where the lane gets streams and which player it drives.
pub(crate) struct YtLink {
    pub source: Arc<dyn StreamSource>,
    pub engine: Arc<dyn YtEngine>,
}

/// Messages of the lane to the actor.
pub(crate) enum YtMsg {
    Enable(YtLink),
    Resolved { seq: u64, result: Box<Result<YtStream, String>> },
    Event { seq: u64, event: YtEvent },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yt(id: &str) -> TrackItem {
        TrackItem {
            track_id: id.into(),
            uri: format!("youtube:track:{id}"),
            title: id.to_uppercase(),
            duration_ms: 200_000,
            available: true,
            ..Default::default()
        }
    }

    fn stream(id: &str) -> YtStream {
        YtStream { video_id: id.into(), url: format!("https://example.invalid/{id}"), ..Default::default() }
    }

    fn uris(lane: &YtLane) -> Vec<(QueueSection, String)> {
        lane.rows().into_iter().map(|row| (row.section, row.track.track_id)).collect()
    }

    fn resolves(effects: &[YtEffect]) -> Vec<(u64, String)> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                YtEffect::Resolve { seq, request } => Some((*seq, request.video_id.clone())),
                _ => None,
            })
            .collect()
    }

    /// Plays `ids` from `row`, resolves the stream and reports it playing.
    fn started(ids: &[&str], row: usize) -> YtLane {
        let mut lane = YtLane::default();
        lane.play_items(ids.iter().map(|id| yt(id)).collect(), row, PlayContext::new("playlist"));
        let seq = lane.seq();
        lane.resolved(seq, Box::new(Ok(stream(ids[row]))));
        lane.event(seq, YtEvent::Playing { position_ms: 0 });
        lane.take_effects();
        lane
    }

    #[test]
    fn a_radio_extends_the_playing_list_without_restarting_it() {
        let mut lane = YtLane::default();
        let radio = PlayContext::with_uri("radio", "youtube:playlist:RDAMVMa");
        lane.play_items(vec![yt("a")], 0, radio);
        let seq = lane.seq();
        lane.take_effects();
        // the seed comes back first in the radio: skipped; a stale context adds nothing
        assert_eq!(lane.extend_context("youtube:playlist:RDAMVMx", vec![yt("z")]), 0);
        assert_eq!(lane.extend_context("youtube:playlist:RDAMVMa", vec![yt("a"), yt("b"), yt("c")]), 2);
        assert_eq!(lane.seq(), seq);
        assert!(resolves(&lane.take_effects()).is_empty());
        assert_eq!(
            uris(&lane),
            [(QueueSection::Current, "a".into()), (QueueSection::Context, "b".into()), (QueueSection::Context, "c".into())]
        );
        lane.next();
        assert_eq!(lane.track().map(|track| track.track_id.as_str()), Some("b"));
    }

    #[test]
    fn playing_a_list_resolves_then_loads_the_chosen_track() {
        let mut lane = YtLane::default();
        let mut items = vec![yt("a"), yt("b"), yt("c")];
        items.insert(1, TrackItem { uri: "spotify:track:x".into(), available: true, ..Default::default() });
        lane.play_items(items, 2, PlayContext::new("playlist"));
        assert!(lane.is_active() && lane.loading() && lane.playing());
        let effects = lane.take_effects();
        assert_eq!(resolves(&effects), [(1, "b".to_owned())]);
        // the Spotify row is not part of the lane
        assert_eq!(
            uris(&lane),
            [(QueueSection::Current, "b".into()), (QueueSection::Context, "c".into())]
        );
        lane.resolved(1, Box::new(Ok(stream("b"))));
        let effects = lane.take_effects();
        assert!(effects.iter().any(|effect| matches!(
            effect,
            YtEffect::Engine(YtCommand::Load { seq: 1, position_ms: 0, play: true, .. })
        )));
        assert!(effects.iter().any(|effect| matches!(effect, YtEffect::Prefetch(request) if request.video_id == "c")));
        lane.event(1, YtEvent::Playing { position_ms: 0 });
        assert!(!lane.loading());
        assert!(matches!(lane.take_effects().as_slice(), [YtEffect::Started(started)] if started.track.track_id == "b"));
    }

    #[test]
    fn local_files_play_in_the_lane_beside_youtube_tracks() {
        let local = |id: &str| TrackItem { track_id: id.into(), uri: format!("local:track:{id}"), available: true, ..Default::default() };
        let mut lane = YtLane::default();
        lane.play_items(vec![local("f1"), yt("a"), local("f2")], 0, PlayContext::new("local"));
        // the source gets a local file's whole URI, a YouTube track's video id
        assert_eq!(resolves(&lane.take_effects()), [(1, "local:track:f1".to_owned())]);
        lane.resolved(1, Box::new(Ok(stream("local:track:f1"))));
        let effects = lane.take_effects();
        assert!(effects.iter().any(|effect| matches!(effect, YtEffect::Prefetch(request) if request.video_id == "a")));
        assert_eq!(uris(&lane).len(), 3);
        lane.play_uris(vec!["local:track:f3".into()], 0, PlayContext::new("history"));
        assert_eq!(uris(&lane), [(QueueSection::Current, "f3".into())]);
        assert!(is_lane_track("local:track:f3") && !is_lane_track("local:track:") && !is_lane_track("spotify:track:x"));
    }

    #[test]
    fn results_and_events_of_an_older_load_are_dropped() {
        let mut lane = started(&["a", "b"], 0);
        let old = lane.seq();
        lane.next();
        let effects = lane.take_effects();
        assert!(effects.iter().any(|effect| matches!(effect, YtEffect::Ended(ended) if !ended.natural)));
        // the old track's Ended must not skip the new one, its stream must not load
        lane.event(old, YtEvent::Ended);
        lane.resolved(old, Box::new(Ok(stream("a"))));
        lane.event(old, YtEvent::Playing { position_ms: 5000 });
        assert!(lane.take_effects().is_empty());
        assert_eq!(lane.track().map(|track| track.track_id.as_str()), Some("b"));
        assert!(lane.loading());
    }

    #[test]
    fn manual_rows_play_before_the_context() {
        let mut lane = started(&["a", "b"], 0);
        assert_eq!(lane.add(vec![yt("q1"), yt("q2"), TrackItem { uri: "spotify:track:s".into(), ..yt("s") }]), 2);
        assert_eq!(
            uris(&lane),
            [
                (QueueSection::Current, "a".into()),
                (QueueSection::Manual, "q1".into()),
                (QueueSection::Manual, "q2".into()),
                (QueueSection::Context, "b".into()),
            ]
        );
        lane.next();
        assert_eq!(lane.track().map(|track| track.track_id.as_str()), Some("q1"));
        lane.next();
        lane.next();
        // after the queued songs the context goes on where it was
        assert_eq!(lane.track().map(|track| track.track_id.as_str()), Some("b"));
    }

    #[test]
    fn a_click_on_a_row_drops_the_rows_above_it() {
        let mut lane = started(&["a", "b", "c", "d"], 0);
        lane.add(vec![yt("q")]);
        let target = lane.rows()[3].uid; // "c"
        assert!(lane.play_row(target));
        assert_eq!(uris(&lane), [(QueueSection::Current, "c".into()), (QueueSection::Context, "d".into())]);
    }

    #[test]
    fn context_rows_can_be_reordered_and_play_in_the_new_order() {
        let mut lane = started(&["a", "b", "c", "d"], 0);
        lane.move_context(2, 0); // "d" first
        lane.move_context(9, 0); // out of range: nothing
        let context = |lane: &YtLane| -> Vec<String> {
            lane.rows().into_iter().filter(|row| row.section == QueueSection::Context).map(|row| row.track.track_id).collect()
        };
        assert_eq!(context(&lane), ["d", "b", "c"]);
        lane.next();
        assert_eq!(lane.track().map(|track| track.track_id.as_str()), Some("d"));
        assert_eq!(context(&lane), ["b", "c"]);
    }

    #[test]
    fn previous_restarts_after_three_seconds() {
        let mut lane = started(&["a", "b"], 1);
        lane.event(lane.seq(), YtEvent::Position(10_000));
        lane.previous();
        assert_eq!(lane.track().map(|track| track.track_id.as_str()), Some("b"));
        assert!(lane.take_effects().contains(&YtEffect::Engine(YtCommand::Seek(0))));
        lane.event(lane.seq(), YtEvent::Position(1_000));
        lane.previous();
        assert_eq!(lane.track().map(|track| track.track_id.as_str()), Some("a"));
        assert_eq!(lane.direction(), -1);
    }

    #[test]
    fn the_end_of_the_list_stops_or_wraps() {
        let mut lane = started(&["a", "b"], 1);
        lane.event(lane.seq(), YtEvent::Ended);
        let effects = lane.take_effects();
        assert!(effects.iter().any(|effect| matches!(effect, YtEffect::Ended(ended) if ended.natural)));
        assert!(!lane.playing());
        assert_eq!(lane.track().map(|track| track.track_id.as_str()), Some("b"));
        // play again loads it from the start
        lane.play();
        assert_eq!(resolves(&lane.take_effects()).len(), 1);

        let mut lane = started(&["a", "b"], 1);
        lane.set_repeat(Repeat::All);
        lane.event(lane.seq(), YtEvent::Ended);
        assert_eq!(lane.track().map(|track| track.track_id.as_str()), Some("a"));

        let mut lane = started(&["a", "b"], 0);
        lane.set_repeat(Repeat::One);
        lane.event(lane.seq(), YtEvent::Ended);
        assert_eq!(lane.track().map(|track| track.track_id.as_str()), Some("a"));
        assert_eq!(resolves(&lane.take_effects()), [(lane.seq(), "a".to_owned())]);
    }

    #[test]
    fn shuffle_keeps_the_current_track_and_every_other_once() {
        let ids: Vec<String> = (0..20).map(|i| format!("t{i}")).collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let mut lane = started(&refs, 5);
        lane.set_shuffle(true);
        let rows = uris(&lane);
        assert_eq!(rows[0], (QueueSection::Current, "t5".into()));
        let mut rest: Vec<String> = rows[1..].iter().map(|(_, id)| id.clone()).collect();
        rest.sort();
        let mut expected: Vec<String> = ids.iter().filter(|id| *id != "t5").cloned().collect();
        expected.sort();
        assert_eq!(rest, expected);
        // shuffle off: the list order again, after the current track
        lane.set_shuffle(false);
        assert_eq!(uris(&lane)[1], (QueueSection::Context, "t6".into()));
    }

    #[test]
    fn a_player_error_retries_once_with_a_fresh_url() {
        let mut lane = started(&["a", "b"], 0);
        lane.event(lane.seq(), YtEvent::Position(42_000));
        lane.event(lane.seq(), YtEvent::Error("403".into()));
        let effects = lane.take_effects();
        assert!(effects.contains(&YtEffect::Invalidate("a".into())));
        let retry = resolves(&effects);
        assert_eq!(retry.len(), 1);
        lane.resolved(retry[0].0, Box::new(Ok(stream("a"))));
        assert!(lane.take_effects().iter().any(|effect| matches!(
            effect,
            YtEffect::Engine(YtCommand::Load { position_ms: 42_000, .. })
        )));
        lane.event(lane.seq(), YtEvent::Error("403 again".into()));
        assert_eq!(lane.error(), "403 again");
        assert!(!lane.playing() && !lane.loading());
        assert!(resolves(&lane.take_effects()).is_empty());
    }

    #[test]
    fn uri_only_tracks_learn_their_metadata_from_the_stream() {
        let mut lane = YtLane::default();
        lane.play_uris(vec!["youtube:track:a".into(), "youtube:track:b".into()], 1, PlayContext::new("history"));
        let seq = lane.seq();
        let effects = lane.take_effects();
        assert!(effects.contains(&YtEffect::Cover(String::new())));
        let resolved = YtStream {
            title: "Song".into(),
            author: "Band".into(),
            thumbnail: "https://i.ytimg.com/b.jpg".into(),
            duration_ms: 180_000,
            ..stream("b")
        };
        lane.resolved(seq, Box::new(Ok(resolved)));
        let track = lane.track().cloned().unwrap();
        assert_eq!((track.title.as_str(), track.artists.as_str(), track.duration_ms), ("Song", "Band", 180_000));
        assert_eq!(lane.duration_ms(), 180_000);
        assert!(lane.take_effects().contains(&YtEffect::Cover("https://i.ytimg.com/b.jpg".into())));
    }

    #[test]
    fn a_resolve_failure_shows_the_error() {
        let mut lane = YtLane::default();
        lane.play_items(vec![yt("a")], 0, PlayContext::new("search"));
        lane.resolved(lane.seq(), Box::new(Err("This song can't be played.".into())));
        assert_eq!(lane.error(), "This song can't be played.");
        assert!(!lane.playing() && !lane.loading());
    }

    #[test]
    fn stopping_ends_the_listening_and_the_player() {
        let mut lane = started(&["a"], 0);
        lane.stop();
        let effects = lane.take_effects();
        assert!(effects.iter().any(|effect| matches!(effect, YtEffect::Ended(_))));
        assert!(effects.contains(&YtEffect::Engine(YtCommand::Stop)));
        assert!(!lane.is_active());
        // late events of the stopped load change nothing
        lane.event(lane.seq() - 1, YtEvent::Playing { position_ms: 0 });
        assert!(!lane.playing());
    }

    #[test]
    fn pause_before_the_stream_arrives_loads_it_paused() {
        let mut lane = YtLane::default();
        lane.play_items(vec![yt("a")], 0, PlayContext::new("search"));
        lane.pause();
        lane.seek(30_000);
        let seq = lane.seq();
        lane.take_effects();
        lane.resolved(seq, Box::new(Ok(stream("a"))));
        assert!(lane.take_effects().iter().any(|effect| matches!(
            effect,
            YtEffect::Engine(YtCommand::Load { position_ms: 30_000, play: false, .. })
        )));
    }

    #[test]
    fn autoplay_asks_for_the_radio_near_the_end_and_appends_it_while_the_song_plays() {
        let mut lane = started(&["a", "b", "c"], 0);
        // two songs left: not yet
        assert!(lane.autoplay_seed().is_none());
        lane.next();
        assert_eq!(lane.autoplay_seed().map(|track| track.track_id.as_str()), Some("b"));
        // "Playing next" or repeat keep the list going on their own
        lane.add(vec![yt("m")]);
        assert!(lane.autoplay_seed().is_none());
        lane.clear_manual();
        lane.set_repeat(Repeat::All);
        assert!(lane.autoplay_seed().is_none());
        lane.set_repeat(Repeat::Off);

        // the radio of another song (the user moved on) is ignored; the seed and songs already listed are skipped
        assert_eq!(lane.autoplay("youtube:track:a", vec![yt("x")]), 0);
        assert_eq!(lane.autoplay("youtube:track:b", vec![yt("b"), yt("c"), yt("r1"), yt("r2")]), 2);
        let ids: Vec<String> = uris(&lane).into_iter().map(|(_, id)| id).collect();
        assert_eq!(ids, ["b", "c", "r1", "r2"]);
        assert!(lane.autoplay_seed().is_none());
    }

    #[test]
    fn a_restored_song_waits_for_play_and_starts_where_it_was() {
        let mut lane = YtLane::default();
        lane.restore(yt("a"), vec![yt("b")], 90_000, PlayContext::new("tracks"));
        assert!(lane.is_active() && !lane.playing() && !lane.loading());
        assert_eq!(lane.position_ms(), 90_000);
        assert!(resolves(&lane.take_effects()).is_empty());
        lane.play();
        assert_eq!(resolves(&lane.take_effects()), [(lane.seq(), "a".to_owned())]);
        let (track, position, rows, _) = lane.session().unwrap();
        assert_eq!((track.track_id.as_str(), position, rows.len()), ("a", 90_000, 1));
    }
}
