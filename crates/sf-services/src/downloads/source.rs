//! The lane's stream source with downloads first: a downloaded YouTube song is answered with its file
//! (`file:<path>`, which the player's opener reads from disk — `engine::youtube::player::open_local`), so it plays
//! without the network. Everything else goes to the wrapped source.
use std::sync::Arc;

use sf_player::engine::youtube::{BoxFuture, StreamRequest, StreamSource, YtStream};

use super::DownloadsService;

pub struct OfflineSource {
    downloads: Arc<DownloadsService>,
    inner: Arc<dyn StreamSource>,
}

impl OfflineSource {
    pub fn new(downloads: Arc<DownloadsService>, inner: Arc<dyn StreamSource>) -> Arc<Self> {
        Arc::new(Self { downloads, inner })
    }

    fn offline(&self, video_id: &str) -> Option<YtStream> {
        let stream = self.downloads.offline_stream(video_id)?;
        self.downloads.mark_played(video_id);
        Some(stream)
    }
}

impl StreamSource for OfflineSource {
    fn resolve(&self, video_id: String) -> BoxFuture<Result<YtStream, String>> {
        match self.offline(&video_id) {
            Some(stream) => Box::pin(async move { Ok(stream) }),
            None => self.inner.resolve(video_id),
        }
    }

    fn prefetch(&self, video_id: String) -> BoxFuture<()> {
        if self.downloads.offline_stream(&video_id).is_some() {
            return Box::pin(async {});
        }
        self.inner.prefetch(video_id)
    }

    fn invalidate(&self, video_id: &str) {
        self.inner.invalidate(video_id);
    }

    fn resolve_track(&self, request: StreamRequest) -> BoxFuture<Result<YtStream, String>> {
        match self.offline(&request.video_id) {
            Some(stream) => Box::pin(async move { Ok(stream) }),
            None => self.inner.resolve_track(request),
        }
    }

    fn prefetch_track(&self, request: StreamRequest) -> BoxFuture<()> {
        if self.downloads.offline_stream(&request.video_id).is_some() {
            return Box::pin(async {});
        }
        self.inner.prefetch_track(request)
    }
}
