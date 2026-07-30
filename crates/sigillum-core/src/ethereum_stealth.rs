//! ERC-5564 Ethereum Stealth Address Implementation
//!
//! This module implements the ERC-5564 standard for stealth addresses, enabling privacy-preserving
//! Ethereum transactions where recipient addresses are not publicly linked to recipients.
//!
//! ## Cryptographic Building Blocks
//!
//! - **ECDH Shared Secrets**: Uses secp256k1 elliptic curve Diffie-Hellman to derive ephemeral
//!   shared points between payers and recipients. New scheme-1 payments hash the compressed
//!   SEC1 point used by ScopeLift's `stealth-address-sdk`; recipient paths also recognize
//!   Sigillum's legacy x-coordinate-only hash so existing payments remain recoverable.
//! - **Keccak256 Hashing**: Hashes shared secrets for key derivation and address computation.
//! - **View Tags**: Compact filtering mechanism (single byte) allowing recipients to quickly identify
//!   stealth payments without full cryptographic verification on every block.
//! - **Stealth Address Derivation**: Combines recipient's spending public key with ECDH-derived
//!   secrets to compute a unique stealth address for each payment.
//! - **EIP-1559 Transaction Signing**: Generates signed transactions compatible with Ethereum's
//!   dynamic fee market.
//!
//! ## Shared-secret hash conventions
//!
//! ERC-5564 scheme 1 leaves the exact byte encoding of the hashed ECDH shared
//! secret implementation-defined, and two encodings exist in the wild:
//!
//! - [`StealthHashConvention::Compressed33`] — **standard, used for all new
//!   payments**: `keccak256` over the 33-byte compressed SEC1 encoding of the
//!   shared-secret point. This is the de-facto scheme-1 convention implemented
//!   by the ScopeLift `stealth-address-sdk` (the reference implementation used
//!   by Umbra-style tooling), confirmed byte-exactly against
//!   `src/utils/crypto/generateStealthAddress.ts` (`getHashedSharedSecret` =
//!   `keccak256(getSharedSecret(...))` where `@noble/secp256k1` v2
//!   `getSharedSecret` returns the compressed point by default) and
//!   `src/utils/crypto/computeStealthKey.ts` (`(spendingPrivateKey +
//!   BigInt(hashedSharedSecret)) % CURVE.n`), retrieved 2026-07-17 from
//!   <https://github.com/ScopeLift/stealth-address-sdk/tree/main/src/utils/crypto>.
//! - [`StealthHashConvention::XOnly32`] — **legacy Sigillum convention**:
//!   `keccak256` over the 32-byte x-coordinate of the shared-secret point
//!   (k256 `SharedSecret::raw_secret_bytes`). Payments created before the
//!   switch remain detectable and spendable through dual-decode probing; see
//!   [`check_ethereum_stealth_address_any`].
//!
//! A third, incompatible variant exists in the wild (Fluidkey hashes the
//! 64-byte uncompressed X‖Y encoding); it is intentionally NOT supported.
//!
//! ## Watch-only detection
//!
//! Recipient-side detection follows the EIP-5564 `checkStealthAddress` key
//! signature: it needs only the stealth address, the ephemeral public key, the
//! viewing private key, and the spending PUBLIC key — never the spending
//! private key. [`EthereumStealthWatchView`] (derived by
//! [`derive_watch_only_sigillum_ethereum_stealth_wallet`]) carries exactly
//! that reduced material, and [`check_ethereum_stealth_address_watch_only`] /
//! [`check_ethereum_stealth_address_any_watch_only`] are the detection core;
//! the full-wallet [`check_ethereum_stealth_address`] /
//! [`check_ethereum_stealth_address_any`] entry points reduce the wallet to
//! its watch view and delegate, so both paths share one implementation and
//! produce byte-identical results. Only sweep signing
//! ([`derive_verified_stealth_key`]) uses the spending private key.
//!
//! Detection still requires the compartment unlocked (the viewing key derives
//! from the master key); the watch-only boundary means spending secret
//! material never enters the scan path, not that scanning works on a locked
//! vault. Watch views are re-derived per operation and never cached, so the
//! zeroize-on-lock invariant is preserved.
//!
//! ## Meta-address key forms
//!
//! Both EIP-5564 meta-address forms parse: the dual-key form
//! (`st:<chain>:0x<spending‖viewing>`, two 33-byte compressed SEC1 keys) and
//! the single-key form (`st:<chain>:0x<key>`, one 33-byte compressed key used
//! as BOTH spending and viewing key). Generation needs no special-casing —
//! the shared secret runs against the viewing key and the address offset
//! against the spending key, which for the single-key form are simply the
//! same point — and a recipient wallet or watch view whose spending and
//! viewing keys are equal checks and sweeps such payments through the
//! unchanged full/watch-only paths. Sigillum-derived wallets always use the
//! dual-key form; the single-key form matters for interoperability with
//! external meta-addresses. Fluidkey's 64-byte X‖Y encoding remains
//! unsupported.
//!
//! ## Announcement metadata SHOULD layouts
//!
//! Beyond the mandatory view-tag first byte, EIP-5564 recommends two 57-byte
//! metadata layouts so recipients learn the asset and amount from the
//! announcement itself: the native-token layout (`0xeeeeeeee` marker +
//! sentinel address + amount) and the token layout (function identifier +
//! token contract + amount). [`encode_erc5564_metadata_native`] and
//! [`encode_erc5564_metadata_erc20_transfer`] produce them (used when a
//! deposit requests payer-attached gas/asset info), and
//! [`decode_erc5564_metadata_hints`] parses them defensively — unknown or
//! malformed trailing bytes always decode to "no hints", never to an error.
//!
//! ## Gas sponsor
//!
//! [`EthereumStealthGasSponsor`] (derived by
//! [`derive_sigillum_ethereum_stealth_gas_sponsor`]) is the canonical gas
//! sponsor source for stealth deposits: a deterministic key on its own HMAC
//! chain from the compartment master key, funded out-of-band by the operator,
//! paying sponsor top-ups to gas-starved stealth deposit addresses.
//!
//! ## Security Properties
//!
//! - Every signing operation derives and verifies the stealth private key against the expected
//!   stealth address before use, preventing key-address mismatches. Convention
//!   probing therefore cannot produce a key for the wrong address: a candidate
//!   key is only usable when its derived address matches the announced one.
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

// ── Shared-secret hash conventions ──

/// Byte-encoding convention for the hashed ECDH shared secret that drives
/// ERC-5564 scheme-1 stealth derivation (view tag, stealth public key/address,
/// and stealth private key all derive from this one hash).
///
/// The serde representation is the stable lowercase string stored on deposit
/// records and queue jobs: `"compressed33"` (standard) and `"x32"` (legacy).
///
/// See the module-level "Shared-secret hash conventions" section for the
/// normative references and the unsupported Fluidkey 64-byte variant.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum StealthHashConvention {
    /// Legacy Sigillum convention: `keccak256` over the 32-byte x-coordinate
    /// of the shared-secret point (k256 `SharedSecret::raw_secret_bytes`).
    /// Retained solely for payments created before the convention switch.
    #[serde(rename = "x32")]
    XOnly32,
    /// De-facto scheme-1 standard: `keccak256` over the 33-byte compressed
    /// SEC1 encoding of the shared-secret point. Byte-compatible with the
    /// ScopeLift `stealth-address-sdk` (`keccak256(getSharedSecret(...))`,
    /// `@noble/secp256k1` v2 returns the compressed point by default).
    #[default]
    #[serde(rename = "compressed33")]
    Compressed33,
}

impl StealthHashConvention {
    /// Convention used for all newly generated payments and deposit records.
    pub const STANDARD: Self = Self::Compressed33;
    /// Convention of every payment created before the switch; pre-existing
    /// deposit records are stamped with it by the store migration.
    pub const LEGACY: Self = Self::XOnly32;
    /// Dual-decode probe order: standard first (the overwhelmingly common
    /// case for new announcements), legacy second.
    pub const PROBE_ORDER: [Self; 2] = [Self::Compressed33, Self::XOnly32];

    /// Stable string form used in records, logs, and API payloads.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::XOnly32 => "x32",
            Self::Compressed33 => "compressed33",
        }
    }

    /// The other convention, used when probing both after a mismatch.
    pub fn other(self) -> Self {
        match self {
            Self::XOnly32 => Self::Compressed33,
            Self::Compressed33 => Self::XOnly32,
        }
    }
}

impl std::fmt::Display for StealthHashConvention {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::str::FromStr for StealthHashConvention {
    type Err = EthereumStealthError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "x32" => Ok(Self::XOnly32),
            "compressed33" => Ok(Self::Compressed33),
            _ => Err(EthereumStealthError::InvalidStealthHashConvention(
                value.to_string(),
            )),
        }
    }
}

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
    #[error("unknown stealth hash convention: {0}")]
    InvalidStealthHashConvention(String),
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
    /// Convention the shared-secret hash was derived with (standard for all
    /// newly generated payments).
    pub stealth_hash_convention: StealthHashConvention,
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
    /// Convention that produced `derived_stealth_address`/`view_tag_hex`. When
    /// `matches` is true this is the convention the payment was actually made
    /// with; callers persist it so later key derivation uses the right one.
    pub stealth_hash_convention: StealthHashConvention,
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

    /// Reduced-privilege view of this wallet for watch-only detection.
    ///
    /// Carries everything EIP-5564 `checkStealthAddress` needs — the viewing
    /// private key and the spending PUBLIC key — and drops the spending
    /// private key from the detection path entirely.
    pub fn watch_view(&self) -> EthereumStealthWatchView {
        EthereumStealthWatchView {
            meta_address: self.meta_address.clone(),
            viewing_private_key: self.viewing_private_key.clone(),
            spending_public_key: self.spending_private_key.public_key(),
        }
    }
}

/// Watch-only view of a Sigillum Ethereum stealth wallet: the viewing private
/// key plus the spending PUBLIC key.
///
/// This is exactly the recipient-side key material of EIP-5564
/// `checkStealthAddress(stealthAddress, ephemeralPubkey, viewingKey,
/// spendingPubkey)`: sufficient to detect payments (view tag → shared-secret
/// hash → stealth address recompute + compare) but NOT to spend them, because
/// the spending private key is absent by construction — the type has no field
/// that could hold it. Detection paths (announcement scans, deposit checks)
/// should operate on this view so spending secret material never enters the
/// scan path; only sweep signing needs the full [`EthereumStealthWallet`].
///
/// # Security
///
/// The viewing private key is still secret (it lets observers recognize your
/// payments), so this view must live in memory only while the compartment is
/// unlocked, exactly like the master key it derives from. It is deliberately
/// re-derived per operation rather than cached: caching it across a lock
/// would weaken the zeroize-on-lock invariant.
#[derive(Clone, Debug)]
pub struct EthereumStealthWatchView {
    meta_address: EthereumStealthMetaAddress,
    viewing_private_key: SecretKey,
    spending_public_key: PublicKey,
}

impl EthereumStealthWatchView {
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

/// Derive the watch-only view of a Sigillum Ethereum stealth wallet from a
/// master key.
///
/// Produces the recipient-side detection material of EIP-5564 — the viewing
/// private key and the spending PUBLIC key (plus the meta-address) — WITHOUT
/// retaining the spending private key. The derivation is two independent
/// HMAC chains (`…/view/…` and `…/spend/…`); the spend chain is touched only
/// to compute the public half, and the resulting secret is dropped (and thus
/// zeroized, k256 `SecretKey: ZeroizeOnDrop`) inside a dedicated scope before
/// this function returns. Callers in the detection path therefore never hold
/// spending secret material at all.
///
/// Detection still requires the compartment unlocked: the viewing key derives
/// from the master key just like the spending key does. The win over
/// [`derive_sigillum_ethereum_stealth_wallet`] is that the spending secret
/// never enters the scan path — not that scanning works on a locked vault.
/// The view is deliberately re-derived per operation and never cached, so
/// locking the compartment keeps zeroizing every path to key material.
pub fn derive_watch_only_sigillum_ethereum_stealth_wallet(
    master_key: &[u8],
    wallet: &str,
    short_name: &str,
) -> Result<EthereumStealthWatchView, EthereumStealthError> {
    if wallet.trim().is_empty() {
        return Err(EthereumStealthError::EmptyWalletLabel);
    }
    let short_name = normalize_short_name(short_name)?;
    let viewing_private_key = derive_wallet_secret_key(master_key, wallet, "view")?;
    let spending_public_key = {
        let spending_private_key = derive_wallet_secret_key(master_key, wallet, "spend")?;
        spending_private_key.public_key()
        // `spending_private_key` is dropped — and zeroized on drop — here,
        // before the watch view is constructed.
    };
    let spending_public_key_hex = encode_public_key(&spending_public_key);
    let viewing_public_key_hex = encode_public_key(&viewing_private_key.public_key());
    let stealth_meta_address =
        format!("st:{short_name}:0x{spending_public_key_hex}{viewing_public_key_hex}");

    Ok(EthereumStealthWatchView {
        meta_address: EthereumStealthMetaAddress {
            wallet: wallet.to_string(),
            short_name,
            scheme_id: ETHEREUM_STEALTH_SCHEME_ID,
            stealth_meta_address,
            spending_public_key_hex,
            viewing_public_key_hex,
        },
        viewing_private_key,
        spending_public_key,
    })
}

// ── Gas sponsor derivation ──

/// Gas-sponsor key of a Sigillum Ethereum stealth wallet: a deterministic
/// secp256k1 key on its own HMAC chain
/// (`sigillum/eth-stealth/v1/{wallet}/sponsor`), independent of the `spend`
/// and `view` chains, whose address serves as the operator-funded gas sponsor
/// for that wallet's stealth deposits.
///
/// This is the canonical sponsor source for stealth-deposit gas top-ups:
/// stealth wallets have no seed phrase and no control branch, so the sponsor
/// key derives from the compartment master key exactly like the spend/view
/// keys — recoverable from the vault alone, with no extra persisted secret.
/// The operator funds `sponsor_address` out-of-band; the daemon then pays
/// 1.5x-estimated-gas top-ups from it to gas-starved stealth deposit
/// addresses. Because one sponsor funds many deposits, every top-up flows
/// through cross-party linkage accounting (a shared sponsor funding deposits
/// of DIFFERENT counterparties links them on-chain).
///
/// # Security
///
/// The sponsor key can move every wei held by the sponsor address, so it is
/// secret material like the spending key: derive it only inside an unlocked
/// compartment scope and let it zeroize on drop (k256 `SecretKey:
/// ZeroizeOnDrop`). It never enters scan/detection paths.
#[derive(Clone, Debug)]
pub struct EthereumStealthGasSponsor {
    wallet: String,
    sponsor_address: String,
    secret_key: SecretKey,
}

impl EthereumStealthGasSponsor {
    pub fn wallet(&self) -> &str {
        &self.wallet
    }

    /// Checksummed-lowercase hex address the operator funds out-of-band.
    pub fn sponsor_address(&self) -> &str {
        &self.sponsor_address
    }

    /// ECDSA signing key for broadcasting sponsor top-up transfers.
    pub fn signing_key(&self) -> SigningKey {
        SigningKey::from(&self.secret_key)
    }
}

/// Derive the gas sponsor of a Sigillum Ethereum stealth wallet from a master
/// key. See [`EthereumStealthGasSponsor`] for the sponsorship model.
pub fn derive_sigillum_ethereum_stealth_gas_sponsor(
    master_key: &[u8],
    wallet: &str,
) -> Result<EthereumStealthGasSponsor, EthereumStealthError> {
    if wallet.trim().is_empty() {
        return Err(EthereumStealthError::EmptyWalletLabel);
    }
    let secret_key = derive_wallet_secret_key(master_key, wallet, "sponsor")?;
    let sponsor_address = ethereum_address_from_public_key(&secret_key.public_key());
    Ok(EthereumStealthGasSponsor {
        wallet: wallet.to_string(),
        sponsor_address,
        secret_key,
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
/// - `convention`: Shared-secret hash convention. New payments MUST use
///   [`StealthHashConvention::STANDARD`] (the ScopeLift-compatible compressed-point
///   convention); the legacy variant exists only for dual-decode of old records.
///
/// # Returns
///
/// The `EthereumStealthPayment` includes the stealth address, ephemeral public key
/// (needed for recipient recovery), and a view tag for fast recipient filtering.
pub fn generate_ethereum_stealth_address(
    stealth_meta_address: &str,
    ephemeral_private_key: Option<[u8; 32]>,
    convention: StealthHashConvention,
) -> Result<EthereumStealthPayment, EthereumStealthError> {
    let meta = parse_meta_address(stealth_meta_address)?;
    let ephemeral_private_key =
        ephemeral_private_key_to_secret(ephemeral_private_key.unwrap_or_else(random_secret_bytes))?;
    let ephemeral_public_key = ephemeral_private_key.public_key();
    let hashed_shared_secret =
        hashed_shared_secret(&ephemeral_private_key, &meta.viewing_public_key, convention)?;
    let stealth_public_key =
        derive_stealth_public_key(&meta.spending_public_key, &hashed_shared_secret)?;

    let payment = EthereumStealthPayment {
        scheme_id: ETHEREUM_STEALTH_SCHEME_ID,
        short_name: meta.short_name,
        stealth_meta_address: stealth_meta_address.to_string(),
        stealth_address: ethereum_address_from_public_key(&stealth_public_key),
        ephemeral_public_key_hex: encode_public_key(&ephemeral_public_key),
        view_tag_hex: hex::encode([derive_view_tag(&hashed_shared_secret)]),
        stealth_hash_convention: convention,
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
    build_erc5564_announcement_with_metadata(payment, &metadata_hex)
}

/// Build the announcer contract call with caller-supplied metadata bytes.
///
/// The metadata MUST carry the payment's view tag in its first byte (checked
/// against `payment.view_tag_hex`); use [`encode_erc5564_metadata_native`] or
/// [`encode_erc5564_metadata_erc20_transfer`] to construct metadata following
/// the EIP-5564 SHOULD layouts.
pub fn build_erc5564_announcement_with_metadata(
    payment: &EthereumStealthPayment,
    metadata_hex: &str,
) -> Result<EthereumStealthAnnouncement, EthereumStealthError> {
    let metadata = decode_hex_bytes(metadata_hex, "metadata")?;
    let view_tag = decode_hex_bytes(&payment.view_tag_hex, "view_tag")?;
    if metadata.first() != view_tag.first() {
        return Err(EthereumStealthError::InvalidAnnouncementField(
            "metadata must include the payment's view tag as its first byte".into(),
        ));
    }
    let metadata_hex = hex::encode(metadata.as_slice());
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

// ── Announcement metadata SHOULD layouts ──

/// Byte length of the EIP-5564 SHOULD metadata layouts: 1 (view tag) + 4
/// (native marker / function identifier) + 20 (address) + 32 (uint256 amount).
pub const ERC5564_METADATA_LAYOUT_LEN: usize = 57;

/// Native-asset marker in metadata bytes 2-5 (`0xeeeeeeee`), per the EIP-5564
/// native-token metadata SHOULD layout.
pub const ERC5564_METADATA_NATIVE_MARKER: [u8; 4] = [0xee, 0xee, 0xee, 0xee];

/// Native-asset sentinel address in metadata bytes 6-25 of the EIP-5564
/// native-token metadata SHOULD layout.
pub const ERC5564_METADATA_NATIVE_SENTINEL_ADDRESS: &str =
    "0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE";

/// Canonical ERC-20 `transfer(address,uint256)` function selector, used as the
/// function identifier (metadata bytes 2-5) when Sigillum announces an ERC-20
/// deposit. The EIP-5564 token metadata SHOULD layout requires the function
/// selector whenever one is available.
pub const ERC5564_METADATA_ERC20_TRANSFER_SELECTOR: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];

/// Asset/amount hints decoded from an EIP-5564 announcement's metadata SHOULD
/// layout.
///
/// The layouts are defined in the EIP-5564 `announce` specification
/// (<https://eips.ethereum.org/EIPS/eip-5564>, byte offsets here are 0-indexed;
/// the EIP text numbers bytes from 1):
///
/// - Byte 0 MUST be the view tag.
/// - Native token (cf. ETH): bytes 1-5 are `0xeeeeeeee`, bytes 5-25 are the
///   address `0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE`, bytes 25-57 are the
///   amount of ETH being sent.
/// - ERC-20/ERC-721/etc. tokens: bytes 1-5 are a function identifier (the
///   function selector when one is available), bytes 5-25 are the token
///   contract address, bytes 25-57 are the token amount (fungible) or token
///   id (non-fungible).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Erc5564MetadataHints {
    /// Native-token layout: the amount of native token (wei) being sent.
    Native { amount_wei: [u8; 32] },
    /// Token layout: the function identifier, token contract address, and
    /// amount (fungible) or token id (non-fungible) being sent.
    Token {
        function_selector: [u8; 4],
        token_address: String,
        amount: [u8; 32],
    },
}

/// Encode the EIP-5564 native-token metadata SHOULD layout:
/// `view_tag ‖ 0xeeeeeeee ‖ 0xEeeeeE…EEeE ‖ amount_wei` (57 bytes).
pub fn encode_erc5564_metadata_native(view_tag: u8, amount_wei: &[u8; 32]) -> String {
    let mut metadata = Vec::with_capacity(ERC5564_METADATA_LAYOUT_LEN);
    metadata.push(view_tag);
    metadata.extend_from_slice(&ERC5564_METADATA_NATIVE_MARKER);
    metadata.extend_from_slice(
        &decode_ethereum_address(ERC5564_METADATA_NATIVE_SENTINEL_ADDRESS)
            .expect("sentinel address literal is valid"),
    );
    metadata.extend_from_slice(amount_wei);
    hex::encode(metadata)
}

/// Encode the EIP-5564 token metadata SHOULD layout for an ERC-20 transfer:
/// `view_tag ‖ transfer(address,uint256) selector ‖ token_address ‖ amount`
/// (57 bytes). Per the EIP, the function identifier MUST be the function
/// selector when one is available.
pub fn encode_erc5564_metadata_erc20_transfer(
    view_tag: u8,
    token_address: &str,
    amount: &[u8; 32],
) -> Result<String, EthereumStealthError> {
    let token_address = decode_ethereum_address(token_address)?;
    let mut metadata = Vec::with_capacity(ERC5564_METADATA_LAYOUT_LEN);
    metadata.push(view_tag);
    metadata.extend_from_slice(&ERC5564_METADATA_ERC20_TRANSFER_SELECTOR);
    metadata.extend_from_slice(&token_address);
    metadata.extend_from_slice(amount);
    Ok(hex::encode(metadata))
}

/// Decode asset/amount hints from announcement metadata, defensively.
///
/// Returns `None` — never an error — for anything that is not exactly one of
/// the two 57-byte EIP-5564 SHOULD layouts: view-tag-only metadata, unknown
/// trailing layouts, truncated layouts, and native markers carrying a
/// non-sentinel address all parse as "no hints". Byte 0 (the view tag) is not
/// interpreted; callers match it against the announcement's view tag.
pub fn decode_erc5564_metadata_hints(metadata: &[u8]) -> Option<Erc5564MetadataHints> {
    if metadata.len() != ERC5564_METADATA_LAYOUT_LEN {
        return None;
    }
    let mut amount = [0u8; 32];
    amount.copy_from_slice(&metadata[25..57]);
    if metadata[1..5] == ERC5564_METADATA_NATIVE_MARKER {
        let sentinel = decode_ethereum_address(ERC5564_METADATA_NATIVE_SENTINEL_ADDRESS)
            .expect("sentinel address literal is valid");
        if metadata[5..25] != sentinel {
            // Native marker with an unrecognized address: an unknown layout,
            // not asset information Sigillum can act on.
            return None;
        }
        return Some(Erc5564MetadataHints::Native { amount_wei: amount });
    }
    let mut function_selector = [0u8; 4];
    function_selector.copy_from_slice(&metadata[1..5]);
    Some(Erc5564MetadataHints::Token {
        function_selector,
        token_address: format!("0x{}", hex::encode(&metadata[5..25])),
        amount,
    })
}

// ── Stealth address verification ──

/// Watch-only recipient check, per the EIP-5564 `checkStealthAddress` key
/// signature: stealth address + ephemeral public key + viewing private key +
/// spending PUBLIC key, under one explicit shared-secret hash convention.
///
/// This is the detection core: view-tag pre-filter → shared-secret hash per
/// `convention` → stealth address recompute (spending public key + hash
/// offset point) → compare. It never touches — and cannot
/// touch, see [`EthereumStealthWatchView`] — the spending private key, so
/// announcement scanning and deposit checking run without spending secret
/// material in memory. The full-wallet entry points
/// ([`check_ethereum_stealth_address`], [`check_ethereum_stealth_address_any`])
/// delegate here, so both paths execute one identical implementation and
/// produce byte-identical results.
///
/// The view tag provides a fast pre-filter: if it doesn't match, the
/// computation short-circuits immediately without performing full point
/// arithmetic.
///
/// # Security
///
/// The view tag is only 1 byte, so false positives are expected (~1 in 256).
/// Always verify the full stealth address derivation before processing.
pub fn check_ethereum_stealth_address_watch_only(
    view: &EthereumStealthWatchView,
    stealth_address: &str,
    ephemeral_public_key_hex: &str,
    view_tag: Option<u8>,
    convention: StealthHashConvention,
) -> Result<EthereumStealthCheck, EthereumStealthError> {
    let ephemeral_public_key = parse_public_key_hex(ephemeral_public_key_hex)?;
    let hashed_shared_secret = hashed_shared_secret_for_recipient(
        &view.viewing_private_key,
        &ephemeral_public_key,
        convention,
    )?;

    if let Some(expected_view_tag) = view_tag {
        let derived_view_tag = derive_view_tag(&hashed_shared_secret);
        if derived_view_tag != expected_view_tag {
            return Err(EthereumStealthError::ViewTagMismatch);
        }
    }

    let stealth_public_key =
        derive_stealth_public_key(&view.spending_public_key, &hashed_shared_secret)?;
    let derived_stealth_address = ethereum_address_from_public_key(&stealth_public_key);
    let expected_stealth_address = normalize_ethereum_address(stealth_address)?;

    Ok(EthereumStealthCheck {
        matches: derived_stealth_address == expected_stealth_address,
        derived_stealth_address,
        view_tag_hex: hex::encode([derive_view_tag(&hashed_shared_secret)]),
        stealth_hash_convention: convention,
    })
}

/// Dual-decode variant of [`check_ethereum_stealth_address_watch_only`]:
/// probes the given conventions in order and returns the first full
/// stealth-address match.
///
/// Detection paths use this with [`StealthHashConvention::PROBE_ORDER`]
/// (standard first, legacy second) so payments created with either convention
/// are found in one pass, including legacy payments whose on-chain view tag
/// only matches under the legacy convention. The returned
/// [`EthereumStealthCheck::stealth_hash_convention`] is the convention the
/// payment was actually made with; callers persist it so sweeping derives the
/// stealth key with the right convention.
///
/// Returns `Ok` with `matches: false` (derived values from the first probed
/// convention that passed the view-tag filter, or the first convention when no
/// view tag was supplied) when no convention matches, and
/// [`EthereumStealthError::ViewTagMismatch`] when every convention is excluded
/// by the view-tag pre-filter.
pub fn check_ethereum_stealth_address_any_watch_only(
    view: &EthereumStealthWatchView,
    stealth_address: &str,
    ephemeral_public_key_hex: &str,
    view_tag: Option<u8>,
    conventions: &[StealthHashConvention],
) -> Result<EthereumStealthCheck, EthereumStealthError> {
    let mut tag_filtered = false;
    let mut first_candidate: Option<EthereumStealthCheck> = None;
    for &convention in conventions {
        match check_ethereum_stealth_address_watch_only(
            view,
            stealth_address,
            ephemeral_public_key_hex,
            view_tag,
            convention,
        ) {
            Ok(check) if check.matches => return Ok(check),
            Ok(check) => {
                if first_candidate.is_none() {
                    first_candidate = Some(check);
                }
            }
            Err(EthereumStealthError::ViewTagMismatch) => tag_filtered = true,
            Err(error) => return Err(error),
        }
    }
    if let Some(candidate) = first_candidate {
        return Ok(EthereumStealthCheck {
            matches: false,
            ..candidate
        });
    }
    if tag_filtered {
        return Err(EthereumStealthError::ViewTagMismatch);
    }
    Err(EthereumStealthError::InvalidStealthHashConvention(
        "no conventions to probe".to_string(),
    ))
}

/// Verify that a stealth address matches a given ephemeral public key under one
/// explicit shared-secret hash convention.
///
/// Recipients call this function during block scanning to determine whether an on-chain
/// payment was intended for them. The function computes the stealth address that would
/// have been generated with the given ephemeral public key and compares it to the
/// provided stealth address. Callers that do not know the payment's convention should
/// use [`check_ethereum_stealth_address_any`] to probe.
///
/// The view tag provides a fast pre-filter: if it doesn't match, the computation short-circuits
/// immediately without performing full ECDH and point arithmetic.
///
/// This is the full-wallet entry point: it reduces the wallet to its
/// [`EthereumStealthWatchView`] and delegates to
/// [`check_ethereum_stealth_address_watch_only`], so detection never depends
/// on the spending private key even when the caller holds it. Detection-only
/// callers should prefer deriving the watch view directly
/// ([`derive_watch_only_sigillum_ethereum_stealth_wallet`]) so the spending
/// secret never enters the scan path at all.
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
    convention: StealthHashConvention,
) -> Result<EthereumStealthCheck, EthereumStealthError> {
    check_ethereum_stealth_address_watch_only(
        &wallet.watch_view(),
        stealth_address,
        ephemeral_public_key_hex,
        view_tag,
        convention,
    )
}

/// Dual-decode variant of [`check_ethereum_stealth_address`]: probes the given
/// conventions in order and returns the first full stealth-address match.
///
/// Detection paths use this with [`StealthHashConvention::PROBE_ORDER`]
/// (standard first, legacy second) so payments created with either convention
/// are found in one pass, including legacy payments whose on-chain view tag
/// only matches under the legacy convention. The returned
/// [`EthereumStealthCheck::stealth_hash_convention`] is the convention the
/// payment was actually made with; callers persist it so sweeping derives the
/// stealth key with the right convention.
///
/// Like the single-convention entry point, this delegates to the watch-only
/// core ([`check_ethereum_stealth_address_any_watch_only`]).
///
/// Returns `Ok` with `matches: false` (derived values from the first probed
/// convention that passed the view-tag filter, or the first convention when no
/// view tag was supplied) when no convention matches, and
/// [`EthereumStealthError::ViewTagMismatch`] when every convention is excluded
/// by the view-tag pre-filter.
pub fn check_ethereum_stealth_address_any(
    wallet: &EthereumStealthWallet,
    stealth_address: &str,
    ephemeral_public_key_hex: &str,
    view_tag: Option<u8>,
    conventions: &[StealthHashConvention],
) -> Result<EthereumStealthCheck, EthereumStealthError> {
    check_ethereum_stealth_address_any_watch_only(
        &wallet.watch_view(),
        stealth_address,
        ephemeral_public_key_hex,
        view_tag,
        conventions,
    )
}

// ── Stealth signing ──

/// Derive the stealth private key for a payment and verify it matches the expected address.
///
/// Combines view-tag verification, ECDH shared-secret derivation, and stealth key
/// computation into a single auditable call site. Every signing operation MUST go
/// through this function to guarantee address verification before key use.
///
/// The `convention` selects the shared-secret hash encoding; it MUST be the
/// convention the payment was actually created with (persisted on the deposit
/// record), otherwise the derived address will not match and this function
/// fails with [`EthereumStealthError::AddressMismatch`] instead of producing a
/// wrong key.
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
    convention: StealthHashConvention,
) -> Result<(SecretKey, [u8; 32]), EthereumStealthError> {
    let ephemeral_public_key = parse_public_key_hex(ephemeral_public_key_hex)?;
    let hashed_shared_secret = hashed_shared_secret_for_recipient(
        &wallet.viewing_private_key,
        &ephemeral_public_key,
        convention,
    )?;

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
/// - `convention`: Shared-secret hash convention of the payment (from the deposit
///   record). A wrong convention fails with `AddressMismatch` before any signing.
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
    convention: StealthHashConvention,
) -> Result<EthereumStealthSignature, EthereumStealthError> {
    if digest.len() != 32 {
        return Err(EthereumStealthError::InvalidDigestLength);
    }

    let (stealth_private_key, hashed_shared_secret) = derive_verified_stealth_key(
        wallet,
        stealth_address,
        ephemeral_public_key_hex,
        view_tag,
        convention,
    )?;

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
/// - `convention`: Shared-secret hash convention of the payment (from the deposit
///   record). A wrong convention fails with `AddressMismatch` before any signing.
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
    convention: StealthHashConvention,
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
        convention,
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
/// - `convention`: Shared-secret hash convention of the payment (from the deposit
///   record). A wrong convention fails with `AddressMismatch` before any signing.
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
    convention: StealthHashConvention,
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
        convention,
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
#[allow(clippy::too_many_arguments)]
fn sign_ethereum_stealth_eip1559_transaction(
    wallet: &EthereumStealthWallet,
    stealth_address: &str,
    ephemeral_public_key_hex: &str,
    view_tag: Option<u8>,
    tx: UnsignedEip1559Transaction,
    kind: &str,
    to_address: String,
    convention: StealthHashConvention,
) -> Result<EthereumSignedTransaction, EthereumStealthError> {
    let (stealth_private_key, _hashed_shared_secret) = derive_verified_stealth_key(
        wallet,
        stealth_address,
        ephemeral_public_key_hex,
        view_tag,
        convention,
    )?;

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

/// An arbitrary EIP-1559 contract call: destination address, calldata, and
/// optional native value. Generic counterpart to [`EthereumEip1559Transfer`]
/// and [`EthereumEip1559Erc20Transfer`] for prepared calldata that does not
/// fit either shape (approvals/revocations, DeFi exit-adapter calls, Merkle
/// claims, NFT transfers). The caller is responsible for building `data` —
/// this module never re-derives calldata from higher-level intent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EthereumEip1559Call {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: [u8; 32],
    pub max_fee_per_gas: [u8; 32],
    pub gas_limit: u64,
    pub to_address: String,
    pub value: [u8; 32],
    pub data: Vec<u8>,
}

/// Sign an arbitrary EIP-1559 contract call (destination + calldata + value)
/// with a directly supplied signing key. Used by callers that hold a
/// prepared call (target address, calldata, value) verbatim and must not
/// rebuild it at signing time.
pub fn sign_ethereum_call(
    signing_key: &SigningKey,
    call: &EthereumEip1559Call,
) -> Result<EthereumSignedTransaction, EthereumStealthError> {
    let to_address = normalize_ethereum_address(&call.to_address)?;
    sign_ethereum_eip1559_transaction(
        signing_key,
        UnsignedEip1559Transaction {
            chain_id: call.chain_id,
            nonce: call.nonce,
            max_priority_fee_per_gas: call.max_priority_fee_per_gas,
            max_fee_per_gas: call.max_fee_per_gas,
            gas_limit: call.gas_limit,
            to_address: to_address.clone(),
            value: call.value,
            data: call.data.clone(),
        },
        "contract-call",
        to_address,
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

    // EIP-5564 allows two meta-address key forms: the dual-key form
    // (spending ‖ viewing compressed SEC1 keys, 66 hex chars each) and the
    // single-key form (one 33-byte compressed key, 66 hex chars) where the
    // SAME key serves as both spending and viewing key. Fluidkey's 64-byte
    // X‖Y encoding (128 hex chars) remains unsupported.
    let (spending_public_key, viewing_public_key) = match payload.len() {
        132 => (
            parse_public_key_hex(&payload[..66])?,
            parse_public_key_hex(&payload[66..])?,
        ),
        66 => {
            let key = parse_public_key_hex(payload)?;
            (key, key)
        }
        _ => return Err(EthereumStealthError::InvalidMetaAddress),
    };

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

/// Hash the ECDH shared secret between `private_key` and `public_key` with the
/// given convention. This single hash drives the view tag, the stealth public
/// key/address, and the stealth private key, so the convention must match the
/// one the payer used.
///
/// - `XOnly32` (legacy): `keccak256` over the 32-byte x-coordinate, exactly the
///   pre-switch code path (k256 `diffie_hellman` + `raw_secret_bytes`).
/// - `Compressed33` (standard): `keccak256` over the 33-byte compressed SEC1
///   encoding of the same shared point. Byte-compatible with the ScopeLift
///   `stealth-address-sdk`, where `getHashedSharedSecret` is
///   `keccak256(getSharedSecret(...))` and `@noble/secp256k1` v2
///   `getSharedSecret(privA, pubB, isCompressed = true)` returns
///   `Point.fromBytes(pubB).multiply(privA).toBytes(true)` — the compressed
///   point (confirmed 2026-07-17, see module docs).
///
/// The shared point is a secret: scalar multiplication uses k256's
/// constant-time arithmetic and the encoding is hashed immediately.
fn hashed_shared_secret(
    private_key: &SecretKey,
    public_key: &PublicKey,
    convention: StealthHashConvention,
) -> Result<[u8; 32], EthereumStealthError> {
    match convention {
        StealthHashConvention::XOnly32 => {
            let shared_secret =
                diffie_hellman(private_key.to_nonzero_scalar(), public_key.as_affine());
            Ok(Keccak256::digest(shared_secret.raw_secret_bytes()).into())
        }
        StealthHashConvention::Compressed33 => {
            let shared_point = (ProjectivePoint::from(*public_key.as_affine())
                * *private_key.to_nonzero_scalar().as_ref())
            .to_affine();
            let compressed = shared_point.to_encoded_point(true);
            debug_assert_eq!(compressed.as_bytes().len(), 33);
            Ok(Keccak256::digest(compressed.as_bytes()).into())
        }
    }
}

fn hashed_shared_secret_for_recipient(
    viewing_private_key: &SecretKey,
    ephemeral_public_key: &PublicKey,
    convention: StealthHashConvention,
) -> Result<[u8; 32], EthereumStealthError> {
    hashed_shared_secret(viewing_private_key, ephemeral_public_key, convention)
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

pub fn encode_erc5564_announce_calldata(
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

    // External ERC-5564 scheme-1 known-answer vectors.
    //
    // Generated on 2026-07-30 by executing the official ScopeLift
    // `stealth-address-sdk` v1.0.0-beta.5 source at commit
    // 88bcc27c3b6163080ee18f330dfd6336dc8bd2e2:
    // https://github.com/ScopeLift/stealth-address-sdk/tree/v1.0.0-beta.5
    //
    // `generateStealthAddress` supplied the ephemeral public key, view tag, and
    // stealth address; `computeStealthKey` independently supplied the recovered
    // private key; and `checkStealthAddress` returned true for each vector. The
    // legacy values were separately derived with the SDK's pinned noble/viem
    // primitive stack by hashing only bytes 1..33 of the compressed shared point,
    // reproducing Sigillum's pre-fix x-coordinate convention. None of these
    // expected values came from a Sigillum generation/detection roundtrip.
    struct StealthInteropVector {
        spending_private_key_hex: &'static str,
        viewing_private_key_hex: &'static str,
        ephemeral_private_key_hex: &'static str,
        stealth_meta_address: &'static str,
        ephemeral_public_key_hex: &'static str,
        scope_lift_view_tag_hex: &'static str,
        scope_lift_stealth_address: &'static str,
        scope_lift_stealth_private_key_hex: &'static str,
        legacy_view_tag_hex: &'static str,
        legacy_stealth_address: &'static str,
        legacy_stealth_private_key_hex: &'static str,
    }

    const STEALTH_INTEROP_VECTORS: [StealthInteropVector; 2] = [
        StealthInteropVector {
            spending_private_key_hex: "1111111111111111111111111111111111111111111111111111111111111111",
            viewing_private_key_hex: "2222222222222222222222222222222222222222222222222222222222222222",
            ephemeral_private_key_hex: "3333333333333333333333333333333333333333333333333333333333333333",
            stealth_meta_address: "st:eth:0x034f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa02466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27",
            ephemeral_public_key_hex: "023c72addb4fdf09af94f0c94d7fe92a386a7e70cf8a1d85916386bb2535c7b1b1",
            scope_lift_view_tag_hex: "20",
            scope_lift_stealth_address: "0xd8606ed2ecdb71fdcb8cca8fa1925ff84238f2a9",
            scope_lift_stealth_private_key_hex: "32074def70f9689560d0eb1b86aa895b735ed5852c9ce187ff0dcd968e8a19d3",
            legacy_view_tag_hex: "83",
            legacy_stealth_address: "0x35cfea8cf9c3e33bc210a65793840aa5a86f51a8",
            legacy_stealth_private_key_hex: "949f681db53715b3d5509806f2bc71df8276fc8c983a4af809e2c8218fff225f",
        },
        StealthInteropVector {
            spending_private_key_hex: "0000000000000000000000000000000000000000000000000000000000000002",
            viewing_private_key_hex: "0000000000000000000000000000000000000000000000000000000000000003",
            ephemeral_private_key_hex: "0000000000000000000000000000000000000000000000000000000000000005",
            stealth_meta_address: "st:eth:0x02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee502f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9",
            ephemeral_public_key_hex: "022f8bde4d1a07209355b4a7250a5c5128e88b84bddc619ab7cba8d569b240efe4",
            scope_lift_view_tag_hex: "05",
            scope_lift_stealth_address: "0x058d1eae9a219a0a1dc1da37462f4e12ae369c7d",
            scope_lift_stealth_private_key_hex: "05043fd4ff06a9e61df614f47987d6325c6a0c77cb9f40ccf00ad2e8e60e1809",
            legacy_view_tag_hex: "b4",
            legacy_stealth_address: "0x82a80277c84afbb61ad48dd65c4c890324d4be8f",
            legacy_stealth_private_key_hex: "b4bfb09a7507847db47c7ea325081971dbec889ea95cf9d1f17b47b590737e2f",
        },
    ];

    fn decode_secret_key(value: &str) -> SecretKey {
        let bytes = hex::decode(value).unwrap();
        SecretKey::from_slice(&bytes).unwrap()
    }

    fn decode_private_key_bytes(value: &str) -> [u8; 32] {
        hex::decode(value).unwrap().try_into().unwrap()
    }

    fn wallet_from_interop_vector(vector: &StealthInteropVector) -> EthereumStealthWallet {
        let spending_private_key = decode_secret_key(vector.spending_private_key_hex);
        let viewing_private_key = decode_secret_key(vector.viewing_private_key_hex);
        let parsed_meta_address = parse_meta_address(vector.stealth_meta_address).unwrap();

        assert_eq!(
            parsed_meta_address.spending_public_key,
            spending_private_key.public_key()
        );
        assert_eq!(
            parsed_meta_address.viewing_public_key,
            viewing_private_key.public_key()
        );

        EthereumStealthWallet {
            meta_address: EthereumStealthMetaAddress {
                wallet: "interop-vector".to_string(),
                short_name: parsed_meta_address.short_name,
                scheme_id: ETHEREUM_STEALTH_SCHEME_ID,
                stealth_meta_address: vector.stealth_meta_address.to_string(),
                spending_public_key_hex: encode_public_key(&spending_private_key.public_key()),
                viewing_public_key_hex: encode_public_key(&viewing_private_key.public_key()),
            },
            spending_private_key,
            viewing_private_key,
        }
    }

    #[test]
    fn generated_payments_match_scope_lift_scheme_1_vectors() {
        for vector in &STEALTH_INTEROP_VECTORS {
            let wallet = wallet_from_interop_vector(vector);
            let payment = generate_ethereum_stealth_address(
                vector.stealth_meta_address,
                Some(decode_private_key_bytes(vector.ephemeral_private_key_hex)),
                StealthHashConvention::Compressed33,
            )
            .unwrap();

            assert_eq!(
                payment.ephemeral_public_key_hex,
                vector.ephemeral_public_key_hex
            );
            assert_eq!(payment.view_tag_hex, vector.scope_lift_view_tag_hex);
            assert_eq!(payment.stealth_address, vector.scope_lift_stealth_address);

            let check = check_ethereum_stealth_address(
                &wallet,
                vector.scope_lift_stealth_address,
                vector.ephemeral_public_key_hex,
                Some(u8::from_str_radix(vector.scope_lift_view_tag_hex, 16).unwrap()),
                StealthHashConvention::Compressed33,
            )
            .unwrap();
            assert!(check.matches);
            assert_eq!(
                check.derived_stealth_address,
                vector.scope_lift_stealth_address
            );

            let (stealth_private_key, hashed_shared_secret) = derive_verified_stealth_key(
                &wallet,
                vector.scope_lift_stealth_address,
                vector.ephemeral_public_key_hex,
                Some(u8::from_str_radix(vector.scope_lift_view_tag_hex, 16).unwrap()),
                StealthHashConvention::Compressed33,
            )
            .unwrap();
            assert_eq!(
                hex::encode(stealth_private_key.to_bytes()),
                vector.scope_lift_stealth_private_key_hex
            );
            assert_eq!(
                derive_view_tag(&hashed_shared_secret),
                u8::from_str_radix(vector.scope_lift_view_tag_hex, 16).unwrap()
            );
        }
    }

    #[test]
    fn legacy_sigillum_x_only_payments_remain_detectable_and_spendable() {
        for vector in &STEALTH_INTEROP_VECTORS {
            let wallet = wallet_from_interop_vector(vector);
            let legacy_view_tag = u8::from_str_radix(vector.legacy_view_tag_hex, 16).unwrap();

            let check = check_ethereum_stealth_address(
                &wallet,
                vector.legacy_stealth_address,
                vector.ephemeral_public_key_hex,
                Some(legacy_view_tag),
                StealthHashConvention::XOnly32,
            )
            .unwrap();
            assert!(check.matches);
            assert_eq!(check.derived_stealth_address, vector.legacy_stealth_address);

            let (stealth_private_key, hashed_shared_secret) = derive_verified_stealth_key(
                &wallet,
                vector.legacy_stealth_address,
                vector.ephemeral_public_key_hex,
                Some(legacy_view_tag),
                StealthHashConvention::XOnly32,
            )
            .unwrap();
            assert_eq!(
                hex::encode(stealth_private_key.to_bytes()),
                vector.legacy_stealth_private_key_hex
            );
            assert_eq!(derive_view_tag(&hashed_shared_secret), legacy_view_tag);

            let signature = sign_ethereum_stealth_digest(
                &wallet,
                vector.legacy_stealth_address,
                vector.ephemeral_public_key_hex,
                Some(legacy_view_tag),
                &[7u8; 32],
                StealthHashConvention::XOnly32,
            )
            .unwrap();
            assert_eq!(signature.stealth_address, vector.legacy_stealth_address);
            assert_eq!(signature.view_tag_hex, vector.legacy_view_tag_hex);

            let error = derive_verified_stealth_key(
                &wallet,
                vector.legacy_stealth_address,
                vector.ephemeral_public_key_hex,
                Some(u8::from_str_radix(vector.scope_lift_view_tag_hex, 16).unwrap()),
                StealthHashConvention::XOnly32,
            )
            .unwrap_err();
            assert_eq!(error, EthereumStealthError::ViewTagMismatch);
        }
    }

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
            StealthHashConvention::STANDARD,
        )
        .unwrap();

        let check = check_ethereum_stealth_address(
            &wallet,
            &payment.stealth_address,
            &payment.ephemeral_public_key_hex,
            Some(hex::decode(&payment.view_tag_hex).unwrap()[0]),
            StealthHashConvention::STANDARD,
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
            StealthHashConvention::STANDARD,
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
            StealthHashConvention::STANDARD,
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
            StealthHashConvention::STANDARD,
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
            StealthHashConvention::STANDARD,
        )
        .unwrap();

        let error = check_ethereum_stealth_address(
            &wallet,
            &payment.stealth_address,
            &payment.ephemeral_public_key_hex,
            Some(0xff),
            StealthHashConvention::STANDARD,
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
            StealthHashConvention::STANDARD,
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
            StealthHashConvention::STANDARD,
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
            StealthHashConvention::STANDARD,
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
            StealthHashConvention::STANDARD,
        )
        .unwrap();

        assert_eq!(signed.kind, "erc20-transfer");
        assert!(signed.data_hex.starts_with("a9059cbb"));
        assert_eq!(signed.value_hex, "0x0");
    }

    #[test]
    fn sign_ethereum_call_signs_arbitrary_prepared_calldata_verbatim() {
        let signing_key = SigningKey::from_slice(&[41u8; 32]).unwrap();
        let call = EthereumEip1559Call {
            chain_id: 1,
            nonce: 3,
            max_priority_fee_per_gas: decode_quantity_hex("0x1").unwrap(),
            max_fee_per_gas: decode_quantity_hex("0x2").unwrap(),
            gas_limit: 65_000,
            to_address: "0x2222222222222222222222222222222222222222".into(),
            value: [0u8; 32],
            data: hex::decode("095ea7b3").unwrap(),
        };

        let signed = sign_ethereum_call(&signing_key, &call).unwrap();

        assert_eq!(signed.kind, "contract-call");
        assert_eq!(
            signed.to_address,
            "0x2222222222222222222222222222222222222222"
        );
        assert_eq!(signed.data_hex, "095ea7b3");
        assert_eq!(signed.value_hex, "0x0");
        assert_eq!(signed.transaction_hash_hex.len(), 64);

        // Reusing the exact same call must sign deterministically...
        let signed_again = sign_ethereum_call(&signing_key, &call).unwrap();
        assert_eq!(signed.raw_transaction_hex, signed_again.raw_transaction_hex);

        // ...and any change to the prepared call must change the signature.
        let mut tampered = call.clone();
        tampered.data = hex::decode("a9059cbb").unwrap();
        let signed_tampered = sign_ethereum_call(&signing_key, &tampered).unwrap();
        assert_ne!(
            signed.transaction_hash_hex,
            signed_tampered.transaction_hash_hex
        );
    }

    // ── Fixed external test vectors (no self-roundtrip) ─────────────────────
    //
    // PROVENANCE (recorded 2026-07-17):
    // * Inputs: the spending/viewing private keys are PUBLISHED by the ScopeLift
    //   stealth-address-sdk test suite (src/utils/crypto/test/computeStealthKey.test.ts
    //   @ main; the spending key is printed there as 63 hex chars with the leading
    //   zero nibble elided — left-padded to 32 bytes here). The reference generator
    //   asserts these keys reproduce the SDK-published meta-address
    //   `st:eth:0x033404e8...97` + `0390ad5e...46e`. Ephemeral private keys are
    //   Sigillum-chosen fixed constants.
    // * Expected values: computed with an independent Node.js reference using the
    //   SDK's own dependency stack (@noble/secp256k1 v2.3.0, @noble/hashes v2.2.0),
    //   mirroring the SDK source retrieved 2026-07-17 byte-for-byte:
    //   - https://raw.githubusercontent.com/ScopeLift/stealth-address-sdk/main/src/utils/crypto/generateStealthAddress.ts
    //     (`getHashedSharedSecret` = `keccak256(getSharedSecret(...))`; `getViewTag`
    //     = most significant byte of the hash; `getStealthPublicKey` =
    //     spending point + `ProjectivePoint.fromPrivateKey(hash)`; address =
    //     keccak256(uncompressed X‖Y)[12..])
    //   - https://raw.githubusercontent.com/ScopeLift/stealth-address-sdk/main/src/utils/crypto/computeStealthKey.ts
    //     (`(spendingPrivateKey + BigInt(hashedSharedSecret)) % CURVE.n`)
    //   `@noble/secp256k1` v2 `getSharedSecret(privA, pubB, isCompressed = true)`
    //   returns the 33-byte compressed SEC1 point by default (confirmed against
    //   the published v2.3.0 source), which makes the standard convention
    //   `keccak256` over those 33 bytes.
    // * The legacy `XOnly32` expected values were additionally verified
    //   byte-exactly against the pre-switch Sigillum implementation (the code
    //   path is unchanged; these vectors pin it for migration safety so
    //   dual-decode of pre-switch deposits can never regress).
    // * The SDK does not publish byte-exact end-to-end vectors for a fixed
    //   ephemeral key; its only fixed hash-level vector is the `getViewTag`
    //   example, pinned verbatim in `sdk_view_tag_vector_matches`.

    /// Spending/viewing keys + meta-address published by the SDK test suite.
    const VECTOR_SPENDING_PRIVATE_KEY: &str =
        "0363721eb9e981558c748b824cb32a840da2b3e8957c2fc3bcb8d9c86cb87456";
    const VECTOR_VIEWING_PRIVATE_KEY: &str =
        "b52a0555f6a8663d89f00365893b1ef9e38eaf2e8bc48a63319c9ea5cb4a27c5";
    const VECTOR_META_ADDRESS: &str = "st:eth:0x033404e82cd2a92321d51e13064ec13a0fb0192a9fdaaca1cfb47b37bd27ec13970390ad5eca026c05ab5cf4d620a2ac65241b11df004ddca360e954db1b26e3846e";

    struct FixedVector {
        ephemeral_private_key: &'static str,
        ephemeral_public_key: &'static str,
        hashed_shared_secret: &'static str,
        view_tag: u8,
        stealth_address: &'static str,
        stealth_private_key: &'static str,
    }

    /// Standard (`compressed33`, ScopeLift-compatible) vectors.
    const STANDARD_VECTORS: [FixedVector; 2] = [
        FixedVector {
            ephemeral_private_key: "0000000000000000000000000000000000000000000000000000000000000003",
            ephemeral_public_key: "02f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9",
            hashed_shared_secret: "040e0548482e80fe8fa5ca6d6b199a19856651812273993bce46595c2b17d4b4",
            view_tag: 0x04,
            stealth_address: "0xc4781e62ebcd5457deef51b90ba4acbb3b17ff30",
            stealth_private_key: "07717767021802541c1a55efb7ccc49d93090569b7efc8ff8aff332497d0490a",
        },
        FixedVector {
            ephemeral_private_key: "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
            ephemeral_public_key: "028db55b05db86c0b1786ca49f095d76344c9e6056b2f02701a7e7f3c20aabfd91",
            hashed_shared_secret: "7257024d9481707535109cde1f6add9bde15dd973d1197ad488e1faefdd64726",
            view_tag: 0x72,
            stealth_address: "0x8f8c077011630d8076a8fe179675221e7d2a2167",
            stealth_private_key: "75ba746c4e6af1cac18528606c1e081febb8917fd28dc7710546f9776a8ebb7c",
        },
    ];

    /// Legacy (`x32`) vectors, pinning the pre-switch implementation.
    const LEGACY_VECTORS: [FixedVector; 2] = [
        FixedVector {
            ephemeral_private_key: "0000000000000000000000000000000000000000000000000000000000000003",
            ephemeral_public_key: "02f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9",
            hashed_shared_secret: "089b8f4b70e7b614390a0daf3dba95b3f389a79d24da91ebc15d8aa2a518f868",
            view_tag: 0x08,
            stealth_address: "0x4039e78bd8141082a667050b3a19b6f58c9fe46b",
            stealth_private_key: "0bff016a2ad13769c57e99318a6dc038012c5b85ba56c1af7e16646b11d16cbe",
        },
        FixedVector {
            ephemeral_private_key: "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
            ephemeral_public_key: "028db55b05db86c0b1786ca49f095d76344c9e6056b2f02701a7e7f3c20aabfd91",
            hashed_shared_secret: "54f9376d308b9f4e06383b663fae19e8ea6d84e83a1fcc1d660b926642b6bfd7",
            view_tag: 0x54,
            stealth_address: "0x239b96cf4b6b15dc2bdd233988d6ac1867adf7fa",
            stealth_private_key: "585ca98bea7520a392acc6e88c61446cf81038d0cf9bfbe122c46c2eaf6f342d",
        },
    ];

    fn fixed_secret_key(hex_key: &str) -> SecretKey {
        SecretKey::from_slice(&hex::decode(hex_key).unwrap()).unwrap()
    }

    fn vector_wallet() -> EthereumStealthWallet {
        let spending_private_key = fixed_secret_key(VECTOR_SPENDING_PRIVATE_KEY);
        let viewing_private_key = fixed_secret_key(VECTOR_VIEWING_PRIVATE_KEY);
        // The SDK-published private keys must reproduce the SDK-published
        // meta-address keys (this is the reference generator's sanity check).
        assert_eq!(
            format!(
                "st:eth:0x{}{}",
                encode_public_key(&spending_private_key.public_key()),
                encode_public_key(&viewing_private_key.public_key())
            ),
            VECTOR_META_ADDRESS
        );
        EthereumStealthWallet {
            meta_address: EthereumStealthMetaAddress {
                wallet: "vectors".into(),
                short_name: "eth".into(),
                scheme_id: ETHEREUM_STEALTH_SCHEME_ID,
                stealth_meta_address: VECTOR_META_ADDRESS.into(),
                spending_public_key_hex: encode_public_key(&spending_private_key.public_key()),
                viewing_public_key_hex: encode_public_key(&viewing_private_key.public_key()),
            },
            spending_private_key,
            viewing_private_key,
        }
    }

    fn assert_fixed_vector(convention: StealthHashConvention, vector: &FixedVector) {
        let wallet = vector_wallet();
        let meta = parse_meta_address(VECTOR_META_ADDRESS).unwrap();
        let mut ephemeral_bytes = [0u8; 32];
        ephemeral_bytes.copy_from_slice(&hex::decode(vector.ephemeral_private_key).unwrap());

        // Payer side: generation from the meta-address is byte-exact.
        let payment = generate_ethereum_stealth_address(
            VECTOR_META_ADDRESS,
            Some(ephemeral_bytes),
            convention,
        )
        .unwrap();
        assert_eq!(payment.stealth_address, vector.stealth_address);
        assert_eq!(
            payment.ephemeral_public_key_hex,
            vector.ephemeral_public_key
        );
        assert_eq!(payment.view_tag_hex, hex::encode([vector.view_tag]));
        assert_eq!(payment.stealth_hash_convention, convention);

        // Hash level: the exact bytes feeding every derivation.
        let ephemeral_secret = ephemeral_private_key_to_secret(ephemeral_bytes).unwrap();
        let hashed =
            hashed_shared_secret(&ephemeral_secret, &meta.viewing_public_key, convention).unwrap();
        assert_eq!(hex::encode(hashed), vector.hashed_shared_secret);
        assert_eq!(derive_view_tag(&hashed), vector.view_tag);

        // Recipient side: single-convention check and dual-decode both match,
        // and the derived stealth private key is byte-exact.
        let check = check_ethereum_stealth_address(
            &wallet,
            vector.stealth_address,
            vector.ephemeral_public_key,
            Some(vector.view_tag),
            convention,
        )
        .unwrap();
        assert!(check.matches);
        assert_eq!(check.stealth_hash_convention, convention);

        let probed = check_ethereum_stealth_address_any(
            &wallet,
            vector.stealth_address,
            vector.ephemeral_public_key,
            Some(vector.view_tag),
            &StealthHashConvention::PROBE_ORDER,
        )
        .unwrap();
        assert!(probed.matches);
        assert_eq!(probed.stealth_hash_convention, convention);

        // Watch-only recipient side: the same checks from the viewing
        // private key + spending PUBLIC key alone are byte-identical to the
        // full-wallet results, for this exact fixed vector. (The full-wallet
        // entry points delegate to the watch-only core, so equality holds by
        // construction — pinned here so the two can never silently diverge.)
        let watch_view = vector_wallet().watch_view();
        let watch_check = check_ethereum_stealth_address_watch_only(
            &watch_view,
            vector.stealth_address,
            vector.ephemeral_public_key,
            Some(vector.view_tag),
            convention,
        )
        .unwrap();
        assert_eq!(watch_check, check);

        let watch_probed = check_ethereum_stealth_address_any_watch_only(
            &watch_view,
            vector.stealth_address,
            vector.ephemeral_public_key,
            Some(vector.view_tag),
            &StealthHashConvention::PROBE_ORDER,
        )
        .unwrap();
        assert_eq!(watch_probed, probed);

        let (stealth_key, _) = derive_verified_stealth_key(
            &wallet,
            vector.stealth_address,
            vector.ephemeral_public_key,
            Some(vector.view_tag),
            convention,
        )
        .unwrap();
        assert_eq!(
            hex::encode(stealth_key.to_bytes()),
            vector.stealth_private_key
        );
    }

    #[test]
    fn standard_convention_matches_fixed_scopelift_vectors() {
        for vector in &STANDARD_VECTORS {
            assert_fixed_vector(StealthHashConvention::Compressed33, vector);
        }
    }

    #[test]
    fn legacy_convention_matches_fixed_pre_switch_vectors() {
        for vector in &LEGACY_VECTORS {
            assert_fixed_vector(StealthHashConvention::XOnly32, vector);
        }
    }

    // ── Single-key (66-hex-char) meta-addresses ──

    /// EIP-5564 single-key meta-address: one compressed SEC1 key serving as
    /// BOTH spending and viewing key. The key is the SDK-published spending
    /// key from `VECTOR_META_ADDRESS`, so the vectors stay anchored to
    /// externally published key material; expected values are computed with
    /// the implementation and pinned (same pinning strategy as the legacy
    /// pre-switch vectors).
    const SINGLE_KEY_META_ADDRESS: &str =
        "st:eth:0x033404e82cd2a92321d51e13064ec13a0fb0192a9fdaaca1cfb47b37bd27ec1397";

    /// Single-key recipient wallet: spending and viewing private keys are the
    /// SAME key (the SDK-published spending key). This is how a recipient of
    /// a single-key meta-address checks and sweeps: no code path
    /// special-cases it because the math only ever pairs "viewing" for ECDH
    /// with "spending" for the offset, and here both are one key.
    fn single_key_wallet() -> EthereumStealthWallet {
        let private_key = fixed_secret_key(VECTOR_SPENDING_PRIVATE_KEY);
        // Sanity: the pinned meta-address carries exactly this key.
        assert_eq!(
            SINGLE_KEY_META_ADDRESS,
            format!("st:eth:0x{}", encode_public_key(&private_key.public_key()))
        );
        EthereumStealthWallet {
            meta_address: EthereumStealthMetaAddress {
                wallet: "vectors".into(),
                short_name: "eth".into(),
                scheme_id: ETHEREUM_STEALTH_SCHEME_ID,
                stealth_meta_address: SINGLE_KEY_META_ADDRESS.into(),
                spending_public_key_hex: encode_public_key(&private_key.public_key()),
                viewing_public_key_hex: encode_public_key(&private_key.public_key()),
            },
            spending_private_key: private_key.clone(),
            viewing_private_key: private_key,
        }
    }

    fn assert_single_key_vector(convention: StealthHashConvention, vector: &FixedVector) {
        let wallet = single_key_wallet();
        let meta = parse_meta_address(SINGLE_KEY_META_ADDRESS).unwrap();
        // The parse collapses spending == viewing to the same point.
        assert_eq!(meta.spending_public_key, meta.viewing_public_key);
        let mut ephemeral_bytes = [0u8; 32];
        ephemeral_bytes.copy_from_slice(&hex::decode(vector.ephemeral_private_key).unwrap());

        // Payer side: generation from the single-key form is byte-exact.
        let payment = generate_ethereum_stealth_address(
            SINGLE_KEY_META_ADDRESS,
            Some(ephemeral_bytes),
            convention,
        )
        .unwrap();
        assert_eq!(payment.stealth_address, vector.stealth_address);
        assert_eq!(
            payment.ephemeral_public_key_hex,
            vector.ephemeral_public_key
        );
        assert_eq!(payment.view_tag_hex, hex::encode([vector.view_tag]));
        assert_eq!(payment.stealth_hash_convention, convention);

        // Hash level: the exact bytes feeding every derivation.
        let ephemeral_secret = ephemeral_private_key_to_secret(ephemeral_bytes).unwrap();
        let hashed =
            hashed_shared_secret(&ephemeral_secret, &meta.viewing_public_key, convention).unwrap();
        assert_eq!(hex::encode(hashed), vector.hashed_shared_secret);
        assert_eq!(derive_view_tag(&hashed), vector.view_tag);

        // Recipient side: single-convention check and dual-decode both match.
        let check = check_ethereum_stealth_address(
            &wallet,
            vector.stealth_address,
            vector.ephemeral_public_key,
            Some(vector.view_tag),
            convention,
        )
        .unwrap();
        assert!(check.matches);
        assert_eq!(check.stealth_hash_convention, convention);

        let probed = check_ethereum_stealth_address_any(
            &wallet,
            vector.stealth_address,
            vector.ephemeral_public_key,
            Some(vector.view_tag),
            &StealthHashConvention::PROBE_ORDER,
        )
        .unwrap();
        assert!(probed.matches);
        assert_eq!(probed.stealth_hash_convention, convention);

        // Watch-only recipient side collapses the same way: viewing private
        // key == spending private key, spending PUBLIC key == its public
        // half — byte-identical to the full-wallet result.
        let watch_view = single_key_wallet().watch_view();
        let watch_check = check_ethereum_stealth_address_watch_only(
            &watch_view,
            vector.stealth_address,
            vector.ephemeral_public_key,
            Some(vector.view_tag),
            convention,
        )
        .unwrap();
        assert_eq!(watch_check, check);

        // Sweep side: the derived stealth private key is byte-exact and
        // address-verified.
        let (stealth_key, _) = derive_verified_stealth_key(
            &wallet,
            vector.stealth_address,
            vector.ephemeral_public_key,
            Some(vector.view_tag),
            convention,
        )
        .unwrap();
        assert_eq!(
            hex::encode(stealth_key.to_bytes()),
            vector.stealth_private_key
        );
    }

    #[test]
    fn single_key_meta_address_parses_with_collapsed_keys() {
        let meta = parse_meta_address(SINGLE_KEY_META_ADDRESS).unwrap();
        assert_eq!(meta.short_name, "eth");
        assert_eq!(meta.spending_public_key, meta.viewing_public_key);
        // The bare form (no `st:chain:` prefix) parses identically.
        let bare = parse_meta_address(
            "0x033404e82cd2a92321d51e13064ec13a0fb0192a9fdaaca1cfb47b37bd27ec1397",
        )
        .unwrap();
        assert_eq!(bare.spending_public_key, meta.spending_public_key);
        // Neither a truncated dual-key form nor Fluidkey's 64-byte X‖Y
        // encoding is accepted as "single-key".
        for payload in [
            format!("0x{}", "03".repeat(32)), // 32 bytes: neither form
            format!("0x{}", "04".repeat(64)), // Fluidkey X‖Y: unsupported
            format!("0x{}", "02".repeat(34)), // 34 bytes: neither form
        ] {
            assert!(
                matches!(
                    parse_meta_address(&payload),
                    Err(EthereumStealthError::InvalidMetaAddress)
                ),
                "{payload}"
            );
        }
    }

    #[test]
    fn single_key_meta_address_end_to_end_fixed_vectors() {
        let vectors = [
            (
                StealthHashConvention::Compressed33,
                FixedVector {
                    ephemeral_private_key: STANDARD_VECTORS[0].ephemeral_private_key,
                    ephemeral_public_key: STANDARD_VECTORS[0].ephemeral_public_key,
                    hashed_shared_secret: "4284cefcb9194b08aadb2c943104476c414eb5ffb913717ef071f7767c2f3a11",
                    view_tag: 0x42,
                    stealth_address: "0x1bf254d39212f105e98e720269b2821a9bd78415",
                    stealth_private_key: "45e8411b7302cc5e374fb8167db771f04ef169e84e8fa142ad2ad13ee8e7ae67",
                },
            ),
            (
                StealthHashConvention::XOnly32,
                FixedVector {
                    ephemeral_private_key: LEGACY_VECTORS[0].ephemeral_private_key,
                    ephemeral_public_key: LEGACY_VECTORS[0].ephemeral_public_key,
                    hashed_shared_secret: "7476dfdeaad242a8b09c0c78b6ecb7167c967451bab830bd2652c381047f25de",
                    view_tag: 0x74,
                    stealth_address: "0xbab0479fe41ae45b113988489444316f88d8c1f6",
                    stealth_private_key: "77da51fd64bbc3fe3d1097fb039fe19a8a39283a50346080e30b9d4971379a34",
                },
            ),
        ];
        for (convention, vector) in &vectors {
            assert_single_key_vector(*convention, vector);
        }
    }

    /// Watch view built from bare parts — the spending private key never
    /// exists in this test process beyond computing the public half here, so
    /// detection through this view provably needs only the viewing private
    /// key and the spending public key.
    fn vector_watch_view_from_parts() -> EthereumStealthWatchView {
        let viewing_private_key = fixed_secret_key(VECTOR_VIEWING_PRIVATE_KEY);
        let spending_public_key = fixed_secret_key(VECTOR_SPENDING_PRIVATE_KEY).public_key();
        EthereumStealthWatchView {
            meta_address: EthereumStealthMetaAddress {
                wallet: "vectors".into(),
                short_name: "eth".into(),
                scheme_id: ETHEREUM_STEALTH_SCHEME_ID,
                stealth_meta_address: VECTOR_META_ADDRESS.into(),
                spending_public_key_hex: encode_public_key(&spending_public_key),
                viewing_public_key_hex: encode_public_key(&viewing_private_key.public_key()),
            },
            viewing_private_key,
            spending_public_key,
        }
    }

    #[test]
    fn watch_only_check_detects_fixed_vectors_without_spending_private_key() {
        // Detection from the EIP-5564 `checkStealthAddress` key material
        // (viewing private key + spending PUBLIC key) alone: matches every
        // fixed vector on both conventions, with byte-exact derived values.
        let view = vector_watch_view_from_parts();
        for (convention, vectors) in [
            (StealthHashConvention::Compressed33, &STANDARD_VECTORS),
            (StealthHashConvention::XOnly32, &LEGACY_VECTORS),
        ] {
            for vector in vectors {
                let check = check_ethereum_stealth_address_watch_only(
                    &view,
                    vector.stealth_address,
                    vector.ephemeral_public_key,
                    Some(vector.view_tag),
                    convention,
                )
                .unwrap();
                assert!(check.matches);
                assert_eq!(check.derived_stealth_address, vector.stealth_address);
                assert_eq!(check.view_tag_hex, hex::encode([vector.view_tag]));
                assert_eq!(check.stealth_hash_convention, convention);

                // Dual-decode probing finds the payment's actual convention.
                let probed = check_ethereum_stealth_address_any_watch_only(
                    &view,
                    vector.stealth_address,
                    vector.ephemeral_public_key,
                    Some(vector.view_tag),
                    &StealthHashConvention::PROBE_ORDER,
                )
                .unwrap();
                assert!(probed.matches);
                assert_eq!(probed, check);

                // The view-tag prefilter still fails fast on a foreign tag
                // (one that matches neither convention's derived tag).
                let other_tag = hex::decode(
                    check_ethereum_stealth_address_watch_only(
                        &view,
                        vector.stealth_address,
                        vector.ephemeral_public_key,
                        None,
                        convention.other(),
                    )
                    .unwrap()
                    .view_tag_hex,
                )
                .unwrap()[0];
                let foreign_tag = (0u8..=255)
                    .find(|tag| *tag != vector.view_tag && *tag != other_tag)
                    .unwrap();
                let error = check_ethereum_stealth_address_any_watch_only(
                    &view,
                    vector.stealth_address,
                    vector.ephemeral_public_key,
                    Some(foreign_tag),
                    &StealthHashConvention::PROBE_ORDER,
                )
                .unwrap_err();
                assert_eq!(error, EthereumStealthError::ViewTagMismatch);
            }
        }
    }

    #[test]
    fn watch_view_matches_full_wallet_detection_for_derived_wallets() {
        // Same master key + labels: the watch-only derivation yields the same
        // meta-address as the full wallet, and detection results are
        // byte-identical on both conventions.
        let master_key = [19u8; 32];
        let full = derive_sigillum_ethereum_stealth_wallet(&master_key, "treasury", "eth").unwrap();
        let watch =
            derive_watch_only_sigillum_ethereum_stealth_wallet(&master_key, "treasury", "eth")
                .unwrap();
        assert_eq!(watch.meta_address(), full.meta_address());
        assert_eq!(watch.meta_address(), full.watch_view().meta_address());

        for convention in [
            StealthHashConvention::Compressed33,
            StealthHashConvention::XOnly32,
        ] {
            let payment = generate_ethereum_stealth_address(
                &full.meta_address().stealth_meta_address,
                Some([23u8; 32]),
                convention,
            )
            .unwrap();
            let view_tag = hex::decode(&payment.view_tag_hex).unwrap()[0];

            let full_check = check_ethereum_stealth_address_any(
                &full,
                &payment.stealth_address,
                &payment.ephemeral_public_key_hex,
                Some(view_tag),
                &StealthHashConvention::PROBE_ORDER,
            )
            .unwrap();
            let watch_check = check_ethereum_stealth_address_any_watch_only(
                &watch,
                &payment.stealth_address,
                &payment.ephemeral_public_key_hex,
                Some(view_tag),
                &StealthHashConvention::PROBE_ORDER,
            )
            .unwrap();
            assert!(watch_check.matches);
            assert_eq!(watch_check, full_check);
        }
    }

    #[test]
    fn watch_only_derivation_validates_labels_like_full_derivation() {
        assert_eq!(
            derive_watch_only_sigillum_ethereum_stealth_wallet(&[7u8; 32], "  ", "eth")
                .unwrap_err(),
            EthereumStealthError::EmptyWalletLabel
        );
        assert!(
            derive_watch_only_sigillum_ethereum_stealth_wallet(&[7u8; 32], "ops", "bad name")
                .is_err()
        );
    }

    #[test]
    fn conventions_produce_distinct_addresses_for_same_inputs() {
        // The two conventions must never collide for the same inputs: the view
        // tags and stealth addresses differ, which is exactly why dual-decode
        // probing is required to find legacy payments in one scan pass.
        for (standard, legacy) in STANDARD_VECTORS.iter().zip(LEGACY_VECTORS.iter()) {
            assert_eq!(standard.ephemeral_private_key, legacy.ephemeral_private_key);
            assert_ne!(standard.hashed_shared_secret, legacy.hashed_shared_secret);
            assert_ne!(standard.view_tag, legacy.view_tag);
            assert_ne!(standard.stealth_address, legacy.stealth_address);
            assert_ne!(standard.stealth_private_key, legacy.stealth_private_key);
        }
    }

    #[test]
    fn dual_decode_rejects_unknown_announcement_by_view_tag() {
        let wallet = vector_wallet();
        // A view tag that matches neither convention's tag for this ephemeral
        // key must fail fast, preserving the scan prefilter semantics.
        let error = check_ethereum_stealth_address_any(
            &wallet,
            STANDARD_VECTORS[0].stealth_address,
            STANDARD_VECTORS[0].ephemeral_public_key,
            Some(0xff),
            &StealthHashConvention::PROBE_ORDER,
        )
        .unwrap_err();
        assert_eq!(error, EthereumStealthError::ViewTagMismatch);
    }

    #[test]
    fn sdk_view_tag_vector_matches() {
        // Verbatim ScopeLift SDK test vector (src/utils/crypto/test/
        // generateStealthAddress.test.ts): view tag = most significant byte.
        let hashed: [u8; 32] =
            hex::decode("158ce29a3dd0c8dca524e5776c2ba6361c280e013f87eee5eb799a713a939501")
                .unwrap()
                .try_into()
                .unwrap();
        assert_eq!(derive_view_tag(&hashed), 0x15);
    }

    #[test]
    fn convention_strings_roundtrip() {
        for convention in StealthHashConvention::PROBE_ORDER {
            let encoded = convention.as_str();
            assert_eq!(
                encoded.parse::<StealthHashConvention>().unwrap(),
                convention
            );
            assert_eq!(
                serde_json::to_string(&convention).unwrap(),
                format!("\"{encoded}\"")
            );
            assert_eq!(
                serde_json::from_str::<StealthHashConvention>(&format!("\"{encoded}\"")).unwrap(),
                convention
            );
        }
        assert_eq!(StealthHashConvention::STANDARD.as_str(), "compressed33");
        assert_eq!(StealthHashConvention::LEGACY.as_str(), "x32");
        assert!("uncompressed64".parse::<StealthHashConvention>().is_err());
    }

    // ── Metadata SHOULD layouts ──

    /// Byte-format assertion for the EIP-5564 native-token metadata SHOULD
    /// layout (`announce` spec: byte 1 view tag; bytes 2-5 `0xeeeeeeee`;
    /// bytes 6-25 `0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE`; bytes 26-57
    /// the amount of ETH being sent — EIP bytes are 1-indexed).
    #[test]
    fn native_metadata_layout_matches_eip5564_byte_format() {
        let mut amount = [0u8; 32];
        amount[31] = 0x2a;
        let metadata = hex::decode(encode_erc5564_metadata_native(0xab, &amount)).unwrap();

        assert_eq!(metadata.len(), ERC5564_METADATA_LAYOUT_LEN);
        assert_eq!(metadata[0], 0xab);
        assert_eq!(&metadata[1..5], &ERC5564_METADATA_NATIVE_MARKER);
        assert_eq!(
            format!("0x{}", hex::encode(&metadata[5..25])),
            ERC5564_METADATA_NATIVE_SENTINEL_ADDRESS.to_ascii_lowercase()
        );
        assert_eq!(&metadata[25..57], &amount);
    }

    /// Byte-format assertion for the EIP-5564 token metadata SHOULD layout
    /// (byte 1 view tag; bytes 2-5 function identifier — the selector when
    /// available; bytes 6-25 token contract; bytes 26-57 amount/token id).
    #[test]
    fn erc20_metadata_layout_matches_eip5564_byte_format() {
        let token_address = "0x2222222222222222222222222222222222222222";
        let mut amount = [0u8; 32];
        amount[30] = 0x0f;
        amount[31] = 0x42;
        let metadata = hex::decode(
            encode_erc5564_metadata_erc20_transfer(0xcd, token_address, &amount).unwrap(),
        )
        .unwrap();

        assert_eq!(metadata.len(), ERC5564_METADATA_LAYOUT_LEN);
        assert_eq!(metadata[0], 0xcd);
        // keccak256("transfer(address,uint256)")[..4] == 0xa9059cbb.
        assert_eq!(&metadata[1..5], &ERC5564_METADATA_ERC20_TRANSFER_SELECTOR);
        assert_eq!(
            ERC5564_METADATA_ERC20_TRANSFER_SELECTOR,
            Keccak256::digest(b"transfer(address,uint256)")[..4]
        );
        assert_eq!(
            &metadata[5..25],
            &decode_ethereum_address(token_address).unwrap()
        );
        assert_eq!(&metadata[25..57], &amount);
    }

    #[test]
    fn metadata_hints_roundtrip_both_layouts() {
        let mut amount = [7u8; 32];
        amount[0] = 0;
        let native = encode_erc5564_metadata_native(0x11, &amount);
        assert_eq!(
            decode_erc5564_metadata_hints(&hex::decode(native).unwrap()),
            Some(Erc5564MetadataHints::Native { amount_wei: amount })
        );

        let token_address = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let token = encode_erc5564_metadata_erc20_transfer(0x22, token_address, &amount).unwrap();
        assert_eq!(
            decode_erc5564_metadata_hints(&hex::decode(token).unwrap()),
            Some(Erc5564MetadataHints::Token {
                function_selector: ERC5564_METADATA_ERC20_TRANSFER_SELECTOR,
                token_address: token_address.to_string(),
                amount,
            })
        );
    }

    #[test]
    fn metadata_hints_decode_is_defensive_never_errors() {
        // View-tag-only metadata (the minimal form Sigillum emits by default).
        assert_eq!(decode_erc5564_metadata_hints(&[0xab]), None);
        // Empty and truncated layouts.
        assert_eq!(decode_erc5564_metadata_hints(&[]), None);
        assert_eq!(decode_erc5564_metadata_hints(&[0xab; 56]), None);
        // Oversized / unknown trailing layouts.
        assert_eq!(decode_erc5564_metadata_hints(&[0xab; 58]), None);
        assert_eq!(decode_erc5564_metadata_hints(&[0xab; 89]), None);
        // Native marker with a non-sentinel address is an unknown layout, not
        // a native hint.
        let mut bogus_native = [0u8; 57];
        bogus_native[1..5].copy_from_slice(&ERC5564_METADATA_NATIVE_MARKER);
        bogus_native[5] = 0x11;
        assert_eq!(decode_erc5564_metadata_hints(&bogus_native), None);
        // A 57-byte payload with an unrecognized function identifier still
        // parses as a token hint; consumers decide which selectors they act
        // on. The view-tag byte (0xee here) is never interpreted.
        let mut unknown_selector = [0u8; 57];
        unknown_selector[0] = 0xee;
        unknown_selector[1..5].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        unknown_selector[5..25].copy_from_slice(&[0x33; 20]);
        assert_eq!(
            decode_erc5564_metadata_hints(&unknown_selector),
            Some(Erc5564MetadataHints::Token {
                function_selector: [0xde, 0xad, 0xbe, 0xef],
                token_address: format!("0x{}", hex::encode([0x33; 20])),
                amount: [0u8; 32],
            })
        );
    }

    #[test]
    fn announcement_with_layout_metadata_carries_it_into_calldata() {
        let wallet =
            derive_sigillum_ethereum_stealth_wallet(&[9u8; 32], "exchange", "eth").unwrap();
        let payment = generate_ethereum_stealth_address(
            &wallet.meta_address.stealth_meta_address,
            Some([3u8; 32]),
            StealthHashConvention::STANDARD,
        )
        .unwrap();
        let view_tag = hex::decode(&payment.view_tag_hex).unwrap()[0];
        let metadata_hex = encode_erc5564_metadata_erc20_transfer(
            view_tag,
            "0x2222222222222222222222222222222222222222",
            &[0u8; 32],
        )
        .unwrap();

        let announcement =
            build_erc5564_announcement_with_metadata(&payment, &metadata_hex).unwrap();
        assert_eq!(announcement.metadata_hex, metadata_hex);
        let calldata = hex::decode(&announcement.calldata_hex).unwrap();
        // The ABI-encoded metadata tail embeds the 57 hinted bytes verbatim.
        assert!(calldata.len() > 4 + 32 * 4 + 32 + 57);

        // A metadata blob whose first byte is not the payment's view tag is
        // rejected (the EIP mandates the view tag as the first metadata byte).
        let mut wrong_tag = hex::decode(&metadata_hex).unwrap();
        wrong_tag[0] ^= 0xff;
        let error = build_erc5564_announcement_with_metadata(&payment, &hex::encode(wrong_tag))
            .unwrap_err();
        assert_eq!(
            error,
            EthereumStealthError::InvalidAnnouncementField(
                "metadata must include the payment's view tag as its first byte".into()
            )
        );
    }

    // ── Gas sponsor derivation ──

    #[test]
    fn gas_sponsor_derivation_is_deterministic_and_wallet_scoped() {
        let first = derive_sigillum_ethereum_stealth_gas_sponsor(&[31u8; 32], "payments").unwrap();
        let second = derive_sigillum_ethereum_stealth_gas_sponsor(&[31u8; 32], "payments").unwrap();
        assert_eq!(first.sponsor_address(), second.sponsor_address());
        assert_eq!(first.wallet(), "payments");
        assert!(first.sponsor_address().starts_with("0x"));
        assert_eq!(first.sponsor_address().len(), 42);

        let other_wallet =
            derive_sigillum_ethereum_stealth_gas_sponsor(&[31u8; 32], "treasury").unwrap();
        assert_ne!(first.sponsor_address(), other_wallet.sponsor_address());
        let other_master =
            derive_sigillum_ethereum_stealth_gas_sponsor(&[37u8; 32], "payments").unwrap();
        assert_ne!(first.sponsor_address(), other_master.sponsor_address());

        // The sponsor chain is independent of the spend/view chains: the
        // sponsor address must not collide with the stealth wallet's own
        // meta-address-derived keys' addresses.
        let wallet =
            derive_sigillum_ethereum_stealth_wallet(&[31u8; 32], "payments", "eth").unwrap();
        let payment = generate_ethereum_stealth_address(
            &wallet.meta_address.stealth_meta_address,
            Some([41u8; 32]),
            StealthHashConvention::STANDARD,
        )
        .unwrap();
        assert_ne!(first.sponsor_address(), &payment.stealth_address);
    }

    #[test]
    fn gas_sponsor_signing_key_matches_sponsor_address() {
        let sponsor =
            derive_sigillum_ethereum_stealth_gas_sponsor(&[43u8; 32], "payments").unwrap();
        let signing_key = sponsor.signing_key();
        // The signing key's address must be the advertised sponsor address —
        // same defense-in-depth invariant the seed signer enforces.
        let address = crate::ethereum_address_from_signing_key(&signing_key);
        assert_eq!(&address, sponsor.sponsor_address());

        assert_eq!(
            derive_sigillum_ethereum_stealth_gas_sponsor(&[43u8; 32], "  ").unwrap_err(),
            EthereumStealthError::EmptyWalletLabel
        );
    }
}
