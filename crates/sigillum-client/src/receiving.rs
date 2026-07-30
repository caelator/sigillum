use reqwest::Method;
use sigillum_api::request::ReceivingDepositTagRequest;
use sigillum_api::response::{
    EthStealthDepositMutationResponse, ReceivingOverviewResponse, ReceivingRefreshResponse,
};
use sigillum_api::route_paths as p;

use crate::{ClientError, SigillumClient};

impl SigillumClient {
    pub async fn receiving_overview(&self) -> Result<ReceivingOverviewResponse, ClientError> {
        let builder = self.request(Method::GET, p::API_RECEIVING_OVERVIEW);
        self.send(builder).await
    }

    pub async fn refresh_receiving_balances(
        &self,
    ) -> Result<ReceivingRefreshResponse, ClientError> {
        let builder = self.request(Method::POST, p::API_RECEIVING_REFRESH_BALANCES);
        self.send(builder).await
    }

    pub async fn tag_stealth_deposit(
        &self,
        request: ReceivingDepositTagRequest,
    ) -> Result<EthStealthDepositMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_RECEIVING_DEPOSITS_TAG)
            .json(&request);
        self.send(builder).await
    }
}
