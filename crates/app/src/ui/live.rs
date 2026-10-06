//! Live raffle view over server-sent events, plus a ticking clock.

use dioxus::prelude::*;

use crate::model::RaffleView;

/// Subscribes to `/api/r/{slug}/events` in the browser. `EventSource`
/// reconnects on its own; a watchdog reconnects if no event (or ping) arrived
/// for a while, e.g. after a laptop wakes from sleep.
pub fn use_live_view(slug: String) -> Signal<Option<RaffleView>> {
    let mut view = use_signal(|| None::<RaffleView>);
    let rx = use_coroutine(move |mut rx: UnboundedReceiver<RaffleView>| async move {
        use futures_util::StreamExt;
        while let Some(v) = rx.next().await {
            let newer = view.peek().as_ref().is_none_or(|cur| v.v > cur.v);
            if newer {
                view.set(Some(v));
            }
        }
    });

    #[cfg(feature = "web")]
    {
        let slug = slug.clone();
        use_hook(move || web::connect(slug, rx));
    }
    #[cfg(not(feature = "web"))]
    let _ = (slug, rx);
    view
}

/// Milliseconds since the epoch, updated every `every_ms` (client only).
pub fn use_clock(every_ms: u32) -> Signal<u64> {
    let mut now = use_signal(crate::model::now_unix_ms);
    use_future(move || async move {
        #[cfg(feature = "web")]
        loop {
            gloo_timers::future::TimeoutFuture::new(every_ms).await;
            now.set(crate::model::now_unix_ms());
        }
        #[cfg(not(feature = "web"))]
        {
            let _ = (every_ms, &mut now);
        }
    });
    now
}

#[cfg(feature = "web")]
mod web {
    use std::{cell::RefCell, rc::Rc};

    use dioxus::prelude::*;
    use wasm_bindgen::{JsCast, closure::Closure};

    use crate::model::RaffleView;

    pub struct Live {
        es: Rc<RefCell<Option<web_sys::EventSource>>>,
    }

    impl Drop for Live {
        fn drop(&mut self) {
            if let Some(es) = self.es.borrow_mut().take() {
                es.close();
            }
        }
    }

    pub fn connect(slug: String, tx: Coroutine<RaffleView>) -> Rc<Live> {
        let es: Rc<RefCell<Option<web_sys::EventSource>>> = Rc::default();
        let last = Rc::new(RefCell::new(js_sys::Date::now()));
        let url = format!("/api/r/{slug}/events");

        let open = {
            let (es, last, url) = (es.clone(), last.clone(), url.clone());
            Rc::new(move || {
                if let Some(old) = es.borrow_mut().take() {
                    old.close();
                }
                let Ok(source) = web_sys::EventSource::new(&url) else {
                    return;
                };
                let last_v = last.clone();
                let on_view = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
                    move |e: web_sys::MessageEvent| {
                        *last_v.borrow_mut() = js_sys::Date::now();
                        if let Some(data) = e.data().as_string()
                            && let Ok(v) = serde_json::from_str::<RaffleView>(&data)
                        {
                            tx.send(v);
                        }
                    },
                );
                let last_p = last.clone();
                let on_ping = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
                    move |_e: web_sys::MessageEvent| {
                        *last_p.borrow_mut() = js_sys::Date::now();
                    },
                );
                let _ = source
                    .add_event_listener_with_callback("view", on_view.as_ref().unchecked_ref());
                let _ = source
                    .add_event_listener_with_callback("ping", on_ping.as_ref().unchecked_ref());
                on_view.forget();
                on_ping.forget();
                *es.borrow_mut() = Some(source);
            })
        };
        open();

        // Watchdog: pings arrive every 15 s; reconnect if we heard nothing for 35 s.
        let (es_w, open_w) = (es.clone(), open.clone());
        spawn(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(5_000).await;
                if es_w.borrow().is_none() {
                    break;
                }
                if js_sys::Date::now() - *last.borrow() > 35_000.0 {
                    *last.borrow_mut() = js_sys::Date::now();
                    open_w();
                }
            }
        });

        Rc::new(Live { es })
    }
}
