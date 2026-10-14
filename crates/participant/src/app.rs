//! The live card: mounted into `#live`, kept current from the channel and
//! from the participant endpoint.

use dioxus::prelude::*;
use partyline_dioxus::{BaseUrl, ChannelOptions, Status, use_channel_latest};
use rafi_live_client::{Card, Follow, ParticipantChannel, ParticipantView, Screen};
use wasm_bindgen::{JsCast, closure::Closure};

use crate::{clock, config::PageConfig, extras};

/// Clears the server-rendered card and mounts the live one in its place.
/// Pages without `#live` (the join form) get no card.
pub fn mount(cfg: PageConfig) {
    let Some(live) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("live"))
    else {
        return;
    };
    live.set_inner_html("");
    dioxus::LaunchBuilder::new()
        .with_cfg(dioxus::web::Config::new().rootname("live"))
        .with_context(cfg)
        .launch(App);
}

fn won(view: &ParticipantView, me: Option<&str>) -> bool {
    me.is_some_and(|me| view.win_of(me).is_some())
}

/// Fetches the current view; newer ones replace the card's.
async fn fetch_view(
    url: String,
    mut view: Signal<ParticipantView>,
    mut announce: Signal<bool>,
    me: Option<String>,
) {
    let sent_at = clock::local_ms();
    let Some((fresh, date)) = crate::net::get_json::<Option<ParticipantView>>(&url).await else {
        return;
    };
    if let Some(date) = date {
        clock::resync(&date, sent_at);
    }
    if let Some(fresh) = fresh {
        apply(fresh, &mut view, &mut announce, me.as_deref());
    }
}

/// Takes a view if it's newer. A win that happens now (not one the page was
/// loaded with) is announced: alert, pop, chime and vibration.
fn apply(
    fresh: ParticipantView,
    view: &mut Signal<ParticipantView>,
    announce: &mut Signal<bool>,
    me: Option<&str>,
) {
    if fresh.v <= view.peek().v {
        return;
    }
    if !won(&view.peek(), me) && won(&fresh, me) {
        announce.set(true);
        extras::announce_win();
    }
    view.set(fresh);
}

/// Calls `f` whenever the page comes back: shown again, restored from the
/// back/forward cache, focused, or back online.
fn on_wake(f: impl Fn() + 'static) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let f = std::rc::Rc::new(f);
    for event in ["pageshow", "focus", "online"] {
        let f = f.clone();
        let cb = Closure::<dyn Fn()>::new(move || f());
        let _ = window.add_event_listener_with_callback(event, cb.as_ref().unchecked_ref());
        cb.forget();
    }
    if let Some(document) = window.document() {
        let doc = document.clone();
        let cb = Closure::<dyn Fn()>::new(move || {
            if doc.visibility_state() == web_sys::VisibilityState::Visible {
                f();
            }
        });
        let _ = document
            .add_event_listener_with_callback("visibilitychange", cb.as_ref().unchecked_ref());
        cb.forget();
    }
}

#[component]
fn App() -> Element {
    let cfg: PageConfig = use_context();
    let me = cfg.entrant_id.clone();
    let view = use_signal(|| cfg.view.clone());
    let announce = use_signal(|| false);
    let mut now = use_signal(clock::now_ms);

    // The countdown ticks only while a draw is pending.
    use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(250).await;
            if view.peek().current.is_some() {
                now.set(clock::now_ms());
            }
        }
    });

    // Refetches run in a task of this component; the page's own event
    // listeners (outside Dioxus) only send on the channel.
    let refetch = use_hook(|| {
        let (tx, mut rx) = futures_channel::mpsc::unbounded::<()>();
        let (url, me) = (cfg.participant_url.clone(), me.clone());
        spawn(async move {
            use futures_util::StreamExt;
            while rx.next().await.is_some() {
                fetch_view(url.clone(), view, announce, me.clone()).await;
            }
        });
        let wake = tx.clone();
        on_wake(move || {
            let _ = wake.unbounded_send(());
        });
        tx
    });

    // A visitor's page predates registration opening: reload for the form.
    // Reading the view inside the effect re-runs it on every new view.
    let (reload_me, page_state) = (me.clone(), cfg.page_state);
    use_effect(move || {
        let screen =
            rafi_live_client::screen(&view(), reload_me.as_deref(), page_state, clock::now_ms());
        if matches!(screen, Screen::Reload)
            && let Some(window) = web_sys::window()
        {
            let _ = window.location().reload();
        }
    });

    let current = view();

    // The server says how to follow the raffle. Partyline subscribes (below);
    // polling (and SSE, which has no client yet) polls, twice as often while a
    // draw is pending.
    let poll_ms = cfg.follow.poll_every_ms();
    use_future({
        let refetch = refetch.clone();
        move || {
            let refetch = refetch.clone();
            async move {
                let Some(every) = poll_ms else {
                    return;
                };
                loop {
                    let pending = view.peek().current.is_some();
                    gloo_timers::future::TimeoutFuture::new(if pending {
                        every / 2
                    } else {
                        every
                    })
                    .await;
                    if view.peek().state.is_live() {
                        let _ = refetch.unbounded_send(());
                    }
                }
            }
        }
    });

    let subscription = match &cfg.follow {
        Follow::Partyline { base_url } if current.state.is_live() => Some(base_url.clone()),
        _ => None,
    };
    let me_for_live = me.clone();
    rsx! {
        Card {
            view: current,
            me: me.clone(),
            claim_code: cfg.claim_code.clone(),
            page_state: cfg.page_state,
            now_ms: now(),
            strings: cfg.strings.clone(),
            announce: announce(),
        }
        if let Some(base_url) = subscription {
            ChannelSubscription {
                raffle_id: cfg.raffle_id.clone(),
                base_url,
                on_view: move |fresh: ParticipantView| {
                    let (mut view, mut announce) = (view, announce);
                    apply(fresh, &mut view, &mut announce, me_for_live.as_deref());
                },
                on_resync: move |_| {
                    let _ = refetch.unbounded_send(());
                },
            }
        }
    }
}

/// The raffle's participant channel. Mounted only while the raffle is live:
/// unmounting (the final view) closes the socket.
#[component]
fn ChannelSubscription(
    raffle_id: String,
    base_url: Option<String>,
    on_view: EventHandler<ParticipantView>,
    on_resync: EventHandler<()>,
) -> Element {
    let mut options = ChannelOptions::new(raffle_id);
    if let Some(url) = base_url {
        options = options.base_url(BaseUrl::Explicit(url));
    }
    let (latest, channel) = use_channel_latest::<ParticipantChannel>(options);
    use_effect(move || {
        if let Some(fresh) = latest() {
            on_view.call(fresh);
        }
    });
    // A reconnect may have missed views; a stopped channel (4403: live updates
    // ended, or too many connections) leaves the endpoint as the only source.
    let mut was_open = use_signal(|| false);
    use_effect(move || match channel.status() {
        Status::Open => {
            if *was_open.peek() {
                on_resync.call(());
            }
            was_open.set(true);
        }
        Status::Stopped { .. } => on_resync.call(()),
        _ => {}
    });
    rsx! {}
}
