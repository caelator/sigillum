//! Audit trail: append-only, typed event log with legacy migration support.
//!
//! ## Audit Trail Design
//!
//! Every security-relevant operation is recorded to an immutable audit log under
//! `~/.sigillum/.audit`. Live storage is SQLite (`audit.db`) with a per-scope
//! HMAC MAC-chain (`prev_mac` / `mac`) so appends are tamper-evident. Events are:
//! - **Append-only**: New events are appended; old events are never modified or deleted.
//! - **Typed**: Each event is a typed variant (ApiKeySet, SecretDelete, Fido2Register, etc.)
//!   enabling structured queries and validation.
//! - **Durable**: Persisted in SQLite via `audit_db`; writes go through
//!   `append_event_chained` / `insert_event_chained`.
//!
//! The audit log is the system of record for compliance and forensics: if you need to
//! explain "what happened in this vault?", the audit log provides definitive answers
//! because it's cryptographically bound to the operation journal and compartment metas.
//!
//! ## Typed Events and Details
//!
//! Each event contains:
//! - `created_at_unix`: Timestamp (UTC)
//! - `compartment_id`: Optional compartment scope
//! - `kind`: Event type as string (e.g., "api_key.set", "snapshot.import")
//! - `details`: JSON object with event-specific fields (key name, reason, etc.)
//!
//! Public events (returned to API clients) hide some details (like deletion reasons)
//! for privacy. Internal events record full details for forensics.
//!
//! ## Legacy Migration
//!
//! Pre-SQLite installs may still have a JSONL `audit.log`. On startup,
//! `audit/migration.rs` imports those lines into `audit.db` (and
//! `StoredAuditEvent::from_legacy_json()` converts pre-versioned shapes).
//! JSONL is migration input only; it is not the live store.

#[cfg(test)]
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sigillum_api::AuditEvent as PublicAuditEvent;

use crate::json_store::{JsonDocument, JsonSchema};
#[cfg(test)]
use crate::json_store::{decode_json_document, encode_json_document_compact};

mod legacy_details;

use legacy_details::*;

// ── Core Structures ─────────────────────────────

/// An audit event persisted to the audit trail.
///
/// Stored in `.audit` as JSONL (one per line) for durability and streaming reads.
/// Includes full details (with reasons, keys, etc.) for forensics.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct StoredAuditEvent {
    /// Unix timestamp (UTC) when the event occurred.
    pub created_at_unix: u64,
    /// Compartment ID if this event is compartment-scoped; None for vault-wide events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compartment_id: Option<usize>,
    /// The event kind and detailed parameters.
    #[serde(flatten)]
    pub spec: AuditEventSpec,
}

impl StoredAuditEvent {
    pub(crate) fn to_public_event(&self) -> PublicAuditEvent {
        PublicAuditEvent {
            created_at_unix: self.created_at_unix,
            kind: self.spec.kind().to_string(),
            compartment_id: self.compartment_id,
            details: self.spec.public_details(),
        }
    }
}

impl JsonDocument for StoredAuditEvent {
    const SCHEMA: JsonSchema = JsonSchema::new("sigillum.audit-event", 1);

    fn from_legacy_json(path: &Path, value: Value) -> Result<Self, std::io::Error> {
        let legacy: PublicAuditEvent = serde_json::from_value(value).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "failed to parse legacy {} document {}: {error}",
                    Self::SCHEMA.name,
                    path.display()
                ),
            )
        })?;

        Ok(Self {
            created_at_unix: legacy.created_at_unix,
            compartment_id: legacy.compartment_id,
            spec: AuditEventSpec::from_legacy_details(path, legacy.kind, legacy.details)?,
        })
    }
}

// ── Queue Job Kinds ────────────────────────────

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuditQueueJobKind {
    EthStealthTransfer,
    EthStealthErc20Transfer,
    EthStealthNativeSweep,
    EthStealthErc20Sweep,
    EthSeedTransfer,
    EthSeedNativeSweep,
    EthSeedErc20Sweep,
    PlanStepExecution,
}

impl AuditQueueJobKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::EthStealthTransfer => "eth_stealth_transfer",
            Self::EthStealthErc20Transfer => "eth_stealth_erc20_transfer",
            Self::EthStealthNativeSweep => "eth_stealth_native_sweep",
            Self::EthStealthErc20Sweep => "eth_stealth_erc20_sweep",
            Self::EthSeedTransfer => "eth_seed_transfer",
            Self::EthSeedNativeSweep => "eth_seed_native_sweep",
            Self::EthSeedErc20Sweep => "eth_seed_erc20_sweep",
            Self::PlanStepExecution => "plan_step_execution",
        }
    }

    pub(crate) fn from_payload(payload: &sigillum_api::QueueJobPayload) -> Self {
        match payload {
            sigillum_api::QueueJobPayload::EthStealthTransfer { .. } => Self::EthStealthTransfer,
            sigillum_api::QueueJobPayload::EthStealthErc20Transfer { .. } => {
                Self::EthStealthErc20Transfer
            }
            sigillum_api::QueueJobPayload::EthStealthNativeSweep { .. } => {
                Self::EthStealthNativeSweep
            }
            sigillum_api::QueueJobPayload::EthStealthErc20Sweep { .. } => {
                Self::EthStealthErc20Sweep
            }
            sigillum_api::QueueJobPayload::EthSeedTransfer { .. } => Self::EthSeedTransfer,
            sigillum_api::QueueJobPayload::EthSeedNativeSweep { .. } => Self::EthSeedNativeSweep,
            sigillum_api::QueueJobPayload::EthSeedErc20Sweep { .. } => Self::EthSeedErc20Sweep,
            sigillum_api::QueueJobPayload::PlanStepExecution { .. } => Self::PlanStepExecution,
        }
    }
}

// ── Event Types ────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "details")]
pub(crate) enum AuditEventSpec {
    #[serde(rename = "api_key.set")]
    ApiKeySet { key: String },
    #[serde(rename = "api_key.delete")]
    ApiKeyDelete { key: String },
    #[serde(rename = "secret.set")]
    SecretSet { key: String },
    #[serde(rename = "secret.read")]
    SecretRead {
        key: String,
        env_name: String,
        tier: u8,
    },
    #[serde(rename = "secret.delete")]
    SecretDelete { key: String },
    #[serde(rename = "secret.push")]
    SecretPush {
        from_compartment: usize,
        to_compartment: usize,
        key: String,
        new_key: String,
        tier: u8,
    },
    #[serde(rename = "compartment.add")]
    CompartmentAdd { label: String, threshold: usize },
    #[serde(rename = "compartment.remove")]
    CompartmentRemove { id: usize },
    #[serde(rename = "compartment.init")]
    CompartmentInit { label: String, threshold: usize },
    #[serde(rename = "compartment.switch")]
    CompartmentSwitch { label: String },
    #[serde(rename = "unlock.passphrase")]
    UnlockPassphrase {
        compartment_ids: Vec<usize>,
        count: usize,
    },
    #[serde(rename = "unlock.fido2")]
    UnlockFido2 {
        compartment_ids: Vec<usize>,
        count: usize,
        tap_count: usize,
    },
    #[serde(rename = "unlock.biometric")]
    UnlockBiometric {
        compartment_id: usize,
        fingerprint_hex: String,
    },
    #[serde(rename = "biometric.enroll")]
    BiometricEnroll { fingerprint_hex: String },
    #[serde(rename = "lock.all")]
    LockAll,
    #[serde(rename = "session.revoke")]
    SessionRevoke,
    #[serde(rename = "profiles.evm_provider.upsert")]
    ProfilesEvmProviderUpsert { name: String, chain_id: u64 },
    #[serde(rename = "profiles.evm_provider.delete")]
    ProfilesEvmProviderDelete { name: String },
    #[serde(rename = "profiles.eth_stealth_wallet.upsert")]
    ProfilesEthStealthWalletUpsert {
        name: String,
        provider_profile: String,
    },
    #[serde(rename = "profiles.eth_stealth_wallet.delete")]
    ProfilesEthStealthWalletDelete { name: String },
    #[serde(rename = "profiles.eth_xpub_wallet.upsert")]
    ProfilesEthXpubWalletUpsert {
        name: String,
        provider_profile: String,
    },
    #[serde(rename = "profiles.eth_xpub_wallet.delete")]
    ProfilesEthXpubWalletDelete { name: String },
    #[serde(rename = "profiles.eth_seed_wallet.upsert")]
    ProfilesEthSeedWalletUpsert {
        name: String,
        provider_profile: String,
        word_count: usize,
    },
    /// A seed wallet profile was created from a daemon-generated mnemonic.
    /// Records metadata only — never any mnemonic or entropy material.
    #[serde(rename = "profiles.eth_seed_wallet.create")]
    ProfilesEthSeedWalletCreate {
        name: String,
        provider_profile: String,
        word_count: usize,
    },
    #[serde(rename = "profiles.eth_seed_wallet.delete")]
    ProfilesEthSeedWalletDelete { name: String },
    #[serde(rename = "snapshot.export")]
    SnapshotExport {
        file_count: usize,
        total_bytes: usize,
    },
    #[serde(rename = "snapshot.restore")]
    SnapshotRestore {
        file_count: usize,
        total_bytes: usize,
    },
    #[serde(rename = "fido2.setup")]
    Fido2Setup {
        label: String,
        compartment_count: usize,
        total_keys: usize,
    },
    #[serde(rename = "fido2.register_poison")]
    Fido2RegisterPoison { label: String, total_keys: usize },
    #[serde(rename = "fido2.register")]
    Fido2Register { label: String, total_keys: usize },
    #[serde(rename = "fido2.remove")]
    Fido2Remove {
        label: String,
        sessions_invalidated: bool,
    },
    #[serde(rename = "queue.enqueue")]
    QueueEnqueue {
        id: String,
        job_kind: AuditQueueJobKind,
    },
    #[serde(rename = "queue.process")]
    QueueProcess {
        processed: usize,
        succeeded: usize,
        blocked: usize,
        retrying: usize,
        failed: usize,
    },
    #[serde(rename = "transit.encrypt")]
    TransitEncrypt { key: String, ciphertext_len: usize },
    #[serde(rename = "transit.decrypt")]
    TransitDecrypt { key: String, plaintext_len: usize },
    #[serde(rename = "transit.hmac")]
    TransitHmac { key: String, input_len: usize },
    #[serde(rename = "evm.broadcast")]
    EvmBroadcast { transaction_hash_hex: String },
    #[serde(rename = "wallet.eth_stealth.export")]
    WalletEthStealthExport { wallet: String, short_name: String },
    #[serde(rename = "wallet.eth_xpub.export")]
    WalletEthXpubExport {
        wallet_profile: String,
        project_account: u32,
    },
    #[serde(rename = "wallet.eth_stealth.check")]
    WalletEthStealthCheck { wallet: String, matches: bool },
    #[serde(rename = "wallet.eth_stealth.sign")]
    WalletEthStealthSign {
        wallet: String,
        stealth_address: String,
    },
    #[serde(rename = "wallet.eth_stealth.sign_transfer")]
    WalletEthStealthSignTransfer {
        wallet: String,
        transaction_kind: String,
        to: String,
        nonce: u64,
    },
    #[serde(rename = "wallet.eth_stealth.sign_erc20_transfer")]
    WalletEthStealthSignErc20Transfer {
        wallet: String,
        transaction_kind: String,
        to: String,
        nonce: u64,
    },
    #[serde(rename = "wallet.eth_stealth.send_transfer")]
    WalletEthStealthSendTransfer {
        wallet: String,
        to: String,
        nonce: u64,
        broadcast: bool,
        transaction_hash_hex: String,
        broadcast_transaction_hash_hex: Option<String>,
    },
    #[serde(rename = "wallet.eth_seed.send_transfer")]
    WalletEthSeedSendTransfer {
        wallet: String,
        to: String,
        nonce: u64,
        broadcast: bool,
        transaction_hash_hex: String,
        broadcast_transaction_hash_hex: Option<String>,
    },
    #[serde(rename = "wallet.eth_stealth.send_erc20_transfer")]
    WalletEthStealthSendErc20Transfer {
        wallet: String,
        to: String,
        nonce: u64,
        broadcast: bool,
        transaction_hash_hex: String,
        broadcast_transaction_hash_hex: Option<String>,
    },
    #[serde(rename = "deposits.eth_stealth.create")]
    DepositsEthStealthCreate {
        id: String,
        wallet_profile: String,
        asset_kind: String,
        token_address: Option<String>,
    },
    #[serde(rename = "deposits.eth_stealth.delete")]
    DepositsEthStealthDelete { id: String },
    #[serde(rename = "deposits.eth_stealth.refresh")]
    DepositsEthStealthRefresh {
        processed: usize,
        detected: usize,
        queued: usize,
    },
    #[serde(rename = "deposits.eth_stealth.enqueue_sweep")]
    DepositsEthStealthEnqueueSweep { id: String, job_id: String },
    #[serde(rename = "deposits.eth_stealth.announcement_scan")]
    DepositsEthStealthAnnouncementScan {
        wallet_profile: String,
        provider_profile: String,
        scanned: usize,
        matched: usize,
        created: usize,
    },
    #[serde(rename = "maintenance.run")]
    MaintenanceRun {
        refreshed: usize,
        detected: usize,
        queued: usize,
        processed: usize,
        succeeded: usize,
        blocked: usize,
        retrying: usize,
        failed: usize,
    },
    #[serde(rename = "wallet_inventory.scan")]
    WalletInventoryScan {
        id: String,
        wallets: usize,
        providers: usize,
        addresses: usize,
        holdings: usize,
    },
    #[serde(rename = "receiving.refresh_balances")]
    ReceivingRefreshBalances {
        addresses_requested: u32,
        addresses_refreshed: u32,
        addresses_skipped: u32,
        stealth_refreshed: bool,
    },
    #[serde(rename = "wallet_inventory.chain_profile.upsert")]
    WalletInventoryChainProfileUpsert { name: String, chain_family: String },
    #[serde(rename = "wallet_inventory.chain_profile.delete")]
    WalletInventoryChainProfileDelete { name: String },
    #[serde(rename = "wallet_inventory.discovery_job.update")]
    WalletInventoryDiscoveryJobUpdate { id: String, status: String },
    #[serde(rename = "wallet_inventory.risk_catalog.upsert")]
    WalletInventoryRiskCatalogUpsert { address: String, risk_level: String },
    #[serde(rename = "wallet_inventory.risk_catalog.delete")]
    WalletInventoryRiskCatalogDelete { address: String },
    #[serde(rename = "wallet_inventory.token_registry.import")]
    WalletInventoryTokenRegistryImport { name: String, entries: usize },
    #[serde(rename = "wallet_inventory.token_registry.delete")]
    WalletInventoryTokenRegistryDelete { name: String },
    #[serde(rename = "wallet_inventory.nft_metadata.opt_in.upsert")]
    WalletInventoryNftMetadataOptInUpsert {
        chain_id: u64,
        contract_address: String,
    },
    #[serde(rename = "wallet_inventory.nft_metadata.opt_in.delete")]
    WalletInventoryNftMetadataOptInDelete {
        chain_id: u64,
        contract_address: String,
    },
    #[serde(rename = "wallet_inventory.nft_metadata.settings.update")]
    WalletInventoryNftMetadataSettingsUpdate { ipfs_gateway_configured: bool },
    #[serde(rename = "wallet_inventory.nft_metadata.fetch")]
    WalletInventoryNftMetadataFetch { fetched: usize, skipped: usize },
    #[serde(rename = "wallet_inventory.watch_address.upsert")]
    WalletInventoryWatchAddressUpsert { address: String, label: String },
    #[serde(rename = "wallet_inventory.watch_address.delete")]
    WalletInventoryWatchAddressDelete { address: String },
    #[serde(rename = "wallet_consolidation.plan.generate")]
    WalletConsolidationPlanGenerate {
        id: String,
        steps: usize,
        blocked: usize,
    },
    #[serde(rename = "wallet_consolidation.plan.approve")]
    WalletConsolidationPlanApprove { id: String, approved: usize },
    #[serde(rename = "wallet_consolidation.plan.simulate")]
    WalletConsolidationPlanSimulate {
        id: String,
        passed: usize,
        failed: usize,
        unsupported: usize,
    },
    #[serde(rename = "wallet_consolidation.plan.export")]
    WalletConsolidationPlanExport {
        id: String,
        format: String,
        exported: usize,
        skipped: usize,
    },
    /// W7.2 plan-step enqueue. Carries ids and the acting session's
    /// fingerprint only — never raw tokens, calldata, or key material.
    #[serde(rename = "wallet_consolidation.plan.enqueue_step")]
    WalletConsolidationPlanEnqueueStep {
        plan_id: String,
        step_id: String,
        job_id: String,
        action_family: String,
        session_fingerprint_hex: String,
    },
    /// W7.3: a plan-step's prepared call was signed (before broadcast). NEVER
    /// carries key material — only ids, the source address, and the resulting
    /// (locally computed) transaction hash.
    #[serde(rename = "wallet_consolidation.plan.step_sign")]
    WalletConsolidationPlanStepSign {
        plan_id: String,
        step_id: String,
        job_id: String,
        action_family: String,
        source_address: String,
        transaction_hash_hex: String,
        session_fingerprint_hex: String,
    },
    /// W7.3: a signed plan-step transaction was accepted by the RPC provider.
    #[serde(rename = "wallet_consolidation.plan.step_broadcast")]
    WalletConsolidationPlanStepBroadcast {
        plan_id: String,
        step_id: String,
        job_id: String,
        action_family: String,
        transaction_hash_hex: String,
        broadcast_transaction_hash_hex: String,
        session_fingerprint_hex: String,
    },
    /// W7.3: a signed plan-step transaction failed to broadcast (the queue
    /// job's own state/last_error carries the retry/terminal disposition;
    /// this event is the forensic record that a real signed transaction
    /// existed even though it never reached the network).
    #[serde(rename = "wallet_consolidation.plan.step_broadcast_failed")]
    WalletConsolidationPlanStepBroadcastFailed {
        plan_id: String,
        step_id: String,
        job_id: String,
        action_family: String,
        transaction_hash_hex: String,
        reason: String,
        session_fingerprint_hex: String,
    },
    #[serde(rename = "treasury.policy.update")]
    TreasuryPolicyUpdate { enabled: bool, destinations: usize },
    #[serde(rename = "treasury.policy.execution_gate.update")]
    TreasuryExecutionGateUpdate {
        gate: String,
        old_value: bool,
        new_value: bool,
        session_fingerprint_hex: String,
    },
    #[serde(rename = "treasury.automation_run")]
    TreasuryAutomationRun {
        generated: usize,
        enqueued: usize,
        skipped: usize,
    },
    /// Derived addresses are intentionally omitted: the audit log records
    /// that an allocation happened, not which address it produced.
    #[serde(rename = "treasury.receive.allocate")]
    TreasuryReceiveAllocate {
        wallet_profile: String,
        purpose: String,
    },
    #[serde(rename = "treasury.receive.rotate")]
    TreasuryReceiveRotate { id: String },
    #[serde(rename = "treasury.party.create")]
    TreasuryPartyCreate { name: String },
    #[serde(rename = "treasury.party.delete")]
    TreasuryPartyDelete { name: String },
    #[serde(rename = "treasury.receive.bind")]
    TreasuryReceiveBind { name: String },
    /// An operator-initiated self-check pass over configured subsystems.
    /// Records only aggregate counts — never check subjects or details.
    #[serde(rename = "selfcheck.run")]
    SelfCheckRun { checks: usize, failures: usize },
    #[serde(rename = "run.complete")]
    RunComplete {
        program: String,
        args: Vec<String>,
        exit_code: Option<i32>,
        signal: Option<i32>,
        success: bool,
    },
}

// ── Event Spec Methods ──────────────────────────

impl AuditEventSpec {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::ApiKeySet { .. } => "api_key.set",
            Self::ApiKeyDelete { .. } => "api_key.delete",
            Self::SecretSet { .. } => "secret.set",
            Self::SecretRead { .. } => "secret.read",
            Self::SecretDelete { .. } => "secret.delete",
            Self::SecretPush { .. } => "secret.push",
            Self::CompartmentAdd { .. } => "compartment.add",
            Self::CompartmentRemove { .. } => "compartment.remove",
            Self::CompartmentInit { .. } => "compartment.init",
            Self::CompartmentSwitch { .. } => "compartment.switch",
            Self::UnlockPassphrase { .. } => "unlock.passphrase",
            Self::UnlockFido2 { .. } => "unlock.fido2",
            Self::UnlockBiometric { .. } => "unlock.biometric",
            Self::BiometricEnroll { .. } => "biometric.enroll",
            Self::LockAll => "lock.all",
            Self::SessionRevoke => "session.revoke",
            Self::ProfilesEvmProviderUpsert { .. } => "profiles.evm_provider.upsert",
            Self::ProfilesEvmProviderDelete { .. } => "profiles.evm_provider.delete",
            Self::ProfilesEthStealthWalletUpsert { .. } => "profiles.eth_stealth_wallet.upsert",
            Self::ProfilesEthStealthWalletDelete { .. } => "profiles.eth_stealth_wallet.delete",
            Self::ProfilesEthXpubWalletUpsert { .. } => "profiles.eth_xpub_wallet.upsert",
            Self::ProfilesEthXpubWalletDelete { .. } => "profiles.eth_xpub_wallet.delete",
            Self::ProfilesEthSeedWalletUpsert { .. } => "profiles.eth_seed_wallet.upsert",
            Self::ProfilesEthSeedWalletCreate { .. } => "profiles.eth_seed_wallet.create",
            Self::ProfilesEthSeedWalletDelete { .. } => "profiles.eth_seed_wallet.delete",
            Self::SnapshotExport { .. } => "snapshot.export",
            Self::SnapshotRestore { .. } => "snapshot.restore",
            Self::Fido2Setup { .. } => "fido2.setup",
            Self::Fido2RegisterPoison { .. } => "fido2.register_poison",
            Self::Fido2Register { .. } => "fido2.register",
            Self::Fido2Remove { .. } => "fido2.remove",
            Self::QueueEnqueue { .. } => "queue.enqueue",
            Self::QueueProcess { .. } => "queue.process",
            Self::TransitEncrypt { .. } => "transit.encrypt",
            Self::TransitDecrypt { .. } => "transit.decrypt",
            Self::TransitHmac { .. } => "transit.hmac",
            Self::EvmBroadcast { .. } => "evm.broadcast",
            Self::WalletEthStealthExport { .. } => "wallet.eth_stealth.export",
            Self::WalletEthXpubExport { .. } => "wallet.eth_xpub.export",
            Self::WalletEthStealthCheck { .. } => "wallet.eth_stealth.check",
            Self::WalletEthStealthSign { .. } => "wallet.eth_stealth.sign",
            Self::WalletEthStealthSignTransfer { .. } => "wallet.eth_stealth.sign_transfer",
            Self::WalletEthStealthSignErc20Transfer { .. } => {
                "wallet.eth_stealth.sign_erc20_transfer"
            }
            Self::WalletEthStealthSendTransfer { .. } => "wallet.eth_stealth.send_transfer",
            Self::WalletEthSeedSendTransfer { .. } => "wallet.eth_seed.send_transfer",
            Self::WalletEthStealthSendErc20Transfer { .. } => {
                "wallet.eth_stealth.send_erc20_transfer"
            }
            Self::DepositsEthStealthCreate { .. } => "deposits.eth_stealth.create",
            Self::DepositsEthStealthDelete { .. } => "deposits.eth_stealth.delete",
            Self::DepositsEthStealthRefresh { .. } => "deposits.eth_stealth.refresh",
            Self::DepositsEthStealthEnqueueSweep { .. } => "deposits.eth_stealth.enqueue_sweep",
            Self::DepositsEthStealthAnnouncementScan { .. } => {
                "deposits.eth_stealth.announcement_scan"
            }
            Self::MaintenanceRun { .. } => "maintenance.run",
            Self::WalletInventoryScan { .. } => "wallet_inventory.scan",
            Self::ReceivingRefreshBalances { .. } => "receiving.refresh_balances",
            Self::WalletInventoryChainProfileUpsert { .. } => {
                "wallet_inventory.chain_profile.upsert"
            }
            Self::WalletInventoryChainProfileDelete { .. } => {
                "wallet_inventory.chain_profile.delete"
            }
            Self::WalletInventoryDiscoveryJobUpdate { .. } => {
                "wallet_inventory.discovery_job.update"
            }
            Self::WalletInventoryRiskCatalogUpsert { .. } => "wallet_inventory.risk_catalog.upsert",
            Self::WalletInventoryRiskCatalogDelete { .. } => "wallet_inventory.risk_catalog.delete",
            Self::WalletInventoryTokenRegistryImport { .. } => {
                "wallet_inventory.token_registry.import"
            }
            Self::WalletInventoryTokenRegistryDelete { .. } => {
                "wallet_inventory.token_registry.delete"
            }
            Self::WalletInventoryNftMetadataOptInUpsert { .. } => {
                "wallet_inventory.nft_metadata.opt_in.upsert"
            }
            Self::WalletInventoryNftMetadataOptInDelete { .. } => {
                "wallet_inventory.nft_metadata.opt_in.delete"
            }
            Self::WalletInventoryNftMetadataSettingsUpdate { .. } => {
                "wallet_inventory.nft_metadata.settings.update"
            }
            Self::WalletInventoryNftMetadataFetch { .. } => "wallet_inventory.nft_metadata.fetch",
            Self::WalletInventoryWatchAddressUpsert { .. } => {
                "wallet_inventory.watch_address.upsert"
            }
            Self::WalletInventoryWatchAddressDelete { .. } => {
                "wallet_inventory.watch_address.delete"
            }
            Self::WalletConsolidationPlanGenerate { .. } => "wallet_consolidation.plan.generate",
            Self::WalletConsolidationPlanApprove { .. } => "wallet_consolidation.plan.approve",
            Self::WalletConsolidationPlanSimulate { .. } => "wallet_consolidation.plan.simulate",
            Self::WalletConsolidationPlanExport { .. } => "wallet_consolidation.plan.export",
            Self::WalletConsolidationPlanEnqueueStep { .. } => {
                "wallet_consolidation.plan.enqueue_step"
            }
            Self::WalletConsolidationPlanStepSign { .. } => "wallet_consolidation.plan.step_sign",
            Self::WalletConsolidationPlanStepBroadcast { .. } => {
                "wallet_consolidation.plan.step_broadcast"
            }
            Self::WalletConsolidationPlanStepBroadcastFailed { .. } => {
                "wallet_consolidation.plan.step_broadcast_failed"
            }
            Self::TreasuryPolicyUpdate { .. } => "treasury.policy.update",
            Self::TreasuryExecutionGateUpdate { .. } => "treasury.policy.execution_gate.update",
            Self::TreasuryAutomationRun { .. } => "treasury.automation_run",
            Self::TreasuryReceiveAllocate { .. } => "treasury.receive.allocate",
            Self::TreasuryReceiveRotate { .. } => "treasury.receive.rotate",
            Self::TreasuryPartyCreate { .. } => "treasury.party.create",
            Self::TreasuryPartyDelete { .. } => "treasury.party.delete",
            Self::TreasuryReceiveBind { .. } => "treasury.receive.bind",
            Self::SelfCheckRun { .. } => "selfcheck.run",
            Self::RunComplete { .. } => "run.complete",
        }
    }

    pub(crate) fn public_details(&self) -> Value {
        match self {
            Self::ApiKeySet { key } => json!({ "key": key, "tier": 1 }),
            Self::ApiKeyDelete { key } => json!({ "key": key, "tier": 1 }),
            Self::SecretSet { key } => json!({ "key": key, "tier": 2 }),
            Self::SecretRead {
                key,
                env_name,
                tier,
            } => {
                json!({ "key": key, "env_name": env_name, "tier": tier })
            }
            Self::SecretDelete { key } => json!({ "key": key, "tier": 2 }),
            Self::SecretPush {
                from_compartment,
                to_compartment,
                key,
                new_key,
                tier,
            } => json!({
                "from_compartment": from_compartment,
                "to_compartment": to_compartment,
                "key": key,
                "new_key": new_key,
                "tier": tier,
            }),
            Self::CompartmentAdd { label, threshold } => {
                json!({ "label": label, "threshold": threshold })
            }
            Self::CompartmentRemove { id } => json!({ "id": id }),
            Self::CompartmentInit { label, threshold } => {
                json!({ "label": label, "threshold": threshold })
            }
            Self::CompartmentSwitch { label } => json!({ "label": label }),
            Self::UnlockPassphrase {
                compartment_ids,
                count,
            } => json!({ "compartment_ids": compartment_ids, "count": count }),
            Self::UnlockFido2 {
                compartment_ids,
                count,
                tap_count,
            } => json!({
                "compartment_ids": compartment_ids,
                "count": count,
                "tap_count": tap_count,
            }),
            Self::UnlockBiometric {
                compartment_id,
                fingerprint_hex,
            } => json!({
                "compartment_id": compartment_id,
                "fingerprint_hex": fingerprint_hex,
            }),
            Self::BiometricEnroll { fingerprint_hex } => {
                json!({ "fingerprint_hex": fingerprint_hex })
            }
            Self::LockAll | Self::SessionRevoke => json!({}),
            Self::ProfilesEvmProviderUpsert { name, chain_id } => {
                json!({ "name": name, "chain_id": chain_id })
            }
            Self::ProfilesEvmProviderDelete { name } => json!({ "name": name }),
            Self::ProfilesEthStealthWalletUpsert {
                name,
                provider_profile,
            } => json!({ "name": name, "provider_profile": provider_profile }),
            Self::ProfilesEthStealthWalletDelete { name } => json!({ "name": name }),
            Self::ProfilesEthXpubWalletUpsert {
                name,
                provider_profile,
            } => json!({ "name": name, "provider_profile": provider_profile }),
            Self::ProfilesEthXpubWalletDelete { name } => json!({ "name": name }),
            Self::ProfilesEthSeedWalletUpsert {
                name,
                provider_profile,
                word_count,
            }
            | Self::ProfilesEthSeedWalletCreate {
                name,
                provider_profile,
                word_count,
            } => json!({
                "name": name,
                "provider_profile": provider_profile,
                "word_count": word_count,
            }),
            Self::ProfilesEthSeedWalletDelete { name } => json!({ "name": name }),
            Self::SnapshotExport {
                file_count,
                total_bytes,
            }
            | Self::SnapshotRestore {
                file_count,
                total_bytes,
            } => json!({ "file_count": file_count, "total_bytes": total_bytes }),
            Self::Fido2Setup {
                label,
                compartment_count,
                total_keys,
            } => json!({
                "label": label,
                "compartment_count": compartment_count,
                "total_keys": total_keys,
            }),
            Self::Fido2RegisterPoison { label, total_keys }
            | Self::Fido2Register { label, total_keys } => {
                json!({ "label": label, "total_keys": total_keys })
            }
            Self::Fido2Remove {
                label,
                sessions_invalidated,
            } => json!({
                "label": label,
                "sessions_invalidated": sessions_invalidated,
            }),
            Self::QueueEnqueue { id, job_kind } => {
                json!({ "id": id, "kind": job_kind.as_str() })
            }
            Self::QueueProcess {
                processed,
                succeeded,
                blocked,
                retrying,
                failed,
            } => json!({
                "processed": processed,
                "succeeded": succeeded,
                "blocked": blocked,
                "retrying": retrying,
                "failed": failed,
            }),
            Self::TransitEncrypt {
                key,
                ciphertext_len,
            } => json!({ "key": key, "ciphertext_len": ciphertext_len }),
            Self::TransitDecrypt { key, plaintext_len } => {
                json!({ "key": key, "plaintext_len": plaintext_len })
            }
            Self::TransitHmac { key, input_len } => {
                json!({ "key": key, "input_len": input_len })
            }
            Self::EvmBroadcast {
                transaction_hash_hex,
            } => json!({ "transaction_hash_hex": transaction_hash_hex }),
            Self::WalletEthStealthExport { wallet, short_name } => {
                json!({ "wallet": wallet, "short_name": short_name })
            }
            Self::WalletEthXpubExport {
                wallet_profile,
                project_account,
            } => json!({
                "wallet_profile": wallet_profile,
                "project_account": project_account,
            }),
            Self::WalletEthStealthCheck { wallet, matches } => {
                json!({ "wallet": wallet, "matches": matches })
            }
            Self::WalletEthStealthSign {
                wallet,
                stealth_address,
            } => json!({ "wallet": wallet, "stealth_address": stealth_address }),
            Self::WalletEthStealthSignTransfer {
                wallet,
                transaction_kind,
                to,
                nonce,
            }
            | Self::WalletEthStealthSignErc20Transfer {
                wallet,
                transaction_kind,
                to,
                nonce,
            } => json!({
                "wallet": wallet,
                "kind": transaction_kind,
                "to": to,
                "nonce": nonce,
            }),
            Self::WalletEthStealthSendTransfer {
                wallet,
                to,
                nonce,
                broadcast,
                transaction_hash_hex,
                broadcast_transaction_hash_hex,
            }
            | Self::WalletEthStealthSendErc20Transfer {
                wallet,
                to,
                nonce,
                broadcast,
                transaction_hash_hex,
                broadcast_transaction_hash_hex,
            }
            | Self::WalletEthSeedSendTransfer {
                wallet,
                to,
                nonce,
                broadcast,
                transaction_hash_hex,
                broadcast_transaction_hash_hex,
            } => json!({
                "wallet": wallet,
                "to": to,
                "nonce": nonce,
                "broadcast": broadcast,
                "transaction_hash_hex": transaction_hash_hex,
                "broadcast_transaction_hash_hex": broadcast_transaction_hash_hex,
            }),
            Self::DepositsEthStealthCreate {
                id,
                wallet_profile,
                asset_kind,
                token_address,
            } => {
                let mut map = Map::new();
                map.insert("id".into(), Value::String(id.clone()));
                map.insert(
                    "wallet_profile".into(),
                    Value::String(wallet_profile.clone()),
                );
                map.insert("asset_kind".into(), Value::String(asset_kind.clone()));
                if let Some(token_address) = token_address {
                    map.insert("token_address".into(), Value::String(token_address.clone()));
                }
                Value::Object(map)
            }
            Self::DepositsEthStealthDelete { id } => json!({ "id": id }),
            Self::DepositsEthStealthRefresh {
                processed,
                detected,
                queued,
            } => json!({
                "processed": processed,
                "detected": detected,
                "queued": queued,
            }),
            Self::DepositsEthStealthEnqueueSweep { id, job_id } => {
                json!({ "id": id, "job_id": job_id })
            }
            Self::MaintenanceRun {
                refreshed,
                detected,
                queued,
                processed,
                succeeded,
                blocked,
                retrying,
                failed,
            } => json!({
                "refreshed": refreshed,
                "detected": detected,
                "queued": queued,
                "processed": processed,
                "succeeded": succeeded,
                "blocked": blocked,
                "retrying": retrying,
                "failed": failed,
            }),
            Self::WalletInventoryScan {
                id,
                wallets,
                providers,
                addresses,
                holdings,
            } => json!({
                "id": id,
                "wallets": wallets,
                "providers": providers,
                "addresses": addresses,
                "holdings": holdings,
            }),
            Self::ReceivingRefreshBalances {
                addresses_requested,
                addresses_refreshed,
                addresses_skipped,
                stealth_refreshed,
            } => json!({
                "addresses_requested": addresses_requested,
                "addresses_refreshed": addresses_refreshed,
                "addresses_skipped": addresses_skipped,
                "stealth_refreshed": stealth_refreshed,
            }),
            Self::DepositsEthStealthAnnouncementScan {
                wallet_profile,
                provider_profile,
                scanned,
                matched,
                created,
            } => json!({
                "wallet_profile": wallet_profile,
                "provider_profile": provider_profile,
                "scanned": scanned,
                "matched": matched,
                "created": created,
            }),
            Self::WalletInventoryChainProfileUpsert { name, chain_family } => {
                json!({ "name": name, "chain_family": chain_family })
            }
            Self::WalletInventoryChainProfileDelete { name } => json!({ "name": name }),
            Self::WalletInventoryDiscoveryJobUpdate { id, status } => {
                json!({ "id": id, "status": status })
            }
            Self::WalletInventoryRiskCatalogUpsert {
                address,
                risk_level,
            } => json!({ "address": address, "risk_level": risk_level }),
            Self::WalletInventoryRiskCatalogDelete { address } => {
                json!({ "address": address })
            }
            Self::WalletInventoryTokenRegistryImport { name, entries } => {
                json!({ "name": name, "entries": entries })
            }
            Self::WalletInventoryTokenRegistryDelete { name } => json!({ "name": name }),
            Self::WalletInventoryNftMetadataOptInUpsert {
                chain_id,
                contract_address,
            }
            | Self::WalletInventoryNftMetadataOptInDelete {
                chain_id,
                contract_address,
            } => json!({ "chain_id": chain_id, "contract_address": contract_address }),
            Self::WalletInventoryNftMetadataSettingsUpdate {
                ipfs_gateway_configured,
            } => json!({ "ipfs_gateway_configured": ipfs_gateway_configured }),
            Self::WalletInventoryNftMetadataFetch { fetched, skipped } => {
                json!({ "fetched": fetched, "skipped": skipped })
            }
            Self::WalletInventoryWatchAddressUpsert { address, label } => {
                json!({ "address": address, "label": label })
            }
            Self::WalletInventoryWatchAddressDelete { address } => {
                json!({ "address": address })
            }
            Self::WalletConsolidationPlanGenerate { id, steps, blocked } => {
                json!({ "id": id, "steps": steps, "blocked": blocked })
            }
            Self::WalletConsolidationPlanApprove { id, approved } => {
                json!({ "id": id, "approved": approved })
            }
            Self::WalletConsolidationPlanSimulate {
                id,
                passed,
                failed,
                unsupported,
            } => json!({
                "id": id,
                "passed": passed,
                "failed": failed,
                "unsupported": unsupported,
            }),
            Self::WalletConsolidationPlanExport {
                id,
                format,
                exported,
                skipped,
            } => json!({
                "id": id,
                "format": format,
                "exported": exported,
                "skipped": skipped,
            }),
            Self::WalletConsolidationPlanEnqueueStep {
                plan_id,
                step_id,
                job_id,
                action_family,
                session_fingerprint_hex,
            } => json!({
                "plan_id": plan_id,
                "step_id": step_id,
                "job_id": job_id,
                "action_family": action_family,
                "session_fingerprint_hex": session_fingerprint_hex,
            }),
            Self::WalletConsolidationPlanStepSign {
                plan_id,
                step_id,
                job_id,
                action_family,
                source_address,
                transaction_hash_hex,
                session_fingerprint_hex,
            } => json!({
                "plan_id": plan_id,
                "step_id": step_id,
                "job_id": job_id,
                "action_family": action_family,
                "source_address": source_address,
                "transaction_hash_hex": transaction_hash_hex,
                "session_fingerprint_hex": session_fingerprint_hex,
            }),
            Self::WalletConsolidationPlanStepBroadcast {
                plan_id,
                step_id,
                job_id,
                action_family,
                transaction_hash_hex,
                broadcast_transaction_hash_hex,
                session_fingerprint_hex,
            } => json!({
                "plan_id": plan_id,
                "step_id": step_id,
                "job_id": job_id,
                "action_family": action_family,
                "transaction_hash_hex": transaction_hash_hex,
                "broadcast_transaction_hash_hex": broadcast_transaction_hash_hex,
                "session_fingerprint_hex": session_fingerprint_hex,
            }),
            Self::WalletConsolidationPlanStepBroadcastFailed {
                plan_id,
                step_id,
                job_id,
                action_family,
                transaction_hash_hex,
                reason,
                session_fingerprint_hex,
            } => json!({
                "plan_id": plan_id,
                "step_id": step_id,
                "job_id": job_id,
                "action_family": action_family,
                "transaction_hash_hex": transaction_hash_hex,
                "reason": reason,
                "session_fingerprint_hex": session_fingerprint_hex,
            }),
            Self::TreasuryPolicyUpdate {
                enabled,
                destinations,
            } => json!({ "enabled": enabled, "destinations": destinations }),
            Self::TreasuryExecutionGateUpdate {
                gate,
                old_value,
                new_value,
                session_fingerprint_hex,
            } => json!({
                "gate": gate,
                "old_value": old_value,
                "new_value": new_value,
                "session_fingerprint_hex": session_fingerprint_hex,
            }),
            Self::TreasuryAutomationRun {
                generated,
                enqueued,
                skipped,
            } => json!({ "generated": generated, "enqueued": enqueued, "skipped": skipped }),
            Self::TreasuryReceiveAllocate {
                wallet_profile,
                purpose,
            } => json!({ "wallet_profile": wallet_profile, "purpose": purpose }),
            Self::TreasuryReceiveRotate { id } => json!({ "id": id }),
            Self::TreasuryPartyCreate { name }
            | Self::TreasuryPartyDelete { name }
            | Self::TreasuryReceiveBind { name } => json!({ "name": name }),
            Self::SelfCheckRun { checks, failures } => {
                json!({ "checks": checks, "failures": failures })
            }
            Self::RunComplete {
                program,
                args,
                exit_code,
                signal,
                success,
            } => json!({
                "program": program,
                "args": args,
                "exit_code": exit_code,
                "signal": signal,
                "success": success,
            }),
        }
    }

    pub(crate) fn indexed_key(&self) -> Option<&str> {
        match self {
            Self::ApiKeySet { key }
            | Self::ApiKeyDelete { key }
            | Self::SecretSet { key }
            | Self::SecretRead { key, .. }
            | Self::SecretDelete { key }
            | Self::SecretPush { key, .. }
            | Self::TransitEncrypt { key, .. }
            | Self::TransitDecrypt { key, .. }
            | Self::TransitHmac { key, .. } => Some(key.as_str()),
            _ => None,
        }
    }

    fn from_legacy_details(
        path: &Path,
        kind: String,
        details: Value,
    ) -> Result<Self, std::io::Error> {
        match kind.as_str() {
            "api_key.set" => {
                let details = parse_legacy_details::<KeyMutationDetails>(path, &kind, details)?;
                Ok(Self::ApiKeySet { key: details.key })
            }
            "api_key.delete" => {
                let details = parse_legacy_details::<KeyMutationDetails>(path, &kind, details)?;
                Ok(Self::ApiKeyDelete { key: details.key })
            }
            "secret.set" => {
                let details = parse_legacy_details::<KeyMutationDetails>(path, &kind, details)?;
                Ok(Self::SecretSet { key: details.key })
            }
            "secret.read" => {
                let details = parse_legacy_details::<SecretReadDetails>(path, &kind, details)?;
                Ok(Self::SecretRead {
                    key: details.key,
                    env_name: details.env_name,
                    tier: details.tier,
                })
            }
            "secret.delete" => {
                let details = parse_legacy_details::<KeyMutationDetails>(path, &kind, details)?;
                Ok(Self::SecretDelete { key: details.key })
            }
            "secret.push" => {
                let details = parse_legacy_details::<SecretPushDetails>(path, &kind, details)?;
                Ok(Self::SecretPush {
                    from_compartment: details.from_compartment,
                    to_compartment: details.to_compartment,
                    key: details.key,
                    new_key: details.new_key,
                    tier: details.tier,
                })
            }
            "compartment.add" => {
                let details =
                    parse_legacy_details::<CompartmentMutationDetails>(path, &kind, details)?;
                Ok(Self::CompartmentAdd {
                    label: details.label,
                    threshold: details.threshold,
                })
            }
            "compartment.remove" => {
                let details =
                    parse_legacy_details::<CompartmentRemoveDetails>(path, &kind, details)?;
                Ok(Self::CompartmentRemove { id: details.id })
            }
            "compartment.init" => {
                let details =
                    parse_legacy_details::<CompartmentMutationDetails>(path, &kind, details)?;
                Ok(Self::CompartmentInit {
                    label: details.label,
                    threshold: details.threshold,
                })
            }
            "compartment.switch" => {
                let details =
                    parse_legacy_details::<CompartmentSwitchDetails>(path, &kind, details)?;
                Ok(Self::CompartmentSwitch {
                    label: details.label,
                })
            }
            "unlock.passphrase" => {
                let details =
                    parse_legacy_details::<UnlockPassphraseDetails>(path, &kind, details)?;
                Ok(Self::UnlockPassphrase {
                    compartment_ids: details.compartment_ids,
                    count: details.count,
                })
            }
            "unlock.fido2" => {
                let details = parse_legacy_details::<UnlockFido2Details>(path, &kind, details)?;
                Ok(Self::UnlockFido2 {
                    compartment_ids: details.compartment_ids,
                    count: details.count,
                    tap_count: details.tap_count,
                })
            }
            "unlock.biometric" => {
                let details = parse_legacy_details::<UnlockBiometricDetails>(path, &kind, details)?;
                Ok(Self::UnlockBiometric {
                    compartment_id: details.compartment_id,
                    fingerprint_hex: details.fingerprint_hex,
                })
            }
            "biometric.enroll" => {
                let details = parse_legacy_details::<BiometricEnrollDetails>(path, &kind, details)?;
                Ok(Self::BiometricEnroll {
                    fingerprint_hex: details.fingerprint_hex,
                })
            }
            "lock.all" => Ok(Self::LockAll),
            "session.revoke" => Ok(Self::SessionRevoke),
            "profiles.evm_provider.upsert" => {
                let details =
                    parse_legacy_details::<ProfilesEvmProviderUpsertDetails>(path, &kind, details)?;
                Ok(Self::ProfilesEvmProviderUpsert {
                    name: details.name,
                    chain_id: details.chain_id,
                })
            }
            "profiles.evm_provider.delete" => {
                let details = parse_legacy_details::<NamedAuditDetails>(path, &kind, details)?;
                Ok(Self::ProfilesEvmProviderDelete { name: details.name })
            }
            "profiles.eth_stealth_wallet.upsert" => {
                let details = parse_legacy_details::<ProfilesEthStealthWalletUpsertDetails>(
                    path, &kind, details,
                )?;
                Ok(Self::ProfilesEthStealthWalletUpsert {
                    name: details.name,
                    provider_profile: details.provider_profile,
                })
            }
            "profiles.eth_stealth_wallet.delete" => {
                let details = parse_legacy_details::<NamedAuditDetails>(path, &kind, details)?;
                Ok(Self::ProfilesEthStealthWalletDelete { name: details.name })
            }
            "profiles.eth_xpub_wallet.upsert" => {
                let details = parse_legacy_details::<ProfilesEthXpubWalletUpsertDetails>(
                    path, &kind, details,
                )?;
                Ok(Self::ProfilesEthXpubWalletUpsert {
                    name: details.name,
                    provider_profile: details.provider_profile,
                })
            }
            "profiles.eth_xpub_wallet.delete" => {
                let details = parse_legacy_details::<NamedAuditDetails>(path, &kind, details)?;
                Ok(Self::ProfilesEthXpubWalletDelete { name: details.name })
            }
            "profiles.eth_seed_wallet.upsert" => {
                let details = parse_legacy_details::<ProfilesEthSeedWalletUpsertDetails>(
                    path, &kind, details,
                )?;
                Ok(Self::ProfilesEthSeedWalletUpsert {
                    name: details.name,
                    provider_profile: details.provider_profile,
                    word_count: details.word_count,
                })
            }
            "profiles.eth_seed_wallet.delete" => {
                let details = parse_legacy_details::<NamedAuditDetails>(path, &kind, details)?;
                Ok(Self::ProfilesEthSeedWalletDelete { name: details.name })
            }
            "snapshot.export" => {
                let details = parse_legacy_details::<SnapshotAuditDetails>(path, &kind, details)?;
                Ok(Self::SnapshotExport {
                    file_count: details.file_count,
                    total_bytes: details.total_bytes,
                })
            }
            "snapshot.restore" => {
                let details = parse_legacy_details::<SnapshotAuditDetails>(path, &kind, details)?;
                Ok(Self::SnapshotRestore {
                    file_count: details.file_count,
                    total_bytes: details.total_bytes,
                })
            }
            "fido2.setup" => {
                let details = parse_legacy_details::<Fido2SetupDetails>(path, &kind, details)?;
                Ok(Self::Fido2Setup {
                    label: details.label,
                    compartment_count: details.compartment_count,
                    total_keys: details.total_keys,
                })
            }
            "fido2.register_poison" => {
                let details = parse_legacy_details::<Fido2RegisterDetails>(path, &kind, details)?;
                Ok(Self::Fido2RegisterPoison {
                    label: details.label,
                    total_keys: details.total_keys,
                })
            }
            "fido2.register" => {
                let details = parse_legacy_details::<Fido2RegisterDetails>(path, &kind, details)?;
                Ok(Self::Fido2Register {
                    label: details.label,
                    total_keys: details.total_keys,
                })
            }
            "fido2.remove" => {
                let details = parse_legacy_details::<Fido2RemoveDetails>(path, &kind, details)?;
                Ok(Self::Fido2Remove {
                    label: details.label,
                    sessions_invalidated: details.sessions_invalidated,
                })
            }
            "queue.enqueue" => {
                let details = parse_legacy_details::<QueueEnqueueDetails>(path, &kind, details)?;
                Ok(Self::QueueEnqueue {
                    id: details.id,
                    job_kind: parse_queue_job_kind(path, &details.kind)?,
                })
            }
            "queue.process" => {
                let details = parse_legacy_details::<QueueProcessDetails>(path, &kind, details)?;
                Ok(Self::QueueProcess {
                    processed: details.processed,
                    succeeded: details.succeeded,
                    blocked: details.blocked,
                    retrying: details.retrying,
                    failed: details.failed,
                })
            }
            "transit.encrypt" => {
                let details = parse_legacy_details::<TransitEncryptDetails>(path, &kind, details)?;
                Ok(Self::TransitEncrypt {
                    key: details.key,
                    ciphertext_len: details.ciphertext_len,
                })
            }
            "transit.decrypt" => {
                let details = parse_legacy_details::<TransitDecryptDetails>(path, &kind, details)?;
                Ok(Self::TransitDecrypt {
                    key: details.key,
                    plaintext_len: details.plaintext_len,
                })
            }
            "transit.hmac" => {
                let details = parse_legacy_details::<TransitHmacDetails>(path, &kind, details)?;
                Ok(Self::TransitHmac {
                    key: details.key,
                    input_len: details.input_len,
                })
            }
            "evm.broadcast" => {
                let details = parse_legacy_details::<EvmBroadcastDetails>(path, &kind, details)?;
                Ok(Self::EvmBroadcast {
                    transaction_hash_hex: details.transaction_hash_hex,
                })
            }
            "wallet.eth_stealth.export" => {
                let details = parse_legacy_details::<WalletExportDetails>(path, &kind, details)?;
                Ok(Self::WalletEthStealthExport {
                    wallet: details.wallet,
                    short_name: details.short_name,
                })
            }
            "wallet.eth_xpub.export" => {
                let details =
                    parse_legacy_details::<WalletXpubExportDetails>(path, &kind, details)?;
                Ok(Self::WalletEthXpubExport {
                    wallet_profile: details.wallet_profile,
                    project_account: details.project_account,
                })
            }
            "wallet.eth_stealth.check" => {
                let details = parse_legacy_details::<WalletCheckDetails>(path, &kind, details)?;
                Ok(Self::WalletEthStealthCheck {
                    wallet: details.wallet,
                    matches: details.matches,
                })
            }
            "wallet.eth_stealth.sign" => {
                let details = parse_legacy_details::<WalletSignDetails>(path, &kind, details)?;
                Ok(Self::WalletEthStealthSign {
                    wallet: details.wallet,
                    stealth_address: details.stealth_address,
                })
            }
            "wallet.eth_stealth.sign_transfer" => {
                let details =
                    parse_legacy_details::<WalletSignedTransactionDetails>(path, &kind, details)?;
                Ok(Self::WalletEthStealthSignTransfer {
                    wallet: details.wallet,
                    transaction_kind: details.kind,
                    to: details.to,
                    nonce: details.nonce,
                })
            }
            "wallet.eth_stealth.sign_erc20_transfer" => {
                let details =
                    parse_legacy_details::<WalletSignedTransactionDetails>(path, &kind, details)?;
                Ok(Self::WalletEthStealthSignErc20Transfer {
                    wallet: details.wallet,
                    transaction_kind: details.kind,
                    to: details.to,
                    nonce: details.nonce,
                })
            }
            "wallet.eth_stealth.send_transfer" => {
                let details =
                    parse_legacy_details::<WalletSendTransactionDetails>(path, &kind, details)?;
                Ok(Self::WalletEthStealthSendTransfer {
                    wallet: details.wallet,
                    to: details.to,
                    nonce: details.nonce,
                    broadcast: details.broadcast,
                    transaction_hash_hex: details.transaction_hash_hex,
                    broadcast_transaction_hash_hex: details.broadcast_transaction_hash_hex,
                })
            }
            "wallet.eth_seed.send_transfer" => {
                let details =
                    parse_legacy_details::<WalletSendTransactionDetails>(path, &kind, details)?;
                Ok(Self::WalletEthSeedSendTransfer {
                    wallet: details.wallet,
                    to: details.to,
                    nonce: details.nonce,
                    broadcast: details.broadcast,
                    transaction_hash_hex: details.transaction_hash_hex,
                    broadcast_transaction_hash_hex: details.broadcast_transaction_hash_hex,
                })
            }
            "wallet.eth_stealth.send_erc20_transfer" => {
                let details =
                    parse_legacy_details::<WalletSendTransactionDetails>(path, &kind, details)?;
                Ok(Self::WalletEthStealthSendErc20Transfer {
                    wallet: details.wallet,
                    to: details.to,
                    nonce: details.nonce,
                    broadcast: details.broadcast,
                    transaction_hash_hex: details.transaction_hash_hex,
                    broadcast_transaction_hash_hex: details.broadcast_transaction_hash_hex,
                })
            }
            "deposits.eth_stealth.create" => {
                let details = parse_legacy_details::<DepositsCreateDetails>(path, &kind, details)?;
                Ok(Self::DepositsEthStealthCreate {
                    id: details.id,
                    wallet_profile: details.wallet_profile,
                    asset_kind: details.asset_kind,
                    token_address: details.token_address,
                })
            }
            "deposits.eth_stealth.delete" => {
                let details = parse_legacy_details::<IdOnlyDetails>(path, &kind, details)?;
                Ok(Self::DepositsEthStealthDelete { id: details.id })
            }
            "deposits.eth_stealth.refresh" => {
                let details = parse_legacy_details::<DepositsRefreshDetails>(path, &kind, details)?;
                Ok(Self::DepositsEthStealthRefresh {
                    processed: details.processed,
                    detected: details.detected,
                    queued: details.queued,
                })
            }
            "deposits.eth_stealth.enqueue_sweep" => {
                let details =
                    parse_legacy_details::<DepositEnqueueSweepDetails>(path, &kind, details)?;
                Ok(Self::DepositsEthStealthEnqueueSweep {
                    id: details.id,
                    job_id: details.job_id,
                })
            }
            "deposits.eth_stealth.announcement_scan" => {
                let details =
                    parse_legacy_details::<DepositsAnnouncementScanDetails>(path, &kind, details)?;
                Ok(Self::DepositsEthStealthAnnouncementScan {
                    wallet_profile: details.wallet_profile,
                    provider_profile: details.provider_profile,
                    scanned: details.scanned,
                    matched: details.matched,
                    created: details.created,
                })
            }
            "maintenance.run" => {
                let details = parse_legacy_details::<MaintenanceRunDetails>(path, &kind, details)?;
                Ok(Self::MaintenanceRun {
                    refreshed: details.refreshed,
                    detected: details.detected,
                    queued: details.queued,
                    processed: details.processed,
                    succeeded: details.succeeded,
                    blocked: details.blocked,
                    retrying: details.retrying,
                    failed: details.failed,
                })
            }
            "wallet_inventory.scan" => {
                let details =
                    parse_legacy_details::<WalletInventoryScanDetails>(path, &kind, details)?;
                Ok(Self::WalletInventoryScan {
                    id: details.id,
                    wallets: details.wallets,
                    providers: details.providers,
                    addresses: details.addresses,
                    holdings: details.holdings,
                })
            }
            "receiving.refresh_balances" => {
                let details =
                    parse_legacy_details::<ReceivingRefreshBalancesDetails>(path, &kind, details)?;
                Ok(Self::ReceivingRefreshBalances {
                    addresses_requested: details.addresses_requested,
                    addresses_refreshed: details.addresses_refreshed,
                    addresses_skipped: details.addresses_skipped,
                    stealth_refreshed: details.stealth_refreshed,
                })
            }
            "wallet_inventory.chain_profile.upsert" => {
                let details = parse_legacy_details::<WalletInventoryChainProfileUpsertDetails>(
                    path, &kind, details,
                )?;
                Ok(Self::WalletInventoryChainProfileUpsert {
                    name: details.name,
                    chain_family: details.chain_family,
                })
            }
            "wallet_inventory.chain_profile.delete" => {
                let details = parse_legacy_details::<WalletInventoryChainProfileDeleteDetails>(
                    path, &kind, details,
                )?;
                Ok(Self::WalletInventoryChainProfileDelete { name: details.name })
            }
            "wallet_inventory.discovery_job.update" => {
                let details = parse_legacy_details::<WalletInventoryDiscoveryJobUpdateDetails>(
                    path, &kind, details,
                )?;
                Ok(Self::WalletInventoryDiscoveryJobUpdate {
                    id: details.id,
                    status: details.status,
                })
            }
            "wallet_inventory.nft_metadata.opt_in.upsert" => {
                let details = parse_legacy_details::<WalletInventoryNftMetadataOptInDetails>(
                    path, &kind, details,
                )?;
                Ok(Self::WalletInventoryNftMetadataOptInUpsert {
                    chain_id: details.chain_id,
                    contract_address: details.contract_address,
                })
            }
            "wallet_inventory.nft_metadata.opt_in.delete" => {
                let details = parse_legacy_details::<WalletInventoryNftMetadataOptInDetails>(
                    path, &kind, details,
                )?;
                Ok(Self::WalletInventoryNftMetadataOptInDelete {
                    chain_id: details.chain_id,
                    contract_address: details.contract_address,
                })
            }
            "wallet_inventory.nft_metadata.settings.update" => {
                let details = parse_legacy_details::<
                    WalletInventoryNftMetadataSettingsUpdateDetails,
                >(path, &kind, details)?;
                Ok(Self::WalletInventoryNftMetadataSettingsUpdate {
                    ipfs_gateway_configured: details.ipfs_gateway_configured,
                })
            }
            "wallet_inventory.nft_metadata.fetch" => {
                let details = parse_legacy_details::<WalletInventoryNftMetadataFetchDetails>(
                    path, &kind, details,
                )?;
                Ok(Self::WalletInventoryNftMetadataFetch {
                    fetched: details.fetched,
                    skipped: details.skipped,
                })
            }
            "wallet_consolidation.plan.generate" => {
                let details = parse_legacy_details::<WalletConsolidationPlanGenerateDetails>(
                    path, &kind, details,
                )?;
                Ok(Self::WalletConsolidationPlanGenerate {
                    id: details.id,
                    steps: details.steps,
                    blocked: details.blocked,
                })
            }
            "wallet_consolidation.plan.approve" => {
                let details = parse_legacy_details::<WalletConsolidationPlanApproveDetails>(
                    path, &kind, details,
                )?;
                Ok(Self::WalletConsolidationPlanApprove {
                    id: details.id,
                    approved: details.approved,
                })
            }
            "wallet_consolidation.plan.simulate" => {
                let details = parse_legacy_details::<WalletConsolidationPlanSimulateDetails>(
                    path, &kind, details,
                )?;
                Ok(Self::WalletConsolidationPlanSimulate {
                    id: details.id,
                    passed: details.passed,
                    failed: details.failed,
                    unsupported: details.unsupported,
                })
            }
            "wallet_consolidation.plan.export" => {
                let details = parse_legacy_details::<WalletConsolidationPlanExportDetails>(
                    path, &kind, details,
                )?;
                Ok(Self::WalletConsolidationPlanExport {
                    id: details.id,
                    format: details.format,
                    exported: details.exported,
                    skipped: details.skipped,
                })
            }
            "treasury.party.create" => {
                let details = parse_legacy_details::<NamedAuditDetails>(path, &kind, details)?;
                Ok(Self::TreasuryPartyCreate { name: details.name })
            }
            "treasury.party.delete" => {
                let details = parse_legacy_details::<NamedAuditDetails>(path, &kind, details)?;
                Ok(Self::TreasuryPartyDelete { name: details.name })
            }
            "treasury.receive.bind" => {
                let details = parse_legacy_details::<NamedAuditDetails>(path, &kind, details)?;
                Ok(Self::TreasuryReceiveBind { name: details.name })
            }
            "run.complete" => {
                let details = parse_legacy_details::<RunCompleteDetails>(path, &kind, details)?;
                Ok(Self::RunComplete {
                    program: details.program,
                    args: details.args,
                    exit_code: details.exit_code,
                    signal: details.signal,
                    success: details.success,
                })
            }
            other => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "unsupported audit event kind {} in {}",
                    other,
                    path.display()
                ),
            )),
        }
    }
}

// Test-only legacy JSONL writer: fixture for audit/migration.rs tests and legacy-format regression tests; live writes go through audit_db::append_event_chained.
#[cfg(test)]
pub(crate) fn append_audit_event(
    path: &Path,
    event: &StoredAuditEvent,
) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let line = encode_json_document_compact(event)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(&line)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

// Test-only legacy JSONL reader: regression-tests the legacy decode fallback that audit/migration.rs relies on.
#[cfg(test)]
pub(crate) fn read_recent_audit_events(
    path: &Path,
    limit: usize,
) -> Result<Vec<PublicAuditEvent>, std::io::Error> {
    let limit = limit.max(1);
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut events = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let event: StoredAuditEvent = decode_json_document(path, line.as_bytes())?;
        events.push(event.to_public_event());
    }

    if events.len() > limit {
        events.drain(0..events.len() - limit);
    }
    events.reverse();
    Ok(events)
}

fn parse_legacy_details<T>(path: &Path, kind: &str, details: Value) -> Result<T, std::io::Error>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(details).map_err(|error| invalid_audit_data(path, kind, error))
}

fn parse_queue_job_kind(path: &Path, value: &str) -> Result<AuditQueueJobKind, std::io::Error> {
    match value {
        "eth_stealth_transfer" => Ok(AuditQueueJobKind::EthStealthTransfer),
        "eth_stealth_erc20_transfer" => Ok(AuditQueueJobKind::EthStealthErc20Transfer),
        "eth_stealth_native_sweep" => Ok(AuditQueueJobKind::EthStealthNativeSweep),
        "eth_stealth_erc20_sweep" => Ok(AuditQueueJobKind::EthStealthErc20Sweep),
        other => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "unsupported queue audit job kind {} in {}",
                other,
                path.display()
            ),
        )),
    }
}

fn invalid_audit_data(path: &Path, kind: &str, error: serde_json::Error) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!(
            "failed to parse audit event {} in {}: {error}",
            kind,
            path.display()
        ),
    )
}

#[cfg(test)]
mod tests;
