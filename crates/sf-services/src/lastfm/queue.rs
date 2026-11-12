//! Scrobbles waiting to be sent: kept in a file (no secrets — user name, track, time), sent in batches of up to 50,
//! dropped once older than Last.fm accepts (14 days). Not in the Kotlin code, which lost a scrobble that failed.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Last.fm takes at most this many scrobbles per track.scrobble call.
pub const BATCH: usize = 50;
/// Last.fm ignores scrobbles older than 14 days; a day of margin for clocks and the trip.
pub const MAX_AGE_S: i64 = 13 * 24 * 3600;
/// The file never grows past this (oldest dropped first): weeks offline at heavy listening.
pub const MAX_QUEUED: usize = 2000;
const VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Pending {
    /// the Last.fm account it belongs to (another account's entries wait until that one is back, or expire)
    pub user: String,
    pub artist: String,
    pub title: String,
    pub album: String,
    /// seconds; 0 — unknown
    pub duration_s: u64,
    /// unix seconds the listen started
    pub timestamp: i64,
}

impl Pending {
    fn same(&self, other: &Self) -> bool {
        self.user == other.user && self.artist == other.artist && self.title == other.title
            && self.timestamp == other.timestamp
    }
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    scrobbles: Vec<Pending>,
}

/// None for `path` — in memory only.
#[derive(Default)]
pub struct Queue {
    path: Option<PathBuf>,
    items: Vec<Pending>,
}

impl Queue {
    /// An unreadable or foreign file is ignored (and replaced on the next save).
    pub fn load(path: Option<PathBuf>) -> Self {
        let items = path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<File>(&bytes).ok())
            .filter(|file| file.version == VERSION)
            .map(|file| file.scrobbles)
            .unwrap_or_default();
        Self { path, items }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn items(&self) -> &[Pending] {
        &self.items
    }

    /// Adds (once — the same listen twice is one) and saves.
    pub fn push(&mut self, pending: Pending) {
        if self.items.iter().any(|item| item.same(&pending)) {
            return;
        }
        self.items.push(pending);
        if self.items.len() > MAX_QUEUED {
            let excess = self.items.len() - MAX_QUEUED;
            self.items.drain(..excess);
        }
        self.save();
    }

    /// Drops what Last.fm would no longer accept; how many were dropped.
    pub fn prune(&mut self, now_unix: i64) -> usize {
        let before = self.items.len();
        self.items.retain(|item| now_unix - item.timestamp <= MAX_AGE_S);
        let dropped = before - self.items.len();
        if dropped > 0 {
            self.save();
        }
        dropped
    }

    /// The oldest up to 50 of this account, oldest first.
    pub fn batch(&self, user: &str) -> Vec<Pending> {
        self.items.iter().filter(|item| item.user == user).take(BATCH).cloned().collect()
    }

    pub fn remove(&mut self, sent: &[Pending]) {
        let before = self.items.len();
        self.items.retain(|item| !sent.iter().any(|done| done.same(item)));
        if self.items.len() != before {
            self.save();
        }
    }

    /// Written to a temporary file and renamed; an empty queue removes the file.
    fn save(&self) {
        let Some(path) = &self.path else { return };
        let result = if self.items.is_empty() {
            match std::fs::remove_file(path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            }
        } else {
            let file = File { version: VERSION, scrobbles: self.items.clone() };
            let temporary = path.with_extension("json.tmp");
            path.parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&temporary, serde_json::to_vec(&file).unwrap_or_default()))
                .and_then(|()| std::fs::rename(&temporary, path))
        };
        if let Err(error) = result {
            tracing::warn!("Last.fm scrobble queue not saved: {error}");
        }
    }
}

/// track.scrobble parameters for a batch: artist[i], track[i], timestamp[i], album[i], duration[i].
pub fn batch_params(batch: &[Pending]) -> Vec<(String, String)> {
    let mut params = Vec::new();
    for (i, item) in batch.iter().enumerate() {
        params.push((format!("artist[{i}]"), item.artist.clone()));
        params.push((format!("track[{i}]"), item.title.clone()));
        params.push((format!("timestamp[{i}]"), item.timestamp.to_string()));
        if !item.album.is_empty() {
            params.push((format!("album[{i}]"), item.album.clone()));
        }
        if item.duration_s > 0 {
            params.push((format!("duration[{i}]"), item.duration_s.to_string()));
        }
    }
    params
}
