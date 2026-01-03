use crate::chan::{Chan, Channel};
use crate::chan::{WaitChan, WaitChannel, Waiting};
use crate::filter::{FilterPub, FilterSub};
use crate::handler::Handler;
use crate::pubsub::{Publisher, Subscriber};
use futures::future::LocalBoxFuture;
use std::cell::RefCell;
use std::marker::PhantomData;

/// Event publisher trait that is dyn-compatible.
///
/// This trait is only useful when you need a
/// dynamic version of [`crate::Publisher`]. Many
/// methods need boxing operation to be dynamic
/// dispatchable, and doing so will inevitably
/// introduce overhead. Think twice before doing so.
pub trait PublisherDyn<E> {
    /// Invokes [`crate::Publisher::publish`].
    fn publish(&self, e: E) -> LocalBoxFuture<'_, ()>;

    /// Invokes [`crate::Publisher::take_publish`].
    fn take_publish(self, e: E) -> LocalBoxFuture<'static, ()>;

    /// Invokes [`crate::Publisher::has_subscriber`].
    fn has_subscriber(&self) -> bool;
}

struct PublisherHolder<E, T>
where
    E: Clone + 'static,
    T: Publisher<E>,
{
    publisher: T,
    _event: PhantomData<E>,
}

impl<E, T> PublisherDyn<E> for PublisherHolder<E, T>
where
    E: Clone + 'static,
    T: Publisher<E>,
{
    fn publish(&self, e: E) -> LocalBoxFuture<'_, ()> {
        Box::pin(self.publisher.publish(e))
    }

    fn take_publish(self, e: E) -> LocalBoxFuture<'static, ()> {
        Box::pin(self.publisher.take_publish(e))
    }

    fn has_subscriber(&self) -> bool {
        self.publisher.has_subscriber()
    }
}

/// Event publisher trait extension.
///
/// This trait extension provides methods for playing
/// with an `impl Publisher` instance easily. It
/// gather other components from this crate to provide
/// a user friendly interface.
///
/// Since `Publisher` is free to clone, many
/// interfaces takes the publisher directly to avoid
/// unnecessary cloning everywhere.
pub trait PublisherExt<E>: Publisher<E>
where
    E: Clone + 'static,
{
    /// Stack an async mapping and filtering function.
    fn map_filter_from_async<D, F>(self, f: F) -> impl Publisher<D>
    where
        D: Clone + 'static,
        F: AsyncFn(D) -> Option<E> + 'static,
    {
        FilterPub::new(self, f)
    }

    /// Stack a mapping and filtering function.
    fn map_filter_from<D, F>(self, f: F) -> impl Publisher<D>
    where
        D: Clone + 'static,
        F: FnMut(D) -> Option<E> + 'static,
    {
        let f = RefCell::new(f);
        self.map_filter_from_async(async move |d| (f.borrow_mut())(d))
    }

    // Stack an async mapping function.
    fn map_from_async<D, F>(self, f: F) -> impl Publisher<D>
    where
        D: Clone + 'static,
        F: AsyncFn(D) -> E + 'static,
    {
        self.map_filter_from_async(async move |d| Some(f(d).await))
    }

    // Stack a mapping function.
    fn map_from<D, F>(self, f: F) -> impl Publisher<D>
    where
        D: Clone + 'static,
        F: FnMut(D) -> E + 'static,
    {
        let f = RefCell::new(f);
        self.map_filter_from(move |d| Some((f.borrow_mut())(d)))
    }

    /// Stack an async filter function.
    fn filter_from_async<F>(self, f: F) -> impl Publisher<E>
    where
        F: AsyncFn(&E) -> bool + 'static,
    {
        self.map_filter_from_async(async move |e| if f(&e).await { Some(e) } else { None })
    }

    /// Stack a filter function.
    fn filter_from<F>(self, f: F) -> impl Publisher<E>
    where
        F: FnMut(&E) -> bool + 'static,
    {
        let f = RefCell::new(f);
        self.map_filter_from(move |e| if (f.borrow_mut())(&e) { Some(e) } else { None })
    }

    /// Box this publisher for dynamic dispatch.
    fn boxed(self) -> Box<dyn PublisherDyn<E>> {
        Box::new(PublisherHolder {
            publisher: self,
            _event: PhantomData,
        })
    }
}

impl<E, T> PublisherExt<E> for T
where
    T: Publisher<E>,
    E: Clone + 'static,
{
}

/// Event subscription trait to be dyn-compatible.
pub trait LedgeDyn {}

struct LedgeHolder<E>
where
    E: 'static,
{
    _holder: E,
}

impl<E> LedgeDyn for LedgeHolder<E> where E: 'static {}

/// Event subscriber trait that is dyn-compatible.
///
/// This trait is only useful when you need a
/// dynamic version of [`crate::Subscriber`]. Many
/// methods need boxing operation to be dynamic
/// dispatchable, and doing so will inevitably
/// introduce overhead. Think twice before doing so.
pub trait SubscriberDyn<E> {
    /// Invokes [`crate::Subscriber::subscribe`].
    fn subscribe(&self, handler: Handler<E>) -> LocalBoxFuture<'_, Box<dyn LedgeDyn>>;
}

struct SubscriberHolder<E, T>
where
    E: Clone + 'static,
    T: Subscriber<E>,
{
    subscriber: T,
    _event: PhantomData<E>,
}

impl<E, T> SubscriberDyn<E> for SubscriberHolder<E, T>
where
    E: Clone + 'static,
    T: Subscriber<E>,
{
    fn subscribe(&self, handler: Handler<E>) -> LocalBoxFuture<'_, Box<dyn LedgeDyn>> {
        Box::pin(async move {
            let result: Box<dyn LedgeDyn> = Box::new(LedgeHolder {
                _holder: self.subscriber.subscribe(handler).await,
            });
            result
        })
    }
}

struct ChanLedge<E, S>
where
    E: Clone + 'static,
    S: 'static,
{
    chan: Chan<E>,
    _subscription: S,
}

impl<E, S> Channel<E> for ChanLedge<E, S>
where
    E: Clone + 'static,
    S: 'static,
{
    async fn recv(&mut self) -> E {
        self.chan.recv().await
    }

    fn ready_wait(&self) -> Box<dyn crate::ReadyWait> {
        self.chan.ready_wait()
    }
}

struct WaitChanLedge<E, S>
where
    E: Clone + 'static,
    S: 'static,
{
    wait_chan: WaitChan<E>,
    _subscription: S,
}

impl<E, S> WaitChannel<E> for WaitChanLedge<E, S>
where
    E: Clone + 'static,
    S: 'static,
{
    type Waiting = Waiting<E>;

    async fn recv(&mut self) -> Self::Waiting {
        self.wait_chan.recv().await
    }

    fn ready_wait(&self) -> Box<dyn crate::ReadyWait> {
        self.wait_chan.ready_wait()
    }
}

/// Event subscriber trait extension.
///
/// This trait extension provides methods for playing
/// with an `impl Subscriber` instance easily. It
/// gather other components from this crate to provide
/// a user friendly interface.
///
/// Since `Subscriber` is free to clone, many
/// interfaces takes the subscriber directly to avoid
/// unnecessary cloning everywhere.
#[allow(async_fn_in_trait)]
pub trait SubscriberExt<E>: Subscriber<E>
where
    E: Clone + 'static,
{
    /// Stack an async mapping and filtering function.
    fn map_filter_async<D, F>(self, f: F) -> impl Subscriber<D>
    where
        D: Clone + 'static,
        F: AsyncFn(E) -> Option<D> + 'static,
    {
        FilterSub::new(self.clone(), f)
    }

    /// Stack a mapping and filtering function.
    fn map_filter<D, F>(self, f: F) -> impl Subscriber<D>
    where
        D: Clone + 'static,
        F: FnMut(E) -> Option<D> + 'static,
    {
        let f = RefCell::new(f);
        self.map_filter_async(async move |d| (f.borrow_mut())(d))
    }

    // Stack an async mapping function.
    fn map_async<D, F>(self, f: F) -> impl Subscriber<D>
    where
        D: Clone + 'static,
        F: AsyncFn(E) -> D + 'static,
    {
        self.map_filter_async(async move |d| Some(f(d).await))
    }

    // Stack a mapping function.
    fn map<D, F>(self, f: F) -> impl Subscriber<D>
    where
        D: Clone + 'static,
        F: FnMut(E) -> D + 'static,
    {
        let f = RefCell::new(f);
        self.map_filter(move |d| Some((f.borrow_mut())(d)))
    }

    /// Stack an async filter function.
    fn filter_async<F>(self, f: F) -> impl Subscriber<E>
    where
        F: AsyncFn(&E) -> bool + 'static,
    {
        self.map_filter_async(async move |e| if f(&e).await { Some(e) } else { None })
    }

    /// Stack a filter function.
    fn filter<F>(self, f: F) -> impl Subscriber<E>
    where
        F: FnMut(&E) -> bool + 'static,
    {
        let f = RefCell::new(f);
        self.map_filter(move |e| if (f.borrow_mut())(&e) { Some(e) } else { None })
    }

    /// Box this subscriber for dynamic dispatch.
    fn boxed(self) -> Box<dyn SubscriberDyn<E>> {
        Box::new(SubscriberHolder {
            subscriber: self,
            _event: PhantomData,
        })
    }

    /// Return a channel dedicated to this subscriber.
    async fn chan(self) -> impl Channel<E> {
        let mut chan = Chan::new();
        let subscription = chan.connect(self).await;
        ChanLedge {
            chan,
            _subscription: subscription,
        }
    }

    /// Return a waiting channel dedicated to this subscriber.
    async fn wait_chan(self) -> impl WaitChannel<E> {
        let mut wait_chan = WaitChan::new();
        let subscription = wait_chan.connect(self).await;
        WaitChanLedge {
            wait_chan,
            _subscription: subscription,
        }
    }
}

impl<E, T> SubscriberExt<E> for T
where
    T: Subscriber<E>,
    E: Clone + 'static,
{
}
