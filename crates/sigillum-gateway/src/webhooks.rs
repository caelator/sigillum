//! Webhook dispatcher — sends HMAC-SHA256 signed merchant webhooks and
//! Ed25519-signed besatas payment confirmations with retry support.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{NaiveDateTime, Utc};
use ed25519_dalek::{Signer, SigningKey, pkcs8::DecodePrivateKey};
use hmac::{Hmac, Mac};
use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::GatewayConfig;
use crate::db;
use crate::db::NewWebhookDelivery;
use crate::db::row::{Payment, Project};
use crate::state::AppState;
use crate::validate;

type HmacSha256 = Hmac<Sha256>;

/// Maximum retry attempts per webhook delivery.
const MAX_RETRIES: i32 = 3;
const BESATAS_PAYMENT_EVENT: &str = "payment.confirmed";
const WEBHOOK_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const WEBHOOK_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Debug, Clone)]
struct OutboundWebhookRequest {
    url: String,
    payload: String,
    headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BesatasPaymentConfirmationPayload {
    event: &'static str,
    invoice_id: String,
    external_invoice_id: String,
    payment_id: String,
    amount_usd: f64,
    currency: String,
    paid_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<Map<String, Value>>,
}

/// Compute an HMAC-SHA256 signature for a webhook payload.
pub fn sign_payload(secret: &str, payload: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn normalize_pem(value: &str) -> String {
    value.replace("\\n", "\n").trim().to_string()
}

fn deterministic_delivery_id(payment_id: &str, event: &str) -> String {
    let digest = Sha256::digest(format!("satas-besatas:{event}:{payment_id}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

fn sign_besatas_payload(
    private_key_pem: &str,
    timestamp: i64,
    payload: &str,
) -> Result<String, String> {
    let signing_key = SigningKey::from_pkcs8_pem(&normalize_pem(private_key_pem))
        .map_err(|error| format!("invalid besatas webhook private key: {error}"))?;
    let signature = signing_key.sign(format!("{timestamp}.{payload}").as_bytes());
    Ok(format!(
        "v1={}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    ))
}

fn take_required_string(map: &mut Map<String, Value>, key: &str) -> Result<String, String> {
    match map.remove(key) {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value),
        Some(_) => Err(format!("metadata.besatas.{key} must be a non-empty string")),
        None => Err(format!("metadata.besatas.{key} is required")),
    }
}

fn take_required_f64(map: &mut Map<String, Value>, key: &str) -> Result<f64, String> {
    let value = map
        .remove(key)
        .ok_or_else(|| format!("metadata.besatas.{key} is required"))?;
    let amount = match value {
        Value::Number(number) => number
            .as_f64()
            .ok_or_else(|| format!("metadata.besatas.{key} must be a finite number"))?,
        Value::String(text) => text
            .parse::<f64>()
            .map_err(|_| format!("metadata.besatas.{key} must be a valid number string"))?,
        _ => return Err(format!("metadata.besatas.{key} must be numeric")),
    };

    if amount <= 0.0 {
        return Err(format!("metadata.besatas.{key} must be greater than zero"));
    }

    Ok(amount)
}

fn take_currency(map: &mut Map<String, Value>) -> Result<String, String> {
    match map.remove("currency") {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value.trim().to_uppercase()),
        Some(_) => Err("metadata.besatas.currency must be a string".into()),
        None => Ok("USD".into()),
    }
}

fn take_metadata_map(
    besatas: &mut Map<String, Value>,
) -> Result<Option<Map<String, Value>>, String> {
    let mut metadata = match besatas.remove("metadata") {
        Some(Value::Object(map)) => map,
        Some(_) => return Err("metadata.besatas.metadata must be an object".into()),
        None => Map::new(),
    };

    for (key, value) in std::mem::take(besatas) {
        metadata.entry(key).or_insert(value);
    }

    if metadata.is_empty() {
        Ok(None)
    } else {
        Ok(Some(metadata))
    }
}

fn format_paid_at(timestamp: &str) -> Result<String, String> {
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(timestamp) {
        return Ok(parsed.with_timezone(&Utc).to_rfc3339());
    }

    let naive = NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S")
        .map_err(|error| format!("invalid payment confirmation timestamp: {error}"))?;
    Ok(naive.and_utc().to_rfc3339())
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
        "confirmed_at": payment.confirmed_at,
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

fn build_besatas_payload(
    payment: &Payment,
    event: &str,
) -> Result<Option<BesatasPaymentConfirmationPayload>, String> {
    if event != BESATAS_PAYMENT_EVENT {
        return Ok(None);
    }

    let metadata_value: Value = serde_json::from_str(&payment.metadata_json)
        .map_err(|error| format!("payment metadata is not valid JSON: {error}"))?;
    let mut metadata = match metadata_value {
        Value::Object(map) => map,
        Value::Null => return Ok(None),
        _ => return Err("payment metadata must be a JSON object".into()),
    };

    let Some(besatas_value) = metadata.remove("besatas") else {
        return Ok(None);
    };
    let mut besatas = match besatas_value {
        Value::Object(map) => map,
        _ => return Err("metadata.besatas must be an object".into()),
    };

    let invoice_id = take_required_string(&mut besatas, "invoiceId")?;
    let external_invoice_id = take_required_string(&mut besatas, "externalInvoiceId")?;
    let amount_usd = take_required_f64(&mut besatas, "amountUsd")?;
    let currency = take_currency(&mut besatas)?;
    let paid_at = format_paid_at(
        payment
            .confirmed_at
            .as_deref()
            .ok_or_else(|| "payment.confirmed requires confirmed_at".to_string())?,
    )?;
    let metadata = take_metadata_map(&mut besatas)?;

    Ok(Some(BesatasPaymentConfirmationPayload {
        event: BESATAS_PAYMENT_EVENT,
        invoice_id,
        external_invoice_id,
        payment_id: payment.id.clone(),
        amount_usd,
        currency,
        paid_at,
        metadata,
    }))
}

fn build_besatas_webhook_request(
    config: &GatewayConfig,
    payment: &Payment,
    event: &str,
) -> Result<Option<OutboundWebhookRequest>, String> {
    let Some(url) = config.besatas_webhook_url.as_deref() else {
        return Ok(None);
    };
    let Some(private_key) = config.besatas_webhook_private_key.as_deref() else {
        return Ok(None);
    };
    let Some(payload) = build_besatas_payload(payment, event)? else {
        return Ok(None);
    };

    let payload_str = serde_json::to_string(&payload)
        .map_err(|error| format!("failed to serialize besatas webhook payload: {error}"))?;
    let delivery_id = deterministic_delivery_id(&payment.id, event);
    let timestamp = Utc::now().timestamp();
    let signature = sign_besatas_payload(private_key, timestamp, &payload_str)?;

    Ok(Some(OutboundWebhookRequest {
        url: url.to_string(),
        payload: payload_str,
        headers: vec![
            ("x-satas-delivery-id".into(), delivery_id),
            ("x-satas-timestamp".into(), timestamp.to_string()),
            ("x-satas-signature".into(), signature),
        ],
    }))
}

fn is_besatas_delivery(config: &GatewayConfig, url: &str, event: &str) -> bool {
    config.besatas_webhook_url.as_deref() == Some(url) && event == BESATAS_PAYMENT_EVENT
}

/// Deliver webhook events for a payment state transition.
pub async fn deliver(state: &AppState, payment: &Payment, event: &str) {
    let project = match db::find_project_by_id(&state.db, &payment.project_id).await {
        Ok(Some(project)) => Some(project),
        _ => None,
    };

    if let Some(project) = project.as_ref() {
        if let Some(request) = build_project_webhook_request(project, payment, event) {
            send_webhook(state, payment, event, &request, 1).await;
        }
    }

    match build_besatas_webhook_request(&state.config, payment, event) {
        Ok(Some(request)) => send_webhook(state, payment, event, &request, 1).await,
        Ok(None) => {}
        Err(error) => {
            tracing::error!(
                payment_id = %payment.id,
                event,
                "Failed to build besatas webhook: {error}"
            );
        }
    }
}

/// Retry all pending webhook deliveries whose next_retry_at has passed.
pub async fn retry_pending(state: &AppState) {
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

        let request = if is_besatas_delivery(&state.config, &delivery.url, &delivery.event) {
            match build_besatas_webhook_request(&state.config, &payment, &delivery.event) {
                Ok(Some(request)) => Some(request),
                Ok(None) => None,
                Err(error) => {
                    tracing::error!(
                        payment_id = %payment.id,
                        event = %delivery.event,
                        "Failed to rebuild besatas webhook retry: {error}"
                    );
                    None
                }
            }
        } else {
            let project = match db::find_project_by_id(&state.db, &payment.project_id).await {
                Ok(Some(project)) => project,
                _ => continue,
            };
            build_project_webhook_request(&project, &payment, &delivery.event)
        };

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
    use super::*;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    use serde_json::json;

    const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIE7qITftonGwYykfPTLxyqDrc1vElPbMKVKqbT1SRNit\n-----END PRIVATE KEY-----";

    fn test_config() -> GatewayConfig {
        GatewayConfig {
            daemon_url: "http://127.0.0.1:9743".into(),
            daemon_session_token: None,
            database_url: "sqlite::memory:".into(),
            bind_addr: "127.0.0.1:8443".parse().expect("bind addr"),
            poll_interval_secs: 30,
            payment_expiry_minutes: 60,
            admin_key_hash: None,
            cors_origins: Vec::new(),
            rate_limit_rps: 0,
            auth_cache_ttl_secs: 30,
            log_json: false,
            besatas_webhook_url: Some(
                "https://besatas.example/api/integrations/satas/payment-confirmed".into(),
            ),
            besatas_webhook_private_key: Some(TEST_PRIVATE_KEY_PEM.into()),
        }
    }

    fn test_payment(metadata_json: Value) -> Payment {
        Payment {
            id: "payment-123".into(),
            project_id: "project-123".into(),
            idempotency_key: Some("idem-123".into()),
            amount_wei: "0x2386F26FC10000".into(),
            chain_id: 1,
            token_address: None,
            stealth_address: "st:address:stub".into(),
            ephemeral_pub: "33".repeat(32),
            view_tag: Some("aa".into()),
            deposit_id: Some("deposit-123".into()),
            status: "confirmed".into(),
            metadata_json: metadata_json.to_string(),
            created_at: "2026-03-22 10:00:00".into(),
            expires_at: Some("2026-03-22 11:00:00".into()),
            confirmed_at: Some("2026-03-22 10:15:30".into()),
            swept_at: None,
        }
    }

    fn header<'a>(headers: &'a [(String, String)], name: &str) -> &'a str {
        headers
            .iter()
            .find(|(header_name, _)| header_name == name)
            .map(|(_, value)| value.as_str())
            .expect("header should exist")
    }

    fn verifying_key() -> VerifyingKey {
        SigningKey::from_pkcs8_pem(TEST_PRIVATE_KEY_PEM)
            .expect("test private key should parse")
            .verifying_key()
    }

    #[test]
    fn besatas_webhook_request_matches_receiver_contract() {
        let config = test_config();
        let payment = test_payment(json!({
            "order_id": "local-only",
            "besatas": {
                "invoiceId": "invoice-123",
                "externalInvoiceId": "sat-456",
                "amountUsd": "149.50",
                "currency": "usd",
                "doctorId": "doctor-42",
                "affiliateId": "affiliate-7"
            }
        }));

        let request = build_besatas_webhook_request(&config, &payment, BESATAS_PAYMENT_EVENT)
            .expect("request should build")
            .expect("request should exist");

        assert_eq!(
            request.url,
            "https://besatas.example/api/integrations/satas/payment-confirmed"
        );

        let payload: Value = serde_json::from_str(&request.payload).expect("payload should parse");
        assert_eq!(payload["event"], "payment.confirmed");
        assert_eq!(payload["invoiceId"], "invoice-123");
        assert_eq!(payload["externalInvoiceId"], "sat-456");
        assert_eq!(payload["paymentId"], "payment-123");
        assert_eq!(payload["amountUsd"], 149.5);
        assert_eq!(payload["currency"], "USD");
        assert_eq!(payload["paidAt"], "2026-03-22T10:15:30+00:00");
        assert_eq!(payload["metadata"]["doctorId"], "doctor-42");
        assert_eq!(payload["metadata"]["affiliateId"], "affiliate-7");
        assert!(payload.get("order_id").is_none());

        let delivery_id = header(&request.headers, "x-satas-delivery-id");
        assert_eq!(
            delivery_id,
            deterministic_delivery_id("payment-123", "payment.confirmed")
        );
        assert!(Uuid::parse_str(delivery_id).is_ok());

        let timestamp = header(&request.headers, "x-satas-timestamp");
        let signature_header = header(&request.headers, "x-satas-signature");
        let encoded_signature = signature_header
            .strip_prefix("v1=")
            .expect("signature header should use v1");
        let signature_bytes = URL_SAFE_NO_PAD
            .decode(encoded_signature)
            .expect("signature should decode");
        let signature = Signature::from_bytes(
            &signature_bytes
                .try_into()
                .expect("signature should be 64 bytes"),
        );

        verifying_key()
            .verify(
                format!("{timestamp}.{}", request.payload).as_bytes(),
                &signature,
            )
            .expect("signature should verify");
    }

    #[test]
    fn besatas_webhook_request_skips_irrelevant_payments() {
        let config = test_config();
        let payment = test_payment(json!({
            "metadata": {
                "unrelated": true
            }
        }));

        assert!(
            build_besatas_webhook_request(&config, &payment, BESATAS_PAYMENT_EVENT)
                .expect("metadata without besatas contract should be ignored")
                .is_none()
        );
        assert!(
            build_besatas_webhook_request(&config, &payment, "payment.swept")
                .expect("non-confirmed events should be ignored")
                .is_none()
        );
    }
}
