//! Ethereum stealth wallet operations.
//!
//! Provides wallet export, address generation, checking, and transaction signing
//! for Ethereum stealth addresses using master key derivation.

use sigillum_api::{
    EthSignedTransactionResponse, EthStealthCheckRequest, EthStealthCheckResponse,
    EthStealthExportRequest, EthStealthGenerateRequest, EthStealthGenerateResponse,
    EthStealthMetaAddressResponse, EthStealthSignErc20TransferRequest, EthStealthSignRequest,
    EthStealthSignResponse, EthStealthSignTransferRequest,
};
use sigillum_core::{
    EthereumEip1559Erc20Transfer, EthereumEip1559Transfer, VaultLifecycle,
    check_ethereum_stealth_address, decode_quantity_hex, derive_sigillum_ethereum_stealth_wallet,
    generate_ethereum_stealth_address, sign_ethereum_stealth_digest,
    sign_ethereum_stealth_erc20_transfer, sign_ethereum_stealth_native_transfer,
};
use zeroize::Zeroizing;

use crate::audit_log::AuditEventSpec;

use super::helpers::{decode_fixed_hex, decode_optional_view_tag, map_wallet_error};
use super::{ServiceError, ServiceResult, SigillumService};

impl SigillumService {
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

        Ok(EthStealthGenerateResponse {
            short_name: payment.short_name,
            scheme_id: payment.scheme_id,
            stealth_meta_address: payment.stealth_meta_address,
            stealth_address: payment.stealth_address,
            ephemeral_public_key_hex: payment.ephemeral_public_key_hex,
            view_tag_hex: payment.view_tag_hex,
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

    pub(crate) fn eth_stealth_sign(
        &self,
        token: Option<&str>,
        body: EthStealthSignRequest,
    ) -> ServiceResult<EthStealthSignResponse> {
        let token = self.require_session(token)?;
        let wallet = body.wallet;
        let view_tag = decode_optional_view_tag(body.stealth.view_tag_hex.as_deref())?;
        let stealth_address = body.stealth.stealth_address;
        let ephemeral_public_key_hex = body.stealth.ephemeral_public_key_hex;
        let digest = Zeroizing::new(decode_fixed_hex::<32>(&body.digest_hex, "digest")?);

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

    pub(crate) fn eth_stealth_sign_transfer(
        &self,
        token: Option<&str>,
        body: EthStealthSignTransferRequest,
    ) -> ServiceResult<EthSignedTransactionResponse> {
        let token = self.require_session(token)?;
        let wallet = body.wallet;
        let view_tag = decode_optional_view_tag(body.stealth.view_tag_hex.as_deref())?;
        let max_priority_fee_per_gas = decode_quantity_hex(&body.fees.max_priority_fee_per_gas_hex)
            .map_err(map_wallet_error)?;
        let max_fee_per_gas =
            decode_quantity_hex(&body.fees.max_fee_per_gas_hex).map_err(map_wallet_error)?;
        let value = decode_quantity_hex(&body.value_wei_hex).map_err(map_wallet_error)?;

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

    pub(crate) fn eth_stealth_sign_erc20_transfer(
        &self,
        token: Option<&str>,
        body: EthStealthSignErc20TransferRequest,
    ) -> ServiceResult<EthSignedTransactionResponse> {
        let token = self.require_session(token)?;
        let wallet = body.wallet;
        let view_tag = decode_optional_view_tag(body.stealth.view_tag_hex.as_deref())?;
        let max_priority_fee_per_gas = decode_quantity_hex(&body.fees.max_priority_fee_per_gas_hex)
            .map_err(map_wallet_error)?;
        let max_fee_per_gas =
            decode_quantity_hex(&body.fees.max_fee_per_gas_hex).map_err(map_wallet_error)?;
        let amount = decode_quantity_hex(&body.amount_hex).map_err(map_wallet_error)?;

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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sigillum_api::{
        EthStealthCheckRequest, EthStealthExportRequest, EthStealthGenerateRequest,
        EthStealthSignErc20TransferRequest, EthStealthSignRequest, EthStealthSignTransferRequest,
    };
    use sigillum_fido2::config::CompartmentMeta;
    use tempfile::TempDir;

    use super::*;
    use crate::AppState;

    fn meta(id: usize, threshold: usize, label: &str) -> CompartmentMeta {
        CompartmentMeta {
            id,
            label: label.into(),
            threshold,
            passphrase_mode: None,
        }
    }

    #[test]
    fn export_generate_check_and_sign_roundtrip() {
        let dir = TempDir::new().unwrap();
        let state = Arc::new(AppState::new(dir.path().to_path_buf()));
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
            .unwrap();
        assert_eq!(signed_erc20.kind, "erc20-transfer");
        assert!(signed_erc20.data_hex.starts_with("a9059cbb"));
    }
}
