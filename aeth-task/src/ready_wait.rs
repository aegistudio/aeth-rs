//! Make futures ready-waitable.
//!
//! This is used when you have a future, and
//! you want to receive an event (so that you
//! can use `aeth_event` primitives) instead
//! of [`futures::select`]-ing it.
//!
//! This will cause the task framework to
//! spawn a coroutine to poll it for you,
//! and send an event when it is ready.
//!
//! Multiple ready waitable futures are
//! possible to race, you must resort to
//! [`futures::select`] when you must
//! retain the semantic of not performing
//! action unless [`Future::poll`] returns
//! [`Poll::Ready`]. However, we provide
//! a graceful cancel interface
//! [`ReadyWaitFuture::cancel`] to make
//! the `ReadyWait` associated with this
//! future from [`ready_wait`] signal
//! readiness immediately, then
//! the [`ReadyWaitFuture`] can be polled.
//! It returns  `Some(T)` if the future
//! has been polled to [`Poll::Ready`],
//! or `None` if it is cancelled.

use crate::{Handle, foreground};
use aeth_event::prelude::*;
use aeth_event::{Pub, ReadyWait, Sub, new_pubsub};
use futures::FutureExt;
use futures::channel::oneshot::Sender;
use futures::channel::oneshot::channel as oneshot;
use futures::future::LocalBoxFuture;
use lives::{Life, LifeRc, LifeWeak};
use std::cell::{Cell, RefCell};
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::task::{Context, Poll};

// Trait of capabaility to be polled from another coroutine.
trait RemotePoll {
    fn poll(&self, cx: &mut Context) -> Poll<()>;
}

#[derive(Life)]
struct BoxRemotePoll<'a>(Box<dyn RemotePoll + 'a>);

struct LifeWeakBoxRemotePoll<'a>(LifeWeak<BoxRemotePoll<'a>>);

impl<'a> Future for LifeWeakBoxRemotePoll<'a> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.0
            .with(|boxed| boxed.0.poll(cx))
            .unwrap_or(Poll::Pending)
    }
}

struct RemotePollInner<'a, T: 'a> {
    future: LocalBoxFuture<'a, T>,
    result: Option<T>,
    done: Rc<Cell<bool>>,
}

struct RemotePollOuter<'a, T: 'a> {
    inner: Rc<RefCell<RemotePollInner<'a, T>>>,
}

impl<'a, T> RemotePoll for RemotePollOuter<'a, T> {
    fn poll(&self, cx: &mut Context) -> Poll<()> {
        let mut inner = self.inner.borrow_mut();
        if inner.result.is_none() {
            if let Poll::Ready(result) = inner.future.poll_unpin(cx) {
                inner.result = Some(result);
                inner.done.replace(true);
            }
        }
        if inner.result.is_some() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

/// Local future wrapper to be ready-waitable.
///
/// This future works only on foreground
/// context, I ellide the "Local" for
/// convenience. It spawns another foreground
/// coroutine to poll the wrapped future
/// and to notify the boxed
/// [`aeth_event::ReadyWait`] when the
/// wrapped future is polled to ready.
/// You can also use the
/// [`ReadyWaitFuture::cancel`] or
/// [`ReadyWaitFuture::cancel_take`] to
/// cancel the future.
///
/// When a future is ready or canceled by
/// [`ReadyWaitFuture::cancel`], the
/// multiplexer will report the future
/// to be ready, only then the future
/// can be awaited. Upon awaited, it
/// returns `Some(T)` if the future
/// future has been polled to ready,
/// otherwise it returns `None`. The
/// wrapped future is guaranteed not to
/// polled to ready if it returns `None`.
///
/// If you drop the future or use
/// [`ReadyWaitFuture::cancel_take`] to
/// cancel the future, the future is
/// canceled but the multiplexer **will
/// never** report this future to be ready.
///
/// # Panics
///
/// You **must not** await the future
/// unless it has been reported to be
/// ready by the multiplexer's
/// [`aeth_event::Mux::poll`] which
/// you register the ready wait to.
/// Failing to fulfil is the wrong usage
/// and will result in panics.
pub struct ReadyWaitFuture<'a, T> {
    _life_rc: LifeRc<BoxRemotePoll<'a>>,
    canceled: Rc<Cell<bool>>,
    inner: Rc<RefCell<RemotePollInner<'a, T>>>,
    poller: Option<Box<dyn Handle<Option<()>>>>,
    ready_pub: Pub<()>,
}

impl<'a, T> ReadyWaitFuture<'a, T> {
    fn poll_inner(&self) -> Option<T> {
        let mut inner = self.inner.borrow_mut();
        if inner.done.get() {
            if inner.result.is_none() {
                panic!("Cannot poll this future to be ready twice.")
            }
            return inner.result.take();
        }
        if self.poller.is_none() {
            return None;
        }
        panic!("Must witness the readiness signal before polling the future.");
    }
}

impl<'a, T> Future for ReadyWaitFuture<'a, T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(self.poll_inner())
    }
}

struct FutureReadyWait {
    done: Weak<Cell<bool>>,
    canceled: Weak<Cell<bool>>,
    ready_sub: Sub<()>,
    begin_send: RefCell<Option<Sender<()>>>,
}

impl FutureReadyWait {
    fn ready_option(&self) -> Option<bool> {
        let done = self.done.upgrade()?;
        let canceled = self.canceled.upgrade()?;
        Some(done.get() || canceled.get())
    }
}

impl ReadyWait for FutureReadyWait {
    fn ready(&self) -> bool {
        self.ready_option().unwrap_or(false)
    }

    fn waiter(&self) -> Sub<()> {
        self.ready_sub.clone()
    }

    fn notify_subscribed(&self) {
        let _ = self.begin_send.borrow_mut().take().unwrap().send(());
    }
}

impl<'a, T> ReadyWaitFuture<'a, T> {
    fn cancel_unnotified(&mut self) {
        std::mem::drop(self.poller.take());
        self.canceled.replace(true);
    }

    /// Gracefully cancel the future.
    ///
    /// An ungraceful cancel is done by
    /// simply dropping [`ReadyWaitFuture`].
    /// In that case, it's possible for
    /// the wrapped future to be polled
    /// to ready, while it's output is
    /// discarded upon dropping.
    ///
    /// The future will be marked as ready
    /// immediately, and races between
    /// either the case that wrapped future
    /// has been polled to ready, then
    /// `Some(T)` will be returned when
    /// polling [`ReadyWaitFuture`],
    /// otherwise `None` will be returned.
    pub fn cancel(&mut self) {
        self.cancel_unnotified();
        let ready_pub = self.ready_pub.clone();
        foreground::spawn(ready_pub.take_publish(())).detach();
    }

    /// Gracefully cancel the future and
    /// take the race result.
    ///
    /// This can be viewed as cancelling
    /// the future and polling it
    /// immediately in the same place,
    /// except for not being async.
    ///
    /// Using [`ReadyWaitFuture::cancel`]
    /// or
    /// [`ReadyWaitFuture::cancel_take`]
    /// is just a preference of style:
    /// Whether you want to process it
    /// in the arm corresponding to
    /// this future, or the place you
    /// gracefully cancel this future?
    ///
    /// The [`ReadyWait`] of this future
    /// will no longer be ready if you
    /// attempt to do so.
    pub fn cancel_take(mut self) -> Option<T> {
        self.cancel_unnotified();
        self.poll_inner()
    }

    /// Consumes self and return a
    /// non-cancelling future.
    /// 
    /// This is useful when you don't need
    /// to care about the racing result of
    /// wrapped futures polled to ready
    /// versus cancellation. It simply
    /// awaits the future and unwraps it.
    pub async fn no_cancel(self) -> T {
        self.await.unwrap()
    }
}

/// Wraps the local future to be ready-waitable.
pub fn ready_wait<'a, T, F>(future: F) -> (ReadyWaitFuture<'a, T>, Box<dyn ReadyWait>)
where
    F: Future<Output = T> + 'a,
    T: 'a,
{
    let done = Rc::new(Cell::new(false));
    let canceled = Rc::new(Cell::new(false));
    let remote = Rc::new(RefCell::new(RemotePollInner {
        future: Box::pin(future),
        result: None,
        done: done.clone(),
    }));
    let boxed = BoxRemotePoll(Box::new(RemotePollOuter {
        inner: remote.clone(),
    }));
    let life_rc = LifeRc::new(boxed);
    let life_weak = LifeRc::downgrade(&life_rc);
    let (ready_pub, ready_sub) = new_pubsub();
    let ready_pub1 = ready_pub.clone();
    let (begin_send, begin_recv) = oneshot();
    let poller = Box::new(foreground::spawn(async move {
        begin_recv.await.ok()?;
        let polling = LifeWeakBoxRemotePoll(life_weak);
        let polling = Box::pin(polling);
        polling.await;
        ready_pub1.take_publish(()).await;
        Some(())
    }));
    let ready_wait = Box::new(FutureReadyWait {
        done: Rc::downgrade(&done),
        canceled: Rc::downgrade(&canceled),
        ready_sub: ready_sub,
        begin_send: RefCell::new(Some(begin_send)),
    });
    let wrapped = ReadyWaitFuture {
        _life_rc: life_rc,
        canceled,
        inner: remote,
        poller: Some(poller),
        ready_pub,
    };
    (wrapped, ready_wait)
}
