use crate::Handler;
use crate::pubsub::{Ledge, Sub};
use indexed_bitmap::IndexedBitmap;
use std::cell::RefCell;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::task::{Context, Poll, Waker};

/// Readiness event source trait.
///
/// The caller can check if the event source
/// is ready by polling the `ready` method,
/// and register itself for readiness
/// notification by `waiter().subscribe` method.
pub trait ReadyWait {
    fn ready(&self) -> bool;

    fn waiter(&self) -> Sub<()>;
}

struct Registry<V>
where
    V: Clone + 'static,
{
    wait: Box<dyn ReadyWait>,
    value: V,
    id: Rc<RefCell<usize>>,
}

struct Inner<V>
where
    V: Clone + 'static,
{
    ready: IndexedBitmap,
    regs: Vec<Registry<V>>,
    waker: Option<Waker>,
}

impl<V> Inner<V>
where
    V: Clone + 'static,
{
    fn evict(&mut self, id: usize) {
        assert!(self.regs.len() > 0);
        let last_id = self.regs.len() - 1;
        assert!(id <= last_id);
        if id < last_id {
            let last_ready = self.ready.bitget(last_id);
            self.ready.bitset(id, last_ready);
            self.regs.swap(id, last_id);
            *self.regs[id].id.borrow_mut() = id;
            *self.regs[last_id].id.borrow_mut() = last_id;
        }
        self.ready.bitset(last_id, false);
        self.regs.truncate(self.regs.len() - 1);
        if self.regs.capacity() >= self.regs.len() * 2 {
            self.regs.shrink_to_fit();
            self.ready.shrink_to(self.regs.capacity());
        }
    }

    fn notify(&mut self, id: usize) {
        self.ready.bitset(id, true);
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    async fn register(rc: Rc<RefCell<Self>>, wait: Box<dyn ReadyWait>, value: V) -> Muxing<V> {
        let mut inner = rc.borrow_mut();
        let slot = inner.regs.len();
        let waiter = wait.waiter();
        let id = Rc::new(RefCell::new(slot));
        let ready = wait.ready();
        inner.regs.push(Registry {
            wait: wait,
            value: value,
            id: id.clone(),
        });
        if ready {
            inner.notify(slot);
        }
        std::mem::drop(inner);
        // Must borrow weak, otherwise the Muxing owner will
        // prolong the Inner even after Mux is destroyed.
        let notify_weak = Rc::downgrade(&rc);
        let notify_id = id.clone();
        let ledge = waiter
            .subscribe(Handler::new_sync_option(move |_| {
                // XXX: since Pub<()>.publish and Sub<()>.subscribe
                // are linearized, it's impossible for this function
                // to be called in the middle of publishing.
                let rc = notify_weak.upgrade()?;
                let mut inner = rc.borrow_mut();
                let id = *notify_id.borrow();
                inner.notify(id);
                Some(())
            }))
            .await;
        Muxing {
            inner: Rc::downgrade(&rc),
            id: id,
            ledge: Some(ledge),
        }
    }
}

/// Mux registry handle.
///
/// Dropping this handle is equivalent to
/// unregistering the corresponding waiter.
pub struct Muxing<V>
where
    V: Clone + 'static,
{
    ledge: Option<Ledge<()>>,
    inner: Weak<RefCell<Inner<V>>>,
    id: Rc<RefCell<usize>>,
}

impl<V> Muxing<V>
where
    V: Clone + 'static,
{
    fn drop_option(&mut self) -> Option<()> {
        std::mem::drop(self.ledge.take());
        let inner = self.inner.upgrade()?;
        let id = *self.id.borrow();
        inner.borrow_mut().evict(id);
        Some(())
    }
}

impl<V> Drop for Muxing<V>
where
    V: Clone,
{
    fn drop(&mut self) {
        self.drop_option();
    }
}

/// Event multiplexer.
///
/// This object is capable of multiplexing the ready
/// events from multiple `ReadyWait` sources. To use,
/// the user first register by associating the
/// `ReadyWait` with a value of type `V`. Then whenever
/// the `ReadyWait` is ready, the `poll` method of the
/// multiplexer returns the previously registered value.
///
/// For convenience, the `poll` method works in
/// level-trigger mode.
pub struct Mux<V>
where
    V: Clone + 'static,
{
    inner: Rc<RefCell<Inner<V>>>,
}

impl<V> Mux<V>
where
    V: Clone,
{
    pub fn new() -> Self {
        let inner = Inner {
            ready: IndexedBitmap::new(),
            regs: Vec::new(),
            waker: None,
        };
        Self {
            inner: Rc::new(RefCell::new(inner)),
        }
    }

    /// Poll or wait for the ready `ReadyWait` source,
    /// and return the value associated with the source.
    pub fn poll(&mut self) -> MuxPollFuture<'_, V> {
        MuxPollFuture { mux: self }
    }

    /// Multiplex `ReadyWait` source into multiplexer.
    #[must_use = "Unregister when Muxing is dropped."]
    pub async fn mux(&mut self, wait: Box<dyn ReadyWait>, value: V) -> Muxing<V> {
        Inner::register(self.inner.clone(), wait, value).await
    }
}

/// Mux::poll future object.
pub struct MuxPollFuture<'a, V>
where
    V: Clone + 'static,
{
    mux: &'a mut Mux<V>,
}

impl<'a, V> Future for MuxPollFuture<'a, V>
where
    V: Clone + 'static,
{
    type Output = V;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.mux.inner.borrow_mut();
        loop {
            let lowest = inner.ready.lowest_one();
            if lowest.is_none() {
                match inner.waker.as_mut() {
                    Some(waker) => waker.clone_from(cx.waker()),
                    None => inner.waker = Some(cx.waker().clone()),
                }
                return Poll::Pending;
            }
            let id = lowest.unwrap();
            inner.ready.bitset(id, false);
            let wait = &inner.regs[id].wait;
            if !wait.ready() {
                continue;
            }
            inner.ready.bitset(id, true);
            return Poll::Ready(inner.regs[id].value.clone());
        }
    }
}
