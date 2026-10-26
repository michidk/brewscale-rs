//! Vendor-neutral coffee-scale API: data types, the [`Protocol`] trait that
//! scale drivers implement, the [`Transport`] traits that BLE backends
//! implement, and the [`Scale`] driver tying them together. Contains no vendor
//! code; protocols live in their own crates (e.g. `brewscale-bookoo`).
//!
//! `no_std`, no heap, no executor, no BLE stack. Protocol decoding
//! ([`Protocol`]) is pure functions over byte slices; Bluetooth is reached only
//! through the [`Transport`]/[`Link`] traits, implemented by backend crates
//! (`brewscale` for desktop, `brewscale-trouble` for embedded).

#[cfg(feature = "std")]
extern crate std;

#[cfg(any(test, feature = "mock"))]
pub mod mock;
mod protocol;
mod scale;
mod transport;

pub use protocol::{
    AutoBrewEvent, AutoBrewState, Capabilities, Command, DecodeError, Detect, EncodeError, Event, Frame, Measurement,
    Protocol, WeightUnit,
};
pub use scale::{find, Error, Scale};
pub use transport::{Advertisement, ErrorKind, Link, Notifications, Transport, TransportError};
