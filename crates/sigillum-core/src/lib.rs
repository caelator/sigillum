//! # Sigillum Core
//!
//! Traits, shared utilities, and file-backed implementation for secure secret management.
//!
//! ## Two-Tier Secret Model
//!
//! Secrets are split into two access tiers:
//! - **Tier 1 (API Keys)**: Stored in plaintext, always readable. No unlock required.
//!   Used for non-secret configuration (e.g., provider URLs). Stored in `api_keys.json`.
//! - **Tier 2 (Secrets)**: Encrypted with AES-256-GCM, requires master key unlock.
//!   Used for sensitive data (credentials, private keys, API tokens). Stored in `vault.enc`.
//!
//! This separation enables fast access to non-sensitive metadata while protecting
//! sensitive secrets behind strong encryption.
//!
//! ## Vault Abstraction
//!
//! The `SecretStore` and `VaultLifecycle` traits abstract over vault implementations.
//! `FileVault` is the current file-backed implementation, storing secrets in
//! `~/.sigillum/` with encrypted payloads and backup copies for durability.
//!
//! The vault can be in one of two states:
//! - **Locked**: Master key is zeroized; Tier 2 reads fail with `Locked` error.
//! - **Unlocked**: Master key is loaded in memory; Tier 1 and Tier 2 reads succeed.
//!
//! ## Ethereum Stealth Address Integration
//!
//! The `ethereum_stealth` module implements EIP-5564 stealth address protocols,
//! enabling private transfers on Ethereum. The vault can derive stealth wallets,
//! sign stealth payments, and generate ephemeral receivers. This is the primary
//! use case for Tier 2 secrets (storing ephemeral private keys).

mod error;
mod ethereum_stealth;
mod ethereum_xpub;
pub mod payload;
mod protected_secret;
mod traits;
pub mod unlock;

pub use error::VaultError;
pub use ethereum_stealth::{
    ERC5564_ANNOUNCE_FUNCTION, ERC5564_ANNOUNCER_ADDRESS, ETHEREUM_STEALTH_SCHEME_ID,
    EthereumEip1559Erc20Transfer, EthereumEip1559Transfer, EthereumSignedTransaction,
    EthereumStealthAnnouncement, EthereumStealthCheck, EthereumStealthError,
    EthereumStealthMetaAddress, EthereumStealthPayment, EthereumStealthSignature,
    EthereumStealthWallet, build_erc5564_announcement, check_ethereum_stealth_address,
    decode_quantity_hex, derive_sigillum_ethereum_stealth_wallet, encode_erc5564_announce_calldata,
    generate_ethereum_stealth_address, sign_ethereum_eip1559_transaction,
    sign_ethereum_erc20_transfer, sign_ethereum_native_transfer, sign_ethereum_stealth_digest,
    sign_ethereum_stealth_erc20_transfer, sign_ethereum_stealth_native_transfer,
};
pub use ethereum_xpub::{
    ETHEREUM_XPUB_COIN_TYPE, ETHEREUM_XPUB_CONTROL_BRANCH, ETHEREUM_XPUB_PURPOSE,
    ETHEREUM_XPUB_RECEIVE_BRANCH, EthereumXpubError, EthereumXpubReceiveAddress,
    EthereumXpubReceiveExport, derive_ethereum_account_xpub_from_mnemonic,
    derive_ethereum_address_from_account_xpub, derive_ethereum_address_from_control_xpub,
    derive_ethereum_address_from_imported_xpub, derive_ethereum_address_from_xpub,
    derive_ethereum_receive_branch_from_account_xpub,
    derive_ethereum_receive_branch_from_account_xpub_with_path,
    derive_ethereum_xpub_control_branch_from_mnemonic,
    derive_ethereum_xpub_receive_branch_from_mnemonic, derive_private_key_at_path,
    derive_sigillum_ethereum_xpub_control_branch, derive_sigillum_ethereum_xpub_receive_address,
    derive_sigillum_ethereum_xpub_receive_branch, ethereum_mnemonic_word_count,
    generate_ethereum_mnemonic, validate_ethereum_imported_xpub_path,
};
pub use protected_secret::PinnedSecretBytes;
pub use traits::{SecretStore, VaultLifecycle};
pub use unlock::provider::{
    Fido2UnlockProvider, PassphraseUnlockProvider, TouchIdUnlockProvider, UnlockProvider,
};

#[cfg(feature = "file-backend")]
pub mod utils;

#[cfg(feature = "file-backend")]
mod file_vault;
#[cfg(feature = "file-backend")]
pub use file_vault::{FileVault, VaultConfig};

#[cfg(feature = "file-backend")]
mod snapshot;
#[cfg(feature = "file-backend")]
pub use snapshot::{SnapshotSummary, export_encrypted_snapshot, restore_encrypted_snapshot};
#[cfg(feature = "file-backend")]
pub use snapshot::{inspect_encrypted_snapshot, recover_snapshot_restore};
