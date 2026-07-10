//! Post-broadcast execution semantics for `PlanStepExecution` jobs (W7.4):
//! broadcast-error classification (nonce-too-low / underpriced / revert),
//! the single allowed fee bump, and receipt-confirmation polling against
//! the chain registry's `finality_blocks` (W1.1).
//!
//! Scope: legacy `EthSeed*`/`EthStealth*` queue families are UNCHANGED by
//! this module — their `sent` state keeps meaning "broadcast, done" exactly
//! as it did before W7.4. Only `PlanStepExecution` jobs get the fuller
//! semantics here, matching the W7.4 task's file scope
//! (`plan_steps.rs`/`processing.rs`) and keeping the legacy families'
//! drain-time behavior byte-identical.

use sigillum_api::{EvmProviderProfile, PlanStepExecutionPayload};

use crate::service::chains::finality_blocks_for_chain;
use crate::service::helpers::now_unix;
use crate::service::{ServiceError, ServiceResult, SigillumService};

/// Wall-clock budget since broadcast before an unconfirmed tx parks to
/// `operator_action_required` (carrying the tx hash — the broadcast is
/// NEVER assumed to have failed). The daemon polls at most once per
/// drain/maintenance cycle rather than blocking or sleeping in-process, so
/// this is a WALL-CLOCK window, not a poll-attempt count — it survives a
/// restart because `broadcast_at_unix` is persisted (E2).
pub(in crate::service::queue) const RECEIPT_CONFIRMATION_TIMEOUT_SECS: u64 = 3600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::service::queue) enum BroadcastErrorClass {
    NonceTooLow,
    Underpriced,
    Revert,
}

/// Text classification of a broadcast-time provider error, mirroring the
/// house style already used for retry/queue-failure classification
/// (`service/queue/failure.rs`). Provider error wording is not standardized
/// across nodes; this matches common geth/erigon/infura phrasings.
pub(in crate::service::queue) fn classify_broadcast_error(
    message: &str,
) -> Option<BroadcastErrorClass> {
    let lower = message.to_ascii_lowercase();
    if lower.contains("nonce too low") || lower.contains("nonce is too low") {
        return Some(BroadcastErrorClass::NonceTooLow);
    }
    if lower.contains("underpriced") || lower.contains("replacement transaction") {
        return Some(BroadcastErrorClass::Underpriced);
    }
    if lower.contains("revert") || lower.contains("always failing transaction") {
        return Some(BroadcastErrorClass::Revert);
    }
    None
}

/// Outcome of one receipt-poll attempt — at most one
/// `eth_getTransactionReceipt` round trip (plus, when confirmations are
/// actually required, one `eth_blockNumber` round trip) — never a blocking
/// loop, satisfying the "bounded polling budget per drain/maintenance
/// cycle" rule.
pub(in crate::service::queue) enum ReceiptPoll {
    /// No receipt yet and the wall-clock timeout has not elapsed.
    Pending,
    /// A receipt exists (success) but fewer than the required confirmations
    /// have accrued; recorded so the operator sees partial progress without
    /// the job leaving `sent`.
    PartiallyConfirmed {
        block_number: u64,
        gas_used_hex: String,
        confirmations: u64,
    },
    /// Success with the chain's configured finality depth satisfied.
    Confirmed {
        block_number: u64,
        gas_used_hex: String,
        confirmations: u64,
    },
    /// The receipt shows an on-chain revert.
    Reverted {
        block_number: u64,
        gas_used_hex: String,
    },
    /// No receipt has appeared within the wall-clock confirmation budget.
    TimedOut,
}

impl SigillumService {
    /// Poll for a `PlanStepExecution` job's receipt. Never signs or
    /// broadcasts anything — safe to call after a restart using only the
    /// persisted transaction hash and broadcast time (E2 crash-resumption).
    pub(in crate::service::queue) async fn poll_plan_step_receipt(
        &self,
        provider: &EvmProviderProfile,
        chain_id: u64,
        transaction_hash_hex: &str,
        broadcast_at_unix: u64,
    ) -> ServiceResult<ReceiptPoll> {
        let receipt = self
            .evm_transaction_receipt_for_provider(
                provider.compartment_id,
                provider,
                transaction_hash_hex,
            )
            .await?;
        let Some(receipt) = receipt else {
            let elapsed = now_unix().saturating_sub(broadcast_at_unix);
            return Ok(if elapsed > RECEIPT_CONFIRMATION_TIMEOUT_SECS {
                ReceiptPoll::TimedOut
            } else {
                ReceiptPoll::Pending
            });
        };
        if !receipt.status_success {
            return Ok(ReceiptPoll::Reverted {
                block_number: receipt.block_number,
                gas_used_hex: receipt.gas_used_hex,
            });
        }
        let required = self.finality_blocks_for_plan_step_chain(chain_id)?;
        let confirmations = if required == 0 {
            1
        } else {
            let current_block = self
                .evm_block_number_for_provider(provider.compartment_id, provider)
                .await?;
            current_block.saturating_sub(receipt.block_number) + 1
        };
        if confirmations >= required.max(1) {
            Ok(ReceiptPoll::Confirmed {
                block_number: receipt.block_number,
                gas_used_hex: receipt.gas_used_hex,
                confirmations,
            })
        } else {
            Ok(ReceiptPoll::PartiallyConfirmed {
                block_number: receipt.block_number,
                gas_used_hex: receipt.gas_used_hex,
                confirmations,
            })
        }
    }

    /// W7.4 resume path: the job already broadcast in a prior drain call.
    /// Poll for a receipt using the persisted transaction hash — never
    /// resolves a signing key, never signs, never broadcasts.
    pub(in crate::service::queue) async fn resume_plan_step_confirmation(
        &self,
        payload: &PlanStepExecutionPayload,
        job_transaction_hash_hex: Option<&str>,
        job_broadcast_at_unix: Option<u64>,
    ) -> ServiceResult<super::QueueExecution> {
        let Some(transaction_hash_hex) = job_transaction_hash_hex else {
            // Defensive: a `sent` `PlanStepExecution` job must carry a tx
            // hash by construction (set the moment the drain loop records
            // `Sent`). Fail closed rather than panic if that invariant is
            // ever violated (e.g. a hand-edited/corrupted queue store).
            return Ok(super::QueueExecution::OperatorActionRequired(
                "sent_job_missing_transaction_hash: cannot resume receipt polling for a \
                 broadcast job with no recorded transaction hash"
                    .into(),
            ));
        };
        // Defensive fallback only: `broadcast_at_unix` is always recorded
        // when a job first reaches `sent` (processing.rs). If it is somehow
        // absent, start the timeout window now rather than treating the
        // job as already-elapsed or panicking.
        let broadcast_at_unix = job_broadcast_at_unix.unwrap_or_else(now_unix);

        // Resolving the provider/wallet profile touches ONLY the plaintext
        // profile registry, never the vault — receipt polling needs no key
        // material and no unlocked compartment beyond what the session
        // already required to call `/api/queue/process` at all.
        let (provider, _wallet) = match self
            .resolve_eth_seed_wallet_profile(&payload.wallet_profile)
        {
            Ok(pair) => pair,
            Err(error) if error.status() == axum::http::StatusCode::NOT_FOUND => {
                return Ok(super::QueueExecution::OperatorActionRequired(format!(
                    "receipt_poll_provider_unavailable: wallet profile no longer resolvable: {}",
                    error.message()
                )));
            }
            Err(error) => return Err(error),
        };

        // A transient provider/network error while POLLING must NEVER be
        // mistaken for a failed broadcast — the tx already broadcast
        // successfully. Swallow the error and stay `sent`; the next drain
        // cycle retries the poll (bounded overall by the wall-clock
        // confirmation timeout, tracked via `broadcast_at_unix`).
        let poll = match self
            .poll_plan_step_receipt(
                &provider,
                payload.chain_id,
                transaction_hash_hex,
                broadcast_at_unix,
            )
            .await
        {
            Ok(poll) => poll,
            Err(_) => {
                return Ok(super::QueueExecution::AwaitingConfirmation {
                    block_number: None,
                    gas_used_hex: None,
                    confirmations: None,
                });
            }
        };

        Ok(match poll {
            ReceiptPoll::Pending => super::QueueExecution::AwaitingConfirmation {
                block_number: None,
                gas_used_hex: None,
                confirmations: None,
            },
            ReceiptPoll::PartiallyConfirmed {
                block_number,
                gas_used_hex,
                confirmations,
            } => super::QueueExecution::AwaitingConfirmation {
                block_number: Some(block_number),
                gas_used_hex: Some(gas_used_hex),
                confirmations: Some(confirmations),
            },
            ReceiptPoll::Confirmed {
                block_number,
                gas_used_hex,
                confirmations,
            } => super::QueueExecution::Confirmed {
                block_number,
                gas_used_hex,
                confirmations,
            },
            ReceiptPoll::Reverted {
                block_number,
                gas_used_hex,
            } => super::QueueExecution::RevertedOnChain {
                reason: format!(
                    "on_chain_revert: transaction {transaction_hash_hex} mined in block \
                     {block_number} with a failure status"
                ),
                block_number,
                gas_used_hex,
            },
            ReceiptPoll::TimedOut => super::QueueExecution::OperatorActionRequired(format!(
                "receipt_timeout: no receipt observed for transaction {transaction_hash_hex} \
                 within {RECEIPT_CONFIRMATION_TIMEOUT_SECS}s of broadcast; the broadcast is NOT \
                 assumed to have failed"
            )),
        })
    }

    fn finality_blocks_for_plan_step_chain(&self, chain_id: u64) -> ServiceResult<u64> {
        let inventory_state = crate::inventory::load_wallet_inventory(&self.state.base_dir)
            .map_err(|error| {
                ServiceError::internal(format!("Failed to load wallet inventory: {error}"))
            })?;
        Ok(finality_blocks_for_chain(
            &inventory_state.chain_profiles,
            chain_id,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_broadcast_error_phrasings() {
        assert_eq!(
            classify_broadcast_error("nonce too low"),
            Some(BroadcastErrorClass::NonceTooLow)
        );
        assert_eq!(
            classify_broadcast_error("Nonce is too low for sender"),
            Some(BroadcastErrorClass::NonceTooLow)
        );
        assert_eq!(
            classify_broadcast_error("replacement transaction underpriced"),
            Some(BroadcastErrorClass::Underpriced)
        );
        assert_eq!(
            classify_broadcast_error("transaction underpriced"),
            Some(BroadcastErrorClass::Underpriced)
        );
        assert_eq!(
            classify_broadcast_error("execution reverted"),
            Some(BroadcastErrorClass::Revert)
        );
        assert_eq!(classify_broadcast_error("connection reset by peer"), None);
    }
}
