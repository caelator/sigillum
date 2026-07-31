use std::collections::HashSet;

use serde_json::Value;
use sigillum_api::{
    EvmProviderProfile, TokenRegistryDeleteRequest, TokenRegistryEntry, TokenRegistryImportRequest,
    TokenRegistryList, TokenRegistryListResponse, TokenRegistryMutationResponse,
    WalletAssetHolding, WalletAssetKind,
};

use crate::audit_log::AuditEventSpec;
use crate::service::evm::normalize_address;
use crate::service::helpers::{now_unix, random_id};
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::support;
use super::wallet_selection::DiscoveryWallet;

const MAX_TOKEN_REGISTRY_JSON_BYTES: usize = 1_000_000;
const MAX_TOKEN_REGISTRY_ENTRIES: usize = 2_000;
const TOKEN_REGISTRY_SOURCE_PASTED_JSON: &str = "pasted-json";
const TOKEN_REGISTRY_SOURCE_LOCAL_FILE: &str = "local-file";

pub(super) fn token_registry_source(list_name: &str) -> String {
    format!("token_registry:{list_name}")
}

impl SigillumService {
    pub(crate) fn list_token_registry(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<TokenRegistryListResponse> {
        let token = self.require_session(token)?;
        let compartment_id = active_compartment_id(self, token)?;
        let state = load_token_registry_state(self)?;
        Ok(TokenRegistryListResponse {
            lists: state
                .lists
                .into_iter()
                .filter(|list| list.compartment_id == compartment_id)
                .collect(),
        })
    }

    pub(crate) async fn import_token_registry(
        &self,
        token: Option<&str>,
        body: TokenRegistryImportRequest,
    ) -> ServiceResult<TokenRegistryMutationResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let compartment_id = active_compartment_id(self, token)?;
        let name = support::trimmed_required("name", &body.name)?;
        let (payload, source) = token_registry_payload(body.entries_json, body.file_path)?;
        let entries = validated_token_registry_entries(parse_token_registry_entries(&payload)?)?;

        let mut state = load_token_registry_state(self)?;
        let now = now_unix();
        let list = if let Some(existing) = state.lists.iter_mut().find(|existing| {
            existing.compartment_id == compartment_id && existing.name.eq_ignore_ascii_case(&name)
        }) {
            existing.name = name.clone();
            existing.source = source.to_string();
            existing.entries = entries;
            existing.updated_at_unix = now;
            existing.clone()
        } else {
            let list = TokenRegistryList {
                id: random_id(),
                name: name.clone(),
                compartment_id,
                source: source.to_string(),
                entries,
                created_at_unix: now,
                updated_at_unix: now,
            };
            state.lists.push(list.clone());
            list
        };

        save_token_registry_state(self, &state)?;
        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryTokenRegistryImport {
                name: list.name.clone(),
                entries: list.entries.len(),
            },
        )?;

        Ok(TokenRegistryMutationResponse {
            status: "imported".into(),
            list,
        })
    }

    pub(crate) async fn delete_token_registry_list(
        &self,
        token: Option<&str>,
        body: TokenRegistryDeleteRequest,
    ) -> ServiceResult<TokenRegistryMutationResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let compartment_id = active_compartment_id(self, token)?;
        let name = support::trimmed_required("name", &body.name)?;
        let mut state = load_token_registry_state(self)?;
        let position = state
            .lists
            .iter()
            .position(|list| {
                list.compartment_id == compartment_id && list.name.eq_ignore_ascii_case(&name)
            })
            .ok_or_else(|| ServiceError::not_found("Token registry list not found."))?;
        let list = state.lists.remove(position);

        save_token_registry_state(self, &state)?;
        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryTokenRegistryDelete {
                name: list.name.clone(),
            },
        )?;

        Ok(TokenRegistryMutationResponse {
            status: "deleted".into(),
            list,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn probe_token_registry_for_address(
        &self,
        wallet: &DiscoveryWallet,
        provider: &EvmProviderProfile,
        address: &str,
        derivation_path: &str,
        block_tag: &str,
        config: &TokenRegistryProbeConfig,
        existing_holdings: &[WalletAssetHolding],
        now: u64,
    ) -> ServiceResult<Vec<WalletAssetHolding>> {
        let mut holdings = Vec::new();
        let context = support::InventoryRecordContext {
            wallet,
            provider,
            address,
            derivation_path,
            now,
        };

        for entry in &config.entries {
            if entry.chain_id != provider.chain_id {
                continue;
            }
            if existing_holdings
                .iter()
                .chain(holdings.iter())
                .any(|holding| holding_has_erc20_token(holding, &entry.token_address))
            {
                continue;
            }

            let amount_hex = self
                .evm_erc20_balance_for_provider(
                    provider.compartment_id,
                    provider,
                    &entry.token_address,
                    address,
                    block_tag,
                )
                .await?;
            if support::quantity_hex_is_nonzero(&amount_hex) {
                let source = token_registry_source(&entry.list_name);
                holdings.push(support::holding_record_with_source(
                    &context,
                    WalletAssetKind::Erc20,
                    Some(entry.token_address.clone()),
                    &amount_hex,
                    &source,
                ));
            }
        }

        Ok(holdings)
    }
}

#[derive(Debug)]
pub(super) struct TokenRegistryProbeEntry {
    pub(super) list_name: String,
    pub(super) chain_id: u64,
    pub(super) token_address: String,
}

#[derive(Debug)]
pub(super) struct TokenRegistryProbeConfig {
    pub(super) entries: Vec<TokenRegistryProbeEntry>,
}

pub(super) fn token_registry_probe_config(
    enabled: Option<bool>,
    lists: &[TokenRegistryList],
) -> ServiceResult<Option<TokenRegistryProbeConfig>> {
    if enabled != Some(true) {
        return Ok(None);
    }

    let entries: Vec<TokenRegistryProbeEntry> = lists
        .iter()
        .flat_map(|list| {
            list.entries
                .iter()
                .map(move |entry| TokenRegistryProbeEntry {
                    list_name: list.name.clone(),
                    chain_id: entry.chain_id,
                    token_address: entry.address.clone(),
                })
        })
        .collect();

    if entries.is_empty() {
        return Err(ServiceError::bad_request(
            "probe_token_registry is enabled but no token registry lists are imported for the active compartment",
        ));
    }

    Ok(Some(TokenRegistryProbeConfig { entries }))
}

fn active_compartment_id(service: &SigillumService, token: &str) -> ServiceResult<usize> {
    service
        .state
        .active_compartment_id_for(token)
        .ok_or_else(|| ServiceError::vault_locked("No active compartment."))
}

fn load_token_registry_state(
    service: &SigillumService,
) -> ServiceResult<crate::token_registry::TokenRegistryState> {
    crate::token_registry::load_token_registry(&service.state.base_dir)
        .map_err(|error| ServiceError::internal(format!("Failed to load token registry: {error}")))
}

fn save_token_registry_state(
    service: &SigillumService,
    state: &crate::token_registry::TokenRegistryState,
) -> ServiceResult<()> {
    crate::token_registry::save_token_registry(&service.state.base_dir, state)
        .map_err(|error| ServiceError::internal(format!("Failed to save token registry: {error}")))
}

fn token_registry_payload(
    entries_json: Option<String>,
    file_path: Option<String>,
) -> ServiceResult<(String, &'static str)> {
    match (entries_json, file_path) {
        (Some(_), Some(_)) | (None, None) => Err(ServiceError::bad_request(
            "Provide exactly one of entries_json or file_path.",
        )),
        (Some(payload), None) => {
            if payload.len() > MAX_TOKEN_REGISTRY_JSON_BYTES {
                return Err(ServiceError::bad_request(format!(
                    "entries_json exceeds maximum length of {MAX_TOKEN_REGISTRY_JSON_BYTES} bytes"
                )));
            }
            Ok((payload, TOKEN_REGISTRY_SOURCE_PASTED_JSON))
        }
        (None, Some(path)) => {
            let path = support::trimmed_required("file_path", &path)?;
            if is_network_token_registry_path(&path) {
                return Err(ServiceError::bad_request(
                    "Token registry lists are never fetched over the network (D-15); import a local file or paste JSON.",
                ));
            }
            let payload = std::fs::read_to_string(&path).map_err(|error| {
                ServiceError::bad_request(format!(
                    "Failed to read token registry file {path}: {error}"
                ))
            })?;
            if payload.len() > MAX_TOKEN_REGISTRY_JSON_BYTES {
                return Err(ServiceError::bad_request(format!(
                    "token registry file exceeds maximum length of {MAX_TOKEN_REGISTRY_JSON_BYTES} bytes"
                )));
            }
            Ok((payload, TOKEN_REGISTRY_SOURCE_LOCAL_FILE))
        }
    }
}

fn is_network_token_registry_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("ipfs://")
        || lower.starts_with("ftp://")
}

fn parse_token_registry_entries(payload: &str) -> ServiceResult<Vec<TokenRegistryEntry>> {
    let value: Value = serde_json::from_str(payload).map_err(|error| {
        ServiceError::bad_request(format!("Failed to parse token registry JSON: {error}"))
    })?;
    let entries = if value.is_array() {
        value
    } else if let Some(tokens) = value.get("tokens") {
        tokens.clone()
    } else {
        return Err(ServiceError::bad_request(
            "Token registry JSON must be an array of entries or an object with a tokens array.",
        ));
    };

    serde_json::from_value(entries).map_err(|error| {
        ServiceError::bad_request(format!("Failed to parse token registry entries: {error}"))
    })
}

fn validated_token_registry_entries(
    entries: Vec<TokenRegistryEntry>,
) -> ServiceResult<Vec<TokenRegistryEntry>> {
    if entries.is_empty() {
        return Err(ServiceError::bad_request(
            "token registry must contain at least 1 entry",
        ));
    }
    if entries.len() > MAX_TOKEN_REGISTRY_ENTRIES {
        return Err(ServiceError::bad_request(format!(
            "token registry exceeds maximum length of {MAX_TOKEN_REGISTRY_ENTRIES} entries"
        )));
    }

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (index, mut entry) in entries.into_iter().enumerate() {
        if entry.chain_id == 0 {
            return Err(ServiceError::bad_request(format!(
                "entries[{index}].chain_id must be greater than 0"
            )));
        }
        let address = normalize_address(&entry.address).map_err(|_| {
            ServiceError::bad_request(format!(
                "entries[{index}].address must be a valid ethereum address"
            ))
        })?;
        let symbol = entry.symbol.trim();
        if symbol.is_empty() {
            return Err(ServiceError::bad_request(format!(
                "entries[{index}].symbol is required"
            )));
        }
        if symbol.len() > 32 {
            return Err(ServiceError::bad_request(format!(
                "entries[{index}].symbol exceeds maximum length of 32 bytes"
            )));
        }
        if entry.decimals > 36 {
            return Err(ServiceError::bad_request(format!(
                "entries[{index}].decimals must be less than or equal to 36"
            )));
        }

        let key = (entry.chain_id, address.to_ascii_lowercase());
        entry.address = address;
        entry.symbol = symbol.to_string();
        if seen.insert(key) {
            out.push(entry);
        }
    }

    Ok(out)
}

fn holding_has_erc20_token(holding: &WalletAssetHolding, token_address: &str) -> bool {
    holding.asset_kind == WalletAssetKind::Erc20
        && holding
            .asset_address
            .as_deref()
            .is_some_and(|address| address.eq_ignore_ascii_case(token_address))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_registry_source_formats_name() {
        assert_eq!(
            token_registry_source("core-list"),
            "token_registry:core-list"
        );
    }

    #[test]
    fn token_registry_probe_config_disabled_returns_none() {
        assert!(token_registry_probe_config(None, &[]).unwrap().is_none());
        assert!(
            token_registry_probe_config(Some(false), &[])
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn token_registry_probe_config_enabled_without_lists_errors() {
        let error = token_registry_probe_config(Some(true), &[]).unwrap_err();
        assert!(error.to_string().contains("probe_token_registry"));
    }
}
