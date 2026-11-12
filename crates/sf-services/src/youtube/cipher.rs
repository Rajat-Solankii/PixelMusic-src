//! The web player's stream URL transform: formats of the web clients come with a ciphered signature (`s`, `sp`)
//! and an `n` parameter that the player script (`base.js`) rewrites. Instead of re-implementing those functions,
//! the player's own URL builder is called inside the sandboxed JS engine (`jsworker.rs`).
//!
//! The script is found on the music.youtube.com page (`jsUrl`), patched so its decipher function is reachable
//! (`prepare`), evaluated once with `env.js`, and kept for a few hours. Its `signatureTimestamp` goes into player
//! requests so the formats it is asked to decipher match it. The script also carries the public API key of
//! BotGuard's service (`X-Goog-Api-Key`), which the PO token minter needs (`potoken.rs`).
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use regex::Regex;

use super::client::InnerTube;
use super::jsworker::{DOM_SURFACE, ENV_JS, JsWorker};

/// How long a loaded player script is used.
const SCRIPT_TTL: Duration = Duration::from_secs(6 * 3600);
const CONTEXT: &str = "player";
const LOAD_TIMEOUT: Duration = Duration::from_secs(60);
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Calls the decipher function and turns the URL object it builds into a string: the zero-argument method on its
/// prototype chain that returns an `http...` string.
const HELPERS: &str = r#"
function __mfStr(r) {
  if (typeof r === "string") return r;
  for (var p = Object.getPrototypeOf(r); p && p !== Object.prototype; p = Object.getPrototypeOf(p)) {
    var names = Object.getOwnPropertyNames(p);
    for (var i = 0; i < names.length; i++) {
      var k = names[i];
      if (k === "constructor" || k === "clone") continue;
      var d = Object.getOwnPropertyDescriptor(p, k);
      if (!d || typeof d.value !== "function" || d.value.length !== 0) continue;
      try { var s = d.value.call(r); if (typeof s === "string" && s.indexOf("http") === 0) return s; } catch (e) {}
    }
  }
  throw new Error("the URL object has no string form");
}
function __mfDecipher(url, sp, s) { return __mfStr(_yt_player.__mf_bs(url, sp, s)); }
"#;

/// The player script's path on the music.youtube.com page (`/s/player/<id>/.../base.js`).
pub fn player_path(page: &str) -> Option<String> {
    static JS_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""jsUrl"\s*:\s*"([^"]+)""#).expect("regex"));
    Some(JS_URL.captures(page)?[1].replace("\\/", "/"))
}

/// The `signatureTimestamp` the script expects in player requests.
pub fn signature_timestamp(js: &str) -> Option<u64> {
    static STS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"signatureTimestamp:(\d+)").expect("regex"));
    STS.captures(js)?[1].parse().ok()
}

/// The API key the script sends to BotGuard's service (`jnn-pa.googleapis.com`): `{["X-Goog-Api-Key"]:"AIza..."}`.
pub fn botguard_key(js: &str) -> Option<String> {
    static KEY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#""X-Goog-Api-Key"\]?\s*:\s*"(AIza[0-9A-Za-z_-]{20,})""#).expect("regex")
    });
    Some(KEY.captures(js)?[1].to_owned())
}

/// The script made usable outside a page: `window` is the global object, and the decipher function (`(url, sp="",
/// s="")`, which builds the URL object and sets `alr=yes`) is exported as `_yt_player.__mf_bs`.
pub fn prepare(js: &str) -> Result<String, &'static str> {
    static DECIPHER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"[;,\n]([A-Za-z0-9_$]+)=function\([\w$]+,[\w$]+="",[\w$]+=""\)\{[\w$]+=new g\.[\w$]+\([\w$]+,!0\);[\w$]+\.set\("alr","yes"\)"#,
        )
        .expect("regex")
    });
    const TAIL: &str = "})(_yt_player);";
    let name = DECIPHER.captures(js).ok_or("the decipher function was not found")?[1].to_owned();
    let tail = js.rfind(TAIL).ok_or("the script has an unexpected shape")?;
    let mut out = String::with_capacity(js.len() + 64);
    out.push_str(&js[..tail]);
    out.push_str(&format!(";g.__mf_bs={name};"));
    out.push_str(&js[tail..]);
    Ok(out.replacen("var window=this;", "var window=globalThis;", 1))
}

/// What a loaded player script gives stream resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Script {
    pub sts: u64,
    /// BotGuard's API key, when the script has it
    pub botguard_key: Option<String>,
}

struct Loaded {
    player: String,
    script: Script,
    at: Instant,
}

/// The loaded player script, shared by every stream resolution.
pub(crate) struct Cipher {
    worker: Arc<JsWorker>,
    state: tokio::sync::Mutex<Option<Loaded>>,
}

impl Cipher {
    pub(crate) fn new(worker: Arc<JsWorker>) -> Self {
        Self { worker, state: tokio::sync::Mutex::new(None) }
    }

    /// Load the current player script unless a recent one is loaded; its `signatureTimestamp` and BotGuard key.
    pub(crate) async fn load(&self, client: &InnerTube) -> Result<Script, String> {
        let mut state = self.state.lock().await;
        if let Some(loaded) = state.as_ref().filter(|loaded| loaded.at.elapsed() < SCRIPT_TTL) {
            return Ok(loaded.script.clone());
        }
        *state = None;
        let page = client.get_text("").await.map_err(|error| format!("music page: {error}"))?;
        let path = player_path(&page).ok_or("no player script on the music page")?;
        let js = client.get_text(&path).await.map_err(|error| format!("player script: {error}"))?;
        let sts = signature_timestamp(&js).ok_or("the player script has no signatureTimestamp")?;
        let botguard_key = botguard_key(&js);
        if botguard_key.is_none() {
            log::info!("youtube cipher: player {path} has no BotGuard key");
        }
        let patched = prepare(&js)?;
        let started = Instant::now();
        self.worker
            .run(LOAD_TIMEOUT, move |engine| {
                engine.fresh_context(CONTEXT, 0)?;
                engine.eval(CONTEXT, &format!("globalThis.__DOM_SURFACE = {DOM_SURFACE};"))?;
                engine.eval(CONTEXT, ENV_JS)?;
                engine.eval(CONTEXT, &patched)?;
                engine.eval(CONTEXT, HELPERS)?;
                engine.eval(CONTEXT, "typeof _yt_player.__mf_bs").and_then(|kind| {
                    if kind == "function" { Ok(()) } else { Err("the decipher function was not exported".to_owned()) }
                })
            })
            .await??;
        log::debug!("youtube cipher: player {path} (sts {sts}) loaded in {:?}", started.elapsed());
        let script = Script { sts, botguard_key };
        *state = Some(Loaded { player: path, script: script.clone(), at: Instant::now() });
        Ok(script)
    }

    /// Load the script again before the next stream (a URL it made was refused). Skipped while a load runs.
    pub(crate) fn reset(&self) {
        if let Ok(mut state) = self.state.try_lock()
            && let Some(loaded) = state.take()
        {
            log::info!("youtube cipher: a stream from player {} was refused; reloading it", loaded.player);
        }
    }

    /// The playable URL of a format: `s` deciphered into `sp`, `n` transformed. Formats with a plain URL go with
    /// empty `sp`/`s`. A failure drops the script so the next call loads it again.
    pub(crate) async fn decipher(&self, url: &str, sp: &str, s: &str) -> Result<String, String> {
        let call = format!(
            "__mfDecipher({}, {}, {})",
            serde_json::Value::from(url),
            serde_json::Value::from(sp),
            serde_json::Value::from(s)
        );
        let result = self
            .worker
            .run(CALL_TIMEOUT, move |engine| {
                if !engine.has_context(CONTEXT) {
                    return Err("the player script is not loaded".to_owned());
                }
                engine.eval(CONTEXT, &call)
            })
            .await
            .and_then(|result| result)
            .and_then(|out| if out.starts_with("http") { Ok(out) } else { Err(format!("unexpected decipher result {out:?}")) });
        if let Err(error) = &result {
            let mut state = self.state.lock().await;
            if let Some(loaded) = state.take() {
                log::info!("youtube cipher: player {} failed ({error}); reloading next time", loaded.player);
            }
        }
        result
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A small script shaped like base.js: a URL class, the decipher function (reverses `s` into `sp` and `n`).
    pub(crate) const SYNTHETIC_PLAYER: &str = r#"var _yt_player={};(function(g){var window=this;g.zs=class{constructor(a,b=!1){this.u=a;this.p={}}set(a,b){this.p[a]=b}get(a){return this.p[a]||null}Ue(){var u=this.u.replace(/([?&]n=)([^&]*)/,function(m,a,b){return a+b.split("").reverse().join("")});for(var k in this.p)u+="&"+k+"="+this.p[k];return u}clone(){}};var Bs=0;Bs=function(a,b="",c=""){a=new g.zs(a,!0);a.set("alr","yes");c&&(c=c.split("").reverse().join(""),a.set(b,c));return a};var x={signatureTimestamp:19999};var k=function(){var c={["X-Goog-Api-Key"]:"AIzaTestKeyForTheFakeBotGuard01"};return c};})(_yt_player);"#;

    #[test]
    fn the_script_is_found_and_patched() {
        let page = r#"<script>ytcfg.set({"jsUrl":"\/s\/player\/a5c2aafe\/player_es6.vflset\/en_GB\/base.js","x":1})</script>"#;
        assert_eq!(player_path(page).as_deref(), Some("/s/player/a5c2aafe/player_es6.vflset/en_GB/base.js"));
        assert_eq!(player_path("<html></html>"), None);
        assert_eq!(signature_timestamp(SYNTHETIC_PLAYER), Some(19999));
        assert_eq!(signature_timestamp("var a=1;"), None);
        assert_eq!(botguard_key(SYNTHETIC_PLAYER).as_deref(), Some("AIzaTestKeyForTheFakeBotGuard01"));
        assert_eq!(botguard_key(r#"c={"X-Goog-Api-Key":"AIzaSyAbcdefghijklmnopqrstuv"}"#).as_deref(), Some("AIzaSyAbcdefghijklmnopqrstuv"));
        assert_eq!(botguard_key("var a=1;"), None);
        let patched = prepare(SYNTHETIC_PLAYER).unwrap();
        assert!(patched.contains("var window=globalThis;"));
        assert!(patched.ends_with(";g.__mf_bs=Bs;})(_yt_player);"), "{patched}");
        assert!(prepare("var _yt_player={};(function(g){})(_yt_player);").is_err());
    }

    #[tokio::test]
    async fn the_decipher_function_runs_in_the_engine() {
        let worker = Arc::new(JsWorker::default());
        let patched = prepare(SYNTHETIC_PLAYER).unwrap();
        let out = worker
            .run(Duration::from_secs(30), move |engine| {
                engine.fresh_context(CONTEXT, 0)?;
                engine.eval(CONTEXT, &format!("globalThis.__DOM_SURFACE = {DOM_SURFACE};"))?;
                engine.eval(CONTEXT, ENV_JS)?;
                engine.eval(CONTEXT, &patched)?;
                engine.eval(CONTEXT, HELPERS)?;
                Ok::<_, String>(())
            })
            .await
            .unwrap();
        out.unwrap();
        let cipher = Cipher::new(worker);
        let url = cipher.decipher("https://h/videoplayback?n=abc&expire=1", "sig", "321").await.unwrap();
        assert_eq!(url, "https://h/videoplayback?n=cba&expire=1&alr=yes&sig=123");
        let plain = cipher.decipher("https://h/videoplayback?n=xy", "", "").await.unwrap();
        assert_eq!(plain, "https://h/videoplayback?n=yx&alr=yes");
    }
}
