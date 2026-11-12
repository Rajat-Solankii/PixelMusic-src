//! YouTube Music sign-in (port of soundwave1 `YouTubeAuth.kt`).
//!
//! The user signs in themselves in a real browser window (Edge or Chrome) running on its own private profile
//! folder. The app never sees the password: once the window lands on music.youtube.com the session cookies and
//! DATASYNC_ID are read over the browser's local DevTools port. The session itself is kept in the OS credential
//! store (`Slot::YouTube`), never in a plain file.
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use tokio_tungstenite::tungstenite::Message;

/// Origin every authenticated InnerTube request is signed for.
pub const ORIGIN: &str = "https://music.youtube.com";
const START_URL: &str =
    "https://accounts.google.com/ServiceLogin?service=youtube&passive=true&continue=https://music.youtube.com/";
/// Cookies that carry the SAPISID secret, in lookup order.
const SAPISID_COOKIES: [&str; 3] = ["SAPISID", "__Secure-3PAPISID", "__Secure-1PAPISID"];
const POLL_INTERVAL: Duration = Duration::from_millis(1200);
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// A signed-in YouTube Music session. No derived Debug: `cookie` is a working secret.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YouTubeAccount {
    /// Full `name=value; ...` cookie header.
    pub cookie: String,
    #[serde(default)]
    pub data_sync_id: Option<String>,
    #[serde(default)]
    pub visitor_data: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
}

impl std::fmt::Debug for YouTubeAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("YouTubeAccount").field("name", &self.name).finish_non_exhaustive()
    }
}

impl YouTubeAccount {
    /// Fallback for machines without Edge/Chrome: a `Cookie:` header copied from the browser's DevTools.
    /// Accepts the bare value or the whole `Cookie: ...` line.
    pub fn from_cookie_string(input: &str) -> Result<Self, YouTubeAuthError> {
        let joined = input.replace(['\r', '\n'], " ");
        let trimmed = joined.trim();
        let value = match trimmed.get(..7) {
            Some(prefix) if prefix.eq_ignore_ascii_case("cookie:") => &trimmed[7..],
            _ => trimmed,
        };
        let account = Self::from_pairs(parse_cookies(value), None, None);
        if account.sapisid().is_none() {
            return Err(YouTubeAuthError::NotSignedIn);
        }
        Ok(account)
    }

    fn from_pairs(pairs: Vec<(String, String)>, data_sync_id: Option<String>, visitor_data: Option<String>) -> Self {
        let cookie = pairs.iter().map(|(name, value)| format!("{name}={value}")).collect::<Vec<_>>().join("; ");
        Self { cookie, data_sync_id, visitor_data, name: None, email: None, avatar: None }
    }

    /// The SAPISID secret used to sign requests; `None` means the cookies are not a signed-in session.
    pub fn sapisid(&self) -> Option<String> {
        let cookies = parse_cookies(&self.cookie);
        SAPISID_COOKIES.iter().find_map(|wanted| {
            cookies.iter().find(|(name, value)| name == wanted && !value.is_empty()).map(|(_, value)| value.clone())
        })
    }

    /// `Authorization` header value for a request sent now from `origin`.
    pub fn authorization(&self, origin: &str) -> Option<String> {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs());
        Some(authorization_header(timestamp, &self.sapisid()?, origin))
    }
}

/// Splits a cookie header into pairs, keeping the first occurrence of each name.
pub fn parse_cookies(header: &str) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    for part in header.split(';') {
        let Some((name, value)) = part.split_once('=') else { continue };
        let name = name.trim();
        if name.is_empty() || pairs.iter().any(|(seen, _)| seen == name) {
            continue;
        }
        pairs.push((name.to_owned(), value.trim().to_owned()));
    }
    pairs
}

/// `sha1("{timestamp} {sapisid} {origin}")` in lowercase hex (InnerTube.kt:247).
pub fn compute_sapisid_hash(timestamp: u64, sapisid: &str, origin: &str) -> String {
    format!("{:x}", Sha1::digest(format!("{timestamp} {sapisid} {origin}").as_bytes()))
}

/// The three-part `SAPISIDHASH` authorization value YouTube Music expects.
pub fn authorization_header(timestamp: u64, sapisid: &str, origin: &str) -> String {
    let hash = compute_sapisid_hash(timestamp, sapisid, origin);
    format!("SAPISIDHASH {timestamp}_{hash} SAPISID1PHASH {timestamp}_{hash} SAPISID3PHASH {timestamp}_{hash}")
}

/// `yt.config_.DATASYNC_ID` is `"<id>||"` or `"<id>||<other>"`; the part before `||` is what goes into
/// `context.user.onBehalfOfUser` (YouTubeAuth.kt:189).
pub fn normalize_data_sync_id(raw: &str) -> Option<String> {
    let id = raw.trim().split("||").next().unwrap_or_default().trim();
    (!id.is_empty() && !id.eq_ignore_ascii_case("null")).then(|| id.to_owned())
}

/// Sign-in failures. Display is interface text through mf_i18n.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YouTubeAuthError {
    NoBrowser,
    Launch,
    Closed,
    TimedOut,
    NotSignedIn,
}

impl std::fmt::Display for YouTubeAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::NoBrowser => mf_i18n::t("Microsoft Edge or Google Chrome is needed to sign in to YouTube Music."),
            Self::Launch => mf_i18n::t("Couldn't open the YouTube Music sign-in window."),
            Self::Closed => mf_i18n::t("The sign-in window was closed before signing in."),
            Self::TimedOut => mf_i18n::t("YouTube Music sign-in timed out."),
            Self::NotSignedIn => mf_i18n::t("These cookies don't contain a signed-in YouTube Music session."),
        };
        f.write_str(&text)
    }
}

impl std::error::Error for YouTubeAuthError {}

fn browser_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    #[cfg(windows)]
    {
        let under = |variable: &str, tail: &str| std::env::var_os(variable).map(|root| PathBuf::from(root).join(tail));
        candidates.extend(
            [
                under("ProgramFiles(x86)", r"Microsoft\Edge\Application\msedge.exe"),
                under("ProgramFiles", r"Microsoft\Edge\Application\msedge.exe"),
                under("ProgramFiles", r"Google\Chrome\Application\chrome.exe"),
                under("ProgramFiles(x86)", r"Google\Chrome\Application\chrome.exe"),
                under("LOCALAPPDATA", r"Google\Chrome\Application\chrome.exe"),
            ]
            .into_iter()
            .flatten(),
        );
    }
    #[cfg(target_os = "macos")]
    candidates.extend(
        [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ]
        .map(PathBuf::from),
    );
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(path) = std::env::var_os("PATH") {
        for name in
            ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "microsoft-edge", "microsoft-edge-stable"]
        {
            candidates.extend(std::env::split_paths(&path).map(|dir| dir.join(name)));
        }
    }
    candidates
}

/// The Chromium-based browser used for the sign-in window, if one is installed.
pub fn browser_path() -> Option<PathBuf> {
    browser_candidates().into_iter().find(|path| path.is_file())
}

/// Kills the sign-in browser (and its child processes) however the sign-in ends, including cancellation.
struct Browser(Child);

impl Browser {
    fn exited(&mut self) -> bool {
        !matches!(self.0.try_wait(), Ok(None))
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let _ = Command::new("taskkill")
                .args(["/PID", &self.0.id().to_string(), "/T", "/F"])
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Opens the sign-in window and waits until the user has signed in, closes the window, or 10 minutes pass.
/// Dropping the future closes the browser. `profile_dir` is the browser's private profile folder
/// (`state_dir/youtube-signin-profile`). The returned account has no name/email/avatar yet: the InnerTube
/// client fills those in from `account/account_menu`.
pub async fn sign_in(profile_dir: &Path) -> Result<YouTubeAccount, YouTubeAuthError> {
    let browser = browser_path().ok_or(YouTubeAuthError::NoBrowser)?;
    // the listener is dropped right away: the browser binds the port itself
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_err(|_| YouTubeAuthError::Launch)?
        .port();
    std::fs::create_dir_all(profile_dir).map_err(|_| YouTubeAuthError::Launch)?;
    let child = Command::new(browser)
        .arg(format!("--remote-debugging-port={port}"))
        .arg(format!("--remote-allow-origins=http://127.0.0.1:{port}"))
        .arg(format!("--user-data-dir={}", profile_dir.display()))
        .args(["--no-first-run", "--no-default-browser-check", "--window-size=520,760"])
        .arg(format!("--app={START_URL}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| YouTubeAuthError::Launch)?;
    let mut browser = Browser(child);
    // loopback only: never through the configured proxy
    let http = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(2))
        .timeout(CALL_TIMEOUT)
        .build()
        .map_err(|_| YouTubeAuthError::Launch)?;
    tokio::time::timeout(SIGN_IN_TIMEOUT, wait_for_session(&http, port, &mut browser))
        .await
        .map_err(|_| YouTubeAuthError::TimedOut)?
}

async fn wait_for_session(
    http: &reqwest::Client,
    port: u16,
    browser: &mut Browser,
) -> Result<YouTubeAccount, YouTubeAuthError> {
    let mut seen_browser = false;
    loop {
        tokio::time::sleep(POLL_INTERVAL).await;
        let Some(targets) = targets(http, port).await else {
            if seen_browser || browser.exited() {
                return Err(YouTubeAuthError::Closed);
            }
            continue;
        };
        seen_browser = true;
        let Some(socket) = music_page_socket(&targets) else { continue };
        // fails until the page has a signed-in session; keep polling
        if let Some(account) = read_session(&socket).await {
            return Ok(account);
        }
    }
}

async fn targets(http: &reqwest::Client, port: u16) -> Option<Vec<Value>> {
    http.get(format!("http://127.0.0.1:{port}/json")).send().await.ok()?.json().await.ok()
}

/// DevTools socket of the first tab that has reached music.youtube.com.
fn music_page_socket(targets: &[Value]) -> Option<String> {
    targets
        .iter()
        .find(|target| {
            target["type"].as_str() == Some("page")
                && target["url"].as_str().is_some_and(|url| url.starts_with("https://music.youtube.com"))
        })
        .and_then(|target| target["webSocketDebuggerUrl"].as_str())
        .map(str::to_owned)
}

/// Name/value pairs from a `Network.getCookies` result, first occurrence of each name.
fn cookie_pairs(result: &Value) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    for cookie in result["cookies"].as_array().into_iter().flatten() {
        let Some(name) = cookie["name"].as_str().filter(|name| !name.is_empty()) else { continue };
        if pairs.iter().any(|(seen, _)| seen == name) {
            continue;
        }
        pairs.push((name.to_owned(), cookie["value"].as_str().unwrap_or_default().to_owned()));
    }
    pairs
}

struct Cdp {
    socket: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    next_id: u64,
}

impl Cdp {
    async fn call(&mut self, method: &str, params: Value) -> Option<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let request = json!({ "id": id, "method": method, "params": params }).to_string();
        tokio::time::timeout(CALL_TIMEOUT, async {
            self.socket.send(Message::text(request)).await.ok()?;
            // events and replies to other calls are skipped
            while let Some(message) = self.socket.next().await {
                let Message::Text(text) = message.ok()? else { continue };
                let Ok(reply) = serde_json::from_str::<Value>(text.as_str()) else { continue };
                if reply["id"].as_u64() != Some(id) {
                    continue;
                }
                return reply["error"].is_null().then(|| reply["result"].clone());
            }
            None
        })
        .await
        .ok()?
    }

    async fn evaluate(&mut self, expression: &str) -> Option<String> {
        let result = self.call("Runtime.evaluate", json!({ "expression": expression, "returnByValue": true })).await?;
        result["result"]["value"].as_str().map(str::to_owned)
    }
}

/// `None` until the page has a signed-in session.
async fn read_session(socket_url: &str) -> Option<YouTubeAccount> {
    let (socket, _) =
        tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(socket_url)).await.ok()?.ok()?;
    let mut cdp = Cdp { socket, next_id: 0 };
    let session = async {
        let cookies = cdp
            .call("Network.getCookies", json!({ "urls": ["https://music.youtube.com", "https://www.youtube.com"] }))
            .await?;
        let pairs = cookie_pairs(&cookies);
        if !pairs.iter().any(|(name, value)| SAPISID_COOKIES.contains(&name.as_str()) && !value.is_empty()) {
            return None;
        }
        // an empty DATASYNC_ID means the page hasn't finished loading its config yet
        let data_sync_id = normalize_data_sync_id(
            &cdp.evaluate("(window.yt&&yt.config_&&yt.config_.DATASYNC_ID)||''").await?,
        )?;
        let visitor_data = cdp
            .evaluate("(window.yt&&yt.config_&&yt.config_.VISITOR_DATA)||''")
            .await
            .filter(|value| !value.trim().is_empty());
        Some(YouTubeAccount::from_pairs(pairs, Some(data_sync_id), visitor_data))
    }
    .await;
    let _ = cdp.socket.close(None).await;
    session
}

/// Sign-out cleanup: forget the browser profile so the next sign-in starts from a clean window.
/// The stored session is removed separately (`Store::revoke(Slot::YouTube)` + `Lease::delete`).
pub fn remove_profile(profile_dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(profile_dir) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_are_parsed_once_per_name() {
        let pairs = parse_cookies(" SID=one; SAPISID=secret/value=; ; broken; SID=two;HSID = three ");
        assert_eq!(
            pairs,
            [("SID", "one"), ("SAPISID", "secret/value="), ("HSID", "three")].map(|(n, v)| (n.to_owned(), v.to_owned()))
        );
    }

    #[test]
    fn sapisid_falls_back_to_the_secure_cookies() {
        let account = |cookie: &str| YouTubeAccount::from_pairs(parse_cookies(cookie), None, None);
        assert_eq!(account("__Secure-1PAPISID=c; __Secure-3PAPISID=b; SAPISID=a").sapisid().as_deref(), Some("a"));
        assert_eq!(account("__Secure-1PAPISID=c; __Secure-3PAPISID=b").sapisid().as_deref(), Some("b"));
        assert_eq!(account("__Secure-1PAPISID=c; SAPISID=").sapisid().as_deref(), Some("c"));
        assert_eq!(account("SID=x").sapisid(), None);
    }

    #[test]
    fn the_authorization_header_matches_the_reference_hash() {
        // printf '%s' '1700000000 dummy-sapisid https://music.youtube.com' | sha1sum
        let hash = "0192dfec5a9771fd5b0a832c5bc05a3174a8438a";
        assert_eq!(compute_sapisid_hash(1_700_000_000, "dummy-sapisid", ORIGIN), hash);
        assert_eq!(
            authorization_header(1_700_000_000, "dummy-sapisid", ORIGIN),
            format!("SAPISIDHASH 1700000000_{hash} SAPISID1PHASH 1700000000_{hash} SAPISID3PHASH 1700000000_{hash}")
        );
        let account = YouTubeAccount::from_cookie_string("SAPISID=dummy-sapisid").unwrap();
        assert!(account.authorization(ORIGIN).unwrap().starts_with("SAPISIDHASH "));
    }

    #[test]
    fn a_pasted_cookie_header_becomes_an_account() {
        let account = YouTubeAccount::from_cookie_string("Cookie: SID=dummy-sid;\r\n SAPISID=dummy-sapisid; SID=again\n")
            .unwrap();
        assert_eq!(account.cookie, "SID=dummy-sid; SAPISID=dummy-sapisid");
        assert_eq!(account.data_sync_id, None);
        assert!(YouTubeAccount::from_cookie_string("cookie: SID=dummy-sid") == Err(YouTubeAuthError::NotSignedIn));
        assert!(YouTubeAccount::from_cookie_string("") == Err(YouTubeAuthError::NotSignedIn));
        assert!(!format!("{account:?}").contains("dummy"));
    }

    #[test]
    fn data_sync_ids_keep_the_part_before_the_separator() {
        assert_eq!(normalize_data_sync_id("123||").as_deref(), Some("123"));
        assert_eq!(normalize_data_sync_id(" 123||456 ").as_deref(), Some("123"));
        assert_eq!(normalize_data_sync_id("123").as_deref(), Some("123"));
        for empty in ["", "  ", "||456", "null"] {
            assert_eq!(normalize_data_sync_id(empty), None);
        }
    }

    #[test]
    fn the_account_round_trips_with_camel_case_fields() {
        let mut account = YouTubeAccount::from_cookie_string("SAPISID=dummy-sapisid").unwrap();
        account.data_sync_id = Some("123".into());
        let text = serde_json::to_string(&account).unwrap();
        assert!(text.contains("\"dataSyncId\":\"123\""));
        assert!(serde_json::from_str::<YouTubeAccount>(&text).unwrap() == account);
        // a record written before the profile was known still loads
        let bare: YouTubeAccount = serde_json::from_str(r#"{"cookie":"SAPISID=dummy-sapisid"}"#).unwrap();
        assert_eq!(bare.name, None);
    }

    #[test]
    fn only_a_music_page_is_picked_from_the_targets() {
        let targets = [
            json!({ "type": "page", "url": "https://accounts.google.com/ServiceLogin", "webSocketDebuggerUrl": "ws://a" }),
            json!({ "type": "service_worker", "url": "https://music.youtube.com/sw.js", "webSocketDebuggerUrl": "ws://b" }),
            json!({ "type": "page", "url": "https://music.youtube.com/", "webSocketDebuggerUrl": "ws://c" }),
        ];
        assert_eq!(music_page_socket(&targets).as_deref(), Some("ws://c"));
        assert_eq!(music_page_socket(&targets[..2]), None);
    }

    /// A one-connection DevTools stand-in; returns its ws:// address.
    async fn fake_devtools(cookies: Value, data_sync_id: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let request: Value = serde_json::from_str(text.as_str()).unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "Network.getCookies" => json!({ "cookies": cookies }),
                    _ if request["params"]["expression"].as_str().unwrap().contains("DATASYNC_ID") => {
                        json!({ "result": { "type": "string", "value": data_sync_id } })
                    }
                    _ => json!({ "result": { "type": "string", "value": "CgtDummyVisitor" } }),
                };
                // an unrelated event first: the client must skip it
                socket.send(Message::text(json!({ "method": "Page.frameNavigated" }).to_string())).await.unwrap();
                socket.send(Message::text(json!({ "id": request["id"], "result": result }).to_string())).await.unwrap();
            }
        });
        format!("ws://{address}/devtools/page/1")
    }

    #[tokio::test]
    async fn a_signed_in_page_yields_the_session() {
        let cookies = json!([
            { "name": "SID", "value": "dummy-sid" },
            { "name": "SAPISID", "value": "dummy-sapisid" },
            { "name": "SID", "value": "dummy-duplicate" },
        ]);
        let account = read_session(&fake_devtools(cookies, "123||").await).await.unwrap();
        assert_eq!(account.cookie, "SID=dummy-sid; SAPISID=dummy-sapisid");
        assert_eq!(account.data_sync_id.as_deref(), Some("123"));
        assert_eq!(account.visitor_data.as_deref(), Some("CgtDummyVisitor"));
    }

    #[tokio::test]
    async fn a_page_that_is_not_ready_yields_nothing() {
        let signed_out = json!([{ "name": "VISITOR_INFO1_LIVE", "value": "dummy" }]);
        assert!(read_session(&fake_devtools(signed_out, "123||").await).await.is_none());
        let signed_in = json!([{ "name": "__Secure-3PAPISID", "value": "dummy-sapisid" }]);
        assert!(read_session(&fake_devtools(signed_in, "").await).await.is_none());
        assert!(read_session("ws://127.0.0.1:1/devtools/page/1").await.is_none());
    }

    #[test]
    fn removing_a_missing_profile_is_fine() {
        let dir = std::env::temp_dir().join(format!("sf-auth-youtube-profile-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Default")).unwrap();
        remove_profile(&dir).unwrap();
        assert!(!dir.exists());
        remove_profile(&dir).unwrap();
    }
}
