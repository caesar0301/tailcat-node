//! Network backend abstraction.
//!
//! `tailcat-node` delegates encrypted connectivity (WireGuard,
//! magicsock, DERP, NAT traversal, P2P) to Tailcat. This module
//! exposes a [`Backend`] trait so the daemon is agnostic to whether
//! we invoke the `tailcat` binary or embed the Tailcat library.

pub mod tailcat;

pub use tailcat::{
    build_backend, ping_result_to_status, tailcat_available, Backend, BackendPeerStatus, PingResult,
};
