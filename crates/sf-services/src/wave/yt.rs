//! YouTube Music radio on `WaveService`: a `youtube:track:<id>` seed plays the song's radio
//! (`RDAMVM<id>` through the `next` endpoint). The station shows in the same snapshot as a Spotify station;
//! its `station_uri` is `youtube:playlist:RDAMVM<id>`.
use std::sync::Arc;

use md3_model::{PlayContext, TrackItem, contexts};

use super::{Then, WaveService};

const EMPTY_TEXT: &str = "YouTube Music returned no songs for this radio.";
use crate::youtube::{playlist_uri, radio_playlist_id};

/// The video id of a YouTube radio seed (`youtube:track:<id>`).
pub(super) fn youtube_seed(seed: &str) -> Option<&str> {
    seed.trim().strip_prefix("youtube:track:").filter(|id| !id.is_empty())
}

/// The seed of a YouTube station URI (inverse of the station made in `yt_request`).
pub(super) fn youtube_station_seed(station: &str) -> Option<String> {
    let id = station.strip_prefix("youtube:playlist:")?.strip_prefix("RDAMVM")?;
    (!id.is_empty()).then(|| format!("youtube:track:{id}"))
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
}
