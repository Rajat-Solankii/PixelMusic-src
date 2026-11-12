//! Hermetic: a local mock of ws.audioscrobbler.com (GET with a query), a fake catalog search, fixed seeds.
use std::sync::atomic::{AtomicUsize, Ordering};

use md3_model::ArtistRef;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

const KEY: &str = "dummy-api-key";
const USER: &str = "dummy-user";

type Params = HashMap<String, String>;
type Handler = Arc<dyn Fn(&Params) -> Value + Send + Sync>;

struct MockServer {
    url: String,
    requests: Arc<Mutex<Vec<Params>>>,
}

impl MockServer {
    async fn start(answer: impl Fn(&Params) -> Value + Send + Sync + 'static) -> Self {
        let answer: Handler = Arc::new(answer);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/2.0/", listener.local_addr().unwrap());
        let requests: Arc<Mutex<Vec<Params>>> = Arc::default();
        let recorded = requests.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (recorded, answer) = (recorded.clone(), answer.clone());
                tokio::spawn(async move {
                    let Some(params) = read_query(&mut socket).await else { return };
                    recorded.lock().unwrap().push(params.clone());
                    let value = answer(&params);
                    let text = value.to_string();
                    let status = if value.get("error").is_some() { "400 Bad Request" } else { "200 OK" };
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                        text.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        Self { url, requests }
    }

    fn calls(&self, method: &str) -> Vec<Params> {
        self.requests.lock().unwrap().iter().filter(|p| p.get("method").is_some_and(|m| m == method)).cloned().collect()
    }

    fn methods(&self) -> Vec<String> {
        let mut methods: Vec<String> = self.requests.lock().unwrap().iter().map(|p| p["method"].clone()).collect();
        methods.sort();
        methods.dedup();
        methods
    }
}

async fn read_query(socket: &mut tokio::net::TcpStream) -> Option<Params> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buffer).to_string();
    let line = head.lines().next()?;
    assert!(line.starts_with("GET /2.0/?"), "{line}");
    let target = line.split(' ').nth(1)?;
    let query = target.split_once('?')?.1;
    Some(
        query
            .split('&')
            .filter(|part| !part.is_empty())
            .map(|part| {
                let (name, value) = part.split_once('=').unwrap_or((part, ""));
                (url_decode(name), url_decode(value))
            })
            .collect(),
    )
}

fn url_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(&text[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

fn http() -> sf_api::Http {
    sf_api::Http::new(reqwest::Client::builder().no_proxy().build().unwrap())
}

fn track_json(name: &str, artist: &str) -> Value {
    json!({ "name": name, "artist": { "name": artist }, "image": [{ "#text": format!("https://img/{name}.jpg"), "size": "extralarge" }] })
}

/// A Last.fm that knows a few tracks per method; names carry the method so buckets can be told apart.
fn fake_lastfm(params: &Params) -> Value {
    let method = params["method"].as_str();
    let list = |prefix: &str, n: usize| -> Vec<Value> {
        (0..n).map(|i| track_json(&format!("{prefix} {i}"), &format!("{prefix} Artist {}", i % 4))).collect()
    };
    match method {
        "user.gettoptracks" => json!({ "toptracks": { "track": list("Top", 12) } }),
        "user.getrecenttracks" => json!({ "recenttracks": { "track": list("Recent", 10) } }),
        "track.getsimilar" => json!({ "similartracks": { "track": list(&format!("Like {}", params["track"]), 3) } }),
        "user.gettopartists" => json!({ "topartists": { "artist": [{ "name": "Fav A" }, { "name": "Fav B" }] } }),
        "artist.getsimilar" => {
            json!({ "similarartists": { "artist": [{ "name": format!("{} Twin", params["artist"]) }, { "name": "Shared Twin" }] } })
        }
        "artist.gettoptracks" => json!({ "toptracks": { "track": list(&format!("By {}", params["artist"]), 2) } }),
        "user.gettoptags" => json!({ "toptags": { "tag": [{ "name": "lofi" }, { "name": "jazz" }] } }),
        "tag.gettoptracks" => json!({ "tracks": { "track": list(&format!("Tag {}", params["tag"]), 4) } }),
        _ => json!({ "error": 3, "message": "Invalid Method" }),
    }
}

/// Finds "{title} {artist}" as a catalog track with the same title and artist, except titles containing "Missing".
struct FakeSearch {
    calls: AtomicUsize,
}

impl TrackSearch for FakeSearch {
    fn search_tracks<'a>(&'a self, query: &'a str) -> BoxFuture<'a, Vec<TrackItem>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if query.contains("Missing") {
                return vec![item("Something Else", "Nobody")];
            }
            // "Top 3 Top Artist 3" → title "Top 3", artist "Top Artist 3"
            let words: Vec<&str> = query.split(' ').collect();
            let (title, artist) = words.split_at(2);
            vec![item("Unrelated Song", "Other Person"), item(&title.join(" "), &artist.join(" "))]
        })
    }
}

fn item(title: &str, artist: &str) -> TrackItem {
    TrackItem {
        title: title.to_owned(),
        artists: artist.to_owned(),
        artist_refs: vec![ArtistRef { name: artist.to_owned(), ..Default::default() }],
        uri: format!("youtube:track:{}", title.to_lowercase().replace(' ', "_")),
        available: true,
        ..Default::default()
    }
}

fn lfm(name: &str, artist: &str) -> LfmTrack {
    LfmTrack { name: name.to_owned(), artist: artist.to_owned(), image: String::new() }
}

// --- parsers ----------------------------------------------------------------------------------------------------

#[test]
fn recent_tracks_drop_now_playing_and_read_both_artist_shapes() {
    let root = json!({ "recenttracks": { "track": [
        { "name": "Live", "artist": { "#text": "Now" }, "@attr": { "nowplaying": "true" } },
        { "name": "Song A", "artist": { "#text": "Artist A" }, "album": { "#text": "Album A" },
          "image": [{ "#text": "https://img/s.jpg", "size": "small" }, { "#text": "https://img/xl.jpg", "size": "extralarge" }],
          "url": "https://last.fm/a", "date": { "uts": "1700000000", "#text": "14 Nov 2023, 22:13" } },
        { "name": "Song B", "artist": "Artist B", "album": { "title": "Album B" } },
        { "name": "No Artist", "artist": "" },
        { "name": "  ", "artist": "X" }
    ] } });
    let recent = parse_recent(&root);
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].title, "Song A");
    assert_eq!(recent[0].artist, "Artist A");
    assert_eq!(recent[0].album, "Album A");
    assert_eq!(recent[0].cover, "https://img/xl.jpg");
    assert_eq!(recent[0].uts, 1_700_000_000);
    assert_eq!(recent[0].date_text, "14 Nov 2023, 22:13");
    assert_eq!(recent[1].artist, "Artist B");
    assert_eq!(recent[1].album, "Album B");
    assert_eq!(recent[1].uts, 0);
}

#[test]
fn a_single_object_counts_as_a_list_of_one() {
    let root = json!({ "toptracks": { "track": { "name": "Only", "artist": { "name": "One" } } } });
    assert_eq!(parse_tracks(&root), vec![lfm("Only", "One")]);
    let search = json!({ "results": { "trackmatches": { "track": [{ "name": "Found", "artist": "Here" }] } } });
    assert_eq!(parse_tracks(&search), vec![lfm("Found", "Here")]);
    let artists = json!({ "results": { "artistmatches": { "artist": { "name": "Solo" } } } });
    assert_eq!(parse_artists(&artists), vec!["Solo"]);
}

#[test]
fn friends_fall_back_to_the_username_and_take_the_last_real_avatar() {
    let placeholder = format!("https://lastfm.freetls.fastly.net/i/u/300x300/{NO_ART_HASH}.png");
    let root = json!({ "friends": { "user": [
        { "name": "alice", "realname": "Alice A", "image": [{ "#text": "https://img/a-s.jpg" }, { "#text": "https://img/a-l.jpg" }, { "#text": placeholder }] },
        { "name": "bob", "realname": "", "image": [] },
        { "name": "" }
    ] } });
    let friends = parse_friends(&root);
    assert_eq!(friends.len(), 2);
    assert_eq!(friends[0].display_name, "Alice A");
    assert_eq!(friends[0].avatar, "https://img/a-l.jpg");
    assert_eq!(friends[1].display_name, "bob");
    assert_eq!(friends[1].avatar, "");
}

#[test]
fn top_albums_skip_unknown_artists() {
    let root = json!({ "topalbums": { "album": [
        { "name": "Kept", "artist": { "name": "Band" }, "playcount": "42", "image": [{ "#text": "https://img/k.jpg" }] },
        { "name": "Dropped", "artist": { "name": "Unknown artist" } },
        { "name": "Flat", "artist": "Plain", "playcount": 7 }
    ] } });
    let albums = parse_top_albums(&root);
    assert_eq!(albums.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(), ["Kept", "Flat"]);
    assert_eq!(albums[0].play_count, 42);
    assert_eq!(albums[0].cover, "https://img/k.jpg");
    assert_eq!(albums[1].play_count, 7);
}

// --- naming -----------------------------------------------------------------------------------------------------

#[test]
fn playlist_names_follow_the_mode() {
    let tracks = vec![lfm("Blinding Lights", "The Weeknd")];
    let request = |mode, period: &str| MixRequest { mode, period: period.to_owned(), ..Default::default() };
    assert_eq!(playlist_name(&request(MixMode::Top, "overall"), &tracks), "TheWeekndEra");
    assert_eq!(playlist_name(&request(MixMode::Top, "7day"), &tracks), "TheWeekndFlow");
    assert_eq!(playlist_name(&request(MixMode::Top, "7day"), &[]), "SonicFlow");
    assert_eq!(playlist_name(&request(MixMode::Library, ""), &[]), "VaultCraft");
    assert_eq!(playlist_name(&request(MixMode::Recent, ""), &tracks), "TheWeekndRotation");
    assert_eq!(playlist_name(&request(MixMode::Mix, ""), &[]), "AlgorhythmFlow");
    assert_eq!(playlist_name(&request(MixMode::Recommendations, ""), &tracks), "TheWeekndMix");
    let seeded = MixRequest { mode: MixMode::SimilarTracks, seed_track: "starboy!".into(), ..Default::default() };
    assert_eq!(playlist_name(&seeded, &tracks), "StarboyEcho");
    let tag = MixRequest { mode: MixMode::Tag, tag: "lo-fi hip hop".into(), ..Default::default() };
    assert_eq!(playlist_name(&tag, &tracks), "LofihiphoHood");
    let artist = MixRequest { mode: MixMode::SimilarArtists, seed_artist_input: "daft punk".into(), ..Default::default() };
    assert_eq!(playlist_name(&artist, &[]), "DaftpunkRadio");
}

#[test]
fn unique_names_count_up() {
    let existing = vec!["SonicEra".to_owned(), "soniceRa #2".to_owned()];
    assert_eq!(unique_name("SonicEra", &existing), "SonicEra #3");
    assert_eq!(unique_name("Other", &existing), "Other");
}

#[test]
fn modes_round_trip_and_seeds_are_checked() {
    for mode in MixMode::ALL {
        assert_eq!(MixMode::from_key(mode.key()), Some(mode));
    }
    assert_eq!(MixMode::from_key("nope"), None);
    assert!(MixRequest { mode: MixMode::SimilarTracks, seed_track: "x".into(), ..Default::default() }.missing_input().is_some());
    assert!(MixRequest { mode: MixMode::Tag, ..Default::default() }.missing_input().is_some());
    assert!(MixRequest { mode: MixMode::Mix, ..Default::default() }.missing_input().is_none());
}

// --- weighting --------------------------------------------------------------------------------------------------

#[test]
fn merge_keeps_the_highest_weight_orders_tiers_and_caps_artists() {
    let weighted = vec![
        (lfm("Low", "A"), 1),
        (lfm("Mid", "B"), 2),
        (lfm("low", "a"), 4), // the same track as "Low", now in the top tier
        (lfm("Two", "C"), 3),
        (lfm("Three", "C"), 3),
        (lfm("Four", "C"), 3),
    ];
    let mut rng = fastrand::Rng::with_seed(7);
    let merged = merge_tiers(weighted.clone(), 10, &mut rng);
    assert_eq!(merged[0].name, "low");
    assert_eq!(merged.iter().filter(|t| t.artist == "C").count(), 2);
    assert_eq!(merged.last().unwrap().name, "Mid");
    assert_eq!(merged.len(), 4);
    assert_eq!(merge_tiers(weighted.clone(), 2, &mut fastrand::Rng::with_seed(7)).len(), 2);
    // the same seed, the same mix
    assert_eq!(merge_tiers(weighted.clone(), 10, &mut fastrand::Rng::with_seed(7)), merged);
}

#[test]
fn recommendations_reject_recent_plays_and_prefer_familiar_artists() {
    let mut taste = Taste::default();
    taste.recent_tracks.insert(lfm("Heard", "X").key());
    taste.top_artists.insert("fav".into());
    taste.top_tracks.insert(lfm("Old Fave", "Fav").key());
    assert_eq!(score(&lfm("Heard", "X"), 4, &taste), -1);
    assert_eq!(score(&lfm("New", "Fav"), 3, &taste), 80);
    assert_eq!(score(&lfm("Old Fave", "Fav"), 2, &taste), 55);
    assert_eq!(score(&lfm("Tag", "Nobody"), 1, &taste), 0);

    let weighted = vec![
        (lfm("Heard", "X"), 4),
        (lfm("New", "Fav"), 3),
        (lfm("Discovery", "Fav"), 2),
        (lfm("Stranger", "Nobody"), 2),
        (lfm("Tagged", "Someone"), 1),
    ];
    let picked = pick_recommendations(weighted, &taste, 4);
    let names: Vec<&str> = picked.iter().map(|t| t.name.as_str()).collect();
    assert!(!names.contains(&"Heard"));
    assert_eq!(names[0], "New");
    assert!(names.contains(&"Discovery"));
}

// --- resolving --------------------------------------------------------------------------------------------------

#[tokio::test]
async fn resolving_checks_the_match_dedupes_and_stops_at_the_count() {
    let search = FakeSearch { calls: AtomicUsize::new(0) };
    let tracks = vec![
        lfm("Song One", "Band"),
        lfm("Missing Song", "Band"),
        lfm("song one", "band"), // finds the same catalog track again
        lfm("Song Two", "Band"),
        lfm("Song Three", "Band"),
        lfm("Song Four", "Band"),
    ];
    let mut progress = Vec::new();
    let found = resolve(&search, &tracks, 3, |n| progress.push(n)).await;
    assert_eq!(found.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["Song One", "Song Two", "Song Three"]);
    assert_eq!(progress, [1, 2, 3]);
    // the stream stopped early: the last track was never (or at most in flight) searched
    assert!(search.calls.load(Ordering::SeqCst) <= tracks.len());
    assert!(best_match(&lfm("Missing Song", "Band"), vec![item("Something Else", "Nobody")]).is_none());
}

// --- the API over the mock --------------------------------------------------------------------------------------

#[tokio::test]
async fn feed_calls_are_unsigned_gets_with_the_key() {
    let server = MockServer::start(|params| match params["method"].as_str() {
        "user.getrecenttracks" => json!({ "recenttracks": { "track": [{ "name": "S", "artist": { "#text": "A" } }] } }),
        "user.getfriends" => json!({ "error": 6, "message": "no friends" }),
        _ => json!({ "topalbums": { "album": [] } }),
    })
    .await;
    let api = FeedApi::new(http(), &server.url, KEY);
    assert_eq!(api.recent_tracks(USER, 30).await.unwrap().len(), 1);
    assert_eq!(api.friends(USER, 20).await, Err(LastFmError::Api { code: 6, message: "no friends".into() }));
    assert!(api.top_albums(USER, 20, "overall").await.unwrap().is_empty());
    // a guest asks nothing
    assert!(api.recent_tracks("Guest User", 30).await.unwrap().is_empty());
    let recent = &server.calls("user.getrecenttracks")[0];
    assert_eq!(recent["api_key"], KEY);
    assert_eq!(recent["format"], "json");
    assert_eq!(recent["user"], USER);
    assert_eq!(recent["limit"], "30");
    assert!(!recent.contains_key("api_sig") && !recent.contains_key("sk"));
    assert_eq!(server.calls("user.gettopalbums")[0]["period"], "overall");
    assert_eq!(server.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn single_modes_ask_the_right_method() {
    let server = MockServer::start(fake_lastfm).await;
    let api = FeedApi::new(http(), &server.url, KEY);
    let mut rng = fastrand::Rng::with_seed(1);
    let top = MixRequest { mode: MixMode::Top, count: 10, period: "3month".into(), ..Default::default() };
    assert_eq!(api.mix_tracks(USER, &top, &mut rng).await.unwrap().len(), 12);
    let call = &server.calls("user.gettoptracks")[0];
    assert_eq!((call["period"].as_str(), call["limit"].as_str()), ("3month", "30"));
    assert!(["1", "2", "3"].contains(&call["page"].as_str()));

    let similar = MixRequest { mode: MixMode::SimilarTracks, count: 60, seed_track: "Hit".into(), seed_artist: "Star".into(), ..Default::default() };
    let tracks = api.mix_tracks(USER, &similar, &mut rng).await.unwrap();
    assert!(tracks.iter().all(|t| t.name.starts_with("Like Hit")));
    let call = &server.calls("track.getsimilar")[0];
    assert_eq!((call["track"].as_str(), call["artist"].as_str(), call["limit"].as_str()), ("Hit", "Star", "200"));

    let artists = MixRequest { mode: MixMode::SimilarArtists, count: 25, seed_artist_input: "Seed".into(), ..Default::default() };
    let tracks = api.mix_tracks(USER, &artists, &mut rng).await.unwrap();
    assert_eq!(tracks.len(), 4);
    assert_eq!(server.calls("artist.gettoptracks").len(), 2);
    assert_eq!(server.calls("artist.gettoptracks")[0]["limit"], "5");

    let tag = MixRequest { mode: MixMode::Tag, count: 5, tag: "lofi".into(), ..Default::default() };
    assert!(api.mix_tracks(USER, &tag, &mut rng).await.unwrap().iter().all(|t| t.name.starts_with("Tag lofi")));
}

#[tokio::test]
async fn the_blend_uses_every_bucket_and_is_reproducible_with_a_seed() {
    let server = MockServer::start(fake_lastfm).await;
    let api = FeedApi::new(http(), &server.url, KEY);
    let request = MixRequest { mode: MixMode::Mix, count: 30, ..Default::default() };
    let first = api.mix_tracks(USER, &request, &mut fastrand::Rng::with_seed(42)).await.unwrap();
    let second = api.mix_tracks(USER, &request, &mut fastrand::Rng::with_seed(42)).await.unwrap();
    assert_eq!(first, second);
    assert_eq!(first.len(), 30);
    assert_eq!(
        server.methods(),
        ["artist.getsimilar", "artist.gettoptracks", "tag.gettoptracks", "track.getsimilar", "user.getrecenttracks",
         "user.gettopartists", "user.gettoptags", "user.gettoptracks"]
    );
    // three periods per run, two runs
    assert_eq!(server.calls("user.gettoptracks").len(), 6);
    // top tier first
    assert!(first[0].name.starts_with("Top"));
    let mut per_artist: HashMap<String, usize> = HashMap::new();
    for track in &first {
        *per_artist.entry(track.artist.to_lowercase()).or_default() += 1;
    }
    assert!(per_artist.values().all(|n| *n <= 2));
}

#[tokio::test]
async fn recommendations_skip_what_was_heard_lately() {
    let server = MockServer::start(fake_lastfm).await;
    let api = FeedApi::new(http(), &server.url, KEY);
    let request = MixRequest { mode: MixMode::Recommendations, count: 20, ..Default::default() };
    let tracks = api.mix_tracks(USER, &request, &mut fastrand::Rng::with_seed(3)).await.unwrap();
    assert!(!tracks.is_empty());
    assert!(tracks.iter().all(|t| !t.name.starts_with("Recent")));
    assert_eq!(server.calls("track.getsimilar").len(), 11);
}

// --- the service ------------------------------------------------------------------------------------------------

async fn settled(service: &LastFmFeedService, done: impl Fn(&LastFmFeedSnapshot) -> bool) -> LastFmFeedSnapshot {
    let mut watch = service.watch();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = watch.borrow_and_update().clone();
            if done(&snapshot) {
                return snapshot;
            }
            watch.changed().await.unwrap();
        }
    })
    .await
    .expect("service settled")
}

fn feed_service(url: &str, account: (&str, &str)) -> Arc<LastFmFeedService> {
    feed_service_with(url, account, None)
}

fn feed_service_with(url: &str, account: (&str, &str), covers: Option<Arc<dyn CoverSearch>>) -> Arc<LastFmFeedService> {
    let account = (account.0.to_owned(), account.1.to_owned());
    let search: Arc<dyn TrackSearch> = Arc::new(FakeSearch { calls: AtomicUsize::new(0) });
    LastFmFeedService::new(
        Handle::current(),
        http(),
        Arc::new(move || account.clone()),
        Some(search),
        None,
        None,
        covers,
        LastFmFeedOptions { api_url: url.to_owned(), seed: Some(9) },
    )
}

#[tokio::test]
async fn generate_resolves_names_and_publishes_the_mix() {
    let server = MockServer::start(fake_lastfm).await;
    let service = feed_service(&server.url, (KEY, USER));
    let request = MixRequest { mode: MixMode::Top, count: 5, period: "1month".into(), ..Default::default() };
    service.generate(request, MixTarget::YouTube, vec!["TopArtistWave".into()]);
    let snapshot = settled(&service, |s| !s.generating && !s.mix_tracks.is_empty()).await;
    assert_eq!(snapshot.mix_tracks.len(), 5);
    assert!(snapshot.mix_tracks.iter().all(|t| t.title.starts_with("Top ")));
    assert_eq!(snapshot.mix_title, "TopArtistWave #2");
    assert_eq!(snapshot.mix_mode, "top");
    assert!(snapshot.mix_cover.starts_with("https://img/"));
    assert!(snapshot.mix_error.is_empty());

    // no Spotify search here
    service.generate(MixRequest::default(), MixTarget::Spotify, Vec::new());
    assert!(!service.snapshot().mix_error.is_empty());
    // a missing seed fails before any request
    let before = server.requests.lock().unwrap().len();
    service.generate(MixRequest { mode: MixMode::Tag, ..Default::default() }, MixTarget::YouTube, Vec::new());
    assert!(!service.snapshot().mix_error.is_empty());
    assert_eq!(server.requests.lock().unwrap().len(), before);
}

#[tokio::test]
async fn feed_refresh_fills_the_three_lists() {
    let server = MockServer::start(|params| match params["method"].as_str() {
        "user.getrecenttracks" => json!({ "recenttracks": { "track": [{ "name": "S", "artist": "A" }] } }),
        "user.getfriends" => json!({ "friends": { "user": { "name": "pal" } } }),
        _ => json!({ "topalbums": { "album": { "name": "LP", "artist": { "name": "A" } } } }),
    })
    .await;
    let service = feed_service(&server.url, (KEY, USER));
    service.refresh_feed();
    let snapshot = settled(&service, |s| !s.feed_loading && !s.recent.is_empty()).await;
    assert_eq!(snapshot.friends.len(), 1);
    assert_eq!(snapshot.top_albums.len(), 1);
    assert!(snapshot.feed_error.is_empty());

    let offline = feed_service(&server.url, (KEY, ""));
    offline.refresh_feed();
    assert!(!offline.snapshot().feed_error.is_empty());
}

// --- covers for feed tiles --------------------------------------------------------------------------------------

#[test]
fn placeholder_and_empty_art_count_as_missing() {
    let star = format!("https://lastfm.freetls.fastly.net/i/u/300x300/{NO_ART_HASH}.png");
    let root = json!({ "recenttracks": { "track": [
        { "name": "Tu hai kahan", "artist": { "#text": "AUR" }, "image": [{ "#text": "", "size": "small" }, { "#text": star, "size": "extralarge" }] }
    ] } });
    assert_eq!(parse_recent(&root)[0].cover, "");
    let albums = json!({ "topalbums": { "album": { "name": "LA RIDE", "artist": { "name": "Someone" }, "image": [{ "#text": star }, { "#text": "" }] } } });
    assert_eq!(parse_top_albums(&albums)[0].cover, "");
}

#[test]
fn album_covers_need_a_matching_album() {
    let album = |title: &str, artist: &str| md3_model::AlbumItem {
        title: title.to_owned(),
        artists: artist.to_owned(),
        artist_refs: vec![ArtistRef { name: artist.to_owned(), ..Default::default() }],
        cover: format!("https://img/{title}.jpg"),
        ..Default::default()
    };
    let found = best_album("Finding Her", "Kushagra", vec![album("Other Record", "Nobody"), album("Finding Her", "Kushagra")]);
    assert_eq!(found.unwrap().cover, "https://img/Finding Her.jpg");
    assert!(best_album("Finding Her", "Kushagra", vec![album("Finding Her", "Somebody Else Entirely")]).is_none());
    assert_eq!(cover_key(CoverKind::Album, " LA RIDE ", "Prabh"), "album|prabh|la ride");
    assert_eq!(CoverKind::from_key("album"), CoverKind::Album);
    assert_eq!(CoverKind::from_key("track"), CoverKind::Track);
}

/// Finds a cover for everything but titles with "Missing"; records how many run at once.
#[derive(Default)]
struct FakeCovers {
    calls: AtomicUsize,
    running: AtomicUsize,
    most: AtomicUsize,
}

impl CoverSearch for FakeCovers {
    fn find_cover<'a>(&'a self, kind: CoverKind, title: &'a str, _artist: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.most.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.running.fetch_sub(1, Ordering::SeqCst);
            (!title.contains("Missing")).then(|| format!("https://yt/{kind:?}/{title}.jpg"))
        })
    }
}

#[tokio::test]
async fn covers_are_looked_up_once_a_few_at_a_time_and_published() {
    let covers = Arc::new(FakeCovers::default());
    let service = feed_service_with("http://127.0.0.1:9/2.0/", (KEY, USER), Some(covers.clone()));
    for i in 0..8 {
        service.request_cover(CoverKind::Track, &format!("Song {i}"), "Artist");
    }
    // the same tile again (other case and spacing), a miss, an album
    service.request_cover(CoverKind::Track, " song 0", "ARTIST");
    service.request_cover(CoverKind::Track, "Missing", "Artist");
    service.request_cover(CoverKind::Album, "LA RIDE", "Prabh");
    let snapshot = settled(&service, |s| s.covers.len() == 9).await;
    assert_eq!(snapshot.covers[&cover_key(CoverKind::Track, "Song 3", "Artist")], "https://yt/Track/Song 3.jpg");
    assert_eq!(snapshot.covers[&cover_key(CoverKind::Album, "LA RIDE", "Prabh")], "https://yt/Album/LA RIDE.jpg");
    // wait for the miss too, then ask everything again: nothing new runs
    tokio::time::timeout(Duration::from_secs(5), async {
        while covers.calls.load(Ordering::SeqCst) < 10 || covers.running.load(Ordering::SeqCst) > 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    service.request_cover(CoverKind::Track, "Missing", "Artist");
    service.request_cover(CoverKind::Track, "Song 1", "Artist");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(covers.calls.load(Ordering::SeqCst), 10);
    assert!(covers.most.load(Ordering::SeqCst) <= COVER_PARALLEL);
    assert!(!service.snapshot().covers.contains_key(&cover_key(CoverKind::Track, "Missing", "Artist")));
}

#[tokio::test]
async fn without_a_cover_search_nothing_is_asked() {
    let service = feed_service("http://127.0.0.1:9/2.0/", (KEY, USER));
    service.request_cover(CoverKind::Track, "Song", "Artist");
    assert!(service.snapshot().covers.is_empty());
}
