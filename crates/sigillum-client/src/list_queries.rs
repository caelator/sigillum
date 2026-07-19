//! List endpoint filter/sort/page options (plan task 1.5).
//!
//! Each `*_with_options` variant mirrors its legacy no-arg counterpart but
//! accepts a typed options struct from `sigillum_api::request` and returns
//! the full response envelope — which carries `pagination` when the request
//! supplied `limit` and/or `offset`. The legacy methods are unchanged: a
//! parameterless request (a default options struct serializes to no query
//! string) returns the full list in store order with no `pagination` key.

use reqwest::Method;
use sigillum_api::request::{
    ConsolidationPlanListOptions, DiscoveryJobListOptions, EthStealthDepositListOptions,
    QueueJobListOptions, RiskFindingListOptions, WalletInventoryListOptions,
};
use sigillum_api::response::{
    ConsolidationPlanListResponse, DiscoveryJobListResponse, EthStealthDepositListResponse,
    QueueJobListResponse, RiskFindingListResponse, WalletInventoryListResponse,
};

use crate::{ClientError, SigillumClient};

/// Serialize a list-options struct into a URL query string. Every field is
/// optional and skipped when absent, so a default struct produces no query
/// string — the exact legacy request shape.
fn options_query<T: serde::Serialize>(path: &str, options: &T) -> String {
    let mut rendered = path.to_string();
    let Ok(serde_json::Value::Object(map)) = serde_json::to_value(options) else {
        return rendered;
    };
    let mut first = true;
    for (key, value) in map {
        let value = match value {
            serde_json::Value::String(text) => text,
            serde_json::Value::Number(number) => number.to_string(),
            serde_json::Value::Bool(flag) => flag.to_string(),
            _ => continue,
        };
        rendered.push(if first { '?' } else { '&' });
        first = false;
        rendered.push_str(&key);
        rendered.push('=');
        rendered.push_str(&urlencoding::encode(&value));
    }
    rendered
}

impl SigillumClient {
    /// List queue jobs with state/kind/chain filters, created/updated sort,
    /// and an optional limit/offset window.
    pub async fn list_queue_jobs_with_options(
        &self,
        options: QueueJobListOptions,
    ) -> Result<QueueJobListResponse, ClientError> {
        let builder = self.request(Method::GET, &options_query("/api/queue/jobs", &options));
        self.send(builder).await
    }

    /// List wallet inventory with chain/funded filters, address/last-scanned
    /// sort, and an optional limit/offset window over the `addresses` list.
    pub async fn list_wallet_inventory_with_options(
        &self,
        options: WalletInventoryListOptions,
    ) -> Result<WalletInventoryListResponse, ClientError> {
        let builder = self.request(
            Method::GET,
            &options_query("/api/inventory/wallets", &options),
        );
        self.send(builder).await
    }

    /// List stealth deposits with status/chain/counterparty filters,
    /// created/updated sort, and an optional limit/offset window.
    pub async fn list_eth_stealth_deposits_with_options(
        &self,
        options: EthStealthDepositListOptions,
    ) -> Result<EthStealthDepositListResponse, ClientError> {
        let builder = self.request(
            Method::GET,
            &options_query("/api/deposits/eth-stealth", &options),
        );
        self.send(builder).await
    }

    /// List consolidation plans with a status filter, created/updated sort,
    /// and an optional limit/offset window.
    pub async fn list_consolidation_plans_with_options(
        &self,
        options: ConsolidationPlanListOptions,
    ) -> Result<ConsolidationPlanListResponse, ClientError> {
        let builder = self.request(
            Method::GET,
            &options_query("/api/plans/consolidation", &options),
        );
        self.send(builder).await
    }

    /// List risk findings with severity/kind/chain filters, severity/found-at
    /// sort, and an optional limit/offset window.
    pub async fn list_risk_findings_with_options(
        &self,
        options: RiskFindingListOptions,
    ) -> Result<RiskFindingListResponse, ClientError> {
        let builder = self.request(Method::GET, &options_query("/api/risk/findings", &options));
        self.send(builder).await
    }

    /// List discovery jobs with a state filter, created/updated sort, and an
    /// optional limit/offset window.
    pub async fn list_discovery_jobs_with_options(
        &self,
        options: DiscoveryJobListOptions,
    ) -> Result<DiscoveryJobListResponse, ClientError> {
        let builder = self.request(Method::GET, &options_query("/api/discovery/jobs", &options));
        self.send(builder).await
    }
}

#[cfg(test)]
mod tests {
    use sigillum_api::request::PaginationQuery;

    use super::*;

    #[test]
    fn default_options_produce_no_query_string() {
        assert_eq!(
            options_query("/api/queue/jobs", &QueueJobListOptions::default()),
            "/api/queue/jobs"
        );
        assert_eq!(
            options_query("/api/risk/findings", &RiskFindingListOptions::default()),
            "/api/risk/findings"
        );
    }

    #[test]
    fn explicit_zero_offset_is_serialized() {
        let options = QueueJobListOptions {
            page: PaginationQuery {
                limit: None,
                offset: Some(0),
            },
            ..Default::default()
        };
        assert_eq!(
            options_query("/api/queue/jobs", &options),
            "/api/queue/jobs?offset=0"
        );
    }

    #[test]
    fn options_serialize_to_query_pairs() {
        let options = QueueJobListOptions {
            page: PaginationQuery {
                limit: Some(25),
                offset: Some(50),
            },
            state: Some("operator_action_required".into()),
            kind: None,
            chain_id: Some(1),
            sort: Some("created".into()),
            order: Some("asc".into()),
        };
        let path = options_query("/api/queue/jobs", &options);
        for pair in [
            "limit=25",
            "offset=50",
            "state=operator_action_required",
            "chain_id=1",
            "sort=created",
            "order=asc",
        ] {
            assert!(path.contains(pair), "missing {pair} in {path}");
        }
        assert!(!path.contains("kind="), "absent filter leaked: {path}");
        assert!(path.starts_with("/api/queue/jobs?"), "{path}");
    }

    #[test]
    fn bool_and_string_values_are_encoded() {
        let options = WalletInventoryListOptions {
            funded: Some(false),
            ..Default::default()
        };
        assert_eq!(
            options_query("/api/inventory/wallets", &options),
            "/api/inventory/wallets?funded=false"
        );
    }
}
