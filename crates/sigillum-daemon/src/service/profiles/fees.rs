//! Static fee resolution for profile-backed sends.

use sigillum_api::{Eip1559Fees, EvmProviderProfile};

use crate::service::{ServiceError, ServiceResult};

pub(super) fn static_profile_fees(
    provider: &EvmProviderProfile,
    chain_id: u64,
) -> ServiceResult<Eip1559Fees> {
    Ok(Eip1559Fees {
        chain_id,
        max_priority_fee_per_gas_hex: provider.max_priority_fee_per_gas_hex.clone().ok_or_else(
            || {
                ServiceError::bad_request(
                    "provider profile is missing max_priority_fee_per_gas_hex",
                )
            },
        )?,
        max_fee_per_gas_hex: provider.max_fee_per_gas_hex.clone().ok_or_else(|| {
            ServiceError::bad_request("provider profile is missing max_fee_per_gas_hex")
        })?,
    })
}
