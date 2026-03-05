use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::Fido2Error;

/// Deterministic application salt for hmac-secret requests.
/// Same salt = same hmac-secret output per credential = deterministic shard decryption key.
pub fn application_salt() -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"sigillum_vault_salt_v1");
    hasher.finalize().into()
}

/// Encrypt a Shamir shard with a 32-byte hmac-secret key via AES-256-GCM.
/// Output format: `[12-byte nonce || ciphertext || 16-byte auth tag]`
pub fn encrypt_shard(hmac_key: &[u8; 32], shard: &[u8]) -> Result<Vec<u8>, Fido2Error> {
    let cipher =
        Aes256Gcm::new_from_slice(hmac_key).map_err(|e| Fido2Error::ShardEncryption(e.to_string()))?;
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
/// Input format: `[12-byte nonce || ciphertext || 16-byte auth tag]`
pub fn decrypt_shard(hmac_key: &[u8; 32], encrypted: &[u8]) -> Result<Vec<u8>, Fido2Error> {
    if encrypted.len() < 12 {
        return Err(Fido2Error::ShardDecryption(
            "encrypted shard too short".into(),
        ));
    }
    let (nonce_bytes, ciphertext) = encrypted.split_at(12);
    let cipher =
        Aes256Gcm::new_from_slice(hmac_key).map_err(|e| Fido2Error::ShardDecryption(e.to_string()))?;
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|e| Fido2Error::ShardDecryption(format!("wrong key or corrupted: {e}")))
}

/// Split a 32-byte master key into `total` shards with a `threshold`-of-N scheme.
pub fn split_master_key(
    master_key: &[u8; 32],
    threshold: usize,
    total: usize,
) -> Result<Vec<Vec<u8>>, Fido2Error> {
    use sharks::Sharks;
    let sharks = Sharks(threshold as u8);
    let dealer = sharks.dealer(master_key);
    let shares: Vec<Vec<u8>> = dealer.take(total).map(|s| Vec::from(&s)).collect();
    Ok(shares)
}

/// Reconstruct a 32-byte master key from `threshold` shards.
pub fn reconstruct_master_key(shards: &[Vec<u8>]) -> Result<Zeroizing<[u8; 32]>, Fido2Error> {
    use sharks::{Share, Sharks};
    let sharks = Sharks(shards.len() as u8);
    let shares: Vec<Share> = shards
        .iter()
        .map(|s| Share::try_from(s.as_slice()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Fido2Error::ShamirFailed(format!("invalid shard: {e}")))?;
    let secret = sharks
        .recover(&shares)
        .map_err(|e| Fido2Error::ShamirFailed(format!("reconstruction failed: {e}")))?;
    if secret.len() < 32 {
        return Err(Fido2Error::ShamirFailed(format!(
            "reconstructed key too short: {} bytes",
            secret.len()
        )));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&secret[..32]);
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
