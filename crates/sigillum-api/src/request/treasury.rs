//! Treasury policy request contracts.

use serde::{Deserialize, Serialize};

/// Destination allowlist entry supplied by an operator.
///
/// Kept separate from the response-side `TreasuryAllowedDestination` so the
/// request surface can stay permissive (raw, un-normalized addresses) while
/// stored policy entries are always normalized by the daemon.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryAllowedDestinationInput {
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Replace the local treasury policy with operator-defined guardrails.
///
/// Updates are whole-document: the daemon normalizes and dedupes the
/// destination allowlist, validates the caps, and preserves the original
/// `created_at_unix`. `require_simulation` defaults to true when omitted so
/// loosening that gate is always an explicit choice.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryPolicyUpdateRequest {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_destinations: Vec<TreasuryAllowedDestinationInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_step_native_wei_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_plan_native_wei_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_simulation: Option<bool>,
}

/// Allocate a fresh purpose-labeled receive address from a wallet profile.
///
/// The daemon derives the next unused receive index for the profile locally
/// from its xpub — no network calls — so each counterparty/purpose gets an
/// address that is not linkable to previously handed-out ones.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryReceiveAllocateRequest {
    pub wallet_profile: String,
    pub purpose: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Retire an active receive allocation and issue the next index for the same
/// wallet profile, carrying over its purpose and label.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryReceiveRotateRequest {
    pub allocation_id: String,
}
