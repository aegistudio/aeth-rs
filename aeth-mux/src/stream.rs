use crate::mux::{Multiplexer, Muxing};
use futures::{Stream, StreamExt, stream::LocalBoxStream};
use std::task::{Context, Poll, Waker};

enum Streaming<'a, T> {
    Normal(LocalBoxStream<'a, T>),
    Paused(LocalBoxStream<'a, T>),
    None,
}

impl<'a, T> Streaming<'a, T> {
    fn pause(self) -> Self {
        match self {
            Self::Normal(stream) => Self::Paused(stream),
            _ => self,
        }
    }

    fn resume(self) -> Self {
        match self {
            Self::Paused(stream) => Self::Normal(stream),
            _ => self,
        }
    }

    fn stream_to_poll(&mut self) -> Option<&mut LocalBoxStream<'a, T>> {
        match self {
            Self::Normal(stream) => Some(stream),
            _ => None,
        }
    }

    fn is_normal(&self) -> bool {
        match self {
            Self::Normal(_) => true,
            _ => false,
        }
    }
}

/// The muxing stream slot.
///
/// This object represents an allocated slot
/// in the multiplexer to poll a stream
/// [`Stream<Item = T>`]. The stream to poll
/// is set upon creation by
/// [`MuxStreamExt::mux_stream`] or manual
/// replacement by
/// [`StreamSlot::restart`].
///
/// Once the inner stream is canceled by
/// [`StreamSlot::stop`] or polled to
/// end-of-stream by witnessing `Some(None)`
/// from [`StreamSlot::try_poll`], it will
/// be dropped automatically, rendering
/// this object as an empty slot. When
/// the object is an empty slot, it
/// behaves as if it were polling a
/// [`Stream<Item = T>`] that never emit
/// an item. An empty slot can also be
/// allocated by calling
/// [`MuxStreamExt::mux_pending_stream`].
///
/// You can also pause (receiving
/// from) the stream by calling
/// [`StreamSlot::pause`]. The paused
/// stream can be resumed by calling
/// [`StreamSlot::resume`] later.
pub struct StreamSlot<'a, T, M: Muxing> {
    stream: Option<Streaming<'a, T>>,
    muxing: M,
    waker: Waker,
}

impl<'a, T, M: Muxing> StreamSlot<'a, T, M> {
    /// Try to poll the stream.
    ///
    /// If the stream generates an item, then
    /// `Some(Some(T))` will be returned. If
    /// the stream is end-of-stream, then
    /// `Some(None)` will be returned. Otherwise
    /// `None` will be returned.
    pub fn try_poll(&mut self) -> Option<Option<T>> {
        self.muxing.acknowledge()?;
        // XXX: Yes, it's `unwrap()` here. The `self.stream`
        // just serve as a cell for calling self comsuming
        // methods of Streaming<'a, T>. Usually those
        // methods don't panic, but if they panics, the
        // caller must not touch this MuxingStream anymore.
        let stream = self.stream.as_mut().unwrap().stream_to_poll()?;

        let mut cx = Context::from_waker(&self.waker);
        match stream.poll_next_unpin(&mut cx) {
            Poll::Ready(option) => {
                if option.is_some() {
                    self.waker.wake_by_ref();
                } else {
                    self.stream = Some(Streaming::None);
                }
                Some(option)
            }
            Poll::Pending => None,
        }
    }

    fn notify_if_normal(&self) {
        if self.stream.as_ref().unwrap().is_normal() {
            self.waker.wake_by_ref();
        }
    }

    fn replace(&mut self, streaming: Streaming<'a, T>) {
        self.stream = Some(streaming);
        self.notify_if_normal();
    }

    /// Pause the receiving from the stream.
    pub fn pause(&mut self) {
        let streaming = self.stream.take().unwrap().pause();
        self.replace(streaming);
    }

    /// Resume the receiving from the stream.
    pub fn resume(&mut self) {
        let streaming = self.stream.take().unwrap().resume();
        self.replace(streaming);
    }

    /// Stop the inner stream to poll.
    pub fn stop(&mut self) {
        self.replace(Streaming::None);
    }

    /// Replace the inner stream to poll.
    pub fn restart<S>(&mut self, stream: S)
    where
        S: Stream<Item = T> + 'a,
    {
        self.replace(Streaming::Normal(Box::pin(stream)));
    }
}

/// Trait extension to multiplex [`futures::Stream`].
///
/// To multiplex [`futures::Stream`],
/// we can create the muxing stream slot
/// object [`StreamSlot`] to hold the
/// stream and polls it by
/// [`StreamSlot::try_poll`].
pub trait MuxStreamExt<K>: Multiplexer<K> {
    /// Create an empty muxing stream slot.
    fn mux_pending_stream<'a, T>(&mut self, key: K) -> StreamSlot<'a, T, Self::Muxing> {
        let (muxing, waker) = self.multiplex(key);
        StreamSlot {
            stream: Some(Streaming::None),
            muxing,
            waker,
        }
    }

    /// Multiplex the provided stream object.
    fn mux_stream<'a, T, S>(&mut self, key: K, stream: S) -> StreamSlot<'a, T, Self::Muxing>
    where
        S: Stream<Item = T> + 'a,
    {
        let mut result = self.mux_pending_stream(key);
        result.replace(Streaming::Normal(Box::pin(stream)));
        result
    }
}

impl<K, T> MuxStreamExt<K> for T where T: Multiplexer<K> {}
