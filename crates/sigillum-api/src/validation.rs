//! Input validation trait and implementations for request DTOs.
//!
//! This module provides per-field length validation to prevent excessive memory use
//! from unbounded string fields. Security fix B6.
//!
//! ## Field-level errors
//!
//! [`Validate::validate`] is the legacy fail-fast contract: it returns the
//! first failure as a single string. [`Validate::validate_fields`] is the
//! structured contract used by the HTTP layer to populate
//! `ErrorResponse.fields`: the default implementation wraps `validate()`
//! into one `validation_failed` failure with no field breakdown, so existing
//! implementations need no changes. DTOs that override `validate_fields()`
//! accumulate *all* field errors (in the same check order as before) and
//! implement `validate()` as the first field error's message, keeping the
//! legacy string byte-identical.

use crate::response::FieldError;

/// Validation trait for request types.
pub trait Validate {
    fn validate(&self) -> Result<(), String>;

    /// Validate the request and report per-field errors.
    ///
    /// The default implementation wraps [`Self::validate`] into a
    /// [`ValidationFailure`] with no field breakdown.
    fn validate_fields(&self) -> Result<(), ValidationFailure> {
        self.validate().map_err(ValidationFailure::without_fields)
    }
}

/// A failed request validation: a human-readable summary plus an optional
/// per-field breakdown for the `fields` array of the error envelope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationFailure {
    message: String,
    fields: Vec<FieldError>,
}

impl ValidationFailure {
    /// Failure without a per-field breakdown (legacy single-string path).
    pub fn without_fields(message: String) -> Self {
        Self {
            message,
            fields: Vec::new(),
        }
    }

    /// Failure from a per-field breakdown. The summary message is the first
    /// field's message so the top-level `error` string matches what the
    /// fail-fast `validate()` returned before the breakdown existed.
    pub fn from_fields(fields: Vec<FieldError>) -> Self {
        debug_assert!(!fields.is_empty());
        let message = fields
            .first()
            .map(|field| field.message.clone())
            .unwrap_or_default();
        Self { message, fields }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn fields(&self) -> &[FieldError] {
        &self.fields
    }

    pub fn into_fields(self) -> Vec<FieldError> {
        self.fields
    }
}

// ── Helper functions ────────────────────────────────────────────────

/// Push a check's error into `fields`, tagged with the offending field path.
fn collect(fields: &mut Vec<FieldError>, field: &str, result: Result<(), String>) {
    if let Err(message) = result {
        fields.push(FieldError {
            field: field.to_string(),
            message,
        });
    }
}

/// Push a pre-formatted failure message into `fields` when `check` fails.
fn collect_if(fields: &mut Vec<FieldError>, field: &str, check: bool, message: String) {
    if check {
        fields.push(FieldError {
            field: field.to_string(),
            message,
        });
    }
}

/// Check a list of ethereum addresses, accumulating one field error per
/// offending item (`field[i]` paths).
fn collect_vec_eth_addresses(fields: &mut Vec<FieldError>, field: &str, items: &[String]) {
    for (i, item) in items.iter().enumerate() {
        let path = format!("{field}[{i}]");
        collect(fields, &path, check_eth_address(&path, item));
    }
}

/// Merge a nested DTO's field breakdown without a path prefix. Used for
/// `#[serde(flatten)]` fields (e.g. `EvmProviderRef` inside
/// `EvmProviderProfileUpsertRequest`) where the nested fields are top-level
/// on the wire, so their paths stay top-level too.
fn collect_flat(fields: &mut Vec<FieldError>, result: Result<(), ValidationFailure>) {
    if let Err(failure) = result {
        fields.extend(failure.into_fields());
    }
}

/// Return the breakdown as a `ValidationFailure`, or `Ok` when empty.
fn finish(fields: Vec<FieldError>) -> Result<(), ValidationFailure> {
    if fields.is_empty() {
        Ok(())
    } else {
        Err(ValidationFailure::from_fields(fields))
    }
}

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

fn check_eth_address(field: &str, value: &str) -> Result<(), String> {
    check_len(field, value, MAX_ADDRESS)?;
    let raw = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if raw.len() != 40 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "{field} must be a valid ethereum address (optional 0x prefix plus 40 hex characters)"
        ));
    }
    Ok(())
}

fn check_optional_eth_address(field: &str, value: &Option<String>) -> Result<(), String> {
    if let Some(v) = value {
        check_eth_address(field, v)?;
    }
    Ok(())
}

fn check_threshold_at_least_one(field: &str, threshold: usize) -> Result<(), String> {
    if threshold < 1 {
        return Err(format!("{field} must be >= 1"));
    }
    Ok(())
}

fn check_optional_threshold_at_least_one(
    field: &str,
    threshold: Option<usize>,
) -> Result<(), String> {
    if let Some(threshold) = threshold {
        check_threshold_at_least_one(field, threshold)?;
    }
    Ok(())
}

/// Patch-style address field where omission retains the stored value and an
/// explicit blank clears it. Nonblank input still has to be a valid address.
fn check_optional_blank_or_eth_address(field: &str, value: &Option<String>) -> Result<(), String> {
    if let Some(value) = value {
        // Preserve the normal raw-input bound even though whitespace is
        // semantically meaningful here as an explicit clear operation.
        check_len(field, value, MAX_ADDRESS)?;
        let value = value.trim();
        if !value.is_empty() {
            check_eth_address(field, value)?;
        }
    }
    Ok(())
}

fn check_optional_bip32_path(field: &str, value: &Option<String>) -> Result<(), String> {
    check_optional_bip32_path_with_terminal_hardened(field, value, false)
}

fn check_optional_account_bip32_path(field: &str, value: &Option<String>) -> Result<(), String> {
    check_optional_bip32_path_with_terminal_hardened(field, value, true)
}

fn check_optional_bip32_path_with_terminal_hardened(
    field: &str,
    value: &Option<String>,
    allow_terminal_hardened: bool,
) -> Result<(), String> {
    let Some(path) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(());
    };
    check_len(field, path, MAX_DERIVATION_PATH)?;
    let mut parts = path.split('/');
    if parts.next() != Some("m") {
        return Err(format!("{field} must start with m/"));
    }
    let remaining: Vec<&str> = parts.collect();
    if remaining.is_empty() {
        return Err(format!("{field} must include at least one child index"));
    }
    for (offset, part) in remaining.iter().enumerate() {
        if part.is_empty() || part.trim() != *part {
            return Err(format!("{field} contains an invalid child index"));
        }
        let is_hardened = part.ends_with('\'');
        if is_hardened && offset + 1 == remaining.len() && !allow_terminal_hardened {
            return Err(format!("{field} must end at a public child branch"));
        }
        let index = if is_hardened {
            &part[..part.len() - 1]
        } else {
            part
        };
        if index.is_empty() || !index.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("{field} contains an invalid child index"));
        }
        index
            .parse::<u32>()
            .map_err(|_| format!("{field} contains an invalid child index"))?;
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
const MAX_XPUB: usize = 512;
const MAX_DERIVATION_PATH: usize = 128;
const MAX_MNEMONIC: usize = 2048;
const MAX_SNAPSHOT_HEX: usize = 10_000_000;
const MAX_NOTE: usize = 1024;
const MAX_ID: usize = 256;
const MAX_ENV_NAME: usize = 256;
const MAX_TOKEN_ADDRESSES: usize = 128;
const MAX_CLAIM_PROOF_WORDS: usize = 64;
const MAX_TOKEN_REGISTRY_JSON: usize = 1_000_000;
const MAX_FILE_PATH: usize = 1024;
const MAX_CAPABILITY_SCOPES: usize = 32;
const MAX_CAPABILITY_SCOPE: usize = 128;
const MAX_CAPABILITY_TTL_SECS: u64 = 24 * 60 * 60;

// ── Validation implementations ──────────────────────────────────────

impl Validate for crate::request::StealthPaymentRef {
    fn validate(&self) -> Result<(), String> {
        check_eth_address("stealth_address", &self.stealth_address)?;
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
        self.validate_fields()
            .map_err(|failure| failure.message().to_string())
    }

    fn validate_fields(&self) -> Result<(), ValidationFailure> {
        let mut fields = Vec::new();
        collect(
            &mut fields,
            "rpc_url",
            check_len("rpc_url", &self.rpc_url, MAX_RPC_URL),
        );
        collect(
            &mut fields,
            "auth_token_key",
            check_optional_len("auth_token_key", &self.auth_token_key, MAX_KEY),
        );
        finish(fields)
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

impl Validate for crate::request::CapabilitySessionRequest {
    fn validate(&self) -> Result<(), String> {
        if self.scopes.is_empty() {
            return Err("scopes must not be empty".into());
        }
        if self.scopes.len() > MAX_CAPABILITY_SCOPES {
            return Err(format!(
                "scopes exceeds maximum length of {MAX_CAPABILITY_SCOPES} items"
            ));
        }
        check_vec_items_len("scopes", &self.scopes, MAX_CAPABILITY_SCOPE)?;
        for scope in &self.scopes {
            if !scope
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'-' | b'_'))
            {
                return Err(format!("invalid capability scope '{scope}'"));
            }
        }
        if let Some(ttl_secs) = self.ttl_secs {
            if ttl_secs == 0 || ttl_secs > MAX_CAPABILITY_TTL_SECS {
                return Err(format!(
                    "ttl_secs must be between 1 and {MAX_CAPABILITY_TTL_SECS}"
                ));
            }
        }
        Ok(())
    }
}

impl Validate for crate::request::BiometricEnrollRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("public_key_hex", &self.public_key_hex, MAX_HEX)?;
        check_len("passphrase", &self.passphrase, MAX_PASSPHRASE)?;
        Ok(())
    }
}

impl Validate for crate::request::BiometricUnlockRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("payload_hex", &self.payload_hex, MAX_HEX)?;
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
        check_threshold_at_least_one("threshold", self.threshold)?;
        check_optional_len("passphrase_mode", &self.passphrase_mode, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::Fido2SetupRequest {
    fn validate(&self) -> Result<(), String> {
        check_optional_len("pin", &self.pin, MAX_PIN)?;
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
        check_optional_len("pin", &self.pin, MAX_PIN)?;
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
        check_optional_len("pin", &self.pin, MAX_PIN)?;
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
        check_threshold_at_least_one("threshold", self.threshold)?;
        check_optional_len("passphrase_mode", &self.passphrase_mode, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::CompartmentInitRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("passphrase", &self.passphrase, MAX_PASSPHRASE)?;
        check_optional_len("label", &self.label, MAX_LABEL)?;
        check_optional_threshold_at_least_one("threshold", self.threshold)?;
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
        check_eth_address("destination_address", &self.destination_address)?;
        check_len("value_wei_hex", &self.value_wei_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthSignErc20TransferRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet", &self.wallet, MAX_LABEL)?;
        self.stealth.validate()?;
        self.fees.validate()?;
        check_eth_address("token_address", &self.token_address)?;
        check_eth_address("recipient_address", &self.recipient_address)?;
        check_len("amount_hex", &self.amount_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::EvmRpcNonceRequest {
    fn validate(&self) -> Result<(), String> {
        self.provider.validate()?;
        check_eth_address("address", &self.address)?;
        check_optional_len("block_tag", &self.block_tag, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::EvmRpcBalanceRequest {
    fn validate(&self) -> Result<(), String> {
        self.provider.validate()?;
        check_eth_address("address", &self.address)?;
        check_optional_len("block_tag", &self.block_tag, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::EvmRpcErc20BalanceRequest {
    fn validate(&self) -> Result<(), String> {
        self.provider.validate()?;
        check_eth_address("token_address", &self.token_address)?;
        check_eth_address("owner_address", &self.owner_address)?;
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

impl Validate for crate::request::EvmFeeEstimateRequest {
    fn validate(&self) -> Result<(), String> {
        self.provider.validate()?;
        if self.chain_id == 0 {
            return Err("chain_id must be >= 1".to_string());
        }
        if matches!(self.gas_limit, Some(0)) {
            return Err("gas_limit must be >= 1".to_string());
        }
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
        check_eth_address("destination_address", &self.destination_address)?;
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
        check_eth_address("token_address", &self.token_address)?;
        check_eth_address("recipient_address", &self.recipient_address)?;
        check_len("amount_hex", &self.amount_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::EvmProviderProfileUpsertRequest {
    fn validate(&self) -> Result<(), String> {
        self.validate_fields()
            .map_err(|failure| failure.message().to_string())
    }

    fn validate_fields(&self) -> Result<(), ValidationFailure> {
        let mut fields = Vec::new();
        collect(
            &mut fields,
            "name",
            check_len("name", &self.name, MAX_LABEL),
        );
        // `provider` is #[serde(flatten)], so its fields keep their
        // top-level wire paths (rpc_url, auth_token_key).
        collect_flat(&mut fields, self.provider.validate_fields());
        collect(
            &mut fields,
            "max_priority_fee_per_gas_hex",
            check_optional_len(
                "max_priority_fee_per_gas_hex",
                &self.max_priority_fee_per_gas_hex,
                MAX_HEX,
            ),
        );
        collect(
            &mut fields,
            "max_fee_per_gas_hex",
            check_optional_len("max_fee_per_gas_hex", &self.max_fee_per_gas_hex, MAX_HEX),
        );
        finish(fields)
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
        check_optional_eth_address(
            "default_destination_address",
            &self.default_destination_address,
        )?;
        Ok(())
    }
}

impl Validate for crate::request::EthXpubWalletProfileUpsertRequest {
    fn validate(&self) -> Result<(), String> {
        self.validate_fields()
            .map_err(|failure| failure.message().to_string())
    }

    fn validate_fields(&self) -> Result<(), ValidationFailure> {
        let mut fields = Vec::new();
        collect(
            &mut fields,
            "name",
            check_len("name", &self.name, MAX_LABEL),
        );
        collect(
            &mut fields,
            "provider_profile",
            check_len("provider_profile", &self.provider_profile, MAX_LABEL),
        );
        collect(
            &mut fields,
            "external_receive_xpub",
            check_optional_len(
                "external_receive_xpub",
                &self.external_receive_xpub,
                MAX_XPUB,
            ),
        );
        collect(
            &mut fields,
            "external_receive_path",
            check_optional_bip32_path("external_receive_path", &self.external_receive_path),
        );
        collect(
            &mut fields,
            "external_account_xpub",
            check_optional_len(
                "external_account_xpub",
                &self.external_account_xpub,
                MAX_XPUB,
            ),
        );
        collect(
            &mut fields,
            "external_account_path",
            check_optional_account_bip32_path("external_account_path", &self.external_account_path),
        );
        let has_external_receive_path = self
            .external_receive_path
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        let has_external_receive_xpub = self
            .external_receive_xpub
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        let has_external_account_path = self
            .external_account_path
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        let has_external_account_xpub = self
            .external_account_xpub
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        collect_if(
            &mut fields,
            "external_receive_path",
            has_external_receive_path && !has_external_receive_xpub,
            "external_receive_path requires external_receive_xpub".into(),
        );
        collect_if(
            &mut fields,
            "external_account_path",
            has_external_account_path && !has_external_account_xpub,
            "external_account_path requires external_account_xpub".into(),
        );
        collect_if(
            &mut fields,
            "external_receive_path",
            has_external_receive_path && has_external_account_path,
            "external_receive_path and external_account_path are mutually exclusive".into(),
        );
        collect_if(
            &mut fields,
            "external_receive_xpub",
            has_external_receive_xpub && has_external_account_xpub,
            "external_receive_xpub and external_account_xpub are mutually exclusive".into(),
        );
        collect(
            &mut fields,
            "default_destination_address",
            check_optional_eth_address(
                "default_destination_address",
                &self.default_destination_address,
            ),
        );
        finish(fields)
    }
}

impl Validate for crate::request::EthSeedWalletProfileUpsertRequest {
    fn validate(&self) -> Result<(), String> {
        self.validate_fields()
            .map_err(|failure| failure.message().to_string())
    }

    fn validate_fields(&self) -> Result<(), ValidationFailure> {
        let mut fields = Vec::new();
        collect(
            &mut fields,
            "name",
            check_len("name", &self.name, MAX_LABEL),
        );
        collect(
            &mut fields,
            "label",
            check_optional_len("label", &self.label, MAX_LABEL),
        );
        collect(
            &mut fields,
            "mnemonic",
            check_len("mnemonic", &self.mnemonic, MAX_MNEMONIC),
        );
        collect(
            &mut fields,
            "mnemonic_passphrase",
            check_optional_len(
                "mnemonic_passphrase",
                &self.mnemonic_passphrase,
                MAX_PASSPHRASE,
            ),
        );
        collect(
            &mut fields,
            "provider_profile",
            check_len("provider_profile", &self.provider_profile, MAX_LABEL),
        );
        collect(
            &mut fields,
            "default_destination_address",
            check_optional_eth_address(
                "default_destination_address",
                &self.default_destination_address,
            ),
        );
        finish(fields)
    }
}

impl Validate for crate::request::EthSeedWalletCreateRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("name", &self.name, MAX_LABEL)?;
        check_optional_len("label", &self.label, MAX_LABEL)?;
        if let Some(word_count) = self.word_count
            && word_count != 12
            && word_count != 24
        {
            return Err("word_count must be 12 or 24".into());
        }
        check_optional_len(
            "mnemonic_passphrase",
            &self.mnemonic_passphrase,
            MAX_PASSPHRASE,
        )?;
        check_len("provider_profile", &self.provider_profile, MAX_LABEL)?;
        check_optional_eth_address(
            "default_destination_address",
            &self.default_destination_address,
        )?;
        Ok(())
    }
}

impl Validate for crate::request::EthXpubExportRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet_profile", &self.wallet_profile, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::EthXpubDeriveRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("xpub", &self.xpub, MAX_XPUB)?;
        Ok(())
    }
}

impl Validate for crate::request::WalletInventoryScanRequest {
    fn validate(&self) -> Result<(), String> {
        self.validate_fields()
            .map_err(|failure| failure.message().to_string())
    }

    fn validate_fields(&self) -> Result<(), ValidationFailure> {
        let mut fields = Vec::new();
        collect(
            &mut fields,
            "wallet_family",
            check_optional_len("wallet_family", &self.wallet_family, MAX_LABEL),
        );
        collect(
            &mut fields,
            "wallet_profile",
            check_optional_len("wallet_profile", &self.wallet_profile, MAX_LABEL),
        );
        collect(
            &mut fields,
            "provider_profile",
            check_optional_len("provider_profile", &self.provider_profile, MAX_LABEL),
        );
        collect_if(
            &mut fields,
            "provider_profile",
            self.all_configured_chains == Some(true)
                && self
                    .provider_profile
                    .as_deref()
                    .is_some_and(|profile| !profile.trim().is_empty()),
            "provider_profile cannot be combined with all_configured_chains".into(),
        );
        collect(
            &mut fields,
            "block_tag",
            check_optional_len("block_tag", &self.block_tag, MAX_LABEL),
        );
        collect(
            &mut fields,
            "token_discovery_from_block",
            check_optional_len(
                "token_discovery_from_block",
                &self.token_discovery_from_block,
                MAX_LABEL,
            ),
        );
        collect(
            &mut fields,
            "token_discovery_to_block",
            check_optional_len(
                "token_discovery_to_block",
                &self.token_discovery_to_block,
                MAX_LABEL,
            ),
        );
        collect(
            &mut fields,
            "nft_discovery_from_block",
            check_optional_len(
                "nft_discovery_from_block",
                &self.nft_discovery_from_block,
                MAX_LABEL,
            ),
        );
        collect(
            &mut fields,
            "nft_discovery_to_block",
            check_optional_len(
                "nft_discovery_to_block",
                &self.nft_discovery_to_block,
                MAX_LABEL,
            ),
        );
        for (field, len) in [
            ("token_addresses", self.token_addresses.len()),
            (
                "allowance_spender_addresses",
                self.allowance_spender_addresses.len(),
            ),
            (
                "permit2_contract_addresses",
                self.permit2_contract_addresses.len(),
            ),
            (
                "permit2_spender_addresses",
                self.permit2_spender_addresses.len(),
            ),
            ("defi_token_probes", self.defi_token_probes.len()),
            ("claim_candidate_probes", self.claim_candidate_probes.len()),
            ("watch_addresses", self.watch_addresses.len()),
            ("nft_operator_addresses", self.nft_operator_addresses.len()),
        ] {
            collect_if(
                &mut fields,
                field,
                len > MAX_TOKEN_ADDRESSES,
                format!("{field} exceeds maximum length of {MAX_TOKEN_ADDRESSES} items"),
            );
        }
        collect_vec_eth_addresses(&mut fields, "token_addresses", &self.token_addresses);
        collect_vec_eth_addresses(
            &mut fields,
            "allowance_spender_addresses",
            &self.allowance_spender_addresses,
        );
        collect_vec_eth_addresses(
            &mut fields,
            "permit2_contract_addresses",
            &self.permit2_contract_addresses,
        );
        collect_vec_eth_addresses(
            &mut fields,
            "permit2_spender_addresses",
            &self.permit2_spender_addresses,
        );
        collect_vec_eth_addresses(
            &mut fields,
            "nft_operator_addresses",
            &self.nft_operator_addresses,
        );
        for (index, probe) in self.defi_token_probes.iter().enumerate() {
            let path = format!("defi_token_probes[{index}].protocol");
            collect(
                &mut fields,
                &path,
                check_len(&path, &probe.protocol, MAX_LABEL),
            );
            let path = format!("defi_token_probes[{index}].token_address");
            collect(
                &mut fields,
                &path,
                check_eth_address(&path, &probe.token_address),
            );
            let path = format!("defi_token_probes[{index}].protocol_address");
            collect(
                &mut fields,
                &path,
                check_optional_eth_address(&path, &probe.protocol_address),
            );
        }
        for (index, probe) in self.claim_candidate_probes.iter().enumerate() {
            let path = format!("claim_candidate_probes[{index}].kind");
            collect(&mut fields, &path, check_len(&path, &probe.kind, MAX_LABEL));
            let path = format!("claim_candidate_probes[{index}].protocol");
            collect(
                &mut fields,
                &path,
                check_len(&path, &probe.protocol, MAX_LABEL),
            );
            let path = format!("claim_candidate_probes[{index}].claimant_address");
            collect(
                &mut fields,
                &path,
                check_eth_address(&path, &probe.claimant_address),
            );
            let path = format!("claim_candidate_probes[{index}].claim_contract_address");
            collect(
                &mut fields,
                &path,
                check_eth_address(&path, &probe.claim_contract_address),
            );
            let path = format!("claim_candidate_probes[{index}].asset_address");
            collect(
                &mut fields,
                &path,
                check_eth_address(&path, &probe.asset_address),
            );
            let path = format!("claim_candidate_probes[{index}].amount_hex");
            collect(
                &mut fields,
                &path,
                check_len(&path, &probe.amount_hex, MAX_HEX),
            );
            let path = format!("claim_candidate_probes[{index}].source_label");
            collect(
                &mut fields,
                &path,
                check_len(&path, &probe.source_label, MAX_LABEL),
            );
            let path = format!("claim_candidate_probes[{index}].claim_adapter");
            collect(
                &mut fields,
                &path,
                check_optional_len(&path, &probe.claim_adapter, MAX_LABEL),
            );
            let path = format!("claim_candidate_probes[{index}].claim_index_hex");
            collect(
                &mut fields,
                &path,
                check_optional_len(&path, &probe.claim_index_hex, MAX_HEX),
            );
            let path = format!("claim_candidate_probes[{index}].claim_proof");
            collect_if(
                &mut fields,
                &path,
                probe.claim_proof.len() > MAX_CLAIM_PROOF_WORDS,
                format!(
                    "claim_candidate_probes[{index}].claim_proof exceeds maximum length of {MAX_CLAIM_PROOF_WORDS} items"
                ),
            );
            if probe.claim_proof.len() <= MAX_CLAIM_PROOF_WORDS {
                for (i, item) in probe.claim_proof.iter().enumerate() {
                    let path = format!("claim_candidate_probes[{index}].claim_proof[{i}]");
                    collect(&mut fields, &path, check_len(&path, item, MAX_HEX));
                }
            }
        }
        for (index, probe) in self.watch_addresses.iter().enumerate() {
            let path = format!("watch_addresses[{index}].address");
            collect(&mut fields, &path, check_eth_address(&path, &probe.address));
            let path = format!("watch_addresses[{index}].label");
            collect(
                &mut fields,
                &path,
                check_optional_len(&path, &probe.label, MAX_LABEL),
            );
        }
        finish(fields)
    }
}

impl Validate for crate::request::NftMetadataOptInUpsertRequest {
    fn validate(&self) -> Result<(), String> {
        check_eth_address("contract_address", &self.contract_address)?;
        Ok(())
    }
}

impl Validate for crate::request::NftMetadataOptInDeleteRequest {
    fn validate(&self) -> Result<(), String> {
        check_eth_address("contract_address", &self.contract_address)?;
        Ok(())
    }
}

impl Validate for crate::request::NftMetadataSettingsUpdateRequest {
    fn validate(&self) -> Result<(), String> {
        check_optional_len("ipfs_gateway_url", &self.ipfs_gateway_url, MAX_RPC_URL)?;
        Ok(())
    }
}

impl Validate for crate::request::NftMetadataFetchRequest {
    fn validate(&self) -> Result<(), String> {
        check_optional_eth_address("contract_address", &self.contract_address)?;
        Ok(())
    }
}

impl Validate for crate::request::WatchAddressBookUpsertRequest {
    fn validate(&self) -> Result<(), String> {
        check_eth_address("address", &self.address)?;
        check_optional_len("label", &self.label, MAX_LABEL)?;
        if self.tags.len() > MAX_TOKEN_ADDRESSES {
            return Err(format!(
                "tags exceeds maximum length of {MAX_TOKEN_ADDRESSES} items"
            ));
        }
        check_vec_items_len("tags", &self.tags, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::WatchAddressBookDeleteRequest {
    fn validate(&self) -> Result<(), String> {
        check_eth_address("address", &self.address)?;
        Ok(())
    }
}

impl Validate for crate::request::WalletInventoryAddressPruneRequest {
    fn validate(&self) -> Result<(), String> {
        // Fail closed on an empty selector set: a selector-less prune would
        // match every row in the store, so the request is invalid outright.
        if self.address.is_none()
            && self.wallet_family.is_none()
            && self.wallet_profile.is_none()
            && self.provider_profile.is_none()
            && self.chain_id.is_none()
            && self.account_index.is_none()
        {
            return Err(
                "at least one selector (address, wallet_family, wallet_profile, provider_profile, chain_id, account_index) is required"
                    .into(),
            );
        }
        check_optional_eth_address("address", &self.address)?;
        if let Some(family) = self.wallet_family.as_deref() {
            match family {
                "eth-seed" | "eth-xpub" | "eth-watch" => {}
                _ => {
                    return Err(
                        "wallet_family must be 'eth-seed', 'eth-xpub', or 'eth-watch'".into(),
                    );
                }
            }
        }
        check_optional_len("wallet_profile", &self.wallet_profile, MAX_LABEL)?;
        check_optional_len("provider_profile", &self.provider_profile, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::TokenRegistryImportRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("name", &self.name, MAX_LABEL)?;
        check_optional_len("entries_json", &self.entries_json, MAX_TOKEN_REGISTRY_JSON)?;
        check_optional_len("file_path", &self.file_path, MAX_FILE_PATH)?;
        Ok(())
    }
}

impl Validate for crate::request::TokenRegistryDeleteRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("name", &self.name, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::ChainProfileUpsertRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("name", &self.name, MAX_LABEL)?;
        check_len("chain_family", &self.chain_family, MAX_LABEL)?;
        check_optional_len("provider_profile", &self.provider_profile, MAX_LABEL)?;
        check_optional_len("native_symbol", &self.native_symbol, MAX_LABEL)?;
        if self.native_decimals == Some(0) {
            return Err("native_decimals must be greater than 0".into());
        }
        if let Some(permit2_address) = &self.permit2_address {
            check_eth_address("permit2_address", permit2_address)?;
        }
        if let Some(uniswap_v2_router_address) = &self.uniswap_v2_router_address {
            check_eth_address("uniswap_v2_router_address", uniswap_v2_router_address)?;
        }
        check_optional_len("explorer_url", &self.explorer_url, MAX_RPC_URL)?;
        check_vec_items_len("capabilities", &self.capabilities, MAX_LABEL)?;
        if self.builtin == Some(true) {
            return Err("builtin chain profiles cannot be created or updated by operators".into());
        }
        Ok(())
    }
}

impl Validate for crate::request::ChainProfileDeleteRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("name", &self.name, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::DiscoveryJobMutationRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("id", &self.id, MAX_ID)?;
        Ok(())
    }
}

impl Validate for crate::request::RiskCatalogUpsertRequest {
    fn validate(&self) -> Result<(), String> {
        check_eth_address("address", &self.address)?;
        check_optional_len("label", &self.label, MAX_LABEL)?;
        check_len("risk_level", &self.risk_level, MAX_LABEL)?;
        if self.notes.len() > MAX_TOKEN_ADDRESSES {
            return Err(format!(
                "notes exceeds maximum length of {MAX_TOKEN_ADDRESSES} items"
            ));
        }
        check_vec_items_len("notes", &self.notes, MAX_NOTE)?;
        Ok(())
    }
}

impl Validate for crate::request::RiskCatalogDeleteRequest {
    fn validate(&self) -> Result<(), String> {
        check_eth_address("address", &self.address)?;
        Ok(())
    }
}

impl Validate for crate::request::ConsolidationPlanGenerateRequest {
    fn validate(&self) -> Result<(), String> {
        check_optional_eth_address("destination_address", &self.destination_address)?;
        check_optional_len("wallet_family", &self.wallet_family, MAX_LABEL)?;
        check_optional_len("wallet_profile", &self.wallet_profile, MAX_LABEL)?;
        check_optional_len("provider_profile", &self.provider_profile, MAX_LABEL)?;
        if self.chain_id == Some(0) {
            return Err("chain_id must be greater than 0".into());
        }
        if let Some(strategy) = self.routing_strategy.as_deref() {
            let strategy = strategy.trim();
            if strategy != "single" && strategy != "per_party" {
                return Err("routing_strategy must be one of: single, per_party".into());
            }
        }
        for (index, dest) in self.party_destinations.iter().enumerate() {
            if dest.counterparty_id.trim().is_empty() {
                return Err(format!(
                    "party_destinations[{index}].counterparty_id must not be empty"
                ));
            }
            check_len(
                &format!("party_destinations[{index}].counterparty_id"),
                &dest.counterparty_id,
                MAX_ID,
            )?;
            if dest.destination_address.trim().is_empty() {
                return Err(format!(
                    "party_destinations[{index}].destination_address must not be empty"
                ));
            }
            check_eth_address(
                &format!("party_destinations[{index}].destination_address"),
                &dest.destination_address,
            )?;
        }
        Ok(())
    }
}

impl Validate for crate::request::ConsolidationPlanApproveRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("plan_id", &self.plan_id, MAX_ID)?;
        check_vec_items_len("step_ids", &self.step_ids, MAX_ID)?;
        Ok(())
    }
}

impl Validate for crate::request::ConsolidationPlanSimulateRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("plan_id", &self.plan_id, MAX_ID)?;
        check_vec_items_len("step_ids", &self.step_ids, MAX_ID)?;
        Ok(())
    }
}

impl Validate for crate::request::ConsolidationPlanExportRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("plan_id", &self.plan_id, MAX_ID)?;
        check_vec_items_len("step_ids", &self.step_ids, MAX_ID)?;
        check_optional_len("format", &self.format, MAX_LABEL)?;
        check_optional_eth_address("safe_address", &self.safe_address)?;
        Ok(())
    }
}

impl Validate for crate::request::PlanEnqueueStepRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("plan_id", &self.plan_id, MAX_ID)?;
        check_len("step_id", &self.step_id, MAX_ID)?;
        Ok(())
    }
}

impl Validate for crate::request::PlanEnqueuePlanRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("plan_id", &self.plan_id, MAX_ID)?;
        check_len("confirmation", &self.confirmation, MAX_LABEL)?;
        Ok(())
    }
}

impl Validate for crate::request::TreasuryPolicyUpdateRequest {
    fn validate(&self) -> Result<(), String> {
        self.validate_fields()
            .map_err(|failure| failure.message().to_string())
    }

    fn validate_fields(&self) -> Result<(), ValidationFailure> {
        let mut fields = Vec::new();
        collect_if(
            &mut fields,
            "allowed_destinations",
            self.allowed_destinations.len() > 256,
            "allowed_destinations exceeds maximum length of 256 items".into(),
        );
        for (index, destination) in self.allowed_destinations.iter().enumerate() {
            let path = format!("allowed_destinations[{index}].address");
            collect_if(
                &mut fields,
                &path,
                destination.address.trim().is_empty(),
                format!("allowed_destinations[{index}].address must not be empty"),
            );
            if !destination.address.trim().is_empty() {
                collect(
                    &mut fields,
                    &path,
                    check_eth_address(&path, &destination.address),
                );
            }
            let path = format!("allowed_destinations[{index}].label");
            collect(
                &mut fields,
                &path,
                check_optional_len(&path, &destination.label, MAX_LABEL),
            );
        }
        collect(
            &mut fields,
            "max_step_native_wei_hex",
            check_optional_len(
                "max_step_native_wei_hex",
                &self.max_step_native_wei_hex,
                MAX_HEX,
            ),
        );
        collect(
            &mut fields,
            "max_plan_native_wei_hex",
            check_optional_len(
                "max_plan_native_wei_hex",
                &self.max_plan_native_wei_hex,
                MAX_HEX,
            ),
        );
        collect(
            &mut fields,
            "max_gas_topup_wei_hex",
            check_optional_len(
                "max_gas_topup_wei_hex",
                &self.max_gas_topup_wei_hex,
                MAX_HEX,
            ),
        );
        collect_if(
            &mut fields,
            "max_gas_topup_wei_hex",
            self.allow_gas_topups == Some(true)
                && self
                    .max_gas_topup_wei_hex
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none(),
            "max_gas_topup_wei_hex is required when allow_gas_topups is true".into(),
        );
        collect_if(
            &mut fields,
            "max_gas_topup_wei_hex",
            self.allow_gas_topups == Some(true)
                && self
                    .max_gas_topup_wei_hex
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|value| value.eq_ignore_ascii_case("0x")),
            "max_gas_topup_wei_hex must include at least one hexadecimal digit".into(),
        );
        collect(
            &mut fields,
            "hot_floor_wei_hex",
            check_optional_len("hot_floor_wei_hex", &self.hot_floor_wei_hex, MAX_HEX),
        );
        collect(
            &mut fields,
            "hot_target_wei_hex",
            check_optional_len("hot_target_wei_hex", &self.hot_target_wei_hex, MAX_HEX),
        );
        collect(
            &mut fields,
            "hot_overflow_wei_hex",
            check_optional_len("hot_overflow_wei_hex", &self.hot_overflow_wei_hex, MAX_HEX),
        );
        finish(fields)
    }
}

impl Validate for crate::request::TreasuryReceiveAllocateRequest {
    fn validate(&self) -> Result<(), String> {
        if self.wallet_profile.trim().is_empty() {
            return Err("wallet_profile must not be empty".into());
        }
        check_len("wallet_profile", &self.wallet_profile, MAX_LABEL)?;
        if self.purpose.trim().is_empty() {
            return Err("purpose must not be empty".into());
        }
        check_len("purpose", &self.purpose, MAX_LABEL)?;
        check_optional_len("label", &self.label, MAX_LABEL)?;
        check_optional_len("counterparty_id", &self.counterparty_id, MAX_ID)?;
        check_optional_len(
            "sweep_destination_address",
            &self.sweep_destination_address,
            MAX_ADDRESS,
        )?;
        check_optional_len("min_sweep_amount_hex", &self.min_sweep_amount_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::TreasuryReceiveRotateRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("allocation_id", &self.allocation_id, MAX_ID)?;
        Ok(())
    }
}

impl Validate for crate::request::TreasuryReceivePurgeRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("allocation_id", &self.allocation_id, MAX_ID)?;
        Ok(())
    }
}

impl Validate for crate::request::CounterpartyCreateRequest {
    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("name must not be empty".into());
        }
        check_len("name", &self.name, MAX_LABEL)?;
        check_optional_len("note", &self.note, MAX_NOTE)?;
        check_optional_eth_address("sweep_destination_address", &self.sweep_destination_address)?;
        Ok(())
    }
}

impl Validate for crate::request::CounterpartyUpdateRequest {
    fn validate(&self) -> Result<(), String> {
        self.validate_fields()
            .map_err(|failure| failure.message().to_string())
    }

    fn validate_fields(&self) -> Result<(), ValidationFailure> {
        let mut fields = Vec::new();
        collect(&mut fields, "id", check_len("id", &self.id, MAX_ID));
        collect_if(
            &mut fields,
            "name",
            self.name.trim().is_empty(),
            "name must not be empty".into(),
        );
        collect(
            &mut fields,
            "name",
            check_len("name", &self.name, MAX_LABEL),
        );
        collect(
            &mut fields,
            "note",
            check_optional_len("note", &self.note, MAX_NOTE),
        );
        collect(
            &mut fields,
            "sweep_destination_address",
            check_optional_blank_or_eth_address(
                "sweep_destination_address",
                &self.sweep_destination_address,
            ),
        );
        finish(fields)
    }
}

impl Validate for crate::request::CounterpartyDeleteRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("id", &self.id, MAX_ID)?;
        Ok(())
    }
}

impl Validate for crate::request::ReceivingDepositTagRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("deposit_id", &self.deposit_id, MAX_ID)?;
        check_optional_len("counterparty_id", &self.counterparty_id, MAX_ID)?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthSendWithProfileRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet_profile", &self.wallet_profile, MAX_LABEL)?;
        self.stealth.validate()?;
        check_len("value_wei_hex", &self.value_wei_hex, MAX_HEX)?;
        check_optional_eth_address("destination_address", &self.destination_address)?;
        Ok(())
    }
}

impl Validate for crate::request::EthStealthSendErc20WithProfileRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet_profile", &self.wallet_profile, MAX_LABEL)?;
        self.stealth.validate()?;
        check_eth_address("token_address", &self.token_address)?;
        check_eth_address("recipient_address", &self.recipient_address)?;
        check_len("amount_hex", &self.amount_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::QueueEthStealthNativeSweepRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet_profile", &self.wallet_profile, MAX_LABEL)?;
        self.stealth.validate()?;
        check_optional_eth_address("destination_address", &self.destination_address)?;
        check_optional_len("min_value_wei_hex", &self.min_value_wei_hex, MAX_HEX)?;
        Ok(())
    }
}

impl Validate for crate::request::QueueEthStealthErc20SweepRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet_profile", &self.wallet_profile, MAX_LABEL)?;
        self.stealth.validate()?;
        check_eth_address("token_address", &self.token_address)?;
        check_optional_eth_address("recipient_address", &self.recipient_address)?;
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
        check_optional_eth_address("sweep_destination_address", &self.sweep_destination_address)?;
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
        check_optional_len("gas_amount_wei_hex", &self.gas_amount_wei_hex, MAX_HEX)?;
        if self.gas_amount_wei_hex.is_some() && self.request_gas != Some(true) {
            return Err("gas_amount_wei_hex requires request_gas".into());
        }
        Ok(())
    }
}

impl Validate for crate::request::EthStealthDepositCreateErc20Request {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet_profile", &self.wallet_profile, MAX_LABEL)?;
        check_eth_address("token_address", &self.token_address)?;
        check_optional_len("expected_amount_hex", &self.expected_amount_hex, MAX_HEX)?;
        check_optional_eth_address("sweep_destination_address", &self.sweep_destination_address)?;
        check_optional_len("min_sweep_amount_hex", &self.min_sweep_amount_hex, MAX_HEX)?;
        check_optional_len("note", &self.note, MAX_NOTE)?;
        check_optional_len(
            "ephemeral_private_key_hex",
            &self.ephemeral_private_key_hex,
            MAX_HEX,
        )?;
        check_optional_len("gas_amount_wei_hex", &self.gas_amount_wei_hex, MAX_HEX)?;
        if self.gas_amount_wei_hex.is_some() && self.request_gas != Some(true) {
            return Err("gas_amount_wei_hex requires request_gas".into());
        }
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

impl Validate for crate::request::EthStealthAnnouncementScanRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("wallet_profile", &self.wallet_profile, MAX_LABEL)?;
        check_optional_len("from_block", &self.from_block, MAX_LABEL)?;
        check_optional_len("to_block", &self.to_block, MAX_LABEL)?;
        check_optional_eth_address("token_address", &self.token_address)?;
        check_optional_eth_address("sweep_destination_address", &self.sweep_destination_address)?;
        check_optional_len("min_sweep_amount_hex", &self.min_sweep_amount_hex, MAX_HEX)?;
        check_optional_len("note", &self.note, MAX_NOTE)?;
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

/// Domains a [`crate::request::SelfCheckRunRequest`] may name.
pub const SELF_CHECK_DOMAINS: &[&str] = &[
    "provider",
    "seed-wallet",
    "xpub-wallet",
    "stealth-wallet",
    "watch-book",
    "policy",
    "receive-allocation",
    "fido2",
];

const MAX_SELF_CHECK_DOMAIN: usize = 64;

impl Validate for crate::request::SelfCheckRunRequest {
    fn validate(&self) -> Result<(), String> {
        check_vec_items_len("domains", &self.domains, MAX_SELF_CHECK_DOMAIN)?;
        for domain in &self.domains {
            if !SELF_CHECK_DOMAINS.contains(&domain.as_str()) {
                return Err(format!(
                    "unknown self-check domain '{domain}' (expected one of: {})",
                    SELF_CHECK_DOMAINS.join(", ")
                ));
            }
        }
        Ok(())
    }
}

impl Validate for crate::request::QueueProcessRequest {
    fn validate(&self) -> Result<(), String> {
        check_optional_len("id", &self.id, MAX_ID)?;
        Ok(())
    }
}

impl Validate for crate::request::SecretResolveRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("env_name", &self.env_name, MAX_ENV_NAME)?;
        check_len("reference", &self.reference, MAX_KEY)?;
        if self.env_name.is_empty() {
            return Err("env_name must not be empty".into());
        }
        if self.reference.is_empty() {
            return Err("reference must not be empty".into());
        }
        Ok(())
    }
}

impl Validate for crate::request::SecretResolveBatchRequest {
    fn validate(&self) -> Result<(), String> {
        if self.entries.is_empty() {
            return Err("entries must not be empty".into());
        }
        if self.entries.len() > 256 {
            return Err("entries exceeds maximum length of 256 items".into());
        }
        for entry in &self.entries {
            entry.validate()?;
        }
        Ok(())
    }
}

impl Validate for crate::request::RunAuditRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("program", &self.program, MAX_LABEL)?;
        check_vec_items_len("args", &self.args, MAX_KEY)?;
        if self.program.is_empty() {
            return Err("program must not be empty".into());
        }
        Ok(())
    }
}

impl Validate for crate::request::GenerateStoreRequest {
    fn validate(&self) -> Result<(), String> {
        check_len("key", &self.key, MAX_KEY)?;
        if self.key.is_empty() {
            return Err("key must not be empty".into());
        }
        match &self.kind {
            crate::request::GenerateStoreKind::Password { length, .. } => {
                if *length == 0 || *length > 1024 {
                    return Err("password length must be between 1 and 1024".into());
                }
            }
            crate::request::GenerateStoreKind::Passphrase {
                word_count,
                separator,
            } => {
                if *word_count == 0 || *word_count > 128 {
                    return Err("word_count must be between 1 and 128".into());
                }
                check_len("separator", separator, MAX_LABEL)?;
            }
        }
        Ok(())
    }
}
