//! YouTube Music radio on `WaveService`: a `youtube:track:<id>` seed plays the song's radio
//! (`RDAMVM<id>` through the `next` endpoint). The station shows in the same snapshot as a Spotify station;
//! its `station_uri` is `youtube:playlist:RDAMVM<id>`.
use std::collections::HashSet;
use std::sync::Arc;

use md3_model::{PlayContext, TrackItem, contexts};

use super::{Then, WaveService};

const EMPTY_TEXT: &str = "YouTube Music returned no songs for this radio.";
use crate::youtube::{NextPage, YouTubeService, playlist_uri, radio_playlist_id};

/// The video id of a YouTube radio seed (`youtube:track:<id>`).
pub(super) fn youtube_seed(seed: &str) -> Option<&str> {
    seed.trim().strip_prefix("youtube:track:").filter(|id| !id.is_empty())
}

/// The seed of a YouTube station URI (inverse of the station made in `yt_request`).
pub(super) fn youtube_station_seed(station: &str) -> Option<String> {
    let id = station.strip_prefix("youtube:playlist:")?.strip_prefix("RDAMVM")?;
    (!id.is_empty()).then(|| format!("youtube:track:{id}"))
}

/// Songs autoplay hands the player at a time.
const AUTOPLAY_BATCH: usize = 20;
/// Radio pages asked for at most per batch.
const AUTOPLAY_PAGES: usize = 4;
/// Songs remembered as sent before the memory starts over (the lane skips songs already listed anyway).
const AUTOPLAY_MEMORY: usize = 3000;

/// Pages of a song's YouTube Music radio (`YouTubeService::radio_page`; a fake in tests).
pub(super) trait RadioPages {
    fn page(&self, video_id: &str, continuation: Option<&str>) -> impl Future<Output = Result<NextPage, String>> + Send;
}

impl RadioPages for YouTubeService {
    async fn page(&self, video_id: &str, continuation: Option<&str>) -> Result<NextPage, String> {
        self.radio_page(video_id, continuation).await.map_err(|error| error.to_string())
    }
}

/// Autoplay's running radio, so it never runs dry: the radio of the song the list ended on, followed page by page
/// (its continuation); when a radio has no more pages, the radio of one of its own songs takes over. Songs sent
/// once are not sent again.
#[derive(Default)]
pub(super) struct AutoplayRadio {
    /// the radio followed now (its seed's video id), whether its first page came, and the next page's token
    seed: Option<String>,
    started: bool,
    continuation: Option<String>,
    /// uris handed to the player
    sent: HashSet<String>,
    /// video ids whose radio was followed
    seeds: HashSet<String>,
    /// video ids of the songs sent, newest last: the next seeds
    pool: Vec<String>,
}

impl AutoplayRadio {
    fn follow(&mut self, video_id: &str) {
        self.seeds.insert(video_id.to_owned());
        self.seed = Some(video_id.to_owned());
        self.started = false;
        self.continuation = None;
    }

    /// The next songs after `seed` (the song the list is ending on): up to `want` songs not sent before that
    /// `keep` accepts. A seed this radio did not send (the user's own list) starts over from its radio.
    pub(super) async fn next_batch(
        &mut self,
        source: &impl RadioPages,
        seed: &TrackItem,
        want: usize,
        keep: impl Fn(&TrackItem) -> bool,
    ) -> Vec<TrackItem> {
        let Some(seed_id) = youtube_seed(&seed.uri).map(str::to_owned) else { return Vec::new() };
        if !self.sent.contains(&seed.uri) || self.sent.len() > AUTOPLAY_MEMORY {
            *self = Self::default();
            self.follow(&seed_id);
        }
        self.sent.insert(seed.uri.clone());
        let mut out = Vec::new();
        for _ in 0..AUTOPLAY_PAGES {
            if out.len() >= want {
                break;
            }
            // the radio followed now is over: the newest song sent whose radio was not followed yet
            if self.started && self.continuation.is_none() {
                let Some(next) = self.pool.iter().rev().find(|id| !self.seeds.contains(*id)).cloned() else { break };
                self.follow(&next);
            }
            let Some(video_id) = self.seed.clone() else { break };
            let page = match source.page(&video_id, self.continuation.as_deref()).await {
                Ok(page) => page,
                Err(error) => {
                    tracing::warn!("youtube autoplay: the radio of {video_id}: {error}");
                    // this radio is done for; the next round takes another seed
                    self.started = true;
                    self.continuation = None;
                    continue;
                }
            };
            self.started = true;
            self.continuation = page.continuation.filter(|token| !token.is_empty());
            for track in page.tracks {
                let Some(id) = youtube_seed(&track.uri).map(str::to_owned) else { continue };
                if out.len() >= want || !track.available || !keep(&track) || !self.sent.insert(track.uri.clone()) {
                    continue;
                }
                self.pool.push(id);
                out.push(track);
            }
        }
        out
    }
}

impl WaveService {
    /// Play the radio of a YouTube song; an open station that already shows songs plays exactly those.
    pub(super) fn yt_play_station(&self, video_id: String, title: String) {
        let station = playlist_uri(&radio_playlist_id(&video_id));
        let snap = self.snapshot();
        if snap.station_uri == station && !snap.tracks.is_empty() && !snap.loading {
            let title = if title.is_empty() { snap.station_title.clone() } else { title };
            self.snapshot.send_modify(|s| {
                s.selected = format!("youtube:track:{video_id}");
                s.station_title = title.clone();
                s.error_text.clear();
            });
            return self.yt_start_playing(&station, &title, &snap.tracks);
        }
        self.yt_request(video_id, title, Then::Play);
    }

    /// Load the radio of a YouTube song without playing it (the radio page).
    pub(super) fn yt_open(&self, video_id: String) {
        let station = playlist_uri(&radio_playlist_id(&video_id));
        let snap = self.snapshot();
        if snap.station_uri == station && (snap.loading || !snap.tracks.is_empty()) {
            return;
        }
        self.yt_request(video_id, String::new(), Then::Show);
    }

    pub(super) fn yt_request(&self, video_id: String, title: String, then: Then) {
        let seed = format!("youtube:track:{video_id}");
        let station = playlist_uri(&radio_playlist_id(&video_id));
        let generation = self.next_generation();
        self.snapshot.send_modify(|s| {
            let same = s.station_uri == station;
            s.selected = seed.clone();
            s.loading = true;
            s.error_text.clear();
            if !same {
                s.station_uri = station.clone();
                s.tracks = Arc::new(Vec::new());
            }
            if !title.is_empty() {
                s.station_title = title.clone();
            }
        });
        let Some(me) = self.weak().upgrade() else { return };
        self.rt.spawn(async move {
            let result = me.auth.youtube().radio(&video_id).await;
            if !me.current(generation) {
                return;
            }
            match result {
                Ok(page) => {
                    let library = me.library.clone();
                    let tracks: Vec<TrackItem> =
                        page.tracks.into_iter().filter(|track| track.available && !library.is_hidden(&track.uri)).collect();
                    if tracks.is_empty() {
                        me.snapshot.send_modify(|s| {
                            s.loading = false;
                            s.error_text = mf_i18n::t(EMPTY_TEXT);
                        });
                        return;
                    }
                    let title = if !title.is_empty() {
                        title
                    } else if !page.title.is_empty() {
                        page.title
                    } else {
                        mf_i18n::tf("{0} Radio", &[&tracks[0].title])
                    };
                    let tracks = Arc::new(tracks);
                    me.snapshot.send_modify(|s| {
                        s.loading = false;
                        s.station_uri = station.clone();
                        s.station_title = title.clone();
                        s.tracks = tracks.clone();
                    });
                    let playing_this = {
                        let p = me.player.snapshot();
                        p.source() == contexts::RADIO && p.context.context_uri.as_deref() == Some(station.as_str())
                    };
                    if then == Then::Play || (then == Then::Refresh && playing_this) {
                        me.yt_start_playing(&station, &title, &tracks);
                    }
                }
                Err(error) => me.snapshot.send_modify(|s| {
                    s.loading = false;
                    s.error_text = error.to_string();
                }),
            }
        });
    }

    /// A song tapped in a list (search): it plays at once as the first song of its radio, and the radio's other
    /// songs are appended once YouTube Music returns them (as the Android app's radio queue does). The station
    /// page is not touched.
    pub(super) fn yt_play_track(&self, video_id: String, track: TrackItem) {
        let station = playlist_uri(&radio_playlist_id(&video_id));
        let title = mf_i18n::tf("{0} Radio", &[&track.title]);
        let cover = track.cover.clone();
        let context = PlayContext::with_uri(contexts::RADIO, &station).titled(&title).with_cover(cover);
        self.player.play_items(vec![track], 0, context);
        let Some(me) = self.weak().upgrade() else { return };
        self.rt.spawn(async move {
            match me.auth.youtube().radio(&video_id).await {
                Ok(page) => {
                    let library = me.library.clone();
                    let tracks: Vec<TrackItem> =
                        page.tracks.into_iter().filter(|track| track.available && !library.is_hidden(&track.uri)).collect();
                    // a context that no longer plays ignores these (`extend_context`)
                    me.player.extend_context(station, tracks);
                }
                Err(error) => tracing::warn!("youtube radio of {video_id}: {error}"),
            }
        });
    }

    /// Autoplay: the list is running out, so YouTube Music radio follows it (as in the Android app), batch after
    /// batch for as long as it plays (`AutoplayRadio`).
    pub(super) fn yt_autoplay(&self, track: TrackItem) {
        if youtube_seed(&track.uri).is_none() {
            return;
        }
        let Some(me) = self.weak().upgrade() else { return };
        self.rt.spawn(async move {
            let youtube = me.auth.youtube();
            let library = me.library.clone();
            let tracks = {
                let mut radio = me.yt_autoplay.lock().await;
                radio.next_batch(youtube.as_ref(), &track, AUTOPLAY_BATCH, |item| !library.is_hidden(&item.uri)).await
            };
            tracing::debug!("youtube autoplay after {}: {} songs", track.uri, tracks.len());
            // the lane ignores these if another song plays by now; an empty answer makes it ask again
            me.player.autoplay(track.uri, tracks);
        });
    }

    /// The rows carry their metadata (YouTube songs can't be looked up by URI the way Spotify ones are).
    pub(super) fn yt_start_playing(&self, station: &str, title: &str, tracks: &[TrackItem]) {
        let cover = tracks.first().map(|track| track.cover.clone()).unwrap_or_default();
        let context = PlayContext::with_uri(contexts::RADIO, station).titled(title).with_cover(cover);
        self.player.play_items(tracks.to_vec(), 0, context);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_seeds_and_stations_round_trip() {
        assert_eq!(youtube_seed(" youtube:track:abc "), Some("abc"));
        assert_eq!(youtube_seed("youtube:album:abc"), None);
        assert_eq!(youtube_seed("spotify:track:abc"), None);
        let station = playlist_uri(&radio_playlist_id("abc"));
        assert_eq!(station, "youtube:playlist:RDAMVMabc");
        assert_eq!(youtube_station_seed(&station).as_deref(), Some("youtube:track:abc"));
        assert_eq!(youtube_station_seed("spotify:station:track:abc"), None);
    }

    /// Radio pages by (seed, continuation); every request is recorded.
    #[derive(Default)]
    struct Pages {
        pages: std::collections::HashMap<(String, Option<String>), Result<NextPage, String>>,
        asked: std::sync::Mutex<Vec<(String, Option<String>)>>,
    }

    impl Pages {
        fn with(mut self, seed: &str, continuation: Option<&str>, ids: &[&str], next: Option<&str>) -> Self {
            let page = NextPage {
                tracks: ids.iter().map(|id| song(id)).collect(),
                continuation: next.map(str::to_owned),
                ..Default::default()
            };
            self.pages.insert((seed.to_owned(), continuation.map(str::to_owned)), Ok(page));
            self
        }

        fn asked(&self) -> Vec<(String, Option<String>)> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl RadioPages for Pages {
        async fn page(&self, video_id: &str, continuation: Option<&str>) -> Result<NextPage, String> {
            let key = (video_id.to_owned(), continuation.map(str::to_owned));
            self.asked.lock().unwrap().push(key.clone());
            self.pages.get(&key).cloned().unwrap_or_else(|| Err("no such page".into()))
        }
    }

    fn song(id: &str) -> TrackItem {
        TrackItem { track_id: id.into(), uri: format!("youtube:track:{id}"), available: true, ..Default::default() }
    }

    fn ids(tracks: &[TrackItem]) -> Vec<&str> {
        tracks.iter().map(|track| track.track_id.as_str()).collect()
    }

    #[tokio::test]
    async fn autoplay_follows_the_radio_page_after_page_and_never_repeats_a_song() {
        let pages = Pages::default()
            .with("s", None, &["s", "a", "b"], Some("c1"))
            .with("s", Some("c1"), &["b", "c", "d", "e"], Some("c2"))
            .with("s", Some("c2"), &["e", "f", "g", "h"], None);
        let mut radio = AutoplayRadio::default();
        let first = radio.next_batch(&pages, &song("s"), 4, |_| true).await;
        assert_eq!(ids(&first), ["a", "b", "c", "d"], "the seed and songs sent already are skipped");
        // the list ends on a song of that radio: its next page, not the song's own radio
        let second = radio.next_batch(&pages, &song("d"), 4, |_| true).await;
        assert_eq!(ids(&second), ["e", "f", "g", "h"], "e did not fit the first batch, so it comes now");
        assert_eq!(
            pages.asked(),
            [("s".into(), None), ("s".into(), Some("c1".into())), ("s".into(), Some("c2".into()))]
        );
    }

    #[tokio::test]
    async fn a_radio_without_more_pages_hands_over_to_the_radio_of_its_newest_song() {
        let pages = Pages::default()
            .with("s", None, &["a", "b"], None)
            .with("b", None, &["b", "a", "x", "y"], None)
            .with("y", None, &["y", "z"], None);
        let mut radio = AutoplayRadio::default();
        assert_eq!(ids(&radio.next_batch(&pages, &song("s"), 2, |_| true).await), ["a", "b"]);
        assert_eq!(ids(&radio.next_batch(&pages, &song("b"), 2, |_| true).await), ["x", "y"]);
        assert_eq!(ids(&radio.next_batch(&pages, &song("y"), 2, |_| true).await), ["z"]);
        let asked: Vec<String> = pages.asked().into_iter().map(|(seed, _)| seed).collect();
        // y's radio ran short: the newest songs' radios were tried (z, x, a have none here), each once
        assert_eq!(asked, ["s", "b", "y", "z", "x", "a"]);
    }

    #[tokio::test]
    async fn autoplay_skips_hidden_and_unavailable_songs_and_survives_failed_pages() {
        let mut gone = song("gone");
        gone.available = false;
        let mut pages = Pages::default().with("s", None, &["hidden", "a"], Some("c1"));
        if let Some(Ok(page)) = pages.pages.get_mut(&("s".to_owned(), None)) {
            page.tracks.push(gone);
        }
        let mut radio = AutoplayRadio::default();
        let batch = radio.next_batch(&pages, &song("s"), 5, |track| track.track_id != "hidden").await;
        assert_eq!(ids(&batch), ["a"], "the next page failed: what there was comes back");
        // the user's own song (not from this radio): its radio from the start
        let pages = pages.with("u", None, &["u", "a", "n"], None);
        assert_eq!(ids(&radio.next_batch(&pages, &song("u"), 5, |_| true).await), ["a", "n"], "a new radio forgets the old one");
        assert!(radio.next_batch(&pages, &TrackItem { uri: "spotify:track:x".into(), ..Default::default() }, 5, |_| true).await.is_empty());
    }
}
