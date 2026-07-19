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
    /// Shared-secret hash convention of the payment (`"compressed33"` standard,
    /// `"x32"` legacy). Absent means unknown: the daemon probes both
    /// conventions (standard first) and verifies by address match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stealth_hash_convention: Option<sigillum_core::StealthHashConvention>,
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

// ── Secret generation ─────────────────────────────────────────

mod generate;
pub use generate::*;

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

/// Mint a scoped daemon token from an existing full session.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilitySessionRequest {
    pub scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<u64>,
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

// ── FIDO2 hardware key operations ────────────────────────────────

mod fido2;
pub use fido2::*;

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
///
/// The response is public payer-facing information only (spending + viewing
/// public keys); the daemon derives it via the watch-only path, without
/// retaining the spending private key.
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
///
/// Watch-only per EIP-5564 `checkStealthAddress`: the daemon runs this from
/// the viewing private key + spending PUBLIC key; the spending private key
/// never enters the check path (the vault must still be unlocked, since the
/// viewing key derives from the compartment master key).
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

/// Estimate EIP-1559 fees from an EVM JSON-RPC provider.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmFeeEstimateRequest {
    #[serde(flatten)]
    pub provider: EvmProviderRef,
    pub chain_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_estimation_enabled: Option<bool>,
}

/// Delete an EVM provider profile by name.
///
/// Shared by every profile delete route (`profiles/evm|eth-stealth|eth-xpub|eth-seed
/// /delete`). `prune_inventory` opts into the forget cascade (plan task 3.2):
/// when true, the profile's wallet-inventory rows (scanned addresses,
/// holdings, scan state), its receive allocations, and the counterparty
/// bindings those allocations carried are removed in the same guarded
/// operation. Absent/false preserves the legacy behavior exactly: only the
/// profile record (and, for seed wallets, the vault secret) is removed and
/// inventory history is left behind.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmProfileDeleteRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prune_inventory: Option<bool>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_enabled: Option<bool>,
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
    pub external_receive_xpub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_receive_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_account_xpub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_account_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_destination_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_enabled: Option<bool>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_enabled: Option<bool>,
}
/// Create a brand-new Ethereum seed-phrase wallet profile from a
/// server-generated BIP-39 mnemonic.
///
/// Unlike [`EthSeedWalletProfileUpsertRequest`], the caller supplies no
/// mnemonic: the daemon generates fresh entropy, derives the profile, and
/// returns the phrase exactly once in
/// [`crate::response::EthSeedWalletCreateResponse`] for operator backup.
/// Creation never overwrites an existing profile of the same name.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthSeedWalletCreateRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Number of BIP-39 words to generate: 12 or 24. Defaults to 24.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_count: Option<usize>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_enabled: Option<bool>,
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

mod inventory;
pub use inventory::*;

// ── List pagination / filtering / sorting ──────────────────────────

mod pagination;
pub use pagination::*;

// ── Treasury policy ─────────────────────────────────────────────────

mod treasury;
pub use treasury::*;

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
    pub estimate_fees: Option<bool>,
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
    pub estimate_fees: Option<bool>,
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
///
/// `request_gas` asks the payer to attach gas for the recipient's subsequent
/// sweep (the EIP-5564 "Recipients' transaction costs" sponsorship pattern):
/// the announcement metadata then follows the EIP-5564 native-token SHOULD
/// layout (`view tag ‖ 0xeeeeeeee ‖ sentinel address ‖ amount`) whose amount
/// is the expected value plus the requested gas, so a standards-aware payer
/// wallet learns the total native value to attach. `gas_amount_wei_hex` is
/// the requested gas; when omitted, the provider profile's static sweep gas
/// estimate is used.
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_gas: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_amount_wei_hex: Option<String>,
}

/// Create an ERC-20 deposit monitor for a fresh stealth address.
///
/// `request_gas` asks the payer to attach native gas for the recipient's
/// subsequent sweep: the announcement metadata then follows the EIP-5564
/// token SHOULD layout (`view tag ‖ transfer(address,uint256) selector ‖
/// token contract ‖ amount`), so a standards-aware payer wallet learns the
/// asset and amount to send; the requested gas amount
/// (`gas_amount_wei_hex`, defaulting to the provider profile's static sweep
/// gas estimate) is recorded on the deposit and shown in the payment
/// instructions as the native amount to attach alongside the token transfer.
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_gas: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_amount_wei_hex: Option<String>,
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
/// `from_block` is optional: when omitted the scan resumes from the persisted
/// per-(wallet, provider) announcement cursor (or `earliest` on the first
/// scan); when supplied it wins over the cursor for manual rescans. A
/// successful scan advances the cursor. `reset_cursor` first drops the
/// stored cursor, so the scan re-anchors from the given `from_block` (or
/// `earliest`); if a successful scan has no trustworthy numeric anchor, the
/// cursor remains absent. A legacy block-only cursor must replay full history
/// before it can become an exact-position cursor; while that migration is
/// pending, an explicit non-genesis `from_block` without `reset_cursor`
/// returns 409.
/// `token_address` turns matches into ERC-20 deposit candidates instead of
/// native deposit candidates.
///
/// Detection is watch-only: matching uses the viewing private key + spending
/// PUBLIC key only; the spending private key is never loaded for scanning
/// (the wallet compartment must still be unlocked — the viewing key derives
/// from its master key).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthAnnouncementScanRequest {
    pub wallet_profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_block: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_cursor: Option<bool>,
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

// ── Self-check ───────────────────────────────────────────────────

/// Run operator self-checks across configured subsystems.
///
/// `domains` filters which check domains run; an empty list (the serde
/// default) runs every domain. Known domains are `provider`, `seed-wallet`,
/// `xpub-wallet`, `stealth-wallet`, `watch-book`, `policy`,
/// `receive-allocation`, and `fido2` — anything else fails validation.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelfCheckRunRequest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
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
    /// Run the cycle as a background daemon operation instead of blocking the
    /// request until it completes.
    ///
    /// When `true`, the daemon validates the request, starts an `Operation`
    /// of kind `maintenance_run` (see `GET /api/operations`) that drives the
    /// same maintenance pipeline in a spawned task, and returns immediately
    /// with the operation tracking it. Absent or `false` keeps the original
    /// synchronous behavior, so existing clients see no contract change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_async: Option<bool>,
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

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
