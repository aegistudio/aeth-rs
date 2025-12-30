use crate::core::Handler;
use futures::lock::Mutex;
use indexed_bitmap::IndexedBitmap;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

struct Registry<E>
where
    E: Clone + 'static,
{
    id: Rc<RefCell<usize>>,
    handler: Weak<RefCell<Handler<E>>>,
}

struct Inner<E>
where
    E: Clone + 'static,
{
    waiter: Mutex<()>,
    bitmap: RefCell<IndexedBitmap>,
    regs: RefCell<Vec<Option<Registry<E>>>>,
}

impl<E> Inner<E>
where
    E: Clone + 'static,
{
    fn new() -> Self {
        Self {
            waiter: Mutex::new(()),
            bitmap: RefCell::new(IndexedBitmap::new()),
            regs: RefCell::new(Vec::new()),
        }
    }

    fn deref_handler(r: &Option<Registry<E>>) -> Option<Rc<RefCell<Handler<E>>>> {
        r.as_ref()?.handler.upgrade()
    }

    fn index_handler(&self, i: usize) -> Option<Rc<RefCell<Handler<E>>>> {
        debug_assert!(i < self.regs.borrow().len());
        Self::deref_handler(&self.regs.borrow()[i])
    }

    async fn publish_locked(&self, e: E) -> Option<()> {
        let (min_id, max_id) = {
            let bitmap = self.bitmap.borrow();
            (bitmap.lowest_one()?, bitmap.highest_one()?)
        };
        let mut i = min_id;
        while i <= max_id {
            match self.index_handler(i) {
                Some(rc) => {
                    let mut handler = rc.borrow_mut();
                    let handler_ref: &mut Handler<E> = &mut handler;
                    let ev = e.clone();
                    match handler_ref {
                        Handler::Sync(f) => f(ev),
                        Handler::Async(f) => f.call_mut_boxed(ev).await,
                    };
                    i += 1;
                }
                None => {
                    i = self.bitmap.borrow().next_one(i)?;
                }
            }
        }
        Some(())
    }

    fn register_locked(self: &Rc<Self>, h: Handler<E>) -> Ledge<E> {
        let src = Rc::downgrade(&self);
        let handler = Rc::new(RefCell::new(h));
        let mut bitmap = self.bitmap.borrow_mut();
        let slot = bitmap.lowest_zero();
        let regs = self.regs.borrow();
        if slot < regs.len() {
            assert!(Self::deref_handler(&regs[slot]).is_none());
        }
        std::mem::drop(regs);
        bitmap.bitset(slot, true);
        let id = Rc::new(RefCell::new(slot));
        let mut regs = self.regs.borrow_mut();
        if regs.len() <= slot {
            regs.resize_with(slot + 1, || None);
        }
        regs[slot] = Some(Registry {
            id: id.clone(),
            handler: Rc::downgrade(&handler),
        });
        Ledge {
            inner: src,
            handler: Some(handler),
            id: Some(id.clone()),
        }
    }

    fn compact_locked(&self) {
        // XXX: Wow, what is this mess! Shouldn't we
        // just move the tail registry to fill the
        // cavity? Well, the point here is, let
        // cavity be i, our notification cursor be
        // j, and the tail be k, so that i < j < k.
        // If we move tail regs[k] to the the cavity
        // regs[i], then we will not notifying regs[k]
        // since it's behind the cursor j!
        //
        // In this way, we will have to split the
        // destruction of registry into two phases:
        // In the first phase, the Legde owner drops
        // and leaves a cavity in regs[i]. In the
        // second phase, when there's no event in
        // notification or when the current event
        // notification is done, we fill the cavities
        // introduced in the first phase back.
        let mut bitmap = self.bitmap.borrow_mut();
        let mut regs = self.regs.borrow_mut();
        loop {
            let i = bitmap.lowest_zero();
            let j = bitmap.highest_one().unwrap_or(0);
            if i >= j {
                break;
            }
            assert!(j < regs.len());
            assert!(self.index_handler(i).is_none());
            assert_eq!(bitmap.bitget(i), false);
            assert_eq!(bitmap.bitget(j), true);
            regs[i] = None;
            let mut mid: Option<Registry<E>> = None;
            std::mem::swap(&mut regs[j], &mut mid);
            let mid_ref = mid.as_mut().unwrap();
            assert!(mid_ref.handler.upgrade().is_some());
            *mid_ref.id.borrow_mut() = i;
            std::mem::swap(&mut regs[i], &mut mid);
            bitmap.bitset(i, true);
            bitmap.bitset(j, false);
        }
        let bitmap_size = bitmap.highest_one().map(|x| x + 1).unwrap_or(0);
        if bitmap_size >= regs.len() {
            return;
        }
        regs.truncate(bitmap_size);
        if regs.capacity() >= regs.len() * 2 {
            regs.shrink_to_fit();
            bitmap.shrink_to(regs.capacity());
        }
    }

    fn evict(&self, id: usize) -> Option<()> {
        self.bitmap.borrow_mut().bitset(id, false);
        let _lock = self.waiter.try_lock()?;
        self.compact_locked();
        Some(())
    }

    fn has_subscriber(&self) -> bool {
        self.bitmap.borrow().lowest_one().is_some()
    }
}

/// Event subscription handle.
///
/// This object is a ledge for whether the owner is
/// still interested in this event, and how will the
/// owner process the event. When the subscriber is
/// dropped, the handling function will no longer be
/// called, unless it's being notified.
pub struct Ledge<E>
where
    E: Clone + 'static,
{
    inner: Weak<Inner<E>>,
    handler: Option<Rc<RefCell<Handler<E>>>>,
    id: Option<Rc<RefCell<usize>>>,
}

impl<E> Ledge<E>
where
    E: Clone + 'static,
{
    fn drop_option(&mut self) -> Option<()> {
        std::mem::drop(self.handler.take());
        let pub_inner = self.inner.upgrade()?;
        pub_inner.evict(*self.id.as_ref()?.borrow())
    }
}

impl<E> Drop for Ledge<E>
where
    E: Clone + 'static,
{
    fn drop(&mut self) {
        self.drop_option();
    }
}

impl<E> Default for Ledge<E>
where
    E: Clone + 'static,
{
    fn default() -> Self {
        Self {
            inner: Weak::new(),
            handler: None,
            id: None,
        }
    }
}

/// Event publisher handle.
///
/// Event publisher is the place for notifying the
/// underlying publishers when the specific
/// event occurs.
///
/// The caller is free to clone the event publisher,
/// so that they can be easily distributed.
#[derive(Clone)]
pub struct Pub<E>
where
    E: Clone + 'static,
{
    inner: Rc<Inner<E>>,
}

impl<E> Pub<E>
where
    E: Clone + 'static,
{
    pub async fn publish(&self, e: E) {
        let _lock = self.inner.waiter.lock().await;
        self.inner.publish_locked(e).await;
        self.inner.compact_locked();
    }

    pub fn has_subscriber(&self) -> bool {
        self.inner.has_subscriber()
    }
}

/// Event subscriber handle.
///
/// Event subscriber is the place for registering
/// the event handlers.
///
/// The caller is free to clone the event subscriber,
/// so that they can be easily distributed.
#[derive(Clone)]
pub struct Sub<E>
where
    E: Clone + 'static,
{
    inner: Weak<Inner<E>>,
}

impl<E> Sub<E>
where
    E: Clone + 'static,
{
    #[must_use = "Unregister when Ledge is dropped."]
    pub async fn try_subscribe(&self, h: Handler<E>) -> Option<Ledge<E>> {
        let inner = self.inner.upgrade()?;
        let _lock = inner.waiter.lock().await;
        Some(inner.register_locked(h))
    }

    #[must_use = "Unregister when Ledge is dropped."]
    pub async fn subscribe(&self, h: Handler<E>) -> Ledge<E> {
        self.try_subscribe(h).await.unwrap_or_default()
    }
}

/// Create a publisher and subscriber for the
/// specified event.
///
/// The publisher and subscriber are separate objects
/// so that they are more dedicated to their roles.
pub fn new_pubsub<E>() -> (Pub<E>, Sub<E>)
where
    E: Clone + 'static,
{
    let inner = Rc::new(Inner::new());
    let inner_weak = Rc::downgrade(&inner);
    (Pub { inner }, Sub { inner: inner_weak })
}

#[cfg(test)]
mod test {
    use crate::core::Handler;
    use crate::pubsub::*;
    use crate::testutil::TestFixture;
    use anyhow::Result;

    async fn async_test_normal() -> Result<()> {
        #[derive(Clone)]
        struct EventSome;

        let (pub_some, sub_some) = new_pubsub::<EventSome>();

        let v1 = Rc::new(RefCell::new(0));
        let mv1 = v1.clone();
        let _ledge1 = sub_some
            .subscribe(Handler::new_sync(move |_| {
                *mv1.borrow_mut() = 2;
            }))
            .await;

        let v2 = Rc::new(RefCell::new(0));
        let mv2 = v2.clone();
        let _legde2 = sub_some
            .subscribe(Handler::new_async(async move |_| {
                *mv2.borrow_mut() = 3;
            }))
            .await;

        // The publisher waits for all handlers to execute, thus
        // we observe the mutated values after awaiting.
        pub_some.publish(EventSome).await;
        assert_eq!(*v1.borrow(), 2);
        assert_eq!(*v2.borrow(), 3);
        Ok(())
    }

    #[test]
    fn test_normal() {
        let mut fixture = TestFixture::new();

        fixture
            .execute(async { async_test_normal().await.unwrap() })
            .assert_done();
    }
}
