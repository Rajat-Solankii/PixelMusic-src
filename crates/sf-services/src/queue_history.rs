//! Queue history (PixelPlayer's "Replay queue"): when something else starts playing, the queue that was playing is
//! kept as a saved queue (its songs in order, the context title, when it was replaced). Newest first, at most
//! `KEPT` queues of at least `MIN_TRACKS` songs; a queue that starts like the newest saved one replaces it. Stored in
//! app_data_dir()/queue_history.json.
//!
//! The player bridge reports every new queue snapshot with `observe(context, rows)`: rows of the same context are
//! collected by uid (so songs that already played stay in the saved queue, the snapshot only holds the current and
//! upcoming ones); a different context saves the collected queue and starts a new one.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use md3_model::{PlayContext, QueueRow, TrackItem};
use tokio::runtime::Handle;
use tokio::sync::broadcast;

use crate::lock;

/// Saved queues kept (newest first).
pub const KEPT: usize = 12;
/// A queue of fewer songs is not worth saving.
pub const MIN_TRACKS: usize = 2;
/// Songs kept per queue (an endless radio would otherwise grow without end).
pub const MAX_TRACKS: usize = 300;

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SavedQueue {
    /// unique id (ms of the save, bumped on a clash)
    pub id: u64,
    /// unix ms when it was replaced
    pub saved_at: i64,
    /// "Playing from" title of the context, may be empty
    pub title: String,
    pub context_uri: String,
    pub tracks: Vec<TrackItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueHistoryEvent {
    Changed,
}

#[derive(Default)]
struct Live {
    context: PlayContext,
    tracks: Vec<TrackItem>,
    seen: HashSet<u64>,
}

#[derive(Default)]
struct State {
    saved: Vec<SavedQueue>,
    live: Option<Live>,
    dirty: bool,
}

pub struct QueueHistoryService {
    rt: Handle,
    path: PathBuf,
    state: Mutex<State>,
    events: broadcast::Sender<QueueHistoryEvent>,
    me: Weak<QueueHistoryService>,
}

impl QueueHistoryService {
    /// app_data_dir()/queue_history.json, or $PIXELMUSIC_QUEUE_HISTORY_FILE (screenshots, benches)
    pub fn new(rt: Handle) -> Arc<Self> {
        let path = match std::env::var_os("PIXELMUSIC_QUEUE_HISTORY_FILE") {
            Some(path) if !path.is_empty() => PathBuf::from(path),
            _ => mf_platform::dirs::app_data_dir().join("queue_history.json"),
        };
        Self::with_file(rt, path)
    }

    pub fn with_file(rt: Handle, path: PathBuf) -> Arc<Self> {
        let saved = load(&path);
        Arc::new_cyclic(|me| Self {
            rt,
            path,
            state: Mutex::new(State { saved, live: None, dirty: false }),
            events: broadcast::channel(16).0,
            me: me.clone(),
        })
    }

    /// A queue snapshot of the player (current + upcoming rows, in order).
    pub fn observe(&self, context: &PlayContext, rows: &[QueueRow]) {
        self.observe_at(context, rows, jiff::Timestamp::now().as_millisecond());
    }

    fn observe_at(&self, context: &PlayContext, rows: &[QueueRow], now_ms: i64) {
        let mut state = lock(&self.state);
        let same = state.live.as_ref().is_some_and(|live| same_context(&live.context, context));
        if !same {
            // the first snapshot after start is the restored session: nothing was replaced yet
            let old = state.live.replace(Live { context: context.clone(), ..Default::default() });
            if rows.is_empty() && old.as_ref().is_some_and(|o| !o.tracks.is_empty()) {
                // the queue emptied (stop / sign out): keep collecting the old one until something plays
                state.live = old;
                return;
            }
            if let Some(old) = old
                && save(&mut state.saved, old, now_ms)
            {
                state.dirty = true;
                drop(state);
                self.changed();
                state = lock(&self.state);
            }
        }
        let Some(live) = state.live.as_mut() else { return };
        live.context = context.clone();
        for row in rows {
            if live.tracks.len() < MAX_TRACKS && !row.track.uri.is_empty() && live.seen.insert(row.uid) {
                live.tracks.push(row.track.clone());
            }
        }
    }

    /// Saved queues, newest first.
    pub fn saved(&self) -> Vec<SavedQueue> {
        lock(&self.state).saved.clone()
    }

    pub fn get(&self, id: u64) -> Option<SavedQueue> {
        lock(&self.state).saved.iter().find(|q| q.id == id).cloned()
    }

    pub fn remove(&self, id: u64) {
        let mut state = lock(&self.state);
        let before = state.saved.len();
        state.saved.retain(|q| q.id != id);
        if state.saved.len() != before {
            state.dirty = true;
            drop(state);
            self.changed();
        }
    }

    pub fn clear(&self) {
        let mut state = lock(&self.state);
        if !state.saved.is_empty() {
            state.saved.clear();
            state.dirty = true;
            drop(state);
            self.changed();
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<QueueHistoryEvent> {
        self.events.subscribe()
    }

    fn changed(&self) {
        let me = self.me.clone();
        self.rt.spawn_blocking(move || {
            if let Some(service) = me.upgrade() {
                service.flush();
            }
        });
        let _ = self.events.send(QueueHistoryEvent::Changed);
    }

    /// Write to disk if changed.
    pub fn flush(&self) {
        let mut state = lock(&self.state);
        if !state.dirty {
            return;
        }
        match serde_json::to_vec(&state.saved) {
            Ok(bytes) => match crate::data::fsio::write_atomic(&self.path, &bytes) {
                Ok(()) => state.dirty = false,
                Err(error) => tracing::warn!("could not write queue history: {error:#}"),
            },
            Err(error) => tracing::warn!("could not serialize queue history: {error:#}"),
        }
    }
}

fn same_context(a: &PlayContext, b: &PlayContext) -> bool {
    a.kind == b.kind && a.context_uri == b.context_uri && a.title == b.title
}

/// The collected queue goes first into `saved`; true if it was kept.
fn save(saved: &mut Vec<SavedQueue>, live: Live, now_ms: i64) -> bool {
    if live.tracks.len() < MIN_TRACKS {
        return false;
    }
    let first = live.tracks[0].uri.clone();
    let context_uri = live.context.context_uri.clone().unwrap_or_default();
    // the same queue again (replayed, or the same playlist from its start): keep only the newer copy
    saved.retain(|q| !(q.tracks.first().is_some_and(|t| t.uri == first) && q.context_uri == context_uri));
    let mut id = now_ms.max(0) as u64;
    while saved.iter().any(|q| q.id == id) {
        id += 1;
    }
    saved.insert(0, SavedQueue { id, saved_at: now_ms, title: live.context.title.clone(), context_uri, tracks: live.tracks });
    saved.truncate(KEPT);
    true
}

fn load(path: &Path) -> Vec<SavedQueue> {
    std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use md3_model::QueueSection;

    fn temp_file() -> PathBuf {
        std::env::temp_dir().join(format!("sf-queue-history-{:016x}.json", rand::random::<u64>()))
    }

    fn row(uid: u64, uri: &str) -> QueueRow {
        QueueRow {
            section: QueueSection::Context,
            uid,
            track: TrackItem { uri: uri.into(), title: uri.to_uppercase(), ..Default::default() },
        }
    }

    fn context(uri: &str, title: &str) -> PlayContext {
        PlayContext { title: title.into(), ..PlayContext::with_uri("playlist", uri) }
    }

    fn uris(queue: &SavedQueue) -> Vec<&str> {
        queue.tracks.iter().map(|t| t.uri.as_str()).collect()
    }

    #[tokio::test]
    async fn a_replaced_queue_is_saved_with_the_songs_that_already_played() {
        let path = temp_file();
        let service = QueueHistoryService::with_file(Handle::current(), path.clone());
        let mix = context("youtube:playlist:A", "Mix A");
        service.observe_at(&mix, &[row(1, "a"), row(2, "b"), row(3, "c")], 1_000);
        // a played, the queue moved on and loaded more
        service.observe_at(&mix, &[row(2, "b"), row(3, "c"), row(4, "d")], 2_000);
        assert!(service.saved().is_empty(), "the playing queue is not history yet");

        service.observe_at(&context("youtube:album:B", "Album B"), &[row(9, "x"), row(10, "y")], 5_000);
        let saved = service.saved();
        assert_eq!(saved.len(), 1);
        assert_eq!(uris(&saved[0]), ["a", "b", "c", "d"]);
        assert_eq!((saved[0].title.as_str(), saved[0].context_uri.as_str(), saved[0].saved_at), ("Mix A", "youtube:playlist:A", 5_000));

        service.flush();
        let reloaded = QueueHistoryService::with_file(Handle::current(), path.clone());
        assert_eq!(reloaded.saved(), saved);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn short_and_repeated_queues_are_not_piled_up() {
        let path = temp_file();
        let service = QueueHistoryService::with_file(Handle::current(), path.clone());
        let a = context("youtube:playlist:A", "A");
        let single = context("", "");
        service.observe_at(&single, &[row(1, "one")], 1);
        service.observe_at(&a, &[row(2, "a1"), row(3, "a2")], 2);
        assert!(service.saved().is_empty(), "a single song is not a queue");

        // stopping empties the queue: the old one is still the live one
        service.observe_at(&PlayContext::default(), &[], 3);
        assert!(service.saved().is_empty());
        service.observe_at(&single, &[row(4, "solo")], 4);
        assert_eq!(service.saved().len(), 1);

        // A again from the start, then replaced: one copy, the newer
        service.observe_at(&a, &[row(5, "a1"), row(6, "a2"), row(7, "a3")], 5);
        service.observe_at(&single, &[row(8, "solo2")], 6);
        let saved = service.saved();
        assert_eq!(saved.len(), 1);
        assert_eq!((uris(&saved[0]), saved[0].saved_at), (vec!["a1", "a2", "a3"], 6));

        service.remove(saved[0].id);
        assert!(service.saved().is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_list_is_capped_and_ids_stay_unique() {
        let mut saved = Vec::new();
        for i in 0..KEPT + 3 {
            let live = Live {
                context: context(&format!("p:{i}"), ""),
                tracks: vec![TrackItem { uri: format!("t{i}"), ..Default::default() }, TrackItem { uri: "z".into(), ..Default::default() }],
                ..Default::default()
            };
            assert!(save(&mut saved, live, 7));
        }
        assert_eq!(saved.len(), KEPT);
        let ids: HashSet<u64> = saved.iter().map(|q| q.id).collect();
        assert_eq!(ids.len(), KEPT);
        assert_eq!(saved[0].tracks[0].uri, format!("t{}", KEPT + 2));
    }
}
