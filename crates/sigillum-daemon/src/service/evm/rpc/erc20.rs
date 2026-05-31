use std::collections::HashMap;

use serde_json::json;

use crate::service::ServiceResult;

use super::{JsonRpcRequest, JsonRpcResponse, ProviderRpcClient, batch_result};
use crate::service::evm::{
    erc20_allowance_call_data, erc20_balance_call_data, normalize_address, parse_quantity_u256,
};

impl ProviderRpcClient {
    pub(in crate::service::evm) async fn get_erc20_balance(
        &self,
        token_address: &str,
        owner_address: &str,
        block_tag: &str,
    ) -> ServiceResult<[u8; 32]> {
        self.get_contract_quantity(
            token_address,
            erc20_balance_call_data(owner_address)?,
            block_tag,
        )
        .await
    }

    pub(in crate::service::evm) async fn get_erc20_allowance(
        &self,
        token_address: &str,
        owner_address: &str,
        spender_address: &str,
        block_tag: &str,
    ) -> ServiceResult<[u8; 32]> {
        self.get_contract_quantity(
            token_address,
            erc20_allowance_call_data(owner_address, spender_address)?,
            block_tag,
        )
        .await
    }

    pub(in crate::service::evm) async fn get_native_and_erc20_balance(
        &self,
        owner_address: &str,
        token_address: &str,
        block_tag: &str,
    ) -> ServiceResult<([u8; 32], [u8; 32])> {
        let responses = self
            .request_batch(&[
                JsonRpcRequest {
                    jsonrpc: "2.0",
                    id: 1,
                    method: "eth_getBalance",
                    params: json!([normalize_address(owner_address)?, block_tag]),
                },
                JsonRpcRequest {
                    jsonrpc: "2.0",
                    id: 2,
                    method: "eth_call",
                    params: json!([{
                        "to": normalize_address(token_address)?,
                        "data": erc20_balance_call_data(owner_address)?,
                    }, block_tag]),
                },
            ])
            .await?;

        let mut by_id: HashMap<u64, JsonRpcResponse> = HashMap::with_capacity(responses.len());
        for response in responses {
            by_id.insert(response.id, response);
        }

        let native_balance = parse_quantity_u256(&batch_result(&mut by_id, 1, "eth_getBalance")?)?;
        let token_balance = parse_quantity_u256(&batch_result(&mut by_id, 2, "eth_call")?)?;
        Ok((native_balance, token_balance))
    }

    async fn get_contract_quantity(
        &self,
        contract_address: &str,
        data: String,
        block_tag: &str,
    ) -> ServiceResult<[u8; 32]> {
        let value = self
            .request(
                1,
                "eth_call",
                json!([{
                    "to": normalize_address(contract_address)?,
                    "data": data,
                }, block_tag]),
            )
            .await?;
        parse_quantity_u256(&value)
    }
}
