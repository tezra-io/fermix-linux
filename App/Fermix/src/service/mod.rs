//! The packaged command line, as five typed operations.
//!
//! Everything that has to work while the daemon is stopped goes through here,
//! and nothing else in the application spawns a process.

pub mod runner;
pub mod types;

pub use runner::{ServiceError, ServiceResult, ServiceRunner};
pub use types::{ActionResult, Alignment, InstallOutcome, ServiceStatus};
