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

// ── Lifecycle ───────────────────────────────────

/// Standard error envelope returned for non-2xx responses.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorResponse {
    pub error: String,
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmProviderProfile {
    pub name: String,
    pub rpc_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_token_key: Option<String>,
    #[serde(default)]
    pub compartment_id: usize,
    pub chain_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_priority_fee_per_gas_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_fee_per_gas_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_gas_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub erc20_gas_limit: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmProviderProfileListResponse {
    pub profiles: Vec<EvmProviderProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmProviderProfileMutationResponse {
    pub status: String,
    pub profile: EvmProviderProfile,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthWalletProfile {
    pub name: String,
    pub wallet: String,
    pub short_name: String,
    pub provider_profile: String,
    #[serde(default)]
    pub compartment_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_destination_address: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthWalletProfileListResponse {
    pub profiles: Vec<EthStealthWalletProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthWalletProfileMutationResponse {
    pub status: String,
    pub profile: EthStealthWalletProfile,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthXpubWalletProfile {
    pub name: String,
    pub project_account: u32,
    pub provider_profile: String,
    #[serde(default)]
    pub compartment_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_destination_address: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthXpubWalletProfileListResponse {
    pub profiles: Vec<EthXpubWalletProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthXpubWalletProfileMutationResponse {
    pub status: String,
    pub profile: EthXpubWalletProfile,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthXpubExportResponse {
    pub wallet_profile: String,
    pub project_account: u32,
    pub account_path: String,
    pub receive_path: String,
    pub receive_xpub: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthXpubAddressResponse {
    pub index: u32,
    pub address: String,
}

// ── Deposits ────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthDeposit {
    pub id: String,
    pub status: String,
    pub asset_kind: String,
    pub wallet_profile: String,
    #[serde(default)]
    pub wallet_compartment_id: usize,
    #[serde(default)]
    pub provider_compartment_id: usize,
    pub wallet: String,
    pub short_name: String,
    pub stealth_meta_address: String,
    pub stealth_address: String,
    pub ephemeral_public_key_hex: String,
    pub view_tag_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_amount_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_amount_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_native_balance_wei_hex: Option<String>,
    pub auto_queue_sweep: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sweep_destination_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_sweep_amount_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_job_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_at_unix: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broadcast_transaction_hash_hex: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthDepositListResponse {
    pub deposits: Vec<EthStealthDeposit>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthDepositMutationResponse {
    pub status: String,
    pub deposit: EthStealthDeposit,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthDepositRefreshResponse {
    pub processed: usize,
    pub detected: usize,
    pub queued: usize,
    pub deposits: Vec<EthStealthDeposit>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthDepositEnqueueSweepResponse {
    pub status: String,
    pub deposit: EthStealthDeposit,
    pub job: QueueJob,
}

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
    pub failed: usize,
    pub deposits: Vec<EthStealthDeposit>,
    pub jobs: Vec<QueueJob>,
}

// ── Queue ───────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueueJobPayload {
    EthStealthTransfer {
        wallet_profile: String,
        stealth_address: String,
        ephemeral_public_key_hex: String,
        value_wei_hex: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        destination_address: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        nonce: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        gas_limit: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        view_tag_hex: Option<String>,
    },
    EthStealthErc20Transfer {
        wallet_profile: String,
        stealth_address: String,
        ephemeral_public_key_hex: String,
        token_address: String,
        recipient_address: String,
        amount_hex: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        nonce: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        gas_limit: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        view_tag_hex: Option<String>,
    },
    EthStealthNativeSweep {
        wallet_profile: String,
        stealth_address: String,
        ephemeral_public_key_hex: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        destination_address: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        min_value_wei_hex: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        gas_limit: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        view_tag_hex: Option<String>,
    },
    EthStealthErc20Sweep {
        wallet_profile: String,
        stealth_address: String,
        ephemeral_public_key_hex: String,
        token_address: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        recipient_address: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        min_amount_hex: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        gas_limit: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        view_tag_hex: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueJob {
    pub id: String,
    pub state: String,
    pub attempts: u32,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_attempt_after_unix: Option<u64>,
    #[serde(flatten)]
    pub payload: QueueJobPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_hash_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broadcast_transaction_hash_hex: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueJobListResponse {
    pub jobs: Vec<QueueJob>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueEnqueueResponse {
    pub status: String,
    pub job: QueueJob,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueProcessResponse {
    pub processed: usize,
    pub succeeded: usize,
    #[serde(default)]
    pub blocked: usize,
    #[serde(default)]
    pub retrying: usize,
    pub failed: usize,
    pub jobs: Vec<QueueJob>,
}

fn default_ethereum_stealth_scheme_id() -> u64 {
    ETHEREUM_STEALTH_SCHEME_ID
}

// ── Tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip_test<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(
        value: T,
    ) {
        let json = serde_json::to_string(&value).unwrap();
        let deserialized: T = serde_json::from_str(&json).unwrap();
        assert_eq!(value, deserialized, "Roundtrip failed for JSON: {}", json);
    }

    #[test]
    fn test_error_response_roundtrip() {
        let resp = ErrorResponse {
            error: "Something went wrong".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_active_compartment_roundtrip() {
        let comp = ActiveCompartment {
            compartment_id: 1,
            compartment_label: "vault1".to_string(),
            api_key_count: 5,
            secret_count: Some(10),
        };
        roundtrip_test(comp);
    }

    #[test]
    fn test_active_compartment_no_secret_count() {
        let comp = ActiveCompartment {
            compartment_id: 2,
            compartment_label: "vault2".to_string(),
            api_key_count: 3,
            secret_count: None,
        };
        roundtrip_test(comp);
    }

    #[test]
    fn test_unlocked_compartment_roundtrip() {
        let comp = UnlockedCompartment {
            id: 1,
            label: "vault_unlocked".to_string(),
            threshold: 2,
            passphrase_mode: Some("FIXED".to_string()),
        };
        roundtrip_test(comp);
    }

    #[test]
    fn test_status_response_full() {
        let resp = StatusResponse {
            locked: false,
            initialized: true,
            active_compartment: Some(ActiveCompartment {
                compartment_id: 1,
                compartment_label: "active".to_string(),
                api_key_count: 2,
                secret_count: Some(5),
            }),
            unlocked_compartments: vec![UnlockedCompartment {
                id: 1,
                label: "vault1".to_string(),
                threshold: 1,
                passphrase_mode: None,
            }],
            fido2: Some(Fido2StatusResponse {
                enabled: true,
                key_count: 2,
            }),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_status_response_locked() {
        let resp = StatusResponse {
            locked: true,
            initialized: true,
            active_compartment: None,
            unlocked_compartments: vec![],
            fido2: None,
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_lock_response_roundtrip() {
        let resp = LockResponse {
            status: "locked".to_string(),
            message: "Vault is now locked".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_unlock_response_roundtrip() {
        let resp = UnlockResponse {
            status: "unlocked".to_string(),
            method: "fido2".to_string(),
            cascading: Some(true),
            session_token: "token123".to_string(),
            unlocked_compartments: vec![UnlockedCompartment {
                id: 1,
                label: "vault1".to_string(),
                threshold: 1,
                passphrase_mode: None,
            }],
            active_compartment_id: Some(1),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_generic_status_response_roundtrip() {
        let resp = GenericStatusResponse {
            status: "success".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_key_list_response_roundtrip() {
        let resp = KeyListResponse {
            keys: vec!["key1".to_string(), "key2".to_string(), "key3".to_string()],
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_key_value_response_roundtrip() {
        let resp = KeyValueResponse {
            key: "mykey".to_string(),
            value: "myvalue".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_key_mutation_response_roundtrip() {
        let resp = KeyMutationResponse {
            status: "created".to_string(),
            key: "newkey".to_string(),
            tier: Some(2),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_push_response_roundtrip() {
        let resp = PushResponse {
            status: "success".to_string(),
            from: 1,
            to: 2,
            key: "transferred_key".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_compartment_info_roundtrip() {
        let info = CompartmentInfo {
            id: 1,
            label: "vault1".to_string(),
            threshold: 2,
            passphrase_mode: Some("FIXED".to_string()),
            is_active: true,
        };
        roundtrip_test(info);
    }

    #[test]
    fn test_compartment_list_response_roundtrip() {
        let resp = CompartmentListResponse {
            compartments: vec![
                CompartmentInfo {
                    id: 1,
                    label: "vault1".to_string(),
                    threshold: 1,
                    passphrase_mode: None,
                    is_active: true,
                },
                CompartmentInfo {
                    id: 2,
                    label: "vault2".to_string(),
                    threshold: 2,
                    passphrase_mode: Some("EPHEMERAL".to_string()),
                    is_active: false,
                },
            ],
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_fido2_setup_response_roundtrip() {
        let resp = Fido2SetupResponse {
            status: "registered".to_string(),
            is_first_key: true,
            total_keys: 1,
            compartments: 2,
            unlocked: true,
            session_token: "new_session".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_transit_encrypt_response_roundtrip() {
        let resp = TransitEncryptResponse {
            key: "encryption_key".to_string(),
            nonce_hex: "0123456789abcdef".to_string(),
            ciphertext_hex: "fedcba9876543210".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_transit_decrypt_response_roundtrip() {
        let resp = TransitDecryptResponse {
            key: "encryption_key".to_string(),
            plaintext_hex: "48656c6c6f".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_transit_hmac_response_roundtrip() {
        let resp = TransitHmacResponse {
            key: "hmac_key".to_string(),
            digest_hex: "abcd1234".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_eth_stealth_meta_address_response_roundtrip() {
        let resp = EthStealthMetaAddressResponse {
            wallet: "0xwallet".to_string(),
            short_name: "my_wallet".to_string(),
            scheme_id: ETHEREUM_STEALTH_SCHEME_ID,
            stealth_meta_address: "st:0x...".to_string(),
            spending_public_key_hex: "0xspend".to_string(),
            viewing_public_key_hex: "0xview".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_eth_stealth_generate_response_roundtrip() {
        let resp = EthStealthGenerateResponse {
            short_name: "wallet1".to_string(),
            scheme_id: ETHEREUM_STEALTH_SCHEME_ID,
            stealth_meta_address: "st:0x...".to_string(),
            stealth_address: "0xstealth".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            view_tag_hex: "0xaa".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_eth_stealth_check_response_roundtrip() {
        let resp = EthStealthCheckResponse {
            wallet: "0xwallet".to_string(),
            matches: true,
            derived_stealth_address: "0xderived".to_string(),
            view_tag_hex: "0xaa".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_eth_stealth_sign_response_roundtrip() {
        let resp = EthStealthSignResponse {
            wallet: "0xwallet".to_string(),
            stealth_address: "0xstealth".to_string(),
            signature_hex: "0xsig".to_string(),
            recovery_id: 27,
            view_tag_hex: "0xaa".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_eth_signed_transaction_response_roundtrip() {
        let resp = EthSignedTransactionResponse {
            wallet: "0xwallet".to_string(),
            kind: "native_transfer".to_string(),
            chain_id: 1,
            nonce: 5,
            from_address: "0xfrom".to_string(),
            to_address: "0xto".to_string(),
            value_hex: "0x100".to_string(),
            data_hex: "0x".to_string(),
            raw_transaction_hex: "0xraw".to_string(),
            transaction_hash_hex: "0xhash".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_evm_rpc_nonce_response_roundtrip() {
        let resp = EvmRpcNonceResponse {
            address: "0xaddr".to_string(),
            nonce: 42,
            block_tag: "latest".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_evm_rpc_balance_response_roundtrip() {
        let resp = EvmRpcBalanceResponse {
            address: "0xaddr".to_string(),
            balance_wei_hex: "0x1000".to_string(),
            block_tag: "latest".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_evm_rpc_erc20_balance_response_roundtrip() {
        let resp = EvmRpcErc20BalanceResponse {
            token_address: "0xtoken".to_string(),
            owner_address: "0xowner".to_string(),
            amount_hex: "0x500".to_string(),
            block_tag: "latest".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_evm_rpc_broadcast_response_roundtrip() {
        let resp = EvmRpcBroadcastResponse {
            transaction_hash_hex: "0xtxhash".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_eth_stealth_send_response_with_broadcast() {
        let resp = EthStealthSendResponse {
            wallet: "0xwallet".to_string(),
            kind: "native_transfer".to_string(),
            chain_id: 1,
            nonce: 5,
            from_address: "0xfrom".to_string(),
            to_address: "0xto".to_string(),
            value_hex: "0x100".to_string(),
            data_hex: "0x".to_string(),
            raw_transaction_hex: "0xraw".to_string(),
            transaction_hash_hex: "0xhash".to_string(),
            broadcast: true,
            broadcast_transaction_hash_hex: Some("0xbcast".to_string()),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_eth_stealth_send_response_no_broadcast() {
        let resp = EthStealthSendResponse {
            wallet: "0xwallet".to_string(),
            kind: "native_transfer".to_string(),
            chain_id: 1,
            nonce: 5,
            from_address: "0xfrom".to_string(),
            to_address: "0xto".to_string(),
            value_hex: "0x100".to_string(),
            data_hex: "0x".to_string(),
            raw_transaction_hex: "0xraw".to_string(),
            transaction_hash_hex: "0xhash".to_string(),
            broadcast: false,
            broadcast_transaction_hash_hex: None,
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_evm_provider_profile_roundtrip() {
        let profile = EvmProviderProfile {
            name: "mainnet".to_string(),
            rpc_url: "https://eth.example.com".to_string(),
            auth_token_key: Some("key123".to_string()),
            compartment_id: 1,
            chain_id: 1,
            max_priority_fee_per_gas_hex: Some("0x3b9aca00".to_string()),
            max_fee_per_gas_hex: Some("0x5f5e100".to_string()),
            native_gas_limit: Some(21000),
            erc20_gas_limit: Some(65000),
        };
        roundtrip_test(profile);
    }

    #[test]
    fn test_eth_stealth_wallet_profile_roundtrip() {
        let profile = EthStealthWalletProfile {
            name: "my_wallet".to_string(),
            wallet: "0xwallet".to_string(),
            short_name: "wallet1".to_string(),
            provider_profile: "mainnet".to_string(),
            compartment_id: 1,
            chain_id: Some(1),
            default_destination_address: Some("0xdest".to_string()),
        };
        roundtrip_test(profile);
    }

    #[test]
    fn test_eth_stealth_wallet_profile_list_response_roundtrip() {
        let resp = EthStealthWalletProfileListResponse {
            profiles: vec![
                EthStealthWalletProfile {
                    name: "wallet1".to_string(),
                    wallet: "0xwallet1".to_string(),
                    short_name: "w1".to_string(),
                    provider_profile: "mainnet".to_string(),
                    compartment_id: 1,
                    chain_id: Some(1),
                    default_destination_address: None,
                },
                EthStealthWalletProfile {
                    name: "wallet2".to_string(),
                    wallet: "0xwallet2".to_string(),
                    short_name: "w2".to_string(),
                    provider_profile: "testnet".to_string(),
                    compartment_id: 2,
                    chain_id: Some(5),
                    default_destination_address: Some("0xdest2".to_string()),
                },
            ],
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_eth_xpub_wallet_profile_roundtrip() {
        let profile = EthXpubWalletProfile {
            name: "receive_tree".to_string(),
            project_account: 9,
            provider_profile: "mainnet".to_string(),
            compartment_id: 1,
            chain_id: Some(1),
            default_destination_address: Some("0xdest".to_string()),
        };
        roundtrip_test(profile);
    }

    #[test]
    fn test_eth_xpub_wallet_profile_list_response_roundtrip() {
        let resp = EthXpubWalletProfileListResponse {
            profiles: vec![
                EthXpubWalletProfile {
                    name: "receive_tree".to_string(),
                    project_account: 0,
                    provider_profile: "mainnet".to_string(),
                    compartment_id: 1,
                    chain_id: Some(1),
                    default_destination_address: None,
                },
                EthXpubWalletProfile {
                    name: "project_b".to_string(),
                    project_account: 15,
                    provider_profile: "testnet".to_string(),
                    compartment_id: 2,
                    chain_id: Some(5),
                    default_destination_address: Some("0xdest2".to_string()),
                },
            ],
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_eth_xpub_export_response_roundtrip() {
        let resp = EthXpubExportResponse {
            wallet_profile: "receive_tree".to_string(),
            project_account: 9,
            account_path: "m/44'/60'/9'".to_string(),
            receive_path: "m/44'/60'/9'/0".to_string(),
            receive_xpub: "xpub661MyMwAqRbcFexample".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_eth_xpub_address_response_roundtrip() {
        let resp = EthXpubAddressResponse {
            index: 4,
            address: "0x1111111111111111111111111111111111111111".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_eth_stealth_deposit_roundtrip() {
        let deposit = EthStealthDeposit {
            id: "deposit_1".to_string(),
            status: "funded".to_string(),
            asset_kind: "native".to_string(),
            wallet_profile: "wallet1".to_string(),
            wallet_compartment_id: 1,
            provider_compartment_id: 1,
            wallet: "0xwallet".to_string(),
            short_name: "w1".to_string(),
            stealth_meta_address: "st:0x...".to_string(),
            stealth_address: "0xstealth".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            view_tag_hex: "0xaa".to_string(),
            token_address: None,
            expected_amount_hex: None,
            observed_amount_hex: Some("0x100".to_string()),
            observed_native_balance_wei_hex: Some("0x200".to_string()),
            auto_queue_sweep: true,
            sweep_destination_address: None,
            min_sweep_amount_hex: None,
            queue_job_id: None,
            queue_job_state: None,
            note: Some("test deposit".to_string()),
            created_at_unix: 1000000,
            updated_at_unix: 1000001,
            last_checked_at_unix: Some(1000002),
            broadcast_transaction_hash_hex: None,
        };
        roundtrip_test(deposit);
    }

    #[test]
    fn test_queue_job_payload_native_transfer_roundtrip() {
        let payload = QueueJobPayload::EthStealthTransfer {
            wallet_profile: "profile".to_string(),
            stealth_address: "0xstealth".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            value_wei_hex: "0x100".to_string(),
            destination_address: Some("0xdest".to_string()),
            nonce: Some(5),
            gas_limit: Some(21000),
            view_tag_hex: Some("0xaa".to_string()),
        };
        roundtrip_test(payload);
    }

    #[test]
    fn test_queue_job_payload_erc20_transfer_roundtrip() {
        let payload = QueueJobPayload::EthStealthErc20Transfer {
            wallet_profile: "profile".to_string(),
            stealth_address: "0xstealth".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            token_address: "0xtoken".to_string(),
            recipient_address: "0xrecip".to_string(),
            amount_hex: "0x100".to_string(),
            nonce: None,
            gas_limit: Some(100000),
            view_tag_hex: None,
        };
        roundtrip_test(payload);
    }

    #[test]
    fn test_queue_job_payload_native_sweep_roundtrip() {
        let payload = QueueJobPayload::EthStealthNativeSweep {
            wallet_profile: "profile".to_string(),
            stealth_address: "0xstealth".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            destination_address: Some("0xdest".to_string()),
            min_value_wei_hex: Some("0x1".to_string()),
            gas_limit: Some(21000),
            view_tag_hex: Some("0xaa".to_string()),
        };
        roundtrip_test(payload);
    }

    #[test]
    fn test_queue_job_payload_erc20_sweep_roundtrip() {
        let payload = QueueJobPayload::EthStealthErc20Sweep {
            wallet_profile: "profile".to_string(),
            stealth_address: "0xstealth".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            token_address: "0xtoken".to_string(),
            recipient_address: Some("0xrecip".to_string()),
            min_amount_hex: Some("0x10".to_string()),
            gas_limit: None,
            view_tag_hex: None,
        };
        roundtrip_test(payload);
    }

    #[test]
    fn test_queue_job_with_flatten_and_tag_roundtrip() {
        let job = QueueJob {
            id: "job_1".to_string(),
            state: "pending".to_string(),
            attempts: 0,
            created_at_unix: 1000000,
            updated_at_unix: 1000001,
            next_attempt_after_unix: None,
            payload: QueueJobPayload::EthStealthTransfer {
                wallet_profile: "profile".to_string(),
                stealth_address: "0xstealth".to_string(),
                ephemeral_public_key_hex: "0xeph".to_string(),
                value_wei_hex: "0x100".to_string(),
                destination_address: Some("0xdest".to_string()),
                nonce: Some(5),
                gas_limit: Some(21000),
                view_tag_hex: None,
            },
            last_error: None,
            transaction_hash_hex: None,
            broadcast_transaction_hash_hex: None,
        };
        roundtrip_test(job);
    }

    #[test]
    fn test_queue_job_with_error_roundtrip() {
        let job = QueueJob {
            id: "job_2".to_string(),
            state: "failed".to_string(),
            attempts: 3,
            created_at_unix: 1000000,
            updated_at_unix: 1000005,
            next_attempt_after_unix: None,
            payload: QueueJobPayload::EthStealthNativeSweep {
                wallet_profile: "profile".to_string(),
                stealth_address: "0xstealth".to_string(),
                ephemeral_public_key_hex: "0xeph".to_string(),
                destination_address: None,
                min_value_wei_hex: None,
                gas_limit: None,
                view_tag_hex: None,
            },
            last_error: Some("Insufficient funds".to_string()),
            transaction_hash_hex: None,
            broadcast_transaction_hash_hex: Some("0xbcast".to_string()),
        };
        roundtrip_test(job);
    }

    #[test]
    fn test_queue_job_list_response_roundtrip() {
        let resp = QueueJobListResponse {
            jobs: vec![QueueJob {
                id: "job_1".to_string(),
                state: "pending".to_string(),
                attempts: 0,
                created_at_unix: 1000000,
                updated_at_unix: 1000001,
                next_attempt_after_unix: None,
                payload: QueueJobPayload::EthStealthTransfer {
                    wallet_profile: "profile".to_string(),
                    stealth_address: "0xstealth".to_string(),
                    ephemeral_public_key_hex: "0xeph".to_string(),
                    value_wei_hex: "0x100".to_string(),
                    destination_address: None,
                    nonce: None,
                    gas_limit: None,
                    view_tag_hex: None,
                },
                last_error: None,
                transaction_hash_hex: None,
                broadcast_transaction_hash_hex: None,
            }],
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_queue_enqueue_response_roundtrip() {
        let resp = QueueEnqueueResponse {
            status: "enqueued".to_string(),
            job: QueueJob {
                id: "job_3".to_string(),
                state: "pending".to_string(),
                attempts: 0,
                created_at_unix: 1000000,
                updated_at_unix: 1000000,
                next_attempt_after_unix: None,
                payload: QueueJobPayload::EthStealthErc20Transfer {
                    wallet_profile: "profile".to_string(),
                    stealth_address: "0xstealth".to_string(),
                    ephemeral_public_key_hex: "0xeph".to_string(),
                    token_address: "0xtoken".to_string(),
                    recipient_address: "0xrecip".to_string(),
                    amount_hex: "0x100".to_string(),
                    nonce: None,
                    gas_limit: None,
                    view_tag_hex: None,
                },
                last_error: None,
                transaction_hash_hex: None,
                broadcast_transaction_hash_hex: None,
            },
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_queue_process_response_roundtrip() {
        let resp = QueueProcessResponse {
            processed: 5,
            succeeded: 4,
            blocked: 0,
            retrying: 0,
            failed: 1,
            jobs: vec![QueueJob {
                id: "job_4".to_string(),
                state: "completed".to_string(),
                attempts: 1,
                created_at_unix: 1000000,
                updated_at_unix: 1000010,
                next_attempt_after_unix: None,
                payload: QueueJobPayload::EthStealthNativeSweep {
                    wallet_profile: "profile".to_string(),
                    stealth_address: "0xstealth".to_string(),
                    ephemeral_public_key_hex: "0xeph".to_string(),
                    destination_address: Some("0xdest".to_string()),
                    min_value_wei_hex: Some("0x1".to_string()),
                    gas_limit: Some(21000),
                    view_tag_hex: Some("0xaa".to_string()),
                },
                last_error: None,
                transaction_hash_hex: Some("0xhash".to_string()),
                broadcast_transaction_hash_hex: None,
            }],
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_snapshot_export_response_roundtrip() {
        let resp = SnapshotExportResponse {
            status: "exported".to_string(),
            snapshot_hex: "deadbeef".to_string(),
            summary: SnapshotSummary {
                created_at_unix: 1000000,
                file_count: 10,
                total_bytes: 5000,
            },
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_snapshot_restore_response_roundtrip() {
        let resp = SnapshotRestoreResponse {
            status: "restored".to_string(),
            summary: SnapshotSummary {
                created_at_unix: 1000000,
                file_count: 10,
                total_bytes: 5000,
            },
            requires_reauth: false,
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_audit_event_roundtrip() {
        let event = AuditEvent {
            created_at_unix: 1000000,
            kind: "unlock".to_string(),
            compartment_id: Some(1),
            details: serde_json::json!({"method": "fido2"}),
        };
        roundtrip_test(event);
    }

    #[test]
    fn test_fido2_status_response_roundtrip() {
        let resp = Fido2StatusResponse {
            enabled: true,
            key_count: 3,
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_fido2_detect_response_roundtrip() {
        let resp = Fido2DetectResponse {
            device_present: true,
            device_count: 2,
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_fido2_key_info_roundtrip() {
        let info = Fido2KeyInfo {
            label: "my_key".to_string(),
            credential_id_short: "abc123".to_string(),
            registered_at: "2024-01-01T00:00:00Z".to_string(),
        };
        roundtrip_test(info);
    }

    #[test]
    fn test_fido2_list_response_roundtrip() {
        let resp = Fido2ListResponse {
            keys: vec![
                Fido2KeyInfo {
                    label: "key1".to_string(),
                    credential_id_short: "abc".to_string(),
                    registered_at: "2024-01-01T00:00:00Z".to_string(),
                },
                Fido2KeyInfo {
                    label: "key2".to_string(),
                    credential_id_short: "def".to_string(),
                    registered_at: "2024-01-02T00:00:00Z".to_string(),
                },
            ],
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_fido2_register_response_with_poison() {
        let resp = Fido2RegisterResponse {
            status: "registered".to_string(),
            label: "new_key".to_string(),
            total_keys: 2,
            poison: Some(true),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_fido2_register_response_no_poison() {
        let resp = Fido2RegisterResponse {
            status: "registered".to_string(),
            label: "new_key".to_string(),
            total_keys: 2,
            poison: None,
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_fido2_remove_response_roundtrip() {
        let resp = Fido2RemoveResponse {
            status: "removed".to_string(),
            label: "old_key".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_fido2_set_pin_response_roundtrip() {
        let resp = Fido2SetPinResponse {
            status: "pin_set".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_compartment_added_response_roundtrip() {
        let resp = CompartmentAddedResponse {
            status: "added".to_string(),
            id: 3,
            label: "new_vault".to_string(),
            threshold: 2,
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_compartment_removed_response_roundtrip() {
        let resp = CompartmentRemovedResponse {
            status: "removed".to_string(),
            id: 2,
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_compartment_initialized_response_roundtrip() {
        let resp = CompartmentInitializedResponse {
            status: "initialized".to_string(),
            compartment_id: 1,
            compartment_label: "vault1".to_string(),
            session_token: "token123".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_switch_compartment_response_roundtrip() {
        let resp = SwitchCompartmentResponse {
            status: "switched".to_string(),
            compartment_id: 2,
            compartment_label: "vault2".to_string(),
        };
        roundtrip_test(resp);
    }

    #[test]
    fn test_session_revoke_response_roundtrip() {
        let resp = SessionRevokeResponse {
            status: "revoked".to_string(),
            requires_reauth: true,
        };
        roundtrip_test(resp);
    }
}
