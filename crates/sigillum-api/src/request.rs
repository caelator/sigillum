//! Request types for the Sigillum daemon API.
//!
//! Types in this module form the JSON wire protocol for all daemon endpoints.
//! Shared field groups are extracted into reusable structs (`StealthPaymentRef`,
//! `EvmProviderRef`, `Eip1559Fees`) and composed via `#[serde(flatten)]` to
//! eliminate duplication while preserving backward-compatible JSON serialization.

use serde::{Deserialize, Serialize};

// ── Shared domain types ─────────────────────────────────────────

/// Reference to a specific stealth payment.
///
/// A stealth meta-address holder publishes `(stealth_address, ephemeral_public_key)`
/// alongside each payment so that the recipient can scan for ownership. The optional
/// `view_tag` is a single-byte bloom hint (ERC-5564 §3) that allows the recipient
/// to skip expensive ECDH computations for non-matching addresses.
///
/// This type appears in every wallet operation that targets a specific stealth
/// address — signing, checking, sending, and sweeping.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StealthPaymentRef {
    pub stealth_address: String,
    pub ephemeral_public_key_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_tag_hex: Option<String>,
}

/// Connection parameters for an EVM JSON-RPC provider.
///
/// Encapsulates the endpoint URL, optional bearer-token key (resolved from
/// the vault at request time), and the compartment that holds the token.
/// Used by all on-chain query and broadcast endpoints.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmProviderRef {
    pub rpc_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_token_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compartment_id: Option<usize>,
}

/// EIP-1559 fee parameters for Ethereum transactions.
///
/// Carries the chain-specific fee fields required to construct a Type-2
/// (EIP-1559) transaction envelope. All hex values are big-endian, 0x-prefixed
/// or raw hex strings representing uint256 quantities.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Eip1559Fees {
    pub chain_id: u64,
    pub max_priority_fee_per_gas_hex: String,
    pub max_fee_per_gas_hex: String,
}

// ── Key/value primitives ────────────────────────────────────────

/// Store or update a key-value pair in the active compartment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyValueRequest {
    pub key: String,
    pub value: Option<String>,
}

/// Supported CLI password generator character sets.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PasswordCharset {
    Loweralpha,
    Mixalpha,
    Numeric,
    AlphaNumeric,
    MixalphaNumeric,
    MixalphaNumericSymbol,
}

impl PasswordCharset {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Loweralpha => "loweralpha",
            Self::Mixalpha => "mixalpha",
            Self::Numeric => "numeric",
            Self::AlphaNumeric => "alpha-numeric",
            Self::MixalphaNumeric => "mixalpha-numeric",
            Self::MixalphaNumericSymbol => "mixalpha-numeric-symbol",
        }
    }
}

/// Retrieve or delete a key by name from the active compartment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyOnlyRequest {
    pub key: String,
}

/// Authenticate with a passphrase to unlock matching compartments.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PassphraseRequest {
    pub passphrase: String,
}

/// Register or replace the biometric verifier for the currently unlocked compartment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BiometricEnrollRequest {
    pub public_key_hex: String,
    pub passphrase: String,
}

/// Consume a daemon-issued one-time challenge and unlock using a helper-produced payload.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BiometricUnlockRequest {
    pub payload_hex: String,
}

/// Restore a vault from an encrypted snapshot archive.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotRestoreRequest {
    pub passphrase: String,
    pub snapshot_hex: String,
}

/// Reset local Sigillum data after an interrupted or abandoned setup flow.
///
/// The caller must send a fixed confirmation phrase so destructive resets are
/// always explicit in both the UI and the API contract.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetupResetRequest {
    pub confirmation: String,
}

/// Definition of a compartment during FIDO2 setup or addition.
///
/// `threshold` determines how many FIDO2 key taps are required to unlock this
/// compartment. `passphrase_mode` controls whether a passphrase fallback is
/// configured (e.g. "FIXED" for a setup-time passphrase).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompartmentDefinition {
    pub label: String,
    pub threshold: usize,
    pub passphrase_mode: Option<String>,
}

// ── FIDO2 hardware key operations ────────────────────────────────

/// Initialize the vault with a FIDO2 hardware key and one or more compartments.
///
/// This is the primary setup path for new vaults. The first key registered
/// becomes the initial Shamir share holder. Compartment thresholds determine
/// how many distinct key taps are needed to unlock each compartment. `pin` is
/// optional so touch-only authenticators can be enrolled without forcing a PIN
/// round-trip; provide it only when the inserted key currently requires one.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2SetupRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    pub label: String,
    pub compartments: Vec<CompartmentDefinition>,
    pub passphrase: Option<String>,
}

/// Register an additional FIDO2 key (or poison key) to the vault.
///
/// When `poison` is `true`, the key is registered as a decoy — tapping it
/// produces plausible deniability by appearing to unlock an empty vault.
/// `skip_keys` lists credential IDs of keys that should not participate
/// in the re-sharing ceremony (e.g. keys that are physically unavailable).
/// `pin` is optional and should only be supplied when the inserted key or the
/// re-sharing ceremony requires the current PIN.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2RegisterRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    pub label: String,
    pub poison: Option<bool>,
    pub skip_keys: Option<Vec<String>>,
}

/// Unlock the vault by tapping one or more FIDO2 hardware keys.
///
/// `tap_count` specifies how many keys will be tapped in sequence.
/// Each key's HMAC-secret is used to decrypt its Shamir share; when enough
/// shares are gathered (meeting a compartment's threshold), that compartment
/// unlocks. Higher tap counts unlock higher-threshold compartments. `pins`
/// may be empty for touch-only authenticators; otherwise provide one PIN per
/// round or a single shared PIN for all rounds.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2UnlockRequest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<String>,
    pub tap_count: usize,
}

/// Remove a FIDO2 key from the vault and re-share master keys among remaining keys.
///
/// `pin` is optional and should be provided only when the remaining enrolled
/// keys require their current PIN during the re-sharing ceremony.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2RemoveRequest {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    pub skip_keys: Option<Vec<String>>,
}

/// Set a brand-new FIDO2 PIN on an authenticator that does not have one yet.
///
/// This is intended for fresh hardware keys during setup or before registering
/// an additional backup key. Existing keys with a configured PIN should use
/// vendor tooling or a future dedicated change-PIN flow.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2SetPinRequest {
    pub new_pin: String,
}

// ── Compartment management ──────────────────────────────────────

/// Add a new compartment to the vault with the given authentication threshold.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompartmentAddRequest {
    pub label: String,
    pub threshold: usize,
    pub passphrase_mode: Option<String>,
}

/// Remove a compartment by ID. The compartment's data directory is destroyed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompartmentRemoveRequest {
    pub id: usize,
}

/// Initialize a compartment with a passphrase-derived master key.
///
/// Used for the passphrase-only setup path where no FIDO2 key is involved.
/// Optionally overrides the compartment's label and threshold.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompartmentInitRequest {
    pub id: usize,
    pub passphrase: String,
    pub label: Option<String>,
    pub threshold: Option<usize>,
}

/// Switch the active compartment for the current session.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompartmentSwitchRequest {
    pub id: usize,
}

/// Copy a secret or API key from one unlocked compartment to another.
///
/// `tier` selects the storage tier: 1 for API keys (plaintext), 2 for
/// encrypted secrets. Defaults to tier 2 if omitted.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretsPushRequest {
    pub from_compartment: usize,
    pub to_compartment: usize,
    pub key: String,
    pub new_key: Option<String>,
    pub tier: Option<u8>,
}

// ── Transit encryption ───────────────────────────────────────────

/// Encrypt plaintext using a named transit key from the active compartment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransitEncryptRequest {
    pub key: String,
    pub plaintext_hex: String,
    pub aad_hex: Option<String>,
}

/// Decrypt ciphertext using a named transit key from the active compartment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransitDecryptRequest {
    pub key: String,
    pub nonce_hex: String,
    pub ciphertext_hex: String,
    pub aad_hex: Option<String>,
}

/// Compute an HMAC-SHA256 digest using a named transit key.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransitHmacRequest {
    pub key: String,
    pub input_hex: String,
}

// ── Ethereum stealth wallet operations ──────────────────────────

/// Export a wallet's stealth meta-address for sharing with senders.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthExportRequest {
    pub wallet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_name: Option<String>,
}

/// Generate a fresh stealth address from a stealth meta-address.
///
/// Optionally accepts a pre-determined ephemeral private key for deterministic
/// address generation (useful for deposit tracking).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthGenerateRequest {
    pub stealth_meta_address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral_private_key_hex: Option<String>,
}

/// Check whether a stealth address belongs to a wallet.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthCheckRequest {
    pub wallet: String,
    #[serde(flatten)]
    pub stealth: StealthPaymentRef,
}

/// Sign an arbitrary digest with the stealth private key derived from a payment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthSignRequest {
    pub wallet: String,
    #[serde(flatten)]
    pub stealth: StealthPaymentRef,
    pub digest_hex: String,
}

/// Sign a native ETH transfer from a stealth address.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthSignTransferRequest {
    pub wallet: String,
    #[serde(flatten)]
    pub stealth: StealthPaymentRef,
    #[serde(flatten)]
    pub fees: Eip1559Fees,
    pub nonce: u64,
    pub gas_limit: u64,
    pub destination_address: String,
    pub value_wei_hex: String,
}

/// Sign an ERC-20 token transfer from a stealth address.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthSignErc20TransferRequest {
    pub wallet: String,
    #[serde(flatten)]
    pub stealth: StealthPaymentRef,
    #[serde(flatten)]
    pub fees: Eip1559Fees,
    pub nonce: u64,
    pub gas_limit: u64,
    pub token_address: String,
    pub recipient_address: String,
    pub amount_hex: String,
}

// ── EVM JSON-RPC operations ─────────────────────────────────────

/// Query the pending nonce for an address via JSON-RPC.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmRpcNonceRequest {
    #[serde(flatten)]
    pub provider: EvmProviderRef,
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_tag: Option<String>,
}

/// Query the native balance for an address via JSON-RPC.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmRpcBalanceRequest {
    #[serde(flatten)]
    pub provider: EvmProviderRef,
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_tag: Option<String>,
}

/// Query an ERC-20 token balance via JSON-RPC.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmRpcErc20BalanceRequest {
    #[serde(flatten)]
    pub provider: EvmProviderRef,
    pub token_address: String,
    pub owner_address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_tag: Option<String>,
}

/// Broadcast a signed transaction via JSON-RPC.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmRpcBroadcastRequest {
    #[serde(flatten)]
    pub provider: EvmProviderRef,
    pub raw_transaction_hex: String,
}

/// Sign and optionally broadcast a native ETH transfer from a stealth address.
///
/// This is the "full-control" variant — the caller provides explicit RPC
/// connection details, fee parameters, and compartment overrides. For a
/// simpler interface backed by saved profiles, see [`EthStealthSendWithProfileRequest`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthSendTransferRequest {
    pub rpc_url: String,
    pub wallet: String,
    #[serde(flatten)]
    pub stealth: StealthPaymentRef,
    #[serde(flatten)]
    pub fees: Eip1559Fees,
    pub destination_address: String,
    pub value_wei_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_token_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_compartment_id: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_compartment_id: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broadcast: Option<bool>,
}

/// Sign and optionally broadcast an ERC-20 transfer from a stealth address.
///
/// See [`EthStealthSendTransferRequest`] for the native ETH variant.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthSendErc20TransferRequest {
    pub rpc_url: String,
    pub wallet: String,
    #[serde(flatten)]
    pub stealth: StealthPaymentRef,
    #[serde(flatten)]
    pub fees: Eip1559Fees,
    pub token_address: String,
    pub recipient_address: String,
    pub amount_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_token_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_compartment_id: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_compartment_id: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broadcast: Option<bool>,
}

// ── Profile management ───────────────────────────────────────────

/// Create or update an EVM provider profile (named RPC endpoint configuration).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmProviderProfileUpsertRequest {
    pub name: String,
    #[serde(flatten)]
    pub provider: EvmProviderRef,
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

/// Delete an EVM provider profile by name.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmProfileDeleteRequest {
    pub name: String,
}

/// Create or update a stealth wallet profile (named wallet + provider binding).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthWalletProfileUpsertRequest {
    pub name: String,
    pub wallet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_name: Option<String>,
    pub provider_profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compartment_id: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_destination_address: Option<String>,
}

/// Create or update an xpub receive-wallet profile.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthXpubWalletProfileUpsertRequest {
    pub name: String,
    pub project_account: u32,
    pub provider_profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compartment_id: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_destination_address: Option<String>,
}

/// Import or update an Ethereum seed-phrase wallet profile.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthSeedWalletProfileUpsertRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub mnemonic: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mnemonic_passphrase: Option<String>,
    pub project_account: u32,
    pub provider_profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compartment_id: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_destination_address: Option<String>,
}

/// Export the receive-branch xpub for a saved xpub wallet profile.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthXpubExportRequest {
    pub wallet_profile: String,
}

/// Derive a public receive address from an exported receive-branch xpub.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthXpubDeriveRequest {
    pub xpub: String,
    pub index: u32,
}

// ── Wallet inventory and discovery ─────────────────────────────────

/// Run read-only EVM wallet discovery for imported seed and xpub profiles.
///
/// When no wallet filter is provided, all `eth-seed` and `eth-xpub` profiles
/// are scanned. When no provider filter is provided, every configured EVM
/// provider profile is scanned so one derived address can be checked across
/// multiple L1/L2 networks.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletInventoryScanRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub token_addresses: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_tag: Option<String>,
}

/// Create or update a local chain profile used by discovery and planning.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChainProfileUpsertRequest {
    pub name: String,
    pub chain_family: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explorer_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// Delete a local chain profile.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChainProfileDeleteRequest {
    pub name: String,
}

/// Mutate a persisted discovery job by ID.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryJobMutationRequest {
    pub id: String,
}

/// Generate a dry-run consolidation plan from the current inventory.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanGenerateRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_watch_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_queue_low_risk: Option<bool>,
}

/// Approve reviewable consolidation plan steps.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanApproveRequest {
    pub plan_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub step_ids: Vec<String>,
}

/// Send a native ETH transfer using a saved wallet profile.
///
/// This is the ergonomic variant of [`EthStealthSendTransferRequest`] — the
/// caller references a saved profile instead of providing raw RPC details.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthSendWithProfileRequest {
    pub wallet_profile: String,
    #[serde(flatten)]
    pub stealth: StealthPaymentRef,
    pub value_wei_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broadcast: Option<bool>,
}

/// Send an ERC-20 transfer using a saved wallet profile.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthSendErc20WithProfileRequest {
    pub wallet_profile: String,
    #[serde(flatten)]
    pub stealth: StealthPaymentRef,
    pub token_address: String,
    pub recipient_address: String,
    pub amount_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broadcast: Option<bool>,
}

// ── Queue operations ─────────────────────────────────────────────

mod queue;
pub use queue::*;

// ── Deposit tracking ─────────────────────────────────────────────

/// Create a native ETH deposit monitor for a fresh stealth address.
///
/// When `auto_queue_sweep` is true, a sweep job is automatically enqueued
/// once the deposit is confirmed on-chain.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthDepositCreateNativeRequest {
    pub wallet_profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_value_wei_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_queue_sweep: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sweep_destination_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_sweep_value_wei_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral_private_key_hex: Option<String>,
}

/// Create an ERC-20 deposit monitor for a fresh stealth address.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthDepositCreateErc20Request {
    pub wallet_profile: String,
    pub token_address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_amount_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_queue_sweep: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sweep_destination_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_sweep_amount_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral_private_key_hex: Option<String>,
}

/// Remove a deposit monitor.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthDepositDeleteRequest {
    pub id: String,
}

/// Refresh deposit status by querying on-chain balances.
///
/// When `id` is provided, only that deposit is refreshed. Otherwise, up to
/// `limit` deposits are refreshed in batch. `auto_enqueue` triggers sweep
/// jobs for newly-funded deposits.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthDepositRefreshRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_enqueue: Option<bool>,
}

/// Scan bounded ERC-5564 announcement logs for a stealth wallet profile.
/// `from_block` is required; `token_address` turns matches into ERC-20 deposit
/// candidates instead of native deposit candidates.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthAnnouncementScanRequest {
    pub wallet_profile: String,
    pub from_block: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_block: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_queue_sweep: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sweep_destination_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_sweep_amount_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Enqueue a sweep job for a specific deposit.
///
/// `force` bypasses the minimum-value threshold check.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthDepositEnqueueSweepRequest {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
}

// ── Maintenance ──────────────────────────────────────────────────

/// Run a batch maintenance cycle: refresh deposits and process queued jobs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaintenanceRunRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deposit_refresh_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_process_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_enqueue: Option<bool>,
}

/// Resolve a secret reference into plaintext for command execution.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretResolveRequest {
    pub env_name: String,
    pub reference: String,
}

/// Resolve multiple secret references in a single authenticated request.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretResolveBatchRequest {
    pub entries: Vec<SecretResolveRequest>,
}

/// Record the terminal outcome of a `sigillum run` child process.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunAuditRequest {
    pub program: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
    pub success: bool,
}

/// Atomically generate a secret value and persist it in the active compartment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GenerateStoreRequest {
    pub key: String,
    pub kind: GenerateStoreKind,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum GenerateStoreKind {
    Password {
        length: usize,
        charset: PasswordCharset,
    },
    Passphrase {
        word_count: usize,
        separator: String,
    },
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
