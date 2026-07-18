/**
 * destinations/portfolio.ts — the Portfolio destination controller
 * (plan task 4.3: real tables, scan stepper, risk/token surfaces).
 *
 * Owns `#/portfolio[/scan|/risk|/tokens]` and renders into the legacy
 * `#inventoryCard` container (the other portfolio-section cards stay legacy
 * until their own migrations). Everything goes through the strict core:
 * store slices for live updates (operations/status/resync), the typed api
 * client where it has methods, and thin local wrappers around
 * `requestWithSession` for the endpoints it does not (chains, risk,
 * token registry, NFT metadata, scan/discovery mutations). Failures branch
 * on the daemon's structured error `code` (vault_locked → unlock guidance,
 * validation_failed → field-level messages).
 *
 * Views:
 * - `#/portfolio`        holdings: addresses + holdings tables (1.5
 *                        filter/sort/pagination), scans section with live
 *                        operation progress from the store.
 * - `#/portfolio/scan`   the scan stepper: wallets → providers (with the
 *                        3.1 partition_providers option) → launch (run_async
 *                        by default) → live progress/results. The full
 *                        legacy scan options live behind "Advanced options"
 *                        and produce the exact legacy request DTO.
 * - `#/portfolio/risk`   risk findings (severity tiers, 1.5 params;
 *                        common_gas_funder findings in plain language) and
 *                        the local risk catalog.
 * - `#/portfolio/tokens` token registry (local import only) and the NFT
 *                        metadata opt-in surfaces.
 *
 * The scan request can only target one wallet profile / one provider per
 * request (or all of each). The stepper NEVER silently widens scope: a
 * partial selection fans out into one background scan per selected wallet /
 * provider, capped and explained. An all-selected launch stays ONE scan,
 * which is also the only configuration where provider partitioning (3.1)
 * applies.
 */

import type {
  ChainProfile,
  NftMetadataCacheEntry,
  NftMetadataCollectionOptIn,
  Operation,
  PaginationInfo,
  RiskCatalogEntry,
  RiskFinding,
  StatusResponse,
  TokenRegistryList,
  WalletAssetHolding,
  WalletDiscoveryJob,
  WalletInventoryAddress,
  WalletInventoryListQuery,
} from "../contracts";
import { requestWithSession } from "../api/session";
import { ApiError, apiFailure, type ApiFailure } from "../core/api";
import { el, renderList } from "../core/dom";
import type { CoreRuntime } from "../core/live";
import {
  formatHash,
  type DestinationController,
  type Route,
} from "../core/router";
import type { Unsubscribe } from "../core/store";
import { confirmDangerDialog, informDialog } from "../render/confirm";
import {
  chainLabel,
  formatTimestamp,
  formatTokenAmount,
} from "../render/format";
import { pillClass } from "../render/html";

// ── Wire shapes the typed client does not cover ─────────────────────
// These mirror sigillum-api response/request contracts exactly. Shared
// response records (including RiskFinding) live in contracts.ts so every
// destination consumes one wire shape.

export interface ProviderPartitionObservation {
  provider_profile: string;
  chain_id: number;
  addresses_observed: number;
}

/** Discovery job as the daemon actually sends it (3.1 partition fields). */
export type DiscoveryJobRecord = WalletDiscoveryJob & {
  partition_providers?: boolean | null;
  provider_partition_observations?: ProviderPartitionObservation[];
};

interface RiskFindingListResponseWire {
  findings?: RiskFinding[];
  pagination?: PaginationInfo | null;
}

interface RiskCatalogListResponseWire {
  entries?: RiskCatalogEntry[];
}

interface TokenRegistryListResponseWire {
  lists?: TokenRegistryList[];
}

interface NftOptInListResponseWire {
  opt_ins?: NftMetadataCollectionOptIn[];
  ipfs_gateway_url?: string | null;
}

interface NftFetchResponseWire {
  fetched?: number;
  skipped?: {
    chain_id?: number;
    contract_address?: string;
    token_id_hex?: string | null;
    reason?: string;
  }[];
}

interface ChainsResponseWire {
  profiles?: ChainProfile[];
}

/** EVM provider profile (response/profiles.rs). */
export interface EvmProviderProfileRecord {
  name: string;
  chain_id: number;
  fee_estimation_enabled?: boolean;
}

interface EvmProviderProfilesResponseWire {
  profiles?: EvmProviderProfileRecord[];
}

interface WalletProfileNameRecord {
  name: string;
  chain_id?: number | null;
}

interface WalletProfilesResponseWire {
  profiles?: WalletProfileNameRecord[];
}

/** The exact legacy scan request DTO (request/inventory.rs). */
export interface EvmScanRequest {
  wallet_family?: string | null;
  wallet_profile?: string | null;
  provider_profile?: string | null;
  all_configured_chains?: boolean;
  derivation_pattern?: string | null;
  account_limit?: number | null;
  watch_addresses?: { address: string; label?: string | null }[];
  include_watch_book?: boolean;
  gap_limit?: number | null;
  max_index?: number | null;
  resume_from_latest_checkpoint?: boolean;
  run_async?: boolean;
  partition_providers?: boolean;
  token_addresses?: string[];
  block_tag?: string;
  probe_token_registry?: boolean;
  discover_erc20_transfers?: boolean;
  token_discovery_from_block?: string | null;
  token_discovery_to_block?: string | null;
  token_discovery_limit?: number | null;
  discover_erc20_allowances?: boolean;
  allowance_spender_addresses?: string[];
  allowance_discovery_limit?: number | null;
  discover_permit2_allowances?: boolean;
  permit2_contract_addresses?: string[];
  permit2_spender_addresses?: string[];
  permit2_allowance_limit?: number | null;
  discover_erc721_transfers?: boolean;
  discover_erc1155_transfers?: boolean;
  discover_nft_operator_approvals?: boolean;
  nft_operator_addresses?: string[];
  nft_operator_approval_limit?: number | null;
  nft_discovery_from_block?: string | null;
  nft_discovery_to_block?: string | null;
  nft_discovery_limit?: number | null;
}

interface EvmScanResponseWire {
  job?: DiscoveryJobRecord;
  addresses?: WalletInventoryAddress[];
  holdings?: WalletAssetHolding[];
  operation?: Operation;
}

interface DiscoveryJobMutationWire {
  status?: string;
  job?: DiscoveryJobRecord;
  operation?: Operation;
}

interface MutationStatusWire {
  status?: string;
}

// ── Thin request helper (same semantics as core/api.ts request<T>) ──

async function daemonRequest<T>(
  method: "GET" | "POST" | "DELETE",
  path: string,
  body?: unknown,
): Promise<T> {
  let payload: unknown;
  try {
    payload = await requestWithSession(method, path, body);
  } catch (error) {
    throw new ApiError({
      code: "unavailable",
      error: error instanceof Error ? error.message : String(error),
    });
  }
  const envelope = payload as {
    code?: string;
    error?: string;
    action?: string;
    fields?: { field: string; message: string }[];
  } | null;
  if (envelope && envelope.error != null) {
    throw new ApiError({
      code: envelope.code ?? "unknown",
      error: envelope.error,
      action: envelope.action,
      fields: envelope.fields,
    });
  }
  return payload as T;
}

type QueryValue = string | number | boolean | null | undefined;

function buildQuery(params: Record<string, QueryValue>): string {
  const parts: string[] = [];
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null) continue;
    parts.push(
      `${encodeURIComponent(key)}=${encodeURIComponent(String(value))}`,
    );
  }
  return parts.length ? `?${parts.join("&")}` : "";
}

// ── Pure helpers (exported for the smoke tests) ─────────────────────

/** "3h ago" style relative time; unix seconds in, plain words out. */
export function relativeTime(
  unix: number | null | undefined,
  nowSecs: number,
): string {
  if (!unix || !Number.isFinite(unix)) return "never";
  const delta = nowSecs - unix;
  if (delta < 0) return "just now";
  if (delta < 45) return "just now";
  const minutes = Math.floor(delta / 60);
  if (minutes <= 1) return "a minute ago";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return hours <= 1 ? "an hour ago" : `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return days <= 1 ? "a day ago" : `${days}d ago`;
  return new Date(unix * 1000).toLocaleDateString();
}

/** Middle-truncate an address/hash: "0x71C7…976F". */
export function middleTruncate(
  value: string | null | undefined,
  head = 8,
  tail = 6,
): string {
  const text = (value || "").trim();
  if (!text) return "-";
  if (text.length <= head + tail + 1) return text;
  return `${text.slice(0, head)}…${text.slice(text.length - tail)}`;
}

export interface ScanWalletOption {
  /** Wallet family: "eth-seed" or "eth-xpub". */
  family: string;
  /** Profile name. */
  profile: string;
}

export interface ScanProviderOption {
  name: string;
  chainId: number | null;
}

export interface ScanRequestSeed {
  wallet_family?: string;
  wallet_profile?: string;
  provider_profile?: string;
  include_watch_book?: boolean;
}

export interface LaunchPlanInput {
  /** Every known wallet profile (seed + xpub). */
  wallets: ScanWalletOption[];
  /** Selected wallet keys ("family/profile"). */
  selectedWallets: ReadonlySet<string>;
  /** Every known EVM provider profile. */
  providers: ScanProviderOption[];
  /** Selected provider names. */
  selectedProviders: ReadonlySet<string>;
  /** Include the saved watch address book. */
  includeWatchBook: boolean;
  /** Safety cap on fanned-out background scans (default 8). */
  maxScans?: number;
}

export type ScanLaunchPlan =
  | {
      ok: true;
      scans: ScanRequestSeed[];
      /** True when the launch is one unfiltered all-providers scan. */
      allProviders: boolean;
      /** True when the launch is one unfiltered all-wallets scan. */
      allWallets: boolean;
    }
  | { ok: false; reason: string };

function walletKey(wallet: ScanWalletOption): string {
  return `${wallet.family}/${wallet.profile}`;
}

/** Cap on fanned-out scans so a wide checkbox selection cannot flood the
 * daemon (or the providers) with dozens of background operations. */
export const MAX_LAUNCH_SCANS = 8;

/**
 * Map the stepper's checkbox selections onto the scan API's
 * one-or-all wallet/provider filters. Partial selections fan out into one
 * scan per selected wallet/provider — the scope NEVER silently widens
 * beyond what the operator checked. Provider partitioning stays possible
 * only on the single all-providers launch.
 */
export function buildScanLaunchPlan(input: LaunchPlanInput): ScanLaunchPlan {
  const maxScans = input.maxScans ?? MAX_LAUNCH_SCANS;

  if (!input.providers.length) {
    return {
      ok: false,
      reason:
        "No EVM provider profiles are configured yet. Save a provider profile first — a scan needs at least one chain endpoint.",
    };
  }
  const knownProviderNames = input.providers.map((provider) => provider.name);
  const chosenProviders = knownProviderNames.filter((name) =>
    input.selectedProviders.has(name),
  );
  if (!chosenProviders.length) {
    return { ok: false, reason: "Select at least one provider to scan with." };
  }
  const allProviders = chosenProviders.length === knownProviderNames.length;
  // One undefined entry = no provider filter (all providers, one scan).
  const providerSeeds: (string | undefined)[] = allProviders
    ? [undefined]
    : chosenProviders;

  const knownWalletKeys = input.wallets.map(walletKey);
  const chosenWallets = input.wallets.filter((wallet) =>
    input.selectedWallets.has(walletKey(wallet)),
  );
  const allWallets =
    knownWalletKeys.length > 0 && chosenWallets.length === knownWalletKeys.length;

  const walletSeeds: ScanRequestSeed[] = [];
  if (allWallets) {
    walletSeeds.push({
      ...(input.includeWatchBook ? { include_watch_book: true } : {}),
    });
  } else {
    for (const wallet of chosenWallets) {
      walletSeeds.push({
        wallet_family: wallet.family,
        wallet_profile: wallet.profile,
      });
    }
    // Watch addresses only join scans that do not filter to a seed/xpub
    // wallet (the daemon excludes them from filtered scans), so a partial
    // selection needs a dedicated watch-book scan.
    if (input.includeWatchBook) {
      walletSeeds.push({
        wallet_family: "eth-watch",
        include_watch_book: true,
      });
    }
  }
  if (!walletSeeds.length) {
    return {
      ok: false,
      reason:
        "Pick at least one wallet, or include the saved watch book so there is something to scan.",
    };
  }

  const scans: ScanRequestSeed[] = [];
  for (const walletSeed of walletSeeds) {
    for (const providerSeed of providerSeeds) {
      scans.push({
        ...walletSeed,
        ...(providerSeed ? { provider_profile: providerSeed } : {}),
      });
    }
  }
  if (scans.length > maxScans) {
    return {
      ok: false,
      reason:
        `This selection would start ${scans.length} background scans. ` +
        `Narrow it to ${maxScans} or fewer, or select every wallet and provider for one combined scan.`,
    };
  }
  return { ok: true, scans, allProviders, allWallets };
}

/** True when >1 provider serves the same chain (partitioning is offered). */
export function hasMultiProviderChain(
  providers: ScanProviderOption[],
): boolean {
  const byChain = new Map<number | null, number>();
  for (const provider of providers) {
    const count = (byChain.get(provider.chainId) ?? 0) + 1;
    if (count > 1) return true;
    byChain.set(provider.chainId, count);
  }
  return false;
}

// ── Humanization (enum → plain words; never print raw enum values) ──

function familyLabel(family: string | null | undefined): string {
  switch (family) {
    case "eth-seed":
      return "Seed wallet";
    case "eth-xpub":
      return "xpub wallet";
    case "eth-watch":
      return "Watch address";
    default:
      return (family || "unknown").replace(/_/g, " ");
  }
}

function activityLabel(state: string | null | undefined): string {
  switch (state) {
    case "funded":
      return "Funded";
    case "active":
      return "Active";
    case "empty":
      return "Empty";
    default:
      return (state || "unknown").replace(/_/g, " ");
  }
}

function operationStateLabel(state: string | null | undefined): string {
  switch (state) {
    case "running":
      return "Running";
    case "cancel_requested":
      return "Canceling";
    case "canceled":
      return "Canceled";
    case "completed":
      return "Completed";
    case "failed":
      return "Failed";
    default:
      return (state || "unknown").replace(/_/g, " ");
  }
}

function jobStatusLabel(status: string | null | undefined): string {
  switch (status) {
    case "resume_requested":
      return "Resume queued";
    case "cancel_requested":
      return "Canceling";
    default:
      return (status || "unknown").replace(/_/g, " ");
  }
}

const RISK_CATEGORY_LABELS: Record<string, string> = {
  common_gas_funder: "Shared gas funder",
  approval_exposure: "Approval exposure",
  stranded_value: "Stranded value",
  watch_only: "Watch-only address",
  dormant_candidate: "Dormant address",
};

function riskCategoryLabel(category: string | null | undefined): string {
  if (!category) return "Finding";
  return RISK_CATEGORY_LABELS[category] ?? category.replace(/_/g, " ");
}

function subjectTypeLabel(subjectType: string | null | undefined): string {
  switch (subjectType) {
    case "gas_funder":
      return "Gas funder";
    case "address":
      return "Address";
    case "spender":
      return "Spender";
    case "contract":
      return "Contract";
    default:
      return (subjectType || "subject").replace(/_/g, " ");
  }
}

/** Consequence tier for a finding severity; undefined = quiet/default row. */
function severityTier(
  riskLevel: string | null | undefined,
): "danger" | "review" | undefined {
  switch ((riskLevel || "").toLowerCase()) {
    case "critical":
    case "high":
      return "danger";
    case "medium":
      return "review";
    default:
      return undefined;
  }
}

function spamLabelText(label: string | null | undefined): string {
  if (!label) return "Unreviewed";
  return label.replace(/_/g, " ");
}

// ── Small DOM builders ──────────────────────────────────────────────

function pill(text: string, classValue?: string): HTMLSpanElement {
  return el("span", { class: `pill ${classValue ?? pillClass(text)}`, text });
}

function setHidden(node: HTMLElement, hidden: boolean): void {
  (node as { hidden?: boolean }).hidden = hidden;
  if (hidden) node.setAttribute("hidden", "");
  else node.removeAttribute("hidden");
}

function clearContainer(node: Element): void {
  for (const child of Array.from(node.childNodes)) {
    (child as ChildNode).remove();
  }
}

function skeletonRows(count: number): HTMLElement {
  const wrap = el("div", { dataset: { portfolio: "skeleton" } });
  for (let index = 0; index < count; index++) {
    wrap.appendChild(el("div", { class: "skeleton skeleton-block" }));
  }
  return wrap;
}

function sectionEmpty(
  title: string,
  body: string,
  action?: { label: string; href?: string; onClick?: () => void },
): HTMLElement {
  const wrap = el("div", { class: "section-empty" });
  wrap.appendChild(el("p", { class: "section-empty-title", text: title }));
  wrap.appendChild(el("p", { class: "section-empty-body", text: body }));
  if (action) {
    if (action.href) {
      wrap.appendChild(
        el("a", {
          class: "btn-primary section-empty-action",
          text: action.label,
          attrs: { href: action.href },
        }),
      );
    } else if (action.onClick) {
      wrap.appendChild(
        el("button", {
          class: "btn-primary section-empty-action",
          text: action.label,
          attrs: { type: "button" },
          on: { click: () => action.onClick?.() },
        }),
      );
    }
  }
  return wrap;
}

/** Raw value one click away (DESIGN.md: raw stays behind a disclosure). */
function rawDetails(raw: string | null | undefined): HTMLElement {
  const details = el("details", { class: "raw-details" });
  details.appendChild(el("summary", { text: "raw" }));
  details.appendChild(el("code", { text: String(raw ?? "").trim() || "-" }));
  return details;
}

interface AmountUnits {
  decimals: number;
  symbol: string | null;
}

/** Human amount cell content: value in tabular numerals + raw behind details. */
function amountContent(
  amountHex: string | null | undefined,
  units: AmountUnits | null,
): HTMLElement {
  const wrap = el("span", { class: "amount" });
  const human = units ? formatTokenAmount(amountHex, units.decimals) : null;
  if (human === null) {
    wrap.appendChild(
      el("span", {
        class: "text-muted",
        text: units ? "-" : "unknown units",
      }),
    );
    wrap.appendChild(rawDetails(amountHex));
    return wrap;
  }
  wrap.appendChild(
    el("span", {
      class: "nums",
      text: units.symbol ? `${human} ${units.symbol}` : human,
    }),
  );
  wrap.appendChild(rawDetails(amountHex));
  return wrap;
}

async function copyValue(
  value: string,
  label: string,
  onCopied: () => void,
): Promise<void> {
  try {
    await navigator.clipboard.writeText(value);
    onCopied();
  } catch (_) {
    void informDialog({
      title: `Copy ${label}`,
      body: "Clipboard access is unavailable in this context. Copy the value manually:",
      valueDisplay: value,
    });
  }
}

function copyButton(
  value: string,
  label: string,
  onCopied: () => void,
): HTMLElement {
  return el("button", {
    class: "btn-ghost btn-small",
    text: "Copy",
    attrs: { type: "button", "aria-label": `Copy ${label}` },
    on: {
      click: () => {
        void copyValue(value, label, onCopied);
      },
    },
  });
}

function addressCell(address: string, onCopied: () => void): HTMLElement {
  const cell = el("span", { class: "addr-cell" });
  cell.appendChild(
    el("span", {
      class: "mono",
      text: middleTruncate(address),
      attrs: { title: address },
    }),
  );
  cell.appendChild(copyButton(address, "address", onCopied));
  return cell;
}

function isTerminalOperationState(state: string | undefined): boolean {
  return state === "completed" || state === "failed" || state === "canceled";
}

const SCAN_OPERATION_KIND = "inventory_scan_evm";
const HOST_ELEMENT_ID = "inventoryCard";
const ADDRESSES_PAGE_SIZE = 25;
const FINDINGS_PAGE_SIZE = 25;

// ── Controller ──────────────────────────────────────────────────────

type LoadState = "idle" | "loading" | "ready" | "error";
type ViewName = "holdings" | "scan" | "risk" | "tokens";

type ResourceGroup = "portfolio" | "risk" | "tokens" | "profiles";

interface AddressFilters {
  chainId: number | null;
  funded: boolean | null;
  sort: "last_scanned" | "address";
  order: "asc" | "desc";
  offset: number;
  limit: number;
}

interface RiskFilters {
  severity: string | null;
  sort: "severity" | "found_at";
  order: "asc" | "desc";
  offset: number;
  limit: number;
}

interface StepperState {
  step: 1 | 2 | 3 | 4;
  selectedWallets: Set<string>;
  selectedProviders: Set<string>;
  includeWatchBook: boolean;
  partitionProviders: boolean;
  runAsync: boolean;
  launching: boolean;
  launchedOperationIds: string[];
  launchedJobIds: string[];
  syncResults: { jobId: string | null; status: string; summary: string }[];
  launchError: string | null;
  selectionTouched: boolean;
}

interface PortfolioState {
  host: HTMLElement | null;
  root: HTMLElement | null;
  bannerRegion: HTMLElement | null;
  viewRoot: HTMLElement | null;
  statusLine: HTMLElement | null;
  refs: Record<string, HTMLElement | null>;
  view: ViewName | null;
  route: Route | null;
  unsubs: Unsubscribe[];
  refetchTimer: unknown;
  locked: boolean;
  load: Record<ResourceGroup, LoadState>;
  everLoaded: Record<ResourceGroup, boolean>;
  stale: Partial<Record<ResourceGroup, string>>;
  chains: ChainProfile[];
  tokenLists: TokenRegistryList[];
  addresses: WalletInventoryAddress[];
  holdings: WalletAssetHolding[];
  jobs: DiscoveryJobRecord[];
  nftCache: NftMetadataCacheEntry[];
  addressesPagination: PaginationInfo | null;
  findings: RiskFinding[];
  findingsPagination: PaginationInfo | null;
  catalog: RiskCatalogEntry[];
  optIns: NftMetadataCollectionOptIn[];
  ipfsGateway: string;
  providers: EvmProviderProfileRecord[];
  wallets: ScanWalletOption[];
  filters: AddressFilters;
  riskFilters: RiskFilters;
  stepper: StepperState;
  opStates: Map<string, string>;
  lastAdvanced: Partial<EvmScanRequest> | null;
  /** Token guarding step-content rebuilds (focus preservation). */
  scanRenderToken: string | null;
}

function initialStepper(): StepperState {
  return {
    step: 1,
    selectedWallets: new Set(),
    selectedProviders: new Set(),
    includeWatchBook: true,
    partitionProviders: false,
    runAsync: true,
    launching: false,
    launchedOperationIds: [],
    launchedJobIds: [],
    syncResults: [],
    launchError: null,
    selectionTouched: false,
  };
}

export function createPortfolioDestination(
  runtime: CoreRuntime,
): DestinationController {
  const state: PortfolioState = {
    host: null,
    root: null,
    bannerRegion: null,
    viewRoot: null,
    statusLine: null,
    refs: {},
    view: null,
    route: null,
    unsubs: [],
    refetchTimer: null,
    locked: false,
    load: { portfolio: "idle", risk: "idle", tokens: "idle", profiles: "idle" },
    everLoaded: { portfolio: false, risk: false, tokens: false, profiles: false },
    stale: {},
    chains: [],
    tokenLists: [],
    addresses: [],
    holdings: [],
    jobs: [],
    nftCache: [],
    addressesPagination: null,
    findings: [],
    findingsPagination: null,
    catalog: [],
    optIns: [],
    ipfsGateway: "",
    providers: [],
    wallets: [],
    filters: {
      chainId: null,
      funded: null,
      sort: "last_scanned",
      order: "desc",
      offset: 0,
      limit: ADDRESSES_PAGE_SIZE,
    },
    riskFilters: {
      severity: null,
      sort: "severity",
      order: "desc",
      offset: 0,
      limit: FINDINGS_PAGE_SIZE,
    },
    stepper: initialStepper(),
    opStates: new Map(),
    lastAdvanced: null,
    scanRenderToken: null,
  };

  // ── Small shared utilities ────────────────────────────────────────

  function nowSecs(): number {
    return Math.floor(Date.now() / 1000);
  }

  function setStatus(text: string): void {
    if (state.statusLine) state.statusLine.textContent = text;
  }

  function chainName(chainId: number | string | null | undefined): string {
    return chainLabel(chainId, state.chains);
  }

  function nativeUnits(chainId: unknown): AmountUnits {
    const numericChainId = Number(chainId);
    const profile = state.chains.find(
      (chain) => chain.chain_id === numericChainId,
    );
    return {
      decimals: profile?.native_decimals ?? 18,
      symbol: profile?.native_symbol || "ETH",
    };
  }

  function tokenUnits(chainId: unknown, assetAddress: unknown): AmountUnits | null {
    const numericChainId = Number(chainId);
    const address = String(assetAddress || "").toLowerCase();
    if (!address) return null;
    for (const list of state.tokenLists) {
      const entry = (list.entries || []).find(
        (candidate) =>
          candidate.chain_id === numericChainId &&
          candidate.address.toLowerCase() === address,
      );
      if (entry) return { decimals: entry.decimals, symbol: entry.symbol };
    }
    return null;
  }

  function holdingUnits(holding: WalletAssetHolding): AmountUnits | null {
    if (holding.asset_kind === "native" || !holding.asset_address) {
      return nativeUnits(holding.chain_id);
    }
    return tokenUnits(holding.chain_id, holding.asset_address);
  }

  function failureText(error: unknown): string {
    const failure = apiFailure(error);
    return failure?.error ?? (error instanceof Error ? error.message : String(error));
  }

  /** Lock-type failures flip the whole destination into unlock guidance. */
  function isLockFailure(failure: ApiFailure | null): boolean {
    return (
      failure?.code === "vault_locked" ||
      failure?.code === "unauthorized" ||
      failure?.code === "not_initialized"
    );
  }

  // ── Data loading ──────────────────────────────────────────────────

  async function loadPortfolio(): Promise<void> {
    state.load.portfolio = "loading";
    renderView();
    try {
      const query: WalletInventoryListQuery = {
        limit: state.filters.limit,
        offset: state.filters.offset,
        sort: state.filters.sort,
        order: state.filters.order,
        ...(state.filters.chainId !== null
          ? { chain_id: state.filters.chainId }
          : {}),
        ...(state.filters.funded !== null ? { funded: state.filters.funded } : {}),
      };
      const [chains, registry, inventory] = await Promise.all([
        daemonRequest<ChainsResponseWire>("GET", "/api/chains"),
        daemonRequest<TokenRegistryListResponseWire>(
          "GET",
          "/api/inventory/token-registry",
        ),
        runtime.api.listInventoryWallets(query),
      ]);
      state.chains = chains.profiles ?? [];
      state.tokenLists = registry.lists ?? [];
      state.addresses = inventory.addresses ?? [];
      state.holdings = inventory.holdings ?? [];
      state.jobs = (inventory.jobs ?? []) as DiscoveryJobRecord[];
      state.nftCache = inventory.nft_metadata_cache ?? [];
      state.addressesPagination = inventory.pagination ?? null;
      state.load.portfolio = "ready";
      state.everLoaded.portfolio = true;
      state.stale.portfolio = undefined;
      state.locked = false;
    } catch (error) {
      const failure = apiFailure(error);
      if (isLockFailure(failure)) {
        state.locked = true;
        state.load.portfolio = "idle";
      } else if (state.everLoaded.portfolio) {
        // Keep showing the last good data; the banner explains staleness.
        state.stale.portfolio = failureText(error);
        state.load.portfolio = "ready";
      } else {
        state.load.portfolio = "error";
        state.stale.portfolio = failureText(error);
      }
    }
    renderView();
  }

  async function loadRisk(): Promise<void> {
    state.load.risk = "loading";
    renderView();
    try {
      const [findings, catalog] = await Promise.all([
        daemonRequest<RiskFindingListResponseWire>(
          "GET",
          `/api/risk/findings${buildQuery({
            limit: state.riskFilters.limit,
            offset: state.riskFilters.offset,
            severity: state.riskFilters.severity,
            sort: state.riskFilters.sort,
            order: state.riskFilters.order,
          })}`,
        ),
        daemonRequest<RiskCatalogListResponseWire>("GET", "/api/risk/catalog"),
      ]);
      state.findings = findings.findings ?? [];
      state.findingsPagination = findings.pagination ?? null;
      state.catalog = catalog.entries ?? [];
      state.load.risk = "ready";
      state.everLoaded.risk = true;
      state.stale.risk = undefined;
      state.locked = false;
    } catch (error) {
      const failure = apiFailure(error);
      if (isLockFailure(failure)) {
        state.locked = true;
        state.load.risk = "idle";
      } else if (state.everLoaded.risk) {
        state.stale.risk = failureText(error);
        state.load.risk = "ready";
      } else {
        state.load.risk = "error";
        state.stale.risk = failureText(error);
      }
    }
    renderView();
  }

  async function loadTokens(): Promise<void> {
    state.load.tokens = "loading";
    renderView();
    try {
      const [optIns, registry] = await Promise.all([
        daemonRequest<NftOptInListResponseWire>(
          "GET",
          "/api/inventory/nft-metadata/opt-ins",
        ),
        daemonRequest<TokenRegistryListResponseWire>(
          "GET",
          "/api/inventory/token-registry",
        ),
      ]);
      state.optIns = optIns.opt_ins ?? [];
      state.ipfsGateway = optIns.ipfs_gateway_url ?? "";
      state.tokenLists = registry.lists ?? [];
      state.load.tokens = "ready";
      state.everLoaded.tokens = true;
      state.stale.tokens = undefined;
      state.locked = false;
    } catch (error) {
      const failure = apiFailure(error);
      if (isLockFailure(failure)) {
        state.locked = true;
        state.load.tokens = "idle";
      } else if (state.everLoaded.tokens) {
        state.stale.tokens = failureText(error);
        state.load.tokens = "ready";
      } else {
        state.load.tokens = "error";
        state.stale.tokens = failureText(error);
      }
    }
    renderView();
  }

  async function loadProfiles(): Promise<void> {
    state.load.profiles = "loading";
    renderView();
    try {
      const [providers, seeds, xpubs] = await Promise.all([
        daemonRequest<EvmProviderProfilesResponseWire>(
          "GET",
          "/api/profiles/evm",
        ),
        daemonRequest<WalletProfilesResponseWire>(
          "GET",
          "/api/profiles/eth-seed",
        ),
        daemonRequest<WalletProfilesResponseWire>(
          "GET",
          "/api/profiles/eth-xpub",
        ),
      ]);
      state.providers = providers.profiles ?? [];
      state.wallets = [
        ...(seeds.profiles ?? []).map((profile) => ({
          family: "eth-seed",
          profile: profile.name,
        })),
        ...(xpubs.profiles ?? []).map((profile) => ({
          family: "eth-xpub",
          profile: profile.name,
        })),
      ];
      if (!state.stepper.selectionTouched) {
        state.stepper.selectedWallets = new Set(
          state.wallets.map((wallet) => `${wallet.family}/${wallet.profile}`),
        );
        state.stepper.selectedProviders = new Set(
          state.providers.map((provider) => provider.name),
        );
      }
      state.load.profiles = "ready";
      state.everLoaded.profiles = true;
      state.stale.profiles = undefined;
      state.locked = false;
    } catch (error) {
      const failure = apiFailure(error);
      if (isLockFailure(failure)) {
        state.locked = true;
        state.load.profiles = "idle";
      } else if (state.everLoaded.profiles) {
        state.stale.profiles = failureText(error);
        state.load.profiles = "ready";
      } else {
        state.load.profiles = "error";
        state.stale.profiles = failureText(error);
      }
    }
    renderView();
  }

  /** Debounced refetch for store-driven updates (resync/operations). */
  function scheduleRefetch(): void {
    if (state.refetchTimer !== null) return;
    state.refetchTimer = setTimeout(() => {
      state.refetchTimer = null;
      if (state.locked) return;
      if (state.everLoaded.portfolio) void loadPortfolio();
      if (state.view === "risk" && state.everLoaded.risk) void loadRisk();
      if (state.view === "tokens" && state.everLoaded.tokens) void loadTokens();
    }, 250);
  }

  // ── Chrome: banner, status line, nav, locked panel ────────────────

  function updateBanner(): void {
    const region = state.bannerRegion;
    if (!region) return;
    clearContainer(region);
    const group: ResourceGroup =
      state.view === "risk"
        ? "risk"
        : state.view === "tokens"
          ? "tokens"
          : state.view === "scan"
            ? "profiles"
            : "portfolio";
    const message = state.stale[group];
    if (!message || state.locked) return;
    const banner = el("div", {
      class: "dest-banner",
      dataset: { tier: "review", portfolio: "banner" },
      attrs: { role: "status" },
    });
    const text = el("div", { class: "dest-banner-text" });
    text.appendChild(
      el("p", {
        class: "dest-banner-title",
        text: "Couldn't refresh — the data below may be stale",
      }),
    );
    text.appendChild(el("p", { class: "dest-banner-body", text: message }));
    banner.appendChild(text);
    banner.appendChild(
      el("button", {
        class: "btn-ghost btn-small",
        text: "Retry now",
        attrs: { type: "button" },
        on: {
          click: () => {
            if (group === "risk") void loadRisk();
            else if (group === "tokens") void loadTokens();
            else if (group === "profiles") void loadProfiles();
            else void loadPortfolio();
          },
        },
      }),
    );
    region.appendChild(banner);
  }

  function renderNav(view: ViewName): HTMLElement {
    const nav = el("nav", {
      class: "dest-nav",
      attrs: { "aria-label": "Portfolio sections" },
    });
    const links: { view: ViewName; label: string; hash: string }[] = [
      { view: "holdings", label: "Holdings", hash: formatHash("portfolio") },
      { view: "scan", label: "Scan", hash: formatHash("portfolio", "scan") },
      { view: "risk", label: "Risk", hash: formatHash("portfolio", "risk") },
      {
        view: "tokens",
        label: "Tokens & NFTs",
        hash: formatHash("portfolio", "tokens"),
      },
    ];
    for (const link of links) {
      nav.appendChild(
        el("a", {
          class: "dest-nav-link",
          text: link.label,
          attrs: {
            href: link.hash,
            ...(link.view === view ? { "aria-current": "page" } : {}),
          },
        }),
      );
    }
    return nav;
  }

  function renderLockedPanel(): void {
    const viewRoot = state.viewRoot;
    if (!viewRoot) return;
    clearContainer(viewRoot);
    state.refs = {};
    state.statusLine = null;
    viewRoot.appendChild(
      sectionEmpty(
        "The vault is locked",
        "Portfolio data lives inside the encrypted vault. Unlock it to see wallets, holdings, and scan results.",
        { label: "Go to Vault to unlock", href: formatHash("vault") },
      ),
    );
  }

  function pageHeader(
    question: string,
    summary: string,
    ...actions: HTMLElement[]
  ): HTMLElement {
    const header = el("div", { class: "page-header" });
    const text = el("div");
    text.appendChild(el("h2", { class: "page-header-title", text: question }));
    text.appendChild(el("p", { class: "page-header-summary", text: summary }));
    header.appendChild(text);
    if (actions.length) {
      const actionRow = el("div", { class: "page-header-actions" });
      for (const action of actions) actionRow.appendChild(action);
      header.appendChild(actionRow);
    }
    return header;
  }

  // ── Root view: addresses, holdings, scans ─────────────────────────

  function chainFilterOptions(): { value: string; label: string }[] {
    const options: { value: string; label: string }[] = [
      { value: "", label: "All chains" },
    ];
    const seen = new Set<number>();
    for (const chain of state.chains) {
      if (!chain.enabled || chain.chain_id === null || chain.chain_id === undefined) {
        continue;
      }
      if (seen.has(chain.chain_id)) continue;
      seen.add(chain.chain_id);
      options.push({ value: String(chain.chain_id), label: chain.name });
    }
    for (const address of state.addresses) {
      if (seen.has(address.chain_id)) continue;
      seen.add(address.chain_id);
      options.push({
        value: String(address.chain_id),
        label: chainName(address.chain_id),
      });
    }
    return options;
  }

  function buildSelect(
    hook: string,
    label: string,
    options: { value: string; label: string }[],
    current: string,
    onChange: (value: string) => void,
  ): HTMLElement {
    const wrap = el("label", { class: "filter-bar-field" });
    wrap.appendChild(el("span", { class: "filter-bar-label", text: label }));
    const select = el("select", {
      dataset: { portfolio: hook },
      attrs: { "aria-label": label },
    });
    for (const option of options) {
      const node = el("option", {
        text: option.label,
        attrs: { value: option.value },
      });
      if (option.value === current) {
        node.setAttribute("selected", "");
      }
      select.appendChild(node);
    }
    (select as HTMLSelectElement).value = current;
    select.addEventListener("change", () => {
      onChange((select as HTMLSelectElement).value);
    });
    wrap.appendChild(select);
    return wrap;
  }

  function buildRootShell(): void {
    const viewRoot = state.viewRoot;
    if (!viewRoot) return;
    clearContainer(viewRoot);
    state.refs = {};

    viewRoot.appendChild(
      pageHeader(
        "What do I hold, and where?",
        "Every discovered address and holding across your wallets, with how fresh that knowledge is. Scans keep it current.",
        el("button", {
          class: "btn-primary",
          text: "New scan",
          attrs: { type: "button" },
          dataset: { portfolio: "new-scan" },
          on: {
            click: () => runtime.router.navigate(formatHash("portfolio", "scan")),
          },
        }),
      ),
    );
    viewRoot.appendChild(renderNav("holdings"));
    const statusLine = el("p", {
      class: "dest-status",
      dataset: { portfolio: "status" },
      attrs: { "aria-live": "polite" },
    });
    viewRoot.appendChild(statusLine);
    state.statusLine = statusLine;

    // ── Addresses section ──
    const addressesSection = el("section", { class: "dest-section" });
    const addressesHead = el("div", { class: "dest-section-head" });
    addressesHead.appendChild(
      el("h3", { class: "section-title", text: "Addresses" }),
    );
    const addressesCount = el("span", {
      class: "dest-count",
      dataset: { portfolio: "addresses-count" },
    });
    addressesHead.appendChild(addressesCount);
    addressesSection.appendChild(addressesHead);

    const filterBar = el("div", {
      class: "filter-bar",
      dataset: { portfolio: "filter-bar" },
    });
    filterBar.appendChild(
      buildSelect("filter-chain", "Chain", chainFilterOptions(), "", (value) => {
        state.filters.chainId = value ? Number(value) : null;
        state.filters.offset = 0;
        void loadPortfolio();
      }),
    );
    filterBar.appendChild(
      buildSelect(
        "filter-funded",
        "Balance",
        [
          { value: "", label: "All balances" },
          { value: "funded", label: "Funded only" },
          { value: "unfunded", label: "Not funded" },
        ],
        "",
        (value) => {
          state.filters.funded =
            value === "funded" ? true : value === "unfunded" ? false : null;
          state.filters.offset = 0;
          void loadPortfolio();
        },
      ),
    );
    filterBar.appendChild(
      buildSelect(
        "filter-sort",
        "Sort",
        [
          { value: "recent", label: "Recently scanned first" },
          { value: "oldest", label: "Stalest scan first" },
          { value: "address", label: "Address A to Z" },
        ],
        "recent",
        (value) => {
          if (value === "address") {
            state.filters.sort = "address";
            state.filters.order = "asc";
          } else if (value === "oldest") {
            state.filters.sort = "last_scanned";
            state.filters.order = "asc";
          } else {
            state.filters.sort = "last_scanned";
            state.filters.order = "desc";
          }
          state.filters.offset = 0;
          void loadPortfolio();
        },
      ),
    );
    addressesSection.appendChild(filterBar);

    const addressesEmpty = el("div", { dataset: { portfolio: "addresses-empty" } });
    addressesSection.appendChild(addressesEmpty);
    const addressesWrap = el("div", {
      class: "table-scroll",
      dataset: { portfolio: "addresses-wrap" },
    });
    const addressesTable = el("table", { class: "table compact" });
    const thead = el("thead");
    const headRow = el("tr");
    for (const title of [
      "Address",
      "Chain",
      "Wallet",
      "Balance",
      "State",
      "Scanned",
      "Details",
    ]) {
      headRow.appendChild(el("th", { text: title }));
    }
    thead.appendChild(headRow);
    addressesTable.appendChild(thead);
    const addressesBody = el("tbody", {
      dataset: { portfolio: "addresses-body" },
    });
    addressesTable.appendChild(addressesBody);
    addressesWrap.appendChild(addressesTable);
    addressesSection.appendChild(addressesWrap);

    const pagination = el("div", {
      class: "dest-pagination",
      dataset: { portfolio: "addresses-pagination" },
    });
    const prevButton = el("button", {
      class: "btn-ghost btn-small",
      text: "Previous",
      attrs: { type: "button" },
      dataset: { portfolio: "addresses-prev" },
      on: {
        click: () => {
          state.filters.offset = Math.max(
            0,
            state.filters.offset - state.filters.limit,
          );
          void loadPortfolio();
        },
      },
    });
    const pageLabel = el("span", {
      class: "dest-page-label nums",
      dataset: { portfolio: "addresses-page-label" },
    });
    const nextButton = el("button", {
      class: "btn-ghost btn-small",
      text: "Next",
      attrs: { type: "button" },
      dataset: { portfolio: "addresses-next" },
      on: {
        click: () => {
          state.filters.offset += state.filters.limit;
          void loadPortfolio();
        },
      },
    });
    pagination.appendChild(prevButton);
    pagination.appendChild(pageLabel);
    pagination.appendChild(nextButton);
    addressesSection.appendChild(pagination);
    viewRoot.appendChild(addressesSection);

    // ── Holdings section ──
    const holdingsSection = el("section", { class: "dest-section" });
    const holdingsHead = el("div", { class: "dest-section-head" });
    holdingsHead.appendChild(
      el("h3", { class: "section-title", text: "Holdings" }),
    );
    const holdingsCount = el("span", {
      class: "dest-count",
      dataset: { portfolio: "holdings-count" },
    });
    holdingsHead.appendChild(holdingsCount);
    holdingsSection.appendChild(holdingsHead);
    const holdingsEmpty = el("div", { dataset: { portfolio: "holdings-empty" } });
    holdingsSection.appendChild(holdingsEmpty);
    const holdingsWrap = el("div", {
      class: "table-scroll",
      dataset: { portfolio: "holdings-wrap" },
    });
    const holdingsTable = el("table", {
      class: "table compact portfolio-holdings-table",
    });
    const holdingsThead = el("thead");
    const holdingsHeadRow = el("tr");
    for (const title of ["Asset", "Amount", "Address", "Chain", "Status", "Updated"]) {
      holdingsHeadRow.appendChild(el("th", { text: title }));
    }
    holdingsThead.appendChild(holdingsHeadRow);
    holdingsTable.appendChild(holdingsThead);
    const holdingsBody = el("tbody", {
      dataset: { portfolio: "holdings-body" },
    });
    holdingsTable.appendChild(holdingsBody);
    holdingsWrap.appendChild(holdingsTable);
    holdingsSection.appendChild(holdingsWrap);
    viewRoot.appendChild(holdingsSection);

    // ── Scans section ──
    const scansSection = el("section", { class: "dest-section" });
    const scansHead = el("div", { class: "dest-section-head" });
    scansHead.appendChild(el("h3", { class: "section-title", text: "Scans" }));
    scansSection.appendChild(scansHead);
    const opsRegion = el("div", {
      class: "dest-ops",
      dataset: { portfolio: "ops" },
    });
    scansSection.appendChild(opsRegion);
    const jobsEmpty = el("div", { dataset: { portfolio: "jobs-empty" } });
    scansSection.appendChild(jobsEmpty);
    const jobsWrap = el("div", {
      class: "table-scroll",
      dataset: { portfolio: "jobs-wrap" },
    });
    const jobsTable = el("table", { class: "table compact" });
    const jobsThead = el("thead");
    const jobsHeadRow = el("tr");
    for (const title of ["Status", "Scope", "Results", "Started", "Actions", "Details"]) {
      jobsHeadRow.appendChild(el("th", { text: title }));
    }
    jobsThead.appendChild(jobsHeadRow);
    jobsTable.appendChild(jobsThead);
    const jobsBody = el("tbody", { dataset: { portfolio: "jobs-body" } });
    jobsTable.appendChild(jobsBody);
    jobsWrap.appendChild(jobsTable);
    scansSection.appendChild(jobsWrap);
    viewRoot.appendChild(scansSection);

    state.refs = {
      addressesCount,
      addressesEmpty,
      addressesWrap,
      addressesBody,
      addressesPrev: prevButton,
      addressesNext: nextButton,
      addressesPageLabel: pageLabel,
      filterBar,
      holdingsCount,
      holdingsEmpty,
      holdingsWrap,
      holdingsBody,
      opsRegion,
      jobsEmpty,
      jobsWrap,
      jobsBody,
    };
  }

  function signerPill(address: WalletInventoryAddress): HTMLElement | null {
    const classifications = address.classifications ?? [];
    if (classifications.includes("signer_available")) {
      return pill("Signer", "pill-good");
    }
    if (classifications.includes("watch_only")) {
      return pill("Watch-only", "pill-info");
    }
    if (classifications.includes("signer_unknown")) {
      return pill("Signer unknown", "pill-neutral");
    }
    if (
      address.wallet_family === "eth-watch" ||
      address.wallet_family === "eth-xpub"
    ) {
      return pill("Watch-only", "pill-info");
    }
    return null;
  }

  function addressRow(
    address: WalletInventoryAddress,
    existing: HTMLElement | null,
  ): HTMLElement {
    if (existing) return existing; // stable rows keep focus/copy affordance
    const row = el("tr", {
      dataset: { portfolio: "address-row" },
    }) as HTMLTableRowElement;
    const addressTd = el("td");
    addressTd.appendChild(
      addressCell(address.address, () => setStatus("Address copied to clipboard")),
    );
    row.appendChild(addressTd);
    row.appendChild(el("td", { text: chainName(address.chain_id) }));
    const walletTd = el("td");
    walletTd.appendChild(
      el("div", {
        class: "cell-primary",
        text: `${familyLabel(address.wallet_family)} · ${address.wallet_profile}`,
      }),
    );
    const signer = signerPill(address);
    if (signer) walletTd.appendChild(signer);
    row.appendChild(walletTd);
    const balanceTd = el("td", { class: "nums" });
    balanceTd.appendChild(
      amountContent(address.native_balance_wei_hex, nativeUnits(address.chain_id)),
    );
    row.appendChild(balanceTd);
    const stateTd = el("td");
    stateTd.appendChild(pill(activityLabel(address.activity_state)));
    if ((address.classifications ?? []).includes("dormant_candidate")) {
      stateTd.appendChild(pill("Dormant", "pill-warn"));
    }
    row.appendChild(stateTd);
    row.appendChild(
      el("td", {
        class: "nums",
        text: relativeTime(address.last_checked_at_unix, nowSecs()),
        attrs: { title: formatTimestamp(address.last_checked_at_unix) },
      }),
    );
    const detailsTd = el("td");
    const details = el("details", { class: "row-details" });
    details.appendChild(el("summary", { text: "Details" }));
    const detailList = el("dl", { class: "detail-list" });
    const pairs: [string, string][] = [
      ["Full address", address.address],
      ["Derivation path", address.derivation_path || "-"],
      ["Transactions", String(address.transaction_count ?? 0)],
      [
        "Last activity block",
        address.last_activity_block != null
          ? String(address.last_activity_block)
          : "none seen",
      ],
      ["First seen", formatTimestamp(address.first_seen_at_unix)],
      ["Source", address.source || "-"],
      [
        "Classifications",
        (address.classifications ?? []).map((c) => c.replace(/_/g, " ")).join(", ") ||
          "none",
      ],
    ];
    for (const [term, value] of pairs) {
      detailList.appendChild(el("dt", { text: term }));
      detailList.appendChild(el("dd", { class: "mono", text: value }));
    }
    details.appendChild(detailList);
    detailsTd.appendChild(details);
    row.appendChild(detailsTd);
    return row;
  }

  function updateAddressesSection(): void {
    const refs = state.refs;
    if (!refs.addressesBody) return;
    const loading = state.load.portfolio === "loading";
    const firstLoad = !state.everLoaded.portfolio;

    if (loading && firstLoad) {
      clearContainer(refs.addressesEmpty!);
      refs.addressesEmpty!.appendChild(skeletonRows(4));
      setHidden(refs.addressesEmpty!, false);
      setHidden(refs.addressesWrap!, true);
      refs.addressesCount!.textContent = "";
      refs.addressesPageLabel!.textContent = "";
      (refs.addressesPrev as HTMLButtonElement).disabled = true;
      (refs.addressesNext as HTMLButtonElement).disabled = true;
      return;
    }
    if (state.load.portfolio === "error" && firstLoad) {
      clearContainer(refs.addressesEmpty!);
      refs.addressesEmpty!.appendChild(
        sectionEmpty(
          "Couldn't load the portfolio",
          state.stale.portfolio ?? "The daemon did not answer.",
          { label: "Retry", onClick: () => void loadPortfolio() },
        ),
      );
      setHidden(refs.addressesEmpty!, false);
      setHidden(refs.addressesWrap!, true);
      return;
    }

    setHidden(refs.addressesWrap!, false);
    const total = state.addressesPagination?.total ?? state.addresses.length;
    refs.addressesCount!.textContent = `${total} address${total === 1 ? "" : "es"}`;
    if (!state.addresses.length) {
      clearContainer(refs.addressesEmpty!);
      refs.addressesEmpty!.appendChild(
        sectionEmpty(
          state.filters.chainId !== null || state.filters.funded !== null
            ? "Nothing matches these filters"
            : "No holdings discovered yet",
          state.filters.chainId !== null || state.filters.funded !== null
            ? "Try widening the chain or balance filters — or run a scan to discover more."
            : "Run a balance scan to discover the addresses and holdings in your wallets. Nothing leaves this machine except the RPC calls you configure.",
          { label: "Run a scan", href: formatHash("portfolio", "scan") },
        ),
      );
      setHidden(refs.addressesEmpty!, false);
      setHidden(refs.addressesWrap!, true);
    } else {
      clearContainer(refs.addressesEmpty!);
      setHidden(refs.addressesEmpty!, true);
      setHidden(refs.addressesWrap!, false);
      renderList(refs.addressesBody!, state.addresses, (a) => a.id, addressRow);
    }

    const offset = state.filters.offset;
    const page = state.addressesPagination;
    if (page) {
      const from = page.total === 0 ? 0 : offset + 1;
      const to = offset + state.addresses.length;
      refs.addressesPageLabel!.textContent = `${from}–${to} of ${page.total}`;
      (refs.addressesPrev as HTMLButtonElement).disabled = offset === 0;
      (refs.addressesNext as HTMLButtonElement).disabled = !page.has_more;
    } else {
      refs.addressesPageLabel!.textContent = state.addresses.length
        ? `1–${state.addresses.length} of ${state.addresses.length}`
        : "";
      (refs.addressesPrev as HTMLButtonElement).disabled = true;
      (refs.addressesNext as HTMLButtonElement).disabled = true;
    }
  }

  function holdingAssetLabel(holding: WalletAssetHolding): string {
    if (holding.asset_kind === "native") {
      return `${nativeUnits(holding.chain_id).symbol ?? "ETH"} (native)`;
    }
    const units = tokenUnits(holding.chain_id, holding.asset_address);
    if (units?.symbol) return units.symbol;
    return String(holding.asset_kind || "asset").replace(/_/g, " ");
  }

  function holdingRow(
    holding: WalletAssetHolding,
    existing: HTMLElement | null,
  ): HTMLElement {
    if (existing) return existing;
    const row = el("tr", { dataset: { portfolio: "holding-row" } });
    row.appendChild(el("td", { text: holdingAssetLabel(holding) }));
    const amountTd = el("td");
    amountTd.appendChild(amountContent(holding.amount_hex, holdingUnits(holding)));
    row.appendChild(amountTd);
    const addressTd = el("td");
    addressTd.appendChild(
      el("span", {
        class: "mono",
        text: middleTruncate(holding.address),
        attrs: { title: holding.address },
      }),
    );
    row.appendChild(addressTd);
    row.appendChild(el("td", { text: chainName(holding.chain_id) }));
    const statusTd = el("td");
    statusTd.appendChild(
      pill(String(holding.status || "unknown").replace(/_/g, " ")),
    );
    if (holding.spam_label && holding.spam_label !== "operator_trusted") {
      statusTd.appendChild(pill(spamLabelText(holding.spam_label), "pill-warn"));
    }
    row.appendChild(statusTd);
    row.appendChild(
      el("td", {
        class: "nums",
        text: relativeTime(holding.last_checked_at_unix, nowSecs()),
        attrs: { title: formatTimestamp(holding.last_checked_at_unix) },
      }),
    );
    return row;
  }

  function updateHoldingsSection(): void {
    const refs = state.refs;
    if (!refs.holdingsBody) return;
    const loading = state.load.portfolio === "loading";
    if (loading && !state.everLoaded.portfolio) {
      clearContainer(refs.holdingsEmpty!);
      setHidden(refs.holdingsEmpty!, true);
      setHidden(refs.holdingsWrap!, true);
      refs.holdingsCount!.textContent = "";
      return;
    }
    refs.holdingsCount!.textContent = `${state.holdings.length} asset${
      state.holdings.length === 1 ? "" : "s"
    }`;
    if (!state.holdings.length) {
      clearContainer(refs.holdingsEmpty!);
      refs.holdingsEmpty!.appendChild(
        sectionEmpty(
          "No asset holdings detected yet",
          "Token, NFT, and approval holdings appear here after a scan with the discovery options you enable.",
          { label: "Run a scan", href: formatHash("portfolio", "scan") },
        ),
      );
      setHidden(refs.holdingsEmpty!, false);
      setHidden(refs.holdingsWrap!, true);
      return;
    }
    clearContainer(refs.holdingsEmpty!);
    setHidden(refs.holdingsEmpty!, true);
    setHidden(refs.holdingsWrap!, false);
    renderList(refs.holdingsBody!, state.holdings, (h) => h.id, holdingRow);
  }

  // ── Scans section: live operations + discovery jobs ───────────────

  function scanOperations(): Operation[] {
    return runtime.store
      .get("operations")
      .filter((operation) => operation.kind === SCAN_OPERATION_KIND);
  }

  function operationRow(operation: Operation): HTMLElement {
    const row = el("div", {
      class: "ops-row",
      dataset: { portfolio: "op-row", opId: operation.id },
    });
    const running = !isTerminalOperationState(operation.state);
    row.appendChild(
      el("span", {
        class: "status-dot",
        dataset: {
          state: running
            ? "busy"
            : operation.state === "completed"
              ? "live"
              : "error",
        },
      }),
    );
    const main = el("div", { class: "ops-row-main" });
    main.appendChild(
      el("p", {
        class: "ops-row-title",
        text: `Balance scan · ${operationStateLabel(operation.state)}`,
      }),
    );
    const total = operation.progress?.total;
    const processed = operation.progress?.processed ?? 0;
    main.appendChild(
      el("p", {
        class: "ops-row-body nums",
        text: operation.error
          ? operation.error
          : total
            ? `${processed.toLocaleString()} of ${total.toLocaleString()} checks`
            : `${processed.toLocaleString()} checks so far`,
      }),
    );
    row.appendChild(main);
    if (running) {
      const cancel = el("button", {
        class: "btn-ghost btn-small",
        text: "Cancel",
        attrs: { type: "button" },
        dataset: { portfolio: "op-cancel" },
        on: {
          click: () => {
            (cancel as HTMLButtonElement).disabled = true;
            void cancelOperation(operation.id);
          },
        },
      });
      row.appendChild(cancel);
    }
    return row;
  }

  async function cancelOperation(id: string): Promise<void> {
    try {
      await runtime.api.cancelOperation(id);
      setStatus("Cancel requested — the scan stops after the address it is checking. Progress so far is kept.");
    } catch (error) {
      const failure = apiFailure(error);
      if (isLockFailure(failure)) {
        state.locked = true;
        renderView();
        return;
      }
      setStatus(`Couldn't cancel the scan: ${failureText(error)}`);
    }
  }

  function updateOpsRegion(): void {
    const region = state.refs.opsRegion;
    if (!region) return;
    const ops = scanOperations().filter(
      (operation) => !isTerminalOperationState(operation.state),
    );
    renderList(
      region,
      ops,
      (operation) => operation.id,
      (operation, existing) => {
        // Rebuild rows on state/progress change; keep stable rows as-is.
        const marker = `${operation.state}:${operation.progress?.processed ?? 0}:${operation.error ?? ""}`;
        if (existing && existing.dataset.opMarker === marker) {
          return existing;
        }
        const row = operationRow(operation);
        row.dataset.opMarker = marker;
        return row;
      },
    );
  }

  function jobScopeSummary(job: DiscoveryJobRecord): string {
    const wallets = (job.wallet_profiles ?? []).length
      ? (job.wallet_profiles ?? []).join(", ")
      : "all wallets";
    const providers = (job.provider_profiles ?? []).length
      ? (job.provider_profiles ?? []).join(", ")
      : "all providers";
    const chains = (job.chain_ids ?? []).length
      ? (job.chain_ids ?? []).map((id) => chainName(id)).join(", ")
      : "all chains";
    return `${wallets} · ${providers} · ${chains}`;
  }

  function jobRow(
    job: DiscoveryJobRecord,
    existing: HTMLElement | null,
  ): HTMLElement {
    if (existing) return existing;
    const row = el("tr", { dataset: { portfolio: "job-row" } });
    const statusTd = el("td");
    statusTd.appendChild(pill(jobStatusLabel(job.status)));
    if (job.partition_providers) {
      statusTd.appendChild(pill("Providers partitioned", "pill-info"));
    }
    row.appendChild(statusTd);
    row.appendChild(
      el("td", {
        class: "cell-wrap",
        text: jobScopeSummary(job),
      }),
    );
    row.appendChild(
      el("td", {
        class: "nums",
        text:
          `${(job.addresses_scanned ?? 0).toLocaleString()} scanned · ` +
          `${(job.active_addresses ?? 0).toLocaleString()} active · ` +
          `${(job.holdings_detected ?? 0).toLocaleString()} holdings`,
      }),
    );
    row.appendChild(
      el("td", {
        class: "nums",
        text: relativeTime(job.started_at_unix, nowSecs()),
        attrs: { title: formatTimestamp(job.started_at_unix) },
      }),
    );
    const actionsTd = el("td", { class: "col-actions" });
    if (job.status === "running" || job.status === "resume_requested") {
      actionsTd.appendChild(
        el("button", {
          class: "btn-ghost btn-small",
          text: "Cancel",
          attrs: { type: "button" },
          dataset: { portfolio: "job-cancel" },
          on: { click: () => void cancelDiscoveryJob(job.id) },
        }),
      );
    } else {
      actionsTd.appendChild(
        el("button", {
          class: "btn-ghost btn-small",
          text: "Resume",
          attrs: { type: "button" },
          dataset: { portfolio: "job-resume" },
          on: { click: () => void resumeDiscoveryJob(job.id) },
        }),
      );
    }
    row.appendChild(actionsTd);
    const detailsTd = el("td");
    const details = el("details", { class: "row-details" });
    details.appendChild(el("summary", { text: "Details" }));
    const list = el("dl", { class: "detail-list" });
    list.appendChild(el("dt", { text: "Job id" }));
    list.appendChild(el("dd", { class: "mono", text: job.id }));
    if (job.last_error) {
      list.appendChild(el("dt", { text: "Last error" }));
      list.appendChild(el("dd", { text: job.last_error }));
    }
    list.appendChild(el("dt", { text: "Completed" }));
    list.appendChild(
      el("dd", {
        text: job.completed_at_unix
          ? formatTimestamp(job.completed_at_unix)
          : "not yet",
      }),
    );
    const observations = job.provider_partition_observations ?? [];
    if (observations.length) {
      list.appendChild(el("dt", { text: "Coverage by provider" }));
      for (const observation of observations) {
        list.appendChild(
          el("dd", {
            class: "nums",
            text: `${observation.provider_profile} (${chainName(
              observation.chain_id,
            )}): ${observation.addresses_observed.toLocaleString()} addresses — disjoint from the other providers`,
          }),
        );
      }
    }
    const cursors = job.block_cursors ?? [];
    if (cursors.length) {
      list.appendChild(el("dt", { text: "Scanned to block" }));
      const byChain = new Map<string, number>();
      for (const cursor of cursors) {
        const key = `${chainName(cursor.chain_id)} ${cursor.topic_family.replace(/_/g, " ")}`;
        byChain.set(
          key,
          Math.max(byChain.get(key) ?? 0, cursor.last_scanned_block),
        );
      }
      for (const [label, block] of Array.from(byChain)) {
        list.appendChild(
          el("dd", { class: "nums", text: `${label}: block ${block.toLocaleString()}` }),
        );
      }
    }
    details.appendChild(list);
    detailsTd.appendChild(details);
    row.appendChild(detailsTd);
    return row;
  }

  async function cancelDiscoveryJob(id: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Cancel discovery scan",
      body: `Stop scan "${id}"? It stops after the address it is currently checking. Progress so far is kept and you can resume from it later.`,
      actionLabel: "Cancel scan",
    });
    if (!confirmed) return;
    try {
      const result = await daemonRequest<DiscoveryJobMutationWire>(
        "POST",
        "/api/discovery/jobs/cancel",
        { id },
      );
      setStatus(
        result.status === "cancel_requested"
          ? "Cancel requested — the scan stops after the current address."
          : "Discovery scan canceled.",
      );
      void loadPortfolio();
    } catch (error) {
      const failure = apiFailure(error);
      if (isLockFailure(failure)) {
        state.locked = true;
        renderView();
        return;
      }
      setStatus(`Couldn't cancel the scan: ${failureText(error)}`);
    }
  }

  async function resumeDiscoveryJob(id: string): Promise<void> {
    try {
      await daemonRequest<DiscoveryJobMutationWire>(
        "POST",
        "/api/discovery/jobs/resume",
        { id },
      );
      setStatus("Scan resumed in the background — progress shows here live.");
      void loadPortfolio();
    } catch (error) {
      const failure = apiFailure(error);
      if (isLockFailure(failure)) {
        state.locked = true;
        renderView();
        return;
      }
      setStatus(`Couldn't resume the scan: ${failureText(error)}`);
    }
  }

  function updateJobsSection(): void {
    const refs = state.refs;
    if (!refs.jobsBody) return;
    const jobs = state.jobs
      .slice()
      .sort((a, b) => (b.started_at_unix ?? 0) - (a.started_at_unix ?? 0));
    if (!jobs.length) {
      clearContainer(refs.jobsEmpty!);
      refs.jobsEmpty!.appendChild(
        sectionEmpty(
          "No scans yet",
          "Discovery scans find the addresses, balances, and approvals in your wallets. Their progress and results appear here.",
          { label: "Start your first scan", href: formatHash("portfolio", "scan") },
        ),
      );
      setHidden(refs.jobsEmpty!, false);
      setHidden(refs.jobsWrap!, true);
      return;
    }
    clearContainer(refs.jobsEmpty!);
    setHidden(refs.jobsEmpty!, true);
    setHidden(refs.jobsWrap!, false);
    renderList(refs.jobsBody!, jobs, (job) => job.id, jobRow);
  }

  // ── Scan stepper (#/portfolio/scan) ───────────────────────────────

  const STEP_TITLES = ["Wallets", "Providers", "Launch", "Results"] as const;

  /** Advanced-form field refs; rebuilt with the step-3 content. */
  let advancedFields: Record<string, HTMLElement> = {};

  function buildScanShell(): void {
    const viewRoot = state.viewRoot;
    if (!viewRoot) return;
    clearContainer(viewRoot);
    state.refs = {};
    state.scanRenderToken = null;

    viewRoot.appendChild(
      pageHeader(
        "Scan for holdings",
        "Pick what to scan, review the scope, and launch. Scans are read-only: they ask your configured RPC providers about public addresses and never move funds.",
      ),
    );
    viewRoot.appendChild(renderNav("scan"));
    const statusLine = el("p", {
      class: "dest-status",
      dataset: { portfolio: "status" },
      attrs: { "aria-live": "polite" },
    });
    viewRoot.appendChild(statusLine);
    state.statusLine = statusLine;

    const steps = el("ol", {
      class: "stepper-steps",
      dataset: { portfolio: "steps" },
    });
    viewRoot.appendChild(steps);
    state.refs.steps = steps;

    const stepContent = el("div", {
      class: "stepper-content",
      dataset: { portfolio: "step-content" },
    });
    viewRoot.appendChild(stepContent);
    state.refs.stepContent = stepContent;

    const navRow = el("div", {
      class: "stepper-nav",
      dataset: { portfolio: "step-nav" },
    });
    viewRoot.appendChild(navRow);
    state.refs.stepNav = navRow;

    updateScanView();
  }

  function updateScanView(): void {
    const steps = state.refs.steps;
    if (!steps) return;
    clearContainer(steps);
    STEP_TITLES.forEach((title, index) => {
      const number = (index + 1) as 1 | 2 | 3 | 4;
      steps.appendChild(
        el("li", {
          class:
            number === state.stepper.step
              ? "stepper-step is-current"
              : number < state.stepper.step
                ? "stepper-step is-done"
                : "stepper-step",
          text: `${index + 1}. ${title}`,
          ...(number === state.stepper.step
            ? { attrs: { "aria-current": "step" } }
            : {}),
        }),
      );
    });
    updateStepContent();
  }

  function updateStepNav(): void {
    const nav = state.refs.stepNav;
    if (!nav) return;
    clearContainer(nav);
    const step = state.stepper.step;
    if (step === 4 || state.stepper.launching) return;
    if (step > 1) {
      nav.appendChild(
        el("button", {
          class: "btn-ghost",
          text: "Back",
          attrs: { type: "button" },
          dataset: { portfolio: "step-back" },
          on: {
            click: () => {
              state.stepper.step = (step - 1) as 1 | 2 | 3;
              updateScanView();
            },
          },
        }),
      );
    }
    if (step === 1 || step === 2) {
      nav.appendChild(
        el("button", {
          class: "btn-primary",
          text: step === 1 ? "Next: providers" : "Next: review and launch",
          attrs: { type: "button" },
          dataset: { portfolio: "step-next" },
          on: {
            click: () => {
              state.stepper.step = (step + 1) as 2 | 3;
              updateScanView();
            },
          },
        }),
      );
    }
  }

  /**
   * Step content rebuilds are token-guarded: store-driven renderView calls
   * (operation progress, resync) MUST NOT wipe in-progress form state or
   * focus. Only step / profile-load / launching / error transitions rebuild;
   * step 4 patches its own stable regions instead.
   */
  function updateStepContent(): void {
    const content = state.refs.stepContent;
    if (!content) return;
    const token = [
      state.stepper.step,
      state.load.profiles,
      state.stepper.launching ? "launching" : "idle",
      state.stepper.launchError ?? "",
    ].join("|");
    if (state.scanRenderToken === token) {
      if (state.stepper.step === 4) patchStepResults();
      return;
    }
    state.scanRenderToken = token;
    clearContainer(content);
    updateStepNav();
    if (state.load.profiles === "loading" && !state.everLoaded.profiles) {
      content.appendChild(skeletonRows(3));
      return;
    }
    if (state.load.profiles === "error" && !state.everLoaded.profiles) {
      content.appendChild(
        sectionEmpty(
          "Couldn't load wallets and providers",
          state.stale.profiles ?? "The daemon did not answer.",
          { label: "Retry", onClick: () => void loadProfiles() },
        ),
      );
      return;
    }
    switch (state.stepper.step) {
      case 1:
        renderStepWallets(content);
        break;
      case 2:
        renderStepProviders(content);
        break;
      case 3:
        renderStepLaunch(content);
        break;
      case 4:
        renderStepResults(content);
        break;
    }
  }

  function checkRow(
    hook: string,
    checked: boolean,
    title: string,
    body: string | null,
    onChange: (next: boolean) => void,
  ): HTMLElement {
    const label = el("label", { class: "check-list-row" });
    const input = el("input", {
      attrs: { type: "checkbox" },
      dataset: { portfolio: hook },
    }) as HTMLInputElement;
    input.checked = checked;
    input.addEventListener("change", () => onChange(input.checked));
    label.appendChild(input);
    const text = el("span", { class: "check-list-text" });
    text.appendChild(el("span", { class: "check-list-title", text: title }));
    if (body) {
      text.appendChild(el("span", { class: "check-list-body", text: body }));
    }
    label.appendChild(text);
    return label;
  }

  function updateWalletNote(): void {
    const note = state.refs.walletNote;
    if (!note) return;
    const partial =
      state.stepper.selectedWallets.size > 0 &&
      state.stepper.selectedWallets.size < state.wallets.length;
    if (partial) {
      note.textContent =
        `A partial selection runs one scan per selected wallet ` +
        `(${state.stepper.selectedWallets.size} scans) — the scope never widens beyond what you check.`;
      setHidden(note, false);
    } else {
      note.textContent = "";
      setHidden(note, true);
    }
  }

  function renderStepWallets(content: HTMLElement): void {
    content.appendChild(
      el("p", {
        class: "stepper-lead",
        text: "Which wallets should the scan look at? Addresses are derived read-only from your wallet profiles.",
      }),
    );
    if (!state.wallets.length) {
      content.appendChild(
        sectionEmpty(
          "No wallet profiles yet",
          "Create a seed or xpub wallet profile in the Wallet manager card below, or include saved watch addresses instead.",
        ),
      );
    }
    const list = el("div", {
      class: "check-list",
      dataset: { portfolio: "wallet-list" },
    });
    for (const wallet of state.wallets) {
      const key = `${wallet.family}/${wallet.profile}`;
      list.appendChild(
        checkRow(
          "wallet-check",
          state.stepper.selectedWallets.has(key),
          `${familyLabel(wallet.family)} · ${wallet.profile}`,
          null,
          (next) => {
            state.stepper.selectionTouched = true;
            if (next) state.stepper.selectedWallets.add(key);
            else state.stepper.selectedWallets.delete(key);
            updateWalletNote();
          },
        ),
      );
    }
    content.appendChild(list);
    const tools = el("div", { class: "check-list-tools" });
    tools.appendChild(
      el("button", {
        class: "btn-ghost btn-small",
        text: "Select all",
        attrs: { type: "button" },
        dataset: { portfolio: "wallets-all" },
        on: {
          click: () => {
            state.stepper.selectionTouched = true;
            state.stepper.selectedWallets = new Set(
              state.wallets.map((wallet) => `${wallet.family}/${wallet.profile}`),
            );
            state.scanRenderToken = null; // deliberate action: rebuild checks
            updateStepContent();
          },
        },
      }),
    );
    tools.appendChild(
      el("button", {
        class: "btn-ghost btn-small",
        text: "Select none",
        attrs: { type: "button" },
        dataset: { portfolio: "wallets-none" },
        on: {
          click: () => {
            state.stepper.selectionTouched = true;
            state.stepper.selectedWallets = new Set();
            state.scanRenderToken = null;
            updateStepContent();
          },
        },
      }),
    );
    content.appendChild(tools);
    content.appendChild(
      checkRow(
        "watch-book-check",
        state.stepper.includeWatchBook,
        "Include the saved watch address book",
        "Watch-only addresses you saved earlier. Extra one-off watch addresses can be added under Advanced options.",
        (next) => {
          state.stepper.includeWatchBook = next;
        },
      ),
    );
    const note = el("p", {
      class: "stepper-note",
      dataset: { portfolio: "wallet-note" },
    });
    content.appendChild(note);
    state.refs.walletNote = note;
    updateWalletNote();
  }

  function renderStepProviders(content: HTMLElement): void {
    content.appendChild(
      el("p", {
        class: "stepper-lead",
        text: "Which providers should check the addresses? Every selected provider sees the addresses it checks — pick endpoints you trust with that.",
      }),
    );
    if (!state.providers.length) {
      content.appendChild(
        sectionEmpty(
          "No EVM provider profiles yet",
          "A provider profile is an RPC endpoint plus fee policy for one chain. Save one in the Wallet manager card below, then come back.",
        ),
      );
      return;
    }
    const byChain = new Map<number, EvmProviderProfileRecord[]>();
    for (const provider of state.providers) {
      const group = byChain.get(provider.chain_id) ?? [];
      group.push(provider);
      byChain.set(provider.chain_id, group);
    }
    const groups = el("div", { dataset: { portfolio: "provider-groups" } });
    for (const [chainId, providers] of Array.from(byChain.entries()).sort(
      (a, b) => a[0] - b[0],
    )) {
      const group = el("div", { class: "check-group" });
      group.appendChild(
        el("p", {
          class: "check-group-title",
          text: `${chainName(chainId)} — ${providers.length} provider${
            providers.length === 1 ? "" : "s"
          }`,
        }),
      );
      const list = el("div", { class: "check-list" });
      for (const provider of providers) {
        list.appendChild(
          checkRow(
            "provider-check",
            state.stepper.selectedProviders.has(provider.name),
            provider.name,
            null,
            (next) => {
              state.stepper.selectionTouched = true;
              if (next) state.stepper.selectedProviders.add(provider.name);
              else state.stepper.selectedProviders.delete(provider.name);
              updatePartitionNote();
            },
          ),
        );
      }
      group.appendChild(list);
      groups.appendChild(group);
    }
    content.appendChild(groups);

    if (
      hasMultiProviderChain(
        state.providers.map((provider) => ({
          name: provider.name,
          chainId: provider.chain_id,
        })),
      )
    ) {
      const partition = el("div", {
        class: "partition-option",
        dataset: { portfolio: "partition-option" },
      });
      partition.appendChild(
        checkRow(
          "partition-check",
          state.stepper.partitionProviders,
          "Split each chain's addresses across its providers",
          "When a chain has more than one provider, each provider sees only a share of the addresses instead of the whole set — no single endpoint learns everything. This needs every provider scanned together.",
          (next) => {
            state.stepper.partitionProviders = next;
            updatePartitionNote();
          },
        ),
      );
      const note = el("p", {
        class: "stepper-note",
        dataset: { portfolio: "partition-note" },
      });
      partition.appendChild(note);
      content.appendChild(partition);
      state.refs.partitionNote = note;
      updatePartitionNote();
    }
  }

  function updatePartitionNote(): void {
    const note = state.refs.partitionNote;
    if (!note) return;
    const allSelected =
      state.providers.length > 0 &&
      state.providers.every((provider) =>
        state.stepper.selectedProviders.has(provider.name),
      );
    if (!state.stepper.partitionProviders) {
      note.textContent = "";
      setHidden(note, true);
    } else if (allSelected) {
      note.textContent =
        "Partitioning is on: each provider will see only a disjoint share of the addresses on its chain.";
      setHidden(note, false);
    } else {
      note.textContent =
        "Partitioning applies only when every provider is scanned together — with a partial provider selection each selected provider is scanned on its own and sees everything you point at it.";
      setHidden(note, false);
    }
  }

  // ── Step 3: review + advanced options + launch ────────────────────

  function seedScopeText(seed: ScanRequestSeed): string {
    const wallet = seed.wallet_profile
      ? `${familyLabel(seed.wallet_family)} · ${seed.wallet_profile}`
      : seed.wallet_family === "eth-watch"
        ? "saved watch addresses"
        : "all wallets";
    const provider = seed.provider_profile
      ? `provider ${seed.provider_profile}`
      : "all providers";
    return `${wallet} × ${provider}`;
  }

  function renderStepLaunch(content: HTMLElement): void {
    content.appendChild(
      el("p", {
        class: "stepper-lead",
        text: "Review the scope, then launch. A scan only reads: it asks the selected providers about public addresses.",
      }),
    );
    const plan = buildScanLaunchPlan({
      wallets: state.wallets,
      selectedWallets: state.stepper.selectedWallets,
      providers: state.providers.map((provider) => ({
        name: provider.name,
        chainId: provider.chain_id,
      })),
      selectedProviders: state.stepper.selectedProviders,
      includeWatchBook: state.stepper.includeWatchBook,
    });

    const summary = el("div", {
      class: "launch-summary",
      dataset: { portfolio: "launch-summary", tier: plan.ok ? "review" : "danger" },
    });
    if (plan.ok === false) {
      summary.appendChild(
        el("p", { class: "launch-summary-title", text: "This selection can't launch" }),
      );
      summary.appendChild(el("p", { class: "launch-summary-body", text: plan.reason }));
      content.appendChild(summary);
      return;
    }
    summary.appendChild(
      el("p", {
        class: "launch-summary-title",
        text:
          plan.scans.length === 1
            ? "One scan will start"
            : `${plan.scans.length} background scans will start`,
      }),
    );
    const list = el("ul", { class: "launch-summary-list" });
    for (const seed of plan.scans) {
      list.appendChild(el("li", { text: seedScopeText(seed) }));
    }
    summary.appendChild(list);
    if (state.stepper.partitionProviders && plan.allProviders) {
      summary.appendChild(
        el("p", {
          class: "launch-summary-body",
          text: "Provider partitioning is on: same-chain providers each see only a share of the addresses.",
        }),
      );
    }
    content.appendChild(summary);

    content.appendChild(
      checkRow(
        "run-async-check",
        state.stepper.runAsync,
        "Run in the background (recommended)",
        "The daemon scans as a cancelable background operation with live progress here. Unchecked, the request blocks until the scan finishes — fine for small scans.",
        (next) => {
          state.stepper.runAsync = next;
        },
      ),
    );

    content.appendChild(buildAdvancedOptions());
    restoreAdvancedFields();

    if (state.stepper.launchError) {
      content.appendChild(
        el("p", {
          class: "field-error",
          dataset: { portfolio: "launch-error" },
          text: state.stepper.launchError,
          attrs: { role: "alert" },
        }),
      );
    }
    const launchButton = el("button", {
      class: "btn-primary",
      text: state.stepper.launching
        ? "Starting…"
        : state.stepper.runAsync
          ? "Start background scan"
          : "Run scan now",
      attrs: { type: "button" },
      dataset: { portfolio: "launch", tier: "review" },
      on: { click: () => void launchScans() },
    }) as HTMLButtonElement;
    launchButton.disabled = state.stepper.launching;
    content.appendChild(launchButton);
  }

  function advancedField(
    name: string,
    labelText: string,
    kind: "text" | "number" | "checkbox" | "textarea",
    options: { value?: string; placeholder?: string; hint?: string } = {},
  ): HTMLElement {
    const id = `portfolioAdv_${name}`;
    const wrap = el("div", { class: "form-row advanced-field" });
    const label = el("label", {
      class: kind === "checkbox" ? "checkbox-row" : "advanced-label",
      attrs: { for: id },
    });
    let input: HTMLElement;
    if (kind === "textarea") {
      input = el("textarea", {
        attrs: { id, name, placeholder: options.placeholder ?? "" },
      });
    } else {
      input = el("input", {
        attrs: {
          id,
          name,
          type: kind,
          ...(options.placeholder ? { placeholder: options.placeholder } : {}),
        },
      });
      if (options.value !== undefined && kind !== "checkbox") {
        (input as HTMLInputElement).value = options.value;
      }
    }
    advancedFields[name] = input;
    if (kind === "checkbox") {
      label.appendChild(input);
      label.appendChild(el("span", { text: ` ${labelText}` }));
      wrap.appendChild(label);
    } else {
      label.appendChild(el("span", { text: labelText }));
      wrap.appendChild(label);
      wrap.appendChild(input);
    }
    if (options.hint) {
      wrap.appendChild(el("p", { class: "field-hint", text: options.hint }));
    }
    return wrap;
  }

  function buildAdvancedOptions(): HTMLElement {
    const details = el("details", {
      class: "advanced-options",
      dataset: { portfolio: "advanced-options" },
    });
    details.appendChild(
      el("summary", {
        text: "Advanced options — derivation, discovery probes, and block ranges",
      }),
    );
    details.appendChild(
      el("p", {
        class: "helper-text",
        text: "The exact options of the legacy scan form. Defaults match it; leave anything blank to use the daemon default.",
      }),
    );
    const grid = el("div", { class: "advanced-grid" });
    const patternWrap = el("div", { class: "form-row advanced-field" });
    const patternLabel = el("label", {
      class: "advanced-label",
      attrs: { for: "portfolioAdv_derivation_pattern" },
      text: "Seed derivation pattern",
    });
    const pattern = el("select", {
      attrs: { id: "portfolioAdv_derivation_pattern", name: "derivation_pattern" },
    }) as HTMLSelectElement;
    for (const [value, label] of [
      ["", "Daemon default (project account)"],
      ["project", "Project account"],
      ["standard", "Standard BIP-44 accounts"],
      ["ledger_live", "Ledger Live accounts"],
    ] as const) {
      const option = el("option", { text: label, attrs: { value } });
      pattern.appendChild(option);
    }
    advancedFields.derivation_pattern = pattern;
    patternWrap.appendChild(patternLabel);
    patternWrap.appendChild(pattern);
    grid.appendChild(patternWrap);
    grid.appendChild(advancedField("account_limit", "Account limit", "number", { value: "3" }));
    grid.appendChild(advancedField("gap_limit", "Gap limit", "number", { value: "20" }));
    grid.appendChild(advancedField("max_index", "Max address index", "number", { value: "200" }));
    grid.appendChild(
      advancedField("resume_from_latest_checkpoint", "Resume from the latest checkpoint", "checkbox", {
        hint: "Continue where the last scan stopped instead of starting over.",
      }),
    );
    grid.appendChild(
      advancedField("watch_addresses", "Extra watch addresses (one per line: address:label or address,label)", "textarea"),
    );
    grid.appendChild(advancedField("token_address", "ERC-20 token address to probe", "text"));
    grid.appendChild(
      advancedField("discover_erc20_transfers", "Discover ERC-20 transfer logs", "checkbox"),
    );
    grid.appendChild(
      advancedField("probe_token_registry", "Probe imported token registry", "checkbox"),
    );
    grid.appendChild(advancedField("token_discovery_from_block", "Token discovery from block", "text"));
    grid.appendChild(advancedField("token_discovery_to_block", "Token discovery to block", "text"));
    grid.appendChild(advancedField("token_discovery_limit", "Token discovery limit", "number", { value: "250" }));
    grid.appendChild(
      advancedField("discover_erc20_allowances", "Probe ERC-20 allowances", "checkbox", {
        hint: "An allowance lets a contract move your tokens — revoking unused ones reduces risk.",
      }),
    );
    grid.appendChild(advancedField("allowance_spender", "Allowance spender address", "text"));
    grid.appendChild(advancedField("allowance_discovery_limit", "Allowance probe limit", "number", { value: "250" }));
    grid.appendChild(
      advancedField("discover_permit2_allowances", "Probe Permit2 allowances", "checkbox"),
    );
    grid.appendChild(advancedField("permit2_contract", "Permit2 contract (blank = canonical)", "text"));
    grid.appendChild(advancedField("permit2_spender", "Permit2 spender address", "text"));
    grid.appendChild(advancedField("permit2_allowance_limit", "Permit2 probe limit", "number", { value: "250" }));
    grid.appendChild(
      advancedField("discover_erc721_transfers", "Discover ERC-721 transfers", "checkbox"),
    );
    grid.appendChild(
      advancedField("discover_erc1155_transfers", "Discover ERC-1155 transfers", "checkbox"),
    );
    grid.appendChild(
      advancedField("discover_nft_operator_approvals", "Probe NFT operator approvals", "checkbox"),
    );
    grid.appendChild(advancedField("nft_operator", "NFT operator address", "text"));
    grid.appendChild(advancedField("nft_operator_approval_limit", "NFT approval probe limit", "number", { value: "250" }));
    grid.appendChild(advancedField("nft_discovery_from_block", "NFT discovery from block", "text"));
    grid.appendChild(advancedField("nft_discovery_to_block", "NFT discovery to block", "text"));
    grid.appendChild(advancedField("nft_discovery_limit", "NFT discovery limit", "number", { value: "100" }));
    details.appendChild(grid);
    return details;
  }

  function fieldText(name: string): string {
    const field = advancedFields[name] as HTMLInputElement | undefined;
    return field?.value?.trim() ?? "";
  }

  function fieldNumber(name: string): number | null {
    const value = fieldText(name);
    if (!value) return null;
    const parsed = parseInt(value, 10);
    return Number.isFinite(parsed) ? parsed : null;
  }

  function fieldChecked(name: string): boolean {
    const field = advancedFields[name] as HTMLInputElement | undefined;
    return field?.checked ?? false;
  }

  function parseWatchProbes(bulk: string): { address: string; label?: string }[] {
    const probes: { address: string; label?: string }[] = [];
    const seen = new Set<string>();
    for (const line of bulk.split(/\r?\n/)) {
      const trimmed = line.trim();
      if (!trimmed || trimmed.startsWith("#")) continue;
      let address = trimmed;
      let label = "";
      const comma = trimmed.indexOf(",");
      const colon = trimmed.indexOf(":");
      const splitAt =
        comma >= 0 && (colon < 0 || comma < colon) ? comma : colon;
      // An EVM address contains no ':' or ',', so the first separator splits label.
      if (splitAt > 0) {
        address = trimmed.slice(0, splitAt).trim();
        label = trimmed.slice(splitAt + 1).trim();
      }
      if (!address) continue;
      const key = address.toLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      probes.push(label ? { address, label } : { address });
    }
    return probes;
  }

  /** The exact legacy scan DTO extras (legacy scanInventoryEvmRequest). */
  function collectAdvanced(): Partial<EvmScanRequest> {
    const token = fieldText("token_address");
    const spender = fieldText("allowance_spender");
    const permit2Contract = fieldText("permit2_contract");
    const permit2Spender = fieldText("permit2_spender");
    const nftOperator = fieldText("nft_operator");
    return {
      derivation_pattern: fieldText("derivation_pattern") || null,
      account_limit: fieldNumber("account_limit"),
      gap_limit: fieldNumber("gap_limit"),
      max_index: fieldNumber("max_index"),
      ...(fieldChecked("resume_from_latest_checkpoint")
        ? { resume_from_latest_checkpoint: true }
        : {}),
      watch_addresses: parseWatchProbes(fieldText("watch_addresses")),
      token_addresses: token ? [token] : [],
      block_tag: "latest",
      discover_erc20_transfers: fieldChecked("discover_erc20_transfers"),
      probe_token_registry: fieldChecked("probe_token_registry"),
      token_discovery_from_block: fieldText("token_discovery_from_block") || null,
      token_discovery_to_block: fieldText("token_discovery_to_block") || null,
      token_discovery_limit: fieldNumber("token_discovery_limit"),
      discover_erc20_allowances: fieldChecked("discover_erc20_allowances"),
      allowance_spender_addresses: spender ? [spender] : [],
      allowance_discovery_limit: fieldNumber("allowance_discovery_limit"),
      discover_permit2_allowances: fieldChecked("discover_permit2_allowances"),
      permit2_contract_addresses: permit2Contract ? [permit2Contract] : [],
      permit2_spender_addresses: permit2Spender ? [permit2Spender] : [],
      permit2_allowance_limit: fieldNumber("permit2_allowance_limit"),
      discover_erc721_transfers: fieldChecked("discover_erc721_transfers"),
      discover_erc1155_transfers: fieldChecked("discover_erc1155_transfers"),
      discover_nft_operator_approvals: fieldChecked(
        "discover_nft_operator_approvals",
      ),
      nft_operator_addresses: nftOperator ? [nftOperator] : [],
      nft_operator_approval_limit: fieldNumber("nft_operator_approval_limit"),
      nft_discovery_from_block: fieldText("nft_discovery_from_block") || null,
      nft_discovery_to_block: fieldText("nft_discovery_to_block") || null,
      nft_discovery_limit: fieldNumber("nft_discovery_limit"),
    };
  }

  /** Re-apply the last launch's advanced values after a step-3 rebuild (a
   * failed launch must not wipe the operator's tweaks). */
  function restoreAdvancedFields(): void {
    const saved = state.lastAdvanced;
    if (!saved) return;
    const setText = (name: string, value: string) => {
      const field = advancedFields[name] as HTMLInputElement | undefined;
      if (field) field.value = value;
    };
    const setCheck = (name: string, value: boolean) => {
      const field = advancedFields[name] as HTMLInputElement | undefined;
      if (field) field.checked = value;
    };
    const num = (value: number | null | undefined) =>
      value === null || value === undefined ? "" : String(value);
    setText("derivation_pattern", saved.derivation_pattern ?? "");
    setText("account_limit", num(saved.account_limit ?? 3));
    setText("gap_limit", num(saved.gap_limit ?? 20));
    setText("max_index", num(saved.max_index ?? 200));
    setCheck("resume_from_latest_checkpoint", saved.resume_from_latest_checkpoint === true);
    setText(
      "watch_addresses",
      (saved.watch_addresses ?? [])
        .map((probe) =>
          probe.label ? `${probe.address}:${probe.label}` : probe.address,
        )
        .join("\n"),
    );
    setText("token_address", saved.token_addresses?.[0] ?? "");
    setCheck("discover_erc20_transfers", saved.discover_erc20_transfers === true);
    setCheck("probe_token_registry", saved.probe_token_registry === true);
    setText("token_discovery_from_block", saved.token_discovery_from_block ?? "");
    setText("token_discovery_to_block", saved.token_discovery_to_block ?? "");
    setText("token_discovery_limit", num(saved.token_discovery_limit ?? 250));
    setCheck("discover_erc20_allowances", saved.discover_erc20_allowances === true);
    setText("allowance_spender", saved.allowance_spender_addresses?.[0] ?? "");
    setText("allowance_discovery_limit", num(saved.allowance_discovery_limit ?? 250));
    setCheck("discover_permit2_allowances", saved.discover_permit2_allowances === true);
    setText("permit2_contract", saved.permit2_contract_addresses?.[0] ?? "");
    setText("permit2_spender", saved.permit2_spender_addresses?.[0] ?? "");
    setText("permit2_allowance_limit", num(saved.permit2_allowance_limit ?? 250));
    setCheck("discover_erc721_transfers", saved.discover_erc721_transfers === true);
    setCheck("discover_erc1155_transfers", saved.discover_erc1155_transfers === true);
    setCheck(
      "discover_nft_operator_approvals",
      saved.discover_nft_operator_approvals === true,
    );
    setText("nft_operator", saved.nft_operator_addresses?.[0] ?? "");
    setText("nft_operator_approval_limit", num(saved.nft_operator_approval_limit ?? 250));
    setText("nft_discovery_from_block", saved.nft_discovery_from_block ?? "");
    setText("nft_discovery_to_block", saved.nft_discovery_to_block ?? "");
    setText("nft_discovery_limit", num(saved.nft_discovery_limit ?? 100));
  }

  function describeFailure(error: unknown): string {
    const failure = apiFailure(error);
    if (!failure) return failureText(error);
    if (failure.code === "validation_failed" && failure.fields?.length) {
      return failure.fields
        .map((field) => `${field.field}: ${field.message}`)
        .join("; ");
    }
    return failure.error;
  }

  async function launchScans(): Promise<void> {
    if (state.stepper.launching) return;
    const plan = buildScanLaunchPlan({
      wallets: state.wallets,
      selectedWallets: state.stepper.selectedWallets,
      providers: state.providers.map((provider) => ({
        name: provider.name,
        chainId: provider.chain_id,
      })),
      selectedProviders: state.stepper.selectedProviders,
      includeWatchBook: state.stepper.includeWatchBook,
    });
    if (plan.ok === false) {
      state.stepper.launchError = plan.reason;
      updateStepContent();
      return;
    }
    state.stepper.launching = true;
    state.stepper.launchError = null;
    updateStepContent();

    const advanced = collectAdvanced();
    state.lastAdvanced = advanced;
    const partitionApplies =
      state.stepper.partitionProviders && plan.allProviders;
    const bodies: EvmScanRequest[] = plan.scans.map((seed) => ({
      ...advanced,
      ...seed,
      ...(state.stepper.runAsync ? { run_async: true } : {}),
      ...(partitionApplies ? { partition_providers: true } : {}),
    }));

    const operationIds: string[] = [];
    const jobIds: string[] = [];
    const syncResults: StepperState["syncResults"] = [];
    let firstError: string | null = null;
    // Sequential launch: keeps provider burst load to one scan at a time.
    for (const body of bodies) {
      try {
        const result = await daemonRequest<EvmScanResponseWire>(
          "POST",
          "/api/inventory/scan/evm",
          body,
        );
        if (result.operation?.id) operationIds.push(result.operation.id);
        if (result.job?.id) jobIds.push(result.job.id);
        if (!result.operation) {
          syncResults.push({
            jobId: result.job?.id ?? null,
            status: result.job?.status ?? "completed",
            summary: jobResultSummary(result.job),
          });
        }
      } catch (error) {
        const failure = apiFailure(error);
        if (isLockFailure(failure)) {
          state.locked = true;
          state.stepper.launching = false;
          renderView();
          return;
        }
        firstError = firstError ?? describeFailure(error);
      }
    }
    state.stepper.launchedOperationIds = operationIds;
    state.stepper.launchedJobIds = jobIds;
    state.stepper.syncResults = syncResults;
    state.stepper.launchError = firstError;
    state.stepper.launching = false;
    state.stepper.step = 4;
    void loadPortfolio();
    renderView();
  }

  function jobResultSummary(job: DiscoveryJobRecord | undefined): string {
    if (!job) return "scan finished";
    return (
      `${(job.addresses_scanned ?? 0).toLocaleString()} addresses checked · ` +
      `${(job.active_addresses ?? 0).toLocaleString()} with activity · ` +
      `${(job.holdings_detected ?? 0).toLocaleString()} holdings found`
    );
  }


  // ── Risk view (#/portfolio/risk) ──────────────────────────────────

  function renderFieldErrors(
    container: HTMLElement,
    fields: { field: string; message: string }[],
    form?: HTMLElement,
  ): void {
    clearContainer(container);
    for (const field of fields) {
      container.appendChild(
        el("p", {
          class: "field-error",
          text: `${field.field}: ${field.message}`,
          attrs: { role: "alert" },
        }),
      );
      if (form) {
        const input = form.querySelector(`[name="${field.field}"]`);
        input?.setAttribute("aria-invalid", "true");
      }
    }
  }

  function clearFieldInvalid(form: HTMLElement): void {
    const walk = (node: HTMLElement): void => {
      for (const child of Array.from(node.childNodes)) {
        const element = child as HTMLElement;
        element.removeAttribute?.("aria-invalid");
        walk(element);
      }
    };
    walk(form);
  }

  function buildRiskShell(): void {
    const viewRoot = state.viewRoot;
    if (!viewRoot) return;
    clearContainer(viewRoot);
    state.refs = {};

    viewRoot.appendChild(
      pageHeader(
        "What looks risky?",
        "Findings from the local risk engine — approval exposure, stranded value, watch-only gaps, and privacy linkages — plus your own address labels.",
      ),
    );
    viewRoot.appendChild(renderNav("risk"));
    const statusLine = el("p", {
      class: "dest-status",
      dataset: { portfolio: "status" },
      attrs: { "aria-live": "polite" },
    });
    viewRoot.appendChild(statusLine);
    state.statusLine = statusLine;

    // Findings section
    const findingsSection = el("section", { class: "dest-section" });
    const findingsHead = el("div", { class: "dest-section-head" });
    findingsHead.appendChild(el("h3", { class: "section-title", text: "Findings" }));
    const findingsCount = el("span", {
      class: "dest-count",
      dataset: { portfolio: "findings-count" },
    });
    findingsHead.appendChild(findingsCount);
    findingsSection.appendChild(findingsHead);

    const filterBar = el("div", {
      class: "filter-bar",
      dataset: { portfolio: "risk-filter-bar" },
    });
    filterBar.appendChild(
      buildSelect(
        "risk-filter-severity",
        "Severity",
        [
          { value: "", label: "All severities" },
          { value: "critical", label: "Critical" },
          { value: "high", label: "High" },
          { value: "medium", label: "Medium" },
          { value: "low", label: "Low" },
          { value: "trusted", label: "Trusted" },
        ],
        state.riskFilters.severity ?? "",
        (value) => {
          state.riskFilters.severity = value || null;
          state.riskFilters.offset = 0;
          void loadRisk();
        },
      ),
    );
    filterBar.appendChild(
      buildSelect(
        "risk-filter-sort",
        "Sort",
        [
          { value: "severity", label: "Most severe first" },
          { value: "found_at", label: "Newest first" },
        ],
        state.riskFilters.sort,
        (value) => {
          state.riskFilters.sort = value === "found_at" ? "found_at" : "severity";
          state.riskFilters.order = "desc";
          state.riskFilters.offset = 0;
          void loadRisk();
        },
      ),
    );
    findingsSection.appendChild(filterBar);

    const findingsEmpty = el("div", { dataset: { portfolio: "findings-empty" } });
    findingsSection.appendChild(findingsEmpty);
    const findingsWrap = el("div", {
      class: "table-scroll",
      dataset: { portfolio: "findings-wrap" },
    });
    const table = el("table", { class: "table compact" });
    const thead = el("thead");
    const headRow = el("tr");
    for (const title of ["Severity", "Finding", "Subject", "Seen"]) {
      headRow.appendChild(el("th", { text: title }));
    }
    thead.appendChild(headRow);
    table.appendChild(thead);
    const findingsBody = el("tbody", { dataset: { portfolio: "findings-body" } });
    table.appendChild(findingsBody);
    findingsWrap.appendChild(table);
    findingsSection.appendChild(findingsWrap);

    const pagination = el("div", {
      class: "dest-pagination",
      dataset: { portfolio: "findings-pagination" },
    });
    const prevButton = el("button", {
      class: "btn-ghost btn-small",
      text: "Previous",
      attrs: { type: "button" },
      on: {
        click: () => {
          state.riskFilters.offset = Math.max(
            0,
            state.riskFilters.offset - state.riskFilters.limit,
          );
          void loadRisk();
        },
      },
    });
    const pageLabel = el("span", {
      class: "dest-page-label nums",
      dataset: { portfolio: "findings-page-label" },
    });
    const nextButton = el("button", {
      class: "btn-ghost btn-small",
      text: "Next",
      attrs: { type: "button" },
      on: {
        click: () => {
          state.riskFilters.offset += state.riskFilters.limit;
          void loadRisk();
        },
      },
    });
    pagination.appendChild(prevButton);
    pagination.appendChild(pageLabel);
    pagination.appendChild(nextButton);
    findingsSection.appendChild(pagination);
    viewRoot.appendChild(findingsSection);

    // Catalog section
    const catalogSection = el("section", { class: "dest-section" });
    const catalogHead = el("div", { class: "dest-section-head" });
    catalogHead.appendChild(
      el("h3", { class: "section-title", text: "Your risk labels" }),
    );
    catalogSection.appendChild(catalogHead);
    catalogSection.appendChild(
      el("p", {
        class: "helper-text",
        text: "Label spender or operator addresses you recognize. Labels steer the findings above; everything stays local.",
      }),
    );
    catalogSection.appendChild(buildCatalogForm());
    const catalogEmpty = el("div", { dataset: { portfolio: "catalog-empty" } });
    catalogSection.appendChild(catalogEmpty);
    const catalogWrap = el("div", {
      class: "table-scroll",
      dataset: { portfolio: "catalog-wrap" },
    });
    const catalogTable = el("table", { class: "table compact" });
    const catalogThead = el("thead");
    const catalogHeadRow = el("tr");
    for (const title of ["Address", "Label", "Level", "Source", "Updated", "Actions"]) {
      catalogHeadRow.appendChild(el("th", { text: title }));
    }
    catalogThead.appendChild(catalogHeadRow);
    catalogTable.appendChild(catalogThead);
    const catalogBody = el("tbody", { dataset: { portfolio: "catalog-body" } });
    catalogTable.appendChild(catalogBody);
    catalogWrap.appendChild(catalogTable);
    catalogSection.appendChild(catalogWrap);
    viewRoot.appendChild(catalogSection);

    state.refs = {
      findingsCount,
      findingsEmpty,
      findingsWrap,
      findingsBody,
      findingsPrev: prevButton,
      findingsNext: nextButton,
      findingsPageLabel: pageLabel,
      catalogEmpty,
      catalogWrap,
      catalogBody,
    };
  }

  function buildCatalogForm(): HTMLElement {
    const form = el("form", {
      class: "dest-form",
      dataset: { portfolio: "catalog-form" },
      attrs: { novalidate: "" },
    });
    const errors = el("div", { dataset: { portfolio: "catalog-errors" } });
    form.appendChild(errors);
    const row = el("div", { class: "form-row" });
    const address = el("input", {
      attrs: { type: "text", name: "address", placeholder: "Spender/operator address", "aria-label": "Address" },
    });
    const label = el("input", {
      attrs: { type: "text", name: "label", placeholder: "Label (optional)", "aria-label": "Label" },
    });
    const level = el("select", {
      attrs: { name: "risk_level", "aria-label": "Risk level" },
    }) as HTMLSelectElement;
    for (const value of ["trusted", "low", "medium", "high", "critical"]) {
      level.appendChild(el("option", { text: value, attrs: { value } }));
    }
    level.value = "trusted";
    const note = el("input", {
      attrs: { type: "text", name: "note", placeholder: "Note (optional)", "aria-label": "Note" },
    });
    row.appendChild(address);
    row.appendChild(label);
    row.appendChild(level);
    row.appendChild(note);
    row.appendChild(
      el("button", {
        class: "btn-primary",
        text: "Save label",
        attrs: { type: "submit" },
        dataset: { portfolio: "catalog-save" },
      }),
    );
    form.appendChild(row);
    form.addEventListener("submit", (event) => {
      (event as Event).preventDefault?.();
      void submitCatalogForm(form, errors);
    });
    return form;
  }

  async function submitCatalogForm(
    form: HTMLElement,
    errors: HTMLElement,
  ): Promise<void> {
    clearFieldInvalid(form);
    clearContainer(errors);
    const read = (name: string) =>
      (form.querySelector(`[name="${name}"]`) as HTMLInputElement | null)?.value.trim() ?? "";
    const address = read("address");
    const riskLevel =
      (form.querySelector('[name="risk_level"]') as HTMLSelectElement | null)
        ?.value ?? "";
    if (!address) {
      renderFieldErrors(errors, [{ field: "address", message: "An address is required" }], form);
      return;
    }
    const note = read("note");
    try {
      await daemonRequest<MutationStatusWire>("POST", "/api/risk/catalog/upsert", {
        address,
        label: read("label") || null,
        risk_level: riskLevel,
        notes: note ? [note] : [],
      });
      setStatus("Risk label saved.");
      (form.querySelector('[name="address"]') as HTMLInputElement).value = "";
      (form.querySelector('[name="label"]') as HTMLInputElement).value = "";
      (form.querySelector('[name="note"]') as HTMLInputElement).value = "";
      void loadRisk();
    } catch (error) {
      const failure = apiFailure(error);
      if (isLockFailure(failure)) {
        state.locked = true;
        renderView();
        return;
      }
      if (failure?.code === "validation_failed" && failure.fields?.length) {
        renderFieldErrors(errors, failure.fields, form);
        return;
      }
      renderFieldErrors(errors, [{ field: "form", message: failureText(error) }]);
    }
  }

  function findingRow(
    finding: RiskFinding,
    existing: HTMLElement | null,
  ): HTMLElement {
    if (existing) return existing;
    const tier = severityTier(finding.risk_level);
    const row = el("tr", {
      dataset: { portfolio: "finding-row", ...(tier ? { tier } : {}) },
    });
    const severityTd = el("td");
    severityTd.appendChild(
      pill(
        (finding.risk_level || "unknown").replace(/_/g, " "),
        pillClass(finding.risk_level),
      ),
    );
    row.appendChild(severityTd);
    const findingTd = el("td", { class: "cell-wrap" });
    findingTd.appendChild(
      el("div", {
        class: "cell-primary",
        text:
          finding.category === "common_gas_funder"
            ? "One gas funder pays into several payer identities"
            : riskCategoryLabel(finding.category),
      }),
    );
    if (finding.recommendation) {
      findingTd.appendChild(
        el("div", { class: "cell-secondary", text: finding.recommendation }),
      );
    }
    if ((finding.evidence ?? []).length) {
      const details = el("details", { class: "row-details" });
      details.appendChild(el("summary", { text: "Evidence" }));
      const list = el("ul", { class: "detail-list-plain" });
      for (const line of finding.evidence ?? []) {
        list.appendChild(el("li", { text: line }));
      }
      details.appendChild(list);
      findingTd.appendChild(details);
    }
    row.appendChild(findingTd);
    const subjectTd = el("td");
    subjectTd.appendChild(
      el("div", {
        class: "cell-primary",
        text: subjectTypeLabel(finding.subject_type),
      }),
    );
    subjectTd.appendChild(
      el("div", {
        class: "mono cell-secondary",
        text: middleTruncate(finding.subject),
        attrs: { title: finding.subject },
      }),
    );
    subjectTd.appendChild(
      el("div", { class: "cell-secondary", text: chainName(finding.chain_id) }),
    );
    row.appendChild(subjectTd);
    row.appendChild(
      el("td", {
        class: "nums",
        text: relativeTime(finding.first_seen_at_unix, nowSecs()),
        attrs: {
          title: `First seen ${formatTimestamp(finding.first_seen_at_unix)}`,
        },
      }),
    );
    return row;
  }

  function updateFindingsSection(): void {
    const refs = state.refs;
    if (!refs.findingsBody) return;
    const loading = state.load.risk === "loading";
    const firstLoad = !state.everLoaded.risk;
    if (loading && firstLoad) {
      clearContainer(refs.findingsEmpty!);
      refs.findingsEmpty!.appendChild(skeletonRows(3));
      setHidden(refs.findingsEmpty!, false);
      setHidden(refs.findingsWrap!, true);
      return;
    }
    if (state.load.risk === "error" && firstLoad) {
      clearContainer(refs.findingsEmpty!);
      refs.findingsEmpty!.appendChild(
        sectionEmpty(
          "Couldn't load risk findings",
          state.stale.risk ?? "The daemon did not answer.",
          { label: "Retry", onClick: () => void loadRisk() },
        ),
      );
      setHidden(refs.findingsEmpty!, false);
      setHidden(refs.findingsWrap!, true);
      return;
    }
    const total = state.findingsPagination?.total ?? state.findings.length;
    refs.findingsCount!.textContent = `${total} finding${total === 1 ? "" : "s"}`;
    if (!state.findings.length) {
      clearContainer(refs.findingsEmpty!);
      refs.findingsEmpty!.appendChild(
        sectionEmpty(
          state.riskFilters.severity
            ? "No findings at this severity"
            : "No risk findings",
          state.riskFilters.severity
            ? "Try another severity, or clear the filter."
            : "The local risk engine found nothing to flag in the current inventory. Findings appear after scans detect exposure.",
          state.riskFilters.severity
            ? undefined
            : { label: "Run a scan", href: formatHash("portfolio", "scan") },
        ),
      );
      setHidden(refs.findingsEmpty!, false);
      setHidden(refs.findingsWrap!, true);
    } else {
      clearContainer(refs.findingsEmpty!);
      setHidden(refs.findingsEmpty!, true);
      setHidden(refs.findingsWrap!, false);
      renderList(refs.findingsBody!, state.findings, (f) => f.id, findingRow);
    }
    const page = state.findingsPagination;
    if (page) {
      const from = page.total === 0 ? 0 : state.riskFilters.offset + 1;
      const to = state.riskFilters.offset + state.findings.length;
      refs.findingsPageLabel!.textContent = `${from}–${to} of ${page.total}`;
      (refs.findingsPrev as HTMLButtonElement).disabled =
        state.riskFilters.offset === 0;
      (refs.findingsNext as HTMLButtonElement).disabled = !page.has_more;
    } else {
      refs.findingsPageLabel!.textContent = state.findings.length
        ? `1–${state.findings.length} of ${state.findings.length}`
        : "";
      (refs.findingsPrev as HTMLButtonElement).disabled = true;
      (refs.findingsNext as HTMLButtonElement).disabled = true;
    }
  }

  function catalogRow(
    entry: RiskCatalogEntry,
    existing: HTMLElement | null,
  ): HTMLElement {
    if (existing) return existing;
    const row = el("tr", { dataset: { portfolio: "catalog-row" } });
    const addressTd = el("td");
    addressTd.appendChild(
      el("span", {
        class: "mono",
        text: middleTruncate(entry.address),
        attrs: { title: entry.address },
      }),
    );
    row.appendChild(addressTd);
    row.appendChild(el("td", { text: entry.label || "-" }));
    const levelTd = el("td");
    levelTd.appendChild(pill(entry.risk_level || "unknown"));
    row.appendChild(levelTd);
    row.appendChild(el("td", { text: entry.source || "-" }));
    row.appendChild(
      el("td", {
        class: "nums",
        text: relativeTime(entry.updated_at_unix, nowSecs()),
        attrs: { title: formatTimestamp(entry.updated_at_unix) },
      }),
    );
    const actionsTd = el("td", { class: "col-actions" });
    actionsTd.appendChild(
      el("button", {
        class: "btn-ghost btn-small",
        text: "Delete",
        attrs: { type: "button" },
        dataset: { portfolio: "catalog-delete" },
        on: { click: () => void deleteCatalogEntry(entry.address) },
      }),
    );
    row.appendChild(actionsTd);
    return row;
  }

  async function deleteCatalogEntry(address: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Delete risk label",
      body: `Delete the risk label for "${address}"? Findings derived from it disappear from the risk view.`,
      actionLabel: "Delete",
    });
    if (!confirmed) return;
    try {
      await daemonRequest<MutationStatusWire>("POST", "/api/risk/catalog/delete", {
        address,
      });
      setStatus("Risk label deleted.");
      void loadRisk();
    } catch (error) {
      const failure = apiFailure(error);
      if (isLockFailure(failure)) {
        state.locked = true;
        renderView();
        return;
      }
      setStatus(`Couldn't delete the label: ${failureText(error)}`);
    }
  }

  function updateCatalogSection(): void {
    const refs = state.refs;
    if (!refs.catalogBody) return;
    if (!state.catalog.length) {
      clearContainer(refs.catalogEmpty!);
      refs.catalogEmpty!.appendChild(
        sectionEmpty(
          "No risk labels yet",
          "Label an address above — for example mark a known exchange spender as trusted, or a suspicious operator as high risk.",
        ),
      );
      setHidden(refs.catalogEmpty!, false);
      setHidden(refs.catalogWrap!, true);
      return;
    }
    clearContainer(refs.catalogEmpty!);
    setHidden(refs.catalogEmpty!, true);
    setHidden(refs.catalogWrap!, false);
    renderList(refs.catalogBody!, state.catalog, (entry) => entry.address, catalogRow);
  }

  // ── Tokens view (#/portfolio/tokens) ──────────────────────────────

  function buildTokensShell(): void {
    const viewRoot = state.viewRoot;
    if (!viewRoot) return;
    clearContainer(viewRoot);
    state.refs = {};

    viewRoot.appendChild(
      pageHeader(
        "Tokens and NFT metadata",
        "Local token registries that give amounts their units, and the per-collection opt-ins that control NFT metadata fetching.",
      ),
    );
    viewRoot.appendChild(renderNav("tokens"));
    const statusLine = el("p", {
      class: "dest-status",
      dataset: { portfolio: "status" },
      attrs: { "aria-live": "polite" },
    });
    viewRoot.appendChild(statusLine);
    state.statusLine = statusLine;

    // Token registry
    const registrySection = el("section", { class: "dest-section" });
    registrySection.appendChild(
      el("h3", { class: "section-title", text: "Token registry (local import)" }),
    );
    registrySection.appendChild(
      el("p", {
        class: "helper-text",
        text: "Token lists are imported from pasted JSON or a local file only — Sigillum never fetches lists from the network.",
      }),
    );
    registrySection.appendChild(buildRegistryForm());
    const registryEmpty = el("div", { dataset: { portfolio: "registry-empty" } });
    registrySection.appendChild(registryEmpty);
    const registryWrap = el("div", {
      class: "table-scroll",
      dataset: { portfolio: "registry-wrap" },
    });
    const registryTable = el("table", { class: "table compact" });
    const registryThead = el("thead");
    const registryHeadRow = el("tr");
    for (const title of ["List", "Entries", "Chains", "Source", "Updated", "Actions"]) {
      registryHeadRow.appendChild(el("th", { text: title }));
    }
    registryThead.appendChild(registryHeadRow);
    registryTable.appendChild(registryThead);
    const registryBody = el("tbody", { dataset: { portfolio: "registry-body" } });
    registryTable.appendChild(registryBody);
    registryWrap.appendChild(registryTable);
    registrySection.appendChild(registryWrap);
    viewRoot.appendChild(registrySection);

    // NFT metadata
    const nftSection = el("section", { class: "dest-section" });
    nftSection.appendChild(
      el("h3", { class: "section-title", text: "NFT metadata (opt-in)" }),
    );
    nftSection.appendChild(
      el("p", {
        class: "helper-text",
        text: "Fetching NFT metadata contacts external servers, like RPC calls do: the metadata host and any IPFS gateway learn your interest in a collection. Nothing is fetched unless you opt a collection in.",
      }),
    );
    nftSection.appendChild(buildOptInForm());
    const optInEmpty = el("div", { dataset: { portfolio: "optin-empty" } });
    nftSection.appendChild(optInEmpty);
    const optInWrap = el("div", {
      class: "table-scroll",
      dataset: { portfolio: "optin-wrap" },
    });
    const optInTable = el("table", { class: "table compact" });
    const optInThead = el("thead");
    const optInHeadRow = el("tr");
    for (const title of ["Collection", "Chain", "Status", "Updated", "Actions"]) {
      optInHeadRow.appendChild(el("th", { text: title }));
    }
    optInThead.appendChild(optInHeadRow);
    optInTable.appendChild(optInThead);
    const optInBody = el("tbody", { dataset: { portfolio: "optin-body" } });
    optInTable.appendChild(optInBody);
    optInWrap.appendChild(optInTable);
    nftSection.appendChild(optInWrap);

    nftSection.appendChild(buildGatewayForm());

    const cacheEmpty = el("div", { dataset: { portfolio: "nftcache-empty" } });
    nftSection.appendChild(cacheEmpty);
    const cacheWrap = el("div", {
      class: "table-scroll",
      dataset: { portfolio: "nftcache-wrap" },
    });
    const cacheTable = el("table", { class: "table compact" });
    const cacheThead = el("thead");
    const cacheHeadRow = el("tr");
    for (const title of ["Name", "Collection", "Chain", "Flag", "Fetched"]) {
      cacheHeadRow.appendChild(el("th", { text: title }));
    }
    cacheThead.appendChild(cacheHeadRow);
    cacheTable.appendChild(cacheThead);
    const cacheBody = el("tbody", { dataset: { portfolio: "nftcache-body" } });
    cacheTable.appendChild(cacheBody);
    cacheWrap.appendChild(cacheTable);
    nftSection.appendChild(cacheWrap);
    viewRoot.appendChild(nftSection);

    state.refs = {
      registryEmpty,
      registryWrap,
      registryBody,
      optInEmpty,
      optInWrap,
      optInBody,
      cacheEmpty,
      cacheWrap,
      cacheBody,
    };
  }

  function buildRegistryForm(): HTMLElement {
    const form = el("form", {
      class: "dest-form",
      dataset: { portfolio: "registry-form" },
      attrs: { novalidate: "" },
    });
    const errors = el("div", { dataset: { portfolio: "registry-errors" } });
    form.appendChild(errors);
    const row = el("div", { class: "form-row" });
    row.appendChild(
      el("input", {
        attrs: { type: "text", name: "name", placeholder: "List name", "aria-label": "List name" },
      }),
    );
    row.appendChild(
      el("input", {
        attrs: {
          type: "text",
          name: "file_path",
          placeholder: "Local file path on daemon host (optional)",
          "aria-label": "Local file path",
        },
      }),
    );
    row.appendChild(
      el("button", {
        class: "btn-primary",
        text: "Import list",
        attrs: { type: "submit" },
        dataset: { portfolio: "registry-import" },
      }),
    );
    form.appendChild(row);
    form.appendChild(
      el("textarea", {
        attrs: {
          name: "entries_json",
          placeholder:
            'Pasted JSON entries: [{"chain_id":1,"address":"0x...","symbol":"USDC","decimals":6}]',
          "aria-label": "Pasted JSON entries",
        },
      }),
    );
    form.addEventListener("submit", (event) => {
      (event as Event).preventDefault?.();
      void submitRegistryForm(form, errors);
    });
    return form;
  }

  async function submitRegistryForm(
    form: HTMLElement,
    errors: HTMLElement,
  ): Promise<void> {
    clearFieldInvalid(form);
    clearContainer(errors);
    const read = (name: string) =>
      (
        form.querySelector(`[name="${name}"]`) as
          | HTMLInputElement
          | HTMLTextAreaElement
          | null
      )?.value.trim() ?? "";
    const name = read("name");
    const entriesJson = read("entries_json");
    const filePath = read("file_path");
    if (!name) {
      renderFieldErrors(errors, [{ field: "name", message: "A list name is required" }], form);
      return;
    }
    if ((entriesJson ? 1 : 0) + (filePath ? 1 : 0) !== 1) {
      renderFieldErrors(errors, [
        {
          field: "entries_json",
          message: "Provide pasted JSON entries or a local file path (not both)",
        },
      ]);
      return;
    }
    try {
      await daemonRequest<MutationStatusWire>(
        "POST",
        "/api/inventory/token-registry/import",
        {
          name,
          entries_json: entriesJson || undefined,
          file_path: filePath || undefined,
        },
      );
      setStatus("Token registry list imported — amounts now humanize with its units.");
      (form.querySelector('[name="name"]') as HTMLInputElement).value = "";
      (form.querySelector('[name="file_path"]') as HTMLInputElement).value = "";
      (form.querySelector('[name="entries_json"]') as HTMLTextAreaElement).value = "";
      void loadTokens();
    } catch (error) {
      const failure = apiFailure(error);
      if (isLockFailure(failure)) {
        state.locked = true;
        renderView();
        return;
      }
      if (failure?.code === "validation_failed" && failure.fields?.length) {
        renderFieldErrors(errors, failure.fields, form);
        return;
      }
      renderFieldErrors(errors, [{ field: "form", message: failureText(error) }]);
    }
  }

  function registryRow(
    list: TokenRegistryList,
    existing: HTMLElement | null,
  ): HTMLElement {
    if (existing) return existing;
    const row = el("tr", { dataset: { portfolio: "registry-row" } });
    row.appendChild(el("td", { text: list.name }));
    row.appendChild(
      el("td", { class: "nums", text: String((list.entries ?? []).length) }),
    );
    const chains = Array.from(
      new Set((list.entries ?? []).map((entry) => entry.chain_id)),
    ).sort((a, b) => a - b);
    row.appendChild(
      el("td", {
        text: chains.length ? chains.map((id) => chainName(id)).join(", ") : "-",
      }),
    );
    row.appendChild(el("td", { text: list.source || "-" }));
    row.appendChild(
      el("td", {
        class: "nums",
        text: relativeTime(list.updated_at_unix, nowSecs()),
        attrs: { title: formatTimestamp(list.updated_at_unix) },
      }),
    );
    const actionsTd = el("td", { class: "col-actions" });
    actionsTd.appendChild(
      el("button", {
        class: "btn-ghost btn-small",
        text: "Delete",
        attrs: { type: "button" },
        dataset: { portfolio: "registry-delete" },
        on: { click: () => void deleteRegistryList(list.name) },
      }),
    );
    row.appendChild(actionsTd);
    return row;
  }

  async function deleteRegistryList(name: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Delete token registry list",
      body: `Delete token registry list "${name}"? Its token metadata is removed from local inventory views.`,
      actionLabel: "Delete",
    });
    if (!confirmed) return;
    try {
      await daemonRequest<MutationStatusWire>(
        "POST",
        "/api/inventory/token-registry/delete",
        { name },
      );
      setStatus("Token registry list deleted.");
      void loadTokens();
    } catch (error) {
      const failure = apiFailure(error);
      if (isLockFailure(failure)) {
        state.locked = true;
        renderView();
        return;
      }
      setStatus(`Couldn't delete the list: ${failureText(error)}`);
    }
  }

  function updateRegistrySection(): void {
    const refs = state.refs;
    if (!refs.registryBody) return;
    if (!state.tokenLists.length) {
      clearContainer(refs.registryEmpty!);
      refs.registryEmpty!.appendChild(
        sectionEmpty(
          "No token lists imported yet",
          "Without a registry, token amounts stay in raw units. Import a list to read balances in the units you recognize.",
        ),
      );
      setHidden(refs.registryEmpty!, false);
      setHidden(refs.registryWrap!, true);
      return;
    }
    clearContainer(refs.registryEmpty!);
    setHidden(refs.registryEmpty!, true);
    setHidden(refs.registryWrap!, false);
    renderList(refs.registryBody!, state.tokenLists, (list) => list.name, registryRow);
  }

  function buildOptInForm(): HTMLElement {
    const form = el("form", {
      class: "dest-form",
      dataset: { portfolio: "optin-form" },
      attrs: { novalidate: "" },
    });
    const errors = el("div", { dataset: { portfolio: "optin-errors" } });
    form.appendChild(errors);
    const row = el("div", { class: "form-row" });
    const chain = el("select", {
      attrs: { name: "chain_id", "aria-label": "Chain" },
    }) as HTMLSelectElement;
    const chainOptions = state.chains.filter(
      (profile) => profile.enabled && profile.chain_id != null,
    );
    if (!chainOptions.length) {
      chain.appendChild(el("option", { text: "1 (not configured)", attrs: { value: "1" } }));
    }
    for (const profile of chainOptions) {
      chain.appendChild(
        el("option", {
          text: profile.name,
          attrs: { value: String(profile.chain_id) },
        }),
      );
    }
    chain.value = String(chainOptions[0]?.chain_id ?? 1);
    row.appendChild(chain);
    row.appendChild(
      el("input", {
        attrs: {
          type: "text",
          name: "contract_address",
          placeholder: "Collection contract address",
          "aria-label": "Collection contract address",
        },
      }),
    );
    row.appendChild(
      el("button", {
        class: "btn-primary",
        text: "Opt in collection",
        attrs: { type: "submit" },
        dataset: { portfolio: "optin-save" },
      }),
    );
    form.appendChild(row);
    form.addEventListener("submit", (event) => {
      (event as Event).preventDefault?.();
      void submitOptInForm(form, errors);
    });
    return form;
  }

  async function submitOptInForm(
    form: HTMLElement,
    errors: HTMLElement,
  ): Promise<void> {
    clearFieldInvalid(form);
    clearContainer(errors);
    const chainValue =
      (form.querySelector('[name="chain_id"]') as HTMLSelectElement | null)?.value ??
      "";
    const chainId = Number(chainValue);
    const contract =
      (
        form.querySelector('[name="contract_address"]') as HTMLInputElement | null
      )?.value.trim() ?? "";
    if (!Number.isFinite(chainId) || !contract) {
      renderFieldErrors(
        errors,
        [
          ...(Number.isFinite(chainId)
            ? []
            : [{ field: "chain_id", message: "Pick a chain" }]),
          ...(contract
            ? []
            : [{ field: "contract_address", message: "A contract address is required" }]),
        ],
        form,
      );
      return;
    }
    try {
      await daemonRequest<MutationStatusWire>(
        "POST",
        "/api/inventory/nft-metadata/opt-ins/upsert",
        { chain_id: chainId, contract_address: contract, enabled: true },
      );
      setStatus("Collection opted in — metadata is fetched only for opted-in collections.");
      (form.querySelector('[name="contract_address"]') as HTMLInputElement).value = "";
      void loadTokens();
    } catch (error) {
      const failure = apiFailure(error);
      if (isLockFailure(failure)) {
        state.locked = true;
        renderView();
        return;
      }
      if (failure?.code === "validation_failed" && failure.fields?.length) {
        renderFieldErrors(errors, failure.fields, form);
        return;
      }
      renderFieldErrors(errors, [{ field: "form", message: failureText(error) }]);
    }
  }

  function optInRow(
    optIn: NftMetadataCollectionOptIn,
    existing: HTMLElement | null,
  ): HTMLElement {
    if (existing) return existing;
    const row = el("tr", { dataset: { portfolio: "optin-row" } });
    const collectionTd = el("td");
    collectionTd.appendChild(
      el("span", {
        class: "mono",
        text: middleTruncate(optIn.contract_address),
        attrs: { title: optIn.contract_address },
      }),
    );
    row.appendChild(collectionTd);
    row.appendChild(el("td", { text: chainName(optIn.chain_id) }));
    const statusTd = el("td");
    statusTd.appendChild(pill(optIn.enabled ? "enabled" : "disabled"));
    row.appendChild(statusTd);
    row.appendChild(
      el("td", {
        class: "nums",
        text: relativeTime(optIn.updated_at_unix, nowSecs()),
        attrs: { title: formatTimestamp(optIn.updated_at_unix) },
      }),
    );
    const actionsTd = el("td", { class: "col-actions" });
    actionsTd.appendChild(
      el("button", {
        class: "btn-ghost btn-small",
        text: optIn.enabled ? "Disable" : "Enable",
        attrs: { type: "button" },
        dataset: { portfolio: "optin-toggle" },
        on: { click: () => void toggleOptIn(optIn) },
      }),
    );
    actionsTd.appendChild(
      el("button", {
        class: "btn-ghost btn-small",
        text: "Delete",
        attrs: { type: "button" },
        dataset: { portfolio: "optin-delete" },
        on: { click: () => void deleteOptIn(optIn) },
      }),
    );
    row.appendChild(actionsTd);
    return row;
  }

  async function toggleOptIn(optIn: NftMetadataCollectionOptIn): Promise<void> {
    try {
      await daemonRequest<MutationStatusWire>(
        "POST",
        "/api/inventory/nft-metadata/opt-ins/upsert",
        {
          chain_id: optIn.chain_id,
          contract_address: optIn.contract_address,
          enabled: !optIn.enabled,
        },
      );
      setStatus(optIn.enabled ? "Collection disabled." : "Collection enabled.");
      void loadTokens();
    } catch (error) {
      const failure = apiFailure(error);
      if (isLockFailure(failure)) {
        state.locked = true;
        renderView();
        return;
      }
      setStatus(`Couldn't update the opt-in: ${failureText(error)}`);
    }
  }

  async function deleteOptIn(optIn: NftMetadataCollectionOptIn): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Delete NFT metadata opt-in",
      body: `Delete the opt-in for "${optIn.contract_address}"? Cached metadata for this collection is dropped and no longer fetched.`,
      actionLabel: "Delete",
    });
    if (!confirmed) return;
    try {
      await daemonRequest<MutationStatusWire>(
        "POST",
        "/api/inventory/nft-metadata/opt-ins/delete",
        { chain_id: optIn.chain_id, contract_address: optIn.contract_address },
      );
      setStatus("Opt-in deleted.");
      void loadTokens();
    } catch (error) {
      const failure = apiFailure(error);
      if (isLockFailure(failure)) {
        state.locked = true;
        renderView();
        return;
      }
      setStatus(`Couldn't delete the opt-in: ${failureText(error)}`);
    }
  }

  function updateOptInSection(): void {
    const refs = state.refs;
    if (!refs.optInBody) return;
    if (!state.optIns.length) {
      clearContainer(refs.optInEmpty!);
      refs.optInEmpty!.appendChild(
        sectionEmpty(
          "No collections opted in",
          "NFT metadata is never fetched without an explicit opt-in — that is what keeps your collection interests private.",
        ),
      );
      setHidden(refs.optInEmpty!, false);
      setHidden(refs.optInWrap!, true);
    } else {
      clearContainer(refs.optInEmpty!);
      setHidden(refs.optInEmpty!, true);
      setHidden(refs.optInWrap!, false);
      renderList(
        refs.optInBody!,
        state.optIns,
        (optIn) => `${optIn.chain_id}:${optIn.contract_address.toLowerCase()}`,
        optInRow,
      );
    }
  }

  function buildGatewayForm(): HTMLElement {
    const form = el("form", {
      class: "dest-form",
      dataset: { portfolio: "gateway-form" },
      attrs: { novalidate: "" },
    });
    const row = el("div", { class: "form-row" });
    const input = el("input", {
      attrs: {
        type: "text",
        name: "ipfs_gateway_url",
        placeholder: "IPFS gateway URL (optional, e.g. https://your-gateway/ipfs/)",
        "aria-label": "IPFS gateway URL",
      },
    }) as HTMLInputElement;
    input.value = state.ipfsGateway;
    row.appendChild(input);
    row.appendChild(
      el("button", {
        class: "btn-ghost",
        text: "Save gateway",
        attrs: { type: "submit" },
        dataset: { portfolio: "gateway-save" },
      }),
    );
    const fetchButton = el("button", {
      class: "btn-ghost",
      text: "Fetch metadata now",
      attrs: { type: "button" },
      dataset: { portfolio: "nft-fetch" },
      on: {
        click: () => void fetchNftMetadata(fetchButton as HTMLButtonElement),
      },
    });
    row.appendChild(fetchButton);
    form.appendChild(row);
    form.addEventListener("submit", (event) => {
      (event as Event).preventDefault?.();
      void saveGateway(input);
    });
    return form;
  }

  async function saveGateway(input: HTMLInputElement): Promise<void> {
    try {
      await daemonRequest<MutationStatusWire>(
        "POST",
        "/api/inventory/nft-metadata/settings",
        { ipfs_gateway_url: input.value.trim() },
      );
      setStatus("NFT metadata gateway saved.");
      void loadTokens();
    } catch (error) {
      const failure = apiFailure(error);
      if (isLockFailure(failure)) {
        state.locked = true;
        renderView();
        return;
      }
      setStatus(`Couldn't save the gateway: ${failureText(error)}`);
    }
  }

  async function fetchNftMetadata(button: HTMLButtonElement): Promise<void> {
    button.disabled = true;
    button.classList.add("btn-busy");
    try {
      const result = await daemonRequest<NftFetchResponseWire>(
        "POST",
        "/api/inventory/nft-metadata/fetch",
        {},
      );
      const skipped = result.skipped ?? [];
      const reasons = skipped
        .slice(0, 3)
        .map((skip) => skip.reason || "skipped")
        .join("; ");
      setStatus(
        `Fetched ${result.fetched ?? 0}, skipped ${skipped.length}` +
          (reasons ? `: ${reasons}` : ""),
      );
      void loadTokens();
    } catch (error) {
      const failure = apiFailure(error);
      if (isLockFailure(failure)) {
        state.locked = true;
        renderView();
        return;
      }
      setStatus(`Couldn't fetch metadata: ${failureText(error)}`);
    } finally {
      button.disabled = false;
      button.classList.remove("btn-busy");
    }
  }

  function nftCacheRow(
    entry: NftMetadataCacheEntry,
    existing: HTMLElement | null,
  ): HTMLElement {
    if (existing) return existing;
    const row = el("tr", { dataset: { portfolio: "nftcache-row" } });
    const nameTd = el("td");
    nameTd.appendChild(
      el("div", { class: "cell-primary", text: entry.name || "(unnamed)" }),
    );
    if ((entry.spam_reasons ?? []).length) {
      nameTd.appendChild(
        el("div", {
          class: "cell-secondary",
          text: (entry.spam_reasons ?? []).join("; "),
        }),
      );
    }
    row.appendChild(nameTd);
    const collectionTd = el("td");
    collectionTd.appendChild(
      el("span", {
        class: "mono",
        text: middleTruncate(entry.contract_address),
        attrs: { title: `${entry.contract_address} · token ${entry.token_id_hex}` },
      }),
    );
    row.appendChild(collectionTd);
    row.appendChild(el("td", { text: chainName(entry.chain_id) }));
    row.appendChild(el("td", { text: spamLabelText(entry.spam_label) }));
    row.appendChild(
      el("td", {
        class: "nums",
        text: entry.fetched_at_unix
          ? relativeTime(entry.fetched_at_unix, nowSecs())
          : entry.fetch_skipped_reason
            ? `skipped: ${entry.fetch_skipped_reason}`
            : "not fetched",
        attrs: {
          title: entry.fetched_at_unix
            ? formatTimestamp(entry.fetched_at_unix)
            : "",
        },
      }),
    );
    return row;
  }

  function updateNftCacheSection(): void {
    const refs = state.refs;
    if (!refs.cacheBody) return;
    if (state.load.portfolio === "loading" && !state.everLoaded.portfolio) {
      clearContainer(refs.cacheEmpty!);
      refs.cacheEmpty!.appendChild(skeletonRows(2));
      setHidden(refs.cacheEmpty!, false);
      setHidden(refs.cacheWrap!, true);
      return;
    }
    // Suspicious entries sort first — they are never auto-hidden.
    const entries = state.nftCache
      .slice()
      .sort((a, b) => {
        const aSuspicious =
          a.spam_label && a.spam_label !== "operator_trusted" ? 1 : 0;
        const bSuspicious =
          b.spam_label && b.spam_label !== "operator_trusted" ? 1 : 0;
        return bSuspicious - aSuspicious;
      });
    if (!entries.length) {
      clearContainer(refs.cacheEmpty!);
      refs.cacheEmpty!.appendChild(
        sectionEmpty(
          "No cached NFT metadata",
          "Metadata appears here after you opt a collection in and fetch it. Suspicious entries are never auto-hidden.",
        ),
      );
      setHidden(refs.cacheEmpty!, false);
      setHidden(refs.cacheWrap!, true);
      return;
    }
    clearContainer(refs.cacheEmpty!);
    setHidden(refs.cacheEmpty!, true);
    setHidden(refs.cacheWrap!, false);
    renderList(
      refs.cacheBody!,
      entries,
      (entry) =>
        `${entry.chain_id}:${entry.contract_address.toLowerCase()}:${entry.token_id_hex}`,
      nftCacheRow,
    );
  }

  // ── Step 4 patching (stable regions; store updates do not rebuild) ──

  function buildStepResultsShell(content: HTMLElement): void {
    const errorRegion = el("div", { dataset: { portfolio: "results-error" } });
    content.appendChild(errorRegion);
    const opsRegion = el("div", {
      class: "dest-ops",
      dataset: { portfolio: "results-ops" },
    });
    content.appendChild(opsRegion);
    const noteRegion = el("div", { dataset: { portfolio: "results-note" } });
    content.appendChild(noteRegion);
    const summaryRegion = el("div", { dataset: { portfolio: "results-summary-region" } });
    content.appendChild(summaryRegion);
    const actions = el("div", { class: "stepper-nav" });
    actions.appendChild(
      el("button", {
        class: "btn-ghost",
        text: "Scan again",
        attrs: { type: "button" },
        dataset: { portfolio: "scan-again" },
        on: {
          click: () => {
            state.stepper.step = 1;
            state.stepper.launchedOperationIds = [];
            state.stepper.launchedJobIds = [];
            state.stepper.syncResults = [];
            state.stepper.launchError = null;
            updateScanView();
          },
        },
      }),
    );
    actions.appendChild(
      el("a", {
        class: "btn-primary",
        text: "View holdings",
        attrs: { href: formatHash("portfolio") },
        dataset: { portfolio: "view-holdings" },
      }),
    );
    content.appendChild(actions);
    state.refs.resultsError = errorRegion;
    state.refs.resultsOps = opsRegion;
    state.refs.resultsNote = noteRegion;
    state.refs.resultsSummary = summaryRegion;
  }

  function patchStepResults(): void {
    const refs = state.refs;
    if (!refs.resultsOps) return;
    const operationIds = state.stepper.launchedOperationIds;
    const syncResults = state.stepper.syncResults;

    clearContainer(refs.resultsError!);
    if (state.stepper.launchError) {
      refs.resultsError!.appendChild(
        el("p", {
          class: "field-error",
          dataset: { portfolio: "launch-error" },
          text: `A scan failed to start: ${state.stepper.launchError}`,
          attrs: { role: "alert" },
        }),
      );
    }

    const operations = runtime.store.get("operations");
    interface OpSlot {
      id: string;
      operation: Operation | undefined;
    }
    const slots: OpSlot[] = operationIds.map((id) => ({
      id,
      operation: operations.find((candidate) => candidate.id === id),
    }));
    renderList(
      refs.resultsOps,
      slots,
      (slot) => slot.id,
      (slot, existing) => {
        if (!slot.operation) {
          const row = el("div", { class: "ops-row" });
          row.appendChild(
            el("span", { class: "status-dot", dataset: { state: "busy" } }),
          );
          const main = el("div", { class: "ops-row-main" });
          main.appendChild(
            el("p", { class: "ops-row-title", text: "Balance scan · starting" }),
          );
          main.appendChild(
            el("p", {
              class: "ops-row-body",
              text: "Waiting for the daemon to report progress…",
            }),
          );
          row.appendChild(main);
          return row;
        }
        const operation = slot.operation;
        const marker = `${operation.state}:${operation.progress?.processed ?? 0}:${operation.error ?? ""}`;
        if (existing && existing.dataset.opMarker === marker) return existing;
        const row = operationRow(operation);
        row.dataset.opMarker = marker;
        return row;
      },
    );

    const running = slots.filter(
      (slot) =>
        !slot.operation || !isTerminalOperationState(slot.operation.state),
    ).length;

    clearContainer(refs.resultsNote!);
    if (operationIds.length && running > 0) {
      refs.resultsNote!.appendChild(
        el("p", {
          class: "stepper-note",
          text: "Progress updates live over the console's event stream. You can leave this page — the scan keeps running in the background.",
        }),
      );
    }

    clearContainer(refs.resultsSummary!);
    const showSyncSummary = syncResults.length > 0;
    const showAsyncSummary = operationIds.length > 0 && running === 0;
    if (showSyncSummary || showAsyncSummary) {
      const results = el("div", {
        class: "launch-summary",
        dataset: { portfolio: "results-summary", tier: "quiet" },
      });
      results.appendChild(
        el("p", { class: "launch-summary-title", text: "Scan results" }),
      );
      const list = el("ul", { class: "launch-summary-list" });
      for (const result of syncResults) {
        list.appendChild(el("li", { text: result.summary }));
      }
      if (showAsyncSummary) {
        const jobs = state.jobs.filter((job) =>
          state.stepper.launchedJobIds.includes(job.id),
        );
        if (!jobs.length) {
          list.appendChild(
            el("li", {
              text: "The daemon is still writing the results — they appear here in a moment.",
            }),
          );
        }
        for (const job of jobs) {
          list.appendChild(
            el("li", {
              text:
                `${jobScopeSummary(job)}: ${jobResultSummary(job)}` +
                (job.last_error ? ` — error: ${job.last_error}` : ""),
            }),
          );
        }
      }
      results.appendChild(list);
      refs.resultsSummary!.appendChild(results);
    }
  }

  function renderStepResults(content: HTMLElement): void {
    buildStepResultsShell(content);
    patchStepResults();
  }

  // ── View dispatcher ───────────────────────────────────────────────

  function currentView(): ViewName {
    const segment = state.route?.path[0];
    if (segment === "scan") return "scan";
    if (segment === "risk") return "risk";
    if (segment === "tokens") return "tokens";
    return "holdings";
  }

  function renderView(): void {
    if (!state.root || !state.viewRoot) return;
    const view = currentView();
    state.view = view;
    updateBanner();
    if (state.locked) {
      renderLockedPanel();
      return;
    }
    switch (view) {
      case "holdings":
        if (!state.refs.addressesBody) buildRootShell();
        updateAddressesSection();
        updateHoldingsSection();
        updateOpsRegion();
        updateJobsSection();
        break;
      case "scan":
        if (!state.refs.stepContent) buildScanShell();
        else updateScanView();
        break;
      case "risk":
        if (!state.refs.findingsBody) buildRiskShell();
        updateFindingsSection();
        updateCatalogSection();
        break;
      case "tokens":
        if (!state.refs.registryBody) buildTokensShell();
        updateRegistrySection();
        updateOptInSection();
        updateNftCacheSection();
        break;
    }
  }

  // ── Store subscriptions ───────────────────────────────────────────

  function onRouteChange(route: Route): void {
    state.route = route;
    renderView();
  }

  function onOperations(operations: Operation[]): void {
    let terminalTransition = false;
    for (const operation of operations) {
      if (operation.kind !== SCAN_OPERATION_KIND) continue;
      const previous = state.opStates.get(operation.id);
      if (
        previous &&
        !isTerminalOperationState(previous) &&
        isTerminalOperationState(operation.state)
      ) {
        terminalTransition = true;
      }
      state.opStates.set(operation.id, operation.state);
    }
    if (state.view === "holdings") updateOpsRegion();
    if (state.view === "scan" && state.stepper.step === 4) patchStepResults();
    if (terminalTransition) scheduleRefetch();
  }

  function onResync(): void {
    scheduleRefetch();
  }

  function onStatus(status: StatusResponse | null): void {
    const locked = status?.locked === true;
    if (locked === state.locked) return;
    state.locked = locked;
    if (!locked) {
      // Freshly unlocked: load whatever the current view needs.
      const view = currentView();
      if (view === "holdings") void loadPortfolio();
      else if (view === "scan") {
        void loadProfiles();
        void loadPortfolio();
      } else if (view === "risk") void loadRisk();
      else void loadTokens();
    }
    renderView();
  }

  // ── Mount / unmount ───────────────────────────────────────────────

  function mount(route: Route): void {
    state.route = route;
    const host = document.getElementById(HOST_ELEMENT_ID);
    if (!host) return;
    state.host = host;
    clearContainer(host);

    const root = el("div", {
      class: "dest-portfolio",
      dataset: { portfolio: "root" },
    });
    const bannerRegion = el("div", { dataset: { portfolio: "banner-region" } });
    root.appendChild(bannerRegion);
    const viewRoot = el("div", { dataset: { portfolio: "view-root" } });
    root.appendChild(viewRoot);
    host.appendChild(root);
    state.root = root;
    state.bannerRegion = bannerRegion;
    state.viewRoot = viewRoot;
    state.refs = {};
    state.view = null;
    state.scanRenderToken = null;
    state.locked = runtime.store.get("status")?.locked === true;

    runtime.router.register("portfolio", "scan");
    runtime.router.register("portfolio", "risk");
    runtime.router.register("portfolio", "tokens");

    state.unsubs.push(runtime.store.subscribe("route", onRouteChange));
    state.unsubs.push(runtime.store.subscribe("operations", onOperations));
    state.unsubs.push(runtime.store.subscribe("resync", onResync));
    state.unsubs.push(runtime.store.subscribe("status", onStatus));

    if (!state.locked) {
      const view = currentView();
      if (view === "holdings") void loadPortfolio();
      else if (view === "scan") {
        void loadProfiles();
        void loadPortfolio();
      } else if (view === "risk") void loadRisk();
      else {
        void loadTokens();
        if (state.load.portfolio === "idle") void loadPortfolio();
      }
    }
    renderView();
  }

  function unmount(): void {
    for (const unsubscribe of state.unsubs.splice(0)) unsubscribe();
    if (state.refetchTimer !== null) {
      clearTimeout(state.refetchTimer as never);
      state.refetchTimer = null;
    }
    state.opStates.clear();
    state.root?.remove();
    state.root = null;
    state.host = null;
    state.bannerRegion = null;
    state.viewRoot = null;
    state.statusLine = null;
    state.refs = {};
    state.view = null;
    state.scanRenderToken = null;
    state.load = { portfolio: "idle", risk: "idle", tokens: "idle", profiles: "idle" };
    state.everLoaded = {
      portfolio: false,
      risk: false,
      tokens: false,
      profiles: false,
    };
    state.stale = {};
    state.stepper = initialStepper();
    state.lastAdvanced = null;
  }

  return {
    id: "portfolio",
    migrated: true,
    mount,
    unmount,
  };
}
