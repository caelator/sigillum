use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use sigillum_core::{FileVault, VaultConfig, VaultLifecycle};
use sigillum_fido2::Fido2Manager;
use zeroize::Zeroizing;

/// Shared daemon state with multi-compartment vault support.
///
/// Each compartment gets its own `FileVault` at `base_dir/compartments/{id}/`.
/// The `active_compartment` tracks which compartment is currently unlocked.
pub struct AppState {
    pub fido2: Fido2Manager,
    pub base_dir: PathBuf,
    vaults: Mutex<HashMap<usize, FileVault>>,
    active: Mutex<Option<usize>>,
}

impl AppState {
    pub fn new(base_dir: PathBuf) -> Self {
        let fido2_config_path = base_dir.join("fido2_keys.json");
        let fido2 = Fido2Manager::new(fido2_config_path);

        // Pre-load vaults for existing compartments
        let config = fido2.load_config_raw();
        let mut vaults = HashMap::new();
        for comp in &config.compartments {
            let vault_dir = base_dir.join("compartments").join(comp.id.to_string());
            let vault_config = VaultConfig {
                base_dir: vault_dir,
                tier1_file: "api_keys.json".into(),
                tier2_file: "vault.enc".into(),
            };
            vaults.insert(comp.id, FileVault::new(vault_config));
        }

        Self {
            fido2,
            base_dir,
            vaults: Mutex::new(vaults),
            active: Mutex::new(None),
        }
    }

    /// Currently active compartment id.
    pub fn active_compartment_id(&self) -> Option<usize> {
        *self.active.lock().unwrap()
    }

    pub fn set_active(&self, id: Option<usize>) {
        *self.active.lock().unwrap() = id;
    }

    /// Path to a compartment's data directory.
    pub fn compartment_dir(&self, id: usize) -> PathBuf {
        self.base_dir.join("compartments").join(id.to_string())
    }

    pub fn salt_path(&self, id: usize) -> PathBuf {
        self.compartment_dir(id).join("passphrase.salt")
    }

    pub fn wrapped_key_path(&self, id: usize) -> PathBuf {
        self.compartment_dir(id).join("passphrase_wrapped_key.enc")
    }

    /// Ensure a FileVault exists for the given compartment, creating if needed.
    pub fn ensure_vault(&self, id: usize) {
        let mut vaults = self.vaults.lock().unwrap();
        if !vaults.contains_key(&id) {
            let vault_dir = self.compartment_dir(id);
            let config = VaultConfig {
                base_dir: vault_dir,
                tier1_file: "api_keys.json".into(),
                tier2_file: "vault.enc".into(),
            };
            vaults.insert(id, FileVault::new(config));
        }
    }

    /// Execute a closure with the active compartment's vault.
    pub fn with_active_vault<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&FileVault) -> R,
    {
        let id = self.active_compartment_id()?;
        let vaults = self.vaults.lock().unwrap();
        let vault = vaults.get(&id)?;
        Some(f(vault))
    }

    /// Execute a closure with a specific compartment's vault.
    pub fn with_vault<F, R>(&self, id: usize, f: F) -> Option<R>
    where
        F: FnOnce(&FileVault) -> R,
    {
        let vaults = self.vaults.lock().unwrap();
        let vault = vaults.get(&id)?;
        Some(f(vault))
    }

    /// Unlock a specific compartment: load master key and set as active.
    pub fn unlock_compartment(&self, id: usize, master_key: [u8; 32]) {
        self.ensure_vault(id);
        let vaults = self.vaults.lock().unwrap();
        if let Some(vault) = vaults.get(&id) {
            vault.load_master_key(master_key);
        }
        drop(vaults);
        self.set_active(Some(id));
    }

    /// Lock all compartments and clear active.
    pub fn lock_all(&self) {
        let vaults = self.vaults.lock().unwrap();
        for vault in vaults.values() {
            vault.zeroize_master_key();
        }
        drop(vaults);
        self.set_active(None);
    }

    /// Check if any compartment vault exists on disk.
    pub fn any_vault_exists(&self) -> bool {
        let vaults = self.vaults.lock().unwrap();
        vaults.values().any(|v| v.vault_exists())
    }

    /// Extract master keys from all unlocked compartment vaults.
    pub fn extract_all_master_keys(&self) -> Vec<(usize, Zeroizing<[u8; 32]>)> {
        let vaults = self.vaults.lock().unwrap();
        vaults
            .iter()
            .filter_map(|(id, vault)| vault.extract_master_key().map(|mk| (*id, mk)))
            .collect()
    }
}
