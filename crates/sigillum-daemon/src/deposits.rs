//! Persistent storage for tracked stealth deposits.
//!
//! Wraps [`EthStealthDeposit`] records in a versioned [`JsonDocument`] so they
//! survive daemon restarts with automatic backup-on-schema-change recovery.
//!
//! ## Schema v3: stealth hash convention stamping
//!
//! Version 3 added `stealth_hash_convention` to every record (see
//! `docs/architecture.md` — the ERC-5564 shared-secret hash convention switch).
//! Records written before the switch (schema v1/v2 and pre-envelope legacy
//! files) were ALL created with the legacy x-only convention, so the migration
//! stamps them `x32` unconditionally. New records are written with the
//! standard `compressed33` convention. If a v3 record ever lacks the field
//! (corrupt or hand-edited store), serde defaults it to the standard
//! convention and detection re-probes both conventions on the next scan/check,
//! correcting the stamp on match (documented fail-safe: signing verifies the
//! derived address, so a wrong stamp can never produce a wrong key).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sigillum_api::EthStealthDeposit;
use sigillum_core::StealthHashConvention;

use crate::json_store::{JsonDocument, JsonSchema};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DepositState {
    #[serde(default)]
    pub eth_stealth: Vec<EthStealthDeposit>,
}

impl DepositState {
    fn parse(path: &std::path::Path, data: serde_json::Value) -> Result<Self, std::io::Error> {
        serde_json::from_value(data).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "failed to parse sigillum.deposits schema payload {}: {error}",
                    path.display()
                ),
            )
        })
    }
}

/// Stamp every record with the legacy convention. Only applied to stores
/// written before the convention switch (schema v1/v2 and legacy unwrapped
/// files), whose records were all created with the x-only hash.
fn stamp_pre_switch_records_legacy(state: &mut DepositState) {
    for deposit in &mut state.eth_stealth {
        deposit.stealth_hash_convention = StealthHashConvention::LEGACY;
    }
}

impl JsonDocument for DepositState {
    const SCHEMA: JsonSchema = JsonSchema::new("sigillum.deposits", 3);

    fn from_enveloped_json(
        path: &std::path::Path,
        version: u32,
        data: serde_json::Value,
    ) -> Result<Self, std::io::Error> {
        match version {
            1 | 2 => {
                let mut state = Self::parse(path, data)?;
                stamp_pre_switch_records_legacy(&mut state);
                Ok(state)
            }
            3 => Self::parse(path, data),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "unsupported sigillum.deposits schema version {} in {}; expected {}",
                    version,
                    path.display(),
                    Self::SCHEMA.version
                ),
            )),
        }
    }

    fn from_legacy_json(
        path: &std::path::Path,
        value: serde_json::Value,
    ) -> Result<Self, std::io::Error> {
        let mut state = Self::parse(path, value)?;
        stamp_pre_switch_records_legacy(&mut state);
        Ok(state)
    }
}

pub fn load_deposits(base_dir: &std::path::Path) -> Result<DepositState, std::io::Error> {
    let path = deposits_path(base_dir);
    Ok(crate::json_store::load_json_document(&path)?.unwrap_or_default())
}

pub fn save_deposits(
    base_dir: &std::path::Path,
    deposits: &DepositState,
) -> Result<(), std::io::Error> {
    let path = deposits_path(base_dir);
    crate::json_store::save_json_document(&path, deposits)
}

pub fn deposits_path(base_dir: &std::path::Path) -> PathBuf {
    base_dir.join("deposits.json")
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    fn sample_deposit() -> EthStealthDeposit {
        EthStealthDeposit {
            id: "dep_1".into(),
            status: "pending".into(),
            asset_kind: "native".into(),
            wallet_profile: "wallet-a".into(),
            chain_id: 1,
            chain_id_assumed: false,
            wallet_compartment_id: 0,
            provider_compartment_id: 0,
            wallet: "wallet-a".into(),
            short_name: "eth".into(),
            stealth_meta_address: "st:eth:example".into(),
            stealth_address: "0x0000000000000000000000000000000000000001".into(),
            ephemeral_public_key_hex: "0x02".into(),
            view_tag_hex: "0xaa".into(),
            stealth_hash_convention: StealthHashConvention::STANDARD,
            announcement: None,
            token_address: None,
            expected_amount_hex: None,
            observed_amount_hex: None,
            observed_native_balance_wei_hex: None,
            auto_queue_sweep: false,
            sweep_destination_address: None,
            min_sweep_amount_hex: None,
            queue_job_id: None,
            queue_job_state: None,
            note: None,
            created_at_unix: 1,
            updated_at_unix: 1,
            last_checked_at_unix: None,
            broadcast_transaction_hash_hex: None,
            counterparty_id: None,
        }
    }

    #[test]
    fn load_returns_default_when_no_file() {
        let dir = TempDir::new().unwrap();
        assert!(load_deposits(dir.path()).unwrap().eth_stealth.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let mut state = DepositState::default();
        state.eth_stealth.push(sample_deposit());

        save_deposits(dir.path(), &state).unwrap();
        let loaded = load_deposits(dir.path()).unwrap();

        assert_eq!(loaded.eth_stealth.len(), 1);
        assert_eq!(loaded.eth_stealth[0].id, "dep_1");
    }

    #[test]
    fn backup_restores_deleted_live_file() {
        let dir = TempDir::new().unwrap();
        let mut state = DepositState::default();
        state.eth_stealth.push(sample_deposit());

        save_deposits(dir.path(), &state).unwrap();
        std::fs::remove_file(deposits_path(dir.path())).unwrap();

        let loaded = load_deposits(dir.path()).unwrap();
        assert_eq!(loaded.eth_stealth.len(), 1);
        assert!(deposits_path(dir.path()).exists());
    }

    #[test]
    fn save_writes_versioned_schema_envelope() {
        let dir = TempDir::new().unwrap();
        let mut state = DepositState::default();
        state.eth_stealth.push(sample_deposit());

        save_deposits(dir.path(), &state).unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(deposits_path(dir.path())).unwrap()).unwrap();
        assert_eq!(saved["schema"], json!("sigillum.deposits"));
        assert_eq!(saved["schema_version"], json!(3));
        assert!(saved["data"]["eth_stealth"].is_array());
        assert_eq!(
            saved["data"]["eth_stealth"][0]["stealth_hash_convention"],
            json!("compressed33")
        );
    }

    #[test]
    fn legacy_unwrapped_deposits_still_load() {
        let dir = TempDir::new().unwrap();
        let path = deposits_path(dir.path());
        let mut state = DepositState::default();
        state.eth_stealth.push(sample_deposit());
        std::fs::write(&path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();

        let loaded = load_deposits(dir.path()).unwrap();
        assert_eq!(loaded.eth_stealth.len(), 1);
        // Pre-envelope files predate the convention switch: stamped legacy.
        assert_eq!(
            loaded.eth_stealth[0].stealth_hash_convention,
            StealthHashConvention::LEGACY
        );
    }

    #[test]
    fn legacy_v1_deposits_load_with_assumed_mainnet_chain_id() {
        let dir = TempDir::new().unwrap();
        let path = deposits_path(dir.path());
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "schema": "sigillum.deposits",
                "schema_version": 1,
                "data": {
                    "eth_stealth": [{
                        "id": "dep_1",
                        "status": "pending",
                        "asset_kind": "native",
                        "wallet_profile": "wallet-a",
                        "wallet_compartment_id": 0,
                        "provider_compartment_id": 0,
                        "wallet": "wallet-a",
                        "short_name": "eth",
                        "stealth_meta_address": "st:eth:example",
                        "stealth_address": "0x0000000000000000000000000000000000000001",
                        "ephemeral_public_key_hex": "0x02",
                        "view_tag_hex": "0xaa",
                        "auto_queue_sweep": false,
                        "created_at_unix": 1,
                        "updated_at_unix": 1
                    }]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let loaded = load_deposits(dir.path()).unwrap();

        assert_eq!(loaded.eth_stealth[0].chain_id, 1);
        assert!(loaded.eth_stealth[0].chain_id_assumed);
        assert_eq!(
            loaded.eth_stealth[0].stealth_hash_convention,
            StealthHashConvention::LEGACY
        );
    }

    #[test]
    fn v2_deposits_migrate_with_legacy_convention_stamp() {
        let dir = TempDir::new().unwrap();
        let path = deposits_path(dir.path());
        // A v2 store (pre-convention-switch) has no convention field; every
        // record was created with the legacy x-only hash and must be stamped
        // `x32` so sweeps keep deriving the right key.
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "schema": "sigillum.deposits",
                "schema_version": 2,
                "data": {
                    "eth_stealth": [{
                        "id": "dep_1",
                        "status": "pending",
                        "asset_kind": "native",
                        "wallet_profile": "wallet-a",
                        "chain_id": 1,
                        "wallet_compartment_id": 0,
                        "provider_compartment_id": 0,
                        "wallet": "wallet-a",
                        "short_name": "eth",
                        "stealth_meta_address": "st:eth:example",
                        "stealth_address": "0x0000000000000000000000000000000000000001",
                        "ephemeral_public_key_hex": "0x02",
                        "view_tag_hex": "0xaa",
                        "auto_queue_sweep": false,
                        "created_at_unix": 1,
                        "updated_at_unix": 1
                    }]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let loaded = load_deposits(dir.path()).unwrap();

        assert_eq!(
            loaded.eth_stealth[0].stealth_hash_convention,
            StealthHashConvention::LEGACY
        );
        // Persisting after migration writes the v3 envelope with the stamp.
        save_deposits(dir.path(), &loaded).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(deposits_path(dir.path())).unwrap()).unwrap();
        assert_eq!(saved["schema_version"], json!(3));
        assert_eq!(
            saved["data"]["eth_stealth"][0]["stealth_hash_convention"],
            json!("x32")
        );
    }

    #[test]
    fn v3_deposits_keep_their_stored_convention() {
        let dir = TempDir::new().unwrap();
        let mut state = DepositState::default();
        let mut legacy = sample_deposit();
        legacy.stealth_hash_convention = StealthHashConvention::LEGACY;
        state.eth_stealth.push(legacy);
        state.eth_stealth.push(sample_deposit());

        save_deposits(dir.path(), &state).unwrap();
        let loaded = load_deposits(dir.path()).unwrap();

        assert_eq!(
            loaded.eth_stealth[0].stealth_hash_convention,
            StealthHashConvention::LEGACY
        );
        assert_eq!(
            loaded.eth_stealth[1].stealth_hash_convention,
            StealthHashConvention::STANDARD
        );
    }

    #[test]
    fn v3_record_missing_convention_defaults_to_standard_for_reprobe() {
        let dir = TempDir::new().unwrap();
        let path = deposits_path(dir.path());
        let mut state = DepositState::default();
        state.eth_stealth.push(sample_deposit());
        save_deposits(dir.path(), &state).unwrap();

        // Simulate corruption/hand-editing: strip the field from the v3 file.
        let mut saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        saved["data"]["eth_stealth"][0]
            .as_object_mut()
            .unwrap()
            .remove("stealth_hash_convention");
        std::fs::write(&path, serde_json::to_vec_pretty(&saved).unwrap()).unwrap();

        // Documented fail-safe: the record loads with the standard default;
        // detection re-probes both conventions and corrects the stamp.
        let loaded = load_deposits(dir.path()).unwrap();
        assert_eq!(
            loaded.eth_stealth[0].stealth_hash_convention,
            StealthHashConvention::STANDARD
        );
    }
}
