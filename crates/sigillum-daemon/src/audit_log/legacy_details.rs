use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub(super) struct KeyMutationDetails {
    pub(super) key: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct SecretReadDetails {
    pub(super) key: String,
    pub(super) env_name: String,
    pub(super) tier: u8,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct SecretPushDetails {
    pub(super) from_compartment: usize,
    pub(super) to_compartment: usize,
    pub(super) key: String,
    pub(super) new_key: String,
    pub(super) tier: u8,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct CompartmentMutationDetails {
    pub(super) label: String,
    pub(super) threshold: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct CompartmentRemoveDetails {
    pub(super) id: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct CompartmentSwitchDetails {
    pub(super) label: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct UnlockPassphraseDetails {
    pub(super) compartment_ids: Vec<usize>,
    pub(super) count: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct UnlockFido2Details {
    pub(super) compartment_ids: Vec<usize>,
    pub(super) count: usize,
    pub(super) tap_count: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct UnlockBiometricDetails {
    pub(super) compartment_id: usize,
    pub(super) fingerprint_hex: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct BiometricEnrollDetails {
    pub(super) fingerprint_hex: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct ProfilesEvmProviderUpsertDetails {
    pub(super) name: String,
    pub(super) chain_id: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct NamedAuditDetails {
    pub(super) name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct ProfilesEthStealthWalletUpsertDetails {
    pub(super) name: String,
    pub(super) provider_profile: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct ProfilesEthXpubWalletUpsertDetails {
    pub(super) name: String,
    pub(super) provider_profile: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct ProfilesEthSeedWalletUpsertDetails {
    pub(super) name: String,
    pub(super) provider_profile: String,
    pub(super) word_count: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct SnapshotAuditDetails {
    pub(super) file_count: usize,
    pub(super) total_bytes: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Fido2SetupDetails {
    pub(super) label: String,
    #[serde(alias = "compartments")]
    pub(super) compartment_count: usize,
    pub(super) total_keys: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Fido2RegisterDetails {
    pub(super) label: String,
    pub(super) total_keys: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Fido2RemoveDetails {
    pub(super) label: String,
    pub(super) sessions_invalidated: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct QueueEnqueueDetails {
    pub(super) id: String,
    pub(super) kind: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct QueueProcessDetails {
    pub(super) processed: usize,
    pub(super) succeeded: usize,
    pub(super) blocked: usize,
    pub(super) retrying: usize,
    pub(super) failed: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct TransitEncryptDetails {
    pub(super) key: String,
    pub(super) ciphertext_len: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct TransitDecryptDetails {
    pub(super) key: String,
    pub(super) plaintext_len: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct TransitHmacDetails {
    pub(super) key: String,
    pub(super) input_len: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct EvmBroadcastDetails {
    pub(super) transaction_hash_hex: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletExportDetails {
    pub(super) wallet: String,
    pub(super) short_name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletXpubExportDetails {
    pub(super) wallet_profile: String,
    pub(super) project_account: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletCheckDetails {
    pub(super) wallet: String,
    pub(super) matches: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletSignDetails {
    pub(super) wallet: String,
    pub(super) stealth_address: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletSignedTransactionDetails {
    pub(super) wallet: String,
    pub(super) kind: String,
    pub(super) to: String,
    pub(super) nonce: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletSendTransactionDetails {
    pub(super) wallet: String,
    pub(super) to: String,
    pub(super) nonce: u64,
    pub(super) broadcast: bool,
    pub(super) transaction_hash_hex: String,
    #[serde(default)]
    pub(super) broadcast_transaction_hash_hex: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct DepositsCreateDetails {
    pub(super) id: String,
    pub(super) wallet_profile: String,
    pub(super) asset_kind: String,
    #[serde(default)]
    pub(super) token_address: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct IdOnlyDetails {
    pub(super) id: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct DepositsRefreshDetails {
    pub(super) processed: usize,
    pub(super) detected: usize,
    pub(super) queued: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct DepositEnqueueSweepDetails {
    pub(super) id: String,
    pub(super) job_id: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct DepositsAnnouncementScanDetails {
    pub(super) wallet_profile: String,
    pub(super) provider_profile: String,
    pub(super) scanned: usize,
    pub(super) matched: usize,
    pub(super) created: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct MaintenanceRunDetails {
    pub(super) refreshed: usize,
    pub(super) detected: usize,
    pub(super) queued: usize,
    pub(super) processed: usize,
    pub(super) succeeded: usize,
    pub(super) blocked: usize,
    pub(super) retrying: usize,
    pub(super) failed: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletInventoryScanDetails {
    pub(super) id: String,
    pub(super) wallets: usize,
    pub(super) providers: usize,
    pub(super) addresses: usize,
    pub(super) holdings: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct ReceivingRefreshBalancesDetails {
    pub(super) addresses_requested: u32,
    pub(super) addresses_refreshed: u32,
    pub(super) addresses_skipped: u32,
    pub(super) stealth_refreshed: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletInventoryChainProfileUpsertDetails {
    pub(super) name: String,
    pub(super) chain_family: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletInventoryChainProfileDeleteDetails {
    pub(super) name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletInventoryDiscoveryJobUpdateDetails {
    pub(super) id: String,
    pub(super) status: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletInventoryNftMetadataOptInDetails {
    pub(super) chain_id: u64,
    pub(super) contract_address: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletInventoryNftMetadataSettingsUpdateDetails {
    pub(super) ipfs_gateway_configured: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletInventoryNftMetadataFetchDetails {
    pub(super) fetched: usize,
    pub(super) skipped: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletConsolidationPlanGenerateDetails {
    pub(super) id: String,
    pub(super) steps: usize,
    pub(super) blocked: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletConsolidationPlanApproveDetails {
    pub(super) id: String,
    pub(super) approved: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletConsolidationPlanSimulateDetails {
    pub(super) id: String,
    pub(super) passed: usize,
    pub(super) failed: usize,
    pub(super) unsupported: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct WalletConsolidationPlanExportDetails {
    pub(super) id: String,
    pub(super) format: String,
    pub(super) exported: usize,
    pub(super) skipped: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct RunCompleteDetails {
    pub(super) program: String,
    #[serde(default)]
    pub(super) args: Vec<String>,
    pub(super) exit_code: Option<i32>,
    pub(super) signal: Option<i32>,
    pub(super) success: bool,
}
