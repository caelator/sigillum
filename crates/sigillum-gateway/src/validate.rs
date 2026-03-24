//! Webhook URL validation — SSRF protection.

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedWebhookTarget {
    pub url: url::Url,
    pub addrs: Vec<SocketAddr>,
    pub dns_name: Option<String>,
}

/// Validate a webhook URL is safe to POST to.
///
/// Rejects:
/// - Non-HTTPS schemes
/// - Private/reserved IP ranges (cloud metadata, loopback, link-local)
/// - URLs that resolve to private IPs
pub fn validate_webhook_url(url: &str) -> Result<(), String> {
    let parsed = parse_https_webhook_url(url)?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;

    // Block private/reserved hosts
    if is_private_host(host) {
        return Err(format!(
            "webhook URL must not point to private network: {host}"
        ));
    }

    // Resolve DNS and verify no result is a private IP (DNS rebinding defense)
    let port = parsed.port().unwrap_or(443);
    if let Ok(addrs) = format!("{host}:{port}").to_socket_addrs() {
        for addr in addrs {
            if is_private_ip(addr.ip()) {
                return Err(format!("webhook URL resolves to private IP: {}", addr.ip()));
            }
        }
    }

    Ok(())
}

/// Resolve and pin a webhook target before delivery so DNS changes after project
/// creation cannot silently redirect delivery into private address space.
pub fn resolve_webhook_target(url: &str) -> Result<ResolvedWebhookTarget, String> {
    let parsed = parse_https_webhook_url(url)?;
    let port = parsed.port().unwrap_or(443);
    let host = parsed
        .host()
        .ok_or_else(|| "URL has no host".to_string())?
        .to_owned();

    match host {
        url::Host::Domain(host) => {
            if is_private_host(&host) {
                return Err(format!(
                    "webhook URL must not point to private network: {host}"
                ));
            }
            let addrs: Vec<SocketAddr> = format!("{host}:{port}")
                .to_socket_addrs()
                .map_err(|error| format!("failed to resolve webhook host {host}: {error}"))?
                .collect();
            if addrs.is_empty() {
                return Err(format!(
                    "failed to resolve webhook host {host}: no addresses found"
                ));
            }
            if let Some(addr) = addrs.iter().find(|addr| is_private_ip(addr.ip())) {
                return Err(format!("webhook URL resolves to private IP: {}", addr.ip()));
            }
            Ok(ResolvedWebhookTarget {
                url: parsed,
                addrs,
                dns_name: Some(host.to_ascii_lowercase()),
            })
        }
        url::Host::Ipv4(ip) => {
            if is_private_ip(IpAddr::V4(ip)) {
                return Err(format!("webhook URL resolves to private IP: {ip}"));
            }
            Ok(ResolvedWebhookTarget {
                url: parsed,
                addrs: vec![SocketAddr::new(IpAddr::V4(ip), port)],
                dns_name: None,
            })
        }
        url::Host::Ipv6(ip) => {
            if is_private_ip(IpAddr::V6(ip)) {
                return Err(format!("webhook URL resolves to private IP: {ip}"));
            }
            Ok(ResolvedWebhookTarget {
                url: parsed,
                addrs: vec![SocketAddr::new(IpAddr::V6(ip), port)],
                dns_name: None,
            })
        }
    }
}

/// Validate that a string looks like an EVM address: `0x` + 40 hex characters.
pub fn validate_evm_address(addr: &str) -> Result<(), String> {
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

fn is_private_host(host: &str) -> bool {
    matches!(
        host,
        "localhost"
            | "127.0.0.1"
            | "[::1]"
            | "0.0.0.0"
            | "metadata.google.internal"
            | "metadata.internal"
    ) || host.starts_with("169.254.")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("172.16.")
        || host.starts_with("172.17.")
        || host.starts_with("172.18.")
        || host.starts_with("172.19.")
        || host.starts_with("172.2")
        || host.starts_with("172.30.")
        || host.starts_with("172.31.")
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                // AWS/GCP metadata endpoint
                || v4.octets()[0] == 169 && v4.octets()[1] == 254
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
    }
}

fn parse_https_webhook_url(url: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;

    // A3: Require HTTPS unconditionally — no localhost exception
    if parsed.scheme() != "https" {
        return Err(format!(
            "webhook URLs must use HTTPS (got '{}')",
            parsed.scheme()
        ));
    }

    if parsed.host().is_none() {
        return Err("URL has no host".to_string());
    }

    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── SSRF tests ─────────────────────────────────────────────────

    #[test]
    fn rejects_http_always() {
        assert!(validate_webhook_url("http://example.com/hook").is_err());
        assert!(validate_webhook_url("http://localhost/hook").is_err());
    }

    #[test]
    fn allows_https() {
        let result = validate_webhook_url("https://example.com/hook");
        if let Err(e) = &result {
            assert!(!e.contains("must use HTTPS"), "Unexpected HTTPS rejection");
            assert!(
                !e.contains("private network"),
                "Unexpected private net rejection"
            );
        }
    }

    #[test]
    fn rejects_private_ips() {
        assert!(validate_webhook_url("https://10.0.0.1/hook").is_err());
        assert!(validate_webhook_url("https://192.168.1.1/hook").is_err());
        assert!(validate_webhook_url("https://169.254.169.254/latest").is_err());
    }

    #[test]
    fn resolve_webhook_target_rejects_private_ip_literals() {
        assert!(resolve_webhook_target("https://127.0.0.1/hook").is_err());
        assert!(resolve_webhook_target("https://[::1]/hook").is_err());
    }

    #[test]
    fn resolve_webhook_target_accepts_public_ip_literals() {
        let target = resolve_webhook_target("https://1.1.1.1/hook").unwrap();
        assert_eq!(target.dns_name, None);
        assert_eq!(target.addrs, vec!["1.1.1.1:443".parse().unwrap()]);
    }

    #[test]
    fn rejects_metadata_endpoint() {
        assert!(
            validate_webhook_url("https://metadata.google.internal/computeMetadata/v1/").is_err()
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(validate_webhook_url("not a url").is_err());
        assert!(validate_webhook_url("").is_err());
        assert!(validate_webhook_url("ftp://example.com").is_err());
    }

    // ── EVM address tests ──────────────────────────────────────────

    #[test]
    fn valid_evm_address() {
        assert!(validate_evm_address("0xdAC17F958D2ee523a2206206994597C13D831ec7").is_ok());
        assert!(validate_evm_address("0x0000000000000000000000000000000000000000").is_ok());
    }

    #[test]
    fn rejects_no_prefix() {
        assert!(validate_evm_address("dAC17F958D2ee523a2206206994597C13D831ec7").is_err());
    }

    #[test]
    fn rejects_short_address() {
        assert!(validate_evm_address("0xdAC17F958D2ee5").is_err());
    }

    #[test]
    fn rejects_long_address() {
        assert!(validate_evm_address("0xdAC17F958D2ee523a2206206994597C13D831ec7FF").is_err());
    }

    #[test]
    fn rejects_non_hex() {
        assert!(validate_evm_address("0xZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ").is_err());
    }
}
