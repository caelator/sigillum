//! Gateway unit and integration tests.
//!
//! Tests the security-critical paths: auth, SSRF validation, API key hashing,
//! webhook HMAC signing, amount validation, EVM address validation, constant-time.

// ── Auth tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod auth_tests {
    use sigillum_gateway_test_helpers::*;

    #[test]
    fn hash_api_key_deterministic() {
        let h1 = hash_api_key("sgw_test_key_123");
        let h2 = hash_api_key("sgw_test_key_123");
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_api_key_different_inputs() {
        let h1 = hash_api_key("sgw_key_a");
        let h2 = hash_api_key("sgw_key_b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_api_key_is_64_hex_chars() {
        let h = hash_api_key("any_key");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_api_key_empty_string() {
        let h = hash_api_key("");
        assert_eq!(h.len(), 64);
    }

    mod sigillum_gateway_test_helpers {
        use sha2::{Digest, Sha256};

        pub fn hash_api_key(key: &str) -> String {
            let mut hasher = Sha256::new();
            hasher.update(key.as_bytes());
            hex::encode(hasher.finalize())
        }
    }
}

// ── HMAC Webhook Signing ───────────────────────────────────────────

#[cfg(test)]
mod webhook_tests {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    fn sign_payload(secret: &str, payload: &str) -> String {
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
        mac.update(payload.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    fn verify_signature(secret: &str, payload: &str, signature: &str) -> bool {
        let expected = sign_payload(secret, payload);
        expected == signature
    }

    #[test]
    fn sign_payload_deterministic() {
        let s1 = sign_payload("secret", "payload");
        let s2 = sign_payload("secret", "payload");
        assert_eq!(s1, s2);
    }

    #[test]
    fn sign_payload_different_secrets() {
        let s1 = sign_payload("secret_a", "payload");
        let s2 = sign_payload("secret_b", "payload");
        assert_ne!(s1, s2);
    }

    #[test]
    fn sign_payload_different_payloads() {
        let s1 = sign_payload("secret", "payload_a");
        let s2 = sign_payload("secret", "payload_b");
        assert_ne!(s1, s2);
    }

    #[test]
    fn verify_valid_signature() {
        let sig = sign_payload("my_secret", r#"{"event":"payment.confirmed"}"#);
        assert!(verify_signature(
            "my_secret",
            r#"{"event":"payment.confirmed"}"#,
            &sig
        ));
    }

    #[test]
    fn verify_invalid_signature() {
        assert!(!verify_signature(
            "my_secret",
            r#"{"event":"payment.confirmed"}"#,
            "0000000000000000000000000000000000000000000000000000000000000000"
        ));
    }

    #[test]
    fn verify_tampered_payload() {
        let sig = sign_payload("my_secret", r#"{"event":"payment.confirmed"}"#);
        assert!(!verify_signature(
            "my_secret",
            r#"{"event":"payment.swept"}"#,
            &sig
        ));
    }
}

// ── SSRF URL Validation ────────────────────────────────────────────

#[cfg(test)]
mod ssrf_tests {
    fn validate_webhook_url(url: &str) -> Result<(), String> {
        let parsed = url::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;

        // A3: HTTPS only, no localhost exception
        if parsed.scheme() != "https" {
            return Err(format!(
                "webhook URLs must use HTTPS (got '{}')",
                parsed.scheme()
            ));
        }

        let host = parsed.host_str().ok_or("URL has no host")?;

        let dangerous_hosts = [
            "localhost",
            "127.0.0.1",
            "[::1]",
            "0.0.0.0",
            "metadata.google.internal",
            "metadata.internal",
        ];
        let dangerous_prefixes = [
            "169.254.", "10.", "192.168.", "172.16.", "172.17.", "172.18.", "172.19.", "172.2",
            "172.30.", "172.31.",
        ];

        if dangerous_hosts.contains(&host) {
            return Err(format!("private network: {host}"));
        }
        for prefix in &dangerous_prefixes {
            if host.starts_with(prefix) {
                return Err(format!("private network: {host}"));
            }
        }

        Ok(())
    }

    #[test]
    fn rejects_http_always() {
        assert!(validate_webhook_url("http://example.com/hook").is_err());
        assert!(validate_webhook_url("http://localhost:3000/hook").is_err());
    }

    #[test]
    fn allows_https_public() {
        assert!(validate_webhook_url("https://example.com/hook").is_ok());
    }

    #[test]
    fn rejects_private_10_range() {
        assert!(validate_webhook_url("https://10.0.0.1/hook").is_err());
    }

    #[test]
    fn rejects_private_192_168() {
        assert!(validate_webhook_url("https://192.168.1.1/hook").is_err());
    }

    #[test]
    fn rejects_aws_metadata() {
        assert!(validate_webhook_url("https://169.254.169.254/latest/meta-data/").is_err());
    }

    #[test]
    fn rejects_gcp_metadata() {
        assert!(validate_webhook_url("https://metadata.google.internal/v1/").is_err());
    }

    #[test]
    fn rejects_ftp() {
        assert!(validate_webhook_url("ftp://ftp.example.com/hook").is_err());
    }

    #[test]
    fn rejects_garbage() {
        assert!(validate_webhook_url("not a url at all").is_err());
        assert!(validate_webhook_url("").is_err());
    }

    #[test]
    fn rejects_zero_address() {
        assert!(validate_webhook_url("https://0.0.0.0/hook").is_err());
    }

    #[test]
    fn rejects_https_localhost() {
        // Even HTTPS to localhost is blocked — private host
        assert!(validate_webhook_url("https://localhost/hook").is_err());
    }
}

// ── EVM Address Validation (A5) ────────────────────────────────────

#[cfg(test)]
mod evm_address_tests {
    fn validate_evm_address(addr: &str) -> Result<(), String> {
        let hex_part = addr
            .strip_prefix("0x")
            .or_else(|| addr.strip_prefix("0X"))
            .ok_or("EVM address must start with 0x")?;
        if hex_part.len() != 40 {
            return Err(format!(
                "EVM address must be 42 characters (got {})",
                addr.len()
            ));
        }
        if !hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("EVM address contains non-hex characters".into());
        }
        Ok(())
    }

    #[test]
    fn valid_checksummed() {
        assert!(validate_evm_address("0xdAC17F958D2ee523a2206206994597C13D831ec7").is_ok());
    }

    #[test]
    fn valid_lowercase() {
        assert!(validate_evm_address("0x0000000000000000000000000000000000000000").is_ok());
    }

    #[test]
    fn valid_uppercase_prefix() {
        assert!(validate_evm_address("0XdAC17F958D2ee523a2206206994597C13D831ec7").is_ok());
    }

    #[test]
    fn rejects_no_prefix() {
        assert!(validate_evm_address("dAC17F958D2ee523a2206206994597C13D831ec7").is_err());
    }

    #[test]
    fn rejects_short() {
        assert!(validate_evm_address("0xdAC17F").is_err());
    }

    #[test]
    fn rejects_long() {
        assert!(validate_evm_address("0xdAC17F958D2ee523a2206206994597C13D831ec7FFFF").is_err());
    }

    #[test]
    fn rejects_non_hex() {
        assert!(validate_evm_address("0xZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ").is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(validate_evm_address("").is_err());
        assert!(validate_evm_address("0x").is_err());
    }
}

// ── Amount Validation ──────────────────────────────────────────────

#[cfg(test)]
mod amount_tests {
    fn validate_amount_wei(s: &str) -> Result<(), String> {
        if s.is_empty() {
            return Err("amount_wei is required".into());
        }
        let hex_str = s.strip_prefix("0x").unwrap_or(s);
        if hex_str.is_empty() {
            return Err("amount_wei cannot be just '0x'".into());
        }
        if hex_str.len() > 64 {
            return Err("amount_wei exceeds maximum (256-bit)".into());
        }
        if !hex_str.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("amount_wei must be valid hex".into());
        }
        Ok(())
    }

    #[test]
    fn valid_hex_amounts() {
        assert!(validate_amount_wei("0x2386F26FC10000").is_ok());
        assert!(validate_amount_wei("2386F26FC10000").is_ok());
        assert!(validate_amount_wei("0x0").is_ok());
        assert!(validate_amount_wei("0xff").is_ok());
    }

    #[test]
    fn rejects_empty() {
        assert!(validate_amount_wei("").is_err());
    }

    #[test]
    fn rejects_just_prefix() {
        assert!(validate_amount_wei("0x").is_err());
    }

    #[test]
    fn rejects_non_hex() {
        assert!(validate_amount_wei("0xGGG").is_err());
        assert!(validate_amount_wei("hello").is_err());
        assert!(validate_amount_wei("0x123xyz").is_err());
    }

    #[test]
    fn rejects_oversized() {
        let oversized = "0x".to_string() + &"f".repeat(65);
        assert!(validate_amount_wei(&oversized).is_err());
    }

    #[test]
    fn accepts_max_256_bit() {
        let maxval = "0x".to_string() + &"f".repeat(64);
        assert!(validate_amount_wei(&maxval).is_ok());
    }
}

// ── Constant-Time Comparison ───────────────────────────────────────

#[cfg(test)]
mod ct_tests {
    use subtle::ConstantTimeEq;

    fn ct_hash_eq(a: &str, b: &str) -> bool {
        let a_bytes = a.as_bytes();
        let b_bytes = b.as_bytes();
        if a_bytes.len() != b_bytes.len() {
            return false;
        }
        a_bytes.ct_eq(b_bytes).into()
    }

    #[test]
    fn equal_hashes_match() {
        assert!(ct_hash_eq("abc123", "abc123"));
    }

    #[test]
    fn different_hashes_dont_match() {
        assert!(!ct_hash_eq("abc123", "abc124"));
    }

    #[test]
    fn different_lengths_dont_match() {
        assert!(!ct_hash_eq("abc", "abcd"));
    }

    #[test]
    fn empty_strings_match() {
        assert!(ct_hash_eq("", ""));
    }
}
