//! Route handlers for compartment management: initialization, addition, removal, and switching.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::request::{
    CompartmentAddRequest, CompartmentInitRequest, CompartmentRemoveRequest,
    CompartmentSwitchRequest,
};

use crate::AppState;
use crate::service::SigillumService;

use super::{bearer_token, service_response, validated};

pub(crate) async fn compartment_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_compartments(bearer_token(&headers).as_deref()))
}

pub(crate) async fn compartment_add(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CompartmentAddRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .add_compartment(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn compartment_remove(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CompartmentRemoveRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .remove_compartment(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn compartment_init(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CompartmentInitRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .init_compartment(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn compartment_switch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CompartmentSwitchRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .switch_compartment(bearer_token(&headers).as_deref(), body)
            .await,
    )
}
