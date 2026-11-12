//! Scans of a temporary folder holding the ffmpeg fixtures in `tests/fixtures/local` (and the YouTube ones).
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::index::{self, LocalIndex};
use super::*;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").join(name)
}

/// A fresh folder under the system temp dir, removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("mf-local-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn copy(&self, from: &str, to: &str) -> PathBuf {
        let target = self.0.join(to);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::copy(fixture(from), &target).unwrap();
        target
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// music/ with every fixture, a text file and a hidden folder; covers/ for the saved pictures
fn library() -> TempDir {
    let dir = TempDir::new("lib");
    dir.copy("local/tagged.flac", "music/Alpha/Test Album/03 flac.flac");
    dir.copy("local/tagged.mp3", "music/Gamma/song.mp3");
    dir.copy("local/untagged.wav", "music/Loose/Untitled Take.wav");
    dir.copy("local/tagged.ogg", "music/Loose/ogg song.ogg");
    dir.copy("youtube/tone_44k_stereo.m4a", "music/tone.m4a");
    dir.copy("youtube/tone_48k_mono.m4a", "music/.hidden/skipped.m4a");
    std::fs::write(dir.path().join("music/Loose/notes.txt"), "not music").unwrap();
    dir
}

fn scan(dir: &TempDir, previous: &LocalIndex) -> LocalIndex {
    let folders = vec![dir.path().join("music").to_string_lossy().into_owned()];
    index::scan(&folders, previous, &dir.path().join("covers"), &AtomicBool::new(false), &()).expect("not cancelled")
}

fn by_title<'a>(index: &'a LocalIndex, title: &str) -> &'a LocalTrack {
    index.tracks.iter().find(|track| track.title == title).unwrap_or_else(|| panic!("{title} in {:#?}", index.tracks))
}

#[test]
fn reads_tags_formats_and_durations() {
    let dir = library();
    let index = scan(&dir, &LocalIndex::default());
    assert_eq!(index.tracks.len(), 5, "{:#?}", index.tracks);

    let flac = by_title(&index, "Flac Song");
    assert_eq!(flac.artists, ["Alpha", "Beta"]);
    assert_eq!((flac.album.as_str(), flac.album_artist.as_str(), flac.track_no, flac.disc_no), ("Test Album", "Alpha", 3, 1));
    assert_eq!(flac.year, "2021");
    assert_eq!((flac.codec.as_str(), flac.lossless, flac.bit_depth, flac.sample_rate), ("FLAC", true, 16, 44_100));
    assert_eq!(flac.quality, "FLAC 16/44.1");
    assert!(flac.duration_ms.abs_diff(1_000) <= 30, "{}", flac.duration_ms);

    let mp3 = by_title(&index, "Mp3 Song");
    assert_eq!(mp3.artists, ["Gamma", "Delta"]);
    assert_eq!((mp3.album.as_str(), mp3.codec.as_str(), mp3.lossless), ("Other Album", "MP3", false));
    assert!(mp3.duration_ms.abs_diff(1_000) <= 80, "{}", mp3.duration_ms);

    // no tags: the file name, the folder as the album
    let wav = by_title(&index, "Untitled Take");
    assert!(wav.artists.is_empty());
    assert_eq!((wav.album.as_str(), wav.codec.as_str(), wav.lossless, wav.quality.as_str()), ("Loose", "PCM", true, "PCM 16/44.1"));
    assert!(wav.duration_ms.abs_diff(1_000) <= 30, "{}", wav.duration_ms);

    let ogg = by_title(&index, "Ogg Song");
    assert_eq!((ogg.artists.as_slice(), ogg.codec.as_str()), (&["Epsilon".to_owned()][..], "VORBIS"));

    let m4a = by_title(&index, "tone");
    assert_eq!((m4a.codec.as_str(), m4a.sample_rate), ("AAC", 44_100));
    assert!(m4a.duration_ms.abs_diff(2_000) <= 50, "{}", m4a.duration_ms);
    assert!(!index.tracks.iter().any(|track| track.path.contains(".hidden")));
}

#[test]
fn saves_the_embedded_cover_and_finds_folder_pictures() {
    let dir = library();
    std::fs::write(dir.path().join("music/Gamma/folder.png"), b"png").unwrap();
    let index = scan(&dir, &LocalIndex::default());
    let flac = by_title(&index, "Flac Song");
    assert!(flac.cover.starts_with(&*dir.path().join("covers").to_string_lossy()), "{}", flac.cover);
    let bytes = std::fs::read(&flac.cover).unwrap();
    assert_eq!(&bytes[..2], [0xFF, 0xD8], "a JPEG");
    assert!(by_title(&index, "Mp3 Song").cover.ends_with("folder.png"));
    assert_eq!(by_title(&index, "Ogg Song").cover, "");
}

#[test]
fn ids_are_stable_and_unchanged_files_are_not_read_again() {
    let dir = library();
    let first = scan(&dir, &LocalIndex::default());
    let again = scan(&dir, &first);
    assert_eq!(first, again);
    // an entry with matching size and time is taken as it is, without opening the file
    let mut doctored = first.clone();
    let row = doctored.tracks.iter().position(|track| track.title == "Mp3 Song").unwrap();
    doctored.tracks[row].title = "Cached Title".into();
    let reused = scan(&dir, &doctored);
    assert!(reused.tracks.iter().any(|track| track.title == "Cached Title"));
    // a changed size reads the file again
    doctored.tracks[row].size += 1;
    let reread = scan(&dir, &doctored);
    assert!(reread.tracks.iter().any(|track| track.title == "Mp3 Song"));
    let ids: Vec<&str> = first.tracks.iter().map(|track| track.id.as_str()).collect();
    assert!(ids.iter().all(|id| id.len() == 16));
    assert_eq!(ids.len(), ids.iter().collect::<std::collections::HashSet<_>>().len());
}

#[test]
fn the_index_round_trips_through_its_file() {
    let dir = library();
    let index = scan(&dir, &LocalIndex::default());
    let file = dir.path().join("state/local_files.json");
    index.save(&file).unwrap();
    assert_eq!(LocalIndex::load(&file), index);
    std::fs::write(&file, b"{broken").unwrap();
    assert_eq!(LocalIndex::load(&file), LocalIndex::default());
    assert_eq!(LocalIndex::load(&dir.path().join("missing.json")), LocalIndex::default());
}

#[test]
fn a_cancelled_scan_returns_nothing() {
    let dir = library();
    let folders = vec![dir.path().join("music").to_string_lossy().into_owned()];
    let cancel = AtomicBool::new(true);
    assert!(index::scan(&folders, &LocalIndex::default(), dir.path(), &cancel, &()).is_none());
}

#[test]
fn rows_carry_local_uris_and_file_covers() {
    let track = LocalTrack {
        id: "00ff".into(),
        title: "T".into(),
        artists: vec!["A".into(), "B".into()],
        album: "Al".into(),
        duration_ms: 1_234,
        cover: r"C:\Music\My Album\cover #1.jpg".into(),
        ..Default::default()
    };
    let item = to_item(&track);
    assert_eq!(item.uri, "local:track:00ff");
    assert_eq!(item.play_uri(), "local:track:00ff");
    assert_eq!((item.artists.as_str(), item.artist_refs.len(), item.duration_ms, item.available), ("A, B", 2, 1_234, true));
    assert_eq!(item.cover, "file:///C:/Music/My%20Album/cover%20%231.jpg");
    assert_eq!(file_url("/home/u/a.jpg"), "file:///home/u/a.jpg");
    assert_eq!(to_item(&LocalTrack::default()).artists, mf_i18n::t("Unknown artist"));
}

#[test]
fn folder_settings_are_normalised() {
    let folders = folders_from_setting("D:\\Music\\\n\n  D:\\Music \nE:/Other/\n");
    assert_eq!(folders, ["D:\\Music", "E:/Other"]);
    assert_eq!(folders_from_setting(&folders_to_setting(&folders)), folders);
    assert_eq!(normalise(vec!["C:\\".into()]), [format!("C:{}", std::path::MAIN_SEPARATOR)]);
}

#[test]
fn the_folder_tree_counts_tracks_below_each_folder() {
    let dir = library();
    let index = scan(&dir, &LocalIndex::default());
    let root = dir.path().join("music").to_string_lossy().into_owned();
    let tree = folder_tree(&[root.clone()], &index.tracks);
    assert_eq!(tree.len(), 1);
    let music = &tree[0];
    assert_eq!((music.path.as_str(), music.tracks, music.total), (root.as_str(), 1, 5));
    let names: Vec<(&str, usize)> = music.children.iter().map(|child| (child.name.as_str(), child.total)).collect();
    assert_eq!(names, [("Alpha", 1), ("Gamma", 1), ("Loose", 2)]);
    assert_eq!(music.children[0].children[0].name, "Test Album");
}

#[test]
fn the_service_shows_the_saved_index_then_scans() {
    let dir = library();
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let service = LocalFilesService::with_paths(rt.handle().clone(), dir.path().join("index.json"), dir.path().join("covers"));
    let folder = dir.path().join("music").to_string_lossy().into_owned();
    let count = rt.block_on(service.scan(vec![folder.clone()], false)).unwrap();
    assert_eq!(count, 5);
    let snapshot = service.snapshot();
    assert_eq!((snapshot.folders.clone(), snapshot.scanning, snapshot.found, snapshot.read), (vec![folder.clone()], false, 5, 5));
    let flac = snapshot.tracks.iter().find(|track| track.codec == "FLAC").unwrap();
    assert_eq!(service.track(&flac.uri()).as_ref(), Some(flac));
    assert_eq!(service.track(&flac.id).as_ref(), Some(flac));
    assert_eq!(service.items().len(), 5);
    assert!(dir.path().join("index.json").is_file());

    // a second service over the same index shows it before its own scan finishes
    let second = LocalFilesService::with_paths(rt.handle().clone(), dir.path().join("index.json"), dir.path().join("covers"));
    let mut updates = second.subscribe();
    second.start(vec![folder.clone()]);
    rt.block_on(async {
        let shown = updates.wait_for(|snapshot| snapshot.loaded && snapshot.tracks.len() == 5);
        tokio::time::timeout(std::time::Duration::from_secs(10), shown).await.expect("in time").expect("open");
    });

    // removing the folder empties the library
    assert_eq!(rt.block_on(service.scan(Vec::new(), false)).unwrap(), 0);
    assert!(service.track(&flac.id).is_none());
}

#[test]
fn the_lane_source_answers_local_tracks_with_their_file() {
    use sf_player::engine::youtube::{LOCAL_FILE_URL, StreamSource};

    let dir = library();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let service = LocalFilesService::with_paths(rt.handle().clone(), dir.path().join("index.json"), dir.path().join("covers"));
    rt.block_on(service.scan(vec![dir.path().join("music").to_string_lossy().into_owned()], false)).unwrap();
    let flac = service.snapshot().tracks.iter().find(|track| track.codec == "FLAC").cloned().unwrap();
    let source = LaneSource::new(service.clone(), None);
    let stream = rt.block_on(source.resolve(flac.uri())).unwrap();
    assert_eq!(stream.url, format!("{LOCAL_FILE_URL}{}", flac.path));
    assert_eq!((stream.mime.as_str(), stream.video_id.as_str(), stream.sample_rate, stream.bit_depth), ("audio/flac", flac.uri().as_str(), 44_100, 16));
    assert_eq!(stream.codec_label(), "FLAC 16/44.1");
    // without YouTube set up, YouTube tracks fail with a message; unknown local ids too
    assert!(rt.block_on(source.resolve("dQw4w9WgXcQ".into())).is_err());
    assert!(rt.block_on(source.resolve("local:track:0000000000000000".into())).is_err());
    // a file deleted since the scan
    std::fs::remove_file(&flac.path).unwrap();
    assert!(rt.block_on(source.resolve(flac.uri())).is_err());
}
