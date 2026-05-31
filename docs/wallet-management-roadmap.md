# Comprehensive Wallet Management Roadmap

Sigillum should become a self-hosted wallet inventory, recovery, and
consolidation system. The existing local vault, provider profiles, stealth
deposit refresh, sweep queue, and maintenance loop are the foundation, but the
next product boundary is broader: find value wherever an operator may have left
it, explain what was found, and only then help consolidate it under explicit
policy.

This document defines the target shape for that work.
For the competitive landscape and product positioning behind this roadmap, see
[Wallet Competitive Landscape And World-Class Strategy](wallet-competitive-landscape.md).

## Product Goal

Sigillum should answer five operator questions without requiring a hosted
portfolio service:

1. Which wallets, accounts, and historical receive addresses exist for my
   imported seeds, xpubs, and Sigillum-managed wallets?
2. Which native coins, tokens, NFTs, DeFi positions, rewards, and airdrops have
   value or may become claimable?
3. Which holdings are stranded because of missing gas, inactive derivation
   paths, revoked providers, risky approvals, or protocol-specific exit rules?
4. What is the safest consolidation plan, including gas funding, swaps, unwraps,
   claims, exits, approvals, and final treasury routing?
5. What can be executed locally now, what needs review or quorum, and what
   should remain watch-only?

## Discovery Coverage

Discovery is the largest missing subsystem. It should be modeled as a local
indexing pipeline with chain/provider adapters, wallet derivation planners,
asset detectors, and protocol classifiers.

Required discovery classes:

- Native L1/L2 holdings across configured networks: Ethereum, Base, Arbitrum,
  Optimism, Polygon, BNB Chain, Avalanche, and other EVM networks supported by
  provider profiles.
- Historical account and receive-address discovery for imported seed wallets:
  standard Ethereum paths, common MetaMask/Ledger/Trezor patterns, project
  account paths, and configurable gap limits.
- Xpub receive branch discovery for public project wallets, including used
  address detection, balance checks, and gap-limit continuation.
- ERC-20 discovery from configured allowlists, transfer logs, token registries,
  and positive balances discovered through provider APIs or local index data.
  The first bounded transfer-log discovery slice is implemented for EVM
  inventory scans; registries, indexers, and richer positive-balance evidence
  remain future work.
- NFT discovery for ERC-721 and ERC-1155 ownership, including metadata caching,
  spam filtering, and optional floor or collection valuation providers.
  The first bounded ERC-721 transfer-log slice is implemented for EVM inventory
  scans and confirms current ownership with `ownerOf`; ERC-1155, metadata,
  spam filtering, and valuation providers remain future work.
- DeFi position discovery for common protocols: lending, staking, liquid
  staking, LP positions, vault shares, bridges, vesting/streaming contracts, and
  rewards contracts.
- Airdrop and reward discovery for claimable or potentially claimable assets,
  with claim-contract risk classification and no blind auto-claiming.
- Allowance and approval discovery, including unlimited ERC-20 approvals, NFT
  operator approvals, known-drainer spenders, and revoke recommendations.
  The first bounded ERC-20 allowance-probe slice is implemented for
  operator-supplied spender addresses; Permit2, NFT operator approvals,
  spender registries, and revoke execution remain future work.
- Dormant-wallet classification using last activity, transaction count, current
  value, token/NFT/DeFi exposure, gas availability, and whether the private key
  or signing path is actually available.

## Local Indexing Architecture

The daemon should add a local chain index layer rather than expanding the
current direct RPC helper into many route-local checks.

Suggested components:

- `chain_profiles`: network metadata, RPC providers, explorer/indexer endpoints,
  native asset metadata, block finality windows, gas-token rules, and chain
  capabilities.
- `wallet_inventory`: discovered wallet groups, derivation paths, addresses,
  ownership type, signing capability, activity windows, labels, and confidence.
- `asset_inventory`: native balances, ERC-20 balances, NFTs, DeFi positions,
  rewards, airdrops, allowances, and valuation snapshots.
- `discovery_jobs`: resumable scans with checkpointed ranges, provider rate
  limits, retry state, freshness targets, and auditable findings.
- `protocol_adapters`: isolated classifiers for ERC-20, ERC-721, ERC-1155,
  Uniswap-style LPs, Aave/Compound-style lending, staking contracts, bridge
  escrow, and airdrop claim contracts.
- `risk_catalog`: locally cached allowlists, blocklists, scam/spam heuristics,
  spender reputation, token metadata provenance, and operator overrides.

All raw wallet secrets remain in the vault. Discovery should use xpubs and
derived public addresses whenever possible. Signing keys should be touched only
when an operator asks Sigillum to simulate, prepare, or execute a specific
transaction plan.

## Consolidation Model

Consolidation should be a planner, not a button.

The planner should produce a reviewable execution graph:

- fund gas where needed
- revoke risky approvals when requested
- claim vetted rewards or airdrops
- unstake, withdraw, or exit protocol positions
- unwrap wrapped native assets where useful
- sweep native assets, ERC-20s, and NFTs
- optionally swap dust or long-tail tokens through approved routes
- route assets to hot, treasury, cold, or external destinations
- leave uneconomic, suspicious, or watch-only assets untouched

Every step should carry:

- source address and derivation path
- asset and chain
- estimated value and gas
- required signer or quorum
- simulation result
- slippage, deadline, and route constraints where swaps are involved
- risk classification
- whether it is automatic, review-required, or blocked

The existing queue and maintenance loop are the right execution substrate, but
the queue needs richer job payloads and pre-flight simulation before it should
move beyond current stealth-sweep behavior.

## Airdrops And Claims

Airdrops are discovery targets, but claiming is high risk. Sigillum should:

- discover possible eligibility through configured allowlists, public claim
  lists, Merkle proofs, protocol APIs, and address history
- store claim metadata locally, including source URL, contract address, chain,
  claim amount, proof hash, expiry, and confidence
- simulate every claim transaction before presenting it
- classify claims by known protocol, verified contract, unknown contract,
  approval-requiring claim, signature-only claim, or suspicious claim
- require explicit review for every claim by default
- never sign arbitrary typed data or permit-style messages without showing the
  exact spender, token, value, deadline, and domain

## DeFi Holdings

DeFi inventory should treat positions as first-class assets rather than opaque
tokens. Examples:

- lending deposits and debts
- LP shares and concentrated-liquidity NFT positions
- staking and liquid-staking balances
- vault shares
- pending rewards
- bridge deposits and pending withdrawals
- vested or streamed tokens

Each adapter should expose:

- current position value
- withdrawal or exit path
- claimable rewards
- liquidation or lockup risks where applicable
- transaction sequence required to consolidate

## NFT Inventory

NFT support should prioritize correctness and safety over gallery polish:

The first implementation scans bounded ERC-721 transfer-log ranges and records
only token IDs whose `ownerOf` result matches the scanned address.

- discover ERC-721 ownership from transfer logs and owner queries
- discover ERC-1155 balances for touched token IDs
- cache metadata locally with provenance
- flag spam NFTs, suspicious metadata links, and unknown collections
- support NFT sweeping only after review, with destination collection policy
- treat NFT approvals as a security surface alongside ERC-20 allowances

## Multi-Chain Direction

The first realistic expansion should be multi-network EVM because the current
service and signing code are already Ethereum-shaped. After that:

- Bitcoin and UTXO chains need a separate model: xpub import, BIP44/49/84/86
  path scanning, UTXO inventory, PSBT construction, fee/coin-control policy,
  and hardware-wallet signing.
- Solana needs account/program discovery, SPL tokens, compressed/NFT inventory,
  stake accounts, address lookup tables, and transaction simulation.
- Tron may matter for USDT recovery, but it should be isolated behind its own
  account, fee, token, and provider adapter rather than forced through EVM
  assumptions.
- Cosmos-family chains need account-number/sequence handling, staking rewards,
  IBC assets, and per-chain bech32 prefixes.

## Operator Experience

The UI should grow a wallet-management console with:

- portfolio inventory by wallet, chain, asset class, and confidence
- dormant wallet and stranded-value views
- discovery job progress and freshness indicators
- findings that distinguish native, token, NFT, DeFi, airdrop, and allowance
  results
- consolidation plan builder with dry-run, simulation, gas budget, and review
  stages
- explicit unsafe/suspicious buckets rather than hiding noisy assets
- exportable audit reports for discovery and execution

The CLI should have parity for automation:

- start/resume/cancel discovery jobs
- list findings and stale inventories
- generate consolidation plans
- run dry-runs and simulations
- approve or execute bounded queue jobs
- export machine-readable reports

## Phasing

1. Inventory schema and read-only EVM discovery.
2. Seed/xpub derivation scanning with gap limits and historical address
   activity.
3. ERC-20 and native multi-L2 discovery. Bounded ERC-20 transfer-log discovery
   is the first implemented slice.
4. NFT and allowance discovery. Operator-bounded ERC-20 allowance probing is
   the first implemented approval-discovery slice, and bounded ERC-721
   transfer-log discovery with owner confirmation is the first NFT slice.
5. DeFi position adapters for the most common protocols.
6. Airdrop/reward discovery with strict claim risk gates.
7. Consolidation planner with dry-run and simulation.
8. Controlled execution for native/ERC-20 sweeps, gas top-ups, NFT transfers,
   DeFi exits, claims, swaps, and treasury routing.
9. Non-EVM chain families, starting with Bitcoin/UTXO and only then Solana,
   Tron, and Cosmos-style networks.

Completion means a recovered or imported wallet can move from "unknown" to
"inventoried" to "reviewed plan" to "executed or intentionally ignored" without
leaving the local Sigillum control plane.
