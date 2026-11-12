//! Search history (local): recent queries stored in app data dir as JSON.
//! Cap of ~20 queries, deduplicated, newest first. Recorded when search is run or result is opened.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use tokio::runtime::Handle;
use tokio::sync::broadcast;

use crate::lock;

/// How many queries to keep (newest first).
pub const KEPT: usize = 20;

/// One search query record.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchQuery {
    pub query: String,
    /// ISO timestamp
    pub searched_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchHistoryEvent {
    Changed,
}

#[derive(Default)]
struct State {
    queries: Vec<SearchQuery>,
    dirty: bool,
}

pub struct SearchHistoryService {
    rt: Handle,
    path: PathBuf,
    state: Mutex<State>,
    events: broadcast::Sender<SearchHistoryEvent>,
    me: Weak<SearchHistoryService>,
}

impl SearchHistoryService {
    /// New service with the default file path.
    pub fn new(rt: Handle) -> Arc<Self> {
        Self::with_file(rt, search_history_file())
    }

    /// New service with a specific file path (for tests).
    pub fn with_file(rt: Handle, path: PathBuf) -> Arc<Self> {
        let queries = load_queries(&path);
        Arc::new_cyclic(|me| Self {
            rt,
            path,
            state: Mutex::new(State { queries, dirty: false }),
            events: broadcast::channel(16).0,
            me: me.clone(),
        })
    }

    /// Add a query (duplicates are moved to front, list is capped).
    pub fn add_query(&self, query: String) {
        if query.trim().is_empty() {
            return;
        }
        let query = query.trim().to_owned();
        let searched_at = jiff::Timestamp::now().to_string();
        let mut state = lock(&self.state);
        // Remove duplicate if exists
        state.queries.retain(|q| q.query != query);
        // Search-as-you-type records "be", "beat", "beatles" one after another: the newest entry that this
        // query merely continues is replaced, not kept beside it
        let lower = query.to_lowercase();
        if state.queries.first().is_some_and(|newest| lower.starts_with(&newest.query.to_lowercase())) {
            state.queries.remove(0);
        }
        // Add new query at the front
        state.queries.insert(0, SearchQuery { query, searched_at });
        // Cap the list
        state.queries.truncate(KEPT);
        state.dirty = true;
        drop(state);
        self.save_later();
        let _ = self.events.send(SearchHistoryEvent::Changed);
    }

    /// Remove a specific query.
    pub fn remove_query(&self, query: &str) {
        let mut state = lock(&self.state);
        let before_len = state.queries.len();
        state.queries.retain(|q| q.query != query);
        if state.queries.len() != before_len {
            state.dirty = true;
            drop(state);
            self.save_later();
            let _ = self.events.send(SearchHistoryEvent::Changed);
        }
    }

    /// Clear all history.
    pub fn clear_history(&self) {
        let mut state = lock(&self.state);
        if !state.queries.is_empty() {
            state.queries.clear();
            state.dirty = true;
            drop(state);
            self.save_later();
            let _ = self.events.send(SearchHistoryEvent::Changed);
        }
    }

    /// Get all queries (newest first).
    pub fn queries(&self) -> Vec<SearchQuery> {
        lock(&self.state).queries.clone()
    }

    /// Get just the query strings (newest first).
    pub fn query_strings(&self) -> Vec<String> {
        lock(&self.state).queries.iter().map(|q| q.query.clone()).collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SearchHistoryEvent> {
        self.events.subscribe()
    }

    fn save_later(&self) {
        let me = self.me.clone();
        self.rt.spawn_blocking(move || {
            if let Some(service) = me.upgrade() {
                service.flush();
            }
        });
    }

    /// Save to disk (called on shutdown).
    pub fn flush(&self) {
        let mut state = lock(&self.state);
        if !state.dirty {
            return;
        }
        match serde_json::to_vec(&state.queries) {
            Ok(bytes) => match crate::data::fsio::write_atomic(&self.path, &bytes) {
                Ok(()) => state.dirty = false,
                Err(error) => tracing::warn!("could not write search history: {error:#}"),
            },
            Err(error) => tracing::warn!("could not serialize search history: {error:#}"),
        }
    }
}

fn load_queries(path: &Path) -> Vec<SearchQuery> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<SearchQuery>>(&text).ok())
        .unwrap_or_default()
}

/// The search history file: app_data_dir()/search_history.json.
pub fn search_history_file() -> PathBuf {
    mf_platform::dirs::app_data_dir().join("search_history.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file() -> PathBuf {
        std::env::temp_dir().join(format!("sf-search-history-{:016x}", rand::random::<u64>()))
    }

    #[tokio::test]
    async fn add_query_trims_and_deduplicates() {
        let rt = Handle::current();
        let path = temp_file();
        let service = SearchHistoryService::with_file(rt.clone(), path.clone());

        service.add_query("  hello world  ".to_string());
        assert_eq!(service.query_strings(), vec!["hello world"]);

        // Duplicate moves to front
        service.add_query("hello world".to_string());
        assert_eq!(service.query_strings(), vec!["hello world"]);

        service.add_query("foo bar".to_string());
        assert_eq!(service.query_strings(), vec!["foo bar", "hello world"]);

        // Another duplicate
        service.add_query("hello world".to_string());
        assert_eq!(service.query_strings(), vec!["hello world", "foo bar"]);

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn typing_on_replaces_the_shorter_query() {
        let path = temp_file();
        let service = SearchHistoryService::with_file(Handle::current(), path.clone());
        service.add_query("queen".to_string());
        for typed in ["be", "Beat", "beatles"] {
            service.add_query(typed.to_string());
        }
        assert_eq!(service.query_strings(), vec!["beatles", "queen"]);
        // an older entry that the query continues stays
        service.add_query("queen live".to_string());
        assert_eq!(service.query_strings(), vec!["queen live", "beatles", "queen"]);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn empty_query_is_ignored() {
        let rt = Handle::current();
        let path = temp_file();
        let service = SearchHistoryService::with_file(rt, path.clone());

        service.add_query("".to_string());
        service.add_query("   ".to_string());
        assert!(service.query_strings().is_empty());

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn remove_query_works() {
        let rt = Handle::current();
        let path = temp_file();
        let service = SearchHistoryService::with_file(rt, path.clone());

        service.add_query("a".to_string());
        service.add_query("b".to_string());
        service.add_query("c".to_string());
        assert_eq!(service.query_strings(), vec!["c", "b", "a"]);

        service.remove_query("b");
        assert_eq!(service.query_strings(), vec!["c", "a"]);

        // Removing non-existent is no-op
        service.remove_query("z");
        assert_eq!(service.query_strings(), vec!["c", "a"]);

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn clear_history_works() {
        let rt = Handle::current();
        let path = temp_file();
        let service = SearchHistoryService::with_file(rt, path.clone());

        service.add_query("a".to_string());
        service.add_query("b".to_string());
        assert!(!service.query_strings().is_empty());

        service.clear_history();
        assert!(service.query_strings().is_empty());

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn cap_limits_kept_queries() {
        let rt = Handle::current();
        let path = temp_file();
        let service = SearchHistoryService::with_file(rt, path.clone());

        for i in 0..KEPT + 10 {
            service.add_query(format!("query_{i}"));
        }
        assert_eq!(service.query_strings().len(), KEPT);
        assert_eq!(service.query_strings()[0], format!("query_{}", KEPT + 9));

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn persistence_works() {
        let rt = Handle::current();
        let path = temp_file();

        {
            let service = SearchHistoryService::with_file(rt.clone(), path.clone());
            service.add_query("persistent".to_string());
            service.flush();
        }

        {
            let service = SearchHistoryService::with_file(rt.clone(), path.clone());
            assert_eq!(service.query_strings(), vec!["persistent"]);
        }

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn events_are_sent() {
        use std::time::Duration;
        
        let rt = Handle::current();
        let path = temp_file();
        let service = SearchHistoryService::with_file(rt.clone(), path.clone());
        let mut rx = service.subscribe();

        // No events initially
        assert!(rx.try_recv().is_err());

        // Add query sends event
        service.add_query("event test".to_string());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err()); // No more events

        // Remove sends event
        service.remove_query("event test");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err()); // No more events

        // Remove non-existent doesn't send event
        service.remove_query("nonexistent");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(rx.try_recv().is_err());

        // Clear with items sends event
        service.add_query("test2".to_string());
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = rx.try_recv(); // Drain add event
        service.clear_history();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err()); // No more events

        // Clear when empty doesn't send event
        service.clear_history();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(rx.try_recv().is_err());

        let _ = std::fs::remove_file(&path);
    }
}
