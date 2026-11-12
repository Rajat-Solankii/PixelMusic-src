//! A sandboxed JavaScript engine (QuickJS through rquickjs) on its own thread, for the scripts YouTube expects a
//! browser to run: the player's signature/`n` transform (`cipher.rs`) and BotGuard (`potoken.rs`).
//!
//! One thread owns the `Runtime` and its named contexts; jobs are closures sent to it and answered over a oneshot.
//! Every job has a deadline enforced by the interrupt handler, the runtime has a memory limit, and a thread that
//! died (a panic in a job) is replaced on the next call. The scripts get no I/O: no network, files or host calls
//! apart from a clock (`__hostNow`).
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rquickjs::convert::Coerced;
use rquickjs::{Context, Ctx, FromJs, Runtime};

/// The browser-like environment the scripts run in (`js/env.js`) and the DOM surface it builds its interfaces from.
pub(crate) const ENV_JS: &str = include_str!("js/env.js");
pub(crate) const DOM_SURFACE: &str = include_str!("js/dom_surface.json");

const STACK_BYTES: usize = 16 << 20;
const JS_STACK_BYTES: usize = 8 << 20;
const MEMORY_LIMIT: usize = 384 << 20;

type Job = Box<dyn FnOnce(&mut Engine) + Send>;

/// The JS thread's state, lent to each job.
pub(crate) struct Engine {
    runtime: Runtime,
    /// the named contexts, each with the extra contexts whose globals it uses as iframe realms
    contexts: HashMap<String, (Context, Vec<Context>)>,
    deadline: Arc<Mutex<Option<Instant>>>,
}

impl Engine {
    fn new() -> rquickjs::Result<Self> {
        let runtime = Runtime::new()?;
        runtime.set_memory_limit(MEMORY_LIMIT);
        runtime.set_max_stack_size(JS_STACK_BYTES);
        let deadline: Arc<Mutex<Option<Instant>>> = Arc::default();
        let watched = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            watched.lock().map(|until| until.is_some_and(|until| Instant::now() > until)).unwrap_or(false)
        })));
        Ok(Self { runtime, contexts: HashMap::new(), deadline })
    }

    /// A new context under `name` (replacing one of that name) with `realms` other contexts' globals in `__realms`
    /// and the host clock in `__hostNow`.
    pub(crate) fn fresh_context(&mut self, name: &str, realms: usize) -> Result<(), String> {
        self.contexts.remove(name);
        let context = Context::full(&self.runtime).map_err(|error| error.to_string())?;
        let extra: Vec<Context> =
            (0..realms).map(|_| Context::full(&self.runtime)).collect::<rquickjs::Result<_>>().map_err(|error| error.to_string())?;
        let globals: Vec<rquickjs::Persistent<rquickjs::Object<'static>>> =
            extra.iter().map(|realm| realm.with(|ctx| rquickjs::Persistent::save(&ctx, ctx.globals()))).collect();
        let origin = Instant::now();
        context
            .with(|ctx| -> rquickjs::Result<()> {
                let list = rquickjs::Array::new(ctx.clone())?;
                for (index, global) in globals.into_iter().enumerate() {
                    list.set(index, global.restore(&ctx)?)?;
                }
                ctx.globals().set("__realms", list)?;
                let now = rquickjs::Function::new(ctx.clone(), move || origin.elapsed().as_secs_f64() * 1000.0)?;
                ctx.globals().set("__hostNow", now)
            })
            .map_err(|error| error.to_string())?;
        self.contexts.insert(name.to_owned(), (context, extra));
        Ok(())
    }

    pub(crate) fn has_context(&self, name: &str) -> bool {
        self.contexts.contains_key(name)
    }

    pub(crate) fn drop_context(&mut self, name: &str) {
        self.contexts.remove(name);
        self.runtime.run_gc();
    }

    fn context(&self, name: &str) -> Result<&Context, String> {
        self.contexts.get(name).map(|(context, _)| context).ok_or_else(|| format!("no JS context {name}"))
    }

    /// Evaluate `source` in the context; the result as a string (`""` for undefined/null), or the exception message.
    pub(crate) fn eval(&self, name: &str, source: &str) -> Result<String, String> {
        self.context(name)?.with(|ctx| {
            let value: rquickjs::Value = ctx.eval(source).map_err(|error| describe(&ctx, error))?;
            if value.is_undefined() || value.is_null() {
                return Ok(String::new());
            }
            if let Some(text) = value.as_string() {
                return text.to_string().map_err(|error| describe(&ctx, error));
            }
            Coerced::<String>::from_js(&ctx, value).map(|text| text.0).map_err(|error| describe(&ctx, error))
        })
    }

    /// Run promise jobs and the timers of `env.js` (`__tick()`) until `check` evaluates to a non-empty string.
    pub(crate) fn pump_until(&self, name: &str, check: &str, timeout: Duration) -> Result<String, String> {
        let start = Instant::now();
        loop {
            while self.runtime.is_job_pending() {
                if let Err(error) = self.runtime.execute_pending_job() {
                    log::debug!("js: a promise job failed: {error}");
                }
            }
            let state = self.eval(name, check)?;
            if !state.is_empty() {
                return Ok(state);
            }
            if start.elapsed() > timeout {
                return Err(format!("timed out waiting for {check}"));
            }
            self.eval(name, "__tick()")?;
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// The message of a JS exception (with the first stack line), else the error itself.
fn describe(ctx: &Ctx<'_>, error: rquickjs::Error) -> String {
    if !matches!(error, rquickjs::Error::Exception) {
        return error.to_string();
    }
    let caught = ctx.catch();
    if let Some(exception) = caught.as_exception() {
        let message = exception.message().unwrap_or_default();
        let at = exception.stack().unwrap_or_default();
        let at = at.lines().next().unwrap_or_default().trim();
        return if at.is_empty() { message } else { format!("{message} ({at})") };
    }
    if let Some(text) = caught.as_string().and_then(|text| text.to_string().ok()) {
        return text;
    }
    format!("{caught:?}")
}

/// Handle to the JS thread; it starts with the first job.
#[derive(Default)]
pub(crate) struct JsWorker {
    sender: Mutex<Option<mpsc::Sender<Job>>>,
}

impl JsWorker {
    fn spawn() -> Option<mpsc::Sender<Job>> {
        let (sender, receiver) = mpsc::channel::<Job>();
        let started = std::thread::Builder::new().name("youtube-js".into()).stack_size(STACK_BYTES).spawn(move || {
            let mut engine = match Engine::new() {
                Ok(engine) => engine,
                Err(error) => {
                    log::warn!("js: the engine did not start: {error}");
                    return;
                }
            };
            while let Ok(job) = receiver.recv() {
                // a job that panicked may have left the engine half-way: start over with a new one
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&mut engine))).is_err() {
                    drop(engine);
                    engine = match Engine::new() {
                        Ok(engine) => engine,
                        Err(error) => {
                            log::warn!("js: the engine did not restart: {error}");
                            return;
                        }
                    };
                }
            }
        });
        match started {
            Ok(_) => Some(sender),
            Err(error) => {
                log::warn!("js: the thread did not start: {error}");
                None
            }
        }
    }

    /// Run `job` on the JS thread; it is interrupted once `timeout` passes. Errors: the thread is gone.
    pub(crate) async fn run<R: Send + 'static>(
        &self,
        timeout: Duration,
        job: impl FnOnce(&mut Engine) -> R + Send + 'static,
    ) -> Result<R, String> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        let job: Job = Box::new(move |engine: &mut Engine| {
            if let Ok(mut deadline) = engine.deadline.lock() {
                *deadline = Some(Instant::now() + timeout);
            }
            let result = job(engine);
            if let Ok(mut deadline) = engine.deadline.lock() {
                *deadline = None;
            }
            let _ = reply.send(result);
        });
        {
            let mut sender = self.sender.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut job = Some(job);
            // a dead thread drops its receiver: start a new one and try once more
            for _ in 0..2 {
                if sender.is_none() {
                    *sender = Self::spawn();
                }
                let Some(current) = sender.as_ref() else { break };
                match current.send(job.take().expect("job")) {
                    Ok(()) => break,
                    Err(mpsc::SendError(returned)) => {
                        job = Some(returned);
                        *sender = None;
                    }
                }
            }
            if job.is_some() {
                return Err("the JavaScript engine is not available".into());
            }
        }
        answer.await.map_err(|_| "the JavaScript engine stopped".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn jobs_run_in_named_contexts_and_runaway_scripts_are_stopped() {
        let worker = JsWorker::default();
        let value = worker
            .run(Duration::from_secs(5), |engine| {
                engine.fresh_context("a", 2)?;
                engine.eval("a", "var x = 40; __realms.length")?;
                engine.eval("a", "x + 2")
            })
            .await
            .unwrap();
        assert_eq!(value.unwrap(), "42");
        let kept = worker.run(Duration::from_secs(5), |engine| engine.eval("a", "typeof __hostNow() + x")).await.unwrap();
        assert_eq!(kept.unwrap(), "number40");
        let thrown = worker.run(Duration::from_secs(5), |engine| engine.eval("a", "throw new Error('nope')")).await.unwrap();
        assert!(thrown.unwrap_err().contains("nope"));
        let endless = worker.run(Duration::from_millis(200), |engine| engine.eval("a", "for(;;){}")).await.unwrap();
        assert!(endless.is_err());
        let after = worker.run(Duration::from_secs(5), |engine| engine.eval("a", "x")).await.unwrap();
        assert_eq!(after.unwrap(), "40", "the context survives an interrupted job");
        let missing = worker.run(Duration::from_secs(5), |engine| engine.eval("b", "1")).await.unwrap();
        assert!(missing.is_err());
    }

    #[tokio::test]
    async fn a_panicking_job_gets_a_new_engine() {
        let worker = JsWorker::default();
        let failed = worker.run(Duration::from_secs(5), |_| -> () { panic!("job panics") }).await;
        assert!(failed.is_err());
        let value = worker
            .run(Duration::from_secs(5), |engine| {
                engine.fresh_context("a", 0)?;
                engine.eval("a", "'back'")
            })
            .await
            .unwrap();
        assert_eq!(value.unwrap(), "back");
    }

    #[tokio::test]
    async fn timers_and_promises_are_pumped() {
        let worker = JsWorker::default();
        let value = worker
            .run(Duration::from_secs(5), |engine| {
                engine.fresh_context("a", 0)?;
                engine.eval("a", &format!("globalThis.__DOM_SURFACE = {DOM_SURFACE};"))?;
                engine.eval("a", ENV_JS)?;
                engine.eval("a", "var done = ''; setTimeout(function(){ Promise.resolve('ok').then(function(v){ done = v; }); }, 5);")?;
                engine.pump_until("a", "done", Duration::from_secs(2))
            })
            .await
            .unwrap();
        assert_eq!(value.unwrap(), "ok");
    }
}
