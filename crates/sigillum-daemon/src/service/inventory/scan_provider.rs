//! One provider observation step with post-await cancellation barriers.

use std::collections::BTreeMap;

use sigillum_api::{ChainProfile, EvmProviderProfile, WalletDiscoveryBlockCursor};

use crate::operation_registry::OperationHandle;
use crate::service::chains::chain_profile_for_id;
use crate::service::{ServiceResult, SigillumService};

use super::permit2_discovery::permit2_allowance_discovery_config;
use super::support::InventoryAddressObservation;
use super::wallet_selection::DiscoveryWallet;
use super::{PreparedEvmScan, TokenRegistryObservationProbe, activity_context_for_observation};

pub(super) struct ScanAddressContext<'a> {
    pub(super) prepared: &'a PreparedEvmScan,
    pub(super) chain_profiles: &'a [ChainProfile],
    pub(super) announcement_activity: &'a BTreeMap<(u64, String), u64>,
    pub(super) chain_tip_blocks: &'a BTreeMap<String, Option<u64>>,
    pub(super) inventory: &'a crate::inventory::WalletInventoryState,
    pub(super) wallet: &'a DiscoveryWallet,
    pub(super) provider: &'a EvmProviderProfile,
    pub(super) address: &'a str,
    pub(super) derivation_path: &'a str,
    pub(super) address_index: u32,
    pub(super) started_at_unix: u64,
}

impl SigillumService {
    /// Observe one address and discard the result unless every provider await
    /// remains authorized. Returning `None` means cancellation won before the
    /// observation became eligible for a durable checkpoint.
    pub(super) async fn observe_scan_address(
        &self,
        operation: &OperationHandle,
        block_cursors: &mut Vec<WalletDiscoveryBlockCursor>,
        context: ScanAddressContext<'_>,
    ) -> ServiceResult<Option<InventoryAddressObservation>> {
        if self.discovery_scan_checkpoint(operation)? {
            return Ok(None);
        }
        let permit2_allowance_discovery = permit2_allowance_discovery_config(
            context.prepared.discover_permit2_allowances,
            &context.prepared.permit2_contract_addresses,
            &context.prepared.permit2_spender_addresses,
            context.prepared.permit2_allowance_limit,
            chain_profile_for_id(context.chain_profiles, context.provider.chain_id)
                .and_then(|profile| profile.permit2_address.as_deref()),
        )?;
        let mut observation = self
            .observe_inventory_address(
                context.wallet,
                context.provider,
                context.address,
                context.derivation_path,
                context.address_index,
                &context.prepared.block_tag,
                &context.prepared.token_addresses,
                context.prepared.token_discovery.as_ref(),
                context.prepared.allowance_discovery.as_ref(),
                permit2_allowance_discovery.as_ref(),
                context.prepared.nft_discovery.as_ref(),
                context.prepared.erc1155_discovery.as_ref(),
                context.prepared.nft_operator_approval_discovery.as_ref(),
                context.prepared.defi_position_discovery.as_ref(),
                context.prepared.claim_candidate_discovery.as_ref(),
                activity_context_for_observation(
                    context.inventory,
                    context.chain_profiles,
                    context.announcement_activity,
                    context.chain_tip_blocks,
                    context.wallet,
                    context.provider,
                    context.address,
                ),
                block_cursors,
                context.started_at_unix,
            )
            .await?;
        if self.discovery_scan_checkpoint(operation)? {
            return Ok(None);
        }
        self.apply_token_registry_probe(
            TokenRegistryObservationProbe {
                wallet: context.wallet,
                provider: context.provider,
                derivation_path: context.derivation_path,
                block_tag: &context.prepared.block_tag,
                config: context.prepared.token_registry_probe.as_ref(),
                now: context.started_at_unix,
            },
            &mut observation,
        )
        .await?;
        if self.discovery_scan_checkpoint(operation)? {
            return Ok(None);
        }
        Ok(Some(observation))
    }
}
