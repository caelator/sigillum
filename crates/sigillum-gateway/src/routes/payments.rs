//! Payment lifecycle endpoints.

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use sigillum_api::request::{
    EthStealthDepositCreateErc20Request, EthStealthDepositCreateNativeRequest,
    EthStealthDepositDeleteRequest,
};
use uuid::Uuid;

use crate::db;
use crate::db::row::Project;
use crate::error::GatewayError;
use crate::state::AppState;
use crate::validate;

fn idempotency_matches(existing: &db::row::Payment, request: &CreatePaymentRequest) -> bool {
    existing.amount_wei == request.amount_wei
        && existing.chain_id == request.chain_id as i64
        && existing.token_address.as_deref() == request.token_address.as_deref()
}

fn created_payment_response(
    payment_id: &str,
    amount_wei: &str,
    chain_id: u64,
    deposit: &sigillum_client::EthStealthDeposit,
    expires_at: &str,
) -> Value {
    json!({
        "payment_id": payment_id,
        "stealth_address": deposit.stealth_address,
        "ephemeral_public_key_hex": deposit.ephemeral_public_key_hex,
        "view_tag_hex": deposit.view_tag_hex,
        "amount_wei": amount_wei,
        "chain_id": chain_id,
        "token_address": deposit.token_address,
        "status": "pending",
        "expires_at": expires_at,
        "deposit_id": deposit.id,
    })
}

fn existing_payment_response(existing: &db::row::Payment) -> Value {
    json!({
        "payment_id": existing.id,
        "stealth_address": existing.stealth_address,
        "ephemeral_public_key_hex": existing.ephemeral_pub,
        "view_tag_hex": existing.view_tag,
        "amount_wei": existing.amount_wei,
        "chain_id": existing.chain_id,
        "token_address": existing.token_address,
        "status": existing.status,
        "expires_at": existing.expires_at,
        "deposit_id": existing.deposit_id,
        "idempotent": true,
    })
}

fn is_unique_constraint(error: &GatewayError) -> bool {
    db::is_unique_constraint(error)
}

async fn rollback_created_deposit(state: &AppState, deposit_id: &str, insert_error: &GatewayError) {
    tracing::warn!(
        deposit_id,
        error = %insert_error,
        "gateway payment persistence failed after daemon deposit creation; attempting rollback",
    );
    match state
        .daemon
        .delete_eth_stealth_deposit(EthStealthDepositDeleteRequest {
            id: deposit_id.to_string(),
        })
        .await
    {
        Ok(_) => tracing::info!(
            deposit_id,
            "rolled back daemon deposit after gateway persistence failure",
        ),
        Err(rollback_error) => tracing::error!(
            deposit_id,
            error = %rollback_error,
            "failed to roll back daemon deposit after gateway persistence failure",
        ),
    }
}

async fn resolve_effective_chain_id(
    state: &AppState,
    wallet_profile: &str,
) -> Result<u64, GatewayError> {
    let wallet = state
        .daemon
        .list_eth_stealth_wallet_profiles()
        .await?
        .into_iter()
        .find(|profile| profile.name == wallet_profile)
        .ok_or_else(|| {
            GatewayError::Conflict(format!(
                "project wallet_profile '{wallet_profile}' is not available in the daemon"
            ))
        })?;

    let provider = state
        .daemon
        .list_evm_provider_profiles()
        .await?
        .into_iter()
        .find(|profile| profile.name == wallet.provider_profile)
        .ok_or_else(|| {
            GatewayError::Conflict(format!(
                "provider profile '{}' for wallet_profile '{}' is not available in the daemon",
                wallet.provider_profile, wallet_profile
            ))
        })?;

    Ok(wallet.chain_id.unwrap_or(provider.chain_id))
}

#[derive(Debug, Deserialize)]
pub struct CreatePaymentRequest {
    /// Amount in wei (hex-encoded, e.g. "0x2386F26FC10000").
    pub amount_wei: String,
    /// EVM chain ID.
    pub chain_id: u64,
    /// ERC-20 token address (omit for native ETH).
    pub token_address: Option<String>,
    /// Arbitrary metadata the merchant wants associated with this payment.
    #[serde(default)]
    pub metadata: Value,
    /// Optional idempotency key — if repeated for the same project, returns the
    /// existing payment instead of creating a duplicate (A4).
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListPaymentsQuery {
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Validate hex-encoded wei amount (R4).
fn validate_amount_wei(s: &str) -> Result<(), GatewayError> {
    if s.is_empty() {
        return Err(GatewayError::BadRequest("amount_wei is required".into()));
    }
    let hex_str = s.strip_prefix("0x").unwrap_or(s);
    if hex_str.is_empty() {
        return Err(GatewayError::BadRequest(
            "amount_wei cannot be just '0x'".into(),
        ));
    }
    if hex_str.len() > 64 {
        return Err(GatewayError::BadRequest(
            "amount_wei exceeds maximum (256-bit)".into(),
        ));
    }
    if !hex_str.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(GatewayError::BadRequest(
            "amount_wei must be valid hex".into(),
        ));
    }
    Ok(())
}

/// `POST /api/v1/payments` — create a payment intent.
///
/// Generates a stealth address via the daemon, registers a deposit for monitoring,
/// and stores the payment in the gateway database.
///
/// Supports idempotency: if `idempotency_key` is provided and a payment with
/// the same key already exists for this project, the existing payment is returned.
pub async fn create_payment(
    State(state): State<AppState>,
    axum::Extension(project): axum::Extension<Project>,
    Json(body): Json<CreatePaymentRequest>,
) -> Result<Json<Value>, GatewayError> {
    // R4: Validate hex amount
    validate_amount_wei(&body.amount_wei)?;

    // A5: Validate token_address format if present
    if let Some(ref addr) = body.token_address {
        validate::validate_evm_address(addr)
            .map_err(|e| GatewayError::BadRequest(format!("invalid token_address: {e}")))?;
    }

    // A4: Idempotency check — return existing payment if key already used
    if let Some(ref key) = body.idempotency_key {
        if let Some(existing) =
            db::find_payment_by_idempotency_key(&state.db, &project.id, key).await?
        {
            if !idempotency_matches(&existing, &body) {
                return Err(GatewayError::Conflict(
                    "idempotency_key was reused with different payment parameters".into(),
                ));
            }

            return Ok(Json(existing_payment_response(&existing)));
        }
    }

    let resolved_chain_id = resolve_effective_chain_id(&state, &project.wallet_profile).await?;
    if body.chain_id != resolved_chain_id {
        return Err(GatewayError::BadRequest(format!(
            "chain_id {} does not match wallet_profile '{}' chain {}",
            body.chain_id, project.wallet_profile, resolved_chain_id
        )));
    }

    // 1. Register the deposit with the daemon and use the daemon response as
    // the single source of truth for the payment address we hand back.
    let deposit = if body.token_address.is_some() {
        state
            .daemon
            .create_eth_stealth_erc20_deposit(EthStealthDepositCreateErc20Request {
                wallet_profile: project.wallet_profile.clone(),
                token_address: body.token_address.clone().unwrap_or_default(),
                expected_amount_hex: Some(body.amount_wei.clone()),
                auto_queue_sweep: Some(true),
                sweep_destination_address: None,
                min_sweep_amount_hex: None,
                note: Some("gateway-payment".to_string()),
                ephemeral_private_key_hex: None,
            })
            .await?
            .deposit
    } else {
        state
            .daemon
            .create_eth_stealth_native_deposit(EthStealthDepositCreateNativeRequest {
                wallet_profile: project.wallet_profile.clone(),
                expected_value_wei_hex: Some(body.amount_wei.clone()),
                auto_queue_sweep: Some(true),
                sweep_destination_address: None,
                min_sweep_value_wei_hex: None,
                note: Some("gateway-payment".to_string()),
                ephemeral_private_key_hex: None,
            })
            .await?
            .deposit
    };

    // 2. Store in the gateway database. If persistence fails after the daemon
    // deposit was created, compensate by deleting that deposit so the sidecar
    // and daemon stay coherent.
    let payment_id = Uuid::new_v4().to_string();
    let expires_at = Utc::now() + chrono::Duration::minutes(state.config.payment_expiry_minutes);
    let expires_str = expires_at.format("%Y-%m-%d %H:%M:%S").to_string();
    let metadata_str = serde_json::to_string(&body.metadata).unwrap_or_default();

    let insert_result = db::insert_payment(
        &state.db,
        &payment_id,
        &project.id,
        body.idempotency_key.as_deref(),
        &body.amount_wei,
        resolved_chain_id as i64,
        deposit.token_address.as_deref(),
        &deposit.stealth_address,
        &deposit.ephemeral_public_key_hex,
        Some(&deposit.view_tag_hex),
        Some(&deposit.id),
        &metadata_str,
        Some(&expires_str),
    )
    .await;

    if let Err(error) = insert_result {
        rollback_created_deposit(&state, &deposit.id, &error).await;

        if let Some(idempotency_key) = body.idempotency_key.as_deref() {
            if is_unique_constraint(&error) {
                if let Some(existing) =
                    db::find_payment_by_idempotency_key(&state.db, &project.id, idempotency_key)
                        .await?
                {
                    if !idempotency_matches(&existing, &body) {
                        return Err(GatewayError::Conflict(
                            "idempotency_key was reused with different payment parameters".into(),
                        ));
                    }
                    return Ok(Json(existing_payment_response(&existing)));
                }
            }
        }

        return Err(error);
    }

    Ok(Json(created_payment_response(
        &payment_id,
        &body.amount_wei,
        resolved_chain_id,
        &deposit,
        &expires_str,
    )))
}

/// `GET /api/v1/payments/:id` — get payment status.
pub async fn get_payment(
    State(state): State<AppState>,
    axum::Extension(project): axum::Extension<Project>,
    Path(id): Path<String>,
) -> Result<Json<Value>, GatewayError> {
    let payment = db::find_payment_by_id(&state.db, &id)
        .await?
        .ok_or_else(|| GatewayError::NotFound(format!("payment {id} not found")))?;

    if payment.project_id != project.id {
        return Err(GatewayError::NotFound(format!("payment {id} not found")));
    }

    Ok(Json(json!({
        "payment_id": payment.id,
        "project_id": payment.project_id,
        "stealth_address": payment.stealth_address,
        "amount_wei": payment.amount_wei,
        "chain_id": payment.chain_id,
        "token_address": payment.token_address,
        "status": payment.status,
        "metadata": serde_json::from_str::<Value>(&payment.metadata_json).unwrap_or(json!({})),
        "created_at": payment.created_at,
        "expires_at": payment.expires_at,
        "confirmed_at": payment.confirmed_at,
        "swept_at": payment.swept_at,
    })))
}

/// `GET /api/v1/payments` — list payments for the authenticated project.
pub async fn list_payments(
    State(state): State<AppState>,
    axum::Extension(project): axum::Extension<Project>,
    Query(params): Query<ListPaymentsQuery>,
) -> Result<Json<Value>, GatewayError> {
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0);

    let payments = db::list_payments_by_project(
        &state.db,
        &project.id,
        params.status.as_deref(),
        limit,
        offset,
    )
    .await?;

    let items: Vec<Value> = payments
        .iter()
        .map(|p| {
            json!({
                "payment_id": p.id,
                "stealth_address": p.stealth_address,
                "amount_wei": p.amount_wei,
                "chain_id": p.chain_id,
                "token_address": p.token_address,
                "status": p.status,
                "created_at": p.created_at,
                "confirmed_at": p.confirmed_at,
                "swept_at": p.swept_at,
            })
        })
        .collect();

    Ok(Json(json!({
        "payments": items,
        "count": items.len(),
        "limit": limit,
        "offset": offset,
    })))
}

/// `POST /api/v1/payments/:id/cancel` — cancel a pending payment.
pub async fn cancel_payment(
    State(state): State<AppState>,
    axum::Extension(project): axum::Extension<Project>,
    Path(id): Path<String>,
) -> Result<Json<Value>, GatewayError> {
    let payment = db::find_payment_by_id(&state.db, &id)
        .await?
        .ok_or_else(|| GatewayError::NotFound(format!("payment {id} not found")))?;

    if payment.project_id != project.id {
        return Err(GatewayError::NotFound(format!("payment {id} not found")));
    }

    if payment.status != "pending" {
        return Err(GatewayError::BadRequest(format!(
            "cannot cancel payment in '{}' status",
            payment.status
        )));
    }

    db::update_payment_status(&state.db, &id, "cancelled").await?;

    Ok(Json(json!({
        "payment_id": id,
        "status": "cancelled",
    })))
}
