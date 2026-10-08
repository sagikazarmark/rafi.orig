//! The live raffle view for the projector and the admin, plus a ticking clock.
//!
//! The full view (`/api/r/{slug}/view`) is polled every few seconds while the
//! raffle is live, and fetched on mount and whenever the page becomes
//! visible, is focused or comes back online. Polls can return the same `v`
//! with newer time-dependent content, so a view with `v >=` the current one
//! is taken.
//!
//! How else to follow the raffle comes from the server ([`RaffleView::live`]):
//!
//! - `partyline`: the participant view is also followed over the raffle's
//!   channel: an event ahead of the full view is kept as [`LiveView::ahead`],
//!   so the projector shows the reveal right away, and the full view is
//!   fetched at once in the background;
//! - `poll`: only the full view, polled every `every_ms`;
//! - `sse`: no client yet, so it polls like `poll`.

use std::rc::Rc;

use dioxus::prelude::*;
use partyline_dioxus::{BaseUrl, ChannelOptions, Status, use_channel_latest};
use rafi_core::raffle::RaffleState;
use rafi_live::{LiveTransport, ParticipantView, RaffleLive};

use crate::model::RaffleView;

/// How often the full view is polled while the raffle is live, unless the
/// server asks for polling at another interval.
const POLL_MS: u32 = 2_500;

/// A raffle followed live.
#[derive(Clone, Copy, PartialEq)]
pub struct LiveView {
    /// The full view; `None` until the first fetch succeeds.
    pub view: Signal<Option<RaffleView>>,
    /// The participant view from the live channel while it's ahead of
    /// [`view`](Self::view) (newer `v`).
    pub ahead: Signal<Option<ParticipantView>>,
    /// Fetches the full view now.
    pub refresh: Callback<()>,
}

impl LiveView {
    /// The participant view, if it's still ahead of the full view.
    pub fn ahead_of(&self, view: &RaffleView) -> Option<ParticipantView> {
        self.ahead.read().clone().filter(|p| p.v > view.v)
    }

    /// The live channel subscription, where the server pushes over
    /// partyline and the raffle is live; render it anywhere in the page.
    pub fn feed(self) -> Element {
        let subscription = self.view.read().as_ref().and_then(|v| match &v.live {
            LiveTransport::Partyline { base_url } if live(v.state) => {
                Some((v.raffle_id.clone(), base_url.clone()))
            }
            // Polling (and SSE, which has no client yet): the full view's poll does it all.
            _ => None,
        });
        rsx! {
            if let Some((raffle_id, base_url)) = subscription {
                ChannelFeed { key: "{raffle_id}", live: self, raffle_id: raffle_id.clone(), base_url }
            }
        }
    }
}

/// Public and not over: when the view changes on its own.
fn live(state: RaffleState) -> bool {
    matches!(
        state,
        RaffleState::Ready
            | RaffleState::Open
            | RaffleState::Closed
            | RaffleState::Frozen
            | RaffleState::Drawing
    )
}

/// Follows the raffle `slug`.
pub fn use_live_view(slug: String) -> LiveView {
    let mut view = use_signal(|| None::<RaffleView>);
    let mut ahead = use_signal(|| None::<ParticipantView>);

    let fetcher = use_coroutine(move |mut rx: UnboundedReceiver<()>| {
        let slug = slug.clone();
        async move {
            use futures_util::StreamExt;
            while rx.next().await.is_some() {
                // Requests that queued up meanwhile are answered by this fetch.
                while rx.try_recv().is_ok() {}
                let Ok(fresh) = crate::api::get_view(slug.clone()).await else {
                    continue;
                };
                if ahead.peek().as_ref().is_some_and(|p| p.v <= fresh.v) {
                    ahead.set(None);
                }
                let take = view.peek().as_ref().is_none_or(|cur| fresh.v >= cur.v);
                if take {
                    view.set(Some(fresh));
                }
            }
        }
    });
    let refresh = use_callback(move |()| fetcher.send(()));

    use_future(move || async move {
        fetcher.send(());
        loop {
            let every = view
                .peek()
                .as_ref()
                .and_then(|v| v.live.poll_every_ms())
                .unwrap_or(POLL_MS);
            gloo_timers::future::TimeoutFuture::new(every).await;
            if view.peek().as_ref().is_none_or(|v| live(v.state)) {
                fetcher.send(());
            }
        }
    });
    use_hook(move || Rc::new(wake::listen(move || fetcher.send(()))));

    LiveView {
        view,
        ahead,
        refresh,
    }
}

/// The raffle's live channel. Mounted only while it's wanted: unmounting
/// closes the socket.
#[component]
fn ChannelFeed(live: LiveView, raffle_id: String, base_url: Option<String>) -> Element {
    let mut options = ChannelOptions::new(raffle_id);
    if let Some(url) = base_url {
        options = options.base_url(BaseUrl::Explicit(url));
    }
    let (latest, channel) = use_channel_latest::<RaffleLive>(options);
    let mut ahead = live.ahead;
    use_effect(move || {
        let Some(p) = latest() else {
            return;
        };
        let behind = live.view.peek().as_ref().is_none_or(|v| p.v > v.v);
        let newer = ahead.peek().as_ref().is_none_or(|a| p.v > a.v);
        if behind && newer {
            ahead.set(Some(p));
            live.refresh.call(());
        }
    });
    // A reconnect may have missed events; a stopped channel leaves polling.
    let mut was_open = use_signal(|| false);
    use_effect(move || match channel.status() {
        Status::Open => {
            if *was_open.peek() {
                live.refresh.call(());
            }
            was_open.set(true);
        }
        Status::Stopped { .. } => live.refresh.call(()),
        _ => {}
    });
    rsx! {}
}

/// Milliseconds since the epoch, updated every `every_ms`.
pub fn use_clock(every_ms: u32) -> Signal<u64> {
    let mut now = use_signal(crate::model::now_unix_ms);
    use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(every_ms).await;
            now.set(crate::model::now_unix_ms());
        }
    });
    now
}

/// The page coming back: shown again, restored, focused or back online.
mod wake {
    use std::rc::Rc;

    use wasm_bindgen::{JsCast, closure::Closure};

    /// A target, its event and the callback.
    type Listener = (web_sys::EventTarget, &'static str, Closure<dyn Fn()>);

    /// Event listeners, removed when dropped.
    pub struct Listeners {
        added: Vec<Listener>,
    }

    impl Drop for Listeners {
        fn drop(&mut self) {
            for (target, event, cb) in &self.added {
                let _ =
                    target.remove_event_listener_with_callback(event, cb.as_ref().unchecked_ref());
            }
        }
    }

    /// Calls `f` whenever the page comes back.
    pub fn listen(f: impl Fn() + 'static) -> Listeners {
        let mut added = Vec::new();
        let Some(window) = web_sys::window() else {
            return Listeners { added };
        };
        let f = Rc::new(f);
        for event in ["pageshow", "focus", "online"] {
            let f = f.clone();
            let cb = Closure::<dyn Fn()>::new(move || f());
            let target: web_sys::EventTarget = window.clone().into();
            if target
                .add_event_listener_with_callback(event, cb.as_ref().unchecked_ref())
                .is_ok()
            {
                added.push((target, event, cb));
            }
        }
        if let Some(document) = window.document() {
            let doc = document.clone();
            let cb = Closure::<dyn Fn()>::new(move || {
                if doc.visibility_state() == web_sys::VisibilityState::Visible {
                    f();
                }
            });
            let target: web_sys::EventTarget = document.into();
            if target
                .add_event_listener_with_callback("visibilitychange", cb.as_ref().unchecked_ref())
                .is_ok()
            {
                added.push((target, "visibilitychange", cb));
            }
        }
        Listeners { added }
    }
}
