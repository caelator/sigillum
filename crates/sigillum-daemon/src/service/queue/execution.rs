//! Internal queue-execution outcomes applied by `outcomes.rs`.

#[allow(clippy::large_enum_variant)]
pub(in crate::service) enum QueueExecution {
    /// Exact signed bytes are ready for the durable pre-broadcast barrier.
    Prepared {
        signed_raw_transaction_hex: String,
        transaction_hash_hex: String,
    },
    /// Prepared authority is retained, but submission is held until a
    /// dependency reaches receipt-confirmed success.
    PreparedHeld(String),
    /// The provider accepted the exact prepared bytes.
    Broadcasted {
        broadcast_transaction_hash_hex: String,
    },
    /// Submission may have reached the provider. Recovery must query the
    /// stored hash or resubmit the exact bytes, never sign again.
    SubmittedUnknown(String),
    /// Submission admission was denied locally while prior submission
    /// uncertainty still exists; retain the exact replay bytes without retrying.
    SubmittedUnknownHeld(String),
    Blocked(String),
    /// Terminal-until-human and never auto-retried.
    OperatorActionRequired(String),
    /// An on-chain revert discovered through receipt polling.
    RevertedOnChain {
        reason: String,
        block_number: u64,
        gas_used_hex: String,
    },
    /// Receipt success at the configured finality depth.
    Confirmed {
        block_number: u64,
        gas_used_hex: String,
        confirmations: u64,
    },
    /// A receipt is absent or has not reached finality; state remains `sent`.
    AwaitingConfirmation {
        block_number: Option<u64>,
        gas_used_hex: Option<String>,
        confirmations: Option<u64>,
    },
}

impl QueueExecution {
    pub(super) fn prepared_from_send(sent: sigillum_api::EthStealthSendResponse) -> Self {
        Self::Prepared {
            signed_raw_transaction_hex: sent.raw_transaction_hex,
            transaction_hash_hex: sent.transaction_hash_hex,
        }
    }

    pub(super) fn prepared_from_signed(signed: sigillum_core::EthereumSignedTransaction) -> Self {
        Self::Prepared {
            signed_raw_transaction_hex: signed.raw_transaction_hex,
            transaction_hash_hex: signed.transaction_hash_hex,
        }
    }
}
