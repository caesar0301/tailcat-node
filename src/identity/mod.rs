//! Identity management.
//!
//! The logical node identity (e.g. `agent-001`) is separate from the
//! Tailcat cryptographic identity (public/private key pair). The
//! network identity can rotate without changing the logical id.
//!
//! `identity.key` holds the persistent Tailcat private key and must
//! survive daemon restart, machine reboot, and tailcat-node upgrade.

pub mod store;
pub mod types;

pub use store::generate;
pub use store::IdentityStore;
pub use types::{Identity, NodeIdentity};
