//! The live raffle view for the projector and the admin, plus a ticking clock.
//!
//! The full view (`/api/r/{slug}/view`) is polled every few seconds while the
//! raffle is live, and fetched on mount and whenever the page becomes
//! visible, is focused or comes back online. Polls can return the same `v`
//! with newer time-dependent content, so a view with `v >=` the current one
//! is taken.
//!
//! How else to follow the raffle comes from the server ([`RaffleView::follow`]):
//!
//! - `partyline`: the participant view is also followed over the raffle's
//!   participant channel: an event ahead of the full view is kept as [`LiveView::ahead`],
//!   so the projector shows the reveal right away, and the full view is
//!   fetched at once in the background;
//! - `poll`: only the full view, polled every `every_ms`.

use std::rc::Rc;

use dioxus::prelude::*;
use partyline_dioxus::{BaseUrl, ChannelOptions, Status, use_channel_latest};
use rafi_live_client::{Follow, ParticipantChannel, ParticipantView};

use rafi_api::RaffleView;

/// How often the full view is polled while the raffle is live, unless the
/// server asks for polling at another interval.
const POLL_MS: u32 = 2_500;

/// The longest a failing view waits before it is fetched again: a quarter of
/// the polling's rate, while a screen in front of a room still comes back
/// within seconds of the server.
const MAX_BACKOFF_MS: u32 = 10_000;

/// How long to wait before the next poll of the view, every `every_ms` while
/// it is fine: doubling with each fetch that failed in a row, to
/// [`MAX_BACKOFF_MS`], so a server in trouble isn't polled at full rate by
/// every screen; `None` while the page is hidden (it fetches again when shown).
pub fn next_poll_ms(every_ms: u32, failures: u32, hidden: bool) -> Option<u32> {
    if hidden {
        return None;
    }
    let backoff = every_ms.saturating_mul(1 << failures.min(10));
    Some(backoff.min(MAX_BACKOFF_MS.max(every_ms)))
}

/// `ms`, give or take a fifth, at random: screens polling together drift
/// apart.
fn jittered(ms: u32) -> u32 {
    let spread = f64::from(ms) * 0.2;
    (f64::from(ms) - spread + 2.0 * spread * js_sys::Math::random()).max(0.0) as u32
}

/// Whether the page is hidden (another tab, a locked screen).
fn page_hidden() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .is_some_and(|d| d.hidden())
}

/// How long the page coming back is one wake: focus, visibility and
/// `pageshow` often come together.
const WAKE_COALESCE_MS: f64 = 2_000.0;

/// How long fetches of the view fail before the page says it's offline: a
/// poll or two lost on venue wifi is nothing to show.
const OFFLINE_AFTER_MS: u64 = 10_000;

/// The server's clock as the page sees it: how far it is from the browser's,
/// taken when a view arrives (its `server_now_ms` against the browser's time
/// then). A countdown reads [`now`](Self::now) every tick; recomputing the
/// offset on each render instead would pin it to the view's own time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ServerClock {
    offset_ms: i64,
}

impl ServerClock {
    /// The clock of a view whose `server_now_ms` arrived at `local_ms`.
    pub fn at(server_now_ms: u64, local_ms: u64) -> Self {
        Self {
            offset_ms: server_now_ms as i64 - local_ms as i64,
        }
    }

    /// The server's time (unix ms) when the browser's is `local_ms`.
    pub fn now(self, local_ms: u64) -> i64 {
        local_ms as i64 + self.offset_ms
    }
}

/// What went wrong fetching the view, while it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TroubleKind {
    /// No published raffle at this address.
    NotFound,
    /// The server sent a view this page can't read: the server is newer.
    Newer,
    /// The server couldn't be reached, or failed.
    Unreachable,
}

/// Fetches of the view failing in a row; gone with the first that succeeds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trouble {
    pub kind: TroubleKind,
    /// How many failed in a row.
    pub failures: u32,
    /// When the first of them failed, and the latest (browser ms).
    pub since_ms: u64,
    pub last_ms: u64,
}

impl Trouble {
    /// After `previous`, a fetch failing at `now_ms` as `kind`.
    pub fn after(previous: Option<Self>, kind: TroubleKind, now_ms: u64) -> Self {
        match previous {
            Some(p) => Self {
                kind,
                failures: p.failures + 1,
                since_ms: p.since_ms,
                last_ms: now_ms,
            },
            None => Self {
                kind,
                failures: 1,
                since_ms: now_ms,
                last_ms: now_ms,
            },
        }
    }

    /// The kind of an error from fetching the view.
    pub fn kind_of(e: &crate::api::ApiError) -> TroubleKind {
        use crate::api::codes;
        match e.code.as_str() {
            codes::NOT_FOUND => TroubleKind::NotFound,
            codes::UNREADABLE => TroubleKind::Newer,
            _ => TroubleKind::Unreachable,
        }
    }

    /// Whether fetches have failed for long enough to say so.
    pub fn offline(&self) -> bool {
        self.last_ms.saturating_sub(self.since_ms) >= OFFLINE_AFTER_MS
    }
}

/// A raffle followed live.
#[derive(Clone, Copy, PartialEq)]
pub struct LiveView {
    /// The full view; `None` until the first fetch succeeds.
    pub view: Signal<Option<RaffleView>>,
    /// The server's clock, as of the view.
    pub clock: Signal<ServerClock>,
    /// Why fetching the view fails, while it does.
    pub trouble: Signal<Option<Trouble>>,
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

    /// The participant channel subscription, where the server pushes over
    /// partyline and the raffle is live; render it anywhere in the page.
    pub fn subscription(self) -> Element {
        let subscription = self.view.read().as_ref().and_then(|v| match &v.follow {
            Follow::Partyline { base_url } if v.state.is_live() => {
                Some((v.raffle_id.clone(), base_url.clone()))
            }
            // Polling: the full view's poll does it all.
            _ => None,
        });
        rsx! {
            if let Some((raffle_id, base_url)) = subscription {
                ChannelSubscription { key: "{raffle_id}", live: self, raffle_id: raffle_id.clone(), base_url }
            }
        }
    }
}

/// Follows the raffle `slug`.
pub fn use_live_view(slug: String) -> LiveView {
    let mut view = use_signal(|| None::<RaffleView>);
    let mut clock = use_signal(ServerClock::default);
    let mut trouble = use_signal(|| None::<Trouble>);
    let mut ahead = use_signal(|| None::<ParticipantView>);

    let fetcher = use_coroutine(move |mut rx: UnboundedReceiver<()>| {
        let slug = slug.clone();
        async move {
            use futures_util::StreamExt;
            while rx.next().await.is_some() {
                // Requests that queued up meanwhile are answered by this fetch.
                while rx.try_recv().is_ok() {}
                let fresh = match crate::api::get_view(slug.clone()).await {
                    Ok(fresh) => fresh,
                    Err(e) => {
                        let previous = *trouble.peek();
                        trouble.set(Some(Trouble::after(
                            previous,
                            Trouble::kind_of(&e),
                            crate::ui::now_ms(),
                        )));
                        continue;
                    }
                };
                if trouble.peek().is_some() {
                    trouble.set(None);
                }
                if ahead.peek().as_ref().is_some_and(|p| p.v <= fresh.v) {
                    ahead.set(None);
                }
                let take = view.peek().as_ref().is_none_or(|cur| fresh.v >= cur.v);
                if take {
                    clock.set(ServerClock::at(fresh.server_now_ms, crate::ui::now_ms()));
                    view.set(Some(fresh));
                }
            }
        }
    });
    let refresh = use_callback(move |()| fetcher.send(()));

    // Polled while live, with jitter, backing off while fetches fail, and not
    // at all while the page is hidden (a wake fetches at once).
    use_future(move || async move {
        fetcher.send(());
        loop {
            let every = view
                .peek()
                .as_ref()
                .and_then(|v| v.follow.poll_every_ms())
                .unwrap_or(POLL_MS);
            let failures = trouble.peek().map_or(0, |t| t.failures);
            let Some(wait) = next_poll_ms(every, failures, page_hidden()) else {
                gloo_timers::future::TimeoutFuture::new(1_000).await;
                continue;
            };
            gloo_timers::future::TimeoutFuture::new(jittered(wait)).await;
            if !page_hidden() && view.peek().as_ref().is_none_or(|v| v.state.is_live()) {
                fetcher.send(());
            }
        }
    });
    use_hook(move || {
        let last = std::cell::Cell::new(f64::NEG_INFINITY);
        Rc::new(wake::listen(move || {
            let now = js_sys::Date::now();
            if now - last.get() >= WAKE_COALESCE_MS {
                last.set(now);
                fetcher.send(());
            }
        }))
    });

    LiveView {
        view,
        clock,
        trouble,
        ahead,
        refresh,
    }
}

/// Reloads the page, unless it did so in the last minute: a page older than
/// the server gets the new one, and a server sending what no page reads
/// doesn't make it reload forever.
pub fn reload_once() {
    document::eval(
        "try { const k = 'rafi-reloaded-at', t = Number(sessionStorage.getItem(k)) || 0; \
         if (Date.now() - t > 60000) { sessionStorage.setItem(k, String(Date.now())); location.reload(); } } catch (e) {}",
    );
}

/// The raffle's participant channel. Mounted only while it's wanted:
/// unmounting closes the socket.
#[component]
fn ChannelSubscription(live: LiveView, raffle_id: String, base_url: Option<String>) -> Element {
    let mut options = ChannelOptions::new(raffle_id);
    if let Some(url) = base_url {
        options = options.base_url(BaseUrl::Explicit(url));
    }
    let (latest, channel) = use_channel_latest::<ParticipantChannel>(options);
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
    let mut now = use_signal(crate::ui::now_ms);
    use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(every_ms).await;
            now.set(crate::ui::now_ms());
        }
    });
    now
}

/// Whether the browser says it's online (`navigator.onLine`), following its
/// `online` and `offline` events.
pub fn use_online() -> Signal<bool> {
    let on_line = || web_sys::window().is_none_or(|w| w.navigator().on_line());
    let online = use_signal(on_line);
    use_hook(move || {
        Rc::new(wake::on(&["online", "offline"], move || {
            let mut online = online;
            online.set(on_line())
        }))
    });
    online
}

/// How the admin's pages are connected, for the badge in their header: why
/// the live view of the raffle shown can't be fetched, while it can't.
#[derive(Clone, Copy, PartialEq)]
pub struct AdminConnection(pub Signal<Option<Trouble>>);

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

    /// Calls `f` on each of the window's `events`.
    pub fn on(events: &[&'static str], f: impl Fn() + 'static) -> Listeners {
        let mut added = Vec::new();
        let Some(window) = web_sys::window() else {
            return Listeners { added };
        };
        let f = Rc::new(f);
        for &event in events {
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
        Listeners { added }
    }

    /// Calls `f` whenever the page comes back.
    pub fn listen(f: impl Fn() + 'static) -> Listeners {
        let f = Rc::new(f);
        let mut listeners = {
            let f = f.clone();
            on(&["pageshow", "focus", "online"], move || f())
        };
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return listeners;
        };
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
            listeners.added.push((target, "visibilitychange", cb));
        }
        listeners
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The server's time moves on with the browser's from the moment the
    /// view arrived, whatever the two clocks say.
    #[test]
    fn the_server_clock_runs_from_the_view() {
        let clock = ServerClock::at(1_000_000, 400_000);
        assert_eq!(clock.now(400_000), 1_000_000);
        assert_eq!(clock.now(401_000), 1_001_000);
        // A browser ahead of the server.
        let clock = ServerClock::at(1_000_000, 1_500_000);
        assert_eq!(clock.now(1_502_500), 1_002_500);
    }

    /// Failures count up from the first; the page says it's offline only
    /// after OFFLINE_AFTER_MS of them.
    #[test]
    fn trouble_counts_failures_in_a_row() {
        let first = Trouble::after(None, TroubleKind::Unreachable, 1_000);
        assert_eq!((first.failures, first.since_ms), (1, 1_000));
        assert!(!first.offline());
        let later = Trouble::after(
            Some(first),
            TroubleKind::Unreachable,
            1_000 + OFFLINE_AFTER_MS - 1,
        );
        assert_eq!((later.failures, later.since_ms), (2, 1_000));
        assert!(!later.offline());
        let later = Trouble::after(Some(later), TroubleKind::NotFound, 1_000 + OFFLINE_AFTER_MS);
        assert_eq!(later.kind, TroubleKind::NotFound);
        assert!(later.offline());
    }

    /// The schedule: the interval while all is well, doubling with each
    /// failure to the ceiling, none while hidden.
    #[test]
    fn polls_back_off_and_pause() {
        assert_eq!(next_poll_ms(2_500, 0, false), Some(2_500));
        assert_eq!(next_poll_ms(2_500, 1, false), Some(5_000));
        assert_eq!(next_poll_ms(2_500, 2, false), Some(10_000));
        assert_eq!(next_poll_ms(2_500, 3, false), Some(MAX_BACKOFF_MS));
        assert_eq!(next_poll_ms(2_500, 40, false), Some(MAX_BACKOFF_MS));
        assert_eq!(next_poll_ms(2_500, 0, true), None);
        // A server asking for a slower interval gets it.
        assert_eq!(next_poll_ms(60_000, 2, false), Some(60_000));
    }

    #[test]
    fn the_kind_of_a_failure() {
        use crate::api::{ApiError, codes};
        let kind = |code| Trouble::kind_of(&ApiError::new(code, ""));
        assert_eq!(kind(codes::NOT_FOUND), TroubleKind::NotFound);
        assert_eq!(kind(codes::UNREADABLE), TroubleKind::Newer);
        assert_eq!(kind(codes::CLIENT), TroubleKind::Unreachable);
        assert_eq!(kind(codes::INTERNAL), TroubleKind::Unreachable);
    }
}
