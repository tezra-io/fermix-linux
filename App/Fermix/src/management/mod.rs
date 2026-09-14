//! The management plane: one framed client over one Unix socket.
//!
//! The daemon decides and the interface renders. Everything under this module
//! decodes what the daemon published and refuses what the contract forbids; no
//! module here composes a sentence a person reads.

pub mod client;
pub mod contract;
pub mod errors;
pub mod framing;
pub mod health;
pub mod transport;
pub mod types;
pub mod vocabulary;

pub use client::{Issued, ManagementClient};
pub use errors::{ManagementError, TransportError, WireError};
pub use vocabulary::{ConfigCondition, Gap, Readiness, Refusal};
