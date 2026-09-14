//! The desktop session: what it is, what the application persists about
//! itself, and the one file that decides whether Fermix opens at login.

pub mod autostart;
pub mod build;
pub mod desktop;
pub mod state;

pub use desktop::{DesktopFacts, DesktopSession};
pub use state::WindowState;
