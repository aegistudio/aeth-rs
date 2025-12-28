//! Async based event primitives.
//!
//! This crate builds event primitives on the top
//! of the async rust language.
//!
//! On the bottom, the `Handler`, `Pub`, `Sub`
//! and `Ledge` builds up a conventional eventing
//! system, but is now async compatible.
//!
//! On the top of them, there're two ways of
//! viewing the event processing:
//!
//! - Horizontally, we can gather the events from
//!   different sources (represented by `Sub`)
//!   into a single queue / channel, the `Chan`.
//!   So that we just need to poll a single
//!   channel in order to get notified by multiple
//!   sources. This also recovers the event handling
//!   logic to channel receiving logic.
//! - Vertically, we can multiplex different
//!   sources by their ready states (represented
//!   by `ReadyWait`), and discriminate them by
//!   associating them with pre-defined values.
//!   So that we just need to poll a single
//!   multiplexer `Mux` to see which source is
//!   ready. This also recovers the event handling
//!   logic to futures selecting logic, but our
//!   version is dynamic and persistent.
//!
//! The crates assert the eventing happens in a
//! single-threaded context.

#[doc(hidden)]
pub mod core;
pub use core::{AsyncHandlerTrait, Handler};

#[doc(hidden)]
pub mod pubsub;
pub use pubsub::{Ledge, Pub, Sub, new_pubsub};

#[doc(hidden)]
pub mod mux;
pub use mux::{Mux, MuxPollFuture, Muxing, ReadyWait};

#[doc(hidden)]
pub mod chan;
pub use chan::Chan;

#[doc(hidden)]
#[cfg(test)]
pub(crate) mod testutil;
