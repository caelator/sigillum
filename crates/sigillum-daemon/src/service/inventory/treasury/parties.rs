use sigillum_api::{
    Counterparty, CounterpartyCreateRequest, CounterpartyDeleteRequest, CounterpartyListResponse,
    CounterpartyMutationResponse, CounterpartyUpdateRequest,
};

use crate::audit_log::AuditEventSpec;
use crate::service::evm::normalize_address;
use crate::service::helpers::{now_unix, random_id};
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::super::support::{
    load_inventory_state, save_inventory_state, trimmed_optional, trimmed_required,
};

impl SigillumService {
    pub(crate) fn list_parties(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<CounterpartyListResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        Ok(CounterpartyListResponse {
            parties: state.parties,
        })
    }

    pub(crate) async fn create_party(
        &self,
        token: Option<&str>,
        body: CounterpartyCreateRequest,
    ) -> ServiceResult<CounterpartyMutationResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let name = trimmed_required("name", &body.name)?;
        let note = body.note.and_then(trimmed_optional);
        let sweep_destination_address = body
            .sweep_destination_address
            .and_then(trimmed_optional)
            .map(|address| normalize_address(&address))
            .transpose()?;

        let mut state = load_inventory_state(&self.state.base_dir)?;
        let party = Counterparty {
            id: random_id(),
            name,
            note,
            sweep_destination_address,
            created_at_unix: now_unix(),
        };
        state.parties.push(party.clone());
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::TreasuryPartyCreate {
                name: party.name.clone(),
            },
        )?;

        Ok(CounterpartyMutationResponse {
            status: "created".into(),
            party: Some(party),
        })
    }

    pub(crate) async fn update_party(
        &self,
        token: Option<&str>,
        body: CounterpartyUpdateRequest,
    ) -> ServiceResult<CounterpartyMutationResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let id = body.id.trim().to_string();
        let name = trimmed_required("name", &body.name)?;
        let note = body.note.and_then(trimmed_optional);
        // Omitted keeps the stored destination; an explicit blank clears it.
        let sweep_destination_address = body
            .sweep_destination_address
            .map(|value| {
                let value = value.trim();
                if value.is_empty() {
                    Ok(None)
                } else {
                    normalize_address(value).map(Some)
                }
            })
            .transpose()?;

        let mut state = load_inventory_state(&self.state.base_dir)?;
        let Some(party) = state
            .parties
            .iter_mut()
            .find(|party| party.id.as_str() == id.as_str())
        else {
            return Err(ServiceError::not_found("Counterparty not found."));
        };
        party.name = name;
        party.note = note;
        if let Some(sweep_destination_address) = sweep_destination_address {
            party.sweep_destination_address = sweep_destination_address;
        }
        let updated = party.clone();
        save_inventory_state(&self.state.base_dir, &state)?;

        Ok(CounterpartyMutationResponse {
            status: "updated".into(),
            party: Some(updated),
        })
    }

    pub(crate) async fn delete_party(
        &self,
        token: Option<&str>,
        body: CounterpartyDeleteRequest,
    ) -> ServiceResult<CounterpartyMutationResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let id = body.id.trim().to_string();

        let mut state = load_inventory_state(&self.state.base_dir)?;
        let Some(position) = state
            .parties
            .iter()
            .position(|party| party.id.as_str() == id.as_str())
        else {
            return Err(ServiceError::not_found("Counterparty not found."));
        };
        let name = state.parties[position].name.clone();
        for allocation in &mut state.receive_allocations {
            if allocation.counterparty_id.as_deref() == Some(id.as_str()) {
                allocation.counterparty_id = None;
            }
        }
        state.parties.remove(position);
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::TreasuryPartyDelete { name },
        )?;

        Ok(CounterpartyMutationResponse {
            status: "deleted".into(),
            party: None,
        })
    }
}
