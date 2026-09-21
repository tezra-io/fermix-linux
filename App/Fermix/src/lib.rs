//! Fermix for Linux.
//!
//! A GTK4 and libadwaita client for the Fermix engine. It contains no engine,
//! writes no unit, reads no configuration file and holds no secret code:
//! everything it shows and changes goes over `daemon.sock` through the
//! management protocol, and everything that must work while the daemon is
//! stopped goes through five typed operations of the packaged command line.
//!
//! The library target exists so the application binary and the fixture daemon
//! share one framing, one client and one vendored contract, and so the tests
//! can reach every layer without a display.

pub mod actions;
pub mod app;
#[cfg(debug_assertions)]
pub mod capture;
pub mod cli;
pub mod copy;
pub mod fixtures;
pub mod management;
pub mod metrics;
pub mod models;
pub mod motion;
pub mod paths;
pub mod registry;
pub mod runtime;
pub mod service;
pub mod session;
pub mod testing;
pub mod tray;
pub mod ui;
pub mod window;
