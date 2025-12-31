//! The window subsystem.
//!
//! This is the window subsystem adapted for
//! aeth-rs. It adapts the winit to make it
//! work with the async task framework, and
//! drives the main UI loop. The user should
//! refer to winit for more usage information.
//!
//! Please notice due to the implementation
//! of certain platform (e.g. iOS), according
//! to the winit authors, the event loop may
//! be a point-of-no-return, so this will be
//! the premise of designing this module.

#[doc(hidden)]
pub mod window;
pub use window::Window;

#[doc(hidden)]
pub mod manager;
pub use manager::{Manager, WakeUpEvent, manager};

pub mod subsystem;
