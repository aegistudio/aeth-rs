use crate::mux::ReadyWait;
use crate::pubsub::{Pub, Sub};
use crate::{Handler, Ledge, new_pubsub};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::{SinkExt, StreamExt};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

struct Inner<E>
where
    E: Clone + 'static,
{
    _send: UnboundedSender<E>,
    recv: UnboundedReceiver<E>,
    head: Option<E>,
}

impl<E> Inner<E>
where
    E: Clone + 'static,
{
    fn try_move_head(&mut self) -> Option<()> {
        let polled = self.recv.try_next().ok()?;
        self.head = Some(polled?);
        Some(())
    }

    fn try_recv_ready(&mut self) -> bool {
        if self.head.is_none() {
            self.try_move_head();
        }
        self.head.is_some()
    }

    async fn recv(&mut self) -> E {
        if let Some(head) = self.head.take() {
            head
        } else {
            // We may use unwrap here, since we hold
            // one sender as a field of inner, and
            // the sender will never be closed.
            self.recv.next().await.unwrap()
        }
    }
}

struct InnerReadyWait<E>
where
    E: Clone + 'static,
{
    rc: Weak<RefCell<Inner<E>>>,
    sub: Sub<()>,
}

impl<E> ReadyWait for InnerReadyWait<E>
where
    E: Clone + 'static,
{
    fn ready(&self) -> bool {
        self.rc
            .upgrade()
            .map(|inner| inner.borrow_mut().try_recv_ready())
            .unwrap_or(false)
    }

    fn waiter(&self) -> Sub<()> {
        self.sub.clone()
    }
}

/// Event channel.
///
/// This object recovers the event handling logic into a
/// channel polling logic. Now we do event processing in
/// rust async language.
pub struct Chan<E>
where
    E: Clone + 'static,
{
    inner: Rc<RefCell<Inner<E>>>,
    sender: UnboundedSender<E>,
    ready_pub: Pub<()>,
    ready_sub: Sub<()>,
}

impl<E> Chan<E>
where
    E: Clone + 'static,
{
    pub fn new() -> Self {
        let (ready_pub, ready_sub) = new_pubsub();
        let (send, recv) = unbounded();
        let inner = Inner {
            _send: send.clone(),
            recv: recv,
            head: None,
        };
        Self {
            inner: Rc::new(RefCell::new(inner)),
            sender: send.clone(),
            ready_pub: ready_pub,
            ready_sub: ready_sub,
        }
    }

    pub fn ready_wait(&self) -> Box<dyn ReadyWait> {
        Box::new(InnerReadyWait {
            rc: Rc::downgrade(&self.inner),
            sub: self.ready_sub.clone(),
        })
    }

    pub async fn recv(&mut self) -> E {
        self.inner.borrow_mut().recv().await
    }

    #[must_use = "Unregister when Ledge is dropped."]
    pub async fn connect(&mut self, sub: Sub<E>) -> Ledge<E> {
        let mut sender = self.sender.clone();
        let ready_pub = self.ready_pub.clone();
        sub.subscribe(Handler::new_async_option(async move |item| {
            sender.send(item).await.ok()?;
            ready_pub.publish(()).await;
            Some(())
        }))
        .await
    }
}

#[cfg(test)]
mod test {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::chan::Chan;
    use crate::testutil::TestFixture;
    use crate::{Mux, new_pubsub};

    #[test]
    fn test_normal() {
        let mut fixture = TestFixture::new();

        let (p1, s1) = new_pubsub::<()>();
        let (p2, s2) = new_pubsub::<usize>();
        let (p3, s3) = new_pubsub::<()>();

        let v1 = Rc::new(RefCell::new(0usize));
        let v2 = Rc::new(RefCell::new(0usize));
        let v3 = Rc::new(RefCell::new(0usize));

        let v1l = v1.clone();
        let v2l = v2.clone();
        let v3l = v3.clone();
        let _ = fixture.execute(async move {
            #[derive(Clone)]
            enum Branch {
                Ch1,
                Ch2,
                Ch3,
                Ch4,
            }
            let mut mux: Mux<Branch> = Mux::new();

            let mut ch1: Chan<()> = Chan::new();
            let _l1 = ch1.connect(s1).await;
            let _m1 = mux.mux(ch1.ready_wait(), Branch::Ch1).await;

            let mut ch2: Chan<usize> = Chan::new();
            let _l2 = ch2.connect(s2.clone()).await;
            let _m2 = mux.mux(ch2.ready_wait(), Branch::Ch2).await;

            let mut ch3: Chan<usize> = Chan::new();
            let l3 = ch3.connect(s2.clone()).await;
            let mut l3 = Some(l3);
            let m3 = mux.mux(ch3.ready_wait(), Branch::Ch3).await;
            let mut m3 = Some(m3);

            let mut ch4: Chan<()> = Chan::new();
            let _l4 = ch4.connect(s3.clone()).await;
            let _m4 = mux.mux(ch4.ready_wait(), Branch::Ch4).await;

            loop {
                match mux.poll().await {
                    Branch::Ch1 => {
                        ch1.recv().await;
                        *v1l.borrow_mut() += 1;
                    }
                    Branch::Ch2 => {
                        let d = ch2.recv().await;
                        *v2l.borrow_mut() += d;
                    }
                    Branch::Ch3 => {
                        let d = ch3.recv().await;
                        *v3l.borrow_mut() += d;
                    }
                    Branch::Ch4 => {
                        ch4.recv().await;
                        std::mem::drop(l3.take());
                        std::mem::drop(m3.take());
                    }
                }
            }
        });

        let p1c = p1.clone();
        fixture
            .execute(async move { p1c.publish(()).await })
            .assert_done();
        assert_eq!(*v1.borrow(), 1);
        assert_eq!(*v2.borrow(), 0);
        assert_eq!(*v3.borrow(), 0);

        let p2c = p2.clone();
        fixture
            .execute(async move { p2c.publish(2).await })
            .assert_done();
        assert_eq!(*v1.borrow(), 1);
        assert_eq!(*v2.borrow(), 2);
        assert_eq!(*v3.borrow(), 2);

        let p3c = p3.clone();
        fixture
            .execute(async move { p3c.publish(()).await })
            .assert_done();
        assert_eq!(*v1.borrow(), 1);
        assert_eq!(*v2.borrow(), 2);
        assert_eq!(*v3.borrow(), 2);

        let p2c = p2.clone();
        fixture
            .execute(async move { p2c.publish(3).await })
            .assert_done();
        assert_eq!(*v1.borrow(), 1);
        assert_eq!(*v2.borrow(), 5);
        assert_eq!(*v3.borrow(), 2);
    }
}
