use serde::{Deserialize, Serialize};

use super::{EthStealthAnnouncementPayload, QueueJob, RiskFinding};

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

/// Persisted per-(wallet profile, provider profile) announcement-scan cursor
/// (plan task 2.6).
///
/// Position version 1 uses `last_scanned_log_index = Some(index)` for a
/// partially consumed block; the next implicit scan resumes at that block and
/// skips log positions up to and including `index`. `None` records a completely
/// covered block, so the next scan starts at `last_scanned_block + 1`.
///
/// Cursors written before intra-block positions existed have neither additive
/// field and deserialize with `position_version = 0`. Their entire historical
/// range is ambiguous: an old limit-capped scan may have skipped a same-block
/// tail and then advanced through later blocks. The daemon therefore replays
/// history once from `earliest` before upgrading the cursor to v1. When that
/// replay spans multiple capped pages, `legacy_replay_through_block` retains
/// the old block boundary until the exact v1 cursor has covered it.
///
/// Stored in the deposits store; mirrored here so responses and clients can
/// share the shape.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthAnnouncementScanCursor {
    pub wallet_profile: String,
    pub provider_profile: String,
    #[serde(default = "default_legacy_mainnet_chain_id")]
    pub chain_id: u64,
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub position_version: u8,
    pub last_scanned_block: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_scanned_log_index: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_replay_through_block: Option<u64>,
    pub updated_at_unix: u64,
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
    /// Structured privacy findings from the sponsor-linkage analysis (plan
    /// task 3.5): a `common_gas_funder` entry when this deposit's gas sponsor
    /// already funds deposits attributed to different payer identities.
    /// Advisory only — blocking stays governed by `block_cross_party_linkage`
    /// (the `linkage_warning`/403 `policy_violation` path is unchanged).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risk_findings: Vec<RiskFinding>,
}

fn default_legacy_mainnet_chain_id() -> u64 {
    1
}

fn default_legacy_chain_id_assumed() -> bool {
    true
}

fn is_zero_u8(value: &u8) -> bool {
    *value == 0
}

#[cfg(test)]
mod tests {
    use super::EthStealthAnnouncementScanCursor;

    #[test]
    fn legacy_announcement_cursor_defaults_to_an_ambiguous_v0_position() {
        let legacy = serde_json::json!({
            "wallet_profile": "payments",
            "provider_profile": "mainnet",
            "chain_id": 1,
            "last_scanned_block": 32,
            "updated_at_unix": 7
        });

        let cursor: EthStealthAnnouncementScanCursor =
            serde_json::from_value(legacy).expect("legacy cursor should deserialize");
        assert_eq!(cursor.position_version, 0);
        assert_eq!(cursor.last_scanned_log_index, None);

        let encoded = serde_json::to_value(cursor).expect("cursor should serialize");
        assert!(encoded.get("position_version").is_none());
        assert!(encoded.get("last_scanned_log_index").is_none());
    }
}
