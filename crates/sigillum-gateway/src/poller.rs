//! Experimental background poller for latest deposit balance observations,
//! sweep progress, webhook delivery, and daemon health.

use sigillum_api::request::EthStealthDepositRefreshRequest;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::time;

use crate::db;
use crate::state::AppState;
use crate::webhooks;

/// Maximum consecutive failures before marking daemon unhealthy.
const MAX_CONSECUTIVE_FAILURES: u32 = 5;

fn canonical_quantity(value: &str) -> Option<String> {
    let raw = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let significant = raw.trim_start_matches('0');
    Some(if significant.is_empty() {
        "0".into()
    } else {
        significant.to_ascii_lowercase()
    })
}

fn observed_amount_meets_payment(observed: Option<&str>, expected: &str) -> bool {
    let (Some(observed), Some(expected)) = (
        observed.and_then(canonical_quantity),
        canonical_quantity(expected),
    ) else {
        return false;
    };
    if expected == "0" {
        return false;
    }

    observed.len() > expected.len()
        || (observed.len() == expected.len() && observed.as_bytes() >= expected.as_bytes())
}

/// Spawn the background polling loop.
pub fn spawn(state: AppState) {
    if !state.config.experimental_payments_enabled {
        tracing::warn!(
            "Refusing to start payment observation poller while experimental payments are disabled"
        );
        return;
    }
    let interval = Duration::from_secs(state.config.poll_interval_secs);
    tokio::spawn(async move {
        let mut ticker = time::interval(interval);
        loop {
            ticker.tick().await;
            if let Err(e) = poll_cycle(&state).await {
                let failures = state.poll_failures.fetch_add(1, Ordering::Relaxed) + 1;
                tracing::error!("Poll cycle error ({failures} consecutive): {e}");
                if failures >= MAX_CONSECUTIVE_FAILURES {
                    state.daemon_healthy.store(false, Ordering::Relaxed);
                    tracing::warn!("Daemon marked UNHEALTHY after {failures} consecutive failures");
                }
            } else {
                // Reset on success
                let prev = state.poll_failures.swap(0, Ordering::Relaxed);
                if prev >= MAX_CONSECUTIVE_FAILURES {
                    state.daemon_healthy.store(true, Ordering::Relaxed);
                    tracing::info!("Daemon recovered — marked healthy");
                }
            }
        }
    });
}

async fn poll_cycle(state: &AppState) -> Result<(), Box<dyn std::error::Error>> {
    if !state.config.experimental_payments_enabled {
        return Ok(());
    }

    // 1. Expire stale payments
    let expired = db::expire_old_payments(&state.db).await?;
    if expired > 0 {
        tracing::info!("Expired {expired} stale payment(s)");
    }

    // 2. Refresh deposit statuses via daemon (S8: don't swallow errors)
    if let Err(e) = state
        .daemon
        .refresh_eth_stealth_deposits(EthStealthDepositRefreshRequest {
            id: None,
            limit: Some(100),
            auto_enqueue: Some(true),
        })
        .await
    {
        tracing::warn!("Deposit refresh failed: {e}");
        // Continue — we can still check cached deposit state
    }

    // 3. Check pending payments against daemon deposits
    let pending = db::list_pending_payments(&state.db).await?;
    if !pending.is_empty() {
        let deposits = state.daemon.list_eth_stealth_deposits().await?;

        for mut payment in pending {
            let deposit_id = match &payment.deposit_id {
                Some(id) => id.clone(),
                None => continue,
            };

            let deposit = match deposits.iter().find(|d| d.id == deposit_id) {
                Some(d) => d,
                None => continue,
            };

            // Defense in depth: never trust a lifecycle label alone. Older or
            // mixed-version daemons may report `funded` without enforcing the
            // payment's expected amount, so the gateway independently compares
            // the full 256-bit quantities and fails closed on malformed values.
            let amount_satisfied = observed_amount_meets_payment(
                deposit.observed_amount_hex.as_deref(),
                &payment.amount_wei,
            );

            // A balance observation is not chain-finality evidence. The
            // experimental gateway therefore reports only the latest balance
            // observation for incoming funds.
            let new_status = match deposit.status.as_str() {
                "funded" if payment.status == "pending" && amount_satisfied => Some("observed"),
                "sweep_queued" | "sweeping"
                    if matches!(payment.status.as_str(), "pending" | "observed")
                        && amount_satisfied =>
                {
                    Some("sweeping")
                }
                "swept" if payment.status != "swept" && amount_satisfied => Some("swept"),
                _ => None,
            };

            if let Some(status) = new_status {
                tracing::info!(
                    "Payment {} transitioning: {} → {status}",
                    payment.id,
                    payment.status
                );
                db::update_payment_status(&state.db, &payment.id, status).await?;
                let Some(updated_payment) = db::find_payment_by_id(&state.db, &payment.id).await?
                else {
                    continue;
                };
                payment = updated_payment;

                // Fire webhook
                let event = format!("payment.{status}");
                webhooks::deliver(state, &payment, &event).await;
            }
        }
    }

    // 4. S9: Retry failed webhook deliveries
    webhooks::retry_pending(state).await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::observed_amount_meets_payment;

    #[test]
    fn payment_amount_comparison_rejects_dust_and_malformed_observations() {
        assert!(!observed_amount_meets_payment(Some("0x1"), "0x100"));
        assert!(!observed_amount_meets_payment(None, "0x100"));
        assert!(!observed_amount_meets_payment(Some("not-hex"), "0x100"));
        assert!(!observed_amount_meets_payment(Some("0x100"), "not-hex"));
        assert!(!observed_amount_meets_payment(Some("0x0"), "0x0"));
    }

    #[test]
    fn payment_amount_comparison_accepts_equal_and_overpayment() {
        assert!(observed_amount_meets_payment(Some("0x0100"), "0x100"));
        assert!(observed_amount_meets_payment(Some("0x101"), "0x100"));
        assert!(observed_amount_meets_payment(Some("0xABCD"), "0xabcc"));
    }
}
