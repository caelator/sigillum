//! # Sigillum FIDO2
//!
//! FIDO2 hardware key registration, cascading compartment unlock, and Shamir
//! secret sharing for Sigillum vault management.
//!
//! ## Quorum Model
//!
//! Multiple FIDO2 keys can be registered, and each compartment has a threshold
//! (quorum requirement). Unlocking a compartment requires tapping at least
//! `threshold` distinct keys. Keys can be skipped during registration/removal
//! (marked as revoked), allowing zero-downtime key rotation.
//!
//! Quorum is per-compartment: you might have key1 (threshold 1), key2+key3 (threshold 2).
//! Tapping 2 keys cascadingly unlocks all compartments with threshold <= 2.
//!
//! ## Shamir Secret Sharing for Master Key Distribution
//!
//! Each master key is split into `n` Shamir shares (where `n = number of active keys`).
//! Each key receives one share, encrypted with its unique HMAC-secret from the FIDO2
//! device. Reconstruction requires at least `threshold` shares.
//!
//! When a new key is registered (or an old one removed), all shares are re-split with
//! fresh polynomials. This prevents an attacker who compromises one key's device from
//! learning anything about shares they didn't receive.
//!
//! ## Deniability Design
//!
//! The vault has 100 compartment slots (0-99). Compartment metadata is stored
//! encrypted per-slot (`compartments/{id}/meta.enc`). The config file (`fido2_keys.json`)
//! contains only registered key entries with opaque shard blobs — no compartment
//! definitions.
//!
//! Dummies are created for all 100 slots: unused slots get random files of the same
//! size as real meta.enc, vault.enc, and key wrapping files. This provides filesystem-level
//! deniability: an observer cannot determine how many compartments exist without
//! decrypting meta.enc files (which requires the master key).
//!
//! A poison pill key can also be registered, which receives random garbage shards
//! instead of real shares. If tapped during unlock, the garbage poisons Shamir
//! reconstruction → wrong master key → silent failure. No data is destroyed.

pub mod config;
pub mod crypto;
pub mod error;
#[cfg(feature = "hid")]
pub mod hid;
pub mod types;

#[cfg(feature = "hid")]
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use rand::rngs::OsRng;
use zeroize::Zeroizing;

use config::{CompartmentMeta, Fido2Config, SHARD_SLOTS, load_config, save_config};
#[cfg(any(feature = "hid", test))]
use config::{RegisteredKey, generate_dummy_shards};
use error::Fido2Error;
#[cfg(feature = "hid")]
use types::QuorumEvent;
use types::{Fido2Status, KeyInfo};

/// Identity of a daemon-journaled FIDO2 config mutation.
///
/// The manager embeds this identity in the same atomic write as the resulting
/// config, allowing startup recovery to clear only the exact matching journal.
#[derive(Clone, Copy, Debug)]
pub struct Fido2MutationContext<'a> {
    pub operation_id: &'a str,
    pub kind: &'a str,
    pub subject: Option<&'a str>,
}

#[cfg(unix)]
struct Fido2WriterLease {
    file: File,
}

#[cfg(unix)]
impl Drop for Fido2WriterLease {
    fn drop(&mut self) {
        // SAFETY: `file` remains open for the lifetime of the lease.
        let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

#[cfg(not(unix))]
struct Fido2WriterLease {
    path: PathBuf,
    _file: File,
}

#[cfg(not(unix))]
impl Drop for Fido2WriterLease {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// RAII guard that zeroizes HMAC secrets on drop.
#[cfg(feature = "hid")]
struct HmacSecrets(Vec<(usize, [u8; 32])>);

#[cfg(feature = "hid")]
impl Drop for HmacSecrets {
    fn drop(&mut self) {
        for (_, hmac) in &mut self.0 {
            zeroize::Zeroize::zeroize(hmac);
        }
    }
}

#[cfg(feature = "hid")]
impl std::ops::Deref for HmacSecrets {
    type Target = Vec<(usize, [u8; 32])>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(feature = "hid")]
impl std::ops::DerefMut for HmacSecrets {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(feature = "hid")]
struct HmacSecretVec(Vec<[u8; 32]>);

#[cfg(feature = "hid")]
impl Drop for HmacSecretVec {
    fn drop(&mut self) {
        for hmac in &mut self.0 {
            zeroize::Zeroize::zeroize(hmac);
        }
    }
}

#[cfg(feature = "hid")]
impl std::ops::Deref for HmacSecretVec {
    type Target = Vec<[u8; 32]>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(feature = "hid")]
impl std::ops::DerefMut for HmacSecretVec {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Byte length of an encrypted tagged shard:
/// 12 (nonce) + 4 (comp_id) + 33 (shamir share) + 16 (aead tag) = 65 bytes.
#[cfg(any(feature = "hid", test))]
const ENCRYPTED_TAGGED_SHARD_BYTES: usize = 65;

/// Result of registering a FIDO2 key.
pub struct RegisterResult {
    /// Master keys for each compartment, newly generated (first key) or
    /// re-extracted from existing shards (Nth key). The caller must load
    /// these into the vault to unlock the compartments.
    pub compartment_keys: Vec<(usize, Zeroizing<[u8; 32]>)>,
    /// Whether this was the first key registered. Affects policy: first key
    /// can never be a poison pill (vault would be permanently inaccessible).
    pub is_first_key: bool,
    /// Total registered keys after this registration. Used to validate
    /// subsequent operations (e.g., key removal must not drop below threshold).
    pub total_keys: usize,
}

/// Manages FIDO2 key registration, cascading compartment authentication, and
/// Shamir secret distribution.
///
/// **Important**: This manager does NOT hold or access the vault directly.
/// It only manages the FIDO2 config file (`fido2_keys.json`) and compartment
/// metadata (`meta.enc` files). The caller (daemon or CLI) is responsible for
/// bridging between `Fido2Manager` outputs and `VaultLifecycle` to actually
/// unlock the vault.
///
/// This separation of concerns prevents the FIDO2 manager from accumulating
/// vault access logic. It's a pure orchestrator of key and shard state.
pub struct Fido2Manager {
    config_path: PathBuf,
}

impl Fido2Manager {
    pub fn new(config_path: PathBuf) -> Self {
        Self { config_path }
    }

    fn load(&self) -> Result<Fido2Config, Fido2Error> {
        load_config(&self.config_path)
    }

    fn save(&self, config: &Fido2Config) -> Result<(), Fido2Error> {
        save_config(&self.config_path, config)
    }

    fn writer_lock_path(&self) -> PathBuf {
        self.config_path.with_extension("lock")
    }

    fn acquire_writer_lease(&self) -> Result<Fido2WriterLease, Fido2Error> {
        let path = self.writer_lock_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| Fido2Error::Config(format!("create writer lock dir: {error}")))?;
        }

        #[cfg(unix)]
        {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .map_err(|error| {
                    Fido2Error::Config(format!(
                        "open FIDO2 writer lock {}: {error}",
                        path.display()
                    ))
                })?;
            // SAFETY: `file` is a valid open descriptor and remains owned by the lease.
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result != 0 {
                let error = std::io::Error::last_os_error();
                if error
                    .raw_os_error()
                    .is_some_and(|code| code == libc::EWOULDBLOCK || code == libc::EAGAIN)
                {
                    return Err(Fido2Error::WriterBusy {
                        path: path.display().to_string(),
                    });
                }
                return Err(Fido2Error::Config(format!(
                    "lock FIDO2 writer lease {}: {error}",
                    path.display()
                )));
            }
            Ok(Fido2WriterLease { file })
        }

        #[cfg(not(unix))]
        {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|error| {
                    if error.kind() == std::io::ErrorKind::AlreadyExists {
                        Fido2Error::WriterBusy {
                            path: path.display().to_string(),
                        }
                    } else {
                        Fido2Error::Config(format!(
                            "create FIDO2 writer lease {}: {error}",
                            path.display()
                        ))
                    }
                })?;
            Ok(Fido2WriterLease { path, _file: file })
        }
    }

    fn persist_mutation(
        &self,
        config: &mut Fido2Config,
        context: Option<Fido2MutationContext<'_>>,
        default_kind: &str,
        default_subject: Option<&str>,
    ) -> Result<(), Fido2Error> {
        let (operation_id, kind, subject) = if let Some(context) = context {
            (
                context.operation_id.to_owned(),
                context.kind.to_owned(),
                context.subject.map(str::to_owned),
            )
        } else {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let mut random = [0u8; 6];
            OsRng.fill_bytes(&mut random);
            (
                format!("standalone-{now:032x}-{}", hex::encode(random)),
                default_kind.to_owned(),
                default_subject.map(str::to_owned),
            )
        };
        config.record_mutation(operation_id, kind, subject)?;
        self.save(config)
    }

    #[cfg(feature = "hid")]
    fn normalize_pin(pin: Option<&str>) -> Option<&str> {
        pin.filter(|pin| !pin.is_empty())
    }

    #[cfg(feature = "hid")]
    fn pin_for_round(pins: &[String], round: usize) -> Option<&str> {
        pins.get(round)
            .and_then(|pin| Self::normalize_pin(Some(pin.as_str())))
            .or_else(|| {
                pins.first()
                    .and_then(|pin| Self::normalize_pin(Some(pin.as_str())))
            })
    }

    /// Load the raw FIDO2 config for external inspection or mutation.
    ///
    /// The `_raw` suffix distinguishes this from the private `load()` method
    /// and signals that the caller receives the full, unfiltered config struct.
    /// Most callers should prefer higher-level methods like `list_keys()` or
    /// `status()` which expose only safe, deniability-preserving views.
    pub fn load_config_raw(&self) -> Result<Fido2Config, Fido2Error> {
        self.load()
    }

    /// Persist a raw FIDO2 config (e.g. after CLI setup creates an empty config).
    ///
    /// See [`load_config_raw`](Self::load_config_raw) for the naming convention.
    #[must_use = "check the Result to ensure config was persisted"]
    pub fn save_config_raw(&self, config: &Fido2Config) -> Result<(), Fido2Error> {
        let _lease = self.acquire_writer_lease()?;
        let current = self.load()?;
        let mut next = config.clone();
        next.generation = next.generation.max(current.generation);
        self.persist_mutation(&mut next, None, "fido2.raw-save", None)
    }

    // ── Query ────────────────────────────────────────────────────

    /// Fido2 status. Deliberately reveals NO compartment info (deniability).
    pub fn status(&self) -> Result<Fido2Status, Fido2Error> {
        let config = self.load()?;
        Ok(Fido2Status {
            enabled: config.is_fido2_enabled(),
            key_count: config.keys.len(),
        })
    }

    pub fn list_keys(&self) -> Result<Vec<KeyInfo>, Fido2Error> {
        Ok(self
            .load()?
            .keys
            .iter()
            .map(|k| KeyInfo {
                label: k.label.clone(),
                credential_id_short: k.credential_id_hex.chars().take(16).collect(),
                registered_at: k.registered_at.clone(),
            })
            .collect())
    }

    pub fn is_enabled(&self) -> Result<bool, Fido2Error> {
        Ok(self.load()?.is_fido2_enabled())
    }

    /// Set a PIN through the local HID transport.
    ///
    /// Builds compiled without the `hid` feature keep this method in the API
    /// and return an explicit error instead of pretending the operation is
    /// available.
    pub fn set_new_pin(&self, pin: &str) -> Result<(), Fido2Error> {
        #[cfg(feature = "hid")]
        {
            hid::set_new_pin(pin)
        }

        #[cfg(not(feature = "hid"))]
        {
            let _ = pin;
            Err(Fido2Error::Other(
                "FIDO2 HID support is disabled in this build".into(),
            ))
        }
    }

    // ── Compartment meta persistence ────────────────────────────

    /// Save encrypted compartment metadata to `base_dir/compartments/{id}/meta.enc`.
    ///
    /// The metadata (compartment ID, label, threshold, passphrase mode) is encrypted
    /// with AES-256-GCM using the master key. This enables deniability: an observer
    /// cannot read metadata without the key, so they cannot determine which compartment
    /// slots are real vs. dummy.
    pub fn save_compartment_meta(
        base_dir: &Path,
        meta: &CompartmentMeta,
        master_key: &[u8; 32],
    ) -> Result<(), Fido2Error> {
        let dir = base_dir.join("compartments").join(meta.id.to_string());
        std::fs::create_dir_all(&dir)
            .map_err(|e| Fido2Error::Config(format!("create compartment dir: {e}")))?;
        let encrypted = crypto::encrypt_compartment_meta(master_key, meta)?;
        let path = dir.join("meta.enc");
        config::atomic_write_bytes(&path, &encrypted)
            .map_err(|e| Fido2Error::Config(format!("write meta.enc: {e}")))?;
        Ok(())
    }

    /// Try to load and decrypt compartment metadata from `meta.enc`.
    ///
    /// Validates that the decrypted `meta.id` matches the expected `comp_id`
    /// to guard against swapped or corrupted files. Silent failure (wrong key,
    /// AEAD tag mismatch, or corrupted file) returns an error — cannot distinguish
    /// between "wrong key", "dummy compartment", or "corrupted meta.enc".
    pub fn load_compartment_meta(
        base_dir: &Path,
        comp_id: usize,
        master_key: &[u8; 32],
    ) -> Result<CompartmentMeta, Fido2Error> {
        let path = base_dir
            .join("compartments")
            .join(comp_id.to_string())
            .join("meta.enc");
        let encrypted =
            std::fs::read(&path).map_err(|e| Fido2Error::Config(format!("read meta.enc: {e}")))?;
        let meta = crypto::decrypt_compartment_meta(master_key, &encrypted)?;
        if meta.id != comp_id {
            return Err(Fido2Error::Config(format!(
                "meta.id mismatch: expected {comp_id}, got {}",
                meta.id
            )));
        }
        Ok(meta)
    }

    /// Create all 100 compartment directories. Real compartments have actual
    /// encrypted files; remaining slots get random dummy files of matching size.
    pub fn setup_dummy_directories(base_dir: &Path, real_ids: &[usize]) -> Result<(), Fido2Error> {
        let comps_dir = base_dir.join("compartments");
        for i in 0..SHARD_SLOTS {
            let dir = comps_dir.join(i.to_string());
            std::fs::create_dir_all(&dir)
                .map_err(|e| Fido2Error::Config(format!("create dir {i}: {e}")))?;

            if !real_ids.contains(&i) {
                // Populate with dummy files matching real compartment file sizes.
                // DENIABILITY: sizes must match real files exactly.
                // - meta.enc: real = 156B (padded plaintext). Dummy = 156B.
                // - vault.enc: real fresh = 30B (12 nonce + 2B `{}` + 16 tag). Dummy = 30B.
                // - passphrase.salt: always 32B.
                // - passphrase_wrapped_key.enc: real = 60B (12+32+16). Dummy = 60B.
                // - api_keys.json: NOT created. Real compartments also lack this
                //   file until the user stores an API key. Creating it with random
                //   bytes in dummies would be distinguishable (not valid JSON).
                crypto::generate_dummy_file(&dir.join("meta.enc"), 156, 156)?;
                crypto::generate_dummy_file(&dir.join("vault.enc"), 30, 30)?;
                crypto::generate_dummy_file(&dir.join("passphrase.salt"), 32, 32)?;
                crypto::generate_dummy_file(&dir.join("passphrase_wrapped_key.enc"), 60, 60)?;
            }
        }

        // Write initialization marker
        let marker = base_dir.join(".initialized");
        std::fs::write(&marker, b"1")
            .map_err(|e| Fido2Error::Config(format!("write .initialized: {e}")))?;

        Ok(())
    }

    /// Pad a shard vector to SHARD_SLOTS with random dummy blobs.
    ///
    /// Every registered key has exactly SHARD_SLOTS shards in its shard vector.
    /// Real shards (one per compartment) are mixed with dummy shards (random bytes
    /// of the same encrypted size). This prevents an observer of the config file
    /// from learning how many compartments exist — real and dummy shards are
    /// cryptographically indistinguishable without the HMAC-secret.
    #[cfg(any(feature = "hid", test))]
    fn pad_shards(shards: Vec<String>) -> Vec<String> {
        let real_count = shards.len();
        if real_count >= SHARD_SLOTS {
            return shards;
        }
        let dummies = generate_dummy_shards(SHARD_SLOTS - real_count, ENCRYPTED_TAGGED_SHARD_BYTES);
        let mut padded = shards;
        padded.extend(dummies);
        // Shuffle so real shards aren't always at the front — preserves deniability
        use rand::seq::SliceRandom;
        padded.shuffle(&mut OsRng);
        padded
    }

    // ── Registration ─────────────────────────────────────────────

    /// Register a FIDO2 key across all compartments with deniable shard format.
    ///
    /// **First key** (is_first_key): `compartment_metas` provides compartment definitions
    /// with empty master keys. New keys are generated and returned in `RegisterResult`.
    ///
    /// **Nth key** (key rotation): `compartment_metas` provides the same compartments
    /// with their existing master keys. All existing shares are re-split with fresh
    /// polynomials using the new key's HMAC-secret. This prevents an attacker who
    /// compromises an old key's device from learning anything about new shares.
    ///
    /// **Skipped keys**: Keys in `skip_labels` are not tapped (revoked). Their old
    /// shards are preserved but excluded from the new polynomial. This enables
    /// zero-downtime key rotation: mark the old key as skipped, register a new one,
    /// then physically destroy the old device.
    #[cfg(feature = "hid")]
    pub fn register_key(
        &self,
        pin: Option<&str>,
        label: &str,
        compartment_metas: &[(CompartmentMeta, &[u8; 32])],
        skip_labels: &[String],
    ) -> Result<RegisterResult, Fido2Error> {
        self.register_key_inner(pin, label, compartment_metas, skip_labels, None)
    }

    /// Register a key and bind the resulting config write to a daemon operation journal.
    #[cfg(feature = "hid")]
    pub fn register_key_for_operation(
        &self,
        pin: Option<&str>,
        label: &str,
        compartment_metas: &[(CompartmentMeta, &[u8; 32])],
        skip_labels: &[String],
        context: Fido2MutationContext<'_>,
    ) -> Result<RegisterResult, Fido2Error> {
        self.register_key_inner(pin, label, compartment_metas, skip_labels, Some(context))
    }

    #[cfg(feature = "hid")]
    fn register_key_inner(
        &self,
        pin: Option<&str>,
        label: &str,
        compartment_metas: &[(CompartmentMeta, &[u8; 32])],
        skip_labels: &[String],
        context: Option<Fido2MutationContext<'_>>,
    ) -> Result<RegisterResult, Fido2Error> {
        let _lease = self.acquire_writer_lease()?;
        let mut config = self.load()?;
        let pin = Self::normalize_pin(pin);

        if config.keys.iter().any(|k| k.label == label) {
            return Err(Fido2Error::DuplicateKey {
                label: label.into(),
            });
        }

        let is_first = config.keys.is_empty();
        let mut result_keys: Vec<(usize, Zeroizing<[u8; 32]>)> = Vec::new();

        // Compute active (non-skipped) existing key indices
        let active_indices: Vec<usize> = if is_first {
            Vec::new()
        } else {
            (0..config.keys.len())
                .filter(|i| !skip_labels.contains(&config.keys[*i].label))
                .collect()
        };
        let total = active_indices.len() + 1; // active keys + new key

        // Determine compartments to process
        let compartments: Vec<(CompartmentMeta, Zeroizing<[u8; 32]>)> = if is_first {
            if compartment_metas.is_empty() {
                return Err(Fido2Error::Config(
                    "compartment definitions required".into(),
                ));
            }
            compartment_metas
                .iter()
                .map(|(meta, _)| {
                    let mut mk = Zeroizing::new([0u8; 32]);
                    OsRng.fill_bytes(mk.as_mut());
                    (meta.clone(), mk)
                })
                .collect()
        } else {
            compartment_metas
                .iter()
                .map(|(meta, mk)| (meta.clone(), Zeroizing::new(**mk)))
                .collect()
        };

        // Cache hmac-secrets for active existing keys (one FIDO2 tap per key)
        let mut active_hmacs = HmacSecrets(Vec::with_capacity(active_indices.len()));
        if !is_first {
            for &idx in &active_indices {
                let existing_cred_id = hex::decode(&config.keys[idx].credential_id_hex)
                    .map_err(|e| Fido2Error::Config(format!("bad credential_id hex: {e}")))?;
                let hmac = hid::get_hmac_secret(&existing_cred_id, pin)?;
                active_hmacs.0.push((idx, hmac));
            }
        }

        // Step 1: Create the new key credential on the one attached device that is
        // not already known to this vault. Existing-key checks happen first so we
        // fail before minting a credential if the currently enrolled quorum is absent.
        let known_credential_ids: Vec<Vec<u8>> = config
            .keys
            .iter()
            .map(|key| {
                hex::decode(&key.credential_id_hex)
                    .map_err(|e| Fido2Error::Config(format!("bad credential_id hex: {e}")))
            })
            .collect::<Result<_, _>>()?;
        let enrollment = hid::make_credential_with_hmac(pin, &known_credential_ids)?;
        let cred = enrollment.credential;
        let cred_id_hex = hex::encode(&cred.credential_id);

        if config
            .keys
            .iter()
            .any(|k| k.credential_id_hex == cred_id_hex)
        {
            return Err(Fido2Error::DuplicateKey {
                label: format!("{label} (same physical key)"),
            });
        }

        let new_hmac = enrollment.hmac_secret;
        let mut new_key_real_shards: Vec<String> = Vec::new();

        // Prepare new shard collections for active keys
        let mut new_shards_per_key: Vec<Option<Vec<String>>> = vec![None; config.keys.len()];
        for &idx in &active_indices {
            new_shards_per_key[idx] = Some(Vec::new());
        }

        // For each compartment: split, encrypt tagged shards
        for (meta, master_key) in &compartments {
            let threshold = meta.threshold.min(total);
            let shards = crypto::split_master_key(master_key, threshold, total)?;

            // Re-encrypt active keys' shards for this compartment
            for (shard_idx, (key_idx, hmac)) in active_hmacs.iter().enumerate() {
                let encrypted = crypto::encrypt_shard_tagged(hmac, meta.id, &shards[shard_idx])?;
                new_shards_per_key[*key_idx]
                    .as_mut()
                    .unwrap()
                    .push(hex::encode(&encrypted));
            }

            // Encrypt new key's shard (last index in the split)
            let encrypted_new =
                crypto::encrypt_shard_tagged(&new_hmac, meta.id, &shards[active_indices.len()])?;
            new_key_real_shards.push(hex::encode(&encrypted_new));

            result_keys.push((meta.id, Zeroizing::new(**master_key)));
        }

        // Rebuild shard vecs: active keys get new shards, skipped keys keep existing
        for (i, key) in config.keys.iter_mut().enumerate() {
            if let Some(new_shards) = new_shards_per_key[i].take() {
                key.shards = Self::pad_shards(new_shards);
            }
        }

        let now = chrono::Utc::now().to_rfc3339();
        config.keys.push(RegisteredKey {
            label: label.into(),
            credential_id_hex: cred_id_hex,
            public_key_der_hex: hex::encode(&cred.public_key_der),
            public_key_pem: cred.public_key_pem,
            shards: Self::pad_shards(new_key_real_shards),
            registered_at: now,
        });

        config.total_shares = config.keys.len();
        self.persist_mutation(&mut config, context, "fido2.register", Some(label))?;

        Ok(RegisterResult {
            compartment_keys: result_keys,
            is_first_key: is_first,
            total_keys: config.keys.len(),
        })
    }

    /// Remove a registered key and re-split shards for remaining active keys.
    ///
    /// After removal, all compartments' shares are re-split using only the remaining
    /// active keys. The threshold must still be satisfiable by the remaining keys
    /// (checked during removal). Keys in `skip_labels` are kept in the config but
    /// not tapped (they're already revoked). This allows staged key retirement.
    #[cfg(feature = "hid")]
    pub fn remove_key(
        &self,
        label: &str,
        compartment_metas: &[(CompartmentMeta, &[u8; 32])],
        pin: Option<&str>,
        skip_labels: &[String],
    ) -> Result<(), Fido2Error> {
        self.remove_key_inner(label, compartment_metas, pin, skip_labels, None)
    }

    /// Remove a key and bind the resulting config write to a daemon operation journal.
    #[cfg(feature = "hid")]
    pub fn remove_key_for_operation(
        &self,
        label: &str,
        compartment_metas: &[(CompartmentMeta, &[u8; 32])],
        pin: Option<&str>,
        skip_labels: &[String],
        context: Fido2MutationContext<'_>,
    ) -> Result<(), Fido2Error> {
        self.remove_key_inner(label, compartment_metas, pin, skip_labels, Some(context))
    }

    #[cfg(feature = "hid")]
    fn remove_key_inner(
        &self,
        label: &str,
        compartment_metas: &[(CompartmentMeta, &[u8; 32])],
        pin: Option<&str>,
        skip_labels: &[String],
        context: Option<Fido2MutationContext<'_>>,
    ) -> Result<(), Fido2Error> {
        let _lease = self.acquire_writer_lease()?;
        let mut config = self.load()?;
        let pin = Self::normalize_pin(pin);

        let idx = config
            .keys
            .iter()
            .position(|k| k.label == label)
            .ok_or_else(|| Fido2Error::KeyNotFound {
                label: label.into(),
            })?;

        config.keys.remove(idx);

        if config.keys.is_empty() {
            config.total_shares = 0;
            self.persist_mutation(&mut config, context, "fido2.remove", Some(label))?;
            return Ok(());
        }

        // Compute active (non-skipped) remaining key indices
        let active_indices: Vec<usize> = (0..config.keys.len())
            .filter(|i| !skip_labels.contains(&config.keys[*i].label))
            .collect();
        let remaining_active = active_indices.len();

        // Quorum check against active keys only
        for (meta, _) in compartment_metas {
            if remaining_active < meta.threshold {
                return Err(Fido2Error::RemovalBelowQuorum {
                    remaining: remaining_active,
                    threshold: meta.threshold,
                });
            }
        }

        // Cache hmac-secrets for active remaining keys
        let mut active_hmacs = HmacSecrets(Vec::with_capacity(remaining_active));
        for &idx in &active_indices {
            let cred_id = hex::decode(&config.keys[idx].credential_id_hex)
                .map_err(|e| Fido2Error::Config(format!("bad credential_id hex: {e}")))?;
            let hmac = hid::get_hmac_secret(&cred_id, pin)?;
            active_hmacs.0.push((idx, hmac));
        }

        // Prepare new shard collections for active keys
        let mut new_shards_per_key: Vec<Option<Vec<String>>> = vec![None; config.keys.len()];
        for &idx in &active_indices {
            new_shards_per_key[idx] = Some(Vec::new());
        }

        // Re-split for active remaining keys, for each compartment
        for (meta, mk) in compartment_metas {
            let threshold = meta.threshold.min(remaining_active);
            let shards = crypto::split_master_key(mk, threshold, remaining_active)?;

            for (shard_idx, (key_idx, hmac)) in active_hmacs.iter().enumerate() {
                let encrypted = crypto::encrypt_shard_tagged(hmac, meta.id, &shards[shard_idx])?;
                new_shards_per_key[*key_idx]
                    .as_mut()
                    .unwrap()
                    .push(hex::encode(&encrypted));
            }
        }

        // Rebuild shard vecs: active keys get new shards, skipped keys keep existing
        for (i, key) in config.keys.iter_mut().enumerate() {
            if let Some(new_shards) = new_shards_per_key[i].take() {
                key.shards = Self::pad_shards(new_shards);
            }
        }

        config.total_shares = config.keys.len();
        self.persist_mutation(&mut config, context, "fido2.remove", Some(label))?;
        Ok(())
    }

    /// Register a poison pill FIDO2 key.
    ///
    /// The poison key receives random garbage shards (33 random bytes per compartment)
    /// instead of real Shamir shares. When tapped during unlock alongside real keys,
    /// the garbage shard corrupts Shamir reconstruction:
    /// - Correct reconstruction requires at least `threshold` shares
    /// - Providing one garbage + k-1 real shares → wrong polynomial interpolation
    /// - Result: derived master key is garbage
    /// - Effect: silent unlock failure (no data destroyed)
    ///
    /// **Use case**: Deniability and security theater. A poison key can be given to
    /// an adversary, who taps it during interrogation. The vault silently fails to unlock,
    /// appearing corrupted. The real keys remain safe elsewhere.
    ///
    /// **Constraints**: Cannot register as the first key (vault would be permanently
    /// inaccessible). Existing keys' shards are NOT re-split.
    #[cfg(feature = "hid")]
    pub fn register_key_poison(
        &self,
        pin: Option<&str>,
        label: &str,
        compartment_metas: &[CompartmentMeta],
    ) -> Result<usize, Fido2Error> {
        self.register_key_poison_inner(pin, label, compartment_metas, None)
    }

    /// Register a poison key and bind the config write to a daemon operation journal.
    #[cfg(feature = "hid")]
    pub fn register_key_poison_for_operation(
        &self,
        pin: Option<&str>,
        label: &str,
        compartment_metas: &[CompartmentMeta],
        context: Fido2MutationContext<'_>,
    ) -> Result<usize, Fido2Error> {
        self.register_key_poison_inner(pin, label, compartment_metas, Some(context))
    }

    #[cfg(feature = "hid")]
    fn register_key_poison_inner(
        &self,
        pin: Option<&str>,
        label: &str,
        compartment_metas: &[CompartmentMeta],
        context: Option<Fido2MutationContext<'_>>,
    ) -> Result<usize, Fido2Error> {
        let _lease = self.acquire_writer_lease()?;
        let mut config = self.load()?;
        let pin = Self::normalize_pin(pin);

        if config.keys.is_empty() {
            return Err(Fido2Error::Config(
                "cannot register poison key as first key — vault would be permanently inaccessible"
                    .into(),
            ));
        }

        if config.keys.iter().any(|k| k.label == label) {
            return Err(Fido2Error::DuplicateKey {
                label: label.into(),
            });
        }

        let known_credential_ids: Vec<Vec<u8>> = config
            .keys
            .iter()
            .map(|key| {
                hex::decode(&key.credential_id_hex)
                    .map_err(|e| Fido2Error::Config(format!("bad credential_id hex: {e}")))
            })
            .collect::<Result<_, _>>()?;
        let enrollment = hid::make_credential_with_hmac(pin, &known_credential_ids)?;
        let cred = enrollment.credential;
        let cred_id_hex = hex::encode(&cred.credential_id);

        if config
            .keys
            .iter()
            .any(|k| k.credential_id_hex == cred_id_hex)
        {
            return Err(Fido2Error::DuplicateKey {
                label: format!("{label} (same physical key)"),
            });
        }

        let new_hmac = enrollment.hmac_secret;

        // Step 3: Generate random shard data for each compartment.
        // Real Shamir shares are 33 bytes (1 byte x + 32 bytes share).
        // Poison shards are 33 random bytes — same length, garbage data.
        let mut poison_shards: Vec<String> = Vec::with_capacity(compartment_metas.len());
        for meta in compartment_metas {
            let mut fake_shard = [0u8; 33];
            OsRng.fill_bytes(&mut fake_shard);
            let encrypted = crypto::encrypt_shard_tagged(&new_hmac, meta.id, &fake_shard)?;
            poison_shards.push(hex::encode(&encrypted));
        }

        // DO NOT touch existing keys' shards
        let now = chrono::Utc::now().to_rfc3339();
        config.keys.push(RegisteredKey {
            label: label.into(),
            credential_id_hex: cred_id_hex,
            public_key_der_hex: hex::encode(&cred.public_key_der),
            public_key_pem: cred.public_key_pem,
            shards: Self::pad_shards(poison_shards),
            registered_at: now,
        });

        config.total_shares = config.keys.len();
        self.persist_mutation(&mut config, context, "fido2.register", Some(label))?;

        Ok(config.keys.len())
    }

    // ── Unlock ───────────────────────────────────────────────────

    /// Cascading unlock: tap N keys, discover and unlock all compartments with threshold ≤ N.
    ///
    /// **Phase 1**: Tap `target_taps` keys in rounds. Each round optionally uses
    /// the provided current PIN, then attempts all registered (non-skipped) keys.
    /// The first key that succeeds is used for that round. If no key matches, the
    /// round fails.
    ///
    /// **Phase 2**: For each tapped key, decrypt all its shard blobs. Dummy shards
    /// fail AEAD validation silently and are ignored. Real shards are grouped by
    /// compartment ID.
    ///
    /// **Phase 3**: For each compartment with collected shards, try Shamir reconstruction.
    /// If successful, verify the key by decrypting `meta.enc`. Include the compartment
    /// only if `meta.threshold ≤ target_taps` (cascading rule).
    ///
    /// **Returns**: `Vec<(CompartmentMeta, master_key)>` sorted by threshold ascending.
    /// Enables cascading unlock: as you tap more keys, you progressively unlock
    /// higher-threshold compartments.
    ///
    /// **Event stream**: Optional `event_tx` receives progress events (RoundStart,
    /// RoundComplete, CascadingUnlock, etc.) for UI feedback during the tap rounds.
    #[cfg(feature = "hid")]
    #[allow(clippy::type_complexity)]
    pub fn authenticate_cascading(
        &self,
        pins: &[String],
        target_taps: usize,
        base_dir: &Path,
        event_tx: Option<std::sync::mpsc::Sender<QuorumEvent>>,
    ) -> Result<Vec<(CompartmentMeta, Zeroizing<[u8; 32]>)>, Fido2Error> {
        let config = self.load()?;

        if config.keys.is_empty() {
            return Err(Fido2Error::NoKeysRegistered);
        }

        if config.keys.len() < target_taps {
            return Err(Fido2Error::QuorumNotMet {
                required: target_taps,
                available: config.keys.len(),
            });
        }

        let emit = |evt: QuorumEvent| {
            if let Some(tx) = &event_tx {
                let _ = tx.send(evt);
            }
        };

        // Phase 1: Tap keys, collect hmac-secrets
        let mut hmac_secrets = HmacSecretVec(Vec::with_capacity(target_taps));
        let mut tapped_keys: Vec<&RegisteredKey> = Vec::with_capacity(target_taps);
        let mut used_cred_ids: Vec<String> = Vec::new();

        for round in 0..target_taps {
            let pin = Self::pin_for_round(pins, round);

            emit(QuorumEvent::RoundStart {
                round: round + 1,
                total: target_taps,
            });

            let mut matched = false;
            let mut last_error = None;
            for key in &config.keys {
                if used_cred_ids.contains(&key.credential_id_hex) {
                    continue;
                }

                let cred_id = match hex::decode(&key.credential_id_hex) {
                    Ok(id) => id,
                    Err(_) => continue,
                };

                match hid::get_hmac_secret(&cred_id, pin) {
                    Ok(hmac) => {
                        hmac_secrets.0.push(hmac);
                        tapped_keys.push(key);
                        used_cred_ids.push(key.credential_id_hex.clone());

                        emit(QuorumEvent::RoundComplete {
                            round: round + 1,
                            total: target_taps,
                            key_label: key.label.clone(),
                        });

                        matched = true;
                        break;
                    }
                    Err(Fido2Error::NoMatchingCredential) => continue,
                    Err(error) => {
                        last_error = Some(error);
                        continue;
                    }
                }
            }

            if !matched {
                emit(QuorumEvent::Error {
                    message: "No matching key found for this round".into(),
                });
                return Err(last_error.unwrap_or(Fido2Error::NoMatchingCredential));
            }

            if round + 1 < target_taps {
                emit(QuorumEvent::SwapKeys {
                    round: round + 1,
                    total: target_taps,
                });
            }
        }

        // Phase 2: Decrypt all shard blobs from tapped keys, group by compartment
        // Key: compartment_id, Value: Vec<shard_data>
        let mut shards_by_comp: HashMap<usize, Vec<Vec<u8>>> = HashMap::new();

        for (key, hmac) in tapped_keys.iter().zip(hmac_secrets.iter()) {
            for shard_hex in &key.shards {
                let encrypted = match hex::decode(shard_hex) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                // Try to decrypt — dummy blobs will fail AEAD silently
                if let Ok((comp_id, shard_data)) = crypto::decrypt_shard_tagged(hmac, &encrypted) {
                    shards_by_comp.entry(comp_id).or_default().push(shard_data);
                }
            }
        }

        // Phase 3: For each discovered compartment, try Shamir reconstruct + verify meta
        let mut results: Vec<(CompartmentMeta, Zeroizing<[u8; 32]>)> = Vec::new();

        for (comp_id, shards) in &shards_by_comp {
            if shards.is_empty() {
                continue;
            }

            // Try reconstruction with available shards
            match crypto::reconstruct_master_key(shards) {
                Ok(master_key) => {
                    // Verify by trying to decrypt meta.enc
                    match Self::load_compartment_meta(base_dir, *comp_id, &master_key) {
                        Ok(meta) => {
                            // Only include if threshold ≤ target_taps (cascading rule)
                            if meta.threshold <= target_taps {
                                results.push((meta, master_key));
                            }
                        }
                        Err(_) => {
                            // meta.enc didn't decrypt — wrong key or dummy compartment
                            continue;
                        }
                    }
                }
                Err(_) => continue,
            }
        }

        // Sort by threshold ascending
        results.sort_by_key(|(meta, _)| meta.threshold);

        if !results.is_empty() {
            emit(QuorumEvent::CascadingUnlock {
                compartments_unlocked: results.len(),
            });
            emit(QuorumEvent::Unlocked);
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_manager() -> (Fido2Manager, TempDir) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fido2_keys.json");
        (Fido2Manager::new(path), dir)
    }

    #[test]
    fn status_empty() {
        let (mgr, _dir) = test_manager();
        let s = mgr.status().unwrap();
        assert!(!s.enabled);
        assert_eq!(s.key_count, 0);
    }

    #[test]
    fn list_keys_empty() {
        let (mgr, _dir) = test_manager();
        assert!(mgr.list_keys().unwrap().is_empty());
    }

    #[test]
    fn save_and_load_compartment_meta() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let meta = CompartmentMeta {
            id: 5,
            label: "test".into(),
            threshold: 2,
            passphrase_mode: None,
        };
        let mut mk = [0u8; 32];
        OsRng.fill_bytes(&mut mk);

        Fido2Manager::save_compartment_meta(base, &meta, &mk).unwrap();
        let loaded = Fido2Manager::load_compartment_meta(base, 5, &mk).unwrap();
        assert_eq!(loaded, meta);
    }

    #[test]
    fn meta_wrong_key_fails() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let meta = CompartmentMeta {
            id: 0,
            label: "hot".into(),
            threshold: 1,
            passphrase_mode: None,
        };
        let mut mk1 = [0u8; 32];
        let mut mk2 = [0u8; 32];
        OsRng.fill_bytes(&mut mk1);
        OsRng.fill_bytes(&mut mk2);

        Fido2Manager::save_compartment_meta(base, &meta, &mk1).unwrap();
        assert!(Fido2Manager::load_compartment_meta(base, 0, &mk2).is_err());
    }

    #[test]
    fn setup_dummy_directories_creates_100() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        Fido2Manager::setup_dummy_directories(base, &[0, 1]).unwrap();

        // All 100 dirs exist
        for i in 0..SHARD_SLOTS {
            let d = base.join("compartments").join(i.to_string());
            assert!(d.exists(), "directory {i} should exist");
        }

        // Real dirs (0, 1) should NOT have dummy meta.enc (they weren't populated)
        // Dummy dirs (2..100) should have meta.enc
        assert!(base.join("compartments/2/meta.enc").exists());
        assert!(base.join("compartments/99/meta.enc").exists());

        // Initialization marker exists
        assert!(base.join(".initialized").exists());
    }

    #[test]
    fn pad_shards_to_shard_slots() {
        let real = vec!["aabb".to_string(), "ccdd".to_string()];
        let padded = Fido2Manager::pad_shards(real);
        assert_eq!(padded.len(), SHARD_SLOTS);
    }

    #[test]
    fn status_with_keys() {
        let (mgr, _dir) = test_manager();
        let config = Fido2Config {
            total_shares: 2,
            keys: vec![RegisteredKey {
                label: "key1".into(),
                credential_id_hex: "aabb".into(),
                public_key_der_hex: "ccdd".into(),
                public_key_pem: "pem".into(),
                shards: vec!["ff".into(); SHARD_SLOTS],
                registered_at: "2026-01-01".into(),
            }],
            ..Default::default()
        };
        mgr.save_config_raw(&config).unwrap();

        let s = mgr.status().unwrap();
        assert!(s.enabled);
        assert_eq!(s.key_count, 1);
    }

    #[test]
    fn malformed_config_surfaces_as_error() {
        let (mgr, dir) = test_manager();
        std::fs::write(dir.path().join("fido2_keys.json"), "{not json").unwrap();

        assert!(matches!(mgr.status(), Err(Fido2Error::Config(_))));
        assert!(matches!(mgr.list_keys(), Err(Fido2Error::Config(_))));
    }

    #[test]
    fn writer_lease_rejects_a_second_manager_and_releases_on_drop() {
        let (first, dir) = test_manager();
        let second = Fido2Manager::new(dir.path().join("fido2_keys.json"));
        let lease = first.acquire_writer_lease().unwrap();

        assert!(matches!(
            second.save_config_raw(&Fido2Config::default()),
            Err(Fido2Error::WriterBusy { .. })
        ));

        drop(lease);
        second.save_config_raw(&Fido2Config::default()).unwrap();
    }

    #[test]
    fn raw_manager_writes_advance_generation() {
        let (mgr, _dir) = test_manager();

        mgr.save_config_raw(&Fido2Config::default()).unwrap();
        mgr.save_config_raw(&Fido2Config::default()).unwrap();

        let config = mgr.load_config_raw().unwrap();
        assert_eq!(config.generation, 2);
        assert_eq!(
            config
                .last_mutation
                .as_ref()
                .map(|receipt| receipt.kind.as_str()),
            Some("fido2.raw-save")
        );
    }

    #[cfg(not(feature = "hid"))]
    #[test]
    fn pin_setup_reports_disabled_hid_feature() {
        let (mgr, _dir) = test_manager();

        assert!(matches!(
            mgr.set_new_pin("1234"),
            Err(Fido2Error::Other(message))
                if message == "FIDO2 HID support is disabled in this build"
        ));
    }
}
