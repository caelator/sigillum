//! Persistent storage for the transaction queue.
//!
//! Holds [`QueueJob`] records — native sweeps, ERC-20 transfers, and other
//! on-chain operations — in a versioned [`JsonDocument`]. The queue processor
//! drains jobs with exponential backoff and updates state in place.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sigillum_api::QueueJob;

use crate::json_store::{JsonDocument, JsonSchema};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QueueState {
    #[serde(default)]
    pub jobs: Vec<QueueJob>,
}

impl JsonDocument for QueueState {
    const SCHEMA: JsonSchema = JsonSchema::new("sigillum.queue", 2);

    fn from_enveloped_json(
        path: &Path,
        version: u32,
        data: serde_json::Value,
    ) -> Result<Self, std::io::Error> {
        if version != 1 && version != Self::SCHEMA.version {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "unsupported {} schema version {} in {}; expected 1 or {}",
                    Self::SCHEMA.name,
                    version,
                    path.display(),
                    Self::SCHEMA.version
                ),
            ));
        }

        serde_json::from_value(data).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "failed to parse {} schema payload {}: {error}",
                    Self::SCHEMA.name,
                    path.display()
                ),
            )
        })
    }
}

pub fn load_queue(base_dir: &std::path::Path) -> Result<QueueState, std::io::Error> {
    let path = queue_path(base_dir);
    Ok(crate::json_store::load_json_document(&path)?.unwrap_or_default())
}

pub fn save_queue(base_dir: &std::path::Path, queue: &QueueState) -> Result<(), std::io::Error> {
    let path = queue_path(base_dir);
    crate::json_store::save_json_document(&path, queue)
}

pub fn queue_path(base_dir: &std::path::Path) -> PathBuf {
    base_dir.join("queue.json")
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use sigillum_api::QueueJobPayload;

    fn sample_job() -> QueueJob {
        QueueJob {
            id: "job_1".into(),
            state: "queued".into(),
            attempts: 0,
            created_at_unix: 1,
            updated_at_unix: 1,
            next_attempt_after_unix: None,
            payload: QueueJobPayload::EthStealthNativeSweep {
                wallet_profile: "wallet-a".into(),
                stealth_address: "0x0000000000000000000000000000000000000001".into(),
                ephemeral_public_key_hex: "0x02".into(),
                destination_address: Some("0x0000000000000000000000000000000000000002".into()),
                min_value_wei_hex: None,
                gas_limit: None,
                view_tag_hex: None,
            },
            last_error: None,
            transaction_hash_hex: None,
            broadcast_transaction_hash_hex: None,
        }
    }

    #[test]
    fn load_returns_default_when_no_file() {
        let dir = TempDir::new().unwrap();
        assert!(load_queue(dir.path()).unwrap().jobs.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let mut state = QueueState::default();
        state.jobs.push(sample_job());

        save_queue(dir.path(), &state).unwrap();
        let loaded = load_queue(dir.path()).unwrap();

        assert_eq!(loaded.jobs.len(), 1);
        assert_eq!(loaded.jobs[0].id, "job_1");
    }

    #[test]
    fn backup_restores_deleted_live_file() {
        let dir = TempDir::new().unwrap();
        let mut state = QueueState::default();
        state.jobs.push(sample_job());

        save_queue(dir.path(), &state).unwrap();
        std::fs::remove_file(queue_path(dir.path())).unwrap();

        let loaded = load_queue(dir.path()).unwrap();
        assert_eq!(loaded.jobs.len(), 1);
        assert!(queue_path(dir.path()).exists());
    }

    #[test]
    fn save_writes_versioned_schema_envelope() {
        let dir = TempDir::new().unwrap();
        let mut state = QueueState::default();
        state.jobs.push(sample_job());

        save_queue(dir.path(), &state).unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(queue_path(dir.path())).unwrap()).unwrap();
        assert_eq!(saved["schema"], json!("sigillum.queue"));
        assert_eq!(saved["schema_version"], json!(2));
        assert!(saved["data"]["jobs"].is_array());
    }

    #[test]
    fn version_1_queue_envelope_still_loads_without_rewriting_jobs() {
        let dir = TempDir::new().unwrap();
        let path = queue_path(dir.path());
        let queue = json!({
            "schema": "sigillum.queue",
            "schema_version": 1,
            "data": {
                "jobs": [
                    sample_job(),
                    {
                        "id": "job_deferred",
                        "state": "deferred",
                        "attempts": 1,
                        "created_at_unix": 1,
                        "updated_at_unix": 2,
                        "kind": "eth_stealth_native_sweep",
                        "wallet_profile": "wallet-a",
                        "stealth_address": "0x0000000000000000000000000000000000000001",
                        "ephemeral_public_key_hex": "0x02",
                        "destination_address": "0x0000000000000000000000000000000000000002"
                    },
                    {
                        "id": "job_operator",
                        "state": "operator_action_required",
                        "attempts": 1,
                        "created_at_unix": 1,
                        "updated_at_unix": 2,
                        "kind": "eth_stealth_native_sweep",
                        "wallet_profile": "wallet-a",
                        "stealth_address": "0x0000000000000000000000000000000000000001",
                        "ephemeral_public_key_hex": "0x02",
                        "destination_address": "0x0000000000000000000000000000000000000002",
                        "last_error": "operator review required"
                    }
                ]
            }
        });
        std::fs::write(&path, serde_json::to_vec_pretty(&queue).unwrap()).unwrap();

        let loaded = load_queue(dir.path()).unwrap();
        let states = loaded
            .jobs
            .iter()
            .map(|job| job.state.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            states,
            vec!["queued", "deferred", "operator_action_required"]
        );

        save_queue(dir.path(), &loaded).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(queue_path(dir.path())).unwrap()).unwrap();
        assert_eq!(saved["schema_version"], json!(2));
        assert_eq!(saved["data"]["jobs"][1]["state"], json!("deferred"));
        assert_eq!(
            saved["data"]["jobs"][2]["state"],
            json!("operator_action_required")
        );
    }

    #[test]
    fn legacy_unwrapped_queue_still_loads() {
        let dir = TempDir::new().unwrap();
        let path = queue_path(dir.path());
        let mut state = QueueState::default();
        state.jobs.push(sample_job());
        std::fs::write(&path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();

        let loaded = load_queue(dir.path()).unwrap();
        assert_eq!(loaded.jobs.len(), 1);
    }
}
