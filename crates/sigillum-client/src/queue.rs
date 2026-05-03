//! Queue API methods.

use reqwest::Method;
use sigillum_api::request::{
    QueueEthStealthErc20SweepRequest, QueueEthStealthErc20TransferRequest,
    QueueEthStealthNativeSweepRequest, QueueEthStealthTransferRequest, QueueProcessRequest,
};

use crate::{
    ClientError, QueueEnqueueResponse, QueueJob, QueueJobListResponse, QueueProcessResponse,
    SigillumClient,
};

impl SigillumClient {
    pub async fn list_queue_jobs(&self) -> Result<Vec<QueueJob>, ClientError> {
        let builder = self.request(Method::GET, "/api/queue/jobs");
        Ok(self.send::<QueueJobListResponse>(builder).await?.jobs)
    }

    pub async fn enqueue_eth_stealth_transfer(
        &self,
        request: QueueEthStealthTransferRequest,
    ) -> Result<QueueEnqueueResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/queue/enqueue/eth-stealth-transfer")
            .json(&request);
        self.send(builder).await
    }

    pub async fn enqueue_eth_stealth_erc20_transfer(
        &self,
        request: QueueEthStealthErc20TransferRequest,
    ) -> Result<QueueEnqueueResponse, ClientError> {
        let builder = self
            .request(
                Method::POST,
                "/api/queue/enqueue/eth-stealth-erc20-transfer",
            )
            .json(&request);
        self.send(builder).await
    }

    pub async fn enqueue_eth_stealth_native_sweep(
        &self,
        request: QueueEthStealthNativeSweepRequest,
    ) -> Result<QueueEnqueueResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/queue/enqueue/eth-stealth-native-sweep")
            .json(&request);
        self.send(builder).await
    }

    pub async fn enqueue_eth_stealth_erc20_sweep(
        &self,
        request: QueueEthStealthErc20SweepRequest,
    ) -> Result<QueueEnqueueResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/queue/enqueue/eth-stealth-erc20-sweep")
            .json(&request);
        self.send(builder).await
    }

    pub async fn process_queue(
        &self,
        request: QueueProcessRequest,
    ) -> Result<QueueProcessResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/queue/process")
            .json(&request);
        self.send(builder).await
    }
}
