//! Creator-side V2 send-dummy implementation.
//!
//! This crate is library-only so any Conduit service binary can act as a creator when
//! its local admin listener is asked to synthesize a test payload.

pub mod client;
pub mod error;
pub mod session;
pub mod upload;

pub use client::{CreatorClient, SendDummyResult};
pub use error::CreatorError;
pub use session::CreatorSession;
pub use upload::{CreatorBridgeRequest, CreatorBridgeResponse};
