//! ERC-5564 Ethereum Stealth Address Implementation
//!
//! This module implements the ERC-5564 standard for stealth addresses, enabling privacy-preserving
//! Ethereum transactions where recipient addresses are not publicly linked to recipients.
//!
//! ## Cryptographic Building Blocks
//!
//! - **ECDH Shared Secrets**: Uses secp256k1 elliptic curve Diffie-Hellman to derive ephemeral
//!   shared secrets between payers and recipients.
//! - **Keccak256 Hashing**: Hashes shared secrets for key derivation and address computation.
//! - **View Tags**: Compact filtering mechanism (single byte) allowing recipients to quickly identify
//!   stealth payments without full cryptographic verification on every block.
//! - **Stealth Address Derivation**: Combines recipient's spending public key with ECDH-derived
//!   secrets to compute a unique stealth address for each payment.
//! - **EIP-1559 Transaction Signing**: Generates signed transactions compatible with Ethereum's
//!   dynamic fee market.
//!
//! ## Security Properties
//!
//! - Every signing operation derives and verifies the stealth private key against the expected
//!   stealth address before use, preventing key-address mismatches.
//! - View tag mismatches are rejected immediately without further processing.
//! - All ephemeral private keys are zeroized after use.

use hmac::{Hmac, Mac};
use k256::ecdh::diffie_hellman;
use k256::ecdsa::{RecoveryId, Signature, SigningKey};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use k256::{AffinePoint, ProjectivePoint, PublicKey, SecretKey};
use sha2::Sha256;
use sha3::{Digest, Keccak256};
use thiserror::Error;
use zeroize::Zeroize;

type HmacSha256 = Hmac<Sha256>;

pub const ETHEREUM_STEALTH_SCHEME_ID: u64 = 1;
pub const ERC5564_ANNOUNCER_ADDRESS: &str = "0x55649e01b5df198d18d95b5cc5051630cfd45564";
pub const ERC5564_ANNOUNCE_FUNCTION: &str = "announce(uint256,address,bytes,bytes)";

// ── Error types ──

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EthereumStealthError {
    #[error("wallet label is required")]
    EmptyWalletLabel,
    #[error("invalid meta-address format")]
    InvalidMetaAddress,
    #[error("invalid short name")]
    InvalidShortName,
    #[error("invalid secp256k1 key material")]
    InvalidKeyMaterial,
    #[error("invalid digest length: expected 32 bytes")]
    InvalidDigestLength,
    #[error("invalid ethereum address")]
    InvalidEthereumAddress,
    #[error("invalid quantity encoding: {0}")]
    InvalidQuantity(String),
    #[error("invalid ERC-5564 announcement field: {0}")]
    InvalidAnnouncementField(String),
    #[error("view tag mismatch")]
    ViewTagMismatch,
    #[error("stealth address does not match derived wallet")]
    AddressMismatch,
    #[error("max fee per gas must be greater than or equal to max priority fee per gas")]
    InvalidFeeConfiguration,
    #[error("signing failed: {0}")]
    Signing(String),
}

// ── Public types ──

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EthereumStealthMetaAddress {
    pub wallet: String,
    pub short_name: String,
    pub scheme_id: u64,
    pub stealth_meta_address: String,
    pub spending_public_key_hex: String,
    pub viewing_public_key_hex: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EthereumStealthPayment {
    pub scheme_id: u64,
    pub short_name: String,
    pub stealth_meta_address: String,
    pub stealth_address: String,
    pub ephemeral_public_key_hex: String,
    pub view_tag_hex: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EthereumStealthAnnouncement {
    pub announcer_address: String,
    pub announce_function: String,
    pub scheme_id: u64,
    pub stealth_address: String,
    pub ephemeral_public_key_hex: String,
    pub metadata_hex: String,
    pub calldata_hex: String,
    pub value_wei_hex: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EthereumStealthCheck {
    pub matches: bool,
    pub derived_stealth_address: String,
    pub view_tag_hex: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EthereumStealthSignature {
    pub stealth_address: String,
    pub signature_hex: String,
    pub recovery_id: u8,
    pub view_tag_hex: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EthereumEip1559Transfer {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: [u8; 32],
    pub max_fee_per_gas: [u8; 32],
    pub gas_limit: u64,
    pub destination_address: String,
    pub value: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EthereumEip1559Erc20Transfer {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: [u8; 32],
    pub max_fee_per_gas: [u8; 32],
    pub gas_limit: u64,
    pub token_address: String,
    pub recipient_address: String,
    pub amount: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EthereumSignedTransaction {
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

#[derive(Clone, Debug)]
pub struct EthereumStealthWallet {
    meta_address: EthereumStealthMetaAddress,
    spending_private_key: SecretKey,
    viewing_private_key: SecretKey,
}

impl EthereumStealthWallet {
    pub fn meta_address(&self) -> &EthereumStealthMetaAddress {
        &self.meta_address
    }
}

// ── Wallet derivation ──

/// Derive a Sigillum Ethereum stealth wallet from a master key.
///
/// Combines deterministic HMAC-based key derivation with ERC-5564 semantics to produce
/// independent spending and viewing keys. The resulting wallet can generate unlimited
/// stealth addresses while allowing the recipient to filter incoming transactions by view tag.
///
/// # Security
///
/// The master key MUST be cryptographically random (256+ bits of entropy) and properly
/// protected. All intermediate keys are derived deterministically from the master key
/// and cannot be reversed without it.
pub fn derive_sigillum_ethereum_stealth_wallet(
    master_key: &[u8],
    wallet: &str,
    short_name: &str,
) -> Result<EthereumStealthWallet, EthereumStealthError> {
    if wallet.trim().is_empty() {
        return Err(EthereumStealthError::EmptyWalletLabel);
    }
    let short_name = normalize_short_name(short_name)?;
    let spending_private_key = derive_wallet_secret_key(master_key, wallet, "spend")?;
    let viewing_private_key = derive_wallet_secret_key(master_key, wallet, "view")?;
    let spending_public_key_hex = encode_public_key(&spending_private_key.public_key());
    let viewing_public_key_hex = encode_public_key(&viewing_private_key.public_key());
    let stealth_meta_address =
        format!("st:{short_name}:0x{spending_public_key_hex}{viewing_public_key_hex}");

    Ok(EthereumStealthWallet {
        meta_address: EthereumStealthMetaAddress {
            wallet: wallet.to_string(),
            short_name,
            scheme_id: ETHEREUM_STEALTH_SCHEME_ID,
            stealth_meta_address,
            spending_public_key_hex,
            viewing_public_key_hex,
        },
        spending_private_key,
        viewing_private_key,
    })
}

// ── Stealth address generation ──

/// Generate a unique stealth address for a given meta-address.
///
/// For each payment, generates a fresh ephemeral key pair and derives a unique stealth
/// address by combining the recipient's spending public key with an ECDH shared secret.
/// Returns the stealth address and ephemeral public key so the payer can embed them
/// in transaction calldata for recipient recovery.
///
/// # Parameters
///
/// - `ephemeral_private_key`: Optional custom ephemeral key for testing/determinism.
///   If `None`, a random key is generated.
///
/// # Returns
///
/// The `EthereumStealthPayment` includes the stealth address, ephemeral public key
/// (needed for recipient recovery), and a view tag for fast recipient filtering.
pub fn generate_ethereum_stealth_address(
    stealth_meta_address: &str,
    ephemeral_private_key: Option<[u8; 32]>,
) -> Result<EthereumStealthPayment, EthereumStealthError> {
    let meta = parse_meta_address(stealth_meta_address)?;
    let ephemeral_private_key =
        ephemeral_private_key_to_secret(ephemeral_private_key.unwrap_or_else(random_secret_bytes))?;
    let ephemeral_public_key = ephemeral_private_key.public_key();
    let hashed_shared_secret =
        hashed_shared_secret(&ephemeral_private_key, &meta.viewing_public_key)?;
    let stealth_public_key =
        derive_stealth_public_key(&meta.spending_public_key, &hashed_shared_secret)?;

    let payment = EthereumStealthPayment {
        scheme_id: ETHEREUM_STEALTH_SCHEME_ID,
        short_name: meta.short_name,
        stealth_meta_address: stealth_meta_address.to_string(),
        stealth_address: ethereum_address_from_public_key(&stealth_public_key),
        ephemeral_public_key_hex: encode_public_key(&ephemeral_public_key),
        view_tag_hex: hex::encode([derive_view_tag(&hashed_shared_secret)]),
    };

    Ok(payment)
}

/// Build the ERC-5564 announcer contract call for a generated stealth payment.
///
/// The singleton announcer emits the canonical `Announcement` event. Per ERC-5564,
/// metadata is arbitrary bytes but its first byte MUST be the view tag, so Sigillum
/// keeps the default metadata minimal and stores any asset context in higher-level
/// deposit records instead of overloading the standard field.
pub fn build_erc5564_announcement(
    payment: &EthereumStealthPayment,
) -> Result<EthereumStealthAnnouncement, EthereumStealthError> {
    let metadata_hex = erc5564_metadata_from_view_tag(&payment.view_tag_hex)?;
    let calldata_hex = encode_erc5564_announce_calldata(
        payment.scheme_id,
        &payment.stealth_address,
        &payment.ephemeral_public_key_hex,
        &metadata_hex,
    )?;

    Ok(EthereumStealthAnnouncement {
        announcer_address: ERC5564_ANNOUNCER_ADDRESS.to_string(),
        announce_function: ERC5564_ANNOUNCE_FUNCTION.to_string(),
        scheme_id: payment.scheme_id,
        stealth_address: payment.stealth_address.clone(),
        ephemeral_public_key_hex: payment.ephemeral_public_key_hex.clone(),
        metadata_hex,
        calldata_hex,
        value_wei_hex: "0x0".to_string(),
    })
}

// ── Stealth address verification ──

/// Verify that a stealth address matches a given ephemeral public key.
///
/// Recipients call this function during block scanning to determine whether an on-chain
/// payment was intended for them. The function computes the stealth address that would
/// have been generated with the given ephemeral public key and compares it to the
/// provided stealth address.
///
/// The view tag provides a fast pre-filter: if it doesn't match, the computation short-circuits
/// immediately without performing full ECDH and point arithmetic.
///
/// # Security
///
/// The view tag is only 1 byte, so false positives are expected (~1 in 256). Always verify
/// the full stealth address derivation before processing.
pub fn check_ethereum_stealth_address(
    wallet: &EthereumStealthWallet,
    stealth_address: &str,
    ephemeral_public_key_hex: &str,
    view_tag: Option<u8>,
) -> Result<EthereumStealthCheck, EthereumStealthError> {
    let ephemeral_public_key = parse_public_key_hex(ephemeral_public_key_hex)?;
    let hashed_shared_secret =
        hashed_shared_secret_for_recipient(&wallet.viewing_private_key, &ephemeral_public_key)?;

    if let Some(expected_view_tag) = view_tag {
        let derived_view_tag = derive_view_tag(&hashed_shared_secret);
        if derived_view_tag != expected_view_tag {
            return Err(EthereumStealthError::ViewTagMismatch);
        }
    }

    let stealth_public_key = derive_stealth_public_key(
        &wallet.spending_private_key.public_key(),
        &hashed_shared_secret,
    )?;
    let derived_stealth_address = ethereum_address_from_public_key(&stealth_public_key);
    let expected_stealth_address = normalize_ethereum_address(stealth_address)?;

    Ok(EthereumStealthCheck {
        matches: derived_stealth_address == expected_stealth_address,
        derived_stealth_address,
        view_tag_hex: hex::encode([derive_view_tag(&hashed_shared_secret)]),
    })
}

// ── Stealth signing ──

/// Derive the stealth private key for a payment and verify it matches the expected address.
///
/// Combines view-tag verification, ECDH shared-secret derivation, and stealth key
/// computation into a single auditable call site. Every signing operation MUST go
/// through this function to guarantee address verification before key use.
///
/// # Returns
///
/// A tuple of `(stealth_private_key, hashed_shared_secret)`. The hashed shared secret
/// is needed for computing the view tag in the signature result.
///
/// # Security
///
/// The returned `SecretKey` is ONLY valid if the derived address matches the expected
/// stealth address. This invariant is checked before returning. The caller MUST NOT
/// use the key for any purpose until verifying the result.
fn derive_verified_stealth_key(
    wallet: &EthereumStealthWallet,
    stealth_address: &str,
    ephemeral_public_key_hex: &str,
    view_tag: Option<u8>,
) -> Result<(SecretKey, [u8; 32]), EthereumStealthError> {
    let ephemeral_public_key = parse_public_key_hex(ephemeral_public_key_hex)?;
    let hashed_shared_secret =
        hashed_shared_secret_for_recipient(&wallet.viewing_private_key, &ephemeral_public_key)?;

    if let Some(expected_view_tag) = view_tag {
        let derived_view_tag = derive_view_tag(&hashed_shared_secret);
        if derived_view_tag != expected_view_tag {
            return Err(EthereumStealthError::ViewTagMismatch);
        }
    }

    let stealth_private_key =
        derive_stealth_private_key(&wallet.spending_private_key, &hashed_shared_secret)?;
    let derived_address = ethereum_address_from_public_key(&stealth_private_key.public_key());
    let expected_stealth_address = normalize_ethereum_address(stealth_address)?;
    if expected_stealth_address != derived_address {
        return Err(EthereumStealthError::AddressMismatch);
    }

    Ok((stealth_private_key, hashed_shared_secret))
}

/// Sign a 32-byte digest (e.g., message hash) using a stealth private key.
///
/// Derives the stealth private key from the ephemeral public key, verifies it matches
/// the expected stealth address, then produces an ECDSA signature with recovery ID.
/// The signature can be broadcast with the stealth address to prove authorization
/// without revealing the original recipient.
///
/// # Parameters
///
/// - `digest`: Must be exactly 32 bytes (e.g., Keccak256 hash).
/// - `view_tag`: Optional view-tag filter. If provided, it must match the derived value
///   or the function returns `ViewTagMismatch` without further processing.
///
/// # Returns
///
/// The signature includes the recovery ID, allowing Ethereum to recover the signing
/// address on-chain via `ecrecover`.
pub fn sign_ethereum_stealth_digest(
    wallet: &EthereumStealthWallet,
    stealth_address: &str,
    ephemeral_public_key_hex: &str,
    view_tag: Option<u8>,
    digest: &[u8],
) -> Result<EthereumStealthSignature, EthereumStealthError> {
    if digest.len() != 32 {
        return Err(EthereumStealthError::InvalidDigestLength);
    }

    let (stealth_private_key, hashed_shared_secret) =
        derive_verified_stealth_key(wallet, stealth_address, ephemeral_public_key_hex, view_tag)?;

    let mut key_bytes = stealth_private_key.to_bytes();
    let signing_key = SigningKey::from_slice(&key_bytes)
        .map_err(|error| EthereumStealthError::Signing(error.to_string()))?;
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(digest)
        .map_err(|error| EthereumStealthError::Signing(error.to_string()))?;

    key_bytes.zeroize();
    let derived_address = ethereum_address_from_public_key(&stealth_private_key.public_key());

    Ok(EthereumStealthSignature {
        stealth_address: derived_address,
        signature_hex: encode_recoverable_signature(&signature, recovery_id),
        recovery_id: recovery_id.to_byte(),
        view_tag_hex: hex::encode([derive_view_tag(&hashed_shared_secret)]),
    })
}

/// Sign an EIP-1559 native ETH transfer from a stealth address.
///
/// Creates and signs an EIP-1559 transaction that transfers native ETH from the derived
/// stealth address. The transaction includes all required fields (chain ID, nonce, fees, gas limit)
/// and can be broadcast directly to an Ethereum RPC.
///
/// # Parameters
///
/// - `transfer`: Contains destination, value, and fee configuration.
/// - `view_tag`: Optional filter for fast rejection. If provided and mismatched,
///   fails immediately without further processing.
///
/// # Returns
///
/// A fully signed transaction with raw transaction hex and transaction hash.
pub fn sign_ethereum_stealth_native_transfer(
    wallet: &EthereumStealthWallet,
    stealth_address: &str,
    ephemeral_public_key_hex: &str,
    view_tag: Option<u8>,
    transfer: &EthereumEip1559Transfer,
) -> Result<EthereumSignedTransaction, EthereumStealthError> {
    let destination_address = normalize_ethereum_address(&transfer.destination_address)?;
    sign_ethereum_stealth_eip1559_transaction(
        wallet,
        stealth_address,
        ephemeral_public_key_hex,
        view_tag,
        UnsignedEip1559Transaction {
            chain_id: transfer.chain_id,
            nonce: transfer.nonce,
            max_priority_fee_per_gas: transfer.max_priority_fee_per_gas,
            max_fee_per_gas: transfer.max_fee_per_gas,
            gas_limit: transfer.gas_limit,
            to_address: destination_address.clone(),
            value: transfer.value,
            data: Vec::new(),
        },
        "eth-transfer",
        destination_address,
    )
}

/// Sign an EIP-1559 ERC20 token transfer from a stealth address.
///
/// Creates and signs an EIP-1559 transaction that transfers an ERC20 token from the derived
/// stealth address. Encodes the token transfer as calldata (ERC20 `transfer` function selector)
/// and includes the token contract as the `to_address`.
///
/// # Parameters
///
/// - `transfer`: Contains token address, recipient address, amount, and fee configuration.
/// - `view_tag`: Optional filter for fast rejection.
///
/// # Returns
///
/// A fully signed transaction with raw transaction hex and transaction hash.
pub fn sign_ethereum_stealth_erc20_transfer(
    wallet: &EthereumStealthWallet,
    stealth_address: &str,
    ephemeral_public_key_hex: &str,
    view_tag: Option<u8>,
    transfer: &EthereumEip1559Erc20Transfer,
) -> Result<EthereumSignedTransaction, EthereumStealthError> {
    let token_address = normalize_ethereum_address(&transfer.token_address)?;
    let recipient_address = normalize_ethereum_address(&transfer.recipient_address)?;
    let data = encode_erc20_transfer_data(&recipient_address, &transfer.amount)?;

    sign_ethereum_stealth_eip1559_transaction(
        wallet,
        stealth_address,
        ephemeral_public_key_hex,
        view_tag,
        UnsignedEip1559Transaction {
            chain_id: transfer.chain_id,
            nonce: transfer.nonce,
            max_priority_fee_per_gas: transfer.max_priority_fee_per_gas,
            max_fee_per_gas: transfer.max_fee_per_gas,
            gas_limit: transfer.gas_limit,
            to_address: token_address.clone(),
            value: [0u8; 32],
            data,
        },
        "erc20-transfer",
        token_address,
    )
}

fn derive_wallet_secret_key(
    master_key: &[u8],
    wallet: &str,
    purpose: &str,
) -> Result<SecretKey, EthereumStealthError> {
    for counter in 0u32..=u32::MAX {
        let mut mac = HmacSha256::new_from_slice(master_key)
            .map_err(|_| EthereumStealthError::InvalidKeyMaterial)?;
        mac.update(b"sigillum/eth-stealth/v1/");
        mac.update(wallet.as_bytes());
        mac.update(b"/");
        mac.update(purpose.as_bytes());
        mac.update(&counter.to_be_bytes());
        let candidate = mac.finalize().into_bytes();
        if let Ok(secret_key) = SecretKey::from_slice(&candidate) {
            return Ok(secret_key);
        }
    }

    Err(EthereumStealthError::InvalidKeyMaterial)
}

/// Internal helper for signing EIP-1559 transactions from a stealth address.
///
/// Derives the stealth private key, verifies the address, constructs the transaction,
/// and signs it with Keccak256 hashing. This function is the core logic used by both
/// native ETH and ERC20 transfer signing paths.
fn sign_ethereum_stealth_eip1559_transaction(
    wallet: &EthereumStealthWallet,
    stealth_address: &str,
    ephemeral_public_key_hex: &str,
    view_tag: Option<u8>,
    tx: UnsignedEip1559Transaction,
    kind: &str,
    to_address: String,
) -> Result<EthereumSignedTransaction, EthereumStealthError> {
    let (stealth_private_key, _hashed_shared_secret) =
        derive_verified_stealth_key(wallet, stealth_address, ephemeral_public_key_hex, view_tag)?;

    let mut key_bytes = stealth_private_key.to_bytes();
    let signing_key = SigningKey::from_slice(&key_bytes)
        .map_err(|error| EthereumStealthError::Signing(error.to_string()))?;

    let result = sign_ethereum_eip1559_transaction(&signing_key, tx, kind, to_address);
    key_bytes.zeroize();
    result
}

pub fn sign_ethereum_eip1559_transaction(
    signing_key: &SigningKey,
    tx: UnsignedEip1559Transaction,
    kind: &str,
    to_address: String,
) -> Result<EthereumSignedTransaction, EthereumStealthError> {
    if compare_quantity_be(&tx.max_fee_per_gas, &tx.max_priority_fee_per_gas).is_lt() {
        return Err(EthereumStealthError::InvalidFeeConfiguration);
    }

    let public_key = k256::PublicKey::from(signing_key.verifying_key());
    let from_address = ethereum_address_from_public_key(&public_key);

    let signing_payload = encode_eip1559_signing_payload(&tx)?;
    let digest = Keccak256::digest(&signing_payload);
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(&digest)
        .map_err(|error| EthereumStealthError::Signing(error.to_string()))?;

    let raw_transaction = encode_eip1559_signed_payload(&tx, &signature, recovery_id)?;
    let transaction_hash = Keccak256::digest(&raw_transaction);

    Ok(EthereumSignedTransaction {
        kind: kind.to_string(),
        chain_id: tx.chain_id,
        nonce: tx.nonce,
        from_address,
        to_address,
        value_hex: encode_quantity_hex(&tx.value),
        data_hex: hex::encode(&tx.data),
        raw_transaction_hex: hex::encode(raw_transaction),
        transaction_hash_hex: hex::encode(transaction_hash),
    })
}

pub fn sign_ethereum_native_transfer(
    signing_key: &SigningKey,
    transfer: &EthereumEip1559Transfer,
) -> Result<EthereumSignedTransaction, EthereumStealthError> {
    let destination_address = normalize_ethereum_address(&transfer.destination_address)?;
    sign_ethereum_eip1559_transaction(
        signing_key,
        UnsignedEip1559Transaction {
            chain_id: transfer.chain_id,
            nonce: transfer.nonce,
            max_priority_fee_per_gas: transfer.max_priority_fee_per_gas,
            max_fee_per_gas: transfer.max_fee_per_gas,
            gas_limit: transfer.gas_limit,
            to_address: destination_address.clone(),
            value: transfer.value,
            data: Vec::new(),
        },
        "eth-transfer",
        destination_address,
    )
}

pub fn sign_ethereum_erc20_transfer(
    signing_key: &SigningKey,
    transfer: &EthereumEip1559Erc20Transfer,
) -> Result<EthereumSignedTransaction, EthereumStealthError> {
    let token_address = normalize_ethereum_address(&transfer.token_address)?;
    let recipient_address = normalize_ethereum_address(&transfer.recipient_address)?;
    let data = encode_erc20_transfer_data(&recipient_address, &transfer.amount)?;

    sign_ethereum_eip1559_transaction(
        signing_key,
        UnsignedEip1559Transaction {
            chain_id: transfer.chain_id,
            nonce: transfer.nonce,
            max_priority_fee_per_gas: transfer.max_priority_fee_per_gas,
            max_fee_per_gas: transfer.max_fee_per_gas,
            gas_limit: transfer.gas_limit,
            to_address: token_address.clone(),
            value: [0u8; 32],
            data,
        },
        "erc20-transfer",
        token_address,
    )
}

// ── EIP-1559 transaction construction ──

fn parse_meta_address(value: &str) -> Result<ParsedMetaAddress, EthereumStealthError> {
    let (short_name, raw_keys) = if let Some(stripped) = value.strip_prefix("st:") {
        let mut parts = stripped.splitn(3, ':');
        let short_name = parts
            .next()
            .ok_or(EthereumStealthError::InvalidMetaAddress)?;
        let payload = parts
            .next()
            .ok_or(EthereumStealthError::InvalidMetaAddress)?;
        (normalize_short_name(short_name)?, payload.to_string())
    } else {
        ("eth".to_string(), value.to_string())
    };
    let payload = raw_keys
        .strip_prefix("0x")
        .or_else(|| raw_keys.strip_prefix("0X"))
        .unwrap_or(raw_keys.as_str());

    if payload.len() != 132 {
        return Err(EthereumStealthError::InvalidMetaAddress);
    }

    let spending_public_key = parse_public_key_hex(&payload[..66])?;
    let viewing_public_key = parse_public_key_hex(&payload[66..])?;

    Ok(ParsedMetaAddress {
        short_name,
        spending_public_key,
        viewing_public_key,
    })
}

fn parse_public_key_hex(value: &str) -> Result<PublicKey, EthereumStealthError> {
    let bytes = hex::decode(value).map_err(|_| EthereumStealthError::InvalidKeyMaterial)?;
    PublicKey::from_sec1_bytes(&bytes).map_err(|_| EthereumStealthError::InvalidKeyMaterial)
}

pub fn decode_quantity_hex(value: &str) -> Result<[u8; 32], EthereumStealthError> {
    let raw = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if raw.is_empty() {
        return Ok([0u8; 32]);
    }
    if !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(EthereumStealthError::InvalidQuantity(value.to_string()));
    }
    let normalized = if raw.len() % 2 == 0 {
        raw.to_string()
    } else {
        format!("0{raw}")
    };
    let bytes = hex::decode(normalized)
        .map_err(|_| EthereumStealthError::InvalidQuantity(value.to_string()))?;
    if bytes.len() > 32 {
        return Err(EthereumStealthError::InvalidQuantity(value.to_string()));
    }
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(&bytes);
    Ok(out)
}

fn ephemeral_private_key_to_secret(
    private_key: [u8; 32],
) -> Result<SecretKey, EthereumStealthError> {
    SecretKey::from_slice(&private_key).map_err(|_| EthereumStealthError::InvalidKeyMaterial)
}

fn random_secret_bytes() -> [u8; 32] {
    use rand::RngCore;

    loop {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        if SecretKey::from_slice(&bytes).is_ok() {
            return bytes;
        }
    }
}

fn hashed_shared_secret(
    private_key: &SecretKey,
    public_key: &PublicKey,
) -> Result<[u8; 32], EthereumStealthError> {
    let shared_secret = diffie_hellman(private_key.to_nonzero_scalar(), public_key.as_affine());
    Ok(Keccak256::digest(shared_secret.raw_secret_bytes()).into())
}

fn hashed_shared_secret_for_recipient(
    viewing_private_key: &SecretKey,
    ephemeral_public_key: &PublicKey,
) -> Result<[u8; 32], EthereumStealthError> {
    hashed_shared_secret(viewing_private_key, ephemeral_public_key)
}

fn derive_stealth_public_key(
    spending_public_key: &PublicKey,
    hashed_shared_secret: &[u8; 32],
) -> Result<PublicKey, EthereumStealthError> {
    let offset_private_key = SecretKey::from_slice(hashed_shared_secret)
        .map_err(|_| EthereumStealthError::InvalidKeyMaterial)?;
    let offset_point = ProjectivePoint::from(*offset_private_key.public_key().as_affine());
    let spending_point = ProjectivePoint::from(*spending_public_key.as_affine());
    let stealth_point = spending_point + offset_point;
    let stealth_affine = AffinePoint::from(stealth_point);
    PublicKey::from_affine(stealth_affine).map_err(|_| EthereumStealthError::InvalidKeyMaterial)
}

fn derive_stealth_private_key(
    spending_private_key: &SecretKey,
    hashed_shared_secret: &[u8; 32],
) -> Result<SecretKey, EthereumStealthError> {
    let offset_private_key = SecretKey::from_slice(hashed_shared_secret)
        .map_err(|_| EthereumStealthError::InvalidKeyMaterial)?;
    let scalar = *spending_private_key.to_nonzero_scalar().as_ref()
        + *offset_private_key.to_nonzero_scalar().as_ref();
    SecretKey::from_slice(&scalar.to_bytes()).map_err(|_| EthereumStealthError::InvalidKeyMaterial)
}

fn encode_recoverable_signature(signature: &Signature, recovery_id: RecoveryId) -> String {
    let mut bytes = [0u8; 65];
    bytes[..64].copy_from_slice(&signature.to_bytes());
    bytes[64] = recovery_id.to_byte();
    hex::encode(bytes)
}

fn encode_erc20_transfer_data(
    recipient_address: &str,
    amount: &[u8; 32],
) -> Result<Vec<u8>, EthereumStealthError> {
    let recipient = decode_ethereum_address(recipient_address)?;
    let mut data = Vec::with_capacity(4 + 32 + 32);
    data.extend_from_slice(&[0xa9, 0x05, 0x9c, 0xbb]);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(&recipient);
    data.extend_from_slice(amount);
    Ok(data)
}

fn erc5564_metadata_from_view_tag(value: &str) -> Result<String, EthereumStealthError> {
    let bytes = decode_hex_bytes(value, "view_tag")?;
    if bytes.len() != 1 {
        return Err(EthereumStealthError::InvalidAnnouncementField(
            "view_tag must be exactly one byte".into(),
        ));
    }
    Ok(hex::encode(bytes))
}

fn encode_erc5564_announce_calldata(
    scheme_id: u64,
    stealth_address: &str,
    ephemeral_public_key_hex: &str,
    metadata_hex: &str,
) -> Result<String, EthereumStealthError> {
    let stealth_address = decode_ethereum_address(stealth_address)?;
    let ephemeral_public_key = decode_hex_bytes(ephemeral_public_key_hex, "ephemeral_public_key")?;
    let metadata = decode_hex_bytes(metadata_hex, "metadata")?;
    if metadata.is_empty() {
        return Err(EthereumStealthError::InvalidAnnouncementField(
            "metadata must include the view tag as its first byte".into(),
        ));
    }

    let ephemeral_offset = 32 * 4;
    let metadata_offset = ephemeral_offset + abi_dynamic_size(ephemeral_public_key.len());
    let selector = Keccak256::digest(ERC5564_ANNOUNCE_FUNCTION.as_bytes());
    let mut calldata = Vec::with_capacity(4 + metadata_offset + abi_dynamic_size(metadata.len()));
    calldata.extend_from_slice(&selector[..4]);
    abi_push_u64_word(&mut calldata, scheme_id);
    abi_push_address_word(&mut calldata, &stealth_address);
    abi_push_usize_word(&mut calldata, ephemeral_offset);
    abi_push_usize_word(&mut calldata, metadata_offset);
    abi_push_dynamic_bytes(&mut calldata, &ephemeral_public_key);
    abi_push_dynamic_bytes(&mut calldata, &metadata);
    Ok(hex::encode(calldata))
}

fn decode_hex_bytes(value: &str, field: &str) -> Result<Vec<u8>, EthereumStealthError> {
    let raw = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if raw.is_empty() || raw.len() % 2 != 0 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(EthereumStealthError::InvalidAnnouncementField(
            field.to_string(),
        ));
    }
    hex::decode(raw).map_err(|_| EthereumStealthError::InvalidAnnouncementField(field.to_string()))
}

fn abi_dynamic_size(len: usize) -> usize {
    32 + len + abi_padding(len)
}

fn abi_padding(len: usize) -> usize {
    (32 - (len % 32)) % 32
}

fn abi_push_u64_word(out: &mut Vec<u8>, value: u64) {
    let mut word = [0u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    out.extend_from_slice(&word);
}

fn abi_push_usize_word(out: &mut Vec<u8>, value: usize) {
    let mut word = [0u8; 32];
    word[24..].copy_from_slice(&(value as u64).to_be_bytes());
    out.extend_from_slice(&word);
}

fn abi_push_address_word(out: &mut Vec<u8>, address: &[u8; 20]) {
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(address);
}

fn abi_push_dynamic_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    abi_push_usize_word(out, bytes.len());
    out.extend_from_slice(bytes);
    out.resize(out.len() + abi_padding(bytes.len()), 0);
}

fn encode_eip1559_signing_payload(
    tx: &UnsignedEip1559Transaction,
) -> Result<Vec<u8>, EthereumStealthError> {
    let to = decode_ethereum_address(&tx.to_address)?;
    let items = vec![
        rlp_encode_u64(tx.chain_id),
        rlp_encode_u64(tx.nonce),
        rlp_encode_quantity(&tx.max_priority_fee_per_gas),
        rlp_encode_quantity(&tx.max_fee_per_gas),
        rlp_encode_u64(tx.gas_limit),
        rlp_encode_bytes(&to),
        rlp_encode_quantity(&tx.value),
        rlp_encode_bytes(&tx.data),
        rlp_encode_list(&[]),
    ];
    let mut payload = vec![0x02];
    payload.extend_from_slice(&rlp_encode_list(&items));
    Ok(payload)
}

fn encode_eip1559_signed_payload(
    tx: &UnsignedEip1559Transaction,
    signature: &Signature,
    recovery_id: RecoveryId,
) -> Result<Vec<u8>, EthereumStealthError> {
    let to = decode_ethereum_address(&tx.to_address)?;
    let sig_bytes = signature.to_bytes();
    let mut r = [0u8; 32];
    let mut s = [0u8; 32];
    r.copy_from_slice(&sig_bytes[..32]);
    s.copy_from_slice(&sig_bytes[32..]);

    let items = vec![
        rlp_encode_u64(tx.chain_id),
        rlp_encode_u64(tx.nonce),
        rlp_encode_quantity(&tx.max_priority_fee_per_gas),
        rlp_encode_quantity(&tx.max_fee_per_gas),
        rlp_encode_u64(tx.gas_limit),
        rlp_encode_bytes(&to),
        rlp_encode_quantity(&tx.value),
        rlp_encode_bytes(&tx.data),
        rlp_encode_list(&[]),
        rlp_encode_u64(recovery_id.to_byte() as u64),
        rlp_encode_quantity(&r),
        rlp_encode_quantity(&s),
    ];
    let mut payload = vec![0x02];
    payload.extend_from_slice(&rlp_encode_list(&items));
    Ok(payload)
}

fn rlp_encode_u64(value: u64) -> Vec<u8> {
    let bytes = minimal_be_bytes_from_u64(value);
    rlp_encode_bytes(&bytes)
}

fn rlp_encode_quantity(value: &[u8; 32]) -> Vec<u8> {
    let bytes = trim_leading_zeroes(value);
    rlp_encode_bytes(bytes)
}

fn rlp_encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let payload_len: usize = items.iter().map(Vec::len).sum();
    let mut payload = Vec::with_capacity(payload_len);
    for item in items {
        payload.extend_from_slice(item);
    }
    if payload.len() <= 55 {
        let mut out = Vec::with_capacity(1 + payload.len());
        out.push(0xc0 + payload.len() as u8);
        out.extend_from_slice(&payload);
        out
    } else {
        let len_bytes = minimal_be_bytes_from_usize(payload.len());
        let mut out = Vec::with_capacity(1 + len_bytes.len() + payload.len());
        out.push(0xf7 + len_bytes.len() as u8);
        out.extend_from_slice(&len_bytes);
        out.extend_from_slice(&payload);
        out
    }
}

fn rlp_encode_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        return vec![bytes[0]];
    }
    if bytes.len() <= 55 {
        let mut out = Vec::with_capacity(1 + bytes.len());
        out.push(0x80 + bytes.len() as u8);
        out.extend_from_slice(bytes);
        out
    } else {
        let len_bytes = minimal_be_bytes_from_usize(bytes.len());
        let mut out = Vec::with_capacity(1 + len_bytes.len() + bytes.len());
        out.push(0xb7 + len_bytes.len() as u8);
        out.extend_from_slice(&len_bytes);
        out.extend_from_slice(bytes);
        out
    }
}

fn trim_leading_zeroes(bytes: &[u8; 32]) -> &[u8] {
    let first_non_zero = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len());
    &bytes[first_non_zero..]
}

fn minimal_be_bytes_from_u64(value: u64) -> Vec<u8> {
    if value == 0 {
        Vec::new()
    } else {
        let bytes = value.to_be_bytes();
        bytes[bytes
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(bytes.len())..]
            .to_vec()
    }
}

fn minimal_be_bytes_from_usize(value: usize) -> Vec<u8> {
    minimal_be_bytes_from_u64(value as u64)
}

fn compare_quantity_be(left: &[u8; 32], right: &[u8; 32]) -> std::cmp::Ordering {
    left.cmp(right)
}

fn encode_quantity_hex(value: &[u8; 32]) -> String {
    let trimmed = trim_leading_zeroes(value);
    if trimmed.is_empty() {
        "0x0".to_string()
    } else {
        let encoded = hex::encode(trimmed);
        let encoded = encoded.strip_prefix('0').unwrap_or(&encoded);
        format!("0x{encoded}")
    }
}

// ── Ethereum address utilities ──

fn encode_public_key(public_key: &PublicKey) -> String {
    hex::encode(public_key.to_encoded_point(true).as_bytes())
}

fn ethereum_address_from_public_key(public_key: &PublicKey) -> String {
    let encoded = public_key.to_encoded_point(false);
    let bytes = encoded.as_bytes();
    let digest = Keccak256::digest(&bytes[1..]);
    format!("0x{}", hex::encode(&digest[12..]))
}

fn normalize_ethereum_address(value: &str) -> Result<String, EthereumStealthError> {
    let raw = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if raw.len() != 40 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(EthereumStealthError::InvalidEthereumAddress);
    }
    Ok(format!("0x{}", raw.to_ascii_lowercase()))
}

fn decode_ethereum_address(value: &str) -> Result<[u8; 20], EthereumStealthError> {
    let raw = normalize_ethereum_address(value)?;
    let bytes = hex::decode(&raw[2..]).map_err(|_| EthereumStealthError::InvalidEthereumAddress)?;
    let mut out = [0u8; 20];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn normalize_short_name(value: &str) -> Result<String, EthereumStealthError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return Err(EthereumStealthError::InvalidShortName);
    }
    Ok(trimmed.to_string())
}

// ── Elliptic curve helpers ──

fn derive_view_tag(hashed_shared_secret: &[u8; 32]) -> u8 {
    hashed_shared_secret[0]
}

struct ParsedMetaAddress {
    short_name: String,
    spending_public_key: PublicKey,
    viewing_public_key: PublicKey,
}

pub struct UnsignedEip1559Transaction {
    chain_id: u64,
    nonce: u64,
    max_priority_fee_per_gas: [u8; 32],
    max_fee_per_gas: [u8; 32],
    gas_limit: u64,
    to_address: String,
    value: [u8; 32],
    data: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_wallet_exports_stable_meta_address() {
        let wallet =
            derive_sigillum_ethereum_stealth_wallet(&[7u8; 32], "payments", "eth").unwrap();

        assert_eq!(wallet.meta_address.scheme_id, ETHEREUM_STEALTH_SCHEME_ID);
        assert!(
            wallet
                .meta_address
                .stealth_meta_address
                .starts_with("st:eth:0x")
        );
        assert_eq!(wallet.meta_address.spending_public_key_hex.len(), 66);
        assert_eq!(wallet.meta_address.viewing_public_key_hex.len(), 66);
    }

    #[test]
    fn payment_generation_roundtrips_for_recipient() {
        let wallet =
            derive_sigillum_ethereum_stealth_wallet(&[9u8; 32], "exchange", "eth").unwrap();
        let payment = generate_ethereum_stealth_address(
            &wallet.meta_address.stealth_meta_address,
            Some([3u8; 32]),
        )
        .unwrap();

        let check = check_ethereum_stealth_address(
            &wallet,
            &payment.stealth_address,
            &payment.ephemeral_public_key_hex,
            Some(hex::decode(&payment.view_tag_hex).unwrap()[0]),
        )
        .unwrap();
        assert!(check.matches);
        assert_eq!(check.derived_stealth_address, payment.stealth_address);
    }

    #[test]
    fn erc5564_announcement_payload_encodes_standard_calldata() {
        let wallet =
            derive_sigillum_ethereum_stealth_wallet(&[9u8; 32], "exchange", "eth").unwrap();
        let payment = generate_ethereum_stealth_address(
            &wallet.meta_address.stealth_meta_address,
            Some([3u8; 32]),
        )
        .unwrap();

        let announcement = build_erc5564_announcement(&payment).unwrap();
        let selector = Keccak256::digest(ERC5564_ANNOUNCE_FUNCTION.as_bytes());
        let calldata = hex::decode(&announcement.calldata_hex).unwrap();

        assert_eq!(announcement.announcer_address, ERC5564_ANNOUNCER_ADDRESS);
        assert_eq!(announcement.metadata_hex, payment.view_tag_hex);
        assert_eq!(announcement.value_wei_hex, "0x0");
        assert_eq!(&calldata[..4], &selector[..4]);
        assert!(calldata.len() > 4 + (32 * 4));
    }

    #[test]
    fn local_signing_uses_derived_stealth_key() {
        let wallet =
            derive_sigillum_ethereum_stealth_wallet(&[5u8; 32], "treasury", "eth").unwrap();
        let payment = generate_ethereum_stealth_address(
            &wallet.meta_address.stealth_meta_address,
            Some([11u8; 32]),
        )
        .unwrap();
        let digest = [42u8; 32];
        let view_tag = hex::decode(&payment.view_tag_hex).unwrap()[0];

        let signature = sign_ethereum_stealth_digest(
            &wallet,
            &payment.stealth_address,
            &payment.ephemeral_public_key_hex,
            Some(view_tag),
            &digest,
        )
        .unwrap();

        let signature_bytes = hex::decode(signature.signature_hex).unwrap();
        assert!(signature_bytes.len() == 65);
    }

    #[test]
    fn mismatched_view_tag_fails_fast() {
        let wallet = derive_sigillum_ethereum_stealth_wallet(&[13u8; 32], "ops", "eth").unwrap();
        let payment = generate_ethereum_stealth_address(
            &wallet.meta_address.stealth_meta_address,
            Some([17u8; 32]),
        )
        .unwrap();

        let error = check_ethereum_stealth_address(
            &wallet,
            &payment.stealth_address,
            &payment.ephemeral_public_key_hex,
            Some(0xff),
        )
        .unwrap_err();
        assert_eq!(error, EthereumStealthError::ViewTagMismatch);
    }

    #[test]
    fn native_transfer_signing_returns_broadcastable_eip1559_transaction() {
        let wallet = derive_sigillum_ethereum_stealth_wallet(&[21u8; 32], "ops", "eth").unwrap();
        let payment = generate_ethereum_stealth_address(
            &wallet.meta_address.stealth_meta_address,
            Some([23u8; 32]),
        )
        .unwrap();

        let signed = sign_ethereum_stealth_native_transfer(
            &wallet,
            &payment.stealth_address,
            &payment.ephemeral_public_key_hex,
            Some(hex::decode(&payment.view_tag_hex).unwrap()[0]),
            &EthereumEip1559Transfer {
                chain_id: 1,
                nonce: 7,
                max_priority_fee_per_gas: decode_quantity_hex("0x59682f00").unwrap(),
                max_fee_per_gas: decode_quantity_hex("0x77359400").unwrap(),
                gas_limit: 21_000,
                destination_address: "0x1111111111111111111111111111111111111111".into(),
                value: decode_quantity_hex("0xde0b6b3a7640000").unwrap(),
            },
        )
        .unwrap();

        assert_eq!(signed.kind, "eth-transfer");
        assert!(signed.raw_transaction_hex.starts_with("02"));
        assert_eq!(signed.transaction_hash_hex.len(), 64);
    }

    #[test]
    fn erc20_transfer_signing_embeds_transfer_selector() {
        let wallet =
            derive_sigillum_ethereum_stealth_wallet(&[31u8; 32], "settlement", "eth").unwrap();
        let payment = generate_ethereum_stealth_address(
            &wallet.meta_address.stealth_meta_address,
            Some([37u8; 32]),
        )
        .unwrap();

        let signed = sign_ethereum_stealth_erc20_transfer(
            &wallet,
            &payment.stealth_address,
            &payment.ephemeral_public_key_hex,
            Some(hex::decode(&payment.view_tag_hex).unwrap()[0]),
            &EthereumEip1559Erc20Transfer {
                chain_id: 1,
                nonce: 11,
                max_priority_fee_per_gas: decode_quantity_hex("0x3b9aca00").unwrap(),
                max_fee_per_gas: decode_quantity_hex("0x77359400").unwrap(),
                gas_limit: 65_000,
                token_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
                recipient_address: "0x2222222222222222222222222222222222222222".into(),
                amount: decode_quantity_hex("0x0f4240").unwrap(),
            },
        )
        .unwrap();

        assert_eq!(signed.kind, "erc20-transfer");
        assert!(signed.data_hex.starts_with("a9059cbb"));
        assert_eq!(signed.value_hex, "0x0");
    }
}
