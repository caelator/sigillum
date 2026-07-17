//! Forget/prune machinery for the wallet-inventory store (plan task 3.2).
//!
//! The at-rest linkage ledger is not write-only: scanned-address history,
//! receive allocations, and the counterparty bindings they carry can be
//! deleted through three surfaces —
//!
//! - [`SigillumService::prune_wallet_inventory_addresses`]: the standalone
//!   `inventory/addresses/delete` route, forgetting scanned-address rows (and
//!   their holdings and per-address block cursors) by AND-combined selectors;
//! - [`SigillumService::purge_treasury_receive_address`] (in `treasury.rs`):
//!   permanently deleting a RETIRED receive allocation;
//! - the profile-delete cascade: [`prune_inventory_state`] driven with
//!   [`InventoryPruneScope::WalletProfile`] / [`InventoryPruneScope::ProviderProfile`]
//!   when a profile delete passes `prune_inventory: true`.
//!
//! What pruning does NOT do: it does not break derivation. A later scan that
//! re-derives a pruned index re-observes it and records a fresh row (new id,
//! fresh `first_seen_at_unix`) — that is expected: the point is removing
//! history, not making addresses unresolvable. Receive allocations and
//! counterparty bindings are never recreated by scans, so once purged they
//! stay forgotten. Counterparty records are operator-managed entities and
//! always remain.

use std::collections::BTreeSet;

use sigillum_api::{
    InventoryPruneSummary, WalletInventoryAddressPruneRequest, WalletInventoryAddressPruneResponse,
};

use crate::audit_log::AuditEventSpec;
use crate::inventory::WalletInventoryState;
use crate::service::evm::normalize_address;
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::support::{load_inventory_state, save_inventory_state};
use super::treasury::RECEIVE_STATUS_ACTIVE;

/// What a prune operation matches against.
pub(in crate::service) enum InventoryPruneScope<'a> {
    /// Standalone route: every provided selector must match (AND semantics).
    Selectors(&'a WalletInventoryAddressPruneRequest),
    /// Wallet-profile-delete cascade: rows attributed to this
    /// (family, profile) pair, plus its scan state and receive allocations.
    WalletProfile { family: &'a str, name: &'a str },
    /// Provider-profile-delete cascade: rows observed through this provider,
    /// plus its scan state. Allocations never reference providers.
    ProviderProfile { name: &'a str },
}

fn normalized_selector(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn address_matches(stored: &str, selector: &str) -> bool {
    stored.eq_ignore_ascii_case(selector)
}

/// Remove everything `scope` selects from `state`, returning per-store
/// counts. Pure in-memory mutation: the caller owns validation, persistence
/// (one atomic store save), and the audit event, so a failed validation
/// leaves the on-disk store untouched.
pub(in crate::service) fn prune_inventory_state(
    state: &mut WalletInventoryState,
    scope: &InventoryPruneScope<'_>,
) -> InventoryPruneSummary {
    let mut summary = InventoryPruneSummary::default();

    let address_selector = match scope {
        InventoryPruneScope::Selectors(request) => normalized_selector(request.address.as_deref()),
        _ => None,
    };

    // ── 1. Scanned-address rows ─────────────────────────────────────
    let mut deleted_address_keys: BTreeSet<(String, String, String, u64, String)> = BTreeSet::new();
    let mut deleted_address_chains: BTreeSet<(String, u64)> = BTreeSet::new();
    state.addresses.retain(|row| {
        let matched = match scope {
            InventoryPruneScope::Selectors(request) => {
                address_selector
                    .as_deref()
                    .is_none_or(|selector| address_matches(&row.address, selector))
                    && normalized_selector(request.wallet_family.as_deref())
                        .is_none_or(|family| row.wallet_family == family)
                    && normalized_selector(request.wallet_profile.as_deref())
                        .is_none_or(|profile| row.wallet_profile == profile)
                    && normalized_selector(request.provider_profile.as_deref())
                        .is_none_or(|provider| row.provider_profile == provider)
                    && request
                        .chain_id
                        .is_none_or(|chain_id| row.chain_id == chain_id)
                    && request
                        .account_index
                        .is_none_or(|account| row.account_index == Some(account))
            }
            InventoryPruneScope::WalletProfile { family, name } => {
                row.wallet_family == *family && row.wallet_profile == *name
            }
            InventoryPruneScope::ProviderProfile { name } => row.provider_profile == *name,
        };
        if matched {
            deleted_address_keys.insert((
                row.wallet_family.clone(),
                row.wallet_profile.clone(),
                row.provider_profile.clone(),
                row.chain_id,
                row.address.to_ascii_lowercase(),
            ));
            deleted_address_chains.insert((row.address.to_ascii_lowercase(), row.chain_id));
        }
        !matched
    });
    summary.addresses = deleted_address_keys.len();

    // ── 2. Holdings recorded for those addresses ────────────────────
    // Holdings carry no account_index, so an account-scoped prune removes a
    // holding exactly when its (family, profile, provider, chain, address)
    // key was among the deleted address rows. Every other scope matches on
    // the holding's own fields.
    let account_scoped = matches!(
        scope,
        InventoryPruneScope::Selectors(request) if request.account_index.is_some()
    );
    let before_holdings = state.holdings.len();
    state.holdings.retain(|row| {
        let key = (
            row.wallet_family.clone(),
            row.wallet_profile.clone(),
            row.provider_profile.clone(),
            row.chain_id,
            row.address.to_ascii_lowercase(),
        );
        let matched = match scope {
            InventoryPruneScope::Selectors(request) => {
                if account_scoped {
                    deleted_address_keys.contains(&key)
                } else {
                    address_selector
                        .as_deref()
                        .is_none_or(|selector| address_matches(&row.address, selector))
                        && normalized_selector(request.wallet_family.as_deref())
                            .is_none_or(|family| row.wallet_family == family)
                        && normalized_selector(request.wallet_profile.as_deref())
                            .is_none_or(|profile| row.wallet_profile == profile)
                        && normalized_selector(request.provider_profile.as_deref())
                            .is_none_or(|provider| row.provider_profile == provider)
                        && request
                            .chain_id
                            .is_none_or(|chain_id| row.chain_id == chain_id)
                }
            }
            InventoryPruneScope::WalletProfile { family, name } => {
                row.wallet_family == *family && row.wallet_profile == *name
            }
            InventoryPruneScope::ProviderProfile { name } => row.provider_profile == *name,
        };
        !matched
    });
    summary.holdings = before_holdings - state.holdings.len();

    // ── 3. Per-address log-scan block cursors ───────────────────────
    // Cursors are keyed by (address, chain): one dies exactly when NO
    // surviving address row covers that pair any more — a cursor for an
    // address still tracked via another provider keeps working.
    let surviving_address_chains: BTreeSet<(String, u64)> = state
        .addresses
        .iter()
        .map(|row| (row.address.to_ascii_lowercase(), row.chain_id))
        .collect();
    for job in &mut state.jobs {
        let before = job.block_cursors.len();
        job.block_cursors.retain(|cursor| {
            let key = (cursor.address.to_ascii_lowercase(), cursor.chain_id);
            !deleted_address_chains.contains(&key) || surviving_address_chains.contains(&key)
        });
        summary.block_cursors += before - job.block_cursors.len();
    }

    // ── 4. Scan state and receive allocations (cascade scopes only) ──
    let before_jobs = state.jobs.len();
    match scope {
        InventoryPruneScope::Selectors(_) => {}
        InventoryPruneScope::WalletProfile { family, name } => {
            for job in &mut state.jobs {
                let before = job.checkpoints.len();
                job.checkpoints.retain(|checkpoint| {
                    !(checkpoint.wallet_family == *family && checkpoint.wallet_profile == *name)
                });
                summary.checkpoints += before - job.checkpoints.len();
                // Job records name profiles without their family; only strip
                // the name from jobs that scanned this family, so a
                // same-named profile of another family (or a watch
                // pseudo-profile) never loses its history.
                if job.wallet_families.iter().any(|f| f == family) {
                    job.wallet_profiles.retain(|profile| profile != name);
                }
            }

            state.receive_allocations.retain(|allocation| {
                let matched =
                    allocation.wallet_family == *family && allocation.wallet_profile == *name;
                if matched {
                    if allocation.status == RECEIVE_STATUS_ACTIVE {
                        summary.allocations_active += 1;
                    } else {
                        summary.allocations_retired += 1;
                    }
                    if allocation.counterparty_id.is_some() {
                        summary.counterparty_bindings += 1;
                    }
                }
                !matched
            });
        }
        InventoryPruneScope::ProviderProfile { name } => {
            for job in &mut state.jobs {
                let before = job.checkpoints.len();
                job.checkpoints
                    .retain(|checkpoint| checkpoint.provider_profile != *name);
                summary.checkpoints += before - job.checkpoints.len();
                job.provider_profiles.retain(|provider| provider != name);
            }
        }
    }
    if !matches!(scope, InventoryPruneScope::Selectors(_)) {
        // A scan record that names no wallet profile or no provider any more
        // is pure linkage history for what was just forgotten; jobs spanning
        // surviving profiles/providers stay.
        state
            .jobs
            .retain(|job| !job.wallet_profiles.is_empty() && !job.provider_profiles.is_empty());
        summary.jobs = before_jobs - state.jobs.len();
    }

    summary
}

/// Audit fields shared by every prune event: counts only, never addresses or
/// other linkage material (same discipline as `treasury.receive.allocate`,
/// which omits derived addresses).
pub(in crate::service) fn prune_summary_fields(
    summary: &InventoryPruneSummary,
) -> (usize, usize, usize, usize, usize, usize, usize, usize) {
    (
        summary.addresses,
        summary.holdings,
        summary.jobs,
        summary.checkpoints,
        summary.block_cursors,
        summary.allocations_active,
        summary.allocations_retired,
        summary.counterparty_bindings,
    )
}

impl SigillumService {
    /// Delete scanned-address records (and their holdings and per-address
    /// block cursors) selected by the request's AND-combined selectors.
    ///
    /// Fails closed: at least one selector is required (DTO validation), a
    /// selector set that matches nothing is a 404, and the store is saved
    /// once, atomically, before the audit event is recorded.
    pub(crate) async fn prune_wallet_inventory_addresses(
        &self,
        token: Option<&str>,
        body: WalletInventoryAddressPruneRequest,
    ) -> ServiceResult<WalletInventoryAddressPruneResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        // Service-layer re-validation for non-HTTP callers: trimming an
        // all-whitespace selector set down to nothing is a validation
        // failure, and a malformed address selector is a bad request.
        let selectors = WalletInventoryAddressPruneRequest {
            address: normalized_selector(body.address.as_deref())
                .map(|address| normalize_address(&address))
                .transpose()?,
            wallet_family: normalized_selector(body.wallet_family.as_deref()),
            wallet_profile: normalized_selector(body.wallet_profile.as_deref()),
            provider_profile: normalized_selector(body.provider_profile.as_deref()),
            chain_id: body.chain_id,
            account_index: body.account_index,
        };
        if selectors.address.is_none()
            && selectors.wallet_family.is_none()
            && selectors.wallet_profile.is_none()
            && selectors.provider_profile.is_none()
            && selectors.chain_id.is_none()
            && selectors.account_index.is_none()
        {
            return Err(ServiceError::validation_failed(
                "at least one selector (address, wallet_family, wallet_profile, provider_profile, chain_id, account_index) is required",
            ));
        }

        let mut state = load_inventory_state(&self.state.base_dir)?;
        let summary =
            prune_inventory_state(&mut state, &InventoryPruneScope::Selectors(&selectors));
        if summary.addresses == 0 && summary.holdings == 0 && summary.block_cursors == 0 {
            return Err(ServiceError::not_found(
                "No inventory records matched the prune selectors.",
            ));
        }
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryAddressesPrune {
                scoped_by_address: selectors.address.is_some(),
                wallet_family: selectors.wallet_family.clone(),
                wallet_profile: selectors.wallet_profile.clone(),
                provider_profile: selectors.provider_profile.clone(),
                chain_id: selectors.chain_id,
                account_index: selectors.account_index,
                addresses: summary.addresses,
                holdings: summary.holdings,
                block_cursors: summary.block_cursors,
            },
        )?;

        Ok(WalletInventoryAddressPruneResponse {
            status: "pruned".into(),
            pruned: summary,
        })
    }

    /// Profile-delete cascade driver (plan task 3.2): prune everything the
    /// scope selects from the wallet-inventory store and record ONE audit
    /// event carrying the per-store counts. Runs inside the caller's
    /// operation guard, before the profile registry mutation, so a prune
    /// failure leaves the profile in place.
    pub(in crate::service) async fn prune_inventory_for_deleted_profile(
        &self,
        token: &str,
        profile_kind: &str,
        scope: InventoryPruneScope<'_>,
    ) -> ServiceResult<InventoryPruneSummary> {
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let summary = prune_inventory_state(&mut state, &scope);
        save_inventory_state(&self.state.base_dir, &state)?;

        let name = match &scope {
            InventoryPruneScope::WalletProfile { name, .. } => (*name).to_string(),
            InventoryPruneScope::ProviderProfile { name } => (*name).to_string(),
            InventoryPruneScope::Selectors(_) => {
                return Err(ServiceError::internal(
                    "profile prune cascade requires a profile scope",
                ));
            }
        };
        let (
            addresses,
            holdings,
            jobs,
            checkpoints,
            block_cursors,
            allocations_active,
            allocations_retired,
            counterparty_bindings,
        ) = prune_summary_fields(&summary);
        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryProfilePrune {
                profile_kind: profile_kind.to_string(),
                name,
                addresses,
                holdings,
                jobs,
                checkpoints,
                block_cursors,
                allocations_active,
                allocations_retired,
                counterparty_bindings,
            },
        )?;
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use sigillum_api::{
        TreasuryReceiveAllocation, WalletAddressActivityState, WalletAssetHolding, WalletAssetKind,
        WalletDiscoveryBlockCursor, WalletDiscoveryCheckpoint, WalletDiscoveryJob,
        WalletInventoryAddress,
    };

    use super::*;

    const ADDR_A: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ADDR_B: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn address_row(
        profile: &str,
        provider: &str,
        address: &str,
        account: Option<u32>,
    ) -> WalletInventoryAddress {
        WalletInventoryAddress {
            id: format!("addr_{profile}_{provider}_{address}"),
            wallet_family: "eth-seed".into(),
            wallet_profile: profile.into(),
            provider_profile: provider.into(),
            chain_id: 1,
            address: address.into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            derivation_pattern: Some("project".into()),
            account_index: account,
            address_index: 0,
            activity_state: WalletAddressActivityState::Funded,
            native_balance_wei_hex: "0x1".into(),
            transaction_count: 0,
            last_activity_block: None,
            classifications: Vec::new(),
            source: "local-rpc".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 2,
        }
    }

    fn holding_row(profile: &str, provider: &str, address: &str) -> WalletAssetHolding {
        WalletAssetHolding {
            id: format!("holding_{profile}_{provider}_{address}"),
            wallet_family: "eth-seed".into(),
            wallet_profile: profile.into(),
            provider_profile: provider.into(),
            chain_id: 1,
            address: address.into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            asset_kind: WalletAssetKind::Native,
            asset_address: None,
            token_id_hex: None,
            counterparty_address: None,
            protocol_address: None,
            claim_adapter: None,
            claim_index_hex: None,
            claim_proof: Vec::new(),
            metadata_uri: None,
            metadata_name: None,
            spam_label: None,
            amount_hex: "0x1".into(),
            source: "local-rpc".into(),
            status: "detected".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 2,
        }
    }

    fn allocation(
        profile: &str,
        status: &str,
        counterparty: Option<&str>,
    ) -> TreasuryReceiveAllocation {
        TreasuryReceiveAllocation {
            id: format!(
                "alloc_{profile}_{status}_{}",
                counterparty.unwrap_or("none")
            ),
            wallet_family: "eth-seed".into(),
            wallet_profile: profile.into(),
            chain_id: 1,
            chain_id_assumed: false,
            address: ADDR_A.into(),
            derivation_path: "m/44'/60'/0'/0/3".into(),
            address_index: 3,
            purpose: "invoices".into(),
            label: None,
            status: status.into(),
            created_at_unix: 1,
            retired_at_unix: (status == "retired").then_some(2),
            counterparty_id: counterparty.map(str::to_string),
        }
    }

    fn job(profiles: &[&str], providers: &[&str]) -> WalletDiscoveryJob {
        WalletDiscoveryJob {
            id: format!("job_{}", profiles.join("_")),
            status: "completed".into(),
            source: "local-rpc".into(),
            wallet_families: vec!["eth-seed".into()],
            wallet_profiles: profiles.iter().map(|p| p.to_string()).collect(),
            provider_profiles: providers.iter().map(|p| p.to_string()).collect(),
            chain_ids: vec![1],
            gap_limit: 20,
            max_index: 100,
            addresses_scanned: 3,
            active_addresses: 1,
            holdings_detected: 1,
            checkpoints: profiles
                .iter()
                .map(|profile| WalletDiscoveryCheckpoint {
                    wallet_family: "eth-seed".into(),
                    wallet_profile: profile.to_string(),
                    provider_profile: providers[0].into(),
                    derivation_pattern: Some("project".into()),
                    account_index: Some(0),
                    next_index: 3,
                    last_scanned_index: Some(2),
                    consecutive_empty: 0,
                    completed: true,
                    updated_at_unix: 2,
                })
                .collect(),
            block_cursors: vec![WalletDiscoveryBlockCursor {
                address: ADDR_A.into(),
                chain_id: 1,
                topic_family: "erc20-transfer".into(),
                last_scanned_block: 10,
                updated_at_unix: 2,
            }],
            started_at_unix: 1,
            completed_at_unix: Some(2),
            last_error: None,
            partition_providers: None,
            provider_partition_observations: Vec::new(),
        }
    }

    fn selectors(request: &WalletInventoryAddressPruneRequest) -> InventoryPruneScope<'_> {
        InventoryPruneScope::Selectors(request)
    }

    #[test]
    fn selector_prune_combines_with_and_semantics() {
        let mut state = WalletInventoryState::default();
        state
            .addresses
            .push(address_row("seed-main", "mainnet", ADDR_A, Some(0)));
        state
            .addresses
            .push(address_row("seed-main", "fallback", ADDR_A, Some(0)));
        state
            .addresses
            .push(address_row("seed-alt", "mainnet", ADDR_B, Some(0)));
        state
            .holdings
            .push(holding_row("seed-main", "mainnet", ADDR_A));
        state
            .holdings
            .push(holding_row("seed-main", "fallback", ADDR_A));
        state.jobs.push(job(&["seed-main"], &["mainnet"]));

        let summary = prune_inventory_state(
            &mut state,
            &selectors(&WalletInventoryAddressPruneRequest {
                address: Some(ADDR_A.into()),
                provider_profile: Some("mainnet".into()),
                ..WalletInventoryAddressPruneRequest::default()
            }),
        );

        // Only the (address, provider) intersection row and its holding die;
        // the fallback-provider twin and the other profile survive.
        assert_eq!(summary.addresses, 1);
        assert_eq!(summary.holdings, 1);
        // The block cursor survives too: the address is still tracked on
        // chain 1 via the fallback provider's surviving row.
        assert_eq!(summary.block_cursors, 0);
        assert_eq!(state.jobs[0].block_cursors.len(), 1);
        assert_eq!(state.addresses.len(), 2);
        assert_eq!(state.holdings.len(), 1);
        // Selector scope never touches checkpoints, jobs, or allocations.
        assert_eq!(summary.jobs, 0);
        assert_eq!(summary.checkpoints, 0);
        assert_eq!(state.jobs.len(), 1);
    }

    #[test]
    fn account_scoped_prune_removes_holdings_only_with_their_address_rows() {
        let mut state = WalletInventoryState::default();
        state
            .addresses
            .push(address_row("seed-main", "mainnet", ADDR_A, Some(0)));
        state
            .addresses
            .push(address_row("seed-main", "mainnet", ADDR_B, Some(3)));
        // A holding on account 3's address and one on account 0's address.
        state
            .holdings
            .push(holding_row("seed-main", "mainnet", ADDR_A));
        state
            .holdings
            .push(holding_row("seed-main", "mainnet", ADDR_B));

        let summary = prune_inventory_state(
            &mut state,
            &selectors(&WalletInventoryAddressPruneRequest {
                account_index: Some(3),
                ..WalletInventoryAddressPruneRequest::default()
            }),
        );

        assert_eq!(summary.addresses, 1);
        assert_eq!(summary.holdings, 1);
        assert_eq!(state.addresses.len(), 1);
        assert_eq!(state.addresses[0].address, ADDR_A);
        assert_eq!(state.holdings.len(), 1);
        assert_eq!(state.holdings[0].address, ADDR_A);
    }

    #[test]
    fn wallet_profile_cascade_removes_rows_scan_state_allocations_and_bindings() {
        let mut state = WalletInventoryState::default();
        state
            .addresses
            .push(address_row("seed-main", "mainnet", ADDR_A, Some(0)));
        state
            .addresses
            .push(address_row("seed-alt", "mainnet", ADDR_B, Some(0)));
        state
            .holdings
            .push(holding_row("seed-main", "mainnet", ADDR_A));
        state.jobs.push(job(&["seed-main"], &["mainnet"]));
        state
            .jobs
            .push(job(&["seed-main", "seed-alt"], &["mainnet"]));
        state
            .receive_allocations
            .push(allocation("seed-main", "active", Some("cp_1")));
        state
            .receive_allocations
            .push(allocation("seed-main", "retired", None));
        state
            .receive_allocations
            .push(allocation("seed-alt", "active", Some("cp_1")));

        let summary = prune_inventory_state(
            &mut state,
            &InventoryPruneScope::WalletProfile {
                family: "eth-seed",
                name: "seed-main",
            },
        );

        assert_eq!(summary.addresses, 1);
        assert_eq!(summary.holdings, 1);
        assert_eq!(summary.jobs, 1, "the single-profile job dies");
        assert_eq!(
            summary.checkpoints, 2,
            "both jobs lose the profile's checkpoints"
        );
        assert_eq!(summary.block_cursors, 2);
        assert_eq!(summary.allocations_active, 1);
        assert_eq!(summary.allocations_retired, 1);
        assert_eq!(summary.counterparty_bindings, 1);
        assert_eq!(state.addresses.len(), 1);
        assert_eq!(state.addresses[0].wallet_profile, "seed-alt");
        assert_eq!(state.jobs.len(), 1);
        assert_eq!(state.jobs[0].wallet_profiles, vec!["seed-alt".to_string()]);
        assert_eq!(state.receive_allocations.len(), 1);
        assert_eq!(state.receive_allocations[0].wallet_profile, "seed-alt");
    }

    #[test]
    fn provider_profile_cascade_removes_only_that_providers_rows() {
        let mut state = WalletInventoryState::default();
        state
            .addresses
            .push(address_row("seed-main", "mainnet", ADDR_A, Some(0)));
        state
            .addresses
            .push(address_row("seed-main", "legacy", ADDR_A, Some(0)));
        state
            .holdings
            .push(holding_row("seed-main", "legacy", ADDR_A));
        state.jobs.push(job(&["seed-main"], &["legacy"]));
        state
            .receive_allocations
            .push(allocation("seed-main", "active", None));

        let summary = prune_inventory_state(
            &mut state,
            &InventoryPruneScope::ProviderProfile { name: "legacy" },
        );

        assert_eq!(summary.addresses, 1);
        assert_eq!(summary.holdings, 1);
        assert_eq!(summary.checkpoints, 1);
        assert_eq!(summary.jobs, 1, "the legacy-only job dies");
        // The cursor is not counted as pruned: the address is still tracked
        // via mainnet (it disappears with the deleted job record).
        assert_eq!(summary.block_cursors, 0);
        // Allocations never reference providers: they survive untouched.
        assert_eq!(summary.allocations_active, 0);
        assert_eq!(state.receive_allocations.len(), 1);
        assert_eq!(state.addresses.len(), 1);
        assert_eq!(state.addresses[0].provider_profile, "mainnet");
    }
}
