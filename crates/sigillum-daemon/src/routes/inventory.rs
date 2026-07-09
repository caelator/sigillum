//! Route handlers for wallet inventory and read-only discovery.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::{
    ChainProfileDeleteRequest, ChainProfileUpsertRequest, ConsolidationPlanApproveRequest,
    ConsolidationPlanExportRequest, ConsolidationPlanGenerateRequest,
    ConsolidationPlanSimulateRequest, CounterpartyCreateRequest, CounterpartyDeleteRequest,
    CounterpartyUpdateRequest, DiscoveryJobMutationRequest, NftMetadataFetchRequest,
    NftMetadataOptInDeleteRequest, NftMetadataOptInUpsertRequest, NftMetadataSettingsUpdateRequest,
    PlanEnqueuePlanRequest, PlanEnqueueStepRequest, RiskCatalogDeleteRequest,
    RiskCatalogUpsertRequest, TokenRegistryDeleteRequest, TokenRegistryImportRequest,
    TreasuryPolicyUpdateRequest, TreasuryReceiveAllocateRequest, TreasuryReceiveRotateRequest,
    WalletInventoryScanRequest, WatchAddressBookDeleteRequest, WatchAddressBookUpsertRequest,
};

use crate::AppState;
use crate::service::SigillumService;

use super::{bearer_token, service_response, validated};

pub(crate) async fn list_wallet_inventory(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_wallet_inventory(bearer_token(&headers).as_deref()))
}

pub(crate) async fn scan_wallet_inventory_evm(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<WalletInventoryScanRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .scan_wallet_inventory_evm(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn list_nft_metadata_optins(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_nft_metadata_optins(bearer_token(&headers).as_deref()))
}

pub(crate) async fn upsert_nft_metadata_optin(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<NftMetadataOptInUpsertRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .upsert_nft_metadata_optin(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn delete_nft_metadata_optin(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<NftMetadataOptInDeleteRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .delete_nft_metadata_optin(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn update_nft_metadata_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<NftMetadataSettingsUpdateRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .update_nft_metadata_settings(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn fetch_nft_metadata(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<NftMetadataFetchRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .fetch_nft_metadata(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn list_watch_address_book(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_watch_address_book(bearer_token(&headers).as_deref()))
}

pub(crate) async fn upsert_watch_address_book_entry(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<WatchAddressBookUpsertRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .upsert_watch_address_book_entry(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn delete_watch_address_book_entry(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<WatchAddressBookDeleteRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .delete_watch_address_book_entry(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn list_token_registry(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_token_registry(bearer_token(&headers).as_deref()))
}

pub(crate) async fn import_token_registry(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<TokenRegistryImportRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .import_token_registry(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn delete_token_registry_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<TokenRegistryDeleteRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .delete_token_registry_list(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn list_chain_profiles(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_chain_profiles(bearer_token(&headers).as_deref()))
}

pub(crate) async fn upsert_chain_profile(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ChainProfileUpsertRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .upsert_chain_profile(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn delete_chain_profile(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ChainProfileDeleteRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .delete_chain_profile(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn list_discovery_jobs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_discovery_jobs(bearer_token(&headers).as_deref()))
}

pub(crate) async fn cancel_discovery_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<DiscoveryJobMutationRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .cancel_discovery_job(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn resume_discovery_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<DiscoveryJobMutationRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .resume_discovery_job(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn list_risk_findings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_risk_findings(bearer_token(&headers).as_deref()))
}

pub(crate) async fn list_risk_catalog(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_risk_catalog(bearer_token(&headers).as_deref()))
}

pub(crate) async fn upsert_risk_catalog_entry(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<RiskCatalogUpsertRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .upsert_risk_catalog_entry(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn delete_risk_catalog_entry(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<RiskCatalogDeleteRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .delete_risk_catalog_entry(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn list_consolidation_plans(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_consolidation_plans(bearer_token(&headers).as_deref()))
}

pub(crate) async fn treasury_overview(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.treasury_overview(bearer_token(&headers).as_deref()))
}

pub(crate) async fn receiving_overview(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.receiving_overview(bearer_token(&headers).as_deref()))
}

pub(crate) async fn refresh_receiving_balances(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .refresh_receiving_balances(bearer_token(&headers).as_deref())
            .await,
    )
}

pub(crate) async fn get_treasury_policy(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.get_treasury_policy(bearer_token(&headers).as_deref()))
}

pub(crate) async fn update_treasury_policy(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<TreasuryPolicyUpdateRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .update_treasury_policy(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn list_treasury_receive_allocations(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_treasury_receive_allocations(bearer_token(&headers).as_deref()))
}

pub(crate) async fn allocate_treasury_receive_address(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<TreasuryReceiveAllocateRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .allocate_treasury_receive_address(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn rotate_treasury_receive_address(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<TreasuryReceiveRotateRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .rotate_treasury_receive_address(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn list_treasury_parties(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_parties(bearer_token(&headers).as_deref()))
}

pub(crate) async fn create_treasury_party(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CounterpartyCreateRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .create_party(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn update_treasury_party(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CounterpartyUpdateRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .update_party(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn delete_treasury_party(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CounterpartyDeleteRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .delete_party(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn generate_consolidation_plan(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ConsolidationPlanGenerateRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .generate_consolidation_plan(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn approve_consolidation_plan(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ConsolidationPlanApproveRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .approve_consolidation_plan(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn simulate_consolidation_plan(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ConsolidationPlanSimulateRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .simulate_consolidation_plan(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn export_consolidation_plan(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ConsolidationPlanExportRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(service.export_consolidation_plan(bearer_token(&headers).as_deref(), body))
}

pub(crate) async fn enqueue_consolidation_plan_step(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<PlanEnqueueStepRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .enqueue_consolidation_plan_step(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn enqueue_consolidation_plan(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<PlanEnqueuePlanRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .enqueue_consolidation_plan(bearer_token(&headers).as_deref(), body)
            .await,
    )
}
