//! EVM provider and transaction operations.
//!
//! Handles RPC calls for querying balances, nonces, and broadcasting signed
//! transactions across EVM networks with optional authentication tokens.

use std::collections::HashMap;

use secrecy::ExposeSecret;
use serde_json::Value;
use sigillum_api::{
    EthStealthSendErc20TransferRequest, EthStealthSendResponse, EthStealthSendTransferRequest,
    EvmRpcBalanceRequest, EvmRpcBalanceResponse, EvmRpcBroadcastRequest, EvmRpcBroadcastResponse,
    EvmRpcErc20BalanceRequest, EvmRpcErc20BalanceResponse, EvmRpcNonceRequest, EvmRpcNonceResponse,
};
use sigillum_core::{
    EthereumEip1559Erc20Transfer, EthereumEip1559Transfer, SecretStore, VaultLifecycle,
    decode_quantity_hex, derive_sigillum_ethereum_stealth_wallet,
    sign_ethereum_stealth_erc20_transfer, sign_ethereum_stealth_native_transfer,
};

use crate::audit_log::AuditEventSpec;

use super::helpers::{decode_optional_view_tag, map_wallet_error};
use super::{ServiceError, ServiceResult, SigillumService};

mod preflight;
mod rpc;

pub(in crate::service) use preflight::EvmContractCallPreflight;
pub(super) use rpc::EvmLogEntry;
use rpc::ProviderRpcClient;

// ── Type Definitions ──────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub(super) struct EvmBalanceObservationPlan {
    pub deposit_index: usize,
    pub provider_compartment_id: usize,
    pub provider: sigillum_api::EvmProviderProfile,
    pub owner_address: String,
    pub token_address: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct EvmBalanceObservation {
    pub deposit_index: usize,
    pub native_balance_wei_hex: String,
    pub observed_amount_hex: String,
}

pub(super) struct Permit2AllowanceProbe<'a> {
    pub(super) permit2_address: &'a str,
    pub(super) owner_address: &'a str,
    pub(super) token_address: &'a str,
    pub(super) spender_address: &'a str,
    pub(super) block_tag: &'a str,
}

// ── RPC Query Operations ──────────────────────────────────────────────────

impl SigillumService {
    pub(crate) async fn evm_nonce(
        &self,
        token: Option<&str>,
        body: EvmRpcNonceRequest,
    ) -> ServiceResult<EvmRpcNonceResponse> {
        let token = self.require_session(token)?;
        let rpc = self.resolve_provider_rpc_client(
            token,
            &body.provider.rpc_url,
            body.provider.compartment_id,
            body.provider.auth_token_key.as_deref(),
        )?;
        let block_tag = normalize_block_tag(body.block_tag.as_deref(), "pending");
        let nonce = rpc.get_transaction_count(&body.address, &block_tag).await?;

        Ok(EvmRpcNonceResponse {
            address: normalize_address(&body.address)?,
            nonce,
            block_tag,
        })
    }

    pub(crate) async fn evm_balance(
        &self,
        token: Option<&str>,
        body: EvmRpcBalanceRequest,
    ) -> ServiceResult<EvmRpcBalanceResponse> {
        let token = self.require_session(token)?;
        let rpc = self.resolve_provider_rpc_client(
            token,
            &body.provider.rpc_url,
            body.provider.compartment_id,
            body.provider.auth_token_key.as_deref(),
        )?;
        let block_tag = normalize_block_tag(body.block_tag.as_deref(), "latest");
        let balance = rpc.get_balance(&body.address, &block_tag).await?;

        Ok(EvmRpcBalanceResponse {
            address: normalize_address(&body.address)?,
            balance_wei_hex: encode_quantity_u256(&balance),
            block_tag,
        })
    }

    pub(crate) async fn evm_erc20_balance(
        &self,
        token: Option<&str>,
        body: EvmRpcErc20BalanceRequest,
    ) -> ServiceResult<EvmRpcErc20BalanceResponse> {
        let token = self.require_session(token)?;
        let rpc = self.resolve_provider_rpc_client(
            token,
            &body.provider.rpc_url,
            body.provider.compartment_id,
            body.provider.auth_token_key.as_deref(),
        )?;
        let block_tag = normalize_block_tag(body.block_tag.as_deref(), "latest");
        let amount = rpc
            .get_erc20_balance(&body.token_address, &body.owner_address, &block_tag)
            .await?;

        Ok(EvmRpcErc20BalanceResponse {
            token_address: normalize_address(&body.token_address)?,
            owner_address: normalize_address(&body.owner_address)?,
            amount_hex: encode_quantity_u256(&amount),
            block_tag,
        })
    }

    pub(crate) async fn evm_broadcast(
        &self,
        token: Option<&str>,
        body: EvmRpcBroadcastRequest,
    ) -> ServiceResult<EvmRpcBroadcastResponse> {
        let token = self.require_session(token)?;
        let rpc = self.resolve_provider_rpc_client(
            token,
            &body.provider.rpc_url,
            body.provider.compartment_id,
            body.provider.auth_token_key.as_deref(),
        )?;
        let tx_hash = rpc.send_raw_transaction(&body.raw_transaction_hex).await?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::EvmBroadcast {
                transaction_hash_hex: tx_hash.clone(),
            },
        )?;

        Ok(EvmRpcBroadcastResponse {
            transaction_hash_hex: tx_hash,
        })
    }

    // ── Stealth Transfers ─────────────────────────────────────────────────

    pub(crate) async fn eth_stealth_send_transfer(
        &self,
        token: Option<&str>,
        body: EthStealthSendTransferRequest,
    ) -> ServiceResult<EthStealthSendResponse> {
        let token = self.require_session(token)?;
        let active_compartment_id = self
            .state
            .active_compartment_id_for(token)
            .ok_or_else(|| ServiceError::forbidden("No active compartment."))?;
        let provider_compartment_id = body
            .provider_compartment_id
            .unwrap_or(active_compartment_id);
        let wallet_compartment_id = body.wallet_compartment_id.unwrap_or(active_compartment_id);
        let rpc = self.resolve_provider_rpc_client_for_compartment(
            provider_compartment_id,
            &body.rpc_url,
            body.auth_token_key.as_deref(),
        )?;
        let nonce = match body.nonce {
            Some(nonce) => nonce,
            None => {
                rpc.get_transaction_count(&body.stealth.stealth_address, "pending")
                    .await?
            }
        };
        let max_priority_fee_per_gas = decode_quantity_hex(&body.fees.max_priority_fee_per_gas_hex)
            .map_err(map_wallet_error)?;
        let max_fee_per_gas =
            decode_quantity_hex(&body.fees.max_fee_per_gas_hex).map_err(map_wallet_error)?;
        let value = decode_quantity_hex(&body.value_wei_hex).map_err(map_wallet_error)?;
        let view_tag = decode_optional_view_tag(body.stealth.view_tag_hex.as_deref())?;
        let gas_limit = body.gas_limit.unwrap_or(21_000);
        let broadcast = body.broadcast.unwrap_or(false);

        let signed = self.with_vault(wallet_compartment_id, |vault| {
            let master_key = vault
                .extract_master_key()
                .ok_or_else(|| ServiceError::forbidden("Vault is locked."))?;
            let derived =
                derive_sigillum_ethereum_stealth_wallet(master_key.as_ref(), &body.wallet, "eth")
                    .map_err(map_wallet_error)?;
            sign_ethereum_stealth_native_transfer(
                &derived,
                &body.stealth.stealth_address,
                &body.stealth.ephemeral_public_key_hex,
                view_tag,
                &EthereumEip1559Transfer {
                    chain_id: body.fees.chain_id,
                    nonce,
                    max_priority_fee_per_gas,
                    max_fee_per_gas,
                    gas_limit,
                    destination_address: body.destination_address.clone(),
                    value,
                },
            )
            .map_err(map_wallet_error)
        })?;

        let broadcast_transaction_hash_hex = if broadcast {
            Some(
                rpc.send_raw_transaction(&signed.raw_transaction_hex)
                    .await?,
            )
        } else {
            None
        };

        self.record_audit(
            Some(wallet_compartment_id),
            AuditEventSpec::WalletEthStealthSendTransfer {
                wallet: body.wallet.clone(),
                to: signed.to_address.clone(),
                nonce: signed.nonce,
                broadcast,
                transaction_hash_hex: signed.transaction_hash_hex.clone(),
                broadcast_transaction_hash_hex: broadcast_transaction_hash_hex.clone(),
            },
        )?;

        Ok(EthStealthSendResponse {
            wallet: body.wallet,
            kind: signed.kind,
            chain_id: signed.chain_id,
            nonce: signed.nonce,
            from_address: signed.from_address,
            to_address: signed.to_address,
            value_hex: signed.value_hex,
            data_hex: signed.data_hex,
            raw_transaction_hex: signed.raw_transaction_hex,
            transaction_hash_hex: signed.transaction_hash_hex,
            broadcast,
            broadcast_transaction_hash_hex,
        })
    }

    pub(crate) async fn eth_stealth_send_erc20_transfer(
        &self,
        token: Option<&str>,
        body: EthStealthSendErc20TransferRequest,
    ) -> ServiceResult<EthStealthSendResponse> {
        let token = self.require_session(token)?;
        let active_compartment_id = self
            .state
            .active_compartment_id_for(token)
            .ok_or_else(|| ServiceError::forbidden("No active compartment."))?;
        let provider_compartment_id = body
            .provider_compartment_id
            .unwrap_or(active_compartment_id);
        let wallet_compartment_id = body.wallet_compartment_id.unwrap_or(active_compartment_id);
        let rpc = self.resolve_provider_rpc_client_for_compartment(
            provider_compartment_id,
            &body.rpc_url,
            body.auth_token_key.as_deref(),
        )?;
        let nonce = match body.nonce {
            Some(nonce) => nonce,
            None => {
                rpc.get_transaction_count(&body.stealth.stealth_address, "pending")
                    .await?
            }
        };
        let max_priority_fee_per_gas = decode_quantity_hex(&body.fees.max_priority_fee_per_gas_hex)
            .map_err(map_wallet_error)?;
        let max_fee_per_gas =
            decode_quantity_hex(&body.fees.max_fee_per_gas_hex).map_err(map_wallet_error)?;
        let amount = decode_quantity_hex(&body.amount_hex).map_err(map_wallet_error)?;
        let view_tag = decode_optional_view_tag(body.stealth.view_tag_hex.as_deref())?;
        let gas_limit = body.gas_limit.unwrap_or(65_000);
        let broadcast = body.broadcast.unwrap_or(false);

        let signed = self.with_vault(wallet_compartment_id, |vault| {
            let master_key = vault
                .extract_master_key()
                .ok_or_else(|| ServiceError::forbidden("Vault is locked."))?;
            let derived =
                derive_sigillum_ethereum_stealth_wallet(master_key.as_ref(), &body.wallet, "eth")
                    .map_err(map_wallet_error)?;
            sign_ethereum_stealth_erc20_transfer(
                &derived,
                &body.stealth.stealth_address,
                &body.stealth.ephemeral_public_key_hex,
                view_tag,
                &EthereumEip1559Erc20Transfer {
                    chain_id: body.fees.chain_id,
                    nonce,
                    max_priority_fee_per_gas,
                    max_fee_per_gas,
                    gas_limit,
                    token_address: body.token_address.clone(),
                    recipient_address: body.recipient_address.clone(),
                    amount,
                },
            )
            .map_err(map_wallet_error)
        })?;

        let broadcast_transaction_hash_hex = if broadcast {
            Some(
                rpc.send_raw_transaction(&signed.raw_transaction_hex)
                    .await?,
            )
        } else {
            None
        };

        self.record_audit(
            Some(wallet_compartment_id),
            AuditEventSpec::WalletEthStealthSendErc20Transfer {
                wallet: body.wallet.clone(),
                to: signed.to_address.clone(),
                nonce: signed.nonce,
                broadcast,
                transaction_hash_hex: signed.transaction_hash_hex.clone(),
                broadcast_transaction_hash_hex: broadcast_transaction_hash_hex.clone(),
            },
        )?;

        Ok(EthStealthSendResponse {
            wallet: body.wallet,
            kind: signed.kind,
            chain_id: signed.chain_id,
            nonce: signed.nonce,
            from_address: signed.from_address,
            to_address: signed.to_address,
            value_hex: signed.value_hex,
            data_hex: signed.data_hex,
            raw_transaction_hex: signed.raw_transaction_hex,
            transaction_hash_hex: signed.transaction_hash_hex,
            broadcast,
            broadcast_transaction_hash_hex,
        })
    }

    // ── RPC Client Resolution ─────────────────────────────────────────────

    fn resolve_provider_rpc_client(
        &self,
        token: &str,
        rpc_url: &str,
        compartment_id: Option<usize>,
        auth_token_key: Option<&str>,
    ) -> ServiceResult<ProviderRpcClient> {
        let compartment_id = compartment_id
            .or_else(|| self.state.active_compartment_id_for(token))
            .ok_or_else(|| ServiceError::forbidden("No active compartment."))?;
        self.resolve_provider_rpc_client_for_compartment(compartment_id, rpc_url, auth_token_key)
    }

    fn resolve_provider_rpc_client_for_compartment(
        &self,
        compartment_id: usize,
        rpc_url: &str,
        auth_token_key: Option<&str>,
    ) -> ServiceResult<ProviderRpcClient> {
        let auth_token =
            self.resolve_provider_auth_token_for_compartment(compartment_id, auth_token_key)?;
        Ok(ProviderRpcClient::new(
            self.state.http_client(),
            rpc_url.to_string(),
            auth_token,
        ))
    }

    fn provider_rpc_for_profile(
        &self,
        provider_compartment_id: usize,
        provider: &sigillum_api::EvmProviderProfile,
    ) -> ServiceResult<ProviderRpcClient> {
        self.resolve_provider_rpc_client_for_compartment(
            provider_compartment_id,
            &provider.rpc_url,
            provider.auth_token_key.as_deref(),
        )
    }

    /// Live `eth_chainId` probe against a provider profile, reusing the
    /// daemon's bounded HTTP client and vault-backed auth-token resolution.
    pub(super) async fn evm_chain_id_for_provider(
        &self,
        provider_compartment_id: usize,
        provider: &sigillum_api::EvmProviderProfile,
    ) -> ServiceResult<u64> {
        self.provider_rpc_for_profile(provider_compartment_id, provider)?
            .get_chain_id()
            .await
    }

    // ── Balance Observation ───────────────────────────────────────────────

    pub(super) async fn evm_native_and_erc20_balance_for_provider(
        &self,
        provider_compartment_id: usize,
        provider: &sigillum_api::EvmProviderProfile,
        owner_address: &str,
        token_address: &str,
        block_tag: &str,
    ) -> ServiceResult<([u8; 32], [u8; 32])> {
        self.provider_rpc_for_profile(provider_compartment_id, provider)?
            .get_native_and_erc20_balance(owner_address, token_address, block_tag)
            .await
    }

    pub(super) async fn evm_native_balance_for_provider(
        &self,
        provider_compartment_id: usize,
        provider: &sigillum_api::EvmProviderProfile,
        owner_address: &str,
        block_tag: &str,
    ) -> ServiceResult<String> {
        let balance = self
            .provider_rpc_for_profile(provider_compartment_id, provider)?
            .get_balance(owner_address, block_tag)
            .await?;
        Ok(encode_quantity_u256(&balance))
    }

    pub(super) async fn evm_erc20_balance_for_provider(
        &self,
        provider_compartment_id: usize,
        provider: &sigillum_api::EvmProviderProfile,
        token_address: &str,
        owner_address: &str,
        block_tag: &str,
    ) -> ServiceResult<String> {
        let balance = self
            .provider_rpc_for_profile(provider_compartment_id, provider)?
            .get_erc20_balance(token_address, owner_address, block_tag)
            .await?;
        Ok(encode_quantity_u256(&balance))
    }

    pub(super) async fn evm_erc20_allowance_for_provider(
        &self,
        provider_compartment_id: usize,
        provider: &sigillum_api::EvmProviderProfile,
        token_address: &str,
        owner_address: &str,
        spender_address: &str,
        block_tag: &str,
    ) -> ServiceResult<String> {
        let allowance = self
            .provider_rpc_for_profile(provider_compartment_id, provider)?
            .get_erc20_allowance(token_address, owner_address, spender_address, block_tag)
            .await?;
        Ok(encode_quantity_u256(&allowance))
    }

    pub(super) async fn evm_permit2_allowance_for_provider(
        &self,
        provider_compartment_id: usize,
        provider: &sigillum_api::EvmProviderProfile,
        probe: Permit2AllowanceProbe<'_>,
    ) -> ServiceResult<(String, u64)> {
        let allowance = self
            .provider_rpc_for_profile(provider_compartment_id, provider)?
            .get_permit2_allowance(
                probe.permit2_address,
                probe.owner_address,
                probe.token_address,
                probe.spender_address,
                probe.block_tag,
            )
            .await?;
        Ok((
            encode_quantity_u256(&allowance.amount),
            allowance.expiration_unix,
        ))
    }

    pub(super) async fn evm_erc721_owner_for_provider(
        &self,
        provider_compartment_id: usize,
        provider: &sigillum_api::EvmProviderProfile,
        contract_address: &str,
        token_id_hex: &str,
        block_tag: &str,
    ) -> ServiceResult<String> {
        self.provider_rpc_for_profile(provider_compartment_id, provider)?
            .get_erc721_owner(contract_address, token_id_hex, block_tag)
            .await
    }

    pub(super) async fn evm_nft_operator_approval_for_provider(
        &self,
        provider_compartment_id: usize,
        provider: &sigillum_api::EvmProviderProfile,
        contract_address: &str,
        owner_address: &str,
        operator_address: &str,
        block_tag: &str,
    ) -> ServiceResult<bool> {
        self.provider_rpc_for_profile(provider_compartment_id, provider)?
            .get_nft_operator_approval(contract_address, owner_address, operator_address, block_tag)
            .await
    }

    pub(super) async fn evm_erc1155_balance_for_provider(
        &self,
        provider_compartment_id: usize,
        provider: &sigillum_api::EvmProviderProfile,
        contract_address: &str,
        owner_address: &str,
        token_id_hex: &str,
        block_tag: &str,
    ) -> ServiceResult<String> {
        let balance = self
            .provider_rpc_for_profile(provider_compartment_id, provider)?
            .get_erc1155_balance(contract_address, owner_address, token_id_hex, block_tag)
            .await?;
        Ok(encode_quantity_u256(&balance))
    }

    pub(super) async fn evm_transaction_count_for_provider(
        &self,
        provider_compartment_id: usize,
        provider: &sigillum_api::EvmProviderProfile,
        owner_address: &str,
        block_tag: &str,
    ) -> ServiceResult<u64> {
        self.provider_rpc_for_profile(provider_compartment_id, provider)?
            .get_transaction_count(owner_address, block_tag)
            .await
    }

    pub(super) async fn evm_logs_for_provider(
        &self,
        provider_compartment_id: usize,
        provider: &sigillum_api::EvmProviderProfile,
        address: &str,
        topics: &[String],
        from_block: &str,
        to_block: &str,
    ) -> ServiceResult<Vec<EvmLogEntry>> {
        self.provider_rpc_for_profile(provider_compartment_id, provider)?
            .get_logs(address, topics, from_block, to_block)
            .await
    }

    pub(super) async fn evm_filtered_logs_for_provider(
        &self,
        provider_compartment_id: usize,
        provider: &sigillum_api::EvmProviderProfile,
        address: Option<&str>,
        topics: &[Option<String>],
        from_block: &str,
        to_block: &str,
    ) -> ServiceResult<Vec<EvmLogEntry>> {
        self.provider_rpc_for_profile(provider_compartment_id, provider)?
            .get_filtered_logs(address, topics, from_block, to_block)
            .await
    }

    pub(super) async fn fetch_balance_observations(
        &self,
        plans: Vec<EvmBalanceObservationPlan>,
    ) -> ServiceResult<Vec<EvmBalanceObservation>> {
        let mut groups: HashMap<(usize, String, Option<String>), Vec<EvmBalanceObservationPlan>> =
            HashMap::new();
        for plan in plans {
            groups
                .entry((
                    plan.provider_compartment_id,
                    plan.provider.rpc_url.clone(),
                    plan.provider.auth_token_key.clone(),
                ))
                .or_default()
                .push(plan);
        }

        let mut observations = Vec::new();
        for group in groups.into_values() {
            observations.extend(self.fetch_balance_observation_group(group).await?);
        }
        observations.sort_by_key(|observation| observation.deposit_index);
        Ok(observations)
    }

    async fn fetch_balance_observation_group(
        &self,
        plans: Vec<EvmBalanceObservationPlan>,
    ) -> ServiceResult<Vec<EvmBalanceObservation>> {
        if plans.is_empty() {
            return Ok(Vec::new());
        }

        let first = &plans[0];
        let rpc = self.resolve_provider_rpc_client_for_compartment(
            first.provider_compartment_id,
            &first.provider.rpc_url,
            first.provider.auth_token_key.as_deref(),
        )?;
        let limit = plans.len().min(
            self.state
                .runtime_policy()
                .provider_balance_observation_concurrency,
        );
        let mut pending = plans.into_iter();
        let mut join_set = tokio::task::JoinSet::new();

        for _ in 0..limit {
            if let Some(plan) = pending.next() {
                let rpc = rpc.clone();
                join_set.spawn(async move { fetch_balance_observation(rpc, plan).await });
            }
        }

        let mut observations = Vec::new();
        while let Some(result) = join_set.join_next().await {
            observations.push(result.map_err(|error| {
                ServiceError::internal(format!("Provider observation task failed: {error}"))
            })??);

            if let Some(plan) = pending.next() {
                let rpc = rpc.clone();
                join_set.spawn(async move { fetch_balance_observation(rpc, plan).await });
            }
        }

        Ok(observations)
    }

    // ── Provider Authentication ───────────────────────────────────────────

    pub(super) fn resolve_provider_auth_token_for_compartment(
        &self,
        compartment_id: usize,
        auth_token_key: Option<&str>,
    ) -> ServiceResult<Option<String>> {
        match auth_token_key {
            None => Ok(None),
            Some(key) => self.with_vault(compartment_id, |vault| {
                let value = vault
                    .read_api_key(key)
                    .map_err(|error| ServiceError::internal(error.to_string()))?
                    .ok_or_else(|| {
                        ServiceError::not_found(format!("Provider auth token '{key}' not found"))
                    })?;
                Ok(Some(value.expose_secret().to_string()))
            }),
        }
    }
}

// ── Balance Observation Helpers ───────────────────────────────────────────

async fn fetch_balance_observation(
    rpc: ProviderRpcClient,
    plan: EvmBalanceObservationPlan,
) -> ServiceResult<EvmBalanceObservation> {
    let (native_balance_wei_hex, observed_amount_hex) =
        if let Some(token_address) = plan.token_address.as_deref() {
            let (native_balance, token_balance) = rpc
                .get_native_and_erc20_balance(&plan.owner_address, token_address, "latest")
                .await?;
            (
                encode_quantity_u256(&native_balance),
                encode_quantity_u256(&token_balance),
            )
        } else {
            let native_balance = rpc.get_balance(&plan.owner_address, "latest").await?;
            let native_balance_wei_hex = encode_quantity_u256(&native_balance);
            (native_balance_wei_hex.clone(), native_balance_wei_hex)
        };

    Ok(EvmBalanceObservation {
        deposit_index: plan.deposit_index,
        native_balance_wei_hex,
        observed_amount_hex,
    })
}

// ── Address & Encoding Utilities ──────────────────────────────────────────

pub(super) fn normalize_address(address: &str) -> ServiceResult<String> {
    let raw = address
        .strip_prefix("0x")
        .or_else(|| address.strip_prefix("0X"))
        .unwrap_or(address);
    if raw.len() != 40 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ServiceError::bad_request("Invalid ethereum address."));
    }
    Ok(format!("0x{}", raw.to_ascii_lowercase()))
}

fn parse_quantity_u64(value: &Value) -> ServiceResult<u64> {
    let as_str = value
        .as_str()
        .ok_or_else(|| ServiceError::internal("Invalid provider quantity response"))?;
    let raw = as_str
        .strip_prefix("0x")
        .or_else(|| as_str.strip_prefix("0X"))
        .unwrap_or(as_str);
    if raw.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(raw, 16)
        .map_err(|error| ServiceError::internal(format!("Invalid provider quantity: {error}")))
}

fn parse_quantity_u256(value: &Value) -> ServiceResult<[u8; 32]> {
    let as_str = value
        .as_str()
        .ok_or_else(|| ServiceError::internal("Invalid provider quantity response"))?;
    decode_quantity_hex(as_str).map_err(map_wallet_error)
}

pub(super) fn encode_quantity_u256(value: &[u8; 32]) -> String {
    let first = value
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(value.len());
    if first == value.len() {
        "0x0".to_string()
    } else {
        let encoded = hex::encode(&value[first..]);
        let encoded = encoded.strip_prefix('0').unwrap_or(&encoded);
        format!("0x{encoded}")
    }
}

// ── Hex Encoding ──────────────────────────────────────────────────────────

fn normalize_hex_blob(value: &str, label: &str) -> ServiceResult<String> {
    let raw = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ServiceError::bad_request(format!(
            "Invalid {label} encoding."
        )));
    }
    Ok(format!("0x{}", raw.to_ascii_lowercase()))
}

fn normalize_hex_blob_allow_empty(value: &str, label: &str) -> ServiceResult<String> {
    let raw = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ServiceError::bad_request(format!(
            "Invalid {label} encoding."
        )));
    }
    Ok(format!("0x{}", raw.to_ascii_lowercase()))
}

fn normalize_block_tag(value: Option<&str>, default: &str) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
        .to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode, header};
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::json;

    use super::*;

    #[derive(Clone, Default)]
    struct RpcTestState {
        requests: Arc<Mutex<Vec<Value>>>,
    }

    #[test]
    fn encode_quantity_formats_zero_and_non_zero() {
        assert_eq!(encode_quantity_u256(&[0u8; 32]), "0x0");
        let value = decode_quantity_hex("0x2a").unwrap();
        assert_eq!(encode_quantity_u256(&value), "0x2a");
    }

    #[test]
    fn normalize_hex_blob_rejects_bad_input() {
        assert!(normalize_hex_blob("xyz", "raw transaction").is_err());
        assert!(normalize_hex_blob("0xdeadbeef", "raw transaction").is_ok());
    }

    #[tokio::test]
    #[cfg_attr(target_os = "macos", ignore = "sandbox blocks loopback bind")]
    async fn provider_rpc_client_batches_dual_balance_reads() {
        async fn handler(
            State(state): State<RpcTestState>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> (StatusCode, Json<Value>) {
            let auth = headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok());
            assert_eq!(auth, Some("Bearer rpc-token"));
            state.requests.lock().unwrap().push(body.clone());
            assert!(body.is_array());
            (
                StatusCode::OK,
                Json(json!([
                    { "jsonrpc": "2.0", "id": 2, "result": "0x2a" },
                    { "jsonrpc": "2.0", "id": 1, "result": "0x1" }
                ])),
            )
        }

        let state = RpcTestState::default();
        let addr = spawn_test_server(state.clone(), handler).await;
        let http = reqwest::Client::new();
        let rpc = ProviderRpcClient::new(&http, format!("http://{addr}"), Some("rpc-token".into()));

        let (native, token) = rpc
            .get_native_and_erc20_balance(
                "0x0000000000000000000000000000000000000001",
                "0x0000000000000000000000000000000000000002",
                "latest",
            )
            .await
            .unwrap();

        assert_eq!(encode_quantity_u256(&native), "0x1");
        assert_eq!(encode_quantity_u256(&token), "0x2a");

        let requests = state.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 1);
        let batch = requests[0].as_array().unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0]["method"], "eth_getBalance");
        assert_eq!(batch[1]["method"], "eth_call");
    }

    #[tokio::test]
    #[cfg_attr(target_os = "macos", ignore = "sandbox blocks loopback bind")]
    async fn provider_rpc_client_preserves_retryable_throttling() {
        async fn handler(
            _state: State<RpcTestState>,
            _headers: HeaderMap,
            Json(_body): Json<Value>,
        ) -> (StatusCode, Json<Value>) {
            (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!([
                    { "jsonrpc": "2.0", "id": 1, "error": { "code": -32005, "message": "rate limit" } }
                ])),
            )
        }

        let addr = spawn_test_server(RpcTestState::default(), handler).await;
        let http = reqwest::Client::new();
        let rpc = ProviderRpcClient::new(&http, format!("http://{addr}"), None);

        let error = rpc
            .get_balance("0x0000000000000000000000000000000000000001", "latest")
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    async fn spawn_test_server<H, Fut>(state: RpcTestState, handler: H) -> SocketAddr
    where
        H: Clone + Send + Sync + 'static + Fn(State<RpcTestState>, HeaderMap, Json<Value>) -> Fut,
        Fut: std::future::Future<Output = (StatusCode, Json<Value>)> + Send + 'static,
    {
        let app = Router::new().route("/", post(handler)).with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        addr
    }
}
