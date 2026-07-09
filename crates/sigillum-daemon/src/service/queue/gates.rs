//! Queue execution gates for post-review treasury plan execution.

use sigillum_api::{QueueJobPayload, TreasuryPolicy};

use crate::service::{ServiceError, ServiceResult, SigillumService};

pub(crate) const EXECUTION_PAUSED_REASON: &str =
    "execution_paused: queue execution is paused by the operator kill switch";

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecutionFamily {
    Sweep,
    Revoke,
    Exit,
    Claim,
    GasTopup,
}

impl ExecutionFamily {
    pub(crate) fn gate_field(&self) -> &'static str {
        match self {
            Self::Sweep => "allow_sweep_execution",
            Self::Revoke => "allow_revoke_execution",
            Self::Exit => "allow_exit_execution",
            Self::Claim => "allow_claim_execution",
            Self::GasTopup => "allow_gas_topups",
        }
    }
}

pub(crate) fn execution_gate_denial(
    policy: Option<&TreasuryPolicy>,
    family: ExecutionFamily,
) -> Option<String> {
    if policy
        .map(|policy| policy.execution_paused)
        .unwrap_or(false)
    {
        return Some(EXECUTION_PAUSED_REASON.to_string());
    }
    let Some(policy) = policy.filter(|policy| policy.enabled) else {
        return Some("execution_gate: plan execution requires an enabled treasury policy".into());
    };
    if !policy.allow_plan_execution {
        return Some("execution_gate: allow_plan_execution is disabled".into());
    }
    if !family_gate_enabled(policy, family) {
        return Some(format!(
            "execution_gate: {} is disabled",
            family.gate_field()
        ));
    }
    None
}

fn family_gate_enabled(policy: &TreasuryPolicy, family: ExecutionFamily) -> bool {
    match family {
        ExecutionFamily::Sweep => policy.allow_sweep_execution,
        ExecutionFamily::Revoke => policy.allow_revoke_execution,
        ExecutionFamily::Exit => policy.allow_exit_execution,
        ExecutionFamily::Claim => policy.allow_claim_execution,
        ExecutionFamily::GasTopup => policy.allow_gas_topups,
    }
}

pub(crate) fn queue_payload_execution_family(payload: &QueueJobPayload) -> Option<ExecutionFamily> {
    match payload {
        // EthStealth* variants are the pre-W7 stealth families: deliberately
        // NOT plan execution, so treasury execution gates must not affect them.
        // EthSeed* variants keep their W7.3 hard block in processing.rs. W7.2's
        // PlanStepExecution payloads will map to execution families here.
        QueueJobPayload::EthStealthTransfer { .. }
        | QueueJobPayload::EthStealthErc20Transfer { .. }
        | QueueJobPayload::EthStealthNativeSweep { .. }
        | QueueJobPayload::EthStealthErc20Sweep { .. }
        | QueueJobPayload::EthSeedTransfer { .. }
        | QueueJobPayload::EthSeedNativeSweep { .. }
        | QueueJobPayload::EthSeedErc20Sweep { .. } => None,
    }
}

impl SigillumService {
    pub(in crate::service) fn current_treasury_policy(
        &self,
    ) -> ServiceResult<Option<TreasuryPolicy>> {
        let state =
            crate::inventory::load_wallet_inventory(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load wallet inventory: {error}"))
            })?;
        Ok(state.treasury_policy)
    }

    pub(in crate::service) fn require_execution_family_allowed(
        &self,
        family: ExecutionFamily,
    ) -> ServiceResult<()> {
        let policy = self.current_treasury_policy()?;
        if let Some(reason) = execution_gate_denial(policy.as_ref(), family) {
            return Err(ServiceError::forbidden(reason));
        }
        Ok(())
    }

    pub(in crate::service) fn execution_gate_block_reason(
        &self,
        payload: &QueueJobPayload,
    ) -> ServiceResult<Option<String>> {
        let Some(family) = queue_payload_execution_family(payload) else {
            return Ok(None);
        };
        let policy = self.current_treasury_policy()?;
        Ok(execution_gate_denial(policy.as_ref(), family))
    }

    pub(in crate::service) fn queue_execution_paused(&self) -> ServiceResult<bool> {
        Ok(self
            .current_treasury_policy()?
            .map(|policy| policy.execution_paused)
            .unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use sigillum_api::TreasuryPolicy;

    use super::*;

    fn sample_policy() -> TreasuryPolicy {
        TreasuryPolicy {
            enabled: true,
            allowed_destinations: Vec::new(),
            max_step_native_wei_hex: None,
            max_plan_native_wei_hex: None,
            require_simulation: true,
            allow_raw_digest_signing: false,
            block_cross_party_linkage: false,
            allow_claim_execution: false,
            allow_gas_topups: false,
            max_gas_topup_wei_hex: None,
            allow_plan_execution: true,
            allow_sweep_execution: false,
            allow_revoke_execution: false,
            allow_exit_execution: false,
            execution_paused: false,
            max_fee_per_gas_cap_hex: None,
            simulation_freshness_secs: 900,
            hot_floor_wei_hex: "0xde0b6b3a7640000".into(),
            hot_target_wei_hex: "0xde0b6b3a7640000".into(),
            created_at_unix: 1,
            updated_at_unix: 1,
        }
    }

    fn enable_family(policy: &mut TreasuryPolicy, family: ExecutionFamily) {
        match family {
            ExecutionFamily::Sweep => policy.allow_sweep_execution = true,
            ExecutionFamily::Revoke => policy.allow_revoke_execution = true,
            ExecutionFamily::Exit => policy.allow_exit_execution = true,
            ExecutionFamily::Claim => policy.allow_claim_execution = true,
            ExecutionFamily::GasTopup => policy.allow_gas_topups = true,
        }
    }

    #[test]
    fn execution_gate_denial_covers_all_families() {
        for family in [
            ExecutionFamily::Sweep,
            ExecutionFamily::Revoke,
            ExecutionFamily::Exit,
            ExecutionFamily::Claim,
            ExecutionFamily::GasTopup,
        ] {
            assert_eq!(
                execution_gate_denial(None, family).as_deref(),
                Some("execution_gate: plan execution requires an enabled treasury policy")
            );

            let mut policy = sample_policy();
            policy.allow_plan_execution = false;
            enable_family(&mut policy, family);
            assert_eq!(
                execution_gate_denial(Some(&policy), family).as_deref(),
                Some("execution_gate: allow_plan_execution is disabled")
            );

            let policy = sample_policy();
            assert_eq!(
                execution_gate_denial(Some(&policy), family),
                Some(format!(
                    "execution_gate: {} is disabled",
                    family.gate_field()
                ))
            );

            let mut policy = sample_policy();
            enable_family(&mut policy, family);
            policy.execution_paused = true;
            assert_eq!(
                execution_gate_denial(Some(&policy), family).as_deref(),
                Some(EXECUTION_PAUSED_REASON)
            );

            let mut policy = sample_policy();
            policy.enabled = false;
            policy.execution_paused = true;
            assert_eq!(
                execution_gate_denial(Some(&policy), family).as_deref(),
                Some(EXECUTION_PAUSED_REASON)
            );

            let mut policy = sample_policy();
            enable_family(&mut policy, family);
            assert_eq!(execution_gate_denial(Some(&policy), family), None);
        }
    }

    #[test]
    fn current_queue_payloads_are_not_plan_execution_families() {
        let payloads = [
            QueueJobPayload::EthStealthTransfer {
                wallet_profile: "stealth".into(),
                stealth_address: "0x1111111111111111111111111111111111111111".into(),
                ephemeral_public_key_hex: "02aa".into(),
                value_wei_hex: "0x1".into(),
                destination_address: None,
                nonce: None,
                gas_limit: None,
                view_tag_hex: None,
            },
            QueueJobPayload::EthStealthErc20Transfer {
                wallet_profile: "stealth".into(),
                stealth_address: "0x1111111111111111111111111111111111111111".into(),
                ephemeral_public_key_hex: "02aa".into(),
                token_address: "0x2222222222222222222222222222222222222222".into(),
                recipient_address: "0x3333333333333333333333333333333333333333".into(),
                amount_hex: "0x1".into(),
                nonce: None,
                gas_limit: None,
                view_tag_hex: None,
            },
            QueueJobPayload::EthStealthNativeSweep {
                wallet_profile: "stealth".into(),
                stealth_address: "0x1111111111111111111111111111111111111111".into(),
                ephemeral_public_key_hex: "02aa".into(),
                destination_address: None,
                min_value_wei_hex: None,
                gas_limit: None,
                view_tag_hex: None,
            },
            QueueJobPayload::EthStealthErc20Sweep {
                wallet_profile: "stealth".into(),
                stealth_address: "0x1111111111111111111111111111111111111111".into(),
                ephemeral_public_key_hex: "02aa".into(),
                token_address: "0x2222222222222222222222222222222222222222".into(),
                recipient_address: None,
                min_amount_hex: None,
                gas_limit: None,
                view_tag_hex: None,
            },
            QueueJobPayload::EthSeedTransfer {
                wallet_profile: "seed".into(),
                address: "0x1111111111111111111111111111111111111111".into(),
                derivation_path: "m/44'/60'/0'/0/0".into(),
                value_wei_hex: "0x1".into(),
                destination_address: "0x3333333333333333333333333333333333333333".into(),
                nonce: None,
                gas_limit: None,
            },
            QueueJobPayload::EthSeedNativeSweep {
                wallet_profile: "seed".into(),
                address: "0x1111111111111111111111111111111111111111".into(),
                derivation_path: "m/44'/60'/0'/0/0".into(),
                destination_address: None,
                min_value_wei_hex: None,
                gas_limit: None,
            },
            QueueJobPayload::EthSeedErc20Sweep {
                wallet_profile: "seed".into(),
                address: "0x1111111111111111111111111111111111111111".into(),
                derivation_path: "m/44'/60'/0'/0/0".into(),
                token_address: "0x2222222222222222222222222222222222222222".into(),
                recipient_address: None,
                min_amount_hex: None,
                gas_limit: None,
            },
        ];

        for payload in payloads {
            assert_eq!(queue_payload_execution_family(&payload), None);
        }
    }
}
