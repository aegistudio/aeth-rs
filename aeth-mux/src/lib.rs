//! Asynchronous multiplexer.
//!
//! This is a simple but async-idiomatic multiplexer.
//! In the currently depended
//! [implementation](https://github.com/rust-lang/futures-rs/blob/master/futures-macro/src/select.rs#L301)
//! of [`futures::select`], they works more like
//! the `select` mechanism of unix or linux, by
//! really polling these futures from one to another.
//! This is okay for small amout of future, but is
//! not ideal when there are large or dynamic amount
//! of them to be raced within the same async task.
//!
//! Contrarily, this multiplexer reflects more
//! like the way how linux `epoll` does. To poll
//! a future, one must first allocate a slot by
//! associating it will a poll key in the
//! multiplexer [`Mux`] first. The multiplexer
//! will return a subscription handle [`Muxing`],
//! plus the waker [`std::task::Waker`] to
//! notify readiness of the associated future.
//! Then the [`Mux`] will schedule the next
//! future to poll. One must await on
//! [`Mux::poll`] to fetch the poll key,
//! lookup the future associated with the
//! key, and [`try_poll`] that future.

#[doc(hidden)]
pub mod mux;
pub use mux::{Multiplexer, Mux, MuxPoll, Muxing};

#[doc(hidden)]
pub mod future;
pub use future::{FutureSlot, MuxFutureExt};

#[doc(hidden)]
pub mod stream;
pub use stream::{MuxStreamExt, StreamSlot};

pub mod prelude {
    //! Prelude to making life easy for
    //! [this module](crate) users.
    //!
    //! We will just import the trait and
    //! extensions so that it can be recognized
    //! by the compiler. The import types
    //! will be clobbered immediately.
    pub use crate::Multiplexer as _;
    pub use crate::MuxFutureExt as _;
    pub use crate::MuxStreamExt as _;
    pub use crate::Muxing as _;
}

/// Try to poll the async target to ready.
///
/// This macro is intended for using in a
/// [`Mux::poll`]-ing loop, most likely be:
///
/// ```rust,compile_fail
/// loop {
///     match mux.poll().await {
///         ... // look up and poll the future
///     }
/// }
/// ```
///
/// The target to poll must have a `try_poll`
/// method. When the `try_poll` method returns
/// `Some(T)`, it will continue the processing,
/// otherwise it will continue the loop.
#[macro_export]
macro_rules! try_poll {
    ($e: expr) => {{
        match ($e).try_poll() {
            std::option::Option::Some(t) => t,
            std::option::Option::None => continue,
        }
    }};
}

#[cfg(test)]
mod test {
    use crate::Mux;
    use crate::prelude::*;
    use futures::executor::LocalPool;
    use futures::select;
    use futures::task::LocalSpawnExt;
    use futures::{StreamExt, channel};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    #[test]
    fn test_normal() {
        const CH1_READY_WORD_1: usize = 0x111111;
        const CH1_READY_WORD_2: usize = 0x222222;
        const CH1_READY_WORD_3: usize = 0x333333;
        const CH1_READY_WORD_4: usize = 0x444444;

        const CH2_READY_WORD_1: usize = 0x111222;
        const CH2_READY_WORD_2: usize = 0x333444;
        const CH2_READY_WORD_3: usize = 0x555666;

        const CH3_READY_WORD_1: usize = 0xabcdef;
        const CH4_READY_WORD_1: usize = 0xfedcba;
        const CH4_READY_WORD_2: usize = 0x987654;

        let mut local_pool = LocalPool::new();

        let rc0 = Rc::new(Cell::new(0usize));
        let rc0_1 = rc0.clone();
        let (keepalive_tx, mut keepalive_rx) = channel::mpsc::unbounded::<usize>();
        let (repl_ch2_tx, mut repl_ch2_rx) = channel::oneshot::channel::<()>();
        let (recv1_pause_tx, mut recv1_pause_rx) = channel::mpsc::unbounded::<()>();
        let (recv1_resume_tx, mut recv1_resume_rx) = channel::mpsc::unbounded::<()>();
        let (recv1_stop_tx, mut recv1_stop_rx) = channel::mpsc::unbounded::<()>();
        let (repl_ch4_tx, mut repl_ch4_rx) = channel::mpsc::unbounded::<()>();
        let (recv2_cancel_tx, mut recv2_cancel_rx) = channel::oneshot::channel::<()>();

        let rc1 = Rc::new(Cell::new(0usize));
        let rc1_1 = rc1.clone();
        let (ch1_tx, ch1_rx) = channel::mpsc::unbounded::<usize>();
        let (ch2_tx, ch2_rx) = channel::mpsc::unbounded::<usize>();

        let rc2 = Rc::new(Cell::new(0usize));
        let rc2_1 = rc2.clone();
        let (ch3_tx, ch3_rx) = channel::oneshot::channel::<usize>();
        let (ch4_tx, ch4_rx) = channel::mpsc::unbounded::<usize>();

        let rc3 = Rc::new(Cell::new(0usize));
        let rc3_1 = rc3.clone();
        let (_ch5_tx, ch5_rx) = channel::oneshot::channel::<usize>();

        let _handle = local_pool
            .spawner()
            .spawn_local_with_handle(async move {
                #[derive(Clone, Debug)]
                enum Branch {
                    Recv1,
                    Recv2,
                    Recv3,
                }
                let mut mux = Mux::new();

                // compiler don't know if we can borrow ch4 multiple times.
                // Note: you must put the RefCell before the muxing future that
                // may borrow it, since they will be destroyed in reversed order.
                let ch4_rx = RefCell::new(ch4_rx);

                let mut recv1 = mux.mux_stream(Branch::Recv1, ch1_rx);
                // compiler is suspecting we will move ch2_recv multiple times.
                let mut ch2_recv = Some(ch2_rx);
                let mut recv2 = mux.mux(Branch::Recv2, ch3_rx);
                let mut recv3 = mux.mux(Branch::Recv3, ch5_rx);

                loop {
                    select! {
                        value = keepalive_rx.next() => {
                            let value = value.unwrap();
                            println!("Keepalive word {:} has been received.", value);
                            rc0_1.replace(value);
                        },
                        _ = repl_ch2_rx => {
                            recv1.restart(ch2_recv.take().unwrap());
                        },
                        _ = recv1_stop_rx.next() => {
                            recv1.stop();
                        },
                        _ = recv1_pause_rx.next() => {
                            recv1.pause();
                        },
                        _ = recv1_resume_rx.next() => {
                            recv1.resume();
                        },
                        _ = repl_ch4_rx.next() => {
                            let mut ch4_rx_mut = ch4_rx.borrow_mut();
                            recv2.replace(async move {
                                Ok(ch4_rx_mut.next().await.unwrap())
                            });
                        },
                        _ = recv2_cancel_rx => {
                            recv2.cancel();
                        },
                        branch = mux.poll() => {
                            println!("Branch {:?} has been triggered.", branch);
                            match branch {
                                Branch::Recv1 => {
                                    rc1_1.replace(try_poll!(recv1).unwrap());
                                },
                                Branch::Recv2 => {
                                    rc2_1.replace(try_poll!(recv2).unwrap());
                                },
                                Branch::Recv3 => {
                                    rc3_1.replace(try_poll!(recv3).unwrap());
                                },
                            };
                        },
                    }
                }
            })
            .unwrap();

        keepalive_tx.unbounded_send(1).unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 1);
        assert_eq!(rc1.get(), 0);
        assert_eq!(rc2.get(), 0);
        assert_eq!(rc3.get(), 0);

        // Recv1 MuxStream will be polled until blocking,
        // the later comer overwrites the early comers.
        keepalive_tx.unbounded_send(2).unwrap();
        ch1_tx.unbounded_send(CH1_READY_WORD_1).unwrap();
        ch1_tx.unbounded_send(CH1_READY_WORD_2).unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 2);
        assert_eq!(rc1.get(), CH1_READY_WORD_2);
        assert_eq!(rc2.get(), 0);
        assert_eq!(rc3.get(), 0);

        // Also see if the Recv2 branch will be
        // selected correctly.
        keepalive_tx.unbounded_send(3).unwrap();
        ch3_tx.send(CH3_READY_WORD_1).unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 3);
        assert_eq!(rc1.get(), CH1_READY_WORD_2);
        assert_eq!(rc2.get(), CH3_READY_WORD_1);
        assert_eq!(rc3.get(), 0);

        keepalive_tx.unbounded_send(4).unwrap();
        recv1_pause_tx.unbounded_send(()).unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 4);
        assert_eq!(rc1.get(), CH1_READY_WORD_2);
        assert_eq!(rc2.get(), CH3_READY_WORD_1);
        assert_eq!(rc3.get(), 0);

        // Since recv1 has been paused, sending will
        // not generate more events there.
        keepalive_tx.unbounded_send(5).unwrap();
        ch1_tx.unbounded_send(CH1_READY_WORD_3).unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 5);
        assert_eq!(rc1.get(), CH1_READY_WORD_2);
        assert_eq!(rc2.get(), CH3_READY_WORD_1);
        assert_eq!(rc3.get(), 0);

        // Unless we resume it again, which will cause
        // it to receive the pending word immediately.
        keepalive_tx.unbounded_send(6).unwrap();
        recv1_resume_tx.unbounded_send(()).unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 6);
        assert_eq!(rc1.get(), CH1_READY_WORD_3);
        assert_eq!(rc2.get(), CH3_READY_WORD_1);
        assert_eq!(rc3.get(), 0);

        // Sending to ch2 with pending data.
        keepalive_tx.unbounded_send(7).unwrap();
        ch2_tx.unbounded_send(CH2_READY_WORD_1).unwrap();
        ch2_tx.unbounded_send(CH2_READY_WORD_2).unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 7);
        assert_eq!(rc1.get(), CH1_READY_WORD_3);
        assert_eq!(rc2.get(), CH3_READY_WORD_1);
        assert_eq!(rc3.get(), 0);

        // If we replace inner stream of recv1 with ch2,
        // then sending to 1 will result in error since
        // the only receiver has been dropped, and we
        // will be receiving events from ch2.
        keepalive_tx.unbounded_send(8).unwrap();
        repl_ch2_tx.send(()).unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 8);
        assert_eq!(rc1.get(), CH2_READY_WORD_2);
        assert_eq!(rc2.get(), CH3_READY_WORD_1);
        assert_eq!(rc3.get(), 0);
        keepalive_tx.unbounded_send(9).unwrap();
        ch1_tx.unbounded_send(CH1_READY_WORD_4).err().unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 9);
        assert_eq!(rc1.get(), CH2_READY_WORD_2);
        assert_eq!(rc2.get(), CH3_READY_WORD_1);
        assert_eq!(rc3.get(), 0);

        // After stopping ch2, nothing will be received.
        keepalive_tx.unbounded_send(10).unwrap();
        recv1_stop_tx.unbounded_send(()).unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 10);
        assert_eq!(rc1.get(), CH2_READY_WORD_2);
        assert_eq!(rc2.get(), CH3_READY_WORD_1);
        assert_eq!(rc3.get(), 0);
        keepalive_tx.unbounded_send(11).unwrap();
        ch2_tx.unbounded_send(CH2_READY_WORD_3).err().unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 11);
        assert_eq!(rc1.get(), CH2_READY_WORD_2);
        assert_eq!(rc2.get(), CH3_READY_WORD_1);
        assert_eq!(rc3.get(), 0);

        // See the functionality of the future again.
        // We can replace the future, and then recv2
        // will go with this future.
        keepalive_tx.unbounded_send(12).unwrap();
        repl_ch4_tx.unbounded_send(()).unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 12);
        assert_eq!(rc1.get(), CH2_READY_WORD_2);
        assert_eq!(rc2.get(), CH3_READY_WORD_1);
        assert_eq!(rc3.get(), 0);
        keepalive_tx.unbounded_send(13).unwrap();
        ch4_tx.unbounded_send(CH4_READY_WORD_1).unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 13);
        assert_eq!(rc1.get(), CH2_READY_WORD_2);
        assert_eq!(rc2.get(), CH4_READY_WORD_1);
        assert_eq!(rc3.get(), 0);

        // If we cancel the future recv2, then
        // it will not receive the new data.
        keepalive_tx.unbounded_send(14).unwrap();
        recv2_cancel_tx.send(()).unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 14);
        assert_eq!(rc1.get(), CH2_READY_WORD_2);
        assert_eq!(rc2.get(), CH4_READY_WORD_1);
        assert_eq!(rc3.get(), 0);
        keepalive_tx.unbounded_send(15).unwrap();
        ch4_tx.unbounded_send(CH4_READY_WORD_2).unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 15);
        assert_eq!(rc1.get(), CH2_READY_WORD_2);
        assert_eq!(rc2.get(), CH4_READY_WORD_1);
        assert_eq!(rc3.get(), 0);

        // However, if we replace it again, we will
        // be receiving the pending data in ch4.
        keepalive_tx.unbounded_send(16).unwrap();
        repl_ch4_tx.unbounded_send(()).unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 16);
        assert_eq!(rc1.get(), CH2_READY_WORD_2);
        assert_eq!(rc2.get(), CH4_READY_WORD_2);
        assert_eq!(rc3.get(), 0);

        // Ensure the coroutine is alive in the end.
        keepalive_tx.unbounded_send(17).unwrap();
        local_pool.run_until_stalled();
        assert_eq!(rc0.get(), 17);
        assert_eq!(rc1.get(), CH2_READY_WORD_2);
        assert_eq!(rc2.get(), CH4_READY_WORD_2);
        assert_eq!(rc3.get(), 0);
    }
}
