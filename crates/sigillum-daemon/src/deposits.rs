//! Persistent storage for tracked stealth deposits.
//!
//! Wraps [`EthStealthDeposit`] records in a versioned [`JsonDocument`] so they
//! survive daemon restarts with automatic backup-on-schema-change recovery.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sigillum_api::EthStealthDeposit;

use crate::json_store::{JsonDocument, JsonSchema};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DepositState {
    #[serde(default)]
    pub eth_stealth: Vec<EthStealthDeposit>,
}

impl JsonDocument for DepositState {
    const SCHEMA: JsonSchema = JsonSchema::new("sigillum.deposits", 1);
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
            wallet_compartment_id: 0,
            provider_compartment_id: 0,
            wallet: "wallet-a".into(),
            short_name: "eth".into(),
            stealth_meta_address: "st:eth:example".into(),
            stealth_address: "0x0000000000000000000000000000000000000001".into(),
            ephemeral_public_key_hex: "0x02".into(),
            view_tag_hex: "0xaa".into(),
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
        assert_eq!(saved["schema_version"], json!(1));
        assert!(saved["data"]["eth_stealth"].is_array());
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
    }
}
