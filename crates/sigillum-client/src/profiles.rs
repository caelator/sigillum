//! EVM provider and wallet profile client methods.

use reqwest::Method;
use sigillum_api::request::{
    EthSeedWalletCreateRequest, EthSeedWalletProfileUpsertRequest,
    EthStealthWalletProfileUpsertRequest, EthXpubWalletProfileUpsertRequest,
    EvmProfileDeleteRequest, EvmProviderProfileUpsertRequest,
};
use sigillum_api::response::{
    EthSeedWalletCreateResponse, EthSeedWalletProfile, EthSeedWalletProfileListResponse,
    EthSeedWalletProfileMutationResponse, EthStealthWalletProfile,
    EthStealthWalletProfileListResponse, EthStealthWalletProfileMutationResponse,
    EthXpubWalletProfile, EthXpubWalletProfileListResponse, EthXpubWalletProfileMutationResponse,
    EvmProviderProfile, EvmProviderProfileListResponse, EvmProviderProfileMutationResponse,
};

use super::{ClientError, SigillumClient};

impl SigillumClient {
    pub async fn list_evm_provider_profiles(&self) -> Result<Vec<EvmProviderProfile>, ClientError> {
        let builder = self.request(Method::GET, "/api/profiles/evm");
        Ok(self
            .send::<EvmProviderProfileListResponse>(builder)
            .await?
            .profiles)
    }

    pub async fn upsert_evm_provider_profile(
        &self,
        request: EvmProviderProfileUpsertRequest,
    ) -> Result<EvmProviderProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/evm/upsert")
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_evm_provider_profile(
        &self,
        request: EvmProfileDeleteRequest,
    ) -> Result<EvmProviderProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/evm/delete")
            .json(&request);
        self.send(builder).await
    }

    pub async fn list_eth_stealth_wallet_profiles(
        &self,
    ) -> Result<Vec<EthStealthWalletProfile>, ClientError> {
        let builder = self.request(Method::GET, "/api/profiles/eth-stealth");
        Ok(self
            .send::<EthStealthWalletProfileListResponse>(builder)
            .await?
            .profiles)
    }

    pub async fn list_eth_xpub_wallet_profiles(
        &self,
    ) -> Result<Vec<EthXpubWalletProfile>, ClientError> {
        let builder = self.request(Method::GET, "/api/profiles/eth-xpub");
        Ok(self
            .send::<EthXpubWalletProfileListResponse>(builder)
            .await?
            .profiles)
    }

    pub async fn list_eth_seed_wallet_profiles(
        &self,
    ) -> Result<Vec<EthSeedWalletProfile>, ClientError> {
        let builder = self.request(Method::GET, "/api/profiles/eth-seed");
        Ok(self
            .send::<EthSeedWalletProfileListResponse>(builder)
            .await?
            .profiles)
    }

    pub async fn upsert_eth_stealth_wallet_profile(
        &self,
        request: EthStealthWalletProfileUpsertRequest,
    ) -> Result<EthStealthWalletProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/eth-stealth/upsert")
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_eth_stealth_wallet_profile(
        &self,
        request: EvmProfileDeleteRequest,
    ) -> Result<EthStealthWalletProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/eth-stealth/delete")
            .json(&request);
        self.send(builder).await
    }

    pub async fn upsert_eth_xpub_wallet_profile(
        &self,
        request: EthXpubWalletProfileUpsertRequest,
    ) -> Result<EthXpubWalletProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/eth-xpub/upsert")
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_eth_xpub_wallet_profile(
        &self,
        request: EvmProfileDeleteRequest,
    ) -> Result<EthXpubWalletProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/eth-xpub/delete")
            .json(&request);
        self.send(builder).await
    }

    pub async fn upsert_eth_seed_wallet_profile(
        &self,
        request: EthSeedWalletProfileUpsertRequest,
    ) -> Result<EthSeedWalletProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/eth-seed/upsert")
            .json(&request);
        self.send(builder).await
    }

    /// Create a brand-new seed wallet profile from a daemon-generated BIP-39
    /// mnemonic.
    ///
    /// The returned [`EthSeedWalletCreateResponse::mnemonic`] is delivered
    /// exactly once for operator backup; the daemon keeps it only as an
    /// encrypted vault secret and never audits it.
    pub async fn create_eth_seed_wallet_profile(
        &self,
        request: EthSeedWalletCreateRequest,
    ) -> Result<EthSeedWalletCreateResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/eth-seed/create")
            .json(&request);
        self.send(builder).await
    }

    pub async fn delete_eth_seed_wallet_profile(
        &self,
        request: EvmProfileDeleteRequest,
    ) -> Result<EthSeedWalletProfileMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/profiles/eth-seed/delete")
            .json(&request);
        self.send(builder).await
    }
}
