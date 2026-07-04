use serde_json::json;

use crate::service::ServiceResult;

use super::super::parse_quantity_u64;
use super::ProviderRpcClient;

impl ProviderRpcClient {
    pub(in crate::service) async fn get_block_number(&self) -> ServiceResult<u64> {
        let value = self.request(1, "eth_blockNumber", json!([])).await?;
        parse_quantity_u64(&value)
    }
}
