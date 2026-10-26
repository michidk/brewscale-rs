//! Protocol-agnostic BLE coffee-scale API for desktop and embedded systems.
//!
//! This crate contains the core traits and driver, the BOOKOO protocol, the
//! optional desktop `btleplug` backend, and the optional embedded TrouBLE
//! backend. Most applications only need this one crate.
#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "bookoo")]
pub mod bookoo;
mod core;
#[cfg(feature = "trouble")]
pub mod trouble;

mod any;
pub use any::{AnyProtocol, AnyScale};
pub use core::*;

#[cfg(feature = "bookoo")]
pub use bookoo as protocols;

#[cfg(feature = "btleplug")]
mod btleplug_transport;
#[cfg(feature = "btleplug")]
pub use btleplug_transport::{BtleplugLink, BtleplugNotifications, BtleplugTransport, Error as BtleplugError};
