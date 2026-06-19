//! Ethereum BIP32/xpub receive-wallet helpers.
//!
//! This module derives a deterministic HD wallet tree from a Sigillum
//! compartment master key and exposes the public receive branch material for
//! project-wallet style deposit generation.

use bip32::{ChildNumber, Prefix, XPrv, XPub};
use bip39::{Language, Mnemonic};
use sha3::{Digest, Keccak256};
use thiserror::Error;
use zeroize::Zeroizing;

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
    #[error("invalid account-branch xpub")]
    InvalidAccountBranchXpub,
    #[error("invalid BIP-39 seed phrase")]
    InvalidMnemonic,
    #[error("mnemonic word count must be 12 or 24")]
    InvalidMnemonicWordCount,
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

/// Generate a fresh BIP-39 English mnemonic phrase with exactly 12 or 24 words.
///
/// Entropy is sourced from the operating-system CSPRNG (16 bytes for 12 words,
/// 32 bytes for 24 words). The entropy buffer is zeroized after encoding, and
/// the phrase is returned inside [`Zeroizing`] so the caller owns the only
/// long-lived copy and it is wiped from memory on drop.
pub fn generate_ethereum_mnemonic(
    word_count: usize,
) -> Result<Zeroizing<String>, EthereumXpubError> {
    use rand::RngCore;
    use rand::rngs::OsRng;

    let entropy_len = match word_count {
        12 => 16,
        24 => 32,
        _ => return Err(EthereumXpubError::InvalidMnemonicWordCount),
    };
    let mut entropy = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(&mut entropy[..entropy_len]);
    let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy[..entropy_len])
        .map_err(|_| EthereumXpubError::InvalidMnemonic)?;
    Ok(Zeroizing::new(mnemonic.to_string()))
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

pub fn derive_ethereum_receive_branch_from_account_xpub(
    account_xpub: &str,
    project_account: u32,
) -> Result<EthereumXpubReceiveExport, EthereumXpubError> {
    let account_xpub = account_xpub
        .parse::<XPub>()
        .map_err(|_| EthereumXpubError::InvalidAccountBranchXpub)?;
    validate_account_branch_xpub(&account_xpub, project_account)?;
    let receive_xpub = account_xpub
        .derive_child(receive_branch_child()?)
        .map_err(|_| EthereumXpubError::InvalidAccountBranchXpub)?;
    Ok(EthereumXpubReceiveExport {
        project_account,
        account_path: account_path(project_account),
        receive_path: receive_path(project_account),
        receive_xpub: receive_xpub.to_string(Prefix::XPUB).to_string(),
    })
}

pub fn derive_ethereum_address_from_account_xpub(
    account_xpub: &str,
    project_account: u32,
    index: u32,
) -> Result<EthereumXpubReceiveAddress, EthereumXpubError> {
    let export = derive_ethereum_receive_branch_from_account_xpub(account_xpub, project_account)?;
    derive_ethereum_address_from_xpub(&export.receive_xpub, index)
}

pub fn derive_ethereum_account_xpub_from_mnemonic(
    mnemonic_phrase: &str,
    mnemonic_passphrase: Option<&str>,
    project_account: u32,
) -> Result<String, EthereumXpubError> {
    let seed = mnemonic_seed(mnemonic_phrase, mnemonic_passphrase)?;
    let account_xprv = derive_account_xprv(&seed, project_account)?;
    Ok(account_xprv
        .public_key()
        .to_string(Prefix::XPUB)
        .to_string())
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

pub fn derive_sigillum_ethereum_xpub_control_branch(
    master_key: &[u8],
    project_account: u32,
) -> Result<EthereumXpubReceiveExport, EthereumXpubError> {
    let control_xprv = derive_control_branch_xprv(master_key, project_account)?;
    Ok(EthereumXpubReceiveExport {
        project_account,
        account_path: format!(
            "m/{ETHEREUM_XPUB_PURPOSE}'/{ETHEREUM_XPUB_COIN_TYPE}'/{project_account}'"
        ),
        receive_path: format!(
            "m/{ETHEREUM_XPUB_PURPOSE}'/{ETHEREUM_XPUB_COIN_TYPE}'/{project_account}'/{ETHEREUM_XPUB_CONTROL_BRANCH}"
        ),
        receive_xpub: control_xprv
            .public_key()
            .to_string(Prefix::XPUB)
            .to_string(),
    })
}

pub fn derive_ethereum_xpub_control_branch_from_mnemonic(
    mnemonic_phrase: &str,
    mnemonic_passphrase: Option<&str>,
    project_account: u32,
) -> Result<EthereumXpubReceiveExport, EthereumXpubError> {
    let seed = mnemonic_seed(mnemonic_phrase, mnemonic_passphrase)?;
    derive_sigillum_ethereum_xpub_control_branch(&seed, project_account)
}

pub fn derive_ethereum_address_from_control_xpub(
    control_xpub: &str,
    index: u32,
) -> Result<EthereumXpubReceiveAddress, EthereumXpubError> {
    let control_xpub = control_xpub
        .parse::<XPub>()
        .map_err(|_| EthereumXpubError::InvalidReceiveBranchXpub)?;
    validate_control_branch_xpub(&control_xpub)?;
    derive_receive_address_from_xpub(&control_xpub, index)
}

fn validate_control_branch_xpub(control_xpub: &XPub) -> Result<(), EthereumXpubError> {
    let attrs = control_xpub.attrs();
    if attrs.depth != 4
        || attrs.child_number.is_hardened()
        || attrs.child_number.index() != ETHEREUM_XPUB_CONTROL_BRANCH
    {
        return Err(EthereumXpubError::InvalidReceiveBranchXpub);
    }
    Ok(())
}

fn derive_control_branch_xprv(
    master_key: &[u8],
    project_account: u32,
) -> Result<XPrv, EthereumXpubError> {
    let account_xprv = derive_account_xprv(master_key, project_account)?;
    account_xprv
        .derive_child(control_branch_child()?)
        .map_err(|_| EthereumXpubError::InvalidKeyMaterial)
}

fn control_branch_child() -> Result<ChildNumber, EthereumXpubError> {
    ChildNumber::new(ETHEREUM_XPUB_CONTROL_BRANCH, false)
        .map_err(|_| EthereumXpubError::InvalidKeyMaterial)
}

pub fn derive_private_key_at_path(
    seed: &[u8],
    path: &str,
) -> Result<k256::ecdsa::SigningKey, EthereumXpubError> {
    let mut current = XPrv::new(seed).map_err(|_| EthereumXpubError::InvalidKeyMaterial)?;
    let parts: Vec<&str> = path.split('/').collect();
    if parts.is_empty() || parts[0] != "m" {
        return Err(EthereumXpubError::InvalidKeyMaterial);
    }
    for part in &parts[1..] {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let is_hardened = part.ends_with('\'');
        let index_str = if is_hardened {
            &part[..part.len() - 1]
        } else {
            part
        };
        let index: u32 = index_str
            .parse()
            .map_err(|_| EthereumXpubError::InvalidKeyMaterial)?;
        let child = ChildNumber::new(index, is_hardened)
            .map_err(|_| EthereumXpubError::InvalidKeyMaterial)?;
        current = current
            .derive_child(child)
            .map_err(|_| EthereumXpubError::InvalidKeyMaterial)?;
    }
    let key_bytes = current.private_key().to_bytes();
    let signing_key = k256::ecdsa::SigningKey::from_slice(&key_bytes)
        .map_err(|_| EthereumXpubError::InvalidKeyMaterial)?;
    Ok(signing_key)
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

fn validate_account_branch_xpub(
    account_xpub: &XPub,
    project_account: u32,
) -> Result<(), EthereumXpubError> {
    let attrs = account_xpub.attrs();
    if attrs.depth != 3
        || !attrs.child_number.is_hardened()
        || attrs.child_number.index() != project_account
        || attrs.parent_fingerprint == [0u8; 4]
    {
        return Err(EthereumXpubError::InvalidAccountBranchXpub);
    }
    Ok(())
}

fn account_path(project_account: u32) -> String {
    format!("m/{ETHEREUM_XPUB_PURPOSE}'/{ETHEREUM_XPUB_COIN_TYPE}'/{project_account}'")
}

fn receive_path(project_account: u32) -> String {
    format!(
        "{}/{}",
        account_path(project_account),
        ETHEREUM_XPUB_RECEIVE_BRANCH
    )
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
    fn account_xpub_derives_matching_receive_branch() {
        let receive_export =
            derive_sigillum_ethereum_xpub_receive_branch(&[12u8; 32], 4).expect("receive");
        let account_xpub = derive_account_xprv(&[12u8; 32], 4)
            .expect("account")
            .public_key()
            .to_string(Prefix::XPUB)
            .to_string();

        let imported =
            derive_ethereum_receive_branch_from_account_xpub(&account_xpub, 4).expect("import");
        assert_eq!(imported, receive_export);
        assert_eq!(
            derive_ethereum_address_from_account_xpub(&account_xpub, 4, 8).expect("account"),
            derive_ethereum_address_from_xpub(&receive_export.receive_xpub, 8).expect("receive")
        );
    }

    #[test]
    fn account_xpub_rejects_wrong_depth_and_account() {
        let receive_export =
            derive_sigillum_ethereum_xpub_receive_branch(&[13u8; 32], 2).expect("receive");
        let account_xpub = derive_account_xprv(&[13u8; 32], 2)
            .expect("account")
            .public_key()
            .to_string(Prefix::XPUB)
            .to_string();

        assert_eq!(
            derive_ethereum_receive_branch_from_account_xpub(&receive_export.receive_xpub, 2),
            Err(EthereumXpubError::InvalidAccountBranchXpub)
        );
        assert_eq!(
            derive_ethereum_receive_branch_from_account_xpub(&account_xpub, 3),
            Err(EthereumXpubError::InvalidAccountBranchXpub)
        );
    }

    #[test]
    fn mnemonic_exports_account_xpub_for_watch_only_import() {
        let twelve_word = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let account_xpub =
            derive_ethereum_account_xpub_from_mnemonic(twelve_word, None, 1).unwrap();
        let receive_from_account =
            derive_ethereum_receive_branch_from_account_xpub(&account_xpub, 1).unwrap();
        let receive_direct =
            derive_ethereum_xpub_receive_branch_from_mnemonic(twelve_word, None, 1).unwrap();

        assert_eq!(receive_from_account, receive_direct);
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
    fn mnemonic_receive_branch_exports_are_account_scoped() {
        let twelve_word = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let account_0 =
            derive_ethereum_xpub_receive_branch_from_mnemonic(twelve_word, None, 0).unwrap();
        let account_1 =
            derive_ethereum_xpub_receive_branch_from_mnemonic(twelve_word, None, 1).unwrap();

        assert_eq!(account_0.receive_path, "m/44'/60'/0'/0");
        assert_eq!(account_1.receive_path, "m/44'/60'/1'/0");
        assert_ne!(account_0.receive_xpub, account_1.receive_xpub);

        let seed = mnemonic_seed(twelve_word, None).unwrap();
        let account_1_private = derive_private_key_at_path(&seed, "m/44'/60'/1'/0/0").unwrap();
        let account_1_public =
            derive_ethereum_address_from_xpub(&account_1.receive_xpub, 0).unwrap();
        assert_eq!(
            ethereum_address_from_verifying_key(account_1_private.verifying_key()),
            account_1_public.address
        );
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

    #[test]
    fn generated_mnemonics_have_requested_word_count_and_parse() {
        for word_count in [12usize, 24usize] {
            let phrase = generate_ethereum_mnemonic(word_count).expect("generate");
            assert_eq!(
                ethereum_mnemonic_word_count(&phrase).expect("word count"),
                word_count
            );
            let export = derive_ethereum_xpub_receive_branch_from_mnemonic(&phrase, None, 0)
                .expect("derive from generated phrase");
            assert!(export.receive_xpub.starts_with("xpub"));
        }
    }

    #[test]
    fn generated_mnemonics_are_unique() {
        let first = generate_ethereum_mnemonic(24).expect("first");
        let second = generate_ethereum_mnemonic(24).expect("second");
        assert_ne!(*first, *second);
    }

    #[test]
    fn unsupported_mnemonic_word_counts_are_rejected() {
        for word_count in [0usize, 11, 15, 18, 21, 23, 25] {
            assert_eq!(
                generate_ethereum_mnemonic(word_count).err(),
                Some(EthereumXpubError::InvalidMnemonicWordCount)
            );
        }
    }

    #[test]
    fn control_branch_derivation_roundtrip() {
        let twelve_word = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let control_export =
            derive_ethereum_xpub_control_branch_from_mnemonic(twelve_word, None, 0).unwrap();
        assert_eq!(control_export.receive_path, "m/44'/60'/0'/1");
        assert!(control_export.receive_xpub.starts_with("xpub"));

        let address_0 =
            derive_ethereum_address_from_control_xpub(&control_export.receive_xpub, 0).unwrap();
        assert!(address_0.address.starts_with("0x"));

        let seed = mnemonic_seed(twelve_word, None).unwrap();
        let pkey = derive_private_key_at_path(&seed, "m/44'/60'/0'/1/0").unwrap();
        assert_eq!(
            ethereum_address_from_verifying_key(pkey.verifying_key()),
            address_0.address
        );
    }
}
