//! Make futures ready-waitable.
//!
//! This package makes it possible to work with
//! `aeth::event::ReadyWait`, that is, to select
//! the readiness of a future just as other events.
//! This feature must be implemented in a thread
//! type aware manner, since we will have to run
//! the event notification logic.

use crate::{Handle, foreground};
use aeth_event::prelude::*;
use aeth_event::{Pub, ReadyWait, Sub, new_pubsub};
use futures::channel::mpsc::UnboundedSender;
use futures::channel::mpsc::unbounded;
use futures::channel::oneshot::Sender as OneshotSender;
use futures::channel::oneshot::channel as oneshot;
use futures::future::LocalBoxFuture;
use futures::select;
use futures::{FutureExt, SinkExt, StreamExt};
use indexed_bitmap::IndexedBitmap;
use std::cell::RefCell;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak as ArcWeak};
use std::task::{Context, Poll, RawWaker, Wake, Waker};
use lives::{Life, LifeRc, LifeWeak};

struct RaceItem {
    id: Arc<AtomicUsize>,
    polling: Rc<RefCell<bool>>,
    subscribed: Rc<RefCell<bool>>,
    ready_sub: Sub<()>,
    free_tx: UnboundedSender<Arc<AtomicUsize>>,
}

impl Drop for RaceItem {
    fn drop(&mut self) {
        let _ = self.free_tx.unbounded_send(self.id.clone());
    }
}

struct RaceAllocRequest {
    result_tx: OneshotSender<RaceItem>,
}

struct RaceItemInner {
    id: Arc<AtomicUsize>,
    subscribed: Rc<RefCell<bool>>,
    polling: Rc<RefCell<bool>>,
    ready_pub: Pub<()>,
}

struct RacerThreadState {
    registry: Vec<RaceItemInner>,
    trigger: IndexedBitmap,
    now_polling: Option<Arc<AtomicUsize>>,
    free_tx: UnboundedSender<Arc<AtomicUsize>>,
}

impl RacerThreadState {
    fn alloc(&mut self) -> RaceItem {
        let slot = self.registry.len();
        let id = Arc::new(AtomicUsize::new(slot));
        let (ready_pub, ready_sub) = new_pubsub();
        let polling = Rc::new(RefCell::new(false));
        let subscribed = Rc::new(RefCell::new(false));
        self.registry.push(RaceItemInner {
            id: id.clone(),
            polling: polling.clone(),
            subscribed: subscribed.clone(),
            ready_pub,
        });
        self.trigger.bitset(slot, true);
        RaceItem {
            id: id.clone(),
            polling,
            subscribed,
            ready_sub,
            free_tx: self.free_tx.clone(),
        }
    }

    fn now_polling_slot(&mut self) -> Option<usize> {
        let now_polling = self.now_polling.as_ref()?;
        Some(now_polling.load(Ordering::SeqCst))
    }

    fn free(&mut self, arc: Arc<AtomicUsize>) {
        let id = arc.load(Ordering::SeqCst);
        assert!(self.registry.len() > 0);
        assert!(self.registry[id].id.load(Ordering::SeqCst) == id);
        let last_id = self.registry.len() - 1;
        assert!(self.registry[last_id].id.load(Ordering::SeqCst) == last_id);
        if self.now_polling_slot() == Some(id) {
            self.now_polling = None;
        }
        if id < last_id {
            // XXX: `self.now_polling_slot() == Some(last_id)` is okay,
            // since it's an Arc and will be updated alongside with
            // self.registry[last_id].id.
            let triggered = self.trigger.bitget(last_id);
            self.registry.swap(id, last_id);
            self.registry[id].id.store(id, Ordering::SeqCst);
            self.trigger.bitset(id, triggered);
        }
        self.trigger.bitset(last_id, false);
        self.registry.pop();
        if self.registry.len() * 2 < self.registry.capacity() {
            self.registry.shrink_to_fit();
            self.trigger.shrink_to(self.registry.capacity());
        }
    }

    fn notify(&mut self, weak: ArcWeak<AtomicUsize>) {
        let arc = weak.upgrade();
        if let Some(arc) = arc {
            let id = arc.load(Ordering::SeqCst);
            assert!(id < self.registry.len());
            self.trigger.bitset(id, true);
        }
    }

    async fn poke(&mut self) {
        if self.now_polling.is_some() {
            let slot = self.now_polling.as_ref().unwrap().load(Ordering::SeqCst);
            let polling = *self.registry[slot].polling.borrow();
            if !polling {
                self.now_polling = None;
            }
        }
        if self.now_polling.is_none() {
            if let Some(slot) = self.trigger.lowest_one() {
                let subscribed = *self.registry[slot].subscribed.borrow();
                if !subscribed {
                    return;
                }
                self.trigger.bitset(slot, false);
                *self.registry[slot].polling.borrow_mut() = true;
                let ready_pub = self.registry[slot].ready_pub.clone();
                ready_pub.take_publish(()).await;
                self.now_polling = Some(self.registry[slot].id.clone());
            }
        }
    }
}

struct RacerInner {
    _handle: Box<dyn Handle<()>>,
    poked: Rc<RefCell<bool>>,
    poke_tx: UnboundedSender<()>,
    alloc_tx: UnboundedSender<RaceAllocRequest>,
    notify_tx: UnboundedSender<ArcWeak<AtomicUsize>>,
}

impl RacerInner {
    fn poke(&self) {
        let poked = *self.poked.borrow();
        if poked {
            return;
        }
        let _ = self.poke_tx.clone().unbounded_send(());
        *self.poked.borrow_mut() = true;
    }
}

fn create_racer_inner() -> RacerInner {
    let poked = Rc::new(RefCell::new(false));
    let (alloc_tx, alloc_rx) = unbounded();
    let (free_tx, free_rx) = unbounded();
    let (notify_tx, notify_rx) = unbounded();
    let (poke_tx, poke_rx) = unbounded();

    let poked1 = poked.clone();
    let free_tx1 = free_tx.clone();
    let handle = Box::new(foreground::spawn(async move {
        let poked = poked1;
        let free_tx = free_tx1;
        let mut alloc_rx = alloc_rx;
        let mut free_rx = free_rx;
        let mut notify_rx = notify_rx;
        let mut poke_rx = poke_rx;
        let mut state = RacerThreadState {
            registry: Vec::new(),
            trigger: IndexedBitmap::new(),
            now_polling: None,
            free_tx: free_tx,
        };
        loop {
            select! {
                alloc = alloc_rx.next() => {
                    let alloc: RaceAllocRequest = alloc.unwrap();
                    // XXX: If it fails to send, then the id of
                    // this item will be sent back in free_tx
                    // later as long as foreground task is alive,
                    // so it's safe doing so.
                    let _ = alloc.result_tx.send(state.alloc());
                },
                id = free_rx.next() => {
                    state.free(id.unwrap());
                    state.poke().await;
                },
                weak = notify_rx.next() => {
                    state.notify(weak.unwrap());
                    state.poke().await;
                },
                _ = poke_rx.next() => {
                    *poked.borrow_mut() = false;
                    state.poke().await;
                },
            }
        }
    }));
    RacerInner {
        _handle: handle,
        poked,
        poke_tx,
        alloc_tx,
        notify_tx,
    }
}

trait RaceReadyWaitable {
    fn do_poll(&self, cx: &mut Context<'_>) -> bool;
}

#[derive(Life)]
struct BoxRaceReadyWaitable<'a> (Box<dyn RaceReadyWaitable + 'a>);

struct RaceReadyFutureCell<'a, T: 'a> {
    future: LocalBoxFuture<'a, T>,
    ready: Option<T>,
}

struct RaceReadyFutureInner<'a, T> {
    cell: Rc<RefCell<RaceReadyFutureCell<'a, T>>>,
}

impl<'a, T> RaceReadyWaitable for RaceReadyFutureInner<'a, T> {
    fn do_poll(&self, cx: &mut Context<'_>) -> bool {
        let mut cell = self.cell.borrow_mut();
        if cell.ready.is_none() {
            let result = cell.future.poll_unpin(cx);
            match result {
                Poll::Ready(ready) => cell.ready = Some(ready),
                Poll::Pending => {}
            }
        }
        cell.ready.is_some()
    }
}

struct RaceItemWaker {
    notify_tx: UnboundedSender<ArcWeak<AtomicUsize>>,
    id: ArcWeak<AtomicUsize>,
}

impl Wake for RaceItemWaker {
    fn wake_by_ref(self: &Arc<Self>) {
        let _ = self.notify_tx.unbounded_send(self.id.clone());
    }

    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
}

struct RaceReadyWait {
    ready_waitable: LifeWeak<BoxRaceReadyWaitable<'static>>,
    race_item: Weak<RaceItem>,
    ready_sub: Sub<()>,
    racer_inner: Weak<RefCell<RacerInner>>,
}

impl RaceReadyWait {
    fn poll_option(&self, id: Arc<AtomicUsize>) -> Option<bool> {
        let race_inner = self.racer_inner.upgrade()?;
        let notify_tx = race_inner.borrow().notify_tx.clone();
        let id = Arc::downgrade(&id);
        let item_waker = Arc::new(RaceItemWaker { notify_tx, id });
        let raw_waker = RawWaker::from(item_waker);
        let waker = unsafe { Waker::from_raw(raw_waker) };
        let mut cx = Context::from_waker(&waker);

        let ready = self.ready_waitable
            .with(|f| f.0.do_poll(&mut cx))
            .unwrap_or_default();

        Some(ready)
    }

    fn ready_option(&self) -> Option<bool> {
        let race_item = self.race_item.upgrade()?;
        let polling = *race_item.polling.borrow();
        if !polling {
            return Some(false);
        }
        let ready = self.poll_option(race_item.id.clone())?;
        *race_item.polling.borrow_mut() = ready;
        Some(ready)
    }

    fn notify_subscribed_option(&self) -> Option<()> {
        let race_item = self.race_item.upgrade()?;
        *race_item.subscribed.borrow_mut() = true;
        let racer_inner = self.racer_inner.upgrade()?;
        racer_inner.borrow().poke();
        Some(())
    }
}

impl ReadyWait for RaceReadyWait {
    fn ready(&self) -> bool {
        self.ready_option().unwrap_or(false)
    }

    fn waiter(&self) -> Sub<()> {
        self.ready_sub.clone()
    }

    fn notify_subscribed(&self) {
        self.notify_subscribed_option();
    }
}

/// Local future wrapper to be ready-waitable.
///
/// Please notice that the `self.ready_wait().ready()`
/// must return true before the future itself can be
/// awaited, otherwise awaiting the future will panic.
///
/// This is intended to be used with a `Mux`, where
/// you first multiplex the future into a `Mux` by a
/// key, then when the `mux.poll().await` returns
/// the key, you await this future.
pub struct ReadyWaitFuture<'a, T> {
    inner: Rc<RefCell<RaceReadyFutureCell<'a, T>>>,
    dyn_rc: LifeRc<BoxRaceReadyWaitable<'a>>,
    race_item: Rc<RaceItem>,
    racer_inner: Weak<RefCell<RacerInner>>,
}

impl<'a, T> ReadyWaitFuture<'a, T> {
    pub fn ready_wait(&self) -> Box<dyn ReadyWait> {
        Box::new(RaceReadyWait {
            ready_waitable: LifeRc::downgrade(&self.dyn_rc),
            race_item: Rc::downgrade(&self.race_item),
            ready_sub: self.race_item.ready_sub.clone(),
            racer_inner: self.racer_inner.clone(),
        })
    }
}

impl<'a, T> Future for ReadyWaitFuture<'a, T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.inner.borrow_mut();
        let result = inner
            .ready
            .take()
            .expect("Must be first reported ready by Mux::poll.");
        Poll::Ready(result)
    }
}

/// Racer to race multiple futures by ready wait.
pub struct Racer {
    racer_inner: Rc<RefCell<RacerInner>>,
}

impl Racer {
    pub fn new() -> Self {
        Self {
            racer_inner: Rc::new(RefCell::new(create_racer_inner())),
        }
    }

    pub async unsafe fn race<'a, T: 'a, F>(&self, future: F) -> ReadyWaitFuture<'a, T>
    where
        F: Future<Output = T> + 'a,
    {
        let (result_tx, result_rx) = oneshot();
        self.racer_inner
            .borrow()
            .alloc_tx
            .clone()
            .send(RaceAllocRequest { result_tx })
            .await
            .unwrap();
        let race_item = result_rx.await.unwrap();
        let race_item = Rc::new(race_item);

        let boxed_future = Box::pin(future);
        let inner_cell = RaceReadyFutureCell{
            future: boxed_future,
            ready: None,
        };
        let inner = Rc::new(RefCell::new(inner_cell));
        let dyn_rc = LifeRc::new(BoxRaceReadyWaitable(Box::new(RaceReadyFutureInner {
            cell: inner.clone(),
        })));
        ReadyWaitFuture {
            inner: inner,
            dyn_rc: dyn_rc,
            race_item: race_item,
            racer_inner: Rc::downgrade(&self.racer_inner),
        }
    }
}
