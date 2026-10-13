//! `overview.get`: the running daemon's own account of itself, for Home's
//! details (spec §1.2). Readiness is not read from here: `setup.state.get`
//! owns it (spec G1).

use crate::management::{CallError, Management};
use crate::view::channel_title;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Overview {
    pub daemon: DaemonFacts,
    pub channels: Vec<ChannelFacts>,
    pub agents: Agents,
    pub capabilities: Capabilities,
    /// Voice as the daemon booted it. Older engines leave it out.
    #[serde(default)]
    pub realtime: Option<RealtimeFacts>,
}

/// `overview.realtime`: whether voice is on and whether its socket is open.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RealtimeFacts {
    pub enabled: bool,
    /// `disabled`, `setup_required`, `degraded` or `ready`.
    pub status: String,
    pub socket_alive: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DaemonFacts {
    /// The daemon's own monotonic uptime, which leaves out time suspended.
    pub uptime_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChannelFacts {
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Agents {
    pub main: MainAgent,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MainAgent {
    pub active_conversations: u64,
    pub pending_conversations: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Capabilities {
    pub builtin: u64,
    pub skill: u64,
    pub mcp: u64,
}

impl Management {
    pub fn overview(&self) -> Result<Overview, CallError> {
        let answer = self.call("overview.get", json!({}))?;
        serde_json::from_value(answer).map_err(|e| {
            CallError::Protocol(format!("overview.get answered an unexpected shape: {e}"))
        })
    }
}

pub fn tools_count(overview: &Overview) -> u64 {
    overview.capabilities.builtin + overview.capabilities.mcp
}

/// The enabled channels by name; the editor connection reads as "Editors".
pub fn channels_line(overview: &Overview) -> String {
    let names: Vec<&str> = overview
        .channels
        .iter()
        .filter(|c| c.enabled)
        .map(|c| channel_title(&c.name))
        .collect();
    if names.is_empty() {
        return "None".into();
    }
    names.join(", ")
}

/// A duration in its two largest units: "3 days, 1 hour", "14 minutes".
pub fn duration_words(ms: u64) -> String {
    let minutes = ms / 60_000;
    let units = [
        (minutes / (24 * 60), "day"),
        ((minutes / 60) % 24, "hour"),
        (minutes % 60, "minute"),
    ];
    let parts: Vec<String> = units
        .iter()
        .skip_while(|(n, _)| *n == 0)
        .take(2)
        .filter(|(n, _)| *n > 0)
        .map(|(n, unit)| format!("{n} {unit}{}", if *n == 1 { "" } else { "s" }))
        .collect();
    if parts.is_empty() {
        return "less than a minute".into();
    }
    parts.join(", ")
}
