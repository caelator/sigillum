//! Background operation registry service methods.
//!
//! Exposes the in-memory [`crate::operation_registry::OperationRegistry`] over
//! the API: list, get, and cooperative cancel. Cancel never touches the
//! daemon's operation mutex — it flips a shared flag the worker polls — so a
//! cancel request returns promptly even while a scan holds the mutation
//! guard. Workers honor the signal at their next checkpoint and transition
//! the record to `canceled`.

use sigillum_api::{OperationListResponse, OperationMutationResponse, OperationResponse};

use crate::operation_registry::{MAX_TRACKED_OPERATIONS, OperationCancelRequest};
use crate::service::{ServiceError, ServiceResult, SigillumService};

impl SigillumService {
    pub(crate) fn list_operations(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<OperationListResponse> {
        let _ = self.require_session(token)?;
        Ok(OperationListResponse {
            operations: self.state.list_operations(MAX_TRACKED_OPERATIONS),
        })
    }

    pub(crate) fn get_operation(
        &self,
        token: Option<&str>,
        id: &str,
    ) -> ServiceResult<OperationResponse> {
        let _ = self.require_session(token)?;
        let operation = self
            .state
            .get_operation(id)
            .ok_or_else(|| ServiceError::not_found("Operation not found."))?;
        Ok(OperationResponse { operation })
    }

    pub(crate) fn cancel_operation(
        &self,
        token: Option<&str>,
        id: &str,
    ) -> ServiceResult<OperationMutationResponse> {
        let _ = self.require_session(token)?;
        match self.state.request_operation_cancel(id) {
            OperationCancelRequest::Signaled(operation)
            | OperationCancelRequest::AlreadyRequested(operation) => {
                let status = operation.state.clone();
                Ok(OperationMutationResponse { status, operation })
            }
            OperationCancelRequest::Terminal(operation) => Err(ServiceError::conflict(format!(
                "Operation is already {} and cannot be canceled.",
                operation.state
            ))),
            OperationCancelRequest::NotFound => {
                Err(ServiceError::not_found("Operation not found."))
            }
        }
    }
}
