use reqwest::Method;
use sigillum_api::request::{
    TreasuryPolicyUpdateRequest, TreasuryReceiveAllocateRequest, TreasuryReceiveRotateRequest,
};
use sigillum_api::response::{
    TreasuryOverviewResponse, TreasuryPolicyMutationResponse, TreasuryPolicyResponse,
    TreasuryReceiveAllocationListResponse, TreasuryReceiveAllocationMutationResponse,
};

use crate::{ClientError, SigillumClient};

impl SigillumClient {
    pub async fn treasury_overview(&self) -> Result<TreasuryOverviewResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/treasury/overview");
        self.send(builder).await
    }

    pub async fn get_treasury_policy(&self) -> Result<TreasuryPolicyResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/treasury/policy");
        self.send(builder).await
    }

    pub async fn update_treasury_policy(
        &self,
        request: TreasuryPolicyUpdateRequest,
    ) -> Result<TreasuryPolicyMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/treasury/policy/update")
            .json(&request);
        self.send(builder).await
    }

    pub async fn list_treasury_receive_allocations(
        &self,
    ) -> Result<TreasuryReceiveAllocationListResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/treasury/receive-addresses");
        self.send(builder).await
    }

    pub async fn allocate_treasury_receive_address(
        &self,
        request: TreasuryReceiveAllocateRequest,
    ) -> Result<TreasuryReceiveAllocationMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/treasury/receive-addresses/allocate")
            .json(&request);
        self.send(builder).await
    }

    pub async fn rotate_treasury_receive_address(
        &self,
        request: TreasuryReceiveRotateRequest,
    ) -> Result<TreasuryReceiveAllocationMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/treasury/receive-addresses/rotate")
            .json(&request);
        self.send(builder).await
    }
}
