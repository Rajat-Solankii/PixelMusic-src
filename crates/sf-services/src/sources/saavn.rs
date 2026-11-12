//! JioSaavn (`www.jiosaavn.com/api.php`): no account, no key; AAC-LC in MP4 at 96/160/320 kbps. Port of both
//! `SaavnService.kt` files (Android `data/remote/saavn/`, desktop `data/SaavnService.kt`).
//!
//! `__call=search.getResults` answers `{"results": [{id, title, image, explicit_content, more_info: {album,
//! duration, encrypted_media_url, artistMap: {primary_artists, featured_artists, artists}, rights}}]}`. The
//! media URL is DES-ECB (key `38346591`, PKCS#5) over base64; its `_96.mp4` suffix picks the bitrate. When a hit
//! has no media URL, `__call=song.getDetails&pids=<id>` has it. Matching follows `SaavnService.resolveStream`:
//! album, then explicit flag first; the cleaned titles contain each other, an artist is shared and the duration
//! is within 12 s; Pro-only songs are skipped. A primary query with the album, then one without.
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use base64::Engine;
use des::cipher::{BlockDecryptMut, KeyInit, block_padding::Pkcs7};
use regex::Regex;
use md3_model::{ArtistRef, TrackItem};
use serde_json::Value;
use sf_player::engine::youtube::{SAAVN_TRACK, StreamRequest, YtStream, saavn_id};

use super::{Cooldown, Outcome, SourceError};

pub const BASE_URL: &str = "https://www.jiosaavn.com/api.php";
pub const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";
const DES_KEY: &[u8; 8] = b"38346591";
/// The Android app gives up on JioSaavn after this long per request.
const TIMEOUT: Duration = Duration::from_secs(3);
/// Same recording when the durations are this close.
const DURATION_TOLERANCE_SECS: i64 = 12;

static QUALITY_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"_(48|96|160|320)\.(mp4|aac|mp3)$").unwrap());
static TITLE_NOISE: LazyLock<[Regex; 8]> = LazyLock::new(|| {
    [
        r"(?i)\(official\s*(music)?\s*(video|audio)?\)",
        r"(?i)\[official\s*(music)?\s*(video|audio)?\]",
        r"(?i)\(lyrics?\)",
        r"(?i)\[lyrics?\]",
        r"(?i)\(audio\)",
        r"(?i)\[audio\]",
        r"(?i)\bft\.?\b.*",
        r"(?i)\bfeat\.?\b.*",
    ]
    .map(|pattern| Regex::new(pattern).unwrap())
});
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// Stream bitrate (`SaavnAudioQuality`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SaavnQuality {
    Kbps96,
    Kbps160,
    #[default]
    Kbps320,
}

impl SaavnQuality {
    pub fn kbps(self) -> u32 {
        match self {
            Self::Kbps96 => 96,
            Self::Kbps160 => 160,
            Self::Kbps320 => 320,
        }
    }
}

/// One song of a search or details answer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SaavnSong {
    pub id: String,
    pub title: String,
    /// all artists (primary and featured when the answer has no full list)
    pub artists: Vec<String>,
    pub album: String,
    /// seconds
    pub duration: Option<i64>,
    pub explicit: bool,
    pub pro_only: bool,
    /// 500×500 cover
    pub image: String,
    /// decrypted media URL ("" — none in this answer)
    pub media_url: String,
}

/// The JioSaavn client (cheap to clone; clones share the cooldown).
#[derive(Clone)]
pub struct Saavn {
    http: sf_api::Http,
    base: String,
    pub(crate) cooldown: Arc<Cooldown>,
}

/// The media URL inside `encrypted_media_url` ("" when it does not decrypt).
pub fn decrypt_url(encrypted: &str) -> String {
    let compact: String = encrypted.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return String::new();
    }
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(compact) else { return String::new() };
    ecb::Decryptor::<des::Des>::new(DES_KEY.into())
        .decrypt_padded_vec_mut::<Pkcs7>(&bytes)
        .ok()
        .and_then(|plain| String::from_utf8(plain).ok())
        .map(|url| url.trim().to_owned())
        .unwrap_or_default()
}

/// The media URL at `quality`, over https (`createDownloadLinks` + the desktop app's https upgrade).
pub fn quality_url(media_url: &str, quality: SaavnQuality) -> String {
    let kbps = quality.kbps();
    let url = if QUALITY_SUFFIX.is_match(media_url) {
        QUALITY_SUFFIX.replace(media_url, format!("_{kbps}.$2")).into_owned()
    } else {
        media_url.replace("_96", &format!("_{kbps}"))
    };
    match url.strip_prefix("http://") {
        Some(rest) => format!("https://{rest}"),
        None => url,
    }
}

/// A title without "(Official Video)", "[Lyrics]", "ft. …" (`SaavnService.cleanTitle`).
pub fn clean_title(title: &str) -> String {
    let mut out = title.to_owned();
    for pattern in TITLE_NOISE.iter() {
        out = pattern.replace_all(&out, "").into_owned();
    }
    SPACES.replace_all(&out, " ").trim().to_owned()
}

fn unescape(text: &str) -> String {
    text.replace("&quot;", "\"").replace("&#039;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => unescape(text.trim()),
        Value::Number(number) => number.to_string(),
        _ => String::new(),
    }
}

/// A song object of a search or details answer.
pub fn parse_song(raw: &Value) -> Option<SaavnSong> {
    let id = text(&raw["id"]);
    if id.is_empty() {
        return None;
    }
    let info = &raw["more_info"];
    let names = |key: &str| -> Vec<String> {
        info["artistMap"][key]
            .as_array()
            .map(|list| list.iter().map(|artist| text(&artist["name"])).filter(|name| !name.is_empty()).collect())
            .unwrap_or_default()
    };
    let mut artists = names("artists");
    if artists.is_empty() {
        artists = names("primary_artists");
        artists.extend(names("featured_artists"));
    }
    let mut seen = std::collections::HashSet::new();
    artists.retain(|name| seen.insert(name.to_lowercase()));
    let rights = &info["rights"];
    let image = text(&raw["image"]).replace("150x150", "500x500").replace("50x50", "500x500");
    Some(SaavnSong {
        id,
        title: text(&raw["title"]),
        artists,
        album: text(&info["album"]),
        duration: text(&info["duration"]).parse().ok(),
        explicit: text(&raw["explicit_content"]) == "1",
        pro_only: text(&rights["code"]) == "1" || text(&rights["reason"]).to_lowercase().contains("pro only"),
        image: image.replacen("http://", "https://", 1),
        media_url: decrypt_url(&text(&info["encrypted_media_url"])),
    })
}

/// The first song that is the request (`findMatch`), from hits already in the API's order.
pub fn best_match<'a>(request: &StreamRequest, songs: &'a [SaavnSong]) -> Option<&'a SaavnSong> {
    let wanted_title = clean_title(&request.title).to_lowercase();
    if wanted_title.is_empty() {
        return None;
    }
    let wanted_artists: Vec<String> =
        request.artists.iter().map(|artist| artist.trim().to_lowercase()).filter(|artist| !artist.is_empty()).collect();
    let wanted_album = request.album.trim().to_lowercase();
    let wanted_seconds = (request.duration_ms > 0).then_some(request.duration_ms / 1000);

    let mut sorted: Vec<&SaavnSong> = songs.iter().collect();
    // stable: album matches first, then the same explicit flag
    sorted.sort_by_key(|song| {
        let album = !wanted_album.is_empty() && song.album.trim().to_lowercase() == wanted_album;
        (!album, song.explicit != request.explicit)
    });
    sorted.into_iter().find(|song| {
        if song.pro_only {
            return false;
        }
        let title = clean_title(&song.title).to_lowercase();
        let title_matches = !title.is_empty()
            && (title == wanted_title || title.contains(&wanted_title) || wanted_title.contains(&title));
        if !title_matches {
            return false;
        }
        let artists: Vec<String> = song.artists.iter().map(|artist| artist.trim().to_lowercase()).collect();
        let artist_matches = wanted_artists.is_empty()
            || wanted_artists.iter().any(|wanted| {
                artists.iter().any(|artist| !artist.is_empty() && (artist.contains(wanted) || wanted.contains(artist.as_str())))
            });
        if !artist_matches {
            return false;
        }
        match (wanted_seconds, song.duration) {
            (Some(wanted), Some(found)) => (wanted - found).abs() <= DURATION_TOLERANCE_SECS,
            _ => true,
        }
    })
}

impl Saavn {
    pub fn new(http: sf_api::Http) -> Self {
        Self { http, base: BASE_URL.to_owned(), cooldown: Arc::default() }
    }

    /// Point requests at another endpoint (tests): `base` stands for `https://www.jiosaavn.com/api.php`.
    pub fn with_base_url(mut self, base: &str) -> Self {
        self.base = base.to_owned();
        self
    }

    pub fn is_up(&self) -> bool {
        self.cooldown.is_up()
    }

    async fn call(&self, call: &str, params: &[(&str, &str)]) -> Result<Value, SourceError> {
        let client = self.http.client().map_err(SourceError::Network)?;
        let mut query: Vec<(&str, &str)> =
            vec![("__call", call), ("_format", "json"), ("_marker", "0"), ("api_version", "4"), ("ctx", "android")];
        query.extend_from_slice(params);
        let sent = client
            .get(&self.base)
            .query(&query)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-IN,en;q=0.9")
            .header(reqwest::header::COOKIE, "explicit_content=1")
            .timeout(TIMEOUT)
            .send()
            .await;
        let result = async {
            let response = sent.map_err(|error| SourceError::Network(error.without_url().to_string()))?;
            let status = response.status();
            if !status.is_success() {
                return Err(SourceError::Status(status.as_u16()));
            }
            let text = response.text().await.map_err(|error| SourceError::Network(error.without_url().to_string()))?;
            serde_json::from_str::<Value>(&text).map_err(|error| SourceError::Parse(error.to_string()))
        }
        .await;
        self.cooldown.record(&result);
        result
    }

    /// Songs for a free-text query (up to 30).
    pub async fn search(&self, query: &str) -> Result<Vec<SaavnSong>, SourceError> {
        let body = self.call("search.getResults", &[("q", query), ("p", "1"), ("n", "30")]).await?;
        Ok(body["results"].as_array().map(|results| results.iter().filter_map(parse_song).collect()).unwrap_or_default())
    }

    /// One song by its JioSaavn id.
    pub async fn details(&self, id: &str) -> Result<Option<SaavnSong>, SourceError> {
        let body = self.call("song.getDetails", &[("pids", id)]).await?;
        // `{"songs": [..]}` or `{"<id>": {..}}`; only the song asked for counts
        let songs = body["songs"].as_array().into_iter().flatten().chain(body.as_object().into_iter().flat_map(|object| object.values()));
        Ok(songs.filter_map(parse_song).find(|song| song.id == id))
    }

    /// The request's match on JioSaavn at `quality`, as a stream (`None`: resting, nothing matched).
    pub async fn resolve(&self, request: &StreamRequest, quality: SaavnQuality) -> Outcome {
        if request.title.trim().is_empty() || !self.is_up() {
            return Ok(None);
        }
        let title = clean_title(&request.title);
        let artists = request.artists.iter().map(|artist| artist.trim()).filter(|artist| !artist.is_empty());
        let artists = artists.collect::<Vec<_>>().join(" ");
        let fallback = SPACES.replace_all(&format!("{title} {artists}"), " ").trim().to_owned();
        let primary = match request.album.trim() {
            "" => fallback.clone(),
            album => SPACES.replace_all(&format!("{album} {title} {artists}"), " ").trim().to_owned(),
        };
        let mut queries = vec![primary];
        if queries[0] != fallback {
            queries.push(fallback);
        }
        let mut found = None;
        for query in queries.iter().filter(|query| !query.is_empty()) {
            let songs = self.search(query).await?;
            if let Some(song) = best_match(request, &songs) {
                found = Some(song.clone());
                break;
            }
        }
        let Some(mut song) = found else {
            log::debug!("jiosaavn: no match for \"{}\"", request.title);
            return Ok(None);
        };
        if song.media_url.is_empty() {
            match self.details(&song.id).await? {
                Some(details) if !details.media_url.is_empty() => song.media_url = details.media_url,
                _ => return Ok(None),
            }
        }
        Ok(Some(stream(request, song, quality)))
    }

    /// The stream of a `saavn:track:<id>` item, straight from `song.getDetails` (`None`: no such song, or it
    /// has no media URL). The cooldown does not apply: the listener picked this song.
    pub async fn stream_of(&self, request: &StreamRequest, quality: SaavnQuality) -> Outcome {
        let Some(id) = saavn_id(&request.video_id) else { return Ok(None) };
        match self.details(id).await? {
            Some(song) if !song.media_url.is_empty() => Ok(Some(stream(request, song, quality))),
            _ => Ok(None),
        }
    }

    /// Songs for the search page, as `saavn:track:<id>` items (Pro-only ones unavailable).
    pub async fn search_tracks(&self, query: &str) -> Result<Vec<TrackItem>, SourceError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.search(query).await?.iter().map(track_item).collect())
    }
}

/// A song as a list item that plays through the YouTube lane.
pub fn track_item(song: &SaavnSong) -> TrackItem {
    TrackItem {
        track_id: song.id.clone(),
        title: song.title.clone(),
        artists: song.artists.join(", "),
        artist_refs: song.artists.iter().map(|name| ArtistRef { id: String::new(), name: name.clone() }).collect(),
        album: song.album.clone(),
        cover: song.image.replace("500x500", "150x150"),
        cover_large: song.image.clone(),
        duration_ms: song.duration.unwrap_or(0) * 1000,
        explicit: song.explicit,
        available: !song.pro_only,
        uri: format!("{SAAVN_TRACK}{}", song.id),
        ..Default::default()
    }
}

fn stream(request: &StreamRequest, song: SaavnSong, quality: SaavnQuality) -> YtStream {
    YtStream {
        video_id: request.video_id.clone(),
        url: quality_url(&song.media_url, quality),
        mime: "audio/mp4".into(),
        codecs: "mp4a.40.2".into(),
        bitrate: i64::from(quality.kbps()) * 1000,
        duration_ms: song.duration.map_or(request.duration_ms, |seconds| seconds * 1000),
        user_agent: USER_AGENT.into(),
        source: "JioSaavn".into(),
        title: song.title,
        author: song.artists.join(", "),
        thumbnail: song.image,
        ..Default::default()
    }
}
