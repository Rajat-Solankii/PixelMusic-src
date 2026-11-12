//! Home, explore, new releases, charts, moods & genres (`YouTube.home` / `explore` / `newReleaseAlbums` /
//! `getChartsPage` / `moodAndGenres`).
use std::time::{Duration, Instant};

use md3_model::AlbumItem;
use serde_json::json;

use super::client::{Login, WEB_REMIX};
use super::error::Result;
use super::models::{BrowseRef, Mood, ShelvesPage, YtItem};
use super::{YouTubeService, parsers};

/// How long the default home feed is served from memory.
pub(crate) const HOME_TTL: Duration = Duration::from_secs(60);

const HOME: &str = "FEmusic_home";
const EXPLORE: &str = "FEmusic_explore";
const NEW_RELEASES: &str = "FEmusic_new_releases_albums";
const CHARTS: &str = "FEmusic_charts";
/// Asked for when the plain charts request comes back without sections.
const CHARTS_FALLBACK_PARAMS: &str = "ggMGCgQIgAQ=";
const MOODS: &str = "FEmusic_moods_and_genres";

impl YouTubeService {
    /// The home feed. The default feed is cached for a minute; `force_refresh` reads it again.
    pub async fn home(&self, force_refresh: bool) -> Result<ShelvesPage> {
        if !force_refresh && let Some(page) = self.cached_home() {
            return Ok(page);
        }
        let _gate = self.home_gate.lock().await;
        // a caller that waited at the gate gets the feed the previous one just read
        if !force_refresh && let Some(page) = self.cached_home() {
            return Ok(page);
        }
        let mut page = parsers::shelves_page(&self.browse(HOME, None, Login::IfSignedIn).await?);
        self.fill_durations(&mut page).await;
        *crate::lock(&self.home_cache) = Some((Instant::now(), page.clone()));
        Ok(page)
    }

    fn cached_home(&self) -> Option<ShelvesPage> {
        crate::lock(&self.home_cache)
            .as_ref()
            .filter(|(read, _)| read.elapsed() < HOME_TTL)
            .map(|(_, page)| page.clone())
    }

    pub(crate) fn invalidate_home(&self) {
        *crate::lock(&self.home_cache) = None;
    }

    /// More home shelves. An empty page ends the feed.
    pub async fn home_continuation(&self, continuation: &str) -> Result<ShelvesPage> {
        let mut page = self.shelves_continuation(continuation).await?;
        self.fill_durations(&mut page).await;
        Ok(page)
    }

    /// The explore page (new releases, charts, moods teasers).
    pub async fn explore(&self) -> Result<ShelvesPage> {
        Ok(parsers::shelves_page(&self.browse(EXPLORE, None, Login::IfSignedIn).await?))
    }

    /// New albums and singles.
    pub async fn new_releases(&self) -> Result<Vec<AlbumItem>> {
        let page = parsers::shelves_page(&self.browse(NEW_RELEASES, None, Login::IfSignedIn).await?);
        let mut seen = std::collections::HashSet::new();
        Ok(page
            .shelves
            .into_iter()
            .flat_map(|shelf| shelf.items)
            .filter_map(|item| match item {
                YtItem::Album(album) => Some(album),
                _ => None,
            })
            .filter(|album| seen.insert(album.album_id.clone()))
            .collect())
    }

    /// Charts of a country (ISO code), or global ones. Always anonymous, as the reference client asks.
    pub async fn charts(&self, country: Option<&str>) -> Result<ShelvesPage> {
        let region = country.filter(|code| !code.trim().is_empty()).unwrap_or("ZZ");
        let request = |params: Option<&str>| {
            let mut body = json!({ "browseId": CHARTS, "formData": { "selectedValues": [region] } });
            if let Some(params) = params {
                body["params"] = json!(params);
            }
            body
        };
        let response = self.client.post(&WEB_REMIX, "browse", request(None), Login::Never).await?;
        let page = parsers::shelves_page(&response);
        if !page.shelves.is_empty() {
            return Ok(page);
        }
        let response =
            self.client.post(&WEB_REMIX, "browse", request(Some(CHARTS_FALLBACK_PARAMS)), Login::Never).await?;
        Ok(parsers::shelves_page(&response))
    }

    /// Mood and genre tiles.
    pub async fn moods(&self) -> Result<Vec<Mood>> {
        Ok(parsers::moods(&self.browse(MOODS, None, Login::IfSignedIn).await?))
    }

    /// Any page made of shelves: a mood's playlists, a "see all", an artist's "more", related items.
    pub async fn browse_shelves(&self, target: &BrowseRef) -> Result<ShelvesPage> {
        let response = self.browse(&target.browse_id, target.params.as_deref(), Login::IfSignedIn).await?;
        Ok(parsers::shelves_page(&response))
    }

    /// More shelves of a page; an empty page has no continuation.
    pub async fn shelves_continuation(&self, continuation: &str) -> Result<ShelvesPage> {
        let mut page = parsers::shelves_page(&self.browse_continuation(continuation, Login::IfSignedIn).await?);
        if page.shelves.is_empty() {
            page.continuation = None;
        }
        Ok(page)
    }
}
