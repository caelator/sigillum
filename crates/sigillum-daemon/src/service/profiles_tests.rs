use std::sync::Arc;

use axum::http::StatusCode;
use sigillum_api::{EthXpubWalletProfileUpsertRequest, EvmProviderProfile};
use sigillum_core::{
    derive_ethereum_account_xpub_from_mnemonic, derive_ethereum_xpub_control_branch_from_mnemonic,
    derive_ethereum_xpub_receive_branch_from_mnemonic,
};
use sigillum_fido2::config::CompartmentMeta;
use tempfile::TempDir;

use super::*;
use crate::AppState;
use crate::profiles::{ProfileRegistry, save_profiles};

const TEST_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

fn provider_profile() -> EvmProviderProfile {
    EvmProviderProfile {
        name: "mainnet".into(),
        rpc_url: "http://127.0.0.1:8545".into(),
        auth_token_key: None,
        compartment_id: 0,
        chain_id: 1,
        max_priority_fee_per_gas_hex: None,
        max_fee_per_gas_hex: None,
        native_gas_limit: None,
        erc20_gas_limit: None,
    }
}

fn unlock_default_compartment(state: &AppState) {
    state.unlock_compartment(
        0,
        [7u8; 32],
        CompartmentMeta {
            id: 0,
            label: "default".into(),
            threshold: 1,
            passphrase_mode: None,
        },
    );
}

fn xpub_upsert_request(
    external_receive_xpub: Option<String>,
    external_receive_path: Option<String>,
    external_account_xpub: Option<String>,
) -> EthXpubWalletProfileUpsertRequest {
    EthXpubWalletProfileUpsertRequest {
        name: "external-ledger".into(),
        project_account: 0,
        provider_profile: "mainnet".into(),
        compartment_id: Some(0),
        chain_id: Some(1),
        external_receive_xpub,
        external_receive_path,
        external_account_xpub,
        default_destination_address: None,
        execution_enabled: Some(true),
    }
}

#[tokio::test]
async fn imported_xpub_profile_forces_watch_only_execution() {
    let dir = TempDir::new().unwrap();
    let state = Arc::new(AppState::new(dir.path().to_path_buf()));
    unlock_default_compartment(&state);
    let session = state.create_session(Some(0));
    let service = SigillumService::new(state);
    save_profiles(
        dir.path(),
        &ProfileRegistry {
            evm_providers: vec![provider_profile()],
            ..Default::default()
        },
    )
    .unwrap();
    let export = derive_ethereum_xpub_receive_branch_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();

    let response = service
        .upsert_eth_xpub_wallet_profile(
            Some(&session),
            xpub_upsert_request(Some(export.receive_xpub.clone()), None, None),
        )
        .await
        .unwrap();

    assert_eq!(
        response.profile.external_receive_xpub,
        Some(export.receive_xpub)
    );
    assert!(!response.profile.execution_enabled);
}

#[tokio::test]
async fn imported_account_xpub_profile_forces_watch_only_execution() {
    let dir = TempDir::new().unwrap();
    let state = Arc::new(AppState::new(dir.path().to_path_buf()));
    unlock_default_compartment(&state);
    let session = state.create_session(Some(0));
    let service = SigillumService::new(state);
    save_profiles(
        dir.path(),
        &ProfileRegistry {
            evm_providers: vec![provider_profile()],
            ..Default::default()
        },
    )
    .unwrap();
    let account_xpub = derive_ethereum_account_xpub_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();

    let response = service
        .upsert_eth_xpub_wallet_profile(
            Some(&session),
            xpub_upsert_request(None, None, Some(account_xpub.clone())),
        )
        .await
        .unwrap();

    assert_eq!(response.profile.external_receive_xpub, None);
    assert_eq!(response.profile.external_account_xpub, Some(account_xpub));
    assert!(!response.profile.execution_enabled);
}

#[tokio::test]
async fn imported_custom_path_xpub_profile_forces_watch_only_execution() {
    let dir = TempDir::new().unwrap();
    let state = Arc::new(AppState::new(dir.path().to_path_buf()));
    unlock_default_compartment(&state);
    let session = state.create_session(Some(0));
    let service = SigillumService::new(state);
    save_profiles(
        dir.path(),
        &ProfileRegistry {
            evm_providers: vec![provider_profile()],
            ..Default::default()
        },
    )
    .unwrap();
    let export = derive_ethereum_xpub_control_branch_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();

    let response = service
        .upsert_eth_xpub_wallet_profile(
            Some(&session),
            xpub_upsert_request(
                Some(export.receive_xpub.clone()),
                Some(export.receive_path.clone()),
                None,
            ),
        )
        .await
        .unwrap();

    assert_eq!(
        response.profile.external_receive_xpub,
        Some(export.receive_xpub)
    );
    assert_eq!(
        response.profile.external_receive_path,
        Some(export.receive_path)
    );
    assert_eq!(response.profile.external_account_xpub, None);
    assert!(!response.profile.execution_enabled);
}

#[tokio::test]
async fn imported_xpub_profile_rejects_invalid_receive_branch() {
    let dir = TempDir::new().unwrap();
    let state = Arc::new(AppState::new(dir.path().to_path_buf()));
    unlock_default_compartment(&state);
    let session = state.create_session(Some(0));
    let service = SigillumService::new(state);
    save_profiles(
        dir.path(),
        &ProfileRegistry {
            evm_providers: vec![provider_profile()],
            ..Default::default()
        },
    )
    .unwrap();

    let error = service
        .upsert_eth_xpub_wallet_profile(
            Some(&session),
            xpub_upsert_request(Some("not-a-real-xpub".into()), None, None),
        )
        .await
        .unwrap_err();

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error.message(), "invalid receive-branch xpub");
}

#[tokio::test]
async fn imported_xpub_profile_rejects_mixed_external_sources() {
    let dir = TempDir::new().unwrap();
    let state = Arc::new(AppState::new(dir.path().to_path_buf()));
    unlock_default_compartment(&state);
    let session = state.create_session(Some(0));
    let service = SigillumService::new(state);
    save_profiles(
        dir.path(),
        &ProfileRegistry {
            evm_providers: vec![provider_profile()],
            ..Default::default()
        },
    )
    .unwrap();
    let receive =
        derive_ethereum_xpub_receive_branch_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();
    let account = derive_ethereum_account_xpub_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();

    let error = service
        .upsert_eth_xpub_wallet_profile(
            Some(&session),
            xpub_upsert_request(Some(receive.receive_xpub), None, Some(account)),
        )
        .await
        .unwrap_err();

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        error.message(),
        "external_receive_xpub and external_account_xpub are mutually exclusive"
    );
}
