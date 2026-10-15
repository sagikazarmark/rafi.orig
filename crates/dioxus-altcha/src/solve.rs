//! [`solve`]: Web Workers in the browser, threads with the `native` feature.

use altcha::Solution;

use crate::{RawChallenge, SolveError, SolveOptions};

/// Solves a challenge.
///
/// - In the browser: `n` Web Workers running [`WORKER_JS`](crate::WORKER_JS)
///   split the counters; if none can start (a CSP, an odd webview), it runs on
///   the main thread, yielding every 150 ms. Dropping the future terminates
///   them.
/// - Elsewhere with the `native` feature: `altcha::solve_challenge` on `n`
///   threads. They can't be cancelled: dropped, they run until the timeout.
/// - Elsewhere: [`SolveError::UnsupportedTarget`].
pub async fn solve(challenge: &RawChallenge, opts: &SolveOptions) -> Result<Solution, SolveError> {
    challenge.check_algorithm()?;
    imp::solve(challenge, opts).await
}

/// A worker's reply, or the main thread's result: a solution or an error.
/// (A struct, not an untagged enum: that costs kilobytes on the phone.)
#[derive(serde::Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
struct Reply {
    error: Option<String>,
    counter: Option<u32>,
    #[serde(rename = "derivedKey")]
    derived_key: Option<String>,
    time: Option<f64>,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn parse_reply(json: &str) -> Result<Solution, SolveError> {
    let unreadable = || SolveError::Worker("unreadable reply".into());
    let reply: Reply = serde_json::from_str(json).map_err(|_| unreadable())?;
    if let Some(error) = reply.error {
        return Err(SolveError::Worker(error));
    }
    match (reply.counter, reply.derived_key) {
        (Some(counter), Some(derived_key)) => Ok(Solution {
            counter,
            derived_key,
            time: reply.time,
        }),
        _ => Err(unreadable()),
    }
}

/// A JS exception's message.
#[cfg(target_arch = "wasm32")]
pub(crate) fn js_error(e: &wasm_bindgen::JsValue) -> String {
    use wasm_bindgen::JsCast;
    if let Some(err) = e.dyn_ref::<js_sys::Error>() {
        return err.message().into();
    }
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::{pin::pin, time::Duration};

    use altcha::Solution;
    use futures_channel::{mpsc, oneshot};
    use futures_util::{
        StreamExt,
        future::{self, Either},
    };
    use gloo_timers::future::TimeoutFuture;
    use js_sys::{Array, Function, Object, Promise, Reflect};
    use wasm_bindgen::{JsCast, JsValue, closure::Closure};
    use wasm_bindgen_futures::JsFuture;
    #[cfg(feature = "embed-worker")]
    use web_sys::{Blob, BlobPropertyBag};
    use web_sys::{MessageEvent, Url, Worker};

    use super::{js_error, parse_reply};
    #[cfg(feature = "embed-worker")]
    use crate::WORKER_JS;
    use crate::{RawChallenge, SolveError, SolveOptions};

    /// The main thread's solver yields this often.
    const YIELD_MS: u32 = 150;

    enum Event {
        Message(String),
        Error,
    }

    type OnMessage = Closure<dyn FnMut(MessageEvent)>;
    type OnError = Closure<dyn FnMut(web_sys::Event)>;

    /// Owns everything `solve` starts; dropping it stops them.
    #[derive(Default)]
    struct Guard {
        workers: Vec<(Worker, OnMessage, OnError)>,
        blob_url: Option<String>,
        /// The main thread's `{cancelled}` flag.
        ctl: Option<Object>,
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            // Detach the handlers before terminating, and drop the closures
            // only then: a queued message must not call a dropped closure.
            for (worker, _, _) in &self.workers {
                worker.set_onmessage(None);
                worker.set_onerror(None);
                worker.terminate();
            }
            self.workers.clear();
            if let Some(ctl) = &self.ctl {
                let _ = Reflect::set(ctl, &"cancelled".into(), &JsValue::TRUE);
            }
            if let Some(url) = self.blob_url.take() {
                let _ = Url::revoke_object_url(&url);
            }
        }
    }

    fn global(name: &str) -> JsValue {
        Reflect::get(&js_sys::global(), &name.into()).unwrap_or(JsValue::UNDEFINED)
    }

    fn secure_context() -> Result<(), SolveError> {
        let subtle = Reflect::get(&global("crypto"), &"subtle".into()).unwrap_or_default();
        if global("isSecureContext").as_bool() == Some(true) && subtle.is_object() {
            Ok(())
        } else {
            Err(SolveError::InsecureContext)
        }
    }

    fn default_workers() -> usize {
        let cores = Reflect::get(&global("navigator"), &"hardwareConcurrency".into())
            .ok()
            .and_then(|v| v.as_f64())
            .filter(|n| *n >= 1.0);
        cores.map_or(2, |n| (n as usize).min(4))
    }

    #[cfg(feature = "embed-worker")]
    fn blob_url() -> Result<String, SolveError> {
        let opts = BlobPropertyBag::new();
        opts.set_type("text/javascript");
        let parts = Array::of1(&JsValue::from_str(WORKER_JS));
        let blob = Blob::new_with_str_sequence_and_options(&parts, &opts)
            .map_err(|e| SolveError::Worker(js_error(&e)))?;
        Url::create_object_url_with_blob(&blob).map_err(|e| SolveError::Worker(js_error(&e)))
    }

    /// Starts `n` workers on `url`; `false` if the first can't be created.
    fn start_workers(
        guard: &mut Guard,
        url: &str,
        challenge: &RawChallenge,
        n: usize,
        events: &mpsc::UnboundedSender<Event>,
    ) -> bool {
        for i in 0..n {
            let Ok(worker) = Worker::new(url) else {
                return !guard.workers.is_empty();
            };
            let tx = events.clone();
            let on_message: OnMessage = Closure::new(move |e: MessageEvent| {
                let _ = tx.unbounded_send(Event::Message(e.data().as_string().unwrap_or_default()));
            });
            let tx = events.clone();
            let on_error: OnError = Closure::new(move |e: web_sys::Event| {
                e.prevent_default();
                let _ = tx.unbounded_send(Event::Error);
            });
            worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));
            let message = format!(
                r#"{{"challenge":{},"start":{i},"step":{n}}}"#,
                challenge.json()
            );
            if worker.post_message(&message.into()).is_err() {
                worker.terminate();
                continue;
            }
            guard.workers.push((worker, on_message, on_error));
        }
        !guard.workers.is_empty()
    }

    /// Loads the solver script into the page (once) and returns `altchaSolve`.
    async fn load_solver(url: &str) -> Result<Function, SolveError> {
        if let Ok(f) = global("altchaSolve").dyn_into::<Function>() {
            return Ok(f);
        }
        let document = web_sys::window()
            .and_then(|w| w.document())
            .ok_or(SolveError::UnsupportedTarget)?;
        let script: web_sys::HtmlScriptElement = document
            .create_element("script")
            .map_err(|e| SolveError::Worker(js_error(&e)))?
            .unchecked_into();
        let (tx, rx) = oneshot::channel::<bool>();
        let tx = std::rc::Rc::new(std::cell::RefCell::new(Some(tx)));
        let (ok_tx, err_tx) = (tx.clone(), tx);
        let on_load = Closure::<dyn FnMut()>::new(move || {
            if let Some(tx) = ok_tx.borrow_mut().take() {
                let _ = tx.send(true);
            }
        });
        let on_error = Closure::<dyn FnMut()>::new(move || {
            if let Some(tx) = err_tx.borrow_mut().take() {
                let _ = tx.send(false);
            }
        });
        script.set_onload(Some(on_load.as_ref().unchecked_ref()));
        script.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        script.set_src(url);
        let parent = document
            .head()
            .map(web_sys::Node::from)
            .or_else(|| document.body().map(Into::into))
            .ok_or_else(|| SolveError::Worker("no <head> to load the solver into".into()))?;
        parent
            .append_child(&script)
            .map_err(|e| SolveError::Worker(js_error(&e)))?;
        let loaded = rx.await.unwrap_or(false);
        script.set_onload(None);
        script.set_onerror(None);
        drop((on_load, on_error));
        if !loaded {
            return Err(SolveError::Worker("the solver script didn't load".into()));
        }
        global("altchaSolve")
            .dyn_into::<Function>()
            .map_err(|_| SolveError::Worker("the solver script defines no altchaSolve".into()))
    }

    async fn main_thread(
        guard: &mut Guard,
        url: &str,
        challenge: &RawChallenge,
    ) -> Result<Solution, SolveError> {
        let solve = load_solver(url).await?;
        let ctl = Object::new();
        let _ = Reflect::set(&ctl, &"yieldMs".into(), &YIELD_MS.into());
        guard.ctl = Some(ctl.clone());
        let parsed = js_sys::JSON::parse(challenge.json())
            .map_err(|e| SolveError::InvalidChallenge(js_error(&e)))?;
        let args = Array::of4(&parsed, &0.into(), &1.into(), &ctl);
        let promise: Promise = solve
            .apply(&JsValue::NULL, &args)
            .map_err(|e| SolveError::Worker(js_error(&e)))?
            .unchecked_into();
        let result = JsFuture::from(promise)
            .await
            .map_err(|e| SolveError::Worker(js_error(&e)))?;
        let json = js_sys::JSON::stringify(&result)
            .ok()
            .and_then(|s| s.as_string())
            .unwrap_or_default();
        parse_reply(&json)
    }

    fn visible() -> bool {
        web_sys::window()
            .and_then(|w| w.document())
            .is_none_or(|d| d.visibility_state() == web_sys::VisibilityState::Visible)
    }

    fn now_ms() -> f64 {
        js_sys::Date::now()
    }

    /// Resolves after `timeout` of time with the page visible: iOS suspends
    /// hidden pages, workers included, so hidden time isn't lost work.
    async fn visible_timeout(timeout: Option<Duration>) {
        let Some(timeout) = timeout else {
            return future::pending().await;
        };
        let mut left = timeout.as_millis() as f64;
        let (mut last, mut was_visible) = (now_ms(), visible());
        while left > 0.0 {
            TimeoutFuture::new(left.clamp(1.0, 250.0) as u32).await;
            let (now, is_visible) = (now_ms(), visible());
            if was_visible && is_visible {
                left -= now - last;
            }
            (last, was_visible) = (now, is_visible);
        }
    }

    pub async fn solve(
        challenge: &RawChallenge,
        opts: &SolveOptions,
    ) -> Result<Solution, SolveError> {
        secure_context()?;
        let mut guard = Guard::default();
        let url = match &opts.worker_url {
            Some(url) => url.clone(),
            #[cfg(feature = "embed-worker")]
            None => {
                let url = blob_url()?;
                guard.blob_url = Some(url.clone());
                url
            }
            #[cfg(not(feature = "embed-worker"))]
            None => {
                return Err(SolveError::Worker(
                    "no worker_url, and the worker isn't embedded (feature embed-worker)".into(),
                ));
            }
        };
        let n = opts.workers.unwrap_or_else(default_workers);
        let (tx, mut rx) = mpsc::unbounded();
        let started = start_workers(&mut guard, &url, challenge, n, &tx);
        drop(tx);

        let work = async {
            if started {
                let mut errors = 0;
                while let Some(event) = rx.next().await {
                    match event {
                        Event::Message(json) => return parse_reply(&json),
                        // A worker that can't load reports `error` before any
                        // message; when all have, fall back.
                        Event::Error => {
                            errors += 1;
                            if errors >= guard.workers.len() {
                                break;
                            }
                        }
                    }
                }
            }
            main_thread(&mut guard, &url, challenge).await
        };
        match future::select(pin!(work), pin!(visible_timeout(opts.timeout_or_default()))).await {
            Either::Left((result, _)) => result,
            Either::Right(_) => Err(SolveError::Timeout),
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
mod imp {
    use altcha::{CounterMode, Solution, SolveChallengeOptions};
    use futures_util::StreamExt;

    use crate::{RawChallenge, SolveError, SolveOptions};

    pub async fn solve(
        challenge: &RawChallenge,
        opts: &SolveOptions,
    ) -> Result<Solution, SolveError> {
        let challenge: altcha::Challenge = serde_json::from_str(challenge.json())
            .map_err(|e| SolveError::InvalidChallenge(e.to_string()))?;
        let n = opts
            .workers
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(2, |n| n.get().min(4)));
        let timeout_ms = opts
            .timeout_or_default()
            .map_or(0, |t| t.as_millis().max(1) as u64);
        let (tx, mut rx) = futures_channel::mpsc::unbounded();
        for i in 0..n {
            let (challenge, tx) = (challenge.clone(), tx.clone());
            std::thread::spawn(move || {
                let result = altcha::solve_challenge(SolveChallengeOptions {
                    challenge: &challenge,
                    counter_mode: CounterMode::Uint32,
                    counter_start: i as u32,
                    counter_step: n as u32,
                    timeout_ms,
                });
                let _ = tx.unbounded_send(result);
            });
        }
        drop(tx);
        while let Some(result) = rx.next().await {
            match result {
                Ok(Some(solution)) => return Ok(solution),
                Ok(None) => {}
                Err(e) => return Err(SolveError::InvalidChallenge(e.to_string())),
            }
        }
        Err(SolveError::Timeout)
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(feature = "native")))]
mod imp {
    use altcha::Solution;

    use crate::{RawChallenge, SolveError, SolveOptions};

    pub async fn solve(_: &RawChallenge, _: &SolveOptions) -> Result<Solution, SolveError> {
        Err(SolveError::UnsupportedTarget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies() {
        let s = parse_reply(r#"{"counter":7,"derivedKey":"00ab","time":1.5}"#).unwrap();
        assert_eq!((s.counter, s.derived_key.as_str()), (7, "00ab"));
        assert_eq!(
            parse_reply(r#"{"error":"boom"}"#).unwrap_err(),
            SolveError::Worker("boom".into())
        );
        assert!(parse_reply("nope").is_err());
    }
}
