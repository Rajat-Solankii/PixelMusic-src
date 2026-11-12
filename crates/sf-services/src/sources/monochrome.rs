//! Monochrome, the free lossless server (`tracks.monochrome.st`): no account, no key. Port of
//! `resolveMonochromeStream` / `checkMonochrome` in the Android app (`LosslessStreamResolver.kt`,
//! `SourceCheckService.kt`).
//!
//! `GET /search/tracks?q=<first artist> <title>&limit=5` answers `{"tracks": [{id | trackId, title,
//! artistNames: [..], albumTitle, duration (ms, or seconds when ≤ 10 000), playable}]}`; every candidate goes
//! through [`matching::evaluate`] and the best accepted one plays from `GET /track/<id>`, a 16-bit/44.1 kHz FLAC
//! file. A 5xx answer or a network error rests the server for [`super::REST`].
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use sf_player::engine::youtube::{StreamRequest, YtStream};

use super::matching::{self, Candidate};
use super::{Cooldown, Outcome, SourceError};

pub const BASE_URL: &str = "https://tracks.monochrome.st";
/// The browser User-Agent the Android app sends.
pub const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";
const SEARCH_LIMIT: u32 = 5;
const TIMEOUT: Duration = Duration::from_secs(8);
/// What `/track/<id>` serves.
const SAMPLE_RATE: u32 = 44_100;
const BIT_DEPTH: u32 = 16;

/// The Monochrome client (cheap to clone; clones share the cooldown).
#[derive(Clone)]
pub struct Monochrome {
    http: sf_api::Http,
    base: String,
    pub(crate) cooldown: Arc<Cooldown>,
}

/// One search hit.
#[derive(Clone, Debug, PartialEq)]
pub struct MonochromeTrack {
    pub id: String,
    pub candidate: Candidate,
}

impl Monochrome {
    pub fn new(http: sf_api::Http) -> Self {
        Self { http, base: BASE_URL.to_owned(), cooldown: Arc::default() }
    }

    /// Point requests at another host (tests).
    pub fn with_base_url(mut self, base: &str) -> Self {
        self.base = base.trim_end_matches('/').to_owned();
        self
    }

    /// Not resting after a failure.
    pub fn is_up(&self) -> bool {
        self.cooldown.is_up()
    }

    pub fn track_url(&self, id: &str) -> String {
        format!("{}/track/{id}", self.base)
    }

    /// The search query the Android app sends: first artist, then the title.
    pub fn query(request: &StreamRequest) -> String {
        match request.artists.first().map(|artist| artist.trim()).filter(|artist| !artist.is_empty()) {
            Some(artist) => format!("{artist} {}", request.title.trim()),
            None => request.title.trim().to_owned(),
        }
    }

    async fn get(&self, url: &str, query: &[(&str, String)]) -> Result<Value, SourceError> {
        let client = self.http.client().map_err(SourceError::Network)?;
        let response = client
            .get(url)
            .query(query)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .timeout(TIMEOUT)
            .send()
            .await
            .map_err(|error| SourceError::Network(error.without_url().to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(SourceError::Status(status.as_u16()));
        }
        let text = response.text().await.map_err(|error| SourceError::Network(error.without_url().to_string()))?;
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|error| SourceError::Parse(error.to_string()))
    }

    /// Search hits for the query (the server rests after a 5xx or network error).
    pub async fn search(&self, query: &str) -> Result<Vec<MonochromeTrack>, SourceError> {
        let url = format!("{}/search/tracks", self.base);
        let result = self.get(&url, &[("q", query.to_owned()), ("limit", SEARCH_LIMIT.to_string())]).await;
        self.cooldown.record(&result);
        Ok(parse_search(&result?))
    }

    /// Is the server answering? (`SourceCheckService.checkMonochrome`)
    pub async fn check(&self) -> Result<(), SourceError> {
        self.search("test").await.map(drop)
    }

    /// The best accepted match of the request, as a stream (`None`: resting, nothing matched).
    pub async fn resolve(&self, request: &StreamRequest) -> Outcome {
        if request.title.trim().is_empty() || !self.is_up() {
            return Ok(None);
        }
        let tracks = self.search(&Self::query(request)).await?;
        let duration = (request.duration_ms > 0).then_some(request.duration_ms);
        let album = Some(request.album.as_str()).filter(|album| !album.trim().is_empty());
        let best = tracks
            .into_iter()
            .map(|track| {
                let verdict = matching::evaluate(&request.title, &request.artists, album, duration, &track.candidate);
                log::debug!(
                    "monochrome: \"{}\" vs \"{}\": {} ({:.2})",
                    request.title,
                    track.candidate.title,
                    verdict.reason,
                    verdict.score
                );
                (track, verdict)
            })
            .filter(|(_, verdict)| verdict.accepted)
            .fold(None::<(MonochromeTrack, f64)>, |best, (track, verdict)| match best {
                Some((_, score)) if score >= verdict.score => best,
                _ => Some((track, verdict.score)),
            });
        Ok(best.map(|(track, _)| self.stream(request, track)))
    }

    fn stream(&self, request: &StreamRequest, track: MonochromeTrack) -> YtStream {
        YtStream {
            video_id: request.video_id.clone(),
            url: self.track_url(&track.id),
            mime: "audio/flac".into(),
            codecs: "flac".into(),
            bitrate: i64::from(SAMPLE_RATE * BIT_DEPTH * 2),
            duration_ms: track.candidate.duration_ms.unwrap_or(request.duration_ms),
            user_agent: USER_AGENT.into(),
            source: "Monochrome".into(),
            title: track.candidate.title,
            author: track.candidate.artist.unwrap_or_default(),
            sample_rate: SAMPLE_RATE,
            bit_depth: BIT_DEPTH,
            ..Default::default()
        }
    }
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_owned()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn number(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64().or_else(|| number.as_f64().map(|float| float as i64)),
        Value::String(text) => text.trim().parse::<f64>().ok().map(|float| float as i64),
        _ => None,
    }
}

/// The hits of a `/search/tracks` answer (unplayable ones and ones without an id dropped).
pub fn parse_search(body: &Value) -> Vec<MonochromeTrack> {
    let Some(tracks) = body["tracks"].as_array() else { return Vec::new() };
    tracks
        .iter()
        .filter(|item| item["playable"].as_bool() != Some(false))
        .filter_map(|item| {
            let id = text(&item["id"]).or_else(|| text(&item["trackId"]))?;
            let artists: Vec<String> =
                item["artistNames"].as_array().map(|names| names.iter().filter_map(text).collect()).unwrap_or_default();
            // the server answers milliseconds, older builds seconds
            let duration = number(&item["duration"]).unwrap_or(0);
            let duration_ms = if duration > 10_000 { duration } else { duration * 1000 };
            Some(MonochromeTrack {
                id,
                candidate: Candidate {
                    title: item["title"].as_str().unwrap_or_default().to_owned(),
                    artist: artists.into_iter().next(),
                    album: text(&item["albumTitle"]),
                    duration_ms: (duration_ms > 0).then_some(duration_ms),
                },
            })
        })
        .collect()
}
