//! The widgets the application arranges for itself.
//!
//! Two labels in a toolkit style, and the leading artwork slot. Nothing here
//! draws a container: a mark is the vendor's own file at the size the row
//! aligns on, and the pill is a label.

pub mod checklist_row;
pub mod mark;
pub mod status_pill;

pub use status_pill::status_pill;
