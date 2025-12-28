//! Async-based task framework.
//!
//! The well-known tokio runtime is general async
//! runtime, and it is good. In fact, we are using
//! tokio as a component of the task framework.
//! Howver, shear tokio is not well-suited for
//! a UI applications:
//!
//! First of all, we want to prioritize UI
//! rendering and response to user interactions,
//! this means we shouldn't treat all tasks in
//! the application equally. However, tokio views
//! all tasks equally, there's no difference
//! between a task in respond to user interaction
//! and a task performing massive computation.
//!
//! What's worse, a UI application usually has a
//! UI thread running an UI event poll, which is
//! the place triggering rendering and interaction
//! logic. Meantime, tokio is a work-stealing
//! async runtime, which might steal the massive
//! computation work to the UI thread if we are
//! running tokio on the UI thread.
//!
//! Actually, separating thread is not enough,
//! there's case when the UI thread and tokio
//! thread running massive task are scheduled
//! on the same CPU, which might also downgrade
//! the performance of UI thread as it relies
//! on timer interrupt to reclaim the CPU. We
//! will also want to set the CPU affinities
//! whenever it's possible.
//!
//! This is why the task framework comes in. We
//! provides facilities for configuring thread
//! model for UI applications, as well as
//! specifying type of work, and spawning them
//! on the correct threads.

#[doc(hidden)]
pub mod spawner;
#[rustfmt::skip]
pub use spawner::{
    Handle, Type,
    spawn_foreground,
    dispatch_background,
    dispatch_foreground,
    drain_foreground_loopback,
    run_foreground,
    current_type,
};

pub mod foreground {
    //! Task primitives in foreground thread flavour.
    //!
    //! This allows the function which is dedicated
    //! to a foreground thread to directly write
    //! `use aeth::task::foreground`, and then
    //! `foreground::spawn` or `foreground::dispatch`.
    pub use super::dispatch_background as dispatch;
    pub use super::spawn_foreground as spawn;

    /// Assert current thread is a foreground thread.
    pub fn assert() {
        assert!(
            super::current_type() == super::Type::Foreground,
            "Not running on a foreground thread.",
        );
    }
}

pub mod background {
    //! Task primitives in background thread flavour.
    //!
    //! This allows the function which is dedicated
    //! to background threads to directly write
    //! `use aeth::task::background`, and then
    //! `background::spawn` or `background::loopback`.
    pub use super::dispatch_background as spawn;
    pub use super::dispatch_foreground as loopback;

    /// Assert current thread is a background thread.
    pub fn assert() {
        assert!(
            super::current_type() == super::Type::Background,
            "Not running on a background thread.",
        );
    }
}

pub mod framework;

pub mod ready_poll;

#[cfg(test)]
mod test;
