//! The event channel module.
//!
//! Most types in this module have been
//! exported to the crate root, however
//! some types are not exported to avoid
//! confusing the users, and they will
//! be held here instead.

use crate::{Handler, Subscriber};
use aeth_mux::{Multiplexer, Muxing};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::channel::oneshot::Sender as OneshotSender;
use futures::channel::oneshot::channel as oneshot;
use futures::{SinkExt, Stream, StreamExt, ready};
use pin_project_lite::pin_project;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

/// Event channel trait.
///
/// This trait generalizes the the
/// receiving behavior of base channel
/// [`Chan`] and its adapters.
///
/// The [`Chan`] and its adapters in
/// this crate are all [`Unpin`]. To
/// keep it simple, we force the channel
/// trait to be unpinned. Custom
/// implementors must resolve the pinning
/// issues internally.
///
/// There're mainly three ways of using this
/// event channel `chan`:
///
/// 1. One can simply wait for the event by
///    [`chan.next().await`](ChannelExt::next).
/// 2. One can convert the channel into a stream by
///    [`chan.into_stream()`](ChannelExt::into_stream).
/// 3. One can multiplex it into a multiplexer
///    [`mux`](aeth_mux::Mux) with some `key` by
///    [`mux.mux_chan(key, chan)`](MuxChanExt::mux_chan).
pub trait Channel<E>: Unpin
where
    E: Clone + 'static,
{
    /// Fetch the next ready event.
    fn poll_next_event(&mut self, cx: &mut Context<'_>) -> Poll<E>;
}

pin_project! {
    struct ChannelStream<E, C> {
        chan: C,
        _phantom: PhantomData<E>,
    }
}

impl<E, C> Stream for ChannelStream<E, C>
where
    E: Clone + 'static,
    C: Channel<E>,
{
    type Item = E;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        Poll::Ready(Some(ready!(this.chan.poll_next_event(cx))))
    }
}

/// Event channel trait extension.
///
/// This extension aims at providing
/// some useful interfaces to deal
/// with the [`Channel`] trait.
pub trait ChannelExt<E>: Channel<E>
where
    E: Clone + 'static,
{
    /// Fetch the next event in channel.
    fn next(&mut self) -> impl Future<Output = E> {
        std::future::poll_fn(move |cx| self.poll_next_event(cx))
    }

    /// Turn the channel into a stream
    /// of event objects.
    ///
    /// Please notice this stream is
    /// never ending, so it's safe to
    /// invoke `next().await.unwrap()`.
    fn into_stream(self) -> impl Stream<Item = E>
    where
        Self: Sized,
    {
        ChannelStream {
            chan: self,
            _phantom: PhantomData,
        }
    }
}

impl<E, T> ChannelExt<E> for T
where
    E: Clone + 'static,
    T: Channel<E>,
{
}

/// The muxing event channel.
///
/// This slot is obtained by wrapping
/// a channel with a [`aeth_mux::Muxing`],
/// so that it can be multiplexed into
/// the corresponding [`aeth_mux::Mux`].
pub struct MuxingChan<E, C, M>
where
    E: Clone + 'static,
    C: Channel<E>,
    M: Muxing,
{
    chan: C,
    muxing: M,
    waker: Waker,
    _phantom: PhantomData<E>,
}

impl<E, C, M> MuxingChan<E, C, M>
where
    E: Clone + 'static,
    C: Channel<E>,
    M: Muxing,
{
    /// Try to poll the next event.
    ///
    /// One should generally cope with
    /// [`aeth_mux::try_poll`].
    pub fn try_poll(&mut self) -> Option<E> {
        self.muxing.acknowledge()?;

        let mut cx = Context::from_waker(&self.waker);
        match self.chan.poll_next_event(&mut cx) {
            Poll::Ready(ready) => {
                self.waker.wake_by_ref();
                Some(ready)
            }
            Poll::Pending => None,
        }
    }

    /// Consume this muxing channel and
    /// take the internal channel out.
    pub fn take(self) -> C {
        self.chan
    }

    /// Borrow the inner channel immutably.
    pub fn chan(&self) -> &C {
        &self.chan
    }

    /// Borrow the inner channel mutably.
    pub fn chan_mut(&mut self) -> &mut C {
        &mut self.chan
    }
}

/// Event channel with back-pressure trait.
///
/// This trait generalizes the the
/// receiving behavior of base channel
/// [`WaitChan`] and its adapters.
///
/// The [`WaitChan`] and its adapters
/// in this crate are all [`Unpin`]. To
/// keep it simple, we force the wait
/// channel trait to be unpinned. Custom
/// implementors must resolve the
/// pinning issue internally.
///
/// There're mainly three ways of using this
/// event channel with guard `wait_chan`:
///
/// 1. One can simply wait for the event by
///    [`wait_chan.next().await`](WaitChannelExt::next).
/// 2. One can convert the channel into a stream by
///    [`wait_chan.into_stream()`](WaitChannelExt::into_stream).
/// 3. One can multiplex it into a multiplexer
///    [`mux`](aeth_mux::Mux) with some `key` by
///    [`mux.mux_wait_chan(key, wait_chan)`](MuxChanExt::mux_wait_chan).
pub trait WaitChannel<E>: Unpin
where
    E: Clone + 'static,
{
    type Waiting: Deref<Target = E> + DerefMut + 'static;

    /// Fetch the next event with guard.
    fn poll_next_event(&mut self, cx: &mut Context<'_>) -> Poll<Self::Waiting>;
}

pin_project! {
    struct WaitChannelStream<E, W> {
        wait_chan: W,
        _phamtom: PhantomData<E>
    }
}

impl<E, W> Stream for WaitChannelStream<E, W>
where
    E: Clone + 'static,
    W: WaitChannel<E>,
{
    type Item = W::Waiting;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        Poll::Ready(Some(ready!(this.wait_chan.poll_next_event(cx))))
    }
}

/// Event channel with back-pressure
/// trait extension.
///
/// This extension aims at providing
/// some useful interfaces to deal
/// with the [`WaitChannel`] trait.
pub trait WaitChannelExt<E>: WaitChannel<E>
where
    E: Clone + 'static,
{
    /// Fetch the next event with guard.
    fn next(&mut self) -> impl Future<Output = Self::Waiting> {
        std::future::poll_fn(move |cx| self.poll_next_event(cx))
    }

    /// Turn the channel into a stream
    /// of events with guards.
    ///
    /// Please notice this stream is
    /// never ending, so it's safe to
    /// invoke `next().await.unwrap()`.
    fn into_stream(self) -> impl Stream<Item = Self::Waiting>
    where
        Self: Sized,
    {
        WaitChannelStream {
            wait_chan: self,
            _phamtom: PhantomData,
        }
    }
}

impl<E, T> WaitChannelExt<E> for T
where
    E: Clone + 'static,
    T: WaitChannel<E>,
{
}

/// The muxing event channel
/// with back-pressure.
///
/// This slot is obtained by wrapping
/// a channel with a [`aeth_mux::Muxing`],
/// so that it can be multiplexed into
/// the corresponding [`aeth_mux::Mux`].
pub struct MuxingWaitChan<E, W, M>
where
    E: Clone + 'static,
    W: WaitChannel<E>,
    M: Muxing,
{
    wait_chan: W,
    muxing: M,
    waker: Waker,
    _phantom: PhantomData<E>,
}

impl<E, W, M> MuxingWaitChan<E, W, M>
where
    E: Clone + 'static,
    W: WaitChannel<E>,
    M: Muxing,
{
    /// Try to poll the next event with guard.
    ///
    /// One should generally cope with
    /// [`aeth_mux::try_poll`].
    pub fn try_poll(&mut self) -> Option<W::Waiting> {
        self.muxing.acknowledge()?;

        let mut cx = Context::from_waker(&self.waker);
        match self.wait_chan.poll_next_event(&mut cx) {
            Poll::Ready(ready) => {
                self.waker.wake_by_ref();
                Some(ready)
            }
            Poll::Pending => None,
        }
    }

    /// Consume this muxing wait channel and
    /// take the internal wait channel out.
    pub fn take(self) -> W {
        self.wait_chan
    }

    /// Borrow the inner wait channel immutably.
    pub fn wait_chan(&self) -> &W {
        &self.wait_chan
    }

    /// Borrow the inner wait channel mutably.
    pub fn wait_chan_mut(&mut self) -> &mut W {
        &mut self.wait_chan
    }
}

/// Trait extension to multiplex
/// [`Channel`] and [`WaitChannel`].
pub trait MuxChanExt<K>: Multiplexer<K> {
    /// Multiplex an event channel.
    fn mux_chan<E, C>(&mut self, key: K, chan: C) -> MuxingChan<E, C, Self::Muxing>
    where
        E: Clone + 'static,
        C: Channel<E>,
    {
        let (muxing, waker) = self.multiplex(key);
        waker.wake_by_ref();
        MuxingChan {
            chan,
            muxing,
            waker,
            _phantom: PhantomData,
        }
    }

    /// Multiplex an event channel with backpressure.
    fn mux_wait_chan<E, W>(&mut self, key: K, wait_chan: W) -> MuxingWaitChan<E, W, Self::Muxing>
    where
        E: Clone + 'static,
        W: WaitChannel<E>,
    {
        let (muxing, waker) = self.multiplex(key);
        waker.wake_by_ref();
        MuxingWaitChan {
            wait_chan,
            muxing,
            waker,
            _phantom: PhantomData,
        }
    }
}

impl<K, T> MuxChanExt<K> for T where T: Multiplexer<K> {}

/// Event channel.
///
/// This object recovers the event handling logic into a
/// channel polling logic. Now we do event processing in
/// rust async language.
///
/// There're mainly three ways of using this
/// event channel `chan`:
///
/// 1. One can simply wait for the event by
///    [`chan.next().await`](ChannelExt::next).
/// 2. One can convert the channel into a stream by
///    [`chan.into_stream()`](ChannelExt::into_stream).
/// 3. One can multiplex it into a multiplexer
///    [`mux`](aeth_mux::Mux) with some `key` by
///    [`mux.mux_chan(key, chan)`](MuxChanExt::mux_chan).
pub struct Chan<E>
where
    E: Clone + 'static,
{
    send: UnboundedSender<E>,
    recv: UnboundedReceiver<E>,
}

impl<E> Chan<E>
where
    E: Clone + 'static,
{
    pub fn new() -> Self {
        let (send, recv) = unbounded();
        Self { send, recv }
    }

    #[must_use = "Unregister when Subscription is dropped."]
    pub async fn connect<S: Subscriber<E>>(&mut self, sub: S) -> S::Subscription {
        let mut sender = self.send.clone();
        sub.subscribe(Handler::new_async(async move |item| {
            let _ = sender.send(item).await;
        }))
        .await
    }
}

impl<E> Channel<E> for Chan<E>
where
    E: Clone + 'static,
{
    fn poll_next_event(&mut self, cx: &mut Context<'_>) -> Poll<E> {
        // XXX: Yes, the channel must be a never ending stream.
        Poll::Ready(ready!(self.recv.poll_next_unpin(cx)).unwrap())
    }
}

/// Back-pressure guarded event.
///
/// This is returned by polling [`WaitChan`],
/// for blocking the event publisher until
/// the back-pressure guard is dropped.
pub struct Waiting<E> {
    event: E,
    done: Option<OneshotSender<()>>,
}

impl<E> Drop for Waiting<E> {
    fn drop(&mut self) {
        let _ = self.done.take().unwrap().send(());
    }
}

impl<E> Deref for Waiting<E>
where
    E: Clone + 'static,
{
    type Target = E;

    fn deref(&self) -> &Self::Target {
        &self.event
    }
}

impl<E> DerefMut for Waiting<E>
where
    E: Clone + 'static,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.event
    }
}

/// Event channel with back-pressure.
///
/// It's totally like [`Chan`],
/// except for events being protected in
/// [`Waiting`] barriers, which will
/// block the publisher until it's dropped.
///
/// There're mainly three ways of using this
/// event channel with guard `wait_chan`:
///
/// 1. One can simply wait for the event by
///    [`wait_chan.next().await`](WaitChannelExt::next).
/// 2. One can convert the channel into a stream by
///    [`wait_chan.into_stream()`](WaitChannelExt::into_stream).
/// 3. One can multiplex it into a multiplexer
///    [`mux`](aeth_mux::Mux) with some `key` by
///    [`mux.mux_wait_chan(key, wait_chan)`](MuxChanExt::mux_wait_chan).
pub struct WaitChan<E> {
    send: UnboundedSender<Waiting<E>>,
    recv: UnboundedReceiver<Waiting<E>>,
}

impl<E> WaitChan<E>
where
    E: Clone + 'static,
{
    pub fn new() -> Self {
        let (send, recv) = unbounded();
        Self { send, recv }
    }

    #[must_use = "Unregister when Subscription is dropped."]
    pub async fn connect<S: Subscriber<E>>(&mut self, sub: S) -> S::Subscription {
        let mut sender = self.send.clone();
        sub.subscribe(Handler::new_async_option(async move |event| {
            let (waiting_pub, waiting_sub) = oneshot();
            let item = Waiting {
                event,
                done: Some(waiting_pub),
            };
            sender.send(item).await.ok()?;
            let _ = waiting_sub.await;
            Some(())
        }))
        .await
    }
}

impl<E> WaitChannel<E> for WaitChan<E>
where
    E: Clone + 'static,
{
    type Waiting = Waiting<E>;

    fn poll_next_event(&mut self, cx: &mut Context<'_>) -> Poll<Self::Waiting> {
        // XXX: Again, never ending stream of events.
        Poll::Ready(ready!(self.recv.poll_next_unpin(cx)).unwrap())
    }
}

#[cfg(test)]
mod test {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::chan::{Chan, WaitChan};
    use crate::prelude::*;
    use crate::pubsub;
    use crate::testutil::TestFixture;
    use aeth_mux::prelude::*;
    use aeth_mux::{Mux, try_poll};

    #[test]
    fn test_normal() {
        let mut fixture = TestFixture::new();

        let (p1, s1) = pubsub::<()>();
        let (p2, s2) = pubsub::<usize>();
        let (p3, s3) = pubsub::<()>();

        let v1 = Rc::new(RefCell::new(0usize));
        let v2 = Rc::new(RefCell::new(0usize));
        let v3 = Rc::new(RefCell::new(0usize));

        let v1l = v1.clone();
        let v2l = v2.clone();
        let v3l = v3.clone();
        let _ = fixture.execute(async move {
            #[derive(Clone)]
            enum Branch {
                Ch1,
                Ch2,
                Ch3,
                Ch4,
            }
            let mut mux: Mux<Branch> = Mux::new();

            let mut ch1: Chan<()> = Chan::new();
            let _l1 = ch1.connect(s1).await;
            let mut ch1 = mux.mux_stream(Branch::Ch1, ch1.into_stream());

            let ch2: Chan<usize> = Chan::new();
            let mut ch2 = mux.mux_chan(Branch::Ch2, ch2);
            let _l2 = ch2.chan_mut().connect(s2.clone()).await;

            let mut ch3: WaitChan<usize> = WaitChan::new();
            let l3 = ch3.connect(s2.clone()).await;
            let mut l3 = Some(l3);
            let ch3 = mux.mux_wait_chan(Branch::Ch3, ch3);
            let mut ch3 = Some(ch3);

            let mut ch4: WaitChan<()> = WaitChan::new();
            let _l4 = ch4.connect(s3.clone()).await;
            let mut ch4 = mux.mux_stream(Branch::Ch4, ch4.into_stream());

            loop {
                match mux.poll().await {
                    Branch::Ch1 => {
                        try_poll!(ch1);
                        *v1l.borrow_mut() += 1;
                    }
                    Branch::Ch2 => {
                        let d = try_poll!(ch2);
                        *v2l.borrow_mut() += d;
                    }
                    Branch::Ch3 => {
                        let d = try_poll!(ch3.as_mut().unwrap());
                        *v3l.borrow_mut() += *d;
                    }
                    Branch::Ch4 => {
                        let _guard = try_poll!(ch4);
                        std::mem::drop(l3.take());
                        std::mem::drop(ch3.take());
                    }
                }
            }
        });

        let p1c = p1.clone();
        fixture
            .execute(async move { p1c.publish(()).await })
            .assert_done();
        assert_eq!(*v1.borrow(), 1);
        assert_eq!(*v2.borrow(), 0);
        assert_eq!(*v3.borrow(), 0);

        let p2c = p2.clone();
        fixture
            .execute(async move { p2c.publish(2).await })
            .assert_done();
        assert_eq!(*v1.borrow(), 1);
        assert_eq!(*v2.borrow(), 2);
        assert_eq!(*v3.borrow(), 2);

        let p3c = p3.clone();
        fixture
            .execute(async move { p3c.publish(()).await })
            .assert_done();
        assert_eq!(*v1.borrow(), 1);
        assert_eq!(*v2.borrow(), 2);
        assert_eq!(*v3.borrow(), 2);

        let p2c = p2.clone();
        fixture
            .execute(async move { p2c.publish(3).await })
            .assert_done();
        assert_eq!(*v1.borrow(), 1);
        assert_eq!(*v2.borrow(), 5);
        assert_eq!(*v3.borrow(), 2);
    }
}
