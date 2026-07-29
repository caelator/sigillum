use reqwest::Method;
use sigillum_api::request::{
    CounterpartyCreateRequest, CounterpartyDeleteRequest, CounterpartyUpdateRequest,
    TreasuryPolicyUpdateRequest, TreasuryReceiveAllocateRequest, TreasuryReceiveRotateRequest,
};
use sigillum_api::response::{
    CounterpartyListResponse, CounterpartyMutationResponse, TreasuryOverviewResponse,
    TreasuryPolicyMutationResponse, TreasuryPolicyResponse, TreasuryReceiveAllocationListResponse,
    TreasuryReceiveAllocationMutationResponse,
};
use sigillum_api::route_paths as p;

use crate::{ClientError, SigillumClient};

impl SigillumClient {
    pub async fn treasury_overview(&self) -> Result<TreasuryOverviewResponse, ClientError> {
        let builder = self.request(Method::GET, p::API_TREASURY_OVERVIEW);
        self.send(builder).await
    }

    pub async fn get_treasury_policy(&self) -> Result<TreasuryPolicyResponse, ClientError> {
        let builder = self.request(Method::GET, p::API_TREASURY_POLICY);
        self.send(builder).await
    }

    pub async fn update_treasury_policy(
        &self,
        request: TreasuryPolicyUpdateRequest,
    ) -> Result<TreasuryPolicyMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_TREASURY_POLICY_UPDATE)
            .json(&request);
        self.send(builder).await
    }

    pub async fn list_treasury_receive_allocations(
        &self,
    ) -> Result<TreasuryReceiveAllocationListResponse, ClientError> {
        let builder = self.request(Method::GET, p::API_TREASURY_RECEIVE_ADDRESSES);
        self.send(builder).await
    }

    pub async fn allocate_treasury_receive_address(
        &self,
        request: TreasuryReceiveAllocateRequest,
    ) -> Result<TreasuryReceiveAllocationMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_TREASURY_RECEIVE_ADDRESSES_ALLOCATE)
            .json(&request);
        self.send(builder).await
    }

    pub async fn rotate_treasury_receive_address(
        &self,
        request: TreasuryReceiveRotateRequest,
    ) -> Result<TreasuryReceiveAllocationMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_TREASURY_RECEIVE_ADDRESSES_ROTATE)
            .json(&request);
        self.send(builder).await
    }

    pub async fn list_parties(&self) -> Result<CounterpartyListResponse, ClientError> {
        let builder = self.request(Method::GET, p::API_TREASURY_PARTIES);
        self.send(builder).await
    }

    pub async fn create_party(
        &self,
        request: CounterpartyCreateRequest,
    ) -> Result<CounterpartyMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_TREASURY_PARTIES)
            .json(&request);
        self.send(builder).await
    }

    pub async fn update_party(
        &self,
        request: CounterpartyUpdateRequest,
    ) -> Result<CounterpartyMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_TREASURY_PARTIES_UPDATE)
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_party(
        &self,
        request: CounterpartyDeleteRequest,
    ) -> Result<CounterpartyMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_TREASURY_PARTIES_DELETE)
            .json(&request);
        self.send(builder).await
    }
}
