# Changelog

Notable changes to this project are documented in this file. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
from 1.0.0 onward, per the stability policy in `docs/stability.md`.

## [Unreleased]

## [1.0.0] - 2026-07-XX

### Added

- **Vault and unlock** — Two-tier local vault storage with Tier 1 plaintext API
  keys for local automation and Tier 2 AES-256-GCM encrypted secrets, Argon2id
  passphrase-derived key wrapping, FIDO2 hardware-key unlock with Shamir shard
  recovery, per-compartment keyspaces with compartment switching,
  passphrase-encrypted snapshot export/restore, and journaled destructive flows
  with tested crash-point recovery.
- **Local daemon and operator console** — Local Axum daemon with bearer
  session-token auth over loopback HTTP, an embedded web operator console
  covering first-time setup, lock/unlock, secret management, FIDO2 key
  management, snapshots, local audit feed, and diagnostics, plus transit-style
  encrypt/decrypt/HMAC endpoints, atomic sidecar-backed persistence with
  automatic backup restore and corrupt-file quarantine, restart recovery with
  telemetry, and categorized maintenance summaries reporting failures by cause.
- **Ethereum stealth custody** — ERC-5564-style stealth meta-address export,
  one-time deposit-address derivation and receiving, bounded announcement-log
  scanning, provider-backed deposit monitoring, local EIP-1559 native and
  ERC-20 transfer signing with optional broadcast, and sweep orchestration
  through a persistent queue.
- **Discovery, inventory, and risk** — Multi-chain EVM inventory scans from one
  provider profile or all configured chains in a single request, resumable
  block-range cursors for ERC-20/ERC-721/ERC-1155 transfer-log discovery,
  ERC-1155 TransferSingle and TransferBatch decoding, operator-imported local
  token registries with provenance-recorded balance probes and no external
  feeds, opt-in per-collection NFT metadata fetching with provenance plus local
  spam heuristics that bucket suspicious assets without auto-hiding them,
  ERC-20/Permit2/NFT operator allowance discovery, last-activity block
  derivation with per-chain dormancy windows, a watch book, and a local
  operator-managed risk catalog.
- **Consolidation planning and policy-gated execution** — Plan generation for
  native/ERC-20/NFT sweeps, approval revokes for ERC-20, Permit2, and NFT
  operators, DeFi exits, Merkle claims, and sponsor-funded `fund_gas` gas
  top-ups; `eth_call` preflight simulation with a recorded fee basis using
  static fees or live EIP-1559 estimates and a policy-configurable freshness
  window; step dependency ordering; export as call manifests or Safe
  Transaction Builder batches; execution OFF by default behind fail-closed
  policy gates including `allow_plan_execution`, per-family sweep/revoke/exit/
  claim/top-up gates, an `execution_paused` kill switch, and gate-flip audit
  events carrying session fingerprints; enqueue with full server-side
  re-validation and typed confirmation naming the step count and total value;
  seed-wallet signing that re-verifies a simulation-evidence hash before
  touching key material and zeroizes derived keys; broadcast-time nonce
  management, receipt confirmation to per-chain finality depth with a terminal
  `confirmed` state, classified failures where provider errors are retried,
  reverts are parked for the operator and never auto-retried, and underpriced
  transactions get one bounded fee bump; crash-safe receipt-polling resume; and
  linkage enforcement at execution enqueue at parity with plan generation and
  approval.
- **Treasury, receiving, and linkage policy** — Purpose-labeled receive-address
  allocations with rotation, first-class counterparties, a treasury console with
  cross-wallet value/risk/plan roll-ups, treasury policy guardrails covering a
  destination allowlist, per-step and per-plan native caps, required simulation,
  and `block_cross_party_linkage` single-hop cross-party linkage blocking
  enforced at plan generation, approval, and enqueue, plus policy-driven
  hot-wallet floor/target routing and maintenance-cycle hot-wallet overflow/
  refill treasury automation behind the default-off `allow_treasury_automation`
  opt-in with floor <= target <= overflow hysteresis and distinct
  generated-versus-enqueued reporting.
- **Chain registry** — Schema-versioned registry with built-in entries for
  Ethereum, Base, Arbitrum One, OP Mainnet, and Polygon PoS plus
  operator-defined custom chains, per-chain finality depth, dormancy block
  window, Permit2 address override, and provider self-check warnings for
  unregistered chains.
- **DeFi exit adapters and Merkle claims** — Bounded exit-adapter set covering
  Aave v3 withdraw, generic ERC-4626 redeem, Uniswap v2 LP removeLiquidity with
  dependency-ordered approve then removeLiquidity, plan-time reserve-derived
  minimum amounts, and per-chain operator-supplied router addresses, and Lido
  wstETH unwrap; positions matching no adapter remain review-only.
  `merkle-distributor-v1` claim execution is behind the `allow_claim_execution`
  opt-in and gated on simulation, risk-catalog review, and explicit step
  approval; claim failures park for the operator and are never auto-retried.
- **Gateway sidecar** — Local-sidecar payment preview surface with project API
  keys, payment intent creation, and webhook delivery that re-resolves and pins
  HTTPS targets at send time, positioned beside the local daemon rather than as
  an internet-facing boundary.
- **Desktop app** — Tauri v2 macOS shell that runs the daemon in-process on a
  fresh loopback port, shares the `~/.sigillum` data directory, keeps a single
  focused instance, shows a tray with live lock state and "Lock now", locks and
  hides to tray on close, and locks before quit so loaded keys are zeroized;
  `.app`/`.dmg` bundling with a generated icon set, ad-hoc signing by default
  with env-gated full signing/notarization; macOS is the supported desktop
  platform, while Linux desktop is compile-only.
- **CLI** — Setup flows, `sigillum doctor` host preflight, snapshot commands, a
  daemon launcher, and JSON operator commands under `sigillum api` covering
  sessions, compartment listing, provider/wallet profiles, deposits, inventory
  scans and discovery controls, token registries, chains, risk catalog, plans
  including the gated enqueue commands, receiving, treasury policy, queue
  inspection and pause/resume, maintenance runs, transit helpers, read-only EVM
  queries, and wallet xpub/stealth export and derive helpers; no sign, send, or
  broadcast commands by design.
- **Release engineering** — Single release gate at `./scripts/check-release.sh`
  running cargo metadata and architecture guardrails, daemon UI
  install/typecheck/tests/build with generated-asset freshness checks, Rust
  fmt/tests/clippy, adversarial property-based API fuzzing, a real local-daemon
  runtime smoke with vault write/read canaries, browser smoke, a desktop bundle
  check, `cargo audit`, `cargo deny`, and whitespace checks; CI runs the full
  gate on Ubuntu and macOS with a nightly deep-fuzz schedule.

[Unreleased]: https://github.com/caelator/sigillum/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/caelator/sigillum/releases/tag/v1.0.0
