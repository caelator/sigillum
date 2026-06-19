use super::encode_quantity_u256;
use super::rpc::ProviderRpcClient;
use crate::service::ServiceResult;

#[derive(Clone, Debug)]
pub(in crate::service) struct EvmBalanceObservationPlan {
    pub(in crate::service) deposit_index: usize,
    pub(in crate::service) provider_compartment_id: usize,
    pub(in crate::service) provider: sigillum_api::EvmProviderProfile,
    pub(in crate::service) owner_address: String,
    pub(in crate::service) token_address: Option<String>,
}

#[derive(Clone, Debug)]
pub(in crate::service) struct EvmBalanceObservation {
    pub(in crate::service) deposit_index: usize,
    pub(in crate::service) native_balance_wei_hex: String,
    pub(in crate::service) observed_amount_hex: String,
}

pub(super) async fn fetch_balance_observation(
    rpc: ProviderRpcClient,
    plan: EvmBalanceObservationPlan,
) -> ServiceResult<EvmBalanceObservation> {
    let (native_balance_wei_hex, observed_amount_hex) =
        if let Some(token_address) = plan.token_address.as_deref() {
            let (native_balance, token_balance) = rpc
                .get_native_and_erc20_balance(&plan.owner_address, token_address, "latest")
                .await?;
            (
                encode_quantity_u256(&native_balance),
                encode_quantity_u256(&token_balance),
            )
        } else {
            let native_balance = rpc.get_balance(&plan.owner_address, "latest").await?;
            let native_balance_wei_hex = encode_quantity_u256(&native_balance);
            (native_balance_wei_hex.clone(), native_balance_wei_hex)
        };

    Ok(EvmBalanceObservation {
        deposit_index: plan.deposit_index,
        native_balance_wei_hex,
        observed_amount_hex,
    })
}
