//! Shared helpers for the service layer.
//!
//! Centralizes hex decoding, view-tag parsing, u256 arithmetic, timestamp
//! generation, and domain error mapping so that individual service modules
//! do not duplicate these utilities.

use std::time::{SystemTime, UNIX_EPOCH};

use sigillum_core::{EthereumStealthError, EthereumXpubError};

use super::{ServiceError, ServiceResult};

// ── Hex decoding ─────────────────────────────────────────────────

/// Decode a hex string, returning a `ServiceError::bad_request` on failure.
pub(crate) fn decode_hex(value: &str, label: &str) -> ServiceResult<Vec<u8>> {
    hex::decode(value)
        .map_err(|error| ServiceError::bad_request(format!("Invalid {label} encoding: {error}")))
}

/// Decode an optional hex string.
pub(crate) fn decode_optional_hex(
    value: Option<&str>,
    label: &str,
) -> ServiceResult<Option<Vec<u8>>> {
    value.map(|value| decode_hex(value, label)).transpose()
}

/// Decode a hex string into a fixed-size byte array.
pub(crate) fn decode_fixed_hex<const N: usize>(value: &str, label: &str) -> ServiceResult<[u8; N]> {
    let bytes = decode_hex(value, label)?;
    if bytes.len() != N {
        return Err(ServiceError::bad_request(format!(
            "{label} has wrong length: expected {N}, got {}",
            bytes.len()
        )));
    }
    let mut out = [0u8; N];
    out.copy_from_slice(&bytes);
    Ok(out)
}

/// Decode an optional hex-encoded view tag into a single byte.
pub(crate) fn decode_optional_view_tag(value: Option<&str>) -> ServiceResult<Option<u8>> {
    value
        .map(|value| {
            let bytes = decode_fixed_hex::<1>(value, "view_tag")?;
            Ok(bytes[0])
        })
        .transpose()
}

// ── Ethereum stealth error mapping ───────────────────────────────

/// Map an `EthereumStealthError` into the appropriate HTTP-level
/// `ServiceError`, keeping input-validation errors as 400 Bad Request,
/// cryptographic mismatches as 401 Unauthorized, and internal failures
/// as 500 Internal Server Error.
pub(crate) fn map_wallet_error(error: EthereumStealthError) -> ServiceError {
    match error {
        EthereumStealthError::EmptyWalletLabel
        | EthereumStealthError::InvalidMetaAddress
        | EthereumStealthError::InvalidShortName
        | EthereumStealthError::InvalidKeyMaterial
        | EthereumStealthError::InvalidDigestLength
        | EthereumStealthError::InvalidEthereumAddress
        | EthereumStealthError::InvalidAnnouncementField(_)
        | EthereumStealthError::InvalidQuantity(_)
        | EthereumStealthError::InvalidFeeConfiguration => {
            ServiceError::bad_request(error.to_string())
        }
        EthereumStealthError::ViewTagMismatch | EthereumStealthError::AddressMismatch => {
            ServiceError::unauthorized(error.to_string())
        }
        EthereumStealthError::Signing(_) => ServiceError::internal(error.to_string()),
    }
}

/// Map an `EthereumXpubError` into the appropriate HTTP-level `ServiceError`.
pub(crate) fn map_xpub_error(error: EthereumXpubError) -> ServiceError {
    match error {
        EthereumXpubError::InvalidProjectAccount
        | EthereumXpubError::InvalidReceiveIndex
        | EthereumXpubError::InvalidReceiveBranchXpub
        | EthereumXpubError::InvalidAccountBranchXpub
        | EthereumXpubError::InvalidImportedXpub
        | EthereumXpubError::InvalidDerivationPath
        | EthereumXpubError::XpubPathDepthMismatch
        | EthereumXpubError::InvalidMnemonic
        | EthereumXpubError::InvalidMnemonicWordCount => {
            ServiceError::bad_request(error.to_string())
        }
        EthereumXpubError::InvalidKeyMaterial => ServiceError::internal(error.to_string()),
    }
}

// ── u256 arithmetic ──────────────────────────────────────────────

/// Compare two big-endian 256-bit unsigned integers.
pub(crate) fn compare_u256(left: &[u8; 32], right: &[u8; 32]) -> std::cmp::Ordering {
    left.as_slice().cmp(right.as_slice())
}

/// Check whether a big-endian 256-bit unsigned integer is zero.
pub(crate) fn is_zero_u256(value: &[u8; 32]) -> bool {
    value.iter().all(|byte| *byte == 0)
}

/// Multiply a big-endian 256-bit unsigned integer by a `u64` factor.
/// Silently wraps on overflow (matching EVM uint256 semantics).
pub(crate) fn multiply_u256_u64(value: &[u8; 32], factor: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut carry = 0u128;
    for (chunk_in, chunk_out) in value.rchunks_exact(8).zip(out.rchunks_exact_mut(8)) {
        let limb = u64::from_be_bytes(chunk_in.try_into().expect("u64 limb"));
        let wide = (limb as u128) * (factor as u128) + carry;
        chunk_out.copy_from_slice(&(wide as u64).to_be_bytes());
        carry = wide >> 64;
    }
    out
}

/// Subtract two big-endian 256-bit unsigned integers (`left - right`).
/// Silently wraps on underflow (matching EVM uint256 semantics).
pub(crate) fn subtract_u256(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut borrow = 0u16;
    for (idx, (l, r)) in left.iter().zip(right.iter()).enumerate().rev() {
        let lhs = *l as i16 - borrow as i16;
        let rhs = *r as i16;
        if lhs >= rhs {
            out[idx] = (lhs - rhs) as u8;
            borrow = 0;
        } else {
            out[idx] = (lhs + 256 - rhs) as u8;
            borrow = 1;
        }
    }
    out
}

// ── Validation ───────────────────────────────────────────────────

/// Minimum acceptable passphrase length.
const MIN_PASSPHRASE_LEN: usize = 8;

/// Validate that a passphrase meets the minimum length requirement.
///
/// Returns `Err(ServiceError::bad_request)` if the passphrase is too short.
pub(crate) fn require_valid_passphrase(passphrase: &str) -> ServiceResult<()> {
    if passphrase.len() < MIN_PASSPHRASE_LEN {
        return Err(ServiceError::bad_request(format!(
            "Passphrase must be at least {MIN_PASSPHRASE_LEN} characters."
        )));
    }
    Ok(())
}

// ── Timestamps and identifiers ───────────────────────────────────

/// Current UNIX timestamp in seconds.
pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Generate a random 128-bit hex identifier suitable for queue jobs,
/// deposit records, and similar opaque ids.
pub(crate) fn random_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
