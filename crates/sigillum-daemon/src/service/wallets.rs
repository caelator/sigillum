//! Ethereum stealth wallet operations.
//!
//! Provides wallet export, address generation, checking, and transaction signing
//! for Ethereum stealth addresses using master key derivation.

use sigillum_api::{
    EthSignedTransactionResponse, EthStealthAnnouncementPayload, EthStealthCheckRequest,
    EthStealthCheckResponse, EthStealthExportRequest, EthStealthGenerateRequest,
    EthStealthGenerateResponse, EthStealthMetaAddressResponse, EthStealthSignErc20TransferRequest,
    EthStealthSignRequest, EthStealthSignResponse, EthStealthSignTransferRequest,
    EthXpubAddressResponse, EthXpubDeriveRequest, EthXpubExportRequest, EthXpubExportResponse,
};
use sigillum_core::{
    EthereumEip1559Erc20Transfer, EthereumEip1559Transfer, VaultLifecycle,
    build_erc5564_announcement, check_ethereum_stealth_address, decode_quantity_hex,
    derive_ethereum_address_from_xpub, derive_ethereum_receive_branch_from_account_xpub,
    derive_ethereum_receive_branch_from_account_xpub_with_path,
    derive_sigillum_ethereum_stealth_wallet, derive_sigillum_ethereum_xpub_receive_branch,
    generate_ethereum_stealth_address, sign_ethereum_stealth_digest,
    sign_ethereum_stealth_erc20_transfer, sign_ethereum_stealth_native_transfer,
};
use zeroize::Zeroizing;

use crate::audit_log::AuditEventSpec;

use super::helpers::{
    decode_fixed_hex, decode_optional_view_tag, map_wallet_error, map_xpub_error,
};
use super::transaction_policy::{TransactionPolicyCheck, TransactionPolicyKind};
use super::{ServiceError, ServiceResult, SigillumService};

impl SigillumService {
    pub(crate) fn eth_xpub_export(
        &self,
        token: Option<&str>,
        body: EthXpubExportRequest,
    ) -> ServiceResult<EthXpubExportResponse> {
        let token = self.require_session(token)?;
        let (_provider, profile) = self.resolve_xpub_wallet_profile(&body.wallet_profile)?;
        let active_compartment_id = self
            .state
            .active_compartment_id_for(token)
            .ok_or_else(|| ServiceError::forbidden("No active compartment."))?;
        if active_compartment_id != profile.compartment_id {
            return Err(ServiceError::forbidden(
                "Wallet profile is not in the active compartment.",
            ));
        }
        let export = if let Some(receive_xpub) = profile
            .external_receive_xpub
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let receive_path = profile
                .external_receive_path
                .clone()
                .unwrap_or_else(|| eth_receive_path(profile.project_account));
            EthXpubExportResponse {
                wallet_profile: profile.name.clone(),
                project_account: profile.project_account,
                account_path: parent_derivation_path(&receive_path),
                receive_path,
                receive_xpub: receive_xpub.to_string(),
            }
        } else if let Some(account_xpub) = profile
            .external_account_xpub
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let export = if let Some(path) = profile
                .external_account_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                derive_ethereum_receive_branch_from_account_xpub_with_path(
                    account_xpub,
                    path,
                    profile.project_account,
                )
            } else {
                derive_ethereum_receive_branch_from_account_xpub(
                    account_xpub,
                    profile.project_account,
                )
            }
            .map_err(map_xpub_error)?;
            EthXpubExportResponse {
                wallet_profile: profile.name.clone(),
                project_account: export.project_account,
                account_path: export.account_path,
                receive_path: export.receive_path,
                receive_xpub: export.receive_xpub,
            }
        } else {
            let export = self.with_active_vault(token, |vault, _| {
                let master_key = vault
                    .extract_master_key()
                    .ok_or_else(|| ServiceError::forbidden("Vault is locked."))?;
                derive_sigillum_ethereum_xpub_receive_branch(
                    master_key.as_ref(),
                    profile.project_account,
                )
                .map_err(map_xpub_error)
            })?;
            EthXpubExportResponse {
                wallet_profile: profile.name.clone(),
                project_account: export.project_account,
                account_path: export.account_path,
                receive_path: export.receive_path,
                receive_xpub: export.receive_xpub,
            }
        };

        self.record_audit(
            Some(profile.compartment_id),
            AuditEventSpec::WalletEthXpubExport {
                wallet_profile: profile.name.clone(),
                project_account: profile.project_account,
            },
        )?;

        Ok(export)
    }

    pub(crate) fn eth_xpub_derive(
        &self,
        body: EthXpubDeriveRequest,
    ) -> ServiceResult<EthXpubAddressResponse> {
        let derived =
            derive_ethereum_address_from_xpub(&body.xpub, body.index).map_err(map_xpub_error)?;
        Ok(EthXpubAddressResponse {
            index: derived.index,
            address: derived.address,
        })
    }

    pub(crate) fn eth_stealth_export(
        &self,
        token: Option<&str>,
        body: EthStealthExportRequest,
    ) -> ServiceResult<EthStealthMetaAddressResponse> {
        let token = self.require_session(token)?;
        let wallet = body.wallet;
        let short_name = body.short_name.unwrap_or_else(|| "eth".to_string());

        let (meta_address, compartment_id) =
            self.with_active_vault(token, |vault, compartment_id| {
                let master_key = vault
                    .extract_master_key()
                    .ok_or_else(|| ServiceError::forbidden("Vault is locked."))?;
                let derived = derive_sigillum_ethereum_stealth_wallet(
                    master_key.as_ref(),
                    &wallet,
                    &short_name,
                )
                .map_err(map_wallet_error)?;
                Ok((derived.meta_address().clone(), compartment_id))
            })?;

        self.record_audit(
            Some(compartment_id),
            AuditEventSpec::WalletEthStealthExport {
                wallet: wallet.clone(),
                short_name: meta_address.short_name.clone(),
            },
        )?;

        Ok(EthStealthMetaAddressResponse {
            wallet: meta_address.wallet,
            short_name: meta_address.short_name,
            scheme_id: meta_address.scheme_id,
            stealth_meta_address: meta_address.stealth_meta_address,
            spending_public_key_hex: meta_address.spending_public_key_hex,
            viewing_public_key_hex: meta_address.viewing_public_key_hex,
        })
    }

    pub(crate) fn eth_stealth_generate(
        &self,
        body: EthStealthGenerateRequest,
    ) -> ServiceResult<EthStealthGenerateResponse> {
        let ephemeral_private_key = body
            .ephemeral_private_key_hex
            .as_deref()
            .map(|value| decode_fixed_hex::<32>(value, "ephemeral_private_key"))
            .transpose()?;
        let payment =
            generate_ethereum_stealth_address(&body.stealth_meta_address, ephemeral_private_key)
                .map_err(map_wallet_error)?;
        let announcement = build_erc5564_announcement(&payment).map_err(map_wallet_error)?;

        Ok(EthStealthGenerateResponse {
            short_name: payment.short_name,
            scheme_id: payment.scheme_id,
            stealth_meta_address: payment.stealth_meta_address,
            stealth_address: payment.stealth_address,
            ephemeral_public_key_hex: payment.ephemeral_public_key_hex,
            view_tag_hex: payment.view_tag_hex,
            announcement: Some(EthStealthAnnouncementPayload {
                announcer_address: announcement.announcer_address,
                announce_function: announcement.announce_function,
                scheme_id: announcement.scheme_id,
                stealth_address: announcement.stealth_address,
                ephemeral_public_key_hex: announcement.ephemeral_public_key_hex,
                metadata_hex: announcement.metadata_hex,
                calldata_hex: announcement.calldata_hex,
                value_wei_hex: announcement.value_wei_hex,
            }),
        })
    }

    pub(crate) fn eth_stealth_check(
        &self,
        token: Option<&str>,
        body: EthStealthCheckRequest,
    ) -> ServiceResult<EthStealthCheckResponse> {
        let token = self.require_session(token)?;
        let wallet = body.wallet;
        let view_tag = decode_optional_view_tag(body.stealth.view_tag_hex.as_deref())?;
        let stealth_address = body.stealth.stealth_address;
        let ephemeral_public_key_hex = body.stealth.ephemeral_public_key_hex;

        let (check, compartment_id) = self.with_active_vault(token, |vault, compartment_id| {
            let master_key = vault
                .extract_master_key()
                .ok_or_else(|| ServiceError::forbidden("Vault is locked."))?;
            let derived =
                derive_sigillum_ethereum_stealth_wallet(master_key.as_ref(), &wallet, "eth")
                    .map_err(map_wallet_error)?;
            let check = check_ethereum_stealth_address(
                &derived,
                &stealth_address,
                &ephemeral_public_key_hex,
                view_tag,
            )
            .map_err(map_wallet_error)?;
            Ok((check, compartment_id))
        })?;

        self.record_audit(
            Some(compartment_id),
            AuditEventSpec::WalletEthStealthCheck {
                wallet: wallet.clone(),
                matches: check.matches,
            },
        )?;

        Ok(EthStealthCheckResponse {
            wallet,
            matches: check.matches,
            derived_stealth_address: check.derived_stealth_address,
            view_tag_hex: check.view_tag_hex,
        })
    }

    pub(crate) async fn eth_stealth_sign(
        &self,
        token: Option<&str>,
        body: EthStealthSignRequest,
    ) -> ServiceResult<EthStealthSignResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        self.authorize_transaction_policy(TransactionPolicyCheck {
            kind: TransactionPolicyKind::RawDigest,
            destination_address: None,
            asset_kind: "raw_digest",
            amount_hex: "0x0",
        })?;
        let wallet = body.wallet;
        let view_tag = decode_optional_view_tag(body.stealth.view_tag_hex.as_deref())?;
        let stealth_address = body.stealth.stealth_address;
        let ephemeral_public_key_hex = body.stealth.ephemeral_public_key_hex;
        let digest = Zeroizing::new(decode_fixed_hex::<32>(&body.digest_hex, "digest")?);
        let _guard = self.acquire_session_operation(&session_context).await?;

        let (signature, compartment_id) =
            self.with_active_vault(token, |vault, compartment_id| {
                let master_key = vault
                    .extract_master_key()
                    .ok_or_else(|| ServiceError::forbidden("Vault is locked."))?;
                let derived =
                    derive_sigillum_ethereum_stealth_wallet(master_key.as_ref(), &wallet, "eth")
                        .map_err(map_wallet_error)?;
                let signature = sign_ethereum_stealth_digest(
                    &derived,
                    &stealth_address,
                    &ephemeral_public_key_hex,
                    view_tag,
                    digest.as_ref(),
                )
                .map_err(map_wallet_error)?;
                Ok((signature, compartment_id))
            })?;

        self.record_audit(
            Some(compartment_id),
            AuditEventSpec::WalletEthStealthSign {
                wallet: wallet.clone(),
                stealth_address: signature.stealth_address.clone(),
            },
        )?;

        Ok(EthStealthSignResponse {
            wallet,
            stealth_address: signature.stealth_address,
            signature_hex: signature.signature_hex,
            recovery_id: signature.recovery_id,
            view_tag_hex: signature.view_tag_hex,
        })
    }

    pub(crate) async fn eth_stealth_sign_transfer(
        &self,
        token: Option<&str>,
        body: EthStealthSignTransferRequest,
    ) -> ServiceResult<EthSignedTransactionResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        self.authorize_transaction_policy(TransactionPolicyCheck {
            kind: TransactionPolicyKind::RoutedTransfer,
            destination_address: Some(&body.destination_address),
            asset_kind: "native",
            amount_hex: &body.value_wei_hex,
        })?;
        let wallet = body.wallet;
        let view_tag = decode_optional_view_tag(body.stealth.view_tag_hex.as_deref())?;
        let max_priority_fee_per_gas = decode_quantity_hex(&body.fees.max_priority_fee_per_gas_hex)
            .map_err(map_wallet_error)?;
        let max_fee_per_gas =
            decode_quantity_hex(&body.fees.max_fee_per_gas_hex).map_err(map_wallet_error)?;
        let value = decode_quantity_hex(&body.value_wei_hex).map_err(map_wallet_error)?;
        let _guard = self.acquire_session_operation(&session_context).await?;

        let (signed, compartment_id) = self.with_active_vault(token, |vault, compartment_id| {
            let master_key = vault
                .extract_master_key()
                .ok_or_else(|| ServiceError::forbidden("Vault is locked."))?;
            let derived =
                derive_sigillum_ethereum_stealth_wallet(master_key.as_ref(), &wallet, "eth")
                    .map_err(map_wallet_error)?;
            let signed = sign_ethereum_stealth_native_transfer(
                &derived,
                &body.stealth.stealth_address,
                &body.stealth.ephemeral_public_key_hex,
                view_tag,
                &EthereumEip1559Transfer {
                    chain_id: body.fees.chain_id,
                    nonce: body.nonce,
                    max_priority_fee_per_gas,
                    max_fee_per_gas,
                    gas_limit: body.gas_limit,
                    destination_address: body.destination_address.clone(),
                    value,
                },
            )
            .map_err(map_wallet_error)?;
            Ok((signed, compartment_id))
        })?;

        self.record_audit(
            Some(compartment_id),
            AuditEventSpec::WalletEthStealthSignTransfer {
                wallet: wallet.clone(),
                transaction_kind: signed.kind.clone(),
                to: signed.to_address.clone(),
                nonce: signed.nonce,
            },
        )?;

        Ok(EthSignedTransactionResponse {
            wallet,
            kind: signed.kind,
            chain_id: signed.chain_id,
            nonce: signed.nonce,
            from_address: signed.from_address,
            to_address: signed.to_address,
            value_hex: signed.value_hex,
            data_hex: signed.data_hex,
            raw_transaction_hex: signed.raw_transaction_hex,
            transaction_hash_hex: signed.transaction_hash_hex,
        })
    }

    pub(crate) async fn eth_stealth_sign_erc20_transfer(
        &self,
        token: Option<&str>,
        body: EthStealthSignErc20TransferRequest,
    ) -> ServiceResult<EthSignedTransactionResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        self.authorize_transaction_policy(TransactionPolicyCheck {
            kind: TransactionPolicyKind::RoutedTransfer,
            destination_address: Some(&body.recipient_address),
            asset_kind: "erc20",
            amount_hex: &body.amount_hex,
        })?;
        let wallet = body.wallet;
        let view_tag = decode_optional_view_tag(body.stealth.view_tag_hex.as_deref())?;
        let max_priority_fee_per_gas = decode_quantity_hex(&body.fees.max_priority_fee_per_gas_hex)
            .map_err(map_wallet_error)?;
        let max_fee_per_gas =
            decode_quantity_hex(&body.fees.max_fee_per_gas_hex).map_err(map_wallet_error)?;
        let amount = decode_quantity_hex(&body.amount_hex).map_err(map_wallet_error)?;
        let _guard = self.acquire_session_operation(&session_context).await?;

        let (signed, compartment_id) = self.with_active_vault(token, |vault, compartment_id| {
            let master_key = vault
                .extract_master_key()
                .ok_or_else(|| ServiceError::forbidden("Vault is locked."))?;
            let derived =
                derive_sigillum_ethereum_stealth_wallet(master_key.as_ref(), &wallet, "eth")
                    .map_err(map_wallet_error)?;
            let signed = sign_ethereum_stealth_erc20_transfer(
                &derived,
                &body.stealth.stealth_address,
                &body.stealth.ephemeral_public_key_hex,
                view_tag,
                &EthereumEip1559Erc20Transfer {
                    chain_id: body.fees.chain_id,
                    nonce: body.nonce,
                    max_priority_fee_per_gas,
                    max_fee_per_gas,
                    gas_limit: body.gas_limit,
                    token_address: body.token_address.clone(),
                    recipient_address: body.recipient_address.clone(),
                    amount,
                },
            )
            .map_err(map_wallet_error)?;
            Ok((signed, compartment_id))
        })?;

        self.record_audit(
            Some(compartment_id),
            AuditEventSpec::WalletEthStealthSignErc20Transfer {
                wallet: wallet.clone(),
                transaction_kind: signed.kind.clone(),
                to: signed.to_address.clone(),
                nonce: signed.nonce,
            },
        )?;

        Ok(EthSignedTransactionResponse {
            wallet,
            kind: signed.kind,
            chain_id: signed.chain_id,
            nonce: signed.nonce,
            from_address: signed.from_address,
            to_address: signed.to_address,
            value_hex: signed.value_hex,
            data_hex: signed.data_hex,
            raw_transaction_hex: signed.raw_transaction_hex,
            transaction_hash_hex: signed.transaction_hash_hex,
        })
    }
}

fn eth_account_path(project_account: u32) -> String {
    format!("m/44'/60'/{project_account}'")
}

fn eth_receive_path(project_account: u32) -> String {
    format!("{}/0", eth_account_path(project_account))
}

fn parent_derivation_path(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sigillum_api::{
        EthStealthCheckRequest, EthStealthExportRequest, EthStealthGenerateRequest,
        EthStealthSignErc20TransferRequest, EthStealthSignRequest, EthStealthSignTransferRequest,
        EthXpubExportRequest, EthXpubWalletProfile, EvmProviderProfile, StealthPaymentRef,
        TreasuryAllowedDestination, TreasuryPolicy,
    };
    use sigillum_core::{
        derive_ethereum_account_xpub_from_mnemonic,
        derive_ethereum_xpub_control_branch_from_mnemonic,
        derive_ethereum_xpub_receive_branch_from_mnemonic,
    };
    use sigillum_fido2::config::CompartmentMeta;
    use tempfile::TempDir;

    use super::*;
    use crate::AppState;
    use crate::inventory::{WalletInventoryState, save_wallet_inventory};
    use crate::profiles::{ProfileRegistry, save_profiles};

    const TEST_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    fn meta(id: usize, threshold: usize, label: &str) -> CompartmentMeta {
        CompartmentMeta {
            id,
            label: label.into(),
            threshold,
            passphrase_mode: None,
        }
    }

    fn payment_ref(payment: &EthStealthGenerateResponse) -> StealthPaymentRef {
        StealthPaymentRef {
            stealth_address: payment.stealth_address.clone(),
            ephemeral_public_key_hex: payment.ephemeral_public_key_hex.clone(),
            view_tag_hex: Some(payment.view_tag_hex.clone()),
        }
    }

    fn digest_sign_request(payment: &EthStealthGenerateResponse) -> EthStealthSignRequest {
        EthStealthSignRequest {
            wallet: "payments".into(),
            stealth: payment_ref(payment),
            digest_hex: hex::encode([9u8; 32]),
        }
    }

    #[tokio::test]
    async fn export_generate_check_and_sign_roundtrip() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [7u8; 32], meta(0, 1, "default"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state);

        let meta = service
            .eth_stealth_export(
                Some(&session),
                EthStealthExportRequest {
                    wallet: "payments".into(),
                    short_name: Some("eth".into()),
                },
            )
            .unwrap();

        let payment = service
            .eth_stealth_generate(EthStealthGenerateRequest {
                stealth_meta_address: meta.stealth_meta_address.clone(),
                ephemeral_private_key_hex: Some(hex::encode([3u8; 32])),
            })
            .unwrap();

        let check = service
            .eth_stealth_check(
                Some(&session),
                EthStealthCheckRequest {
                    wallet: "payments".into(),
                    stealth: sigillum_api::StealthPaymentRef {
                        stealth_address: payment.stealth_address.clone(),
                        ephemeral_public_key_hex: payment.ephemeral_public_key_hex.clone(),
                        view_tag_hex: Some(payment.view_tag_hex.clone()),
                    },
                },
            )
            .unwrap();
        assert!(check.matches);

        let signature = service
            .eth_stealth_sign(
                Some(&session),
                EthStealthSignRequest {
                    wallet: "payments".into(),
                    stealth: sigillum_api::StealthPaymentRef {
                        stealth_address: payment.stealth_address.clone(),
                        ephemeral_public_key_hex: payment.ephemeral_public_key_hex.clone(),
                        view_tag_hex: Some(payment.view_tag_hex.clone()),
                    },
                    digest_hex: hex::encode([9u8; 32]),
                },
            )
            .await
            .unwrap();
        assert_eq!(signature.stealth_address, payment.stealth_address);
        assert_eq!(hex::decode(signature.signature_hex).unwrap().len(), 65);

        let signed_transfer = service
            .eth_stealth_sign_transfer(
                Some(&session),
                EthStealthSignTransferRequest {
                    wallet: "payments".into(),
                    stealth: sigillum_api::StealthPaymentRef {
                        stealth_address: payment.stealth_address.clone(),
                        ephemeral_public_key_hex: payment.ephemeral_public_key_hex.clone(),
                        view_tag_hex: Some(payment.view_tag_hex.clone()),
                    },
                    fees: sigillum_api::Eip1559Fees {
                        chain_id: 1,
                        max_priority_fee_per_gas_hex: "0x59682f00".into(),
                        max_fee_per_gas_hex: "0x77359400".into(),
                    },
                    nonce: 9,
                    gas_limit: 21_000,
                    destination_address: "0x1111111111111111111111111111111111111111".into(),
                    value_wei_hex: "0xde0b6b3a7640000".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(signed_transfer.kind, "eth-transfer");
        assert!(signed_transfer.raw_transaction_hex.starts_with("02"));

        let signed_erc20 = service
            .eth_stealth_sign_erc20_transfer(
                Some(&session),
                EthStealthSignErc20TransferRequest {
                    wallet: "payments".into(),
                    stealth: sigillum_api::StealthPaymentRef {
                        stealth_address: payment.stealth_address,
                        ephemeral_public_key_hex: payment.ephemeral_public_key_hex.clone(),
                        view_tag_hex: Some(payment.view_tag_hex.clone()),
                    },
                    fees: sigillum_api::Eip1559Fees {
                        chain_id: 1,
                        max_priority_fee_per_gas_hex: "0x59682f00".into(),
                        max_fee_per_gas_hex: "0x77359400".into(),
                    },
                    nonce: 10,
                    gas_limit: 65_000,
                    token_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
                    recipient_address: "0x2222222222222222222222222222222222222222".into(),
                    amount_hex: "0x0f4240".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(signed_erc20.kind, "erc20-transfer");
        assert!(signed_erc20.data_hex.starts_with("a9059cbb"));
    }

    #[tokio::test]
    async fn queued_signature_rejects_token_rotated_by_compartment_switch() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [7u8; 32], meta(0, 1, "daily"));
        state.unlock_compartment(1, [8u8; 32], meta(1, 2, "secure"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state.clone());
        let exported = service
            .eth_stealth_export(
                Some(&session),
                EthStealthExportRequest {
                    wallet: "payments".into(),
                    short_name: Some("eth".into()),
                },
            )
            .unwrap();
        let payment = service
            .eth_stealth_generate(EthStealthGenerateRequest {
                stealth_meta_address: exported.stealth_meta_address,
                ephemeral_private_key_hex: Some(hex::encode([3u8; 32])),
            })
            .unwrap();

        let held_operation = state.operation_guard().await;
        let mut switch = Box::pin(service.switch_compartment(
            Some(&session),
            sigillum_api::CompartmentSwitchRequest { id: 1 },
        ));
        tokio::select! {
            biased;
            result = &mut switch => panic!("switch must wait for the operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }

        let mut sign =
            Box::pin(service.eth_stealth_sign(Some(&session), digest_sign_request(&payment)));
        tokio::select! {
            biased;
            result = &mut sign => panic!("signature must wait for the operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }

        drop(held_operation);
        let switched = switch.await.unwrap();
        let sign_error = sign
            .await
            .expect_err("old-token signature must fail after token rotation");
        assert_eq!(sign_error.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert!(!state.verify_token(&session));
        assert_eq!(
            state.active_compartment_id_for(&switched.session_token),
            Some(1)
        );
    }

    #[tokio::test]
    async fn lock_latch_preempts_signature_waiting_for_operation_mutex() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [7u8; 32], meta(0, 1, "default"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state.clone());
        let exported = service
            .eth_stealth_export(
                Some(&session),
                EthStealthExportRequest {
                    wallet: "payments".into(),
                    short_name: Some("eth".into()),
                },
            )
            .unwrap();
        let payment = service
            .eth_stealth_generate(EthStealthGenerateRequest {
                stealth_meta_address: exported.stealth_meta_address,
                ephemeral_private_key_hex: Some(hex::encode([3u8; 32])),
            })
            .unwrap();

        let held_operation = state.operation_guard().await;
        let mut sign =
            Box::pin(service.eth_stealth_sign(Some(&session), digest_sign_request(&payment)));
        tokio::select! {
            biased;
            result = &mut sign => panic!("signature must wait for the operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }

        let mut lock = Box::pin(service.lock_all(Some(&session)));
        tokio::select! {
            biased;
            result = &mut lock => panic!("lock must wait for the operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }
        assert!(state.is_locking());

        drop(held_operation);
        let sign_error = sign
            .await
            .expect_err("latched lock must preempt a queued signature");
        assert_eq!(sign_error.status(), axum::http::StatusCode::LOCKED);
        assert_eq!(lock.await.unwrap().status, "locked");
        assert!(!state.is_unlocked());
    }

    #[tokio::test]
    async fn enabled_policy_rejects_raw_digest_signing_by_default() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [7u8; 32], meta(0, 1, "default"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state);

        let meta = service
            .eth_stealth_export(
                Some(&session),
                EthStealthExportRequest {
                    wallet: "payments".into(),
                    short_name: Some("eth".into()),
                },
            )
            .unwrap();
        let payment = service
            .eth_stealth_generate(EthStealthGenerateRequest {
                stealth_meta_address: meta.stealth_meta_address,
                ephemeral_private_key_hex: Some(hex::encode([3u8; 32])),
            })
            .unwrap();

        save_wallet_inventory(
            dir.path(),
            &WalletInventoryState {
                treasury_policy: Some(TreasuryPolicy {
                    enabled: true,
                    allowed_destinations: Vec::<TreasuryAllowedDestination>::new(),
                    max_step_native_wei_hex: None,
                    max_plan_native_wei_hex: None,
                    require_simulation: true,
                    allow_raw_digest_signing: false,
                    block_cross_party_linkage: false,
                    allow_claim_execution: false,
                    allow_gas_topups: false,
                    max_gas_topup_wei_hex: None,
                    allow_plan_execution: false,
                    allow_sweep_execution: false,
                    allow_revoke_execution: false,
                    allow_exit_execution: false,
                    execution_paused: false,
                    max_fee_per_gas_cap_hex: None,
                    simulation_freshness_secs: 900,
                    hot_floor_wei_hex: "0xde0b6b3a7640000".into(),
                    hot_target_wei_hex: "0xde0b6b3a7640000".into(),
                    hot_overflow_wei_hex: None,
                    allow_treasury_automation: false,
                    created_at_unix: 1,
                    updated_at_unix: 1,
                }),
                ..Default::default()
            },
        )
        .unwrap();

        let error = service
            .eth_stealth_sign(
                Some(&session),
                EthStealthSignRequest {
                    wallet: "payments".into(),
                    stealth: payment_ref(&payment),
                    digest_hex: hex::encode([9u8; 32]),
                },
            )
            .await
            .unwrap_err();

        assert_eq!(error.message(), "policy_violation");
        assert_eq!(error.action(), Some("block_raw_digest"));
    }

    #[test]
    fn xpub_export_requires_active_compartment_match() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [7u8; 32], meta(0, 1, "default"));
        state.unlock_compartment(1, [8u8; 32], meta(1, 2, "secure"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state);

        save_profiles(
            dir.path(),
            &ProfileRegistry {
                evm_providers: vec![EvmProviderProfile {
                    name: "mainnet".into(),
                    rpc_url: "http://127.0.0.1:8545".into(),
                    auth_token_key: None,
                    compartment_id: 0,
                    chain_id: 1,
                    max_priority_fee_per_gas_hex: None,
                    max_fee_per_gas_hex: None,
                    native_gas_limit: None,
                    erc20_gas_limit: None,
                    fee_estimation_enabled: false,
                }],
                eth_stealth_wallets: vec![],
                eth_xpub_wallets: vec![EthXpubWalletProfile {
                    name: "treasury".into(),
                    project_account: 7,
                    provider_profile: "mainnet".into(),
                    compartment_id: 1,
                    chain_id: Some(1),
                    external_receive_xpub: None,
                    external_receive_path: None,
                    external_account_xpub: None,
                    external_account_path: None,
                    default_destination_address: None,
                    execution_enabled: false,
                }],
                ..Default::default()
            },
        )
        .unwrap();

        let error = service
            .eth_xpub_export(
                Some(&session),
                EthXpubExportRequest {
                    wallet_profile: "treasury".into(),
                },
            )
            .unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::FORBIDDEN);
        assert_eq!(
            error.message(),
            "Wallet profile is not in the active compartment."
        );
    }

    #[test]
    fn xpub_export_uses_active_compartment_wallet_profile() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [7u8; 32], meta(0, 1, "default"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state);

        save_profiles(
            dir.path(),
            &ProfileRegistry {
                evm_providers: vec![EvmProviderProfile {
                    name: "mainnet".into(),
                    rpc_url: "http://127.0.0.1:8545".into(),
                    auth_token_key: None,
                    compartment_id: 0,
                    chain_id: 1,
                    max_priority_fee_per_gas_hex: None,
                    max_fee_per_gas_hex: None,
                    native_gas_limit: None,
                    erc20_gas_limit: None,
                    fee_estimation_enabled: false,
                }],
                eth_stealth_wallets: vec![],
                eth_xpub_wallets: vec![EthXpubWalletProfile {
                    name: "treasury".into(),
                    project_account: 7,
                    provider_profile: "mainnet".into(),
                    compartment_id: 0,
                    chain_id: Some(1),
                    external_receive_xpub: None,
                    external_receive_path: None,
                    external_account_xpub: None,
                    external_account_path: None,
                    default_destination_address: None,
                    execution_enabled: false,
                }],
                ..Default::default()
            },
        )
        .unwrap();

        let export = service
            .eth_xpub_export(
                Some(&session),
                EthXpubExportRequest {
                    wallet_profile: "treasury".into(),
                },
            )
            .unwrap();
        assert_eq!(export.wallet_profile, "treasury");
        assert_eq!(export.project_account, 7);
        assert_eq!(export.account_path, "m/44'/60'/7'");
        assert_eq!(export.receive_path, "m/44'/60'/7'/0");
        assert!(export.receive_xpub.starts_with("xpub"));
    }

    #[test]
    fn xpub_export_returns_imported_receive_xpub_without_local_derivation() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [7u8; 32], meta(0, 1, "default"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state);
        let imported =
            derive_ethereum_xpub_receive_branch_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();

        save_profiles(
            dir.path(),
            &ProfileRegistry {
                evm_providers: vec![EvmProviderProfile {
                    name: "mainnet".into(),
                    rpc_url: "http://127.0.0.1:8545".into(),
                    auth_token_key: None,
                    compartment_id: 0,
                    chain_id: 1,
                    max_priority_fee_per_gas_hex: None,
                    max_fee_per_gas_hex: None,
                    native_gas_limit: None,
                    erc20_gas_limit: None,
                    fee_estimation_enabled: false,
                }],
                eth_stealth_wallets: vec![],
                eth_xpub_wallets: vec![EthXpubWalletProfile {
                    name: "external-ledger".into(),
                    project_account: 0,
                    provider_profile: "mainnet".into(),
                    compartment_id: 0,
                    chain_id: Some(1),
                    external_receive_xpub: Some(imported.receive_xpub.clone()),
                    external_receive_path: None,
                    external_account_xpub: None,
                    external_account_path: None,
                    default_destination_address: None,
                    execution_enabled: false,
                }],
                ..Default::default()
            },
        )
        .unwrap();

        let export = service
            .eth_xpub_export(
                Some(&session),
                EthXpubExportRequest {
                    wallet_profile: "external-ledger".into(),
                },
            )
            .unwrap();

        assert_eq!(export.wallet_profile, "external-ledger");
        assert_eq!(export.account_path, imported.account_path);
        assert_eq!(export.receive_path, imported.receive_path);
        assert_eq!(export.receive_xpub, imported.receive_xpub);
    }

    #[test]
    fn xpub_export_normalizes_imported_account_xpub() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [7u8; 32], meta(0, 1, "default"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state);
        let account_xpub =
            derive_ethereum_account_xpub_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();
        let expected =
            derive_ethereum_xpub_receive_branch_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();

        save_profiles(
            dir.path(),
            &ProfileRegistry {
                evm_providers: vec![EvmProviderProfile {
                    name: "mainnet".into(),
                    rpc_url: "http://127.0.0.1:8545".into(),
                    auth_token_key: None,
                    compartment_id: 0,
                    chain_id: 1,
                    max_priority_fee_per_gas_hex: None,
                    max_fee_per_gas_hex: None,
                    native_gas_limit: None,
                    erc20_gas_limit: None,
                    fee_estimation_enabled: false,
                }],
                eth_stealth_wallets: vec![],
                eth_xpub_wallets: vec![EthXpubWalletProfile {
                    name: "external-ledger".into(),
                    project_account: 0,
                    provider_profile: "mainnet".into(),
                    compartment_id: 0,
                    chain_id: Some(1),
                    external_receive_xpub: None,
                    external_receive_path: None,
                    external_account_xpub: Some(account_xpub),
                    external_account_path: None,
                    default_destination_address: None,
                    execution_enabled: false,
                }],
                ..Default::default()
            },
        )
        .unwrap();

        let export = service
            .eth_xpub_export(
                Some(&session),
                EthXpubExportRequest {
                    wallet_profile: "external-ledger".into(),
                },
            )
            .unwrap();

        assert_eq!(export.account_path, expected.account_path);
        assert_eq!(export.receive_path, expected.receive_path);
        assert_eq!(export.receive_xpub, expected.receive_xpub);
    }

    #[test]
    fn xpub_export_uses_imported_account_xpub_custom_path() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [7u8; 32], meta(0, 1, "default"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state);
        let account_xpub =
            derive_ethereum_account_xpub_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();
        let expected = derive_ethereum_receive_branch_from_account_xpub_with_path(
            &account_xpub,
            "m/44'/60'/99'",
            99,
        )
        .unwrap();

        save_profiles(
            dir.path(),
            &ProfileRegistry {
                evm_providers: vec![EvmProviderProfile {
                    name: "mainnet".into(),
                    rpc_url: "http://127.0.0.1:8545".into(),
                    auth_token_key: None,
                    compartment_id: 0,
                    chain_id: 1,
                    max_priority_fee_per_gas_hex: None,
                    max_fee_per_gas_hex: None,
                    native_gas_limit: None,
                    erc20_gas_limit: None,
                    fee_estimation_enabled: false,
                }],
                eth_stealth_wallets: vec![],
                eth_xpub_wallets: vec![EthXpubWalletProfile {
                    name: "external-ledger".into(),
                    project_account: 99,
                    provider_profile: "mainnet".into(),
                    compartment_id: 0,
                    chain_id: Some(1),
                    external_receive_xpub: None,
                    external_receive_path: None,
                    external_account_xpub: Some(account_xpub),
                    external_account_path: Some("m/44'/60'/99'".into()),
                    default_destination_address: None,
                    execution_enabled: false,
                }],
                ..Default::default()
            },
        )
        .unwrap();

        let export = service
            .eth_xpub_export(
                Some(&session),
                EthXpubExportRequest {
                    wallet_profile: "external-ledger".into(),
                },
            )
            .unwrap();

        assert_eq!(export.project_account, 99);
        assert_eq!(export.account_path, "m/44'/60'/99'");
        assert_eq!(export.receive_path, "m/44'/60'/99'/0");
        assert_eq!(export.receive_xpub, expected.receive_xpub);
    }

    #[test]
    fn xpub_export_returns_imported_custom_path() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [7u8; 32], meta(0, 1, "default"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state);
        let imported =
            derive_ethereum_xpub_control_branch_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();

        save_profiles(
            dir.path(),
            &ProfileRegistry {
                evm_providers: vec![EvmProviderProfile {
                    name: "mainnet".into(),
                    rpc_url: "http://127.0.0.1:8545".into(),
                    auth_token_key: None,
                    compartment_id: 0,
                    chain_id: 1,
                    max_priority_fee_per_gas_hex: None,
                    max_fee_per_gas_hex: None,
                    native_gas_limit: None,
                    erc20_gas_limit: None,
                    fee_estimation_enabled: false,
                }],
                eth_stealth_wallets: vec![],
                eth_xpub_wallets: vec![EthXpubWalletProfile {
                    name: "external-control".into(),
                    project_account: 99,
                    provider_profile: "mainnet".into(),
                    compartment_id: 0,
                    chain_id: Some(1),
                    external_receive_xpub: Some(imported.receive_xpub.clone()),
                    external_receive_path: Some(imported.receive_path.clone()),
                    external_account_xpub: None,
                    external_account_path: None,
                    default_destination_address: None,
                    execution_enabled: false,
                }],
                ..Default::default()
            },
        )
        .unwrap();

        let export = service
            .eth_xpub_export(
                Some(&session),
                EthXpubExportRequest {
                    wallet_profile: "external-control".into(),
                },
            )
            .unwrap();

        assert_eq!(export.project_account, 99);
        assert_eq!(export.account_path, imported.account_path);
        assert_eq!(export.receive_path, imported.receive_path);
        assert_eq!(export.receive_xpub, imported.receive_xpub);
    }
}
