//! # Sigillum Client
//!
//! Async HTTP client for the local Sigillum daemon API.
//!
//! [`SigillumClient`] wraps a `reqwest::Client` and maintains a session token
//! that is automatically extracted from daemon responses and injected as a
//! Bearer token on subsequent requests.  Session tokens are cleared on 401
//! responses and explicit lock/revoke calls.
//!
//! ## Authentication flow
//!
//! 1. Create a client with [`SigillumClient::new`].
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
//! `SigillumClient` is `Send + Sync`.  The session token is protected by a
//! `std::sync::Mutex` with poison recovery, making it safe to share across
//! tasks.

use std::sync::Mutex;
use std::time::Duration;

use reqwest::Method;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use sigillum_api::request::{
    CompartmentSwitchRequest, EthStealthCheckRequest, EthStealthDepositCreateErc20Request,
    EthStealthDepositCreateNativeRequest, EthStealthDepositDeleteRequest,
    EthStealthDepositEnqueueSweepRequest, EthStealthDepositRefreshRequest, EthStealthExportRequest,
    EthStealthGenerateRequest, EthStealthSendErc20TransferRequest,
    EthStealthSendErc20WithProfileRequest, EthStealthSendTransferRequest,
    EthStealthSendWithProfileRequest, EthStealthSignErc20TransferRequest, EthStealthSignRequest,
    EthStealthSignTransferRequest, EthStealthWalletProfileUpsertRequest, EthXpubDeriveRequest,
    EthXpubExportRequest, EthXpubWalletProfileUpsertRequest, EvmProfileDeleteRequest,
    EvmProviderProfileUpsertRequest, EvmRpcBalanceRequest, EvmRpcBroadcastRequest,
    EvmRpcErc20BalanceRequest, EvmRpcNonceRequest, Fido2RegisterRequest, Fido2RemoveRequest,
    Fido2SetupRequest, Fido2UnlockRequest, KeyOnlyRequest, KeyValueRequest, MaintenanceRunRequest,
    PassphraseRequest, QueueEthStealthErc20SweepRequest, QueueEthStealthErc20TransferRequest,
    QueueEthStealthNativeSweepRequest, QueueEthStealthTransferRequest, QueueProcessRequest,
    SnapshotRestoreRequest, StealthPaymentRef, TransitDecryptRequest, TransitEncryptRequest,
    TransitHmacRequest,
};
pub use sigillum_api::response::Fido2StatusResponse as DaemonFido2Status;
pub use sigillum_api::response::{
    ActiveCompartment, AuditEvent as DaemonAuditEvent, CompartmentInfo, CompartmentListResponse,
    DiagnosticsResponse, ErrorResponse, EthSignedTransactionResponse, EthStealthCheckResponse,
    EthStealthDeposit, EthStealthDepositEnqueueSweepResponse, EthStealthDepositListResponse,
    EthStealthDepositMutationResponse, EthStealthDepositRefreshResponse,
    EthStealthGenerateResponse, EthStealthMetaAddressResponse, EthStealthSendResponse,
    EthStealthSignResponse, EthStealthWalletProfile, EthStealthWalletProfileListResponse,
    EthStealthWalletProfileMutationResponse, EthXpubAddressResponse, EthXpubExportResponse,
    EthXpubWalletProfile, EthXpubWalletProfileListResponse, EthXpubWalletProfileMutationResponse,
    EvmProviderProfile, EvmProviderProfileListResponse, EvmProviderProfileMutationResponse,
    EvmRpcBalanceResponse, EvmRpcBroadcastResponse, EvmRpcErc20BalanceResponse,
    EvmRpcNonceResponse, Fido2DetectResponse, Fido2KeyInfo, Fido2ListResponse,
    Fido2RegisterResponse, Fido2RemoveResponse, Fido2SetupResponse, Fido2StatusResponse,
    GenericStatusResponse, KeyListResponse, KeyValueResponse, MaintenanceRunResponse,
    QueueEnqueueResponse, QueueJob, QueueJobListResponse, QueueProcessResponse,
    SessionRevokeResponse, SnapshotExportResponse, SnapshotRestoreResponse, StatusResponse,
    SwitchCompartmentResponse, TransitDecryptResponse, TransitEncryptResponse, TransitHmacResponse,
    UnlockResponse, UnlockedCompartment,
};
use sigillum_core::SnapshotSummary;
use thiserror::Error;

pub use sigillum_core::{SecretStore, VaultError};

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
}

/// Async HTTP client for the Sigillum vault daemon.
///
/// Holds a shared `reqwest::Client`, a normalized base URL, and an
/// auto-managed session token.  All public methods return
/// `Result<T, ClientError>`.
pub struct SigillumClient {
    http: reqwest::Client,
    base_url: String,
    session_token: Mutex<Option<String>>,
}

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

impl SigillumClient {
    // ── Construction ───────────────────────────────────────────

    /// Create a client pointing at `base_url` (e.g. `http://127.0.0.1:3200`).
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
                .timeout(DEFAULT_REQUEST_TIMEOUT)
                .build()
                .expect("client HTTP client should build"),
            base_url: normalize_base_url(base_url.into()),
            session_token: Mutex::new(None),
        }
    }

    /// Create a client with a pre-configured `reqwest::Client` (custom timeouts, TLS, etc.).
    pub fn with_http_client(base_url: impl Into<String>, http: reqwest::Client) -> Self {
        Self {
            http,
            base_url: normalize_base_url(base_url.into()),
            session_token: Mutex::new(None),
        }
    }

    // ── Session management ──────────────────────────────────────

    /// Return a clone of the current session token, if any.
    pub fn session_token(&self) -> Option<String> {
        self.session_token
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn set_session_token(&self, token: impl Into<String>) {
        *self.session_token.lock().unwrap_or_else(|e| e.into_inner()) = Some(token.into());
    }

    pub fn clear_session_token(&self) {
        *self.session_token.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    // ── Lifecycle ───────────────────────────────────────────────

    /// Query daemon status (locked / unlocked, active compartment, etc.).
    pub async fn status(&self) -> Result<StatusResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/status");
        self.send(builder).await
    }

    pub async fn unlock_with_passphrase(
        &self,
        passphrase: &str,
    ) -> Result<UnlockResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/unlock")
            .json(&PassphraseRequest {
                passphrase: passphrase.to_string(),
            });
        self.send(builder).await
    }

    pub async fn lock(&self) -> Result<GenericStatusResponse, ClientError> {
        let builder = self.request(Method::POST, "/api/lock");
        let response = self.send(builder).await?;
        self.clear_session_token();
        Ok(response)
    }

    pub async fn revoke_session(&self) -> Result<SessionRevokeResponse, ClientError> {
        let builder = self.request(Method::POST, "/api/session/revoke");
        let response: SessionRevokeResponse = self.send(builder).await?;
        if response.requires_reauth {
            self.clear_session_token();
        }
        Ok(response)
    }

    // ── Compartments ────────────────────────────────────────────

    pub async fn list_compartments(&self) -> Result<Vec<CompartmentInfo>, ClientError> {
        let builder = self.request(Method::GET, "/api/compartment/list");
        Ok(self
            .send::<CompartmentListResponse>(builder)
            .await?
            .compartments)
    }

    pub async fn switch_compartment(
        &self,
        id: usize,
    ) -> Result<SwitchCompartmentResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/compartment/switch")
            .json(&CompartmentSwitchRequest { id });
        self.send(builder).await
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
        let builder =
            self.request(Method::POST, "/api/backup/restore")
                .json(&SnapshotRestoreRequest {
                    passphrase: passphrase.to_string(),
                    snapshot_hex: hex::encode(snapshot),
                });
        let response: SnapshotRestoreResponse = self.send(builder).await?;
        if response.requires_reauth {
            self.clear_session_token();
        }
        Ok(response)
    }

    // ── Observability ───────────────────────────────────────────

    pub async fn audit_events(&self, limit: usize) -> Result<Vec<DaemonAuditEvent>, ClientError> {
        let builder = self.request(Method::GET, &format!("/api/audit?limit={}", limit.max(1)));
        Ok(self
            .send::<sigillum_api::response::AuditResponse>(builder)
            .await?
            .events)
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

    // ── Queue ──────────────────────────────────────────────────

    pub async fn list_queue_jobs(&self) -> Result<Vec<QueueJob>, ClientError> {
        let builder = self.request(Method::GET, "/api/queue/jobs");
        Ok(self.send::<QueueJobListResponse>(builder).await?.jobs)
    }

    pub async fn enqueue_eth_stealth_transfer(
        &self,
        request: QueueEthStealthTransferRequest,
    ) -> Result<QueueEnqueueResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/queue/enqueue/eth-stealth-transfer")
            .json(&request);
        self.send(builder).await
    }

    pub async fn enqueue_eth_stealth_erc20_transfer(
        &self,
        request: QueueEthStealthErc20TransferRequest,
    ) -> Result<QueueEnqueueResponse, ClientError> {
        let builder = self
            .request(
                Method::POST,
                "/api/queue/enqueue/eth-stealth-erc20-transfer",
            )
            .json(&request);
        self.send(builder).await
    }

    pub async fn enqueue_eth_stealth_native_sweep(
        &self,
        request: QueueEthStealthNativeSweepRequest,
    ) -> Result<QueueEnqueueResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/queue/enqueue/eth-stealth-native-sweep")
            .json(&request);
        self.send(builder).await
    }

    pub async fn enqueue_eth_stealth_erc20_sweep(
        &self,
        request: QueueEthStealthErc20SweepRequest,
    ) -> Result<QueueEnqueueResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/queue/enqueue/eth-stealth-erc20-sweep")
            .json(&request);
        self.send(builder).await
    }

    pub async fn process_queue(
        &self,
        request: QueueProcessRequest,
    ) -> Result<QueueProcessResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/queue/process")
            .json(&request);
        self.send(builder).await
    }

    // ── FIDO2 ───────────────────────────────────────────────────────

    pub async fn fido2_status(&self) -> Result<Fido2StatusResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/fido2/status");
        self.send(builder).await
    }

    pub async fn fido2_detect(&self) -> Result<Fido2DetectResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/fido2/detect");
        self.send(builder).await
    }

    pub async fn fido2_list_keys(&self) -> Result<Fido2ListResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/fido2/list");
        self.send(builder).await
    }

    pub async fn fido2_setup(
        &self,
        request: Fido2SetupRequest,
    ) -> Result<Fido2SetupResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/fido2/setup")
            .json(&request);
        self.send(builder).await
    }

    pub async fn fido2_register(
        &self,
        request: Fido2RegisterRequest,
    ) -> Result<Fido2RegisterResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/fido2/register")
            .json(&request);
        self.send(builder).await
    }

    pub async fn fido2_unlock(
        &self,
        request: Fido2UnlockRequest,
    ) -> Result<UnlockResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/fido2/unlock")
            .json(&request);
        self.send(builder).await
    }

    pub async fn fido2_remove(
        &self,
        request: Fido2RemoveRequest,
    ) -> Result<Fido2RemoveResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/fido2/remove")
            .json(&request);
        self.send(builder).await
    }

    // ── Internal helpers ─────────────────────────────────────────

    /// Decode a hex-encoded response field into raw bytes.
    fn decode_hex(field: &str) -> Result<Vec<u8>, ClientError> {
        hex::decode(field).map_err(|error| ClientError::Encoding(error.to_string()))
    }

    fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
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
    /// Handles: session-token extraction from responses, 401 token clearing,
    /// empty-body tolerance, and error-response unwrapping.
    async fn send<T: DeserializeOwned>(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<T, ClientError> {
        let response = builder.send().await?;
        let status = response.status();
        let text = response.text().await?;
        let value = if text.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str::<serde_json::Value>(&text)?
        };

        if let Some(token) = value.get("session_token").and_then(|v| v.as_str()) {
            self.set_session_token(token.to_string());
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            self.clear_session_token();
        }

        if !status.is_success() {
            let message = serde_json::from_value::<ErrorResponse>(value.clone())
                .map(|error| error.error)
                .unwrap_or_else(|_| {
                    if text.is_empty() {
                        format!("request failed with status {status}")
                    } else {
                        text.clone()
                    }
                });
            return Err(ClientError::Api { status, message });
        }

        Ok(serde_json::from_value(value)?)
    }
}

fn normalize_base_url(mut base_url: String) -> String {
    while base_url.ends_with('/') {
        base_url.pop();
    }
    base_url
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::http::{HeaderMap, StatusCode, header};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::json;
    use sigillum_api::request::{Eip1559Fees, EvmProviderRef};

    use super::*;

    #[derive(Clone)]
    struct TestState;

    async fn unlock() -> Json<serde_json::Value> {
        Json(json!({
            "status": "unlocked",
            "method": "passphrase",
            "session_token": "test-token",
            "unlocked_compartments": [],
        }))
    }

    async fn api_keys(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (StatusCode::OK, Json(json!({ "keys": ["alpha", "beta"] })))
    }

    async fn export_snapshot_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "status": "exported",
                "snapshot_hex": "6869",
                "summary": {
                    "created_at_unix": 1,
                    "file_count": 1,
                    "total_bytes": 2,
                }
            })),
        )
    }

    async fn restore_snapshot_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        assert_eq!(body["snapshot_hex"], "6f6b");
        (
            StatusCode::OK,
            Json(json!({
                "status": "restored",
                "summary": {
                    "created_at_unix": 2,
                    "file_count": 1,
                    "total_bytes": 2,
                },
                "requires_reauth": true,
            })),
        )
    }

    async fn audit_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "events": [
                    {
                        "created_at_unix": 1,
                        "kind": "secret.set",
                        "compartment_id": 0,
                        "details": { "key": "db_pass" }
                    }
                ]
            })),
        )
    }

    async fn revoke_session_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "status": "revoked",
                "requires_reauth": true,
            })),
        )
    }

    async fn diagnostics_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "status": "ok",
                "version": "0.1.0",
                "unlock_scope": "process-global",
                "session_scope": "per-session-active-compartment",
                "started_at_unix": 42,
                "initialized": true,
                "unlocked_compartment_count": 1,
                "active_session_count": 1,
                "default_active_compartment_id": 0,
                "max_unlocked_threshold": 1,
                "audit_log_present": true,
                "pending_operation_count": 0,
                "queue_job_count": 1,
                "blocked_queue_job_count": 0,
                "retrying_queue_job_count": 0,
                "failed_queue_job_count": 0,
                "deferred_queue_job_count": 0,
                "startup_interrupted_operation_count": 0,
                "startup_recovered_queue_job_count": 0,
                "startup_reconciled_deposit_count": 0,
                "runtime_policy": {
                    "queue_default_process_limit": 50,
                    "queue_max_process_limit": 500,
                    "deposit_default_refresh_limit": 100,
                    "deposit_max_refresh_limit": 500,
                    "audit_default_limit": 25,
                    "audit_max_limit": 200,
                    "queue_retry_base_delay_secs": 5,
                    "queue_retry_max_delay_secs": 300,
                    "provider_balance_observation_concurrency": 8
                },
                "eth_stealth_deposit_count": 1,
                "funded_eth_stealth_deposit_count": 1,
            })),
        )
    }

    async fn maintenance_run_route(
        headers: HeaderMap,
        Json(_body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "status": "ok",
                "refreshed": 1,
                "detected": 1,
                "queued": 1,
                "processed": 1,
                "succeeded": 1,
                "blocked": 0,
                "retrying": 0,
                "failed": 0,
                "deposits": [],
                "jobs": []
            })),
        )
    }

    async fn transit_encrypt_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        assert_eq!(body["key"], "payments");
        (
            StatusCode::OK,
            Json(json!({
                "key": "payments",
                "nonce_hex": "000102030405060708090a0b",
                "ciphertext_hex": "deadbeef",
            })),
        )
    }

    async fn transit_decrypt_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        assert_eq!(body["key"], "payments");
        (
            StatusCode::OK,
            Json(json!({
                "key": "payments",
                "plaintext_hex": "736563726574",
            })),
        )
    }

    async fn transit_hmac_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        assert_eq!(body["key"], "payments");
        (
            StatusCode::OK,
            Json(json!({
                "key": "payments",
                "digest_hex": "00112233",
            })),
        )
    }

    async fn sign_transfer_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        assert_eq!(body["wallet"], "payments");
        (
            StatusCode::OK,
            Json(json!({
                "wallet": "payments",
                "kind": "eth-transfer",
                "chain_id": 1,
                "nonce": 7,
                "from_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "to_address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "value_hex": "0x1",
                "data_hex": "",
                "raw_transaction_hex": "02deadbeef",
                "transaction_hash_hex": "11".repeat(32),
            })),
        )
    }

    async fn sign_erc20_transfer_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        assert_eq!(body["wallet"], "payments");
        (
            StatusCode::OK,
            Json(json!({
                "wallet": "payments",
                "kind": "erc20-transfer",
                "chain_id": 1,
                "nonce": 8,
                "from_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "to_address": "0xcccccccccccccccccccccccccccccccccccccccc",
                "value_hex": "0x0",
                "data_hex": format!("a9059cbb{}", "00".repeat(64)),
                "raw_transaction_hex": "02cafebabe",
                "transaction_hash_hex": "22".repeat(32),
            })),
        )
    }

    async fn evm_nonce_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        assert_eq!(
            body["address"],
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        (
            StatusCode::OK,
            Json(json!({
                "address": body["address"],
                "nonce": 12,
                "block_tag": "pending",
            })),
        )
    }

    async fn evm_balance_route(
        headers: HeaderMap,
        Json(_body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "balance_wei_hex": "0xde0b6b3a7640000",
                "block_tag": "latest",
            })),
        )
    }

    async fn evm_erc20_balance_route(
        headers: HeaderMap,
        Json(_body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "token_address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
                "owner_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "amount_hex": "0xf4240",
                "block_tag": "latest",
            })),
        )
    }

    async fn evm_broadcast_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        assert!(
            body["raw_transaction_hex"]
                .as_str()
                .unwrap()
                .starts_with("0x02")
        );
        (
            StatusCode::OK,
            Json(json!({
                "transaction_hash_hex": "33".repeat(32),
            })),
        )
    }

    async fn send_transfer_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        assert_eq!(body["wallet"], "payments");
        (
            StatusCode::OK,
            Json(json!({
                "wallet": "payments",
                "kind": "eth-transfer",
                "chain_id": 1,
                "nonce": 12,
                "from_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "to_address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "value_hex": "0x1",
                "data_hex": "",
                "raw_transaction_hex": "02deadbeef",
                "transaction_hash_hex": "44".repeat(32),
                "broadcast": true,
                "broadcast_transaction_hash_hex": "55".repeat(32),
            })),
        )
    }

    async fn send_erc20_transfer_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        assert_eq!(body["wallet"], "payments");
        (
            StatusCode::OK,
            Json(json!({
                "wallet": "payments",
                "kind": "erc20-transfer",
                "chain_id": 1,
                "nonce": 13,
                "from_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "to_address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
                "value_hex": "0x0",
                "data_hex": format!("a9059cbb{}", "00".repeat(64)),
                "raw_transaction_hex": "02feedface",
                "transaction_hash_hex": "66".repeat(32),
                "broadcast": false,
                "broadcast_transaction_hash_hex": null,
            })),
        )
    }

    async fn profiles_evm_list_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "profiles": [{
                    "name": "mainnet",
                    "rpc_url": "https://provider.invalid",
                    "auth_token_key": "alchemy",
                    "compartment_id": 0,
                    "chain_id": 1,
                    "max_priority_fee_per_gas_hex": "0x1",
                    "max_fee_per_gas_hex": "0x2",
                    "native_gas_limit": 21000,
                    "erc20_gas_limit": 65000
                }]
            })),
        )
    }

    async fn profiles_evm_upsert_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({ "status": "ok", "profile": body })),
        )
    }

    async fn profiles_eth_stealth_list_route(
        headers: HeaderMap,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "profiles": [{
                    "name": "payments-mainnet",
                    "wallet": "payments",
                    "short_name": "eth",
                    "provider_profile": "mainnet",
                    "compartment_id": 0,
                    "default_destination_address": "0x1111111111111111111111111111111111111111"
                }]
            })),
        )
    }

    async fn profiles_eth_stealth_upsert_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({ "status": "ok", "profile": body })),
        )
    }

    async fn send_with_profile_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        assert_eq!(body["wallet_profile"], "payments-mainnet");
        (
            StatusCode::OK,
            Json(json!({
                "wallet": "payments",
                "kind": "eth-transfer",
                "chain_id": 1,
                "nonce": 14,
                "from_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "to_address": "0x1111111111111111111111111111111111111111",
                "value_hex": "0x1",
                "data_hex": "",
                "raw_transaction_hex": "02deadbeef",
                "transaction_hash_hex": "77".repeat(32),
                "broadcast": false,
                "broadcast_transaction_hash_hex": null
            })),
        )
    }

    async fn deposits_list_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "deposits": [{
                    "id": "dep-1",
                    "status": "pending",
                    "asset_kind": "native",
                    "wallet_profile": "payments-mainnet",
                    "wallet_compartment_id": 0,
                    "provider_compartment_id": 0,
                    "wallet": "payments",
                    "short_name": "eth",
                    "stealth_meta_address": "st:eth:0x1234",
                    "stealth_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "ephemeral_public_key_hex": "03".repeat(33),
                    "view_tag_hex": "01",
                    "auto_queue_sweep": true,
                    "sweep_destination_address": "0x1111111111111111111111111111111111111111",
                    "created_at_unix": 1,
                    "updated_at_unix": 1
                }]
            })),
        )
    }

    async fn deposits_create_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        let token_address = body
            .get("token_address")
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        (
            StatusCode::OK,
            Json(json!({
                "status": "created",
                "deposit": {
                    "id": "dep-2",
                    "status": "pending",
                    "asset_kind": if token_address.is_some() { "erc20" } else { "native" },
                    "wallet_profile": body["wallet_profile"],
                    "wallet_compartment_id": 0,
                    "provider_compartment_id": 0,
                    "wallet": "payments",
                    "short_name": "eth",
                    "stealth_meta_address": "st:eth:0x1234",
                    "stealth_address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "ephemeral_public_key_hex": "03".repeat(33),
                    "view_tag_hex": "01",
                    "token_address": token_address,
                    "expected_amount_hex": body.get("expected_amount_hex").or_else(|| body.get("expected_value_wei_hex")),
                    "auto_queue_sweep": body.get("auto_queue_sweep").cloned().unwrap_or(json!(false)),
                    "sweep_destination_address": body.get("sweep_destination_address"),
                    "min_sweep_amount_hex": body.get("min_sweep_amount_hex").or_else(|| body.get("min_sweep_value_wei_hex")),
                    "note": body.get("note"),
                    "created_at_unix": 2,
                    "updated_at_unix": 2
                }
            })),
        )
    }

    async fn deposits_refresh_route(
        headers: HeaderMap,
        Json(_body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "processed": 1,
                "detected": 1,
                "queued": 1,
                "deposits": [{
                    "id": "dep-2",
                    "status": "sweep_queued",
                    "asset_kind": "native",
                    "wallet_profile": "payments-mainnet",
                    "wallet_compartment_id": 0,
                    "provider_compartment_id": 0,
                    "wallet": "payments",
                    "short_name": "eth",
                    "stealth_meta_address": "st:eth:0x1234",
                    "stealth_address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "ephemeral_public_key_hex": "03".repeat(33),
                    "view_tag_hex": "01",
                    "observed_amount_hex": "0xde0b6b3a7640000",
                    "auto_queue_sweep": true,
                    "sweep_destination_address": "0x1111111111111111111111111111111111111111",
                    "queue_job_id": "job-3",
                    "queue_job_state": "queued",
                    "created_at_unix": 2,
                    "updated_at_unix": 3,
                    "last_checked_at_unix": 3
                }]
            })),
        )
    }

    async fn deposits_delete_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "status": "deleted",
                "deposit": {
                    "id": body["id"],
                    "status": "pending",
                    "asset_kind": "native",
                    "wallet_profile": "payments-mainnet",
                    "wallet_compartment_id": 0,
                    "provider_compartment_id": 0,
                    "wallet": "payments",
                    "short_name": "eth",
                    "stealth_meta_address": "st:eth:0x1234",
                    "stealth_address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "ephemeral_public_key_hex": "03".repeat(33),
                    "view_tag_hex": "01",
                    "auto_queue_sweep": false,
                    "created_at_unix": 2,
                    "updated_at_unix": 5
                }
            })),
        )
    }

    async fn deposits_enqueue_sweep_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "status": "queued",
                "deposit": {
                    "id": body["id"],
                    "status": "sweep_queued",
                    "asset_kind": "native",
                    "wallet_profile": "payments-mainnet",
                    "wallet_compartment_id": 0,
                    "provider_compartment_id": 0,
                    "wallet": "payments",
                    "short_name": "eth",
                    "stealth_meta_address": "st:eth:0x1234",
                    "stealth_address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "ephemeral_public_key_hex": "03".repeat(33),
                    "view_tag_hex": "01",
                    "auto_queue_sweep": true,
                    "sweep_destination_address": "0x1111111111111111111111111111111111111111",
                    "queue_job_id": "job-4",
                    "queue_job_state": "queued",
                    "created_at_unix": 2,
                    "updated_at_unix": 4
                },
                "job": {
                    "id": "job-4",
                    "state": "queued",
                    "attempts": 0,
                    "created_at_unix": 4,
                    "updated_at_unix": 4,
                    "kind": "eth_stealth_native_sweep",
                    "wallet_profile": "payments-mainnet",
                    "stealth_address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "ephemeral_public_key_hex": "03".repeat(33),
                    "destination_address": "0x1111111111111111111111111111111111111111"
                }
            })),
        )
    }

    async fn queue_list_route(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "jobs": [{
                    "id": "job-1",
                    "state": "queued",
                    "attempts": 0,
                    "created_at_unix": 1,
                    "updated_at_unix": 1,
                    "kind": "eth_stealth_transfer",
                    "wallet_profile": "payments-mainnet",
                    "stealth_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "ephemeral_public_key_hex": "03".repeat(33),
                    "value_wei_hex": "0x1"
                }]
            })),
        )
    }

    async fn queue_enqueue_route(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        let kind = if body.get("token_address").is_some() && body.get("recipient_address").is_some()
        {
            "eth_stealth_erc20_sweep"
        } else if body.get("token_address").is_some() {
            "eth_stealth_erc20_transfer"
        } else if body.get("min_value_wei_hex").is_some()
            || body.get("destination_address").is_some()
        {
            "eth_stealth_native_sweep"
        } else {
            "eth_stealth_transfer"
        };
        (
            StatusCode::OK,
            Json(json!({
                "status": "queued",
                "job": {
                    "id": "job-2",
                    "state": "queued",
                    "attempts": 0,
                    "created_at_unix": 2,
                    "updated_at_unix": 2,
                    "kind": kind,
                    "wallet_profile": body["wallet_profile"],
                    "stealth_address": body["stealth_address"],
                    "ephemeral_public_key_hex": body["ephemeral_public_key_hex"],
                    "value_wei_hex": body["value_wei_hex"],
                    "destination_address": body["destination_address"],
                    "token_address": body["token_address"],
                    "recipient_address": body["recipient_address"],
                    "min_value_wei_hex": body["min_value_wei_hex"],
                    "min_amount_hex": body["min_amount_hex"]
                }
            })),
        )
    }

    async fn queue_process_route(
        headers: HeaderMap,
        Json(_body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing auth" })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "processed": 1,
                "succeeded": 1,
                "blocked": 0,
                "retrying": 0,
                "failed": 0,
                "jobs": [{
                    "id": "job-2",
                    "state": "sent",
                    "attempts": 1,
                    "created_at_unix": 2,
                    "updated_at_unix": 3,
                    "kind": "eth_stealth_transfer",
                    "wallet_profile": "payments-mainnet",
                    "stealth_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "ephemeral_public_key_hex": "03".repeat(33),
                    "value_wei_hex": "0x1",
                    "transaction_hash_hex": "88".repeat(32),
                    "broadcast_transaction_hash_hex": "99".repeat(32)
                }]
            })),
        )
    }

    async fn spawn_test_server() -> SocketAddr {
        let app = Router::new()
            .route("/api/unlock", post(unlock))
            .route("/api/api-keys", get(api_keys))
            .route("/api/audit", get(audit_route))
            .route("/api/diagnostics", get(diagnostics_route))
            .route("/api/maintenance/run", post(maintenance_run_route))
            .route("/api/session/revoke", post(revoke_session_route))
            .route("/api/backup/export", post(export_snapshot_route))
            .route("/api/backup/restore", post(restore_snapshot_route))
            .route("/api/transit/encrypt", post(transit_encrypt_route))
            .route("/api/transit/decrypt", post(transit_decrypt_route))
            .route("/api/transit/hmac", post(transit_hmac_route))
            .route("/api/evm/nonce", post(evm_nonce_route))
            .route("/api/evm/balance", post(evm_balance_route))
            .route("/api/evm/erc20-balance", post(evm_erc20_balance_route))
            .route("/api/evm/broadcast", post(evm_broadcast_route))
            .route("/api/profiles/evm", get(profiles_evm_list_route))
            .route("/api/profiles/evm/upsert", post(profiles_evm_upsert_route))
            .route(
                "/api/profiles/eth-stealth",
                get(profiles_eth_stealth_list_route),
            )
            .route(
                "/api/profiles/eth-stealth/upsert",
                post(profiles_eth_stealth_upsert_route),
            )
            .route(
                "/api/wallets/eth-stealth/sign-transfer",
                post(sign_transfer_route),
            )
            .route(
                "/api/wallets/eth-stealth/sign-erc20-transfer",
                post(sign_erc20_transfer_route),
            )
            .route(
                "/api/wallets/eth-stealth/send-transfer",
                post(send_transfer_route),
            )
            .route(
                "/api/wallets/eth-stealth/send-erc20-transfer",
                post(send_erc20_transfer_route),
            )
            .route(
                "/api/wallets/eth-stealth/send-with-profile",
                post(send_with_profile_route),
            )
            .route("/api/deposits/eth-stealth", get(deposits_list_route))
            .route(
                "/api/deposits/eth-stealth/create-native",
                post(deposits_create_route),
            )
            .route(
                "/api/deposits/eth-stealth/create-erc20",
                post(deposits_create_route),
            )
            .route(
                "/api/deposits/eth-stealth/delete",
                post(deposits_delete_route),
            )
            .route(
                "/api/deposits/eth-stealth/refresh",
                post(deposits_refresh_route),
            )
            .route(
                "/api/deposits/eth-stealth/enqueue-sweep",
                post(deposits_enqueue_sweep_route),
            )
            .route("/api/queue/jobs", get(queue_list_route))
            .route(
                "/api/queue/enqueue/eth-stealth-transfer",
                post(queue_enqueue_route),
            )
            .route(
                "/api/queue/enqueue/eth-stealth-erc20-transfer",
                post(queue_enqueue_route),
            )
            .route(
                "/api/queue/enqueue/eth-stealth-native-sweep",
                post(queue_enqueue_route),
            )
            .route(
                "/api/queue/enqueue/eth-stealth-erc20-sweep",
                post(queue_enqueue_route),
            )
            .route("/api/queue/process", post(queue_process_route))
            .with_state(TestState);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        addr
    }

    #[tokio::test]
    async fn unlock_stores_session_for_follow_up_requests() {
        let addr = spawn_test_server().await;
        let client = SigillumClient::new(format!("http://{addr}/"));

        let unlocked = client.unlock_with_passphrase("passphrase").await.unwrap();
        assert_eq!(unlocked.status, "unlocked");
        assert_eq!(client.session_token().as_deref(), Some("test-token"));

        let keys = client.list_api_keys().await.unwrap();
        assert_eq!(keys, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[tokio::test]
    async fn snapshot_methods_roundtrip_payload_shape() {
        let addr = spawn_test_server().await;
        let client = SigillumClient::new(format!("http://{addr}"));
        client.set_session_token("test-token");

        let (snapshot, summary) = client.export_snapshot("passphrase").await.unwrap();
        assert_eq!(snapshot, b"hi");
        assert_eq!(summary.file_count, 1);

        let restored = client.restore_snapshot("passphrase", b"ok").await.unwrap();
        assert_eq!(restored.status, "restored");
        assert!(client.session_token().is_none());
    }

    #[tokio::test]
    async fn audit_events_reads_recent_feed() {
        let addr = spawn_test_server().await;
        let client = SigillumClient::new(format!("http://{addr}"));
        client.set_session_token("test-token");

        let events = client.audit_events(10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "secret.set");
        assert_eq!(events[0].compartment_id, Some(0));
    }

    #[tokio::test]
    async fn revoke_session_clears_cached_token() {
        let addr = spawn_test_server().await;
        let client = SigillumClient::new(format!("http://{addr}"));
        client.set_session_token("test-token");

        let response = client.revoke_session().await.unwrap();
        assert_eq!(response.status, "revoked");
        assert!(response.requires_reauth);
        assert!(client.session_token().is_none());
    }

    #[tokio::test]
    async fn diagnostics_reads_operational_metadata() {
        let addr = spawn_test_server().await;
        let client = SigillumClient::new(format!("http://{addr}"));
        client.set_session_token("test-token");

        let response = client.diagnostics().await.unwrap();
        assert_eq!(response.status, "ok");
        assert_eq!(response.version, "0.1.0");
        assert_eq!(response.unlock_scope, "process-global");
        assert_eq!(response.session_scope, "per-session-active-compartment");
        assert_eq!(response.started_at_unix, 42);
        assert_eq!(response.unlocked_compartment_count, 1);
        assert_eq!(response.active_session_count, 1);
        assert_eq!(response.default_active_compartment_id, Some(0));
        assert_eq!(response.max_unlocked_threshold, Some(1));
        assert!(response.audit_log_present);
        assert_eq!(response.pending_operation_count, 0);
        assert_eq!(response.queue_job_count, 1);
        assert_eq!(response.blocked_queue_job_count, 0);
        assert_eq!(response.retrying_queue_job_count, 0);
        assert_eq!(response.failed_queue_job_count, 0);
        assert_eq!(response.deferred_queue_job_count, 0);
        assert_eq!(response.startup_interrupted_operation_count, 0);
        assert_eq!(response.startup_recovered_queue_job_count, 0);
        assert_eq!(response.startup_reconciled_deposit_count, 0);
        assert_eq!(response.runtime_policy.queue_default_process_limit, 50);
        assert_eq!(response.runtime_policy.queue_max_process_limit, 500);
        assert_eq!(response.runtime_policy.deposit_default_refresh_limit, 100);
        assert_eq!(response.runtime_policy.deposit_max_refresh_limit, 500);
        assert_eq!(response.runtime_policy.audit_default_limit, 25);
        assert_eq!(response.runtime_policy.audit_max_limit, 200);
        assert_eq!(response.runtime_policy.queue_retry_base_delay_secs, 5);
        assert_eq!(response.runtime_policy.queue_retry_max_delay_secs, 300);
        assert_eq!(
            response
                .runtime_policy
                .provider_balance_observation_concurrency,
            8
        );
        assert_eq!(response.eth_stealth_deposit_count, 1);
        assert_eq!(response.funded_eth_stealth_deposit_count, 1);
    }

    #[tokio::test]
    async fn transit_helpers_roundtrip_response_shapes() {
        let addr = spawn_test_server().await;
        let client = SigillumClient::new(format!("http://{addr}"));
        client.set_session_token("test-token");

        let (nonce, ciphertext) = client
            .transit_encrypt("payments", b"secret", Some(b"aad"))
            .await
            .unwrap();
        assert_eq!(nonce.len(), 12);
        assert_eq!(ciphertext, hex::decode("deadbeef").unwrap());

        let plaintext = client
            .transit_decrypt("payments", &nonce, &ciphertext, Some(b"aad"))
            .await
            .unwrap();
        assert_eq!(plaintext, b"secret");

        let digest = client.transit_hmac("payments", b"payload").await.unwrap();
        assert_eq!(digest, hex::decode("00112233").unwrap());
    }

    #[tokio::test]
    async fn transaction_signing_helpers_roundtrip_response_shapes() {
        let addr = spawn_test_server().await;
        let client = SigillumClient::new(format!("http://{addr}"));
        client.set_session_token("test-token");

        let transfer = client
            .sign_eth_stealth_transfer(EthStealthSignTransferRequest {
                wallet: "payments".into(),
                stealth: sigillum_api::StealthPaymentRef {
                    stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                    ephemeral_public_key_hex: "03".repeat(33),
                    view_tag_hex: Some("01".into()),
                },
                fees: sigillum_api::Eip1559Fees {
                    chain_id: 1,
                    max_priority_fee_per_gas_hex: "0x1".into(),
                    max_fee_per_gas_hex: "0x2".into(),
                },
                nonce: 7,
                gas_limit: 21_000,
                destination_address: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                value_wei_hex: "0x1".into(),
            })
            .await
            .unwrap();
        assert_eq!(transfer.kind, "eth-transfer");
        assert!(transfer.raw_transaction_hex.starts_with("02"));

        let erc20 = client
            .sign_eth_stealth_erc20_transfer(EthStealthSignErc20TransferRequest {
                wallet: "payments".into(),
                stealth: sigillum_api::StealthPaymentRef {
                    stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                    ephemeral_public_key_hex: "03".repeat(33),
                    view_tag_hex: Some("01".into()),
                },
                fees: sigillum_api::Eip1559Fees {
                    chain_id: 1,
                    max_priority_fee_per_gas_hex: "0x1".into(),
                    max_fee_per_gas_hex: "0x2".into(),
                },
                nonce: 8,
                gas_limit: 65_000,
                token_address: "0xcccccccccccccccccccccccccccccccccccccccc".into(),
                recipient_address: "0xdddddddddddddddddddddddddddddddddddddddd".into(),
                amount_hex: "0x5".into(),
            })
            .await
            .unwrap();
        assert_eq!(erc20.kind, "erc20-transfer");
        assert!(erc20.data_hex.starts_with("a9059cbb"));
    }

    #[tokio::test]
    async fn evm_provider_helpers_roundtrip_response_shapes() {
        let addr = spawn_test_server().await;
        let client = SigillumClient::new(format!("http://{addr}"));
        client.set_session_token("test-token");

        let nonce = client
            .evm_nonce(EvmRpcNonceRequest {
                provider: sigillum_api::EvmProviderRef {
                    rpc_url: "https://provider.invalid".into(),
                    auth_token_key: Some("alchemy".into()),
                    compartment_id: None,
                },
                address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                block_tag: Some("pending".into()),
            })
            .await
            .unwrap();
        assert_eq!(nonce.nonce, 12);

        let balance = client
            .evm_balance(EvmRpcBalanceRequest {
                provider: sigillum_api::EvmProviderRef {
                    rpc_url: "https://provider.invalid".into(),
                    auth_token_key: None,
                    compartment_id: None,
                },
                address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                block_tag: None,
            })
            .await
            .unwrap();
        assert_eq!(balance.balance_wei_hex, "0xde0b6b3a7640000");

        let erc20 = client
            .evm_erc20_balance(EvmRpcErc20BalanceRequest {
                provider: sigillum_api::EvmProviderRef {
                    rpc_url: "https://provider.invalid".into(),
                    auth_token_key: None,
                    compartment_id: None,
                },
                token_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
                owner_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                block_tag: None,
            })
            .await
            .unwrap();
        assert_eq!(erc20.amount_hex, "0xf4240");

        let broadcast = client
            .evm_broadcast(EvmRpcBroadcastRequest {
                provider: sigillum_api::EvmProviderRef {
                    rpc_url: "https://provider.invalid".into(),
                    auth_token_key: None,
                    compartment_id: None,
                },
                raw_transaction_hex: "0x02deadbeef".into(),
            })
            .await
            .unwrap();
        assert_eq!(broadcast.transaction_hash_hex, "33".repeat(32));
    }

    #[tokio::test]
    async fn stealth_send_helpers_roundtrip_response_shapes() {
        let addr = spawn_test_server().await;
        let client = SigillumClient::new(format!("http://{addr}"));
        client.set_session_token("test-token");

        let sent = client
            .send_eth_stealth_transfer(EthStealthSendTransferRequest {
                rpc_url: "https://provider.invalid".into(),
                wallet: "payments".into(),
                stealth: StealthPaymentRef {
                    stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                    ephemeral_public_key_hex: "03".repeat(33),
                    view_tag_hex: Some("01".into()),
                },
                fees: Eip1559Fees {
                    chain_id: 1,
                    max_priority_fee_per_gas_hex: "0x1".into(),
                    max_fee_per_gas_hex: "0x2".into(),
                },
                destination_address: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                value_wei_hex: "0x1".into(),
                auth_token_key: None,
                provider_compartment_id: None,
                wallet_compartment_id: None,
                nonce: None,
                gas_limit: None,
                broadcast: Some(true),
            })
            .await
            .unwrap();
        assert_eq!(sent.kind, "eth-transfer");
        assert!(sent.broadcast);

        let sent_erc20 = client
            .send_eth_stealth_erc20_transfer(EthStealthSendErc20TransferRequest {
                rpc_url: "https://provider.invalid".into(),
                wallet: "payments".into(),
                stealth: StealthPaymentRef {
                    stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                    ephemeral_public_key_hex: "03".repeat(33),
                    view_tag_hex: Some("01".into()),
                },
                fees: Eip1559Fees {
                    chain_id: 1,
                    max_priority_fee_per_gas_hex: "0x1".into(),
                    max_fee_per_gas_hex: "0x2".into(),
                },
                token_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
                recipient_address: "0xdddddddddddddddddddddddddddddddddddddddd".into(),
                amount_hex: "0x5".into(),
                auth_token_key: None,
                provider_compartment_id: None,
                wallet_compartment_id: None,
                nonce: Some(13),
                gas_limit: Some(65_000),
                broadcast: Some(false),
            })
            .await
            .unwrap();
        assert_eq!(sent_erc20.kind, "erc20-transfer");
        assert!(!sent_erc20.broadcast);
    }

    #[tokio::test]
    async fn profile_and_queue_helpers_roundtrip_response_shapes() {
        let addr = spawn_test_server().await;
        let client = SigillumClient::new(format!("http://{addr}"));
        client.set_session_token("test-token");

        let providers = client.list_evm_provider_profiles().await.unwrap();
        assert_eq!(providers[0].name, "mainnet");

        let provider = client
            .upsert_evm_provider_profile(EvmProviderProfileUpsertRequest {
                name: "mainnet".into(),
                provider: EvmProviderRef {
                    rpc_url: "https://provider.invalid".into(),
                    auth_token_key: Some("alchemy".into()),
                    compartment_id: Some(0),
                },
                chain_id: 1,
                max_priority_fee_per_gas_hex: Some("0x1".into()),
                max_fee_per_gas_hex: Some("0x2".into()),
                native_gas_limit: Some(21_000),
                erc20_gas_limit: Some(65_000),
            })
            .await
            .unwrap();
        assert_eq!(provider.profile.name, "mainnet");

        let wallets = client.list_eth_stealth_wallet_profiles().await.unwrap();
        assert_eq!(wallets[0].name, "payments-mainnet");

        let wallet = client
            .upsert_eth_stealth_wallet_profile(EthStealthWalletProfileUpsertRequest {
                name: "payments-mainnet".into(),
                wallet: "payments".into(),
                short_name: Some("eth".into()),
                provider_profile: "mainnet".into(),
                compartment_id: Some(0),
                chain_id: Some(1),
                default_destination_address: Some(
                    "0x1111111111111111111111111111111111111111".into(),
                ),
            })
            .await
            .unwrap();
        assert_eq!(wallet.profile.provider_profile, "mainnet");

        let sent = client
            .send_eth_stealth_with_profile(EthStealthSendWithProfileRequest {
                wallet_profile: "payments-mainnet".into(),
                stealth: StealthPaymentRef {
                    stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                    ephemeral_public_key_hex: "03".repeat(33),
                    view_tag_hex: Some("01".into()),
                },
                value_wei_hex: "0x1".into(),
                destination_address: None,
                nonce: None,
                gas_limit: None,
                broadcast: Some(false),
            })
            .await
            .unwrap();
        assert_eq!(sent.kind, "eth-transfer");

        let queued = client.list_queue_jobs().await.unwrap();
        assert_eq!(queued[0].id, "job-1");

        let enqueued = client
            .enqueue_eth_stealth_transfer(QueueEthStealthTransferRequest {
                wallet_profile: "payments-mainnet".into(),
                stealth: StealthPaymentRef {
                    stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                    ephemeral_public_key_hex: "03".repeat(33),
                    view_tag_hex: Some("01".into()),
                },
                value_wei_hex: "0x1".into(),
                destination_address: None,
                nonce: None,
                gas_limit: None,
                broadcast: None,
            })
            .await
            .unwrap();
        assert_eq!(enqueued.job.id, "job-2");

        let processed = client
            .process_queue(QueueProcessRequest {
                id: None,
                limit: None,
            })
            .await
            .unwrap();
        assert_eq!(processed.succeeded, 1);
        assert_eq!(processed.blocked, 0);
        assert_eq!(processed.retrying, 0);

        let deposits = client.list_eth_stealth_deposits().await.unwrap();
        assert_eq!(deposits[0].id, "dep-1");

        let native_deposit = client
            .create_eth_stealth_native_deposit(EthStealthDepositCreateNativeRequest {
                wallet_profile: "payments-mainnet".into(),
                expected_value_wei_hex: Some("0x1".into()),
                auto_queue_sweep: Some(true),
                sweep_destination_address: Some(
                    "0x1111111111111111111111111111111111111111".into(),
                ),
                min_sweep_value_wei_hex: Some("0x1".into()),
                note: Some("invoice-42".into()),
                ephemeral_private_key_hex: None,
            })
            .await
            .unwrap();
        assert_eq!(native_deposit.deposit.asset_kind, "native");

        let erc20_deposit = client
            .create_eth_stealth_erc20_deposit(EthStealthDepositCreateErc20Request {
                wallet_profile: "payments-mainnet".into(),
                token_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
                expected_amount_hex: Some("0xf4240".into()),
                auto_queue_sweep: Some(true),
                sweep_destination_address: Some(
                    "0x1111111111111111111111111111111111111111".into(),
                ),
                min_sweep_amount_hex: Some("0xf4240".into()),
                note: None,
                ephemeral_private_key_hex: None,
            })
            .await
            .unwrap();
        assert_eq!(
            erc20_deposit.deposit.token_address.as_deref(),
            Some("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48")
        );

        let refreshed = client
            .refresh_eth_stealth_deposits(EthStealthDepositRefreshRequest {
                id: None,
                limit: None,
                auto_enqueue: Some(true),
            })
            .await
            .unwrap();
        assert_eq!(refreshed.detected, 1);
        assert_eq!(refreshed.queued, 1);

        let deposit_sweep = client
            .enqueue_eth_stealth_deposit_sweep(EthStealthDepositEnqueueSweepRequest {
                id: "dep-2".into(),
                force: Some(true),
            })
            .await
            .unwrap();
        assert_eq!(deposit_sweep.job.id, "job-4");

        let native_sweep = client
            .enqueue_eth_stealth_native_sweep(QueueEthStealthNativeSweepRequest {
                wallet_profile: "payments-mainnet".into(),
                stealth: StealthPaymentRef {
                    stealth_address: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                    ephemeral_public_key_hex: "03".repeat(33),
                    view_tag_hex: Some("01".into()),
                },
                destination_address: Some("0x1111111111111111111111111111111111111111".into()),
                min_value_wei_hex: Some("0x1".into()),
                gas_limit: Some(21_000),
            })
            .await
            .unwrap();
        assert_eq!(native_sweep.job.id, "job-2");

        let erc20_sweep = client
            .enqueue_eth_stealth_erc20_sweep(QueueEthStealthErc20SweepRequest {
                wallet_profile: "payments-mainnet".into(),
                stealth: StealthPaymentRef {
                    stealth_address: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                    ephemeral_public_key_hex: "03".repeat(33),
                    view_tag_hex: Some("01".into()),
                },
                token_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
                recipient_address: Some("0x1111111111111111111111111111111111111111".into()),
                min_amount_hex: Some("0xf4240".into()),
                gas_limit: Some(65_000),
            })
            .await
            .unwrap();
        assert_eq!(erc20_sweep.job.id, "job-2");

        let deleted = client
            .delete_eth_stealth_deposit(EthStealthDepositDeleteRequest { id: "dep-2".into() })
            .await
            .unwrap();
        assert_eq!(deleted.status, "deleted");

        let maintenance = client
            .run_maintenance(MaintenanceRunRequest {
                deposit_refresh_limit: Some(10),
                queue_process_limit: Some(10),
                auto_enqueue: Some(true),
            })
            .await
            .unwrap();
        assert_eq!(maintenance.status, "ok");
        assert_eq!(maintenance.succeeded, 1);
    }
}
