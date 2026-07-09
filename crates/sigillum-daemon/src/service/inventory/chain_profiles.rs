use sigillum_api::{
    ChainProfile, ChainProfileDeleteRequest, ChainProfileListResponse,
    ChainProfileMutationResponse, ChainProfileUpsertRequest, DEFAULT_DORMANCY_BLOCK_WINDOW,
};

use crate::audit_log::AuditEventSpec;
use crate::service::chains::ensure_builtin_chain_profiles;
use crate::service::evm::normalize_address;
use crate::service::helpers::now_unix;
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::DISCOVERY_SOURCE_OPERATOR;
use super::support::{
    default_native_symbol, load_inventory_state, save_inventory_state, trimmed_optional,
    trimmed_required, unique_strings,
};

impl SigillumService {
    pub(crate) fn list_chain_profiles(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<ChainProfileListResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        Ok(ChainProfileListResponse {
            profiles: state.chain_profiles,
        })
    }

    pub(crate) async fn upsert_chain_profile(
        &self,
        token: Option<&str>,
        body: ChainProfileUpsertRequest,
    ) -> ServiceResult<ChainProfileMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let now = now_unix();
        let name = trimmed_required("name", &body.name)?;
        if body.builtin == Some(true) {
            return Err(ServiceError::bad_request(
                "builtin chain profiles cannot be created or updated by operators",
            ));
        }
        let chain_id = body
            .chain_id
            .ok_or_else(|| ServiceError::bad_request("chain_id is required"))?;
        if chain_id == 0 {
            return Err(ServiceError::bad_request("chain_id must be greater than 0"));
        }
        let existing = state
            .chain_profiles
            .iter()
            .find(|existing| existing.name == name)
            .cloned();
        let dormancy_block_window = match body.dormancy_block_window {
            Some(0) => {
                return Err(ServiceError::bad_request(
                    "dormancy_block_window must be greater than 0",
                ));
            }
            Some(window) => window,
            None => existing
                .as_ref()
                .map(|profile| profile.dormancy_block_window)
                .filter(|window| *window > 0)
                .unwrap_or(DEFAULT_DORMANCY_BLOCK_WINDOW),
        };
        if state
            .chain_profiles
            .iter()
            .any(|existing| existing.name != name && existing.chain_id == Some(chain_id))
        {
            return Err(ServiceError::conflict(format!(
                "chain_id {chain_id} is already registered"
            )));
        }
        if existing.as_ref().is_some_and(|profile| profile.builtin)
            && existing.as_ref().and_then(|profile| profile.chain_id) != Some(chain_id)
        {
            return Err(ServiceError::bad_request(
                "Built-in chain profile chain_id cannot be changed.",
            ));
        }
        let permit2_address = body
            .permit2_address
            .and_then(trimmed_optional)
            .map(|address| normalize_address(&address))
            .transpose()?;
        let uniswap_v2_router_address = body
            .uniswap_v2_router_address
            .and_then(trimmed_optional)
            .map(|address| normalize_address(&address))
            .transpose()?;
        let mut profile = ChainProfile {
            name: name.clone(),
            chain_family: trimmed_required("chain_family", &body.chain_family)?,
            chain_id: Some(chain_id),
            provider_profile: body.provider_profile.and_then(trimmed_optional),
            native_symbol: body
                .native_symbol
                .and_then(trimmed_optional)
                .unwrap_or_else(|| default_native_symbol(&body.chain_family).to_string()),
            native_decimals: body
                .native_decimals
                .or_else(|| existing.as_ref().map(|profile| profile.native_decimals))
                .unwrap_or(18),
            finality_blocks: body
                .finality_blocks
                .or_else(|| existing.as_ref().map(|profile| profile.finality_blocks))
                .unwrap_or_default(),
            dormancy_block_window,
            permit2_address,
            uniswap_v2_router_address,
            explorer_url: body.explorer_url.and_then(trimmed_optional),
            capabilities: unique_strings(
                body.capabilities.into_iter().filter_map(trimmed_optional),
            ),
            enabled: body.enabled.unwrap_or(true),
            source: existing
                .as_ref()
                .filter(|profile| profile.builtin)
                .map(|profile| profile.source.clone())
                .unwrap_or_else(|| DISCOVERY_SOURCE_OPERATOR.into()),
            builtin: existing.as_ref().is_some_and(|profile| profile.builtin),
            created_at_unix: now,
            updated_at_unix: now,
        };

        if let Some(existing) = state
            .chain_profiles
            .iter_mut()
            .find(|existing| existing.name == name)
        {
            profile.created_at_unix = existing.created_at_unix;
            *existing = profile.clone();
        } else {
            state.chain_profiles.push(profile.clone());
        }
        ensure_builtin_chain_profiles(&mut state.chain_profiles);
        profile = state
            .chain_profiles
            .iter()
            .find(|existing| existing.name == name)
            .cloned()
            .unwrap_or(profile);
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryChainProfileUpsert {
                name: profile.name.clone(),
                chain_family: profile.chain_family.clone(),
            },
        )?;

        Ok(ChainProfileMutationResponse {
            status: "upserted".into(),
            profile,
        })
    }

    pub(crate) async fn delete_chain_profile(
        &self,
        token: Option<&str>,
        body: ChainProfileDeleteRequest,
    ) -> ServiceResult<ChainProfileMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let name = trimmed_required("name", &body.name)?;
        let position = state
            .chain_profiles
            .iter()
            .position(|profile| profile.name == name)
            .ok_or_else(|| ServiceError::not_found("Chain profile not found."))?;
        if state.chain_profiles[position].builtin {
            return Err(ServiceError::bad_request(
                "Built-in chain profiles cannot be deleted.",
            ));
        }
        let profile = state.chain_profiles.remove(position);
        ensure_builtin_chain_profiles(&mut state.chain_profiles);
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryChainProfileDelete {
                name: profile.name.clone(),
            },
        )?;

        Ok(ChainProfileMutationResponse {
            status: "deleted".into(),
            profile,
        })
    }
}
