//! Client and service tests against an in-process HTTP server (`tokio::net::TcpListener`): what goes on
//! the wire, retries, errors and paging. Nothing here talks to YouTube.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use sf_auth::YouTubeAccount;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::client::{ANDROID_VR, InnerTube, Login};
use super::library::LIKED_PLAYLISTS;
use super::parser_tests::fixture;
use super::{Rating, SearchFilter, YouTubeError, YouTubeService};

const ANONYMOUS_VISITOR: &str = "CgtANONYMOUS";

#[derive(Clone, Debug)]
struct Request {
    method: String,
    /// path and query
    target: String,
    /// lower-case names
    headers: HashMap<String, String>,
    body: Value,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    fn is_api(&self) -> bool {
        self.target.starts_with("/youtubei/v1/")
    }
}

/// Answers a request given how many API requests came before it. A `Value::String` answers a non-API request
/// with that text as it is (pages, scripts).
type Handler = Arc<dyn Fn(&Request, usize) -> (u16, Value) + Send + Sync>;

struct MockServer {
    base: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl MockServer {
    /// `GET /sw.js_data` always answers with `ANONYMOUS_VISITOR`; everything else goes to `handler`.
    async fn start(handler: impl Fn(&Request, usize) -> (u16, Value) + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let requests: Arc<Mutex<Vec<Request>>> = Arc::default();
        let handler: Handler = Arc::new(handler);
        let recorded = requests.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (handler, recorded) = (handler.clone(), recorded.clone());
                tokio::spawn(async move {
                    let Some(request) = read_request(&mut socket).await else { return };
                    let (status, body) = if request.target.starts_with("/sw.js_data") {
                        let text = format!(")]}}'\n[[\"x\",1,[\"en\",\"{ANONYMOUS_VISITOR}\"]]]");
                        write_response(&mut socket, 200, &text).await;
                        recorded.lock().unwrap().push(request);
                        return;
                    } else {
                        let earlier = recorded.lock().unwrap().iter().filter(|request| request.is_api()).count();
                        handler(&request, earlier)
                    };
                    let text = match body {
                        Value::String(text) if !request.is_api() => text,
                        other => other.to_string(),
                    };
                    recorded.lock().unwrap().push(request);
                    write_response(&mut socket, status, &text).await;
                });
            }
        });
        Self { base, requests }
    }

    fn api_requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().iter().filter(|request| request.is_api()).cloned().collect()
    }

    fn all_requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    fn service(&self) -> YouTubeService {
        let http = sf_api::Http::new(reqwest::Client::builder().no_proxy().build().expect("client"));
        let client = InnerTube::new(http).with_base_url(&self.base).with_retry_delay(Duration::from_millis(1));
        YouTubeService::with_client(client)
    }
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> Option<Request> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break end;
        }
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target) = (first.next()?.to_owned(), first.next()?.to_owned());
    let headers: HashMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_owned()))
        .collect();
    let length: usize = headers.get("content-length").and_then(|value| value.parse().ok()).unwrap_or(0);
    let mut body = buffer[head_end + 4..].to_vec();
    while body.len() < length {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
    Some(Request { method, target, headers, body })
}

async fn write_response(socket: &mut tokio::net::TcpStream, status: u16, body: &str) {
    let response = format!(
        "HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.shutdown().await;
}

fn account() -> YouTubeAccount {
    serde_json::from_value(fixture("auth_session.json")).expect("account fixture")
}

#[tokio::test]
async fn signed_in_requests_carry_the_cookie_signature_visitor_and_user() {
    let server = MockServer::start(|_, _| (200, fixture("browse_library.json"))).await;
    let service = server.service();
    service.set_account(Some(account()));

    let playlists = service.user_playlists().await.expect("playlists");
    let ids: Vec<&str> = playlists.iter().map(|playlist| playlist.playlist_id.as_str()).collect();
    assert_eq!(ids, ["LM", "PL1234567890abcdef", "PLsynthwave999xyz"]);

    let all = server.all_requests();
    assert_eq!(all.len(), 1, "no anonymous visitor lookup when the account has one: {all:?}");
    let request = &all[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/youtubei/v1/browse?prettyPrint=false");
    let account = account();
    assert_eq!(request.header("cookie"), Some(account.cookie.as_str()));
    let authorization = request.header("authorization").expect("authorization");
    assert!(authorization.starts_with("SAPISIDHASH "), "{authorization}");
    assert!(authorization.contains(" SAPISID1PHASH ") && authorization.contains(" SAPISID3PHASH "), "{authorization}");
    assert_eq!(request.header("x-goog-visitor-id"), account.visitor_data.as_deref());
    assert_eq!(request.header("x-youtube-client-name"), Some("67"));
    assert_eq!(request.header("origin"), Some("https://music.youtube.com"));
    assert_eq!(request.body["browseId"], LIKED_PLAYLISTS);
    assert_eq!(request.body["context"]["user"]["onBehalfOfUser"], "11223344556677889900");
    assert_eq!(request.body["context"]["client"]["clientName"], "WEB_REMIX");
    assert_eq!(request.body["context"]["client"]["visitorData"], json!(account.visitor_data));
}

#[tokio::test]
async fn anonymous_requests_carry_no_credentials() {
    let server = MockServer::start(|_, _| (200, fixture("search_response.json"))).await;
    let service = server.service();

    let page = service.search("queen", SearchFilter::Songs).await.expect("search");
    assert_eq!(page.items.len(), 4);
    // the anonymous visitor id is fetched once and reused
    service.search("queen", SearchFilter::Albums).await.expect("search");

    let all = server.all_requests();
    assert_eq!(all.iter().filter(|request| request.target.starts_with("/sw.js_data")).count(), 1);
    let api = server.api_requests();
    assert_eq!(api.len(), 2);
    for request in &api {
        assert_eq!(request.target, "/youtubei/v1/search?prettyPrint=false");
        assert_eq!(request.header("cookie"), None);
        assert_eq!(request.header("authorization"), None);
        assert_eq!(request.header("x-goog-visitor-id"), Some(ANONYMOUS_VISITOR));
        assert_eq!(request.body["context"]["client"]["visitorData"], ANONYMOUS_VISITOR);
        assert!(request.body["context"]["user"].get("onBehalfOfUser").is_none());
        assert_eq!(request.body["query"], "queen");
    }
    assert_eq!(api[0].body["params"], SearchFilter::Songs.params());
    assert_eq!(api[1].body["params"], SearchFilter::Albums.params());
}

#[tokio::test]
async fn clients_without_login_support_stay_anonymous_when_signed_in() {
    let server = MockServer::start(|_, _| (200, json!({}))).await;
    let service = server.service();
    service.set_account(Some(account()));

    service.client().post(&ANDROID_VR, "player", json!({ "videoId": "x" }), Login::IfSignedIn).await.expect("player");
    service.client().post(&super::client::WEB_REMIX, "next", json!({}), Login::Never).await.expect("next");

    for request in server.api_requests() {
        assert_eq!(request.header("cookie"), None, "{}", request.target);
        assert_eq!(request.header("authorization"), None, "{}", request.target);
        assert!(request.body["context"]["user"].get("onBehalfOfUser").is_none());
    }
    let player = &server.api_requests()[0];
    assert_eq!(player.header("x-youtube-client-name"), Some("28"));
    assert_eq!(player.body["context"]["client"]["deviceModel"], "Quest 3");
}

#[tokio::test]
async fn rate_limits_and_server_errors_are_retried() {
    let server = MockServer::start(|_, earlier| match earlier {
        0 => (429, fixture("error_responses.json")["rate_limited_429"].clone()),
        1 => (500, json!({})),
        _ => (200, fixture("search_response.json")),
    })
    .await;
    let page = server.service().search("queen", SearchFilter::Songs).await.expect("third attempt");
    assert_eq!(page.items.len(), 4);
    assert_eq!(server.api_requests().len(), 3);
}

#[tokio::test]
async fn retries_stop_after_four_attempts() {
    let server = MockServer::start(|_, _| (503, json!({}))).await;
    assert_eq!(server.service().search("q", SearchFilter::Songs).await, Err(YouTubeError::Http(503)));
    assert_eq!(server.api_requests().len(), 4);

    let server = MockServer::start(|_, _| (429, json!({}))).await;
    assert_eq!(server.service().search("q", SearchFilter::Songs).await, Err(YouTubeError::RateLimited));
    assert_eq!(server.api_requests().len(), 4);
}

#[tokio::test]
async fn an_unauthorized_answer_is_an_expired_session_without_retries() {
    let server = MockServer::start(|_, _| (401, fixture("error_responses.json")["unauthorized_401"].clone())).await;
    let service = server.service();
    service.set_account(Some(account()));
    assert_eq!(service.library(LIKED_PLAYLISTS).await, Err(YouTubeError::SessionExpired));
    assert_eq!(server.api_requests().len(), 1);
}

#[tokio::test]
async fn account_calls_without_an_account_fail_before_any_request() {
    let server = MockServer::start(|_, _| (200, json!({}))).await;
    let service = server.service();
    assert_eq!(service.library(LIKED_PLAYLISTS).await, Err(YouTubeError::NotSignedIn));
    assert_eq!(service.account_info().await, Err(YouTubeError::NotSignedIn));
    assert_eq!(service.rate_track("v", Rating::Like).await, Err(YouTubeError::NotSignedIn));
    assert_eq!(service.playlist("LM").await.map(|page| page.info), Err(YouTubeError::NotSignedIn));
    assert!(server.all_requests().is_empty());
}

#[tokio::test]
async fn a_multi_page_playlist_is_read_to_the_end() {
    let server = MockServer::start(|request, _| {
        if request.body.get("continuation").is_some() {
            (200, fixture("playlist_tracks_page2.json"))
        } else {
            (200, fixture("playlist_tracks_page1.json"))
        }
    })
    .await;
    let reported = Arc::new(Mutex::new(Vec::new()));
    let progress = reported.clone();
    let page = server
        .service()
        .playlist_full("PL1234567890abcdef", move |done| progress.lock().unwrap().push(done))
        .await
        .expect("playlist");
    let ids: Vec<&str> = page.tracks.iter().map(|row| row.track.track_id.as_str()).collect();
    assert_eq!(ids, ["vid_track_001", "vid_track_051"]);
    assert_eq!(page.continuation, None);
    assert_eq!(*reported.lock().unwrap(), [1, 2]);

    let api = server.api_requests();
    assert_eq!(api.len(), 2);
    assert_eq!(api[0].body["browseId"], "VLPL1234567890abcdef");
    let token = api[1].body["continuation"].as_str().expect("token in the body");
    assert!(token.contains("ctok_page2_token"), "{token}");
    assert!(api[1].body.get("browseId").is_none());
}

#[tokio::test]
async fn paging_stops_on_a_repeated_token() {
    let server = MockServer::start(|request, _| {
        if request.body.get("continuation").is_some() {
            // a broken server that hands out the same token again
            let mut page = fixture("playlist_tracks_page2.json");
            page["continuationContents"]["musicPlaylistShelfContinuation"]["continuations"] =
                json!([{ "nextContinuationData": { "continuation": "same-token" } }]);
            (200, page)
        } else {
            let mut page = fixture("playlist_tracks_page1.json");
            page["contents"]["twoColumnBrowseResultsRenderer"]["secondaryContents"]["sectionListRenderer"]["contents"][0]
                ["musicPlaylistShelfRenderer"]["continuations"] =
                json!([{ "nextContinuationData": { "continuation": "same-token" } }]);
            (200, page)
        }
    })
    .await;
    let page = server.service().playlist_full("PLloop", |_| {}).await.expect("playlist");
    assert_eq!(page.tracks.len(), 2);
    assert_eq!(server.api_requests().len(), 2, "the repeated token is not fetched again");
    assert_eq!(page.continuation.as_deref(), Some("same-token"));
}

#[tokio::test]
async fn search_continuations_go_in_the_query_and_edits_in_the_body() {
    let server = MockServer::start(|request, _| {
        if request.target.contains("edit_playlist") {
            (200, json!({ "status": "STATUS_SUCCEEDED", "playlistEditResults": [
                { "playlistEditVideoAddedResultData": { "setVideoId": "new-set" } }
            ]}))
        } else {
            (200, json!({}))
        }
    })
    .await;
    let service = server.service();
    service.set_account(Some(account()));

    let page = service.search_continuation("tok/=").await.expect("continuation");
    assert!(page.items.is_empty() && page.continuation.is_none());
    let added = service.add_to_playlist("VLPLmine", &["v1", "v2"]).await.expect("added");
    assert_eq!(added, ["new-set"]);

    let api = server.api_requests();
    assert_eq!(api[0].target, "/youtubei/v1/search?prettyPrint=false&continuation=tok%2F%3D&ctoken=tok%2F%3D");
    assert_eq!(api[1].target, "/youtubei/v1/browse/edit_playlist?prettyPrint=false");
    assert_eq!(api[1].body["playlistId"], "PLmine");
    assert_eq!(
        api[1].body["actions"],
        json!([
            { "action": "ACTION_ADD_VIDEO", "addedVideoId": "v1" },
            { "action": "ACTION_ADD_VIDEO", "addedVideoId": "v2" }
        ])
    );
}

#[tokio::test]
async fn a_failed_edit_status_is_an_error() {
    let server = MockServer::start(|_, _| (200, json!({ "status": "STATUS_FAILED" }))).await;
    let service = server.service();
    service.set_account(Some(account()));
    assert!(matches!(service.rename_playlist("PLx", "New").await, Err(YouTubeError::Unexpected(_))));
}

#[tokio::test]
async fn the_home_feed_is_cached_until_a_refresh_or_a_new_account() {
    let server = MockServer::start(|_, _| (200, json!({}))).await;
    let service = server.service();
    service.home(false).await.expect("home");
    service.home(false).await.expect("cached");
    assert_eq!(server.api_requests().len(), 1);
    service.home(true).await.expect("refreshed");
    assert_eq!(server.api_requests().len(), 2);
    service.set_account(Some(account()));
    service.home(false).await.expect("new identity");
    assert_eq!(server.api_requests().len(), 3);
}

#[tokio::test]
async fn a_youtube_playlist_link_is_imported_whole() {
    let server = MockServer::start(|request, _| {
        if request.body.get("continuation").is_some() {
            (200, fixture("playlist_tracks_page2.json"))
        } else {
            (200, fixture("playlist_tracks_page1.json"))
        }
    })
    .await;
    let result = server
        .service()
        .import_playlist("https://music.youtube.com/playlist?list=PL1234567890abcdef&si=x", |_, _| {})
        .await
        .expect("import");
    assert_eq!(result.title, "Chill Lofi Beats");
    assert_eq!(result.total, 2);
    assert!(result.missed.is_empty());
    let uris: Vec<&str> = result.tracks.iter().map(|track| track.uri.as_str()).collect();
    assert_eq!(uris, ["youtube:track:vid_track_001", "youtube:track:vid_track_051"]);
}

#[tokio::test]
async fn songs_of_a_file_are_matched_through_search() {
    let dir = std::env::temp_dir().join(format!("mf-youtube-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("Queen favourites.m3u");
    std::fs::write(&path, "#EXTM3U\n#EXTINF:354,Queen - Bohemian Rhapsody\nbohemian.mp3\n#EXTINF:100,Nobody - Unknown Song\nx.mp3\n")
        .expect("m3u");
    let server = MockServer::start(|_, _| (200, fixture("search_response.json"))).await;
    let progress = Arc::new(Mutex::new(Vec::new()));
    let seen = progress.clone();
    let result = server
        .service()
        .import_playlist(path.to_str().expect("path"), move |done, total| seen.lock().unwrap().push((done, total)))
        .await
        .expect("import");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(result.title, "Queen favourites");
    assert_eq!(result.total, 2);
    assert_eq!(result.tracks.len(), 1);
    assert_eq!(result.tracks[0].track_id, "fJ9rUzIMcZQ");
    assert_eq!(result.missed, ["Unknown Song · Nobody"]);
    let progress = progress.lock().unwrap();
    assert_eq!(progress.first(), Some(&(0, 2)));
    assert_eq!(progress.last(), Some(&(2, 2)));
    // every search asked for songs only
    assert!(server.api_requests().iter().all(|request| request.body["params"] == SearchFilter::Songs.params()));
}

// ---- stream resolution ----

/// The recorded ANDROID_VR player response, valid for hours, its stream URL on `base`.
fn player_response(base: &str) -> Value {
    let text = fixture("player_response_android_vr.json").to_string();
    let expire = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 6 * 3600;
    let text = text
        .replace("expire=1710003600", &format!("expire={expire}"))
        .replace("https://rr1---sn-4g5ednls.googlevideo.com", base);
    serde_json::from_str(&text).unwrap()
}

fn player_requests(server: &MockServer) -> Vec<Request> {
    server.api_requests().into_iter().filter(|request| request.target.starts_with("/youtubei/v1/player")).collect()
}

#[tokio::test]
async fn concurrent_resolves_share_one_request_and_the_cache() {
    let base = Arc::new(Mutex::new(String::new()));
    let shared = base.clone();
    let server = MockServer::start(move |_, _| {
        std::thread::sleep(Duration::from_millis(50));
        (200, player_response(&shared.lock().unwrap()))
    })
    .await;
    *base.lock().unwrap() = server.base.clone();
    let service = Arc::new(server.service());
    let calls = (0..3).map(|_| {
        let service = service.clone();
        tokio::spawn(async move { service.resolve_stream("fJ9rUzIMcZQ").await })
    });
    for call in futures::future::join_all(calls).await {
        assert_eq!(call.unwrap().expect("stream").itag, 140);
    }
    assert_eq!(player_requests(&server).len(), 1);
    let stream = service.resolve_stream("fJ9rUzIMcZQ").await.expect("cached");
    assert_eq!(player_requests(&server).len(), 1, "the second resolve came from the cache");
    assert_eq!(stream.source, "ANDROID_VR");

    let request = &player_requests(&server)[0];
    assert_eq!(request.body["videoId"], "fJ9rUzIMcZQ");
    assert_eq!(request.body["context"]["client"]["clientName"], "ANDROID_VR");
    assert_eq!(request.header("cookie"), None);

    service.invalidate_stream("fJ9rUzIMcZQ");
    service.resolve_stream("fJ9rUzIMcZQ").await.expect("again");
    assert_eq!(player_requests(&server).len(), 2);
}

#[tokio::test]
async fn a_bot_check_rests_the_anonymous_tier() {
    let server = MockServer::start(|_, _| (200, fixture("error_responses.json")["bot_challenge_login_required"].clone())).await;
    let service = server.service();
    let error = service.resolve_stream("abc").await.unwrap_err();
    assert!(matches!(&error, YouTubeError::Unplayable(reason) if reason.contains("not a robot")), "{error:?}");
    let again = service.resolve_stream("abc").await.unwrap_err();
    assert_eq!(again, error, "while it rests, the bot check is still the reason");
    assert_eq!(player_requests(&server).len(), 1, "ANDROID_VR rests and nobody is signed in");
}

// ---- the web tier: player script, BotGuard, PO token ----

const PLAYER_PATH: &str = "/s/player/test/player_es6.vflset/en_US/base.js";
/// A BotGuard stand-in: its minter answers the integrity token's bytes followed by the identifier's.
const FAKE_BOTGUARD: &str = r#"globalThis.fakebg={a:function(p,cb){cb(function(done,args){args[2][0]=function(it){return function(id){var o=new Uint8Array(it.length+id.length);o.set(it);o.set(id,it.length);return o}};done("snap:"+p)})}}"#;

fn integrity_ok() -> Value {
    json!(["AQID", 3600, 100])
}

/// The music page, the player script and BotGuard's service; `None` for anything else.
fn web_assets(request: &Request, integrity: &Value) -> Option<(u16, Value)> {
    match request.target.as_str() {
        "/" => Some((200, Value::String(format!(r#"<script>ytcfg.set({{"jsUrl":"{PLAYER_PATH}"}})</script>"#)))),
        PLAYER_PATH => Some((200, Value::String(super::cipher::tests::SYNTHETIC_PLAYER.to_owned()))),
        "/api/jnn/v1/Create" => Some((200, json!([["id", [FAKE_BOTGUARD], null, "h", "PROGRAM", "fakebg", null, "blob"]]))),
        "/api/jnn/v1/GenerateIT" => Some((200, integrity.clone())),
        _ => None,
    }
}

/// A web player response whose only AAC format is ciphered (`s` reversed, `n=abc`), its URL on `base`.
fn web_player_response(base: &str, video_id: &Value) -> Value {
    let expire = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 6 * 3600;
    let mut form = reqwest::Url::parse("http://x/").unwrap();
    form.query_pairs_mut()
        .append_pair("s", "321")
        .append_pair("sp", "sig")
        .append_pair("url", &format!("{base}/videoplayback?n=abc&expire={expire}"));
    json!({
        "playabilityStatus": { "status": "OK" },
        "videoDetails": { "videoId": video_id, "title": "Bohemian Rhapsody", "author": "Queen", "lengthSeconds": "355" },
        "streamingData": { "adaptiveFormats": [
            { "itag": 251, "mimeType": "audio/webm; codecs=\"opus\"", "bitrate": 150000, "signatureCipher": form.query().unwrap() },
            { "itag": 140, "mimeType": "audio/mp4; codecs=\"mp4a.40.2\"", "bitrate": 130000, "signatureCipher": form.query().unwrap() },
        ] },
    })
}

/// What the fake minter signs for `video_id` (integrity token `AQID` = bytes 1, 2, 3).
fn expected_token(video_id: &str) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = [1u8, 2, 3].into_iter().chain(video_id.bytes()).collect();
    base64::engine::general_purpose::URL_SAFE.encode(bytes)
}

fn client_name(request: &Request) -> &str {
    request.body["context"]["client"]["clientName"].as_str().unwrap_or_default()
}

fn count(server: &MockServer, target: &str) -> usize {
    server.all_requests().iter().filter(|request| request.target.starts_with(target)).count()
}

/// A server where ANDROID_VR meets `vr` and the web player answers `web_player_response`.
async fn web_server(vr: Value, integrity: Value) -> MockServer {
    let base = Arc::new(Mutex::new(String::new()));
    let shared = base.clone();
    let server = MockServer::start(move |request, _| {
        if let Some(answer) = web_assets(request, &integrity) {
            return answer;
        }
        if client_name(request) == "ANDROID_VR" { (200, vr.clone()) } else { (200, web_player_response(&shared.lock().unwrap(), &request.body["videoId"])) }
    })
    .await;
    *base.lock().unwrap() = server.base.clone();
    server
}

#[tokio::test]
async fn the_web_tier_deciphers_the_url_and_adds_a_video_bound_token() {
    let server = web_server(fixture("error_responses.json")["bot_challenge_login_required"].clone(), integrity_ok()).await;
    let service = server.service();
    let stream = service.resolve_stream("fJ9rUzIMcZQ").await.expect("stream");
    assert_eq!(stream.source, "WEB_REMIX");
    assert_eq!(stream.itag, 140);
    assert_eq!(stream.user_agent, super::client::WEB_REMIX.user_agent);
    let token = expected_token("fJ9rUzIMcZQ");
    let prefix = format!("{}/videoplayback?n=cba&expire=", server.base);
    assert!(stream.url.starts_with(&prefix), "{}", stream.url);
    assert!(stream.url.ends_with(&format!("&alr=yes&sig=123&pot={token}")), "{}", stream.url);

    let web: Vec<Request> = player_requests(&server).into_iter().filter(|request| client_name(request) == "WEB_REMIX").collect();
    assert_eq!(web.len(), 1);
    assert_eq!(web[0].body["playbackContext"]["contentPlaybackContext"]["signatureTimestamp"], 19999);
    assert_eq!(web[0].body["serviceIntegrityDimensions"]["poToken"], json!(token));
    assert_eq!(web[0].header("cookie"), None, "signed out, the web tier goes anonymous");
    let create = server.all_requests().into_iter().find(|request| request.target == "/api/jnn/v1/Create").expect("Create");
    assert_eq!(create.body, json!([super::potoken::REQUEST_KEY]));
    assert_eq!(create.header("content-type"), Some("application/json+protobuf"));
    assert_eq!(create.header("x-goog-api-key"), Some("AIzaTestKeyForTheFakeBotGuard01"), "the key the player script carries");
    let generate = server.all_requests().into_iter().find(|request| request.target == "/api/jnn/v1/GenerateIT").expect("GenerateIT");
    assert_eq!(generate.body, json!([super::potoken::REQUEST_KEY, "snap:PROGRAM"]));

    // another video: the script and the minter are reused, the token is the new video's
    let other = service.resolve_stream("other_video").await.expect("stream");
    assert!(other.url.ends_with(&format!("&pot={}", expected_token("other_video"))), "{}", other.url);
    assert_eq!(count(&server, PLAYER_PATH), 1, "the player script is loaded once");
    // a refused URL forgets its token and the script that made it; the minter signs again
    service.invalidate_stream("fJ9rUzIMcZQ");
    let again = service.resolve_stream("fJ9rUzIMcZQ").await.expect("stream");
    assert!(again.url.ends_with(&format!("&pot={token}")));
    assert_eq!(count(&server, PLAYER_PATH), 2, "the player script was loaded again after the refusal");
    assert_eq!(count(&server, "/api/jnn/v1/Create"), 1, "one minter serves every video");
    assert_eq!(count(&server, "/api/jnn/v1/GenerateIT"), 1);
}

#[tokio::test]
async fn warming_up_loads_the_player_script_and_the_minter_before_the_first_song() {
    let server = web_server(fixture("error_responses.json")["bot_challenge_login_required"].clone(), integrity_ok()).await;
    let service = server.service();
    service.warm_streams().await;
    assert!(player_requests(&server).is_empty(), "warming up asks for no song");
    assert_eq!((count(&server, PLAYER_PATH), count(&server, "/api/jnn/v1/Create"), count(&server, "/api/jnn/v1/GenerateIT")), (1, 1, 1));
    // a second warm-up has nothing left to do
    service.warm_streams().await;
    assert_eq!(server.all_requests().len(), 4);
    let stream = service.resolve_stream("fJ9rUzIMcZQ").await.expect("stream");
    assert!(stream.url.ends_with(&format!("&pot={}", expected_token("fJ9rUzIMcZQ"))), "{}", stream.url);
    assert_eq!((count(&server, PLAYER_PATH), count(&server, "/api/jnn/v1/Create"), count(&server, "/api/jnn/v1/GenerateIT")), (1, 1, 1));
    // signing in drops the minter: the next warm-up starts a new one with the same script
    service.set_account(Some(account()));
    service.warm_streams().await;
    assert_eq!((count(&server, PLAYER_PATH), count(&server, "/api/jnn/v1/Create")), (1, 2));
}

#[tokio::test]
async fn the_signed_in_web_tier_follows_a_refusal() {
    let server = web_server(fixture("error_responses.json")["unplayable_age_restricted"].clone(), integrity_ok()).await;
    let service = server.service();
    service.set_account(Some(account()));
    let stream = service.resolve_stream("fJ9rUzIMcZQ").await.expect("stream");
    assert_eq!(stream.source, "WEB_REMIX");
    let requests = player_requests(&server);
    assert_eq!(requests.len(), 3);
    let visitor = |request: &Request| request.body["context"]["client"]["visitorData"].as_str().map(str::to_owned);
    assert_eq!((client_name(&requests[0]), requests[0].header("cookie")), ("ANDROID_VR", None));
    assert_ne!(visitor(&requests[0]), account().visitor_data, "the anonymous request never carries the account's id");
    assert_eq!((client_name(&requests[1]), requests[1].header("cookie")), ("ANDROID_VR", None), "ANDROID_VR takes no cookies");
    assert_eq!(visitor(&requests[1]), account().visitor_data, "then ANDROID_VR with the account's visitor id");
    assert!(requests[1].body["context"]["user"].get("onBehalfOfUser").is_none());
    assert!(requests[2].header("cookie").is_some(), "WEB_REMIX goes as the user");
    assert_eq!(requests[2].body["context"]["user"]["onBehalfOfUser"], "11223344556677889900");
    assert!(requests[2].body["serviceIntegrityDimensions"]["poToken"].is_string());
    // an age wall does not rest ANDROID_VR
    service.invalidate_stream("fJ9rUzIMcZQ");
    service.resolve_stream("fJ9rUzIMcZQ").await.expect("stream");
    assert_eq!(client_name(&player_requests(&server)[3]), "ANDROID_VR");
    assert_eq!(player_requests(&server).len(), 6);
}

/// A server where ANDROID_VR answers `user` to requests with the account's visitor id and a bot check to the rest;
/// no web player.
async fn bot_checked_server(user: Value) -> MockServer {
    let base = Arc::new(Mutex::new(String::new()));
    let shared = base.clone();
    let server = MockServer::start(move |request, _| {
        let bot_check = fixture("error_responses.json")["bot_challenge_login_required"].clone();
        let for_user = request.body["context"]["client"]["visitorData"].as_str() == account().visitor_data.as_deref();
        match (for_user, &user) {
            (true, Value::Null) => (200, player_response(&shared.lock().unwrap())),
            (true, Value::Number(status)) => (status.as_u64().unwrap_or(500) as u16, json!({})),
            (true, answer) => (200, answer.clone()),
            (false, _) => (200, bot_check),
        }
    })
    .await;
    *base.lock().unwrap() = server.base.clone();
    server
}

#[tokio::test]
async fn signed_in_android_vr_goes_with_the_accounts_visitor_id_and_no_cookies() {
    let server = bot_checked_server(Value::Null).await;
    let service = server.service();
    // signed out: the bot check, and ANDROID_VR rests
    let error = service.resolve_stream("fJ9rUzIMcZQ").await.unwrap_err();
    assert!(matches!(&error, YouTubeError::Unplayable(reason) if reason.contains("not a robot")), "{error:?}");
    // signing in forgets the rest; the anonymous client is refused again, the account's device is not
    service.set_account(Some(account()));
    let stream = service.resolve_stream("fJ9rUzIMcZQ").await.expect("stream");
    assert_eq!((stream.source.as_str(), stream.itag), ("ANDROID_VR", 140));
    assert_eq!(stream.user_agent, ANDROID_VR.user_agent);
    let requests = player_requests(&server);
    let user = requests.last().unwrap();
    assert_eq!(client_name(user), "ANDROID_VR");
    assert_eq!(user.header("cookie"), None);
    assert_eq!(user.header("authorization"), None);
    assert_eq!(user.header("x-goog-visitor-id"), account().visitor_data.as_deref());
    assert!(user.body["context"]["user"].get("onBehalfOfUser").is_none(), "answered 401 without the cookie");
    assert_eq!(user.header("x-youtube-client-name"), Some("28"));
    // the next song skips the resting anonymous client
    service.resolve_stream("other").await.expect("stream");
    let after = player_requests(&server);
    assert_eq!(after.len(), requests.len() + 1);
    assert_eq!(after.last().unwrap().header("x-goog-visitor-id"), account().visitor_data.as_deref());
}

#[tokio::test]
async fn a_bot_check_or_a_refused_request_rests_the_accounts_tier_too() {
    let bot_check = fixture("error_responses.json")["bot_challenge_login_required"].clone();
    for answer in [bot_check, json!(401), json!(400)] {
        let server = bot_checked_server(answer.clone()).await;
        let service = server.service();
        service.set_account(Some(account()));
        for _ in 0..2 {
            let error = service.resolve_stream("abc").await.unwrap_err();
            assert!(matches!(&error, YouTubeError::Unplayable(reason) if reason.contains("not a robot")), "{answer}: {error:?}");
        }
        let vr = player_requests(&server).into_iter().filter(|request| client_name(request) == "ANDROID_VR").count();
        assert_eq!(vr, 2, "{answer}: one anonymous and one request for the user, then both rest");
    }
}

#[tokio::test]
async fn a_bot_checked_account_gets_the_web_stream_anonymously() {
    let base = Arc::new(Mutex::new(String::new()));
    let shared = base.clone();
    let server = MockServer::start(move |request, _| {
        if let Some(answer) = web_assets(request, &integrity_ok()) {
            return answer;
        }
        let bot_check = fixture("error_responses.json")["bot_challenge_login_required"].clone();
        if client_name(request) == "ANDROID_VR" || request.header("cookie").is_some() {
            (200, bot_check)
        } else {
            (200, web_player_response(&shared.lock().unwrap(), &request.body["videoId"]))
        }
    })
    .await;
    *base.lock().unwrap() = server.base.clone();
    let service = server.service();
    service.set_account(Some(account()));
    let stream = service.resolve_stream("fJ9rUzIMcZQ").await.expect("stream");
    assert_eq!(stream.source, "WEB_REMIX");
    assert!(stream.url.ends_with(&format!("&pot={}", expected_token("fJ9rUzIMcZQ"))), "{}", stream.url);
    let web: Vec<Request> = player_requests(&server).into_iter().filter(|request| client_name(request) == "WEB_REMIX").collect();
    assert_eq!(web.len(), 2);
    assert!(web[0].header("cookie").is_some(), "first as the user");
    assert_eq!(web[1].header("cookie"), None, "then anonymously");
    assert_ne!(web[1].header("x-goog-visitor-id"), account().visitor_data.as_deref());
    assert_eq!(count(&server, "/api/jnn/v1/Create"), 1, "both go with the same video-bound token");
}

#[tokio::test]
async fn signing_in_or_out_forgets_the_tokens_the_minter_and_the_streams() {
    let server = web_server(fixture("error_responses.json")["bot_challenge_login_required"].clone(), integrity_ok()).await;
    let service = server.service();
    service.set_account(Some(account()));
    service.resolve_stream("fJ9rUzIMcZQ").await.expect("stream");
    let web = || player_requests(&server).into_iter().filter(|request| client_name(request) == "WEB_REMIX").count();
    assert_eq!((web(), count(&server, "/api/jnn/v1/Create")), (1, 1));
    // the same account again (a refreshed card): nothing is forgotten
    service.set_account(Some(account()));
    service.resolve_stream("fJ9rUzIMcZQ").await.expect("cached");
    assert_eq!(web(), 1, "the stream came from the cache");
    // signed out: the stream, the token and the minter belonged to the account
    service.set_account(None);
    let stream = service.resolve_stream("fJ9rUzIMcZQ").await.expect("stream");
    assert!(stream.url.ends_with(&format!("&pot={}", expected_token("fJ9rUzIMcZQ"))));
    assert_eq!(web(), 2, "resolved again");
    assert_eq!(count(&server, "/api/jnn/v1/Create"), 2, "a fresh BotGuard challenge for the new identity");
    let last = player_requests(&server).into_iter().filter(|request| client_name(request) == "WEB_REMIX").last().unwrap();
    assert_eq!(last.header("cookie"), None);
    assert_ne!(last.header("x-goog-visitor-id"), account().visitor_data.as_deref());
}

#[tokio::test]
async fn without_an_integrity_token_the_stream_goes_without_pot() {
    let refused = json!([null, 3600, 100]);
    let server = web_server(fixture("error_responses.json")["bot_challenge_login_required"].clone(), refused).await;
    let service = server.service();
    let stream = service.resolve_stream("fJ9rUzIMcZQ").await.expect("stream");
    assert!(stream.url.ends_with("&alr=yes&sig=123"), "{}", stream.url);
    let web: Vec<Request> = player_requests(&server).into_iter().filter(|request| client_name(request) == "WEB_REMIX").collect();
    assert!(web[0].body.get("serviceIntegrityDimensions").is_none());
    service.resolve_stream("other").await.expect("stream");
    assert_eq!(count(&server, "/api/jnn/v1/Create"), 1, "a refused BotGuard is not asked again right away");
}

#[tokio::test]
async fn an_external_provider_supplies_the_token() {
    let base = Arc::new(Mutex::new(String::new()));
    let shared = base.clone();
    let server = MockServer::start(move |request, _| {
        if request.target == "/pot/get_pot" {
            return (200, json!({ "poToken": format!("EXT-{}", request.body["content_binding"].as_str().unwrap_or_default()) }));
        }
        if let Some(answer) = web_assets(request, &integrity_ok()) {
            return answer;
        }
        if client_name(request) == "ANDROID_VR" {
            (200, fixture("error_responses.json")["bot_challenge_login_required"].clone())
        } else {
            (200, web_player_response(&shared.lock().unwrap(), &request.body["videoId"]))
        }
    })
    .await;
    *base.lock().unwrap() = server.base.clone();
    let service = server.service();
    service.set_po_token_server(Some(format!("{}/pot/", server.base)));
    let stream = service.resolve_stream("fJ9rUzIMcZQ").await.expect("stream");
    assert!(stream.url.ends_with("&pot=EXT-fJ9rUzIMcZQ"), "{}", stream.url);
    assert_eq!(count(&server, "/api/jnn/v1/"), 0, "the built-in minter was not needed");
    let asked = server.all_requests().into_iter().find(|request| request.target == "/pot/get_pot").expect("provider");
    assert_eq!(asked.method, "POST");
    assert_eq!(asked.body, json!({ "content_binding": "fJ9rUzIMcZQ" }));
}

#[tokio::test]
async fn a_resting_bot_check_outranks_a_web_refusal() {
    // the recorded web response only has an Opus format: no stream, but not the tier's fault
    let server = MockServer::start(|request, _| {
        if let Some(answer) = web_assets(request, &integrity_ok()) {
            return answer;
        }
        if client_name(request) == "ANDROID_VR" {
            (200, fixture("error_responses.json")["bot_challenge_login_required"].clone())
        } else {
            (200, fixture("player_response_web_remix.json"))
        }
    })
    .await;
    let service = server.service();
    for _ in 0..3 {
        let error = service.resolve_stream("x").await.unwrap_err();
        assert!(matches!(&error, YouTubeError::Unplayable(reason) if reason.contains("not a robot")), "{error:?}");
    }
    let clients: Vec<String> = player_requests(&server).iter().map(|request| client_name(request).to_owned()).collect();
    assert_eq!(clients, ["ANDROID_VR", "WEB_REMIX", "WEB_REMIX", "WEB_REMIX"], "ANDROID_VR rests; refusals do not rest the web tier");
}

#[tokio::test]
async fn the_web_tier_rests_after_two_technical_failures() {
    let server = MockServer::start(|request, _| {
        if request.target == PLAYER_PATH {
            return (404, Value::String(String::new()));
        }
        if let Some(answer) = web_assets(request, &integrity_ok()) {
            return answer;
        }
        (200, fixture("error_responses.json")["unplayable_age_restricted"].clone())
    })
    .await;
    let service = server.service();
    for _ in 0..3 {
        let error = service.resolve_stream("x").await.unwrap_err();
        assert_eq!(error, YouTubeError::Unplayable("Sign in to confirm your age".into()));
    }
    assert_eq!(count(&server, PLAYER_PATH), 2, "the third resolve skipped the resting web tier");
    assert_eq!(player_requests(&server).len(), 3, "ANDROID_VR was asked every time");
}

#[tokio::test]
async fn a_refused_preheat_forgets_the_stream() {
    let base = Arc::new(Mutex::new(String::new()));
    let shared = base.clone();
    let server = MockServer::start(move |request, _| {
        if request.target.starts_with("/videoplayback") { (403, Value::Null) } else { (200, player_response(&shared.lock().unwrap())) }
    })
    .await;
    *base.lock().unwrap() = server.base.clone();
    let service = server.service();
    service.preheat_stream("fJ9rUzIMcZQ").await;
    let preheat = server.all_requests().into_iter().find(|request| request.target.starts_with("/videoplayback")).expect("preheat");
    assert_eq!(preheat.header("range"), Some("bytes=0-1024"));
    assert_eq!(preheat.header("user-agent"), Some(ANDROID_VR.user_agent));
    service.resolve_stream("fJ9rUzIMcZQ").await.expect("stream");
    assert_eq!(player_requests(&server).len(), 2, "resolved again after the 403");
}

#[tokio::test]
async fn songs_without_a_length_get_it_from_one_get_queue_request() {
    use super::models::{Shelf, ShelvesPage, YtItem};
    let server = MockServer::start(|_, _| (200, super::parser_tests::get_queue_response())).await;
    let service = server.service();
    let song = |id: &str, duration_ms: i64| {
        YtItem::Track(md3_model::TrackItem {
            track_id: id.to_owned(),
            uri: format!("youtube:track:{id}"),
            duration_ms,
            ..Default::default()
        })
    };
    let shelf = |items| Shelf { title: "Quick picks".to_owned(), items, more: None, continuation: None, ..Default::default() };
    let page = || ShelvesPage {
        shelves: vec![shelf(vec![song("plain", 0), song("wrapped", 0), song("known", 5000)]), shelf(vec![song("plain", 0)])],
        continuation: None,
    };
    let mut first = page();
    service.fill_durations(&mut first).await;
    let lengths: Vec<i64> = first.shelves.iter().flat_map(|shelf| &shelf.items).map(|item| match item {
        YtItem::Track(track) => track.duration_ms,
        _ => -1,
    }).collect();
    assert_eq!(lengths, [196_000, 3_723_000, 5000, 196_000]);
    let requests = server.api_requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].target.starts_with("/youtubei/v1/music/get_queue"), "{}", requests[0].target);
    assert_eq!(requests[0].body["videoIds"], json!(["plain", "wrapped"]));
    // a known length comes from the cache, without a request
    let mut again = ShelvesPage { shelves: vec![shelf(vec![song("plain", 0)])], continuation: None };
    service.fill_durations(&mut again).await;
    assert_eq!(server.api_requests().len(), 1);
}

#[tokio::test]
async fn a_play_is_reported_with_the_tracking_address_of_the_signed_in_player_response() {
    let server = MockServer::start(|request, _| {
        if request.target.starts_with("/youtubei/v1/player") {
            (200, json!({ "playbackTracking": { "videostatsPlaybackUrl": {
                "baseUrl": "https://s.youtube.com/api/stats/playback?docid=vid1&ei=abc"
            } } }))
        } else {
            (204, Value::String(String::new()))
        }
    })
    .await;
    let service = server.service();
    assert_eq!(service.report_play("vid1", None).await, Err(YouTubeError::NotSignedIn));
    assert!(server.all_requests().is_empty());

    service.set_account(Some(account()));
    service.report_play("vid1", Some("PLmix")).await.expect("reported");
    let api = server.api_requests();
    assert_eq!(api.len(), 1);
    assert_eq!(api[0].body["videoId"], "vid1");
    assert_eq!(api[0].body["playlistId"], "PLmix");
    assert_eq!(api[0].body["context"]["client"]["clientName"], "WEB_REMIX");
    assert!(api[0].header("cookie").is_some());
    let report = server.all_requests().into_iter().find(|r| r.target.starts_with("/api/stats/playback")).expect("report");
    assert_eq!(report.method, "GET");
    let (head, cpn) = report.target.split_once("&cpn=").expect("cpn");
    assert_eq!(head, "/api/stats/playback?docid=vid1&ei=abc&ver=2&c=WEB_REMIX");
    assert_eq!(cpn.split('&').next().unwrap().len(), 16);
    assert!(report.target.contains("&list=PLmix&referrer=https%3A%2F%2Fmusic.youtube.com%2Fplaylist%3Flist%3DPLmix"));
    assert!(report.header("cookie").is_some());
    assert!(report.header("authorization").is_some_and(|value| value.starts_with("SAPISIDHASH")));
}

#[tokio::test]
async fn a_player_response_without_tracking_or_a_refused_report_is_an_error() {
    let server = MockServer::start(|request, _| {
        if request.is_api() { (200, json!({ "playabilityStatus": { "status": "OK" } })) } else { (403, Value::String(String::new())) }
    })
    .await;
    let service = server.service();
    service.set_account(Some(account()));
    assert!(matches!(service.report_play("vid1", None).await, Err(YouTubeError::Unexpected(_))));

    let server = MockServer::start(|request, _| {
        if request.is_api() {
            (200, json!({ "playbackTracking": { "videostatsPlaybackUrl": { "baseUrl": "https://s.youtube.com/api/stats/playback?docid=v" } } }))
        } else {
            (403, Value::String(String::new()))
        }
    })
    .await;
    let service = server.service();
    service.set_account(Some(account()));
    assert_eq!(service.report_play("v", None).await, Err(YouTubeError::Http(403)));
}

/// `FEmusic_history`: one music shelf per day group.
fn history_response() -> Value {
    let row = |id: &str, title: &str| json!({ "musicResponsiveListItemRenderer": {
        "playlistItemData": { "videoId": id },
        "flexColumns": [
            { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": title }] } } },
            { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Singer" }] } } }
        ]
    }});
    let shelf = |title: &str, rows: Vec<Value>| json!({ "musicShelfRenderer": {
        "title": { "runs": [{ "text": title }] }, "contents": rows
    }});
    json!({ "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{ "tabRenderer": { "content": {
        "sectionListRenderer": { "contents": [
            shelf("Today", vec![row("t1", "First"), row("t2", "Second")]),
            shelf("Yesterday", vec![row("y1", "Third")]),
            shelf("March 2025", vec![row("m1", "Fourth")])
        ]}
    }}}]}}})
}

#[tokio::test]
async fn the_history_becomes_recent_plays_in_youtubes_order_with_day_estimates() {
    let server = MockServer::start(|_, _| (200, history_response())).await;
    let service = server.service();
    assert_eq!(service.history().await, Err(YouTubeError::NotSignedIn));
    service.set_account(Some(account()));
    let page = service.history().await.expect("history");
    assert_eq!(server.api_requests()[0].body["browseId"], "FEmusic_history");

    let now: jiff::Timestamp = "2026-11-13T12:00:00Z".parse().unwrap();
    let records = crate::history::youtube_records(&page, now);
    let got: Vec<(&str, &str)> = records.iter().map(|r| (r.track.uri.as_str(), r.played_at.as_str())).collect();
    assert_eq!(
        got,
        [
            ("youtube:track:t1", "2026-11-13T11:59:00Z"),
            ("youtube:track:t2", "2026-11-13T11:58:00Z"),
            ("youtube:track:y1", "2026-11-12T11:59:00Z"),
            ("youtube:track:m1", "2026-11-05T11:59:00Z"),
        ]
    );
    assert_eq!(records[0].track.title, "First");
}
