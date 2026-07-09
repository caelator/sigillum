use sigillum_api::TreasuryPolicy;
use sigillum_core::decode_quantity_hex;

use super::evm::normalize_address;
use super::helpers::compare_u256;
use super::{ServiceError, ServiceResult, SigillumService};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransactionPolicyAction {
    Allow,
    BlockDestination,
    BlockStepCap,
    // consumed by W7.2 enqueue-time treasury cap re-checks (docs/release-1.0-plan.md)
    #[allow(dead_code)]
    BlockPlanCap,
    // consumed by W7.2 enqueue-time simulation re-check (docs/release-1.0-plan.md)
    #[allow(dead_code)]
    BlockUnsimulated,
    BlockRawDigest,
    /// Signer resolution defensively re-checked the source wallet family and
    /// found it non-derivable (watch-only or unknown) at execution time.
    /// Enqueue-validation makes this unreachable by construction (blockers
    /// refuse watch-only steps before they can be enqueued); this variant is
    /// the fail-closed re-check per W7.3's spec.
    BlockWatchOnlySigner,
}

impl TransactionPolicyAction {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::BlockDestination => "block_destination",
            Self::BlockStepCap => "block_step_cap",
            Self::BlockPlanCap => "block_plan_cap",
            Self::BlockUnsimulated => "block_unsimulated",
            Self::BlockRawDigest => "block_raw_digest",
            Self::BlockWatchOnlySigner => "block_watch_only_signer",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransactionPolicyKind {
    RawDigest,
    RoutedTransfer,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TransactionPolicyCheck<'a> {
    pub(crate) kind: TransactionPolicyKind,
    pub(crate) destination_address: Option<&'a str>,
    pub(crate) asset_kind: &'a str,
    pub(crate) amount_hex: &'a str,
}

pub(crate) fn transaction_policy_actions(
    policy: &TreasuryPolicy,
    check: TransactionPolicyCheck<'_>,
) -> Vec<TransactionPolicyAction> {
    if !policy.enabled {
        return Vec::new();
    }

    let mut actions = Vec::new();
    if matches!(check.kind, TransactionPolicyKind::RawDigest) && !policy.allow_raw_digest_signing {
        actions.push(TransactionPolicyAction::BlockRawDigest);
    }

    if matches!(check.kind, TransactionPolicyKind::RoutedTransfer) {
        if let Some(destination) = check.destination_address {
            let normalized = normalize_address(destination).unwrap_or_else(|_| destination.into());
            let allowlisted = policy
                .allowed_destinations
                .iter()
                .any(|allowed| allowed.address.eq_ignore_ascii_case(&normalized));
            if !allowlisted {
                actions.push(TransactionPolicyAction::BlockDestination);
            }
        }
    }

    if check.asset_kind == "native" {
        if let Some(cap_hex) = policy.max_step_native_wei_hex.as_deref() {
            if let Ok(cap) = decode_quantity_hex(cap_hex) {
                let amount = decode_quantity_hex(check.amount_hex).unwrap_or([0u8; 32]);
                if compare_u256(&amount, &cap).is_gt() {
                    actions.push(TransactionPolicyAction::BlockStepCap);
                }
            }
        }
    }

    actions
}

impl SigillumService {
    pub(crate) fn authorize_transaction_policy(
        &self,
        check: TransactionPolicyCheck<'_>,
    ) -> ServiceResult<()> {
        let state =
            crate::inventory::load_wallet_inventory(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load wallet inventory: {error}"))
            })?;
        let Some(policy) = state.treasury_policy.as_ref() else {
            return Ok(());
        };
        let actions = transaction_policy_actions(policy, check);
        if actions.is_empty() {
            Ok(())
        } else {
            let action = actions
                .first()
                .copied()
                .unwrap_or(TransactionPolicyAction::Allow)
                .as_str();
            Err(ServiceError::policy_violation(action))
        }
    }
}
