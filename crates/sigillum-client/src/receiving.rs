use reqwest::Method;
use sigillum_api::request::ReceivingDepositTagRequest;
use sigillum_api::response::{
    EthStealthDepositMutationResponse, ReceivingOverviewResponse, ReceivingRefreshResponse,
};

use crate::{ClientError, SigillumClient};

impl SigillumClient {
    pub async fn receiving_overview(&self) -> Result<ReceivingOverviewResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/receiving/overview");
        self.send(builder).await
    }

    pub async fn refresh_receiving_balances(
        &self,
    ) -> Result<ReceivingRefreshResponse, ClientError> {
        let builder = self.request(Method::POST, "/api/receiving/refresh-balances");
        self.send(builder).await
    }

    pub async fn tag_stealth_deposit(
        &self,
        request: ReceivingDepositTagRequest,
    ) -> Result<EthStealthDepositMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/receiving/deposits/tag")
            .json(&request);
        self.send(builder).await
    }
}
