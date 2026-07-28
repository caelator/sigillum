//! Queue request contracts.

use serde::{Deserialize, Serialize};

use super::{
    EthStealthSendErc20WithProfileRequest, EthStealthSendWithProfileRequest, StealthPaymentRef,
};

/// Enqueue a native transfer (reuses the profile-based send structure).
pub type QueueEthStealthTransferRequest = EthStealthSendWithProfileRequest;

/// Enqueue an ERC-20 transfer (reuses the profile-based send structure).
pub type QueueEthStealthErc20TransferRequest = EthStealthSendErc20WithProfileRequest;

/// Enqueue a native ETH sweep from a stealth address.
///
/// A sweep sends the entire balance (minus gas) to a destination address.
/// `min_value_wei_hex` sets a dust threshold — addresses below this amount
/// are skipped during batch sweeps.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueEthStealthNativeSweepRequest {
    pub wallet_profile: String,
    #[serde(flatten)]
    pub stealth: StealthPaymentRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_value_wei_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,
}

/// Enqueue an ERC-20 token sweep from a stealth address.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueEthStealthErc20SweepRequest {
    pub wallet_profile: String,
    #[serde(flatten)]
    pub stealth: StealthPaymentRef,
    pub token_address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_amount_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,
}

/// Process queued jobs. When `id` is set, only that job is processed.
/// Otherwise, up to `limit` pending jobs are processed in FIFO order.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueProcessRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}
