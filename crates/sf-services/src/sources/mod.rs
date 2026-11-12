//! Other catalogs' copies of a YouTube Music track: the free lossless Monochrome server (FLAC) and JioSaavn
//! (320 kbps AAC). Neither needs an account or a key. Ported from the Android app's `LosslessStreamResolver` /
//! `SaavnService` (see `LOSSLESS_SAAVN_GUIDE.md` at the repository root).
//!
//! [`SourceChain`] is the [`StreamSource`] the player's YouTube lane uses. For each track it tries, in the
//! Android app's order: Monochrome (setting "lossless streams", not with the data saver), JioSaavn (setting
//! "JioSaavn streams", 96 kbps with the data saver), then the video's own YouTube stream. A source that answers
//! 5xx or fails on the network rests for [`REST`]; a found stream is kept for [`CACHE_TTL`]; a stream that
//! stopped playing sends that track straight to YouTube for a while.
//!
//! JioSaavn is also a small catalogue: [`Saavn::search_tracks`] gives `saavn:track:<id>` items (search page, when
//! "JioSaavn streams" is on), and the chain plays them from `song.getDetails` whatever the settings say.
pub mod matching;
pub mod monochrome;
pub mod saavn;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use sf_player::engine::youtube::{self, StreamRequest, StreamSource, YtStream};

pub use monochrome::Monochrome;
pub use saavn::{Saavn, SaavnQuality};

/// How long a source rests after a 5xx answer or a network error (the Android app's Monochrome back-off).
pub const REST: Duration = Duration::from_secs(5 * 60);
/// How long a found stream is reused (the desktop app keeps JioSaavn URLs 30 minutes).
pub const CACHE_TTL: Duration = Duration::from_secs(30 * 60);
/// How long a track whose other-catalog stream failed goes straight to YouTube.
pub const SKIP_TTL: Duration = Duration::from_secs(30 * 60);
const CACHE_LIMIT: usize = 200;

/// The search page with JioSaavn's songs after the catalogue's tracks; when the catalogue failed, JioSaavn's songs
/// alone (if any).
pub fn with_saavn_tracks(
    result: Result<crate::catalog::SearchResults, crate::ServiceError>,
    tracks: Vec<md3_model::TrackItem>,
) -> Result<crate::catalog::SearchResults, crate::ServiceError> {
    match result {
        Ok(mut results) => {
            results.tracks.extend(tracks);
            Ok(results)
        }
        Err(_) if !tracks.is_empty() => Ok(crate::catalog::SearchResults { tracks, ..Default::default() }),
        Err(error) => Err(error),
    }
}

/// Why a source gave no answer.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    #[error("network: {0}")]
    Network(String),
    #[error("HTTP {0}")]
    Status(u16),
    #[error("bad answer: {0}")]
    Parse(String),
}

impl SourceError {
    /// The server is down or unreachable (not merely "no such track").
    pub fn rests_source(&self) -> bool {
        match self {
            Self::Network(_) => true,
            Self::Status(status) => (500..600).contains(status),
            Self::Parse(_) => false,
        }
    }
}

/// A source's answer: a stream, nothing that matched, or an error.
pub type Outcome = Result<Option<YtStream>, SourceError>;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// "Down until": `markMonochromeDown` / `markMonochromeUp` / `isMonochromeUp`.
#[derive(Debug)]
pub struct Cooldown {
    until: Mutex<Option<Instant>>,
    rest: Duration,
}

impl Default for Cooldown {
    fn default() -> Self {
        Self::new(REST)
    }
}

impl Cooldown {
    pub fn new(rest: Duration) -> Self {
        Self { until: Mutex::new(None), rest }
    }

    pub fn is_up(&self) -> bool {
        lock(&self.until).is_none_or(|until| Instant::now() >= until)
    }

    pub fn mark_down(&self) {
        *lock(&self.until) = Some(Instant::now() + self.rest);
    }

    pub fn mark_up(&self) {
        *lock(&self.until) = None;
    }

    /// Up after an answer, down after a 5xx or a network error; other failures leave it as it is.
    pub fn record<T>(&self, result: &Result<T, SourceError>) {
        match result {
            Ok(_) => self.mark_up(),
            Err(error) if error.rests_source() => self.mark_down(),
            Err(_) => {}
        }
    }
}

/// Which sources the chain asks before YouTube.
#[derive(Debug, Default)]
struct Prefs {
    lossless: AtomicBool,
    saavn: AtomicBool,
    data_saver: AtomicBool,
}

type Flight = Shared<BoxFuture<'static, Option<YtStream>>>;

struct Inner {
    youtube: Arc<dyn StreamSource>,
    monochrome: Monochrome,
    saavn: Saavn,
    prefs: Prefs,
    /// video id → another catalog's stream, and when it was found
    cache: Mutex<HashMap<String, (YtStream, Instant)>>,
    /// video id → until when it goes straight to YouTube
    skip: Mutex<HashMap<String, Instant>>,
    flights: Mutex<HashMap<String, Flight>>,
}

/// Monochrome → JioSaavn → YouTube, as one [`StreamSource`].
#[derive(Clone)]
pub struct SourceChain {
    inner: Arc<Inner>,
}

impl SourceChain {
    /// Both other sources start switched off (as in the Android app); YouTube is always last.
    pub fn new(youtube: Arc<dyn StreamSource>, http: sf_api::Http) -> Self {
        Self::with_sources(youtube, Monochrome::new(http.clone()), Saavn::new(http))
    }

    pub fn with_sources(youtube: Arc<dyn StreamSource>, monochrome: Monochrome, saavn: Saavn) -> Self {
        Self {
            inner: Arc::new(Inner {
                youtube,
                monochrome,
                saavn,
                prefs: Prefs::default(),
                cache: Mutex::default(),
                skip: Mutex::default(),
                flights: Mutex::default(),
            }),
        }
    }

    /// Ask Monochrome for a FLAC copy first.
    pub fn set_lossless(&self, on: bool) {
        self.inner.prefs.lossless.store(on, Ordering::Relaxed);
    }

    /// Ask JioSaavn for an AAC copy before YouTube.
    pub fn set_saavn(&self, on: bool) {
        self.inner.prefs.saavn.store(on, Ordering::Relaxed);
    }

    /// No lossless streams, JioSaavn at 96 kbps.
    pub fn set_data_saver(&self, on: bool) {
        self.inner.prefs.data_saver.store(on, Ordering::Relaxed);
    }

    pub fn monochrome(&self) -> &Monochrome {
        &self.inner.monochrome
    }

    pub fn saavn(&self) -> &Saavn {
        &self.inner.saavn
    }

    /// Forget every found stream (a setting changed).
    pub fn clear(&self) {
        lock(&self.inner.cache).clear();
        lock(&self.inner.skip).clear();
    }
}

impl Inner {
    fn wants_alternatives(&self) -> bool {
        let prefs = &self.prefs;
        (prefs.lossless.load(Ordering::Relaxed) && !prefs.data_saver.load(Ordering::Relaxed))
            || prefs.saavn.load(Ordering::Relaxed)
    }

    fn cached(&self, video_id: &str) -> Option<YtStream> {
        let mut cache = lock(&self.cache);
        match cache.get(video_id) {
            Some((stream, found)) if found.elapsed() < CACHE_TTL => Some(stream.clone()),
            Some(_) => {
                cache.remove(video_id);
                None
            }
            None => None,
        }
    }

    fn store(&self, stream: &YtStream) {
        let mut cache = lock(&self.cache);
        cache.retain(|_, (_, found)| found.elapsed() < CACHE_TTL);
        while cache.len() >= CACHE_LIMIT {
            let Some(oldest) = cache.iter().min_by_key(|(_, (_, found))| *found).map(|(id, _)| id.clone()) else { break };
            cache.remove(&oldest);
        }
        cache.insert(stream.video_id.clone(), (stream.clone(), Instant::now()));
    }

    fn skipped(&self, video_id: &str) -> bool {
        let mut skip = lock(&self.skip);
        skip.retain(|_, until| Instant::now() < *until);
        skip.contains_key(video_id)
    }

    /// The other catalogs in order; errors only rest their source.
    async fn find(self: Arc<Self>, request: StreamRequest) -> Option<YtStream> {
        let prefs = &self.prefs;
        let data_saver = prefs.data_saver.load(Ordering::Relaxed);
        if prefs.lossless.load(Ordering::Relaxed) && !data_saver {
            match self.monochrome.resolve(&request).await {
                Ok(Some(stream)) => return Some(stream),
                Ok(None) => {}
                Err(error) => log::info!("monochrome: {error}"),
            }
        }
        if prefs.saavn.load(Ordering::Relaxed) {
            let quality = if data_saver { SaavnQuality::Kbps96 } else { SaavnQuality::Kbps320 };
            match self.saavn.resolve(&request, quality).await {
                Ok(Some(stream)) => return Some(stream),
                Ok(None) => {}
                Err(error) => log::info!("jiosaavn: {error}"),
            }
        }
        None
    }

    /// A `saavn:track:<id>` item from the search page: its own stream, whatever the settings say (96 kbps
    /// with the data saver).
    async fn saavn_track(&self, request: StreamRequest) -> Result<YtStream, String> {
        if let Some(stream) = self.cached(&request.video_id) {
            return Ok(stream);
        }
        let quality = if self.prefs.data_saver.load(Ordering::Relaxed) { SaavnQuality::Kbps96 } else { SaavnQuality::Kbps320 };
        match self.saavn.stream_of(&request, quality).await {
            Ok(Some(stream)) => {
                self.store(&stream);
                Ok(stream)
            }
            Ok(None) => Err(mf_i18n::t("This JioSaavn song can’t be played")),
            Err(error) => {
                log::info!("jiosaavn: {} not resolved: {error}", request.video_id);
                Err(mf_i18n::t("JioSaavn isn’t answering. Try again later"))
            }
        }
    }

    /// Another catalog's stream for the request (cached, one search per track at a time).
    async fn alternative(self: Arc<Self>, request: StreamRequest) -> Option<YtStream> {
        if request.video_id.is_empty()
            || request.title.trim().is_empty()
            || !self.wants_alternatives()
            || self.skipped(&request.video_id)
        {
            return None;
        }
        if let Some(stream) = self.cached(&request.video_id) {
            return Some(stream);
        }
        let video_id = request.video_id.clone();
        let flight = lock(&self.flights)
            .entry(video_id.clone())
            .or_insert_with(|| self.clone().find(request).boxed().shared())
            .clone();
        let found = flight.await;
        lock(&self.flights).remove(&video_id);
        if let Some(stream) = &found {
            log::info!("streams: {video_id} plays from {} ({})", stream.source, stream.codec_label());
            self.store(stream);
        }
        found
    }
}

impl StreamSource for SourceChain {
    fn resolve(&self, video_id: String) -> youtube::BoxFuture<Result<YtStream, String>> {
        self.resolve_track(StreamRequest::video(video_id))
    }

    fn prefetch(&self, video_id: String) -> youtube::BoxFuture<()> {
        self.prefetch_track(StreamRequest::video(video_id))
    }

    fn invalidate(&self, video_id: &str) {
        let inner = &self.inner;
        if youtube::is_saavn_track(video_id) {
            // the song's own URL: the retry asks for its details again
            lock(&inner.cache).remove(video_id);
            return;
        }
        if lock(&inner.cache).remove(video_id).is_some() {
            // the other catalog's URL failed: the retry and the next plays go to YouTube
            lock(&inner.skip).insert(video_id.to_owned(), Instant::now() + SKIP_TTL);
        }
        inner.youtube.invalidate(video_id);
    }

    fn resolve_track(&self, request: StreamRequest) -> youtube::BoxFuture<Result<YtStream, String>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            if youtube::is_saavn_track(&request.video_id) {
                return inner.saavn_track(request).await;
            }
            if let Some(stream) = inner.clone().alternative(request.clone()).await {
                return Ok(stream);
            }
            inner.youtube.resolve_track(request).await
        })
    }

    fn prefetch_track(&self, request: StreamRequest) -> youtube::BoxFuture<()> {
        let inner = self.inner.clone();
        Box::pin(async move {
            if youtube::is_saavn_track(&request.video_id) {
                let _ = inner.saavn_track(request).await;
                return;
            }
            if inner.clone().alternative(request.clone()).await.is_none() {
                inner.youtube.prefetch_track(request).await;
            }
        })
    }
}
