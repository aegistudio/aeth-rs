//! Make futures to be ready-pollable.
//!
//! This is suitable for the user who wants to
//! know whether a future has been polled to
//! ready by the async runtime. We provide a
//! mechanism of wrapping a future and returning
//! a poller to observe the readiness.
//!
//! This is not commonly required for a task
//! writer, which should rely on the async/await
//! primitives to observe the readines. So we
//! pick this module out of the crate root.

use futures::FutureExt;
use futures::future::{BoxFuture, LocalBoxFuture};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

#[derive(Clone)]
struct ReadyInner {
    // XXX: We pay the price of using atomic operations
    // for whichever kind of future, given that the
    // price of an atomic operation is not unaffordable
    // compared to other operations to be done in the
    // system. This also reduce the chance of misuse.
    ready: Arc<AtomicBool>,
}

impl ReadyInner {
    fn new() -> Self {
        Self {
            ready: Arc::new(AtomicBool::new(false)),
        }
    }

    fn ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    fn notify(&self) {
        self.ready.store(true, Ordering::SeqCst);
    }
}

/// Future wrapper to be ready-pollable.
///
/// This wrappr capture the lifecycle and polling of
/// the underlying future. Whenever the future is
/// destroyed or polled to ready, it will set the
/// ready flag so that the ready state of this
/// future can be tested later.
pub struct ReadyPollFuture<'a, T> {
    ready: ReadyInner,
    future: BoxFuture<'a, T>,
}

impl<'a, T> Drop for ReadyPollFuture<'a, T> {
    fn drop(&mut self) {
        // XXX: this is required when we are combining
        // the future with Remote, which may cancel
        // the future. The readiness will be
        // broadcasted when it's dropped.
        self.ready.notify();
    }
}

impl<'a, T> Future for ReadyPollFuture<'a, T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // XXX: for simplicity, we don't handle the
        // case when future.poll panics. This will be
        // handled by its caller, especially when
        // used with Remote, to drop ReadyPollFuture
        // and then notify readiness there.
        let result = self.future.poll_unpin(cx);
        if result.is_ready() {
            self.ready.notify();
        }
        result
    }
}

/// Ready poller object.
///
/// This is the tester side result of the ready
/// poll, providing `ready()` method to check if
/// the wrapped future is ready.
pub struct ReadyPoll {
    ready: ReadyInner,
}

impl ReadyPoll {
    pub fn ready(&self) -> bool {
        self.ready.ready()
    }
}

/// Wraps the future to be ready-pollable.
///
/// The future is thought to be ready when the
/// future is **polled to be ready** or destroyed.
/// It's not suitable for scenario to capture the
/// ready state without resuming its processing.
///
/// This is useful from outside the async runtime,
/// where no async context is available. This
/// wrapper smuggles the ready state of a specific
/// future out of the async runtime.
pub fn ready_poll<'a, T, F>(future: F) -> (ReadyPollFuture<'a, T>, ReadyPoll)
where
    F: Future<Output = T> + Send + 'a,
{
    let ready = ReadyInner::new();
    let ready1 = ready.clone();
    (
        ReadyPollFuture {
            ready,
            future: Box::pin(future),
        },
        ReadyPoll { ready: ready1 },
    )
}

/// Local future wrapper to be ready-pollable.
pub struct LocalReadyPollFuture<'a, T> {
    ready: ReadyInner,
    future: LocalBoxFuture<'a, T>,
}

impl<'a, T> Drop for LocalReadyPollFuture<'a, T> {
    fn drop(&mut self) {
        self.ready.notify();
    }
}

impl<'a, T> Future for LocalReadyPollFuture<'a, T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let result = self.future.poll_unpin(cx);
        if result.is_ready() {
            self.ready.notify();
        }
        result
    }
}

/// Wraps the local future to be ready pollable.
///
/// This is the local version of the `ready_poll`
/// counterpart. You will need this when your
/// future is `!Send`, which is allowed in the
/// case of a foreground thread.
///
/// This method also assumes that the future and
/// poller runs on the same thread, in which no
/// atomic operation is required.
pub fn local_ready_poll<'a, T, F>(future: F) -> (LocalReadyPollFuture<'a, T>, ReadyPoll)
where
    F: Future<Output = T> + 'a,
{
    let ready = ReadyInner::new();
    let ready1 = ready.clone();
    (
        LocalReadyPollFuture {
            ready,
            future: Box::pin(future),
        },
        ReadyPoll { ready: ready1 },
    )
}
