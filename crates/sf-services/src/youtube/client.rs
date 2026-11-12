//! InnerTube HTTP client (port of soundwave1 `InnerTube.kt`): client presets, request context, SAPISIDHASH
//! signing, retries and the anonymous visitor id.
use std::sync::RwLock;
use std::time::Duration;

use serde_json::{Value, json};
use sf_auth::YouTubeAccount;
use sf_auth::youtube::ORIGIN;

use super::error::{Result, YouTubeError};

pub(crate) const USER_AGENT_WEB: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
const REFERER: &str = "https://music.youtube.com/";
/// BotGuard's challenge service (`potoken.rs`).
const BOTGUARD_BASE: &str = "https://www.youtube.com/api/jnn/v1/";
/// The public key the web client sends to BotGuard's service.
const BOTGUARD_API_KEY: &str = "REDACTED_API_KEY";
const MAX_ATTEMPTS: u32 = 4;
const RETRY_DELAY: Duration = Duration::from_millis(800);

/// One InnerTube client identity (`YouTubeClient.kt`).
#[derive(Debug)]
pub struct ClientPreset {
    pub name: &'static str,
    pub version: &'static str,
    /// X-YouTube-Client-Name
    pub id: &'static str,
    pub user_agent: &'static str,
    /// false: never send cookies or `onBehalfOfUser`
    pub login_supported: bool,
    pub os_name: Option<&'static str>,
    pub os_version: Option<&'static str>,
    pub device_make: Option<&'static str>,
    pub device_model: Option<&'static str>,
    pub android_sdk_version: Option<&'static str>,
}

/// Browse, library, search, mutations and `next`.
pub const WEB_REMIX: ClientPreset = ClientPreset {
    name: "WEB_REMIX",
    version: "1.20260114.01.00",
    id: "67",
    user_agent: USER_AGENT_WEB,
    login_supported: true,
    os_name: None,
    os_version: None,
    device_make: None,
    device_model: None,
    android_sdk_version: None,
};

/// Unauthenticated stream resolution (no PoToken needed).
pub const ANDROID_VR: ClientPreset = ClientPreset {
    name: "ANDROID_VR",
    version: "1.61.48",
    id: "28",
    user_agent: "com.google.android.apps.youtube.vr.oculus/1.61.48 (Linux; U; Android 12; en_US; Quest 3; Build/SQ3A.220605.009.A1; Cronet/132.0.6808.3)",
    login_supported: false,
    os_name: Some("Android"),
    os_version: Some("12"),
    device_make: Some("Oculus"),
    device_model: Some("Quest 3"),
    android_sdk_version: Some("32"),
};

/// Transcripts.
pub const WEB: ClientPreset = ClientPreset {
    name: "WEB",
    version: "2.20260114.00.00",
    id: "1",
    user_agent: USER_AGENT_WEB,
    login_supported: false,
    os_name: None,
    os_version: None,
    device_make: None,
    device_model: None,
    android_sdk_version: None,
};

/// Whether a request is sent as the signed-in user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Login {
    /// always anonymous
    Never,
    /// as the user when signed in, anonymous otherwise (browse, search)
    IfSignedIn,
    /// fails with `NotSignedIn` without an account (library, mutations)
    Required,
}

struct Session {
    account: Option<YouTubeAccount>,
    /// anonymous visitor id from sw.js_data
    visitor_data: Option<String>,
    gl: String,
    hl: String,
}

pub struct InnerTube {
    http: sf_api::Http,
    api_base: String,
    site_base: String,
    botguard_base: String,
    retry_delay: Duration,
    session: RwLock<Session>,
    /// held while the visitor id is fetched; true once tried
    visitor_tried: tokio::sync::Mutex<bool>,
}

impl InnerTube {
    pub fn new(http: sf_api::Http) -> Self {
        Self {
            http,
            api_base: "https://music.youtube.com/youtubei/v1/".to_owned(),
            site_base: REFERER.to_owned(),
            botguard_base: BOTGUARD_BASE.to_owned(),
            retry_delay: RETRY_DELAY,
            session: RwLock::new(Session { account: None, visitor_data: None, gl: "US".to_owned(), hl: "en".to_owned() }),
            visitor_tried: tokio::sync::Mutex::new(false),
        }
    }

    /// Point every request at another host (tests): `base` stands for `https://music.youtube.com/` and BotGuard's
    /// service is at `{base}/api/jnn/v1/`.
    pub fn with_base_url(mut self, base: &str) -> Self {
        let base = format!("{}/", base.trim_end_matches('/'));
        self.api_base = format!("{base}youtubei/v1/");
        self.botguard_base = format!("{base}api/jnn/v1/");
        self.site_base = base;
        self
    }

    /// First retry delay (doubles on every attempt).
    pub fn with_retry_delay(mut self, delay: Duration) -> Self {
        self.retry_delay = delay;
        self
    }

    fn session(&self) -> std::sync::RwLockReadGuard<'_, Session> {
        self.session.read().unwrap_or_else(|lock| lock.into_inner())
    }

    fn session_mut(&self) -> std::sync::RwLockWriteGuard<'_, Session> {
        self.session.write().unwrap_or_else(|lock| lock.into_inner())
    }

    /// Sign in (`Some`) or out (`None`).
    pub fn set_account(&self, account: Option<YouTubeAccount>) {
        self.session_mut().account = account;
    }

    pub fn account(&self) -> Option<YouTubeAccount> {
        self.session().account.clone()
    }

    pub fn signed_in(&self) -> bool {
        self.session().account.is_some()
    }

    /// Country (`gl`) and interface language (`hl`) of the request context.
    pub fn set_locale(&self, gl: &str, hl: &str) {
        let mut session = self.session_mut();
        session.gl = gl.to_owned();
        session.hl = hl.to_owned();
    }

    pub(crate) fn http_client(&self) -> Result<reqwest::Client> {
        self.http.client().map_err(YouTubeError::Unavailable)
    }

    /// The visitor id sent with requests: the account's own, else the anonymous one (fetched once).
    async fn visitor_data(&self) -> Option<String> {
        {
            let session = self.session();
            let known = session.account.as_ref().and_then(|account| account.visitor_data.clone()).or(session.visitor_data.clone());
            if known.is_some() {
                return known;
            }
        }
        let mut tried = self.visitor_tried.lock().await;
        if let Some(known) = self.session().visitor_data.clone() {
            return Some(known);
        }
        if *tried {
            return None;
        }
        *tried = true;
        let fetched = self.fetch_visitor_data().await;
        if fetched.is_none() {
            tracing::debug!("YouTube Music visitor id is unavailable; continuing without it");
        }
        self.session_mut().visitor_data = fetched.clone();
        fetched
    }

    async fn fetch_visitor_data(&self) -> Option<String> {
        let response = self
            .http_client()
            .ok()?
            .get(format!("{}sw.js_data", self.site_base))
            .header(reqwest::header::USER_AGENT, USER_AGENT_WEB)
            .send()
            .await
            .ok()?;
        parse_visitor_data(&response.text().await.ok()?)
    }

    /// `GET` a page or script of the site (`path` relative to music.youtube.com), as the web browser.
    pub(crate) async fn get_text(&self, path: &str) -> Result<String> {
        let url = format!("{}{}", self.site_base, path.trim_start_matches('/'));
        let response = self
            .http_client()?
            .get(url)
            .header(reqwest::header::USER_AGENT, USER_AGENT_WEB)
            // past the cookie consent page
            .header(reqwest::header::COOKIE, "SOCS=CAI")
            .send()
            .await
            .map_err(|error| YouTubeError::Network(error.without_url().to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(YouTubeError::Http(status.as_u16()));
        }
        response.text().await.map_err(|error| YouTubeError::Network(error.without_url().to_string()))
    }

    /// `POST` to BotGuard's service (`Create`, `GenerateIT`), JSON+protobuf in and out.
    pub(crate) async fn botguard(&self, method: &str, body: Value) -> Result<Value> {
        let response = self
            .http_client()?
            .post(format!("{}{method}", self.botguard_base))
            .header(reqwest::header::CONTENT_TYPE, "application/json+protobuf")
            .header("x-goog-api-key", BOTGUARD_API_KEY)
            .header("x-user-agent", "grpc-web-javascript/0.1")
            .header(reqwest::header::USER_AGENT, USER_AGENT_WEB)
            .body(body.to_string())
            .send()
            .await
            .map_err(|error| YouTubeError::Network(error.without_url().to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(YouTubeError::Http(status.as_u16()));
        }
        let text = response.text().await.map_err(|error| YouTubeError::Network(error.without_url().to_string()))?;
        serde_json::from_str(&text).map_err(|_| YouTubeError::Unexpected("response is not JSON"))
    }

    fn context(&self, preset: &ClientPreset, visitor_data: Option<&str>, on_behalf_of: Option<&str>) -> Value {
        let session = self.session();
        let mut client = json!({
            "clientName": preset.name,
            "clientVersion": preset.version,
            "gl": session.gl,
            "hl": session.hl,
        });
        let optional = [
            ("osName", preset.os_name),
            ("osVersion", preset.os_version),
            ("deviceMake", preset.device_make),
            ("deviceModel", preset.device_model),
            ("androidSdkVersion", preset.android_sdk_version),
            ("visitorData", visitor_data),
        ];
        for (key, value) in optional {
            if let Some(value) = value {
                client[key] = json!(value);
            }
        }
        let mut user = json!({ "lockedSafetyMode": false });
        if let Some(id) = on_behalf_of {
            user["onBehalfOfUser"] = json!(id);
        }
        json!({
            "client": client,
            "request": { "internalExperimentFlags": [], "useSsl": true },
            "user": user,
        })
    }

    /// `POST <endpoint>` with the request context added to `body`.
    pub async fn post(&self, preset: &ClientPreset, endpoint: &str, body: Value, login: Login) -> Result<Value> {
        self.post_query(preset, endpoint, body, login, &[]).await
    }

    /// As `post`, with extra query parameters (search continuations, the transcript key).
    pub async fn post_query(
        &self,
        preset: &ClientPreset,
        endpoint: &str,
        mut body: Value,
        login: Login,
        query: &[(&str, &str)],
    ) -> Result<Value> {
        let account = match login {
            Login::Never => None,
            Login::IfSignedIn | Login::Required => self.account().filter(|_| preset.login_supported),
        };
        if login == Login::Required && account.is_none() {
            return Err(YouTubeError::NotSignedIn);
        }
        let visitor_data = self.visitor_data().await;
        let on_behalf_of = account.as_ref().and_then(|account| account.data_sync_id.as_deref());
        body["context"] = self.context(preset, visitor_data.as_deref(), on_behalf_of);

        let url = format!("{}{endpoint}", self.api_base);
        let mut delay = self.retry_delay;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut request = self
                .http_client()?
                .post(&url)
                .query(&[("prettyPrint", "false")])
                .query(query)
                .header("X-Goog-Api-Format-Version", "1")
                .header("X-YouTube-Client-Name", preset.id)
                .header("X-YouTube-Client-Version", preset.version)
                .header("X-Origin", ORIGIN)
                .header(reqwest::header::ORIGIN, ORIGIN)
                .header(reqwest::header::REFERER, REFERER)
                .header(reqwest::header::USER_AGENT, preset.user_agent)
                .json(&body);
            if let Some(visitor) = &visitor_data {
                request = request.header("X-Goog-Visitor-Id", visitor);
            }
            if let Some(account) = &account {
                request = request.header(reqwest::header::COOKIE, &account.cookie);
                // signed for now on every attempt: the hash carries the timestamp
                if let Some(authorization) = account.authorization(ORIGIN) {
                    request = request.header(reqwest::header::AUTHORIZATION, authorization);
                }
            }
            let last = attempt >= MAX_ATTEMPTS;
            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return response.json().await.map_err(|_| YouTubeError::Unexpected("response is not JSON"));
                    }
                    let retryable = status.as_u16() == 429 || status.is_server_error();
                    if !retryable || last {
                        return Err(match status.as_u16() {
                            401 => YouTubeError::SessionExpired,
                            429 => YouTubeError::RateLimited,
                            code => YouTubeError::Http(code),
                        });
                    }
                }
                Err(error) if error.is_builder() || last => {
                    return Err(YouTubeError::Network(error.without_url().to_string()));
                }
                Err(_) => {}
            }
            tokio::time::sleep(delay).await;
            delay *= 2;
        }
    }
}

/// The visitor id inside `sw.js_data`: `)]}'` + JSON, first string of `[0][2]` that starts with `Cgt`/`Cgs`.
pub fn parse_visitor_data(text: &str) -> Option<String> {
    let json: Value = serde_json::from_str(text.get(5..)?).ok()?;
    json[0][2]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find(|candidate| candidate.starts_with("Cgt") || candidate.starts_with("Cgs"))
        .map(percent_decode)
}

/// `sw.js_data` keeps the id URL-encoded (`...%3D`); requests carry it decoded, as the web player does.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|byte| (*byte as char).to_digit(16));
        match (bytes[index], hex(index + 1), hex(index + 2)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                index += 3;
            }
            (byte, ..) => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_visitor_id_is_found_in_the_service_worker_data() {
        let text = ")]}'\n[[\"x\",1,[\"en\",\"US\",null,\"CgtWMTIzNDU2Nzg5MCiA\",42]]]";
        assert_eq!(parse_visitor_data(text).as_deref(), Some("CgtWMTIzNDU2Nzg5MCiA"));
        assert_eq!(parse_visitor_data(")]}'\n[[\"x\",1,[\"en\"]]]"), None);
        assert_eq!(parse_visitor_data("oops"), None);
        let encoded = ")]}'\n[[\"x\",1,[\"CgtWMTIz%3D%3D\"]]]";
        assert_eq!(parse_visitor_data(encoded).as_deref(), Some("CgtWMTIz=="));
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("a%zz%41"), "a%zzA");
    }

    #[test]
    fn the_context_carries_the_client_and_only_known_fields() {
        let tube = InnerTube::new(sf_api::Http::default());
        let web = tube.context(&WEB_REMIX, Some("CgtVisitor"), Some("123"));
        assert_eq!(web["client"]["clientName"], "WEB_REMIX");
        assert_eq!(web["client"]["gl"], "US");
        assert_eq!(web["client"]["visitorData"], "CgtVisitor");
        assert_eq!(web["user"]["onBehalfOfUser"], "123");
        assert!(web["client"].get("osName").is_none());
        let vr = tube.context(&ANDROID_VR, None, None);
        assert_eq!(vr["client"]["deviceModel"], "Quest 3");
        assert_eq!(vr["client"]["androidSdkVersion"], "32");
        assert!(vr["client"].get("visitorData").is_none());
        assert!(vr["user"].get("onBehalfOfUser").is_none());
    }
}
