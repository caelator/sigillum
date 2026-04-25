//! Ethereum BIP32/xpub receive-wallet helpers.
//!
//! This module derives a deterministic HD wallet tree from a Sigillum
//! compartment master key and exposes the public receive branch material for
//! project-wallet style deposit generation.

use bip32::{ChildNumber, Prefix, XPrv, XPub};
use bip39::{Language, Mnemonic};
use sha3::{Digest, Keccak256};
use thiserror::Error;

pub const ETHEREUM_XPUB_PURPOSE: u32 = 44;
pub const ETHEREUM_XPUB_COIN_TYPE: u32 = 60;
pub const ETHEREUM_XPUB_RECEIVE_BRANCH: u32 = 0;
pub const ETHEREUM_XPUB_CONTROL_BRANCH: u32 = 1;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EthereumXpubError {
    #[error("invalid HD wallet key material")]
    InvalidKeyMaterial,
    #[error("invalid project account")]
    InvalidProjectAccount,
    #[error("invalid receive index")]
    InvalidReceiveIndex,
    #[error("invalid receive-branch xpub")]
    InvalidReceiveBranchXpub,
    #[error("invalid BIP-39 seed phrase")]
    InvalidMnemonic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EthereumXpubReceiveExport {
    pub project_account: u32,
    pub account_path: String,
    pub receive_path: String,
    pub receive_xpub: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EthereumXpubReceiveAddress {
    pub index: u32,
    pub address: String,
}

pub fn derive_sigillum_ethereum_xpub_receive_branch(
    master_key: &[u8],
    project_account: u32,
) -> Result<EthereumXpubReceiveExport, EthereumXpubError> {
    let receive_xprv = derive_receive_branch_xprv(master_key, project_account)?;
    Ok(EthereumXpubReceiveExport {
        project_account,
        account_path: format!(
            "m/{ETHEREUM_XPUB_PURPOSE}'/{ETHEREUM_XPUB_COIN_TYPE}'/{project_account}'"
        ),
        receive_path: format!(
            "m/{ETHEREUM_XPUB_PURPOSE}'/{ETHEREUM_XPUB_COIN_TYPE}'/{project_account}'/{ETHEREUM_XPUB_RECEIVE_BRANCH}"
        ),
        receive_xpub: receive_xprv
            .public_key()
            .to_string(Prefix::XPUB)
            .to_string(),
    })
}

pub fn derive_ethereum_xpub_receive_branch_from_mnemonic(
    mnemonic_phrase: &str,
    mnemonic_passphrase: Option<&str>,
    project_account: u32,
) -> Result<EthereumXpubReceiveExport, EthereumXpubError> {
    let seed = mnemonic_seed(mnemonic_phrase, mnemonic_passphrase)?;
    derive_sigillum_ethereum_xpub_receive_branch(&seed, project_account)
}

pub fn ethereum_mnemonic_word_count(mnemonic_phrase: &str) -> Result<usize, EthereumXpubError> {
    Ok(parse_mnemonic(mnemonic_phrase)?.word_count())
}

pub fn derive_sigillum_ethereum_xpub_receive_address(
    master_key: &[u8],
    project_account: u32,
    index: u32,
) -> Result<EthereumXpubReceiveAddress, EthereumXpubError> {
    let receive_xprv = derive_receive_branch_xprv(master_key, project_account)?;
    derive_receive_address_from_xpub(&receive_xprv.public_key(), index)
}

pub fn derive_ethereum_address_from_xpub(
    receive_xpub: &str,
    index: u32,
) -> Result<EthereumXpubReceiveAddress, EthereumXpubError> {
    let receive_xpub = receive_xpub
        .parse::<XPub>()
        .map_err(|_| EthereumXpubError::InvalidReceiveBranchXpub)?;
    validate_receive_branch_xpub(&receive_xpub)?;
    derive_receive_address_from_xpub(&receive_xpub, index)
}

fn mnemonic_seed(
    mnemonic_phrase: &str,
    mnemonic_passphrase: Option<&str>,
) -> Result<[u8; 64], EthereumXpubError> {
    let mnemonic = parse_mnemonic(mnemonic_phrase)?;
    Ok(mnemonic.to_seed(mnemonic_passphrase.unwrap_or_default()))
}

fn parse_mnemonic(mnemonic_phrase: &str) -> Result<Mnemonic, EthereumXpubError> {
    Mnemonic::parse_in_normalized(Language::English, mnemonic_phrase)
        .map_err(|_| EthereumXpubError::InvalidMnemonic)
}

fn derive_receive_branch_xprv(
    master_key: &[u8],
    project_account: u32,
) -> Result<XPrv, EthereumXpubError> {
    let account_xprv = derive_account_xprv(master_key, project_account)?;
    account_xprv
        .derive_child(receive_branch_child()?)
        .map_err(|_| EthereumXpubError::InvalidKeyMaterial)
}

fn derive_account_xprv(master_key: &[u8], project_account: u32) -> Result<XPrv, EthereumXpubError> {
    let root_xprv = XPrv::new(master_key).map_err(|_| EthereumXpubError::InvalidKeyMaterial)?;
    let purpose = hardened_child(ETHEREUM_XPUB_PURPOSE)?;
    let coin_type = hardened_child(ETHEREUM_XPUB_COIN_TYPE)?;
    let account = hardened_child(project_account)?;

    root_xprv
        .derive_child(purpose)
        .and_then(|xprv| xprv.derive_child(coin_type))
        .and_then(|xprv| xprv.derive_child(account))
        .map_err(|_| EthereumXpubError::InvalidKeyMaterial)
}

fn derive_receive_address_from_xpub(
    receive_xpub: &XPub,
    index: u32,
) -> Result<EthereumXpubReceiveAddress, EthereumXpubError> {
    let child =
        ChildNumber::new(index, false).map_err(|_| EthereumXpubError::InvalidReceiveIndex)?;
    let child_xpub = receive_xpub
        .derive_child(child)
        .map_err(|_| EthereumXpubError::InvalidReceiveIndex)?;
    Ok(EthereumXpubReceiveAddress {
        index,
        address: ethereum_address_from_verifying_key(child_xpub.public_key()),
    })
}

fn validate_receive_branch_xpub(receive_xpub: &XPub) -> Result<(), EthereumXpubError> {
    let attrs = receive_xpub.attrs();
    if attrs.depth != 4
        || attrs.child_number.is_hardened()
        || attrs.child_number.index() != ETHEREUM_XPUB_RECEIVE_BRANCH
    {
        return Err(EthereumXpubError::InvalidReceiveBranchXpub);
    }
    Ok(())
}

fn hardened_child(index: u32) -> Result<ChildNumber, EthereumXpubError> {
    ChildNumber::new(index, true).map_err(|_| EthereumXpubError::InvalidProjectAccount)
}

fn receive_branch_child() -> Result<ChildNumber, EthereumXpubError> {
    ChildNumber::new(ETHEREUM_XPUB_RECEIVE_BRANCH, false)
        .map_err(|_| EthereumXpubError::InvalidKeyMaterial)
}

fn ethereum_address_from_verifying_key(
    public_key: &bip32::secp256k1::ecdsa::VerifyingKey,
) -> String {
    let encoded = public_key.to_encoded_point(false);
    let bytes = encoded.as_bytes();
    let digest = Keccak256::digest(&bytes[1..]);
    format!("0x{}", hex::encode(&digest[12..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_receive_branch_returns_xpub() {
        let export = derive_sigillum_ethereum_xpub_receive_branch(&[7u8; 32], 12).expect("export");
        assert_eq!(export.project_account, 12);
        assert_eq!(export.account_path, "m/44'/60'/12'");
        assert_eq!(export.receive_path, "m/44'/60'/12'/0");
        assert!(export.receive_xpub.starts_with("xpub"));
    }

    #[test]
    fn derived_receive_address_matches_exported_xpub() {
        let export = derive_sigillum_ethereum_xpub_receive_branch(&[9u8; 32], 3).expect("export");
        let from_master =
            derive_sigillum_ethereum_xpub_receive_address(&[9u8; 32], 3, 5).expect("master");
        let from_xpub = derive_ethereum_address_from_xpub(&export.receive_xpub, 5).expect("xpub");
        assert_eq!(from_master, from_xpub);
        assert_eq!(from_master.index, 5);
        assert!(from_master.address.starts_with("0x"));
        assert_eq!(from_master.address.len(), 42);
    }

    #[test]
    fn xpub_requires_receive_branch_depth() {
        let export = derive_sigillum_ethereum_xpub_receive_branch(&[11u8; 32], 1).expect("export");
        let account_xprv = derive_account_xprv(&[11u8; 32], 1).expect("account");
        let account_xpub = account_xprv
            .public_key()
            .to_string(Prefix::XPUB)
            .to_string();

        assert!(derive_ethereum_address_from_xpub(&export.receive_xpub, 0).is_ok());
        assert_eq!(
            derive_ethereum_address_from_xpub(&account_xpub, 0),
            Err(EthereumXpubError::InvalidReceiveBranchXpub)
        );
    }

    #[test]
    fn mnemonic_seed_phrase_exports_receive_branch() {
        let twelve_word = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let export =
            derive_ethereum_xpub_receive_branch_from_mnemonic(twelve_word, None, 0).unwrap();

        assert_eq!(ethereum_mnemonic_word_count(twelve_word).unwrap(), 12);
        assert_eq!(export.account_path, "m/44'/60'/0'");
        assert_eq!(export.receive_path, "m/44'/60'/0'/0");
        assert!(export.receive_xpub.starts_with("xpub"));
    }

    #[test]
    fn twenty_four_word_mnemonic_seed_phrase_is_supported() {
        let twenty_four_word = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";
        let export =
            derive_ethereum_xpub_receive_branch_from_mnemonic(twenty_four_word, None, 1).unwrap();

        assert_eq!(ethereum_mnemonic_word_count(twenty_four_word).unwrap(), 24);
        assert_eq!(export.account_path, "m/44'/60'/1'");
        assert!(export.receive_xpub.starts_with("xpub"));
    }

    #[test]
    fn invalid_mnemonic_seed_phrase_is_rejected() {
        assert_eq!(
            derive_ethereum_xpub_receive_branch_from_mnemonic("abandon abandon", None, 0),
            Err(EthereumXpubError::InvalidMnemonic)
        );
    }
}
