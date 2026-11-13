//! Тексты (исполнитель «services-extra»): по PlayerEvent::TrackChanged с задержкой 350 мс (пролистывание подряд
//! не запускает поиск на каждый трек) → mf-lyrics: кэш → Spotify первым (ExtraLrc: сессия librespot, ответ
//! color-lyrics разбирает mf-lyrics) → NetEase/Musixmatch/LRCLIB. Выпуски — без текстов. Ответ для сменившегося трека отбрасывается.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::FutureExt;
use md3_model::TrackItem;
use sf_player::{PlayerEvent, PlayerHandle};
use tokio::runtime::Handle;
use tokio::sync::{broadcast, watch};
use mf_lyrics::{ExtraLrc, LyricsResult};

/// Задержка поиска после смены трека.
pub const DEBOUNCE_MS: u64 = 350;
/// Ожидание текста Spotify через сессию.
const SPOTIFY_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LyricsStatus {
    #[default]
    Idle,
    Loading,
    Ok,
    NotFound,
    Error,
}

impl LyricsStatus {
    /// "idle" | "loading" | "ok" | "notfound" | "error"
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Loading => "loading",
            Self::Ok => "ok",
            Self::NotFound => "notfound",
            Self::Error => "error",
        }
    }

    /// Статус по результату: ok | notfound (источники ответили «нет») | error (источник ошибся).
    pub fn of(result: &LyricsResult) -> Self {
        if result.ok {
            Self::Ok
        } else if result.details.is_empty() {
            Self::NotFound
        } else {
            Self::Error
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricsSnapshot {
    pub track_id: String,
    pub status: LyricsStatus,
    /// "Spotify" | "NetEase" | "Musixmatch" | "LRCLIB" | "YouTube Music"
    pub source: String,
    /// "word" | "line" | "plain" (YouTube Music: untimed lines)
    pub kind: String,
    pub lines: Arc<Vec<mf_lyrics::Line>>,
}

pub struct LyricsService {
    rt: Handle,
    player: PlayerHandle,
    fetcher: mf_lyrics::LyricsFetcher,
    snapshot: watch::Sender<LyricsSnapshot>,
    generation: AtomicU64,
    /// YouTube Music lyrics, the last source for `youtube:track:` songs (`set_youtube`)
    youtube: std::sync::OnceLock<Arc<crate::youtube::YouTubeService>>,
    me: Weak<LyricsService>,
}

impl LyricsService {
    pub fn new(rt: Handle, player: PlayerHandle, fetcher: mf_lyrics::LyricsFetcher) -> Arc<Self> {
        let mut events = player.subscribe().events;
        let service = Arc::new_cyclic(|me| Self {
            rt: rt.clone(),
            player,
            fetcher,
            snapshot: watch::channel(LyricsSnapshot::default()).0,
            generation: AtomicU64::new(0),
            youtube: std::sync::OnceLock::new(),
            me: me.clone(),
        });
        let weak = Arc::downgrade(&service);
        rt.spawn(async move {
            let mut deadline: Option<tokio::time::Instant> = None;
            loop {
                let at = deadline;
                let wait = async move {
                    match at {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    event = events.recv() => match event {
                        Ok(PlayerEvent::TrackChanged) | Err(broadcast::error::RecvError::Lagged(_)) => {
                            deadline = Some(tokio::time::Instant::now() + Duration::from_millis(DEBOUNCE_MS));
                        }
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Closed) => return,
                    },
                    () = wait => {
                        deadline = None;
                        let Some(me) = weak.upgrade() else { return };
                        me.load(false);
                    }
                }
            }
        });
        service
    }

    pub fn snapshot(&self) -> LyricsSnapshot {
        self.snapshot.borrow().clone()
    }

    pub fn watch(&self) -> watch::Receiver<LyricsSnapshot> {
        self.snapshot.subscribe()
    }

    /// Повторить для текущего трека (после ошибки).
    pub fn retry(&self) {
        // a cached "not found" would come back as is: ask the sources again
        if let Some(track) = self.player.snapshot().track {
            let _ = std::fs::remove_file(self.fetcher.cache_path(&query_for(&track).track_id));
        }
        self.load(true);
    }

    /// LRCLIB search for the Search Lyrics dialog (the user's own query).
    pub async fn search(&self, query: &str) -> Result<Vec<mf_lyrics::sources::SearchHit>, String> {
        self.fetcher.search(query).await
    }

    /// Show this text for the playing song and keep it in the cache in place of what the sources found: a picked
    /// search hit (`source` "LRCLIB") or an imported .lrc/.txt file. Timed text plays in sync, other text as plain.
    pub fn apply_text(&self, source: &str, text: &str) -> Result<(), String> {
        let track = self.player.snapshot().track.filter(|t| !t.track_id.is_empty());
        let Some(track) = track else { return Err(mf_i18n::t("Nothing is playing")) };
        let query = query_for(&track);
        let result = mf_lyrics::result_from_text(source, text, query.duration)
            .ok_or_else(|| mf_i18n::t("There are no lyrics in this text"))?;
        // a load still running for this song must not replace the chosen text
        self.generation.fetch_add(1, Ordering::SeqCst);
        let (fetcher, cached) = (self.fetcher.clone(), result.clone());
        self.rt.spawn(async move {
            if let Err(e) = fetcher.write_cache(&query.track_id, &cached).await {
                tracing::warn!("lyrics: chosen text for {} not cached: {e}", query.track_id);
            }
        });
        self.set(track.track_id.clone(), LyricsStatus::Ok, Some(result));
        Ok(())
    }

    /// The shown lyrics as an .lrc file (plain lyrics without stamps); None when there are none.
    pub fn lrc(&self) -> Option<String> {
        let current = self.snapshot.borrow();
        (current.status == LyricsStatus::Ok && !current.lines.is_empty()).then(|| mf_lyrics::to_lrc(&current.lines, &current.kind))
    }

    fn load(&self, force: bool) {
        let track = self.player.snapshot().track;
        let track_id = track.as_ref().map(|t| t.track_id.clone()).unwrap_or_default();
        if !force {
            let current = self.snapshot.borrow();
            let settled = matches!(current.status, LyricsStatus::Ok | LyricsStatus::Loading | LyricsStatus::NotFound);
            if current.track_id == track_id && settled {
                return;
            }
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let Some(track) = track.filter(|_| !track_id.is_empty()) else {
            self.set(track_id, LyricsStatus::Idle, None);
            return;
        };
        if track.is_episode {
            self.set(track_id, LyricsStatus::NotFound, None);
            return;
        }
        self.set(track_id.clone(), LyricsStatus::Loading, None);
        let Some(me) = self.me.upgrade() else { return };
        let task = self.rt.spawn({
            let me = me.clone();
            async move { me.fetch(&track).await }
        });
        self.rt.spawn(async move {
            let result = task.await;
            if me.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            match result {
                Ok(result) => {
                    let status = LyricsStatus::of(&result);
                    tracing::info!("lyrics {track_id}: {} {}", if result.source.is_empty() { "—" } else { &result.source }, result.kind);
                    me.set(track_id, status, (status == LyricsStatus::Ok).then_some(result));
                }
                Err(e) => {
                    tracing::warn!("lyrics failed to load: {e}");
                    me.set(track_id, LyricsStatus::Error, None);
                }
            }
        });
    }

    /// Give the service YouTube Music (Services::start); without it YouTube songs use the other sources only.
    pub fn set_youtube(&self, youtube: Arc<crate::youtube::YouTubeService>) {
        let _ = self.youtube.set(youtube);
    }

    /// Кэш → Spotify → остальные источники mf-lyrics.
    async fn fetch(&self, track: &TrackItem) -> LyricsResult {
        let query = query_for(track);
        if let Some(video_id) = track.uri.strip_prefix("youtube:track:") {
            return self.fetch_youtube(&query, video_id).await;
        }
        let extra = spotify_json(self.player.clone(), track.play_uri());
        self.fetcher.fetch_cached(&query, Some(extra)).await
    }

    /// A YouTube song: the synced sources of mf-lyrics by title and artist (no Spotify), then YouTube Music's
    /// own lyrics, which are plain text (kind "plain": lines without timing).
    async fn fetch_youtube(&self, query: &mf_lyrics::LyricsQuery, video_id: &str) -> LyricsResult {
        let found = self.fetcher.fetch_cached(query, None).await;
        if found.ok {
            return found;
        }
        let Some(youtube) = self.youtube.get() else { return found };
        match youtube.lyrics(video_id).await {
            Ok(Some(text)) => LyricsResult::found(YOUTUBE_SOURCE, "plain", plain_lines(&text)),
            Ok(None) => found,
            Err(error) => {
                tracing::debug!("YouTube lyrics: {error:?}");
                found
            }
        }
    }

    fn set(&self, track_id: String, status: LyricsStatus, result: Option<LyricsResult>) {
        let result = result.unwrap_or_default();
        self.snapshot.send_replace(LyricsSnapshot {
            track_id,
            status,
            source: result.source,
            kind: result.kind,
            lines: Arc::new(result.lines),
        });
    }
}

/// Ответ color-lyrics Spotify через сессию librespot. Без сессии (играет другое устройство) — «нет текста», не ошибка.
fn spotify_json(player: PlayerHandle, uri: String) -> ExtraLrc {
    Arc::new(move || {
        let (player, uri) = (player.clone(), uri.clone());
        async move {
            if player.session().is_none() {
                return Ok(None);
            }
            match tokio::time::timeout(SPOTIFY_TIMEOUT, player.lyrics_json(&uri)).await {
                Ok(Ok(json)) => Ok(json),
                Ok(Err(e)) => Err(e),
                Err(_) => Err("Spotify didn’t respond".to_owned()),
            }
        }
        .boxed()
    })
}

const YOUTUBE_SOURCE: &str = "YouTube Music";

/// Plain text as untimed lines (every `t`/`e` is 0); blank lines are kept as empty rows between verses.
pub fn plain_lines(text: &str) -> Vec<mf_lyrics::Line> {
    mf_lyrics::plain_lines(text)
}

/// Запрос mf-lyrics по треку (title/artist/album/duration).
pub fn query_for(track: &TrackItem) -> mf_lyrics::LyricsQuery {
    mf_lyrics::LyricsQuery {
        track_id: track.track_id.clone(),
        title: track.title.clone(),
        artist: track.artists.clone(),
        album: track.album.clone(),
        duration: track.duration_ms as f64 / 1000.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses() {
        assert_eq!(LyricsStatus::NotFound.as_str(), "notfound");
        assert_eq!(LyricsStatus::of(&LyricsResult { ok: true, ..Default::default() }), LyricsStatus::Ok);
        assert_eq!(LyricsStatus::of(&LyricsResult::default()), LyricsStatus::NotFound);
        let failed = LyricsResult { details: vec!["NetEase: 503".into()], ..Default::default() };
        assert_eq!(LyricsStatus::of(&failed), LyricsStatus::Error);
    }

    #[test]
    fn query_from_track() {
        let track = TrackItem {
            track_id: "1".into(),
            title: "Song".into(),
            artists: "A, B".into(),
            album: "Al".into(),
            duration_ms: 90_500,
            ..Default::default()
        };
        let q = query_for(&track);
        assert_eq!((q.title.as_str(), q.artist.as_str(), q.album.as_str(), q.duration), ("Song", "A, B", "Al", 90.5));
    }
}

#[cfg(test)]
mod youtube_tests {
    #[test]
    fn plain_lyrics_become_untimed_lines() {
        let lines = super::plain_lines("First line\n\n Second ");
        assert_eq!(lines.len(), 3);
        assert_eq!((lines[0].text(), lines[1].text(), lines[2].text()), ("First line".to_owned(), String::new(), "Second".to_owned()));
        assert_eq!((lines[2].start(), lines[2].end()), (0.0, 0.0));
    }
}
