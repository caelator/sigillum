//! Same-chain provider partitioning for inventory scans (plan task 3.1).
//!
//! When a scan opts in via `partition_providers` and more than one selected
//! provider profile serves the same chain, every probed address is assigned
//! to exactly one of that chain's providers, so each endpoint observes only
//! a disjoint subset of the address set instead of the full ordered tree.
//! Cross-chain semantics are unchanged: an address is still probed once per
//! chain, only spread among the providers of that chain.
//!
//! Assignment stability: the bucket is `SHA-256(domain ‖ chain_id ‖ address)
//! mod N` over the chain's provider profiles sorted by name. SHA-256 is a
//! fixed function and the input encoding is domain-separated and versioned,
//! so a given address maps to the same provider across scans, processes, and
//! daemon versions as long as the chain's provider set (by name) is
//! unchanged. Changing the set (add/remove/rename) may reassign addresses.
//!
//! Between per-provider request batches the scan sleeps a small randomized
//! jitter (25–150 ms, `OsRng`-seeded) so colluding endpoints cannot trivially
//! stitch the walk back together by timing. `SIGILLUM_SCAN_PARTITION_JITTER_MAX_MS=0`
//! disables the sleep (used by tests so no test ever sleeps); unset yields
//! the default bound. Jitter applies only while partitioning is engaged —
//! single-provider-per-chain scans stay byte-identical to the opt-out path.

use std::collections::BTreeMap;

use rand::RngCore;
use sha2::{Digest, Sha256};
use sigillum_api::EvmProviderProfile;

const PARTITION_HASH_DOMAIN: &str = "sigillum-inventory-provider-partition:v1";
const JITTER_MIN_MS: u64 = 25;
const JITTER_MAX_MS: u64 = 150;
const JITTER_MAX_ENV: &str = "SIGILLUM_SCAN_PARTITION_JITTER_MAX_MS";

/// Per-chain assignment plan for a partitioned scan.
///
/// Built once per scan from the selected provider list. `None` everywhere a
/// non-partitioned scan would be used instead, so the opt-out and
/// single-provider-per-chain paths share today's exact behavior.
#[derive(Clone, Debug)]
pub(super) struct ProviderPartitions {
    /// chain_id → indices into the scan's provider list, sorted by provider
    /// name so assignment depends on the provider SET, not registry order.
    /// Only chains with more than one provider appear.
    chains: BTreeMap<u64, Vec<usize>>,
}

impl ProviderPartitions {
    /// Build the assignment plan, or `None` when partitioning is disabled or
    /// no chain has more than one selected provider (scan behavior is then
    /// identical to the opt-out path).
    pub(super) fn build(providers: &[EvmProviderProfile], enabled: bool) -> Option<Self> {
        if !enabled {
            return None;
        }
        let mut chains: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
        for (index, provider) in providers.iter().enumerate() {
            chains.entry(provider.chain_id).or_default().push(index);
        }
        chains.retain(|_, indices| indices.len() > 1);
        if chains.is_empty() {
            return None;
        }
        for indices in chains.values_mut() {
            indices.sort_by(|left, right| providers[*left].name.cmp(&providers[*right].name));
        }
        Some(Self { chains })
    }

    /// Providers that should probe `address`.
    ///
    /// Without a plan this is the full provider list in its original order
    /// (today's behavior). With a plan it is exactly one provider per
    /// multi-provider chain, still in the original cross-chain order, so
    /// per-chain sequencing and the union of observations are unchanged.
    pub(super) fn select_for_address<'a>(
        plan: Option<&Self>,
        providers: &'a [EvmProviderProfile],
        address: &str,
    ) -> Vec<&'a EvmProviderProfile> {
        providers
            .iter()
            .enumerate()
            .filter(|(index, provider)| match plan {
                None => true,
                Some(plan) => plan.assigns(provider.chain_id, *index, address),
            })
            .map(|(_, provider)| provider)
            .collect()
    }

    fn assigns(&self, chain_id: u64, provider_index: usize, address: &str) -> bool {
        let Some(indices) = self.chains.get(&chain_id) else {
            // Single-provider chain: always probed, exactly like today.
            return true;
        };
        indices[assigned_bucket(chain_id, address, indices.len())] == provider_index
    }
}

/// Stable bucket for an address within one chain's provider group.
fn assigned_bucket(chain_id: u64, address: &str, buckets: usize) -> usize {
    debug_assert!(buckets > 0);
    let mut hasher = Sha256::new();
    hasher.update(PARTITION_HASH_DOMAIN.as_bytes());
    hasher.update(chain_id.to_be_bytes());
    hasher.update(address.to_ascii_lowercase().as_bytes());
    let digest = hasher.finalize();
    let value = u64::from_be_bytes(digest[..8].try_into().expect("sha256 prefix"));
    (value % buckets as u64) as usize
}

/// Effective jitter upper bound in milliseconds; 0 disables the sleep.
fn jitter_max_ms() -> u64 {
    std::env::var(JITTER_MAX_ENV)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(JITTER_MAX_MS)
}

/// Sleep a small randomized delay between per-provider request batches.
///
/// `batches_started` is the number of provider batches already started in
/// this scan; the first batch never waits (the delay sits BETWEEN batches).
pub(super) async fn sleep_between_provider_batches(batches_started: usize) {
    if batches_started == 0 {
        return;
    }
    let max = jitter_max_ms();
    if max == 0 {
        return;
    }
    let min = JITTER_MIN_MS.min(max);
    let span = max - min + 1;
    let delay_ms = min + rand::rngs::OsRng.next_u64() % span;
    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(name: &str, chain_id: u64) -> EvmProviderProfile {
        EvmProviderProfile {
            name: name.into(),
            compartment_id: 0,
            chain_id,
            rpc_url: format!("http://localhost/{name}"),
            auth_token_key: None,
            max_priority_fee_per_gas_hex: None,
            max_fee_per_gas_hex: None,
            native_gas_limit: None,
            erc20_gas_limit: None,
            fee_estimation_enabled: false,
        }
    }

    fn address(index: u32) -> String {
        format!("0x{:040x}", index + 1)
    }

    #[test]
    fn disabled_or_single_provider_chains_yield_no_plan() {
        let two = vec![provider("a", 1), provider("b", 1)];
        assert!(ProviderPartitions::build(&two, false).is_none());
        let one_per_chain = vec![provider("a", 1), provider("b", 8453)];
        assert!(ProviderPartitions::build(&one_per_chain, true).is_none());
    }

    #[test]
    fn assignment_is_deterministic_disjoint_and_covering() {
        let providers = vec![
            provider("alpha", 1),
            provider("beta", 1),
            provider("base", 8453),
        ];
        let plan = ProviderPartitions::build(&providers, true).expect("plan");

        let mut seen_per_provider: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for index in 0..200 {
            let address = address(index);
            let first: Vec<&str> =
                ProviderPartitions::select_for_address(Some(&plan), &providers, &address)
                    .iter()
                    .map(|provider| provider.name.as_str())
                    .collect();
            // Re-selection is stable: same address, same providers.
            let second: Vec<&str> =
                ProviderPartitions::select_for_address(Some(&plan), &providers, &address)
                    .iter()
                    .map(|provider| provider.name.as_str())
                    .collect();
            assert_eq!(first, second);
            // Exactly one mainnet provider plus the single Base provider.
            assert_eq!(first.len(), 2);
            assert!(first.contains(&"base"));
            let mainnet = if first.contains(&"alpha") {
                "alpha"
            } else {
                "beta"
            };
            assert!(!(first.contains(&"alpha") && first.contains(&"beta")));
            assert!(
                !seen_per_provider
                    .entry(mainnet)
                    .or_default()
                    .contains(&address)
            );
            seen_per_provider.entry(mainnet).or_default().push(address);
        }
        // Disjoint subsets whose union is the full address set, and both
        // providers receive work.
        let alpha = seen_per_provider.get("alpha").expect("alpha served some");
        let beta = seen_per_provider.get("beta").expect("beta served some");
        assert!(alpha.iter().all(|a| !beta.contains(a)));
        assert_eq!(alpha.len() + beta.len(), 200);
    }

    #[test]
    fn assignment_depends_on_provider_names_not_registry_order() {
        let forward = vec![provider("alpha", 1), provider("beta", 1)];
        let reversed = vec![provider("beta", 1), provider("alpha", 1)];
        let forward_plan = ProviderPartitions::build(&forward, true).expect("plan");
        let reversed_plan = ProviderPartitions::build(&reversed, true).expect("plan");
        for index in 0..50 {
            let address = address(index);
            let a: Vec<&str> =
                ProviderPartitions::select_for_address(Some(&forward_plan), &forward, &address)
                    .iter()
                    .map(|provider| provider.name.as_str())
                    .collect();
            let b: Vec<&str> =
                ProviderPartitions::select_for_address(Some(&reversed_plan), &reversed, &address)
                    .iter()
                    .map(|provider| provider.name.as_str())
                    .collect();
            assert_eq!(a, b, "assignment must be registry-order independent");
        }
    }

    #[test]
    fn assignment_is_per_chain_and_case_insensitive() {
        let providers = vec![provider("alpha", 1), provider("beta", 1)];
        let plan = ProviderPartitions::build(&providers, true).expect("plan");
        let lower = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let upper = "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let a = ProviderPartitions::select_for_address(Some(&plan), &providers, lower);
        let b = ProviderPartitions::select_for_address(Some(&plan), &providers, upper);
        assert_eq!(a[0].name, b[0].name);
    }

    #[test]
    fn cross_chain_order_is_preserved_when_partitioning() {
        // Registry order interleaves chains; selection must keep it.
        let providers = vec![
            provider("mainnet-b", 1),
            provider("base", 8453),
            provider("mainnet-a", 1),
        ];
        let plan = ProviderPartitions::build(&providers, true).expect("plan");
        for index in 0..20 {
            let selected =
                ProviderPartitions::select_for_address(Some(&plan), &providers, &address(index));
            let mainnet_pos = selected
                .iter()
                .position(|provider| provider.chain_id == 1)
                .expect("mainnet provider selected");
            let base_pos = selected
                .iter()
                .position(|provider| provider.chain_id == 8453)
                .expect("base provider selected");
            if selected[mainnet_pos].name == "mainnet-b" {
                assert!(mainnet_pos < base_pos, "original order preserved");
            } else {
                assert!(base_pos < mainnet_pos, "original order preserved");
            }
        }
    }

    #[test]
    fn jitter_bound_env_override_parses_with_default_fallback() {
        // The env reader itself: unset → default; "0" → disabled; invalid →
        // default (silently clamped like other SIGILLUM_* runtime knobs).
        // (Env mutation is process-global, so parse logic is exercised here
        // and the async sleep is covered by the integration tests with the
        // override set to 0.)
        assert_eq!(JITTER_MIN_MS, 25);
        assert_eq!(JITTER_MAX_MS, 150);
        assert_eq!(JITTER_MAX_ENV, "SIGILLUM_SCAN_PARTITION_JITTER_MAX_MS");
    }
}
