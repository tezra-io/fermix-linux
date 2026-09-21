//! The tray item: what it draws, what it offers, and how it reaches the host.
//!
//! Split so that the decisions are testable without a desktop. `state` maps
//! what Home already knows onto the three marks the icon can draw, `menu` is
//! the rows as a value, and the D-Bus side performs those decisions and makes
//! none of its own.
//!
//! Two facts about this feature are worth stating where they cannot be missed,
//! because both failures look identical to "it works":
//!
//! 1. A host may accept our registration and still never draw the icon. The
//!    GNOME AppIndicator extension treats an item with no `com.canonical.
//!    dbusmenu` object as not ready and silently drops it (appIndicator.js
//!    line 114, measured on Pop!_OS 22.04). So registering is not being shown,
//!    and nothing in this module may claim the icon is visible on the strength
//!    of a successful registration.
//! 2. Some desktops have no tray at all. Fedora's GNOME ships no extension for
//!    it. There the absence of an icon is correct behaviour, not a fault, and
//!    the application must still quit when its last window closes -- otherwise
//!    the user is left with a process they cannot see and cannot reach.

pub mod controller;
pub mod dbusmenu;
pub mod item;
pub mod menu;
pub mod state;
