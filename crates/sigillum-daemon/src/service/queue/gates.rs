//! Queue execution gates for post-review treasury plan execution.

use sigillum_api::{QueueJobPayload, TreasuryPolicy, WalletPlanStepAction};

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

    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Sweep => "sweep",
            Self::Revoke => "revoke",
            Self::Exit => "exit",
            Self::Claim => "claim",
            Self::GasTopup => "gas_topup",
        }
    }
}

/// Execution family for a consolidation plan-step action; `None` means the
/// action is not executable (`ReviewAsset`) and must never be enqueued.
pub(crate) fn plan_action_execution_family(
    action: &WalletPlanStepAction,
) -> Option<ExecutionFamily> {
    match action {
        WalletPlanStepAction::SweepNative
        | WalletPlanStepAction::SweepErc20
        | WalletPlanStepAction::SweepNft => Some(ExecutionFamily::Sweep),
        WalletPlanStepAction::RevokeErc20Approval
        | WalletPlanStepAction::RevokePermit2Allowance
        | WalletPlanStepAction::RevokeNftOperatorApproval
        | WalletPlanStepAction::RevokeApproval
        | WalletPlanStepAction::ApproveErc20 => Some(ExecutionFamily::Revoke),
        WalletPlanStepAction::ExitDefiPosition => Some(ExecutionFamily::Exit),
        WalletPlanStepAction::ClaimReward => Some(ExecutionFamily::Claim),
        WalletPlanStepAction::FundGas => Some(ExecutionFamily::GasTopup),
        // ReviewAsset is informational and unknown wire values (D-18 `Other`)
        // have no execution semantics: neither is ever executable.
        WalletPlanStepAction::ReviewAsset | WalletPlanStepAction::Other(_) => None,
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
        QueueJobPayload::EthStealthTransfer { .. }
        | QueueJobPayload::EthStealthErc20Transfer { .. }
        | QueueJobPayload::EthStealthNativeSweep { .. }
        | QueueJobPayload::EthStealthErc20Sweep { .. } => None,
        // EthSeed* variants (W7.3): fund movement out of a seed-derived
        // wallet is a Sweep-family execution regardless of shape (plain
        // transfer or threshold sweep) — there is no separate "transfer"
        // execution family, and none of these three should ever bypass the
        // gates the way the pre-W7 stealth families do.
        QueueJobPayload::EthSeedTransfer { .. }
        | QueueJobPayload::EthSeedNativeSweep { .. }
        | QueueJobPayload::EthSeedErc20Sweep { .. } => Some(ExecutionFamily::Sweep),
        // A non-executable action (ReviewAsset) is unreachable by construction
        // (enqueue refuses it); if such a payload ever appears, gate it under
        // a family rather than exempting it from gates (fail closed).
        QueueJobPayload::PlanStepExecution(payload) => {
            Some(plan_action_execution_family(&payload.action).unwrap_or(ExecutionFamily::Sweep))
        }
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
    use std::sync::Arc;

    use sigillum_api::QueueJobPayload;
    use sigillum_api::TreasuryPolicy;
    use tempfile::TempDir;

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
            hot_overflow_wei_hex: None,
            allow_treasury_automation: false,
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

    fn all_families() -> [ExecutionFamily; 5] {
        [
            ExecutionFamily::Sweep,
            ExecutionFamily::Revoke,
            ExecutionFamily::Exit,
            ExecutionFamily::Claim,
            ExecutionFamily::GasTopup,
        ]
    }

    fn test_service() -> (TempDir, SigillumService) {
        let dir = TempDir::new().unwrap();
        let state = Arc::new(
            crate::AppState::new(dir.path().to_path_buf()).expect("app state should initialize"),
        );
        let service = SigillumService::new(state);
        (dir, service)
    }

    fn persist_policy(dir: &TempDir, policy: Option<TreasuryPolicy>) {
        let mut state = crate::inventory::load_wallet_inventory(dir.path()).unwrap();
        state.treasury_policy = policy;
        crate::inventory::save_wallet_inventory(dir.path(), &state).unwrap();
    }

    fn forbidden_message(result: ServiceResult<()>) -> String {
        let error = result.unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::FORBIDDEN);
        error.message().to_string()
    }

    fn deny_from_current_policy(service: &SigillumService, family: ExecutionFamily) -> String {
        execution_gate_denial(service.current_treasury_policy().unwrap().as_ref(), family).unwrap()
    }

    #[test]
    fn execution_gate_denial_covers_all_families() {
        for family in all_families() {
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
    fn current_stealth_queue_payloads_are_not_plan_execution_families() {
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
        ];

        for payload in payloads {
            assert_eq!(queue_payload_execution_family(&payload), None);
        }
    }

    /// W7.3: EthSeed* queue payloads now gate the same way `PlanStepExecution`
    /// sweep steps do (`ExecutionFamily::Sweep`) — the block on
    /// `service/queue/processing.rs`'s hard-coded seed-wallet message lifts
    /// behind these gates instead of applying unconditionally.
    #[test]
    fn eth_seed_queue_payloads_are_gated_as_sweep_family() {
        let payloads = [
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
            assert_eq!(
                queue_payload_execution_family(&payload),
                Some(ExecutionFamily::Sweep)
            );
        }
    }

    #[test]
    fn enqueue_time_gate_negatives_per_family() {
        for family in all_families() {
            let (dir, service) = test_service();

            persist_policy(&dir, None);
            let message = forbidden_message(service.require_execution_family_allowed(family));
            assert!(message.contains("enabled treasury policy"));

            let mut policy = sample_policy();
            policy.allow_plan_execution = false;
            enable_family(&mut policy, family);
            persist_policy(&dir, Some(policy));
            let message = forbidden_message(service.require_execution_family_allowed(family));
            assert!(message.contains("allow_plan_execution"));

            let policy = sample_policy();
            persist_policy(&dir, Some(policy));
            let message = forbidden_message(service.require_execution_family_allowed(family));
            assert!(message.contains(family.gate_field()));

            let mut policy = sample_policy();
            enable_family(&mut policy, family);
            policy.execution_paused = true;
            persist_policy(&dir, Some(policy));
            let message = forbidden_message(service.require_execution_family_allowed(family));
            assert!(message.contains("execution_paused"));

            let mut policy = sample_policy();
            enable_family(&mut policy, family);
            persist_policy(&dir, Some(policy));
            service.require_execution_family_allowed(family).unwrap();
        }
    }

    #[test]
    fn execution_time_gate_negatives_per_family() {
        for family in all_families() {
            let (dir, service) = test_service();

            persist_policy(&dir, None);
            assert_eq!(
                deny_from_current_policy(&service, family),
                "execution_gate: plan execution requires an enabled treasury policy"
            );
            assert!(!service.queue_execution_paused().unwrap());

            let mut policy = sample_policy();
            policy.allow_plan_execution = false;
            enable_family(&mut policy, family);
            persist_policy(&dir, Some(policy));
            assert_eq!(
                deny_from_current_policy(&service, family),
                "execution_gate: allow_plan_execution is disabled"
            );
            assert!(!service.queue_execution_paused().unwrap());

            let policy = sample_policy();
            persist_policy(&dir, Some(policy));
            assert_eq!(
                deny_from_current_policy(&service, family),
                format!("execution_gate: {} is disabled", family.gate_field())
            );
            assert!(!service.queue_execution_paused().unwrap());

            let mut policy = sample_policy();
            enable_family(&mut policy, family);
            policy.execution_paused = true;
            persist_policy(&dir, Some(policy));
            assert_eq!(
                deny_from_current_policy(&service, family),
                EXECUTION_PAUSED_REASON
            );
            assert!(service.queue_execution_paused().unwrap());

            let mut policy = sample_policy();
            enable_family(&mut policy, family);
            persist_policy(&dir, Some(policy));
            assert_eq!(
                execution_gate_denial(service.current_treasury_policy().unwrap().as_ref(), family),
                None
            );
            assert!(!service.queue_execution_paused().unwrap());
        }
    }

    #[test]
    fn stealth_payloads_bypass_gate_block_reason_even_under_hostile_policy() {
        let (dir, service) = test_service();
        let mut policy = sample_policy();
        policy.allow_plan_execution = false;
        policy.execution_paused = true;
        persist_policy(&dir, Some(policy));

        let payload = QueueJobPayload::EthStealthNativeSweep {
            wallet_profile: "stealth".into(),
            stealth_address: "0x1111111111111111111111111111111111111111".into(),
            ephemeral_public_key_hex: "02aa".into(),
            destination_address: None,
            min_value_wei_hex: None,
            gas_limit: None,
            view_tag_hex: None,
        };

        assert_eq!(service.execution_gate_block_reason(&payload).unwrap(), None);
        assert!(service.queue_execution_paused().unwrap());
    }

    fn plan_step_payload(action: &str) -> QueueJobPayload {
        QueueJobPayload::PlanStepExecution(Box::new(sigillum_api::PlanStepExecutionPayload {
            plan_id: "plan_1".into(),
            step_id: "step_1".into(),
            chain_id: 1,
            source_address: "0x1111111111111111111111111111111111111111".into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            wallet_family: "eth-seed".into(),
            wallet_profile: "seed-main".into(),
            provider_profile: "mainnet".into(),
            action: action.into(),
            asset_kind: "native".into(),
            asset_address: None,
            amount_hex: "0x1".into(),
            destination_address: Some("0x9999999999999999999999999999999999999999".into()),
            call_label: "native.transfer(value)".into(),
            call_target_address: "0x9999999999999999999999999999999999999999".into(),
            call_data_hex: "0x".into(),
            call_value_wei_hex: Some("0x1".into()),
            simulation_evidence_hash_hex: "ab".repeat(32),
            fee_basis: None,
            max_priority_fee_per_gas_hex: None,
            max_fee_per_gas_hex: None,
            prerequisite_job_ids: Vec::new(),
        }))
    }

    #[test]
    fn plan_step_actions_map_to_execution_families() {
        let expectations = [
            ("sweep_native", Some(ExecutionFamily::Sweep)),
            ("sweep_erc20", Some(ExecutionFamily::Sweep)),
            ("sweep_nft", Some(ExecutionFamily::Sweep)),
            ("revoke_erc20_approval", Some(ExecutionFamily::Revoke)),
            ("revoke_permit2_allowance", Some(ExecutionFamily::Revoke)),
            (
                "revoke_nft_operator_approval",
                Some(ExecutionFamily::Revoke),
            ),
            ("revoke_approval", Some(ExecutionFamily::Revoke)),
            ("approve_erc20", Some(ExecutionFamily::Revoke)),
            ("exit_defi_position", Some(ExecutionFamily::Exit)),
            ("claim_reward", Some(ExecutionFamily::Claim)),
            ("fund_gas", Some(ExecutionFamily::GasTopup)),
            ("review_asset", None),
        ];
        for (action, expected) in expectations {
            assert_eq!(
                plan_action_execution_family(&action.into()),
                expected,
                "action {action}"
            );
        }
    }

    #[test]
    fn plan_step_payloads_are_gated_per_action_family() {
        assert_eq!(
            queue_payload_execution_family(&plan_step_payload("sweep_native")),
            Some(ExecutionFamily::Sweep)
        );
        assert_eq!(
            queue_payload_execution_family(&plan_step_payload("claim_reward")),
            Some(ExecutionFamily::Claim)
        );
        // Fail closed: a non-executable action never escapes the gates.
        assert_eq!(
            queue_payload_execution_family(&plan_step_payload("review_asset")),
            Some(ExecutionFamily::Sweep)
        );
    }

    #[test]
    fn plan_step_payload_blocked_by_gates_under_hostile_policy() {
        let (dir, service) = test_service();

        persist_policy(&dir, None);
        assert_eq!(
            service
                .execution_gate_block_reason(&plan_step_payload("sweep_native"))
                .unwrap()
                .as_deref(),
            Some("execution_gate: plan execution requires an enabled treasury policy")
        );

        let mut policy = sample_policy();
        policy.allow_plan_execution = false;
        policy.allow_sweep_execution = true;
        persist_policy(&dir, Some(policy));
        assert_eq!(
            service
                .execution_gate_block_reason(&plan_step_payload("sweep_native"))
                .unwrap()
                .as_deref(),
            Some("execution_gate: allow_plan_execution is disabled")
        );

        let policy = sample_policy();
        persist_policy(&dir, Some(policy));
        assert_eq!(
            service
                .execution_gate_block_reason(&plan_step_payload("sweep_native"))
                .unwrap()
                .as_deref(),
            Some("execution_gate: allow_sweep_execution is disabled")
        );

        let mut policy = sample_policy();
        policy.allow_sweep_execution = true;
        policy.execution_paused = true;
        persist_policy(&dir, Some(policy));
        assert_eq!(
            service
                .execution_gate_block_reason(&plan_step_payload("sweep_native"))
                .unwrap()
                .as_deref(),
            Some(EXECUTION_PAUSED_REASON)
        );

        let mut policy = sample_policy();
        policy.allow_sweep_execution = true;
        persist_policy(&dir, Some(policy));
        assert_eq!(
            service
                .execution_gate_block_reason(&plan_step_payload("sweep_native"))
                .unwrap(),
            None
        );
    }
}
