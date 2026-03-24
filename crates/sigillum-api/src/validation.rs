//! Input validation trait and implementations for request DTOs.
//!
//! This module provides per-field length validation to prevent excessive memory use
//! from unbounded string fields. Security fix B6.

/// Validation trait for request types.
pub trait Validate {
    fn validate(&self) -> Result<(), String>;
}

// ── Helper functions ────────────────────────────────────────────────

/// Check a string field length.
fn check_len(field: &str, value: &str, max: usize) -> Result<(), String> {
    if value.len() > max {
        return Err(format!(
            "{field} exceeds maximum length of {max} bytes (got {len} bytes)",
            len = value.len()
        ));
    }
    Ok(())
}

/// Check an optional string field length.
fn check_optional_len(field: &str, value: &Option<String>, max: usize) -> Result<(), String> {
    if let Some(v) = value {
        check_len(field, v, max)?;
    }
    Ok(())
}

/// Check vector of string items length.
fn check_vec_items_len(field: &str, items: &[String], max: usize) -> Result<(), String> {
    for (i, item) in items.iter().enumerate() {
        if item.len() > max {
            return Err(format!(
                "{field}[{i}] exceeds maximum length of {max} bytes (got {len} bytes)",
                len = item.len()
            ));
        }
    }
    Ok(())
}

// ── Field-specific limits ───────────────────────────────────────────

const MAX_PASSPHRASE: usize = 1024;
const MAX_LABEL: usize = 256;
const MAX_KEY: usize = 512;
const MAX_HEX: usize = 4096;
const MAX_RPC_URL: usize = 2048;
const MAX_PIN: usize = 64;
const MAX_ADDRESS: usize = 128;
const MAX_META_ADDRESS: usize = 256;
const MAX_SNAPSHOT_HEX: usize = 10_000_000;
const MAX_NOTE: usize = 1024;
const MAX_ID: usize = 256;

// ── Validation implementations ──────────────────────────────────────

impl Validate for crate::request::StealthPaymentRef {
    fn validate(&self) -> Result<(), String> {
        check_len("stealth_address", &self.stealth_address, MAX_ADDRESS)?;
        check_len(
            "ephemeral_public_key_hex",
            &self.ephemeral_public_key_hex,
            MAX_HEX,
        )?;
        check_optional_len("view_tag_hex", &self.view_tag_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::EvmProviderRef {
    fn validate(&self) -> Result<(), String> {
        check_len("rpc_url", &self.rpc_url, MAX_RPC_URL)?;
        check_optional_len("auth_token_key", &self.auth_token_key, MAX_KEY)?;
        Ok(())
    }
}

impl Validate for crate::request::Eip1559Fees {
    fn validate(&self) -> Result<(), String> {
        check_len(
            "max_priority_fee_per_gas_hex",
            &self.max_priority_fee_per_gas_hex,
            MAX_HEX,
        )?;
        check_len("max_fee_per_gas_hex", &self.max_fee_per_gas_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::KeyValueRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("key", &self.key, MAX_KEY)?;
        check_optional_len("value", &self.value, MAX_KEY)?;
        Ok(())
    }
}

impl Validate for crate::request::KeyOnlyRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("key", &self.key, MAX_KEY)?;
        Ok(())
    }
}

impl Validate for crate::request::PassphraseRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("passphrase", &self.passphrase, MAX_PASSPHRASE)?;
        Ok(())
    }
}

impl Validate for crate::request::SnapshotRestoreRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("passphrase", &self.passphrase, MAX_PASSPHRASE)?;
        check_len("snapshot_hex", &self.snapshot_hex, MAX_SNAPSHOT_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::SetupResetRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("confirmation", &self.confirmation, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::CompartmentDefinition {
    fn validate(&self) -> Result<(), String> {
        check_len("label", &self.label, MAX_LABEL)?;
        check_optional_len("passphrase_mode", &self.passphrase_mode, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::Fido2SetupRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("pin", &self.pin, MAX_PIN)?;
        check_len("label", &self.label, MAX_LABEL)?;
        check_optional_len("passphrase", &self.passphrase, MAX_PASSPHRASE)?;
        for comp in &self.compartments {
            comp.validate()?;
        }
        Ok(())
    }
}

impl Validate for crate::request::Fido2RegisterRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("pin", &self.pin, MAX_PIN)?;
        check_len("label", &self.label, MAX_LABEL)?;
        if let Some(skip_keys) = &self.skip_keys {
            check_vec_items_len("skip_keys", skip_keys, MAX_KEY)?;
        }
        Ok(())
    }
}

impl Validate for crate::request::Fido2UnlockRequest {
    fn validate(&self) -> Result<(), String> {
        check_vec_items_len("pins", &self.pins, MAX_PIN)?;
        Ok(())
    }
}

impl Validate for crate::request::Fido2RemoveRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("label", &self.label, MAX_LABEL)?;
        check_len("pin", &self.pin, MAX_PIN)?;
        if let Some(skip_keys) = &self.skip_keys {
            check_vec_items_len("skip_keys", skip_keys, MAX_KEY)?;
        }
        Ok(())
    }
}

impl Validate for crate::request::Fido2SetPinRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("new_pin", &self.new_pin, MAX_PIN)?;
        Ok(())
    }
}

impl Validate for crate::request::CompartmentAddRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("label", &self.label, MAX_LABEL)?;
        check_optional_len("passphrase_mode", &self.passphrase_mode, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::CompartmentInitRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("passphrase", &self.passphrase, MAX_PASSPHRASE)?;
        check_optional_len("label", &self.label, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::SecretsPushRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("key", &self.key, MAX_KEY)?;
        check_optional_len("new_key", &self.new_key, MAX_KEY)?;
        Ok(())
    }
}

impl Validate for crate::request::TransitEncryptRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("key", &self.key, MAX_KEY)?;
        check_len("plaintext_hex", &self.plaintext_hex, MAX_HEX)?;
        check_optional_len("aad_hex", &self.aad_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::TransitDecryptRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("key", &self.key, MAX_KEY)?;
        check_len("nonce_hex", &self.nonce_hex, MAX_HEX)?;
        check_len("ciphertext_hex", &self.ciphertext_hex, MAX_HEX)?;
        check_optional_len("aad_hex", &self.aad_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::TransitHmacRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("key", &self.key, MAX_KEY)?;
        check_len("input_hex", &self.input_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthExportRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet", &self.wallet, MAX_LABEL)?;
        check_optional_len("short_name", &self.short_name, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthGenerateRequest {
    fn validate(&self) -> Result<(), String> {
        check_len(
            "stealth_meta_address",
            &self.stealth_meta_address,
            MAX_META_ADDRESS,
        )?;
        check_optional_len(
            "ephemeral_private_key_hex",
            &self.ephemeral_private_key_hex,
            MAX_HEX,
        )?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthCheckRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet", &self.wallet, MAX_LABEL)?;
        self.stealth.validate()?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthSignRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet", &self.wallet, MAX_LABEL)?;
        self.stealth.validate()?;
        check_len("digest_hex", &self.digest_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthSignTransferRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet", &self.wallet, MAX_LABEL)?;
        self.stealth.validate()?;
        self.fees.validate()?;
        check_len(
            "destination_address",
            &self.destination_address,
            MAX_ADDRESS,
        )?;
        check_len("value_wei_hex", &self.value_wei_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthSignErc20TransferRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet", &self.wallet, MAX_LABEL)?;
        self.stealth.validate()?;
        self.fees.validate()?;
        check_len("token_address", &self.token_address, MAX_ADDRESS)?;
        check_len("recipient_address", &self.recipient_address, MAX_ADDRESS)?;
        check_len("amount_hex", &self.amount_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::EvmRpcNonceRequest {
    fn validate(&self) -> Result<(), String> {
        self.provider.validate()?;
        check_len("address", &self.address, MAX_ADDRESS)?;
        check_optional_len("block_tag", &self.block_tag, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::EvmRpcBalanceRequest {
    fn validate(&self) -> Result<(), String> {
        self.provider.validate()?;
        check_len("address", &self.address, MAX_ADDRESS)?;
        check_optional_len("block_tag", &self.block_tag, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::EvmRpcErc20BalanceRequest {
    fn validate(&self) -> Result<(), String> {
        self.provider.validate()?;
        check_len("token_address", &self.token_address, MAX_ADDRESS)?;
        check_len("owner_address", &self.owner_address, MAX_ADDRESS)?;
        check_optional_len("block_tag", &self.block_tag, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::EvmRpcBroadcastRequest {
    fn validate(&self) -> Result<(), String> {
        self.provider.validate()?;
        check_len("raw_transaction_hex", &self.raw_transaction_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthSendTransferRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("rpc_url", &self.rpc_url, MAX_RPC_URL)?;
        check_optional_len("auth_token_key", &self.auth_token_key, MAX_KEY)?;
        check_len("wallet", &self.wallet, MAX_LABEL)?;
        self.stealth.validate()?;
        self.fees.validate()?;
        check_len(
            "destination_address",
            &self.destination_address,
            MAX_ADDRESS,
        )?;
        check_len("value_wei_hex", &self.value_wei_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthSendErc20TransferRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("rpc_url", &self.rpc_url, MAX_RPC_URL)?;
        check_optional_len("auth_token_key", &self.auth_token_key, MAX_KEY)?;
        check_len("wallet", &self.wallet, MAX_LABEL)?;
        self.stealth.validate()?;
        self.fees.validate()?;
        check_len("token_address", &self.token_address, MAX_ADDRESS)?;
        check_len("recipient_address", &self.recipient_address, MAX_ADDRESS)?;
        check_len("amount_hex", &self.amount_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::EvmProviderProfileUpsertRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("name", &self.name, MAX_LABEL)?;
        self.provider.validate()?;
        check_optional_len(
            "max_priority_fee_per_gas_hex",
            &self.max_priority_fee_per_gas_hex,
            MAX_HEX,
        )?;
        check_optional_len("max_fee_per_gas_hex", &self.max_fee_per_gas_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::EvmProfileDeleteRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("name", &self.name, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthWalletProfileUpsertRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("name", &self.name, MAX_LABEL)?;
        check_len("wallet", &self.wallet, MAX_LABEL)?;
        check_optional_len("short_name", &self.short_name, MAX_LABEL)?;
        check_len("provider_profile", &self.provider_profile, MAX_LABEL)?;
        check_optional_len(
            "default_destination_address",
            &self.default_destination_address,
            MAX_ADDRESS,
        )?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthSendWithProfileRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet_profile", &self.wallet_profile, MAX_LABEL)?;
        self.stealth.validate()?;
        check_len("value_wei_hex", &self.value_wei_hex, MAX_HEX)?;
        check_optional_len(
            "destination_address",
            &self.destination_address,
            MAX_ADDRESS,
        )?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthSendErc20WithProfileRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet_profile", &self.wallet_profile, MAX_LABEL)?;
        self.stealth.validate()?;
        check_len("token_address", &self.token_address, MAX_ADDRESS)?;
        check_len("recipient_address", &self.recipient_address, MAX_ADDRESS)?;
        check_len("amount_hex", &self.amount_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::QueueEthStealthNativeSweepRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet_profile", &self.wallet_profile, MAX_LABEL)?;
        self.stealth.validate()?;
        check_optional_len(
            "destination_address",
            &self.destination_address,
            MAX_ADDRESS,
        )?;
        check_optional_len("min_value_wei_hex", &self.min_value_wei_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::QueueEthStealthErc20SweepRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet_profile", &self.wallet_profile, MAX_LABEL)?;
        self.stealth.validate()?;
        check_len("token_address", &self.token_address, MAX_ADDRESS)?;
        check_optional_len("recipient_address", &self.recipient_address, MAX_ADDRESS)?;
        check_optional_len("min_amount_hex", &self.min_amount_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthDepositCreateNativeRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet_profile", &self.wallet_profile, MAX_LABEL)?;
        check_optional_len(
            "expected_value_wei_hex",
            &self.expected_value_wei_hex,
            MAX_HEX,
        )?;
        check_optional_len(
            "sweep_destination_address",
            &self.sweep_destination_address,
            MAX_ADDRESS,
        )?;
        check_optional_len(
            "min_sweep_value_wei_hex",
            &self.min_sweep_value_wei_hex,
            MAX_HEX,
        )?;
        check_optional_len("note", &self.note, MAX_NOTE)?;
        check_optional_len(
            "ephemeral_private_key_hex",
            &self.ephemeral_private_key_hex,
            MAX_HEX,
        )?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthDepositCreateErc20Request {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet_profile", &self.wallet_profile, MAX_LABEL)?;
        check_len("token_address", &self.token_address, MAX_ADDRESS)?;
        check_optional_len("expected_amount_hex", &self.expected_amount_hex, MAX_HEX)?;
        check_optional_len(
            "sweep_destination_address",
            &self.sweep_destination_address,
            MAX_ADDRESS,
        )?;
        check_optional_len("min_sweep_amount_hex", &self.min_sweep_amount_hex, MAX_HEX)?;
        check_optional_len("note", &self.note, MAX_NOTE)?;
        check_optional_len(
            "ephemeral_private_key_hex",
            &self.ephemeral_private_key_hex,
            MAX_HEX,
        )?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthDepositDeleteRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("id", &self.id, MAX_ID)?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthDepositRefreshRequest {
    fn validate(&self) -> Result<(), String> {
        check_optional_len("id", &self.id, MAX_ID)?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthDepositEnqueueSweepRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("id", &self.id, MAX_ID)?;
        Ok(())
    }
}

impl Validate for crate::request::CompartmentSwitchRequest {
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

impl Validate for crate::request::CompartmentRemoveRequest {
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

impl Validate for crate::request::MaintenanceRunRequest {
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

impl Validate for crate::request::QueueProcessRequest {
    fn validate(&self) -> Result<(), String> {
        check_optional_len("id", &self.id, MAX_ID)?;
        Ok(())
    }
}
