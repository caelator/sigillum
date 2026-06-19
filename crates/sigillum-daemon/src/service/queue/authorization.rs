use sigillum_api::QueueJobPayload;

use crate::service::transaction_policy::{TransactionPolicyCheck, TransactionPolicyKind};
use crate::service::{ServiceError, ServiceResult, SigillumService};

pub(super) fn require_queue_execution_enabled(
    service: &SigillumService,
    payload: &QueueJobPayload,
) -> ServiceResult<()> {
    let registry = crate::profiles::load_profiles(&service.state.base_dir).map_err(|error| {
        ServiceError::internal(format!("Failed to load profile registry: {error}"))
    })?;
    let (profile_name, family) = queue_payload_wallet_profile(payload);
    let enabled = match family {
        "eth-stealth" => registry
            .eth_stealth_wallets
            .iter()
            .find(|profile| profile.name == profile_name)
            .map(|profile| profile.execution_enabled),
        "eth-seed" => registry
            .eth_seed_wallets
            .iter()
            .find(|profile| profile.name == profile_name)
            .map(|profile| profile.execution_enabled),
        _ => None,
    }
    .ok_or_else(|| ServiceError::not_found("Wallet profile not found."))?;
    if !enabled {
        return Err(ServiceError::forbidden(
            "Wallet profile execution is disabled.",
        ));
    }
    Ok(())
}

pub(super) fn authorize_queue_payload_policy(
    service: &SigillumService,
    payload: &QueueJobPayload,
) -> ServiceResult<()> {
    if let Some((destination_address, asset_kind, amount_hex)) = queue_payload_policy_check(payload)
    {
        service.authorize_transaction_policy(TransactionPolicyCheck {
            kind: TransactionPolicyKind::RoutedTransfer,
            destination_address: Some(destination_address),
            asset_kind,
            amount_hex,
        })?;
    }
    Ok(())
}

fn queue_payload_wallet_profile(payload: &QueueJobPayload) -> (&str, &'static str) {
    match payload {
        QueueJobPayload::EthStealthTransfer { wallet_profile, .. }
        | QueueJobPayload::EthStealthErc20Transfer { wallet_profile, .. }
        | QueueJobPayload::EthStealthNativeSweep { wallet_profile, .. }
        | QueueJobPayload::EthStealthErc20Sweep { wallet_profile, .. } => {
            (wallet_profile.as_str(), "eth-stealth")
        }
        QueueJobPayload::EthSeedTransfer { wallet_profile, .. }
        | QueueJobPayload::EthSeedNativeSweep { wallet_profile, .. }
        | QueueJobPayload::EthSeedErc20Sweep { wallet_profile, .. } => {
            (wallet_profile.as_str(), "eth-seed")
        }
    }
}

fn queue_payload_policy_check(payload: &QueueJobPayload) -> Option<(&str, &str, &str)> {
    match payload {
        QueueJobPayload::EthStealthTransfer {
            destination_address,
            value_wei_hex,
            ..
        } => destination_address
            .as_deref()
            .map(|destination| (destination, "native", value_wei_hex.as_str())),
        QueueJobPayload::EthStealthErc20Transfer {
            recipient_address,
            amount_hex,
            ..
        } => Some((recipient_address.as_str(), "erc20", amount_hex.as_str())),
        QueueJobPayload::EthStealthNativeSweep {
            destination_address,
            min_value_wei_hex,
            ..
        } => destination_address.as_deref().map(|destination| {
            (
                destination,
                "native",
                min_value_wei_hex.as_deref().unwrap_or("0x0"),
            )
        }),
        QueueJobPayload::EthStealthErc20Sweep {
            recipient_address,
            min_amount_hex,
            ..
        } => recipient_address.as_deref().map(|destination| {
            (
                destination,
                "erc20",
                min_amount_hex.as_deref().unwrap_or("0x0"),
            )
        }),
        QueueJobPayload::EthSeedTransfer {
            destination_address,
            value_wei_hex,
            ..
        } => Some((
            destination_address.as_str(),
            "native",
            value_wei_hex.as_str(),
        )),
        QueueJobPayload::EthSeedNativeSweep {
            destination_address,
            min_value_wei_hex,
            ..
        } => destination_address.as_deref().map(|destination| {
            (
                destination,
                "native",
                min_value_wei_hex.as_deref().unwrap_or("0x0"),
            )
        }),
        QueueJobPayload::EthSeedErc20Sweep {
            recipient_address,
            min_amount_hex,
            ..
        } => recipient_address.as_deref().map(|destination| {
            (
                destination,
                "erc20",
                min_amount_hex.as_deref().unwrap_or("0x0"),
            )
        }),
    }
}
