use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use sigillum_api::request::{
    CompartmentDefinition, CompartmentSwitchRequest, CounterpartyCreateRequest,
    CounterpartyDeleteRequest, CounterpartyUpdateRequest, Eip1559Fees, EvmProviderRef,
    Fido2SetupRequest, Fido2UnlockRequest, NftMetadataFetchRequest, NftMetadataOptInDeleteRequest,
    NftMetadataOptInUpsertRequest, NftMetadataSettingsUpdateRequest,
    QueueEthStealthErc20SweepRequest, QueueEthStealthNativeSweepRequest,
    QueueEthStealthTransferRequest, QueueProcessRequest, ReceivingDepositTagRequest,
    TokenRegistryImportRequest,
};

use super::*;

#[derive(Clone)]
struct TestState;

const NFT_METADATA_CONTRACT: &str = "0xdead00000000000000000000000000000000dead";
const NFT_METADATA_GATEWAY: &str = "https://ipfs.example.invalid/ipfs/";
const SESSION_T: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SESSION_T2: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const SESSION_T3: &str = "3333333333333333333333333333333333333333333333333333333333333333";

#[test]
fn constructor_normalizes_base_url_without_session_token() {
    let client = SigillumClient::new("http://127.0.0.1:3200///").expect("client builds");

    assert_eq!(client.session_token(), None);
    assert_eq!(
        normalize_base_url("http://127.0.0.1:3200///".to_string()),
        "http://127.0.0.1:3200"
    );
}

#[test]
fn poisoned_session_token_lock_restores_logged_out_invariant() {
    let client =
        std::sync::Arc::new(SigillumClient::new("http://127.0.0.1:3200").expect("client builds"));
    client.set_session_token("stale-token");
    let poisoner = client.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoner.session_token.lock().unwrap();
        panic!("intentional session-token poison");
    })
    .join();

    assert_eq!(client.session_token(), None);
    assert!(!client.session_token.is_poisoned());
    client.set_session_token("fresh-token");
    assert_eq!(client.session_token().as_deref(), Some("fresh-token"));
}

async fn unlock() -> Json<serde_json::Value> {
    Json(json!({
        "status": "unlocked",
        "method": "passphrase",
        "session_token": SESSION_T,
        "unlocked_compartments": [{
            "id": 0,
            "label": "default",
            "threshold": 1
        }],
        "active_compartment_id": 0
    }))
}

async fn api_keys(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" && auth != format!("Bearer {SESSION_T}") {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (StatusCode::OK, Json(json!({ "keys": ["alpha", "beta"] })))
}

async fn nft_metadata_optins_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" && auth != format!("Bearer {SESSION_T}") {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "opt_ins": [{
                "chain_id": 1,
                "contract_address": NFT_METADATA_CONTRACT,
                "enabled": true,
                "created_at_unix": 1,
                "updated_at_unix": 2
            }],
            "ipfs_gateway_url": NFT_METADATA_GATEWAY
        })),
    )
}

async fn nft_metadata_optin_upsert_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" && auth != format!("Bearer {SESSION_T}") {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["chain_id"], 1);
    assert_eq!(body["contract_address"], NFT_METADATA_CONTRACT);
    assert_eq!(body["enabled"], true);
    (
        StatusCode::OK,
        Json(json!({
            "status": "upserted",
            "opt_in": {
                "chain_id": 1,
                "contract_address": NFT_METADATA_CONTRACT,
                "enabled": true,
                "created_at_unix": 1,
                "updated_at_unix": 3
            }
        })),
    )
}

async fn nft_metadata_optin_delete_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" && auth != format!("Bearer {SESSION_T}") {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["chain_id"], 1);
    assert_eq!(body["contract_address"], NFT_METADATA_CONTRACT);
    (
        StatusCode::OK,
        Json(json!({
            "status": "deleted",
            "opt_in": {
                "chain_id": 1,
                "contract_address": NFT_METADATA_CONTRACT,
                "enabled": false,
                "created_at_unix": 1,
                "updated_at_unix": 4
            }
        })),
    )
}

async fn nft_metadata_settings_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["ipfs_gateway_url"], NFT_METADATA_GATEWAY);
    (
        StatusCode::OK,
        Json(json!({
            "status": "updated",
            "ipfs_gateway_url": NFT_METADATA_GATEWAY
        })),
    )
}

async fn nft_metadata_fetch_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["chain_id"], 1);
    assert_eq!(body["contract_address"], NFT_METADATA_CONTRACT);
    assert_eq!(body["limit"], 1);
    (
        StatusCode::OK,
        Json(json!({
            "fetched": 1,
            "entries": [{
                "chain_id": 1,
                "contract_address": NFT_METADATA_CONTRACT,
                "token_id_hex": "0x1",
                "metadata_uri": "ipfs://fake/1",
                "name": "Fake NFT",
                "spam_label": "ok",
                "updated_at_unix": 5
            }]
        })),
    )
}

async fn resolve_batch_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }

    let values = body["entries"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|entry| {
            json!({
                "env_name": entry["env_name"],
                "reference": entry["reference"],
                "value": format!("resolved:{}", entry["reference"].as_str().unwrap_or_default()),
            })
        })
        .collect::<Vec<_>>();

    (StatusCode::OK, Json(json!({ "values": values })))
}

async fn audit_run_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }

    (
        StatusCode::OK,
        Json(json!({
            "status": if body["success"].as_bool().unwrap_or(false) { "ok" } else { "failed" }
        })),
    )
}

async fn export_snapshot_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "status": "exported",
            "snapshot_hex": "6869",
            "summary": {
                "created_at_unix": 1,
                "file_count": 1,
                "total_bytes": 2,
            }
        })),
    )
}

async fn restore_snapshot_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["snapshot_hex"], "6f6b");
    (
        StatusCode::OK,
        Json(json!({
            "status": "restored",
            "summary": {
                "created_at_unix": 2,
                "file_count": 1,
                "total_bytes": 2,
            },
            "requires_reauth": true,
        })),
    )
}

async fn audit_route(
    headers: HeaderMap,
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(query.0.get("tail").map(String::as_str), Some("10"));
    (
        StatusCode::OK,
        Json(json!({
            "events": [
                {
                    "created_at_unix": 1,
                    "kind": "secret.set",
                    "compartment_id": 0,
                    "details": { "key": "db_pass" }
                }
            ]
        })),
    )
}

async fn audit_verify_route(
    headers: HeaderMap,
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(query.0.get("scope").map(String::as_str), Some("daemon"));
    (
        StatusCode::OK,
        Json(json!({
            "scope": "daemon",
            "status": "verified",
            "verified": 3,
            "broken": 0,
            "legacy": 1,
        })),
    )
}

async fn revoke_session_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "status": "revoked",
            "requires_reauth": true,
        })),
    )
}

async fn diagnostics_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "version": "0.1.0",
            "unlock_scope": "process-global",
            "session_scope": "per-session-active-compartment",
            "started_at_unix": 42,
            "initialized": true,
            "unlocked_compartment_count": 1,
            "active_session_count": 1,
            "default_active_compartment_id": 0,
            "max_unlocked_threshold": 1,
            "audit_log_present": true,
            "pending_operation_count": 0,
            "queue_job_count": 1,
            "blocked_queue_job_count": 0,
            "retrying_queue_job_count": 0,
            "failed_queue_job_count": 0,
            "operator_action_required_queue_job_count": 0,
            "deferred_queue_job_count": 0,
            "startup_interrupted_operation_count": 0,
            "startup_recovered_queue_job_count": 0,
            "startup_reconciled_deposit_count": 0,
            "runtime_policy": {
                "queue_default_process_limit": 50,
                "queue_max_process_limit": 500,
                "deposit_default_refresh_limit": 100,
                "deposit_max_refresh_limit": 500,
                "audit_default_limit": 25,
                "audit_max_limit": 200,
                "queue_retry_base_delay_secs": 5,
                "queue_retry_max_delay_secs": 300,
                "provider_balance_observation_concurrency": 8,
                "receiving_refresh_address_cap": 200,
                "idle_lock_secs": 900,
                "idle_lock_drain_secs": 60,
                "idle_lock_force_after_secs": 0
            },
            "eth_stealth_deposit_count": 1,
            "funded_eth_stealth_deposit_count": 1,
        })),
    )
}

async fn maintenance_run_route(
    headers: HeaderMap,
    Json(_body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "refreshed": 1,
            "detected": 1,
            "queued": 1,
            "processed": 1,
            "succeeded": 1,
            "blocked": 0,
            "retrying": 0,
            "failed": 0,
            "failures_by_cause": {
                "provider_error": 0,
                "policy_block": 0,
                "insufficient_gas": 0,
                "validation": 0,
                "unknown": 0
            },
            "deposits": [],
            "jobs": []
        })),
    )
}

async fn transit_encrypt_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["key"], "payments");
    (
        StatusCode::OK,
        Json(json!({
            "key": "payments",
            "nonce_hex": "000102030405060708090a0b",
            "ciphertext_hex": "deadbeef",
        })),
    )
}

async fn transit_decrypt_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["key"], "payments");
    (
        StatusCode::OK,
        Json(json!({
            "key": "payments",
            "plaintext_hex": "736563726574",
        })),
    )
}

async fn transit_hmac_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["key"], "payments");
    (
        StatusCode::OK,
        Json(json!({
            "key": "payments",
            "digest_hex": "00112233",
        })),
    )
}

async fn sign_transfer_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["wallet"], "payments");
    (
        StatusCode::OK,
        Json(json!({
            "wallet": "payments",
            "kind": "eth-transfer",
            "chain_id": 1,
            "nonce": 7,
            "from_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "to_address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "value_hex": "0x1",
            "data_hex": "",
            "raw_transaction_hex": "02deadbeef",
            "transaction_hash_hex": "11".repeat(32),
        })),
    )
}

async fn sign_erc20_transfer_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["wallet"], "payments");
    (
        StatusCode::OK,
        Json(json!({
            "wallet": "payments",
            "kind": "erc20-transfer",
            "chain_id": 1,
            "nonce": 8,
            "from_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "to_address": "0xcccccccccccccccccccccccccccccccccccccccc",
            "value_hex": "0x0",
            "data_hex": format!("a9059cbb{}", "00".repeat(64)),
            "raw_transaction_hex": "02cafebabe",
            "transaction_hash_hex": "22".repeat(32),
        })),
    )
}

async fn evm_nonce_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(
        body["address"],
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    (
        StatusCode::OK,
        Json(json!({
            "address": body["address"],
            "nonce": 12,
            "block_tag": "pending",
        })),
    )
}

async fn evm_balance_route(
    headers: HeaderMap,
    Json(_body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "balance_wei_hex": "0xde0b6b3a7640000",
            "block_tag": "latest",
        })),
    )
}

async fn evm_erc20_balance_route(
    headers: HeaderMap,
    Json(_body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "token_address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
            "owner_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "amount_hex": "0xf4240",
            "block_tag": "latest",
        })),
    )
}

async fn evm_fee_estimate_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["chain_id"], 1);
    assert_eq!(body["gas_limit"], 21_000);
    (
        StatusCode::OK,
        Json(json!({
            "fees": {
                "chain_id": 1,
                "max_priority_fee_per_gas_hex": "0x3b9aca00",
                "max_fee_per_gas_hex": "0x77359400",
            },
            "gas_limit": 21_000,
            "estimated_gas_cost_wei_hex": "0xf61809315000",
            "source": "provider",
        })),
    )
}

async fn evm_broadcast_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert!(
        body["raw_transaction_hex"]
            .as_str()
            .unwrap()
            .starts_with("0x02")
    );
    (
        StatusCode::OK,
        Json(json!({
            "transaction_hash_hex": "33".repeat(32),
        })),
    )
}

async fn send_transfer_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["wallet"], "payments");
    (
        StatusCode::OK,
        Json(json!({
            "wallet": "payments",
            "kind": "eth-transfer",
            "chain_id": 1,
            "nonce": 12,
            "from_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "to_address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "value_hex": "0x1",
            "data_hex": "",
            "raw_transaction_hex": "02deadbeef",
            "transaction_hash_hex": "44".repeat(32),
            "broadcast": true,
            "broadcast_transaction_hash_hex": "55".repeat(32),
        })),
    )
}

async fn send_erc20_transfer_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["wallet"], "payments");
    (
        StatusCode::OK,
        Json(json!({
            "wallet": "payments",
            "kind": "erc20-transfer",
            "chain_id": 1,
            "nonce": 13,
            "from_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "to_address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
            "value_hex": "0x0",
            "data_hex": format!("a9059cbb{}", "00".repeat(64)),
            "raw_transaction_hex": "02feedface",
            "transaction_hash_hex": "66".repeat(32),
            "broadcast": false,
            "broadcast_transaction_hash_hex": null,
        })),
    )
}

async fn profiles_evm_list_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "profiles": [{
                "name": "mainnet",
                "rpc_url": "https://provider.invalid",
                "auth_token_key": "alchemy",
                "compartment_id": 0,
                "chain_id": 1,
                "max_priority_fee_per_gas_hex": "0x1",
                "max_fee_per_gas_hex": "0x2",
                "native_gas_limit": 21000,
                "erc20_gas_limit": 65000
            }]
        })),
    )
}

async fn profiles_evm_upsert_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({ "status": "ok", "profile": body })),
    )
}

async fn profiles_eth_stealth_list_route(
    headers: HeaderMap,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "profiles": [{
                "name": "payments-mainnet",
                "wallet": "payments",
                "short_name": "eth",
                "provider_profile": "mainnet",
                "compartment_id": 0,
                "default_destination_address": "0x1111111111111111111111111111111111111111"
            }]
        })),
    )
}

async fn profiles_eth_stealth_upsert_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({ "status": "ok", "profile": body })),
    )
}

async fn send_with_profile_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["wallet_profile"], "payments-mainnet");
    (
        StatusCode::OK,
        Json(json!({
            "wallet": "payments",
            "kind": "eth-transfer",
            "chain_id": 1,
            "nonce": 14,
            "from_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "to_address": "0x1111111111111111111111111111111111111111",
            "value_hex": "0x1",
            "data_hex": "",
            "raw_transaction_hex": "02deadbeef",
            "transaction_hash_hex": "77".repeat(32),
            "broadcast": false,
            "broadcast_transaction_hash_hex": null
        })),
    )
}

async fn deposits_list_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "deposits": [{
                "id": "dep-1",
                "status": "pending",
                "asset_kind": "native",
                "wallet_profile": "payments-mainnet",
                "wallet_compartment_id": 0,
                "provider_compartment_id": 0,
                "wallet": "payments",
                "short_name": "eth",
                "stealth_meta_address": "st:eth:0x1234",
                "stealth_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "ephemeral_public_key_hex": "03".repeat(33),
                "view_tag_hex": "01",
                "auto_queue_sweep": true,
                "sweep_destination_address": "0x1111111111111111111111111111111111111111",
                "created_at_unix": 1,
                "updated_at_unix": 1
            }]
        })),
    )
}

fn token_registry_list_json(name: &str, source: &str) -> serde_json::Value {
    json!({
        "id": format!("token-registry-{name}"),
        "name": name,
        "compartment_id": 0,
        "source": source,
        "entries": [{
            "chain_id": 1,
            "address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "symbol": "FAKE",
            "decimals": 18
        }],
        "created_at_unix": 10,
        "updated_at_unix": 11
    })
}

async fn token_registry_list_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" && auth != format!("Bearer {SESSION_T}") {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "lists": [token_registry_list_json("stablecoins", "entries-json")]
        })),
    )
}

async fn token_registry_import_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" && auth != format!("Bearer {SESSION_T}") {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["name"], "stablecoins");
    assert!(body["entries_json"].is_string());
    assert!(body.get("file_path").is_none());
    (
        StatusCode::OK,
        Json(json!({
            "status": "imported",
            "list": token_registry_list_json("stablecoins", "entries-json")
        })),
    )
}

async fn token_registry_delete_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" && auth != format!("Bearer {SESSION_T}") {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["name"], "stablecoins");
    (
        StatusCode::OK,
        Json(json!({
            "status": "deleted",
            "list": token_registry_list_json("stablecoins", "entries-json")
        })),
    )
}

async fn deposits_create_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    let token_address = body
        .get("token_address")
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    (
        StatusCode::OK,
        Json(json!({
            "status": "created",
            "deposit": {
                "id": "dep-2",
                "status": "pending",
                "asset_kind": if token_address.is_some() { "erc20" } else { "native" },
                "wallet_profile": body["wallet_profile"],
                "wallet_compartment_id": 0,
                "provider_compartment_id": 0,
                "wallet": "payments",
                "short_name": "eth",
                "stealth_meta_address": "st:eth:0x1234",
                "stealth_address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "ephemeral_public_key_hex": "03".repeat(33),
                "view_tag_hex": "01",
                "token_address": token_address,
                "expected_amount_hex": body.get("expected_amount_hex").or_else(|| body.get("expected_value_wei_hex")),
                "auto_queue_sweep": body.get("auto_queue_sweep").cloned().unwrap_or(json!(false)),
                "sweep_destination_address": body.get("sweep_destination_address"),
                "min_sweep_amount_hex": body.get("min_sweep_amount_hex").or_else(|| body.get("min_sweep_value_wei_hex")),
                "note": body.get("note"),
                "created_at_unix": 2,
                "updated_at_unix": 2
            }
        })),
    )
}

async fn deposits_refresh_route(
    headers: HeaderMap,
    Json(_body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "processed": 1,
            "detected": 1,
            "queued": 1,
            "deposits": [{
                "id": "dep-2",
                "status": "sweep_queued",
                "asset_kind": "native",
                "wallet_profile": "payments-mainnet",
                "wallet_compartment_id": 0,
                "provider_compartment_id": 0,
                "wallet": "payments",
                "short_name": "eth",
                "stealth_meta_address": "st:eth:0x1234",
                "stealth_address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "ephemeral_public_key_hex": "03".repeat(33),
                "view_tag_hex": "01",
                "observed_amount_hex": "0xde0b6b3a7640000",
                "auto_queue_sweep": true,
                "sweep_destination_address": "0x1111111111111111111111111111111111111111",
                "queue_job_id": "job-3",
                "queue_job_state": "queued",
                "created_at_unix": 2,
                "updated_at_unix": 3,
                "last_checked_at_unix": 3
            }]
        })),
    )
}

async fn deposits_scan_announcements_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "status": "scanned",
            "wallet_profile": body["wallet_profile"],
            "provider_profile": "mainnet",
            "from_block": body["from_block"],
            "to_block": body.get("to_block").cloned().unwrap_or(json!("latest")),
            "scanned": 2,
            "matched": 1,
            "created": 1,
            "existing": 0,
            "deposits": [{
                "id": "dep-announced",
                "status": "pending",
                "asset_kind": if body.get("token_address").is_some() { "erc20" } else { "native" },
                "wallet_profile": body["wallet_profile"],
                "wallet_compartment_id": 0,
                "provider_compartment_id": 0,
                "wallet": "payments",
                "short_name": "eth",
                "stealth_meta_address": "st:eth:0x1234",
                "stealth_address": "0xcccccccccccccccccccccccccccccccccccccccc",
                "ephemeral_public_key_hex": "03".repeat(33),
                "view_tag_hex": "01",
                "token_address": body.get("token_address"),
                "auto_queue_sweep": body.get("auto_queue_sweep").cloned().unwrap_or(json!(false)),
                "sweep_destination_address": body.get("sweep_destination_address"),
                "min_sweep_amount_hex": body.get("min_sweep_amount_hex"),
                "note": "erc5564-announcement; block=0x100",
                "created_at_unix": 4,
                "updated_at_unix": 4
            }]
        })),
    )
}

async fn deposits_delete_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "status": "deleted",
            "deposit": {
                "id": body["id"],
                "status": "pending",
                "asset_kind": "native",
                "wallet_profile": "payments-mainnet",
                "wallet_compartment_id": 0,
                "provider_compartment_id": 0,
                "wallet": "payments",
                "short_name": "eth",
                "stealth_meta_address": "st:eth:0x1234",
                "stealth_address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "ephemeral_public_key_hex": "03".repeat(33),
                "view_tag_hex": "01",
                "auto_queue_sweep": false,
                "created_at_unix": 2,
                "updated_at_unix": 5
            }
        })),
    )
}

async fn deposits_enqueue_sweep_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "status": "queued",
            "deposit": {
                "id": body["id"],
                "status": "sweep_queued",
                "asset_kind": "native",
                "wallet_profile": "payments-mainnet",
                "wallet_compartment_id": 0,
                "provider_compartment_id": 0,
                "wallet": "payments",
                "short_name": "eth",
                "stealth_meta_address": "st:eth:0x1234",
                "stealth_address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "ephemeral_public_key_hex": "03".repeat(33),
                "view_tag_hex": "01",
                "auto_queue_sweep": true,
                "sweep_destination_address": "0x1111111111111111111111111111111111111111",
                "queue_job_id": "job-4",
                "queue_job_state": "queued",
                "created_at_unix": 2,
                "updated_at_unix": 4
            },
            "job": {
                "id": "job-4",
                "state": "queued",
                "attempts": 0,
                "created_at_unix": 4,
                "updated_at_unix": 4,
                "kind": "eth_stealth_native_sweep",
                "wallet_profile": "payments-mainnet",
                "stealth_address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "ephemeral_public_key_hex": "03".repeat(33),
                "destination_address": "0x1111111111111111111111111111111111111111"
            }
        })),
    )
}

async fn treasury_parties_list_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "parties": [{
                "id": "party-1",
                "name": "Acme Treasury",
                "note": "ops",
                "sweep_destination_address": "0x1111111111111111111111111111111111111111",
                "created_at_unix": 10
            }]
        })),
    )
}

async fn treasury_parties_create_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "status": "created",
            "party": {
                "id": "party-created",
                "name": body["name"],
                "note": body.get("note"),
                "sweep_destination_address": body.get("sweep_destination_address"),
                "created_at_unix": 11
            }
        })),
    )
}

async fn treasury_parties_update_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "status": "updated",
            "party": {
                "id": body["id"],
                "name": body["name"],
                "note": body.get("note"),
                "sweep_destination_address": body.get("sweep_destination_address"),
                "created_at_unix": 12
            }
        })),
    )
}

async fn treasury_parties_delete_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["id"], "party-1");
    (
        StatusCode::OK,
        Json(json!({
            "status": "deleted",
            "party": null
        })),
    )
}

async fn receiving_overview_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "generated_at_unix": 20,
            "include_retired": false,
            "groups": [{
                "counterparty": {
                    "id": "party-1",
                    "name": "Acme Treasury",
                    "note": null,
                    "sweep_destination_address": "0x1111111111111111111111111111111111111111",
                    "created_at_unix": 10
                },
                "item_count": 2,
                "native_total_wei_hex": "0x3",
                "items": [{
                    "source_type": "hd",
                    "address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "chain_id": 1,
                    "derivation_path": "m/44'/60'/0'/0/0",
                    "purpose": "invoice",
                    "label": "invoice-1",
                    "counterparty_id": "party-1",
                    "linkage_warning": null,
                    "balance_native_wei_hex": "0x1",
                    "balance_known": true,
                    "status": "active",
                    "created_at_unix": 21
                }, {
                    "source_type": "stealth",
                    "address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "chain_id": 1,
                    "counterparty_id": "party-1",
                    "linkage_warning": "shared sweep destination",
                    "balance_native_wei_hex": "0x2",
                    "balance_known": true,
                    "status": "pending",
                    "created_at_unix": 22
                }]
            }],
            "totals": {
                "item_count": 2,
                "hd_count": 1,
                "stealth_count": 1,
                "native_total_wei_hex": "0x3"
            },
            "coverage": {
                "addresses_total": 2,
                "addresses_with_known_balance": 2,
                "note": "all balances fresh"
            }
        })),
    )
}

async fn receiving_refresh_balances_route(
    headers: HeaderMap,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "generated_at_unix": 23,
            "addresses_requested": 2,
            "addresses_refreshed": 2,
            "addresses_skipped": 0,
            "stealth_refreshed": true,
            "provider_status": "ok",
            "errors": []
        })),
    )
}

async fn receiving_deposits_tag_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    assert_eq!(body["deposit_id"], "dep-1");
    (
        StatusCode::OK,
        Json(json!({
            "status": "tagged",
            "deposit": {
                "id": body["deposit_id"],
                "status": "pending",
                "asset_kind": "native",
                "wallet_profile": "payments-mainnet",
                "wallet_compartment_id": 0,
                "provider_compartment_id": 0,
                "wallet": "payments",
                "short_name": "eth",
                "stealth_meta_address": "st:eth:0x1234",
                "stealth_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "ephemeral_public_key_hex": "03".repeat(33),
                "view_tag_hex": "01",
                "auto_queue_sweep": true,
                "sweep_destination_address": "0x1111111111111111111111111111111111111111",
                "created_at_unix": 1,
                "updated_at_unix": 24,
                "counterparty_id": body.get("counterparty_id")
            }
        })),
    )
}

async fn queue_list_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "jobs": [{
                "id": "job-1",
                "state": "queued",
                "attempts": 0,
                "created_at_unix": 1,
                "updated_at_unix": 1,
                "kind": "eth_stealth_transfer",
                "wallet_profile": "payments-mainnet",
                "stealth_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "ephemeral_public_key_hex": "03".repeat(33),
                "value_wei_hex": "0x1"
            }]
        })),
    )
}

async fn queue_enqueue_route(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    let kind = if body.get("token_address").is_some() && body.get("recipient_address").is_some() {
        "eth_stealth_erc20_sweep"
    } else if body.get("token_address").is_some() {
        "eth_stealth_erc20_transfer"
    } else if body.get("min_value_wei_hex").is_some() || body.get("destination_address").is_some() {
        "eth_stealth_native_sweep"
    } else {
        "eth_stealth_transfer"
    };
    (
        StatusCode::OK,
        Json(json!({
            "status": "queued",
            "job": {
                "id": "job-2",
                "state": "queued",
                "attempts": 0,
                "created_at_unix": 2,
                "updated_at_unix": 2,
                "kind": kind,
                "wallet_profile": body["wallet_profile"],
                "stealth_address": body["stealth_address"],
                "ephemeral_public_key_hex": body["ephemeral_public_key_hex"],
                "value_wei_hex": body["value_wei_hex"],
                "destination_address": body["destination_address"],
                "token_address": body["token_address"],
                "recipient_address": body["recipient_address"],
                "min_value_wei_hex": body["min_value_wei_hex"],
                "min_amount_hex": body["min_amount_hex"]
            }
        })),
    )
}

async fn queue_process_route(
    headers: HeaderMap,
    Json(_body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "processed": 1,
            "succeeded": 1,
            "blocked": 0,
            "retrying": 0,
            "failed": 0,
            "jobs": [{
                "id": "job-2",
                "state": "sent",
                "attempts": 1,
                "created_at_unix": 2,
                "updated_at_unix": 3,
                "kind": "eth_stealth_transfer",
                "wallet_profile": "payments-mainnet",
                "stealth_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "ephemeral_public_key_hex": "03".repeat(33),
                "value_wei_hex": "0x1",
                "transaction_hash_hex": "88".repeat(32),
                "broadcast_transaction_hash_hex": "99".repeat(32)
            }]
        })),
    )
}

async fn queue_pause_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "status": "paused",
            "execution_paused": true
        })),
    )
}

async fn queue_resume_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer test-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing auth" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "status": "resumed",
            "execution_paused": false
        })),
    )
}

async fn spawn_test_server() -> Option<SocketAddr> {
    let app = Router::new()
        .route("/api/unlock", post(unlock))
        .route("/api/api-keys", get(api_keys))
        .route(
            "/api/inventory/nft-metadata/opt-ins",
            get(nft_metadata_optins_route),
        )
        .route(
            "/api/inventory/nft-metadata/opt-ins/upsert",
            post(nft_metadata_optin_upsert_route),
        )
        .route(
            "/api/inventory/nft-metadata/opt-ins/delete",
            post(nft_metadata_optin_delete_route),
        )
        .route(
            "/api/inventory/nft-metadata/settings",
            post(nft_metadata_settings_route),
        )
        .route(
            "/api/inventory/nft-metadata/fetch",
            post(nft_metadata_fetch_route),
        )
        .route("/api/secrets/resolve-batch", post(resolve_batch_route))
        .route("/api/audit", get(audit_route))
        .route("/api/audit/verify", get(audit_verify_route))
        .route("/api/audit/run", post(audit_run_route))
        .route("/api/diagnostics", get(diagnostics_route))
        .route("/api/maintenance/run", post(maintenance_run_route))
        .route("/api/session/revoke", post(revoke_session_route))
        .route("/api/backup/export", post(export_snapshot_route))
        .route("/api/backup/restore", post(restore_snapshot_route))
        .route("/api/transit/encrypt", post(transit_encrypt_route))
        .route("/api/transit/decrypt", post(transit_decrypt_route))
        .route("/api/transit/hmac", post(transit_hmac_route))
        .route("/api/evm/nonce", post(evm_nonce_route))
        .route("/api/evm/balance", post(evm_balance_route))
        .route("/api/evm/erc20-balance", post(evm_erc20_balance_route))
        .route("/api/evm/fees/estimate", post(evm_fee_estimate_route))
        .route("/api/evm/broadcast", post(evm_broadcast_route))
        .route("/api/profiles/evm", get(profiles_evm_list_route))
        .route("/api/profiles/evm/upsert", post(profiles_evm_upsert_route))
        .route(
            "/api/profiles/eth-stealth",
            get(profiles_eth_stealth_list_route),
        )
        .route(
            "/api/profiles/eth-stealth/upsert",
            post(profiles_eth_stealth_upsert_route),
        )
        .route(
            "/api/wallets/eth-stealth/sign-transfer",
            post(sign_transfer_route),
        )
        .route(
            "/api/wallets/eth-stealth/sign-erc20-transfer",
            post(sign_erc20_transfer_route),
        )
        .route(
            "/api/wallets/eth-stealth/send-transfer",
            post(send_transfer_route),
        )
        .route(
            "/api/wallets/eth-stealth/send-erc20-transfer",
            post(send_erc20_transfer_route),
        )
        .route(
            "/api/wallets/eth-stealth/send-with-profile",
            post(send_with_profile_route),
        )
        .route("/api/deposits/eth-stealth", get(deposits_list_route))
        .route(
            "/api/inventory/token-registry",
            get(token_registry_list_route),
        )
        .route(
            "/api/inventory/token-registry/import",
            post(token_registry_import_route),
        )
        .route(
            "/api/inventory/token-registry/delete",
            post(token_registry_delete_route),
        )
        .route(
            "/api/deposits/eth-stealth/create-native",
            post(deposits_create_route),
        )
        .route(
            "/api/deposits/eth-stealth/create-erc20",
            post(deposits_create_route),
        )
        .route(
            "/api/deposits/eth-stealth/scan-announcements",
            post(deposits_scan_announcements_route),
        )
        .route(
            "/api/deposits/eth-stealth/delete",
            post(deposits_delete_route),
        )
        .route(
            "/api/deposits/eth-stealth/refresh",
            post(deposits_refresh_route),
        )
        .route(
            "/api/deposits/eth-stealth/enqueue-sweep",
            post(deposits_enqueue_sweep_route),
        )
        .route(
            "/api/treasury/parties",
            get(treasury_parties_list_route).post(treasury_parties_create_route),
        )
        .route(
            "/api/treasury/parties/update",
            post(treasury_parties_update_route),
        )
        .route(
            "/api/treasury/parties/delete",
            post(treasury_parties_delete_route),
        )
        .route("/api/receiving/overview", get(receiving_overview_route))
        .route(
            "/api/receiving/refresh-balances",
            post(receiving_refresh_balances_route),
        )
        .route(
            "/api/receiving/deposits/tag",
            post(receiving_deposits_tag_route),
        )
        .route("/api/queue/jobs", get(queue_list_route))
        .route(
            "/api/queue/enqueue/eth-stealth-transfer",
            post(queue_enqueue_route),
        )
        .route(
            "/api/queue/enqueue/eth-stealth-erc20-transfer",
            post(queue_enqueue_route),
        )
        .route(
            "/api/queue/enqueue/eth-stealth-native-sweep",
            post(queue_enqueue_route),
        )
        .route(
            "/api/queue/enqueue/eth-stealth-erc20-sweep",
            post(queue_enqueue_route),
        )
        .route("/api/queue/process", post(queue_process_route))
        .route("/api/queue/pause", post(queue_pause_route))
        .route("/api/queue/resume", post(queue_resume_route))
        .with_state(TestState);

    let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("skipping loopback test: sandbox blocks loopback bind: {error}");
            return None;
        }
    };
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Some(addr)
}

async fn spawn_router(app: Router) -> Option<SocketAddr> {
    let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("skipping loopback test: sandbox blocks loopback bind: {error}");
            return None;
        }
    };
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Some(addr)
}

#[derive(Clone)]
struct DelayedResponseState {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    response: serde_json::Value,
    status: StatusCode,
}

async fn delayed_session_response(
    State(state): State<DelayedResponseState>,
    headers: HeaderMap,
) -> (StatusCode, Json<serde_json::Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(
        auth.is_empty() || auth == format!("Bearer {SESSION_T}"),
        "unexpected request authorization: {auth:?}"
    );
    state.entered.notify_one();
    state.release.notified().await;
    (state.status, Json(state.response))
}

#[derive(Clone, Copy)]
enum EstablishingTransition {
    Passphrase,
    Biometric,
    Fido2Unlock,
    Fido2Setup,
}

impl EstablishingTransition {
    fn path(self) -> &'static str {
        match self {
            Self::Passphrase => "/api/unlock",
            Self::Biometric => "/api/biometric/unlock",
            Self::Fido2Unlock => "/api/fido2/unlock",
            Self::Fido2Setup => "/api/fido2/setup",
        }
    }

    fn response(self) -> serde_json::Value {
        match self {
            Self::Passphrase => unlock_response("passphrase"),
            Self::Biometric => unlock_response("biometric"),
            Self::Fido2Unlock => unlock_response("fido2"),
            Self::Fido2Setup => json!({
                "status": "setup_complete",
                "is_first_key": true,
                "total_keys": 1,
                "compartments": 1,
                "unlocked": true,
                "session_token": SESSION_T2
            }),
        }
    }
}

fn unlock_response(method: &str) -> serde_json::Value {
    json!({
        "status": "unlocked",
        "method": method,
        "session_token": SESSION_T2,
        "unlocked_compartments": [{
            "id": 0,
            "label": "default",
            "threshold": 1
        }],
        "active_compartment_id": 0
    })
}

async fn assert_aborted_establishing_transition_completes(kind: EstablishingTransition) {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let app = Router::new()
        .route(kind.path(), post(delayed_session_response))
        .with_state(DelayedResponseState {
            entered: entered.clone(),
            release: release.clone(),
            response: kind.response(),
            status: StatusCode::OK,
        });
    let addr = spawn_router(app)
        .await
        .expect("loopback is required for establishing-transition cancellation test");
    let client = Arc::new(SigillumClient::new(format!("http://{addr}")).unwrap());

    let caller = tokio::spawn({
        let client = client.clone();
        async move {
            match kind {
                EstablishingTransition::Passphrase => client
                    .unlock_with_passphrase("passphrase")
                    .await
                    .map(|_| ()),
                EstablishingTransition::Biometric => {
                    client.biometric_unlock("payload".into()).await.map(|_| ())
                }
                EstablishingTransition::Fido2Unlock => client
                    .fido2_unlock(Fido2UnlockRequest {
                        pins: Vec::new(),
                        tap_count: 1,
                    })
                    .await
                    .map(|_| ()),
                EstablishingTransition::Fido2Setup => client
                    .fido2_setup(Fido2SetupRequest {
                        pin: None,
                        label: "primary".into(),
                        compartments: vec![CompartmentDefinition {
                            label: "default".into(),
                            threshold: 1,
                            passphrase_mode: None,
                        }],
                        passphrase: None,
                    })
                    .await
                    .map(|_| ()),
            }
        }
    });
    entered.notified().await;

    // Dropping the public future must detach only the waiter, never the owned
    // transition worker that holds the writer gate and adopts the issued token.
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    release.notify_one();
    let transition = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.session_transition.read(),
    )
    .await
    .expect("owned establishing worker must release the transition gate");
    drop(transition);
    assert_eq!(client.session_token().as_deref(), Some(SESSION_T2));
}

#[derive(Clone)]
struct SwitchResponseState {
    response: serde_json::Value,
}

async fn switch_response(
    State(state): State<SwitchResponseState>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let expected = format!("Bearer {SESSION_T}");
    assert_eq!(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some(expected.as_str())
    );
    Json(state.response)
}

async fn confirmed_lock_response(headers: HeaderMap) -> Json<serde_json::Value> {
    let expected = format!("Bearer {SESSION_T}");
    assert_eq!(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some(expected.as_str())
    );
    Json(json!({ "status": "locked" }))
}

#[derive(Clone)]
struct TransitionOrderingState {
    switch_entered: Arc<tokio::sync::Notify>,
    switch_release: Arc<tokio::sync::Notify>,
    status_calls: Arc<AtomicUsize>,
}

async fn delayed_rotating_switch(
    State(state): State<TransitionOrderingState>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let expected = format!("Bearer {SESSION_T}");
    assert_eq!(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some(expected.as_str())
    );
    state.switch_entered.notify_one();
    state.switch_release.notified().await;
    Json(json!({
        "status": "switched",
        "compartment_id": 2,
        "compartment_label": "secure",
        "session_token": SESSION_T2
    }))
}

async fn counted_status(State(state): State<TransitionOrderingState>) -> Json<serde_json::Value> {
    state.status_calls.fetch_add(1, Ordering::SeqCst);
    Json(json!({
        "locked": false,
        "initialized": true,
        "active_compartment": null,
        "unlocked_compartments": [],
        "fido2": null
    }))
}

#[derive(Clone)]
struct EmergencyLockState {
    operation_entered: Arc<tokio::sync::Notify>,
    operation_release: Arc<tokio::sync::Notify>,
    lock_entered: Arc<tokio::sync::Notify>,
    lock_release: Arc<tokio::sync::Notify>,
    expected_lock_token: &'static str,
}

async fn held_status_for_emergency_lock(
    State(state): State<EmergencyLockState>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let expected = format!("Bearer {SESSION_T}");
    assert_eq!(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some(expected.as_str())
    );
    state.operation_entered.notify_one();
    state.operation_release.notified().await;
    Json(json!({
        "locked": false,
        "initialized": true,
        "active_compartment": null,
        "unlocked_compartments": [],
        "fido2": null
    }))
}

async fn held_unlock_for_emergency_lock(
    State(state): State<EmergencyLockState>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    assert!(headers.get(header::AUTHORIZATION).is_none());
    state.operation_entered.notify_one();
    state.operation_release.notified().await;
    Json(unlock_response("passphrase"))
}

async fn held_emergency_lock(
    State(state): State<EmergencyLockState>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let expected = format!("Bearer {}", state.expected_lock_token);
    assert_eq!(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some(expected.as_str())
    );
    state.lock_entered.notify_one();
    state.lock_release.notified().await;
    Json(json!({ "status": "locked" }))
}

async fn counted_unlock_request(
    State(calls): State<Arc<AtomicUsize>>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    assert!(headers.get(header::AUTHORIZATION).is_none());
    calls.fetch_add(1, Ordering::SeqCst);
    Json(unlock_response("passphrase"))
}

#[derive(Clone)]
struct FallbackQueueState {
    switch_entered: Arc<tokio::sync::Notify>,
    switch_release: Arc<tokio::sync::Notify>,
    unlock_calls: Arc<AtomicUsize>,
}

async fn delayed_ambiguous_switch(
    State(state): State<FallbackQueueState>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let expected = format!("Bearer {SESSION_T}");
    assert_eq!(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some(expected.as_str())
    );
    state.switch_entered.notify_one();
    state.switch_release.notified().await;
    Json(json!({
        "status": "switched",
        "compartment_id": 2,
        "compartment_label": "secure",
        "session_token": SESSION_T
    }))
}

async fn counted_queued_unlock(State(state): State<FallbackQueueState>) -> Json<serde_json::Value> {
    state.unlock_calls.fetch_add(1, Ordering::SeqCst);
    Json(unlock_response("passphrase"))
}

async fn confirmed_fallback_lock(
    State(_state): State<FallbackQueueState>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let expected = format!("Bearer {SESSION_T}");
    assert_eq!(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some(expected.as_str())
    );
    Json(json!({ "status": "locked" }))
}

#[derive(Clone)]
struct SerializedSwitchState {
    first_entered: Arc<tokio::sync::Notify>,
    first_release: Arc<tokio::sync::Notify>,
    calls: Arc<AtomicUsize>,
}

async fn serialized_switch_response(
    State(state): State<SerializedSwitchState>,
    headers: HeaderMap,
    Json(request): Json<CompartmentSwitchRequest>,
) -> Json<serde_json::Value> {
    state.calls.fetch_add(1, Ordering::SeqCst);
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    match request.id {
        2 => {
            let expected = format!("Bearer {SESSION_T}");
            assert_eq!(auth, Some(expected.as_str()));
            state.first_entered.notify_one();
            state.first_release.notified().await;
            Json(json!({
                "status": "switched",
                "compartment_id": 2,
                "compartment_label": "secure",
                "session_token": SESSION_T2
            }))
        }
        3 => {
            let expected = format!("Bearer {SESSION_T2}");
            assert_eq!(auth, Some(expected.as_str()));
            Json(json!({
                "status": "switched",
                "compartment_id": 3,
                "compartment_label": "treasury",
                "session_token": SESSION_T3
            }))
        }
        id => panic!("unexpected compartment id {id}"),
    }
}

#[tokio::test]
async fn late_unauthorized_response_cannot_clear_newer_session() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let app = Router::new()
        .route("/api/status", get(delayed_session_response))
        .with_state(DelayedResponseState {
            entered: entered.clone(),
            release: release.clone(),
            response: json!({ "error": "expired" }),
            status: StatusCode::UNAUTHORIZED,
        });
    let Some(addr) = spawn_router(app).await else {
        return;
    };
    let client = Arc::new(SigillumClient::new(format!("http://{addr}")).unwrap());
    client.set_session_token(SESSION_T);
    let pending = tokio::spawn({
        let client = client.clone();
        async move { client.status().await }
    });
    entered.notified().await;
    client.set_session_token(SESSION_T2);
    release.notify_one();

    assert!(matches!(
        pending.await.unwrap(),
        Err(ClientError::SessionContextChanged)
    ));
    assert_eq!(client.session_token().as_deref(), Some(SESSION_T2));
}

#[tokio::test]
async fn malformed_unauthorized_response_still_clears_session_boundary() {
    let app = Router::new().route(
        "/api/status",
        get(|| async { (StatusCode::UNAUTHORIZED, "not-json") }),
    );
    let Some(addr) = spawn_router(app).await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).unwrap();
    client.set_session_token(SESSION_T);

    assert!(matches!(
        client.status().await,
        Err(ClientError::Api {
            status: StatusCode::UNAUTHORIZED,
            ..
        })
    ));
    assert_eq!(client.session_token(), None);
}

#[tokio::test]
async fn malformed_locked_response_clears_global_session_boundary() {
    let app = Router::new().route(
        "/api/status",
        get(|| async { (StatusCode::LOCKED, "not-json") }),
    );
    let Some(addr) = spawn_router(app).await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).unwrap();
    client.set_session_token(SESSION_T);

    assert!(matches!(
        client.status().await,
        Err(ClientError::Api {
            status: StatusCode::LOCKED,
            ..
        })
    ));
    assert_eq!(client.session_token(), None);
}

#[tokio::test]
async fn late_success_response_is_rejected_after_explicit_session_change() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let app = Router::new()
        .route("/api/status", get(delayed_session_response))
        .with_state(DelayedResponseState {
            entered: entered.clone(),
            release: release.clone(),
            response: json!({
                "locked": false,
                "initialized": true,
                "active_compartment": null,
                "unlocked_compartments": [],
                "fido2": null
            }),
            status: StatusCode::OK,
        });
    let Some(addr) = spawn_router(app).await else {
        return;
    };
    let client = Arc::new(SigillumClient::new(format!("http://{addr}")).unwrap());
    client.set_session_token(SESSION_T);
    let pending = tokio::spawn({
        let client = client.clone();
        async move { client.status().await }
    });
    entered.notified().await;
    client.set_session_token(SESSION_T2);
    release.notify_one();

    assert!(matches!(
        pending.await.unwrap(),
        Err(ClientError::SessionContextChanged)
    ));
    assert_eq!(client.session_token().as_deref(), Some(SESSION_T2));
}

#[tokio::test]
async fn late_unlock_response_cannot_overwrite_newer_session() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let app = Router::new()
        .route("/api/unlock", post(delayed_session_response))
        .with_state(DelayedResponseState {
            entered: entered.clone(),
            release: release.clone(),
            response: json!({
                "status": "unlocked",
                "method": "passphrase",
                "session_token": SESSION_T2,
                "unlocked_compartments": []
            }),
            status: StatusCode::OK,
        });
    let Some(addr) = spawn_router(app).await else {
        return;
    };
    let client = Arc::new(SigillumClient::new(format!("http://{addr}")).unwrap());
    let pending = tokio::spawn({
        let client = client.clone();
        async move { client.unlock_with_passphrase("passphrase").await }
    });
    entered.notified().await;
    client.set_session_token(SESSION_T3);
    release.notify_one();

    assert!(matches!(
        pending.await.unwrap(),
        Err(ClientError::SessionContextChanged)
    ));
    assert_eq!(client.session_token().as_deref(), Some(SESSION_T3));
}

#[tokio::test]
async fn aborted_passphrase_unlock_still_adopts_issued_session() {
    assert_aborted_establishing_transition_completes(EstablishingTransition::Passphrase).await;
}

#[tokio::test]
async fn aborted_biometric_unlock_still_adopts_issued_session() {
    assert_aborted_establishing_transition_completes(EstablishingTransition::Biometric).await;
}

#[tokio::test]
async fn aborted_fido2_unlock_still_adopts_issued_session() {
    assert_aborted_establishing_transition_completes(EstablishingTransition::Fido2Unlock).await;
}

#[tokio::test]
async fn aborted_fido2_setup_still_adopts_issued_session() {
    assert_aborted_establishing_transition_completes(EstablishingTransition::Fido2Setup).await;
}

#[tokio::test]
async fn malformed_establishing_response_requires_daemon_restart() {
    let app = Router::new().route(
        "/api/unlock",
        post(|| async {
            Json(json!({
                "status": "unlocked",
                "session_token": SESSION_T2,
                "unlocked_compartments": []
            }))
        }),
    );
    let Some(addr) = spawn_router(app).await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).unwrap();

    assert!(matches!(
        client.unlock_with_passphrase("passphrase").await,
        Err(ClientError::SessionTransitionLockUnconfirmed(_))
    ));
    assert_eq!(client.session_token(), None);
}

#[tokio::test]
async fn unlock_requires_active_compartment_in_unlocked_set() {
    for response in [
        json!({
            "status": "unlocked",
            "method": "passphrase",
            "session_token": SESSION_T2,
            "unlocked_compartments": [],
            "active_compartment_id": 0
        }),
        json!({
            "status": "unlocked",
            "method": "passphrase",
            "session_token": SESSION_T2,
            "unlocked_compartments": [{
                "id": 1,
                "label": "other",
                "threshold": 1
            }],
            "active_compartment_id": 0
        }),
    ] {
        let app = Router::new().route("/api/unlock", post(move || async move { Json(response) }));
        let Some(addr) = spawn_router(app).await else {
            return;
        };
        let client = SigillumClient::new(format!("http://{addr}")).unwrap();

        assert!(matches!(
            client.unlock_with_passphrase("passphrase").await,
            Err(ClientError::SessionTransitionLockUnconfirmed(_))
        ));
        assert_eq!(client.session_token(), None);
    }
}

#[tokio::test]
async fn fido2_setup_requires_nonzero_keys_and_exact_compartment_count() {
    for (total_keys, compartments) in [(0, 1), (1, 2)] {
        let app = Router::new().route(
            "/api/fido2/setup",
            post(move || async move {
                Json(json!({
                    "status": "setup_complete",
                    "is_first_key": true,
                    "total_keys": total_keys,
                    "compartments": compartments,
                    "unlocked": true,
                    "session_token": SESSION_T2
                }))
            }),
        );
        let Some(addr) = spawn_router(app).await else {
            return;
        };
        let client = SigillumClient::new(format!("http://{addr}")).unwrap();
        let request = Fido2SetupRequest {
            pin: None,
            label: "primary".into(),
            compartments: vec![CompartmentDefinition {
                label: "default".into(),
                threshold: 1,
                passphrase_mode: None,
            }],
            passphrase: None,
        };

        assert!(matches!(
            client.fido2_setup(request).await,
            Err(ClientError::SessionTransitionLockUnconfirmed(_))
        ));
        assert_eq!(client.session_token(), None);
    }
}

async fn run_switch_response_test(
    response: serde_json::Value,
) -> (
    Result<SwitchCompartmentResponse, ClientError>,
    SigillumClient,
) {
    let app = Router::new()
        .route("/api/compartment/switch", post(switch_response))
        .route("/api/lock", post(confirmed_lock_response))
        .with_state(SwitchResponseState { response });
    let addr = spawn_router(app)
        .await
        .expect("loopback is required for switch response tests");
    let client = SigillumClient::new(format!("http://{addr}")).unwrap();
    client.set_session_token(SESSION_T);
    let result = client.switch_compartment(2).await;
    (result, client)
}

#[tokio::test]
async fn valid_compartment_switch_atomically_adopts_rotated_token() {
    let (result, client) = run_switch_response_test(json!({
        "status": "switched",
        "compartment_id": 2,
        "compartment_label": "secure",
        "session_token": SESSION_T2
    }))
    .await;

    assert_eq!(result.unwrap().session_token, SESSION_T2);
    assert_eq!(client.session_token().as_deref(), Some(SESSION_T2));
}

#[tokio::test]
async fn switch_excludes_stale_generic_request_before_rotated_token_adoption() {
    let switch_entered = Arc::new(tokio::sync::Notify::new());
    let switch_release = Arc::new(tokio::sync::Notify::new());
    let status_calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/api/compartment/switch", post(delayed_rotating_switch))
        .route("/api/status", get(counted_status))
        .with_state(TransitionOrderingState {
            switch_entered: switch_entered.clone(),
            switch_release: switch_release.clone(),
            status_calls: status_calls.clone(),
        });
    let addr = spawn_router(app)
        .await
        .expect("loopback is required for transition ordering test");
    let client = Arc::new(SigillumClient::new(format!("http://{addr}")).unwrap());
    client.set_session_token(SESSION_T);

    let switching = tokio::spawn({
        let client = client.clone();
        async move { client.switch_compartment(2).await }
    });
    switch_entered.notified().await;

    // Capture T while the daemon has committed to the switch but its T2
    // response is deliberately withheld. The read side must wait for the
    // transition and reject this stale builder before it reaches the daemon.
    let stale_builder = client.request(Method::GET, "/api/status");
    let stale_request = tokio::spawn({
        let client = client.clone();
        async move { client.send::<StatusResponse>(stale_builder).await }
    });
    tokio::task::yield_now().await;
    assert_eq!(status_calls.load(Ordering::SeqCst), 0);

    switch_release.notify_one();
    assert_eq!(switching.await.unwrap().unwrap().session_token, SESSION_T2);
    assert!(matches!(
        stale_request.await.unwrap(),
        Err(ClientError::SessionContextChanged)
    ));
    assert_eq!(status_calls.load(Ordering::SeqCst), 0);
    assert_eq!(client.session_token().as_deref(), Some(SESSION_T2));
}

#[tokio::test]
async fn aborted_switch_caller_cannot_abandon_rotated_session() {
    let switch_entered = Arc::new(tokio::sync::Notify::new());
    let switch_release = Arc::new(tokio::sync::Notify::new());
    let status_calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/api/compartment/switch", post(delayed_rotating_switch))
        .route("/api/status", get(counted_status))
        .with_state(TransitionOrderingState {
            switch_entered: switch_entered.clone(),
            switch_release: switch_release.clone(),
            status_calls: status_calls.clone(),
        });
    let addr = spawn_router(app)
        .await
        .expect("loopback is required for switch cancellation test");
    let client = Arc::new(SigillumClient::new(format!("http://{addr}")).unwrap());
    client.set_session_token(SESSION_T);

    let caller = tokio::spawn({
        let client = client.clone();
        async move { client.switch_compartment(2).await }
    });
    switch_entered.notified().await;

    // The daemon has committed T -> T2 but withholds the response. Cancel the
    // public caller, then prove its owned worker retains the writer gate and
    // finishes adoption before any request captured with stale T can run.
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    let stale_builder = client.request(Method::GET, "/api/status");
    let stale_request = tokio::spawn({
        let client = client.clone();
        async move { client.send::<StatusResponse>(stale_builder).await }
    });
    tokio::task::yield_now().await;
    assert_eq!(status_calls.load(Ordering::SeqCst), 0);

    switch_release.notify_one();
    let stale_result = tokio::time::timeout(std::time::Duration::from_secs(2), stale_request)
        .await
        .expect("owned switch worker must finish after caller cancellation")
        .unwrap();
    assert!(matches!(
        stale_result,
        Err(ClientError::SessionContextChanged)
    ));
    assert_eq!(status_calls.load(Ordering::SeqCst), 0);
    assert_eq!(client.session_token().as_deref(), Some(SESSION_T2));
}

#[tokio::test]
async fn emergency_lock_prevents_inflight_switch_token_resurrection() {
    let switch_entered = Arc::new(tokio::sync::Notify::new());
    let switch_release = Arc::new(tokio::sync::Notify::new());
    let status_calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/api/compartment/switch", post(delayed_rotating_switch))
        .route("/api/lock", post(confirmed_lock_response))
        .with_state(TransitionOrderingState {
            switch_entered: switch_entered.clone(),
            switch_release: switch_release.clone(),
            status_calls,
        });
    let addr = spawn_router(app)
        .await
        .expect("loopback is required for switch-versus-Lock test");
    let client = Arc::new(SigillumClient::new(format!("http://{addr}")).unwrap());
    client.set_session_token(SESSION_T);

    let switching = tokio::spawn({
        let client = client.clone();
        async move { client.switch_compartment(2).await }
    });
    switch_entered.notified().await;

    assert_eq!(client.lock().await.unwrap().status, "locked");
    assert_eq!(client.session_token(), None);
    switch_release.notify_one();

    assert!(matches!(
        switching.await.unwrap(),
        Err(ClientError::SessionContextChanged)
    ));
    assert_eq!(client.session_token(), None);
}

#[tokio::test]
async fn confirmed_fallback_lock_rejects_preboundary_queued_unlock() {
    let switch_entered = Arc::new(tokio::sync::Notify::new());
    let switch_release = Arc::new(tokio::sync::Notify::new());
    let unlock_calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/api/compartment/switch", post(delayed_ambiguous_switch))
        .route("/api/unlock", post(counted_queued_unlock))
        .route("/api/lock", post(confirmed_fallback_lock))
        .with_state(FallbackQueueState {
            switch_entered: switch_entered.clone(),
            switch_release: switch_release.clone(),
            unlock_calls: unlock_calls.clone(),
        });
    let addr = spawn_router(app)
        .await
        .expect("loopback is required for fallback-boundary queue test");
    let client = Arc::new(SigillumClient::new(format!("http://{addr}")).unwrap());
    client.set_session_token(SESSION_T);

    let switching = tokio::spawn({
        let client = client.clone();
        async move { client.switch_compartment(2).await }
    });
    switch_entered.notified().await;
    let queued_unlock = tokio::spawn({
        let client = client.clone();
        async move { client.unlock_with_passphrase("passphrase").await }
    });
    while Arc::strong_count(&client.session_transition) < 3 {
        tokio::task::yield_now().await;
    }
    tokio::task::yield_now().await;
    switch_release.notify_one();

    assert!(matches!(
        switching.await.unwrap(),
        Err(ClientError::SessionTransitionLocked(_))
    ));
    assert!(matches!(
        queued_unlock.await.unwrap(),
        Err(ClientError::SessionContextChanged)
    ));
    assert_eq!(unlock_calls.load(Ordering::SeqCst), 0);
    assert_eq!(client.session_token(), None);
}

#[tokio::test]
async fn concurrent_compartment_switches_serialize_token_rotation() {
    let first_entered = Arc::new(tokio::sync::Notify::new());
    let first_release = Arc::new(tokio::sync::Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/api/compartment/switch", post(serialized_switch_response))
        .with_state(SerializedSwitchState {
            first_entered: first_entered.clone(),
            first_release: first_release.clone(),
            calls: calls.clone(),
        });
    let addr = spawn_router(app)
        .await
        .expect("loopback is required for serialized switch test");
    let client = Arc::new(SigillumClient::new(format!("http://{addr}")).unwrap());
    client.set_session_token(SESSION_T);

    let first = tokio::spawn({
        let client = client.clone();
        async move { client.switch_compartment(2).await }
    });
    first_entered.notified().await;
    let second = tokio::spawn({
        let client = client.clone();
        async move { client.switch_compartment(3).await }
    });
    tokio::task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    first_release.notify_one();
    assert_eq!(first.await.unwrap().unwrap().session_token, SESSION_T2);
    assert_eq!(second.await.unwrap().unwrap().session_token, SESSION_T3);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(client.session_token().as_deref(), Some(SESSION_T3));
}

#[tokio::test]
async fn malformed_compartment_switch_response_confirms_fallback_lock() {
    let (result, client) = run_switch_response_test(json!({
        "status": "switched",
        "session_token": SESSION_T2
    }))
    .await;

    assert!(matches!(
        result,
        Err(ClientError::SessionTransitionLocked(_))
    ));
    assert_eq!(client.session_token(), None);
}

#[tokio::test]
async fn same_token_compartment_switch_response_is_rejected() {
    let (result, client) = run_switch_response_test(json!({
        "status": "switched",
        "compartment_id": 2,
        "compartment_label": "secure",
        "session_token": SESSION_T
    }))
    .await;

    assert!(matches!(
        result,
        Err(ClientError::SessionTransitionLocked(_))
    ));
    assert_eq!(client.session_token(), None);
}

#[tokio::test]
async fn wrong_compartment_switch_response_is_rejected() {
    let (result, client) = run_switch_response_test(json!({
        "status": "switched",
        "compartment_id": 7,
        "compartment_label": "wrong",
        "session_token": SESSION_T2
    }))
    .await;

    assert!(matches!(
        result,
        Err(ClientError::SessionTransitionLocked(_))
    ));
    assert_eq!(client.session_token(), None);
}

#[tokio::test]
async fn ambiguous_switch_reports_when_fallback_lock_is_unconfirmed() {
    let app = Router::new()
        .route("/api/compartment/switch", post(switch_response))
        .route(
            "/api/lock",
            post(|| async {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "lock unavailable" })),
                )
            }),
        )
        .with_state(SwitchResponseState {
            response: json!({
                "status": "switched",
                "compartment_id": 2,
                "compartment_label": "secure",
                "session_token": SESSION_T
            }),
        });
    let Some(addr) = spawn_router(app).await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).unwrap();
    client.set_session_token(SESSION_T);

    assert!(matches!(
        client.switch_compartment(2).await,
        Err(ClientError::SessionTransitionLockUnconfirmed(_))
    ));
    assert_eq!(client.session_token(), None);
}

#[tokio::test]
async fn fallback_lock_requires_structural_locked_confirmation() {
    let app = Router::new()
        .route("/api/compartment/switch", post(switch_response))
        .route(
            "/api/lock",
            post(|| async { Json(json!({ "status": "ok" })) }),
        )
        .with_state(SwitchResponseState {
            response: json!({
                "status": "switched",
                "compartment_id": 2,
                "compartment_label": "secure",
                "session_token": SESSION_T
            }),
        });
    let Some(addr) = spawn_router(app).await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).unwrap();
    client.set_session_token(SESSION_T);

    assert!(matches!(
        client.switch_compartment(2).await,
        Err(ClientError::SessionTransitionLockUnconfirmed(_))
    ));
    assert_eq!(client.session_token(), None);
}

#[tokio::test]
async fn fallback_lock_rejects_misleading_locked_response_with_error() {
    let app = Router::new()
        .route("/api/compartment/switch", post(switch_response))
        .route(
            "/api/lock",
            post(|| async {
                Json(json!({
                    "status": "locked",
                    "error": "Lock was not confirmed"
                }))
            }),
        )
        .with_state(SwitchResponseState {
            response: json!({
                "status": "switched",
                "compartment_id": 2,
                "compartment_label": "secure",
                "session_token": SESSION_T
            }),
        });
    let Some(addr) = spawn_router(app).await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).unwrap();
    client.set_session_token(SESSION_T);

    assert!(matches!(
        client.switch_compartment(2).await,
        Err(ClientError::SessionTransitionLockUnconfirmed(_))
    ));
    assert_eq!(client.session_token(), None);
}

#[tokio::test]
async fn explicit_switch_api_rejection_does_not_trigger_fallback_lock() {
    let app = Router::new().route(
        "/api/compartment/switch",
        post(|| async {
            (
                StatusCode::CONFLICT,
                Json(json!({ "error": "compartment not unlocked" })),
            )
        }),
    );
    let Some(addr) = spawn_router(app).await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).unwrap();
    client.set_session_token(SESSION_T);

    assert!(matches!(
        client.switch_compartment(2).await,
        Err(ClientError::Api {
            status: StatusCode::CONFLICT,
            ..
        })
    ));
    assert_eq!(client.session_token().as_deref(), Some(SESSION_T));
}

#[tokio::test]
async fn server_error_during_switch_triggers_confirmed_fallback_lock() {
    let app = Router::new()
        .route(
            "/api/compartment/switch",
            post(|| async {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "switch completion is unknown" })),
                )
            }),
        )
        .route("/api/lock", post(confirmed_lock_response));
    let Some(addr) = spawn_router(app).await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).unwrap();
    client.set_session_token(SESSION_T);

    assert!(matches!(
        client.switch_compartment(2).await,
        Err(ClientError::SessionTransitionLocked(_))
    ));
    assert_eq!(client.session_token(), None);
}

#[tokio::test]
async fn unlock_stores_session_for_follow_up_requests() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}/")).expect("client should build");

    let unlocked = client.unlock_with_passphrase("passphrase").await.unwrap();
    assert_eq!(unlocked.status, "unlocked");
    assert_eq!(client.session_token().as_deref(), Some(SESSION_T));

    let keys = client.list_api_keys().await.unwrap();
    assert_eq!(keys, vec!["alpha".to_string(), "beta".to_string()]);
}

#[tokio::test]
async fn nft_metadata_routes_roundtrip() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let opt_ins = client.list_nft_metadata_optins().await.unwrap();
    assert_eq!(opt_ins.opt_ins.len(), 1);
    assert_eq!(opt_ins.opt_ins[0].contract_address, NFT_METADATA_CONTRACT);
    assert!(opt_ins.opt_ins[0].enabled);
    assert_eq!(
        opt_ins.ipfs_gateway_url.as_deref(),
        Some(NFT_METADATA_GATEWAY)
    );

    let upserted = client
        .upsert_nft_metadata_optin(NftMetadataOptInUpsertRequest {
            chain_id: 1,
            contract_address: NFT_METADATA_CONTRACT.into(),
            enabled: Some(true),
        })
        .await
        .unwrap();
    assert_eq!(upserted.status, "upserted");
    assert!(upserted.opt_in.enabled);

    let deleted = client
        .delete_nft_metadata_optin(NftMetadataOptInDeleteRequest {
            chain_id: 1,
            contract_address: NFT_METADATA_CONTRACT.into(),
        })
        .await
        .unwrap();
    assert_eq!(deleted.status, "deleted");
    assert!(!deleted.opt_in.enabled);

    let settings = client
        .update_nft_metadata_settings(NftMetadataSettingsUpdateRequest {
            ipfs_gateway_url: Some(NFT_METADATA_GATEWAY.into()),
        })
        .await
        .unwrap();
    assert_eq!(settings.status, "updated");
    assert_eq!(
        settings.ipfs_gateway_url.as_deref(),
        Some(NFT_METADATA_GATEWAY)
    );

    let fetched = client
        .fetch_nft_metadata(NftMetadataFetchRequest {
            chain_id: Some(1),
            contract_address: Some(NFT_METADATA_CONTRACT.into()),
            limit: Some(1),
        })
        .await
        .unwrap();
    assert_eq!(fetched.fetched, 1);
    assert_eq!(fetched.entries.len(), 1);
    assert_eq!(fetched.entries[0].contract_address, NFT_METADATA_CONTRACT);
    assert_eq!(fetched.entries[0].name.as_deref(), Some("Fake NFT"));
}

#[tokio::test]
async fn snapshot_methods_roundtrip_payload_shape() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let (snapshot, summary) = client.export_snapshot("passphrase").await.unwrap();
    assert_eq!(snapshot, b"hi");
    assert_eq!(summary.file_count, 1);

    let restored = client.restore_snapshot("passphrase", b"ok").await.unwrap();
    assert_eq!(restored.status, "restored");
    assert!(client.session_token().is_none());
}

#[tokio::test]
async fn audit_events_reads_recent_feed() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let events = client
        .audit_events_query(AuditEventQuery {
            tail: Some(10),
            ..AuditEventQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "secret.set");
    assert_eq!(events[0].compartment_id, Some(0));
}

#[tokio::test]
async fn audit_verify_reads_chain_report() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let report = client.audit_verify(Some("daemon")).await.unwrap();
    assert_eq!(report.scope, "daemon");
    assert_eq!(report.status, "verified");
    assert_eq!(report.verified, 3);
    assert_eq!(report.broken, 0);
    assert_eq!(report.legacy, 1);
}

#[tokio::test]
async fn resolve_secret_batch_roundtrips_response_shape() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let values = client
        .resolve_secret_batch(SecretResolveBatchRequest {
            entries: vec![sigillum_api::SecretResolveRequest {
                env_name: "DB_PASS".into(),
                reference: "prod:db.password".into(),
            }],
        })
        .await
        .unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].env_name, "DB_PASS");
    assert_eq!(values[0].value, "resolved:prod:db.password");
}

#[tokio::test]
async fn record_run_audit_posts_terminal_status() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let response = client
        .record_run_audit(RunAuditRequest {
            program: "npm".into(),
            args: vec!["start".into()],
            exit_code: Some(0),
            signal: None,
            success: true,
        })
        .await
        .unwrap();
    assert_eq!(response.status, "ok");
}

#[tokio::test]
async fn revoke_session_clears_cached_token() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let response = client.revoke_session().await.unwrap();
    assert_eq!(response.status, "revoked");
    assert!(response.requires_reauth);
    assert!(client.session_token().is_none());
}

#[tokio::test]
async fn emergency_lock_bypasses_hung_generic_request() {
    let operation_entered = Arc::new(tokio::sync::Notify::new());
    let operation_release = Arc::new(tokio::sync::Notify::new());
    let lock_entered = Arc::new(tokio::sync::Notify::new());
    let lock_release = Arc::new(tokio::sync::Notify::new());
    lock_release.notify_one();
    let app = Router::new()
        .route("/api/status", get(held_status_for_emergency_lock))
        .route("/api/lock", post(held_emergency_lock))
        .with_state(EmergencyLockState {
            operation_entered: operation_entered.clone(),
            operation_release: operation_release.clone(),
            lock_entered: lock_entered.clone(),
            lock_release,
            expected_lock_token: SESSION_T,
        });
    let addr = spawn_router(app)
        .await
        .expect("loopback is required for emergency Lock test");
    let client = Arc::new(SigillumClient::new(format!("http://{addr}")).unwrap());
    client.set_session_token(SESSION_T);
    let held_request = tokio::spawn({
        let client = client.clone();
        async move { client.status().await }
    });
    operation_entered.notified().await;

    let lock_result = tokio::time::timeout(std::time::Duration::from_secs(2), client.lock())
        .await
        .expect("emergency Lock must not wait for the held read-side request")
        .unwrap();
    assert_eq!(lock_result.status, "locked");
    lock_entered.notified().await;
    assert_eq!(client.session_token(), None);

    operation_release.notify_one();
    assert!(matches!(
        held_request.await.unwrap(),
        Err(ClientError::SessionContextChanged)
    ));
}

#[tokio::test]
async fn lock_intent_hides_token_refuses_replacement_and_rejects_second_lock() {
    let operation_entered = Arc::new(tokio::sync::Notify::new());
    let operation_release = Arc::new(tokio::sync::Notify::new());
    let lock_entered = Arc::new(tokio::sync::Notify::new());
    let lock_release = Arc::new(tokio::sync::Notify::new());
    let app = Router::new()
        .route("/api/lock", post(held_emergency_lock))
        .with_state(EmergencyLockState {
            operation_entered,
            operation_release,
            lock_entered: lock_entered.clone(),
            lock_release: lock_release.clone(),
            expected_lock_token: SESSION_T,
        });
    let addr = spawn_router(app)
        .await
        .expect("loopback is required for Lock intent test");
    let client = Arc::new(SigillumClient::new(format!("http://{addr}")).unwrap());
    client.set_session_token(SESSION_T);

    let locking = tokio::spawn({
        let client = client.clone();
        async move { client.lock().await }
    });
    lock_entered.notified().await;
    assert_eq!(client.session_token(), None);
    assert_eq!(
        client.session_token.lock().unwrap().as_deref(),
        Some(SESSION_T)
    );
    let generation = client.session_boundary_generation();

    client.set_session_token(SESSION_T3);
    assert_eq!(
        client.session_token.lock().unwrap().as_deref(),
        Some(SESSION_T)
    );
    client.clear_session_token();
    assert_eq!(
        client.session_token.lock().unwrap().as_deref(),
        Some(SESSION_T)
    );
    assert!(matches!(
        client.lock().await,
        Err(ClientError::SessionContextChanged)
    ));
    assert_eq!(client.session_boundary_generation(), generation);

    lock_release.notify_one();
    assert_eq!(locking.await.unwrap().unwrap().status, "locked");
    assert_eq!(client.session_token(), None);
}

#[tokio::test]
async fn preboundary_queued_unlock_never_reaches_daemon_after_lock_intent() {
    let unlock_calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/api/unlock", post(counted_unlock_request))
        .with_state(unlock_calls.clone());
    let addr = spawn_router(app)
        .await
        .expect("loopback is required for queued unlock test");
    let client = Arc::new(SigillumClient::new(format!("http://{addr}")).unwrap());
    let held_read = client.session_transition.read().await;

    let unlocking = tokio::spawn({
        let client = client.clone();
        async move { client.unlock_with_passphrase("passphrase").await }
    });
    while Arc::strong_count(&client.session_transition) < 2 {
        tokio::task::yield_now().await;
    }
    tokio::task::yield_now().await;
    let locking = tokio::spawn({
        let client = client.clone();
        async move { client.lock().await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while client.session_lock_state.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("Lock must publish intent before the held read is released");
    drop(held_read);

    assert!(matches!(
        unlocking.await.unwrap(),
        Err(ClientError::SessionContextChanged)
    ));
    assert!(matches!(
        locking.await.unwrap(),
        Err(ClientError::SessionTransitionLockUnconfirmed(_))
    ));
    assert_eq!(unlock_calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        client.status().await,
        Err(ClientError::SessionStateUnconfirmed)
    ));
}

#[tokio::test]
async fn aborted_lock_caller_does_not_cancel_emergency_lock_worker() {
    let operation_entered = Arc::new(tokio::sync::Notify::new());
    let operation_release = Arc::new(tokio::sync::Notify::new());
    let lock_entered = Arc::new(tokio::sync::Notify::new());
    let lock_release = Arc::new(tokio::sync::Notify::new());
    let app = Router::new()
        .route("/api/lock", post(held_emergency_lock))
        .with_state(EmergencyLockState {
            operation_entered,
            operation_release,
            lock_entered: lock_entered.clone(),
            lock_release: lock_release.clone(),
            expected_lock_token: SESSION_T,
        });
    let addr = spawn_router(app)
        .await
        .expect("loopback is required for Lock cancellation test");
    let client = Arc::new(SigillumClient::new(format!("http://{addr}")).unwrap());
    client.set_session_token(SESSION_T);

    let caller = tokio::spawn({
        let client = client.clone();
        async move { client.lock().await }
    });
    lock_entered.notified().await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    lock_release.notify_one();

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let raw_token = client.session_token.lock().unwrap().clone();
            let lock_state = client.session_lock_state.load(Ordering::SeqCst);
            if raw_token.is_none() && lock_state == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owned Lock worker must clear authority after caller cancellation");
}

#[tokio::test]
async fn no_token_lock_waits_for_inflight_unlock_then_locks_new_session() {
    let operation_entered = Arc::new(tokio::sync::Notify::new());
    let operation_release = Arc::new(tokio::sync::Notify::new());
    let lock_entered = Arc::new(tokio::sync::Notify::new());
    let lock_release = Arc::new(tokio::sync::Notify::new());
    let app = Router::new()
        .route("/api/unlock", post(held_unlock_for_emergency_lock))
        .route("/api/lock", post(held_emergency_lock))
        .with_state(EmergencyLockState {
            operation_entered: operation_entered.clone(),
            operation_release: operation_release.clone(),
            lock_entered: lock_entered.clone(),
            lock_release: lock_release.clone(),
            expected_lock_token: SESSION_T2,
        });
    let addr = spawn_router(app)
        .await
        .expect("loopback is required for Lock-versus-unlock test");
    let client = Arc::new(SigillumClient::new(format!("http://{addr}")).unwrap());

    let unlocking = tokio::spawn({
        let client = client.clone();
        async move { client.unlock_with_passphrase("passphrase").await }
    });
    operation_entered.notified().await;
    let locking = tokio::spawn({
        let client = client.clone();
        async move { client.lock().await }
    });
    tokio::task::yield_now().await;

    operation_release.notify_one();
    assert!(matches!(
        unlocking.await.unwrap(),
        Err(ClientError::SessionContextChanged)
    ));
    lock_entered.notified().await;
    assert_eq!(client.session_token(), None);
    assert_eq!(
        client.session_token.lock().unwrap().as_deref(),
        Some(SESSION_T2)
    );
    lock_release.notify_one();
    assert_eq!(locking.await.unwrap().unwrap().status, "locked");
    assert_eq!(client.session_token(), None);
}

#[tokio::test]
async fn lock_already_in_progress_response_clears_cached_token() {
    let app = Router::new().route(
        "/api/lock",
        post(|headers: HeaderMap| async move {
            let expected = format!("Bearer {SESSION_T}");
            assert_eq!(
                headers
                    .get(header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok()),
                Some(expected.as_str())
            );
            (
                StatusCode::LOCKED,
                Json(json!({ "error": "vault Lock is already in progress" })),
            )
        }),
    );
    let addr = spawn_router(app)
        .await
        .expect("loopback is required for Lock transition test");
    let client = SigillumClient::new(format!("http://{addr}")).unwrap();
    client.set_session_token(SESSION_T);

    assert_eq!(client.lock().await.unwrap().status, "locked");
    assert_eq!(client.session_token(), None);
}

#[tokio::test]
async fn fresh_unlock_is_allowed_after_confirmed_lock_boundary() {
    let app = Router::new()
        .route("/api/lock", post(confirmed_lock_response))
        .route(
            "/api/unlock",
            post(|| async { Json(unlock_response("passphrase")) }),
        );
    let addr = spawn_router(app)
        .await
        .expect("loopback is required for post-Lock unlock test");
    let client = SigillumClient::new(format!("http://{addr}")).unwrap();
    client.set_session_token(SESSION_T);

    assert_eq!(client.lock().await.unwrap().status, "locked");
    let response = client.unlock_with_passphrase("passphrase").await.unwrap();
    assert_eq!(response.session_token, SESSION_T2);
    assert_eq!(client.session_token().as_deref(), Some(SESSION_T2));
}

#[tokio::test]
async fn lock_rejects_misleading_success_response_with_error() {
    let app = Router::new().route(
        "/api/lock",
        post(|| async {
            Json(json!({
                "status": "locked",
                "error": "Lock was not confirmed"
            }))
        }),
    );
    let addr = spawn_router(app)
        .await
        .expect("loopback is required for Lock response validation test");
    let client = SigillumClient::new(format!("http://{addr}")).unwrap();
    client.set_session_token(SESSION_T);

    assert!(matches!(
        client.lock().await,
        Err(ClientError::SessionTransitionLockUnconfirmed(_))
    ));
    assert_eq!(client.session_token(), None);
    assert!(matches!(
        client.status().await,
        Err(ClientError::SessionStateUnconfirmed)
    ));
    client.set_session_token(SESSION_T3);
    assert_eq!(client.session_token(), None);
    assert_eq!(*client.session_token.lock().unwrap(), None);
}

#[tokio::test]
async fn diagnostics_reads_operational_metadata() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let response = client.diagnostics().await.unwrap();
    assert_eq!(response.status, "ok");
    assert_eq!(response.version, "0.1.0");
    assert_eq!(response.unlock_scope, "process-global");
    assert_eq!(response.session_scope, "per-session-active-compartment");
    assert_eq!(response.started_at_unix, 42);
    assert_eq!(response.unlocked_compartment_count, 1);
    assert_eq!(response.active_session_count, 1);
    assert_eq!(response.default_active_compartment_id, Some(0));
    assert_eq!(response.max_unlocked_threshold, Some(1));
    assert!(response.audit_log_present);
    assert_eq!(response.pending_operation_count, 0);
    assert_eq!(response.queue_job_count, 1);
    assert_eq!(response.blocked_queue_job_count, 0);
    assert_eq!(response.retrying_queue_job_count, 0);
    assert_eq!(response.failed_queue_job_count, 0);
    assert_eq!(response.operator_action_required_queue_job_count, 0);
    assert_eq!(response.deferred_queue_job_count, 0);
    assert_eq!(response.startup_interrupted_operation_count, 0);
    assert_eq!(response.startup_recovered_queue_job_count, 0);
    assert_eq!(response.startup_reconciled_deposit_count, 0);
    assert_eq!(response.runtime_policy.queue_default_process_limit, 50);
    assert_eq!(response.runtime_policy.queue_max_process_limit, 500);
    assert_eq!(response.runtime_policy.deposit_default_refresh_limit, 100);
    assert_eq!(response.runtime_policy.deposit_max_refresh_limit, 500);
    assert_eq!(response.runtime_policy.audit_default_limit, 25);
    assert_eq!(response.runtime_policy.audit_max_limit, 200);
    assert_eq!(response.runtime_policy.queue_retry_base_delay_secs, 5);
    assert_eq!(response.runtime_policy.queue_retry_max_delay_secs, 300);
    assert_eq!(
        response
            .runtime_policy
            .provider_balance_observation_concurrency,
        8
    );
    assert_eq!(response.runtime_policy.receiving_refresh_address_cap, 200);
    assert_eq!(response.runtime_policy.idle_lock_secs, 900);
    assert_eq!(response.runtime_policy.idle_lock_drain_secs, 60);
    assert_eq!(response.runtime_policy.idle_lock_force_after_secs, 0);
    assert_eq!(response.eth_stealth_deposit_count, 1);
    assert_eq!(response.funded_eth_stealth_deposit_count, 1);
}

#[tokio::test]
async fn transit_helpers_roundtrip_response_shapes() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let (nonce, ciphertext) = client
        .transit_encrypt("payments", b"secret", Some(b"aad"))
        .await
        .unwrap();
    assert_eq!(nonce.len(), 12);
    assert_eq!(ciphertext, hex::decode("deadbeef").unwrap());

    let plaintext = client
        .transit_decrypt("payments", &nonce, &ciphertext, Some(b"aad"))
        .await
        .unwrap();
    assert_eq!(plaintext, b"secret");

    let digest = client.transit_hmac("payments", b"payload").await.unwrap();
    assert_eq!(digest, hex::decode("00112233").unwrap());
}

#[tokio::test]
async fn transaction_signing_helpers_roundtrip_response_shapes() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let transfer = client
        .sign_eth_stealth_transfer(EthStealthSignTransferRequest {
            wallet: "payments".into(),
            stealth: sigillum_api::StealthPaymentRef {
                stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                ephemeral_public_key_hex: "03".repeat(33),
                view_tag_hex: Some("01".into()),
            },
            fees: sigillum_api::Eip1559Fees {
                chain_id: 1,
                max_priority_fee_per_gas_hex: "0x1".into(),
                max_fee_per_gas_hex: "0x2".into(),
            },
            nonce: 7,
            gas_limit: 21_000,
            destination_address: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            value_wei_hex: "0x1".into(),
        })
        .await
        .unwrap();
    assert_eq!(transfer.kind, "eth-transfer");
    assert!(transfer.raw_transaction_hex.starts_with("02"));

    let erc20 = client
        .sign_eth_stealth_erc20_transfer(EthStealthSignErc20TransferRequest {
            wallet: "payments".into(),
            stealth: sigillum_api::StealthPaymentRef {
                stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                ephemeral_public_key_hex: "03".repeat(33),
                view_tag_hex: Some("01".into()),
            },
            fees: sigillum_api::Eip1559Fees {
                chain_id: 1,
                max_priority_fee_per_gas_hex: "0x1".into(),
                max_fee_per_gas_hex: "0x2".into(),
            },
            nonce: 8,
            gas_limit: 65_000,
            token_address: "0xcccccccccccccccccccccccccccccccccccccccc".into(),
            recipient_address: "0xdddddddddddddddddddddddddddddddddddddddd".into(),
            amount_hex: "0x5".into(),
        })
        .await
        .unwrap();
    assert_eq!(erc20.kind, "erc20-transfer");
    assert!(erc20.data_hex.starts_with("a9059cbb"));
}

#[tokio::test]
async fn evm_provider_helpers_roundtrip_response_shapes() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let nonce = client
        .evm_nonce(EvmRpcNonceRequest {
            provider: sigillum_api::EvmProviderRef {
                rpc_url: "https://provider.invalid".into(),
                auth_token_key: Some("alchemy".into()),
                compartment_id: None,
            },
            address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            block_tag: Some("pending".into()),
        })
        .await
        .unwrap();
    assert_eq!(nonce.nonce, 12);

    let balance = client
        .evm_balance(EvmRpcBalanceRequest {
            provider: sigillum_api::EvmProviderRef {
                rpc_url: "https://provider.invalid".into(),
                auth_token_key: None,
                compartment_id: None,
            },
            address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            block_tag: None,
        })
        .await
        .unwrap();
    assert_eq!(balance.balance_wei_hex, "0xde0b6b3a7640000");

    let erc20 = client
        .evm_erc20_balance(EvmRpcErc20BalanceRequest {
            provider: sigillum_api::EvmProviderRef {
                rpc_url: "https://provider.invalid".into(),
                auth_token_key: None,
                compartment_id: None,
            },
            token_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
            owner_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            block_tag: None,
        })
        .await
        .unwrap();
    assert_eq!(erc20.amount_hex, "0xf4240");

    let fees = client
        .evm_estimate_fees(EvmFeeEstimateRequest {
            provider: sigillum_api::EvmProviderRef {
                rpc_url: "https://provider.invalid".into(),
                auth_token_key: None,
                compartment_id: None,
            },
            chain_id: 1,
            gas_limit: Some(21_000),
        })
        .await
        .unwrap();
    assert_eq!(fees.fees.chain_id, 1);
    assert_eq!(fees.gas_limit, 21_000);
    assert_eq!(fees.source, "provider");

    let broadcast = client
        .evm_broadcast(EvmRpcBroadcastRequest {
            provider: sigillum_api::EvmProviderRef {
                rpc_url: "https://provider.invalid".into(),
                auth_token_key: None,
                compartment_id: None,
            },
            raw_transaction_hex: "0x02deadbeef".into(),
        })
        .await
        .unwrap();
    assert_eq!(broadcast.transaction_hash_hex, "33".repeat(32));
}

#[tokio::test]
async fn stealth_send_helpers_roundtrip_response_shapes() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let sent = client
        .send_eth_stealth_transfer(EthStealthSendTransferRequest {
            rpc_url: "https://provider.invalid".into(),
            wallet: "payments".into(),
            stealth: StealthPaymentRef {
                stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                ephemeral_public_key_hex: "03".repeat(33),
                view_tag_hex: Some("01".into()),
            },
            fees: Eip1559Fees {
                chain_id: 1,
                max_priority_fee_per_gas_hex: "0x1".into(),
                max_fee_per_gas_hex: "0x2".into(),
            },
            destination_address: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            value_wei_hex: "0x1".into(),
            auth_token_key: None,
            provider_compartment_id: None,
            wallet_compartment_id: None,
            nonce: None,
            gas_limit: None,
            broadcast: Some(true),
        })
        .await
        .unwrap();
    assert_eq!(sent.kind, "eth-transfer");
    assert!(sent.broadcast);

    let sent_erc20 = client
        .send_eth_stealth_erc20_transfer(EthStealthSendErc20TransferRequest {
            rpc_url: "https://provider.invalid".into(),
            wallet: "payments".into(),
            stealth: StealthPaymentRef {
                stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                ephemeral_public_key_hex: "03".repeat(33),
                view_tag_hex: Some("01".into()),
            },
            fees: Eip1559Fees {
                chain_id: 1,
                max_priority_fee_per_gas_hex: "0x1".into(),
                max_fee_per_gas_hex: "0x2".into(),
            },
            token_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
            recipient_address: "0xdddddddddddddddddddddddddddddddddddddddd".into(),
            amount_hex: "0x5".into(),
            auth_token_key: None,
            provider_compartment_id: None,
            wallet_compartment_id: None,
            nonce: Some(13),
            gas_limit: Some(65_000),
            broadcast: Some(false),
        })
        .await
        .unwrap();
    assert_eq!(sent_erc20.kind, "erc20-transfer");
    assert!(!sent_erc20.broadcast);
}

#[tokio::test]
async fn profile_and_queue_helpers_roundtrip_response_shapes() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let providers = client.list_evm_provider_profiles().await.unwrap();
    assert_eq!(providers[0].name, "mainnet");

    let provider = client
        .upsert_evm_provider_profile(EvmProviderProfileUpsertRequest {
            name: "mainnet".into(),
            provider: EvmProviderRef {
                rpc_url: "https://provider.invalid".into(),
                auth_token_key: Some("alchemy".into()),
                compartment_id: Some(0),
            },
            chain_id: 1,
            max_priority_fee_per_gas_hex: Some("0x1".into()),
            max_fee_per_gas_hex: Some("0x2".into()),
            native_gas_limit: Some(21_000),
            erc20_gas_limit: Some(65_000),
            fee_estimation_enabled: Some(false),
        })
        .await
        .unwrap();
    assert_eq!(provider.profile.name, "mainnet");

    let wallets = client.list_eth_stealth_wallet_profiles().await.unwrap();
    assert_eq!(wallets[0].name, "payments-mainnet");

    let wallet = client
        .upsert_eth_stealth_wallet_profile(EthStealthWalletProfileUpsertRequest {
            name: "payments-mainnet".into(),
            wallet: "payments".into(),
            short_name: Some("eth".into()),
            provider_profile: "mainnet".into(),
            compartment_id: Some(0),
            chain_id: Some(1),
            default_destination_address: Some("0x1111111111111111111111111111111111111111".into()),
            execution_enabled: Some(false),
        })
        .await
        .unwrap();
    assert_eq!(wallet.profile.provider_profile, "mainnet");

    let sent = client
        .send_eth_stealth_with_profile(EthStealthSendWithProfileRequest {
            wallet_profile: "payments-mainnet".into(),
            stealth: StealthPaymentRef {
                stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                ephemeral_public_key_hex: "03".repeat(33),
                view_tag_hex: Some("01".into()),
            },
            value_wei_hex: "0x1".into(),
            destination_address: None,
            nonce: None,
            gas_limit: None,
            estimate_fees: None,
            broadcast: Some(false),
        })
        .await
        .unwrap();
    assert_eq!(sent.kind, "eth-transfer");

    let queued = client.list_queue_jobs().await.unwrap();
    assert_eq!(queued[0].id, "job-1");

    let enqueued = client
        .enqueue_eth_stealth_transfer(QueueEthStealthTransferRequest {
            wallet_profile: "payments-mainnet".into(),
            stealth: StealthPaymentRef {
                stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                ephemeral_public_key_hex: "03".repeat(33),
                view_tag_hex: Some("01".into()),
            },
            value_wei_hex: "0x1".into(),
            destination_address: None,
            nonce: None,
            gas_limit: None,
            estimate_fees: None,
            broadcast: None,
        })
        .await
        .unwrap();
    assert_eq!(enqueued.job.id, "job-2");

    let processed = client
        .process_queue(QueueProcessRequest {
            id: None,
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(processed.succeeded, 1);
    assert_eq!(processed.blocked, 0);
    assert_eq!(processed.retrying, 0);
    assert_eq!(processed.operator_action_required, 0);

    let paused = client.pause_queue().await.unwrap();
    assert_eq!(paused.status, "paused");
    assert!(paused.execution_paused);

    let resumed = client.resume_queue().await.unwrap();
    assert_eq!(resumed.status, "resumed");
    assert!(!resumed.execution_paused);

    let deposits = client.list_eth_stealth_deposits().await.unwrap();
    assert_eq!(deposits[0].id, "dep-1");

    let native_deposit = client
        .create_eth_stealth_native_deposit(EthStealthDepositCreateNativeRequest {
            wallet_profile: "payments-mainnet".into(),
            expected_value_wei_hex: Some("0x1".into()),
            auto_queue_sweep: Some(true),
            sweep_destination_address: Some("0x1111111111111111111111111111111111111111".into()),
            min_sweep_value_wei_hex: Some("0x1".into()),
            note: Some("invoice-42".into()),
            ephemeral_private_key_hex: None,
        })
        .await
        .unwrap();
    assert_eq!(native_deposit.deposit.asset_kind, "native");

    let erc20_deposit = client
        .create_eth_stealth_erc20_deposit(EthStealthDepositCreateErc20Request {
            wallet_profile: "payments-mainnet".into(),
            token_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
            expected_amount_hex: Some("0xf4240".into()),
            auto_queue_sweep: Some(true),
            sweep_destination_address: Some("0x1111111111111111111111111111111111111111".into()),
            min_sweep_amount_hex: Some("0xf4240".into()),
            note: None,
            ephemeral_private_key_hex: None,
        })
        .await
        .unwrap();
    assert_eq!(
        erc20_deposit.deposit.token_address.as_deref(),
        Some("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48")
    );

    let announced = client
        .scan_eth_stealth_announcements(EthStealthAnnouncementScanRequest {
            wallet_profile: "payments-mainnet".into(),
            from_block: "0x100".into(),
            to_block: Some("latest".into()),
            token_address: Some("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into()),
            limit: Some(100),
            auto_queue_sweep: Some(false),
            sweep_destination_address: Some("0x1111111111111111111111111111111111111111".into()),
            min_sweep_amount_hex: Some("0xf4240".into()),
            note: None,
        })
        .await
        .unwrap();
    assert_eq!(announced.created, 1);
    assert_eq!(announced.deposits[0].id, "dep-announced");

    let refreshed = client
        .refresh_eth_stealth_deposits(EthStealthDepositRefreshRequest {
            id: None,
            limit: None,
            auto_enqueue: Some(true),
        })
        .await
        .unwrap();
    assert_eq!(refreshed.detected, 1);
    assert_eq!(refreshed.queued, 1);

    let deposit_sweep = client
        .enqueue_eth_stealth_deposit_sweep(EthStealthDepositEnqueueSweepRequest {
            id: "dep-2".into(),
            force: Some(true),
        })
        .await
        .unwrap();
    assert_eq!(deposit_sweep.job.id, "job-4");

    let native_sweep = client
        .enqueue_eth_stealth_native_sweep(QueueEthStealthNativeSweepRequest {
            wallet_profile: "payments-mainnet".into(),
            stealth: StealthPaymentRef {
                stealth_address: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                ephemeral_public_key_hex: "03".repeat(33),
                view_tag_hex: Some("01".into()),
            },
            destination_address: Some("0x1111111111111111111111111111111111111111".into()),
            min_value_wei_hex: Some("0x1".into()),
            gas_limit: Some(21_000),
        })
        .await
        .unwrap();
    assert_eq!(native_sweep.job.id, "job-2");

    let erc20_sweep = client
        .enqueue_eth_stealth_erc20_sweep(QueueEthStealthErc20SweepRequest {
            wallet_profile: "payments-mainnet".into(),
            stealth: StealthPaymentRef {
                stealth_address: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                ephemeral_public_key_hex: "03".repeat(33),
                view_tag_hex: Some("01".into()),
            },
            token_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
            recipient_address: Some("0x1111111111111111111111111111111111111111".into()),
            min_amount_hex: Some("0xf4240".into()),
            gas_limit: Some(65_000),
        })
        .await
        .unwrap();
    assert_eq!(erc20_sweep.job.id, "job-2");

    let deleted = client
        .delete_eth_stealth_deposit(EthStealthDepositDeleteRequest { id: "dep-2".into() })
        .await
        .unwrap();
    assert_eq!(deleted.status, "deleted");

    let maintenance = client
        .run_maintenance(MaintenanceRunRequest {
            deposit_refresh_limit: Some(10),
            queue_process_limit: Some(10),
            auto_enqueue: Some(true),
        })
        .await
        .unwrap();
    assert_eq!(maintenance.status, "ok");
    assert_eq!(maintenance.succeeded, 1);
    assert_eq!(maintenance.failures_by_cause.provider_error, 0);
}

fn assert_stablecoin_registry(list: &sigillum_api::response::TokenRegistryList) {
    assert_eq!(list.name, "stablecoins");
    assert_eq!(list.source, "entries-json");
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].chain_id, 1);
    assert_eq!(
        list.entries[0].address,
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(list.entries[0].symbol, "FAKE");
    assert_eq!(list.entries[0].decimals, 18);
}

#[tokio::test]
async fn token_registry_helpers_roundtrip_response_shapes() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}/")).expect("client should build");
    client.unlock_with_passphrase("passphrase").await.unwrap();

    let lists = client.list_token_registry().await.unwrap();
    assert_eq!(lists.lists.len(), 1);
    assert_stablecoin_registry(&lists.lists[0]);

    let imported = client
        .import_token_registry(TokenRegistryImportRequest {
            name: "stablecoins".into(),
            entries_json: Some(
                r#"[{"chainId":1,"address":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","symbol":"FAKE","decimals":18}]"#
                    .into(),
            ),
            file_path: None,
        })
        .await
        .unwrap();
    assert_eq!(imported.status, "imported");
    assert_stablecoin_registry(&imported.list);

    let deleted = client
        .delete_token_registry_list("stablecoins")
        .await
        .unwrap();
    assert_eq!(deleted.status, "deleted");
    assert_stablecoin_registry(&deleted.list);
}

#[tokio::test]
async fn list_parties_parses_parties() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let response = client.list_parties().await.unwrap();
    assert_eq!(response.parties.len(), 1);
    assert_eq!(response.parties[0].id, "party-1");
    assert_eq!(response.parties[0].name, "Acme Treasury");
    assert_eq!(
        response.parties[0].sweep_destination_address.as_deref(),
        Some("0x1111111111111111111111111111111111111111")
    );
}

#[tokio::test]
async fn create_party_echoes_name() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let response = client
        .create_party(CounterpartyCreateRequest {
            name: "New Payer".into(),
            note: Some("invoice rail".into()),
            sweep_destination_address: Some("0x2222222222222222222222222222222222222222".into()),
        })
        .await
        .unwrap();
    assert_eq!(response.status, "created");
    assert_eq!(response.party.unwrap().name, "New Payer");
}

#[tokio::test]
async fn update_party_parses_mutation() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let response = client
        .update_party(CounterpartyUpdateRequest {
            id: "party-1".into(),
            name: "Updated Payer".into(),
            note: None,
            sweep_destination_address: Some("0x3333333333333333333333333333333333333333".into()),
        })
        .await
        .unwrap();
    let party = response.party.unwrap();
    assert_eq!(response.status, "updated");
    assert_eq!(party.id, "party-1");
    assert_eq!(party.name, "Updated Payer");
}

#[tokio::test]
async fn delete_party_echoes_status() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let response = client
        .delete_party(CounterpartyDeleteRequest {
            id: "party-1".into(),
        })
        .await
        .unwrap();
    assert_eq!(response.status, "deleted");
    assert!(response.party.is_none());
}

#[tokio::test]
async fn receiving_overview_parses_totals_and_coverage() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let response = client.receiving_overview().await.unwrap();
    assert_eq!(response.totals.item_count, 2);
    assert_eq!(response.totals.hd_count, 1);
    assert_eq!(response.totals.stealth_count, 1);
    assert_eq!(response.coverage.addresses_total, 2);
    assert_eq!(response.coverage.addresses_with_known_balance, 2);
    assert_eq!(
        response.groups[0].items[1].linkage_warning.as_deref(),
        Some("shared sweep destination")
    );
}

#[tokio::test]
async fn refresh_receiving_balances_parses_provider_status() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let response = client.refresh_receiving_balances().await.unwrap();
    assert_eq!(response.provider_status, "ok");
    assert_eq!(response.addresses_refreshed, 2);
    assert!(response.stealth_refreshed);
}

#[tokio::test]
async fn tag_stealth_deposit_posts_deposit_id_and_parses_status() {
    let Some(addr) = spawn_test_server().await else {
        return;
    };
    let client = SigillumClient::new(format!("http://{addr}")).expect("client should build");
    client.set_session_token("test-token");

    let response = client
        .tag_stealth_deposit(ReceivingDepositTagRequest {
            deposit_id: "dep-1".into(),
            counterparty_id: Some("party-1".into()),
        })
        .await
        .unwrap();
    assert_eq!(response.status, "tagged");
    assert_eq!(response.deposit.id, "dep-1");
    assert_eq!(response.deposit.counterparty_id.as_deref(), Some("party-1"));
}
