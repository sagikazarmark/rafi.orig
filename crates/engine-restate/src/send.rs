//! `Send` futures from libraries whose wasm32 futures aren't (`reqwest`
//! over `fetch`, for example), for interfaces and frameworks that require
//! `Send` (axum, the Restate SDK).

use std::future::Future;

/// The future, as `Send`.
///
/// On wasm32 without threads (Workers, browsers) every future can be: there
/// is only one thread to run it on. Everywhere else the future must already
/// be `Send`, and this returns it unchanged.
#[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
pub(crate) fn sendable<F: Future>(future: F) -> impl Future<Output = F::Output> + Send {
    one_thread::OneThread(future)
}

/// The future, as `Send`.
///
/// On wasm32 without threads (Workers, browsers) every future can be: there
/// is only one thread to run it on. Everywhere else the future must already
/// be `Send`, and this returns it unchanged.
#[cfg(not(all(target_arch = "wasm32", not(target_feature = "atomics"))))]
pub(crate) fn sendable<F: Future + Send>(future: F) -> impl Future<Output = F::Output> + Send {
    future
}

#[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
mod one_thread {
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };

    pub(super) struct OneThread<F>(pub(super) F);

    // SAFETY: this module only exists on wasm32 without the `atomics`
    // feature, where there is a single thread, so the future is never sent
    // or shared across threads.
    unsafe impl<F> Send for OneThread<F> {}

    impl<F: Future> Future for OneThread<F> {
        type Output = F::Output;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
            // SAFETY: a structural pin projection: the field is never moved
            // out, and `OneThread` has no `Drop` or `Unpin` impl of its own.
            unsafe { self.map_unchecked_mut(|s| &mut s.0) }.poll(cx)
        }
    }
}
