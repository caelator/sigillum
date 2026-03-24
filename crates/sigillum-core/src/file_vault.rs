//! Two-tier file-backed vault with plaintext and encrypted storage.
//!
//! This module implements a two-tier secret storage system:
//! - **Tier 1**: API keys stored as plaintext JSON in `api_keys.json`.
//! - **Tier 2**: Secrets encrypted with AES-256-GCM under a master key held in memory in `vault.enc`.
//!
//! ## Design Rationale
//!
//! The two-tier design reflects different threat models: API keys may be acceptable to store
//! in plaintext for convenience, while long-lived secrets require encryption. The master key
//! is stored in memory wrapped in `Mutex<Option<Zeroizing<[u8;32]>>>` and is `None` when locked,
//! preventing any Tier 2 operations until unlocked.
//!
//! ## Key Invariants
//!
//! - **Atomic writes**: All file mutations go through `atomic_write()` to prevent partial writes
//!   and data loss on crash.
//! - **Serialized updates**: A `write_lock` Mutex serializes all read-modify-write operations,
//!   preventing concurrent writes that could lose data.
//! - **Secure key storage**: The master key uses `Zeroizing<[u8;32]>` to ensure memory is
//!   wiped on drop. The key is never exposed outside the Mutex.
//! - **Locked state**: When the master key is `None`, all Tier 2 operations return
//!   `Err(VaultError::Locked)` to enforce explicit unlock before access.
//!
//! ## Directory Permissions
//!
//! On Unix systems, the vault directory is created with mode `0o700` (readable/writable/executable
//! by owner only) to restrict access to the current user.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use rand::RngCore;
use rand::rngs::OsRng;
use secrecy::SecretString;
use zeroize::Zeroize;
use zeroize::Zeroizing;

use crate::utils::atomic_write;
use crate::{SecretStore, VaultError, VaultLifecycle};

/// Configuration for the file-backed vault.
pub struct VaultConfig {
    /// Base directory for vault files (e.g., `~/.sigillum`).
    pub base_dir: PathBuf,
    /// Filename for Tier 1 (plaintext) keys.
    pub tier1_file: String,
    /// Filename for Tier 2 (encrypted) secrets.
    pub tier2_file: String,
}

impl Default for VaultConfig {
    fn default() -> Self {
        let base = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".sigillum");
        Self {
            base_dir: base,
            tier1_file: "api_keys.json".into(),
            tier2_file: "vault.enc".into(),
        }
    }
}

/// File-backed vault with AES-256-GCM encryption.
///
/// Master key is held in a `Mutex<Option<Zeroizing<[u8; 32]>>>`.
/// When `None`, the vault is locked and Tier 2 operations return
/// `None` or `Err(VaultError::Locked)`.
///
/// All read-modify-write operations on vault files are serialized through
/// `write_lock` to prevent concurrent-write data loss (B3).
pub struct FileVault {
    config: VaultConfig,
    master_key: Mutex<Option<Zeroizing<[u8; 32]>>>,
    /// Serializes all file write operations to prevent read-modify-write races.
    write_lock: Mutex<()>,
}

impl FileVault {
    /// Create a new file vault with the given configuration.
    pub fn new(config: VaultConfig) -> Self {
        Self {
            config,
            master_key: Mutex::new(None),
            write_lock: Mutex::new(()),
        }
    }

    /// Return the base directory path for vault files.
    pub fn config_base_dir(&self) -> &std::path::Path {
        &self.config.base_dir
    }

    /// Check if the encrypted vault file exists on disk.
    pub fn vault_exists(&self) -> bool {
        self.tier2_path().exists()
    }

    /// Verify that the currently loaded master key can decrypt the vault.
    /// Returns `true` if decryption succeeds, `false` otherwise.
    pub fn verify_master_key(&self) -> bool {
        self.with_master_key(|mk| self.load_store(mk).is_ok())
            .unwrap_or(false)
    }

    /// Execute a closure with a reference to the master key.
    /// The key never leaves the Mutex. Returns `None` if locked.
    #[must_use]
    pub fn with_master_key<F, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&[u8; 32]) -> T,
    {
        let mk = self.master_key.lock().unwrap_or_else(|e| e.into_inner());
        mk.as_ref().map(|key| f(key))
    }

    // ── Path helpers ──────────────────────────────────────────────

    fn tier1_path(&self) -> PathBuf {
        self.config.base_dir.join(&self.config.tier1_file)
    }

    fn tier2_path(&self) -> PathBuf {
        self.config.base_dir.join(&self.config.tier2_file)
    }

    fn ensure_dir(&self) -> Result<(), VaultError> {
        std::fs::create_dir_all(&self.config.base_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &self.config.base_dir,
                std::fs::Permissions::from_mode(0o700),
            )?;
        }
        Ok(())
    }

    // ── Tier 1: plaintext JSON ────────────────────────────────────

    fn load_api_store(&self) -> Result<HashMap<String, String>, VaultError> {
        let path = self.tier1_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => Ok(serde_json::from_str(&content)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(e) => Err(VaultError::Io(e)),
        }
    }

    fn save_api_store(&self, store: &HashMap<String, String>) -> Result<(), VaultError> {
        self.ensure_dir()?;
        let path = self.tier1_path();
        let json = serde_json::to_string_pretty(store)?;
        atomic_write(&path, json.as_bytes())?;
        Ok(())
    }

    // ── Tier 2: AES-256-GCM encrypted ────────────────────────────

    fn load_store(&self, master_key: &[u8; 32]) -> Result<HashMap<String, String>, VaultError> {
        let data = match std::fs::read(self.tier2_path()) {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(VaultError::NotInitialized);
            }
            Err(e) => return Err(VaultError::Io(e)),
        };
        if data.len() < 12 {
            return Err(VaultError::Decryption("ciphertext too short".into()));
        }
        let (nonce_bytes, ciphertext) = data.split_at(12);
        let cipher = Aes256Gcm::new_from_slice(master_key)
            .map_err(|e| VaultError::Encryption(e.to_string()))?;
        let nonce = Nonce::from_slice(nonce_bytes);
        let mut plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| VaultError::Decryption("failed to decrypt vault".into()))?;
        let store = serde_json::from_slice(&plaintext).map_err(VaultError::Serialization);
        plaintext.zeroize();
        store
    }

    fn save_store(
        &self,
        master_key: &[u8; 32],
        store: &HashMap<String, String>,
    ) -> Result<(), VaultError> {
        self.ensure_dir()?;
        let mut plaintext = serde_json::to_vec(store)?;
        let cipher = Aes256Gcm::new_from_slice(master_key)
            .map_err(|e| VaultError::Encryption(e.to_string()))?;

        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_ref())
            .map_err(|e| VaultError::Encryption(e.to_string()))?;
        plaintext.zeroize();

        let mut output = Vec::with_capacity(12 + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);

        let path = self.tier2_path();
        atomic_write(&path, &output)?;
        Ok(())
    }
}

impl SecretStore for FileVault {
    // ── Tier 1 ────────────────────────────────────────────────────

    fn read_api_key(&self, key: &str) -> Result<Option<SecretString>, VaultError> {
        Ok(self
            .load_api_store()?
            .get(key)
            .map(|v| SecretString::from(v.clone())))
    }

    fn set_api_key(&self, key: &str, value: &str) -> Result<(), VaultError> {
        let _wl = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut store = self.load_api_store()?;
        store.insert(key.to_string(), value.to_string());
        self.save_api_store(&store)
    }

    fn delete_api_key(&self, key: &str) -> Result<(), VaultError> {
        let _wl = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut store = self.load_api_store()?;
        store.remove(key);
        self.save_api_store(&store)
    }

    fn read_api_keys(&self) -> Result<Vec<String>, VaultError> {
        let mut keys: Vec<String> = self.load_api_store()?.keys().cloned().collect();
        keys.sort();
        Ok(keys)
    }

    // ── Tier 2 ────────────────────────────────────────────────────

    fn read_secret(&self, key: &str) -> Result<Option<SecretString>, VaultError> {
        self.with_master_key(|mk| {
            Ok(self
                .load_store(mk)?
                .get(key)
                .map(|v| SecretString::from(v.clone())))
        })
        .unwrap_or(Err(VaultError::Locked))
    }

    fn set_secret(&self, key: &str, value: &str) -> Result<(), VaultError> {
        let _wl = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        self.with_master_key(|mk| {
            let mut store = self.load_store(mk)?;
            store.insert(key.to_string(), value.to_string());
            self.save_store(mk, &store)
        })
        .unwrap_or(Err(VaultError::Locked))
    }

    fn delete_secret(&self, key: &str) -> Result<(), VaultError> {
        let _wl = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        self.with_master_key(|mk| {
            let mut store = self.load_store(mk)?;
            store.remove(key);
            self.save_store(mk, &store)
        })
        .unwrap_or(Err(VaultError::Locked))
    }

    fn read_secrets(&self) -> Result<Vec<String>, VaultError> {
        self.with_master_key(|mk| {
            let mut keys: Vec<String> = self.load_store(mk)?.keys().cloned().collect();
            keys.sort();
            Ok(keys)
        })
        .unwrap_or(Err(VaultError::Locked))
    }

    // ── Common ────────────────────────────────────────────────────

    fn contains_key(&self, key: &str) -> Result<bool, VaultError> {
        // Check Tier 1 first (no unlock needed)
        if self.load_api_store()?.contains_key(key) {
            return Ok(true);
        }
        // Then Tier 2 (requires unlock)
        self.with_master_key(|mk| Ok(self.load_store(mk)?.contains_key(key)))
            .unwrap_or(Ok(false))
    }

    fn is_unlocked(&self) -> bool {
        self.master_key
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
}

impl VaultLifecycle for FileVault {
    fn load_master_key(&self, key: [u8; 32]) {
        let mut mk = self.master_key.lock().unwrap_or_else(|e| e.into_inner());
        *mk = Some(Zeroizing::new(key));
    }

    fn zeroize_master_key(&self) {
        let mut mk = self.master_key.lock().unwrap_or_else(|e| e.into_inner());
        *mk = None; // Zeroizing<[u8;32]> zeros on drop
    }

    fn initialize(&self, master_key: &[u8; 32]) -> Result<(), VaultError> {
        let store: HashMap<String, String> = HashMap::new();
        self.save_store(master_key, &store)
    }

    fn extract_master_key(&self) -> Option<Zeroizing<[u8; 32]>> {
        self.with_master_key(|mk| Zeroizing::new(*mk))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;
    use tempfile::TempDir;

    fn test_vault() -> (FileVault, TempDir) {
        let dir = TempDir::new().unwrap();
        let vault = FileVault::new(VaultConfig {
            base_dir: dir.path().to_path_buf(),
            tier1_file: "api_keys.json".into(),
            tier2_file: "vault.enc".into(),
        });
        (vault, dir)
    }

    fn test_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        key
    }

    // ── Tier 1 tests ──────────────────────────────────────────────

    #[test]
    fn tier1_set_get_roundtrip() {
        let (vault, _dir) = test_vault();
        vault.set_api_key("test_key", "test_value").unwrap();
        let val = vault.get_api_key("test_key").unwrap();
        assert_eq!(val.expose_secret(), "test_value");
    }

    #[test]
    fn tier1_get_missing_returns_none() {
        let (vault, _dir) = test_vault();
        assert!(vault.get_api_key("nonexistent").is_none());
    }

    #[test]
    fn tier1_delete() {
        let (vault, _dir) = test_vault();
        vault.set_api_key("k", "v").unwrap();
        assert!(vault.get_api_key("k").is_some());
        vault.delete_api_key("k").unwrap();
        assert!(vault.get_api_key("k").is_none());
    }

    #[test]
    fn tier1_list() {
        let (vault, _dir) = test_vault();
        vault.set_api_key("b_key", "1").unwrap();
        vault.set_api_key("a_key", "2").unwrap();
        let keys = vault.list_api_keys();
        assert_eq!(keys, vec!["a_key", "b_key"]);
    }

    #[test]
    fn tier1_overwrite() {
        let (vault, _dir) = test_vault();
        vault.set_api_key("k", "old").unwrap();
        vault.set_api_key("k", "new").unwrap();
        assert_eq!(vault.get_api_key("k").unwrap().expose_secret(), "new");
    }

    #[test]
    fn tier1_malformed_store_blocks_write_and_preserves_data() {
        let (vault, dir) = test_vault();
        let path = dir.path().join("api_keys.json");
        std::fs::write(&path, "{not json").unwrap();

        assert!(matches!(
            vault.set_api_key("k", "v"),
            Err(VaultError::Serialization(_))
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{not json");
    }

    #[test]
    fn tier1_malformed_store_surfaces_on_read() {
        let (vault, dir) = test_vault();
        let path = dir.path().join("api_keys.json");
        std::fs::write(&path, "{not json").unwrap();

        assert!(matches!(
            vault.read_api_keys(),
            Err(VaultError::Serialization(_))
        ));
        assert!(matches!(
            vault.read_api_key("missing"),
            Err(VaultError::Serialization(_))
        ));
        assert!(matches!(
            vault.contains_key("missing"),
            Err(VaultError::Serialization(_))
        ));
    }

    // ── Tier 2 tests ──────────────────────────────────────────────

    #[test]
    fn tier2_locked_returns_none() {
        let (vault, _dir) = test_vault();
        assert!(vault.get_secret("anything").is_none());
        assert!(matches!(
            vault.read_secret("anything"),
            Err(VaultError::Locked)
        ));
        assert!(matches!(vault.read_secrets(), Err(VaultError::Locked)));
    }

    #[test]
    fn tier2_locked_set_returns_err() {
        let (vault, _dir) = test_vault();
        assert!(vault.set_secret("k", "v").is_err());
    }

    #[test]
    fn tier2_set_get_roundtrip() {
        let (vault, _dir) = test_vault();
        let key = test_key();
        vault.initialize(&key).unwrap();
        vault.load_master_key(key);

        vault.set_secret("db_pass", "hunter2").unwrap();
        let val = vault.get_secret("db_pass").unwrap();
        assert_eq!(val.expose_secret(), "hunter2");
    }

    #[test]
    fn tier2_delete() {
        let (vault, _dir) = test_vault();
        let key = test_key();
        vault.initialize(&key).unwrap();
        vault.load_master_key(key);

        vault.set_secret("k", "v").unwrap();
        vault.delete_secret("k").unwrap();
        assert!(vault.get_secret("k").is_none());
    }

    #[test]
    fn tier2_list() {
        let (vault, _dir) = test_vault();
        let key = test_key();
        vault.initialize(&key).unwrap();
        vault.load_master_key(key);

        vault.set_secret("z_secret", "1").unwrap();
        vault.set_secret("a_secret", "2").unwrap();
        let keys = vault.list_secrets();
        assert_eq!(keys, vec!["a_secret", "z_secret"]);
    }

    // ── Lifecycle tests ───────────────────────────────────────────

    #[test]
    fn unlock_lock_cycle() {
        let (vault, _dir) = test_vault();
        let key = test_key();

        assert!(!vault.is_unlocked());
        vault.load_master_key(key);
        assert!(vault.is_unlocked());
        vault.zeroize_master_key();
        assert!(!vault.is_unlocked());
    }

    #[test]
    fn lock_makes_tier2_inaccessible() {
        let (vault, _dir) = test_vault();
        let key = test_key();
        vault.initialize(&key).unwrap();
        vault.load_master_key(key);

        vault.set_secret("s", "secret").unwrap();
        vault.zeroize_master_key();

        assert!(vault.get_secret("s").is_none());
        assert!(vault.set_secret("new", "val").is_err());
    }

    #[test]
    fn relock_and_reunlock_persists() {
        let (vault, _dir) = test_vault();
        let key = test_key();
        vault.initialize(&key).unwrap();
        vault.load_master_key(key);

        vault.set_secret("persistent", "data").unwrap();
        vault.zeroize_master_key();

        // Re-unlock with same key
        vault.load_master_key(key);
        let val = vault.get_secret("persistent").unwrap();
        assert_eq!(val.expose_secret(), "data");
    }

    // ── has_key tests ─────────────────────────────────────────────

    #[test]
    fn has_key_tier1() {
        let (vault, _dir) = test_vault();
        vault.set_api_key("exists", "v").unwrap();
        assert!(vault.has_key("exists"));
        assert!(!vault.has_key("nope"));
    }

    #[test]
    fn has_key_tier2_requires_unlock() {
        let (vault, _dir) = test_vault();
        let key = test_key();
        vault.initialize(&key).unwrap();
        vault.load_master_key(key);
        vault.set_secret("hidden", "v").unwrap();

        assert!(vault.has_key("hidden"));
        vault.zeroize_master_key();
        // Tier 2 key not visible when locked
        assert!(!vault.has_key("hidden"));
    }

    // ── Wrong key test ────────────────────────────────────────────

    #[test]
    fn wrong_key_cannot_decrypt() {
        let (vault, _dir) = test_vault();
        let key1 = test_key();
        let key2 = test_key();

        vault.initialize(&key1).unwrap();
        vault.load_master_key(key1);
        vault.set_secret("s", "secret").unwrap();
        vault.zeroize_master_key();

        // Try with wrong key
        vault.load_master_key(key2);
        assert!(vault.get_secret("s").is_none());
        assert!(matches!(
            vault.read_secret("s"),
            Err(VaultError::Decryption(_))
        ));
    }

    #[test]
    fn wrong_key_cannot_overwrite_existing_store() {
        let (vault, _dir) = test_vault();
        let key1 = test_key();
        let key2 = test_key();

        vault.initialize(&key1).unwrap();
        vault.load_master_key(key1);
        vault.set_secret("s", "secret").unwrap();
        vault.zeroize_master_key();

        vault.load_master_key(key2);
        assert!(matches!(
            vault.set_secret("new", "value"),
            Err(VaultError::Decryption(_))
        ));
        vault.zeroize_master_key();

        vault.load_master_key(key1);
        assert_eq!(vault.get_secret("s").unwrap().expose_secret(), "secret");
        assert!(vault.get_secret("new").is_none());
    }

    #[test]
    fn corrupted_tier2_store_surfaces_on_read() {
        let (vault, dir) = test_vault();
        let key = test_key();

        vault.initialize(&key).unwrap();
        vault.load_master_key(key);
        vault.set_secret("s", "secret").unwrap();

        std::fs::write(dir.path().join("vault.enc"), b"bad").unwrap();

        assert!(matches!(
            vault.read_secret("s"),
            Err(VaultError::Decryption(_))
        ));
        assert!(matches!(
            vault.read_secrets(),
            Err(VaultError::Decryption(_))
        ));
        assert!(matches!(
            vault.contains_key("s"),
            Err(VaultError::Decryption(_))
        ));
    }
}
