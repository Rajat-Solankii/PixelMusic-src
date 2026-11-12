//! YouTube Music playback on this computer, beside librespot: a resolved stream URL is downloaded in ranged
//! chunks (`download`), decoded in-process with symphonia (`decode`: AAC in MP4, YouTube's itag 140) and
//! written to the same output chain librespot uses — `RodioSink` behind `Tapped` (EQ, limiter, level tap,
//! volume ramp) — by a thread of its own (`player`).
//!
//! Stream resolution lives in sf-services (it needs the InnerTube client); it reaches the player through
//! [`StreamSource`]. The source may answer a video with another catalog's copy of the same track (Monochrome
//! FLAC, JioSaavn AAC: `sf_services::sources`), so the decoder picks its container by the stream's mime.
pub mod decode;
pub mod download;
pub mod player;

use std::future::Future;
use std::pin::Pin;

pub use player::{YtCommand, YtEvent, YtPlayer};

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// A directly playable audio stream of one video, with what the player response says about it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct YtStream {
    pub video_id: String,
    pub url: String,
    /// `audio/mp4`
    pub mime: String,
    /// `mp4a.40.2`
    pub codecs: String,
    pub itag: i64,
    /// bits per second
    pub bitrate: i64,
    /// bytes, when the response names it
    pub content_length: Option<u64>,
    pub duration_ms: i64,
    /// the track's loudness relative to YouTube's target, for normalisation
    pub loudness_db: Option<f64>,
    /// the User-Agent the URL was issued for
    pub user_agent: String,
    /// which request produced it (`ANDROID_VR`, `WEB_REMIX`), for the log
    pub source: String,
    pub title: String,
    pub author: String,
    pub thumbnail: String,
    /// Hz, when the source names it (lossless sources)
    pub sample_rate: u32,
    /// bits per sample, when the source names it (lossless sources)
    pub bit_depth: u32,
}

impl YtStream {
    /// A lossless stream (FLAC).
    pub fn is_lossless(&self) -> bool {
        self.codecs.eq_ignore_ascii_case("flac") || self.mime.ends_with("flac")
    }

    /// "AAC 128" (or "FLAC 16/44.1") for the player's codec label.
    pub fn codec_label(&self) -> String {
        if self.is_lossless() {
            return match (self.bit_depth, self.sample_rate) {
                (0, _) | (_, 0) => "FLAC".to_owned(),
                (bits, rate) => format!("FLAC {bits}/{}", format_khz(rate)),
            };
        }
        let codec = if self.codecs.starts_with("mp4a") || self.mime.ends_with("mp4") { "AAC" } else { "AUDIO" };
        format!("{codec} {}", (self.bitrate + 500) / 1000)
    }

    /// The now-playing quality chip: the codec, with the catalog in front when the stream is not YouTube's
    /// own ("Monochrome · FLAC 16/44.1", "JioSaavn · AAC 320").
    pub fn quality_label(&self) -> String {
        match self.source.as_str() {
            "Monochrome" | "JioSaavn" => format!("{} · {}", self.source, self.codec_label()),
            _ => self.codec_label(),
        }
    }
}

/// 44100 → "44.1", 96000 → "96".
fn format_khz(rate: u32) -> String {
    let khz = format!("{:.1}", f64::from(rate) / 1000.0);
    khz.strip_suffix(".0").map(str::to_owned).unwrap_or(khz)
}

/// What the lane knows about the track it wants a stream for: the video, and the metadata another catalog
/// matches against.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StreamRequest {
    pub video_id: String,
    pub title: String,
    /// each artist on its own
    pub artists: Vec<String>,
    pub album: String,
    /// 0 — unknown
    pub duration_ms: i64,
    pub explicit: bool,
}

impl StreamRequest {
    /// Only the video (no metadata to match with).
    pub fn video(video_id: impl Into<String>) -> Self {
        Self { video_id: video_id.into(), ..Default::default() }
    }

    /// The request for a `youtube:track:` item.
    pub fn from_track(track: &md3_model::TrackItem) -> Self {
        let mut artists: Vec<String> = track.artist_refs.iter().map(|artist| artist.name.trim().to_owned()).collect();
        artists.retain(|name| !name.is_empty());
        if artists.is_empty() {
            artists = track.artists.split(", ").map(str::trim).filter(|name| !name.is_empty()).map(str::to_owned).collect();
        }
        Self {
            video_id: lane_id(&track.uri).unwrap_or_default().to_owned(),
            title: track.title.clone(),
            artists,
            album: track.album.clone(),
            duration_ms: track.duration_ms.max(0),
            explicit: track.explicit,
        }
    }
}

/// Where the player gets stream URLs (sf-services implements it over InnerTube).
pub trait StreamSource: Send + Sync {
    /// A playable stream for the video (cached by the implementation while its URL is valid).
    fn resolve(&self, video_id: String) -> BoxFuture<Result<YtStream, String>>;
    /// Resolve the next track ahead of time and warm the connection to its host.
    fn prefetch(&self, video_id: String) -> BoxFuture<()> {
        let _ = video_id;
        Box::pin(async {})
    }
    /// The URL stopped working (expired, 403): forget it so the next resolve asks again.
    fn invalidate(&self, video_id: &str) {
        let _ = video_id;
    }
    /// A playable stream for the track, which a source may find in another catalog by its metadata; without
    /// that, the video's own stream.
    fn resolve_track(&self, request: StreamRequest) -> BoxFuture<Result<YtStream, String>> {
        self.resolve(request.video_id)
    }
    /// [`StreamSource::prefetch`] with the track's metadata.
    fn prefetch_track(&self, request: StreamRequest) -> BoxFuture<()> {
        self.prefetch(request.video_id)
    }
}

/// The output settings the YouTube player opens its sink with.
#[derive(Clone)]
pub struct YtOutput {
    /// output device name from Settings; `None` — the default device
    pub audio_device: Option<String>,
    pub buffer_ms: u32,
    pub eq: super::eq::SharedEq,
    /// scale each track to YouTube's loudness target (attenuation only)
    pub normalisation: bool,
}

impl Default for YtOutput {
    fn default() -> Self {
        Self {
            audio_device: None,
            buffer_ms: super::sink::DEFAULT_BUFFER_MS,
            eq: super::eq::shared(),
            normalisation: false,
        }
    }
}

/// The video id of a `youtube:track:<id>` URI.
pub fn video_id(uri: &str) -> Option<&str> {
    uri.strip_prefix("youtube:track:").filter(|id| !id.is_empty())
}

pub fn is_youtube_track(uri: &str) -> bool {
    video_id(uri).is_some()
}

/// A music file on this computer (sf-services `local`): `local:track:<id>`. It plays through this lane too.
pub const LOCAL_TRACK: &str = "local:track:";
/// The stream URL of a local file: this prefix and its path; the opener reads it from disk.
pub const LOCAL_FILE_URL: &str = "file:";

/// The id of a `local:track:<id>` URI.
pub fn local_id(uri: &str) -> Option<&str> {
    uri.strip_prefix(LOCAL_TRACK).filter(|id| !id.is_empty())
}

pub fn is_local_track(uri: &str) -> bool {
    local_id(uri).is_some()
}

/// A JioSaavn song (sf-services `sources::saavn`): `saavn:track:<id>`. It plays through this lane too.
pub const SAAVN_TRACK: &str = "saavn:track:";

/// The JioSaavn song id of a `saavn:track:<id>` URI.
pub fn saavn_id(uri: &str) -> Option<&str> {
    uri.strip_prefix(SAAVN_TRACK).filter(|id| !id.is_empty())
}

pub fn is_saavn_track(uri: &str) -> bool {
    saavn_id(uri).is_some()
}

/// The bare id of any lane track URI (YouTube video, local file, JioSaavn song): a URI-only item's `track_id`.
pub fn lane_track_id(uri: &str) -> Option<&str> {
    video_id(uri).or_else(|| local_id(uri)).or_else(|| saavn_id(uri))
}

/// What the lane asks its source for: a YouTube track's video id, or a local or JioSaavn track's whole URI.
pub fn lane_id(uri: &str) -> Option<&str> {
    video_id(uri).or_else(|| local_id(uri).or_else(|| saavn_id(uri)).map(|_| uri))
}

/// A track the lane plays (YouTube, a local file or JioSaavn).
pub fn is_lane_track(uri: &str) -> bool {
    lane_id(uri).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_track_uris_are_recognised() {
        assert_eq!(video_id("youtube:track:dQw4w9WgXcQ"), Some("dQw4w9WgXcQ"));
        assert_eq!(video_id("youtube:track:"), None);
        assert_eq!(video_id("spotify:track:abc"), None);
        assert!(!is_youtube_track("youtube:playlist:LM"));
        let stream = YtStream { codecs: "mp4a.40.2".into(), bitrate: 129_482, ..Default::default() };
        assert_eq!(stream.codec_label(), "AAC 129");
    }

    #[test]
    fn lossless_streams_are_labelled() {
        let mut stream = YtStream { mime: "audio/flac".into(), codecs: "flac".into(), bitrate: 1_411_200, ..Default::default() };
        assert!(stream.is_lossless());
        assert_eq!(stream.codec_label(), "FLAC");
        (stream.bit_depth, stream.sample_rate) = (16, 44_100);
        assert_eq!(stream.codec_label(), "FLAC 16/44.1");
        (stream.bit_depth, stream.sample_rate) = (24, 96_000);
        assert_eq!(stream.codec_label(), "FLAC 24/96");
    }

    #[test]
    fn other_catalogs_lead_the_quality_label() {
        let flac = YtStream { codecs: "flac".into(), sample_rate: 44_100, bit_depth: 16, source: "Monochrome".into(), ..Default::default() };
        assert_eq!(flac.quality_label(), "Monochrome · FLAC 16/44.1");
        let aac = YtStream { codecs: "mp4a.40.2".into(), bitrate: 320_000, source: "JioSaavn".into(), ..Default::default() };
        assert_eq!(aac.quality_label(), "JioSaavn · AAC 320");
        let youtube = YtStream { codecs: "mp4a.40.2".into(), bitrate: 129_482, source: "ANDROID_VR".into(), ..Default::default() };
        assert_eq!(youtube.quality_label(), "AAC 129");
    }

    #[test]
    fn saavn_tracks_play_in_the_lane() {
        assert_eq!(saavn_id("saavn:track:OqVAbcde"), Some("OqVAbcde"));
        assert!(!is_saavn_track("saavn:track:") && !is_saavn_track("youtube:track:x"));
        assert_eq!(lane_id("saavn:track:OqVAbcde"), Some("saavn:track:OqVAbcde"));
        assert!(is_lane_track("saavn:track:OqVAbcde"));
        assert_eq!(lane_track_id("saavn:track:OqVAbcde"), Some("OqVAbcde"));
        assert_eq!(lane_track_id("youtube:track:vid"), Some("vid"));
        assert_eq!(lane_track_id("local:track:f3"), Some("f3"));
        assert_eq!(lane_track_id("spotify:track:x"), None);
    }

    #[test]
    fn requests_carry_the_track_metadata() {
        let track = md3_model::TrackItem {
            title: "Song".into(),
            artists: "A, B".into(),
            album: "Album".into(),
            duration_ms: 1_000,
            explicit: true,
            uri: "youtube:track:vid".into(),
            ..Default::default()
        };
        let request = StreamRequest::from_track(&track);
        assert_eq!(request.video_id, "vid");
        assert_eq!(request.artists, ["A", "B"]);
        assert_eq!((request.album.as_str(), request.duration_ms, request.explicit), ("Album", 1_000, true));
        let refs = md3_model::TrackItem {
            artist_refs: vec![md3_model::ArtistRef { id: String::new(), name: "Only, One".into() }],
            ..track
        };
        assert_eq!(StreamRequest::from_track(&refs).artists, ["Only, One"]);
    }
}
