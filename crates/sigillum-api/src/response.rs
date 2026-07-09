//! JSON response types for the Sigillum HTTP daemon API.
//!
//! Every response struct derives `Serialize`, `Deserialize`, `PartialEq`, and `Eq`
//! so that client code can round-trip through JSON and compare values in tests.
//!
//! ## Compartment ID naming convention
//!
//! Structs that *are* a compartment (e.g. [`UnlockedCompartment`], [`CompartmentInfo`])
//! use the short field name `id` because the context is unambiguous.  Structs that
//! *reference* a compartment among other resources (e.g. [`ActiveCompartment`],
//! [`SwitchCompartmentResponse`]) use the qualified name `compartment_id` to
//! disambiguate.  This distinction is part of the stable JSON wire protocol.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sigillum_core::{ETHEREUM_STEALTH_SCHEME_ID, SnapshotSummary};

use crate::request::Eip1559Fees;

// ── Lifecycle ───────────────────────────────────

/// Standard error envelope returned for non-2xx responses.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorResponse {
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

/// Summary of the currently active compartment within a session.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveCompartment {
    pub compartment_id: usize,
    pub compartment_label: String,
    pub api_key_count: usize,
    pub secret_count: Option<usize>,
}

/// Lightweight descriptor for a compartment returned during unlock.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnlockedCompartment {
    pub id: usize,
    pub label: String,
    pub threshold: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passphrase_mode: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusResponse {
    pub locked: bool,
    pub initialized: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_compartment: Option<ActiveCompartment>,
    #[serde(default)]
    pub unlocked_compartments: Vec<UnlockedCompartment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fido2: Option<Fido2StatusResponse>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockResponse {
    pub status: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRevokeResponse {
    pub status: String,
    pub requires_reauth: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnlockResponse {
    pub status: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cascading: Option<bool>,
    pub session_token: String,
    #[serde(default)]
    pub unlocked_compartments: Vec<UnlockedCompartment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_compartment_id: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilitySessionResponse {
    pub status: String,
    pub session_token: String,
    pub scopes: Vec<String>,
    pub expires_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BiometricChallengeResponse {
    pub challenge_id_hex: String,
    pub nonce_hex: String,
    pub expires_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BiometricEnrollResponse {
    pub status: String,
    pub compartment_id: usize,
    pub fingerprint_hex: String,
    pub vault_key_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GenericStatusResponse {
    pub status: String,
}

// ── Secrets ────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyListResponse {
    pub keys: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyValueResponse {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyMutationResponse {
    pub status: String,
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PushResponse {
    pub status: String,
    pub from: usize,
    pub to: usize,
    pub key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretResolveValue {
    pub env_name: String,
    pub reference: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretResolveBatchResponse {
    pub values: Vec<SecretResolveValue>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GenerateStoreResponse {
    pub status: String,
    pub key: String,
    pub value: String,
    pub kind: String,
}

// ── Compartments ────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompartmentInfo {
    pub id: usize,
    pub label: String,
    pub threshold: usize,
    pub passphrase_mode: Option<String>,
    pub is_active: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompartmentListResponse {
    pub compartments: Vec<CompartmentInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompartmentAddedResponse {
    pub status: String,
    pub id: usize,
    pub label: String,
    pub threshold: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompartmentRemovedResponse {
    pub status: String,
    pub id: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompartmentInitializedResponse {
    pub status: String,
    pub compartment_id: usize,
    pub compartment_label: String,
    pub session_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SwitchCompartmentResponse {
    pub status: String,
    pub compartment_id: usize,
    pub compartment_label: String,
}

// ── Snapshot ────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotExportResponse {
    pub status: String,
    pub snapshot_hex: String,
    pub summary: SnapshotSummary,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotRestoreResponse {
    pub status: String,
    pub summary: SnapshotSummary,
    pub requires_reauth: bool,
}

/// Result of resetting local Sigillum data back to first-run setup.
///
/// Reset never destroys key material: when the data directory had contents,
/// they are moved to a timestamped sibling archive whose path is returned in
/// `archived_to` so the operator can restore or remove it deliberately later.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetupResetResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_to: Option<String>,
}

// ── Audit ───────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditEvent {
    pub created_at_unix: u64,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compartment_id: Option<usize>,
    #[serde(default)]
    pub details: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditResponse {
    pub events: Vec<AuditEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditVerifyReport {
    pub scope: String,
    pub status: String,
    pub verified: usize,
    pub broken: usize,
    pub legacy: usize,
}

// ── Diagnostics ────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimePolicyResponse {
    pub queue_default_process_limit: usize,
    pub queue_max_process_limit: usize,
    pub deposit_default_refresh_limit: usize,
    pub deposit_max_refresh_limit: usize,
    pub audit_default_limit: usize,
    pub audit_max_limit: usize,
    pub queue_retry_base_delay_secs: u64,
    pub queue_retry_max_delay_secs: u64,
    pub provider_balance_observation_concurrency: usize,
    pub receiving_refresh_address_cap: usize,
    pub idle_lock_secs: u64,
    pub idle_lock_drain_secs: u64,
    pub idle_lock_force_after_secs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiagnosticsResponse {
    pub status: String,
    pub version: String,
    pub unlock_scope: String,
    pub session_scope: String,
    pub started_at_unix: u64,
    pub initialized: bool,
    pub unlocked_compartment_count: usize,
    pub active_session_count: usize,
    pub default_active_compartment_id: Option<usize>,
    pub max_unlocked_threshold: Option<usize>,
    pub audit_log_present: bool,
    pub pending_operation_count: usize,
    pub queue_job_count: usize,
    #[serde(default)]
    pub blocked_queue_job_count: usize,
    #[serde(default)]
    pub retrying_queue_job_count: usize,
    #[serde(default)]
    pub failed_queue_job_count: usize,
    #[serde(default)]
    pub operator_action_required_queue_job_count: usize,
    pub deferred_queue_job_count: usize,
    #[serde(default)]
    pub startup_interrupted_operation_count: usize,
    #[serde(default)]
    pub startup_recovered_operation_count: usize,
    #[serde(default)]
    pub startup_unresolved_operation_count: usize,
    #[serde(default)]
    pub startup_recovered_queue_job_count: usize,
    #[serde(default)]
    pub startup_reconciled_deposit_count: usize,
    #[serde(default)]
    pub runtime_policy: RuntimePolicyResponse,
    pub eth_stealth_deposit_count: usize,
    pub funded_eth_stealth_deposit_count: usize,
}

// ── Self-check ──────────────────────────────────

/// One self-check verdict for a single configured subject.
///
/// `id` is the stable `"<domain>:<subject>"` pair so UIs can track a check
/// across runs. `status` is `"pass"`, `"warn"`, or `"fail"`. `latency_ms` is
/// only present for checks that performed a live probe (provider RPC).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelfCheckResult {
    pub id: String,
    pub domain: String,
    pub subject: String,
    pub status: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
}

/// Outcome of a self-check run: `status` is the worst individual check
/// status (`"fail"` > `"warn"` > `"pass"`; an empty run is `"pass"`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelfCheckRunResponse {
    pub status: String,
    pub generated_at_unix: u64,
    pub checks: Vec<SelfCheckResult>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2StatusResponse {
    pub enabled: bool,
    pub key_count: usize,
}

pub type DaemonFido2Status = Fido2StatusResponse;

// ── FIDO2 ───────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2DetectResponse {
    pub device_present: bool,
    pub device_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2KeyInfo {
    pub label: String,
    pub credential_id_short: String,
    pub registered_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2ListResponse {
    pub keys: Vec<Fido2KeyInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2SetupResponse {
    pub status: String,
    pub is_first_key: bool,
    pub total_keys: usize,
    pub compartments: usize,
    pub unlocked: bool,
    pub session_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2RegisterResponse {
    pub status: String,
    pub label: String,
    pub total_keys: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poison: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2RemoveResponse {
    pub status: String,
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2SetPinResponse {
    pub status: String,
}

// ── Transit ────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransitEncryptResponse {
    pub key: String,
    pub nonce_hex: String,
    pub ciphertext_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransitDecryptResponse {
    pub key: String,
    pub plaintext_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransitHmacResponse {
    pub key: String,
    pub digest_hex: String,
}

// ── Ethereum Stealth Wallets ────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthMetaAddressResponse {
    pub wallet: String,
    pub short_name: String,
    #[serde(default = "default_ethereum_stealth_scheme_id")]
    pub scheme_id: u64,
    pub stealth_meta_address: String,
    pub spending_public_key_hex: String,
    pub viewing_public_key_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthGenerateResponse {
    pub short_name: String,
    #[serde(default = "default_ethereum_stealth_scheme_id")]
    pub scheme_id: u64,
    pub stealth_meta_address: String,
    pub stealth_address: String,
    pub ephemeral_public_key_hex: String,
    pub view_tag_hex: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub announcement: Option<EthStealthAnnouncementPayload>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthAnnouncementPayload {
    pub announcer_address: String,
    pub announce_function: String,
    #[serde(default = "default_ethereum_stealth_scheme_id")]
    pub scheme_id: u64,
    pub stealth_address: String,
    pub ephemeral_public_key_hex: String,
    pub metadata_hex: String,
    pub calldata_hex: String,
    pub value_wei_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthCheckResponse {
    pub wallet: String,
    pub matches: bool,
    pub derived_stealth_address: String,
    pub view_tag_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthSignResponse {
    pub wallet: String,
    pub stealth_address: String,
    pub signature_hex: String,
    pub recovery_id: u8,
    pub view_tag_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthSignedTransactionResponse {
    pub wallet: String,
    pub kind: String,
    pub chain_id: u64,
    pub nonce: u64,
    pub from_address: String,
    pub to_address: String,
    pub value_hex: String,
    pub data_hex: String,
    pub raw_transaction_hex: String,
    pub transaction_hash_hex: String,
}

// ── EVM ─────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmRpcNonceResponse {
    pub address: String,
    pub nonce: u64,
    pub block_tag: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmRpcBalanceResponse {
    pub address: String,
    pub balance_wei_hex: String,
    pub block_tag: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmRpcErc20BalanceResponse {
    pub token_address: String,
    pub owner_address: String,
    pub amount_hex: String,
    pub block_tag: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmRpcBroadcastResponse {
    pub transaction_hash_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmFeeEstimateResponse {
    pub fees: Eip1559Fees,
    pub gas_limit: u64,
    pub estimated_gas_cost_wei_hex: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthSendResponse {
    pub wallet: String,
    pub kind: String,
    pub chain_id: u64,
    pub nonce: u64,
    pub from_address: String,
    pub to_address: String,
    pub value_hex: String,
    pub data_hex: String,
    pub raw_transaction_hex: String,
    pub transaction_hash_hex: String,
    pub broadcast: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broadcast_transaction_hash_hex: Option<String>,
}

// ── Profiles ────────────────────────────────────

mod profiles;
pub use profiles::*;

// ── Wallet inventory and discovery ─────────────────
mod wallet_domains;
pub use wallet_domains::*;

mod inventory;
pub use inventory::*;

mod consolidation_export;
pub use consolidation_export::*;

mod watch_book;
pub use watch_book::*;

mod treasury;
pub use treasury::*;

// ── Queue ───────────────────────────────────────

mod queue;
pub use queue::*;

mod queue_process;
pub use queue_process::*;

// ── Deposits ────────────────────────────────────

mod deposits;
pub use deposits::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaintenanceRunResponse {
    pub status: String,
    pub refreshed: usize,
    pub detected: usize,
    pub queued: usize,
    pub processed: usize,
    pub succeeded: usize,
    #[serde(default)]
    pub blocked: usize,
    #[serde(default)]
    pub retrying: usize,
    #[serde(default)]
    pub operator_action_required: usize,
    pub failed: usize,
    /// W7.4: mirrors `QueueProcessResponse::confirmed` (see there).
    #[serde(default)]
    pub confirmed: usize,
    #[serde(default)]
    pub failures_by_cause: MaintenanceFailureBreakdown,
    pub deposits: Vec<EthStealthDeposit>,
    pub jobs: Vec<QueueJob>,
}

fn default_ethereum_stealth_scheme_id() -> u64 {
    ETHEREUM_STEALTH_SCHEME_ID
}

// ── Tests ───────────────────────────────────────

#[cfg(test)]
#[path = "response_tests.rs"]
mod tests;
