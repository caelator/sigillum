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
  provider profiles. The first registry-backed chain slice is implemented with
  non-deletable built-ins for Ethereum, Base, Arbitrum One, OP Mainnet, and
  Polygon PoS, plus operator-defined custom EVM entries exposed through
  daemon API, CLI, and UI. EVM inventory scans can now explicitly target all
  configured provider profiles in one operator action, and discovery jobs
  record the scanned chain IDs.
- Historical account and receive-address discovery for imported seed wallets:
  standard Ethereum paths, common MetaMask/Ledger/Trezor patterns, project
  account paths, and configurable gap limits. The first multi-account seed
  discovery slice is implemented for EVM inventory scans with optional
  `derivation_pattern` values of `project`, `standard`, or `ledger_live`;
  `standard` and `ledger_live` derive read-only receive xpubs from the
  encrypted seed secret and scan account branches up to `account_limit`.
- Xpub receive branch discovery for public project wallets and imported
  external watch-only receive branches, including used address detection,
  balance checks, and gap-limit continuation. The first true imported-xpub
  slice is implemented for Ethereum receive-branch xpubs
  (`m/44'/60'/account'/0`) through the additive `external_receive_xpub`
  profile field. Ethereum account-level xpub import is also implemented through
  `external_account_xpub`; Sigillum validates `m/44'/60'/account'` account
  public keys, normalizes them into receive-branch xpubs for scanning, and
  keeps them non-executable/watch-only. Custom imported receive-xpub paths are
  implemented through `external_receive_path` paired with
  `external_receive_xpub`; Sigillum validates BIP-32 path syntax, checks the
  xpub depth matches the supplied path, uses that path in inventory/export/
  self-check evidence, and surfaces it as operator-asserted metadata because a
  watch-only path cannot be cryptographically bound to an xpub. Custom imported
  account-level paths are also implemented through `external_account_path`
  paired with `external_account_xpub`; Sigillum validates the terminal hardened
  account path, derives the receive branch locally, and keeps the profile
  watch-only.
- Ad-hoc and saved EVM watch-address discovery for old exchange, hardware
  wallet, client, or externally found addresses. The first slices are
  implemented as bounded read-only `eth-watch` probes in EVM inventory scans,
  bulk operator input in the UI, CLI `--watch-address-file` import, duplicate
  probe collapse, JSON inventory report export, and a persisted daemon/CLI/UI
  watch-address book that scans with `--include-watch-book`, preserving
  watch-only classification and blocking consolidation execution unless signer
  material is later introduced through an explicit profile.
- ERC-20 discovery from configured allowlists, transfer logs, token registries,
  and positive balances discovered through provider APIs or local index data.
  The first bounded transfer-log discovery slice is implemented for EVM
  inventory scans and now persists per-address/chain/topic block cursors so
  later scans resume from the last scanned transfer-log block. Operator-imported
  local token registries are now implemented: lists are imported from pasted JSON
  or a local file path, never fetched from the network (D-15). Scans with
  `probe_token_registry` probe matching-chain entries with `balanceOf`, record
  positive balances as holdings with `token_registry:<list-name>` provenance,
  and skip wrong-chain entries. Indexers and richer positive-balance evidence
  remain future work.
- NFT discovery for ERC-721 and ERC-1155 ownership, including metadata caching,
  spam filtering, and optional floor or collection valuation providers.
  The first bounded ERC-721 transfer-log slice is implemented for EVM inventory
  scans, resumes from per-address/chain/topic block cursors, and confirms
  current ownership with `ownerOf`; bounded ERC-1155 transfer discovery is
  implemented with `balanceOf` confirmation for touched token IDs and uses the
  same resumable block-cursor model. Per-collection opt-in metadata fetching is
  implemented: `tokenURI`/`uri` resolves through provider RPC, downloads use the
  daemon's bounded HTTP client, and cached results record provenance (URI, fetch
  time, sha-256 content hash); IPFS metadata is fetched only through an
  operator-configured gateway, otherwise it is skipped with a recorded reason.
  Local conservative spam heuristics now record reasons for suspected airdrops,
  trusted-name lookalikes, and operator risk-catalog overrides, and surface them
  in a never-auto-hidden Suspicious NFTs bucket. Floor or collection valuation
  providers remain out of scope for 1.0 (D-16).
- DeFi position discovery for common protocols: lending, staking, liquid
  staking, LP positions, vault shares, bridges, vesting/streaming contracts, and
  rewards contracts. The first local slice records operator-configured ERC-20
  receipt/share token probes as `defi` holdings with protocol provenance, which
  covers many lending, vault, staking, and LP receipt-token positions before
  protocol-specific exit adapters exist.
- Airdrop and reward discovery for claimable or potentially claimable assets,
  with claim-contract risk classification and no blind auto-claiming. The first
  local slice records operator-configured trusted claim candidates as `airdrop`
  or `reward` holdings keyed to the claimant address, asset contract, claim
  contract, amount, protocol, optional standard Merkle proof evidence, and
  source label. These candidates are reviewable in inventory, planning, and
  local risk findings. The risk engine uses the claim contract as the review
  subject and applies local risk-catalog overrides. Candidates with
  `merkle-distributor-v1` evidence can be simulated with provider-backed
  `eth_call`, but claim execution remains blocked until an explicit execution
  adapter and operator approval are added.
- Allowance and approval discovery, including unlimited ERC-20 approvals, NFT
  operator approvals, known-drainer spenders, and revoke recommendations.
  Bounded ERC-20 allowance probes and NFT operator-approval probes are
  implemented for operator-supplied spender/operator addresses. Bounded Permit2
  allowance probes are implemented for operator-supplied spenders and either
  request-supplied Permit2 contracts, a per-chain registry `permit2_address`
  override, or the canonical Permit2 contract fallback. A local
  operator-managed risk catalog is implemented for spender/operator labels and
  trusted, low, medium, high, or critical risk overrides. Reviewable
  consolidation-plan revoke steps are implemented for discovered ERC-20,
  Permit2, and NFT operator approvals. Generated consolidation plans are
  single-chain; all-chain inventory produces separate per-chain plans, and
  operators can request a specific `chain_id` when generating a plan. Revoke
  and sweep transaction calldata can be preflighted and exported after
  explicit approval and successful simulation. External spender registries,
  expiration-aware Permit2 scoring, dynamic fee policy, and direct queue
  execution remain future work.
- Dormant-wallet classification using derived last activity, current value,
  token/NFT/DeFi exposure, gas availability, and whether the private key or
  signing path is actually available. Inventory addresses now carry a derived
  `last_activity_block` from the max block across observed transfer logs and
  ERC-5564 stealth announcement scans; scan-progress cursors never count as
  activity. Dormancy is classified against each chain-registry entry's
  `dormancy_block_window` (default `1,000,000` blocks) instead of transaction
  count alone, and dormant findings carry block evidence. Valuation remains
  future work.

## Local Indexing Architecture

The daemon should add a local chain index layer rather than expanding the
current direct RPC helper into many route-local checks.

Suggested components:

- `chain_profiles`: network metadata, RPC providers, explorer/indexer endpoints,
  native asset metadata, block finality windows, gas-token rules, and chain
  capabilities. The current 1.0 slice persists schema-versioned chain profiles
  with `chain_id`, native asset metadata, `finality_blocks`, optional
  `permit2_address`, and built-in/custom provenance.
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

The first execution handoff boundary is now explicit export, not signing:
approved, simulated, unblocked plan steps can be exported as local call
manifests grouped by source wallet, chain, and provider. When an operator
supplies a matching Safe address, Sigillum can also emit a Safe Transaction
Builder-compatible batch. Suspicious, blocked, unsimulated, unapproved,
watch-only, and source-mismatched steps are skipped with reasons.

Treasury policy guardrails are implemented on top of this model: an
operator-managed local policy (destination allowlist plus per-step and
per-plan native value caps) is enforced at both plan generation and approval
time. Sweep steps routed to non-allowlisted destinations and native sweeps
above the step cap are blocked with explicit blockers, plans whose
non-blocked native total exceeds the plan cap carry plan-level
`policy_violations`, and approval re-evaluates the current policy so an
allowlist change between generation and approval still blocks. The treasury
console, the policy editor, and locally derived purpose-labeled receive
allocations (with rotation) are exposed through `/api/treasury/*` routes,
the CLI `sigillum api treasury` namespace, and the operator UI treasury
card.

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

The first implementation scans bounded ERC-721 and ERC-1155 transfer-log ranges
and records only ERC-721 token IDs whose `ownerOf` result matches the scanned
address or ERC-1155 token IDs with a positive `balanceOf(address,id)` result.

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
- approve plans and export bounded execution manifests
- execute bounded queue jobs where a Sigillum signing path already exists
- export machine-readable reports

## Phasing

1. Inventory schema and read-only EVM discovery.
2. Seed/xpub derivation scanning with gap limits and historical address
   activity.
3. ERC-20 and native multi-L2 discovery. Bounded ERC-20 transfer-log discovery
   is the first implemented slice.
4. NFT and allowance discovery. Operator-bounded ERC-20 allowance probing,
   Permit2 allowance probing, NFT operator-approval probing, a local risk
   catalog, and reviewable approval revoke plan steps are the first
   approval-management slices, and bounded ERC-721/ERC-1155 transfer-log
   discovery with owner/balance confirmation is the first NFT inventory slice.
   The opt-in NFT metadata fetch pipeline and extended local spam heuristics are
   now implemented on top of that transfer-log slice.
5. Consolidation preflight. Provider-backed `eth_call` simulation is
   implemented for ERC-20 `approve(spender, 0)` revokes and NFT
   `setApprovalForAll(operator, false)` revokes. Permit2 allowance probes now
   retain the Permit2 contract address so `approve(token, spender, 0, 0)`
   revokes can be simulated against the correct protocol contract. Native and
   ERC-20 sweep plan steps now also build provider-backed preflight calls before
   they become executable. Native sweep simulation reserves gas using the
   provider profile's max-fee policy and gas limit, records the resulting
   spendable amount, and blocks plans that cannot pay gas. ERC-20 sweeps and
   approval revokes also verify inventoried native gas against the provider fee
   policy before simulation can pass. ERC-721 and ERC-1155 NFT sweep plan steps
   now build standard `safeTransferFrom` calls, require explicit token IDs, and
   verify enough inventoried native gas against a conservative NFT gas floor.
5a. Wallet archaeology labels. Discovery now classifies addresses for signer
    availability, watch-only status, gas availability, token/NFT/protocol
    value, stranded value, approval exposure, and dormant-candidate state, and
    the local risk engine emits review findings for watch-only value, stranded
    value, and dormant funded addresses.
5b. Consolidation execution handoff. Approved, simulated, unblocked plan steps
    can be exported as call manifests or matching-Safe Transaction Builder
    batches. This creates auditable execution evidence for native/ERC-20/NFT
    sweeps and approval revokes while keeping direct signing and queue
    execution disabled until signer-specific policy is implemented.
6. DeFi position adapters. The first implemented slice records
   operator-configured ERC-20 receipt/share token probes as protocol holdings.
   Protocol-specific exit adapters, reward accounting, lockup metadata, and
   valuation remain future work.
7. Airdrop/reward discovery with strict claim risk gates. The first trusted
   candidate-ingestion, claim-contract risk-finding, and standard Merkle claim
   simulation slices are implemented; verified source adapters, richer external
   risk feeds, and explicit claim execution adapters remain future work.
8. Consolidation planner with broader dry-run simulation for dynamic fee
   estimation, gas top-ups, exits, claims, swaps, and treasury routing.
9. Controlled execution for native/ERC-20 sweeps, gas top-ups, NFT transfers,
   DeFi exits, claims, swaps, and treasury routing.
10. Non-EVM chain families, starting with Bitcoin/UTXO and only then Solana,
   Tron, and Cosmos-style networks.

Completion means a recovered or imported wallet can move from "unknown" to
"inventoried" to "reviewed plan" to "executed or intentionally ignored" without
leaving the local Sigillum control plane.
