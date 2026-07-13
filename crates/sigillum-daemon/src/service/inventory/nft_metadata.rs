use serde_json::Value;
use sha2::{Digest, Sha256};
use sigillum_api::{
    NftMetadataCacheEntry, NftMetadataCollectionOptIn, NftMetadataFetchRequest,
    NftMetadataFetchResponse, NftMetadataFetchSkip, NftMetadataOptInDeleteRequest,
    NftMetadataOptInListResponse, NftMetadataOptInMutationResponse, NftMetadataOptInUpsertRequest,
    NftMetadataSettingsResponse, NftMetadataSettingsUpdateRequest, WalletAssetHolding,
    WalletAssetKind,
};

use crate::audit_log::AuditEventSpec;
use crate::service::helpers::now_unix;

use super::super::evm::normalize_address;
use super::super::{ServiceError, ServiceResult, SigillumService};
use super::support::{
    NftSpamAssessment, conservative_nft_spam_label, load_inventory_state, save_inventory_state,
    upsert_nft_metadata_cache,
};

const DEFAULT_NFT_METADATA_FETCH_LIMIT: usize = 25;
const MAX_NFT_METADATA_FETCH_LIMIT: usize = 100;
const MAX_NFT_METADATA_BODY_BYTES: usize = 262_144;
const MAX_NFT_METADATA_NAME_CHARS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
struct CacheKey {
    chain_id: u64,
    contract_address: String,
    token_id_hex: String,
}

#[derive(Clone, Debug)]
struct FetchCandidate {
    holding_index: usize,
    key: CacheKey,
    provider_profile: String,
    asset_kind: WalletAssetKind,
}

impl SigillumService {
    pub(crate) fn list_nft_metadata_optins(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<NftMetadataOptInListResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        Ok(NftMetadataOptInListResponse {
            opt_ins: state.nft_metadata_optins,
            ipfs_gateway_url: state.nft_metadata_ipfs_gateway,
        })
    }

    pub(crate) async fn upsert_nft_metadata_optin(
        &self,
        token: Option<&str>,
        body: NftMetadataOptInUpsertRequest,
    ) -> ServiceResult<NftMetadataOptInMutationResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        if body.chain_id == 0 {
            return Err(ServiceError::bad_request("chain_id must be greater than 0"));
        }
        let contract_address = normalize_address(&body.contract_address)?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let now = now_unix();
        let mut opt_in = NftMetadataCollectionOptIn {
            chain_id: body.chain_id,
            contract_address: contract_address.clone(),
            enabled: body.enabled.unwrap_or(true),
            created_at_unix: now,
            updated_at_unix: now,
        };

        if let Some(existing) = state.nft_metadata_optins.iter_mut().find(|existing| {
            existing.chain_id == body.chain_id
                && existing
                    .contract_address
                    .eq_ignore_ascii_case(&contract_address)
        }) {
            opt_in.created_at_unix = existing.created_at_unix;
            *existing = opt_in.clone();
        } else {
            state.nft_metadata_optins.push(opt_in.clone());
        }
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryNftMetadataOptInUpsert {
                chain_id: opt_in.chain_id,
                contract_address: opt_in.contract_address.clone(),
            },
        )?;

        Ok(NftMetadataOptInMutationResponse {
            status: "upserted".into(),
            opt_in,
        })
    }

    pub(crate) async fn delete_nft_metadata_optin(
        &self,
        token: Option<&str>,
        body: NftMetadataOptInDeleteRequest,
    ) -> ServiceResult<NftMetadataOptInMutationResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        if body.chain_id == 0 {
            return Err(ServiceError::bad_request("chain_id must be greater than 0"));
        }
        let contract_address = normalize_address(&body.contract_address)?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let position = state
            .nft_metadata_optins
            .iter()
            .position(|existing| {
                existing.chain_id == body.chain_id
                    && existing
                        .contract_address
                        .eq_ignore_ascii_case(&contract_address)
            })
            .ok_or_else(|| ServiceError::not_found("NFT metadata opt-in not found."))?;
        let opt_in = state.nft_metadata_optins.remove(position);
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryNftMetadataOptInDelete {
                chain_id: opt_in.chain_id,
                contract_address: opt_in.contract_address.clone(),
            },
        )?;

        Ok(NftMetadataOptInMutationResponse {
            status: "deleted".into(),
            opt_in,
        })
    }

    pub(crate) async fn update_nft_metadata_settings(
        &self,
        token: Option<&str>,
        body: NftMetadataSettingsUpdateRequest,
    ) -> ServiceResult<NftMetadataSettingsResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let ipfs_gateway_url = normalize_ipfs_gateway_setting(body.ipfs_gateway_url)?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        state.nft_metadata_ipfs_gateway = ipfs_gateway_url.clone();
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryNftMetadataSettingsUpdate {
                ipfs_gateway_configured: ipfs_gateway_url.is_some(),
            },
        )?;

        Ok(NftMetadataSettingsResponse {
            status: "updated".into(),
            ipfs_gateway_url,
        })
    }

    pub(crate) async fn fetch_nft_metadata(
        &self,
        token: Option<&str>,
        body: NftMetadataFetchRequest,
    ) -> ServiceResult<NftMetadataFetchResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let limit = validate_fetch_limit(body.limit)?;
        if body.chain_id == Some(0) {
            return Err(ServiceError::bad_request("chain_id must be greater than 0"));
        }
        let contract_filter = normalize_optional_contract(body.contract_address.as_deref())?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;

        if let Some(contract_address) = contract_filter.as_deref() {
            let has_enabled_opt_in = state.nft_metadata_optins.iter().any(|opt_in| {
                opt_in.enabled
                    && opt_in
                        .contract_address
                        .eq_ignore_ascii_case(contract_address)
                    && body
                        .chain_id
                        .is_none_or(|chain_id| opt_in.chain_id == chain_id)
            });
            if !has_enabled_opt_in {
                return Err(ServiceError::bad_request(
                    "collection is not opted in for metadata fetch",
                ));
            }
        }

        let candidates = nft_metadata_fetch_candidates(
            &state.holdings,
            &state.nft_metadata_optins,
            body.chain_id,
            contract_filter.as_deref(),
            limit,
        );
        let candidate_keys = candidates
            .iter()
            .map(|candidate| candidate.key.clone())
            .collect::<Vec<_>>();
        let mut fetched = 0;
        let mut skipped = Vec::new();

        for candidate in candidates {
            let Some(provider) = registry
                .evm_providers
                .iter()
                .find(|provider| provider.name == candidate.provider_profile)
                .cloned()
            else {
                record_fetch_skip(
                    &mut state.nft_metadata_cache,
                    &mut skipped,
                    &candidate,
                    "provider_profile_not_found",
                );
                continue;
            };

            let onchain_uri = match self
                .evm_nft_token_uri_for_provider(
                    provider.compartment_id,
                    &provider,
                    &candidate.key.contract_address,
                    &candidate.key.token_id_hex,
                    matches!(candidate.asset_kind, WalletAssetKind::Erc1155),
                    "latest",
                )
                .await
            {
                Ok(uri) => uri,
                Err(_) => {
                    record_fetch_skip(
                        &mut state.nft_metadata_cache,
                        &mut skipped,
                        &candidate,
                        "token_uri_unavailable",
                    );
                    continue;
                }
            };

            let fetch_uri = if matches!(candidate.asset_kind, WalletAssetKind::Erc1155) {
                match substitute_erc1155_id(&onchain_uri, &candidate.key.token_id_hex) {
                    Ok(uri) => uri,
                    Err(_) => {
                        record_fetch_skip(
                            &mut state.nft_metadata_cache,
                            &mut skipped,
                            &candidate,
                            "token_uri_unavailable",
                        );
                        continue;
                    }
                }
            } else {
                onchain_uri.clone()
            };
            let resolved_uri = match resolve_metadata_uri(
                &fetch_uri,
                state.nft_metadata_ipfs_gateway.as_deref(),
            ) {
                Ok(uri) => uri,
                Err(reason) => {
                    record_fetch_skip(
                        &mut state.nft_metadata_cache,
                        &mut skipped,
                        &candidate,
                        reason,
                    );
                    continue;
                }
            };

            let response = match self.state.http_client().get(&resolved_uri).send().await {
                Ok(response) => response,
                Err(_) => {
                    record_fetch_skip(
                        &mut state.nft_metadata_cache,
                        &mut skipped,
                        &candidate,
                        "fetch_failed_transport",
                    );
                    continue;
                }
            };
            let status = response.status();
            if !status.is_success() {
                let reason = format!("fetch_failed_status_{}", status.as_u16());
                record_fetch_skip(
                    &mut state.nft_metadata_cache,
                    &mut skipped,
                    &candidate,
                    &reason,
                );
                continue;
            }
            let body_bytes = match response.bytes().await {
                Ok(bytes) => bytes,
                Err(_) => {
                    record_fetch_skip(
                        &mut state.nft_metadata_cache,
                        &mut skipped,
                        &candidate,
                        "fetch_failed_transport",
                    );
                    continue;
                }
            };
            if body_bytes.len() > MAX_NFT_METADATA_BODY_BYTES {
                record_fetch_skip(
                    &mut state.nft_metadata_cache,
                    &mut skipped,
                    &candidate,
                    "metadata_too_large",
                );
                continue;
            }

            let now = now_unix();
            let content_sha256 = hex::encode(Sha256::digest(&body_bytes));
            let metadata_name = extract_metadata_name(&body_bytes);
            if let Some(holding) = state.holdings.get_mut(candidate.holding_index) {
                holding.metadata_uri = Some(onchain_uri.clone());
                holding.metadata_name = metadata_name.clone();
                holding.last_checked_at_unix = now;
            }
            let Some(updated_holding) = state.holdings.get(candidate.holding_index).cloned() else {
                continue;
            };
            let assessment = conservative_nft_spam_label(
                &updated_holding,
                &state.addresses,
                &state.holdings,
                &state.risk_catalog,
            )
            .unwrap_or_else(default_nft_spam_assessment);
            if let Some(holding) = state.holdings.get_mut(candidate.holding_index) {
                holding.spam_label = Some(assessment.label.clone());
            }
            let mut cache_holding = updated_holding;
            cache_holding.spam_label = Some(assessment.label.clone());
            upsert_nft_metadata_cache(
                &mut state.nft_metadata_cache,
                &cache_holding,
                &assessment.label,
                &assessment.reasons,
            );
            if let Some(entry) = find_cache_entry_mut(&mut state.nft_metadata_cache, &candidate.key)
            {
                entry.metadata_uri = Some(onchain_uri);
                entry.name = metadata_name;
                entry.spam_label = assessment.label;
                entry.spam_reasons = assessment.reasons;
                entry.fetched_at_unix = Some(now);
                entry.fetched_uri = Some(resolved_uri);
                entry.content_sha256 = Some(content_sha256);
                entry.fetch_skipped_reason = None;
                entry.updated_at_unix = now;
            }
            fetched += 1;
        }

        let entries = candidate_keys
            .iter()
            .filter_map(|key| find_cache_entry(&state.nft_metadata_cache, key).cloned())
            .collect::<Vec<_>>();
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryNftMetadataFetch {
                fetched,
                skipped: skipped.len(),
            },
        )?;

        Ok(NftMetadataFetchResponse {
            fetched,
            skipped,
            entries,
        })
    }
}

fn nft_metadata_fetch_candidates(
    holdings: &[WalletAssetHolding],
    opt_ins: &[NftMetadataCollectionOptIn],
    chain_filter: Option<u64>,
    contract_filter: Option<&str>,
    limit: usize,
) -> Vec<FetchCandidate> {
    let mut candidates = Vec::new();
    for (index, holding) in holdings.iter().enumerate() {
        if candidates.len() >= limit {
            break;
        }
        if !matches!(
            holding.asset_kind,
            WalletAssetKind::Erc721 | WalletAssetKind::Erc1155 | WalletAssetKind::Nft
        ) {
            continue;
        }
        if chain_filter.is_some_and(|chain_id| holding.chain_id != chain_id) {
            continue;
        }
        let (Some(contract_address), Some(token_id_hex)) = (
            holding.asset_address.as_deref(),
            holding.token_id_hex.as_deref(),
        ) else {
            continue;
        };
        if contract_filter.is_some_and(|filter| !contract_address.eq_ignore_ascii_case(filter)) {
            continue;
        }
        if !opt_ins.iter().any(|opt_in| {
            opt_in.enabled
                && opt_in.chain_id == holding.chain_id
                && opt_in
                    .contract_address
                    .eq_ignore_ascii_case(contract_address)
        }) {
            continue;
        }
        candidates.push(FetchCandidate {
            holding_index: index,
            key: CacheKey {
                chain_id: holding.chain_id,
                contract_address: contract_address.to_string(),
                token_id_hex: token_id_hex.to_string(),
            },
            provider_profile: holding.provider_profile.clone(),
            asset_kind: holding.asset_kind.clone(),
        });
    }
    candidates
}

fn normalize_ipfs_gateway_setting(value: Option<String>) -> ServiceResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if !value.starts_with("http://") && !value.starts_with("https://") {
        return Err(ServiceError::bad_request(
            "ipfs_gateway_url must be an http(s) URL",
        ));
    }
    Ok(Some(value.to_string()))
}

fn normalize_optional_contract(value: Option<&str>) -> ServiceResult<Option<String>> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_address)
        .transpose()
}

fn validate_fetch_limit(limit: Option<usize>) -> ServiceResult<usize> {
    let limit = limit.unwrap_or(DEFAULT_NFT_METADATA_FETCH_LIMIT);
    if limit == 0 || limit > MAX_NFT_METADATA_FETCH_LIMIT {
        return Err(ServiceError::bad_request(format!(
            "limit must be between 1 and {MAX_NFT_METADATA_FETCH_LIMIT}"
        )));
    }
    Ok(limit)
}

fn resolve_metadata_uri(raw_uri: &str, ipfs_gateway: Option<&str>) -> Result<String, &'static str> {
    let raw_uri = raw_uri.trim();
    if raw_uri.starts_with("http://") || raw_uri.starts_with("https://") {
        return Ok(raw_uri.to_string());
    }
    if let Some(path) = raw_uri.strip_prefix("ipfs://") {
        if path.is_empty() {
            return Err("unsupported_uri_scheme");
        }
        let Some(gateway) = ipfs_gateway
            .map(str::trim)
            .filter(|gateway| !gateway.is_empty())
        else {
            return Err("ipfs_gateway_not_configured");
        };
        return Ok(join_ipfs_gateway(gateway, path));
    }
    Err("unsupported_uri_scheme")
}

fn join_ipfs_gateway(gateway: &str, path: &str) -> String {
    format!(
        "{}/{}",
        gateway.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn substitute_erc1155_id(uri: &str, token_id_hex: &str) -> ServiceResult<String> {
    Ok(uri.replace("{id}", &erc1155_token_id_path_hex(token_id_hex)?))
}

fn erc1155_token_id_path_hex(token_id_hex: &str) -> ServiceResult<String> {
    let raw = token_id_hex
        .strip_prefix("0x")
        .or_else(|| token_id_hex.strip_prefix("0X"))
        .unwrap_or(token_id_hex);
    if raw.is_empty() || raw.len() > 64 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ServiceError::internal(
            "Invalid provider response: ERC-1155 token id must fit in 32 bytes",
        ));
    }
    Ok(format!(
        "{}{}",
        "0".repeat(64 - raw.len()),
        raw.to_ascii_lowercase()
    ))
}

fn extract_metadata_name(body: &[u8]) -> Option<String> {
    let json: Value = serde_json::from_slice(body).ok()?;
    json.get("name")
        .and_then(Value::as_str)
        .map(|name| name.chars().take(MAX_NFT_METADATA_NAME_CHARS).collect())
}

fn record_fetch_skip(
    cache: &mut [NftMetadataCacheEntry],
    skipped: &mut Vec<NftMetadataFetchSkip>,
    candidate: &FetchCandidate,
    reason: &str,
) {
    skipped.push(NftMetadataFetchSkip {
        chain_id: candidate.key.chain_id,
        contract_address: candidate.key.contract_address.clone(),
        token_id_hex: Some(candidate.key.token_id_hex.clone()),
        reason: reason.to_string(),
    });
    if let Some(entry) = find_cache_entry_mut(cache, &candidate.key) {
        entry.fetch_skipped_reason = Some(reason.to_string());
        entry.updated_at_unix = now_unix();
    }
}

fn find_cache_entry<'a>(
    cache: &'a [NftMetadataCacheEntry],
    key: &CacheKey,
) -> Option<&'a NftMetadataCacheEntry> {
    cache
        .iter()
        .find(|entry| cache_entry_matches_key(entry, key))
}

fn find_cache_entry_mut<'a>(
    cache: &'a mut [NftMetadataCacheEntry],
    key: &CacheKey,
) -> Option<&'a mut NftMetadataCacheEntry> {
    cache
        .iter_mut()
        .find(|entry| cache_entry_matches_key(entry, key))
}

fn cache_entry_matches_key(entry: &NftMetadataCacheEntry, key: &CacheKey) -> bool {
    entry.chain_id == key.chain_id
        && entry
            .contract_address
            .eq_ignore_ascii_case(&key.contract_address)
        && entry.token_id_hex.eq_ignore_ascii_case(&key.token_id_hex)
}

fn default_nft_spam_assessment() -> NftSpamAssessment {
    NftSpamAssessment {
        label: "unverified_nft_metadata".into(),
        reasons: vec!["metadata_not_verified_locally".into()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_ipfs_uri_with_gateway_joining_one_slash() {
        assert_eq!(
            resolve_metadata_uri(
                "ipfs://bafy/token.json",
                Some("https://gateway.example/ipfs/")
            )
            .unwrap(),
            "https://gateway.example/ipfs/bafy/token.json"
        );
    }

    #[test]
    fn ipfs_uri_without_gateway_is_skipped() {
        assert_eq!(
            resolve_metadata_uri("ipfs://bafy/token.json", None).unwrap_err(),
            "ipfs_gateway_not_configured"
        );
    }

    #[test]
    fn unsupported_metadata_uri_scheme_is_skipped() {
        assert_eq!(
            resolve_metadata_uri("data:application/json,{}", Some("https://gateway.example"))
                .unwrap_err(),
            "unsupported_uri_scheme"
        );
    }

    #[test]
    fn substitutes_erc1155_id_with_zero_padded_lowercase_hex() {
        let resolved = substitute_erc1155_id("ipfs://bafy/{id}.json", "0xAbC").unwrap();

        assert_eq!(
            resolved,
            "ipfs://bafy/0000000000000000000000000000000000000000000000000000000000000abc.json"
        );
    }

    #[test]
    fn extracts_and_truncates_metadata_name() {
        let long_name = "a".repeat(300);
        let body = serde_json::json!({ "name": long_name }).to_string();

        let name = extract_metadata_name(body.as_bytes()).unwrap();

        assert_eq!(name.len(), MAX_NFT_METADATA_NAME_CHARS);
        assert!(name.chars().all(|ch| ch == 'a'));
    }

    #[test]
    fn validates_fetch_limit_bounds() {
        assert_eq!(
            validate_fetch_limit(None).unwrap(),
            DEFAULT_NFT_METADATA_FETCH_LIMIT
        );
        assert_eq!(validate_fetch_limit(Some(100)).unwrap(), 100);
        assert!(validate_fetch_limit(Some(0)).is_err());
        assert!(validate_fetch_limit(Some(101)).is_err());
    }
}
