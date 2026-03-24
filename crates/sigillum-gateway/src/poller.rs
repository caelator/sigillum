//! Background poller — periodically checks for deposit confirmations, triggers sweeps,
//! retries failed webhooks, and tracks daemon health.

use sigillum_api::request::EthStealthDepositRefreshRequest;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::time;

use crate::db;
use crate::state::AppState;
use crate::webhooks;

/// Maximum consecutive failures before marking daemon unhealthy.
const MAX_CONSECUTIVE_FAILURES: u32 = 5;

/// Spawn the background polling loop.
pub fn spawn(state: AppState) {
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

            // Map daemon deposit status to gateway payment status
            let new_status = match deposit.status.as_str() {
                "confirmed" | "funded" if payment.status == "pending" => Some("confirmed"),
                "sweep_queued" | "sweeping" if payment.status == "confirmed" => Some("sweeping"),
                "swept" if payment.status != "swept" => Some("swept"),
                _ => None,
            };

            if let Some(status) = new_status {
                tracing::info!(
                    "Payment {} transitioning: {} → {status}",
                    payment.id,
                    payment.status
                );
                db::update_payment_status(&state.db, &payment.id, status).await?;
                payment.status = status.to_string();

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
