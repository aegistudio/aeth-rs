//! Async-based event primitives.
//!
//! This crate builds event primitives on the top
//! of the async rust language.
//!
//! On the bottom, the [`Handler`], [`Pub`], [`Sub`]
//! and [`Ledge`] builds up a classical eventing
//! system, but is now async compatible.
//!
//! On the top of them, there're two ways of
//! viewing the event processing:
//!
//! - Horizontally, we can gather the events from
//!   different sources (represented by [`Sub`])
//!   into a single queue / channel, the [`Chan`].
//!   So that we just need to poll a single
//!   channel in order to get notified by multiple
//!   sources. This also recovers the event handling
//!   logic to channel receiving logic. For the
//!   case of creating one [`Chan`] dedicated to
//!   the [`Sub`], it's also good to see
//!   [`SubscriberExt::chan`].
//! - Vertically, with the help of [`aeth_mux::Mux`],
//!   we can multiplex multiple channels [`Chan`]
//!   into a single [`aeth_mux::Mux::poll`],
//!   and discriminate them by associating them
//!   with a pre-defined key, using
//!   [`MuxChanExt::mux_chan`].
//!
//! The crates assert the eventing happens in a
//! single-threaded context.

#[doc(hidden)]
pub mod handler;
pub use handler::Handler;

#[doc(hidden)]
pub mod event;
#[rustfmt::skip]
pub use event::{
    Publisher, Subscriber,
    Ledge, Pub, Sub, pubsub,
};

pub mod filter;

pub mod chan;
#[doc(inline)]
#[rustfmt::skip]
pub use chan::{
    Chan, Channel, ChannelExt,
    WaitChan, WaitChannel, WaitChannelExt, Waiting,
    MuxChanExt,
};

#[doc(hidden)]
pub mod event_ext;
#[rustfmt::skip]
pub use event_ext::{
    PublisherDyn, SubscriberDyn,
    PublisherExt, SubscriberExt,
};

pub mod prelude {
    //! Prelude to making life easy for
    //! [this module](crate) users.
    //!
    //! The prelude will import the traits to make the
    //! trait methods visible to the rust compiler, and
    //! then clobber them immediately. Therefore, user
    //! must explicitly import the type they need.
    pub use crate::Channel as _;
    pub use crate::ChannelExt as _;
    pub use crate::MuxChanExt as _;
    pub use crate::Publisher as _;
    pub use crate::PublisherDyn as _;
    pub use crate::PublisherExt as _;
    pub use crate::Subscriber as _;
    pub use crate::SubscriberDyn as _;
    pub use crate::SubscriberExt as _;
    pub use crate::WaitChannel as _;
    pub use crate::WaitChannelExt as _;
}

#[doc(hidden)]
#[cfg(test)]
pub(crate) mod testutil;
