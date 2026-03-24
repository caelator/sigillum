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
/// how many distinct key taps are needed to unlock each compartment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2SetupRequest {
    pub pin: String,
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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2RegisterRequest {
    pub pin: String,
    pub label: String,
    pub poison: Option<bool>,
    pub skip_keys: Option<Vec<String>>,
}

/// Unlock the vault by tapping one or more FIDO2 hardware keys.
///
/// `tap_count` specifies how many keys will be tapped in sequence.
/// Each key's HMAC-secret is used to decrypt its Shamir share; when enough
/// shares are gathered (meeting a compartment's threshold), that compartment
/// unlocks. Higher tap counts unlock higher-threshold compartments.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2UnlockRequest {
    pub pins: Vec<String>,
    pub tap_count: usize,
}

/// Remove a FIDO2 key from the vault and re-share master keys among remaining keys.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2RemoveRequest {
    pub label: String,
    pub pin: String,
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

/// Enqueue a native transfer (reuses the profile-based send structure).
pub type QueueEthStealthTransferRequest = EthStealthSendWithProfileRequest;

/// Enqueue an ERC-20 transfer (reuses the profile-based send structure).
pub type QueueEthStealthErc20TransferRequest = EthStealthSendErc20WithProfileRequest;

/// Enqueue a native ETH sweep from a stealth address.
///
/// A sweep sends the entire balance (minus gas) to a destination address.
/// `min_value_wei_hex` sets a dust threshold — addresses below this amount
/// are skipped during batch sweeps.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueEthStealthNativeSweepRequest {
    pub wallet_profile: String,
    #[serde(flatten)]
    pub stealth: StealthPaymentRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_value_wei_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,
}

/// Enqueue an ERC-20 token sweep from a stealth address.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueEthStealthErc20SweepRequest {
    pub wallet_profile: String,
    #[serde(flatten)]
    pub stealth: StealthPaymentRef,
    pub token_address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_amount_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,
}

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

/// Process queued jobs. When `id` is set, only that job is processed.
/// Otherwise, up to `limit` pending jobs are processed in FIFO order.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueProcessRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

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
    fn test_key_value_request_roundtrip() {
        let req = KeyValueRequest {
            key: "test_key".to_string(),
            value: Some("test_value".to_string()),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_key_value_request_none_value() {
        let req = KeyValueRequest {
            key: "test_key".to_string(),
            value: None,
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_key_only_request_roundtrip() {
        let req = KeyOnlyRequest {
            key: "my_key".to_string(),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_passphrase_request_roundtrip() {
        let req = PassphraseRequest {
            passphrase: "my_secure_passphrase".to_string(),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_snapshot_restore_request_roundtrip() {
        let req = SnapshotRestoreRequest {
            passphrase: "passphrase123".to_string(),
            snapshot_hex: "deadbeef".to_string(),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_setup_reset_request_roundtrip() {
        let req = SetupResetRequest {
            confirmation: "RESET LOCAL SIGILLUM DATA".to_string(),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_compartment_definition_roundtrip() {
        let def = CompartmentDefinition {
            label: "vault_1".to_string(),
            threshold: 2,
            passphrase_mode: Some("FIXED".to_string()),
        };
        roundtrip_test(def);
    }

    #[test]
    fn test_fido2_setup_request_roundtrip() {
        let req = Fido2SetupRequest {
            pin: "1234".to_string(),
            label: "my_key".to_string(),
            compartments: vec![CompartmentDefinition {
                label: "vault_1".to_string(),
                threshold: 1,
                passphrase_mode: None,
            }],
            passphrase: Some("setup_pass".to_string()),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_fido2_set_pin_request_roundtrip() {
        let req = Fido2SetPinRequest {
            new_pin: "2468".to_string(),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_fido2_unlock_request_roundtrip() {
        let req = Fido2UnlockRequest {
            pins: vec!["1234".to_string(), "5678".to_string()],
            tap_count: 3,
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_compartment_init_request_roundtrip() {
        let req = CompartmentInitRequest {
            id: 1,
            passphrase: "init_pass".to_string(),
            label: Some("new_label".to_string()),
            threshold: Some(2),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_secrets_push_request_roundtrip() {
        let req = SecretsPushRequest {
            from_compartment: 1,
            to_compartment: 2,
            key: "secret_key".to_string(),
            new_key: Some("renamed_key".to_string()),
            tier: Some(3),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_transit_encrypt_request_roundtrip() {
        let req = TransitEncryptRequest {
            key: "encryption_key".to_string(),
            plaintext_hex: "48656c6c6f".to_string(),
            aad_hex: Some("aabbccdd".to_string()),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_transit_decrypt_request_with_aad() {
        let req = TransitDecryptRequest {
            key: "encryption_key".to_string(),
            nonce_hex: "0123456789abcdef".to_string(),
            ciphertext_hex: "fedcba9876543210".to_string(),
            aad_hex: Some("aabbccdd".to_string()),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_transit_decrypt_request_no_aad() {
        let req = TransitDecryptRequest {
            key: "encryption_key".to_string(),
            nonce_hex: "0123456789abcdef".to_string(),
            ciphertext_hex: "fedcba9876543210".to_string(),
            aad_hex: None,
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_eth_stealth_export_request_roundtrip() {
        let req = EthStealthExportRequest {
            wallet: "0xabc123".to_string(),
            short_name: Some("my_wallet".to_string()),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_eth_stealth_export_request_no_short_name() {
        let req = EthStealthExportRequest {
            wallet: "0xabc123".to_string(),
            short_name: None,
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_eth_stealth_generate_request_roundtrip() {
        let req = EthStealthGenerateRequest {
            stealth_meta_address: "st:0x...".to_string(),
            ephemeral_private_key_hex: Some("abcd1234".to_string()),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_eth_stealth_check_request_roundtrip() {
        let req = EthStealthCheckRequest {
            wallet: "0xwallet".to_string(),
            stealth: StealthPaymentRef {
                stealth_address: "0xstealth".to_string(),
                ephemeral_public_key_hex: "0xephemeral".to_string(),
                view_tag_hex: Some("0xaa".to_string()),
            },
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_evm_rpc_nonce_request_full() {
        let req = EvmRpcNonceRequest {
            provider: EvmProviderRef {
                rpc_url: "https://rpc.example.com".to_string(),
                auth_token_key: Some("token_key".to_string()),
                compartment_id: Some(1),
            },
            address: "0xaddress".to_string(),
            block_tag: Some("latest".to_string()),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_evm_rpc_nonce_request_minimal() {
        let req = EvmRpcNonceRequest {
            provider: EvmProviderRef {
                rpc_url: "https://rpc.example.com".to_string(),
                auth_token_key: None,
                compartment_id: None,
            },
            address: "0xaddress".to_string(),
            block_tag: None,
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_evm_rpc_balance_request_roundtrip() {
        let req = EvmRpcBalanceRequest {
            provider: EvmProviderRef {
                rpc_url: "https://rpc.example.com".to_string(),
                auth_token_key: None,
                compartment_id: Some(2),
            },
            address: "0xaddress".to_string(),
            block_tag: Some("safe".to_string()),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_evm_rpc_erc20_balance_request_roundtrip() {
        let req = EvmRpcErc20BalanceRequest {
            provider: EvmProviderRef {
                rpc_url: "https://rpc.example.com".to_string(),
                auth_token_key: Some("key".to_string()),
                compartment_id: None,
            },
            token_address: "0xtoken".to_string(),
            owner_address: "0xowner".to_string(),
            block_tag: None,
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_evm_rpc_broadcast_request_roundtrip() {
        let req = EvmRpcBroadcastRequest {
            provider: EvmProviderRef {
                rpc_url: "https://rpc.example.com".to_string(),
                auth_token_key: None,
                compartment_id: None,
            },
            raw_transaction_hex: "0xrxn".to_string(),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_eth_stealth_send_transfer_request_full() {
        let req = EthStealthSendTransferRequest {
            rpc_url: "https://rpc.example.com".to_string(),
            wallet: "0xwallet".to_string(),
            stealth: StealthPaymentRef {
                stealth_address: "0xstealth".to_string(),
                ephemeral_public_key_hex: "0xeph".to_string(),
                view_tag_hex: Some("0xaa".to_string()),
            },
            fees: Eip1559Fees {
                chain_id: 1,
                max_priority_fee_per_gas_hex: "0x1".to_string(),
                max_fee_per_gas_hex: "0x2".to_string(),
            },
            destination_address: "0xdest".to_string(),
            value_wei_hex: "0x100".to_string(),
            auth_token_key: Some("key".to_string()),
            provider_compartment_id: Some(1),
            wallet_compartment_id: Some(2),
            nonce: Some(5),
            gas_limit: Some(21000),
            broadcast: Some(true),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_eth_stealth_send_transfer_request_minimal() {
        let req = EthStealthSendTransferRequest {
            rpc_url: "https://rpc.example.com".to_string(),
            wallet: "0xwallet".to_string(),
            stealth: StealthPaymentRef {
                stealth_address: "0xstealth".to_string(),
                ephemeral_public_key_hex: "0xeph".to_string(),
                view_tag_hex: None,
            },
            fees: Eip1559Fees {
                chain_id: 1,
                max_priority_fee_per_gas_hex: "0x1".to_string(),
                max_fee_per_gas_hex: "0x2".to_string(),
            },
            destination_address: "0xdest".to_string(),
            value_wei_hex: "0x100".to_string(),
            auth_token_key: None,
            provider_compartment_id: None,
            wallet_compartment_id: None,
            nonce: None,
            gas_limit: None,
            broadcast: None,
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_eth_stealth_send_erc20_transfer_request_roundtrip() {
        let req = EthStealthSendErc20TransferRequest {
            rpc_url: "https://rpc.example.com".to_string(),
            wallet: "0xwallet".to_string(),
            stealth: StealthPaymentRef {
                stealth_address: "0xstealth".to_string(),
                ephemeral_public_key_hex: "0xeph".to_string(),
                view_tag_hex: None,
            },
            fees: Eip1559Fees {
                chain_id: 1,
                max_priority_fee_per_gas_hex: "0x1".to_string(),
                max_fee_per_gas_hex: "0x2".to_string(),
            },
            token_address: "0xtoken".to_string(),
            recipient_address: "0xrecipient".to_string(),
            amount_hex: "0x100".to_string(),
            auth_token_key: None,
            provider_compartment_id: None,
            wallet_compartment_id: None,
            nonce: Some(3),
            gas_limit: None,
            broadcast: Some(false),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_evm_provider_profile_upsert_request_full() {
        let req = EvmProviderProfileUpsertRequest {
            name: "mainnet".to_string(),
            provider: EvmProviderRef {
                rpc_url: "https://eth.example.com".to_string(),
                auth_token_key: Some("key".to_string()),
                compartment_id: Some(1),
            },
            chain_id: 1,
            max_priority_fee_per_gas_hex: Some("0x3b9aca00".to_string()),
            max_fee_per_gas_hex: Some("0x5f5e100".to_string()),
            native_gas_limit: Some(21000),
            erc20_gas_limit: Some(65000),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_evm_provider_profile_upsert_request_minimal() {
        let req = EvmProviderProfileUpsertRequest {
            name: "testnet".to_string(),
            provider: EvmProviderRef {
                rpc_url: "https://test.example.com".to_string(),
                auth_token_key: None,
                compartment_id: None,
            },
            chain_id: 5,
            max_priority_fee_per_gas_hex: None,
            max_fee_per_gas_hex: None,
            native_gas_limit: None,
            erc20_gas_limit: None,
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_eth_stealth_send_with_profile_request_roundtrip() {
        let req = EthStealthSendWithProfileRequest {
            wallet_profile: "my_profile".to_string(),
            stealth: StealthPaymentRef {
                stealth_address: "0xstealth".to_string(),
                ephemeral_public_key_hex: "0xeph".to_string(),
                view_tag_hex: None,
            },
            value_wei_hex: "0x100".to_string(),
            destination_address: Some("0xdest".to_string()),
            nonce: Some(5),
            gas_limit: Some(21000),
            broadcast: Some(true),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_eth_stealth_send_erc20_with_profile_request_roundtrip() {
        let req = EthStealthSendErc20WithProfileRequest {
            wallet_profile: "profile1".to_string(),
            stealth: StealthPaymentRef {
                stealth_address: "0xstealth".to_string(),
                ephemeral_public_key_hex: "0xeph".to_string(),
                view_tag_hex: Some("0xff".to_string()),
            },
            token_address: "0xtoken".to_string(),
            recipient_address: "0xrecip".to_string(),
            amount_hex: "0x64".to_string(),
            nonce: None,
            gas_limit: Some(100000),
            broadcast: None,
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_queue_eth_stealth_native_sweep_request_full() {
        let req = QueueEthStealthNativeSweepRequest {
            wallet_profile: "profile".to_string(),
            stealth: StealthPaymentRef {
                stealth_address: "0xstealth".to_string(),
                ephemeral_public_key_hex: "0xeph".to_string(),
                view_tag_hex: Some("0xaa".to_string()),
            },
            destination_address: Some("0xdest".to_string()),
            min_value_wei_hex: Some("0x1".to_string()),
            gas_limit: Some(21000),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_eth_stealth_deposit_create_native_request_roundtrip() {
        let req = EthStealthDepositCreateNativeRequest {
            wallet_profile: "profile1".to_string(),
            expected_value_wei_hex: Some("0x100".to_string()),
            auto_queue_sweep: Some(true),
            sweep_destination_address: Some("0xsweep".to_string()),
            min_sweep_value_wei_hex: Some("0x10".to_string()),
            note: Some("test deposit".to_string()),
            ephemeral_private_key_hex: None,
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_eth_stealth_deposit_create_erc20_request_roundtrip() {
        let req = EthStealthDepositCreateErc20Request {
            wallet_profile: "profile2".to_string(),
            token_address: "0xtoken".to_string(),
            expected_amount_hex: Some("0x1000".to_string()),
            auto_queue_sweep: Some(false),
            sweep_destination_address: None,
            min_sweep_amount_hex: None,
            note: None,
            ephemeral_private_key_hex: Some("0xkey".to_string()),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_eth_stealth_deposit_delete_request_roundtrip() {
        let req = EthStealthDepositDeleteRequest {
            id: "deposit_123".to_string(),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_eth_stealth_deposit_refresh_request_all_options() {
        let req = EthStealthDepositRefreshRequest {
            id: Some("deposit_456".to_string()),
            limit: Some(10),
            auto_enqueue: Some(true),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_eth_stealth_deposit_refresh_request_empty() {
        let req = EthStealthDepositRefreshRequest {
            id: None,
            limit: None,
            auto_enqueue: None,
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_eth_stealth_deposit_enqueue_sweep_request_roundtrip() {
        let req = EthStealthDepositEnqueueSweepRequest {
            id: "deposit_789".to_string(),
            force: Some(true),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_maintenance_run_request_full() {
        let req = MaintenanceRunRequest {
            deposit_refresh_limit: Some(50),
            queue_process_limit: Some(100),
            auto_enqueue: Some(true),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_maintenance_run_request_empty() {
        let req = MaintenanceRunRequest {
            deposit_refresh_limit: None,
            queue_process_limit: None,
            auto_enqueue: None,
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_queue_process_request_with_id() {
        let req = QueueProcessRequest {
            id: Some("job_123".to_string()),
            limit: Some(5),
        };
        roundtrip_test(req);
    }

    #[test]
    fn test_queue_process_request_empty() {
        let req = QueueProcessRequest {
            id: None,
            limit: None,
        };
        roundtrip_test(req);
    }
}
