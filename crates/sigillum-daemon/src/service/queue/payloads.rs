//! Queue payload and job construction helpers.

use sigillum_api::{
    QueueEthStealthErc20SweepRequest, QueueEthStealthErc20TransferRequest,
    QueueEthStealthNativeSweepRequest, QueueEthStealthTransferRequest, QueueJob, QueueJobPayload,
};

use super::QUEUE_STATE_QUEUED;

pub(in crate::service) fn queued_job(id: String, now: u64, payload: QueueJobPayload) -> QueueJob {
    QueueJob {
        id,
        state: QUEUE_STATE_QUEUED.into(),
        attempts: 0,
        created_at_unix: now,
        updated_at_unix: now,
        next_attempt_after_unix: None,
        payload,
        last_error: None,
        transaction_hash_hex: None,
        broadcast_transaction_hash_hex: None,
        receipt: Default::default(),
    }
}

pub(super) fn eth_stealth_transfer_payload(
    body: QueueEthStealthTransferRequest,
) -> QueueJobPayload {
    QueueJobPayload::EthStealthTransfer {
        wallet_profile: body.wallet_profile,
        stealth_address: body.stealth.stealth_address,
        ephemeral_public_key_hex: body.stealth.ephemeral_public_key_hex,
        value_wei_hex: body.value_wei_hex,
        destination_address: body.destination_address,
        nonce: body.nonce,
        gas_limit: body.gas_limit,
        view_tag_hex: body.stealth.view_tag_hex,
        stealth_hash_convention: body.stealth.stealth_hash_convention,
    }
}

pub(super) fn eth_stealth_erc20_transfer_payload(
    body: QueueEthStealthErc20TransferRequest,
) -> QueueJobPayload {
    QueueJobPayload::EthStealthErc20Transfer {
        wallet_profile: body.wallet_profile,
        stealth_address: body.stealth.stealth_address,
        ephemeral_public_key_hex: body.stealth.ephemeral_public_key_hex,
        token_address: body.token_address,
        recipient_address: body.recipient_address,
        amount_hex: body.amount_hex,
        nonce: body.nonce,
        gas_limit: body.gas_limit,
        view_tag_hex: body.stealth.view_tag_hex,
        stealth_hash_convention: body.stealth.stealth_hash_convention,
    }
}

pub(super) fn eth_stealth_native_sweep_payload(
    body: QueueEthStealthNativeSweepRequest,
) -> QueueJobPayload {
    QueueJobPayload::EthStealthNativeSweep {
        wallet_profile: body.wallet_profile,
        stealth_address: body.stealth.stealth_address,
        ephemeral_public_key_hex: body.stealth.ephemeral_public_key_hex,
        destination_address: body.destination_address,
        min_value_wei_hex: body.min_value_wei_hex,
        gas_limit: body.gas_limit,
        view_tag_hex: body.stealth.view_tag_hex,
        stealth_hash_convention: body.stealth.stealth_hash_convention,
    }
}

pub(super) fn eth_stealth_erc20_sweep_payload(
    body: QueueEthStealthErc20SweepRequest,
) -> QueueJobPayload {
    QueueJobPayload::EthStealthErc20Sweep {
        wallet_profile: body.wallet_profile,
        stealth_address: body.stealth.stealth_address,
        ephemeral_public_key_hex: body.stealth.ephemeral_public_key_hex,
        token_address: body.token_address,
        recipient_address: body.recipient_address,
        min_amount_hex: body.min_amount_hex,
        gas_limit: body.gas_limit,
        view_tag_hex: body.stealth.view_tag_hex,
        stealth_hash_convention: body.stealth.stealth_hash_convention,
        // The public enqueue endpoint wires no dependencies; sponsor top-up
        // prerequisites are set only by the deposit sweep flow internally.
        prerequisite_job_ids: Vec::new(),
    }
}
