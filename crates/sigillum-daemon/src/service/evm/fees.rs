use sigillum_api::{Eip1559Fees, EvmFeeEstimateResponse};

use super::encode_quantity_u256;
use super::rpc::ProviderRpcClient;
use crate::service::ServiceResult;
use crate::service::helpers::multiply_u256_u64;

pub(super) async fn estimate_eip1559_fees(
    rpc: &ProviderRpcClient,
    chain_id: u64,
    gas_limit: u64,
) -> ServiceResult<EvmFeeEstimateResponse> {
    let base_fee = rpc.latest_base_fee_per_gas().await?;
    let priority_fee = rpc.max_priority_fee_per_gas().await?;
    let doubled_base = multiply_u256_u64(&base_fee, 2);
    let max_fee = add_u256_saturating(&doubled_base, &priority_fee);
    let estimated_gas_cost = multiply_u256_u64(&max_fee, gas_limit);
    Ok(EvmFeeEstimateResponse {
        fees: Eip1559Fees {
            chain_id,
            max_priority_fee_per_gas_hex: encode_quantity_u256(&priority_fee),
            max_fee_per_gas_hex: encode_quantity_u256(&max_fee),
        },
        gas_limit,
        estimated_gas_cost_wei_hex: encode_quantity_u256(&estimated_gas_cost),
        source: "eth_feeHistory+eth_maxPriorityFeePerGas".into(),
    })
}

fn add_u256_saturating(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut carry = 0u16;
    for index in (0..32).rev() {
        let sum = left[index] as u16 + right[index] as u16 + carry;
        out[index] = (sum & 0xff) as u8;
        carry = sum >> 8;
    }
    if carry > 0 { [0xff; 32] } else { out }
}
