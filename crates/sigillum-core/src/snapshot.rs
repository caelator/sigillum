//! Encrypted vault snapshots for backup and restore with crash-safe recovery.
//!
//! This module provides passphrase-protected backup and restore functionality for the entire
//! vault directory tree. Snapshots use a two-layer encryption scheme:
//! - **Archive format**: Collect all files in the vault into a JSON manifest with hex-encoded content.
//! - **Encryption**: Encrypt the archive with AES-256-GCM using a key derived from the passphrase
//!   via Argon2id KDF with a random salt.
//!
//! ## Design Rationale
//!
//! The snapshot mechanism allows users to back up the entire vault state with a passphrase alone,
//! without exposing the vault's master key. The KDF ensures that weak passphrases are resistant
//! to brute-force attacks.
//!
//! ## Crash-Safe Recovery
//!
//! The restore process uses atomic directory swaps to ensure consistency:
//! 1. Create a `.sigillum.restoring` staging directory with new content.
//! 2. Atomically rename the current vault to `.sigillum.rollback`.
//! 3. Atomically rename the staging directory to the vault path.
//! 4. Delete the rollback directory.
//!
//! If a crash occurs during steps 1-3, the next startup calls `recover_restore_layout()` to:
//! - Complete step 4 if needed (both base and rollback exist).
//! - Restore from rollback if base is missing (steps 3-4 failed).
//! - Adopt staging if only staging exists (steps 2-3 failed).
//!
//! ## Path Traversal Protection
//!
//! `sanitize_relative_path()` rejects any paths with `..` or absolute components to prevent
//! archive entries from escaping the vault directory during restore.

use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::VaultError;
use crate::utils::{atomic_write, derive_key_with_salt};

const SNAPSHOT_VERSION: u32 = 1;

// ── Type Definitions & Metadata ────────────────────────────────────────────

/// Summary metadata for a snapshot: creation timestamp, file count, and total data size.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotSummary {
    /// Unix timestamp when the snapshot was created.
    pub created_at_unix: u64,
    /// Number of files included in the snapshot.
    pub file_count: usize,
    /// Total unencrypted data size in bytes.
    pub total_bytes: usize,
}

#[derive(Serialize, Deserialize)]
struct SnapshotArchive {
    version: u32,
    created_at_unix: u64,
    entries: Vec<SnapshotEntry>,
}

#[derive(Serialize, Deserialize)]
struct SnapshotEntry {
    path: String,
    data_hex: String,
}

#[derive(Serialize, Deserialize)]
struct EncryptedSnapshot {
    version: u32,
    created_at_unix: u64,
    salt_hex: String,
    nonce_hex: String,
    ciphertext_hex: String,
}

// ── Export & Restore API ───────────────────────────────────────────────────

/// Export the vault directory tree as a passphrase-encrypted snapshot.
///
/// Recursively collects all files from `base_dir`, serializes them into a JSON archive,
/// derives a key from the passphrase using Argon2id, and encrypts the archive with AES-256-GCM.
///
/// Returns the encrypted snapshot bytes and a summary of the export.
pub fn export_encrypted_snapshot(
    base_dir: &Path,
    passphrase: &str,
) -> Result<(Vec<u8>, SnapshotSummary), VaultError> {
    if !base_dir.exists() {
        return Err(VaultError::NotInitialized);
    }

    let entries = collect_entries(base_dir, base_dir)?;
    let created_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| VaultError::Other(format!("system time before unix epoch: {e}")))?
        .as_secs();
    let total_bytes = entries.iter().map(|entry| entry.data_hex.len() / 2).sum();

    let archive = SnapshotArchive {
        version: SNAPSHOT_VERSION,
        created_at_unix,
        entries,
    };
    let plaintext = Zeroizing::new(serde_json::to_vec(&archive)?);

    let mut salt = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    let wrap_key = derive_key_with_salt(passphrase, &salt);
    let cipher = Aes256Gcm::new_from_slice(&wrap_key[..])
        .map_err(|e| VaultError::Encryption(format!("snapshot key init: {e}")))?;

    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|e| VaultError::Encryption(format!("snapshot encrypt: {e}")))?;

    let envelope = EncryptedSnapshot {
        version: SNAPSHOT_VERSION,
        created_at_unix,
        salt_hex: hex::encode(salt),
        nonce_hex: hex::encode(nonce_bytes),
        ciphertext_hex: hex::encode(ciphertext),
    };
    let snapshot_bytes = serde_json::to_vec_pretty(&envelope)?;

    Ok((
        snapshot_bytes,
        SnapshotSummary {
            created_at_unix,
            file_count: archive.entries.len(),
            total_bytes,
        },
    ))
}

/// Restore the vault from an encrypted snapshot using crash-safe atomic swaps.
///
/// Decrypts the snapshot using the passphrase, validates paths for traversal attacks,
/// restores all files to a staging directory, then atomically swaps it into place.
/// If the process crashes mid-restore, call `recover_snapshot_restore()` on next startup
/// to repair the state.
pub fn restore_encrypted_snapshot(
    base_dir: &Path,
    passphrase: &str,
    snapshot_bytes: &[u8],
) -> Result<SnapshotSummary, VaultError> {
    let (archive, summary) = decode_encrypted_snapshot(passphrase, snapshot_bytes)?;
    restore_archive(base_dir, &archive)?;
    Ok(summary)
}

/// Inspect a snapshot without restoring, returning its summary metadata.
///
/// Decrypts the snapshot and extracts file count and total size without modifying the vault.
pub fn inspect_encrypted_snapshot(
    passphrase: &str,
    snapshot_bytes: &[u8],
) -> Result<SnapshotSummary, VaultError> {
    let (_, summary) = decode_encrypted_snapshot(passphrase, snapshot_bytes)?;
    Ok(summary)
}

// ── Recovery & Crash Safety ───────────────────────────────────────────────

/// Recover a partial restore on startup after an unexpected crash or shutdown.
///
/// Cleans up intermediate `.sigillum.restoring` and `.sigillum.rollback` directories
/// to restore a consistent state. Returns `true` if recovery was needed, `false` otherwise.
pub fn recover_snapshot_restore(base_dir: &Path) -> Result<bool, VaultError> {
    recover_restore_layout(base_dir)
}

// ── File Collection & Entry Building ───────────────────────────────────────

fn collect_entries(root: &Path, dir: &Path) -> Result<Vec<SnapshotEntry>, VaultError> {
    let mut entries = Vec::new();
    let mut dir_entries: Vec<_> = std::fs::read_dir(dir)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(VaultError::Io)?;
    dir_entries.sort_by_key(|entry| entry.file_name());

    for entry in dir_entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;

        if metadata.file_type().is_symlink() {
            return Err(VaultError::Other(format!(
                "refusing to snapshot symlink: {}",
                path.display()
            )));
        }

        if metadata.is_dir() {
            entries.extend(collect_entries(root, &path)?);
            continue;
        }

        if metadata.is_file() {
            let rel = path
                .strip_prefix(root)
                .map_err(|e| VaultError::Other(format!("strip prefix: {e}")))?;
            let rel = sanitize_relative_path(rel)?;
            let mut data = std::fs::read(&path)?;
            let data_hex = hex::encode(&data);
            data.zeroize();
            entries.push(SnapshotEntry {
                path: rel.to_string_lossy().into_owned(),
                data_hex,
            });
            continue;
        }

        return Err(VaultError::Other(format!(
            "unsupported file type in snapshot: {}",
            path.display()
        )));
    }

    Ok(entries)
}

// ── Archive Restoration & Atomic Swap ──────────────────────────────────────

fn restore_archive(base_dir: &Path, archive: &SnapshotArchive) -> Result<(), VaultError> {
    let parent = base_dir.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    recover_restore_layout(base_dir)?;

    let staging = snapshot_temp_path(base_dir, "restoring");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;

    for entry in &archive.entries {
        let rel = sanitize_relative_path(Path::new(&entry.path))?;
        let data = hex::decode(&entry.data_hex).map_err(|e| {
            VaultError::Serialization(serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("decode {}: {e}", entry.path),
            )))
        })?;
        let target = staging.join(rel);
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        atomic_write(&target, &data)?;
    }

    let previous = if base_dir.exists() {
        let rollback = snapshot_temp_path(base_dir, "rollback");
        if rollback.exists() {
            std::fs::remove_dir_all(&rollback)?;
        }
        std::fs::rename(base_dir, &rollback)?;
        Some(rollback)
    } else {
        None
    };

    if let Err(e) = std::fs::rename(&staging, base_dir) {
        let _ = if let Some(previous) = &previous {
            std::fs::rename(previous, base_dir)
        } else {
            Ok(())
        };
        let _ = std::fs::remove_dir_all(&staging);
        return Err(VaultError::Io(e));
    }

    if let Some(previous) = previous {
        std::fs::remove_dir_all(previous)?;
    }

    Ok(())
}

fn recover_restore_layout(base_dir: &Path) -> Result<bool, VaultError> {
    let staging = snapshot_temp_path(base_dir, "restoring");
    let rollback = snapshot_temp_path(base_dir, "rollback");
    let mut recovered = false;

    if base_dir.exists() {
        if rollback.exists() {
            std::fs::remove_dir_all(&rollback)?;
            recovered = true;
        }
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
            recovered = true;
        }
        return Ok(recovered);
    }

    if rollback.exists() {
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }
        std::fs::rename(&rollback, base_dir)?;
        return Ok(true);
    }

    if staging.exists() {
        std::fs::rename(&staging, base_dir)?;
        return Ok(true);
    }

    Ok(false)
}

// ── Decryption & Deserialization ──────────────────────────────────────────

fn decode_encrypted_snapshot(
    passphrase: &str,
    snapshot_bytes: &[u8],
) -> Result<(SnapshotArchive, SnapshotSummary), VaultError> {
    let envelope: EncryptedSnapshot = serde_json::from_slice(snapshot_bytes)?;
    if envelope.version != SNAPSHOT_VERSION {
        return Err(VaultError::Other(format!(
            "unsupported snapshot version: {}",
            envelope.version
        )));
    }

    let salt = decode_fixed_hex::<32>(&envelope.salt_hex, "snapshot salt")?;
    let nonce_bytes = decode_fixed_hex::<12>(&envelope.nonce_hex, "snapshot nonce")?;
    let ciphertext = hex::decode(&envelope.ciphertext_hex)
        .map_err(|e| VaultError::Decryption(format!("decode snapshot ciphertext: {e}")))?;

    let wrap_key = derive_key_with_salt(passphrase, &salt);
    let cipher = Aes256Gcm::new_from_slice(&wrap_key[..])
        .map_err(|e| VaultError::Decryption(format!("snapshot key init: {e}")))?;
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
            .map_err(|_| VaultError::Decryption("failed to decrypt snapshot".into()))?,
    );

    let archive: SnapshotArchive = serde_json::from_slice(plaintext.as_ref())?;
    if archive.version != SNAPSHOT_VERSION {
        return Err(VaultError::Other(format!(
            "unsupported archive version: {}",
            archive.version
        )));
    }

    let summary = SnapshotSummary {
        created_at_unix: archive.created_at_unix,
        file_count: archive.entries.len(),
        total_bytes: archive
            .entries
            .iter()
            .map(|entry| entry.data_hex.len() / 2)
            .sum(),
    };

    Ok((archive, summary))
}

// ── Path Validation & Security ────────────────────────────────────────────

fn sanitize_relative_path(path: &Path) -> Result<PathBuf, VaultError> {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            _ => {
                return Err(VaultError::Other(format!(
                    "invalid snapshot path: {}",
                    path.display()
                )));
            }
        }
    }

    if clean.as_os_str().is_empty() {
        return Err(VaultError::Other("invalid empty snapshot path".into()));
    }

    Ok(clean)
}

// ── Helper Functions ──────────────────────────────────────────────────────

fn snapshot_temp_path(base_dir: &Path, suffix: &str) -> PathBuf {
    let parent = base_dir.parent().unwrap_or(Path::new("."));
    let name = base_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sigillum".into());
    parent.join(format!(".{name}.{suffix}"))
}

fn decode_fixed_hex<const N: usize>(value: &str, label: &str) -> Result<[u8; N], VaultError> {
    let bytes =
        hex::decode(value).map_err(|e| VaultError::Decryption(format!("decode {label}: {e}")))?;
    if bytes.len() != N {
        return Err(VaultError::Decryption(format!(
            "{label} has wrong length: expected {N}, got {}",
            bytes.len()
        )));
    }
    let mut out = [0u8; N];
    out.copy_from_slice(&bytes);
    Ok(out)
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn snapshot_roundtrip_restores_tree() {
        let src = TempDir::new().unwrap();
        let restore_parent = TempDir::new().unwrap();

        std::fs::create_dir_all(src.path().join("compartments/0")).unwrap();
        std::fs::write(src.path().join(".initialized"), b"1").unwrap();
        std::fs::write(
            src.path().join("fido2_keys.json"),
            br#"{"keys":[],"total_shares":0}"#,
        )
        .unwrap();
        std::fs::write(
            src.path().join("compartments/0/api_keys.json"),
            br#"{"a":"b"}"#,
        )
        .unwrap();
        std::fs::write(src.path().join("compartments/0/vault.enc"), b"ciphertext").unwrap();

        let (snapshot, summary) = export_encrypted_snapshot(src.path(), "correct horse").unwrap();
        assert_eq!(summary.file_count, 4);

        let dest = restore_parent.path().join("restored");
        let restored = restore_encrypted_snapshot(&dest, "correct horse", &snapshot).unwrap();
        assert_eq!(restored.file_count, 4);
        assert_eq!(std::fs::read(dest.join(".initialized")).unwrap(), b"1");
        assert_eq!(
            std::fs::read(dest.join("compartments/0/api_keys.json")).unwrap(),
            br#"{"a":"b"}"#
        );
        assert_eq!(
            std::fs::read(dest.join("compartments/0/vault.enc")).unwrap(),
            b"ciphertext"
        );
    }

    #[test]
    fn snapshot_restore_rejects_wrong_passphrase() {
        let src = TempDir::new().unwrap();
        std::fs::write(src.path().join(".initialized"), b"1").unwrap();

        let (snapshot, _) = export_encrypted_snapshot(src.path(), "correct horse").unwrap();
        assert!(matches!(
            restore_encrypted_snapshot(src.path(), "wrong", &snapshot),
            Err(VaultError::Decryption(_))
        ));
    }

    #[test]
    fn inspect_snapshot_returns_summary_without_restoring() {
        let src = TempDir::new().unwrap();
        std::fs::write(src.path().join(".initialized"), b"1").unwrap();
        std::fs::write(
            src.path().join("fido2_keys.json"),
            br#"{"keys":[],"total_shares":0}"#,
        )
        .unwrap();

        let (snapshot, summary) = export_encrypted_snapshot(src.path(), "correct horse").unwrap();
        let inspected = inspect_encrypted_snapshot("correct horse", &snapshot).unwrap();
        assert_eq!(inspected, summary);
    }

    #[test]
    fn snapshot_restore_rejects_path_traversal() {
        let archive = SnapshotArchive {
            version: SNAPSHOT_VERSION,
            created_at_unix: 1,
            entries: vec![SnapshotEntry {
                path: "../escape".into(),
                data_hex: hex::encode(b"nope"),
            }],
        };
        let plaintext = serde_json::to_vec(&archive).unwrap();
        let salt = [7u8; 32];
        let wrap_key = derive_key_with_salt("passphrase", &salt);
        let cipher = Aes256Gcm::new_from_slice(&wrap_key[..]).unwrap();
        let nonce_bytes = [9u8; 12];
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
            .unwrap();
        let envelope = EncryptedSnapshot {
            version: SNAPSHOT_VERSION,
            created_at_unix: 1,
            salt_hex: hex::encode(salt),
            nonce_hex: hex::encode(nonce_bytes),
            ciphertext_hex: hex::encode(ciphertext),
        };
        let encoded = serde_json::to_vec(&envelope).unwrap();

        let dest = TempDir::new().unwrap();
        assert!(matches!(
            restore_encrypted_snapshot(dest.path(), "passphrase", &encoded),
            Err(VaultError::Other(_))
        ));
    }

    #[test]
    fn snapshot_recovery_restores_rollback_tree() {
        let parent = TempDir::new().unwrap();
        let base = parent.path().join("sigillum");
        let rollback = parent.path().join(".sigillum.rollback");
        let staging = parent.path().join(".sigillum.restoring");

        std::fs::create_dir_all(&rollback).unwrap();
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(rollback.join(".initialized"), b"1").unwrap();

        let recovered = recover_snapshot_restore(&base).unwrap();
        assert!(recovered);
        assert!(base.exists());
        assert!(!rollback.exists());
        assert!(!staging.exists());
        assert_eq!(std::fs::read(base.join(".initialized")).unwrap(), b"1");
    }
}
