//! # Sigillum Client
//!
//! Async HTTP client for the local Sigillum daemon API.
//!
//! [`SigillumClient`] wraps a `reqwest::Client` and maintains a session token
//! that is adopted only by validated unlock/setup/switch responses and
//! injected as a Bearer token on subsequent requests. Session changes use a
//! compare-and-swap boundary, so a late response cannot overwrite or clear a
//! newer concurrent session.
//!
//! ## Authentication flow
//!
//! 1. Create a client with [`SigillumClient::new`] and handle any HTTP-client
//!    construction error.
//! 2. Call [`SigillumClient::unlock_with_passphrase`] or
//!    [`SigillumClient::fido2_unlock`] — the returned session token is stored
//!    automatically.
//! 3. All subsequent calls attach the token as `Authorization: Bearer <token>`.
//! 4. The token is cleared when the daemon responds with 401, or explicitly
//!    via [`SigillumClient::lock`] / [`SigillumClient::revoke_session`].
//!
//! ## Binary data
//!
//! The daemon communicates binary payloads as hex-encoded strings.  Methods
//! that accept or return raw bytes (`transit_encrypt`, `export_snapshot`, etc.)
//! handle hex encoding/decoding internally so callers work with `&[u8]` /
//! `Vec<u8>` directly.
//!
//! ## Thread safety
//!
//! `SigillumClient` is `Send + Sync`. An async transition gate serializes
//! token-changing operations against ordinary requests, while the token itself
//! is protected by a `std::sync::Mutex`. If a panic poisons that mutex, the
//! client explicitly clears the cached token and poison flag, restoring the
//! safe logged-out invariant before any later request can proceed.

use std::sync::atomic::{AtomicU8, AtomicU64};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Method;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use sigillum_api::request::{
    DiscoveryJobMutationRequest, EthSeedWalletCreateRequest, EthSeedWalletProfileUpsertRequest,
    EthStealthAnnouncementScanRequest, EthStealthCheckRequest, EthStealthDepositCreateErc20Request,
    EthStealthDepositCreateNativeRequest, EthStealthDepositDeleteRequest,
    EthStealthDepositEnqueueSweepRequest, EthStealthDepositRefreshRequest, EthStealthExportRequest,
    EthStealthGenerateRequest, EthStealthSendErc20TransferRequest,
    EthStealthSendErc20WithProfileRequest, EthStealthSendTransferRequest,
    EthStealthSendWithProfileRequest, EthStealthSignErc20TransferRequest, EthStealthSignRequest,
    EthStealthSignTransferRequest, EthStealthWalletProfileUpsertRequest, EthXpubDeriveRequest,
    EthXpubExportRequest, EthXpubWalletProfileUpsertRequest, EvmFeeEstimateRequest,
    EvmProfileDeleteRequest, EvmProviderProfileUpsertRequest, EvmRpcBalanceRequest,
    EvmRpcBroadcastRequest, EvmRpcErc20BalanceRequest, EvmRpcNonceRequest, GenerateStoreRequest,
    KeyOnlyRequest, KeyValueRequest, MaintenanceRunRequest, PassphraseRequest,
    RiskCatalogDeleteRequest, RiskCatalogUpsertRequest, RunAuditRequest, SecretResolveBatchRequest,
    SnapshotRestoreRequest, StealthPaymentRef, TransitDecryptRequest, TransitEncryptRequest,
    TransitHmacRequest,
};
pub use sigillum_api::response::Fido2StatusResponse as DaemonFido2Status;
pub use sigillum_api::response::{
    ActiveCompartment, AuditEvent as DaemonAuditEvent, AuditVerifyReport,
    BiometricChallengeResponse, BiometricEnrollResponse, ChainProfile, ChainProfileListResponse,
    ChainProfileMutationResponse, CompartmentInfo, CompartmentListResponse, ConsolidationPlan,
    ConsolidationPlanExportBundle, ConsolidationPlanExportCall, ConsolidationPlanExportResponse,
    ConsolidationPlanExportSkippedStep, ConsolidationPlanListResponse,
    ConsolidationPlanMutationResponse, ConsolidationPlanStep, ConsolidationPlanSummary,
    DiagnosticsResponse, DiscoveryJobListResponse, DiscoveryJobMutationResponse, ErrorResponse,
    EthSeedWalletCreateResponse, EthSeedWalletProfile, EthSeedWalletProfileListResponse,
    EthSeedWalletProfileMutationResponse, EthSignedTransactionResponse,
    EthStealthAnnouncementScanResponse, EthStealthCheckResponse, EthStealthDeposit,
    EthStealthDepositEnqueueSweepResponse, EthStealthDepositListResponse,
    EthStealthDepositMutationResponse, EthStealthDepositRefreshResponse,
    EthStealthGenerateResponse, EthStealthMetaAddressResponse, EthStealthSendResponse,
    EthStealthSignResponse, EthStealthWalletProfile, EthStealthWalletProfileListResponse,
    EthStealthWalletProfileMutationResponse, EthXpubAddressResponse, EthXpubExportResponse,
    EthXpubWalletProfile, EthXpubWalletProfileListResponse, EthXpubWalletProfileMutationResponse,
    EvmFeeEstimateResponse, EvmProviderProfile, EvmProviderProfileListResponse,
    EvmProviderProfileMutationResponse, EvmRpcBalanceResponse, EvmRpcBroadcastResponse,
    EvmRpcErc20BalanceResponse, EvmRpcNonceResponse, Fido2DetectResponse, Fido2KeyInfo,
    Fido2ListResponse, Fido2RegisterResponse, Fido2RemoveResponse, Fido2SetupResponse,
    Fido2StatusResponse, GenerateStoreResponse, GenericStatusResponse, KeyListResponse,
    KeyValueResponse, MaintenanceRunResponse, QueueEnqueueResponse, QueueExecutionPauseResponse,
    QueueJob, QueueJobListResponse, QueueProcessResponse, RiskCatalogEntry,
    RiskCatalogListResponse, RiskCatalogMutationResponse, RiskFinding, RiskFindingListResponse,
    SafeTransactionBuilderBatch, SafeTransactionBuilderMeta, SafeTransactionBuilderTransaction,
    SecretResolveBatchResponse, SecretResolveValue, SelfCheckResult, SelfCheckRunResponse,
    SessionRevokeResponse, SnapshotExportResponse, SnapshotRestoreResponse, StatusResponse,
    SwitchCompartmentResponse, TransitDecryptResponse, TransitEncryptResponse, TransitHmacResponse,
    UnlockResponse, UnlockedCompartment, WalletAssetHolding, WalletDiscoveryJob,
    WalletInventoryAddress, WalletInventoryListResponse, WalletInventoryScanResponse,
    WatchAddressBookListResponse, WatchAddressBookMutationResponse,
};
use sigillum_core::SnapshotSummary;
use thiserror::Error;

pub use sigillum_core::{SecretStore, VaultError};

mod inventory;
mod plans;
mod queue;
mod receiving;
mod selfcheck;
mod session;
mod treasury;

// ── Error types ────────────────────────────────────────────────

/// Errors that can occur when communicating with the Sigillum daemon.
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("api error ({status}): {message}")]
    Api {
        status: reqwest::StatusCode,
        message: String,
    },

    #[error("invalid snapshot encoding: {0}")]
    SnapshotEncoding(String),

    #[error("invalid response encoding: {0}")]
    Encoding(String),

    #[error("session context changed while the daemon request was in flight")]
    SessionContextChanged,

    #[error("invalid session transition response: {0}")]
    InvalidSessionTransition(String),

    #[error("session transition was ambiguous; fallback Lock confirmed: {0}")]
    SessionTransitionLocked(String),

    #[error(
        "session transition was ambiguous and Lock could not be confirmed; stop/restart the daemon before continuing: {0}"
    )]
    SessionTransitionLockUnconfirmed(String),

    #[error(
        "session state is fail-closed after an unconfirmed Lock boundary; stop/restart the daemon and construct a new client"
    )]
    SessionStateUnconfirmed,
}

/// Async HTTP client for the Sigillum vault daemon.
///
/// Holds a shared `reqwest::Client`, a normalized base URL, and an
/// auto-managed session token. Clones share the same session token and
/// transition gate. All public methods return `Result<T, ClientError>`.
#[derive(Clone)]
pub struct SigillumClient {
    http: reqwest::Client,
    base_url: String,
    session_token: Arc<Mutex<Option<String>>>,
    session_transition: Arc<tokio::sync::RwLock<()>>,
    session_lock_state: Arc<AtomicU8>,
    session_boundary_generation: Arc<AtomicU64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditEventQuery {
    pub tail: Option<usize>,
    pub kind: Option<String>,
    pub since: Option<u64>,
    pub key: Option<String>,
}

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

impl SigillumClient {
    // ── Construction ───────────────────────────────────────────

    /// Create a client pointing at `base_url` (e.g. `http://127.0.0.1:3200`).
    ///
    /// Returns an error if the underlying HTTP client cannot be built.
    pub fn new(base_url: impl Into<String>) -> Result<Self, ClientError> {
        Ok(Self {
            http: reqwest::Client::builder()
                .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
                .timeout(DEFAULT_REQUEST_TIMEOUT)
                .build()?,
            base_url: normalize_base_url(base_url.into()),
            session_token: Arc::new(Mutex::new(None)),
            session_transition: Arc::new(tokio::sync::RwLock::new(())),
            session_lock_state: Arc::new(AtomicU8::new(0)),
            session_boundary_generation: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Create a client with a pre-configured `reqwest::Client` (custom timeouts, TLS, etc.).
    pub fn with_http_client(base_url: impl Into<String>, http: reqwest::Client) -> Self {
        Self {
            http,
            base_url: normalize_base_url(base_url.into()),
            session_token: Arc::new(Mutex::new(None)),
            session_transition: Arc::new(tokio::sync::RwLock::new(())),
            session_lock_state: Arc::new(AtomicU8::new(0)),
            session_boundary_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    // ── Secrets & API keys ──────────────────────────────────────

    pub async fn list_api_keys(&self) -> Result<Vec<String>, ClientError> {
        let builder = self.request(Method::GET, "/api/api-keys");
        Ok(self.send::<KeyListResponse>(builder).await?.keys)
    }

    pub async fn get_api_key(&self, key: &str) -> Result<SecretString, ClientError> {
        let builder = self
            .request(Method::POST, "/api/api-keys/get")
            .json(&KeyOnlyRequest {
                key: key.to_string(),
            });
        let response: KeyValueResponse = self.send(builder).await?;
        Ok(SecretString::from(response.value))
    }

    pub async fn set_api_key(&self, key: &str, value: &str) -> Result<(), ClientError> {
        let builder = self
            .request(Method::POST, "/api/api-keys/set")
            .json(&KeyValueRequest {
                key: key.to_string(),
                value: Some(value.to_string()),
            });
        let _: GenericStatusResponse = self.send(builder).await?;
        Ok(())
    }

    pub async fn delete_api_key(&self, key: &str) -> Result<(), ClientError> {
        let builder = self
            .request(Method::POST, "/api/api-keys/delete")
            .json(&KeyOnlyRequest {
                key: key.to_string(),
            });
        let _: GenericStatusResponse = self.send(builder).await?;
        Ok(())
    }

    pub async fn list_secrets(&self) -> Result<Vec<String>, ClientError> {
        let builder = self.request(Method::GET, "/api/secrets");
        Ok(self.send::<KeyListResponse>(builder).await?.keys)
    }

    pub async fn get_secret(&self, key: &str) -> Result<SecretString, ClientError> {
        let builder = self
            .request(Method::POST, "/api/secrets/get")
            .json(&KeyOnlyRequest {
                key: key.to_string(),
            });
        let response: KeyValueResponse = self.send(builder).await?;
        Ok(SecretString::from(response.value))
    }

    pub async fn set_secret(&self, key: &str, value: &str) -> Result<(), ClientError> {
        let builder = self
            .request(Method::POST, "/api/secrets/set")
            .json(&KeyValueRequest {
                key: key.to_string(),
                value: Some(value.to_string()),
            });
        let _: GenericStatusResponse = self.send(builder).await?;
        Ok(())
    }

    pub async fn delete_secret(&self, key: &str) -> Result<(), ClientError> {
        let builder = self
            .request(Method::POST, "/api/secrets/delete")
            .json(&KeyOnlyRequest {
                key: key.to_string(),
            });
        let _: GenericStatusResponse = self.send(builder).await?;
        Ok(())
    }

    pub async fn resolve_secret_batch(
        &self,
        request: SecretResolveBatchRequest,
    ) -> Result<Vec<SecretResolveValue>, ClientError> {
        let builder = self
            .request(Method::POST, "/api/secrets/resolve-batch")
            .json(&request);
        Ok(self
            .send::<SecretResolveBatchResponse>(builder)
            .await?
            .values)
    }

    pub async fn record_run_audit(
        &self,
        request: RunAuditRequest,
    ) -> Result<GenericStatusResponse, ClientError> {
        let builder = self.request(Method::POST, "/api/audit/run").json(&request);
        self.send(builder).await
    }

    pub async fn generate_and_store(
        &self,
        request: GenerateStoreRequest,
    ) -> Result<GenerateStoreResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/generate/store")
            .json(&request);
        self.send(builder).await
    }

    // ── Backup & restore ─────────────────────────────────────────

    /// Export an encrypted vault snapshot, returning raw bytes and metadata.
    pub async fn export_snapshot(
        &self,
        passphrase: &str,
    ) -> Result<(Vec<u8>, SnapshotSummary), ClientError> {
        let builder = self
            .request(Method::POST, "/api/backup/export")
            .json(&PassphraseRequest {
                passphrase: passphrase.to_string(),
            });
        let response: SnapshotExportResponse = self.send(builder).await?;
        let bytes = hex::decode(response.snapshot_hex)
            .map_err(|e| ClientError::SnapshotEncoding(e.to_string()))?;
        Ok((bytes, response.summary))
    }

    pub async fn restore_snapshot(
        &self,
        passphrase: &str,
        snapshot: &[u8],
    ) -> Result<SnapshotRestoreResponse, ClientError> {
        let boundary_generation = self.session_boundary_generation();
        self.ensure_session_requests_allowed()?;
        let _transition = self.acquire_session_transition("snapshot restore").await?;
        self.ensure_session_requests_allowed()?;
        self.ensure_session_boundary_generation(boundary_generation)?;
        let builder =
            self.request(Method::POST, "/api/backup/restore")
                .json(&SnapshotRestoreRequest {
                    passphrase: passphrase.to_string(),
                    snapshot_hex: hex::encode(snapshot),
                });
        let (response, expected_token): (SnapshotRestoreResponse, _) =
            self.send_with_session_context_guarded(builder).await?;
        self.ensure_session_requests_allowed()?;
        self.ensure_session_boundary_generation(boundary_generation)?;
        if response.status != "restored" || !response.requires_reauth {
            return Err(ClientError::InvalidSessionTransition(format!(
                "snapshot restore returned status {:?} with requires_reauth={}",
                response.status, response.requires_reauth
            )));
        }
        self.clear_session_token_if_current(expected_token.as_deref())?;
        Ok(response)
    }

    // ── Observability ───────────────────────────────────────────

    pub async fn audit_events(&self, limit: usize) -> Result<Vec<DaemonAuditEvent>, ClientError> {
        self.audit_events_query(AuditEventQuery {
            tail: Some(limit.max(1)),
            ..AuditEventQuery::default()
        })
        .await
    }

    pub async fn audit_events_query(
        &self,
        query: AuditEventQuery,
    ) -> Result<Vec<DaemonAuditEvent>, ClientError> {
        let mut path = format!("/api/audit?tail={}", query.tail.unwrap_or(25).max(1));
        if let Some(kind) = query.kind {
            path.push_str("&kind=");
            path.push_str(&urlencoding::encode(&kind));
        }
        if let Some(since) = query.since {
            path.push_str("&since=");
            path.push_str(&since.to_string());
        }
        if let Some(key) = query.key {
            path.push_str("&key=");
            path.push_str(&urlencoding::encode(&key));
        }

        let builder = self.request(Method::GET, &path);
        Ok(self
            .send::<sigillum_api::response::AuditResponse>(builder)
            .await?
            .events)
    }

    pub async fn audit_verify(
        &self,
        scope: Option<&str>,
    ) -> Result<AuditVerifyReport, ClientError> {
        let mut path = "/api/audit/verify".to_string();
        if let Some(scope) = scope {
            path.push_str("?scope=");
            path.push_str(&urlencoding::encode(scope));
        }
        let builder = self.request(Method::GET, &path);
        self.send(builder).await
    }

    pub async fn diagnostics(&self) -> Result<DiagnosticsResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/diagnostics");
        self.send(builder).await
    }

    // ── Transit encryption ──────────────────────────────────────

    /// Encrypt `plaintext` with a named vault key, returning `(nonce, ciphertext)`.
    pub async fn transit_encrypt(
        &self,
        key: &str,
        plaintext: &[u8],
        aad: Option<&[u8]>,
    ) -> Result<(Vec<u8>, Vec<u8>), ClientError> {
        let builder =
            self.request(Method::POST, "/api/transit/encrypt")
                .json(&TransitEncryptRequest {
                    key: key.to_string(),
                    plaintext_hex: hex::encode(plaintext),
                    aad_hex: aad.map(hex::encode),
                });
        let response: TransitEncryptResponse = self.send(builder).await?;
        let nonce = Self::decode_hex(&response.nonce_hex)?;
        let ciphertext = Self::decode_hex(&response.ciphertext_hex)?;
        Ok((nonce, ciphertext))
    }

    pub async fn transit_decrypt(
        &self,
        key: &str,
        nonce: &[u8],
        ciphertext: &[u8],
        aad: Option<&[u8]>,
    ) -> Result<Vec<u8>, ClientError> {
        let builder =
            self.request(Method::POST, "/api/transit/decrypt")
                .json(&TransitDecryptRequest {
                    key: key.to_string(),
                    nonce_hex: hex::encode(nonce),
                    ciphertext_hex: hex::encode(ciphertext),
                    aad_hex: aad.map(hex::encode),
                });
        let response: TransitDecryptResponse = self.send(builder).await?;
        Self::decode_hex(&response.plaintext_hex)
    }

    pub async fn transit_hmac(&self, key: &str, input: &[u8]) -> Result<Vec<u8>, ClientError> {
        let builder = self
            .request(Method::POST, "/api/transit/hmac")
            .json(&TransitHmacRequest {
                key: key.to_string(),
                input_hex: hex::encode(input),
            });
        let response: TransitHmacResponse = self.send(builder).await?;
        Self::decode_hex(&response.digest_hex)
    }

    // ── Ethereum stealth wallets ─────────────────────────────────

    pub async fn export_eth_stealth_meta_address(
        &self,
        wallet: &str,
        short_name: Option<&str>,
    ) -> Result<EthStealthMetaAddressResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/wallets/eth-stealth/export")
            .json(&EthStealthExportRequest {
                wallet: wallet.to_string(),
                short_name: short_name.map(str::to_owned),
            });
        self.send(builder).await
    }

    pub async fn generate_eth_stealth_address(
        &self,
        stealth_meta_address: &str,
        ephemeral_private_key: Option<&[u8; 32]>,
    ) -> Result<EthStealthGenerateResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/wallets/eth-stealth/generate")
            .json(&EthStealthGenerateRequest {
                stealth_meta_address: stealth_meta_address.to_string(),
                ephemeral_private_key_hex: ephemeral_private_key.map(hex::encode),
            });
        self.send(builder).await
    }

    pub async fn export_eth_xpub_receive_branch(
        &self,
        wallet_profile: &str,
    ) -> Result<EthXpubExportResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/wallets/eth-xpub/export")
            .json(&EthXpubExportRequest {
                wallet_profile: wallet_profile.to_string(),
            });
        self.send(builder).await
    }

    pub async fn derive_eth_xpub_receive_address(
        &self,
        xpub: &str,
        index: u32,
    ) -> Result<EthXpubAddressResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/wallets/eth-xpub/derive")
            .json(&EthXpubDeriveRequest {
                xpub: xpub.to_string(),
                index,
            });
        self.send(builder).await
    }

    pub async fn check_eth_stealth_address(
        &self,
        wallet: &str,
        stealth_address: &str,
        ephemeral_public_key: &[u8],
        view_tag: Option<u8>,
    ) -> Result<EthStealthCheckResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/wallets/eth-stealth/check")
            .json(&EthStealthCheckRequest {
                wallet: wallet.to_string(),
                stealth: StealthPaymentRef {
                    stealth_address: stealth_address.to_string(),
                    ephemeral_public_key_hex: hex::encode(ephemeral_public_key),
                    view_tag_hex: view_tag.map(|value| hex::encode([value])),
                },
            });
        self.send(builder).await
    }

    pub async fn sign_eth_stealth_digest(
        &self,
        wallet: &str,
        stealth_address: &str,
        ephemeral_public_key: &[u8],
        digest: &[u8; 32],
        view_tag: Option<u8>,
    ) -> Result<EthStealthSignResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/wallets/eth-stealth/sign")
            .json(&EthStealthSignRequest {
                wallet: wallet.to_string(),
                stealth: StealthPaymentRef {
                    stealth_address: stealth_address.to_string(),
                    ephemeral_public_key_hex: hex::encode(ephemeral_public_key),
                    view_tag_hex: view_tag.map(|value| hex::encode([value])),
                },
                digest_hex: hex::encode(digest),
            });
        self.send(builder).await
    }

    pub async fn sign_eth_stealth_transfer(
        &self,
        request: EthStealthSignTransferRequest,
    ) -> Result<EthSignedTransactionResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/wallets/eth-stealth/sign-transfer")
            .json(&request);
        self.send(builder).await
    }

    pub async fn sign_eth_stealth_erc20_transfer(
        &self,
        request: EthStealthSignErc20TransferRequest,
    ) -> Result<EthSignedTransactionResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/wallets/eth-stealth/sign-erc20-transfer")
            .json(&request);
        self.send(builder).await
    }

    // ── EVM RPC ────────────────────────────────────────────────

    pub async fn evm_nonce(
        &self,
        request: EvmRpcNonceRequest,
    ) -> Result<EvmRpcNonceResponse, ClientError> {
        let builder = self.request(Method::POST, "/api/evm/nonce").json(&request);
        self.send(builder).await
    }

    pub async fn evm_balance(
        &self,
        request: EvmRpcBalanceRequest,
    ) -> Result<EvmRpcBalanceResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/evm/balance")
            .json(&request);
        self.send(builder).await
    }

    pub async fn evm_erc20_balance(
        &self,
        request: EvmRpcErc20BalanceRequest,
    ) -> Result<EvmRpcErc20BalanceResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/evm/erc20-balance")
            .json(&request);
        self.send(builder).await
    }

    pub async fn evm_estimate_fees(
        &self,
        request: EvmFeeEstimateRequest,
    ) -> Result<EvmFeeEstimateResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/evm/fees/estimate")
            .json(&request);
        self.send(builder).await
    }

    pub async fn evm_broadcast(
        &self,
        request: EvmRpcBroadcastRequest,
    ) -> Result<EvmRpcBroadcastResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/evm/broadcast")
            .json(&request);
        self.send(builder).await
    }

    // ── Send transactions ───────────────────────────────────────

    pub async fn send_eth_stealth_transfer(
        &self,
        request: EthStealthSendTransferRequest,
    ) -> Result<EthStealthSendResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/wallets/eth-stealth/send-transfer")
            .json(&request);
        self.send(builder).await
    }

    pub async fn send_eth_stealth_erc20_transfer(
        &self,
        request: EthStealthSendErc20TransferRequest,
    ) -> Result<EthStealthSendResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/wallets/eth-stealth/send-erc20-transfer")
            .json(&request);
        self.send(builder).await
    }

    // ── Profiles ────────────────────────────────────────────────

    pub async fn list_evm_provider_profiles(&self) -> Result<Vec<EvmProviderProfile>, ClientError> {
        let builder = self.request(Method::GET, "/api/profiles/evm");
        Ok(self
            .send::<EvmProviderProfileListResponse>(builder)
            .await?
            .profiles)
    }

    pub async fn upsert_evm_provider_profile(
        &self,
        request: EvmProviderProfileUpsertRequest,
    ) -> Result<EvmProviderProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/evm/upsert")
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_evm_provider_profile(
        &self,
        name: &str,
    ) -> Result<EvmProviderProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/evm/delete")
            .json(&EvmProfileDeleteRequest { name: name.into() });
        self.send(builder).await
    }

    pub async fn list_eth_stealth_wallet_profiles(
        &self,
    ) -> Result<Vec<EthStealthWalletProfile>, ClientError> {
        let builder = self.request(Method::GET, "/api/profiles/eth-stealth");
        Ok(self
            .send::<EthStealthWalletProfileListResponse>(builder)
            .await?
            .profiles)
    }

    pub async fn list_eth_xpub_wallet_profiles(
        &self,
    ) -> Result<Vec<EthXpubWalletProfile>, ClientError> {
        let builder = self.request(Method::GET, "/api/profiles/eth-xpub");
        Ok(self
            .send::<EthXpubWalletProfileListResponse>(builder)
            .await?
            .profiles)
    }

    pub async fn list_eth_seed_wallet_profiles(
        &self,
    ) -> Result<Vec<EthSeedWalletProfile>, ClientError> {
        let builder = self.request(Method::GET, "/api/profiles/eth-seed");
        Ok(self
            .send::<EthSeedWalletProfileListResponse>(builder)
            .await?
            .profiles)
    }

    pub async fn upsert_eth_stealth_wallet_profile(
        &self,
        request: EthStealthWalletProfileUpsertRequest,
    ) -> Result<EthStealthWalletProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/eth-stealth/upsert")
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_eth_stealth_wallet_profile(
        &self,
        name: &str,
    ) -> Result<EthStealthWalletProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/eth-stealth/delete")
            .json(&EvmProfileDeleteRequest { name: name.into() });
        self.send(builder).await
    }

    pub async fn upsert_eth_xpub_wallet_profile(
        &self,
        request: EthXpubWalletProfileUpsertRequest,
    ) -> Result<EthXpubWalletProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/eth-xpub/upsert")
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_eth_xpub_wallet_profile(
        &self,
        name: &str,
    ) -> Result<EthXpubWalletProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/eth-xpub/delete")
            .json(&EvmProfileDeleteRequest { name: name.into() });
        self.send(builder).await
    }

    pub async fn upsert_eth_seed_wallet_profile(
        &self,
        request: EthSeedWalletProfileUpsertRequest,
    ) -> Result<EthSeedWalletProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/eth-seed/upsert")
            .json(&request);
        self.send(builder).await
    }

    /// Create a brand-new seed wallet profile from a daemon-generated BIP-39
    /// mnemonic.
    ///
    /// The returned [`EthSeedWalletCreateResponse::mnemonic`] is delivered
    /// exactly once for operator backup; the daemon keeps it only as an
    /// encrypted vault secret and never audits it.
    pub async fn create_eth_seed_wallet_profile(
        &self,
        request: EthSeedWalletCreateRequest,
    ) -> Result<EthSeedWalletCreateResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/eth-seed/create")
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_eth_seed_wallet_profile(
        &self,
        name: &str,
    ) -> Result<EthSeedWalletProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/eth-seed/delete")
            .json(&EvmProfileDeleteRequest { name: name.into() });
        self.send(builder).await
    }

    pub async fn send_eth_stealth_with_profile(
        &self,
        request: EthStealthSendWithProfileRequest,
    ) -> Result<EthStealthSendResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/wallets/eth-stealth/send-with-profile")
            .json(&request);
        self.send(builder).await
    }

    pub async fn send_eth_stealth_erc20_with_profile(
        &self,
        request: EthStealthSendErc20WithProfileRequest,
    ) -> Result<EthStealthSendResponse, ClientError> {
        let builder = self
            .request(
                Method::POST,
                "/api/wallets/eth-stealth/send-erc20-with-profile",
            )
            .json(&request);
        self.send(builder).await
    }

    pub async fn list_discovery_jobs(&self) -> Result<DiscoveryJobListResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/discovery/jobs");
        self.send(builder).await
    }

    pub async fn cancel_discovery_job(
        &self,
        id: &str,
    ) -> Result<DiscoveryJobMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/discovery/jobs/cancel")
            .json(&DiscoveryJobMutationRequest { id: id.into() });
        self.send(builder).await
    }

    pub async fn resume_discovery_job(
        &self,
        id: &str,
    ) -> Result<DiscoveryJobMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/discovery/jobs/resume")
            .json(&DiscoveryJobMutationRequest { id: id.into() });
        self.send(builder).await
    }

    pub async fn list_risk_findings(&self) -> Result<RiskFindingListResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/risk/findings");
        self.send(builder).await
    }

    pub async fn list_risk_catalog(&self) -> Result<RiskCatalogListResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/risk/catalog");
        self.send(builder).await
    }

    pub async fn upsert_risk_catalog_entry(
        &self,
        request: RiskCatalogUpsertRequest,
    ) -> Result<RiskCatalogMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/risk/catalog/upsert")
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_risk_catalog_entry(
        &self,
        request: RiskCatalogDeleteRequest,
    ) -> Result<RiskCatalogMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/risk/catalog/delete")
            .json(&request);
        self.send(builder).await
    }

    // ── Deposits ────────────────────────────────────────────────

    pub async fn list_eth_stealth_deposits(&self) -> Result<Vec<EthStealthDeposit>, ClientError> {
        let builder = self.request(Method::GET, "/api/deposits/eth-stealth");
        Ok(self
            .send::<EthStealthDepositListResponse>(builder)
            .await?
            .deposits)
    }

    pub async fn create_eth_stealth_native_deposit(
        &self,
        request: EthStealthDepositCreateNativeRequest,
    ) -> Result<EthStealthDepositMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/deposits/eth-stealth/create-native")
            .json(&request);
        self.send(builder).await
    }

    pub async fn create_eth_stealth_erc20_deposit(
        &self,
        request: EthStealthDepositCreateErc20Request,
    ) -> Result<EthStealthDepositMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/deposits/eth-stealth/create-erc20")
            .json(&request);
        self.send(builder).await
    }

    pub async fn scan_eth_stealth_announcements(
        &self,
        request: EthStealthAnnouncementScanRequest,
    ) -> Result<EthStealthAnnouncementScanResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/deposits/eth-stealth/scan-announcements")
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_eth_stealth_deposit(
        &self,
        request: EthStealthDepositDeleteRequest,
    ) -> Result<EthStealthDepositMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/deposits/eth-stealth/delete")
            .json(&request);
        self.send(builder).await
    }

    pub async fn refresh_eth_stealth_deposits(
        &self,
        request: EthStealthDepositRefreshRequest,
    ) -> Result<EthStealthDepositRefreshResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/deposits/eth-stealth/refresh")
            .json(&request);
        self.send(builder).await
    }

    pub async fn enqueue_eth_stealth_deposit_sweep(
        &self,
        request: EthStealthDepositEnqueueSweepRequest,
    ) -> Result<EthStealthDepositEnqueueSweepResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/deposits/eth-stealth/enqueue-sweep")
            .json(&request);
        self.send(builder).await
    }

    // ── Maintenance ─────────────────────────────────────────────

    pub async fn run_maintenance(
        &self,
        request: MaintenanceRunRequest,
    ) -> Result<MaintenanceRunResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/maintenance/run")
            .json(&request);
        self.send(builder).await
    }

    // ── Internal helpers ─────────────────────────────────────────

    /// Decode a hex-encoded response field into raw bytes.
    fn decode_hex(field: &str) -> Result<Vec<u8>, ClientError> {
        hex::decode(field).map_err(|error| ClientError::Encoding(error.to_string()))
    }

    pub(crate) fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        let mut builder = self
            .http
            .request(method, format!("{}{}", self.base_url, path));
        if let Some(token) = self.session_token() {
            builder = builder.bearer_auth(token);
        }
        builder
    }

    /// Send a request and deserialize the JSON response.
    ///
    /// Session-token adoption is deliberately not generic: only the public
    /// unlock/setup/switch methods validate their typed transition response
    /// and compare-and-swap its token into the client. A 401 clears only the
    /// exact token carried by this request, never a newer concurrent session.
    pub(crate) async fn send<T: DeserializeOwned>(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<T, ClientError> {
        let boundary_generation = self.session_boundary_generation();
        self.ensure_session_requests_allowed()?;
        let _transition = self.session_transition.read().await;
        self.ensure_session_requests_allowed()?;
        self.ensure_session_boundary_generation(boundary_generation)?;
        let (response, _) = self.send_with_session_context_guarded(builder).await?;
        self.ensure_session_requests_allowed()?;
        self.ensure_session_boundary_generation(boundary_generation)?;
        Ok(response)
    }

    async fn send_with_session_context_guarded<T: DeserializeOwned>(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<(T, Option<String>), ClientError> {
        let request = builder.build()?;
        let expected_token = request_session_token(&request);
        if expected_token != self.session_token() {
            return Err(ClientError::SessionContextChanged);
        }
        let decoded = self
            .send_built_with_session_context(request, expected_token.as_deref())
            .await?;
        Ok((decoded, expected_token))
    }

    async fn send_built_with_session_context<T: DeserializeOwned>(
        &self,
        request: reqwest::Request,
        expected_token: Option<&str>,
    ) -> Result<T, ClientError> {
        self.ensure_session_requests_allowed()?;
        let response = self.http.execute(request).await?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::LOCKED {
            if !self.replace_session_token_if_current(expected_token, None) {
                return Err(ClientError::SessionContextChanged);
            }
            let text = response.text().await?;
            return Err(ClientError::Api {
                status,
                message: api_error_message(status, &text),
            });
        }

        let text = response.text().await?;
        let value = if text.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str::<serde_json::Value>(&text)?
        };

        if !status.is_success() {
            let message = serde_json::from_value::<ErrorResponse>(value.clone())
                .map(|error| error.error)
                .unwrap_or_else(|_| api_error_message(status, &text));
            return Err(ClientError::Api { status, message });
        }

        // `set_session_token` remains a synchronous public escape hatch and
        // therefore cannot participate in the async transition gate. Never
        // return a successful response if that explicit owner changed while
        // the request was in flight.
        if self.session_token().as_deref() != expected_token {
            return Err(ClientError::SessionContextChanged);
        }

        let decoded = serde_json::from_value(value)?;
        Ok(decoded)
    }
}

fn api_error_message(status: reqwest::StatusCode, text: &str) -> String {
    serde_json::from_str::<ErrorResponse>(text)
        .map(|error| error.error)
        .unwrap_or_else(|_| {
            if text.is_empty() {
                format!("request failed with status {status}")
            } else {
                text.to_owned()
            }
        })
}

fn request_session_token(request: &reqwest::Request) -> Option<String> {
    request
        .headers()
        .get(reqwest::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_owned)
}

fn normalize_base_url(mut base_url: String) -> String {
    while base_url.ends_with('/') {
        base_url.pop();
    }
    base_url
}

#[cfg(test)]
mod tests;
