//! Progress and status types for FIDO2 quorum operations.
//!
//! [`QuorumEvent`] provides a structured stream of events during multi-key
//! unlock flows, enabling CLI progress bars and daemon status reporting.
//! [`Fido2Status`] and [`KeyInfo`] expose safe, deniability-preserving
//! views of the registered key set.

use serde::Serialize;

/// Events emitted during quorum authentication for progress tracking.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum QuorumEvent {
    #[serde(rename = "round_start")]
    RoundStart { round: usize, total: usize },

    #[serde(rename = "round_complete")]
    RoundComplete {
        round: usize,
        total: usize,
        key_label: String,
    },

    #[serde(rename = "swap_keys")]
    SwapKeys { round: usize, total: usize },

    #[serde(rename = "cascading_unlock")]
    CascadingUnlock { compartments_unlocked: usize },

    #[serde(rename = "unlocked")]
    Unlocked,

    #[serde(rename = "error")]
    Error { message: String },
}

/// Summary of FIDO2 subsystem state. Deliberately reveals NO compartment
/// information (deniability). Compartment details are only available after
/// unlock, via the discovered `CompartmentMeta` structs.
#[derive(Debug, Clone, Serialize)]
pub struct Fido2Status {
    pub enabled: bool,
    pub key_count: usize,
}

/// Public info about a registered key (no secrets exposed).
/// `shard_count` is always SHARD_SLOTS — reveals nothing about compartments.
#[derive(Debug, Clone, Serialize)]
pub struct KeyInfo {
    pub label: String,
    pub credential_id_short: String,
    pub registered_at: String,
}
