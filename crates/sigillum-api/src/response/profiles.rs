//! Wallet and provider profile response contracts.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmProviderProfile {
    pub name: String,
    pub rpc_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_token_key: Option<String>,
    #[serde(default)]
    pub compartment_id: usize,
    pub chain_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_priority_fee_per_gas_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_fee_per_gas_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_gas_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub erc20_gas_limit: Option<u64>,
    /// Opt-in: use live EIP-1559 fee estimation for plan gas verification
    /// instead of only the static profile fee fields. Default false.
    #[serde(default)]
    pub fee_estimation_enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmProviderProfileListResponse {
    pub profiles: Vec<EvmProviderProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmProviderProfileMutationResponse {
    pub status: String,
    pub profile: EvmProviderProfile,
    /// Present only when the delete requested `prune_inventory`: what the
    /// forget cascade removed (plan task 3.2). Absent otherwise, so legacy
    /// delete responses are byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pruned_inventory: Option<crate::response::InventoryPruneSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthWalletProfile {
    pub name: String,
    pub wallet: String,
    pub short_name: String,
    pub provider_profile: String,
    #[serde(default)]
    pub compartment_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_destination_address: Option<String>,
    #[serde(default)]
    pub execution_enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthWalletProfileListResponse {
    pub profiles: Vec<EthStealthWalletProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthStealthWalletProfileMutationResponse {
    pub status: String,
    pub profile: EthStealthWalletProfile,
    /// Present only when the delete requested `prune_inventory` (plan task
    /// 3.2). Stealth wallets have no wallet-inventory rows, so the summary
    /// reports zeros; stealth deposit monitors live in the separate deposits
    /// store with their own delete route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pruned_inventory: Option<crate::response::InventoryPruneSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthXpubWalletProfile {
    pub name: String,
    pub project_account: u32,
    pub provider_profile: String,
    #[serde(default)]
    pub compartment_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    /// Imported receive-branch xpub for external watch-only wallets.
    ///
    /// When omitted, the profile uses Sigillum's legacy project-derived xpub
    /// from the bound compartment master key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_receive_xpub: Option<String>,
    /// Operator-supplied BIP-32 path for an imported receive xpub.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_receive_path: Option<String>,
    /// Imported account-level xpub for external watch-only wallets.
    ///
    /// Sigillum normalizes this to a receive branch before discovery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_account_xpub: Option<String>,
    /// Operator-supplied BIP-32 path for an imported account xpub.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_account_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_destination_address: Option<String>,
    #[serde(default)]
    pub execution_enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthXpubWalletProfileListResponse {
    pub profiles: Vec<EthXpubWalletProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthXpubWalletProfileMutationResponse {
    pub status: String,
    pub profile: EthXpubWalletProfile,
    /// Present only when the delete requested `prune_inventory`: what the
    /// forget cascade removed (plan task 3.2). Absent otherwise, so legacy
    /// delete responses are byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pruned_inventory: Option<crate::response::InventoryPruneSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthSeedWalletProfile {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub project_account: u32,
    pub provider_profile: String,
    #[serde(default)]
    pub compartment_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    pub word_count: usize,
    pub mnemonic_secret_key: String,
    pub account_path: String,
    pub receive_path: String,
    pub receive_xpub: String,
    pub first_receive_address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_destination_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_xpub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sponsor_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hot_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub treasury_address: Option<String>,
    #[serde(default)]
    pub execution_enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthSeedWalletProfileListResponse {
    pub profiles: Vec<EthSeedWalletProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthSeedWalletProfileMutationResponse {
    pub status: String,
    pub profile: EthSeedWalletProfile,
    /// Present only when the delete requested `prune_inventory`: what the
    /// forget cascade removed (plan task 3.2). Absent otherwise, so legacy
    /// delete responses are byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pruned_inventory: Option<crate::response::InventoryPruneSummary>,
}

/// Response for a freshly created seed wallet profile.
///
/// `mnemonic` is the server-generated BIP-39 phrase and is returned **exactly
/// once** — in this response only — so the operator can back it up. The daemon
/// never persists the phrase in plaintext beyond the encrypted vault secret
/// referenced by `profile.mnemonic_secret_key` (the same vault-secret path the
/// import/upsert flow uses), and never writes any mnemonic material to the
/// audit log.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthSeedWalletCreateResponse {
    pub status: String,
    pub mnemonic: String,
    pub profile: EthSeedWalletProfile,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthXpubExportResponse {
    pub wallet_profile: String,
    pub project_account: u32,
    pub account_path: String,
    pub receive_path: String,
    pub receive_xpub: String,
    /// Non-blocking cautionary warning restating that an xpub exposes the
    /// wallet's ENTIRE past and future receive-address tree to any holder.
    /// Always populated by current daemons; empty when deserialized from
    /// payloads produced before this field existed.
    #[serde(default)]
    pub warning: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthXpubAddressResponse {
    pub index: u32,
    pub address: String,
}
