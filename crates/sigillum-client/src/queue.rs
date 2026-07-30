//! Queue API methods.

use reqwest::Method;
use sigillum_api::request::{
    QueueEthStealthErc20SweepRequest, QueueEthStealthErc20TransferRequest,
    QueueEthStealthNativeSweepRequest, QueueEthStealthTransferRequest, QueueProcessRequest,
};
use sigillum_api::route_paths as p;

use crate::{
    ClientError, QueueEnqueueResponse, QueueExecutionPauseResponse, QueueJob, QueueJobListResponse,
    QueueProcessResponse, SigillumClient,
};

impl SigillumClient {
    pub async fn list_queue_jobs(&self) -> Result<Vec<QueueJob>, ClientError> {
        let builder = self.request(Method::GET, p::API_QUEUE_JOBS);
        Ok(self.send::<QueueJobListResponse>(builder).await?.jobs)
    }

    pub async fn enqueue_eth_stealth_transfer(
        &self,
        request: QueueEthStealthTransferRequest,
    ) -> Result<QueueEnqueueResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_QUEUE_ENQUEUE_ETH_STEALTH_TRANSFER)
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
                p::API_QUEUE_ENQUEUE_ETH_STEALTH_ERC20_TRANSFER,
            )
            .json(&request);
        self.send(builder).await
    }

    pub async fn enqueue_eth_stealth_native_sweep(
        &self,
        request: QueueEthStealthNativeSweepRequest,
    ) -> Result<QueueEnqueueResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_QUEUE_ENQUEUE_ETH_STEALTH_NATIVE_SWEEP)
            .json(&request);
        self.send(builder).await
    }

    pub async fn enqueue_eth_stealth_erc20_sweep(
        &self,
        request: QueueEthStealthErc20SweepRequest,
    ) -> Result<QueueEnqueueResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_QUEUE_ENQUEUE_ETH_STEALTH_ERC20_SWEEP)
            .json(&request);
        self.send(builder).await
    }

    pub async fn process_queue(
        &self,
        request: QueueProcessRequest,
    ) -> Result<QueueProcessResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_QUEUE_PROCESS)
            .json(&request);
        self.send(builder).await
    }

    pub async fn pause_queue(&self) -> Result<QueueExecutionPauseResponse, ClientError> {
        let builder = self.request(Method::POST, p::API_QUEUE_PAUSE);
        self.send(builder).await
    }

    pub async fn resume_queue(&self) -> Result<QueueExecutionPauseResponse, ClientError> {
        let builder = self.request(Method::POST, p::API_QUEUE_RESUME);
        self.send(builder).await
    }
}
