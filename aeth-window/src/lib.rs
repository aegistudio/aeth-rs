//! The window subsystem.
//!
//! This is the window subsystem adapted for
//! aeth-rs. It adapts [`winit`] to make it
//! work with the async task framework, and
//! drives the main UI loop. The user should
//! refer to [`winit`] for more usage information.
//!
//! Please notice due to the implementation
//! of certain platform (e.g. iOS), according
//! to the [`winit`] authors, the event loop
//! may be a point-of-no-return, so this will
//! be the premise of designing this module.

#[doc(hidden)]
pub mod window;
pub use window::Window;

#[doc(hidden)]
pub mod access_winit_active_event_loop;
#[rustfmt::skip]
pub use access_winit_active_event_loop::{
    AccessWinitActiveEventLoop,
    AccessWinitActiveEventLoopExt,
    forward_winit_active_event_loop_method,
};

#[doc(hidden)]
pub mod access_winit_window;
#[rustfmt::skip]
pub use access_winit_window::{
    AccessWinitWindow,
    AccessWinitWindowExt,
    forward_winit_window_method,
};

#[doc(hidden)]
pub mod manager;
#[rustfmt::skip]
pub use manager::{
    Manager, manager,
    WakeUpEvent, DeviceEvent,
};

pub mod subsystem;
