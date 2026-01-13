use crate::framework::{Config, initialize};
use crate::ready_poll::local_ready_poll;
use crate::ready_wait::ready_wait;
use crate::{Handle, drain_foreground_loopback};
use crate::{background, foreground, run_foreground};
use futures::channel;
use futures::future::{BoxFuture, join_all};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[test]
fn test_normal() {
    let cfg = Config::default();
    let _fx = initialize(cfg).unwrap();

    thread_local! {
        static THREAD_LOCAL: RefCell<usize> = RefCell::new(0usize);
    }

    let rc = Rc::new(RefCell::new(0usize));
    let rc1 = rc.clone();
    let (future, ready) = local_ready_poll(async move {
        println!("foreground aggregation task started");
        foreground::assert();
        let mut handles: Vec<BoxFuture<usize>> = Vec::new();
        for i in 0..=100 {
            let value = i;
            handles.push(Box::pin(foreground::dispatch(async move {
                println!("background task {} started", value);
                background::assert();
                background::loopback(async move {
                    println!("foreground loopback task {} started", value);
                    foreground::assert();
                    // XXX: If the code still runs in background thread,
                    // it will fail to modify the RECEIVER value. Also,
                    // the foreground thread runs them linearly, so no
                    // atomic operation is required.
                    THREAD_LOCAL.with_borrow_mut(|v| {
                        *v += value * 100;
                    });
                    println!("foreground loopback task {} done", value);
                })
                .await;
                println!("background task {} done", value);
                value * 2
            })));
        }
        *rc1.borrow_mut() = join_all(handles).await.into_iter().sum::<usize>();
        println!("foreground aggregation task done");
    });
    foreground::spawn(future).detach();

    loop {
        println!("foreground iteration started");
        run_foreground();
        drain_foreground_loopback();
        println!("foreground iteration done");
        if ready.ready() {
            println!("foreground aggregation task polled done");
            break;
        }
        println!("foreground aggregation task not polled done, waiting...");
        std::thread::yield_now();
    }
    assert_eq!(*rc.borrow(), 10100);
    assert_eq!(THREAD_LOCAL.take(), 505000);
}

#[test]
fn test_ready_wait_normal() {
    use aeth_event::prelude::*;

    const RC1_READY_WORD: usize = 0x123456;
    const RC2_READY_WORD: usize = 0x7890ab;
    const RC2_CANCEL_WORD: usize = 0xcdef01;
    const RC3_READY_WORD: usize = 0x234567;

    let cfg = Config::default();
    let _fx = initialize(cfg).unwrap();

    let rc1 = Rc::new(Cell::new(0usize));
    let rc1_1 = rc1.clone();
    let (send1, recv1) = channel::oneshot::channel::<usize>();

    let rc2 = Rc::new(Cell::new(0usize));
    let rc2_1 = rc2.clone();
    let (send2, recv2) = channel::oneshot::channel::<usize>();

    let rc3 = Rc::new(Cell::new(0usize));
    let rc3_1 = rc3.clone();
    let (send3, recv3) = channel::oneshot::channel::<usize>();

    let (pub1, sub1) = aeth_event::new_pubsub::<()>();
    let (pub2, sub2) = aeth_event::new_pubsub::<()>();

    let (future, ready) = local_ready_poll(async move {
        #[derive(Clone)]
        enum Branch {
            Recv1,
            Recv2,
            Recv3,
            Ch1,
            Ch2,
        }
        let mut mux = aeth_event::Mux::new();

        let (recv1, rw1) = ready_wait(recv1);
        let mut recv1 = Some(recv1.no_cancel());
        let _ledge1 = mux.mux(rw1, Branch::Recv1).await;

        let (recv2, rw2) = ready_wait(recv2);
        let mut recv2 = Some(recv2);
        let _ledge2 = mux.mux(rw2, Branch::Recv2).await;

        let (recv3, rw3) = ready_wait(recv3);
        let mut recv3 = Some(recv3.no_cancel());
        let _ledge3 = mux.mux(rw3, Branch::Recv3).await;

        let mut ch1 = sub1.chan().await;
        let _ledge4 = mux.mux(ch1.ready_wait(), Branch::Ch1).await;
        let mut ch2 = sub2.chan().await;
        let _ledge5 = mux.mux(ch2.ready_wait(), Branch::Ch2).await;

        loop {
            match mux.poll().await {
                Branch::Recv1 => {
                    rc1_1.replace(recv1.take().unwrap().await.unwrap());
                }
                Branch::Recv2 => {
                    match recv2.take().unwrap().await {
                        Some(result) => rc2_1.replace(result.unwrap()),
                        None => rc2_1.replace(RC2_CANCEL_WORD),
                    };
                }
                Branch::Recv3 => {
                    rc3_1.replace(recv3.take().unwrap().await.unwrap());
                }
                Branch::Ch1 => {
                    ch1.recv().await;
                    recv2.as_mut().unwrap().cancel();
                }
                Branch::Ch2 => {
                    ch2.recv().await;
                    std::mem::drop(recv3.take());
                }
            }
        }
    });
    foreground::spawn(future).detach();

    // Main task is running, nothing has changed.
    run_foreground();
    assert!(!ready.ready());
    assert_eq!(rc1.get(), 0);
    assert_eq!(rc2.get(), 0);
    assert_eq!(rc3.get(), 0);

    // The Branch::Recv1 activates, and change rc1.
    send1.send(RC1_READY_WORD).unwrap();
    run_foreground();
    assert!(!ready.ready());
    assert_eq!(rc1.get(), RC1_READY_WORD);
    assert_eq!(rc2.get(), 0);
    assert_eq!(rc3.get(), 0);

    // Notifying pub1 cancel recv2, thus sending to
    // send2 will do nothing and the canceled
    // branch is taken instead.
    foreground::spawn(pub1.take_publish(())).detach();
    run_foreground();
    assert!(!ready.ready());
    let state = send2.send(RC2_READY_WORD);
    assert!(state.is_err()); // It's wrapped in future and dropped.
    run_foreground();
    assert!(!ready.ready());
    assert_eq!(rc1.get(), RC1_READY_WORD);
    assert_eq!(rc2.get(), RC2_CANCEL_WORD);
    assert_eq!(rc3.get(), 0);

    // Notifying pub2 drops recv3, thus sending to
    // send3 will do nothing and none of the ready
    // or canceled branch are called.
    foreground::spawn(pub2.take_publish(())).detach();
    run_foreground();
    assert!(!ready.ready());
    let state = send3.send(RC3_READY_WORD);
    assert!(state.is_err()); // It's wrapped in future and dropped.
    run_foreground();
    assert!(!ready.ready());
    assert_eq!(rc1.get(), RC1_READY_WORD);
    assert_eq!(rc2.get(), RC2_CANCEL_WORD);
    assert_eq!(rc3.get(), 0);
}
