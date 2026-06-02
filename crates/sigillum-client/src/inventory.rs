use reqwest::Method;
use sigillum_api::request::{
    ChainProfileDeleteRequest, ChainProfileUpsertRequest, WalletInventoryScanRequest,
    WatchAddressBookDeleteRequest, WatchAddressBookUpsertRequest,
};
use sigillum_api::response::{
    ChainProfileListResponse, ChainProfileMutationResponse, WalletInventoryListResponse,
    WalletInventoryScanResponse, WatchAddressBookListResponse, WatchAddressBookMutationResponse,
};

use crate::{ClientError, SigillumClient};

impl SigillumClient {
    pub async fn list_wallet_inventory(&self) -> Result<WalletInventoryListResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/inventory/wallets");
        self.send(builder).await
    }

    pub async fn scan_evm_wallet_inventory(
        &self,
        request: WalletInventoryScanRequest,
    ) -> Result<WalletInventoryScanResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/inventory/scan/evm")
            .json(&request);
        self.send(builder).await
    }

    pub async fn list_watch_address_book(
        &self,
    ) -> Result<WatchAddressBookListResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/inventory/watch-addresses");
        self.send(builder).await
    }

    pub async fn upsert_watch_address_book_entry(
        &self,
        request: WatchAddressBookUpsertRequest,
    ) -> Result<WatchAddressBookMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/inventory/watch-addresses/upsert")
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_watch_address_book_entry(
        &self,
        address: &str,
    ) -> Result<WatchAddressBookMutationResponse, ClientError> {
        let request = WatchAddressBookDeleteRequest {
            address: address.to_string(),
        };
        let builder = self
            .request(Method::POST, "/api/inventory/watch-addresses/delete")
            .json(&request);
        self.send(builder).await
    }

    pub async fn list_chain_profiles(&self) -> Result<ChainProfileListResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/inventory/chains");
        self.send(builder).await
    }

    pub async fn upsert_chain_profile(
        &self,
        request: ChainProfileUpsertRequest,
    ) -> Result<ChainProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/inventory/chains/upsert")
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_chain_profile(
        &self,
        name: &str,
    ) -> Result<ChainProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/inventory/chains/delete")
            .json(&ChainProfileDeleteRequest { name: name.into() });
        self.send(builder).await
    }
}
