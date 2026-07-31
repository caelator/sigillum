use std::sync::Arc;

use axum::http::StatusCode;
use sigillum_api::{
    EthStealthWalletProfile, QueueEthStealthTransferRequest, StealthPaymentRef, TreasuryPolicy,
};
use sigillum_fido2::config::CompartmentMeta;
use tempfile::TempDir;

use super::{SigillumService, payloads};
use crate::AppState;

const SIGNED_RAW_TRANSACTION: &str = "0x02deadbeef";

#[derive(Clone, Copy)]
enum AdmissionInvalidation {
    RevokeSession,
    BeginLocking,
    SwitchCompartment,
}

fn compartment(id: usize, label: &str) -> CompartmentMeta {
    CompartmentMeta {
        id,
        label: label.into(),
        threshold: 1,
        passphrase_mode: None,
    }
}

fn open_policy() -> TreasuryPolicy {
    TreasuryPolicy {
        enabled: true,
        allowed_destinations: Vec::new(),
        max_step_native_wei_hex: None,
        max_plan_native_wei_hex: None,
        require_simulation: true,
        allow_raw_digest_signing: false,
        block_cross_party_linkage: false,
        allow_claim_execution: false,
        allow_gas_topups: false,
        max_gas_topup_wei_hex: None,
        allow_plan_execution: true,
        allow_sweep_execution: true,
        allow_revoke_execution: false,
        allow_exit_execution: false,
        execution_paused: false,
        max_fee_per_gas_cap_hex: None,
        simulation_freshness_secs: 900,
        hot_floor_wei_hex: "0xde0b6b3a7640000".into(),
        hot_target_wei_hex: "0xde0b6b3a7640000".into(),
        hot_overflow_wei_hex: None,
        allow_treasury_automation: false,
        created_at_unix: 1,
        updated_at_unix: 1,
    }
}

fn transfer_request(value_wei_hex: &str) -> QueueEthStealthTransferRequest {
    QueueEthStealthTransferRequest {
        wallet_profile: "payments-mainnet".into(),
        stealth: StealthPaymentRef {
            stealth_address: "0x1111111111111111111111111111111111111111".into(),
            ephemeral_public_key_hex: "02".repeat(33),
            view_tag_hex: Some("01".into()),
            stealth_hash_convention: None,
        },
        value_wei_hex: value_wei_hex.into(),
        destination_address: None,
        nonce: Some(7),
        gas_limit: Some(21_000),
        estimate_fees: None,
        broadcast: None,
    }
}

fn test_service() -> (TempDir, Arc<AppState>, SigillumService, String) {
    let dir = TempDir::new().expect("temporary directory");
    let state = Arc::new(AppState::new(dir.path().to_path_buf()).expect("state should initialize"));
    state.unlock_compartment(0, [1u8; 32], compartment(0, "admitted"));
    state.unlock_compartment(1, [2u8; 32], compartment(1, "later"));
    let session = state.create_session(Some(0));

    crate::profiles::save_profiles(
        &state.base_dir,
        &crate::profiles::ProfileRegistry {
            eth_stealth_wallets: vec![EthStealthWalletProfile {
                name: "payments-mainnet".into(),
                wallet: "payments".into(),
                short_name: "eth".into(),
                provider_profile: "mainnet".into(),
                compartment_id: 0,
                chain_id: Some(1),
                default_destination_address: None,
                execution_enabled: true,
            }],
            ..Default::default()
        },
    )
    .expect("profile registry");
    let mut inventory =
        crate::inventory::load_wallet_inventory(&state.base_dir).expect("inventory");
    inventory.treasury_policy = Some(open_policy());
    crate::inventory::save_wallet_inventory(&state.base_dir, &inventory).expect("treasury policy");

    let service = SigillumService::new(state.clone());
    (dir, state, service, session)
}

fn seed_prepared_queue(state: &AppState) -> Vec<u8> {
    let mut job = payloads::queued_job(
        "prepared-existing".into(),
        1,
        payloads::eth_stealth_transfer_payload(transfer_request("0x1")),
    );
    job.state = "prepared".into();
    job.receipt.signed_raw_transaction_hex = Some(SIGNED_RAW_TRANSACTION.into());
    job.receipt.prepared_at_unix = Some(1);
    job.receipt.prepared_payload_hash_hex = Some(format!("0x{}", "11".repeat(32)));
    job.receipt.prepared_binding_hash_hex = Some(format!("0x{}", "22".repeat(32)));
    crate::queue_store::save_queue(
        &state.base_dir,
        &crate::queue_store::QueueState { jobs: vec![job] },
    )
    .expect("seed prepared queue");
    std::fs::read(crate::queue_store::queue_path(&state.base_dir)).expect("read seeded queue bytes")
}

async fn assert_waiting_enqueue_rejected(invalidation: AdmissionInvalidation) {
    let (_dir, state, service, session) = test_service();
    let queue_bytes_before = seed_prepared_queue(&state);
    let held_operation = state.operation_guard().await;
    let mut enqueue =
        Box::pin(service.enqueue_eth_stealth_transfer(Some(&session), transfer_request("0x2")));

    tokio::select! {
        biased;
        result = &mut enqueue => {
            panic!("enqueue must wait behind the held operation guard: {result:?}");
        }
        _ = tokio::task::yield_now() => {}
    }

    let (expected_status, expected_message) = match invalidation {
        AdmissionInvalidation::RevokeSession => {
            state.revoke_session(&session);
            (
                StatusCode::UNAUTHORIZED,
                "Invalid or missing session token.",
            )
        }
        AdmissionInvalidation::BeginLocking => {
            assert!(state.begin_locking(), "test must latch daemon locking");
            (StatusCode::LOCKED, "Daemon is locking.")
        }
        AdmissionInvalidation::SwitchCompartment => {
            state
                .switch_active_for(&session, 1)
                .expect("test compartment switch succeeds");
            (
                StatusCode::CONFLICT,
                "Session compartment changed while the operation was waiting.",
            )
        }
    };
    drop(held_operation);

    let error = enqueue.await.expect_err("invalidated enqueue must fail");
    if matches!(invalidation, AdmissionInvalidation::BeginLocking) {
        state.finish_locking();
    }
    assert_eq!(error.status(), expected_status);
    assert_eq!(error.message(), expected_message);

    let queue_path = crate::queue_store::queue_path(&state.base_dir);
    assert_eq!(
        std::fs::read(&queue_path).expect("read queue after rejection"),
        queue_bytes_before,
        "rejected enqueue must not rewrite the durable queue"
    );
    let queue = crate::queue_store::load_queue(&state.base_dir).expect("reload queue");
    assert_eq!(queue.jobs.len(), 1);
    assert_eq!(queue.jobs[0].id, "prepared-existing");
    assert_eq!(
        queue.jobs[0].receipt.signed_raw_transaction_hex.as_deref(),
        Some(SIGNED_RAW_TRANSACTION),
        "existing exact signed bytes must remain untouched"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn manual_enqueue_rejects_revoked_session_while_waiting() {
    assert_waiting_enqueue_rejected(AdmissionInvalidation::RevokeSession).await;
}

#[tokio::test(flavor = "current_thread")]
async fn manual_enqueue_rejects_lock_latch_while_waiting() {
    assert_waiting_enqueue_rejected(AdmissionInvalidation::BeginLocking).await;
}

#[tokio::test(flavor = "current_thread")]
async fn manual_enqueue_rejects_compartment_switch_while_waiting() {
    assert_waiting_enqueue_rejected(AdmissionInvalidation::SwitchCompartment).await;
}
