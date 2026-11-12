//! Stream resolution (`StreamResolver.kt`): `POST player` for a video, then the best `audio/mp4` format (AAC, itag
//! 140 usually; symphonia has no Opus decoder).
//!
//! Tier 1 is `ANDROID_VR`, anonymous and without a PoToken: direct URLs when it works. After `LOGIN_REQUIRED` or
//! a bot check it rests for [`COOLDOWN`]. With an account, the same client is then asked as the user
//! (`ANDROID_VR_USER`): a bot check or an age wall is what signing in lifts. The last tier is the web player (`WEB_REMIX`, as the user when signed in): the
//! request carries the player script's `signatureTimestamp` and a PO token bound to the video (`potoken.rs`), the
//! chosen format's signature and `n` go through the player's own code (`cipher.rs`), and the URL gets `&pot=`. It
//! rests after two technical failures in a row (script, BotGuard, HTTP), not after a song's own refusal. Results
//! are cached until shortly before the URL's `expire=`; concurrent requests for one video share a resolution.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use serde_json::{Value, json};
use sf_player::engine::youtube::{StreamSource, YtStream};

use super::cipher::Cipher;
use super::client::{ANDROID_VR, ANDROID_VR_USER, ClientPreset, InnerTube, Login, WEB_REMIX};
use super::jsworker::JsWorker;
use super::potoken::PoTokens;
use super::{Result, YouTubeError, YouTubeService};

/// How long a tier rests after it was refused.
pub const COOLDOWN: Duration = Duration::from_secs(15 * 60);
/// Most streams kept.
const CACHE_LIMIT: usize = 200;
/// A URL counts as expired this long before its `expire=`.
const EXPIRY_MARGIN_SECS: u64 = 600;
/// Validity of a URL without `expire=`.
const DEFAULT_TTL_SECS: u64 = 3600;
/// Bitrate the data saver aims for.
const DATA_SAVER_BPS: i64 = 64_000;
/// Technical failures of the web tier in a row before it rests.
const WEB_FAILURE_LIMIT: u32 = 2;

type Flight = Shared<BoxFuture<'static, Result<YtStream>>>;

struct Cached {
    stream: YtStream,
    /// unix seconds
    valid_until: u64,
    added: Instant,
}

#[derive(Default)]
struct Tiers {
    android_vr_until: Option<Instant>,
    /// what ANDROID_VR said when it began to rest, reported while it rests
    android_vr_reason: String,
    /// the signed-in ANDROID_VR, after a bot check of its own
    android_vr_user_until: Option<Instant>,
    web_failures: u32,
    web_until: Option<Instant>,
}

/// The resolver's state, shared with the futures it hands out.
pub(crate) struct Streams {
    cache: Mutex<HashMap<String, Cached>>,
    flights: Mutex<HashMap<String, Flight>>,
    tiers: Mutex<Tiers>,
    data_saver: AtomicBool,
    /// the web player's URL transform
    cipher: Cipher,
    tokens: PoTokens,
}

impl Default for Streams {
    fn default() -> Self {
        // the player script and BotGuard share one JS thread
        let js = Arc::new(JsWorker::default());
        Self {
            cache: Mutex::default(),
            flights: Mutex::default(),
            tiers: Mutex::default(),
            data_saver: AtomicBool::default(),
            cipher: Cipher::new(js.clone()),
            tokens: PoTokens::new(js),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

impl Streams {
    fn cached(&self, video_id: &str, now: u64) -> Option<YtStream> {
        let mut cache = lock(&self.cache);
        match cache.get(video_id) {
            Some(entry) if entry.valid_until > now => Some(entry.stream.clone()),
            Some(_) => {
                cache.remove(video_id);
                None
            }
            None => None,
        }
    }

    fn store(&self, stream: &YtStream, now: u64) {
        let valid_until = valid_until(&stream.url, now);
        if valid_until <= now {
            return;
        }
        let mut cache = lock(&self.cache);
        cache.retain(|_, entry| entry.valid_until > now);
        while cache.len() >= CACHE_LIMIT {
            let Some(oldest) = cache.iter().min_by_key(|(_, entry)| entry.added).map(|(id, _)| id.clone()) else { break };
            cache.remove(&oldest);
        }
        cache.insert(stream.video_id.clone(), Cached { stream: stream.clone(), valid_until, added: Instant::now() });
    }

    /// Another identity (signed in, out, another account): what the tiers refused and what was resolved belonged
    /// to the previous one.
    pub(crate) fn account_changed(&self) {
        *lock(&self.tiers) = Tiers::default();
        lock(&self.cache).clear();
    }

    fn invalidate(&self, video_id: &str) {
        let removed = lock(&self.cache).remove(video_id);
        self.tokens.forget(video_id);
        // a refused web URL may come from a stale player script (its signature or `n` transform): load it again
        if removed.is_some_and(|entry| entry.stream.source == WEB_REMIX.name) {
            self.cipher.reset();
        }
    }
}

/// Until when (unix seconds) a stream URL can be used: its `expire=` less the margin, or an hour from now.
pub fn valid_until(url: &str, now: u64) -> u64 {
    url_param(url, "expire")
        .and_then(|value| value.parse::<u64>().ok())
        .map_or(now + DEFAULT_TTL_SECS, |expire| expire.saturating_sub(EXPIRY_MARGIN_SECS))
}

fn url_param<'a>(url: &'a str, name: &str) -> Option<&'a str> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
}

/// Why a player response gave no stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// `playabilityStatus.status` (`LOGIN_REQUIRED`, `UNPLAYABLE`, ...), or `NO_STREAM`
    pub status: String,
    pub reason: String,
}

impl Refusal {
    /// A bot check or a sign-in wall: the anonymous tier should rest.
    pub fn is_bot_check(&self) -> bool {
        let reason = self.reason.to_lowercase();
        self.status == "LOGIN_REQUIRED"
            || reason.contains("not a bot")
            || reason.contains("not a robot")
            || reason.contains("unusual traffic")
    }

    fn into_error(self) -> YouTubeError {
        YouTubeError::Unplayable(self.reason)
    }
}

/// The ciphered parts of a format without a plain URL (`signatureCipher`, form-encoded): `s` deciphers into the
/// query parameter `sp` of `url`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ciphered {
    pub url: String,
    pub sp: String,
    pub s: String,
}

impl Ciphered {
    pub fn parse(text: &str) -> Option<Self> {
        let parsed = reqwest::Url::parse(&format!("http://cipher/?{text}")).ok()?;
        let mut out = Self { sp: "signature".into(), ..Default::default() };
        for (key, value) in parsed.query_pairs() {
            match key.as_ref() {
                "url" => out.url = value.into_owned(),
                "sp" => out.sp = value.into_owned(),
                "s" => out.s = value.into_owned(),
                _ => {}
            }
        }
        (!out.url.is_empty() && !out.s.is_empty()).then_some(out)
    }
}

/// The stream a player response offers: the best direct `audio/mp4` format (nearest 64 kbps with
/// `data_saver`), described with the response's video details.
pub fn parse_player(
    response: &Value,
    preset: &ClientPreset,
    data_saver: bool,
) -> std::result::Result<YtStream, Refusal> {
    choose(response, preset, data_saver, false).map(|(stream, _)| stream)
}

/// As `parse_player`; with `ciphered`, formats with only a `signatureCipher` count too, and the chosen one's
/// cipher comes back (the stream's `url` is then empty).
pub fn choose(
    response: &Value,
    preset: &ClientPreset,
    data_saver: bool,
    ciphered: bool,
) -> std::result::Result<(YtStream, Option<Ciphered>), Refusal> {
    let playability = &response["playabilityStatus"];
    let status = playability["status"].as_str().unwrap_or("UNKNOWN");
    if status != "OK" {
        let reason = playability["reason"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| {
                playability["errorScreen"]["playerErrorMessageRenderer"]["subreason"]["runs"][0]["text"]
                    .as_str()
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        return Err(Refusal { status: status.to_owned(), reason });
    }
    let no_stream = || Refusal { status: "NO_STREAM".into(), reason: String::new() };
    let formats = response["streamingData"]["adaptiveFormats"].as_array().ok_or_else(no_stream)?;
    let cipher_of = |format: &Value| format["signatureCipher"].as_str().or(format["cipher"].as_str()).and_then(Ciphered::parse);
    let playable = formats.iter().filter(|format| {
        format["mimeType"].as_str().is_some_and(|mime| mime.starts_with("audio/mp4"))
            && (format["url"].as_str().is_some_and(|url| !url.is_empty()) || (ciphered && cipher_of(format).is_some()))
    });
    let bitrate = |format: &&Value| format["bitrate"].as_i64().unwrap_or(0);
    let chosen = if data_saver {
        playable.min_by_key(|format| (bitrate(format) - DATA_SAVER_BPS).abs())
    } else {
        playable.max_by_key(bitrate)
    }
    .ok_or_else(no_stream)?;

    let details = &response["videoDetails"];
    let mime_type = chosen["mimeType"].as_str().unwrap_or_default();
    let (mime, codecs) = match mime_type.split_once(';') {
        Some((mime, rest)) => {
            let codecs = rest.trim().strip_prefix("codecs=").unwrap_or(rest.trim()).trim_matches('"');
            (mime.trim().to_owned(), codecs.to_owned())
        }
        None => (mime_type.to_owned(), String::new()),
    };
    let number = |value: &Value| value.as_i64().or_else(|| value.as_str().and_then(|text| text.parse().ok()));
    let duration_ms = number(&chosen["approxDurationMs"])
        .or_else(|| number(&details["lengthSeconds"]).map(|seconds| seconds * 1000))
        .unwrap_or(0);
    let thumbnail = details["thumbnail"]["thumbnails"]
        .as_array()
        .and_then(|list| list.iter().max_by_key(|item| item["width"].as_i64().unwrap_or(0)))
        .and_then(|item| item["url"].as_str())
        .unwrap_or_default();
    let cipher = if chosen["url"].as_str().is_some_and(|url| !url.is_empty()) { None } else { cipher_of(chosen) };
    let stream = YtStream {
        video_id: details["videoId"].as_str().unwrap_or_default().to_owned(),
        url: chosen["url"].as_str().unwrap_or_default().to_owned(),
        mime,
        codecs,
        itag: chosen["itag"].as_i64().unwrap_or(0),
        bitrate: bitrate(&chosen),
        content_length: number(&chosen["contentLength"]).and_then(|length| u64::try_from(length).ok()),
        duration_ms,
        loudness_db: chosen["loudnessDb"].as_f64().or_else(|| response["playerConfig"]["audioConfig"]["loudnessDb"].as_f64()),
        user_agent: preset.user_agent.to_owned(),
        source: preset.name.to_owned(),
        title: details["title"].as_str().unwrap_or_default().to_owned(),
        author: details["author"].as_str().unwrap_or_default().to_owned(),
        thumbnail: thumbnail.to_owned(),
        ..Default::default()
    };
    Ok((stream, cipher))
}

/// One `ANDROID_VR` player request. A refusal comes with the bot check's reason when it was one.
async fn vr_request(
    client: &InnerTube,
    preset: &'static ClientPreset,
    login: Login,
    video_id: &str,
    data_saver: bool,
) -> std::result::Result<YtStream, (YouTubeError, Option<String>)> {
    let who = if login == Login::Never { "anonymous" } else { "signed in" };
    match client.post(preset, "player", json!({ "videoId": video_id }), login).await {
        Ok(response) => match parse_player(&response, preset, data_saver) {
            Ok(stream) => Ok(with_id(stream, video_id)),
            Err(refusal) => {
                log_refusal(preset, video_id, &refusal, &response);
                let bot_check = refusal.is_bot_check().then(|| refusal.reason.clone());
                if bot_check.is_some() {
                    log::info!("youtube streams: ANDROID_VR ({who}) refused ({}), resting", refusal.status);
                }
                Err((refusal.into_error(), bot_check))
            }
        },
        Err(error) => {
            log::debug!("youtube streams: {video_id}: ANDROID_VR ({who}) request failed: {error:?}");
            Err((error, None))
        }
    }
}

/// Asks the tiers in turn.
async fn fetch(client: Arc<InnerTube>, streams: Arc<Streams>, video_id: String) -> Result<YtStream> {
    let data_saver = streams.data_saver.load(Ordering::Relaxed);
    let mut last_error = None;

    let vr_resting = {
        let tiers = lock(&streams.tiers);
        let resting = tiers.android_vr_until.is_some_and(|until| Instant::now() < until);
        if resting {
            log::debug!("youtube streams: {video_id}: ANDROID_VR is resting");
            // otherwise a song tried while it rests only gets the generic "can't be played"
            last_error = Some(YouTubeError::Unplayable(tiers.android_vr_reason.clone()));
        }
        resting
    };
    if !vr_resting {
        match vr_request(&client, &ANDROID_VR, Login::Never, &video_id, data_saver).await {
            Ok(stream) => return Ok(stream),
            Err((error, bot_check)) => {
                if let Some(reason) = bot_check {
                    let mut tiers = lock(&streams.tiers);
                    tiers.android_vr_until = Some(Instant::now() + COOLDOWN);
                    tiers.android_vr_reason = reason;
                }
                last_error = Some(error);
            }
        }
    }

    // the same client as the user: the account is what a bot check and an age wall ask for
    let user_resting = lock(&streams.tiers).android_vr_user_until.is_some_and(|until| Instant::now() < until);
    if client.signed_in() && !user_resting {
        match vr_request(&client, &ANDROID_VR_USER, Login::Required, &video_id, data_saver).await {
            Ok(stream) => return Ok(stream),
            Err((error, bot_check)) => {
                if bot_check.is_some() {
                    lock(&streams.tiers).android_vr_user_until = Some(Instant::now() + COOLDOWN);
                }
                // what the account was told says more than the anonymous answer; a failed request does not
                if last_error.is_none() || matches!(error, YouTubeError::Unplayable(ref reason) if !reason.is_empty()) {
                    last_error = Some(error);
                }
            }
        }
    }

    let web_resting = lock(&streams.tiers).web_until.is_some_and(|until| Instant::now() < until);
    if web_resting {
        log::debug!("youtube streams: {video_id}: the web tier is resting");
    } else {
        match web_tier(&client, &streams, &video_id, data_saver).await {
            Ok(stream) => {
                lock(&streams.tiers).web_failures = 0;
                return Ok(stream);
            }
            Err(WebError::Refused(error)) => {
                // the anonymous tier's reason says more than "no stream"
                if last_error.is_none() || !matches!(error, YouTubeError::Unplayable(ref reason) if reason.is_empty()) {
                    last_error = Some(error);
                }
            }
            Err(WebError::Failed(error)) => {
                let mut tiers = lock(&streams.tiers);
                tiers.web_failures += 1;
                if tiers.web_failures >= WEB_FAILURE_LIMIT {
                    log::info!("youtube streams: the web tier failed {} times, resting", tiers.web_failures);
                    tiers.web_failures = 0;
                    tiers.web_until = Some(Instant::now() + COOLDOWN);
                }
                if last_error.is_none() {
                    last_error = Some(error);
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| YouTubeError::Unplayable(String::new())))
}

enum WebError {
    /// the song's own refusal (or no usable format)
    Refused(YouTubeError),
    /// the tier itself failed: player script, BotGuard, HTTP
    Failed(YouTubeError),
}

/// The web player's stream: `signatureTimestamp` and a PO token in the request, the format's URL deciphered by
/// the player script, `&pot=` added.
async fn web_tier(client: &InnerTube, streams: &Streams, video_id: &str, data_saver: bool) -> std::result::Result<YtStream, WebError> {
    let failed = |what: &'static str| {
        move |error: String| {
            log::info!("youtube streams: {video_id}: {what} failed: {error}");
            WebError::Failed(YouTubeError::Unexpected(what))
        }
    };
    let sts = streams.cipher.load(client).await.map_err(failed("the player script"))?;
    let token = streams.tokens.token(client, video_id).await.map_err(failed("the PO token"))?;
    let mut body = json!({
        "videoId": video_id,
        "playbackContext": { "contentPlaybackContext": { "signatureTimestamp": sts } },
    });
    if let Some(token) = &token {
        body["serviceIntegrityDimensions"] = json!({ "poToken": token });
    }
    let response = client.post(&WEB_REMIX, "player", body, Login::IfSignedIn).await.map_err(|error| {
        log::debug!("youtube streams: {video_id}: WEB_REMIX request failed: {error:?}");
        WebError::Failed(error)
    })?;
    let (mut stream, cipher) = choose(&response, &WEB_REMIX, data_saver, true).map_err(|refusal| {
        log_refusal(&WEB_REMIX, video_id, &refusal, &response);
        WebError::Refused(refusal.into_error())
    })?;
    let cipher = cipher.unwrap_or_else(|| Ciphered { url: stream.url.clone(), sp: String::new(), s: String::new() });
    let mut url = streams.cipher.decipher(&cipher.url, &cipher.sp, &cipher.s).await.map_err(failed("deciphering the stream"))?;
    match &token {
        Some(token) => {
            url.push_str("&pot=");
            url.push_str(token);
        }
        None => log::info!("youtube streams: {video_id}: no PO token; the stream may stop after the first megabyte"),
    }
    stream.url = url;
    log::debug!("youtube streams: {video_id}: web stream itag {} (signed in: {})", stream.itag, client.signed_in());
    Ok(with_id(stream, video_id))
}

/// The playability status and reason of a refused player response (debug log); for `NO_STREAM`, how many audio
/// formats only came with a `signatureCipher` (they would need the player's decipher code).
fn log_refusal(preset: &ClientPreset, video_id: &str, refusal: &Refusal, response: &Value) {
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    let ciphered = response["streamingData"]["adaptiveFormats"].as_array().map_or(0, |formats| {
        formats
            .iter()
            .filter(|format| {
                format["mimeType"].as_str().is_some_and(|mime| mime.starts_with("audio/"))
                    && format.get("url").is_none()
                    && (format.get("signatureCipher").is_some() || format.get("cipher").is_some())
            })
            .count()
    });
    log::debug!(
        "youtube streams: {video_id}: {} playability {} reason {:?} (ciphered audio formats: {ciphered})",
        preset.name,
        refusal.status,
        refusal.reason
    );
}

fn with_id(mut stream: YtStream, video_id: &str) -> YtStream {
    if stream.video_id.is_empty() {
        stream.video_id = video_id.to_owned();
    }
    stream
}

async fn resolve(client: Arc<InnerTube>, streams: Arc<Streams>, video_id: String) -> Result<YtStream> {
    if let Some(stream) = streams.cached(&video_id, unix_now()) {
        return Ok(stream);
    }
    let flight = {
        let mut flights = lock(&streams.flights);
        flights
            .entry(video_id.clone())
            .or_insert_with(|| fetch(client, streams.clone(), video_id.clone()).boxed().shared())
            .clone()
    };
    let result = flight.await;
    {
        let mut flights = lock(&streams.flights);
        flights.remove(&video_id);
        if let Ok(stream) = &result {
            streams.store(stream, unix_now());
        }
    }
    result
}

/// Opens the first kilobyte of the stream so the connection to its host is warm; a refused URL is forgotten.
async fn preheat(client: Arc<InnerTube>, streams: Arc<Streams>, video_id: String) {
    let Ok(stream) = resolve(client.clone(), streams.clone(), video_id.clone()).await else { return };
    let Ok(http) = client.http_client() else { return };
    let request = http
        .get(&stream.url)
        .header(reqwest::header::RANGE, "bytes=0-1024")
        .header(reqwest::header::USER_AGENT, &stream.user_agent)
        .timeout(Duration::from_secs(10));
    match request.send().await {
        Ok(response) if matches!(response.status().as_u16(), 403 | 410) => {
            log::info!("youtube streams: {video_id} refused on preheat ({})", response.status());
            streams.invalidate(&video_id);
        }
        Ok(_) => {}
        Err(error) => log::debug!("youtube streams: preheat failed: {}", error.without_url()),
    }
}

impl YouTubeService {
    /// A directly playable stream of the video (cached while its URL is valid).
    pub async fn resolve_stream(&self, video_id: &str) -> Result<YtStream> {
        resolve(self.client.clone(), self.streams.clone(), video_id.to_owned()).await
    }

    /// Resolves the video ahead of time and warms the connection to its host.
    pub async fn preheat_stream(&self, video_id: &str) {
        preheat(self.client.clone(), self.streams.clone(), video_id.to_owned()).await
    }

    /// Forget a stream that stopped working.
    pub fn invalidate_stream(&self, video_id: &str) {
        self.streams.invalidate(video_id);
    }

    /// Where web-player PO tokens come from: an external bgutil-style provider (`Some(base URL)`), or the built-in
    /// BotGuard minter (`None`). `MATERIALFY_POT_PROVIDER` sets the provider at start.
    pub fn set_po_token_server(&self, server: Option<String>) {
        self.streams.tokens.set_server(server);
    }

    /// Prefer the stream nearest 64 kbps.
    pub fn set_data_saver(&self, on: bool) {
        self.streams.data_saver.store(on, Ordering::Relaxed);
    }
}

impl StreamSource for YouTubeService {
    fn resolve(&self, video_id: String) -> sf_player::engine::youtube::BoxFuture<std::result::Result<YtStream, String>> {
        let (client, streams) = (self.client.clone(), self.streams.clone());
        Box::pin(async move { resolve(client, streams, video_id).await.map_err(|error| error.to_string()) })
    }

    fn prefetch(&self, video_id: String) -> sf_player::engine::youtube::BoxFuture<()> {
        Box::pin(preheat(self.client.clone(), self.streams.clone(), video_id))
    }

    fn invalidate(&self, video_id: &str) {
        self.invalidate_stream(video_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::youtube::parser_tests::fixture;

    /// The recorded VR response with its URLs made valid for another few hours.
    pub(crate) fn android_vr_response() -> Value {
        let text = fixture("player_response_android_vr.json").to_string();
        let future = (unix_now() + 6 * 3600).to_string();
        serde_json::from_str(&text.replace("expire=1710003600", &format!("expire={future}"))).unwrap()
    }

    #[test]
    fn picks_the_aac_stream() {
        let stream = parse_player(&android_vr_response(), &ANDROID_VR, false).unwrap();
        assert_eq!(stream.itag, 140);
        assert_eq!(stream.mime, "audio/mp4");
        assert_eq!(stream.codecs, "mp4a.40.2");
        assert_eq!(stream.bitrate, 129_482);
        assert_eq!(stream.content_length, Some(5_690_120));
        assert_eq!(stream.duration_ms, 355_000);
        assert_eq!(stream.loudness_db, Some(-3.2));
        assert_eq!(stream.video_id, "fJ9rUzIMcZQ");
        assert_eq!((stream.title.as_str(), stream.author.as_str()), ("Bohemian Rhapsody", "Queen"));
        assert_eq!(stream.thumbnail, "https://i.ytimg.com/vi/fJ9rUzIMcZQ/hqdefault.jpg");
        assert_eq!(stream.source, "ANDROID_VR");
        assert_eq!(stream.user_agent, ANDROID_VR.user_agent);
        assert!(stream.url.starts_with("https://rr1---sn-4g5ednls.googlevideo.com/videoplayback?"));
        assert_eq!(stream.codec_label(), "AAC 129");
    }

    #[test]
    fn data_saver_takes_the_nearest_64k() {
        let mut response = android_vr_response();
        let formats = response["streamingData"]["adaptiveFormats"].as_array_mut().unwrap();
        let mut low = formats[1].clone();
        low["itag"] = json!(139);
        low["bitrate"] = json!(50_000);
        formats.push(low);
        assert_eq!(parse_player(&response, &ANDROID_VR, true).unwrap().itag, 139);
        assert_eq!(parse_player(&response, &ANDROID_VR, false).unwrap().itag, 140);
    }

    #[test]
    fn ciphered_formats_are_no_stream() {
        let refusal = parse_player(&fixture("player_response_web_remix.json"), &WEB_REMIX, false).unwrap_err();
        assert_eq!(refusal.status, "NO_STREAM");
        assert!(!refusal.is_bot_check());
    }

    #[test]
    fn ciphered_formats_come_with_their_cipher_for_the_web_tier() {
        let mut response = fixture("player_response_web_remix.json");
        response["streamingData"]["adaptiveFormats"][0]["mimeType"] = json!("audio/mp4; codecs=\"mp4a.40.2\"");
        let (stream, cipher) = choose(&response, &WEB_REMIX, false, true).unwrap();
        assert!(stream.url.is_empty());
        let cipher = cipher.expect("cipher");
        assert_eq!(cipher.sp, "sig");
        assert_eq!(cipher.s, "mR3k...cipher_sig...");
        assert_eq!(cipher.url, "https://rr1---sn-4g5ednls.googlevideo.com/videoplayback?expire=1710003600&itag=251");
        assert!(parse_player(&response, &WEB_REMIX, false).is_err(), "ANDROID_VR's reading takes plain URLs only");
        assert_eq!(Ciphered::parse("sp=sig&url=x"), None);
        assert_eq!(Ciphered::parse("s=a%2Bb&url=u").unwrap().sp, "signature");
        assert_eq!(Ciphered::parse("s=a%2Bb&url=u").unwrap().s, "a+b");
    }

    #[test]
    fn refusals_are_classified() {
        let errors = fixture("error_responses.json");
        let age = parse_player(&errors["unplayable_age_restricted"], &ANDROID_VR, false).unwrap_err();
        assert_eq!(age.status, "UNPLAYABLE");
        assert_eq!(age.reason, "Sign in to confirm your age");
        assert!(!age.is_bot_check());
        let bot = parse_player(&errors["bot_challenge_login_required"], &ANDROID_VR, false).unwrap_err();
        assert!(bot.is_bot_check());
        assert_eq!(bot.clone().into_error(), YouTubeError::Unplayable(bot.reason));
    }

    #[test]
    fn validity_follows_expire() {
        assert_eq!(valid_until("https://x/videoplayback?expire=5000&ei=1", 100), 4_400);
        assert_eq!(valid_until("https://x/videoplayback?ei=1", 100), 3_700);
        assert_eq!(valid_until("https://x/videoplayback?notexpire=9&ei=1", 100), 3_700);
    }

    #[test]
    fn the_cache_drops_expired_then_oldest() {
        let streams = Streams::default();
        let now = unix_now();
        let stream = |id: &str, expire: u64| YtStream {
            video_id: id.into(),
            url: format!("https://x/videoplayback?expire={expire}"),
            ..Default::default()
        };
        // already inside the margin: never stored
        streams.store(&stream("soon", now + 100), now);
        assert!(streams.cached("soon", now).is_none());
        for index in 0..CACHE_LIMIT {
            streams.store(&stream(&format!("v{index}"), now + 7_200), now);
        }
        streams.store(&stream("newest", now + 7_200), now);
        assert_eq!(lock(&streams.cache).len(), CACHE_LIMIT);
        assert!(streams.cached("v0", now).is_none(), "the oldest went");
        assert!(streams.cached("newest", now).is_some());
        // past its validity
        assert!(streams.cached("newest", now + 7_000).is_none());
        streams.store(&stream("again", now + 7_200), now);
        streams.invalidate("again");
        assert!(streams.cached("again", now).is_none());
    }
}
