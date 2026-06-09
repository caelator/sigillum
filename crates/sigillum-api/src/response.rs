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
pub struct EthSeedWalletProfile {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub project_account: u32,
    pub provider_profile: String,
    #[serde(default)]
    pub compartment_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    pub word_count: usize,
    pub mnemonic_secret_key: String,
    pub account_path: String,
    pub receive_path: String,
    pub receive_xpub: String,
    pub first_receive_address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_destination_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_xpub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sponsor_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hot_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub treasury_address: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthSeedWalletProfileListResponse {
    pub profiles: Vec<EthSeedWalletProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthSeedWalletProfileMutationResponse {
    pub status: String,
    pub profile: EthSeedWalletProfile,
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

// ── Wallet inventory and discovery ─────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletInventoryAddress {
    pub id: String,
    pub wallet_family: String,
    pub wallet_profile: String,
    pub provider_profile: String,
    pub chain_id: u64,
    pub address: String,
    pub derivation_path: String,
    pub address_index: u32,
    pub activity_state: String,
    pub native_balance_wei_hex: String,
    pub transaction_count: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classifications: Vec<String>,
    pub source: String,
    pub first_seen_at_unix: u64,
    pub last_checked_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletAssetHolding {
    pub id: String,
    pub wallet_family: String,
    pub wallet_profile: String,
    pub provider_profile: String,
    pub chain_id: u64,
    pub address: String,
    pub derivation_path: String,
    pub asset_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterparty_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_adapter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_index_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claim_proof: Vec<String>,
    pub amount_hex: String,
    pub source: String,
    pub status: String,
    pub first_seen_at_unix: u64,
    pub last_checked_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletDiscoveryJob {
    pub id: String,
    pub status: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wallet_families: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wallet_profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_profiles: Vec<String>,
    pub gap_limit: u32,
    pub max_index: u32,
    pub addresses_scanned: usize,
    pub active_addresses: usize,
    pub holdings_detected: usize,
    pub started_at_unix: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at_unix: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletInventoryListResponse {
    pub jobs: Vec<WalletDiscoveryJob>,
    pub addresses: Vec<WalletInventoryAddress>,
    pub holdings: Vec<WalletAssetHolding>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletInventoryScanResponse {
    pub job: WalletDiscoveryJob,
    pub addresses: Vec<WalletInventoryAddress>,
    pub holdings: Vec<WalletAssetHolding>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChainProfile {
    pub name: String,
    pub chain_family: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_profile: Option<String>,
    pub native_symbol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explorer_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    pub enabled: bool,
    pub source: String,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChainProfileListResponse {
    pub profiles: Vec<ChainProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChainProfileMutationResponse {
    pub status: String,
    pub profile: ChainProfile,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryJobListResponse {
    pub jobs: Vec<WalletDiscoveryJob>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryJobMutationResponse {
    pub status: String,
    pub job: WalletDiscoveryJob,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskCatalogEntry {
    pub address: String,
    pub label: String,
    pub risk_level: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskCatalogListResponse {
    pub entries: Vec<RiskCatalogEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskCatalogMutationResponse {
    pub status: String,
    pub entry: RiskCatalogEntry,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskFinding {
    pub id: String,
    pub category: String,
    pub risk_level: String,
    pub status: String,
    pub wallet_family: String,
    pub wallet_profile: String,
    pub provider_profile: String,
    pub chain_id: u64,
    pub address: String,
    pub subject_type: String,
    pub subject: String,
    pub source: String,
    pub recommendation: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    pub first_seen_at_unix: u64,
    pub last_checked_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskFindingListResponse {
    pub findings: Vec<RiskFinding>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanSummary {
    pub total_steps: usize,
    pub blocked_steps: usize,
    pub review_required_steps: usize,
    pub approved_steps: usize,
    pub executable_steps: usize,
    pub value_items: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanStep {
    pub id: String,
    pub action: String,
    pub status: String,
    pub wallet_family: String,
    pub wallet_profile: String,
    pub provider_profile: String,
    pub chain_id: u64,
    pub address: String,
    pub derivation_path: String,
    pub asset_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterparty_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_adapter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_index_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claim_proof: Vec<String>,
    pub amount_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_address: Option<String>,
    pub signer_status: String,
    pub simulation_status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub simulation_evidence: Vec<String>,
    pub risk_level: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
    pub auto_eligible: bool,
    pub approved: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlan {
    pub id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_address: Option<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub summary: ConsolidationPlanSummary,
    /// Plan-wide treasury policy violations (e.g. plan native cap exceeded).
    /// Step-level violations live in each step's `blockers` instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policy_violations: Vec<String>,
    pub steps: Vec<ConsolidationPlanStep>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanListResponse {
    pub plans: Vec<ConsolidationPlan>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanMutationResponse {
    pub status: String,
    pub plan: ConsolidationPlan,
}

mod consolidation_export;
pub use consolidation_export::*;

mod watch_book;
pub use watch_book::*;

mod treasury;
pub use treasury::*;

// ── Queue ───────────────────────────────────────

mod queue;
pub use queue::*;

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
    pub failed: usize,
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
