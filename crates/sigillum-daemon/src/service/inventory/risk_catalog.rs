use sigillum_api::{
    RiskCatalogDeleteRequest, RiskCatalogEntry, RiskCatalogListResponse,
    RiskCatalogMutationResponse, RiskCatalogUpsertRequest,
};

use crate::audit_log::AuditEventSpec;

use super::super::evm::normalize_address;
use super::super::{ServiceError, ServiceResult, SigillumService};
use super::DISCOVERY_SOURCE_OPERATOR;
use super::support::{
    load_inventory_state, save_inventory_state, trimmed_optional, unique_strings,
};
use crate::service::helpers::now_unix;

impl SigillumService {
    pub(crate) fn list_risk_catalog(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<RiskCatalogListResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        Ok(RiskCatalogListResponse {
            entries: state.risk_catalog,
        })
    }

    pub(crate) async fn upsert_risk_catalog_entry(
        &self,
        token: Option<&str>,
        body: RiskCatalogUpsertRequest,
    ) -> ServiceResult<RiskCatalogMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let now = now_unix();
        let address = normalize_address(&body.address)?;
        let risk_level = normalized_risk_catalog_level(&body.risk_level)?;
        let label = body
            .label
            .and_then(trimmed_optional)
            .unwrap_or_else(|| address.clone());
        let mut entry = RiskCatalogEntry {
            address: address.clone(),
            label,
            risk_level,
            source: DISCOVERY_SOURCE_OPERATOR.into(),
            notes: unique_strings(body.notes.into_iter().filter_map(trimmed_optional)),
            created_at_unix: now,
            updated_at_unix: now,
        };

        if let Some(existing) = state
            .risk_catalog
            .iter_mut()
            .find(|existing| existing.address.eq_ignore_ascii_case(&address))
        {
            entry.created_at_unix = existing.created_at_unix;
            *existing = entry.clone();
        } else {
            state.risk_catalog.push(entry.clone());
        }
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryRiskCatalogUpsert {
                address: entry.address.clone(),
                risk_level: entry.risk_level.clone(),
            },
        )?;

        Ok(RiskCatalogMutationResponse {
            status: "upserted".into(),
            entry,
        })
    }

    pub(crate) async fn delete_risk_catalog_entry(
        &self,
        token: Option<&str>,
        body: RiskCatalogDeleteRequest,
    ) -> ServiceResult<RiskCatalogMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let address = normalize_address(&body.address)?;
        let position = state
            .risk_catalog
            .iter()
            .position(|entry| entry.address.eq_ignore_ascii_case(&address))
            .ok_or_else(|| ServiceError::not_found("Risk catalog entry not found."))?;
        let entry = state.risk_catalog.remove(position);
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryRiskCatalogDelete {
                address: entry.address.clone(),
            },
        )?;

        Ok(RiskCatalogMutationResponse {
            status: "deleted".into(),
            entry,
        })
    }
}

fn normalized_risk_catalog_level(level: &str) -> ServiceResult<String> {
    let normalized = level.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "trusted" | "low" | "medium" | "high" | "critical" => Ok(normalized),
        _ => Err(ServiceError::bad_request(
            "risk_level must be one of trusted, low, medium, high, or critical",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_supported_risk_levels() {
        assert_eq!(
            normalized_risk_catalog_level(" CRITICAL ").unwrap(),
            "critical"
        );
        assert!(normalized_risk_catalog_level("unknown").is_err());
    }
}
