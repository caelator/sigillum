//! Health, status, and audit logging endpoints.
//!
//! Provides vault status reporting, recent audit event retrieval, and
//! system diagnostics for monitoring and debugging.

use sigillum_api::response::{
    ActiveCompartment, AuditResponse, DiagnosticsResponse, Fido2StatusResponse, StatusResponse,
    UnlockedCompartment,
};
use sigillum_core::SecretStore;

use super::{ServiceError, ServiceResult, SigillumService};

impl SigillumService {
    pub(crate) fn status(&self, token: Option<&str>) -> ServiceResult<StatusResponse> {
        let initialized = self.state.is_initialized();

        if !self.state.is_unlocked() {
            return Ok(StatusResponse {
                locked: true,
                initialized,
                active_compartment: None,
                unlocked_compartments: Vec::new(),
                fido2: None,
            });
        }

        let Some(token) = self.optional_session(token) else {
            return Ok(StatusResponse {
                locked: true,
                initialized,
                active_compartment: None,
                unlocked_compartments: Vec::new(),
                fido2: None,
            });
        };

        let unlocked = self.state.unlocked_compartments();
        let active_id = self.state.active_compartment_id_for(token);
        let active_compartment = if let Some(id) = active_id {
            Some(self.with_vault(id, |vault| {
                let meta = unlocked.iter().find(|meta| meta.id == id);
                let api_key_count = vault
                    .read_api_keys()
                    .map_err(|error| ServiceError::internal(error.to_string()))?
                    .len();
                let secret_count = if vault.is_unlocked() {
                    Some(
                        vault
                            .read_secrets()
                            .map_err(|error| ServiceError::internal(error.to_string()))?
                            .len(),
                    )
                } else {
                    None
                };
                Ok(ActiveCompartment {
                    compartment_id: id,
                    compartment_label: meta.map(|meta| meta.label.clone()).unwrap_or_default(),
                    api_key_count,
                    secret_count,
                })
            })?)
        } else {
            None
        };

        let fido2 = self.state.fido2.status().map_err(|error| {
            ServiceError::internal(format!("Failed to load FIDO2 status: {error}"))
        })?;

        Ok(StatusResponse {
            locked: false,
            initialized,
            active_compartment,
            unlocked_compartments: unlocked
                .into_iter()
                .map(|meta| UnlockedCompartment {
                    id: meta.id,
                    label: meta.label,
                    threshold: meta.threshold,
                    passphrase_mode: meta.passphrase_mode,
                })
                .collect(),
            fido2: Some(Fido2StatusResponse {
                enabled: fido2.enabled,
                key_count: fido2.key_count,
            }),
        })
    }

    pub(crate) async fn audit_recent(
        &self,
        token: Option<&str>,
        query: crate::audit_db::AuditQuery,
    ) -> ServiceResult<AuditResponse> {
        let _ = self.require_session(token)?;
        let events = self.state.read_audit_events(query).map_err(|error| {
            ServiceError::internal(format!("Failed to read audit log: {error}"))
        })?;
        Ok(AuditResponse { events })
    }

    pub(crate) fn audit_verify(
        &self,
        token: Option<&str>,
        scope: &str,
    ) -> ServiceResult<sigillum_api::AuditVerifyReport> {
        let _ = self.require_session(token)?;
        self.state.verify_audit_chain(scope).map_err(|error| {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                ServiceError::locked(error.to_string())
            } else if error.kind() == std::io::ErrorKind::InvalidInput {
                ServiceError::bad_request(error.to_string())
            } else {
                ServiceError::internal(format!("Failed to verify audit chain: {error}"))
            }
        })
    }

    pub(crate) fn diagnostics(&self, token: Option<&str>) -> ServiceResult<DiagnosticsResponse> {
        let _ = self.require_session(token)?;
        let queue = crate::queue_store::load_queue(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load queue: {error}")))?;
        let deposits = crate::deposits::load_deposits(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load deposits: {error}")))?;
        let queue_counts = super::queue::count_queue_states(&queue);
        let recovery = self.state.startup_recovery_summary();
        let runtime_policy = self.state.runtime_policy();
        Ok(DiagnosticsResponse {
            status: "ok".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            unlock_scope: "process-global".into(),
            session_scope: "per-session-active-compartment".into(),
            started_at_unix: self.state.started_at_unix(),
            initialized: self.state.is_initialized(),
            unlocked_compartment_count: self.state.unlocked_compartments().len(),
            active_session_count: self.state.session_count(),
            default_active_compartment_id: self.state.default_active_compartment_id(),
            max_unlocked_threshold: self.state.max_unlocked_threshold(),
            audit_log_present: self.state.audit_db_path().exists(),
            pending_operation_count: self.state.pending_operation_count(),
            queue_job_count: queue.jobs.len(),
            blocked_queue_job_count: queue_counts.blocked,
            retrying_queue_job_count: queue_counts.retrying,
            failed_queue_job_count: queue_counts.failed,
            deferred_queue_job_count: queue_counts.blocked + queue_counts.deferred_legacy,
            startup_interrupted_operation_count: recovery.interrupted_operation_count,
            startup_recovered_operation_count: recovery.recovered_operation_count,
            startup_unresolved_operation_count: recovery.unresolved_operation_count,
            startup_recovered_queue_job_count: recovery.recovered_queue_job_count,
            startup_reconciled_deposit_count: recovery.reconciled_deposit_count,
            runtime_policy: runtime_policy.as_response(),
            eth_stealth_deposit_count: deposits.eth_stealth.len(),
            funded_eth_stealth_deposit_count: deposits
                .eth_stealth
                .iter()
                .filter(|deposit| {
                    matches!(
                        deposit.status.as_str(),
                        "funded" | "funded_needs_gas" | "sweep_queued" | "sweep_failed"
                    )
                })
                .count(),
        })
    }
}
