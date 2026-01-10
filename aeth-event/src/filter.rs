//! Map and filter adapter of event publisher and subscriber.
//!
//! Normally one should not use this package directly,
//! but instead the `map_from`, `filter_from` or
//! `map_filter_from` methods of publisher trait
//! extension [`crate::PublisherExt`], and the `map`,
//! `filter` and `map_filter` methods
//! of subscriber trait extension [`crate::SubscriberExt`].

use crate::handler::Handler;
use crate::pubsub::{Ledge, Sub, new_pubsub};
use crate::pubsub::{Publisher, Subscriber};
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::{Rc, Weak};

/// Filtered event publisher handle.
///
/// For any event `d: D` passed in, it will
/// go through an event filter `D -> Option<E>`.
/// If the filter returns `Some(e: E)`, then
/// `e` will be published to the underlying
/// publisher, otherwise (if `None` is
/// returned) `d` will be discard.
pub struct FilterPub<D, E, F, P>
where
    D: Clone + 'static,
    E: Clone + 'static,
    F: AsyncFn(D) -> Option<E> + 'static,
    P: Publisher<E>,
{
    publisher: P,
    filter: Rc<F>,
    _source: PhantomData<D>,
}

impl<D, E, F, P> FilterPub<D, E, F, P>
where
    D: Clone + 'static,
    E: Clone + 'static,
    F: AsyncFn(D) -> Option<E> + 'static,
    P: Publisher<E>,
{
    pub fn new(publisher: P, filter: F) -> Self {
        Self {
            publisher,
            filter: Rc::new(filter),
            _source: PhantomData,
        }
    }

    async fn publish_option(&self, d: D) -> Option<()> {
        let e = (self.filter)(d).await?;
        self.publisher.publish(e).await;
        Some(())
    }
}

impl<D, E, F, P> Clone for FilterPub<D, E, F, P>
where
    D: Clone + 'static,
    E: Clone + 'static,
    F: AsyncFn(D) -> Option<E> + 'static,
    P: Publisher<E>,
{
    fn clone(&self) -> Self {
        Self {
            publisher: self.publisher.clone(),
            filter: self.filter.clone(),
            _source: PhantomData,
        }
    }
}

impl<D, E, F, P> Publisher<D> for FilterPub<D, E, F, P>
where
    D: Clone + 'static,
    E: Clone + 'static,
    P: Publisher<E>,
    F: AsyncFn(D) -> Option<E> + 'static,
{
    async fn publish(&self, d: D) {
        self.publish_option(d).await;
    }

    fn has_subscriber(&self) -> bool {
        self.publisher.has_subscriber()
    }
}

struct FilterSubProxy<D, E, S>
where
    D: Clone + 'static,
    E: Clone + 'static,
    S: Subscriber<D>,
{
    _ledge: S::Subscription,
    sub: Sub<E>,
}

/// Filtered event subscription handle.
pub struct FilterLedge<D, E, S>
where
    D: Clone + 'static,
    E: Clone + 'static,
    S: Subscriber<D>,
{
    _proxy: Rc<FilterSubProxy<D, E, S>>,
    _ledge: Ledge<E>,
}

struct FilterSubInner<D, E, F, S>
where
    D: Clone + 'static,
    E: Clone + 'static,
    F: AsyncFn(D) -> Option<E> + 'static,
    S: Subscriber<D>,
{
    subscriber: S,
    filter: Rc<F>,
    proxy: RefCell<Weak<FilterSubProxy<D, E, S>>>,
    _source: PhantomData<D>,
}

impl<D, E, F, S> FilterSubInner<D, E, F, S>
where
    D: Clone + 'static,
    E: Clone + 'static,
    F: AsyncFn(D) -> Option<E> + 'static,
    S: Subscriber<D>,
{
    async fn create_or_reuse_proxy(&self) -> Rc<FilterSubProxy<D, E, S>> {
        let rc = self.proxy.borrow().upgrade();
        match rc {
            Some(result) => result,
            None => {
                let (filter_pub, filter_sub) = new_pubsub();
                let filter = self.filter.clone();
                let ledge = self
                    .subscriber
                    .subscribe(Handler::new_async_option(async move |d: D| {
                        let e = (filter)(d).await?;
                        filter_pub.publish(e).await;
                        Some(())
                    }))
                    .await;
                let proxy = Rc::new(FilterSubProxy {
                    _ledge: ledge,
                    sub: filter_sub,
                });
                *self.proxy.borrow_mut() = Rc::downgrade(&proxy);
                proxy
            }
        }
    }
}

/// Filtered event subscriber handle.
///
/// For any event `d: D` from the upstream,
/// it will go through an event filter
/// `D -> Option<E>`. If the filter returns
/// `Some(e: E)`, then `e` will be dispatched
/// to the underlying handlers, otherwise (if
/// `None` is returned) `d` will be discard.
pub struct FilterSub<D, E, F, S>
where
    D: Clone + 'static,
    E: Clone + 'static,
    F: AsyncFn(D) -> Option<E> + 'static,
    S: Subscriber<D>,
{
    inner: Rc<FilterSubInner<D, E, F, S>>,
}

impl<D, E, F, S> FilterSub<D, E, F, S>
where
    D: Clone + 'static,
    E: Clone + 'static,
    F: AsyncFn(D) -> Option<E> + 'static,
    S: Subscriber<D>,
{
    pub fn new(subscriber: S, filter: F) -> Self {
        let inner = Rc::new(FilterSubInner {
            subscriber,
            filter: Rc::new(filter),
            proxy: RefCell::new(Weak::new()),
            _source: PhantomData,
        });
        Self { inner }
    }
}

impl<D, E, F, S> Clone for FilterSub<D, E, F, S>
where
    D: Clone + 'static,
    E: Clone + 'static,
    F: AsyncFn(D) -> Option<E> + 'static,
    S: Subscriber<D>,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<D, E, F, S> Subscriber<E> for FilterSub<D, E, F, S>
where
    D: Clone + 'static,
    E: Clone + 'static,
    F: AsyncFn(D) -> Option<E> + 'static,
    S: Subscriber<D>,
{
    type Subscription = FilterLedge<D, E, S>;

    async fn subscribe(&self, handler: Handler<E>) -> Self::Subscription {
        let proxy = self.inner.create_or_reuse_proxy().await;
        let ledge = proxy.sub.subscribe(handler).await;
        Self::Subscription {
            _proxy: proxy,
            _ledge: ledge,
        }
    }
}
