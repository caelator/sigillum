//! Persistent storage for local ERC-20 token registry imports.
//!
//! Token registries are operator-provided local inputs only. The daemon stores
//! imported lists separately from wallet inventory so D-15 registry state can
//! evolve without changing the main inventory schema.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sigillum_api::TokenRegistryList;

use crate::json_store::{JsonDocument, JsonSchema};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TokenRegistryState {
    #[serde(default)]
    pub lists: Vec<TokenRegistryList>,
}

impl JsonDocument for TokenRegistryState {
    const SCHEMA: JsonSchema = JsonSchema::new("sigillum.token-registry", 1);
}

pub fn load_token_registry(
    base_dir: &std::path::Path,
) -> Result<TokenRegistryState, std::io::Error> {
    let path = token_registry_path(base_dir);
    Ok(crate::json_store::load_json_document(&path)?.unwrap_or_default())
}

pub fn save_token_registry(
    base_dir: &std::path::Path,
    state: &TokenRegistryState,
) -> Result<(), std::io::Error> {
    let path = token_registry_path(base_dir);
    crate::json_store::save_json_document(&path, state)
}

pub fn token_registry_path(base_dir: &std::path::Path) -> PathBuf {
    base_dir.join("token_registry.json")
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use sigillum_api::TokenRegistryEntry;
    use tempfile::TempDir;

    use super::*;

    fn sample_list() -> TokenRegistryList {
        TokenRegistryList {
            id: "registry_1".into(),
            name: "core-list".into(),
            compartment_id: 0,
            source: "pasted-json".into(),
            entries: vec![TokenRegistryEntry {
                chain_id: 1,
                address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                symbol: "AAA".into(),
                decimals: 18,
            }],
            created_at_unix: 1,
            updated_at_unix: 2,
        }
    }

    #[test]
    fn load_returns_default_when_no_file() {
        let dir = TempDir::new().unwrap();

        let state = load_token_registry(dir.path()).unwrap();

        assert!(state.lists.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let state = TokenRegistryState {
            lists: vec![sample_list()],
        };

        save_token_registry(dir.path(), &state).unwrap();
        let loaded = load_token_registry(dir.path()).unwrap();

        assert_eq!(loaded.lists.len(), 1);
        assert_eq!(loaded.lists[0].name, "core-list");
        assert_eq!(loaded.lists[0].entries.len(), 1);
        assert_eq!(loaded.lists[0].entries[0].symbol, "AAA");
    }

    #[test]
    fn save_writes_versioned_schema_envelope() {
        let dir = TempDir::new().unwrap();
        let state = TokenRegistryState::default();

        save_token_registry(dir.path(), &state).unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(token_registry_path(dir.path())).unwrap())
                .unwrap();
        assert_eq!(saved["schema"], json!("sigillum.token-registry"));
        assert_eq!(saved["schema_version"], json!(1));
        assert!(saved["data"]["lists"].is_array());
    }

    #[test]
    fn corrupt_live_file_is_quarantined_and_restored_from_backup() {
        let dir = TempDir::new().unwrap();
        let state = TokenRegistryState {
            lists: vec![sample_list()],
        };

        save_token_registry(dir.path(), &state).unwrap();
        std::fs::write(token_registry_path(dir.path()), b"not json {{{").unwrap();

        let loaded = load_token_registry(dir.path()).unwrap();

        assert_eq!(loaded.lists.len(), 1);
        assert_eq!(loaded.lists[0].name, "core-list");

        let corrupt_files = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("token_registry.json.corrupt-")
            })
            .count();
        assert_eq!(corrupt_files, 1);
    }

    #[test]
    fn future_schema_version_is_rejected() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            token_registry_path(dir.path()),
            serde_json::to_vec_pretty(&json!({
                "schema": "sigillum.token-registry",
                "schema_version": 9,
                "data": { "lists": [] }
            }))
            .unwrap(),
        )
        .unwrap();

        let error = load_token_registry(dir.path()).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("unsupported"));
    }
}
