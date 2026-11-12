//! GVS PO tokens ("proof of origin"): web-client stream URLs stop after the first megabyte without `&pot=`, a
//! token bound to the video id. The built-in minter runs Google's BotGuard challenge in the sandboxed JS engine
//! (`jsworker.rs`, `js/env.js`, `js/botguard.js`): `Create` gives the challenge, its snapshot goes to `GenerateIT`
//! for an integrity token, and the minter that token unlocks signs video ids until the token expires. Both calls
//! carry the public API key found in the player script (`cipher::botguard_key`; `BOTGUARD_API_KEY` overrides it).
//! Signing in or out drops the tokens and the minter (`reset`).
//!
//! An external bgutil-style provider can be used instead: `POST {server}/get_pot` with `{"content_binding": id}`
//! answers `{"poToken": ...}` (`YouTubeService::set_po_token_server`, or `MATERIALFY_POT_PROVIDER`).
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::client::InnerTube;
use super::jsworker::{DOM_SURFACE, ENV_JS, JsWorker};

const BOTGUARD_JS: &str = include_str!("js/botguard.js");
/// The request key of the web client's BotGuard program.
pub(crate) const REQUEST_KEY: &str = "O43z0dpjhgX20SCx4KAo";
const CONTEXT: &str = "botguard";
/// Iframe realms BotGuard may create.
const REALMS: usize = 4;
/// A minter is replaced this long before its integrity token expires.
const EXPIRY_MARGIN: Duration = Duration::from_secs(600);
/// After BotGuard was refused, the built-in minter waits this long before trying again.
const REFUSAL_REST: Duration = Duration::from_secs(15 * 60);
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(20);
const MINT_TIMEOUT: Duration = Duration::from_secs(10);
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(20);
pub(crate) const PROVIDER_ENV: &str = "MATERIALFY_POT_PROVIDER";
/// Overrides the BotGuard API key read from the player script.
pub(crate) const KEY_ENV: &str = "BOTGUARD_API_KEY";

enum Minter {
    /// the context `CONTEXT` holds a minter, usable until then
    Ready(Instant),
    /// GenerateIT gave no integrity token: no tokens until then
    Refused(Instant),
}

pub(crate) struct PoTokens {
    worker: Arc<JsWorker>,
    server: RwLock<Option<String>>,
    minter: tokio::sync::Mutex<Option<Minter>>,
    /// binding -> (token, valid until)
    cache: Mutex<HashMap<String, (String, Instant)>>,
    /// set by `reset`: the minter belongs to the previous identity
    stale: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl PoTokens {
    pub(crate) fn new(worker: Arc<JsWorker>) -> Self {
        let server = std::env::var(PROVIDER_ENV).ok().map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
        Self { worker, server: RwLock::new(server), minter: tokio::sync::Mutex::new(None), cache: Mutex::default(), stale: AtomicBool::new(false) }
    }

    /// Use an external provider (`Some(base URL)`) or the built-in minter (`None`).
    pub(crate) fn set_server(&self, server: Option<String>) {
        let server = server.map(|value| value.trim().trim_end_matches('/').to_owned()).filter(|value| !value.is_empty());
        *self.server.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = server;
        lock(&self.cache).clear();
    }

    /// Forget the video's token (its stream was refused).
    pub(crate) fn forget(&self, video_id: &str) {
        lock(&self.cache).remove(video_id);
    }

    /// Another identity (signed in or out): drop every token and the minter, so nothing minted for the previous
    /// identity is reused. The next token starts a fresh BotGuard challenge.
    pub(crate) fn reset(&self) {
        self.stale.store(true, Ordering::SeqCst);
        lock(&self.cache).clear();
    }

    /// Gets the built-in minter ready ahead of the first token (the BotGuard challenge takes a second or two).
    /// Nothing to do with an external provider or a minter that is still good.
    pub(crate) async fn warm(&self, client: &InnerTube, key: Option<&str>) -> Result<(), String> {
        if self.server.read().unwrap_or_else(|poisoned| poisoned.into_inner()).is_some() {
            return Ok(());
        }
        let key = std::env::var(KEY_ENV).ok().filter(|key| !key.trim().is_empty()).or(key.map(str::to_owned));
        let Some(key) = key else { return Err("no BotGuard API key in the player script".into()) };
        let mut minter = self.minter.lock().await;
        if self.stale.swap(false, Ordering::SeqCst) {
            *minter = None;
        }
        if let Some(Minter::Ready(until) | Minter::Refused(until)) = *minter
            && Instant::now() < until
        {
            return Ok(());
        }
        *minter = None;
        *minter = Some(self.start_minter(client, &key).await?);
        Ok(())
    }

    /// A token bound to `binding` (a video id, or a signed-in user's datasync id). `key` is BotGuard's API key
    /// from the player script (`BOTGUARD_API_KEY` overrides it). `Ok(None)`: BotGuard refused this environment, go
    /// on without one.
    pub(crate) async fn token(&self, client: &InnerTube, key: Option<&str>, binding: &str) -> Result<Option<String>, String> {
        let now = Instant::now();
        {
            let mut cache = lock(&self.cache);
            cache.retain(|_, (_, until)| *until > now);
            if let Some((token, _)) = cache.get(binding) {
                return Ok(Some(token.clone()));
            }
        }
        let server = self.server.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
        if let Some(server) = server {
            match from_provider(client, &server, binding).await {
                Ok(token) => {
                    // the provider keeps its own cache; an hour here spares the round trip
                    lock(&self.cache).insert(binding.to_owned(), (token.clone(), now + Duration::from_secs(3600)));
                    return Ok(Some(token));
                }
                Err(error) => log::info!("youtube po token: the provider failed ({error}); using the built-in minter"),
            }
        }
        let key = std::env::var(KEY_ENV).ok().filter(|key| !key.trim().is_empty()).or(key.map(str::to_owned));
        let Some(key) = key else { return Err("no BotGuard API key in the player script".into()) };
        let mut minter = self.minter.lock().await;
        if self.stale.swap(false, Ordering::SeqCst) {
            *minter = None;
        }
        // one fresh minter if the one we had is gone (expired, or the JS thread was replaced)
        for attempt in 0..2 {
            let until = match *minter {
                Some(Minter::Refused(until)) if Instant::now() < until => return Ok(None),
                Some(Minter::Ready(until)) if Instant::now() < until => until,
                _ => {
                    *minter = None;
                    let ready = self.start_minter(client, &key).await?;
                    *minter = Some(ready);
                    match *minter {
                        Some(Minter::Ready(until)) => until,
                        _ => return Ok(None),
                    }
                }
            };
            match self.mint(binding).await {
                Ok(token) => {
                    // a reset meanwhile: the token is still good for this request, not for the next identity
                    if !self.stale.load(Ordering::SeqCst) {
                        lock(&self.cache).insert(binding.to_owned(), (token.clone(), until));
                    }
                    return Ok(Some(token));
                }
                Err(error) => {
                    log::debug!("youtube po token: minting failed (attempt {}): {error}", attempt + 1);
                    *minter = None;
                    if attempt == 1 {
                        return Err(error);
                    }
                }
            }
        }
        Err("no PO token".into())
    }

    /// Run the BotGuard challenge and get its minter ready in the JS context.
    async fn start_minter(&self, client: &InnerTube, key: &str) -> Result<Minter, String> {
        let started = Instant::now();
        let create = client.botguard("Create", json!([REQUEST_KEY]), key).await.map_err(|error| format!("Create: {error}"))?;
        let create = create.to_string();
        let snapshot = self
            .worker
            .run(SNAPSHOT_TIMEOUT + Duration::from_secs(10), move |engine| {
                engine.fresh_context(CONTEXT, REALMS)?;
                engine.eval(CONTEXT, &format!("globalThis.__DOM_SURFACE = {DOM_SURFACE};"))?;
                engine.eval(CONTEXT, ENV_JS)?;
                engine.eval(CONTEXT, BOTGUARD_JS)?;
                engine.eval(CONTEXT, &format!("__start({create});"))?;
                let state = engine.pump_until(CONTEXT, "__error || (__bgResponse ? 'ok' : '')", SNAPSHOT_TIMEOUT)?;
                if state != "ok" {
                    return Err(format!("BotGuard: {state}"));
                }
                engine.eval(CONTEXT, "__bgResponse")
            })
            .await??;
        let integrity = client
            .botguard("GenerateIT", json!([REQUEST_KEY, snapshot]), key)
            .await
            .map_err(|error| format!("GenerateIT: {error}"))?;
        if !integrity[0].is_string() {
            log::info!("youtube po token: BotGuard gave no integrity token; streams go without one for a while");
            self.worker.run(MINT_TIMEOUT, |engine| engine.drop_context(CONTEXT)).await?;
            return Ok(Minter::Refused(Instant::now() + REFUSAL_REST));
        }
        let ttl = Duration::from_secs(integrity[1].as_u64().unwrap_or(3600));
        let integrity = integrity.to_string();
        self.worker
            .run(MINT_TIMEOUT + Duration::from_secs(5), move |engine| {
                engine.eval(CONTEXT, &format!("__integrity({integrity});"))?;
                let state = engine.pump_until(CONTEXT, "__error || (__minter ? 'ok' : '')", MINT_TIMEOUT)?;
                if state == "ok" { Ok(()) } else { Err(format!("BotGuard minter: {state}")) }
            })
            .await??;
        log::debug!("youtube po token: minter ready in {:?}, valid for {ttl:?}", started.elapsed());
        let usable = ttl.saturating_sub(EXPIRY_MARGIN).max(Duration::from_secs(60));
        Ok(Minter::Ready(Instant::now() + usable))
    }

    async fn mint(&self, video_id: &str) -> Result<String, String> {
        let call = format!("__token = ''; __error = ''; __mint({});", Value::from(video_id));
        self.worker
            .run(MINT_TIMEOUT + Duration::from_secs(5), move |engine| {
                if !engine.has_context(CONTEXT) {
                    return Err("the minter is gone".to_owned());
                }
                engine.eval(CONTEXT, &call)?;
                let token = engine.pump_until(CONTEXT, "__error ? 'error: ' + __error : __token", MINT_TIMEOUT)?;
                if token.starts_with("error: ") { Err(token) } else { Ok(token) }
            })
            .await?
    }
}

async fn from_provider(client: &InnerTube, server: &str, video_id: &str) -> Result<String, String> {
    let response = client
        .http_client()
        .map_err(|error| error.to_string())?
        .post(format!("{}/get_pot", server.trim_end_matches('/')))
        .json(&json!({ "content_binding": video_id }))
        .timeout(PROVIDER_TIMEOUT)
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let body: Value = response.json().await.map_err(|_| "the answer is not JSON".to_owned())?;
    body["poToken"].as_str().filter(|token| !token.is_empty()).map(str::to_owned).ok_or_else(|| "no poToken".to_owned())
}
