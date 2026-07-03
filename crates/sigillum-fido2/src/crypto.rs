//! Cryptographic primitives for FIDO2 hardware key integration.
//!
//! This module provides the core cryptographic operations for splitting, encrypting, and
//! reconstructing secrets using FIDO2 hardware keys as escrow devices. Key operations include:
//!
//! - **Shamir Secret Sharing** (via `blahaj` crate): Split a 32-byte master key into
//!   threshold-of-N shards (e.g., 2-of-3).
//! - **Shard Encryption**: AES-256-GCM encryption of shards under an hmac-secret key
//!   derived from the hardware key.
//! - **Tagged Shards**: Embed compartment ID (4 bytes LE) in the ciphertext for deniable
//!   discovery — an observer cannot determine which compartment a shard belongs to without
//!   the correct hmac-secret.
//! - **Compartment Metadata**: Encrypt metadata with fixed-size padding to hide label length
//!   and prevent side-channel attacks.
//! - **Dummy Files**: Generate random-looking padding entries indistinguishable from real
//!   encrypted shards.
//!
//! ## Design Rationale
//!
//! The scheme achieves deniability by storing only encrypted shards and metadata. The compartment
//! definitions themselves are never written to disk — they are recovered at unlock time by
//! attempting to decrypt each shard with the derived hmac-secret.
//!
//! ## Key Invariants
//!
//! - **Shard Format**: `[12-byte nonce || ciphertext || 16-byte auth tag]`
//! - **Tagged Shard Plaintext**: `[4-byte compartment ID (LE) || shard data]`
//! - **Meta Padding**: All meta.enc files are `12 + 128 + 16 = 156` bytes (fixed size).
//! - **Dummy Shards**: Indistinguishable from real shards; decryption with wrong key fails
//!   at AEAD tag verification.
//! - **Deterministic Salt**: `application_salt()` is constant per vault, ensuring that the same
//!   credential always produces the same hmac-secret across different unlock attempts.

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::config::CompartmentMeta;
use crate::error::Fido2Error;

/// Deterministic application salt for hmac-secret requests.
///
/// Returns a constant 32-byte salt derived from a fixed string. This ensures that the same
/// hardware credential always produces the same hmac-secret across different vault unlocks,
/// enabling deterministic shard decryption.
pub fn application_salt() -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"sigillum_vault_salt_v1");
    hasher.finalize().into()
}

/// Encrypt a Shamir shard with a 32-byte hmac-secret key via AES-256-GCM.
///
/// Returns the encrypted shard as `[12-byte nonce || ciphertext || 16-byte auth tag]`.
/// The nonce is randomly generated; ciphertext includes the authentication tag.
#[must_use = "check the Result for encryption errors"]
pub fn encrypt_shard(hmac_key: &[u8; 32], shard: &[u8]) -> Result<Vec<u8>, Fido2Error> {
    let cipher = Aes256Gcm::new_from_slice(hmac_key)
        .map_err(|e| Fido2Error::ShardEncryption(e.to_string()))?;
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, shard)
        .map_err(|e| Fido2Error::ShardEncryption(e.to_string()))?;
    let mut output = Vec::with_capacity(12 + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

/// Decrypt a Shamir shard with a 32-byte hmac-secret key via AES-256-GCM.
///
/// Expects input as `[12-byte nonce || ciphertext || 16-byte auth tag]`.
/// Returns the decrypted plaintext on success, or an error if the nonce is invalid,
/// the ciphertext is corrupted, or the hmac-secret is incorrect.
#[must_use = "check the Result for decryption errors"]
pub fn decrypt_shard(hmac_key: &[u8; 32], encrypted: &[u8]) -> Result<Vec<u8>, Fido2Error> {
    if encrypted.len() < 12 {
        return Err(Fido2Error::ShardDecryption(
            "encrypted shard too short".into(),
        ));
    }
    let (nonce_bytes, ciphertext) = encrypted.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(hmac_key)
        .map_err(|e| Fido2Error::ShardDecryption(e.to_string()))?;
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|e| Fido2Error::ShardDecryption(format!("wrong key or corrupted: {e}")))
}

/// Encrypt a Shamir shard with the compartment ID embedded in the plaintext.
///
/// Creates a tagged shard with compartment metadata encoded as:
/// `AES-256-GCM(hmac_key, [4B LE compartment ID || shard_data])`.
///
/// The compartment ID is only discoverable after successful decryption, providing deniable
/// storage: an observer cannot determine which compartment a shard belongs to without the
/// correct hmac-secret.
pub fn encrypt_shard_tagged(
    hmac_key: &[u8; 32],
    compartment_id: usize,
    shard: &[u8],
) -> Result<Vec<u8>, Fido2Error> {
    let comp_id_u32: u32 = compartment_id.try_into().map_err(|_| {
        Fido2Error::Config(format!("compartment_id {compartment_id} exceeds u32::MAX"))
    })?;
    let mut plaintext = Vec::with_capacity(4 + shard.len());
    plaintext.extend_from_slice(&comp_id_u32.to_le_bytes());
    plaintext.extend_from_slice(shard);
    encrypt_shard(hmac_key, &plaintext)
}

/// Decrypt a tagged shard, returning `(compartment_id, shard_data)`.
///
/// Extracts the compartment ID and shard data from an encrypted blob created by
/// `encrypt_shard_tagged()`. Fails with a decryption error when the hmac-secret is incorrect
/// or the shard is corrupted (including dummy padding entries that fail AEAD tag verification).
pub fn decrypt_shard_tagged(
    hmac_key: &[u8; 32],
    encrypted: &[u8],
) -> Result<(usize, Vec<u8>), Fido2Error> {
    let plaintext = decrypt_shard(hmac_key, encrypted)?;
    if plaintext.len() < 4 {
        return Err(Fido2Error::ShardDecryption("tagged shard too short".into()));
    }
    let comp_id = u32::from_le_bytes(
        plaintext[..4]
            .try_into()
            .map_err(|_| Fido2Error::ShardDecryption("tagged shard header read failed".into()))?,
    ) as usize;
    Ok((comp_id, plaintext[4..].to_vec()))
}

/// Fixed plaintext size for meta.enc. All meta files (real and dummy) are the
/// same encrypted size: 12 (nonce) + META_PADDED_LEN + 16 (tag) = 156 bytes.
/// This prevents label-length side-channel leaks.
const META_PADDED_LEN: usize = 128;

/// Encrypt compartment metadata with a master key for storage as `meta.enc`.
///
/// Serializes the metadata to JSON, zero-pads it to a fixed size (`META_PADDED_LEN` = 128 bytes)
/// to hide label length from observers, then encrypts with AES-256-GCM.
/// The result is always 156 bytes (12 nonce + 128 padded plaintext + 16 auth tag),
/// preventing side-channel leaks about compartment label sizes.
pub fn encrypt_compartment_meta(
    master_key: &[u8; 32],
    meta: &CompartmentMeta,
) -> Result<Vec<u8>, Fido2Error> {
    let json = serde_json::to_vec(meta)
        .map_err(|e| Fido2Error::ShardEncryption(format!("serialize meta: {e}")))?;
    if json.len() > META_PADDED_LEN {
        return Err(Fido2Error::ShardEncryption(format!(
            "meta JSON too large: {} bytes (max {})",
            json.len(),
            META_PADDED_LEN
        )));
    }
    let mut padded = vec![0u8; META_PADDED_LEN];
    padded[..json.len()].copy_from_slice(&json);
    encrypt_shard(master_key, &padded)
}

/// Decrypt compartment metadata from `meta.enc` contents.
///
/// Decrypts with AES-256-GCM and strips the zero-padding added during encryption,
/// then deserializes the JSON metadata. Returns an error if the master key is incorrect
/// or the ciphertext is corrupted.
pub fn decrypt_compartment_meta(
    master_key: &[u8; 32],
    encrypted: &[u8],
) -> Result<CompartmentMeta, Fido2Error> {
    let padded = decrypt_shard(master_key, encrypted)?;
    // Find the end of JSON by trimming trailing zero-padding
    let end = padded
        .iter()
        .rposition(|&b| b != 0)
        .map(|i| i + 1)
        .unwrap_or(0);
    serde_json::from_slice(&padded[..end])
        .map_err(|e| Fido2Error::ShardDecryption(format!("deserialize meta: {e}")))
}

/// Write random bytes to a file, simulating encrypted content for dummy compartment padding.
///
/// Generates a file at `path` with random content sized between `min_size` and `max_size` bytes.
/// Indistinguishable from actual encrypted files, providing deniability padding for the vault.
pub fn generate_dummy_file(
    path: &std::path::Path,
    min_size: usize,
    max_size: usize,
) -> Result<(), Fido2Error> {
    let size = if min_size == max_size {
        min_size
    } else {
        use rand::Rng;
        OsRng.gen_range(min_size..=max_size)
    };
    let mut buf = vec![0u8; size];
    OsRng.fill_bytes(&mut buf);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Fido2Error::Config(format!("create dir: {e}")))?;
    }
    std::fs::write(path, &buf).map_err(|e| Fido2Error::Config(format!("write dummy: {e}")))?;
    Ok(())
}

/// Split a 32-byte master key into `total` shards with a `threshold`-of-N Shamir scheme.
///
/// Uses the `blahaj` crate to create Shamir shares. Any `threshold` shards can reconstruct
/// the original key; fewer than `threshold` reveal no information.
/// Both `threshold` and `total` must be in range 1..=255.
pub fn split_master_key(
    master_key: &[u8; 32],
    threshold: usize,
    total: usize,
) -> Result<Vec<Vec<u8>>, Fido2Error> {
    if threshold == 0 || threshold > 255 {
        return Err(Fido2Error::ShamirFailed(format!(
            "threshold must be 1..=255, got {threshold}"
        )));
    }
    if total == 0 || total > 255 {
        return Err(Fido2Error::ShamirFailed(format!(
            "total shares must be 1..=255, got {total}"
        )));
    }
    use blahaj::Sharks;
    let sharks = Sharks(threshold as u8);
    let dealer = sharks.dealer(master_key);
    let shares: Vec<Vec<u8>> = dealer.take(total).map(|s| Vec::from(&s)).collect();
    Ok(shares)
}

/// Reconstruct a 32-byte master key from shards.
///
/// Uses Lagrange interpolation with the provided shards as the threshold value
/// (i.e., `shards.len()` shards are required). Both the number of shards and their validity
/// are validated. The reconstructed key is wrapped in `Zeroizing<[u8;32]>` to ensure
/// memory is securely wiped on drop.
///
/// **Important**: Callers MUST verify the reconstructed key by attempting to decrypt
/// compartment metadata or other encrypted data, as this function cannot detect if the shards
/// are invalid (e.g., from wrong hardware keys).
#[must_use = "check the Result and verify the reconstructed key"]
pub fn reconstruct_master_key(shards: &[Vec<u8>]) -> Result<Zeroizing<[u8; 32]>, Fido2Error> {
    use blahaj::{Share, Sharks};
    if shards.is_empty() || shards.len() > 255 {
        return Err(Fido2Error::ShamirFailed(format!(
            "shard count must be 1..=255, got {}",
            shards.len()
        )));
    }
    let sharks = Sharks(shards.len() as u8);
    let shares: Vec<Share> = shards
        .iter()
        .map(|s| Share::try_from(s.as_slice()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Fido2Error::ShamirFailed(format!("invalid shard: {e}")))?;
    let mut secret = sharks
        .recover(&shares)
        .map_err(|e| Fido2Error::ShamirFailed(format!("reconstruction failed: {e}")))?;
    if secret.len() != 32 {
        zeroize::Zeroize::zeroize(&mut secret);
        return Err(Fido2Error::ShamirFailed(format!(
            "reconstructed key has wrong length: expected 32, got {} bytes",
            secret.len()
        )));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&secret[..32]);
    zeroize::Zeroize::zeroize(&mut secret);
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shard_encrypt_decrypt_roundtrip() {
        let mut hmac_key = [0u8; 32];
        OsRng.fill_bytes(&mut hmac_key);
        let shard = b"this is a test shard payload";

        let encrypted = encrypt_shard(&hmac_key, shard).unwrap();
        let decrypted = decrypt_shard(&hmac_key, &encrypted).unwrap();
        assert_eq!(decrypted, shard);
    }

    #[test]
    fn shard_wrong_key_fails() {
        let mut key1 = [0u8; 32];
        let mut key2 = [0u8; 32];
        OsRng.fill_bytes(&mut key1);
        OsRng.fill_bytes(&mut key2);

        let encrypted = encrypt_shard(&key1, b"secret").unwrap();
        assert!(decrypt_shard(&key2, &encrypted).is_err());
    }

    #[test]
    fn shard_too_short_fails() {
        let key = [0u8; 32];
        assert!(decrypt_shard(&key, &[0u8; 5]).is_err());
    }

    #[test]
    fn shamir_split_reconstruct_1_of_1() {
        let mut master = [0u8; 32];
        OsRng.fill_bytes(&mut master);

        let shards = split_master_key(&master, 1, 1).unwrap();
        assert_eq!(shards.len(), 1);

        let reconstructed = reconstruct_master_key(&shards).unwrap();
        assert_eq!(*reconstructed, master);
    }

    #[test]
    fn shamir_split_reconstruct_2_of_3() {
        let mut master = [0u8; 32];
        OsRng.fill_bytes(&mut master);

        let shards = split_master_key(&master, 2, 3).unwrap();
        assert_eq!(shards.len(), 3);

        // Any 2 of 3 should work
        let r1 = reconstruct_master_key(&shards[0..2]).unwrap();
        assert_eq!(*r1, master);

        let r2 = reconstruct_master_key(&shards[1..3]).unwrap();
        assert_eq!(*r2, master);

        let r3 = reconstruct_master_key(&[shards[0].clone(), shards[2].clone()]).unwrap();
        assert_eq!(*r3, master);
    }

    #[test]
    fn shamir_split_reconstruct_3_of_5() {
        let mut master = [0u8; 32];
        OsRng.fill_bytes(&mut master);

        let shards = split_master_key(&master, 3, 5).unwrap();
        assert_eq!(shards.len(), 5);

        let r = reconstruct_master_key(&shards[0..3]).unwrap();
        assert_eq!(*r, master);

        let r2 = reconstruct_master_key(&shards[2..5]).unwrap();
        assert_eq!(*r2, master);
    }

    #[test]
    fn application_salt_is_deterministic() {
        assert_eq!(application_salt(), application_salt());
    }

    #[test]
    fn tagged_shard_roundtrip() {
        let mut hmac_key = [0u8; 32];
        OsRng.fill_bytes(&mut hmac_key);
        let shard = b"test shard data for compartment";
        let comp_id = 42usize;

        let encrypted = encrypt_shard_tagged(&hmac_key, comp_id, shard).unwrap();
        let (dec_id, dec_data) = decrypt_shard_tagged(&hmac_key, &encrypted).unwrap();
        assert_eq!(dec_id, comp_id);
        assert_eq!(dec_data, shard);
    }

    #[test]
    fn tagged_shard_wrong_key_fails() {
        let mut key1 = [0u8; 32];
        let mut key2 = [0u8; 32];
        OsRng.fill_bytes(&mut key1);
        OsRng.fill_bytes(&mut key2);

        let encrypted = encrypt_shard_tagged(&key1, 5, b"data").unwrap();
        assert!(decrypt_shard_tagged(&key2, &encrypted).is_err());
    }

    #[test]
    fn compartment_meta_encrypt_decrypt() {
        let mut master = [0u8; 32];
        OsRng.fill_bytes(&mut master);

        let meta = CompartmentMeta {
            id: 7,
            label: "legacy".into(),
            threshold: 3,
            passphrase_mode: Some("wrapped".into()),
        };

        let encrypted = encrypt_compartment_meta(&master, &meta).unwrap();
        let decrypted = decrypt_compartment_meta(&master, &encrypted).unwrap();
        assert_eq!(decrypted, meta);
    }

    #[test]
    fn compartment_meta_wrong_key_fails() {
        let mut key1 = [0u8; 32];
        let mut key2 = [0u8; 32];
        OsRng.fill_bytes(&mut key1);
        OsRng.fill_bytes(&mut key2);

        let meta = CompartmentMeta {
            id: 0,
            label: "test".into(),
            threshold: 1,
            passphrase_mode: None,
        };

        let encrypted = encrypt_compartment_meta(&key1, &meta).unwrap();
        assert!(decrypt_compartment_meta(&key2, &encrypted).is_err());
    }

    #[test]
    fn dummy_file_is_random() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("dummy.enc");
        generate_dummy_file(&path, 100, 200).unwrap();
        let data = std::fs::read(&path).unwrap();
        assert!(data.len() >= 100 && data.len() <= 200);
    }

    #[test]
    fn full_flow_encrypt_split_reconstruct_decrypt() {
        // Simulate: master key split into 2-of-3, each shard encrypted with a different hmac key
        let mut master = [0u8; 32];
        OsRng.fill_bytes(&mut master);

        let shards = split_master_key(&master, 2, 3).unwrap();

        // Each key has a different hmac-secret output
        let mut hmac_keys: Vec<[u8; 32]> = Vec::new();
        for _ in 0..3 {
            let mut k = [0u8; 32];
            OsRng.fill_bytes(&mut k);
            hmac_keys.push(k);
        }

        // Encrypt each shard
        let encrypted: Vec<Vec<u8>> = shards
            .iter()
            .zip(hmac_keys.iter())
            .map(|(shard, hmac)| encrypt_shard(hmac, shard).unwrap())
            .collect();

        // Decrypt any 2 and reconstruct
        let shard0 = decrypt_shard(&hmac_keys[0], &encrypted[0]).unwrap();
        let shard2 = decrypt_shard(&hmac_keys[2], &encrypted[2]).unwrap();

        let reconstructed = reconstruct_master_key(&[shard0, shard2]).unwrap();
        assert_eq!(*reconstructed, master);
    }
}
