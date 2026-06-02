use std::{fs, process};

use sigillum_api::request::{ClaimCandidateProbe, DefiTokenProbe, WatchAddressProbe};

pub(super) fn parse_defi_token_probes(args: &[String]) -> Vec<DefiTokenProbe> {
    parse_multi_flag(args, "--defi-token-probe")
        .into_iter()
        .map(|value| parse_defi_token_probe_value(&value))
        .collect()
}

pub(super) fn parse_claim_candidate_probes(args: &[String]) -> Vec<ClaimCandidateProbe> {
    parse_multi_flag(args, "--claim-candidate")
        .into_iter()
        .map(|value| parse_claim_candidate_value(&value))
        .collect()
}

pub(super) fn parse_watch_address_probes(args: &[String]) -> Vec<WatchAddressProbe> {
    let mut probes = Vec::new();
    for value in parse_multi_flag(args, "--watch-address") {
        push_unique_watch_probe(&mut probes, parse_watch_address_value(&value));
    }
    for path in parse_multi_flag(args, "--watch-address-file") {
        for probe in parse_watch_address_file(&path) {
            push_unique_watch_probe(&mut probes, probe);
        }
    }
    probes
}

fn parse_defi_token_probe_value(value: &str) -> DefiTokenProbe {
    let parts = value.split(':').collect::<Vec<_>>();
    if !(parts.len() == 2 || parts.len() == 3) || parts.iter().any(|part| part.trim().is_empty()) {
        eprintln!(
            "Invalid value for --defi-token-probe: expected protocol:token_address[:protocol_address]"
        );
        process::exit(1);
    }
    DefiTokenProbe {
        protocol: parts[0].trim().to_string(),
        token_address: parts[1].trim().to_string(),
        protocol_address: parts.get(2).map(|value| value.trim().to_string()),
    }
}

fn parse_claim_candidate_value(value: &str) -> ClaimCandidateProbe {
    let parts = value.split(':').collect::<Vec<_>>();
    if !(parts.len() == 7 || parts.len() == 10) || parts.iter().any(|part| part.trim().is_empty()) {
        eprintln!(
            "Invalid value for --claim-candidate: expected kind:protocol:claimant_address:claim_contract_address:asset_address:amount_hex:source_label[:claim_adapter:claim_index_hex:proof1,proof2]"
        );
        process::exit(1);
    }
    let (claim_adapter, claim_index_hex, claim_proof) = if parts.len() == 10 {
        (
            Some(parts[7].trim().to_string()),
            Some(parts[8].trim().to_string()),
            parts[9]
                .split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_string)
                .collect(),
        )
    } else {
        (None, None, Vec::new())
    };
    ClaimCandidateProbe {
        kind: parts[0].trim().to_string(),
        protocol: parts[1].trim().to_string(),
        claimant_address: parts[2].trim().to_string(),
        claim_contract_address: parts[3].trim().to_string(),
        asset_address: parts[4].trim().to_string(),
        amount_hex: parts[5].trim().to_string(),
        source_label: parts[6].trim().to_string(),
        claim_adapter,
        claim_index_hex,
        claim_proof,
    }
}

fn parse_watch_address_value(value: &str) -> WatchAddressProbe {
    parse_watch_address_record(value).unwrap_or_else(|| {
        eprintln!("Invalid value for --watch-address: expected address[:label]");
        process::exit(1);
    })
}

fn parse_watch_address_file(path: &str) -> Vec<WatchAddressProbe> {
    let contents = fs::read_to_string(path).unwrap_or_else(|error| {
        eprintln!("Failed to read --watch-address-file {path}: {error}");
        process::exit(1);
    });
    contents
        .lines()
        .filter_map(parse_watch_address_record)
        .collect()
}

fn parse_watch_address_record(value: &str) -> Option<WatchAddressProbe> {
    let value = value.trim();
    if value.is_empty()
        || value.starts_with('#')
        || value.to_ascii_lowercase().starts_with("address,")
    {
        return None;
    }
    if let Some((address, label)) = value.split_once(',') {
        return build_watch_address_probe(address, Some(label));
    }
    if let Some((address, label)) = value.split_once(':') {
        return build_watch_address_probe(address, Some(label));
    }
    build_watch_address_probe(value, None)
}

fn build_watch_address_probe(address: &str, label: Option<&str>) -> Option<WatchAddressProbe> {
    let address = address.trim();
    if address.is_empty() {
        return None;
    }
    Some(WatchAddressProbe {
        address: address.to_string(),
        label: label
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .map(str::to_string),
    })
}

fn push_unique_watch_probe(probes: &mut Vec<WatchAddressProbe>, probe: WatchAddressProbe) {
    if let Some(existing) = probes
        .iter_mut()
        .find(|existing| existing.address.eq_ignore_ascii_case(&probe.address))
    {
        if existing.label.is_none() && probe.label.is_some() {
            existing.label = probe.label;
        }
    } else {
        probes.push(probe);
    }
}

fn parse_multi_flag(args: &[String], flag: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == flag && i + 1 < args.len() {
            values.push(args[i + 1].clone());
            i += 1;
        }
        i += 1;
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_watch_address_flags_and_files_with_dedupe() {
        let path = std::env::temp_dir().join(format!(
            "sigillum-watch-addresses-{}-{}.csv",
            std::process::id(),
            "bulk"
        ));
        std::fs::write(
            &path,
            [
                "address,label",
                "# imported from old client sheet",
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,old-ledger",
                "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:client-vault",
                "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA,duplicate",
            ]
            .join("\n"),
        )
        .unwrap();
        let args = vec![
            "scan-evm".into(),
            "--watch-address".into(),
            "0xcccccccccccccccccccccccccccccccccccccccc:single".into(),
            "--watch-address-file".into(),
            path.to_string_lossy().into_owned(),
        ];

        let probes = parse_watch_address_probes(&args);
        let _ = std::fs::remove_file(path);

        assert_eq!(probes.len(), 3);
        assert_eq!(
            probes[0].address,
            "0xcccccccccccccccccccccccccccccccccccccccc"
        );
        assert_eq!(probes[0].label.as_deref(), Some("single"));
        assert_eq!(
            probes[1].address,
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(probes[1].label.as_deref(), Some("old-ledger"));
        assert_eq!(
            probes[2].address,
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
        assert_eq!(probes[2].label.as_deref(), Some("client-vault"));
    }

    #[test]
    fn parse_watch_address_record_accepts_line_formats() {
        assert!(parse_watch_address_record("").is_none());
        assert!(parse_watch_address_record("# comment").is_none());
        assert!(parse_watch_address_record("address,label").is_none());

        let plain =
            parse_watch_address_record("0xdddddddddddddddddddddddddddddddddddddddd").unwrap();
        assert_eq!(plain.label, None);

        let csv =
            parse_watch_address_record("0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee, old seed")
                .unwrap();
        assert_eq!(csv.label.as_deref(), Some("old seed"));
    }
}
