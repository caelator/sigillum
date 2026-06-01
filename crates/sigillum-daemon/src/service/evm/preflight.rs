use crate::service::{ServiceResult, SigillumService};

pub(in crate::service) struct EvmContractCallPreflight<'a> {
    pub(in crate::service) from_address: &'a str,
    pub(in crate::service) target_address: &'a str,
    pub(in crate::service) data_hex: &'a str,
    pub(in crate::service) block_tag: &'a str,
}

impl SigillumService {
    pub(in crate::service) async fn evm_contract_call_preflight_for_provider(
        &self,
        provider_compartment_id: usize,
        provider: &sigillum_api::EvmProviderProfile,
        call: EvmContractCallPreflight<'_>,
    ) -> ServiceResult<String> {
        self.provider_rpc_for_profile(provider_compartment_id, provider)?
            .simulate_contract_call(
                call.from_address,
                call.target_address,
                call.data_hex,
                call.block_tag,
            )
            .await
    }
}
