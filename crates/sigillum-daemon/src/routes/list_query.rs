//! Raw query-string DTOs and parsing for the list endpoints (plan task 1.5).
//!
//! The URL query is deserialized into all-`Option<String>` DTOs so malformed
//! values (e.g. `limit=abc`) surface as the daemon's 400 `validation_failed`
//! envelope naming the offending parameter instead of axum's default query
//! rejection. Value-domain checks (unknown `state`/`status`/`severity`
//! values) happen in the service layer, which owns those domains.

// These parsing helpers intentionally return the route's final Axum response
// so every malformed query preserves the structured validation envelope.
// Boxing that boundary solely to reduce the `Result` enum size would add
// allocation and unboxing at every list handler without reducing live data.
#![allow(clippy::result_large_err)]

use axum::response::Response;
use serde::Deserialize;
use sigillum_api::response::FieldError;

use crate::service::list_query::{
    ConsolidationPlanListQuery, CreatedUpdatedSort, DiscoveryJobListQuery,
    EthStealthDepositListQuery, ListSortField, PageParams, QueueJobListQuery, RiskFindingListQuery,
    RiskFindingSort, SortOrder, WalletInventoryListQuery, WalletInventorySort,
};

use super::err_validation;

fn invalid_param(param: &str, message: String) -> Response {
    err_validation(
        &format!("invalid parameter '{param}': {message}"),
        vec![FieldError {
            field: param.to_string(),
            message,
        }],
    )
}

fn parse_u32(param: &str, raw: Option<String>) -> Result<Option<u32>, Response> {
    match raw {
        None => Ok(None),
        Some(value) => value
            .parse::<u32>()
            .map(Some)
            .map_err(|_| invalid_param(param, format!("'{value}' is not a non-negative integer"))),
    }
}

fn parse_u64(param: &str, raw: Option<String>) -> Result<Option<u64>, Response> {
    match raw {
        None => Ok(None),
        Some(value) => value
            .parse::<u64>()
            .map(Some)
            .map_err(|_| invalid_param(param, format!("'{value}' is not a non-negative integer"))),
    }
}

fn parse_bool(param: &str, raw: Option<String>) -> Result<Option<bool>, Response> {
    match raw.as_deref() {
        None => Ok(None),
        Some("true") => Ok(Some(true)),
        Some("false") => Ok(Some(false)),
        Some(value) => Err(invalid_param(
            param,
            format!("'{value}' is not 'true' or 'false'"),
        )),
    }
}

fn parse_page(limit: Option<String>, offset: Option<String>) -> Result<PageParams, Response> {
    Ok(PageParams {
        limit: parse_u32("limit", limit)?,
        offset: parse_u32("offset", offset)?,
    })
}

fn parse_order(raw: Option<String>) -> Result<Option<SortOrder>, Response> {
    match raw {
        None => Ok(None),
        Some(value) => SortOrder::parse(&value)
            .map(Some)
            .ok_or_else(|| invalid_param("order", format!("'{value}' is not 'asc' or 'desc'"))),
    }
}

fn parse_sort<S: ListSortField>(raw: Option<String>) -> Result<Option<S>, Response> {
    match raw {
        None => Ok(None),
        Some(value) => S::parse(&value).map(Some).ok_or_else(|| {
            invalid_param(
                "sort",
                format!("'{value}' is not one of {}", S::valid_values().join(", ")),
            )
        }),
    }
}

/// `order` without `sort` is a client bug: reject it rather than silently
/// dropping the requested direction.
fn require_sort_for_order<S>(sort: &Option<S>, order: &Option<SortOrder>) -> Result<(), Response> {
    if sort.is_none() && order.is_some() {
        return Err(invalid_param(
            "order",
            "requires 'sort' to be set".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub(crate) struct QueueJobsRawQuery {
    limit: Option<String>,
    offset: Option<String>,
    state: Option<String>,
    kind: Option<String>,
    chain_id: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}

impl QueueJobsRawQuery {
    pub(crate) fn resolve(self) -> Result<QueueJobListQuery, Response> {
        let page = parse_page(self.limit, self.offset)?;
        let sort = parse_sort::<CreatedUpdatedSort>(self.sort)?;
        let order = parse_order(self.order)?;
        require_sort_for_order(&sort, &order)?;
        Ok(QueueJobListQuery {
            page,
            state: self.state,
            kind: self.kind,
            chain_id: parse_u64("chain_id", self.chain_id)?,
            sort,
            order,
        })
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct WalletInventoryRawQuery {
    limit: Option<String>,
    offset: Option<String>,
    chain_id: Option<String>,
    funded: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}

impl WalletInventoryRawQuery {
    pub(crate) fn resolve(self) -> Result<WalletInventoryListQuery, Response> {
        let page = parse_page(self.limit, self.offset)?;
        let sort = parse_sort::<WalletInventorySort>(self.sort)?;
        let order = parse_order(self.order)?;
        require_sort_for_order(&sort, &order)?;
        Ok(WalletInventoryListQuery {
            page,
            chain_id: parse_u64("chain_id", self.chain_id)?,
            funded: parse_bool("funded", self.funded)?,
            sort,
            order,
        })
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct EthStealthDepositsRawQuery {
    limit: Option<String>,
    offset: Option<String>,
    status: Option<String>,
    chain_id: Option<String>,
    counterparty_id: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}

impl EthStealthDepositsRawQuery {
    pub(crate) fn resolve(self) -> Result<EthStealthDepositListQuery, Response> {
        let page = parse_page(self.limit, self.offset)?;
        let sort = parse_sort::<CreatedUpdatedSort>(self.sort)?;
        let order = parse_order(self.order)?;
        require_sort_for_order(&sort, &order)?;
        Ok(EthStealthDepositListQuery {
            page,
            status: self.status,
            chain_id: parse_u64("chain_id", self.chain_id)?,
            counterparty_id: self.counterparty_id,
            sort,
            order,
        })
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConsolidationPlansRawQuery {
    limit: Option<String>,
    offset: Option<String>,
    status: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}

impl ConsolidationPlansRawQuery {
    pub(crate) fn resolve(self) -> Result<ConsolidationPlanListQuery, Response> {
        let page = parse_page(self.limit, self.offset)?;
        let sort = parse_sort::<CreatedUpdatedSort>(self.sort)?;
        let order = parse_order(self.order)?;
        require_sort_for_order(&sort, &order)?;
        Ok(ConsolidationPlanListQuery {
            page,
            status: self.status,
            sort,
            order,
        })
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RiskFindingsRawQuery {
    limit: Option<String>,
    offset: Option<String>,
    severity: Option<String>,
    kind: Option<String>,
    chain_id: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}

impl RiskFindingsRawQuery {
    pub(crate) fn resolve(self) -> Result<RiskFindingListQuery, Response> {
        let page = parse_page(self.limit, self.offset)?;
        let sort = parse_sort::<RiskFindingSort>(self.sort)?;
        let order = parse_order(self.order)?;
        require_sort_for_order(&sort, &order)?;
        Ok(RiskFindingListQuery {
            page,
            severity: self.severity,
            kind: self.kind,
            chain_id: parse_u64("chain_id", self.chain_id)?,
            sort,
            order,
        })
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct DiscoveryJobsRawQuery {
    limit: Option<String>,
    offset: Option<String>,
    state: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}

impl DiscoveryJobsRawQuery {
    pub(crate) fn resolve(self) -> Result<DiscoveryJobListQuery, Response> {
        let page = parse_page(self.limit, self.offset)?;
        let sort = parse_sort::<CreatedUpdatedSort>(self.sort)?;
        let order = parse_order(self.order)?;
        require_sort_for_order(&sort, &order)?;
        Ok(DiscoveryJobListQuery {
            page,
            state: self.state,
            sort,
            order,
        })
    }
}
