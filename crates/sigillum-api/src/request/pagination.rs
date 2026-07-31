//! Query-string parameters for list endpoints: offset pagination, filters,
//! and sorting.
//!
//! These structs describe the optional query parameters accepted by the
//! daemon's list endpoints (`GET /api/queue/jobs`, `/api/inventory/wallets`,
//! `/api/deposits/eth-stealth`, `/api/plans/consolidation`,
//! `/api/risk/findings`, `/api/discovery/jobs`). They travel as URL query
//! strings, not JSON bodies, so every field is optional and a fully-default
//! struct serializes to an empty query string — which is exactly the legacy
//! request shape (full list, store order, no `pagination` envelope).
//!
//! Validated value domains are documented on each field; the daemon rejects
//! unknown values with a 400 `validation_failed` error naming the parameter.

use serde::{Deserialize, Serialize};

/// Offset pagination window shared by all paginated list endpoints.
///
/// Supplying either field makes the response carry a
/// [`crate::response::PaginationInfo`] envelope. Supplying neither returns
/// the full (still filtered/sorted) list with no envelope.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaginationQuery {
    /// Maximum number of items returned (after filtering and sorting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Number of items skipped before `limit` applies. Defaults to 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

/// Filter/sort/page options for `GET /api/queue/jobs`.
///
/// `order` accepts `asc` or `desc`; when `sort` is given without `order`,
/// the daemon defaults to `desc` for time and severity fields and to `asc`
/// for the `address` field.
///
/// - `state`: `queued`, `blocked`, `retrying`, `prepared`,
///   `submitted_unknown`, `sent`, `confirmed`, `failed_terminal`,
///   `operator_action_required`, or a legacy state (`deferred`, `failed`).
/// - `kind`: job payload kind (`eth_stealth_transfer`,
///   `eth_stealth_erc20_transfer`, `eth_stealth_native_sweep`,
///   `eth_stealth_erc20_sweep`, `eth_seed_transfer`, `eth_seed_native_sweep`,
///   `eth_seed_erc20_sweep`, `plan_step_execution`).
/// - `chain_id`: matches only payloads that carry a chain id (currently
///   `plan_step_execution` jobs).
/// - `sort`: `created` (created_at_unix) or `updated` (updated_at_unix).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueJobListOptions {
    #[serde(flatten)]
    pub page: PaginationQuery,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
}

/// Filter/sort/page options for `GET /api/inventory/wallets`.
///
/// Applies to the response's `addresses` list only; `jobs`, `holdings`, and
/// `nft_metadata_cache` are always returned in full.
///
/// - `funded`: true keeps addresses with a non-zero native balance
///   (`activity_state == "funded"`), false keeps the rest.
/// - `sort`: `address` (lexicographic) or `last_scanned`
///   (last_checked_at_unix).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletInventoryListOptions {
    #[serde(flatten)]
    pub page: PaginationQuery,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funded: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
}

/// Filter/sort/page options for `GET /api/deposits/eth-stealth`.
///
/// - `status`: `pending`, `underfunded`, `funded_needs_gas`, `funded`, or a
///   sweep status (`sweep_queued`, `sweep_blocked`, `sweep_retrying`,
///   `sweep_prepared`, `sweep_submitted_unknown`, `sweep_sent`,
///   `sweep_confirmed`, `sweep_failed`, `sweep_operator_action_required`).
/// - `counterparty_id`: exact match, free-form (not value-validated).
/// - `sort`: `created` (created_at_unix) or `updated` (updated_at_unix).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthDepositListOptions {
    #[serde(flatten)]
    pub page: PaginationQuery,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterparty_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
}

/// Filter/sort/page options for `GET /api/plans/consolidation`.
///
/// - `status`: `empty`, `blocked`, `review_required`, or `approved`.
/// - `sort`: `created` (created_at_unix) or `updated` (updated_at_unix).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanListOptions {
    #[serde(flatten)]
    pub page: PaginationQuery,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
}

/// Filter/sort/page options for `GET /api/risk/findings`.
///
/// - `severity`: exact match on the finding's `risk_level` (`critical`,
///   `high`, `medium`, `low`, `trusted`).
/// - `kind`: exact match on the finding's `category`, free-form (not
///   value-validated).
/// - `sort`: `severity` (critical > high > medium > low > trusted) or
///   `found_at` (first_seen_at_unix).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskFindingListOptions {
    #[serde(flatten)]
    pub page: PaginationQuery,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
}

/// Filter/sort/page options for `GET /api/discovery/jobs`.
///
/// - `state`: exact match on the job's `status` (`running`, `completed`,
///   `canceled`, `failed`).
/// - `sort`: `created` (started_at_unix) or `updated` (completed_at_unix,
///   falling back to started_at_unix for still-running jobs).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryJobListOptions {
    #[serde(flatten)]
    pub page: PaginationQuery,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
}
