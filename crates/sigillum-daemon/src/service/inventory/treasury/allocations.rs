use sigillum_api::{
    TreasuryReceiveAllocateRequest, TreasuryReceiveAllocation,
    TreasuryReceiveAllocationListResponse, TreasuryReceiveAllocationMutationResponse,
    TreasuryReceivePurgeRequest, TreasuryReceivePurgeResponse, TreasuryReceiveRotateRequest,
    WalletInventoryAddress,
};

use crate::audit_log::AuditEventSpec;
use crate::inventory::WalletInventoryState;
use crate::service::evm::normalize_address;
use crate::service::helpers::{map_xpub_error, now_unix, random_id};
use crate::service::transaction_policy::{TransactionPolicyCheck, TransactionPolicyKind};
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::super::support::{
    load_inventory_state, save_inventory_state, trimmed_optional, trimmed_required,
};
use super::super::wallet_selection::{
    DiscoveryWallet, SeedDerivationPattern, derive_discovery_wallet_address,
    select_discovery_wallets,
};
use super::super::{WALLET_FAMILY_ETH_SEED, WALLET_FAMILY_ETH_XPUB};
use super::policy::validated_cap_hex;

pub(in crate::service::inventory) const RECEIVE_STATUS_ACTIVE: &str = "active";
pub(in crate::service::inventory) const RECEIVE_STATUS_RETIRED: &str = "retired";
/// Absurdly large but bounded: receive indices beyond this point indicate a
/// runaway caller, not a treasury that genuinely needs a million addresses.
const MAX_RECEIVE_INDEX: u32 = 1_000_000;

struct OneTimePolicy {
    sweep_destination_address: String,
    min_sweep_amount_hex: Option<String>,
    purge_after_sweep: bool,
}

impl SigillumService {
    pub(crate) fn list_treasury_receive_allocations(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<TreasuryReceiveAllocationListResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        let queue = crate::queue_store::load_queue(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load queue: {error}")))?;
        let pause_latched = self.state.queue_execution_pause_latched();
        let allocations = state
            .receive_allocations
            .iter()
            .map(|allocation| {
                super::super::one_time::with_one_time_lifecycle(
                    allocation,
                    &state,
                    &queue,
                    pause_latched,
                )
            })
            .collect();
        Ok(TreasuryReceiveAllocationListResponse { allocations })
    }

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

        let one_time = body.one_time.unwrap_or(false);
        if !one_time
            && (body.sweep_destination_address.is_some()
                || body.min_sweep_amount_hex.is_some()
                || body.purge_after_sweep.is_some())
        {
            return Err(ServiceError::bad_request(
                "sweep_destination_address, min_sweep_amount_hex, and purge_after_sweep require one_time.",
            ));
        }
        let one_time_policy = if one_time {
            let destination = body
                .sweep_destination_address
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ServiceError::bad_request(
                        "sweep_destination_address is required when one_time is set.",
                    )
                })?;
            let destination = normalize_address(destination)?;
            let min_sweep_amount_hex =
                validated_cap_hex("min_sweep_amount_hex", body.min_sweep_amount_hex)?;
            // Same destination allowlist/policy rules as any sweep
            // destination (re-checked at every enqueue evaluation too).
            self.authorize_transaction_policy(TransactionPolicyCheck {
                kind: TransactionPolicyKind::RoutedTransfer,
                destination_address: Some(&destination),
                asset_kind: "native",
                amount_hex: min_sweep_amount_hex.as_deref().unwrap_or("0x0"),
            })?;
            Some(OneTimePolicy {
                sweep_destination_address: destination,
                min_sweep_amount_hex,
                purge_after_sweep: body.purge_after_sweep.unwrap_or(false),
            })
        } else {
            None
        };

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
            one_time_policy,
        )?;
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::TreasuryReceiveAllocate {
                wallet_profile: allocation.wallet_profile.clone(),
                purpose: allocation.purpose.clone(),
                one_time: allocation.one_time,
            },
        )?;
        if let Some(name) = counterparty_name {
            self.record_audit(
                self.state.active_compartment_id_for(token),
                AuditEventSpec::TreasuryReceiveBind { name },
            )?;
        }

        let allocation = self.with_lifecycle_fields(&state, &allocation)?;
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
        // Rotation carries the one-time policy to the replacement: rotate
        // means "same promise, fresh address". A one-time record always has a
        // destination (creation validates); a blank one means corruption.
        let one_time_policy = if existing.one_time {
            let destination = existing
                .sweep_destination_address
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ServiceError::bad_request(
                        "One-time receive allocation is missing its sweep destination.",
                    )
                })?
                .to_string();
            Some(OneTimePolicy {
                sweep_destination_address: destination,
                min_sweep_amount_hex: existing.min_sweep_amount_hex.clone(),
                purge_after_sweep: existing.purge_after_sweep,
            })
        } else {
            None
        };
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
            one_time_policy,
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

        let allocation = self.with_lifecycle_fields(&state, &allocation)?;
        Ok(TreasuryReceiveAllocationMutationResponse {
            status: "rotated".into(),
            allocation,
        })
    }

    /// Permanently delete a RETIRED receive allocation (plan task 3.2).
    ///
    /// Purging is the forget half of the receive-address lifecycle: the
    /// allocation record — and the address → purpose → counterparty linkage
    /// it carries — leaves the store for good. Active allocations are
    /// refused with 409 (rotate retires first; a profile delete with
    /// `prune_inventory` retire-then-purges in one operation). The
    /// counterparty record itself always remains; only the binding dies.
    pub(crate) async fn purge_treasury_receive_address(
        &self,
        token: Option<&str>,
        body: TreasuryReceivePurgeRequest,
    ) -> ServiceResult<TreasuryReceivePurgeResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let allocation_id = body.allocation_id.trim().to_string();

        let mut state = load_inventory_state(&self.state.base_dir)?;
        let Some(position) = state
            .receive_allocations
            .iter()
            .position(|allocation| allocation.id == allocation_id)
        else {
            return Err(ServiceError::not_found("Receive allocation not found."));
        };
        if state.receive_allocations[position].status == RECEIVE_STATUS_ACTIVE {
            return Err(ServiceError::conflict(
                "Receive allocation is still active; rotate it (rotation retires the address) before purging.",
            ));
        }
        let allocation = state.receive_allocations.remove(position);
        save_inventory_state(&self.state.base_dir, &state)?;

        let counterparty_binding_removed = allocation.counterparty_id.is_some();
        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::TreasuryReceivePurge {
                id: allocation_id.clone(),
                counterparty_binding_removed,
            },
        )?;

        Ok(TreasuryReceivePurgeResponse {
            status: "purged".into(),
            allocation_id,
            counterparty_binding_removed,
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
    ///
    /// One-time mode (plan task 3.3) requires a signing wallet: the
    /// auto-sweep signs from the seed vault, so xpub/watch-only profiles are
    /// rejected up front rather than failing at sweep time.
    fn issue_receive_allocation(
        &self,
        state: &mut WalletInventoryState,
        wallet_profile: &str,
        purpose: String,
        label: Option<String>,
        counterparty_id: Option<String>,
        one_time_policy: Option<OneTimePolicy>,
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
        if one_time_policy.is_some() && wallet.family != WALLET_FAMILY_ETH_SEED {
            return Err(ServiceError::bad_request(
                "One-time allocations require an eth-seed wallet profile (the auto-sweep signs locally).",
            ));
        }
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
        let (one_time, sweep_destination_address, min_sweep_amount_hex, purge_after_sweep) =
            match one_time_policy {
                Some(policy) => (
                    true,
                    Some(policy.sweep_destination_address),
                    policy.min_sweep_amount_hex,
                    policy.purge_after_sweep,
                ),
                None => (false, None, None, false),
            };
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
            one_time,
            sweep_destination_address,
            min_sweep_amount_hex,
            purge_after_sweep,
            sweep_job_id: None,
            lifecycle_state: None,
            sweep_blocker: None,
        };
        state.receive_allocations.push(allocation.clone());
        Ok(allocation)
    }

    /// Clone an allocation with its read-time one-time lifecycle fields
    /// populated (no-op for non-one-time records). Never mutates the store.
    fn with_lifecycle_fields(
        &self,
        state: &WalletInventoryState,
        allocation: &TreasuryReceiveAllocation,
    ) -> ServiceResult<TreasuryReceiveAllocation> {
        if !allocation.one_time {
            return Ok(allocation.clone());
        }
        let queue = crate::queue_store::load_queue(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load queue: {error}")))?;
        Ok(super::super::one_time::with_one_time_lifecycle(
            allocation,
            state,
            &queue,
            self.state.queue_execution_pause_latched(),
        ))
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
