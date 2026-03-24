//! Shared utilities for Sigillum crates.
//!
//! Centralizes atomic filesystem writes, KDF helpers, and wrapped key
//! management so that the daemon, CLI, and FIDO2 crates share a single
//! implementation.

use std::io::Write;
use std::path::Path;

use zeroize::Zeroizing;

use crate::VaultError;

// ── Argon2id parameters ─────────────────────────────────────────

/// Argon2id memory cost in KiB (64 MiB). Meets OWASP minimum.
pub const ARGON2_MEMORY_KIB: u32 = 65536;
/// Argon2id time cost (iterations).
pub const ARGON2_TIME_COST: u32 = 3;
/// Argon2id parallelism lanes.
pub const ARGON2_PARALLELISM: u32 = 1;
/// Argon2id output length in bytes.
pub const ARGON2_OUTPUT_LEN: usize = 32;

// ── Atomic write ────────────────────────────────────────────────

/// Atomic write: write to a temp file in the same directory, fsync it, rename,
/// then fsync the parent directory when the platform supports it.
/// This prevents torn writes and makes the rename durable across crashes.
///
/// Temp file names include PID and a random suffix to avoid collisions
/// when multiple threads or processes write to the same directory.
#[must_use = "check the Result to ensure data was persisted"]
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<(), std::io::Error> {
    use rand::RngCore;
    use rand::rngs::OsRng;

    let parent = path.parent().unwrap_or(Path::new("."));
    let suffix: u32 = OsRng.next_u32();
    let tmp_path = parent.join(format!(".tmp_{}_{:08x}", std::process::id(), suffix));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)?;
    file.write_all(data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))?;
    }
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp_path, path)?;
    sync_parent_dir(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> Result<(), std::io::Error> {
    std::fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

// ── KDF helpers ─────────────────────────────────────────────────

/// Derive a wrap key from a passphrase using Argon2id with a fresh random salt.
/// Returns `(wrap_key, salt)`.
#[must_use]
pub fn derive_key_from_passphrase(passphrase: &str) -> (Zeroizing<[u8; 32]>, [u8; 32]) {
    use rand::RngCore;
    use rand::rngs::OsRng;

    let mut salt = [0u8; 32];
    OsRng.fill_bytes(&mut salt);
    let key = derive_key_with_salt(passphrase, &salt);
    (key, salt)
}

/// Derive a wrap key from a passphrase and existing salt using Argon2id.
#[must_use]
pub fn derive_key_with_salt(passphrase: &str, salt: &[u8]) -> Zeroizing<[u8; 32]> {
    use argon2::Argon2;

    let mut key = Zeroizing::new([0u8; 32]);
    let argon2 = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2::Params::new(
            ARGON2_MEMORY_KIB,
            ARGON2_TIME_COST,
            ARGON2_PARALLELISM,
            Some(ARGON2_OUTPUT_LEN),
        )
        .expect("valid Argon2 params"),
    );
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut *key)
        .expect("Argon2id derivation failed");
    key
}

// ── Salt persistence ────────────────────────────────────────────

#[must_use = "check the Result to ensure salt was persisted"]
pub fn save_salt(salt: &[u8; 32], path: &Path) -> Result<(), std::io::Error> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    atomic_write(path, salt)
}

// ── Wrapped key persistence ─────────────────────────────────────

/// Encrypt a master key with a wrap key (AES-256-GCM) and save atomically.
/// Format: `[12-byte nonce || ciphertext || 16-byte auth tag]`.
pub fn save_wrapped_master_key(
    master_key: &[u8; 32],
    wrap_key: &[u8; 32],
    path: &Path,
) -> Result<(), VaultError> {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
    use rand::RngCore;
    use rand::rngs::OsRng;

    let cipher = Aes256Gcm::new_from_slice(wrap_key)
        .map_err(|e| VaultError::Encryption(format!("wrap key init: {e}")))?;
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, master_key.as_ref())
        .map_err(|e| VaultError::Encryption(format!("wrap encryption: {e}")))?;

    let mut output = Vec::with_capacity(12 + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    atomic_write(path, &output)?;
    Ok(())
}

/// Load and decrypt a wrapped master key from disk.
/// Returns `None` if the file doesn't exist, is too short, or decryption fails.
#[must_use]
pub fn load_wrapped_master_key(wrap_key: &[u8; 32], path: &Path) -> Option<Zeroizing<[u8; 32]>> {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

    let data = std::fs::read(path).ok()?;
    if data.len() < 12 {
        return None;
    }
    let (nonce_bytes, ciphertext) = data.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(wrap_key).ok()?;
    let mut plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .ok()?;
    if plaintext.len() != 32 {
        zeroize::Zeroize::zeroize(&mut plaintext);
        return None;
    }
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&plaintext[..32]);
    zeroize::Zeroize::zeroize(&mut plaintext);
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;
    use rand::rngs::OsRng;
    use tempfile::TempDir;

    #[test]
    fn atomic_write_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.dat");
        let data = b"hello world";
        atomic_write(&path, data).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), data);
    }

    #[test]
    fn atomic_write_sets_permissions() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.dat");
        atomic_write(&path, b"secret").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn kdf_deterministic_with_same_salt() {
        let salt = [42u8; 32];
        let k1 = derive_key_with_salt("test-passphrase", &salt);
        let k2 = derive_key_with_salt("test-passphrase", &salt);
        assert_eq!(*k1, *k2);
    }

    #[test]
    fn kdf_different_with_different_salt() {
        let k1 = derive_key_with_salt("test", &[1u8; 32]);
        let k2 = derive_key_with_salt("test", &[2u8; 32]);
        assert_ne!(*k1, *k2);
    }

    #[test]
    fn wrapped_key_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wrapped.enc");
        let mut master = [0u8; 32];
        let mut wrap = [0u8; 32];
        OsRng.fill_bytes(&mut master);
        OsRng.fill_bytes(&mut wrap);

        save_wrapped_master_key(&master, &wrap, &path).unwrap();
        let loaded = load_wrapped_master_key(&wrap, &path).unwrap();
        assert_eq!(*loaded, master);
    }

    #[test]
    fn wrapped_key_wrong_key_fails() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wrapped.enc");
        let mut master = [0u8; 32];
        let mut wrap1 = [0u8; 32];
        let mut wrap2 = [0u8; 32];
        OsRng.fill_bytes(&mut master);
        OsRng.fill_bytes(&mut wrap1);
        OsRng.fill_bytes(&mut wrap2);

        save_wrapped_master_key(&master, &wrap1, &path).unwrap();
        assert!(load_wrapped_master_key(&wrap2, &path).is_none());
    }

    #[test]
    fn save_salt_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("subdir").join("salt.bin");
        let salt = [99u8; 32];
        save_salt(&salt, &path).unwrap();
        let loaded = std::fs::read(&path).unwrap();
        assert_eq!(loaded, salt);
    }
}
