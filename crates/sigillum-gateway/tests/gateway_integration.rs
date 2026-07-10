mod support;

use reqwest::Method;
use serde_json::{Value, json};
use std::collections::HashSet;
use support::{GatewayHarness, StubDaemon, StubDaemonConfig};

fn project_request(wallet_profile: &str) -> Value {
    json!({
        "name": "merchant-a",
        "wallet_profile": wallet_profile,
        "webhook_url": null
    })
}

fn payment_request(chain_id: u64, idempotency_key: Option<&str>) -> Value {
    json!({
        "amount_wei": "0x2386F26FC10000",
        "chain_id": chain_id,
        "token_address": null,
        "metadata": {
            "order_id": "order-123",
            "source": "gateway-test"
        },
        "idempotency_key": idempotency_key,
    })
}

async fn create_project(
    gateway: &GatewayHarness,
    wallet_profile: &str,
    token: Option<&str>,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let response = gateway
        .request_json(
            Method::POST,
            "/api/v1/projects",
            project_request(wallet_profile),
            token,
        )
        .await;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        return Err(format!("project creation failed: {status} {body}").into());
    }
    Ok(response.json().await?)
}

async fn create_payment(
    gateway: &GatewayHarness,
    token: &str,
    chain_id: u64,
    idempotency_key: Option<&str>,
) -> Result<(reqwest::StatusCode, Value), Box<dyn std::error::Error + Send + Sync>> {
    let response = gateway
        .request_json(
            Method::POST,
            "/api/v1/payments",
            payment_request(chain_id, idempotency_key),
            Some(token),
        )
        .await;
    let status = response.status();
    let body = response.json().await?;
    Ok((status, body))
}

fn default_stub_config() -> StubDaemonConfig {
    StubDaemonConfig::default()
}

fn payment_failure_stub_config() -> StubDaemonConfig {
    StubDaemonConfig {
        reject_export_wallet_profiles: HashSet::from([String::from("payments-mainnet")]),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_creation_requires_admin_token()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let stub = StubDaemon::spawn(default_stub_config()).await?;
    let mut gateway = GatewayHarness::spawn(&stub.base_url(), "admin-secret", 0).await?;
    gateway.wait_until_ready().await?;

    let missing = gateway
        .request_json(
            Method::POST,
            "/api/v1/projects",
            project_request("payments-mainnet"),
            None,
        )
        .await;
    assert_eq!(missing.status(), reqwest::StatusCode::UNAUTHORIZED);

    let wrong = gateway
        .request_json(
            Method::POST,
            "/api/v1/projects",
            project_request("payments-mainnet"),
            Some("wrong-secret"),
        )
        .await;
    assert_eq!(wrong.status(), reqwest::StatusCode::UNAUTHORIZED);

    let project = create_project(&gateway, "payments-mainnet", Some("admin-secret")).await?;
    assert_eq!(project["name"], "merchant-a");
    assert!(project["api_key"].as_str().is_some());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_scope_update_blocks_missing_payment_scope()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let stub = StubDaemon::spawn(default_stub_config()).await?;
    let mut gateway = GatewayHarness::spawn(&stub.base_url(), "admin-secret", 0).await?;
    gateway.wait_until_ready().await?;

    let project = create_project(&gateway, "payments-mainnet", Some("admin-secret")).await?;
    let project_id = project["id"].as_str().unwrap();
    let api_key = project["api_key"].as_str().unwrap();

    let update = gateway
        .request_json(
            Method::PATCH,
            &format!("/api/v1/projects/{project_id}/scopes"),
            json!({ "scopes": ["payments:read"] }),
            Some("admin-secret"),
        )
        .await;
    assert_eq!(update.status(), reqwest::StatusCode::OK);
    let update_body: Value = update.json().await?;
    assert_eq!(update_body["scopes"], json!(["payments:read"]));

    let (status, body) = create_payment(&gateway, api_key, 1, Some("idem-scope")).await?;
    assert_eq!(status, reqwest::StatusCode::FORBIDDEN);
    assert_eq!(body["error"], json!("missing_scope"));
    assert_eq!(body["required"], json!("payments:create"));
    assert_eq!(stub.counts().native_deposit_calls, 0);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn payment_preview_is_disabled_by_default_without_finality_proof()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let stub = StubDaemon::spawn(default_stub_config()).await?;
    let mut gateway =
        GatewayHarness::spawn_payments_disabled(&stub.base_url(), "admin-secret", 0).await?;
    gateway.wait_until_ready().await?;

    let project = create_project(&gateway, "payments-mainnet", Some("admin-secret")).await?;
    let project_id = project["id"].as_str().unwrap();
    let api_key = project["api_key"].as_str().unwrap();
    let retry_delivery_id = gateway.install_pending_webhook_retry(project_id).await?;

    let payment_read = gateway
        .get("/api/v1/payments/disabled-retry-payment", Some(api_key))
        .await;
    assert_eq!(payment_read.status(), reqwest::StatusCode::OK);
    let payment_read_body: Value = payment_read.json().await?;
    assert!(payment_read_body.get("confirmed_at").is_none());
    assert!(
        payment_read_body
            .get("latest_balance_observation_at")
            .is_some()
    );

    let (status, body) = create_payment(&gateway, api_key, 1, Some("disabled-preview")).await?;
    assert_eq!(status, reqwest::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "feature_disabled");
    assert!(
        body["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("GATEWAY_ENABLE_EXPERIMENTAL_PAYMENTS=1")
    );
    tokio::time::sleep(std::time::Duration::from_millis(1_250)).await;
    let counts = stub.counts();
    assert_eq!(counts.native_deposit_calls, 0);
    assert_eq!(counts.deposit_refresh_calls, 0);
    assert_eq!(counts.deposit_list_calls, 0);
    assert_eq!(counts.queue_process_calls, 0);
    assert_eq!(gateway.sqlite_row_count(project_id).await?, 1);
    assert_eq!(
        gateway
            .sqlite_webhook_retry_state(retry_delivery_id)
            .await?,
        Some((1, Some("2000-01-01 00:00:00".into())))
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_creation_rejects_unknown_wallet_profile()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let stub = StubDaemon::spawn(default_stub_config()).await?;
    let mut gateway = GatewayHarness::spawn(&stub.base_url(), "admin-secret", 0).await?;
    gateway.wait_until_ready().await?;

    let response = gateway
        .request_json(
            Method::POST,
            "/api/v1/projects",
            project_request("missing-wallet"),
            Some("admin-secret"),
        )
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await?;
    assert_eq!(
        body["error"],
        "wallet_profile 'missing-wallet' was not found in the daemon"
    );
    assert_eq!(stub.counts().export_calls, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn payment_creation_rejects_chain_mismatch_before_daemon_side_effects()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let stub = StubDaemon::spawn(default_stub_config()).await?;
    let mut gateway = GatewayHarness::spawn(&stub.base_url(), "admin-secret", 0).await?;
    gateway.wait_until_ready().await?;

    let project = create_project(&gateway, "payments-mainnet", Some("admin-secret")).await?;
    let project_id = project["id"]
        .as_str()
        .expect("project id should be present")
        .to_string();
    let api_key = project["api_key"]
        .as_str()
        .expect("api key should be present")
        .to_string();

    let (status, body) = create_payment(&gateway, &api_key, 5, None).await?;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(
        body["error"],
        "chain_id 5 does not match wallet_profile 'payments-mainnet' chain 1"
    );
    assert_eq!(stub.counts().export_calls, 0);
    assert_eq!(gateway.sqlite_row_count(&project_id).await?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn payment_creation_reports_daemon_deposit_failure()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let stub = StubDaemon::spawn(payment_failure_stub_config()).await?;
    let mut gateway = GatewayHarness::spawn(&stub.base_url(), "admin-secret", 0).await?;
    gateway.wait_until_ready().await?;

    let project = create_project(&gateway, "payments-mainnet", Some("admin-secret")).await?;
    let api_key = project["api_key"]
        .as_str()
        .expect("api key should be present")
        .to_string();

    let (status, body) = create_payment(&gateway, &api_key, 1, None).await?;
    assert_eq!(status, reqwest::StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"], "Daemon unavailable");
    let counts = stub.counts();
    assert_eq!(counts.export_calls, 0);
    assert_eq!(counts.generate_calls, 0);
    assert_eq!(counts.native_deposit_calls, 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn payment_creation_is_idempotent_for_duplicate_request()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let stub = StubDaemon::spawn(default_stub_config()).await?;
    let mut gateway = GatewayHarness::spawn(&stub.base_url(), "admin-secret", 0).await?;
    gateway.wait_until_ready().await?;

    let project = create_project(&gateway, "payments-mainnet", Some("admin-secret")).await?;
    let project_id = project["id"]
        .as_str()
        .expect("project id should be present")
        .to_string();
    let api_key = project["api_key"]
        .as_str()
        .expect("api key should be present")
        .to_string();

    let (status1, body1) = create_payment(&gateway, &api_key, 1, Some("idem-123")).await?;
    assert_eq!(status1, reqwest::StatusCode::OK);
    assert_eq!(body1["chain_id"], 1);
    assert_eq!(body1["stealth_address"], "st:address:stub");
    assert_eq!(body1["ephemeral_public_key_hex"], "33".repeat(32));
    assert_eq!(body1["deposit_id"], "native-deposit-1");
    assert!(body1.get("confirmed_at").is_none());
    assert!(body1["latest_balance_observation_at"].is_null());

    let (status2, body2) = create_payment(&gateway, &api_key, 1, Some("idem-123")).await?;
    assert_eq!(status2, reqwest::StatusCode::OK);

    assert_eq!(body1["payment_id"], body2["payment_id"]);
    assert_eq!(body2["idempotent"], true);
    assert!(body2.get("confirmed_at").is_none());
    assert!(body2["latest_balance_observation_at"].is_null());

    let counts = stub.counts();
    assert_eq!(counts.export_calls, 0);
    assert_eq!(counts.generate_calls, 0);
    assert_eq!(counts.native_deposit_calls, 1);
    assert_eq!(gateway.sqlite_row_count(&project_id).await?, 1);

    let row = gateway
        .sqlite_payment_by_idempotency(&project_id, "idem-123")
        .await?
        .expect("idempotency row should exist");
    assert_eq!(row.1, 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn payment_creation_rolls_back_deposit_when_gateway_insert_fails()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let stub = StubDaemon::spawn(default_stub_config()).await?;
    let mut gateway = GatewayHarness::spawn(&stub.base_url(), "admin-secret", 0).await?;
    gateway.wait_until_ready().await?;

    let project = create_project(&gateway, "payments-mainnet", Some("admin-secret")).await?;
    let project_id = project["id"]
        .as_str()
        .expect("project id should be present")
        .to_string();
    let api_key = project["api_key"]
        .as_str()
        .expect("api key should be present")
        .to_string();

    gateway.install_payment_insert_failure_trigger().await?;

    let (status, body) = create_payment(&gateway, &api_key, 1, None).await?;
    assert_eq!(status, reqwest::StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["error"], "Internal server error");
    assert_eq!(gateway.sqlite_row_count(&project_id).await?, 0);

    let counts = stub.counts();
    assert_eq!(counts.native_deposit_calls, 1);
    assert_eq!(counts.delete_deposit_calls, 1);
    assert_eq!(
        stub.deleted_deposit_ids(),
        vec![String::from("native-deposit-1")]
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idempotency_key_conflicts_on_different_parameters()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let stub = StubDaemon::spawn(default_stub_config()).await?;
    let mut gateway = GatewayHarness::spawn(&stub.base_url(), "admin-secret", 0).await?;
    gateway.wait_until_ready().await?;

    let project = create_project(&gateway, "payments-mainnet", Some("admin-secret")).await?;
    let api_key = project["api_key"]
        .as_str()
        .expect("api key should be present")
        .to_string();

    let (status1, _) = create_payment(&gateway, &api_key, 1, Some("idem-variant")).await?;
    assert_eq!(status1, reqwest::StatusCode::OK);

    let (status2, body2) = create_payment(&gateway, &api_key, 5, Some("idem-variant")).await?;
    assert_eq!(status2, reqwest::StatusCode::CONFLICT);
    assert_eq!(
        body2["error"],
        "idempotency_key was reused with different payment parameters"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gateway_rate_limits_repeated_requests()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let stub = StubDaemon::spawn(default_stub_config()).await?;
    let mut gateway = GatewayHarness::spawn(&stub.base_url(), "admin-secret", 1).await?;
    gateway.wait_until_ready().await?;

    let mut statuses = Vec::new();
    for _ in 0..3 {
        statuses.push(gateway.get("/api/v1/health", None).await.status());
    }

    assert!(
        statuses.contains(&reqwest::StatusCode::TOO_MANY_REQUESTS),
        "expected the gateway to rate limit repeated requests, got {statuses:?}"
    );
    Ok(())
}
