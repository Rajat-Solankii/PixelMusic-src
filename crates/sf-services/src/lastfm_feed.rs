//! Last.fm feed and Smart Mix — port of soundwave1 `data/feed/FeedLastFmRepository.kt` and
//! `presentation/viewmodel/SmartMixViewModel.kt` (fetchMix, fetchRecommendations, the mode fetchers,
//! resolveTrackToYoutubeSong, generatePlaylistName).
//!
//! Read calls only: GET ws.audioscrobbler.com/2.0/?method&api_key&format=json with the user's own API key and the
//! connected username from `LastFmService` (no signature, no session key). The feed is recent tracks
//! (user.getrecenttracks, the "now playing" row dropped), friends (user.getfriends) and top albums
//! (user.gettopalbums). Smart Mix builds a list of Last.fm tracks for one of eight modes, then resolves each one to a
//! YouTube Music or Spotify track through that catalog's search and the metadata gate of `sources/matching.rs`
//! (the Kotlin code took the first YouTube result unchecked). Every random choice (pages, shuffles, picks) goes
//! through one `fastrand::Rng`, seeded in tests.
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use futures::StreamExt;
use futures::future::BoxFuture;
use md3_model::{AlbumItem, PlayContext, TrackItem, contexts};
use serde::Serialize;
use serde_json::{Map, Value};
use sf_player::PlayerHandle;
use tokio::runtime::Handle;
use tokio::sync::watch;

use crate::lastfm::{API_URL, LastFmError, form_encode};
use crate::sources::matching::{self, Candidate};
use crate::youtube::models::{SearchFilter, YtItem};
use crate::{CatalogService, SearchKind, YouTubeService, lock};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const USER_AGENT: &str = concat!("Materialfy/", env!("CARGO_PKG_VERSION"));
/// Last.fm's grey star placeholder ("no artwork").
const NO_ART_HASH: &str = "2a96cbd8b46e442fc41c2b86b821562f";
/// Searches running at once while resolving a mix (the Kotlin Semaphore(3)).
const RESOLVE_PARALLEL: usize = 3;
/// Cover lookups (feed tiles without Last.fm art) running at once.
const COVER_PARALLEL: usize = 3;
pub const FEED_RECENT: u32 = 30;
pub const FEED_FRIENDS: u32 = 20;
pub const FEED_TOP_ALBUMS: u32 = 20;
/// The periods user.gettoptracks accepts, in the order the page shows them.
pub const PERIODS: [&str; 6] = ["overall", "12month", "6month", "3month", "1month", "7day"];

fn is_guest(username: &str) -> bool {
    username.trim().is_empty() || username.trim().eq_ignore_ascii_case("Guest User")
}

// --- JSON helpers (FeedLastFmRepository.kt) ---------------------------------------------------------------------

/// A single object or an array of them (Last.fm drops the array for one item).
fn objects(value: Option<&Value>) -> Vec<&Map<String, Value>> {
    match value {
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_object).collect(),
        Some(Value::Object(object)) => vec![object],
        _ => Vec::new(),
    }
}

fn text<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

fn number(object: &Map<String, Value>, key: &str) -> Option<i64> {
    match object.get(key)? {
        Value::String(text) => text.trim().parse().ok(),
        value => value.as_i64(),
    }
}

fn is_real_image(url: &str) -> bool {
    !url.trim().is_empty() && !url.contains(NO_ART_HASH)
}

/// "artist": a string or {name | #text}.
fn artist_name(object: &Map<String, Value>) -> String {
    match object.get("artist") {
        Some(Value::String(name)) => name.trim().to_owned(),
        Some(Value::Object(artist)) => text(artist, "name")
            .filter(|name| !name.trim().is_empty())
            .or_else(|| text(artist, "#text"))
            .unwrap_or_default()
            .trim()
            .to_owned(),
        _ => String::new(),
    }
}

/// "album": a string or {#text | title}.
fn album_name(object: &Map<String, Value>) -> String {
    match object.get("album") {
        Some(Value::String(name)) => name.trim().to_owned(),
        Some(Value::Object(album)) => text(album, "#text")
            .filter(|name| !name.trim().is_empty())
            .or_else(|| text(album, "title"))
            .unwrap_or_default()
            .trim()
            .to_owned(),
        _ => String::new(),
    }
}

fn images(object: &Map<String, Value>) -> Vec<(&str, &str)> {
    objects(object.get("image")).into_iter().map(|image| (text(image, "#text").unwrap_or_default(), text(image, "size").unwrap_or_default())).collect()
}

/// extralarge → large → medium → the first real one ("" — none).
fn best_image(object: &Map<String, Value>) -> String {
    let images = images(object);
    ["extralarge", "large", "medium"]
        .iter()
        .find_map(|size| images.iter().find(|(url, s)| s == size && is_real_image(url)))
        .or_else(|| images.iter().find(|(url, _)| is_real_image(url)))
        .map(|(url, _)| (*url).to_owned())
        .unwrap_or_default()
}

/// The last real image (the largest: Last.fm lists them small → large).
fn last_image(object: &Map<String, Value>) -> String {
    images(object).into_iter().rev().find(|(url, _)| is_real_image(url)).map(|(url, _)| url.to_owned()).unwrap_or_default()
}

// --- feed items -------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentTrack {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub cover: String,
    pub url: String,
    /// unix seconds of the scrobble (0 — unknown)
    pub uts: i64,
    /// Last.fm's own date text ("12 Nov 2026, 09:14")
    pub date_text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Friend {
    pub name: String,
    /// the real name, else the username
    pub display_name: String,
    pub real_name: String,
    pub avatar: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopAlbum {
    pub title: String,
    pub artist: String,
    pub cover: String,
    pub play_count: i64,
}

/// user.getrecenttracks without the "now playing" row.
pub fn parse_recent(root: &Value) -> Vec<RecentTrack> {
    objects(root.get("recenttracks").and_then(|recent| recent.get("track")))
        .into_iter()
        .filter(|track| track.get("@attr").and_then(|attr| attr.get("nowplaying")).is_none())
        .filter_map(|track| {
            let title = text(track, "name").map(str::trim).filter(|name| !name.is_empty())?.to_owned();
            let artist = artist_name(track);
            if artist.is_empty() {
                return None;
            }
            let date = track.get("date").and_then(Value::as_object);
            Some(RecentTrack {
                title,
                artist,
                album: album_name(track),
                cover: best_image(track),
                url: text(track, "url").unwrap_or_default().to_owned(),
                uts: date.and_then(|date| number(date, "uts")).unwrap_or(0),
                date_text: date.and_then(|date| text(date, "#text")).unwrap_or_default().to_owned(),
            })
        })
        .collect()
}

pub fn parse_friends(root: &Value) -> Vec<Friend> {
    objects(root.get("friends").and_then(|friends| friends.get("user")))
        .into_iter()
        .filter_map(|user| {
            let name = text(user, "name").map(str::trim).filter(|name| !name.is_empty())?.to_owned();
            let real_name = text(user, "realname").unwrap_or_default().trim().to_owned();
            let display_name = if real_name.is_empty() { name.clone() } else { real_name.clone() };
            Some(Friend { name, display_name, real_name, avatar: last_image(user) })
        })
        .collect()
}

pub fn parse_top_albums(root: &Value) -> Vec<TopAlbum> {
    objects(root.get("topalbums").and_then(|albums| albums.get("album")))
        .into_iter()
        .filter_map(|album| {
            let title = text(album, "name").map(str::trim).filter(|name| !name.is_empty())?.to_owned();
            let artist = artist_name(album);
            if artist.is_empty() || artist.eq_ignore_ascii_case("Unknown artist") {
                return None;
            }
            Some(TopAlbum { title, artist, cover: last_image(album), play_count: number(album, "playcount").unwrap_or(0) })
        })
        .collect()
}

// --- Smart Mix parsers (SmartMixViewModel.kt) -------------------------------------------------------------------

/// A track as Last.fm names it, before it is found in a catalog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LfmTrack {
    pub name: String,
    pub artist: String,
    pub image: String,
}

impl LfmTrack {
    /// "name|artist" lower case: the dedupe key.
    fn key(&self) -> String {
        format!("{}|{}", self.name, self.artist).to_lowercase()
    }
}

/// Tracks of toptracks, recenttracks, similartracks, tracks (tag.gettoptracks) or results.trackmatches.
pub fn parse_tracks(root: &Value) -> Vec<LfmTrack> {
    let list = ["toptracks", "recenttracks", "similartracks", "tracks"]
        .iter()
        .find_map(|key| root.get(*key))
        .and_then(|found| found.get("track"))
        .or_else(|| root.get("results").and_then(|r| r.get("trackmatches")).and_then(|m| m.get("track")));
    objects(list)
        .into_iter()
        .filter_map(|track| {
            let name = text(track, "name").map(str::trim).filter(|name| !name.is_empty())?.to_owned();
            let artist = artist_name(track);
            (!artist.is_empty()).then(|| LfmTrack { name, artist, image: best_image(track) })
        })
        .collect()
}

fn names(list: Option<&Value>) -> Vec<String> {
    objects(list)
        .into_iter()
        .filter_map(|item| text(item, "name").map(str::trim).filter(|name| !name.is_empty()).map(str::to_owned))
        .collect()
}

/// artist.getsimilar (similarartists) or artist.search (results.artistmatches).
pub fn parse_artists(root: &Value) -> Vec<String> {
    let list = root
        .get("similarartists")
        .and_then(|s| s.get("artist"))
        .or_else(|| root.get("results").and_then(|r| r.get("artistmatches")).and_then(|m| m.get("artist")));
    names(list)
}

/// user.gettopartists
pub fn parse_top_artists(root: &Value) -> Vec<String> {
    names(root.get("topartists").and_then(|top| top.get("artist")))
}

/// user.gettoptags
pub fn parse_tags(root: &Value) -> Vec<String> {
    names(root.get("toptags").and_then(|top| top.get("tag")))
}

// --- modes and naming -------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MixMode {
    /// user.gettoptracks for the chosen period
    #[default]
    Top,
    /// top tracks of all time
    Library,
    Recent,
    /// track.getsimilar of a seed track
    SimilarTracks,
    /// artist.getsimilar → artist.gettoptracks of a seed artist
    SimilarArtists,
    /// tag.gettoptracks
    Tag,
    /// weighted blend of top, recent, similar and tags
    Mix,
    /// scored "fresh for you": similar to what you play, minus what you heard lately
    Recommendations,
}

impl MixMode {
    pub const ALL: [MixMode; 8] = [
        Self::Top,
        Self::Recent,
        Self::SimilarTracks,
        Self::SimilarArtists,
        Self::Tag,
        Self::Mix,
        Self::Recommendations,
        Self::Library,
    ];

    /// The Kotlin mode ids ("top", "similar-tracks" …), also used by QML.
    pub fn key(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Library => "library",
            Self::Recent => "recent",
            Self::SimilarTracks => "similar-tracks",
            Self::SimilarArtists => "similar-artists",
            Self::Tag => "tag",
            Self::Mix => "mix",
            Self::Recommendations => "recommendations",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == key.trim())
    }
}

/// What to generate (SmartMixUiState's inputs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MixRequest {
    pub mode: MixMode,
    /// tracks wanted
    pub count: usize,
    /// one of PERIODS (Top only)
    pub period: String,
    pub seed_track: String,
    pub seed_artist: String,
    /// SimilarArtists' seed
    pub seed_artist_input: String,
    pub tag: String,
}

impl Default for MixRequest {
    fn default() -> Self {
        Self {
            mode: MixMode::Top,
            count: 25,
            period: "overall".to_owned(),
            seed_track: String::new(),
            seed_artist: String::new(),
            seed_artist_input: String::new(),
            tag: String::new(),
        }
    }
}

impl MixRequest {
    /// The seed the mode needs is missing (the Kotlin IllegalArgumentException texts).
    pub fn missing_input(&self) -> Option<String> {
        match self.mode {
            MixMode::SimilarTracks if self.seed_track.trim().is_empty() || self.seed_artist.trim().is_empty() => {
                Some(mf_i18n::t("Enter a seed track and artist"))
            }
            MixMode::SimilarArtists if self.seed_artist_input.trim().is_empty() => Some(mf_i18n::t("Enter a seed artist")),
            MixMode::Tag if self.tag.trim().is_empty() => Some(mf_i18n::t("Enter a genre or tag")),
            _ => None,
        }
    }
}

/// Letters and digits only, at most 9 characters, first one upper case ("the weeknd" → "Theweeknd").
fn name_part(raw: &str) -> Option<String> {
    let clean: String = raw.chars().filter(char::is_ascii_alphanumeric).take(9).collect();
    let mut chars = clean.chars();
    let first = chars.next()?;
    Some(first.to_ascii_uppercase().to_string() + chars.as_str())
}

/// generatePlaylistName: "{Artist}Era", "{Track}Echo", "{Tag}Hood" …
pub fn playlist_name(request: &MixRequest, tracks: &[LfmTrack]) -> String {
    let first = tracks.first();
    let pick = |values: &[Option<&str>]| values.iter().flatten().find(|value| !value.trim().is_empty()).and_then(|v| name_part(v));
    let artist = pick(&[first.map(|t| t.artist.as_str()), Some(&request.seed_artist_input), Some(&request.seed_artist)]);
    let track = pick(&[Some(&request.seed_track), first.map(|t| t.name.as_str())]);
    let tag = pick(&[Some(&request.tag)]);
    let or = |part: &Option<String>, suffix: &str, fallback: &str| match part {
        Some(part) => format!("{part}{suffix}"),
        None => fallback.to_owned(),
    };
    match request.mode {
        MixMode::Top => {
            let prefix = artist.unwrap_or_else(|| "Sonic".to_owned());
            let suffix = match request.period.as_str() {
                "overall" => "Era",
                "12month" => "Spectrum",
                "6month" => "Pulse",
                "3month" => "Drift",
                "1month" => "Wave",
                "7day" => "Flow",
                _ => "Sphere",
            };
            format!("{prefix}{suffix}")
        }
        MixMode::Library => or(&artist, "Vault", "VaultCraft"),
        MixMode::Recent => or(&artist, "Rotation", "HeavyRotation"),
        MixMode::SimilarTracks => format!("{}Echo", track.or(artist).unwrap_or_else(|| "Track".to_owned())),
        MixMode::SimilarArtists => format!("{}Radio", artist.unwrap_or_else(|| "Artist".to_owned())),
        MixMode::Tag => format!("{}Hood", tag.or(artist).unwrap_or_else(|| "Genre".to_owned())),
        MixMode::Mix => or(&artist, "Algo", "AlgorhythmFlow"),
        MixMode::Recommendations => or(&artist, "Mix", "AstroPulse"),
    }
}

/// "Name", then "Name #2", "Name #3" … until no playlist has it (case ignored), so older mixes are kept.
pub fn unique_name(base: &str, existing: &[String]) -> String {
    let taken = |name: &str| existing.iter().any(|other| other.eq_ignore_ascii_case(name));
    let mut name = base.to_owned();
    let mut n = 1;
    while taken(&name) {
        n += 1;
        name = format!("{base} #{n}");
    }
    name
}

// --- weighting --------------------------------------------------------------------------------------------------

fn shuffled<T>(mut items: Vec<T>, rng: &mut fastrand::Rng) -> Vec<T> {
    rng.shuffle(&mut items);
    items
}

/// One entry per key, the highest weight kept (first seen wins a tie), in first-seen order.
fn dedupe(weighted: Vec<(LfmTrack, u8)>) -> Vec<(LfmTrack, u8)> {
    let mut at: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<(LfmTrack, u8)> = Vec::new();
    for (track, weight) in weighted {
        match at.get(&track.key()) {
            Some(&i) if out[i].1 >= weight => {}
            Some(&i) => out[i] = (track, weight),
            None => {
                at.insert(track.key(), out.len());
                out.push((track, weight));
            }
        }
    }
    out
}

/// At most `cap` tracks per artist, order kept.
fn cap_artists(tracks: Vec<LfmTrack>, cap: usize) -> Vec<LfmTrack> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    tracks
        .into_iter()
        .filter(|track| {
            let count = seen.entry(track.artist.to_lowercase()).or_default();
            *count += 1;
            *count <= cap
        })
        .collect()
}

/// fetchMix's merge: dedupe, tiers 4 → 1 each shuffled, two tracks per artist, `total` at most.
pub fn merge_tiers(weighted: Vec<(LfmTrack, u8)>, total: usize, rng: &mut fastrand::Rng) -> Vec<LfmTrack> {
    let deduped = dedupe(weighted);
    let mut merged = Vec::new();
    for tier in [4u8, 3, 2, 1] {
        let level: Vec<LfmTrack> = deduped.iter().filter(|(_, w)| *w == tier).map(|(t, _)| t.clone()).collect();
        merged.extend(shuffled(level, rng));
    }
    cap_artists(merged, 2).into_iter().take(total).collect()
}

/// The listener's taste, for Recommendations' scoring.
#[derive(Clone, Debug, Default)]
pub struct Taste {
    pub top_tracks: HashSet<String>,
    pub recent_tracks: HashSet<String>,
    pub top_artists: HashSet<String>,
    pub recent_artists: HashSet<String>,
}

/// scoreTrack: -1 rejects a track heard lately; bucket 4/3/2 → 40/30/20, a top artist +50, a recent artist +30,
/// an all-time favourite −15, never below 0.
pub fn score(track: &LfmTrack, weight: u8, taste: &Taste) -> i32 {
    let key = track.key();
    if taste.recent_tracks.contains(&key) {
        return -1;
    }
    let artist = track.artist.to_lowercase();
    let mut score = match weight {
        4 => 40,
        3 => 30,
        2 => 20,
        _ => 0,
    };
    if taste.top_artists.contains(&artist) {
        score += 50;
    }
    if taste.recent_artists.contains(&artist) {
        score += 30;
    }
    if taste.top_tracks.contains(&key) {
        score -= 15;
    }
    score.max(0)
}

/// fetchRecommendations' selection: score, keep ≥ 30 (≥ 10 when thin, then any non-rejected), best first,
/// 60 % familiar (bucket ≥ 3) / 40 % discovery, topped up, two tracks per artist.
pub fn pick_recommendations(weighted: Vec<(LfmTrack, u8)>, taste: &Taste, total: usize) -> Vec<LfmTrack> {
    let scored: Vec<(LfmTrack, u8, i32)> =
        dedupe(weighted).into_iter().map(|(track, weight)| { let s = score(&track, weight, taste); (track, weight, s) }).collect();
    let at_least = |min: i32| scored.iter().filter(|(_, _, s)| *s >= min).cloned().collect::<Vec<_>>();
    let mut filtered = at_least(30);
    if filtered.len() < total * 65 / 100 {
        filtered = at_least(10);
    }
    if filtered.len() < total / 2 {
        let keys: HashSet<String> = filtered.iter().map(|(t, _, _)| t.key()).collect();
        let mut extra: Vec<_> = scored.iter().filter(|(t, _, s)| *s != -1 && !keys.contains(&t.key())).cloned().collect();
        extra.sort_by(|a, b| b.2.cmp(&a.2));
        filtered.extend(extra);
    }
    // stable: equal scores keep their order
    filtered.sort_by(|a, b| b.2.cmp(&a.2));
    let familiar = filtered.iter().filter(|(_, w, _)| *w >= 3).take(total * 60 / 100);
    let discovery = filtered.iter().filter(|(_, w, _)| *w <= 2).take(total * 40 / 100);
    let mut balanced: Vec<LfmTrack> = familiar.chain(discovery).map(|(t, _, _)| t.clone()).collect();
    if balanced.len() < total {
        let keys: HashSet<String> = balanced.iter().map(LfmTrack::key).collect();
        balanced.extend(filtered.iter().filter(|(t, _, _)| !keys.contains(&t.key())).map(|(t, _, _)| t.clone()));
    }
    cap_artists(balanced, 2).into_iter().take(total).collect()
}

// --- the API ----------------------------------------------------------------------------------------------------

fn pair(name: &str, value: impl ToString) -> (String, String) {
    (name.to_owned(), value.to_string())
}

/// Unsigned read calls with one API key.
#[derive(Clone)]
pub struct FeedApi {
    http: sf_api::Http,
    url: String,
    key: String,
}

impl FeedApi {
    pub fn new(http: sf_api::Http, url: &str, key: &str) -> Self {
        Self { http, url: url.to_owned(), key: key.to_owned() }
    }

    pub async fn get(&self, method: &str, extra: Vec<(String, String)>) -> Result<Value, LastFmError> {
        if self.key.trim().is_empty() {
            return Err(LastFmError::NoKeys);
        }
        let mut params = vec![pair("method", method), pair("api_key", &self.key), pair("format", "json")];
        params.extend(extra);
        let client = self.http.client().map_err(|_| LastFmError::Network)?;
        let url = format!("{}?{}", self.url, form_encode(&params));
        let response = client
            .get(&url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|error| {
                // without the URL: it carries the API key
                tracing::warn!("Last.fm {method}: {}", error.without_url());
                LastFmError::Network
            })?;
        let text = response.text().await.map_err(|_| LastFmError::Network)?;
        let root: Value = serde_json::from_str(&text).map_err(|_| LastFmError::Unreadable)?;
        if let Some(code) = root.get("error").and_then(Value::as_i64) {
            let message = root.get("message").and_then(Value::as_str).unwrap_or_default().to_owned();
            return Err(LastFmError::Api { code, message });
        }
        if root.is_object() { Ok(root) } else { Err(LastFmError::Unreadable) }
    }

    /// A failed call counts as an empty answer (the Kotlin getOrNull in the blended modes).
    async fn get_or_empty(&self, method: &str, extra: Vec<(String, String)>) -> Value {
        self.get(method, extra).await.unwrap_or_else(|error| {
            tracing::debug!("Last.fm {method}: {error}");
            Value::Null
        })
    }

    pub async fn recent_tracks(&self, user: &str, limit: u32) -> Result<Vec<RecentTrack>, LastFmError> {
        if is_guest(user) {
            return Ok(Vec::new());
        }
        let root = self.get("user.getrecenttracks", vec![pair("user", user), pair("limit", limit)]).await?;
        Ok(parse_recent(&root))
    }

    pub async fn friends(&self, user: &str, limit: u32) -> Result<Vec<Friend>, LastFmError> {
        if is_guest(user) {
            return Ok(Vec::new());
        }
        let root = self.get("user.getfriends", vec![pair("user", user), pair("limit", limit)]).await?;
        Ok(parse_friends(&root))
    }

    pub async fn top_albums(&self, user: &str, limit: u32, period: &str) -> Result<Vec<TopAlbum>, LastFmError> {
        if is_guest(user) {
            return Ok(Vec::new());
        }
        let params = vec![pair("user", user), pair("limit", limit), pair("period", period)];
        Ok(parse_top_albums(&self.get("user.gettopalbums", params).await?))
    }

    /// The Last.fm tracks of one mode (before resolving): what generatePlaylist's `when (mode)` fetches.
    pub async fn mix_tracks(
        &self,
        user: &str,
        request: &MixRequest,
        rng: &mut fastrand::Rng,
    ) -> Result<Vec<LfmTrack>, LastFmError> {
        let limit = request.count.max(1);
        let tracks = match request.mode {
            MixMode::Top => self.top_tracks(user, limit, &request.period, rng).await?,
            MixMode::Library => self.top_tracks(user, limit, "overall", rng).await?,
            MixMode::Recent => {
                let params = vec![pair("user", user), pair("limit", (limit * 3).min(100))];
                parse_tracks(&self.get("user.getrecenttracks", params).await?)
            }
            MixMode::SimilarTracks => {
                let params = vec![
                    pair("track", request.seed_track.trim()),
                    pair("artist", request.seed_artist.trim()),
                    pair("limit", (limit * 4).min(200)),
                ];
                shuffled(parse_tracks(&self.get("track.getsimilar", params).await?), rng)
            }
            MixMode::SimilarArtists => self.similar_artist_tracks(request.seed_artist_input.trim(), limit, rng).await?,
            MixMode::Tag => {
                let page = rng.u32(1..=8);
                let params = vec![pair("tag", request.tag.trim()), pair("limit", (limit * 3).min(100)), pair("page", page)];
                shuffled(parse_tracks(&self.get("tag.gettoptracks", params).await?), rng)
            }
            MixMode::Mix => self.blend(user, limit, rng).await,
            MixMode::Recommendations => self.recommendations(user, limit, rng).await,
        };
        Ok(tracks)
    }

    async fn top_tracks(&self, user: &str, limit: usize, period: &str, rng: &mut fastrand::Rng) -> Result<Vec<LfmTrack>, LastFmError> {
        let page = rng.u32(1..=3);
        let params =
            vec![pair("user", user), pair("limit", (limit * 3).min(100)), pair("period", period), pair("page", page)];
        Ok(shuffled(parse_tracks(&self.get("user.gettoptracks", params).await?), rng))
    }

    async fn similar_artist_tracks(&self, artist: &str, limit: usize, rng: &mut fastrand::Rng) -> Result<Vec<LfmTrack>, LastFmError> {
        let root = self.get("artist.getsimilar", vec![pair("artist", artist), pair("limit", 20)]).await?;
        let artists: Vec<String> = shuffled(parse_artists(&root), rng).into_iter().take(8).collect();
        let mut list = Vec::new();
        for artist in artists {
            let params = vec![pair("artist", &artist), pair("limit", (limit / 5).max(4)), pair("page", rng.u32(1..=4))];
            // one artist failing doesn't stop the rest
            list.extend(parse_tracks(&self.get_or_empty("artist.gettoptracks", params).await));
        }
        Ok(shuffled(list, rng))
    }

    /// fetchMix: four buckets run together, each with its own fork of `rng` (so the result stays reproducible).
    async fn blend(&self, user: &str, total: usize, rng: &mut fastrand::Rng) -> Vec<LfmTrack> {
        let (mut ra, mut rb, mut rc, mut rd) = (rng.fork(), rng.fork(), rng.fork(), rng.fork());
        // A (weight 4): top tracks over three random periods
        let a = async {
            let mut out = Vec::new();
            let periods: Vec<&str> = shuffled(vec!["7day", "1month", "3month", "6month", "12month", "overall"], &mut ra);
            for period in periods.into_iter().take(3) {
                let params = vec![pair("user", user), pair("limit", 50), pair("period", period), pair("page", ra.u32(1..=3))];
                let tracks = parse_tracks(&self.get_or_empty("user.gettoptracks", params).await);
                out.extend(shuffled(tracks, &mut ra).into_iter().map(|t| (t, 4)));
            }
            out
        };
        // B (weight 3): eight recent plays, and tracks similar to five of them
        let b = async {
            let mut out = Vec::new();
            let recent = parse_tracks(&self.get_or_empty("user.getrecenttracks", vec![pair("user", user), pair("limit", 50)]).await);
            let seeds: Vec<LfmTrack> = shuffled(recent, &mut rb).into_iter().take(8).collect();
            out.extend(seeds.iter().cloned().map(|t| (t, 3)));
            for seed in seeds.iter().take(5) {
                let params = vec![pair("track", &seed.name), pair("artist", &seed.artist), pair("limit", 30)];
                let tracks = parse_tracks(&self.get_or_empty("track.getsimilar", params).await);
                out.extend(shuffled(tracks, &mut rb).into_iter().map(|t| (t, 3)));
            }
            out
        };
        // C (weight 2): top tracks of artists similar to five top artists
        let c = async {
            let mut out = Vec::new();
            let period = ["overall", "12month", "6month", "3month"][rc.usize(0..4)];
            let params = vec![pair("user", user), pair("limit", 40), pair("period", period)];
            let top = parse_top_artists(&self.get_or_empty("user.gettopartists", params).await);
            for artist in shuffled(top, &mut rc).into_iter().take(5) {
                let similar = parse_artists(&self.get_or_empty("artist.getsimilar", vec![pair("artist", &artist), pair("limit", 15)]).await);
                for other in shuffled(similar, &mut rc).into_iter().take(3) {
                    let params = vec![pair("artist", &other), pair("limit", 12), pair("page", rc.u32(1..=6))];
                    let tracks = parse_tracks(&self.get_or_empty("artist.gettoptracks", params).await);
                    out.extend(shuffled(tracks, &mut rc).into_iter().map(|t| (t, 2)));
                }
            }
            out
        };
        // D (weight 1): four of the listener's top tags
        let d = async {
            let mut out = Vec::new();
            let tags = parse_tags(&self.get_or_empty("user.gettoptags", vec![pair("user", user), pair("limit", 12)]).await);
            for tag in shuffled(tags, &mut rd).into_iter().take(4) {
                let params = vec![pair("tag", &tag), pair("limit", 25), pair("page", rd.u32(1..=10))];
                let tracks = parse_tracks(&self.get_or_empty("tag.gettoptracks", params).await);
                out.extend(shuffled(tracks, &mut rd).into_iter().map(|t| (t, 1)));
            }
            out
        };
        let (a, b, c, d) = futures::join!(a, b, c, d);
        merge_tiers([a, b, c, d].concat(), total, rng)
    }

    /// fetchRecommendations: a taste profile, then four candidate buckets, then scoring.
    async fn recommendations(&self, user: &str, total: usize, rng: &mut fastrand::Rng) -> Vec<LfmTrack> {
        let (top, recent, artists, tags) = futures::join!(
            self.get_or_empty("user.gettoptracks", vec![pair("user", user), pair("limit", 50), pair("period", "overall")]),
            self.get_or_empty("user.getrecenttracks", vec![pair("user", user), pair("limit", 50)]),
            self.get_or_empty("user.gettopartists", vec![pair("user", user), pair("limit", 30), pair("period", "overall")]),
            self.get_or_empty("user.gettoptags", vec![pair("user", user), pair("limit", 15)]),
        );
        let (top, recent) = (parse_tracks(&top), parse_tracks(&recent));
        let mut taste = Taste::default();
        taste.top_tracks.extend(top.iter().map(LfmTrack::key));
        taste.recent_tracks.extend(recent.iter().map(LfmTrack::key));
        taste.recent_artists.extend(recent.iter().map(|t| t.artist.to_lowercase()));
        // in first-seen order (the Kotlin set was a LinkedHashSet)
        let mut top_artists: Vec<String> = Vec::new();
        for artist in parse_top_artists(&artists) {
            if taste.top_artists.insert(artist.to_lowercase()) {
                top_artists.push(artist.to_lowercase());
            }
        }
        let tags: Vec<String> = parse_tags(&tags).into_iter().map(|t| t.to_lowercase()).collect();

        let (mut ra, mut rb, mut rc, mut rd) = (rng.fork(), rng.fork(), rng.fork(), rng.fork());
        let similar_to = |seeds: Vec<LfmTrack>, limit: u32, weight: u8| async move {
            let mut out = Vec::new();
            for seed in seeds {
                let params = vec![pair("track", &seed.name), pair("artist", &seed.artist), pair("limit", limit)];
                out.extend(parse_tracks(&self.get_or_empty("track.getsimilar", params).await).into_iter().map(|t| (t, weight)));
            }
            out
        };
        // A (4): similar to six recent plays; B (3): similar to five favourites
        let a = similar_to(shuffled(recent, &mut ra).into_iter().take(6).collect(), 30, 4);
        let b = similar_to(shuffled(top, &mut rb).into_iter().take(5).collect(), 25, 3);
        // C (2): top tracks of artists similar to six top artists
        let c = async {
            let mut out = Vec::new();
            for artist in shuffled(top_artists, &mut rc).into_iter().take(6) {
                let similar = parse_artists(&self.get_or_empty("artist.getsimilar", vec![pair("artist", &artist), pair("limit", 15)]).await);
                for other in shuffled(similar, &mut rc).into_iter().take(4) {
                    let params = vec![pair("artist", &other), pair("limit", 10), pair("page", rc.u32(1..=4))];
                    out.extend(parse_tracks(&self.get_or_empty("artist.gettoptracks", params).await).into_iter().map(|t| (t, 2)));
                }
            }
            out
        };
        // D (1): four top tags
        let d = async {
            let mut out = Vec::new();
            let limit = (total * 35 / 100).max(5);
            for tag in shuffled(tags, &mut rd).into_iter().take(4) {
                let params = vec![pair("tag", &tag), pair("limit", limit), pair("page", rd.u32(1..=8))];
                out.extend(parse_tracks(&self.get_or_empty("tag.gettoptracks", params).await).into_iter().map(|t| (t, 1)));
            }
            out
        };
        let (a, b, c, d) = futures::join!(a, b, c, d);
        pick_recommendations([a, b, c, d].concat(), &taste, total)
    }
}

// --- resolving to a catalog -------------------------------------------------------------------------------------

/// A catalog whose tracks a mix can be made of.
pub trait TrackSearch: Send + Sync {
    fn search_tracks<'a>(&'a self, query: &'a str) -> BoxFuture<'a, Vec<TrackItem>>;
}

impl TrackSearch for YouTubeService {
    fn search_tracks<'a>(&'a self, query: &'a str) -> BoxFuture<'a, Vec<TrackItem>> {
        Box::pin(async move {
            self.search_songs(query).await.unwrap_or_else(|error| {
                tracing::debug!("YouTube search for a mix track failed: {error}");
                Vec::new()
            })
        })
    }
}

impl TrackSearch for CatalogService {
    fn search_tracks<'a>(&'a self, query: &'a str) -> BoxFuture<'a, Vec<TrackItem>> {
        Box::pin(async move {
            match self.search(query, SearchKind::Tracks).await {
                Ok(results) => results.tracks,
                Err(error) => {
                    tracing::debug!("Spotify search for a mix track failed: {error}");
                    Vec::new()
                }
            }
        })
    }
}

/// The search result that is this Last.fm track (the best score the matching gate accepts), if any.
pub fn best_match(track: &LfmTrack, results: Vec<TrackItem>) -> Option<TrackItem> {
    let wanted = [track.artist.clone()];
    results
        .into_iter()
        .filter(|item| item.available && !item.is_episode)
        .filter_map(|item| {
            let artist = item.artist_refs.first().map(|a| a.name.clone()).or_else(|| (!item.artists.is_empty()).then(|| item.artists.clone()));
            let album = (!item.album.is_empty()).then(|| item.album.clone());
            let candidate = Candidate { title: item.title.clone(), artist, album, duration_ms: (item.duration_ms > 0).then_some(item.duration_ms) };
            let verdict = matching::evaluate(&track.name, &wanted, None, None, &candidate);
            verdict.accepted.then_some((verdict.score, item))
        })
        // max_by keeps the last of equal scores: reverse so the catalog's first wins
        .rev()
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, item)| item)
}

async fn search_one(search: &dyn TrackSearch, track: LfmTrack) -> Option<TrackItem> {
    let query = format!("{} {}", track.name, track.artist);
    best_match(&track, search.search_tracks(&query).await)
}

/// Resolves in order, a few searches at once, until `count` distinct tracks are found. `progress(found)` after each.
pub async fn resolve(
    search: &dyn TrackSearch,
    tracks: &[LfmTrack],
    count: usize,
    mut progress: impl FnMut(usize),
) -> Vec<TrackItem> {
    let mut found: Vec<TrackItem> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    // owned items: a closure over borrowed ones trips the higher-ranked lifetime check inside tokio::spawn
    let mut searches =
        futures::stream::iter(tracks.to_vec()).map(|track| search_one(search, track)).buffered(RESOLVE_PARALLEL);
    while let Some(item) = searches.next().await {
        if let Some(item) = item
            && seen.insert(item.play_uri())
        {
            found.push(item);
            progress(found.len());
            if found.len() >= count {
                break;
            }
        }
    }
    found
}

// --- covers for feed tiles -------------------------------------------------------------------------------------

/// Last.fm often has no art (an empty URL or its grey star): the feed asks a catalog for the cover instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CoverKind {
    Track,
    Album,
}

impl CoverKind {
    pub fn from_key(key: &str) -> Self {
        if key.trim().eq_ignore_ascii_case("album") { Self::Album } else { Self::Track }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Track => "track",
            Self::Album => "album",
        }
    }
}

/// The key of a looked-up cover in `LastFmFeedSnapshot::covers` (QML builds the same string).
pub fn cover_key(kind: CoverKind, title: &str, artist: &str) -> String {
    format!("{}|{}|{}", kind.key(), artist.trim(), title.trim()).to_lowercase()
}

/// Finds the cover of a song or an album by its title and artist.
pub trait CoverSearch: Send + Sync {
    fn find_cover<'a>(&'a self, kind: CoverKind, title: &'a str, artist: &'a str) -> BoxFuture<'a, Option<String>>;
}

/// The large cover, else the small one (None — neither).
fn cover_of(large: &str, small: &str) -> Option<String> {
    [large, small].into_iter().find(|url| !url.trim().is_empty()).map(str::to_owned)
}

/// The album result that is this album (the matching gate over title and first artist).
pub fn best_album(title: &str, artist: &str, albums: Vec<AlbumItem>) -> Option<AlbumItem> {
    let wanted = [artist.to_owned()];
    albums
        .into_iter()
        .filter_map(|album| {
            let first = album.artist_refs.first().map(|a| a.name.clone()).or_else(|| (!album.artists.is_empty()).then(|| album.artists.clone()));
            let candidate = Candidate { title: album.title.clone(), artist: first, album: None, duration_ms: None };
            let verdict = matching::evaluate(title, &wanted, None, None, &candidate);
            verdict.accepted.then_some((verdict.score, album))
        })
        // max_by keeps the last of equal scores: reverse so the catalog's first wins
        .rev()
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, album)| album)
}

impl CoverSearch for YouTubeService {
    fn find_cover<'a>(&'a self, kind: CoverKind, title: &'a str, artist: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            let query = format!("{artist} {title}");
            match kind {
                CoverKind::Track => {
                    let songs = self.search_songs(&query).await.ok()?;
                    let wanted = LfmTrack { name: title.to_owned(), artist: artist.to_owned(), image: String::new() };
                    let track = best_match(&wanted, songs)?;
                    cover_of(&track.cover_large, &track.cover)
                }
                CoverKind::Album => {
                    let page = self.search(&query, SearchFilter::Albums).await.ok()?;
                    let albums = page.items.into_iter().filter_map(|item| match item {
                        YtItem::Album(album) => Some(album),
                        _ => None,
                    });
                    let album = best_album(title, artist, albums.collect())?;
                    cover_of(&album.cover_large, &album.cover)
                }
            }
        })
    }
}

#[derive(Default)]
struct CoverState {
    /// key → url ("" — looked up, nothing found)
    done: HashMap<String, String>,
    /// queued or running
    pending: HashSet<String>,
    queue: VecDeque<(CoverKind, String, String)>,
    running: usize,
}

// --- the service ------------------------------------------------------------------------------------------------

/// Where a Smart Mix's tracks come from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MixTarget {
    #[default]
    YouTube,
    Spotify,
}

impl MixTarget {
    pub fn from_key(key: &str) -> Self {
        if key.trim().eq_ignore_ascii_case("spotify") { Self::Spotify } else { Self::YouTube }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LastFmFeedSnapshot {
    pub feed_loading: bool,
    pub recent: Arc<Vec<RecentTrack>>,
    pub friends: Arc<Vec<Friend>>,
    pub top_albums: Arc<Vec<TopAlbum>>,
    pub feed_error: String,
    pub generating: bool,
    pub progress: String,
    /// the generated mix ("" — none yet)
    pub mix_title: String,
    pub mix_mode: String,
    pub mix_cover: String,
    pub mix_tracks: Arc<Vec<TrackItem>>,
    pub mix_error: String,
    /// covers found in a catalog for feed tiles Last.fm has no art for (`cover_key` → url)
    pub covers: Arc<HashMap<String, String>>,
}

/// (api_key, username) of the connected Last.fm account; "" when not connected.
pub type AccountSource = Arc<dyn Fn() -> (String, String) + Send + Sync>;

#[derive(Clone)]
pub struct LastFmFeedOptions {
    pub api_url: String,
    /// fixed seed for every random choice (tests); None — random
    pub seed: Option<u64>,
}

impl Default for LastFmFeedOptions {
    fn default() -> Self {
        Self { api_url: API_URL.to_owned(), seed: None }
    }
}

#[derive(Default)]
struct Counters {
    feed: u64,
    mix: u64,
}

pub struct LastFmFeedService {
    rt: Handle,
    http: sf_api::Http,
    account: AccountSource,
    youtube: Option<Arc<dyn TrackSearch>>,
    spotify: Option<Arc<dyn TrackSearch>>,
    player: Option<PlayerHandle>,
    cover_search: Option<Arc<dyn CoverSearch>>,
    options: LastFmFeedOptions,
    snapshot: watch::Sender<LastFmFeedSnapshot>,
    counters: Mutex<Counters>,
    covers: Mutex<CoverState>,
    me: Weak<LastFmFeedService>,
}

impl LastFmFeedService {
    pub fn new(
        rt: Handle,
        http: sf_api::Http,
        account: AccountSource,
        youtube: Option<Arc<dyn TrackSearch>>,
        spotify: Option<Arc<dyn TrackSearch>>,
        player: Option<PlayerHandle>,
        cover_search: Option<Arc<dyn CoverSearch>>,
        options: LastFmFeedOptions,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            rt,
            http,
            account,
            youtube,
            spotify,
            player,
            cover_search,
            options,
            snapshot: watch::channel(LastFmFeedSnapshot::default()).0,
            counters: Mutex::default(),
            covers: Mutex::default(),
            me: me.clone(),
        })
    }

    pub fn snapshot(&self) -> LastFmFeedSnapshot {
        self.snapshot.borrow().clone()
    }

    pub fn watch(&self) -> watch::Receiver<LastFmFeedSnapshot> {
        self.snapshot.subscribe()
    }

    fn update(&self, f: impl FnOnce(&mut LastFmFeedSnapshot)) {
        self.snapshot.send_modify(f);
    }

    fn api(&self) -> Result<(FeedApi, String), LastFmError> {
        let (key, username) = (self.account)();
        if key.trim().is_empty() {
            return Err(LastFmError::NoKeys);
        }
        if is_guest(&username) {
            return Err(LastFmError::NotConnected);
        }
        Ok((FeedApi::new(self.http.clone(), &self.options.api_url, &key), username))
    }

    fn rng(&self) -> fastrand::Rng {
        self.options.seed.map_or_else(fastrand::Rng::new, fastrand::Rng::with_seed)
    }

    fn search_for(&self, target: MixTarget) -> Option<Arc<dyn TrackSearch>> {
        match target {
            MixTarget::YouTube => self.youtube.clone(),
            MixTarget::Spotify => self.spotify.clone(),
        }
    }

    /// Recent tracks, friends and top albums (a failure of one leaves the others).
    pub fn refresh_feed(&self) {
        let generation = {
            let mut counters = lock(&self.counters);
            counters.feed += 1;
            counters.feed
        };
        let (api, user) = match self.api() {
            Ok(found) => found,
            Err(error) => {
                self.update(|s| {
                    s.feed_loading = false;
                    s.recent = Arc::default();
                    s.friends = Arc::default();
                    s.top_albums = Arc::default();
                    s.feed_error = error.to_string();
                });
                return;
            }
        };
        self.update(|s| {
            s.feed_loading = true;
            s.feed_error.clear();
        });
        let me = self.me.clone();
        self.rt.spawn(async move {
            let (recent, friends, albums) = futures::join!(
                api.recent_tracks(&user, FEED_RECENT),
                api.friends(&user, FEED_FRIENDS),
                api.top_albums(&user, FEED_TOP_ALBUMS, "overall"),
            );
            let Some(service) = me.upgrade() else { return };
            if lock(&service.counters).feed != generation {
                return;
            }
            let error = [recent.as_ref().err(), friends.as_ref().err(), albums.as_ref().err()].into_iter().flatten().next().map(ToString::to_string);
            service.update(|s| {
                s.feed_loading = false;
                s.recent = Arc::new(recent.unwrap_or_default());
                s.friends = Arc::new(friends.unwrap_or_default());
                s.top_albums = Arc::new(albums.unwrap_or_default());
                s.feed_error = error.unwrap_or_default();
            });
        });
    }

    /// Smart Mix: Last.fm tracks for the mode, found in `target`; `existing` are the playlist names a saved mix must
    /// not repeat.
    pub fn generate(&self, request: MixRequest, target: MixTarget, existing: Vec<String>) {
        let generation = {
            let mut counters = lock(&self.counters);
            counters.mix += 1;
            counters.mix
        };
        let fail = |text: String| {
            self.update(|s| {
                s.generating = false;
                s.progress.clear();
                s.mix_error = text;
            })
        };
        if let Some(text) = request.missing_input() {
            return fail(text);
        }
        let (api, user) = match self.api() {
            Ok(found) => found,
            Err(error) => return fail(error.to_string()),
        };
        let Some(search) = self.search_for(target) else {
            return fail(mf_i18n::t("This catalog isn't available."));
        };
        self.update(|s| {
            s.generating = true;
            s.mix_error.clear();
            s.progress = mf_i18n::t("Connecting to Last.fm…");
        });
        let me = self.me.clone();
        let mut rng = self.rng();
        self.rt.spawn(async move {
            let current = |me: &Weak<Self>| me.upgrade().filter(|service| lock(&service.counters).mix == generation);
            let result = api.mix_tracks(&user, &request, &mut rng).await;
            let Some(service) = current(&me) else { return };
            let tracks = match result {
                Ok(tracks) if !tracks.is_empty() => tracks,
                Ok(_) => return service.finish(generation, Err(mf_i18n::t("No tracks found on Last.fm for your selections."))),
                Err(error) => return service.finish(generation, Err(error.to_string())),
            };
            let total = tracks.len();
            let progress = |found: usize| {
                if let Some(service) = current(&me) {
                    service.update(|s| s.progress = mf_i18n::tf("Finding playable tracks ({0}/{1})…", &[&found, &total]));
                }
            };
            progress(0);
            drop(service);
            let found = resolve(search.as_ref(), &tracks, request.count.max(1), progress).await;
            let Some(service) = current(&me) else { return };
            if found.is_empty() {
                return service.finish(generation, Err(mf_i18n::t("Couldn't find playable matches for any of the tracks.")));
            }
            let title = unique_name(&playlist_name(&request, &tracks), &existing);
            // Last.fm's art, else the first found track's
            let cover = tracks
                .iter()
                .map(|t| t.image.clone())
                .find(|image| !image.is_empty())
                .or_else(|| found.iter().find_map(|t| cover_of(&t.cover_large, &t.cover)))
                .unwrap_or_default();
            service.update(|s| {
                s.mix_title = title;
                s.mix_mode = request.mode.key().to_owned();
                s.mix_cover = cover;
                s.mix_tracks = Arc::new(found);
            });
            service.finish(generation, Ok(()));
        });
    }

    fn finish(&self, generation: u64, result: Result<(), String>) {
        if lock(&self.counters).mix != generation {
            return;
        }
        self.update(|s| {
            s.generating = false;
            s.progress.clear();
            s.mix_error = result.err().unwrap_or_default();
        });
    }

    pub fn cancel(&self) {
        lock(&self.counters).mix += 1;
        self.update(|s| {
            s.generating = false;
            s.progress.clear();
        });
    }

    /// A feed tile without Last.fm art asks for its cover (QML, when the tile is created). Each one is looked up
    /// once per run of the app, a few at a time; a found cover lands in `covers` under `cover_key`.
    pub fn request_cover(&self, kind: CoverKind, title: &str, artist: &str) {
        if self.cover_search.is_none() || title.trim().is_empty() {
            return;
        }
        let key = cover_key(kind, title, artist);
        {
            let mut state = lock(&self.covers);
            if state.done.contains_key(&key) || !state.pending.insert(key) {
                return;
            }
            state.queue.push_back((kind, title.trim().to_owned(), artist.trim().to_owned()));
        }
        self.pump_covers();
    }

    fn pump_covers(&self) {
        let Some(search) = self.cover_search.clone() else { return };
        loop {
            let (kind, title, artist) = {
                let mut state = lock(&self.covers);
                if state.running >= COVER_PARALLEL {
                    return;
                }
                let Some(next) = state.queue.pop_front() else { return };
                state.running += 1;
                next
            };
            let (me, search) = (self.me.clone(), search.clone());
            self.rt.spawn(async move {
                let url = search.find_cover(kind, &title, &artist).await.unwrap_or_default();
                let Some(service) = me.upgrade() else { return };
                let key = cover_key(kind, &title, &artist);
                let covers: Option<HashMap<String, String>> = {
                    let mut state = lock(&service.covers);
                    state.running -= 1;
                    state.pending.remove(&key);
                    state.done.insert(key, url.clone());
                    (!url.is_empty())
                        .then(|| state.done.iter().filter(|(_, u)| !u.is_empty()).map(|(k, u)| (k.clone(), u.clone())).collect())
                };
                if let Some(covers) = covers {
                    service.update(|s| s.covers = Arc::new(covers));
                }
                service.pump_covers();
            });
        }
    }

    /// A feed row (recent track): find it in `target` and play it.
    pub fn play_track(&self, title: String, artist: String, target: MixTarget) {
        let (Some(search), Some(player)) = (self.search_for(target), self.player.clone()) else { return };
        self.rt.spawn(async move {
            let track = LfmTrack { name: title, artist, image: String::new() };
            let query = format!("{} {}", track.name, track.artist);
            match best_match(&track, search.search_tracks(&query).await) {
                Some(item) => player.play_items(vec![item], 0, PlayContext::new(contexts::TRACKS)),
                None => tracing::info!("no playable match for a Last.fm track"),
            }
        });
    }
}

#[cfg(test)]
mod tests;
