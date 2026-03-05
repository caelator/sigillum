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

    #[serde(rename = "unlocked")]
    Unlocked,

    #[serde(rename = "error")]
    Error { message: String },
}

/// Summary of FIDO2 subsystem state.
#[derive(Debug, Clone, Serialize)]
pub struct Fido2Status {
    pub enabled: bool,
    pub key_count: usize,
    pub quorum_threshold: usize,
    pub unlock_method: String,
}

/// Public info about a registered key (no secrets exposed).
#[derive(Debug, Clone, Serialize)]
pub struct KeyInfo {
    pub label: String,
    pub credential_id_short: String,
    pub registered_at: String,
}
