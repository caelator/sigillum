//! Sponsor gas top-ups for stealth deposits (execution + dependency ordering).
//!
//! A stealth ERC-20 deposit whose stealth address lacks native gas cannot be
//! swept. When the treasury policy allows sponsor top-ups
//! (`allow_gas_topups`), the deposit sweep flow enqueues an
//! `EthStealthGasTopup` job ahead of the sweep: a native transfer of 1.5x the
//! sweep's estimated gas from the stealth wallet's gas sponsor to the stealth
//! address. The sweep job records the top-up in `prerequisite_job_ids` and
//! the drain loop defers it until the top-up has broadcast (W6.4-style
//! dependency ordering, mirroring `PlanStepExecution`); the sweep's own
//! on-chain gas balance check then remains the authoritative gate until the
//! top-up confirms.
//!
//! The sponsor is NOT operator-configured per deposit: it is derived
//! deterministically from the compartment master key
//! ([`sigillum_core::derive_sigillum_ethereum_stealth_gas_sponsor`]), so the
//! sponsorship model is recoverable from the vault alone. The operator funds
//! the sponsor address out-of-band. Execution re-verifies both the sponsor's
//! solvency (balance >= top-up value + its own transfer gas) and that the
//! derived key matches the recorded sponsor address before any signing —
//! both fail closed to `blocked`, never to a signature.

use std::collections::HashMap;

use sigillum_api::EvmProviderProfile;
use sigillum_core::{
    EthereumEip1559Transfer, VaultLifecycle, decode_quantity_hex,
    derive_sigillum_ethereum_stealth_gas_sponsor, sign_ethereum_native_transfer,
};

use crate::service::helpers::{compare_u256, map_wallet_error, multiply_u256_u64};
use crate::service::inventory::treasury::add_u256;
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::state::normalize_queue_state;
use super::{
    QUEUE_STATE_CONFIRMED, QUEUE_STATE_FAILED_TERMINAL, QUEUE_STATE_LEGACY_FAILED,
    QUEUE_STATE_OPERATOR_ACTION_REQUIRED, QUEUE_STATE_SENT, QueueExecution,
};

impl SigillumService {
    /// Execute an `EthStealthGasTopup` job: broadcast a native transfer of
    /// `value_wei_hex` from the stealth wallet's gas sponsor to the deposit's
    /// stealth address.
    pub(in crate::service::queue) async fn process_eth_stealth_gas_topup(
        &self,
        wallet_profile: &str,
        sponsor_address: &str,
        destination_address: &str,
        value_wei_hex: &str,
        gas_limit_override: Option<u64>,
    ) -> ServiceResult<QueueExecution> {
        let (provider, wallet) = self.resolve_wallet_profile(wallet_profile)?;
        let value = decode_quantity_hex(value_wei_hex).map_err(map_wallet_error)?;
        let gas_limit = gas_limit_override
            .or(provider.native_gas_limit)
            .unwrap_or(21_000);
        let max_priority_fee = static_max_priority_fee(&provider)?;
        let max_fee = static_max_fee(&provider)?;

        // Solvency re-check at execution time: the sponsor must still cover
        // the top-up value plus its own transfer gas. Planned against a fresh
        // balance at enqueue time, re-verified here before any signing.
        let sponsor_balance_hex = self
            .evm_native_balance_for_provider(
                provider.compartment_id,
                &provider,
                sponsor_address,
                "latest",
            )
            .await?;
        let sponsor_balance =
            decode_quantity_hex(&sponsor_balance_hex).map_err(map_wallet_error)?;
        let required = add_u256(&value, &multiply_u256_u64(&max_fee, gas_limit));
        if compare_u256(&sponsor_balance, &required).is_lt() {
            return Ok(QueueExecution::Blocked(
                "gas sponsor has insufficient native balance for top-up".into(),
            ));
        }

        // Derive the sponsor key from the compartment master key; a locked
        // compartment defers the job (fail closed, retryable), mirroring the
        // plan-step `signer_unavailable` handling.
        let sponsor = self.with_vault(wallet.compartment_id, |vault| {
            Ok(vault.extract_master_key().and_then(|master_key| {
                derive_sigillum_ethereum_stealth_gas_sponsor(master_key.as_ref(), &wallet.wallet)
                    .ok()
            }))
        })?;
        let Some(sponsor) = sponsor else {
            return Ok(QueueExecution::Blocked(
                "signer_unavailable: wallet compartment is locked".into(),
            ));
        };
        // Defense in depth: the derived key must match the recorded sponsor
        // address, exactly like the seed signer's source-address check.
        if !sponsor
            .sponsor_address()
            .eq_ignore_ascii_case(sponsor_address)
        {
            return Ok(QueueExecution::Blocked(format!(
                "block_watch_only_signer: derived gas sponsor {} does not match the job's \
                 sponsor address {sponsor_address}",
                sponsor.sponsor_address()
            )));
        }

        let nonce = self
            .evm_transaction_count_for_provider(
                provider.compartment_id,
                &provider,
                sponsor_address,
                "pending",
            )
            .await?;
        let signing_key = sponsor.signing_key();
        let signed = sign_ethereum_native_transfer(
            &signing_key,
            &EthereumEip1559Transfer {
                chain_id: wallet.chain_id.unwrap_or(provider.chain_id),
                nonce,
                max_priority_fee_per_gas: max_priority_fee,
                max_fee_per_gas: max_fee,
                gas_limit,
                destination_address: destination_address.into(),
                value,
            },
        )
        .map_err(map_wallet_error)?;
        drop(sponsor);
        Ok(QueueExecution::prepared_from_signed(signed))
    }

    /// Derive the gas sponsor address for a stealth wallet profile, when the
    /// wallet's compartment is unlocked. `None` (locked compartment) means
    /// "no sponsor available right now" — callers skip top-up planning and
    /// leave the deposit on its manual-funding path.
    pub(in crate::service) fn stealth_gas_sponsor_address(
        &self,
        compartment_id: usize,
        wallet: &str,
    ) -> ServiceResult<Option<String>> {
        self.with_vault(compartment_id, |vault| {
            Ok(vault.extract_master_key().and_then(|master_key| {
                derive_sigillum_ethereum_stealth_gas_sponsor(master_key.as_ref(), wallet)
                    .ok()
                    .map(|sponsor| sponsor.sponsor_address().to_string())
            }))
        })
    }
}

/// W6.4-style dependency ordering for stealth sweep jobs, mirroring
/// `plan_steps.rs::dependency_block_reason` with one extension: a
/// prerequisite in `confirmed` also counts as met (strictly beyond `sent`;
/// stealth-family jobs treat `sent` as their terminal broadcast state, but
/// accepting `confirmed` keeps the check correct if a plan-step job is ever
/// wired as a prerequisite). An unmet prerequisite defers the dependent
/// (`blocked`, retried on a later drain); a failed or missing prerequisite
/// halts it the same way, naming the prerequisite.
pub(in crate::service::queue) fn sweep_dependency_block_reason(
    prerequisite_job_ids: &[String],
    job_states: &HashMap<String, String>,
) -> Option<String> {
    for prerequisite_id in prerequisite_job_ids {
        match job_states.get(prerequisite_id).map(String::as_str) {
            Some(state)
                if matches!(
                    normalize_queue_state(state),
                    QUEUE_STATE_SENT | QUEUE_STATE_CONFIRMED
                ) =>
            {
                continue;
            }
            Some(state)
                if matches!(
                    normalize_queue_state(state),
                    QUEUE_STATE_FAILED_TERMINAL
                        | QUEUE_STATE_LEGACY_FAILED
                        | QUEUE_STATE_OPERATOR_ACTION_REQUIRED
                ) =>
            {
                return Some(format!(
                    "dependency_failed: prerequisite job {prerequisite_id} is in state \
                     '{state}'; this sweep cannot proceed until the prerequisite is resolved"
                ));
            }
            Some(state) => {
                return Some(format!(
                    "dependency_pending: prerequisite job {prerequisite_id} has not yet \
                     succeeded (state='{state}')"
                ));
            }
            None => {
                return Some(format!(
                    "dependency_missing: prerequisite job {prerequisite_id} was not found in \
                     the queue"
                ));
            }
        }
    }
    None
}

fn static_max_priority_fee(provider: &EvmProviderProfile) -> ServiceResult<[u8; 32]> {
    let hex = provider
        .max_priority_fee_per_gas_hex
        .as_deref()
        .ok_or_else(|| {
            ServiceError::bad_request("provider profile is missing max_priority_fee_per_gas_hex")
        })?;
    decode_quantity_hex(hex).map_err(map_wallet_error)
}

fn static_max_fee(provider: &EvmProviderProfile) -> ServiceResult<[u8; 32]> {
    let hex = provider.max_fee_per_gas_hex.as_deref().ok_or_else(|| {
        ServiceError::bad_request("provider profile is missing max_fee_per_gas_hex")
    })?;
    decode_quantity_hex(hex).map_err(map_wallet_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn states(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(id, state)| (id.to_string(), state.to_string()))
            .collect()
    }

    #[test]
    fn dependency_defers_on_pending_prerequisite() {
        let reason =
            sweep_dependency_block_reason(&["job_a".into()], &states(&[("job_a", "queued")]))
                .unwrap();
        assert!(reason.starts_with("dependency_pending:"), "{reason}");
    }

    #[test]
    fn dependency_halts_on_failed_prerequisite_naming_it() {
        let reason = sweep_dependency_block_reason(
            &["job_a".into()],
            &states(&[("job_a", "failed_terminal")]),
        )
        .unwrap();
        assert!(reason.starts_with("dependency_failed:"), "{reason}");
        assert!(reason.contains("job_a"), "{reason}");

        let reason = sweep_dependency_block_reason(
            &["job_a".into()],
            &states(&[("job_a", "operator_action_required")]),
        )
        .unwrap();
        assert!(reason.starts_with("dependency_failed:"), "{reason}");
    }

    #[test]
    fn dependency_blocks_on_missing_prerequisite() {
        let reason = sweep_dependency_block_reason(&["ghost".into()], &states(&[])).unwrap();
        assert!(reason.starts_with("dependency_missing:"), "{reason}");
    }

    #[test]
    fn dependency_clears_once_prerequisite_broadcasts_or_confirms() {
        let ids = vec!["job_a".to_string()];
        assert_eq!(
            sweep_dependency_block_reason(&ids, &states(&[("job_a", "sent")])),
            None
        );
        assert_eq!(
            sweep_dependency_block_reason(&ids, &states(&[("job_a", "confirmed")])),
            None
        );
        // Legacy alias normalizes to failed, not to met.
        assert!(
            sweep_dependency_block_reason(&ids, &states(&[("job_a", "failed")]))
                .unwrap()
                .starts_with("dependency_failed:")
        );
    }

    #[test]
    fn dependency_none_for_a_sweep_with_no_prerequisites() {
        assert_eq!(sweep_dependency_block_reason(&[], &states(&[])), None);
    }
}
