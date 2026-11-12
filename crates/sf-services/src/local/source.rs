//! The stream source the player's lane gets: `local:track:` items are answered with their file (the player's
//! opener reads `file:` URLs from disk), everything else goes to the wrapped YouTube source.
use std::sync::Arc;

use sf_player::engine::youtube::{BoxFuture, LOCAL_FILE_URL, StreamRequest, StreamSource, YtStream, local_id};

use super::{LocalFilesService, LocalTrack, file_url, tags};

pub struct LaneSource {
    local: Arc<LocalFilesService>,
    /// YouTube streams; `None` while YouTube Music isn't set up (local files still play)
    youtube: Option<Arc<dyn StreamSource>>,
}

impl LaneSource {
    pub fn new(local: Arc<LocalFilesService>, youtube: Option<Arc<dyn StreamSource>>) -> Arc<Self> {
        Arc::new(Self { local, youtube })
    }

    fn local(&self, id: &str) -> Result<YtStream, String> {
        let track = self.local.track(id).ok_or_else(|| mf_i18n::t("This file is no longer in your library"))?;
        if !std::path::Path::new(&track.path).is_file() {
            return Err(mf_i18n::t("The file was moved or deleted"));
        }
        Ok(stream_of(&track))
    }

    fn youtube(&self) -> Result<&Arc<dyn StreamSource>, String> {
        self.youtube.as_ref().ok_or_else(|| mf_i18n::t("Sign in to YouTube Music"))
    }
}

/// The stream the player opens for a scanned file.
pub fn stream_of(track: &LocalTrack) -> YtStream {
    let path = std::path::Path::new(&track.path);
    let bitrate = if track.duration_ms > 0 { (track.size * 8 * 1000 / track.duration_ms as u64) as i64 } else { 0 };
    YtStream {
        video_id: track.uri(),
        url: format!("{LOCAL_FILE_URL}{}", track.path),
        mime: tags::mime_of(path).to_owned(),
        codecs: track.codec.to_ascii_lowercase(),
        bitrate,
        content_length: Some(track.size),
        duration_ms: track.duration_ms,
        source: "LOCAL".into(),
        title: track.title.clone(),
        author: track.artists.join(", "),
        thumbnail: file_url(&track.cover),
        sample_rate: track.sample_rate,
        bit_depth: track.bit_depth,
        ..Default::default()
    }
}

impl StreamSource for LaneSource {
    fn resolve(&self, video_id: String) -> BoxFuture<Result<YtStream, String>> {
        if let Some(id) = local_id(&video_id) {
            let result = self.local(id);
            return Box::pin(async move { result });
        }
        match self.youtube() {
            Ok(youtube) => youtube.resolve(video_id),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }

    fn prefetch(&self, video_id: String) -> BoxFuture<()> {
        match &self.youtube {
            Some(youtube) if local_id(&video_id).is_none() => youtube.prefetch(video_id),
            _ => Box::pin(async {}),
        }
    }

    fn invalidate(&self, video_id: &str) {
        if let Some(youtube) = &self.youtube
            && local_id(video_id).is_none()
        {
            youtube.invalidate(video_id);
        }
    }

    fn resolve_track(&self, request: StreamRequest) -> BoxFuture<Result<YtStream, String>> {
        if local_id(&request.video_id).is_some() {
            return self.resolve(request.video_id);
        }
        match self.youtube() {
            Ok(youtube) => youtube.resolve_track(request),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }

    fn prefetch_track(&self, request: StreamRequest) -> BoxFuture<()> {
        match &self.youtube {
            Some(youtube) if local_id(&request.video_id).is_none() => youtube.prefetch_track(request),
            _ => Box::pin(async {}),
        }
    }
}
