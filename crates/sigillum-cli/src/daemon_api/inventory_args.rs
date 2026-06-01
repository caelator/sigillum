use std::process;

use sigillum_api::request::{ClaimCandidateProbe, DefiTokenProbe};

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
    if parts.len() != 7 || parts.iter().any(|part| part.trim().is_empty()) {
        eprintln!(
            "Invalid value for --claim-candidate: expected kind:protocol:claimant_address:claim_contract_address:asset_address:amount_hex:source_label"
        );
        process::exit(1);
    }
    ClaimCandidateProbe {
        kind: parts[0].trim().to_string(),
        protocol: parts[1].trim().to_string(),
        claimant_address: parts[2].trim().to_string(),
        claim_contract_address: parts[3].trim().to_string(),
        asset_address: parts[4].trim().to_string(),
        amount_hex: parts[5].trim().to_string(),
        source_label: parts[6].trim().to_string(),
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
