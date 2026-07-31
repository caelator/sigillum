//! Route handlers for EVM provider and wallet profiles management.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::{
    EthSeedWalletCreateRequest, EthSeedWalletProfileUpsertRequest,
    EthStealthSendErc20WithProfileRequest, EthStealthSendWithProfileRequest,
    EthStealthWalletProfileUpsertRequest, EthXpubWalletProfileUpsertRequest,
    EvmProfileDeleteRequest, EvmProviderProfileUpsertRequest,
};

use crate::AppState;
use crate::service::SigillumService;

use super::{ValidatedJson, bearer_token, service_response};

pub(crate) async fn evm_provider_profiles_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_evm_provider_profiles(bearer_token(&headers).as_deref()))
}

pub(crate) async fn evm_provider_profiles_upsert(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EvmProviderProfileUpsertRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .upsert_evm_provider_profile(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn evm_provider_profiles_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EvmProfileDeleteRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .delete_evm_provider_profile(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn eth_stealth_wallet_profiles_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_eth_stealth_wallet_profiles(bearer_token(&headers).as_deref()))
}

pub(crate) async fn eth_stealth_wallet_profiles_upsert(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthStealthWalletProfileUpsertRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .upsert_eth_stealth_wallet_profile(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn eth_stealth_wallet_profiles_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EvmProfileDeleteRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .delete_eth_stealth_wallet_profile(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn eth_xpub_wallet_profiles_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_eth_xpub_wallet_profiles(bearer_token(&headers).as_deref()))
}

pub(crate) async fn eth_xpub_wallet_profiles_upsert(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthXpubWalletProfileUpsertRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .upsert_eth_xpub_wallet_profile(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn eth_xpub_wallet_profiles_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EvmProfileDeleteRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .delete_eth_xpub_wallet_profile(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn eth_seed_wallet_profiles_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_eth_seed_wallet_profiles(bearer_token(&headers).as_deref()))
}

pub(crate) async fn eth_seed_wallet_profiles_upsert(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthSeedWalletProfileUpsertRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .upsert_eth_seed_wallet_profile(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn eth_seed_wallet_profiles_create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthSeedWalletCreateRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .create_eth_seed_wallet_profile(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn eth_seed_wallet_profiles_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EvmProfileDeleteRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .delete_eth_seed_wallet_profile(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn eth_stealth_send_with_profile(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthStealthSendWithProfileRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .eth_stealth_send_with_profile(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn eth_stealth_send_erc20_with_profile(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthStealthSendErc20WithProfileRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .eth_stealth_send_erc20_with_profile(bearer_token(&headers).as_deref(), body)
            .await,
    )
}
