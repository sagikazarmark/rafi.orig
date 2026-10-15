//! The Dioxus layer: [`use_altcha`] and [`AltchaField`].

use std::{cell::RefCell, fmt, fmt::Display, future::Future, rc::Rc};

use dioxus::{core::Task, prelude::*};

use crate::{RawChallenge, SolveError, SolveOptions};

/// When the hook fetches and solves on its own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Auto {
    /// On mount, and again when a solved challenge expires or after
    /// [`UseAltcha::refresh`].
    #[default]
    OnMount,
    /// Only on [`UseAltcha::start`] (e.g. from the form's `onfocusin`) or
    /// [`UseAltcha::solved`] (on submit).
    Off,
}

/// How [`use_altcha_with_options`] behaves. Read on the first render only.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct AltchaOptions {
    auto: Auto,
    solve: SolveOptions,
}

impl AltchaOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Default: [`Auto::OnMount`].
    pub fn auto(mut self, auto: Auto) -> Self {
        self.auto = auto;
        self
    }

    pub fn solve(mut self, opts: SolveOptions) -> Self {
        self.solve = opts;
        self
    }
}

/// Where the hook is. The first render is always `Idle`, on the server too,
/// so hydration matches.
#[derive(Debug, Clone, PartialEq)]
pub enum AltchaState {
    Idle,
    Fetching,
    Solving,
    /// `payload` is the form value.
    Solved {
        payload: String,
    },
    /// The solved challenge expired; with [`Auto::OnMount`] a new one is on
    /// its way.
    Expired,
    Failed(AltchaError),
}

/// Why the hook failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AltchaError {
    #[error("fetching the challenge failed: {0}")]
    Fetch(String),
    #[error(transparent)]
    Solve(#[from] SolveError),
    #[error("the solve was cancelled")]
    Cancelled,
}

/// The handle [`use_altcha`] returns. `Copy`; pass it to children and to
/// [`AltchaField`].
#[derive(Clone, Copy)]
pub struct UseAltcha {
    state: Signal<AltchaState>,
    /// Made once: each `Signal -> ReadSignal` conversion allocates in the
    /// current scope.
    read: ReadSignal<AltchaState>,
    /// When the solved payload stops being usable, on [`clock::now`]'s scale.
    deadline: Signal<Option<f64>>,
    run: Callback<()>,
    refresh: Callback<()>,
    done: UseWaker<Result<String, AltchaError>>,
}

impl PartialEq for UseAltcha {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
    }
}

impl fmt::Debug for UseAltcha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UseAltcha")
            .field("state", &*self.state.peek())
            .finish_non_exhaustive()
    }
}

impl UseAltcha {
    /// The current state (subscribes).
    pub fn state(&self) -> AltchaState {
        self.state.cloned()
    }

    pub fn signal(&self) -> ReadSignal<AltchaState> {
        self.read
    }

    /// The form value, if solved and not yet past its deadline (subscribes).
    pub fn payload(&self) -> Option<String> {
        match &*self.state.read() {
            AltchaState::Solved { payload }
                if clock::usable(*self.deadline.peek(), clock::now()) =>
            {
                Some(payload.clone())
            }
            _ => None,
        }
    }

    /// Fetches and solves, unless that's under way or a usable solution is
    /// already in.
    pub fn start(&self) {
        if can_start(&self.state.peek(), *self.deadline.peek(), clock::now()) {
            self.run.call(());
        }
    }

    /// Drops the current challenge, e.g. after a submit spent it: starts
    /// again with [`Auto::OnMount`], goes `Idle` with [`Auto::Off`].
    pub fn refresh(&self) {
        self.refresh.call(());
    }

    /// The form value, solving first if need be: for solving on submit.
    ///
    /// ```ignore
    /// onsubmit: move |e| async move {
    ///     e.prevent_default();
    ///     let payload = altcha.solved().await?;
    ///     submit(payload).await
    /// }
    /// ```
    pub async fn solved(&self) -> Result<String, AltchaError> {
        if let Some(payload) = self.usable_payload() {
            return Ok(payload);
        }
        if !matches!(
            *self.state.peek(),
            AltchaState::Fetching | AltchaState::Solving
        ) {
            self.run.call(());
        }
        self.done
            .wait()
            .await
            .unwrap_or(Err(AltchaError::Cancelled))
    }

    fn usable_payload(&self) -> Option<String> {
        match &*self.state.peek() {
            AltchaState::Solved { payload }
                if clock::usable(*self.deadline.peek(), clock::now()) =>
            {
                Some(payload.clone())
            }
            _ => None,
        }
    }
}

/// Whether [`UseAltcha::start`] starts: not while fetching or solving, nor
/// while a solved payload is usable.
fn can_start(state: &AltchaState, deadline: Option<f64>, now: f64) -> bool {
    match state {
        AltchaState::Fetching | AltchaState::Solving => false,
        AltchaState::Solved { .. } => !clock::usable(deadline, now),
        AltchaState::Idle | AltchaState::Expired | AltchaState::Failed(_) => true,
    }
}

/// [`use_altcha_with_options`] with the defaults: solves on mount.
///
/// ```ignore
/// let altcha = use_altcha(|| fetch_challenge("/api/altcha"));
/// rsx! { form { AltchaField { altcha } /* ... */ } }
/// ```
pub fn use_altcha<F, Fut, E>(fetch: F) -> UseAltcha
where
    F: FnMut() -> Fut + 'static,
    Fut: Future<Output = Result<RawChallenge, E>> + 'static,
    E: Display + 'static,
{
    use_altcha_with_options(fetch, AltchaOptions::default())
}

/// Fetches a challenge with `fetch` (a `GET` with
/// [`fetch_challenge`](crate::fetch_challenge), or a server function) and
/// solves it. The work belongs to the component that calls the hook: it's
/// cancelled when that unmounts.
pub fn use_altcha_with_options<F, Fut, E>(fetch: F, opts: AltchaOptions) -> UseAltcha
where
    F: FnMut() -> Fut + 'static,
    Fut: Future<Output = Result<RawChallenge, E>> + 'static,
    E: Display + 'static,
{
    // The latest closure, as in `use_future`.
    let fetch_cell = use_hook(|| Rc::new(RefCell::new(None::<F>)));
    *fetch_cell.borrow_mut() = Some(fetch);
    let opts = use_hook(move || opts);

    let mut state = use_signal(|| AltchaState::Idle);
    let mut deadline = use_signal(|| None::<f64>);
    let mut task = use_signal(|| None::<Task>);
    let mut done = use_waker::<Result<String, AltchaError>>();

    // Callbacks run in this scope whoever calls them, so the task is ours.
    let run = use_callback({
        let opts = opts.clone();
        move |()| {
            if let Some(t) = task.write().take() {
                t.cancel();
            }
            let (fetch, opts) = (fetch_cell.clone(), opts.clone());
            task.set(Some(spawn(run_loop(fetch, opts, state, deadline, done))));
        }
    });
    let auto = opts.auto;
    let refresh = use_callback(move |()| {
        // The spent payload goes at once: until the new task runs, a
        // `solved()` must wait for a fresh one, not return this.
        deadline.set(None);
        state.set(AltchaState::Idle);
        if auto == Auto::OnMount {
            // Cancels the current task and starts another; `solved()` callers
            // keep waiting for its result.
            run.call(());
        } else if let Some(t) = task.write().take() {
            t.cancel();
            done.wake(Err(AltchaError::Cancelled));
        }
    });

    let read = use_hook(move || ReadSignal::from(state));
    let handle = UseAltcha {
        state,
        read,
        deadline,
        run,
        refresh,
        done,
    };
    use_hook(move || {
        if auto == Auto::OnMount {
            run.call(());
        }
    });
    #[cfg(target_arch = "wasm32")]
    bfcache::use_refresh_on_restore(refresh);
    handle
}

async fn run_loop<F, Fut, E>(
    fetch: Rc<RefCell<Option<F>>>,
    opts: AltchaOptions,
    mut state: Signal<AltchaState>,
    mut deadline: Signal<Option<f64>>,
    mut done: UseWaker<Result<String, AltchaError>>,
) where
    F: FnMut() -> Fut + 'static,
    Fut: Future<Output = Result<RawChallenge, E>> + 'static,
    E: Display + 'static,
{
    let mut fail = move |e: AltchaError| {
        state.set(AltchaState::Failed(e.clone()));
        done.wake(Err(e));
    };
    loop {
        // Set inside the task, never during render: hydration sees `Idle`.
        deadline.set(None);
        state.set(AltchaState::Fetching);
        let request = {
            let mut fetch = fetch.borrow_mut();
            (fetch.as_mut().expect("set on every render"))()
        };
        let challenge = match request.await {
            Ok(c) => c,
            Err(e) => return fail(AltchaError::Fetch(e.to_string())),
        };
        let until = challenge
            .usable_for()
            .map(|left| clock::now() + left.as_secs_f64() * 1000.0);
        state.set(AltchaState::Solving);
        match crate::solve(&challenge, &opts.solve).await {
            Ok(solution) => {
                let payload = crate::payload(&challenge, &solution);
                deadline.set(until);
                state.set(AltchaState::Solved {
                    payload: payload.clone(),
                });
                done.wake(Ok(payload));
            }
            Err(e) => return fail(e.into()),
        }
        let Some(until) = until else { return };
        clock::sleep_until(until).await;
        state.set(AltchaState::Expired);
        if opts.auto != Auto::OnMount {
            return;
        }
    }
}

/// The form's hidden input: empty until solved.
#[component]
pub fn AltchaField(
    altcha: UseAltcha,
    #[props(default = "altcha".to_string(), into)] name: String,
    #[props(extends = input)] attributes: Vec<Attribute>,
) -> Element {
    rsx! {
        input {
            r#type: "hidden",
            name,
            value: altcha.payload().unwrap_or_default(),
            ..attributes,
        }
    }
}

/// Deadlines on a monotonic clock, in milliseconds.
pub(crate) mod clock {
    /// Expire this long before the server would, so a payload doesn't expire
    /// in transit; at most half the time left.
    pub const MARGIN_MS: f64 = 30_000.0;
    /// A challenge claiming to last longer than this means the device's
    /// clock is off: leave expiry to the server.
    pub const MAX_TTL_MS: f64 = 24.0 * 3600.0 * 1000.0;

    /// When a challenge received at `received` (monotonic) stops being
    /// usable: `None` if it doesn't expire, or the device's clock is too far
    /// off to tell (it would otherwise loop fetching already-expired ones).
    pub fn deadline(received: f64, expires_at: Option<i64>, wall_ms: f64) -> Option<f64> {
        let left = expires_at? as f64 * 1000.0 - wall_ms;
        if !(left > 0.0 && left <= MAX_TTL_MS) {
            return None;
        }
        Some(received + left - MARGIN_MS.min(left / 2.0))
    }

    pub fn usable(deadline: Option<f64>, now: f64) -> bool {
        deadline.is_none_or(|d| now < d)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn now() -> f64 {
        web_sys::window()
            .and_then(|w| w.performance())
            .map_or_else(js_sys::Date::now, |p| p.now())
    }

    #[cfg(target_arch = "wasm32")]
    pub fn wall_ms() -> f64 {
        js_sys::Date::now()
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn sleep_until(deadline: f64) {
        // Background tabs throttle timers: re-check rather than trust one.
        loop {
            let left = deadline - now();
            if left <= 0.0 {
                return;
            }
            gloo_timers::future::TimeoutFuture::new(left.min(60_000.0) as u32).await;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start() -> std::time::Instant {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        *START.get_or_init(std::time::Instant::now)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn now() -> f64 {
        start().elapsed().as_secs_f64() * 1000.0
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn wall_ms() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0
    }

    /// A thread and a oneshot: no async runtime to assume.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn sleep_until(deadline: f64) {
        let left = (deadline - now()).max(0.0);
        let (tx, rx) = futures_channel::oneshot::channel::<()>();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs_f64(left / 1000.0));
            let _ = tx.send(());
        });
        let _ = rx.await;
    }
}

/// Back/forward cache: a restored page would post a spent challenge.
#[cfg(target_arch = "wasm32")]
mod bfcache {
    use std::rc::Rc;

    use dioxus::prelude::*;
    use wasm_bindgen::{JsCast, closure::Closure};
    use web_sys::PageTransitionEvent;

    pub fn use_refresh_on_restore(refresh: Callback<()>) {
        let listener = use_hook(|| {
            let closure =
                Closure::<dyn FnMut(PageTransitionEvent)>::new(move |e: PageTransitionEvent| {
                    if e.persisted() {
                        refresh.call(());
                    }
                });
            if let Some(window) = web_sys::window() {
                let _ = window
                    .add_event_listener_with_callback("pageshow", closure.as_ref().unchecked_ref());
            }
            Rc::new(closure)
        });
        use_drop(move || {
            if let Some(window) = web_sys::window() {
                let _ = window.remove_event_listener_with_callback(
                    "pageshow",
                    (*listener).as_ref().unchecked_ref(),
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{clock::*, *};

    #[test]
    fn deadlines() {
        let wall = 1_000_000_000_000.0; // ms
        // 20 minutes left: expire 30 s early, on the monotonic scale.
        let d = deadline(500.0, Some((wall / 1000.0) as i64 + 1200), wall).unwrap();
        assert_eq!(d, 500.0 + 1_200_000.0 - 30_000.0);
        // 40 s left: the margin is at most half.
        let d = deadline(0.0, Some((wall / 1000.0) as i64 + 40), wall).unwrap();
        assert_eq!(d, 20_000.0);
        // No expiry, already expired (clock ahead), or a day away (clock behind).
        assert_eq!(deadline(0.0, None, wall), None);
        assert_eq!(deadline(0.0, Some((wall / 1000.0) as i64 - 5), wall), None);
        assert_eq!(
            deadline(0.0, Some((wall / 1000.0) as i64 + 90_000), wall),
            None
        );
    }

    #[test]
    fn starting() {
        let solved = AltchaState::Solved {
            payload: "p".into(),
        };
        assert!(can_start(&AltchaState::Idle, None, 0.0));
        assert!(can_start(&AltchaState::Expired, None, 0.0));
        assert!(can_start(
            &AltchaState::Failed(AltchaError::Cancelled),
            None,
            0.0
        ));
        assert!(!can_start(&AltchaState::Fetching, None, 0.0));
        assert!(!can_start(&AltchaState::Solving, None, 0.0));
        assert!(!can_start(&solved, None, 0.0));
        assert!(!can_start(&solved, Some(10.0), 5.0));
        assert!(can_start(&solved, Some(10.0), 10.0));
    }
}
