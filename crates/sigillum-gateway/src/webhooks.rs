//! Webhook dispatcher — sends project-scoped HMAC-SHA256 webhooks with retry support.

use chrono::Utc;
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;

use crate::db;
use crate::db::NewWebhookDelivery;
use crate::db::row::{Payment, Project};
use crate::state::AppState;
use crate::validate;

type HmacSha256 = Hmac<Sha256>;

/// Maximum retry attempts per webhook delivery.
const MAX_RETRIES: i32 = 3;
const WEBHOOK_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const WEBHOOK_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Debug, Clone)]
struct OutboundWebhookRequest {
    url: String,
    payload: String,
    headers: Vec<(String, String)>,
}

/// Compute an HMAC-SHA256 signature for a webhook payload.
pub fn sign_payload(secret: &str, payload: &str) -> String {
    // Infallible: HMAC-SHA256 accepts keys of any length (RFC 2104), so new_from_slice cannot fail.
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn build_project_webhook_request(
    project: &Project,
    payment: &Payment,
    event: &str,
) -> Option<OutboundWebhookRequest> {
    let url = match &project.webhook_url {
        Some(url) if !url.is_empty() => url.clone(),
        _ => return None,
    };

    let secret = project.webhook_secret.as_deref().unwrap_or("");
    let payload = json!({
        "event": event,
        "payment_id": payment.id,
        "project_id": payment.project_id,
        "stealth_address": payment.stealth_address,
        "amount_wei": payment.amount_wei,
        "chain_id": payment.chain_id,
        "token_address": payment.token_address,
        "status": payment.status,
        "latest_balance_observation_at": payment.latest_balance_observation_at,
        "swept_at": payment.swept_at,
    });
    let payload_str = serde_json::to_string(&payload).unwrap_or_default();
    let signature = sign_payload(secret, &payload_str);

    Some(OutboundWebhookRequest {
        url,
        payload: payload_str,
        headers: vec![
            ("X-Sigillum-Signature".into(), signature),
            ("X-Sigillum-Event".into(), event.to_string()),
        ],
    })
}

/// Deliver webhook events for a payment state transition.
pub async fn deliver(state: &AppState, payment: &Payment, event: &str) {
    if !state.config.experimental_payments_enabled {
        return;
    }

    let project = match db::find_project_by_id(&state.db, &payment.project_id).await {
        Ok(Some(project)) => Some(project),
        _ => None,
    };

    if let Some(project) = project.as_ref() {
        if let Some(request) = build_project_webhook_request(project, payment, event) {
            send_webhook(state, payment, event, &request, 1).await;
        }
    }
}

/// Retry all pending webhook deliveries whose next_retry_at has passed.
pub async fn retry_pending(state: &AppState) {
    if !state.config.experimental_payments_enabled {
        return;
    }

    let pending = match db::list_pending_webhook_retries(&state.db).await {
        Ok(pending) => pending,
        Err(error) => {
            tracing::error!("Failed to load pending webhook retries: {error}");
            return;
        }
    };

    for delivery in pending {
        if delivery.attempt >= MAX_RETRIES {
            tracing::warn!(
                "Webhook delivery {} exhausted retries (payment {})",
                delivery.id,
                delivery.payment_id
            );
            let _ = db::clear_webhook_retry(&state.db, delivery.id).await;
            continue;
        }

        let payment = match db::find_payment_by_id(&state.db, &delivery.payment_id).await {
            Ok(Some(payment)) => payment,
            _ => continue,
        };

        let project = match db::find_project_by_id(&state.db, &payment.project_id).await {
            Ok(Some(project)) => project,
            _ => continue,
        };
        let request = build_project_webhook_request(&project, &payment, &delivery.event);

        let Some(request) = request else {
            let _ = db::clear_webhook_retry(&state.db, delivery.id).await;
            continue;
        };

        let _ = db::clear_webhook_retry(&state.db, delivery.id).await;
        send_webhook(
            state,
            &payment,
            &delivery.event,
            &request,
            delivery.attempt + 1,
        )
        .await;
    }
}

async fn send_webhook(
    state: &AppState,
    payment: &Payment,
    event: &str,
    request: &OutboundWebhookRequest,
    attempt: i32,
) {
    let resolved_target = match validate::resolve_webhook_target(&request.url) {
        Ok(target) => target,
        Err(error) => {
            tracing::warn!(
                "Webhook delivery rejected before send: {event} → {}: {error} [attempt {attempt}/{MAX_RETRIES}]",
                request.url
            );
            let _ = db::insert_webhook_delivery(
                &state.db,
                NewWebhookDelivery {
                    payment_id: &payment.id,
                    event,
                    url: &request.url,
                    attempt,
                    status_code: None,
                    response_body: Some(&error),
                    next_retry_at: None,
                },
            )
            .await;
            return;
        }
    };

    let mut client_builder = reqwest::Client::builder()
        .connect_timeout(WEBHOOK_CONNECT_TIMEOUT)
        .timeout(WEBHOOK_REQUEST_TIMEOUT);
    if let Some(dns_name) = resolved_target.dns_name.as_deref() {
        client_builder = client_builder.resolve_to_addrs(dns_name, &resolved_target.addrs);
    }
    let client = match client_builder.build() {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(
                "Failed to build pinned webhook client for {}: {error}",
                request.url
            );
            return;
        }
    };

    let mut builder = client
        .post(resolved_target.url)
        .header("Content-Type", "application/json")
        .body(request.payload.clone());

    for (name, value) in &request.headers {
        builder = builder.header(name, value);
    }

    let result = builder.send().await;

    match result {
        Ok(resp) => {
            let status_code = resp.status().as_u16() as i32;
            let body = resp.text().await.unwrap_or_default();
            let success = (200..300).contains(&(status_code as u16));

            let next_retry = if success || attempt >= MAX_RETRIES {
                None
            } else {
                Some(retry_at(attempt))
            };

            if let Err(error) = db::insert_webhook_delivery(
                &state.db,
                NewWebhookDelivery {
                    payment_id: &payment.id,
                    event,
                    url: &request.url,
                    attempt,
                    status_code: Some(status_code),
                    response_body: Some(&body),
                    next_retry_at: next_retry.as_deref(),
                },
            )
            .await
            {
                tracing::error!("Failed to log webhook delivery: {error}");
            }

            if success {
                tracing::info!(
                    "Webhook delivered: {event} → {} ({}) [attempt {attempt}]",
                    request.url,
                    status_code
                );
            } else {
                tracing::warn!(
                    "Webhook failed: {event} → {} ({}) [attempt {attempt}/{MAX_RETRIES}]",
                    request.url,
                    status_code
                );
            }
        }
        Err(error) => {
            tracing::error!(
                "Webhook request failed: {event} → {}: {error} [attempt {attempt}/{MAX_RETRIES}]",
                request.url
            );
            let next_retry = if attempt >= MAX_RETRIES {
                None
            } else {
                Some(retry_at(attempt))
            };
            let error_text = error.to_string();
            let _ = db::insert_webhook_delivery(
                &state.db,
                NewWebhookDelivery {
                    payment_id: &payment.id,
                    event,
                    url: &request.url,
                    attempt,
                    status_code: None,
                    response_body: Some(&error_text),
                    next_retry_at: next_retry.as_deref(),
                },
            )
            .await;
        }
    }
}

/// Calculate retry timestamp using exponential backoff.
fn retry_at(attempt: i32) -> String {
    let delay_secs: i64 = match attempt {
        1 => 10,
        2 => 60,
        _ => 300,
    };
    let next = Utc::now() + chrono::Duration::seconds(delay_secs);
    next.format("%Y-%m-%d %H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn privileged_invoice_signing_path_is_absent() {
        let source = include_str!("webhooks.rs").to_ascii_lowercase();
        assert!(!source.contains(&["be", "satas"].concat()));
        assert!(!source.contains(&["ed", "25519"].concat()));
        assert!(!source.contains(&["x-", "sat", "as-", "signature"].concat()));
    }
}
