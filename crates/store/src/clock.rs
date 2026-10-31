//! What time it is: the [`Clock`] the dates go by, and the wall clock
//! ([`now_ms`]) the screens' countdowns go by.

use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

/// The time in unix seconds that Rafi's dates go by: every timestamp the
/// store writes (it takes them as `now`), the erasure date and the winner
/// hold, housekeeping's runs, and when a joined name shows on the screen.
///
/// In production it is the system's clock ([`Clock::system`]). A test sets
/// it instead ([`Clock::fixed`]) and moves it ([`Clock::set`],
/// [`Clock::advance`]): "it is now the erasure date", rather than an event
/// dated ten days ago. Clones share the time, so the app and the in-process
/// engine's [`RaffleWriter`](crate::RaffleWriter) can follow one test clock.
///
/// What is real time stays off it: drand's rounds and the engine's waits,
/// the screens' countdowns ([`now_ms`]), the proof-of-work's expiry (its
/// library's clock) and the rate limits.
#[derive(Clone, Default)]
pub struct Clock {
    /// `None` for the system's clock.
    fixed: Option<Arc<AtomicI64>>,
}

impl Clock {
    /// The system's clock (on Workers, JavaScript's).
    pub fn system() -> Self {
        Self::default()
    }

    /// A clock that says `now` until it is set or advanced: for tests.
    pub fn fixed(now: i64) -> Self {
        Self {
            fixed: Some(Arc::new(AtomicI64::new(now))),
        }
    }

    /// The current time in unix seconds.
    pub fn now(&self) -> i64 {
        match &self.fixed {
            Some(now) => now.load(Ordering::SeqCst),
            None => system_now(),
        }
    }

    /// Sets a [fixed](Clock::fixed) clock to `now`, for it and its clones.
    ///
    /// # Panics
    ///
    /// On the system's clock, which nothing sets.
    pub fn set(&self, now: i64) {
        self.fixed_time().store(now, Ordering::SeqCst);
    }

    /// Moves a [fixed](Clock::fixed) clock `secs` on (back, if negative).
    ///
    /// # Panics
    ///
    /// On the system's clock, which nothing sets.
    pub fn advance(&self, secs: i64) {
        self.fixed_time().fetch_add(secs, Ordering::SeqCst);
    }

    fn fixed_time(&self) -> &AtomicI64 {
        self.fixed
            .as_deref()
            .expect("only a fixed clock is set; the system's is real time")
    }
}

impl std::fmt::Debug for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.fixed {
            Some(now) => f
                .debug_tuple("Clock::fixed")
                .field(&now.load(Ordering::SeqCst))
                .finish(),
            None => f.write_str("Clock::system"),
        }
    }
}

/// The system's time in unix seconds. `std::time` aborts on
/// `wasm32-unknown-unknown`, so Workers read the JavaScript clock.
fn system_now() -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.0) as i64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    }
}

/// The wall clock in unix milliseconds, never a [`Clock`]: the screens
/// measure their offset from it to count down to a drand round, which is
/// real time.
pub fn now_ms() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::Clock;

    #[test]
    fn a_fixed_clock_moves_only_when_told_and_its_clones_follow() {
        let clock = Clock::fixed(100);
        let clone = clock.clone();
        assert_eq!(clock.now(), 100);
        clock.advance(20);
        assert_eq!(clone.now(), 120);
        clone.set(7);
        assert_eq!(clock.now(), 7);
    }

    #[test]
    fn the_system_clock_is_now() {
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let now = Clock::system().now();
        assert!((before..before + 5).contains(&now));
    }

    #[test]
    #[should_panic(expected = "only a fixed clock is set")]
    fn the_system_clock_is_not_set() {
        Clock::system().advance(1);
    }
}
