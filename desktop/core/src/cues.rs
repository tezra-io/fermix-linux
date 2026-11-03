//! Cues (M60 cues): outside items that can wake a task, from feeds, streams, webhooks and the
//! owner's scripts, and what the daemon lets a task do about them. Every state, action, load and
//! approval word is the daemon's; this module decodes them, decides which controls a row draws
//! from the published action ids, holds the source and mandate forms as values, and holds every
//! sentence the Cues pane adds of its own.

pub mod calls;
pub mod form;
pub mod mandate;
pub mod view;
pub mod wire;
pub mod words;
