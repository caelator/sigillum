//! Queue response contracts.

use serde::{Deserialize, Serialize};

mod plan_step;
pub use plan_step::PlanStepExecutionPayload;

mod receipt;
pub use receipt::QueueJobReceipt;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueueJobPayload {
    EthStealthTransfer {
        wallet_profile: String,
        stealth_address: String,
        ephemeral_public_key_hex: String,
        value_wei_hex: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        destination_address: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        nonce: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        gas_limit: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        view_tag_hex: Option<String>,
        /// Shared-secret hash convention copied from the deposit record at
        /// enqueue time; `None` (jobs enqueued before the convention switch)
        /// makes execution probe both conventions with address verification.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stealth_hash_convention: Option<sigillum_core::StealthHashConvention>,
    },
    EthStealthErc20Transfer {
        wallet_profile: String,
        stealth_address: String,
        ephemeral_public_key_hex: String,
        token_address: String,
        recipient_address: String,
        amount_hex: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        nonce: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        gas_limit: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        view_tag_hex: Option<String>,
        /// Shared-secret hash convention copied from the deposit record at
        /// enqueue time; `None` (jobs enqueued before the convention switch)
        /// makes execution probe both conventions with address verification.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stealth_hash_convention: Option<sigillum_core::StealthHashConvention>,
    },
    EthStealthNativeSweep {
        wallet_profile: String,
        stealth_address: String,
        ephemeral_public_key_hex: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        destination_address: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        min_value_wei_hex: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        gas_limit: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        view_tag_hex: Option<String>,
        /// Shared-secret hash convention copied from the deposit record at
        /// enqueue time; `None` (jobs enqueued before the convention switch)
        /// makes execution probe both conventions with address verification.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stealth_hash_convention: Option<sigillum_core::StealthHashConvention>,
    },
    EthStealthErc20Sweep {
        wallet_profile: String,
        stealth_address: String,
        ephemeral_public_key_hex: String,
        token_address: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        recipient_address: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        min_amount_hex: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        gas_limit: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        view_tag_hex: Option<String>,
        /// Shared-secret hash convention copied from the deposit record at
        /// enqueue time; `None` (jobs enqueued before the convention switch)
        /// makes execution probe both conventions with address verification.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stealth_hash_convention: Option<sigillum_core::StealthHashConvention>,
        /// Queue job ids that must reach `sent` (or `confirmed`) before this
        /// sweep may execute — the sponsor gas top-up funding this deposit's
        /// stealth address, when one was enqueued. Mirrors the W6.4
        /// `PlanStepExecution` prerequisite semantics; empty for jobs
        /// enqueued before sponsor top-ups existed.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        prerequisite_job_ids: Vec<String>,
    },
    /// Sponsor gas top-up funding a gas-starved stealth deposit address:
    /// a native transfer from the stealth wallet's gas sponsor (derived from
    /// the compartment master key; see `docs/architecture.md`) to the
    /// deposit's stealth address. Enqueued only by the deposit sweep flow
    /// (never via a public enqueue endpoint) when policy `allow_gas_topups`
    /// is on, at 1.5x the dependent sweep's estimated gas, with the same
    /// cross-party linkage accounting as seed-plan sponsor funding.
    EthStealthGasTopup {
        wallet_profile: String,
        /// Sponsor source address (derived per stealth wallet); the operator
        /// funds it out-of-band. Re-verified against the derived key at
        /// execution time.
        sponsor_address: String,
        /// The gas-starved stealth deposit address receiving the top-up.
        destination_address: String,
        value_wei_hex: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        gas_limit: Option<u64>,
    },
    EthSeedTransfer {
        wallet_profile: String,
        address: String,
        derivation_path: String,
        value_wei_hex: String,
        destination_address: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        nonce: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        gas_limit: Option<u64>,
    },
    EthSeedNativeSweep {
        wallet_profile: String,
        address: String,
        derivation_path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        destination_address: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        min_value_wei_hex: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        gas_limit: Option<u64>,
    },
    EthSeedErc20Sweep {
        wallet_profile: String,
        address: String,
        derivation_path: String,
        token_address: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        recipient_address: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        min_amount_hex: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        gas_limit: Option<u64>,
    },
    /// W7.2 consolidation plan-step job. Hard-blocked at drain time until W7.3.
    PlanStepExecution(Box<PlanStepExecutionPayload>),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueJob {
    pub id: String,
    pub state: String,
    pub attempts: u32,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_attempt_after_unix: Option<u64>,
    #[serde(flatten)]
    pub payload: QueueJobPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_hash_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broadcast_transaction_hash_hex: Option<String>,
    /// W7.4 receipt confirmation fields, flattened (see `QueueJobReceipt`).
    #[serde(flatten)]
    pub receipt: QueueJobReceipt,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueJobListResponse {
    pub jobs: Vec<QueueJob>,
    /// Pagination window metadata. Present only when the request supplied
    /// `limit` and/or `offset`; absent on legacy (parameterless) calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<super::PaginationInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueEnqueueResponse {
    pub status: String,
    pub job: QueueJob,
}
