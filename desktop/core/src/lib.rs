//! The Fermix desktop app's connection to the daemon. Pure Rust and blocking:
//! the app runs each call off the GTK main thread, and this crate tests on any host.

pub mod acp;
pub mod capabilities;
pub mod chat;
pub mod companion;
pub mod doctor;
pub mod frame;
pub mod job;
pub mod ledger;
pub mod logs;
pub mod management;
pub mod markdown;
pub mod mascot;
pub mod model;
pub mod onboarding;
pub mod overview;
pub mod plugins;
pub mod providers;
pub mod realtime;
pub mod service;
pub mod settings;
pub mod view;
pub mod voice;
