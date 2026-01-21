use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::future::FusedFuture;
use futures::ready;
use futures::{FutureExt, StreamExt};
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::task::{Context, Poll, Wake, Waker};

struct Registry<K: Clone> {
    key: K,
    id: Arc<AtomicUsize>,
}

struct Inner<K: Clone> {
    send: UnboundedSender<Arc<AtomicUsize>>,
    recv: UnboundedReceiver<Arc<AtomicUsize>>,
    registries: Vec<Registry<K>>,
    pending: Option<Arc<AtomicUsize>>,
}

impl<K: Clone> Inner<K> {
    fn new() -> Self {
        let (send, recv) = unbounded();
        Self {
            send,
            recv,
            registries: Vec::new(),
            pending: None,
        }
    }

    fn acknowledge(&mut self, syncing: Arc<AtomicUsize>) {
        if let Some(ref pending) = self.pending {
            let pending_id = pending.load(Ordering::SeqCst);
            let syncing_id = syncing.load(Ordering::SeqCst);
            if pending_id == syncing_id {
                self.pending.take();
                return;
            }
        }
        panic!("Incorrect acknowledgement, maybe the wrong branch or future taken.")
    }

    fn evict(&mut self, id: Arc<AtomicUsize>) -> Option<Registry<K>> {
        let id = id.load(Ordering::SeqCst);
        if id == usize::MAX {
            return None;
        }
        assert!(id < self.registries.len());
        assert!(self.registries[id].id.load(Ordering::SeqCst) == id);
        let last_id = self.registries.len() - 1;
        if id < last_id {
            self.registries.swap(id, last_id);
            self.registries[id].id.store(id, Ordering::SeqCst);
            self.registries[last_id].id.store(last_id, Ordering::SeqCst);
        }
        let result = self.registries.pop()?;
        result.id.store(usize::MAX, Ordering::SeqCst);
        Some(result)
    }

    pub fn allocate(&mut self, key: K) -> Arc<AtomicUsize> {
        let new_id = self.registries.len();
        if new_id == usize::MAX {
            panic!("Out of memory, cannot multiplex more on this multiplexer.")
        }
        let id = Arc::new(AtomicUsize::new(new_id));
        self.registries.push(Registry {
            key,
            id: id.clone(),
        });
        id
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<K> {
        if let Some(pending) = self.pending.take() {
            if pending.load(Ordering::SeqCst) != usize::MAX {
                panic!("Previously signaled future has not been acknowledged.")
            }
        }
        loop {
            let id = ready!(self.recv.next().poll_unpin(cx)).unwrap();
            let id = id.load(Ordering::SeqCst);
            if id == usize::MAX {
                continue;
            }
            assert_eq!(self.registries[id].id.load(Ordering::SeqCst), id);
            self.pending = Some(self.registries[id].id.clone());
            return Poll::Ready(self.registries[id].key.clone());
        }
    }
}

/// The future of [`Mux::poll`].
pub struct MuxPoll<'a, K: Clone> {
    inner: &'a RefCell<Inner<K>>,
}

impl<'a, K: Clone> Future for MuxPoll<'a, K> {
    type Output = K;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<K> {
        let mut inner = self.inner.borrow_mut();
        inner.poll(cx)
    }
}

impl<'a, K: Clone> FusedFuture for MuxPoll<'a, K> {
    fn is_terminated(&self) -> bool {
        return false;
    }
}

/// Raw implementation of [`Muxing`].
pub struct RawMuxing<K: Clone> {
    id: Arc<AtomicUsize>,
    inner: Weak<RefCell<Inner<K>>>,
}

impl<K: Clone> RawMuxing<K> {
    fn new(rc: &Rc<RefCell<Inner<K>>>, value: K) -> Self {
        let inner = Rc::downgrade(rc);
        let id = rc.borrow_mut().allocate(value);
        Self { id, inner }
    }

    fn drop_option(&mut self) -> Option<()> {
        let inner = self.inner.upgrade()?;
        inner.borrow_mut().evict(self.id.clone());
        Some(())
    }
}

impl<K: Clone> Drop for RawMuxing<K> {
    fn drop(&mut self) {
        self.drop_option();
    }
}

struct MuxWaker {
    id: Arc<AtomicUsize>,
    send: UnboundedSender<Arc<AtomicUsize>>,
}

impl MuxWaker {
    fn mux_wake(&self) {
        let _ = self.send.unbounded_send(self.id.clone());
    }
}

impl Wake for MuxWaker {
    fn wake(self: Arc<Self>) {
        self.mux_wake();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.mux_wake();
    }
}

/// The muxing subscription handle trait.
///
/// This is the handle to track whether a
/// slot allocated by [`Multiplexer::multiplex`]
/// is still alive.
///
/// This object also tracks the regularity
/// of usage of multiplexer. Whenever the
/// [`Mux::poll`] returns a poll key, the
/// associated muxing subscription
/// **must be** [`Muxing::acknowledge`]-d
/// before [`Mux::poll`]-ing the next ready
/// future again. Polling the wrong future
/// will result in panic, starting a new
/// round of [`Mux::poll`] without
/// acknowledgement will also result in panic.
/// Panic means your usage is wrong.
pub trait Muxing {
    /// Acknowledge the ready signal from [`Mux::poll`].
    ///
    /// For every poll key retured by [`Mux::poll`],
    /// the caller must find the [`Muxing`] associated
    /// and acknowledge the readiness of this key.
    /// Failing to find the correct [`Muxing`] or
    /// starting a new [`Mux::poll`] without
    /// acknowledging the [`Muxing`] is a signal of
    /// wrong usage and will result in panic.
    ///
    /// On the other hand, one must not continue on
    /// polling the future if acknowlege returns None.
    fn acknowledge(&mut self) -> Option<()>;
}

impl<K: Clone> Muxing for RawMuxing<K> {
    fn acknowledge(&mut self) -> Option<()> {
        let inner = self.inner.upgrade()?;
        inner.borrow_mut().acknowledge(self.id.clone());
        Some(())
    }
}

/// The async multiplexer trait.
///
/// This trait is intended for behavioral
/// extension of [`Mux`]. For usage example,
/// one may also want to look into
/// [`crate::MuxFutureExt`] and
/// [`crate::MuxStreamExt`].
pub trait Multiplexer<K> {
    type Muxing: Muxing;

    /// Allocating a multiplexer slot by
    /// associating with the `key`.
    ///
    /// The muxing subscription will end
    /// as soon as [`Muxing`] is destroyed.
    /// The [`Mux::poll`] will return the
    /// readiness of the `key` as soon as
    /// the returned [`Waker`] is
    /// [`Waker::wake`]-ed or
    /// [`Waker::wake_by_ref`]-ed.
    fn multiplex(&mut self, key: K) -> (Self::Muxing, Waker);
}

/// The async multiplexer.
///
/// This is the multiplexer that the crate
/// is providing. To work, one must
/// associate the future with a poll key
/// in [`Multiplexer::multiplex`], so
/// that when the future is ready, the
/// multiplexer can report the readiness
/// by returning the associated poll
/// key from [`Mux::poll`].
///
/// It's also worthwhile to look into
/// [`crate::MuxFutureExt`] and
/// [`crate::MuxStreamExt`],
/// they suffice in most use cases.
pub struct Mux<K: Clone> {
    inner: Rc<RefCell<Inner<K>>>,
}

impl<K: Clone> Mux<K> {
    /// Create a new multiplexer.
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(Inner::new())),
        }
    }

    /// Poll the next ready key.
    ///
    /// After receiving the poll key,
    /// one must lookup the corresponding
    /// future and polls it. Failing to
    /// do so is regarded as wrong usage
    /// and will result in panic.
    pub fn poll(&mut self) -> MuxPoll<'_, K> {
        MuxPoll {
            inner: self.inner.as_ref(),
        }
    }
}

impl<K: Clone> Multiplexer<K> for Mux<K> {
    type Muxing = RawMuxing<K>;

    fn multiplex(&mut self, key: K) -> (Self::Muxing, Waker) {
        let muxing = RawMuxing::new(&self.inner, key);
        let id = muxing.id.clone();
        let send = self.inner.borrow().send.clone();
        let waker = Waker::from(Arc::new(MuxWaker { id, send }));
        (muxing, waker)
    }
}
