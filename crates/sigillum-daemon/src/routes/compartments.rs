//! Route handlers for compartment management: initialization, addition, removal, and switching.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::request::{
    CompartmentAddRequest, CompartmentInitRequest, CompartmentRemoveRequest,
    CompartmentSwitchRequest,
};

use crate::AppState;
use crate::service::SigillumService;

use super::{ValidatedJson, bearer_token, service_response};

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
    ValidatedJson(body): ValidatedJson<CompartmentAddRequest>,
) -> Response {
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
    ValidatedJson(body): ValidatedJson<CompartmentRemoveRequest>,
) -> Response {
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
    ValidatedJson(body): ValidatedJson<CompartmentInitRequest>,
) -> Response {
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
    ValidatedJson(body): ValidatedJson<CompartmentSwitchRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .switch_compartment(bearer_token(&headers).as_deref(), body)
            .await,
    )
}
