use reqwest::Method;
use sigillum_api::request::{
    ChainProfileDeleteRequest, ChainProfileUpsertRequest, TokenRegistryDeleteRequest,
    TokenRegistryImportRequest, WalletInventoryScanRequest, WatchAddressBookDeleteRequest,
    WatchAddressBookUpsertRequest,
};
use sigillum_api::response::{
    ChainProfileListResponse, ChainProfileMutationResponse, TokenRegistryListResponse,
    TokenRegistryMutationResponse, WalletInventoryListResponse, WalletInventoryScanResponse,
    WatchAddressBookListResponse, WatchAddressBookMutationResponse,
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

    pub async fn list_token_registry(&self) -> Result<TokenRegistryListResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/inventory/token-registry");
        self.send(builder).await
    }

    pub async fn import_token_registry(
        &self,
        request: TokenRegistryImportRequest,
    ) -> Result<TokenRegistryMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/inventory/token-registry/import")
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_token_registry_list(
        &self,
        name: &str,
    ) -> Result<TokenRegistryMutationResponse, ClientError> {
        let request = TokenRegistryDeleteRequest {
            name: name.to_string(),
        };
        let builder = self
            .request(Method::POST, "/api/inventory/token-registry/delete")
            .json(&request);
        self.send(builder).await
    }

    pub async fn list_chain_profiles(&self) -> Result<ChainProfileListResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/chains");
        self.send(builder).await
    }

    pub async fn upsert_chain_profile(
        &self,
        request: ChainProfileUpsertRequest,
    ) -> Result<ChainProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/chains/upsert")
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_chain_profile(
        &self,
        name: &str,
    ) -> Result<ChainProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/chains/delete")
            .json(&ChainProfileDeleteRequest { name: name.into() });
        self.send(builder).await
    }
}
