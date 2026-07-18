use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuditQueueJobKind {
    EthStealthTransfer,
    EthStealthErc20Transfer,
    EthStealthNativeSweep,
    EthStealthErc20Sweep,
    EthStealthGasTopup,
    EthSeedTransfer,
    EthSeedNativeSweep,
    EthSeedErc20Sweep,
    PlanStepExecution,
}

impl AuditQueueJobKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::EthStealthTransfer => "eth_stealth_transfer",
            Self::EthStealthErc20Transfer => "eth_stealth_erc20_transfer",
            Self::EthStealthNativeSweep => "eth_stealth_native_sweep",
            Self::EthStealthErc20Sweep => "eth_stealth_erc20_sweep",
            Self::EthStealthGasTopup => "eth_stealth_gas_topup",
            Self::EthSeedTransfer => "eth_seed_transfer",
            Self::EthSeedNativeSweep => "eth_seed_native_sweep",
            Self::EthSeedErc20Sweep => "eth_seed_erc20_sweep",
            Self::PlanStepExecution => "plan_step_execution",
        }
    }

    pub(crate) fn from_payload(payload: &sigillum_api::QueueJobPayload) -> Self {
        match payload {
            sigillum_api::QueueJobPayload::EthStealthTransfer { .. } => Self::EthStealthTransfer,
            sigillum_api::QueueJobPayload::EthStealthErc20Transfer { .. } => {
                Self::EthStealthErc20Transfer
            }
            sigillum_api::QueueJobPayload::EthStealthNativeSweep { .. } => {
                Self::EthStealthNativeSweep
            }
            sigillum_api::QueueJobPayload::EthStealthErc20Sweep { .. } => {
                Self::EthStealthErc20Sweep
            }
            sigillum_api::QueueJobPayload::EthStealthGasTopup { .. } => Self::EthStealthGasTopup,
            sigillum_api::QueueJobPayload::EthSeedTransfer { .. } => Self::EthSeedTransfer,
            sigillum_api::QueueJobPayload::EthSeedNativeSweep { .. } => Self::EthSeedNativeSweep,
            sigillum_api::QueueJobPayload::EthSeedErc20Sweep { .. } => Self::EthSeedErc20Sweep,
            sigillum_api::QueueJobPayload::PlanStepExecution { .. } => Self::PlanStepExecution,
        }
    }
}

pub(super) fn parse_queue_job_kind(
    path: &Path,
    value: &str,
) -> Result<AuditQueueJobKind, std::io::Error> {
    // This decoder is for the legacy unversioned `queue.enqueue` record,
    // whose accepted wire contract contained exactly these four kinds. Do
    // not infer legacy evidence from the larger live enum: unknown kinds
    // must keep failing closed.
    match value {
        "eth_stealth_transfer" => Ok(AuditQueueJobKind::EthStealthTransfer),
        "eth_stealth_erc20_transfer" => Ok(AuditQueueJobKind::EthStealthErc20Transfer),
        "eth_stealth_native_sweep" => Ok(AuditQueueJobKind::EthStealthNativeSweep),
        "eth_stealth_erc20_sweep" => Ok(AuditQueueJobKind::EthStealthErc20Sweep),
        other => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "unsupported queue audit job kind {} in {}",
                other,
                path.display()
            ),
        )),
    }
}
