//! M3U playlist export, ported from soundwave1's Android `M3uManager.exportToM3u`. The file reads back with
//! youtube::import::parse_m3u: `#EXTINF:<seconds>,<artist> - <title>` and a location per track.
//!
//! Locations: Spotify tracks and episodes become open.spotify.com links, YouTube tracks music.youtube.com links
//! (both open in a browser and the YouTube ones import back as is). A track on this computer needs its file path,
//! which only the caller knows, so [`export_m3u`] asks `locate` first.
use std::path::Path;

use md3_model::TrackItem;

/// The playlist's text and how many tracks had no location and were left out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct M3uExport {
    pub content: String,
    pub written: usize,
    pub skipped: usize,
}

/// Where a track can be found outside the app, from its URI alone.
pub fn track_location(track: &TrackItem) -> Option<String> {
    let uri = track.play_uri();
    let mut parts = uri.splitn(3, ':');
    let (scheme, kind, id) = (parts.next()?, parts.next()?, parts.next()?);
    if id.is_empty() {
        return None;
    }
    match (scheme, kind) {
        ("spotify", "track" | "episode") => Some(format!("https://open.spotify.com/{kind}/{id}")),
        ("youtube", _) => Some(format!("https://music.youtube.com/watch?v={id}")),
        _ => None,
    }
}

/// One line of text: no line breaks, and no comma in the duration part.
fn one_line(text: &str) -> String {
    text.split(['\r', '\n']).map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
}

/// An extended M3U of `tracks`. `locate` gives a track's location (a local file's path, say) before the
/// URI-based one; tracks with neither are left out.
pub fn export_m3u(name: &str, tracks: &[TrackItem], locate: impl Fn(&TrackItem) -> Option<String>) -> M3uExport {
    let mut content = String::from("#EXTM3U\n");
    let name = one_line(name);
    if !name.is_empty() {
        content.push_str(&format!("#PLAYLIST:{name}\n"));
    }
    let (mut written, mut skipped) = (0, 0);
    for track in tracks {
        let Some(location) = locate(track).or_else(|| track_location(track)).map(|l| one_line(&l)) else {
            skipped += 1;
            continue;
        };
        if location.is_empty() {
            skipped += 1;
            continue;
        }
        let seconds = if track.duration_ms > 0 { (track.duration_ms / 1000).to_string() } else { "-1".into() };
        let title = one_line(&track.title);
        let artists = one_line(&track.artists);
        let display = if artists.is_empty() { title } else { format!("{artists} - {title}") };
        content.push_str(&format!("#EXTINF:{seconds},{display}\n{location}\n"));
        written += 1;
    }
    M3uExport { content, written, skipped }
}

/// A file name for the playlist: its name without characters file systems refuse, ending in .m3u8.
pub fn file_name(name: &str) -> String {
    let cleaned: String = one_line(name)
        .chars()
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.');
    format!("{}.m3u8", if cleaned.is_empty() { "playlist" } else { cleaned })
}

/// Write the playlist as UTF-8 (the .m3u8 convention).
pub fn write(path: &Path, export: &M3uExport) -> Result<(), String> {
    crate::data::fsio::write_atomic(path, export.content.as_bytes()).map_err(|e| format!("{e:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::youtube::import::{M3uEntry, parse_m3u};

    fn track(uri: &str, title: &str, artists: &str, duration_ms: i64) -> TrackItem {
        TrackItem { uri: uri.into(), title: title.into(), artists: artists.into(), duration_ms, ..Default::default() }
    }

    #[test]
    fn exports_spotify_youtube_and_local_tracks_and_reads_back() {
        let tracks = vec![
            track("spotify:track:4uLU6hMCjMI75M1A2tKUQC", "One More Time", "Daft Punk", 320_000),
            track("youtube:track:dQw4w9WgXcQ", "Never Gonna\nGive You Up", "Rick Astley", 213_500),
            track("local:track:00ff", "Bicycle Race", "Queen", 0),
            track("local:track:0abc", "Lost", "", 1000),
            track("spotify:episode:ep1", "Episode", "", 60_000),
        ];
        let export = export_m3u("Mix: one", &tracks, |t| {
            (t.uri == "local:track:00ff").then(|| "C:\\Music\\Queen - Bicycle Race.flac".to_owned())
        });
        assert_eq!((export.written, export.skipped), (4, 1));
        assert_eq!(
            export.content,
            "#EXTM3U\n#PLAYLIST:Mix: one\n\
             #EXTINF:320,Daft Punk - One More Time\nhttps://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC\n\
             #EXTINF:213,Rick Astley - Never Gonna Give You Up\nhttps://music.youtube.com/watch?v=dQw4w9WgXcQ\n\
             #EXTINF:-1,Queen - Bicycle Race\nC:\\Music\\Queen - Bicycle Race.flac\n\
             #EXTINF:60,Episode\nhttps://open.spotify.com/episode/ep1\n"
        );
        let back = parse_m3u(&export.content);
        assert_eq!(back.len(), 4);
        assert_eq!(
            back[1],
            M3uEntry {
                duration_sec: Some(213),
                artist: Some("Rick Astley".into()),
                title: "Never Gonna Give You Up".into(),
                location: "https://music.youtube.com/watch?v=dQw4w9WgXcQ".into(),
            }
        );
        assert_eq!(back[2].duration_sec, None);
        assert_eq!(back[3].artist, None);
    }

    #[test]
    fn locations_and_file_names() {
        assert_eq!(track_location(&TrackItem { track_id: "abc".into(), ..Default::default() }).as_deref(),
            Some("https://open.spotify.com/track/abc"));
        assert_eq!(track_location(&track("youtube:track:", "", "", 0)), None);
        assert_eq!(track_location(&track("spotify:local:a:b:c:1", "", "", 0)), None);
        assert_eq!(file_name("Road / Trip: 2026?"), "Road _ Trip_ 2026_.m3u8");
        assert_eq!(file_name(" ... "), "playlist.m3u8");
        assert_eq!(export_m3u("", &[], |_| None).content, "#EXTM3U\n");
    }
}
