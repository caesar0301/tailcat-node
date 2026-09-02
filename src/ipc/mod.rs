//! Local IPC API over a Unix socket.
//!
//! Don't make every component invoke the CLI. Give the daemon a
//! local IPC API that the Agent Runtime can consume directly.
//!
//! API:
//! ```text
//! GET  /v1/node
//! GET  /v1/peers
//! GET  /v1/peers/:id
//! POST /v1/peers
//! DELETE /v1/peers/:id
//! POST /v1/connect/:id
//! POST /v1/disconnect/:id
//! GET  /v1/services
//! GET  /v1/status
//! ```
//!
//! Responses are JSON. Errors use `{"error": "..."}`.

pub mod client;
pub mod server;

pub use client::IpcClient;
pub use server::serve;
