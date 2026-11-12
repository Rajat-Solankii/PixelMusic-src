//! Playlist import (port of soundwave1 `desktopApp/.../PlaylistImport.kt`): a YouTube / YouTube Music
//! playlist link or id, a public Spotify playlist link (read from Spotify's embed page), an M3U / M3U8 file
//! or a JSON file. Songs that are not YouTube videos already are looked up on YouTube Music and matched by
//! title, artist and length. The result is the matched tracks plus what had no match; turning them into a
//! playlist is up to the caller.
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::StreamExt;
use md3_model::{ArtistRef, TrackItem};
use regex::Regex;
use serde_json::Value;

use super::YouTubeService;
use super::error::{Result, YouTubeError};
use super::models::track_uri;

/// Lowest score (`title*2 + artist*1.5 + length`) a search result needs to count as the same song.
const MIN_MATCH_SCORE: f64 = 2.2;
/// Lookups running at once, so YouTube Music does not throttle the import.
const MATCH_CONCURRENCY: usize = 4;
/// Spotify's embed page lists about this many songs; more come from its Web API.
const EMBED_PAGE_SIZE: usize = 100;
const SPOTIFY_MAX_TRACKS: usize = 5_000;
const BROWSER_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

static SPOTIFY_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:https?://open\.spotify\.com/(?:intl-[a-zA-Z-]+/)?(?:embed/)?playlist/|spotify:(?:user:[^:]+:)?playlist:)([a-zA-Z0-9]+)")
        .expect("regex")
});
static SPOTIFY_SHORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://(?:spotify\.link|spotify\.app\.link)/\S+").expect("regex"));
static NEXT_DATA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<script\s+id="__NEXT_DATA__"\s+type="application/json">([^<]+)</script>"#).expect("regex")
});
static LIST_PARAM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[?&]list=([A-Za-z0-9_-]+)").expect("regex"));
static BROWSE_VL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"youtube\.com/browse/VL([A-Za-z0-9_-]+)").expect("regex"));
static BARE_PLAYLIST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(PL|RD|OLAK5uy_|UU|LL|LM)[A-Za-z0-9_-]{2,}$").expect("regex"));
static VIDEO_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_-]{11}$").expect("regex"));
static VIDEO_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:youtube\.com/(?:watch\?(?:\S*&)?v=|shorts/)|youtu\.be/)([A-Za-z0-9_-]{11})").expect("regex")
});
static CREDITS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[(\[][^)\]]*(feat|ft\.|with|remaster|version|edit|mix|live|from)[^)\]]*[)\]]").expect("regex")
});
static SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s-\s.*(remaster|version|edit|live|mono|stereo|mix).*$").expect("regex"));
static NOT_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\p{L}\p{N} ]").expect("regex"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("regex"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportSource {
    YouTube,
    Spotify,
    M3u,
    Json,
}

/// What an import found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportResult {
    /// the source playlist's name (or the file's)
    pub title: String,
    /// matched songs in source order, each once
    pub tracks: Vec<TrackItem>,
    /// songs in the source
    pub total: usize,
    /// "title · artist" of every song without a match
    pub missed: Vec<String>,
}

/// A song known only by its name: looked up on YouTube Music.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WantedTrack {
    pub title: String,
    /// one or more names, `", "` / `" & "` separated
    pub artist: String,
    /// 0 when unknown
    pub duration_ms: i64,
}

impl WantedTrack {
    fn label(&self) -> String {
        if self.artist.trim().is_empty() { self.title.clone() } else { format!("{} · {}", self.title, self.artist) }
    }
}

/// One song of a source: a YouTube video already, or something to look up.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Entry {
    Known(Box<TrackItem>),
    Wanted(WantedTrack),
}

fn import_error(text: &str) -> YouTubeError {
    YouTubeError::Import(mf_i18n::t(text))
}

fn unquote(input: &str) -> &str {
    let trimmed = input.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = trimmed.strip_prefix(quote).and_then(|rest| rest.strip_suffix(quote)) {
            return inner;
        }
    }
    trimmed
}

/// Which kind of source a pasted link or a file path is, or `None`.
pub fn detect(input: &str) -> Option<ImportSource> {
    let value = unquote(input);
    let lower = value.to_lowercase();
    if lower.ends_with(".m3u") || lower.ends_with(".m3u8") {
        Some(ImportSource::M3u)
    } else if lower.ends_with(".json") {
        Some(ImportSource::Json)
    } else if SPOTIFY_ID.is_match(value) || SPOTIFY_SHORT.is_match(value) {
        Some(ImportSource::Spotify)
    } else if youtube_playlist_id(value).is_some() {
        Some(ImportSource::YouTube)
    } else {
        None
    }
}

/// `…?list=PL…` of any YouTube / YouTube Music link, `/browse/VL…`, or a bare playlist id.
pub fn youtube_playlist_id(input: &str) -> Option<String> {
    let value = input.trim();
    if let Some(found) = LIST_PARAM.captures(value).or_else(|| BROWSE_VL.captures(value)) {
        return Some(found[1].to_owned());
    }
    let bare = value.strip_prefix("VL").unwrap_or(value);
    BARE_PLAYLIST.is_match(bare).then(|| bare.to_owned())
}

/// The playlist id of an `open.spotify.com/playlist/…` link or a `spotify:playlist:…` URI.
pub fn spotify_playlist_id(input: &str) -> Option<String> {
    SPOTIFY_ID.captures(input.trim()).map(|found| found[1].to_owned())
}

/// Lower case, without credits ("(feat. …)", "- Remastered 2011"), punctuation or repeated spaces.
pub fn normalize(value: &str) -> String {
    let lower = value.to_lowercase();
    let without_credits = CREDITS.replace_all(&lower, " ");
    let without_suffix = SUFFIX.replace_all(&without_credits, " ");
    let words = NOT_WORD.replace_all(&without_suffix, " ");
    SPACES.replace_all(&words, " ").trim().to_owned()
}

fn artist_parts(artist: &str) -> impl Iterator<Item = &str> {
    artist.split([',', '&'])
}

/// How well a search result fits: title similarity counts double, a shared artist 1.5, the length decides
/// between versions. A different title is never the same song.
pub(crate) fn score(want: &WantedTrack, track: &TrackItem) -> f64 {
    let want_title = normalize(&want.title);
    let title = normalize(&track.title);
    let title_score = if title == want_title {
        1.0
    } else if title.starts_with(&want_title) || want_title.starts_with(&title) {
        0.8
    } else if title.contains(&want_title) || want_title.contains(&title) {
        0.6
    } else {
        return 0.0;
    };
    let wanted_artists: Vec<String> = artist_parts(&want.artist).map(normalize).filter(|name| !name.is_empty()).collect();
    let track_artists: Vec<String> = if track.artist_refs.is_empty() {
        artist_parts(&track.artists).map(normalize).collect()
    } else {
        track.artist_refs.iter().map(|artist| normalize(&artist.name)).collect()
    };
    let shared = wanted_artists
        .iter()
        .any(|wanted| track_artists.iter().any(|name| !name.is_empty() && (name.contains(wanted) || wanted.contains(name))));
    let artist_score = if shared { 1.0 } else { 0.0 };
    let duration_score = if want.duration_ms > 0 && track.duration_ms > 0 {
        match (track.duration_ms - want.duration_ms).abs() {
            0..=3_000 => 1.0,
            3_001..=10_000 => 0.6,
            10_001..=30_000 => 0.2,
            _ => -0.5,
        }
    } else {
        0.0
    };
    title_score * 2.0 + artist_score * 1.5 + duration_score
}

/// The best-scoring candidate, if it scores enough.
pub(crate) fn best_match<'a>(want: &WantedTrack, candidates: &'a [TrackItem]) -> Option<&'a TrackItem> {
    candidates
        .iter()
        .map(|track| (score(want, track), track))
        .filter(|(score, _)| *score >= MIN_MATCH_SCORE)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, track)| track)
}

fn known_track(video_id: &str, title: &str, artist: &str, duration_ms: i64, cover: &str) -> TrackItem {
    let artist_refs: Vec<ArtistRef> = artist_parts(artist)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| ArtistRef { id: String::new(), name: name.to_owned() })
        .collect();
    TrackItem {
        track_id: video_id.to_owned(),
        uri: track_uri(video_id),
        title: if title.trim().is_empty() { video_id.to_owned() } else { title.trim().to_owned() },
        artists: artist_refs.iter().map(|artist| artist.name.as_str()).collect::<Vec<_>>().join(", "),
        artist_refs,
        cover: cover.to_owned(),
        cover_large: cover.to_owned(),
        duration_ms,
        available: true,
        ..Default::default()
    }
}

// ── Spotify ─────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SpotifyEmbed {
    pub title: String,
    pub tracks: Vec<WantedTrack>,
    /// the page's anonymous Web API token (reads songs past the first page)
    pub token: Option<String>,
}

fn str_of<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value[key].as_str().map(str::trim).filter(|text| !text.is_empty())
}

/// The playlist inside Spotify's embed page (`__NEXT_DATA__`).
pub(crate) fn parse_spotify_embed(html: &str, playlist_id: &str) -> Result<SpotifyEmbed> {
    let data = NEXT_DATA.captures(html).ok_or_else(|| import_error("Couldn't read the playlist from Spotify."))?;
    let root: Value =
        serde_json::from_str(&data[1]).map_err(|_| import_error("Couldn't read the playlist from Spotify."))?;
    let state = &root["props"]["pageProps"]["state"];
    let entity = &state["data"]["entity"];
    if !entity.is_object() {
        // Spotify answers "not found" for private playlists and for its own editorial ones (ids 37i9…)
        return Err(if playlist_id.starts_with("37i9") {
            import_error(
                "Spotify doesn't share its own playlists (like Today's Top Hits) for import. Playlists made by people work.",
            )
        } else {
            import_error("This playlist is private or unavailable. Make it public on Spotify and try again.")
        });
    }
    let title = str_of(entity, "name").or_else(|| str_of(entity, "title")).map(str::to_owned);
    let tracks = entity["trackList"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some(WantedTrack {
                title: str_of(item, "title")?.to_owned(),
                artist: str_of(item, "subtitle").unwrap_or_default().to_owned(),
                duration_ms: item["duration"].as_i64().unwrap_or(0),
            })
        })
        .collect();
    Ok(SpotifyEmbed {
        title: title.unwrap_or_else(|| mf_i18n::t("Spotify playlist")),
        tracks,
        token: str_of(&state["settings"]["session"], "accessToken").map(str::to_owned),
    })
}

/// One page of `GET /v1/playlists/{id}/tracks`: its songs, the number of items and the playlist's total.
pub(crate) fn parse_spotify_tracks_page(page: &Value) -> (Vec<WantedTrack>, usize, usize) {
    let items = page["items"].as_array().map_or(&[][..], Vec::as_slice);
    let tracks = items
        .iter()
        .map(|item| &item["track"])
        .filter_map(|track| {
            let artists: Vec<&str> =
                track["artists"].as_array().into_iter().flatten().filter_map(|artist| str_of(artist, "name")).collect();
            Some(WantedTrack {
                title: str_of(track, "name")?.to_owned(),
                artist: artists.join(", "),
                duration_ms: track["duration_ms"].as_i64().unwrap_or(0),
            })
        })
        .collect();
    (tracks, items.len(), page["total"].as_u64().unwrap_or(0) as usize)
}

// ── M3U ─────────────────────────────────────────────────────────────────────────────────────────

/// One entry of an M3U file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct M3uEntry {
    pub duration_sec: Option<i64>,
    pub artist: Option<String>,
    pub title: String,
    pub location: String,
}

/// "Artist - Title" → (artist, title); anything else is a title.
fn split_artist_title(name: &str) -> (Option<String>, Option<String>) {
    let non_blank = |value: &str| Some(value.trim().to_owned()).filter(|value| !value.is_empty());
    match name.split_once(" - ") {
        Some((artist, title)) => (non_blank(artist), non_blank(title)),
        None => (None, non_blank(name)),
    }
}

/// `#EXTINF:<seconds>,<artist> - <title>` lines and the locations after them.
pub fn parse_m3u(content: &str) -> Vec<M3uEntry> {
    let mut entries = Vec::new();
    let (mut duration, mut artist, mut title): (Option<i64>, Option<String>, Option<String>) = (None, None, None);
    for raw in content.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        if line.len() >= 8 && line[..8].eq_ignore_ascii_case("#EXTINF:") {
            let rest = line[8..].trim();
            let (length, name) = match rest.split_once(',') {
                Some((length, name)) => (length.split([' ', '\t']).next().unwrap_or_default(), Some(name)),
                None => (rest, None),
            };
            duration = length.trim().parse::<f64>().ok().map(|seconds| seconds as i64).filter(|seconds| *seconds > 0);
            (artist, title) = name.map_or((None, None), split_artist_title);
        } else if !line.starts_with('#') {
            let title = match title.take() {
                Some(title) => title,
                None => {
                    let file = line.rsplit(['/', '\\']).next().unwrap_or(line);
                    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
                    let (file_artist, file_title) = split_artist_title(stem);
                    if file_artist.is_some() {
                        artist = file_artist;
                    }
                    file_title.unwrap_or_default()
                }
            };
            entries.push(M3uEntry {
                duration_sec: duration.take(),
                artist: artist.take(),
                title: if title.is_empty() { mf_i18n::t("Unknown track") } else { title },
                location: line.to_owned(),
            });
        }
    }
    entries
}

/// A YouTube link in the entry is played as is; anything else is looked up (local files cannot be played).
pub(crate) fn m3u_entry(entry: &M3uEntry) -> Entry {
    let artist = entry.artist.clone().unwrap_or_default();
    let duration_ms = entry.duration_sec.unwrap_or(0) * 1000;
    match VIDEO_LINK.captures(&entry.location) {
        Some(found) => Entry::Known(Box::new(known_track(&found[1], &entry.title, &artist, duration_ms, ""))),
        None => Entry::Wanted(WantedTrack { title: entry.title.clone(), artist, duration_ms }),
    }
}

// ── JSON ────────────────────────────────────────────────────────────────────────────────────────

fn json_artist(object: &Value) -> String {
    if let Some(artist) = str_of(object, "artist").or_else(|| str_of(object, "artistName")) {
        return artist.to_owned();
    }
    object["artists"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|artist| artist.as_str().or_else(|| artist["name"].as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Length in ms: `durationMs` / `duration_ms`, `durationSec`, or `duration` (ms when large, else seconds —
/// the reference app's own exports use seconds, other tools milliseconds).
fn json_duration_ms(object: &Value) -> i64 {
    let number = |key: &str| object[key].as_i64().or_else(|| object[key].as_f64().map(|value| value as i64)).filter(|n| *n > 0);
    number("durationMs")
        .or_else(|| number("duration_ms"))
        .or_else(|| number("durationSec").map(|seconds| seconds * 1000))
        .or_else(|| number("duration").map(|value| if value > 10_000 { value } else { value * 1000 }))
        .unwrap_or(0)
}

fn json_entry(object: &Value) -> Option<Entry> {
    let title = str_of(object, "title").or_else(|| str_of(object, "name")).or_else(|| str_of(object, "trackName"));
    let artist = json_artist(object);
    let duration_ms = json_duration_ms(object);
    let id = str_of(object, "videoId").or_else(|| str_of(object, "id")).filter(|id| VIDEO_ID.is_match(id));
    if let Some(id) = id {
        let cover = str_of(object, "thumbnail").or_else(|| str_of(object, "artwork")).unwrap_or_default();
        return Some(Entry::Known(Box::new(known_track(id, title.unwrap_or_default(), &artist, duration_ms, cover))));
    }
    Some(Entry::Wanted(WantedTrack { title: title?.to_owned(), artist, duration_ms }))
}

/// A playlist name and its songs from any of the JSON shapes the reference app reads: its own export
/// (`{name, tracks: [...]}`), a bare array of tracks, or `{name|title|playlist_name, tracks|songs|items|trackList|playlist}`.
pub(crate) fn parse_json_playlist(text: &str, default_name: &str) -> Result<(String, Vec<Entry>)> {
    let text = text.trim().trim_start_matches('\u{feff}');
    if text.is_empty() {
        return Err(import_error("The JSON file is empty."));
    }
    let root: Value = serde_json::from_str(text).map_err(|_| import_error("Invalid JSON structure."))?;
    let (name, list) = match &root {
        Value::Array(list) => (None, list),
        Value::Object(_) => {
            let name = ["name", "title", "playlist_name"].into_iter().find_map(|key| str_of(&root, key));
            let list = ["tracks", "songs", "items", "trackList", "playlist"]
                .into_iter()
                .find_map(|key| root[key].as_array())
                .ok_or_else(|| import_error("Could not find any playlist tracks in the JSON file."))?;
            (name, list)
        }
        _ => return Err(import_error("Invalid JSON structure.")),
    };
    let entries: Vec<Entry> = list.iter().filter(|item| item.is_object()).filter_map(json_entry).collect();
    if entries.is_empty() {
        return Err(import_error("No tracks found in the JSON playlist."));
    }
    Ok((name.unwrap_or(default_name).to_owned(), entries))
}

fn file_stem(path: &str) -> String {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    file.rsplit_once('.').map_or(file, |(stem, _)| stem).to_owned()
}

// ── the import itself ───────────────────────────────────────────────────────────────────────────

impl YouTubeService {
    /// Import whatever `input` is (see `detect`). `progress(done, total)` is called as songs are read or
    /// matched; `total` is 0 while unknown.
    pub async fn import_playlist(&self, input: &str, progress: impl Fn(usize, usize) + Send + Sync) -> Result<ImportResult> {
        match detect(input) {
            Some(ImportSource::YouTube) => self.import_youtube(input, &progress).await,
            Some(ImportSource::Spotify) => self.import_spotify(input, &progress).await,
            Some(ImportSource::M3u) => {
                let path = unquote(input);
                let entries: Vec<Entry> = parse_m3u(&read_file(path).await?).iter().map(m3u_entry).collect();
                if entries.is_empty() {
                    return Err(import_error("No songs found in that M3U file."));
                }
                self.resolve_entries(non_empty_or(file_stem(path), "M3U playlist"), entries, &progress).await
            }
            Some(ImportSource::Json) => {
                let path = unquote(input);
                let (title, entries) = parse_json_playlist(&read_file(path).await?, &file_stem(path))?;
                self.resolve_entries(non_empty_or(title, "Imported playlist"), entries, &progress).await
            }
            None => Err(import_error(
                "Paste a YouTube, YouTube Music or Spotify playlist link, or select an M3U/JSON file.",
            )),
        }
    }

    async fn import_youtube(&self, input: &str, progress: &(impl Fn(usize, usize) + Sync)) -> Result<ImportResult> {
        let id = youtube_playlist_id(unquote(input))
            .ok_or_else(|| import_error("That doesn't look like a YouTube playlist link."))?;
        let page = self.playlist_full(&id, |done| progress(done, 0)).await.map_err(|error| match error {
            YouTubeError::PlaylistUnavailable | YouTubeError::Http(_) | YouTubeError::Unexpected(_) => {
                import_error("Couldn't open that playlist. It may be private or deleted.")
            }
            other => other,
        })?;
        let mut seen = std::collections::HashSet::new();
        let tracks: Vec<TrackItem> = page
            .tracks
            .into_iter()
            .map(|row| row.track)
            .filter(|track| track.available && seen.insert(track.track_id.clone()))
            .collect();
        if tracks.is_empty() {
            return Err(import_error("That playlist has no songs this app can play."));
        }
        Ok(ImportResult {
            title: non_empty_or(page.info.title, "YouTube playlist"),
            total: tracks.len(),
            tracks,
            missed: Vec::new(),
        })
    }

    async fn import_spotify(&self, input: &str, progress: &(impl Fn(usize, usize) + Sync)) -> Result<ImportResult> {
        let http = self.client.http_client()?;
        let input = unquote(input);
        let id = match spotify_playlist_id(input) {
            Some(id) => Some(id),
            None => resolve_short_link(&http, input).await,
        }
        .ok_or_else(|| import_error("That doesn't look like a Spotify playlist link. Copy it from Share → Copy link to playlist."))?;

        let response = http
            .get(format!("https://open.spotify.com/embed/playlist/{id}"))
            .header(reqwest::header::USER_AGENT, BROWSER_UA)
            .send()
            .await
            .map_err(|error| YouTubeError::Network(error.without_url().to_string()))?;
        match response.status().as_u16() {
            200..=299 => {}
            404 => return Err(import_error("Playlist not found. It may be private or deleted.")),
            code => return Err(YouTubeError::Import(mf_i18n::tf("Spotify answered HTTP {0}", &[&code]))),
        }
        let html = response.text().await.map_err(|error| YouTubeError::Network(error.without_url().to_string()))?;
        let mut embed = parse_spotify_embed(&html, &id)?;
        if embed.tracks.len() >= EMBED_PAGE_SIZE
            && let Some(token) = embed.token.clone()
        {
            read_spotify_rest(&http, &id, &token, &mut embed.tracks).await;
        }
        if embed.tracks.is_empty() {
            return Err(import_error("That Spotify playlist is empty, or private. Make it public on Spotify and try again."));
        }
        let entries = embed.tracks.into_iter().map(Entry::Wanted).collect();
        self.resolve_entries(embed.title, entries, progress).await
    }

    /// Look up every song not known yet (a few at a time, keeping the order); fails when nothing matched.
    async fn resolve_entries(
        &self,
        title: String,
        entries: Vec<Entry>,
        progress: &(impl Fn(usize, usize) + Sync),
    ) -> Result<ImportResult> {
        let total = entries.len();
        progress(0, total);
        let done = AtomicUsize::new(0);
        let resolved: Vec<(Option<TrackItem>, String)> = futures::stream::iter(entries)
            .map(|entry| {
                let done = &done;
                async move {
                    let result = match entry {
                        Entry::Known(track) => {
                            let label = track.title.clone();
                            (Some(*track), label)
                        }
                        Entry::Wanted(want) => (self.find_match(&want).await, want.label()),
                    };
                    progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
                    result
                }
            })
            .buffered(MATCH_CONCURRENCY)
            .collect()
            .await;
        let mut seen = std::collections::HashSet::new();
        let mut tracks = Vec::new();
        let mut missed = Vec::new();
        for (track, label) in resolved {
            match track {
                Some(track) if seen.insert(track.track_id.clone()) => tracks.push(track),
                Some(_) => {}
                None => missed.push(label),
            }
        }
        if tracks.is_empty() {
            return Err(import_error("None of the songs could be found on YouTube Music."));
        }
        Ok(ImportResult { title, tracks, total, missed })
    }

    /// The YouTube Music song for a name: "title artist" first, then the cleaned-up title (credits and
    /// "- Remastered" suffixes often derail the first query). Lookup failures count as no match.
    async fn find_match(&self, want: &WantedTrack) -> Option<TrackItem> {
        let primary_artist = artist_parts(&want.artist).next().unwrap_or_default().trim().to_owned();
        let mut candidates = self.search_songs(&format!("{} {primary_artist}", want.title)).await.unwrap_or_default();
        if let Some(found) = best_match(want, &candidates) {
            return Some(found.clone());
        }
        let retry = self.search_songs(&format!("{} {primary_artist}", normalize(&want.title))).await.unwrap_or_default();
        candidates.extend(retry);
        best_match(want, &candidates).cloned()
    }
}

fn non_empty_or(value: String, fallback: &str) -> String {
    if value.trim().is_empty() { mf_i18n::t(fallback) } else { value }
}

async fn read_file(path: &str) -> Result<String> {
    tokio::fs::read_to_string(path)
        .await
        .map_err(|_| YouTubeError::Import(mf_i18n::tf("Couldn't read the file {0}", &[&path])))
}

/// Share-sheet links (`spotify.link/…`) redirect to open.spotify.com; follow them to the playlist id.
async fn resolve_short_link(http: &reqwest::Client, input: &str) -> Option<String> {
    let url = SPOTIFY_SHORT.find(input)?.as_str();
    let response = http.get(url).header(reqwest::header::USER_AGENT, "Mozilla/5.0").send().await.ok()?;
    if let Some(id) = spotify_playlist_id(response.url().as_str()) {
        return Some(id);
    }
    spotify_playlist_id(&response.text().await.ok()?)
}

/// Songs past the embed page, from Spotify's Web API with the page's anonymous token. Stops quietly on the
/// first failure: the songs read so far are still imported.
async fn read_spotify_rest(http: &reqwest::Client, id: &str, token: &str, out: &mut Vec<WantedTrack>) {
    let mut offset = out.len();
    while offset < SPOTIFY_MAX_TRACKS {
        let url = format!("https://api.spotify.com/v1/playlists/{id}/tracks?offset={offset}&limit=100");
        let Ok(response) = http.get(url).bearer_auth(token).header(reqwest::header::USER_AGENT, "Mozilla/5.0").send().await
        else {
            return;
        };
        if !response.status().is_success() {
            return;
        }
        let Ok(page) = response.json::<Value>().await else { return };
        let (tracks, listed, total) = parse_spotify_tracks_page(&page);
        out.extend(tracks);
        offset += 100;
        if listed < 100 || offset >= total {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn track(title: &str, artist: &str, seconds: i64) -> TrackItem {
        TrackItem {
            track_id: format!("{title}-{artist}-{seconds}"),
            title: title.to_owned(),
            artists: artist.to_owned(),
            artist_refs: vec![ArtistRef { id: String::new(), name: artist.to_owned() }],
            duration_ms: seconds * 1000,
            ..Default::default()
        }
    }

    #[test]
    fn sources_are_detected() {
        assert_eq!(detect("https://music.youtube.com/playlist?list=PLabc123"), Some(ImportSource::YouTube));
        assert_eq!(detect("https://www.youtube.com/watch?v=x&list=RDxyz99"), Some(ImportSource::YouTube));
        assert_eq!(detect("VLPLabcdef"), Some(ImportSource::YouTube));
        assert_eq!(detect("https://open.spotify.com/intl-de/playlist/37i9dQZF1DXcBWIGoYBM5M?si=1"), Some(ImportSource::Spotify));
        assert_eq!(detect("spotify:user:me:playlist:abc"), Some(ImportSource::Spotify));
        assert_eq!(detect("https://spotify.link/AbCdEf"), Some(ImportSource::Spotify));
        assert_eq!(detect("\"C:\\Music\\road trip.M3U8\""), Some(ImportSource::M3u));
        assert_eq!(detect("/home/me/list.json"), Some(ImportSource::Json));
        assert_eq!(detect("hello"), None);
        assert_eq!(detect("PL"), None);
    }

    #[test]
    fn playlist_ids_are_read_from_links_and_bare_ids() {
        assert_eq!(youtube_playlist_id("https://music.youtube.com/playlist?list=PLabc-_1").as_deref(), Some("PLabc-_1"));
        assert_eq!(youtube_playlist_id("https://music.youtube.com/browse/VLPLxyz").as_deref(), Some("PLxyz"));
        assert_eq!(youtube_playlist_id("OLAK5uy_abc").as_deref(), Some("OLAK5uy_abc"));
        assert_eq!(youtube_playlist_id("LM").as_deref(), None);
        assert_eq!(youtube_playlist_id("LMxx").as_deref(), Some("LMxx"));
        assert_eq!(spotify_playlist_id("https://open.spotify.com/embed/playlist/3cEYpjA9oz9GiPac4AsH4n").as_deref(), Some("3cEYpjA9oz9GiPac4AsH4n"));
    }

    #[test]
    fn names_are_normalized_without_credits_and_suffixes() {
        assert_eq!(normalize("Under Pressure (feat. David Bowie)"), "under pressure");
        assert_eq!(normalize("Heroes - 2017 Remaster"), "heroes");
        assert_eq!(normalize("Don't Stop Me Now!"), "don t stop me now");
        assert_eq!(normalize("  Déjà   Vu "), "déjà vu");
    }

    #[test]
    fn the_match_needs_the_title_and_prefers_the_right_length() {
        let want = WantedTrack { title: "Bohemian Rhapsody - Remastered 2011".into(), artist: "Queen".into(), duration_ms: 354_000 };
        let exact = track("Bohemian Rhapsody", "Queen", 355);
        let live = track("Bohemian Rhapsody (Live Aid)", "Queen", 400);
        let cover = track("Bohemian Rhapsody", "Some Choir", 354);
        let other = track("Killer Queen", "Queen", 354);
        assert_eq!(score(&want, &exact), 2.0 + 1.5 + 1.0);
        assert_eq!(score(&want, &other), 0.0);
        let candidates = [live.clone(), cover.clone(), other, exact.clone()];
        assert_eq!(best_match(&want, &candidates), Some(&exact));
        // a cover with only the title and the length right still clears the bar; a wrong title never does
        assert_eq!(best_match(&want, std::slice::from_ref(&cover)), Some(&cover));
        assert_eq!(best_match(&WantedTrack { title: "Nothing alike".into(), ..want }, &candidates), None);
    }

    #[test]
    fn the_spotify_embed_page_gives_the_title_songs_and_token() {
        let data = json!({ "props": { "pageProps": { "state": {
            "data": { "entity": { "name": "Road Trip", "trackList": [
                { "title": "Song A", "subtitle": "Artist A, Artist B", "duration": 201000 },
                { "title": "  ", "subtitle": "Nobody" },
                { "title": "Song B", "subtitle": "Artist C" }
            ]}},
            "settings": { "session": { "accessToken": "anon-token" } }
        }}}});
        let html = format!(r#"<html><script id="__NEXT_DATA__" type="application/json">{data}</script></html>"#);
        let embed = parse_spotify_embed(&html, "abc").expect("embed");
        assert_eq!(embed.title, "Road Trip");
        assert_eq!(embed.token.as_deref(), Some("anon-token"));
        assert_eq!(
            embed.tracks,
            [
                WantedTrack { title: "Song A".into(), artist: "Artist A, Artist B".into(), duration_ms: 201_000 },
                WantedTrack { title: "Song B".into(), artist: "Artist C".into(), duration_ms: 0 },
            ]
        );
        let missing = json!({ "props": { "pageProps": { "state": { "data": {} } } } });
        let html = format!(r#"<script id="__NEXT_DATA__" type="application/json">{missing}</script>"#);
        assert!(matches!(parse_spotify_embed(&html, "37i9dQ"), Err(YouTubeError::Import(text)) if text.contains("Top Hits")));
        assert!(matches!(parse_spotify_embed("<html></html>", "x"), Err(YouTubeError::Import(_))));

        let page = json!({ "total": 150, "items": [
            { "track": { "name": "Song C", "duration_ms": 1000, "artists": [{ "name": "X" }, { "name": "Y" }] } },
            { "track": null }
        ]});
        let (tracks, listed, total) = parse_spotify_tracks_page(&page);
        assert_eq!(tracks, [WantedTrack { title: "Song C".into(), artist: "X, Y".into(), duration_ms: 1000 }]);
        assert_eq!((listed, total), (2, 150));
    }

    #[test]
    fn m3u_files_give_names_lengths_and_youtube_links() {
        let content = "\u{feff}#EXTM3U\n#EXTINF:215,Daft Punk - One More Time\nmusic/one more time.mp3\n\n#EXTINF:-1,Just A Title\nhttps://www.youtube.com/watch?v=dQw4w9WgXcQ\nC:\\Music\\Queen - Bicycle Race.flac\n";
        let entries = parse_m3u(content);
        assert_eq!(
            entries,
            [
                M3uEntry { duration_sec: Some(215), artist: Some("Daft Punk".into()), title: "One More Time".into(), location: "music/one more time.mp3".into() },
                M3uEntry { duration_sec: None, artist: None, title: "Just A Title".into(), location: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".into() },
                M3uEntry { duration_sec: None, artist: Some("Queen".into()), title: "Bicycle Race".into(), location: "C:\\Music\\Queen - Bicycle Race.flac".into() },
            ]
        );
        assert_eq!(
            m3u_entry(&entries[0]),
            Entry::Wanted(WantedTrack { title: "One More Time".into(), artist: "Daft Punk".into(), duration_ms: 215_000 })
        );
        let Entry::Known(linked) = m3u_entry(&entries[1]) else { panic!("{:?}", entries[1]) };
        assert_eq!(linked.uri, "youtube:track:dQw4w9WgXcQ");
        assert_eq!(linked.title, "Just A Title");
    }

    #[test]
    fn json_playlists_of_every_known_shape_are_read() {
        let exported = r#"{ "name": "Mine", "tracks": [
            { "id": "dQw4w9WgXcQ", "title": "Never", "artists": [{ "name": "Rick" }], "durationSec": 213, "thumbnail": "https://i.ytimg.com/x.jpg" },
            { "title": "Wanted", "artistName": "Someone", "duration_ms": 180000 },
            { "nothing": true }
        ]}"#;
        let (name, entries) = parse_json_playlist(exported, "file").expect("json");
        assert_eq!(name, "Mine");
        assert_eq!(entries.len(), 2);
        let Entry::Known(known) = &entries[0] else { panic!("{:?}", entries[0]) };
        assert_eq!((known.track_id.as_str(), known.artists.as_str(), known.duration_ms), ("dQw4w9WgXcQ", "Rick", 213_000));
        assert_eq!(known.cover, "https://i.ytimg.com/x.jpg");
        assert_eq!(entries[1], Entry::Wanted(WantedTrack { title: "Wanted".into(), artist: "Someone".into(), duration_ms: 180_000 }));

        let (name, entries) = parse_json_playlist(r#"[{ "name": "Bare", "artists": ["A", "B"], "duration": 200 }]"#, "file").expect("array");
        assert_eq!(name, "file");
        assert_eq!(entries, [Entry::Wanted(WantedTrack { title: "Bare".into(), artist: "A, B".into(), duration_ms: 200_000 })]);

        let (name, _) = parse_json_playlist(r#"{ "playlist_name": "P", "songs": [{ "trackName": "S" }] }"#, "file").expect("songs");
        assert_eq!(name, "P");
        for broken in ["", "{}", "[1, 2]", "nope", "42"] {
            assert!(matches!(parse_json_playlist(broken, "file"), Err(YouTubeError::Import(_))), "{broken}");
        }
    }
}
