use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::request::{BiometricEnrollRequest, BiometricUnlockRequest};

use crate::AppState;

use super::{bearer_token, service_response, validated};

pub(crate) async fn biometric_challenge(State(state): State<Arc<AppState>>) -> Response {
    service_response(crate::api::biometric::issue_challenge(state).await)
}

pub(crate) async fn biometric_unlock(
    State(state): State<Arc<AppState>>,
    Json(body): Json<BiometricUnlockRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(body) => body,
        Err(response) => return response,
    };
    service_response(crate::api::biometric::unlock(state, body).await)
}

pub(crate) async fn biometric_enroll(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<BiometricEnrollRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(body) => body,
        Err(response) => return response,
    };
    service_response(
        crate::api::biometric::enroll(state, bearer_token(&headers).as_deref(), body).await,
    )
}
