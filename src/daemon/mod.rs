//! Daemon: node lifecycle, peer management and agent-level semantics.
//!
//! `tailcat-node` owns node lifecycle, peer management and
//! agent-level semantics. Tailcat owns encrypted connectivity.

pub mod lifecycle;
pub mod manager;

pub use lifecycle::LifecycleEvent;
pub use manager::{Daemon, NodeInfo};
