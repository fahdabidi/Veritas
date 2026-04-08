//! # GBN Protocol
//!
//! Shared protocol definitions for the Global Broadcast Network.
//!
//! This crate contains **no executable logic** — only type definitions, trait
//! interfaces, wire format structs, and protocol constants that all GBN
//! components depend on. It is the "constitution" of the GBN protocol.
//!
//! In the production multi-repo layout, this crate will be its own repository
//! (`gbn-protocol`) and consumed by all other components via version-pinned
//! git dependency.

pub mod chunk;
pub mod crypto;
pub mod manifest;
pub mod error;
pub mod dht;
pub mod onion;

/// Protocol version constant. Nodes exchange this during handshake
/// and must agree on a compatible version to communicate.
pub const PROTOCOL_VERSION: u32 = 1;

/// Default chunk size for MCN video chunking (1 MB).
pub const DEFAULT_MCN_CHUNK_SIZE: usize = 1024 * 1024;

/// Default chunk size for GDS storage shards (4 MB).
pub const DEFAULT_GDS_CHUNK_SIZE: usize = 4 * 1024 * 1024;

/// Reed-Solomon default parameters.
pub const DEFAULT_RS_DATA_SHARDS: usize = 14;
pub const DEFAULT_RS_PARITY_SHARDS: usize = 6;
pub const DEFAULT_RS_TOTAL_SHARDS: usize = DEFAULT_RS_DATA_SHARDS + DEFAULT_RS_PARITY_SHARDS;

/// Minimum relay hops for onion routing (constitutional invariant).
pub const MIN_RELAY_HOPS: usize = 3;
