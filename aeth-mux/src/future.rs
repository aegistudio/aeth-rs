use crate::mux::{Multiplexer, Muxing};
use futures::FutureExt;
use futures::future::LocalBoxFuture;
use std::task::{Context, Poll, Waker};

/// The muxing future slot.
///
/// This object represents an allocated
/// slot in the multiplexer to poll a
/// future [`Future<Output = T>`]. The
/// future to poll is set upon creation
/// by [`MuxFutureExt::mux`] or manual
/// replacement by [`FutureSlot::replace`].
///
/// Once the inner future is canceled by
/// [`FutureSlot::cancel`] or polled
/// to ready by witnessing `Some(T)` from
/// [`FutureSlot::try_poll`], it will
/// be dropped automatically, rendering
/// this object as an empty slot. When
/// the object is an empty slot, it
/// behaves as if it was polling a
/// future returning [Poll::Pending]
/// indefinitely. An empty slot can
/// also be allocated by
/// [`MuxFutureExt::mux_pending`].
///
/// Therefore you want to poll a stream
/// and don't want to do the rearming
/// work by [`FutureSlot::replace`]
/// from time to time, it's better to
/// use [`crate::StreamSlot`]
/// and [`crate::MuxStreamExt`].
pub struct FutureSlot<'a, T, M: Muxing> {
    future: Option<LocalBoxFuture<'a, T>>,
    muxing: M,
    waker: Waker,
}

impl<'a, T, M: Muxing> FutureSlot<'a, T, M> {
    /// Try to poll the future.
    ///
    /// If the future is polled to ready,
    /// then `Some(T)` will be returned.
    /// Otherwise `None` will be returned.
    pub fn try_poll(&mut self) -> Option<T> {
        self.muxing.acknowledge()?;
        let future = self.future.as_mut()?;

        let mut cx = Context::from_waker(&self.waker);

        match future.poll_unpin(&mut cx) {
            Poll::Ready(ready) => {
                std::mem::drop(self.future.take());
                Some(ready)
            }
            Poll::Pending => None,
        }
    }

    /// Cancel the inner future to poll.
    pub fn cancel(&mut self) {
        std::mem::drop(self.future.take());
    }

    /// Replace the inner future to poll.
    pub fn replace<F>(&mut self, future: F)
    where
        F: Future<Output = T> + 'a,
    {
        self.future = Some(Box::pin(future));
        self.waker.wake_by_ref();
    }
}

/// Trait extension to multiplex [`std::future::Future`].
///
/// To multiplex [`std::future::Future`],
/// we can create the muxing future slot
/// object [`FutureSlot`] to hold the
/// future and poll it by
/// [`FutureSlot::try_poll`].
pub trait MuxFutureExt<K>: Multiplexer<K> {
    /// Create an empty muxing future slot.
    fn mux_pending<'a, T>(&mut self, key: K) -> FutureSlot<'a, T, Self::Muxing> {
        let (muxing, waker) = self.multiplex(key);
        FutureSlot {
            future: None,
            muxing,
            waker,
        }
    }

    /// Multiplex the provided future object.
    fn mux<'a, T, F>(&mut self, key: K, future: F) -> FutureSlot<'a, T, Self::Muxing>
    where
        F: Future<Output = T> + 'a,
    {
        let future: LocalBoxFuture<'a, T> = Box::pin(future);
        let mut result = self.mux_pending::<T>(key);
        result.future = Some(future);
        result.waker.wake_by_ref();
        result
    }
}

impl<K, T> MuxFutureExt<K> for T where T: Multiplexer<K> {}
