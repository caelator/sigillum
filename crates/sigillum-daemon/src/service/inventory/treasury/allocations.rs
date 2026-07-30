use sigillum_api::{
    TreasuryReceiveAllocateRequest, TreasuryReceiveAllocation,
    TreasuryReceiveAllocationListResponse, TreasuryReceiveAllocationMutationResponse,
    TreasuryReceiveRotateRequest, WalletInventoryAddress,
};

use crate::audit_log::AuditEventSpec;
use crate::inventory::WalletInventoryState;
use crate::service::helpers::{map_xpub_error, now_unix, random_id};
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::super::support::{
    load_inventory_state, save_inventory_state, trimmed_optional, trimmed_required,
};
use super::super::wallet_selection::{
    DiscoveryWallet, SeedDerivationPattern, derive_discovery_wallet_address,
    select_discovery_wallets,
};
use super::super::{WALLET_FAMILY_ETH_SEED, WALLET_FAMILY_ETH_XPUB};

pub(super) const RECEIVE_STATUS_ACTIVE: &str = "active";
pub(super) const RECEIVE_STATUS_RETIRED: &str = "retired";
/// Absurdly large but bounded: receive indices beyond this point indicate a
/// runaway caller, not a treasury that genuinely needs a million addresses.
const MAX_RECEIVE_INDEX: u32 = 1_000_000;

impl SigillumService {
    /// All receive allocations, active and retired, in allocation order.
    pub(crate) fn list_treasury_receive_allocations(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<TreasuryReceiveAllocationListResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        Ok(TreasuryReceiveAllocationListResponse {
            allocations: state.receive_allocations,
        })
    }

    /// Allocate a fresh purpose-labeled receive address for a wallet profile.
    ///
    /// Privacy: derivation is pure local xpub math — no provider or network
    /// I/O — and every allocation takes the next unused receive index, so a
    /// counterparty only ever sees an address that has never been handed out
    /// for another purpose. Fresh-per-purpose addresses keep unrelated
    /// payments unlinkable on-chain.
    pub(crate) async fn allocate_treasury_receive_address(
        &self,
        token: Option<&str>,
        body: TreasuryReceiveAllocateRequest,
    ) -> ServiceResult<TreasuryReceiveAllocationMutationResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let wallet_profile = trimmed_required("wallet_profile", &body.wallet_profile)?;
        let purpose = trimmed_required("purpose", &body.purpose)?;
        let label = body.label.and_then(trimmed_optional);

        let mut state = load_inventory_state(&self.state.base_dir)?;
        let counterparty_id = body.counterparty_id.and_then(trimmed_optional);
        let counterparty_name = if let Some(id) = counterparty_id.as_deref() {
            let Some(party) = state.parties.iter().find(|party| party.id.as_str() == id) else {
                return Err(ServiceError::not_found("Counterparty not found."));
            };
            Some(party.name.clone())
        } else {
            None
        };
        let allocation = self.issue_receive_allocation(
            &mut state,
            &wallet_profile,
            purpose,
            label,
            counterparty_id,
        )?;
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::TreasuryReceiveAllocate {
                wallet_profile: allocation.wallet_profile.clone(),
                purpose: allocation.purpose.clone(),
            },
        )?;
        if let Some(name) = counterparty_name {
            self.record_audit(
                self.state.active_compartment_id_for(token),
                AuditEventSpec::TreasuryReceiveBind { name },
            )?;
        }

        Ok(TreasuryReceiveAllocationMutationResponse {
            status: "allocated".into(),
            allocation,
        })
    }

    /// Retire an active allocation and issue the next index for the same
    /// wallet profile, carrying over its purpose and label.
    ///
    /// Both mutations land in a single state save: either the old allocation
    /// is retired AND its replacement exists, or neither change persists.
    /// Like allocation, the replacement address is derived locally from the
    /// profile xpub with no network I/O.
    pub(crate) async fn rotate_treasury_receive_address(
        &self,
        token: Option<&str>,
        body: TreasuryReceiveRotateRequest,
    ) -> ServiceResult<TreasuryReceiveAllocationMutationResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let allocation_id = body.allocation_id.trim().to_string();

        let mut state = load_inventory_state(&self.state.base_dir)?;
        let Some(existing) = state
            .receive_allocations
            .iter_mut()
            .find(|allocation| allocation.id == allocation_id)
        else {
            return Err(ServiceError::not_found("Receive allocation not found."));
        };
        if existing.status != RECEIVE_STATUS_ACTIVE {
            return Err(ServiceError::bad_request(
                "Receive allocation is not active.",
            ));
        }
        existing.status = RECEIVE_STATUS_RETIRED.into();
        existing.retired_at_unix = Some(now_unix());
        let wallet_profile = existing.wallet_profile.clone();
        let purpose = existing.purpose.clone();
        let label = existing.label.clone();
        let counterparty_id = existing.counterparty_id.clone();
        let counterparty_name = counterparty_id.as_deref().and_then(|id| {
            state
                .parties
                .iter()
                .find(|party| party.id.as_str() == id)
                .map(|party| party.name.clone())
        });

        let allocation = self.issue_receive_allocation(
            &mut state,
            &wallet_profile,
            purpose,
            label,
            counterparty_id,
        )?;
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::TreasuryReceiveRotate { id: allocation_id },
        )?;
        if let Some(name) = counterparty_name {
            self.record_audit(
                self.state.active_compartment_id_for(token),
                AuditEventSpec::TreasuryReceiveBind { name },
            )?;
        }

        Ok(TreasuryReceiveAllocationMutationResponse {
            status: "rotated".into(),
            allocation,
        })
    }

    /// Derive the next receive allocation for `wallet_profile` and append it
    /// to `state` (callers persist the state afterwards).
    ///
    /// Profile resolution prefers seed profiles over xpub profiles with the
    /// same name, matching discovery-wallet selection. The next index is one
    /// past the highest index either previously allocated or already observed
    /// in scanned inventory, so fresh allocations never collide with
    /// addresses the treasury has used before.
    fn issue_receive_allocation(
        &self,
        state: &mut WalletInventoryState,
        wallet_profile: &str,
        purpose: String,
        label: Option<String>,
        counterparty_id: Option<String>,
    ) -> ServiceResult<TreasuryReceiveAllocation> {
        let registry = crate::profiles::load_profiles(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load profiles: {error}")))?;
        let wallets = select_discovery_wallets(
            self,
            &registry.eth_seed_wallets,
            &registry.eth_xpub_wallets,
            None,
            Some(wallet_profile),
            SeedDerivationPattern::Project,
            1,
        )?;
        let Some(wallet) = wallets.into_iter().next() else {
            return Err(ServiceError::not_found("Wallet profile not found."));
        };
        let chain_id = provider_chain_id_for_discovery_wallet(&registry, &wallet)?;

        let next_index = next_receive_index(
            &state.receive_allocations,
            &state.addresses,
            &wallet.family,
            &wallet.profile,
        );
        if next_index > MAX_RECEIVE_INDEX {
            return Err(ServiceError::bad_request("Receive index space exhausted."));
        }

        let derived =
            derive_discovery_wallet_address(&wallet, next_index).map_err(map_xpub_error)?;
        let allocation = TreasuryReceiveAllocation {
            id: random_id(),
            wallet_family: wallet.family.clone(),
            wallet_profile: wallet.profile.clone(),
            chain_id,
            chain_id_assumed: false,
            address: derived.address,
            derivation_path: format!("{}/{}", wallet.receive_path, next_index),
            address_index: next_index,
            purpose,
            label,
            status: RECEIVE_STATUS_ACTIVE.into(),
            created_at_unix: now_unix(),
            retired_at_unix: None,
            counterparty_id,
        };
        state.receive_allocations.push(allocation.clone());
        Ok(allocation)
    }
}

fn provider_chain_id_for_discovery_wallet(
    registry: &crate::profiles::ProfileRegistry,
    wallet: &DiscoveryWallet,
) -> ServiceResult<u64> {
    let provider_profile = match wallet.family.as_str() {
        WALLET_FAMILY_ETH_SEED => registry
            .eth_seed_wallets
            .iter()
            .find(|profile| profile.name == wallet.profile)
            .map(|profile| profile.provider_profile.as_str()),
        WALLET_FAMILY_ETH_XPUB => registry
            .eth_xpub_wallets
            .iter()
            .find(|profile| profile.name == wallet.profile)
            .map(|profile| profile.provider_profile.as_str()),
        _ => None,
    }
    .ok_or_else(|| ServiceError::not_found("Wallet profile not found."))?;

    registry
        .evm_providers
        .iter()
        .find(|provider| provider.name == provider_profile)
        .map(|provider| provider.chain_id)
        .ok_or_else(|| ServiceError::not_found("Provider profile not found."))
}

/// Next unused receive index for one wallet profile.
///
/// Considers both prior allocations (including retired ones, which must never
/// be re-issued) and inventory rows discovered by scans, so the allocator
/// stays ahead of any index the wallet has already exposed on-chain. Returns
/// 0 only when neither source knows the profile.
pub(super) fn next_receive_index(
    allocations: &[TreasuryReceiveAllocation],
    addresses: &[WalletInventoryAddress],
    wallet_family: &str,
    wallet_profile: &str,
) -> u32 {
    let allocated_max = allocations
        .iter()
        .filter(|allocation| allocation.wallet_profile == wallet_profile)
        .map(|allocation| allocation.address_index)
        .max();
    let observed_max = addresses
        .iter()
        .filter(|address| {
            address.wallet_family == wallet_family && address.wallet_profile == wallet_profile
        })
        .map(|address| address.address_index)
        .max();
    match allocated_max.into_iter().chain(observed_max).max() {
        Some(max_index) => max_index.saturating_add(1),
        None => 0,
    }
}
