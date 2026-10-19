//! Debug Adapter Protocol: framing, message types and an async client.
//!
//! This crate knows nothing about tuitor; it only speaks the protocol.

mod client;
mod codec;
pub mod events;
pub mod protocol;
pub mod requests;
pub mod types;

pub use client::{AdapterCommand, Client, Error, Incoming, Pending, Transport};
pub use codec::{CodecError, DapCodec};
pub use events::Event;
