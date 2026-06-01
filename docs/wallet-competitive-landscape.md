# Wallet Competitive Landscape And World-Class Strategy

Reviewed: 2026-05-31

This note evaluates Sigillum against adjacent wallet, portfolio, security, and
operations tools. It is meant to keep roadmap work grounded in the current
wallet market while preserving Sigillum's local-first design.

## Research Refresh: 2026-05-31

- MetaMask Portfolio remains the mainstream reference for multichain account
  portfolio views, watched accounts, tokens, NFTs, staking, and related
  dashboard workflows:
  <https://support.metamask.io/manage-crypto/portfolio/getting-started-with-the-metamask-portfolio-dashboard/>
- MetaMask's NFT surface emphasizes autodetection, metadata, pricing data, and
  a dedicated NFT tab across supported networks:
  <https://support.metamask.io/manage-crypto/nfts/nft-tokens-in-your-metamask-wallet/>
- Rabby remains the EVM power-user reference for pre-signing simulation and
  risk review:
  <https://support.rabby.io/hc/en-us/articles/11495560464143-Why-do-I-see-Simulation-Failed-error>
- Revoke.cash remains the focused reference for token approval hygiene,
  Permit-style approval education, and revocation workflows:
  <https://beta.revoke.cash/learn/approvals>
- Rotki remains the closest self-hosted/privacy reference for local portfolio
  tracking, snapshots, exchange/blockchain/manual balances, DeFi liquidity
  pools, and optional NFT balance views:
  <https://docs.rotki.com/usage-guides/portfolio/dashboard.html>
- Phantom is a useful security reference for NFT spam handling because it hides
  suspicious collectibles and warns users not to interact with unknown NFTs:
  <https://help.phantom.com/hc/en-us/articles/12983715032083-How-to-hide-or-report-an-NFT-in-Phantom>

## Product Thesis

Sigillum should not compete as another browser wallet. The durable opportunity
is a self-hosted wallet operations workstation:

1. Import or reference wallets, seeds, xpubs, stealth profiles, and managed
   accounts.
2. Discover every meaningful holding and approval with provenance and freshness.
3. Classify whether each asset is recoverable, stranded, suspicious,
   watch-only, or ready to move.
4. Generate a reviewable consolidation graph before any transfer, claim,
   revoke, swap, or DeFi exit.
5. Execute only policy-approved steps through the local daemon, vault, audit
   log, and queue.

This positioning lets consumer wallets remain the dapp front door while
Sigillum becomes the operator console for wallet archaeology, recovery, risk
review, and consolidation.

## Landscape Map

### Consumer EVM And Multichain Wallets

Representative tools: MetaMask, Rabby, Phantom, Rainbow, Zerion, Coinbase
Wallet.

Strengths:

- Browser/mobile UX, WalletConnect/dapp connectivity, swaps, bridges, staking,
  and NFT displays.
- MetaMask Portfolio is explicitly an asset dashboard for accounts, tokens,
  NFTs, transactions, swaps, bridges, staking, and smart transactions across
  supported networks.
- Rabby differentiates on pre-transaction simulation, risk scanning, approvals,
  and DeFi-oriented EVM UX.
- Phantom demonstrates how strong mobile/browser UX can make multichain assets,
  NFTs, token swaps, and staking feel routine to non-operators.

Sigillum implication: do not chase extension parity first. Borrow the best
review patterns, but prioritize discovery depth, provenance, policy, and local
execution evidence.

### Portfolio And Indexing Products

Representative tools: Rotki, Zerion, Zapper, DeBank.

Strengths:

- Broad portfolio discovery across wallets, chains, DeFi, NFTs, historical
  transactions, prices, and tax/accounting style views.
- Rotki is the closest philosophical neighbor because it emphasizes local,
  open-source portfolio tracking and privacy-preserving data ownership.
- Hosted products tend to win on breadth, latency, pricing feeds, and protocol
  adapters because they operate large shared indexers.

Sigillum implication: the world-class version needs a local indexing model with
optional external adapters and clear provenance. It should not silently become a
hosted portfolio service.

### Approval, Risk, And Simulation Tools

Representative tools: Revoke.cash, Rabby, Wallet Guard, Pocket Universe.

Strengths:

- Approval inventory and revocation for ERC-20, NFT operator approvals, and
  Permit2-style allowances.
- Human-readable risk warnings before signing.
- Simulation or preview of token movements before a transaction is submitted.

Sigillum implication: risk review and simulation are core wallet-operations
features, not add-ons. Consolidation should fail closed when risk, spender,
claim contract, signer, or simulation evidence is missing.

### Institutional And Policy Wallets

Representative tools: Safe, Fireblocks, Ledger Enterprise, Coinbase Prime.

Strengths:

- Quorum, roles, policies, transaction builders, modules, smart accounts, and
  operational auditability.
- Safe is especially relevant because it treats wallet control as policy and
  transaction workflow rather than a single private-key action.

Sigillum implication: keep local single-machine custody, but adopt the policy
shape: proposal, simulation, review, approval, queue, execution, and audit.

### Hardware Suites And Bitcoin Operations Wallets

Representative tools: Ledger Live, Trezor Suite, Sparrow Wallet, Electrum,
Liana, Specter, Caravan.

Strengths:

- Hardware-backed signing, account discovery, PSBT workflows, xpub/descriptors,
  coin control, multisig, timelocked recovery, and fee policy.
- Sparrow and Electrum are especially strong references for Bitcoin operator
  UX: explicit UTXOs, PSBTs, hardware signing, and coin selection.
- Liana shows a recovery-oriented wallet can make timelocks and inheritance
  part of the primary product model.

Sigillum implication: Bitcoin support should be a first-class UTXO subsystem,
not an EVM-shaped bolt-on. Descriptors, xpub gap scanning, PSBT construction,
coin control, and hardware signing must have their own model.

### Privacy And Stealth Address Systems

Representative tools and standards: ERC-5564, Umbra, Fluidkey, Railgun.

Strengths:

- Private receive addresses, announcement logs, stealth meta-addresses, and
  public discoverability without publishing a stable receive address.
- ERC-5564 standardizes stealth meta-address handling and announcement events,
  which gives Sigillum a real interoperability target.

Sigillum implication: stealth receive support is already a real differentiator,
and bounded ERC-5564 announcement-log scanning now gives it a recovery path for
unregistered one-time accounts. The next jump is richer stealth inventory and
safe consolidation from those accounts.

## Capability Matrix

| Capability | Sigillum Today | Market Reference | World-Class Sigillum Target |
| --- | --- | --- | --- |
| Local self-hosting | Strong local daemon, vault, UI, client, CLI, audit, snapshots | Rotki for local portfolio ownership | Keep all wallet operations local by default; external providers are opt-in and provenance-tagged |
| Consumer dapp UX | Not a browser wallet | MetaMask, Rabby, Phantom, Rainbow | Defer extension parity; add WalletConnect or hardware/Safe flows only when needed for simulation or signing |
| ERC-5564 stealth receiving | Meta-address export, one-time deposit derivation, bounded announcement-log scanning, local checks, announcement payloads, ETH/ERC-20 signing and sending | ERC-5564, Umbra, Fluidkey | Add stealth-aware consolidation, richer inventory, and reviewed recovery workflows from one-time accounts |
| Seed/xpub discovery | Initial EVM scan foundation for seed/xpub profiles with gap-limit style scan over configured providers | MetaMask/Ledger/Trezor account discovery, Bitcoin xpub wallets | Scan common Ethereum derivation paths, historical receive addresses, dormant wallets, and configured L1/L2 chains |
| Native and token inventory | Native balances, manually supplied ERC-20 probes, and bounded ERC-20 transfer-log discovery | Portfolio/indexing tools | Expand ERC-20 discovery with registries, indexers, allowlists, and positive balance evidence |
| NFTs | Bounded ERC-721 and ERC-1155 transfer discovery with current-owner/balance confirmation and token IDs in inventory/plans | MetaMask Portfolio, Phantom, Rotki, Zerion, Zapper | Full ERC-1155 batch/history coverage, metadata cache, spam flags, unknown collection policy, reviewed NFT sweeps |
| DeFi positions | Not implemented | Rotki, Zapper, DeBank, Zerion, Rabby | Protocol adapters for lending, staking, LPs, vaults, bridges, vesting, streams, and rewards |
| Airdrops and rewards | Not implemented | Portfolio tools, protocol claim portals | Trusted-source candidate discovery, claim-contract verification, simulation, explicit review only |
| Approvals and revokes | Bounded ERC-20 allowance probes, Permit2 allowance probes, and NFT operator-approval probes for operator-supplied addresses, persisted as approval holdings with risk findings | Revoke.cash, Rabby | Spender registries, expiration-aware Permit2 scoring, risky spender labels, simulation, and revoke recommendations |
| Transaction simulation | Not implemented as a required planning step | Rabby, Safe tools, Tenderly-style infrastructure | Every plan step carries simulation evidence or remains blocked |
| Consolidation planning | Foundation exists: inventory, risk findings, dry-run plans, and approval state | No single consumer wallet owns this end to end | Execution graph for gas top-ups, revokes, claims, DeFi exits, swaps, sweeps, and treasury routing |
| Bitcoin/UTXO | Not implemented | Sparrow, Electrum, Liana, Specter, Caravan | Descriptor/xpub scan, UTXO inventory, PSBT construction, coin control, fee policy, hardware signing |
| Policy and audit | Strong local audit/queue posture for current operations | Safe, institutional custody tools | Policy gates across all asset classes with exportable discovery and execution evidence |

## Sigillum Innovations Already Present

- Local vault plus local daemon is a better base for wallet operations than a
  browser extension when the operator needs recovery, audit, and policy.
- ERC-5564 receive payloads and local stealth-deposit custody give Sigillum a
  privacy-first wallet lane that most portfolio tools do not own.
- The queue and maintenance loop create an execution substrate for planned work
  instead of forcing every action to be a one-off UI click.
- The inventory, risk, and consolidation-plan foundation already points toward
  a planner-based model rather than a "sweep everything" button.
- The project can combine visibility and signing capability labels in one place:
  signer-available, watch-only, stranded, suspicious, or blocked.

## Where Sigillum Can Innovate Further

### Wallet Archaeology

Discover old value from imported seeds, xpubs, stealth profiles, hardware
exports, and project accounts. Track each discovered address with derivation
path, source wallet, provider evidence, last activity, signer availability, and
confidence.

### Provenance-First Inventory

Every asset, NFT, DeFi position, reward, approval, and price should explain how
it was found: local RPC, transfer log, trusted indexer, registry, protocol API,
operator import, or cached prior evidence. Stale or single-source findings
should look different from fresh, corroborated findings.

### Claim Firewall

Airdrops are high-risk. Sigillum can stand out by treating claims as hostile
until proven otherwise: trusted source, verified contract, exact calldata,
simulation, no surprise approvals, no blind typed-data signing, and operator
review every time.

### Consolidation Graph

The best version is not "send all." It is a graph that explains dependencies:
gas funding before sweep, revoke before consolidation, claim before swap,
unstake before withdrawal, bridge finalization before routing, and blocked
branches for uneconomic dust, suspicious assets, watch-only wallets, or failed
simulation.

### Privacy-Preserving Discovery

Use local-first scanning where possible, support provider partitioning and rate
limits, avoid sending a full wallet graph to one hosted API unless explicitly
configured, and mark the privacy cost of each discovery adapter.

### Signability Ladder

Classify each source as watch-only, signer available in Sigillum vault,
hardware-required, multisig/quorum-required, external-wallet-required, or
missing signer. This is more useful than a simple balance table because it tells
the operator which value can actually be recovered.

## Execution Priorities

1. Harden the local inventory substrate: timestamps, stale/fresh transitions,
   resumable jobs, provider evidence, and chain-profile capability flags.
2. Finish EVM discovery breadth: common derivation paths, L1/L2 native balances,
   token discovery, activity history, and dormant wallet labels.
3. Extend approvals, NFTs, DeFi, and airdrops as separate adapter families with
   spam/risk provenance; ERC-721 and ERC-1155 transfer discovery are now the
   first NFT slices, while metadata and spam classification remain open.
4. Require simulation evidence for any consolidation step that could execute.
5. Expand the planner into real multi-step graphs for gas top-ups, revokes,
   claims, exits, unwraps, swaps, sweeps, and treasury routing.
6. Add Bitcoin as a distinct UTXO subsystem with descriptors, PSBTs, coin
   control, fee policy, and hardware signing.
7. Add optional WalletConnect, Safe, and hardware-wallet interoperability only
   where it strengthens simulation, policy review, or signing.

## Non-Goals

- No hosted Sigillum backend as a hidden dependency.
- No blind signing, blind claims, or automatic typed-data signing.
- No automatic execution for unknown claims, suspicious assets, failed
  simulation, watch-only wallets, or missing-gas conditions.
- No browser-extension clone as the near-term product center.
- No silent use of external indexers or pricing sources without explicit
  operator configuration and source labeling.

## Research Sources

- [MetaMask Portfolio dashboard](https://support.metamask.io/manage-crypto/portfolio/getting-started-with-the-metamask-portfolio-dashboard/)
- [MetaMask NFT tokens](https://support.metamask.io/manage-crypto/nfts/nft-tokens-in-your-metamask-wallet/)
- [Rabby Wallet](https://rabby.io/)
- [Rabby transaction simulation guide](https://support.rabby.io/hc/en-us/articles/11495560464143-Why-do-I-see-Simulation-Failed-error)
- [Safe Wallet](https://safe.global/wallet)
- [Rotki documentation](https://docs.rotki.com/)
- [Rotki portfolio dashboard](https://docs.rotki.com/usage-guides/portfolio/dashboard.html)
- [Revoke.cash learn pages](https://revoke.cash/learn)
- [Sparrow Wallet features](https://sparrowwallet.com/features/)
- [Electrum features](https://electrum.org/)
- [Ledger Live](https://www.ledger.com/ledger-live)
- [Trezor Suite coin control](https://trezor.io/guides/trezor-suite/trezor-suite-desktop/coin-control-in-trezor-suite)
- [Phantom multichain wallet announcement](https://www.phantom.com/learn/blog/introducing-phantom-multichain)
- [Phantom hidden collectibles](https://help.phantom.com/hc/en-us/articles/12983715032083-How-to-hide-or-report-an-NFT-in-Phantom)
- [EIP-5564: Stealth Addresses](https://eips.ethereum.org/EIPS/eip-5564)
- [Umbra Protocol](https://app.umbra.cash/)
- [Fluidkey](https://fluidkey.com/)
- [Liana wallet](https://wizardsardine.com/liana/)
