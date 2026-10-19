//! The chat's wire to the daemon, `companion.sock`: its codec, the timeline reducer, and the
//! session that runs it. Named apart from `companion`, which is the pet.

pub mod grants;
pub mod protocol;
pub mod session;
pub mod timeline;
