use futures::FutureExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::executor::{LocalPool, LocalSpawner};
use futures::future::{BoxFuture, RemoteHandle};
use futures::lock::Mutex;
use futures::task::{LocalSpawnExt, SpawnExt};
use std::cell::RefCell;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::runtime::Runtime;

/// Task handle trait.
///
/// This handle is used to receive result from the
/// task, as well as controlling the task like
/// cancelling and detaching it:
///
/// - Awaiting this handle receives the result.
/// - Dropping this handle cancels the task.
/// - Calling `detach` consumes and detaches the task.
pub trait Handle<T>: Future<Output = T>
where
    T: 'static,
{
    fn detach(self);
}

struct TaskHandle<T>
where
    T: 'static,
{
    handle: RemoteHandle<T>,
}

impl<T> TaskHandle<T>
where
    T: 'static,
{
    pub fn new(handle: RemoteHandle<T>) -> Self {
        Self { handle }
    }
}

impl<T> Future for TaskHandle<T>
where
    T: 'static,
{
    type Output = T;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.handle.poll_unpin(cx)
    }
}

impl<T> Handle<T> for TaskHandle<T>
where
    T: 'static,
{
    fn detach(self) {
        self.handle.forget();
    }
}

pub(crate) struct ForegroundSpawner {
    pub(crate) local_pool: RefCell<LocalPool>,
    pub(crate) local_spawner: LocalSpawner,
    pub(crate) tokio_runtime: Runtime,
    pub(crate) _loopback_send: UnboundedSender<BoxFuture<'static, ()>>,
    pub(crate) loopback_recv: Mutex<UnboundedReceiver<BoxFuture<'static, ()>>>,
}

impl ForegroundSpawner {
    fn spawn_foreground<F, T>(&self, future: F) -> TaskHandle<T>
    where
        F: Future<Output = T> + 'static,
        T: 'static,
    {
        let handle = self.local_spawner.spawn_local_with_handle(future).unwrap();
        TaskHandle::new(handle)
    }

    fn spawn_background<F, T>(&self, future: F) -> TaskHandle<T>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        // XXX: The foreground thread does not have tokio
        // context initialized, so we will need to use
        // the tokio_runtime handle to create the task.
        let (remote, handle) = future.remote_handle();
        let _ = self.tokio_runtime.spawn(remote);
        TaskHandle::new(handle)
    }

    fn drain_foreground_loopback(&self) -> Option<()> {
        let mut loopback_recv = self.loopback_recv.try_lock()?;
        loop {
            self.local_spawner
                .spawn(loopback_recv.try_next().ok()??)
                .unwrap();
        }
    }

    fn run_foreground(&self) {
        // XXX: This is okay since there's only one
        // foreground thread.
        self.local_pool.borrow_mut().run_until_stalled();
    }
}

pub(crate) struct BackgroundSpawner {
    pub(crate) loopback_send: UnboundedSender<BoxFuture<'static, ()>>,
}

impl BackgroundSpawner {
    fn spawn_background<F, T>(&self, future: F) -> TaskHandle<T>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        // XXX: Every background thread will enter the
        // tokio context, so we can use tokio::spawn.
        let (remote, handle) = future.remote_handle();
        let _ = tokio::spawn(remote);
        TaskHandle::new(handle)
    }

    fn spawn_foreground<F, T>(&self, future: F) -> TaskHandle<T>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let (remote, handle) = future.remote_handle();
        // XXX: note that the main thread holds one copy
        // of the sender and receiver, so there'll never
        // be an error sending it.
        self.loopback_send.unbounded_send(Box::pin(remote)).unwrap();
        TaskHandle::new(handle)
    }
}

/// Enumeration of the task / thread types.
///
/// Since we have a well-defined primitives of creating
/// tasks of different kind and using async / await
/// language to control them, for convenience, each task
/// must only be associated with one type, and each
/// thread is only allowed to execute one type of task,
/// so there's no ambiguity while querying the type.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Type {
    Uninit,
    Foreground,
    Background,
}

pub(crate) enum Spawner {
    Uninit,
    Foreground(ForegroundSpawner),
    Background(BackgroundSpawner),
}

thread_local! {
    pub(crate) static SPAWNER: RefCell<Spawner> = RefCell::new(Spawner::Uninit);
}

impl Spawner {
    fn panic_uninit<T>(&self) -> T {
        panic!("Task framework not initialized");
    }

    fn foreground(&self) -> &ForegroundSpawner {
        match self {
            Spawner::Foreground(fg) => fg,
            Spawner::Uninit => self.panic_uninit(),
            _ => {
                panic!("Not on foreground thread");
            }
        }
    }

    fn which_type(&self) -> Type {
        match self {
            Spawner::Foreground(_) => Type::Foreground,
            Spawner::Background(_) => Type::Background,
            Spawner::Uninit => Type::Uninit,
        }
    }
}

/// Spawn a foreground task from foreground thread.
///
/// This function can only be called from the foreground
/// thread. Actually, if a background thread want to run
/// a task in foreground thread, the Future must be Send,
/// while this method does not enforces it.
#[must_use = "Dropping the Handle is equivalent to canceling the future."]
pub fn spawn_foreground<F, T>(future: F) -> impl Handle<T>
where
    F: Future<Output = T> + 'static,
    T: 'static,
{
    SPAWNER.with_borrow(|w| w.foreground().spawn_foreground(future))
}

/// Spawns a background task.
///
/// This function can either be called from a foreground
/// thread or a background thread, and the future must be
/// Send since we are using tokio as the task executor,
/// which will possibly move tasks between threads.
#[must_use = "Dropping the Handle is equivalent to canceling the future."]
pub fn dispatch_background<F, T>(future: F) -> impl Handle<T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    SPAWNER.with_borrow(|w| {
        // WARN: calling tokio::spawn requires tokio context, which
        // is available on background thread, while we must explicitly
        // call tokio_runtime.spawn on foreground thread.
        match w {
            Spawner::Foreground(fg) => fg.spawn_background(future),
            Spawner::Background(bg) => bg.spawn_background(future),
            Spawner::Uninit => w.panic_uninit(),
            #[allow(unreachable_patterns)]
            _ => {
                panic!("Not on foreground or background thread.")
            }
        }
    })
}

/// Spawns a foreground task.
///
/// This function can either be called from a foreground
/// thread or a background thread, and the future must be
/// Send since the future is sent to the foreground thread
/// when dispatching from a background thread. The result
/// will also need to be moved to the background thread.
#[must_use = "Dropping the Handle is equivalent to canceling the future."]
pub fn dispatch_foreground<F, T>(future: F) -> impl Handle<T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    SPAWNER.with_borrow(|w| match w {
        Spawner::Foreground(fg) => fg.spawn_foreground(future),
        Spawner::Background(bg) => bg.spawn_foreground(future),
        Spawner::Uninit => w.panic_uninit(),
        #[allow(unreachable_patterns)]
        _ => {
            panic!("Not on foreground or background thread.")
        }
    })
}

/// Drains the foreground tasks dispatched from
/// background threads and push them to the task queue
/// of foreground thread.
///
/// This function must be called from the foreground
/// thread. It's non-blocking, which means it returns
/// immediately after pushing the tasks.
pub fn drain_foreground_loopback() {
    SPAWNER.with_borrow(|w| {
        w.foreground().drain_foreground_loopback();
    })
}

/// Runs the foreground tasks until stalled.
///
/// This function must only be called from the
/// foreground thread. It will return as soon as
/// all foreground tasks are stalled.
pub fn run_foreground() {
    SPAWNER.with_borrow(|w| {
        w.foreground().run_foreground();
    })
}

/// Returns the current task / thread type.
pub fn current_type() -> Type {
    SPAWNER.with_borrow(Spawner::which_type)
}
