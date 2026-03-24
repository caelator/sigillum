//! Property / fuzz-style tests for cryptographic boundaries.
//!
//! Uses `proptest` strategies to generate random inputs and verify that
//! core cryptographic operations maintain their invariants.

use proptest::prelude::*;
use sigillum_core::decode_quantity_hex;

// ── Hex Codec Roundtrip ────────────────────────────────────────────

proptest! {
    /// Hex-encoding and decoding a random byte sequence always roundtrips.
    #[test]
    fn hex_encode_decode_roundtrip(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        let encoded = hex::encode(&bytes);
        let decoded = hex::decode(&encoded).unwrap();
        prop_assert_eq!(&bytes, &decoded);
    }

    /// A 32-byte quantity roundtrips through hex encoding → decode_quantity_hex.
    #[test]
    fn quantity_hex_roundtrip(value in prop::collection::vec(any::<u8>(), 32..=32)) {
        let encoded = format!("0x{}", hex::encode(&value));
        let decoded = decode_quantity_hex(&encoded);
        prop_assert!(decoded.is_ok(), "valid 32-byte hex should decode");
        let result = decoded.unwrap();
        prop_assert_eq!(&result[..], &value[..]);
    }

    /// Empty or malformed hex strings are rejected by decode_quantity_hex.
    #[test]
    fn invalid_hex_rejected(
        noise in "[^0-9a-fA-Fx]{1,100}"
    ) {
        let result = decode_quantity_hex(&noise);
        prop_assert!(result.is_err(), "garbage input should be rejected");
    }

    /// Oversized hex payloads are rejected (more than 32 bytes).
    #[test]
    fn oversized_hex_rejected(data in prop::collection::vec(any::<u8>(), 33..128usize)) {
        let encoded = format!("0x{}", hex::encode(&data));
        let result = decode_quantity_hex(&encoded);
        prop_assert!(result.is_err(), "oversized hex should be rejected");
    }
}

// ── Vault Secret Roundtrip ─────────────────────────────────────────

#[cfg(feature = "file-backend")]
mod vault_tests {
    use proptest::prelude::*;
    use rand::RngCore;
    use secrecy::ExposeSecret;
    use sigillum_core::{FileVault, SecretStore, VaultConfig, VaultLifecycle};

    fn make_vault(base: std::path::PathBuf) -> FileVault {
        FileVault::new(VaultConfig {
            base_dir: base,
            tier1_file: "api_keys.json".into(),
            tier2_file: "vault.enc".into(),
        })
    }

    fn random_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut key);
        key
    }

    proptest! {
        /// Secret values survive a set → get roundtrip for arbitrary keys/values.
        #[test]
        fn secret_set_get_roundtrip(
            key in "[a-zA-Z0-9_-]{1,64}",
            value in ".{1,512}"
        ) {
            let tmp = tempfile::tempdir().unwrap();
            let vault = make_vault(tmp.path().to_path_buf());
            let mk = random_key();
            vault.initialize(&mk).unwrap();
            vault.load_master_key(mk);

            vault.set_secret(&key, &value).unwrap();
            let got = vault.read_secret(&key).unwrap();
            prop_assert!(got.is_some(), "secret should exist after set");
            let retrieved = got.unwrap();
            let exposed = retrieved.expose_secret();
            prop_assert_eq!(exposed, &value);
        }

        /// API key values survive a set → get roundtrip.
        #[test]
        fn api_key_set_get_roundtrip(
            key in "[a-zA-Z0-9_-]{1,64}",
            value in ".{1,512}"
        ) {
            let tmp = tempfile::tempdir().unwrap();
            let vault = make_vault(tmp.path().to_path_buf());

            vault.set_api_key(&key, &value).unwrap();
            let got = vault.read_api_key(&key).unwrap();
            prop_assert!(got.is_some(), "api key should exist after set");
            let retrieved = got.unwrap();
            let exposed = retrieved.expose_secret();
            prop_assert_eq!(exposed, &value);
        }

        /// Deleting a secret makes it unreadable.
        #[test]
        fn secret_delete_removes(
            key in "[a-zA-Z0-9_-]{1,32}"
        ) {
            let tmp = tempfile::tempdir().unwrap();
            let vault = make_vault(tmp.path().to_path_buf());
            let mk = random_key();
            vault.initialize(&mk).unwrap();
            vault.load_master_key(mk);

            vault.set_secret(&key, "ephemeral").unwrap();
            vault.delete_secret(&key).unwrap();
            let got = vault.read_secret(&key).unwrap();
            prop_assert!(got.is_none(), "deleted secret should be gone");
        }
    }
}

// ── API Serde Roundtrip ────────────────────────────────────────────

mod api_serde {
    use proptest::prelude::*;

    proptest! {
        /// Arbitrary stealth address hex strings parse → serialize → parse.
        #[test]
        fn stealth_payment_ref_serde_roundtrip(
            addr in "0x[0-9a-f]{40}",
            epk in "[0-9a-f]{66}",
            view_tag in prop::option::of("[0-9a-f]{2}")
        ) {
            let original = sigillum_api::request::StealthPaymentRef {
                stealth_address: addr.clone(),
                ephemeral_public_key_hex: epk.clone(),
                view_tag_hex: view_tag.clone(),
            };
            let json = serde_json::to_string(&original).unwrap();
            let deserialized: sigillum_api::request::StealthPaymentRef =
                serde_json::from_str(&json).unwrap();
            prop_assert_eq!(&original.stealth_address, &deserialized.stealth_address);
            prop_assert_eq!(&original.ephemeral_public_key_hex, &deserialized.ephemeral_public_key_hex);
            prop_assert_eq!(&original.view_tag_hex, &deserialized.view_tag_hex);
        }

        /// EvmProviderRef serde roundtrip
        #[test]
        fn evm_provider_ref_serde_roundtrip(
            url in "https?://[a-z]{3,12}\\.[a-z]{2,4}",
            auth in prop::option::of("[a-z]{3,20}"),
            cid in prop::option::of(0usize..100)
        ) {
            let original = sigillum_api::request::EvmProviderRef {
                rpc_url: url.clone(),
                auth_token_key: auth.clone(),
                compartment_id: cid,
            };
            let json = serde_json::to_string(&original).unwrap();
            let deserialized: sigillum_api::request::EvmProviderRef =
                serde_json::from_str(&json).unwrap();
            prop_assert_eq!(&original.rpc_url, &deserialized.rpc_url);
            prop_assert_eq!(&original.auth_token_key, &deserialized.auth_token_key);
            prop_assert_eq!(&original.compartment_id, &deserialized.compartment_id);
        }
    }
}
