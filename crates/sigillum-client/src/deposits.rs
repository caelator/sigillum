//! Ethereum stealth deposit client methods.

use reqwest::Method;
use sigillum_api::request::{
    EthStealthAnnouncementScanRequest, EthStealthDepositCreateErc20Request,
    EthStealthDepositCreateNativeRequest, EthStealthDepositDeleteRequest,
    EthStealthDepositEnqueueSweepRequest, EthStealthDepositRefreshRequest,
};
use sigillum_api::response::{
    EthStealthAnnouncementScanResponse, EthStealthDeposit, EthStealthDepositEnqueueSweepResponse,
    EthStealthDepositListResponse, EthStealthDepositMutationResponse,
    EthStealthDepositRefreshResponse,
};

use super::{ClientError, SigillumClient};

impl SigillumClient {
    pub async fn list_eth_stealth_deposits(&self) -> Result<Vec<EthStealthDeposit>, ClientError> {
        let builder = self.request(Method::GET, "/api/deposits/eth-stealth");
        Ok(self
            .send::<EthStealthDepositListResponse>(builder)
            .await?
            .deposits)
    }

    pub async fn create_eth_stealth_native_deposit(
        &self,
        request: EthStealthDepositCreateNativeRequest,
    ) -> Result<EthStealthDepositMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/deposits/eth-stealth/create-native")
            .json(&request);
        self.send(builder).await
    }

    pub async fn create_eth_stealth_erc20_deposit(
        &self,
        request: EthStealthDepositCreateErc20Request,
    ) -> Result<EthStealthDepositMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/deposits/eth-stealth/create-erc20")
            .json(&request);
        self.send(builder).await
    }

    pub async fn scan_eth_stealth_announcements(
        &self,
        request: EthStealthAnnouncementScanRequest,
    ) -> Result<EthStealthAnnouncementScanResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/deposits/eth-stealth/scan-announcements")
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_eth_stealth_deposit(
        &self,
        request: EthStealthDepositDeleteRequest,
    ) -> Result<EthStealthDepositMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/deposits/eth-stealth/delete")
            .json(&request);
        self.send(builder).await
    }

    pub async fn refresh_eth_stealth_deposits(
        &self,
        request: EthStealthDepositRefreshRequest,
    ) -> Result<EthStealthDepositRefreshResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/deposits/eth-stealth/refresh")
            .json(&request);
        self.send(builder).await
    }

    pub async fn enqueue_eth_stealth_deposit_sweep(
        &self,
        request: EthStealthDepositEnqueueSweepRequest,
    ) -> Result<EthStealthDepositEnqueueSweepResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/deposits/eth-stealth/enqueue-sweep")
            .json(&request);
        self.send(builder).await
    }
}
