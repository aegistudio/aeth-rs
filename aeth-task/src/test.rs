use crate::framework::{Config, initialize};
use crate::ready_poll::local_ready_poll;
use crate::{Handle, drain_foreground_loopback};
use crate::{background, foreground, run_foreground};
use futures::future::{BoxFuture, join_all};
use std::cell::RefCell;
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
