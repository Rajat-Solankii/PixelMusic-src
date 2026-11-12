//! Search (`YouTube.search` / `searchSummary` / `searchContinuation` / `searchSuggestions`).
use md3_model::TrackItem;
use serde_json::json;

use super::client::{Login, WEB_REMIX};
use super::error::Result;
use super::models::{ItemsPage, SearchFilter, Shelf, Suggestions, YtItem};
use super::{YouTubeService, parsers};

/// The suggestion list shows at most this many queries.
const MAX_SUGGESTIONS: usize = 8;

impl YouTubeService {
    /// One kind of result (songs, albums, ...).
    pub async fn search(&self, query: &str, filter: SearchFilter) -> Result<ItemsPage> {
        let body = json!({ "query": query, "params": filter.params() });
        let response = self.client.post(&WEB_REMIX, "search", body, Login::IfSignedIn).await?;
        Ok(parsers::search_page(&response))
    }

    /// More results of a filtered search: the token goes in the query (`continuation` and `ctoken`).
    pub async fn search_continuation(&self, continuation: &str) -> Result<ItemsPage> {
        let query = [("continuation", continuation), ("ctoken", continuation)];
        let response = self.client.post_query(&WEB_REMIX, "search", json!({}), Login::IfSignedIn, &query).await?;
        Ok(parsers::search_continuation(&response))
    }

    /// Unfiltered search: "Top results" and one shelf per kind.
    pub async fn search_summary(&self, query: &str) -> Result<Vec<Shelf>> {
        let response = self.client.post(&WEB_REMIX, "search", json!({ "query": query }), Login::IfSignedIn).await?;
        Ok(parsers::search_summary(&response))
    }

    /// Completions while typing, and the items YouTube Music suggests with them.
    pub async fn suggestions(&self, input: &str) -> Result<Suggestions> {
        let body = json!({ "input": input });
        let response = self.client.post(&WEB_REMIX, "music/get_search_suggestions", body, Login::Never).await?;
        let mut suggestions = parsers::suggestions(&response);
        suggestions.queries.truncate(MAX_SUGGESTIONS);
        Ok(suggestions)
    }

    /// Songs only (playlist import matches against these).
    pub async fn search_songs(&self, query: &str) -> Result<Vec<TrackItem>> {
        Ok(self.search(query, SearchFilter::Songs).await?.items.into_iter().filter_map(YtItem::into_track).collect())
    }
}
