use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use rand::rngs::OsRng;
use rand::RngCore;
use secrecy::SecretString;
use zeroize::Zeroizing;

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
pub struct FileVault {
    config: VaultConfig,
    master_key: Mutex<Option<Zeroizing<[u8; 32]>>>,
}

impl FileVault {
    pub fn new(config: VaultConfig) -> Self {
        Self {
            config,
            master_key: Mutex::new(None),
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
        self.with_master_key(|mk| self.load_store(mk).is_some())
            .unwrap_or(false)
    }

    /// Execute a closure with a reference to the master key.
    /// The key never leaves the Mutex. Returns `None` if locked.
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
            let _ = std::fs::set_permissions(
                &self.config.base_dir,
                std::fs::Permissions::from_mode(0o700),
            );
        }
        Ok(())
    }

    fn set_file_perms(&self, path: &std::path::Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        #[cfg(not(unix))]
        let _ = path;
    }

    // ── Tier 1: plaintext JSON ────────────────────────────────────

    fn load_api_store(&self) -> HashMap<String, String> {
        let path = self.tier1_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => HashMap::new(),
        }
    }

    fn save_api_store(&self, store: &HashMap<String, String>) -> Result<(), VaultError> {
        self.ensure_dir()?;
        let path = self.tier1_path();
        let json = serde_json::to_string_pretty(store)?;
        std::fs::write(&path, &json)?;
        self.set_file_perms(&path);
        Ok(())
    }

    // ── Tier 2: AES-256-GCM encrypted ────────────────────────────

    fn load_store(&self, master_key: &[u8; 32]) -> Option<HashMap<String, String>> {
        let data = std::fs::read(self.tier2_path()).ok()?;
        if data.len() < 12 {
            return None;
        }
        let (nonce_bytes, ciphertext) = data.split_at(12);
        let cipher = Aes256Gcm::new_from_slice(master_key).ok()?;
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher.decrypt(nonce, ciphertext).ok()?;
        serde_json::from_slice(&plaintext).ok()
    }

    fn save_store(
        &self,
        master_key: &[u8; 32],
        store: &HashMap<String, String>,
    ) -> Result<(), VaultError> {
        self.ensure_dir()?;
        let plaintext = serde_json::to_vec(store)?;
        let cipher = Aes256Gcm::new_from_slice(master_key)
            .map_err(|e| VaultError::Encryption(e.to_string()))?;

        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_ref())
            .map_err(|e| VaultError::Encryption(e.to_string()))?;

        let mut output = Vec::with_capacity(12 + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);

        let path = self.tier2_path();
        std::fs::write(&path, &output)?;
        self.set_file_perms(&path);
        Ok(())
    }
}

impl SecretStore for FileVault {
    // ── Tier 1 ────────────────────────────────────────────────────

    fn get_api_key(&self, key: &str) -> Option<SecretString> {
        self.load_api_store()
            .get(key)
            .map(|v| SecretString::from(v.clone()))
    }

    fn set_api_key(&self, key: &str, value: &str) -> Result<(), VaultError> {
        let mut store = self.load_api_store();
        store.insert(key.to_string(), value.to_string());
        self.save_api_store(&store)
    }

    fn delete_api_key(&self, key: &str) -> Result<(), VaultError> {
        let mut store = self.load_api_store();
        store.remove(key);
        self.save_api_store(&store)
    }

    fn list_api_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.load_api_store().keys().cloned().collect();
        keys.sort();
        keys
    }

    // ── Tier 2 ────────────────────────────────────────────────────

    fn get_secret(&self, key: &str) -> Option<SecretString> {
        self.with_master_key(|mk| {
            self.load_store(mk)?
                .get(key)
                .map(|v| SecretString::from(v.clone()))
        })
        .flatten()
    }

    fn set_secret(&self, key: &str, value: &str) -> Result<(), VaultError> {
        self.with_master_key(|mk| {
            let mut store = self.load_store(mk).unwrap_or_default();
            store.insert(key.to_string(), value.to_string());
            self.save_store(mk, &store)
        })
        .unwrap_or(Err(VaultError::Locked))
    }

    fn delete_secret(&self, key: &str) -> Result<(), VaultError> {
        self.with_master_key(|mk| {
            let mut store = self.load_store(mk).unwrap_or_default();
            store.remove(key);
            self.save_store(mk, &store)
        })
        .unwrap_or(Err(VaultError::Locked))
    }

    fn list_secrets(&self) -> Vec<String> {
        self.with_master_key(|mk| {
            let mut keys: Vec<String> = self
                .load_store(mk)
                .unwrap_or_default()
                .keys()
                .cloned()
                .collect();
            keys.sort();
            keys
        })
        .unwrap_or_default()
    }

    // ── Common ────────────────────────────────────────────────────

    fn has_key(&self, key: &str) -> bool {
        // Check Tier 1 first (no unlock needed)
        if self.load_api_store().contains_key(key) {
            return true;
        }
        // Then Tier 2 (requires unlock)
        self.with_master_key(|mk| {
            self.load_store(mk)
                .map(|s| s.contains_key(key))
                .unwrap_or(false)
        })
        .unwrap_or(false)
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

    // ── Tier 2 tests ──────────────────────────────────────────────

    #[test]
    fn tier2_locked_returns_none() {
        let (vault, _dir) = test_vault();
        assert!(vault.get_secret("anything").is_none());
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
    }
}
