use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};
use sigillum_api::{
    EthXpubWalletProfile, EvmProviderProfile, TokenRegistryEntry, TokenRegistryList,
    WalletInventoryScanRequest,
};
use sigillum_fido2::config::CompartmentMeta;
use tempfile::TempDir;

use super::*;
use crate::profiles::{ProfileRegistry, save_profiles};
use crate::token_registry::{TokenRegistryState, save_token_registry};
use crate::{AppState, service::SigillumService};

const TEST_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const COMPARTMENT_ZERO_TOKEN: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const COMPARTMENT_ONE_TOKEN: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn compartment(id: usize, label: &str) -> CompartmentMeta {
    CompartmentMeta {
        id,
        label: label.into(),
        threshold: id + 1,
        passphrase_mode: None,
    }
}

async fn spawn_recording_rpc() -> (
    SocketAddr,
    Arc<Mutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
) {
    fn rpc_response(request: &Value, calls: &Mutex<Vec<String>>) -> Value {
        let method = request["method"].as_str().unwrap_or_default();
        let result = match method {
            "eth_chainId" => json!("0x1"),
            "eth_blockNumber" => json!("0x20"),
            "eth_getBalance" | "eth_getTransactionCount" => json!("0x0"),
            "eth_call" => {
                let address = request["params"][0]["to"]
                    .as_str()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                calls.lock().unwrap().push(address);
                json!("0x1")
            }
            _ => json!("0x0"),
        };
        json!({
            "jsonrpc": "2.0",
            "id": request.get("id").cloned().unwrap_or_else(|| json!(1)),
            "result": result,
        })
    }

    async fn rpc_handler(
        State(calls): State<Arc<Mutex<Vec<String>>>>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        if let Some(requests) = body.as_array() {
            Json(Value::Array(
                requests
                    .iter()
                    .map(|request| rpc_response(request, &calls))
                    .collect(),
            ))
        } else {
            Json(rpc_response(&body, &calls))
        }
    }

    let calls = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/", post(rpc_handler))
        .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (address, calls, handle)
}

#[tokio::test(flavor = "current_thread")]
async fn token_registry_preparation_rejects_active_compartment_aba() {
    let dir = TempDir::new().unwrap();
    let app_state =
        Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
    app_state.unlock_compartment(0, [1u8; 32], compartment(0, "daily"));
    app_state.unlock_compartment(1, [2u8; 32], compartment(1, "secure"));
    let session = app_state.create_session(Some(0));
    let service = SigillumService::new(app_state.clone());
    let (rpc_address, rpc_calls, rpc_handle) = spawn_recording_rpc().await;
    let exported =
        sigillum_core::derive_ethereum_xpub_receive_branch_from_mnemonic(TEST_MNEMONIC, None, 0)
            .unwrap();

    save_profiles(
        &app_state.base_dir,
        &ProfileRegistry {
            evm_providers: vec![EvmProviderProfile {
                name: "mainnet".into(),
                rpc_url: format!("http://{rpc_address}/"),
                auth_token_key: None,
                compartment_id: 0,
                chain_id: 1,
                max_priority_fee_per_gas_hex: None,
                max_fee_per_gas_hex: None,
                native_gas_limit: None,
                erc20_gas_limit: None,
                fee_estimation_enabled: false,
            }],
            eth_xpub_wallets: vec![EthXpubWalletProfile {
                name: "watch-main".into(),
                project_account: 0,
                provider_profile: "mainnet".into(),
                compartment_id: 0,
                chain_id: Some(1),
                external_receive_xpub: Some(exported.receive_xpub),
                external_receive_path: Some(exported.receive_path),
                external_account_xpub: None,
                external_account_path: None,
                default_destination_address: None,
                execution_enabled: false,
            }],
            ..Default::default()
        },
    )
    .unwrap();
    save_token_registry(
        &app_state.base_dir,
        &TokenRegistryState {
            lists: vec![
                TokenRegistryList {
                    id: "compartment-zero".into(),
                    name: "compartment-zero".into(),
                    compartment_id: 0,
                    source: "test".into(),
                    entries: vec![TokenRegistryEntry {
                        chain_id: 1,
                        address: COMPARTMENT_ZERO_TOKEN.into(),
                        symbol: "ZERO".into(),
                        decimals: 18,
                    }],
                    created_at_unix: 1,
                    updated_at_unix: 1,
                },
                TokenRegistryList {
                    id: "compartment-one".into(),
                    name: "compartment-one".into(),
                    compartment_id: 1,
                    source: "test".into(),
                    entries: vec![TokenRegistryEntry {
                        chain_id: 1,
                        address: COMPARTMENT_ONE_TOKEN.into(),
                        symbol: "ONE".into(),
                        decimals: 18,
                    }],
                    created_at_unix: 1,
                    updated_at_unix: 1,
                },
            ],
        },
    )
    .unwrap();

    let session_context = service
        .capture_session_operation_context(Some(&session))
        .unwrap();
    let held_operation = app_state.operation_guard().await;

    // Preparation runs while the live session points at compartment 1. It
    // must still resolve compartment-scoped inputs from the immutable
    // compartment-0 admission context captured above.
    app_state.switch_active_for(&session, 1).unwrap();
    let prepared = service
        .prepare_evm_scan(
            &session_context,
            WalletInventoryScanRequest {
                wallet_family: Some(WALLET_FAMILY_ETH_XPUB.into()),
                wallet_profile: Some("watch-main".into()),
                provider_profile: Some("mainnet".into()),
                gap_limit: Some(1),
                max_index: Some(0),
                probe_token_registry: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
    let prepared_tokens = prepared
        .token_registry_probe
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .map(|entry| entry.token_address.as_str())
        .collect::<Vec<_>>();
    assert_eq!(prepared_tokens, vec![COMPARTMENT_ZERO_TOKEN]);

    // Queue the prepared scan behind the held admission boundary, restore the
    // live session to compartment 0, and only then permit acquire. A live
    // preparation lookup would now survive the acquire-time ABA check and
    // leak compartment 1's registry into provider calls and durable state.
    let operation = app_state.start_operation(OPERATION_KIND_INVENTORY_SCAN_EVM, Vec::new());
    let queued_service = service.clone();
    let queued = tokio::spawn(async move {
        queued_service
            .execute_evm_scan(session_context, prepared, operation, None)
            .await
    });
    tokio::task::yield_now().await;
    assert!(
        crate::inventory::load_wallet_inventory(&app_state.base_dir)
            .unwrap()
            .jobs
            .is_empty()
    );
    app_state.switch_active_for(&session, 0).unwrap();
    drop(held_operation);

    let response = queued.await.unwrap().unwrap();
    let calls = rpc_calls.lock().unwrap().clone();
    let inventory = crate::inventory::load_wallet_inventory(&app_state.base_dir).unwrap();

    assert!(
        calls
            .iter()
            .any(|address| address == COMPARTMENT_ZERO_TOKEN)
    );
    assert!(!calls.iter().any(|address| address == COMPARTMENT_ONE_TOKEN));
    assert!(response.holdings.iter().any(|holding| {
        holding.asset_address.as_deref() == Some(COMPARTMENT_ZERO_TOKEN)
            && holding.source == "token_registry:compartment-zero"
    }));
    assert!(
        !response
            .holdings
            .iter()
            .any(|holding| holding.asset_address.as_deref() == Some(COMPARTMENT_ONE_TOKEN))
    );
    assert!(inventory.holdings.iter().any(|holding| {
        holding.asset_address.as_deref() == Some(COMPARTMENT_ZERO_TOKEN)
            && holding.source == "token_registry:compartment-zero"
    }));
    assert!(
        !inventory
            .holdings
            .iter()
            .any(|holding| holding.asset_address.as_deref() == Some(COMPARTMENT_ONE_TOKEN))
    );

    rpc_handle.abort();
}
