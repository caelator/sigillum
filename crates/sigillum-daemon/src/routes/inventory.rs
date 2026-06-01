//! Route handlers for wallet inventory and read-only discovery.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::{
    ChainProfileDeleteRequest, ChainProfileUpsertRequest, ConsolidationPlanApproveRequest,
    ConsolidationPlanExportRequest, ConsolidationPlanGenerateRequest,
    ConsolidationPlanSimulateRequest, DiscoveryJobMutationRequest, RiskCatalogDeleteRequest,
    RiskCatalogUpsertRequest, WalletInventoryScanRequest,
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
