//! Manipulation of the window subsystem itself.
//!
//! This module is for those setting up the window
//! subsystem only, which doesn't need to bother
//! most of `aeth::window` users, so it is moved
//! to a submodule.

use crate::manager::{MANAGER, ManagerInner, WakeUpEvent};
use crate::window::Windows;
use aeth_task::framework::Framework;
use aeth_task::ready_poll::ReadyPoll;
use aeth_task::{Handle, drain_foreground_loopback, foreground, run_foreground};
use std::cell::RefCell;
use std::process::exit;
use std::rc::Rc;
use winit::application::ApplicationHandler;
use winit::error::EventLoopError;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::WindowId;

struct SubsystemInner {
    _fx: Framework,
    manager: Rc<ManagerInner>,
    windows: Rc<RefCell<Windows>>,
    ready_poll: ReadyPoll,
}

impl Drop for SubsystemInner {
    fn drop(&mut self) {
        MANAGER.take();
    }
}

impl SubsystemInner {
    fn new(fx: Framework, ready_poll: ReadyPoll) -> Self {
        MANAGER.with_borrow(|v| {
            assert!(v.is_none(), "Window manager already initialized.");
        });
        let windows = Rc::new(RefCell::new(Windows::new()));
        let windows1 = windows.clone();
        let manager = Rc::new(ManagerInner::new(windows));
        MANAGER.replace(Some(Rc::downgrade(&manager)));
        Self {
            _fx: fx,
            manager: manager,
            windows: windows1,
            ready_poll: ready_poll,
        }
    }

    fn run(&mut self, event_loop: &dyn ActiveEventLoop) -> bool {
        loop {
            if self.manager.is_draining_foreground_loopback() {
                drain_foreground_loopback();
            }
            run_foreground();
            if self.ready_poll.ready() {
                return true;
            }
            if self.manager.drain_active_event_loop_jobs(event_loop) {
                continue;
            }
            break;
        }
        false
    }
}

struct Subsystem {
    inner: Option<SubsystemInner>,
}

impl Subsystem {
    fn run(&mut self, event_loop: &dyn ActiveEventLoop) -> Option<()> {
        if self.inner.as_mut()?.run(event_loop) {
            std::mem::drop(self.inner.take());
            event_loop.exit();
        }
        Some(())
    }

    fn window_event_option(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) -> Option<()> {
        let window_event_pub = self
            .inner
            .as_ref()?
            .windows
            .borrow_mut()
            .find_inner_by_window_id(window_id)?
            .window_event_pub();
        if window_event_pub.has_subscriber() {
            #[rustfmt::skip]
            foreground::spawn(async move {
                window_event_pub.publish(event).await
            }).detach();
        }
        self.run(event_loop);
        Some(())
    }

    fn about_to_wait_option(&mut self, event_loop: &dyn ActiveEventLoop) -> Option<()> {
        self.run(event_loop);
        self.inner.as_ref()?.windows.borrow().broadcast_refresh();
        Some(())
    }

    fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) -> Option<()> {
        let wakeup_event_pub = self.inner.as_ref()?.manager.wakeup_event_pub();
        if wakeup_event_pub.has_subscriber() {
            #[rustfmt::skip]
            foreground::spawn(async move {
                wakeup_event_pub.publish(WakeUpEvent).await
            }).detach();
        }
        self.run(event_loop);
        Some(())
    }
}

impl ApplicationHandler for Subsystem {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.run(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        self.window_event_option(event_loop, window_id, event);
    }

    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.about_to_wait_option(event_loop);
    }

    fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.proxy_wake_up(event_loop);
    }
}

/// Run the window subsystem.
///
/// The event loop exits when the ready poll
/// exits, which is usually the ready poll
/// for the main task.
///
/// Please notice this function is taken as a
/// point-of-no-return, it will drop the async
/// task framework and the ready poll when the
/// event loop exits. The program exits directly
/// as the main eventloop exits.
///
/// This means any `Drop` logic may never run.
/// Therefore its recommended never to allocate
/// any resource whose recycling is mandatory
/// outside the main async task, do them in the
/// main async task instead.
pub fn run(fx: Framework, ready_poll: ReadyPoll) -> Result<(), EventLoopError> {
    let inner = SubsystemInner::new(fx, ready_poll);
    let subsystem = Subsystem { inner: Some(inner) };
    // TODO: handle them with crashpad module?
    let event_loop = EventLoop::builder().build()?;
    event_loop.run_app(subsystem)?;
    exit(0)
}
