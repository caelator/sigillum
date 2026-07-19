//! Typed filter/sort/page queries for the list endpoints.
//!
//! Plan task 1.5 adds optional query parameters to the six unbounded list
//! endpoints (`GET /api/queue/jobs`, `/api/inventory/wallets`,
//! `/api/deposits/eth-stealth`, `/api/plans/consolidation`,
//! `/api/risk/findings`, `/api/discovery/jobs`). Route handlers parse the raw
//! query strings into these typed structs (see `crate::routes::list_query`);
//! the service methods apply value-domain validation, filtering, stable
//! sorting, and the pagination window.
//!
//! Contract invariants:
//!
//! - A parameterless request is byte-identical to the pre-1.5 response:
//!   store order, full list, no `pagination` key.
//! - Filters validate their value domain; unknown values fail with 400
//!   `validation_failed` naming the parameter.
//! - Sorts are stable: ties (and equal timestamps) keep store order. Without
//!   an explicit `order`, time and severity fields sort `desc` (newest /
//!   most severe first) and `address` sorts `asc`.

use sigillum_api::PaginationInfo;

use super::{ServiceError, ServiceResult};

// ── Sort order ─────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SortOrder {
    Asc,
    Desc,
}

impl SortOrder {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "asc" => Some(Self::Asc),
            "desc" => Some(Self::Desc),
            _ => None,
        }
    }
}

/// A sortable field on a list endpoint, parsed from the `sort` parameter.
pub(crate) trait ListSortField: Sized {
    fn parse(value: &str) -> Option<Self>;
    /// Valid `sort` values, used in validation error messages.
    fn valid_values() -> &'static [&'static str];
    /// Direction applied when `order` is absent.
    fn default_order(&self) -> SortOrder {
        SortOrder::Desc
    }
}

/// `created` / `updated` timestamp sort shared by queue jobs, deposits,
/// consolidation plans, and discovery jobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CreatedUpdatedSort {
    Created,
    Updated,
}

impl ListSortField for CreatedUpdatedSort {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "created" => Some(Self::Created),
            "updated" => Some(Self::Updated),
            _ => None,
        }
    }

    fn valid_values() -> &'static [&'static str] {
        &["created", "updated"]
    }
}

/// Sort fields for `GET /api/inventory/wallets`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WalletInventorySort {
    Address,
    LastScanned,
}

impl ListSortField for WalletInventorySort {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "address" => Some(Self::Address),
            "last_scanned" => Some(Self::LastScanned),
            _ => None,
        }
    }

    fn valid_values() -> &'static [&'static str] {
        &["address", "last_scanned"]
    }

    fn default_order(&self) -> SortOrder {
        match self {
            Self::Address => SortOrder::Asc,
            Self::LastScanned => SortOrder::Desc,
        }
    }
}

/// Sort fields for `GET /api/risk/findings`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RiskFindingSort {
    Severity,
    FoundAt,
}

impl ListSortField for RiskFindingSort {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "severity" => Some(Self::Severity),
            "found_at" => Some(Self::FoundAt),
            _ => None,
        }
    }

    fn valid_values() -> &'static [&'static str] {
        &["severity", "found_at"]
    }
}

/// Severity rank for the `severity` sort: critical first under `desc`.
/// Levels outside the validated set (defensive; findings are daemon-written)
/// rank below `trusted`.
pub(crate) fn severity_rank(risk_level: &str) -> u8 {
    match risk_level {
        "critical" => 5,
        "high" => 4,
        "medium" => 3,
        "low" => 2,
        "trusted" => 1,
        _ => 0,
    }
}

// ── Pagination window ──────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PageParams {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

impl PageParams {
    /// True when the request supplied `limit` and/or `offset` — only then
    /// does the response carry the `pagination` envelope. Offset presence is
    /// retained so an explicit zero remains distinct from an omitted field.
    pub(crate) fn requested(&self) -> bool {
        self.limit.is_some() || self.offset.is_some()
    }
}

/// Apply the pagination window to an already filtered and sorted list.
///
/// Returns the window plus the envelope, or the untouched list and `None`
/// when no pagination parameter was supplied (legacy response shape).
pub(crate) fn paginate<T>(items: Vec<T>, page: PageParams) -> (Vec<T>, Option<PaginationInfo>) {
    if !page.requested() {
        return (items, None);
    }
    let total = items.len() as u64;
    let offset_value = page.offset.unwrap_or(0);
    let offset = u64::from(offset_value);
    let remaining = total.saturating_sub(offset);
    // With only `offset` supplied, the window is the whole remainder and
    // `limit` reports its size so `offset + window < total` stays the
    // `has_more` invariant.
    let limit = page.limit.map(u64::from).unwrap_or(remaining);
    let take = limit.min(remaining) as usize;
    let window: Vec<T> = items.into_iter().skip(offset as usize).take(take).collect();
    let info = PaginationInfo {
        total,
        limit: u32::try_from(limit).unwrap_or(u32::MAX),
        offset: offset_value,
        has_more: offset + (window.len() as u64) < total,
    };
    (window, Some(info))
}

// ── Value-domain validation ────────────────────────────────────────

/// Persisted queue job states accepted by the `state` filter, including the
/// legacy `deferred`/`failed` values that old queue files may still hold.
pub(crate) const QUEUE_JOB_STATES: [&str; 11] = [
    "queued",
    "blocked",
    "retrying",
    "prepared",
    "submitted_unknown",
    "sent",
    "confirmed",
    "failed_terminal",
    "operator_action_required",
    "deferred",
    "failed",
];

/// Queue job payload kinds accepted by the `kind` filter.
pub(crate) const QUEUE_JOB_KINDS: [&str; 9] = [
    "eth_stealth_transfer",
    "eth_stealth_erc20_transfer",
    "eth_stealth_native_sweep",
    "eth_stealth_erc20_sweep",
    "eth_stealth_gas_topup",
    "eth_seed_transfer",
    "eth_seed_native_sweep",
    "eth_seed_erc20_sweep",
    "plan_step_execution",
];

/// Deposit statuses accepted by the `status` filter: the refresh-derived
/// states plus the sweep-lifecycle mirror of the queue states.
pub(crate) const DEPOSIT_STATUSES: [&str; 13] = [
    "pending",
    "underfunded",
    "funded_needs_gas",
    "funded",
    "sweep_queued",
    "sweep_blocked",
    "sweep_retrying",
    "sweep_prepared",
    "sweep_submitted_unknown",
    "sweep_sent",
    "sweep_confirmed",
    "sweep_failed",
    "sweep_operator_action_required",
];

/// Consolidation plan statuses accepted by the `status` filter.
pub(crate) const PLAN_STATUSES: [&str; 4] = ["empty", "blocked", "review_required", "approved"];

/// Risk severities accepted by the `severity` filter (matches `risk_level`).
pub(crate) const RISK_SEVERITIES: [&str; 5] = ["critical", "high", "medium", "low", "trusted"];

/// Discovery job states accepted by the `state` filter; `resume_requested`
/// is a legacy value that pre-real-resume stores may still hold.
pub(crate) const DISCOVERY_JOB_STATES: [&str; 5] = [
    "running",
    "completed",
    "canceled",
    "failed",
    "resume_requested",
];

/// Validate a filter value against its domain, failing closed with 400
/// `validation_failed` that names the offending parameter.
pub(crate) fn validated_value(
    param: &str,
    value: String,
    allowed: &[&str],
) -> ServiceResult<String> {
    if allowed.contains(&value.as_str()) {
        Ok(value)
    } else {
        Err(ServiceError::validation_failed(format!(
            "invalid value '{value}' for parameter '{param}': expected one of {}",
            allowed.join(", ")
        )))
    }
}

// ── Per-endpoint query structs ─────────────────────────────────────

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct QueueJobListQuery {
    pub page: PageParams,
    pub state: Option<String>,
    pub kind: Option<String>,
    pub chain_id: Option<u64>,
    pub sort: Option<CreatedUpdatedSort>,
    pub order: Option<SortOrder>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct WalletInventoryListQuery {
    pub page: PageParams,
    pub chain_id: Option<u64>,
    pub funded: Option<bool>,
    pub sort: Option<WalletInventorySort>,
    pub order: Option<SortOrder>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EthStealthDepositListQuery {
    pub page: PageParams,
    pub status: Option<String>,
    pub chain_id: Option<u64>,
    pub counterparty_id: Option<String>,
    pub sort: Option<CreatedUpdatedSort>,
    pub order: Option<SortOrder>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ConsolidationPlanListQuery {
    pub page: PageParams,
    pub status: Option<String>,
    pub sort: Option<CreatedUpdatedSort>,
    pub order: Option<SortOrder>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RiskFindingListQuery {
    pub page: PageParams,
    pub severity: Option<String>,
    pub kind: Option<String>,
    pub chain_id: Option<u64>,
    pub sort: Option<RiskFindingSort>,
    pub order: Option<SortOrder>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DiscoveryJobListQuery {
    pub page: PageParams,
    pub state: Option<String>,
    pub sort: Option<CreatedUpdatedSort>,
    pub order: Option<SortOrder>,
}

/// Resolve the effective direction: explicit `order`, else the field's
/// documented default (`asc` for `address`, `desc` otherwise).
pub(crate) fn effective_order<S: ListSortField>(
    sort: Option<&S>,
    order: Option<SortOrder>,
) -> SortOrder {
    order
        .or_else(|| sort.map(ListSortField::default_order))
        .unwrap_or(SortOrder::Desc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paginate_returns_legacy_shape_when_not_requested() {
        let (window, info) = paginate(vec![1, 2, 3], PageParams::default());
        assert_eq!(window, vec![1, 2, 3]);
        assert_eq!(info, None);
    }

    #[test]
    fn paginate_boundaries() {
        let items = vec![1, 2, 3, 4, 5];
        let page = |limit, offset| PageParams {
            limit,
            offset: Some(offset),
        };

        // Exact page: has_more flips false when the window reaches the end.
        let (window, info) = paginate(items.clone(), page(Some(5), 0));
        let info = info.unwrap();
        assert_eq!(window, vec![1, 2, 3, 4, 5]);
        assert_eq!(
            (info.total, info.limit, info.offset, info.has_more),
            (5, 5, 0, false)
        );

        // Mid-list window.
        let (window, info) = paginate(items.clone(), page(Some(2), 1));
        let info = info.unwrap();
        assert_eq!(window, vec![2, 3]);
        assert_eq!(
            (info.total, info.limit, info.offset, info.has_more),
            (5, 2, 1, true)
        );

        // Offset beyond the end: empty window, has_more false.
        let (window, info) = paginate(items.clone(), page(Some(2), 9));
        let info = info.unwrap();
        assert!(window.is_empty());
        assert_eq!(
            (info.total, info.limit, info.offset, info.has_more),
            (5, 2, 9, false)
        );

        // Offset-only: window is the remainder, limit reports its size.
        let (window, info) = paginate(items.clone(), page(None, 3));
        let info = info.unwrap();
        assert_eq!(window, vec![4, 5]);
        assert_eq!(
            (info.total, info.limit, info.offset, info.has_more),
            (5, 2, 3, false)
        );

        // Explicit offset=0 is still a pagination request. The full list is
        // returned, but with the additive envelope promised by the API.
        let (window, info) = paginate(items.clone(), page(None, 0));
        let info = info.unwrap();
        assert_eq!(window, items);
        assert_eq!(
            (info.total, info.limit, info.offset, info.has_more),
            (5, 5, 0, false)
        );

        // limit=0 is a valid empty page; more items exist beyond it.
        let (window, info) = paginate(vec![1, 2, 3, 4, 5], page(Some(0), 0));
        let info = info.unwrap();
        assert!(window.is_empty());
        assert_eq!(
            (info.total, info.limit, info.offset, info.has_more),
            (5, 0, 0, true)
        );

        // Empty list at offset 0.
        let (window, info) = paginate(Vec::<u8>::new(), page(Some(10), 0));
        let info = info.unwrap();
        assert!(window.is_empty());
        assert_eq!(
            (info.total, info.limit, info.offset, info.has_more),
            (0, 10, 0, false)
        );
    }

    #[test]
    fn validated_value_names_the_parameter() {
        let ok = validated_value("state", "queued".into(), &QUEUE_JOB_STATES).unwrap();
        assert_eq!(ok, "queued");
        let error = validated_value("state", "bogus".into(), &QUEUE_JOB_STATES).unwrap_err();
        assert_eq!(error.code(), sigillum_api::error_codes::VALIDATION_FAILED);
        assert!(error.message().contains("'state'"));
        assert!(error.message().contains("bogus"));
    }

    #[test]
    fn severity_rank_orders_critical_first() {
        assert!(severity_rank("critical") > severity_rank("high"));
        assert!(severity_rank("high") > severity_rank("medium"));
        assert!(severity_rank("medium") > severity_rank("low"));
        assert!(severity_rank("low") > severity_rank("trusted"));
        assert!(severity_rank("trusted") > severity_rank("unknown-value"));
    }
}
