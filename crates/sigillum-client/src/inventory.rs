use reqwest::Method;
use sigillum_api::request::{
    ChainProfileDeleteRequest, ChainProfileUpsertRequest, NftMetadataFetchRequest,
    NftMetadataOptInDeleteRequest, NftMetadataOptInUpsertRequest, NftMetadataSettingsUpdateRequest,
    TokenRegistryDeleteRequest, TokenRegistryImportRequest, WalletInventoryAddressPruneRequest,
    WalletInventoryScanRequest, WatchAddressBookDeleteRequest, WatchAddressBookUpsertRequest,
};
use sigillum_api::response::{
    ChainProfileListResponse, ChainProfileMutationResponse, NftMetadataFetchResponse,
    NftMetadataOptInListResponse, NftMetadataOptInMutationResponse, NftMetadataSettingsResponse,
    TokenRegistryListResponse, TokenRegistryMutationResponse, WalletInventoryAddressPruneResponse,
    WalletInventoryListResponse, WalletInventoryScanResponse, WatchAddressBookListResponse,
    WatchAddressBookMutationResponse,
};
use sigillum_api::route_paths as p;

use crate::{ClientError, SigillumClient};

impl SigillumClient {
    pub async fn list_wallet_inventory(&self) -> Result<WalletInventoryListResponse, ClientError> {
        let builder = self.request(Method::GET, p::API_INVENTORY_WALLETS);
        self.send(builder).await
    }

    pub async fn scan_evm_wallet_inventory(
        &self,
        request: WalletInventoryScanRequest,
    ) -> Result<WalletInventoryScanResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_INVENTORY_SCAN_EVM)
            .json(&request);
        self.send(builder).await
    }

    pub async fn prune_wallet_inventory_addresses(
        &self,
        request: WalletInventoryAddressPruneRequest,
    ) -> Result<WalletInventoryAddressPruneResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/inventory/addresses/delete")
            .json(&request);
        self.send(builder).await
    }

    pub async fn list_nft_metadata_optins(
        &self,
    ) -> Result<NftMetadataOptInListResponse, ClientError> {
        let builder = self.request(Method::GET, p::API_INVENTORY_NFT_METADATA_OPT_INS);
        self.send(builder).await
    }

    pub async fn upsert_nft_metadata_optin(
        &self,
        request: NftMetadataOptInUpsertRequest,
    ) -> Result<NftMetadataOptInMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_INVENTORY_NFT_METADATA_OPT_INS_UPSERT)
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_nft_metadata_optin(
        &self,
        request: NftMetadataOptInDeleteRequest,
    ) -> Result<NftMetadataOptInMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_INVENTORY_NFT_METADATA_OPT_INS_DELETE)
            .json(&request);
        self.send(builder).await
    }

    pub async fn update_nft_metadata_settings(
        &self,
        request: NftMetadataSettingsUpdateRequest,
    ) -> Result<NftMetadataSettingsResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_INVENTORY_NFT_METADATA_SETTINGS)
            .json(&request);
        self.send(builder).await
    }

    pub async fn fetch_nft_metadata(
        &self,
        request: NftMetadataFetchRequest,
    ) -> Result<NftMetadataFetchResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_INVENTORY_NFT_METADATA_FETCH)
            .json(&request);
        self.send(builder).await
    }

    pub async fn list_watch_address_book(
        &self,
    ) -> Result<WatchAddressBookListResponse, ClientError> {
        let builder = self.request(Method::GET, p::API_INVENTORY_WATCH_ADDRESSES);
        self.send(builder).await
    }

    pub async fn upsert_watch_address_book_entry(
        &self,
        request: WatchAddressBookUpsertRequest,
    ) -> Result<WatchAddressBookMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_INVENTORY_WATCH_ADDRESSES_UPSERT)
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
            .request(Method::POST, p::API_INVENTORY_WATCH_ADDRESSES_DELETE)
            .json(&request);
        self.send(builder).await
    }

    pub async fn list_token_registry(&self) -> Result<TokenRegistryListResponse, ClientError> {
        let builder = self.request(Method::GET, p::API_INVENTORY_TOKEN_REGISTRY);
        self.send(builder).await
    }

    pub async fn import_token_registry(
        &self,
        request: TokenRegistryImportRequest,
    ) -> Result<TokenRegistryMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_INVENTORY_TOKEN_REGISTRY_IMPORT)
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
            .request(Method::POST, p::API_INVENTORY_TOKEN_REGISTRY_DELETE)
            .json(&request);
        self.send(builder).await
    }

    pub async fn list_chain_profiles(&self) -> Result<ChainProfileListResponse, ClientError> {
        let builder = self.request(Method::GET, p::API_CHAINS);
        self.send(builder).await
    }

    pub async fn upsert_chain_profile(
        &self,
        request: ChainProfileUpsertRequest,
    ) -> Result<ChainProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_CHAINS_UPSERT)
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_chain_profile(
        &self,
        name: &str,
    ) -> Result<ChainProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_CHAINS_DELETE)
            .json(&ChainProfileDeleteRequest { name: name.into() });
        self.send(builder).await
    }
}
