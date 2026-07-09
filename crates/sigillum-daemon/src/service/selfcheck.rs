//! Operator self-check across every configured subsystem.
//!
//! One action verifies that user-entered configuration is both well-formed
//! and actually functioning: provider profiles answer live `eth_chainId` RPC
//! with the chain id they claim, wallet profiles re-derive their recorded
//! addresses, treasury policy guardrails are sane, watch-book entries parse,
//! receive allocations still match their owning profiles, and registered
//! FIDO2 keys satisfy compartment thresholds.
//!
//! Everything is read-only against local state; the provider probes are the
//! only outbound calls and each is bounded to [`PROVIDER_PROBE_TIMEOUT`].
//! Unconfigured domains produce no results, except `policy` (an absent policy
//! is itself a warning) and `provider` (a daemon with zero providers cannot
//! reach any chain).

use std::time::{Duration, Instant};

use sigillum_api::{
    EthSeedWalletProfile, EthStealthWalletProfile, EthXpubWalletProfile, EvmProviderProfile,
    SelfCheckResult, SelfCheckRunRequest, SelfCheckRunResponse, TreasuryPolicy,
    TreasuryReceiveAllocation, WatchAddressBookEntry,
};
use sigillum_core::{
    SecretStore, VaultLifecycle, decode_quantity_hex, derive_ethereum_address_from_imported_xpub,
    derive_ethereum_address_from_xpub, derive_ethereum_receive_branch_from_account_xpub,
    derive_ethereum_receive_branch_from_account_xpub_with_path,
    derive_sigillum_ethereum_xpub_receive_branch, validate_ethereum_imported_xpub_path,
};

use crate::audit_log::AuditEventSpec;
use crate::profiles::ProfileRegistry;

use super::chains::chain_profile_for_id;
use super::evm::normalize_address;
use super::helpers::{compare_u256, now_unix};
use super::{ServiceError, ServiceResult, SigillumService};

// ── Wire vocabulary ───────────────────────────────────────────────

const STATUS_PASS: &str = "pass";
const STATUS_WARN: &str = "warn";
const STATUS_FAIL: &str = "fail";

const DOMAIN_PROVIDER: &str = "provider";
const DOMAIN_SEED_WALLET: &str = "seed-wallet";
const DOMAIN_XPUB_WALLET: &str = "xpub-wallet";
const DOMAIN_STEALTH_WALLET: &str = "stealth-wallet";
const DOMAIN_WATCH_BOOK: &str = "watch-book";
const DOMAIN_POLICY: &str = "policy";
const DOMAIN_RECEIVE_ALLOCATION: &str = "receive-allocation";
const DOMAIN_FIDO2: &str = "fido2";

const WALLET_FAMILY_ETH_SEED: &str = "eth-seed";
const WALLET_FAMILY_ETH_XPUB: &str = "eth-xpub";
const RECEIVE_STATUS_ACTIVE: &str = "active";

/// Hard upper bound per provider probe, inside the daemon HTTP client's own
/// connect/request timeouts (5s connect / 15s request).
const PROVIDER_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether the vault for a compartment could vouch for a stored secret.
enum VaultSecretPresence {
    /// Compartment is locked (or its vault is not registered) — unverifiable.
    Locked,
    Present,
    Missing,
    Unreadable(String),
}

/// Whether a wallet's derivation material could be resolved from the vault.
enum VaultDerivation {
    Locked,
    Resolvable {
        receive_xpub: String,
        operator_asserted_path: bool,
    },
    Invalid(String),
}

impl SigillumService {
    /// Run every requested self-check domain and return per-subject verdicts.
    ///
    /// `body.domains` is pre-validated by the API layer (unknown domains are
    /// rejected with 400 before this method runs); an empty list runs all
    /// domains.
    pub(crate) async fn run_self_check(
        &self,
        token: Option<&str>,
        body: SelfCheckRunRequest,
    ) -> ServiceResult<SelfCheckRunResponse> {
        let token = self.require_session(token)?;
        let requested =
            |domain: &str| body.domains.is_empty() || body.domains.iter().any(|d| d == domain);

        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        let inventory =
            crate::inventory::load_wallet_inventory(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load wallet inventory: {error}"))
            })?;

        let mut checks = Vec::new();
        if requested(DOMAIN_PROVIDER) {
            self.check_providers(
                &registry.evm_providers,
                &inventory.chain_profiles,
                &mut checks,
            )
            .await;
        }
        if requested(DOMAIN_SEED_WALLET) {
            for profile in &registry.eth_seed_wallets {
                checks.push(self.check_seed_wallet(&registry, profile));
            }
        }
        if requested(DOMAIN_XPUB_WALLET) {
            for profile in &registry.eth_xpub_wallets {
                checks.push(self.check_xpub_wallet(&registry, profile));
            }
        }
        if requested(DOMAIN_STEALTH_WALLET) {
            for profile in &registry.eth_stealth_wallets {
                checks.push(check_stealth_wallet(&registry, profile));
            }
        }
        if requested(DOMAIN_WATCH_BOOK) {
            for entry in &inventory.watch_address_book {
                checks.push(check_watch_book_entry(entry));
            }
        }
        if requested(DOMAIN_POLICY) {
            checks.push(check_policy(inventory.treasury_policy.as_ref()));
        }
        if requested(DOMAIN_RECEIVE_ALLOCATION) {
            for allocation in &inventory.receive_allocations {
                if allocation.status == RECEIVE_STATUS_ACTIVE {
                    checks.push(self.check_receive_allocation(&registry, allocation));
                }
            }
        }
        if requested(DOMAIN_FIDO2) {
            if let Some(check) = self.check_fido2() {
                checks.push(check);
            }
        }

        let failures = checks
            .iter()
            .filter(|check| check.status == STATUS_FAIL)
            .count();
        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::SelfCheckRun {
                checks: checks.len(),
                failures,
            },
        )?;

        Ok(SelfCheckRunResponse {
            status: aggregate_status(&checks).to_string(),
            generated_at_unix: now_unix(),
            checks,
        })
    }

    /// Probe every provider profile with a live, bounded `eth_chainId` call.
    ///
    /// Probes run sequentially: self-check is an operator action, not a hot
    /// path, and sequencing keeps load on rate-limited endpoints negligible.
    async fn check_providers(
        &self,
        providers: &[EvmProviderProfile],
        chain_profiles: &[sigillum_api::ChainProfile],
        checks: &mut Vec<SelfCheckResult>,
    ) {
        if providers.is_empty() {
            checks.push(result(
                DOMAIN_PROVIDER,
                "registry",
                STATUS_WARN,
                "No EVM provider profiles configured",
                None,
            ));
            return;
        }

        for provider in providers {
            if chain_profile_for_id(chain_profiles, provider.chain_id).is_none() {
                checks.push(result(
                    DOMAIN_PROVIDER,
                    &provider.name,
                    STATUS_WARN,
                    &format!("No chain registry entry for chain id {}", provider.chain_id),
                    None,
                ));
            }
            let started = Instant::now();
            let probe = tokio::time::timeout(
                PROVIDER_PROBE_TIMEOUT,
                self.evm_chain_id_for_provider(provider.compartment_id, provider),
            )
            .await;
            let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

            let check = match probe {
                Ok(Ok(chain_id)) if chain_id == provider.chain_id => result(
                    DOMAIN_PROVIDER,
                    &provider.name,
                    STATUS_PASS,
                    &format!("Chain id {chain_id} verified"),
                    Some(latency_ms),
                ),
                // Wrong-chain treasury operations are dangerous, so a
                // reachable provider on the wrong network is a hard failure.
                Ok(Ok(chain_id)) => result(
                    DOMAIN_PROVIDER,
                    &provider.name,
                    STATUS_FAIL,
                    &format!(
                        "Chain ID mismatch: provider reports {chain_id}, profile says {}",
                        provider.chain_id
                    ),
                    Some(latency_ms),
                ),
                Ok(Err(error)) => result(
                    DOMAIN_PROVIDER,
                    &provider.name,
                    STATUS_FAIL,
                    &format!("RPC unreachable: {}", error.message()),
                    None,
                ),
                Err(_) => result(
                    DOMAIN_PROVIDER,
                    &provider.name,
                    STATUS_FAIL,
                    &format!(
                        "RPC unreachable: probe timed out after {}s",
                        PROVIDER_PROBE_TIMEOUT.as_secs()
                    ),
                    None,
                ),
            };
            checks.push(check);
        }
    }

    fn check_seed_wallet(
        &self,
        registry: &ProfileRegistry,
        profile: &EthSeedWalletProfile,
    ) -> SelfCheckResult {
        let mnemonic = self
            .with_vault(profile.compartment_id, |vault| {
                if !vault.is_unlocked() {
                    return Ok(VaultSecretPresence::Locked);
                }
                Ok(match vault.read_secret(&profile.mnemonic_secret_key) {
                    Ok(Some(_)) => VaultSecretPresence::Present,
                    Ok(None) => VaultSecretPresence::Missing,
                    Err(error) => VaultSecretPresence::Unreadable(error.to_string()),
                })
            })
            // A vault that is not registered in this process is
            // indistinguishable from a locked one for verification purposes.
            .unwrap_or(VaultSecretPresence::Locked);

        let (failures, warnings) = seed_wallet_issues(registry, profile, &mnemonic);
        compose(
            DOMAIN_SEED_WALLET,
            &profile.name,
            failures,
            warnings,
            "Derivation verified; mnemonic present; configuration well-formed",
        )
    }

    fn check_xpub_wallet(
        &self,
        registry: &ProfileRegistry,
        profile: &EthXpubWalletProfile,
    ) -> SelfCheckResult {
        let mut failures = Vec::new();
        let mut warnings = Vec::new();

        if !provider_exists(registry, &profile.provider_profile) {
            failures.push(missing_provider_detail(&profile.provider_profile));
        }

        let derivation = self.resolve_xpub_wallet_derivation(profile);
        match derivation {
            VaultDerivation::Resolvable {
                operator_asserted_path,
                ..
            } => {
                if operator_asserted_path {
                    warnings.push(
                        "external xpub path is operator-asserted metadata; xpub depth matches, but the path cannot be cryptographically bound to the imported xpub"
                            .to_string(),
                    );
                }
            }
            VaultDerivation::Locked => warnings.push(format!(
                "Vault locked for compartment {} — derivation material not verifiable",
                profile.compartment_id
            )),
            VaultDerivation::Invalid(error) => {
                failures.push(format!("Derivation material unresolvable: {error}"));
            }
        }

        if let Some(detail) = malformed_address(
            "default_destination_address",
            &profile.default_destination_address,
        ) {
            failures.push(detail);
        }

        compose(
            DOMAIN_XPUB_WALLET,
            &profile.name,
            failures,
            warnings,
            "Derivation material resolvable; configuration well-formed",
        )
    }

    fn check_receive_allocation(
        &self,
        registry: &ProfileRegistry,
        allocation: &TreasuryReceiveAllocation,
    ) -> SelfCheckResult {
        let xpub = match allocation.wallet_family.as_str() {
            WALLET_FAMILY_ETH_SEED => registry
                .eth_seed_wallets
                .iter()
                .find(|profile| profile.name == allocation.wallet_profile)
                .map(|profile| VaultDerivation::Resolvable {
                    receive_xpub: profile.receive_xpub.clone(),
                    operator_asserted_path: false,
                }),
            WALLET_FAMILY_ETH_XPUB => registry
                .eth_xpub_wallets
                .iter()
                .find(|profile| profile.name == allocation.wallet_profile)
                .map(|profile| self.resolve_xpub_wallet_derivation(profile)),
            _ => None,
        };

        let (status, detail) = match xpub {
            None => (
                STATUS_FAIL,
                "Orphaned allocation — wallet profile deleted".to_string(),
            ),
            Some(VaultDerivation::Locked) => (
                STATUS_WARN,
                format!(
                    "Vault locked — address for {} not verifiable",
                    allocation.wallet_profile
                ),
            ),
            Some(VaultDerivation::Invalid(error)) => (
                STATUS_FAIL,
                format!("Derivation material unresolvable: {error}"),
            ),
            Some(VaultDerivation::Resolvable { receive_xpub, .. }) => {
                match derive_ethereum_address_from_imported_xpub(
                    &receive_xpub,
                    allocation.address_index,
                ) {
                    Ok(derived) if derived.address.eq_ignore_ascii_case(&allocation.address) => (
                        STATUS_PASS,
                        format!(
                            "Address re-derived from {} at index {}",
                            allocation.wallet_profile, allocation.address_index
                        ),
                    ),
                    _ => (
                        STATUS_FAIL,
                        "Derivation mismatch — allocation address does not match the profile xpub"
                            .to_string(),
                    ),
                }
            }
        };
        result(
            DOMAIN_RECEIVE_ALLOCATION,
            &allocation.id,
            status,
            &detail,
            None,
        )
    }

    fn resolve_xpub_wallet_derivation(&self, profile: &EthXpubWalletProfile) -> VaultDerivation {
        if let Some(xpub) = profile
            .external_receive_xpub
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if let Some(path) = profile
                .external_receive_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return match validate_ethereum_imported_xpub_path(xpub, path)
                    .and_then(|_| derive_ethereum_address_from_imported_xpub(xpub, 0))
                {
                    Ok(_) => VaultDerivation::Resolvable {
                        receive_xpub: xpub.to_string(),
                        operator_asserted_path: true,
                    },
                    Err(error) => VaultDerivation::Invalid(error.to_string()),
                };
            }
            return match derive_ethereum_address_from_xpub(xpub, 0) {
                Ok(_) => VaultDerivation::Resolvable {
                    receive_xpub: xpub.to_string(),
                    operator_asserted_path: false,
                },
                Err(error) => VaultDerivation::Invalid(error.to_string()),
            };
        }
        if let Some(xpub) = profile
            .external_account_xpub
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let derivation = if let Some(path) = profile
                .external_account_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                derive_ethereum_receive_branch_from_account_xpub_with_path(
                    xpub,
                    path,
                    profile.project_account,
                )
                .map(|export| (export, true))
            } else {
                derive_ethereum_receive_branch_from_account_xpub(xpub, profile.project_account)
                    .map(|export| (export, false))
            };
            return match derivation {
                Ok((export, operator_asserted_path)) => VaultDerivation::Resolvable {
                    receive_xpub: export.receive_xpub,
                    operator_asserted_path,
                },
                Err(error) => VaultDerivation::Invalid(error.to_string()),
            };
        }

        self.with_vault(profile.compartment_id, |vault| {
            let Some(master_key) = vault.extract_master_key() else {
                return Ok(VaultDerivation::Locked);
            };
            Ok(
                match derive_sigillum_ethereum_xpub_receive_branch(
                    master_key.as_ref(),
                    profile.project_account,
                ) {
                    Ok(export) => VaultDerivation::Resolvable {
                        receive_xpub: export.receive_xpub,
                        operator_asserted_path: false,
                    },
                    Err(error) => VaultDerivation::Invalid(error.to_string()),
                },
            )
        })
        .unwrap_or(VaultDerivation::Locked)
    }

    /// FIDO2 sanity: enough registered keys to satisfy the highest unlocked
    /// compartment threshold, and (best effort) a device within reach.
    ///
    /// Returns `None` when FIDO2 is unconfigured — no keys means nothing to
    /// verify, matching the no-result rule for unconfigured domains.
    fn check_fido2(&self) -> Option<SelfCheckResult> {
        let status = match self.state.fido2.status() {
            Ok(status) => status,
            Err(error) => {
                return Some(result(
                    DOMAIN_FIDO2,
                    "keys",
                    STATUS_FAIL,
                    &format!("FIDO2 status unavailable: {error}"),
                    None,
                ));
            }
        };
        if !status.enabled && status.key_count == 0 {
            return None;
        }

        let mut warnings = Vec::new();
        let max_threshold = self.state.max_unlocked_threshold();
        if let Some(threshold) = max_threshold
            && status.key_count < threshold
        {
            warnings.push(format!(
                "Highest threshold {threshold} exceeds registered keys {}",
                status.key_count
            ));
        }
        // Local-only HID enumeration; never reaches the network.
        if status.key_count > 0 && sigillum_fido2::hid::detect_devices() == 0 {
            warnings.push(format!(
                "No FIDO2 device detected — {} key(s) registered",
                status.key_count
            ));
        }

        let pass_detail = match max_threshold {
            Some(threshold) => format!(
                "{} key(s) registered; highest unlocked threshold {threshold}",
                status.key_count
            ),
            None => format!(
                "{} key(s) registered; no unlocked compartment thresholds to compare",
                status.key_count
            ),
        };
        Some(compose(
            DOMAIN_FIDO2,
            "keys",
            Vec::new(),
            warnings,
            &pass_detail,
        ))
    }
}

// ── Pure checks ───────────────────────────────────────────────────

/// Seed wallet issues, split into failures and warnings.
///
/// Pure given the registry snapshot and the pre-resolved vault presence of
/// the mnemonic secret, so it is directly unit-testable.
fn seed_wallet_issues(
    registry: &ProfileRegistry,
    profile: &EthSeedWalletProfile,
    mnemonic: &VaultSecretPresence,
) -> (Vec<String>, Vec<String>) {
    let mut failures = Vec::new();
    let mut warnings = Vec::new();

    let derived_first = derive_ethereum_address_from_xpub(&profile.receive_xpub, 0);
    if !matches!(
        &derived_first,
        Ok(derived) if derived.address.eq_ignore_ascii_case(&profile.first_receive_address)
    ) {
        failures.push("Derivation mismatch — profile material is corrupt".to_string());
    }

    if !provider_exists(registry, &profile.provider_profile) {
        failures.push(missing_provider_detail(&profile.provider_profile));
    }

    match mnemonic {
        VaultSecretPresence::Present => {}
        VaultSecretPresence::Locked => warnings.push(format!(
            "Vault locked for compartment {} — mnemonic presence not verifiable",
            profile.compartment_id
        )),
        VaultSecretPresence::Missing => failures.push(format!(
            "Mnemonic secret '{}' missing from vault — re-import this seed wallet",
            profile.mnemonic_secret_key
        )),
        VaultSecretPresence::Unreadable(error) => {
            failures.push(format!("Mnemonic secret unreadable: {error}"));
        }
    }

    for (label, value) in [
        ("hot_address", &profile.hot_address),
        ("treasury_address", &profile.treasury_address),
        (
            "default_destination_address",
            &profile.default_destination_address,
        ),
    ] {
        if let Some(detail) = malformed_address(label, value) {
            failures.push(detail);
        }
    }

    (failures, warnings)
}

/// Stealth wallet profiles are pure local configuration: validate the
/// derivation label, short name, provider link, and destination format.
/// Deliberately no network calls.
fn check_stealth_wallet(
    registry: &ProfileRegistry,
    profile: &EthStealthWalletProfile,
) -> SelfCheckResult {
    let mut failures = Vec::new();

    if profile.wallet.trim().is_empty() {
        failures.push("Wallet derivation label is empty".to_string());
    }
    // Mirrors sigillum-core's short-name rule (lowercase ASCII alphanumerics)
    // without requiring an unlocked vault to exercise the derivation itself.
    let short_name = profile.short_name.trim();
    if short_name.is_empty()
        || !short_name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        failures.push(format!(
            "Invalid short name '{}' — use lowercase letters and digits",
            profile.short_name
        ));
    }
    if !provider_exists(registry, &profile.provider_profile) {
        failures.push(missing_provider_detail(&profile.provider_profile));
    }
    if let Some(detail) = malformed_address(
        "default_destination_address",
        &profile.default_destination_address,
    ) {
        failures.push(detail);
    }

    compose(
        DOMAIN_STEALTH_WALLET,
        &profile.name,
        failures,
        Vec::new(),
        "Configuration well-formed",
    )
}

fn check_watch_book_entry(entry: &WatchAddressBookEntry) -> SelfCheckResult {
    if !entry.enabled {
        return result(
            DOMAIN_WATCH_BOOK,
            &entry.address,
            STATUS_PASS,
            "disabled",
            None,
        );
    }
    match normalize_address(&entry.address) {
        Ok(_) => result(
            DOMAIN_WATCH_BOOK,
            &entry.address,
            STATUS_PASS,
            "Address well-formed",
            None,
        ),
        Err(_) => result(
            DOMAIN_WATCH_BOOK,
            &entry.address,
            STATUS_FAIL,
            "Malformed watch address",
            None,
        ),
    }
}

/// Treasury policy sanity. An absent policy is a warning, not silence:
/// operators running sweeps without guardrails should hear about it.
fn check_policy(policy: Option<&TreasuryPolicy>) -> SelfCheckResult {
    let Some(policy) = policy else {
        return result(
            DOMAIN_POLICY,
            "treasury",
            STATUS_WARN,
            "No treasury policy configured — sweeps are unguarded",
            None,
        );
    };

    let mut failures = Vec::new();
    let mut warnings = Vec::new();

    for destination in &policy.allowed_destinations {
        if normalize_address(&destination.address).is_err() {
            failures.push(format!(
                "Malformed allowlist destination: {}",
                destination.address
            ));
        }
    }

    let step_cap = decode_cap(
        "max_step_native_wei_hex",
        &policy.max_step_native_wei_hex,
        &mut failures,
    );
    let plan_cap = decode_cap(
        "max_plan_native_wei_hex",
        &policy.max_plan_native_wei_hex,
        &mut failures,
    );

    if policy.enabled && policy.allowed_destinations.is_empty() {
        warnings.push("Enabled policy with empty allowlist blocks every routed sweep".to_string());
    }
    if let (Some(step), Some(plan)) = (step_cap, plan_cap)
        && compare_u256(&step, &plan).is_gt()
    {
        warnings.push(
            "Step cap exceeds plan cap — no single step can ever use its full allowance"
                .to_string(),
        );
    }

    let pass_detail = format!(
        "Policy {} with {} allowlisted destination(s)",
        if policy.enabled {
            "enabled"
        } else {
            "disabled"
        },
        policy.allowed_destinations.len()
    );
    compose(DOMAIN_POLICY, "treasury", failures, warnings, &pass_detail)
}

// ── Shared helpers ────────────────────────────────────────────────

fn provider_exists(registry: &ProfileRegistry, name: &str) -> bool {
    registry
        .evm_providers
        .iter()
        .any(|provider| provider.name == name)
}

fn missing_provider_detail(name: &str) -> String {
    format!("Provider profile '{name}' not found in registry")
}

/// `Some(detail)` when an optional configured address fails address parsing.
fn malformed_address(label: &str, value: &Option<String>) -> Option<String> {
    let address = value.as_deref()?;
    normalize_address(address)
        .is_err()
        .then(|| format!("Malformed {label}: {address}"))
}

/// Decode an optional policy cap, recording a failure when it cannot decode
/// during enforcement.
fn decode_cap(label: &str, value: &Option<String>, failures: &mut Vec<String>) -> Option<[u8; 32]> {
    let raw = value.as_deref()?;
    match decode_quantity_hex(raw) {
        Ok(decoded) => Some(decoded),
        Err(_) => {
            failures.push(format!("{label} does not decode as a hex quantity: {raw}"));
            None
        }
    }
}

fn result(
    domain: &str,
    subject: &str,
    status: &str,
    detail: &str,
    latency_ms: Option<u64>,
) -> SelfCheckResult {
    SelfCheckResult {
        id: format!("{domain}:{subject}"),
        domain: domain.to_string(),
        subject: subject.to_string(),
        status: status.to_string(),
        detail: detail.to_string(),
        latency_ms,
    }
}

/// Fold accumulated failures and warnings into one result: any failure makes
/// the check fail, otherwise any warning makes it warn, otherwise it passes.
fn compose(
    domain: &str,
    subject: &str,
    failures: Vec<String>,
    warnings: Vec<String>,
    pass_detail: &str,
) -> SelfCheckResult {
    if !failures.is_empty() {
        return result(domain, subject, STATUS_FAIL, &failures.join("; "), None);
    }
    if !warnings.is_empty() {
        return result(domain, subject, STATUS_WARN, &warnings.join("; "), None);
    }
    result(domain, subject, STATUS_PASS, pass_detail, None)
}

/// Worst individual status wins; an empty run passes.
fn aggregate_status(checks: &[SelfCheckResult]) -> &'static str {
    if checks.iter().any(|check| check.status == STATUS_FAIL) {
        STATUS_FAIL
    } else if checks.iter().any(|check| check.status == STATUS_WARN) {
        STATUS_WARN
    } else {
        STATUS_PASS
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use sigillum_api::TreasuryAllowedDestination;

    use super::*;

    fn sample_provider(name: &str) -> EvmProviderProfile {
        EvmProviderProfile {
            name: name.into(),
            rpc_url: "http://127.0.0.1:9/".into(),
            auth_token_key: None,
            compartment_id: 0,
            chain_id: 1,
            max_priority_fee_per_gas_hex: None,
            max_fee_per_gas_hex: None,
            native_gas_limit: None,
            erc20_gas_limit: None,
            fee_estimation_enabled: false,
        }
    }

    fn registry_with_provider(name: &str) -> ProfileRegistry {
        ProfileRegistry {
            evm_providers: vec![sample_provider(name)],
            ..ProfileRegistry::default()
        }
    }

    fn sample_seed_profile() -> EthSeedWalletProfile {
        EthSeedWalletProfile {
            name: "seed-main".into(),
            label: None,
            project_account: 0,
            provider_profile: "mainnet".into(),
            compartment_id: 0,
            chain_id: None,
            word_count: 12,
            mnemonic_secret_key: "wallet.seed.seed-main.mnemonic".into(),
            account_path: "m/44'/60'/0'".into(),
            receive_path: "m/44'/60'/0'/0".into(),
            receive_xpub: "not-a-real-xpub".into(),
            first_receive_address: "0x9858effd232b4033e47d90003d41ec34ecaeda94".into(),
            default_destination_address: None,
            control_xpub: None,
            sponsor_address: None,
            hot_address: None,
            treasury_address: None,
            execution_enabled: false,
        }
    }

    fn check(status: &str) -> SelfCheckResult {
        result("provider", "x", status, "detail", None)
    }

    #[test]
    fn aggregate_status_takes_the_worst_check() {
        assert_eq!(aggregate_status(&[]), "pass");
        assert_eq!(aggregate_status(&[check("pass"), check("pass")]), "pass");
        assert_eq!(aggregate_status(&[check("pass"), check("warn")]), "warn");
        assert_eq!(
            aggregate_status(&[check("warn"), check("fail"), check("pass")]),
            "fail"
        );
    }

    #[test]
    fn compose_prefers_failures_over_warnings() {
        let failed = compose(
            "policy",
            "treasury",
            vec!["bad".into()],
            vec!["meh".into()],
            "ok",
        );
        assert_eq!(failed.status, "fail");
        assert_eq!(failed.detail, "bad");
        assert_eq!(failed.id, "policy:treasury");

        let warned = compose("policy", "treasury", Vec::new(), vec!["meh".into()], "ok");
        assert_eq!(warned.status, "warn");

        let passed = compose("policy", "treasury", Vec::new(), Vec::new(), "ok");
        assert_eq!(passed.status, "pass");
        assert_eq!(passed.detail, "ok");
        assert!(passed.latency_ms.is_none());
    }

    #[test]
    fn malformed_address_flags_only_bad_values() {
        assert!(malformed_address("hot_address", &None).is_none());
        assert!(
            malformed_address(
                "hot_address",
                &Some("0x9858effd232b4033e47d90003d41ec34ecaeda94".into())
            )
            .is_none()
        );
        let detail = malformed_address("hot_address", &Some("0x123".into())).unwrap();
        assert!(detail.contains("hot_address"));
        assert!(detail.contains("0x123"));
    }

    #[test]
    fn watch_book_disabled_entries_pass_as_disabled() {
        let entry = WatchAddressBookEntry {
            id: "w1".into(),
            address: "definitely-not-an-address".into(),
            label: "old".into(),
            tags: Vec::new(),
            source: "operator".into(),
            enabled: false,
            created_at_unix: 1,
            updated_at_unix: 1,
        };
        let check = check_watch_book_entry(&entry);
        assert_eq!(check.status, "pass");
        assert_eq!(check.detail, "disabled");

        let malformed = WatchAddressBookEntry {
            enabled: true,
            ..entry
        };
        assert_eq!(check_watch_book_entry(&malformed).status, "fail");
    }

    #[test]
    fn absent_policy_warns_about_unguarded_sweeps() {
        let check = check_policy(None);
        assert_eq!(check.status, "warn");
        assert_eq!(
            check.detail,
            "No treasury policy configured — sweeps are unguarded"
        );
        assert_eq!(check.id, "policy:treasury");
    }

    fn sample_policy() -> TreasuryPolicy {
        TreasuryPolicy {
            enabled: true,
            allowed_destinations: vec![TreasuryAllowedDestination {
                address: "0x9999999999999999999999999999999999999999".into(),
                label: None,
            }],
            max_step_native_wei_hex: None,
            max_plan_native_wei_hex: None,
            require_simulation: true,
            allow_raw_digest_signing: false,
            block_cross_party_linkage: false,
            simulation_freshness_secs: 900,
            created_at_unix: 1,
            updated_at_unix: 2,
        }
    }

    #[test]
    fn enabled_policy_with_empty_allowlist_warns() {
        let mut policy = sample_policy();
        policy.allowed_destinations.clear();
        let check = check_policy(Some(&policy));
        assert_eq!(check.status, "warn");
        assert_eq!(
            check.detail,
            "Enabled policy with empty allowlist blocks every routed sweep"
        );
    }

    #[test]
    fn policy_step_cap_above_plan_cap_warns() {
        let mut policy = sample_policy();
        policy.max_step_native_wei_hex = Some("0x2".into());
        policy.max_plan_native_wei_hex = Some("0x1".into());
        let check = check_policy(Some(&policy));
        assert_eq!(check.status, "warn");
        assert!(check.detail.contains("Step cap exceeds plan cap"));
    }

    #[test]
    fn policy_with_bad_destination_or_cap_fails() {
        let mut policy = sample_policy();
        policy.allowed_destinations[0].address = "0xnope".into();
        policy.max_step_native_wei_hex = Some("zz".into());
        let check = check_policy(Some(&policy));
        assert_eq!(check.status, "fail");
        assert!(check.detail.contains("Malformed allowlist destination"));
        assert!(check.detail.contains("max_step_native_wei_hex"));
    }

    #[test]
    fn healthy_policy_passes() {
        let mut policy = sample_policy();
        policy.max_step_native_wei_hex = Some("0x1".into());
        policy.max_plan_native_wei_hex = Some("0xde0b6b3a7640000".into());
        let check = check_policy(Some(&policy));
        assert_eq!(check.status, "pass");
        assert!(check.detail.contains("enabled"));
    }

    #[test]
    fn seed_wallet_corrupt_xpub_is_a_derivation_mismatch() {
        let registry = registry_with_provider("mainnet");
        let profile = sample_seed_profile();
        let (failures, warnings) =
            seed_wallet_issues(&registry, &profile, &VaultSecretPresence::Present);
        assert_eq!(
            failures,
            vec!["Derivation mismatch — profile material is corrupt".to_string()]
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn seed_wallet_reports_missing_provider_and_locked_vault() {
        let registry = ProfileRegistry::default();
        let mut profile = sample_seed_profile();
        profile.hot_address = Some("0xbroken".into());
        let (failures, warnings) =
            seed_wallet_issues(&registry, &profile, &VaultSecretPresence::Locked);
        assert!(
            failures
                .iter()
                .any(|f| f.contains("Provider profile 'mainnet' not found"))
        );
        assert!(failures.iter().any(|f| f.contains("Malformed hot_address")));
        assert_eq!(
            warnings,
            vec!["Vault locked for compartment 0 — mnemonic presence not verifiable".to_string()]
        );
    }

    #[test]
    fn seed_wallet_missing_mnemonic_fails_when_unlocked() {
        let registry = registry_with_provider("mainnet");
        let profile = sample_seed_profile();
        let (failures, _) = seed_wallet_issues(&registry, &profile, &VaultSecretPresence::Missing);
        assert!(failures.iter().any(|f| f.contains("missing from vault")));
    }

    #[test]
    fn stealth_wallet_validates_fields_without_network() {
        let registry = registry_with_provider("mainnet");
        let good = EthStealthWalletProfile {
            name: "ops".into(),
            wallet: "ops-wallet".into(),
            short_name: "eth".into(),
            provider_profile: "mainnet".into(),
            compartment_id: 0,
            chain_id: None,
            default_destination_address: Some("0x9999999999999999999999999999999999999999".into()),
            execution_enabled: false,
        };
        assert_eq!(check_stealth_wallet(&registry, &good).status, "pass");

        let bad = EthStealthWalletProfile {
            wallet: "  ".into(),
            short_name: "ETH!".into(),
            provider_profile: "missing".into(),
            default_destination_address: Some("0xnope".into()),
            ..good
        };
        let check = check_stealth_wallet(&registry, &bad);
        assert_eq!(check.status, "fail");
        assert!(check.detail.contains("derivation label is empty"));
        assert!(check.detail.contains("Invalid short name"));
        assert!(check.detail.contains("Provider profile 'missing'"));
        assert!(check.detail.contains("default_destination_address"));
    }
}
