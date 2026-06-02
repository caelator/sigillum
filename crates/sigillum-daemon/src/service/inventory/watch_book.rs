use sigillum_api::{
    WatchAddressBookDeleteRequest, WatchAddressBookEntry, WatchAddressBookListResponse,
    WatchAddressBookMutationResponse, WatchAddressBookUpsertRequest,
};

use crate::audit_log::AuditEventSpec;
use crate::service::evm::normalize_address;
use crate::service::helpers::{now_unix, random_id};
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::DISCOVERY_SOURCE_OPERATOR;
use super::support::{
    load_inventory_state, save_inventory_state, trimmed_optional, unique_strings,
};

impl SigillumService {
    pub(crate) fn list_watch_address_book(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<WatchAddressBookListResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        Ok(WatchAddressBookListResponse {
            entries: state.watch_address_book,
        })
    }

    pub(crate) async fn upsert_watch_address_book_entry(
        &self,
        token: Option<&str>,
        body: WatchAddressBookUpsertRequest,
    ) -> ServiceResult<WatchAddressBookMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let now = now_unix();
        let address = normalize_address(&body.address)?;
        let label = body
            .label
            .and_then(trimmed_optional)
            .unwrap_or_else(|| watch_label_from_address(&address));
        let tags = unique_strings(body.tags.into_iter().filter_map(trimmed_optional));

        let entry = if let Some(existing) = state
            .watch_address_book
            .iter_mut()
            .find(|entry| entry.address.eq_ignore_ascii_case(&address))
        {
            existing.label = label.clone();
            existing.tags = tags;
            existing.enabled = body.enabled.unwrap_or(existing.enabled);
            existing.updated_at_unix = now;
            existing.clone()
        } else {
            let entry = WatchAddressBookEntry {
                id: random_id(),
                address: address.clone(),
                label: label.clone(),
                tags,
                source: DISCOVERY_SOURCE_OPERATOR.into(),
                enabled: body.enabled.unwrap_or(true),
                created_at_unix: now,
                updated_at_unix: now,
            };
            state.watch_address_book.push(entry.clone());
            entry
        };

        save_inventory_state(&self.state.base_dir, &state)?;
        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryWatchAddressUpsert {
                address: entry.address.clone(),
                label: entry.label.clone(),
            },
        )?;

        Ok(WatchAddressBookMutationResponse {
            status: "saved".into(),
            entry,
        })
    }

    pub(crate) async fn delete_watch_address_book_entry(
        &self,
        token: Option<&str>,
        body: WatchAddressBookDeleteRequest,
    ) -> ServiceResult<WatchAddressBookMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let address = normalize_address(&body.address)?;
        let index = state
            .watch_address_book
            .iter()
            .position(|entry| entry.address.eq_ignore_ascii_case(&address))
            .ok_or_else(|| ServiceError::not_found("Watch address not found."))?;
        let entry = state.watch_address_book.remove(index);

        save_inventory_state(&self.state.base_dir, &state)?;
        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryWatchAddressDelete {
                address: entry.address.clone(),
            },
        )?;

        Ok(WatchAddressBookMutationResponse {
            status: "deleted".into(),
            entry,
        })
    }
}

fn watch_label_from_address(address: &str) -> String {
    address
        .get(address.len().saturating_sub(8)..)
        .unwrap_or(address)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_watch_label_uses_address_suffix() {
        assert_eq!(
            watch_label_from_address("0x777777777777777777777777777777777777abcd"),
            "7777abcd"
        );
    }
}
