use serde::{Deserialize, Serialize};

use super::{EthStealthAnnouncementPayload, QueueJob};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthDeposit {
    pub id: String,
    pub status: String,
    pub asset_kind: String,
    pub wallet_profile: String,
    #[serde(default = "default_legacy_mainnet_chain_id")]
    pub chain_id: u64,
    #[serde(default = "default_legacy_chain_id_assumed")]
    pub chain_id_assumed: bool,
    #[serde(default)]
    pub wallet_compartment_id: usize,
    #[serde(default)]
    pub provider_compartment_id: usize,
    pub wallet: String,
    pub short_name: String,
    pub stealth_meta_address: String,
    pub stealth_address: String,
    pub ephemeral_public_key_hex: String,
    pub view_tag_hex: String,
    /// Shared-secret hash convention this deposit's stealth address was
    /// derived with. New records are always stamped `compressed33` (standard);
    /// records predating the convention switch are stamped `x32` by the
    /// deposits-store migration. Defaults to the standard convention when the
    /// field is absent (e.g. hand-written records); a wrong stamp is corrected
    /// the next time detection re-probes the record.
    #[serde(default)]
    pub stealth_hash_convention: sigillum_core::StealthHashConvention,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub announcement: Option<EthStealthAnnouncementPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_amount_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_amount_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_native_balance_wei_hex: Option<String>,
    pub auto_queue_sweep: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sweep_destination_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_sweep_amount_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_job_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_at_unix: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broadcast_transaction_hash_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterparty_id: Option<String>,
    /// Native gas the operator asked the payer to attach for this deposit's
    /// sweep (set at creation when `request_gas` was used; the payer-facing
    /// amount in the payment instructions). Actual native gas observed on the
    /// stealth address is tracked in `observed_native_balance_wei_hex`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_gas_wei_hex: Option<String>,
    /// Sponsor gas top-up queue job funding this deposit's stealth address,
    /// when one was enqueued (ERC-20 deposits lacking native gas; policy
    /// `allow_gas_topups` on). Its state mirrors into `gas_topup_job_state`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas_topup_job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas_topup_job_state: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthDepositListResponse {
    pub deposits: Vec<EthStealthDeposit>,
    /// Pagination window metadata. Present only when the request supplied
    /// `limit` and/or `offset`; absent on legacy (parameterless) calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<super::PaginationInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthDepositMutationResponse {
    pub status: String,
    pub deposit: EthStealthDeposit,
    /// Non-blocking cautionary warnings propagated from stealth generation
    /// (e.g. ephemeral key reuse). Empty for non-create mutations and when
    /// nothing suspicious was detected.
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthDepositRefreshResponse {
    pub processed: usize,
    pub detected: usize,
    pub queued: usize,
    pub deposits: Vec<EthStealthDeposit>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthAnnouncementScanResponse {
    pub status: String,
    pub wallet_profile: String,
    pub provider_profile: String,
    pub from_block: String,
    pub to_block: String,
    pub scanned: usize,
    pub matched: usize,
    pub created: usize,
    pub existing: usize,
    pub deposits: Vec<EthStealthDeposit>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthDepositEnqueueSweepResponse {
    pub status: String,
    pub deposit: EthStealthDeposit,
    pub job: QueueJob,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linkage_warning: Option<String>,
}

fn default_legacy_mainnet_chain_id() -> u64 {
    1
}

fn default_legacy_chain_id_assumed() -> bool {
    true
}
