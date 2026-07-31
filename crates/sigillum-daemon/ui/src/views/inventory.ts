import { ROUTE_PATHS } from "../routePaths";
import type {
  ChainProfile,
  ConsolidationPlan,
  ConsolidationPlanGenerateRequest,
  ConsolidationPlanExportResponse,
  ConsolidationPlanStep,
  Counterparty,
  NftMetadataCacheEntry,
  NftMetadataCollectionOptIn,
  PartyDestination,
  RiskCatalogEntry,
  RiskFinding,
  TokenRegistryList,
  WatchAddressBookEntry,
  WalletDiscoveryJob,
} from "../contracts";
import {
  clearFields,
  optionalNumberValue,
  optionalTextValue,
  renderEntityList,
  textValue,
} from "../render/forms";
import { confirmDangerDialog, confirmTypedDialog } from "../render/confirm";
import {
  amountWithRawHtml,
  chainLabel as resolveChainLabel,
  formatTimestamp,
} from "../render/format";
import { esc, escAttr, statusPill } from "../render/html";

/// Per-family W7.1 execution gate field for each plan-step action.
/// review_asset is deliberately absent: it is never executable.
const EXECUTION_FAMILY_GATE: Record<string, string> = {
  sweep_native: "allow_sweep_execution",
  sweep_erc20: "allow_sweep_execution",
  sweep_nft: "allow_sweep_execution",
  revoke_erc20_approval: "allow_revoke_execution",
  revoke_permit2_allowance: "allow_revoke_execution",
  revoke_nft_operator_approval: "allow_revoke_execution",
  revoke_approval: "allow_revoke_execution",
  approve_erc20: "allow_revoke_execution",
  exit_defi_position: "allow_exit_execution",
  claim_reward: "allow_claim_execution",
  fund_gas: "allow_gas_topups",
};

function stepSimulatedAtUnix(step: ConsolidationPlanStep): number | null {
  let simulatedAt: number | null = null;
  for (const item of step.simulation_evidence || []) {
    if (item.startsWith("simulated_at_unix=")) {
      const value = Number(item.slice("simulated_at_unix=".length));
      if (Number.isFinite(value)) simulatedAt = value;
    }
  }
  return simulatedAt;
}

/// Mirror of the daemon's enqueue eligibility gates for the Execute
/// affordance ONLY: policy on + gates on + not paused + approved + fresh
/// passed simulation + unblocked + not already enqueued. The daemon
/// re-validates everything server-side; this never widens what it allows.
export function stepExecutionEligible(
  step: ConsolidationPlanStep,
  policy: Record<string, unknown> | null | undefined,
  nowSecs: number,
): boolean {
  if (!policy || !policy.enabled) return false;
  if (policy.execution_paused) return false;
  if (!policy.allow_plan_execution) return false;
  const gateField = EXECUTION_FAMILY_GATE[step.action];
  if (!gateField || !policy[gateField]) return false;
  if (!step.approved || step.status !== "approved") return false;
  if ((step.blockers || []).length) return false;
  if (step.simulation_status !== "passed") return false;
  const simulatedAt = stepSimulatedAtUnix(step);
  if (simulatedAt === null) return false;
  const freshnessSecs = Number(policy.simulation_freshness_secs ?? 900);
  if (nowSecs - simulatedAt > freshnessSecs) return false;
  if (step.queued_job_id) return false;
  return true;
}

export function blockerLabel(code: string): string {
  switch (code) {
    case "missing_party_destination":
      return "no destination set for this payer";
    case "missing_destination":
      return "no destination set";
    case "cross_party_linkage":
      return "Destination shared with another payer";
    case "claim_execution_disabled":
      return "claim execution disabled (needs policy opt-in, passed simulation, trusted/reviewed claim contract, and approval)";
    default:
      return code;
  }
}

export interface InventoryActionsDeps {
  api: (method: string, path: string, body?: unknown) => Promise<any>;
  toast: (message: string, type?: string) => void;
  downloadJson: (filename: string, payload: unknown) => void;
}

export interface WatchAddressProbe {
  address: string;
  label?: string | null;
}

function input(id: string): HTMLInputElement {
  return document.getElementById(id) as HTMLInputElement;
}

export function createInventoryActions(deps: InventoryActionsDeps) {
  let planRoutingListenerBound = false;
  let planPartyDestinationInputIds: string[] = [];
  let latestChainProfiles: ChainProfile[] = [];
  let latestTokenLists: TokenRegistryList[] = [];
  let latestTreasuryPolicy: Record<string, unknown> | null = null;

  function planRoutingStrategy(): "single" | "per_party" {
    const routingEl = document.getElementById(
      "planRoutingStrategy",
    ) as HTMLSelectElement | null;
    return routingEl?.value === "per_party" ? "per_party" : "single";
  }

  function setHidden(el: HTMLElement | null, hidden: boolean): void {
    if (!el) return;
    el.hidden = hidden;
    if (hidden) el.setAttribute("hidden", "");
    else el.removeAttribute("hidden");
  }

  function bindPlanRoutingSelect(): void {
    if (planRoutingListenerBound) return;
    const routingEl = document.getElementById(
      "planRoutingStrategy",
    ) as HTMLSelectElement | null;
    if (!routingEl) return;
    routingEl.addEventListener("change", () => {
      void renderPlanPartyDestinations();
    });
    planRoutingListenerBound = true;
  }

  async function renderPlanPartyDestinations(): Promise<void> {
    const routingStrategy = planRoutingStrategy();
    const showPerParty = routingStrategy === "per_party";
    const container = document.getElementById("planPartyDestinations");
    const hint = document.getElementById("planPerPartyHint");

    setHidden(container, !showPerParty);
    setHidden(hint, !showPerParty);
    if (!container) return;
    if (!showPerParty) {
      planPartyDestinationInputIds = [];
      container.innerHTML = "";
      return;
    }

    try {
      const r = await deps.api("GET", ROUTE_PATHS.API_TREASURY_PARTIES);
      if (r.error) {
        planPartyDestinationInputIds = [];
        container.innerHTML = "";
        return;
      }
      const parties = (r.parties || []) as Counterparty[];
      planPartyDestinationInputIds = parties.map((party) => "planPartyDest_" + party.id);
      container.innerHTML = parties
        .map((party) => {
          const id = "planPartyDest_" + party.id;
          const name = party.name || party.id;
          return (
            '<div class="form-row">' +
            '<label class="checkbox-row" for="' +
            escAttr(id) +
            '">' +
            esc(name) +
            "</label>" +
            '<input type="text" id="' +
            escAttr(id) +
            '" data-counterparty-id="' +
            escAttr(party.id) +
            '" placeholder="Destination for ' +
            escAttr(name) +
            '" class="input-wide">' +
            "</div>"
          );
        })
        .join("");
    } catch (_) {
      planPartyDestinationInputIds = [];
      container.innerHTML = "";
    }
  }

  function collectPlanPartyDestinations(): PartyDestination[] {
    const container = document.getElementById("planPartyDestinations");
    const inputs: HTMLInputElement[] = [];
    if (
      container &&
      typeof (
        container as HTMLElement & {
          querySelectorAll?: HTMLElement["querySelectorAll"];
        }
      ).querySelectorAll === "function"
    ) {
      inputs.push(
        ...Array.from(
          container.querySelectorAll<HTMLInputElement>(
            "input[data-counterparty-id]",
          ),
        ),
      );
    }
    if (!inputs.length) {
      planPartyDestinationInputIds.forEach((inputId) => {
        const inputEl = document.getElementById(inputId) as HTMLInputElement | null;
        if (inputEl) inputs.push(inputEl);
      });
    }
    return inputs
      .map((inputEl) => {
        const counterpartyId =
          inputEl.dataset.counterpartyId ||
          inputEl.getAttribute("data-counterparty-id") ||
          inputEl.id.replace(/^planPartyDest_/, "");
        return {
          counterparty_id: counterpartyId,
          destination_address: inputEl.value.trim(),
        };
      })
      .filter((destination) =>
        Boolean(destination.counterparty_id && destination.destination_address),
      );
  }

  function renderChainProfiles(profiles: any[]): void {
    renderEntityList(
      "chainProfileList",
      profiles,
      "No chain profiles yet. Save one to describe discovery/indexing capabilities for a network.",
      (profile) =>
        '<li><div class="entity-main">' +
        '<div class="entity-title">' +
        esc(profile.name) +
        " " +
        statusPill(profile.enabled ? "enabled" : "disabled") +
        (profile.builtin ? " " + statusPill("builtin") : "") +
        "</div>" +
        '<div class="entity-meta">' +
        "family=" +
        esc(profile.chain_family) +
        " · chainId=" +
        esc(String(profile.chain_id || "-")) +
        " · provider=" +
        esc(profile.provider_profile || "-") +
        " · native=" +
        esc(profile.native_symbol || "-") +
        " · decimals=" +
        esc(String(profile.native_decimals ?? 18)) +
        " · finality=" +
        esc(String(profile.finality_blocks ?? 0)) +
        "<br>" +
        "permit2=" +
        esc(profile.permit2_address || "-") +
        " · univ2Router=" +
        esc(profile.uniswap_v2_router_address || "-") +
        " · " +
        "capabilities=" +
        esc((profile.capabilities || []).join(", ") || "-") +
        " · source=" +
        esc(profile.source || "-") +
        "</div></div>" +
        (profile.builtin
          ? ""
          : '<div class="entity-actions">' +
            '<button class="btn-danger" data-action="deleteChainProfile" data-arg0="' +
            escAttr(profile.name) +
            '">Delete</button>' +
            "</div>") +
        "</li>",
    );
  }

  function chainLabel(chainId: number | string | null | undefined): string {
    return resolveChainLabel(chainId, latestChainProfiles);
  }

  interface AmountUnits {
    decimals: number;
    symbol: string | null;
  }

  /// Native amounts use the chain profile's decimals/symbol when the chain
  /// is configured, else the EVM-native 18-decimal ETH convention.
  function nativeUnits(chainId: unknown): AmountUnits {
    const numericChainId = Number(chainId);
    const profile = latestChainProfiles.find(
      (chain) => chain.chain_id === numericChainId,
    );
    return {
      decimals: profile?.native_decimals ?? 18,
      symbol: profile?.native_symbol || "ETH",
    };
  }

  /// Token amounts only humanize when an imported token registry list knows
  /// the contract's decimals; otherwise the raw hex stays (never guess units).
  function tokenUnits(chainId: unknown, assetAddress: unknown): AmountUnits | null {
    const numericChainId = Number(chainId);
    const address = String(assetAddress || "").toLowerCase();
    if (!address) return null;
    for (const list of latestTokenLists) {
      const entry = (list.entries || []).find(
        (candidate) =>
          candidate.chain_id === numericChainId &&
          candidate.address.toLowerCase() === address,
      );
      if (entry) return { decimals: entry.decimals, symbol: entry.symbol };
    }
    return null;
  }

  function assetUnits(
    assetKind: unknown,
    chainId: unknown,
    assetAddress: unknown,
  ): AmountUnits | null {
    if (assetKind === "native" || !assetAddress) return nativeUnits(chainId);
    return tokenUnits(chainId, assetAddress);
  }

  function assetAmountHtml(
    amountHex: string | null | undefined,
    assetKind: unknown,
    chainId: unknown,
    assetAddress: unknown,
  ): string {
    const units = assetUnits(assetKind, chainId, assetAddress);
    return units
      ? amountWithRawHtml(amountHex, units)
      : esc(amountHex || "-");
  }

  function blockCursorSummary(job: WalletDiscoveryJob): string {
    const cursors = job.block_cursors || [];
    if (!cursors.length) return "-";
    const byFamily = new Map<string, number>();
    for (const cursor of cursors) {
      const key = `${chainLabel(cursor.chain_id)} ${cursor.topic_family}`;
      byFamily.set(key, Math.max(byFamily.get(key) || 0, cursor.last_scanned_block));
    }
    return Array.from(byFamily.entries())
      .map(([label, block]) => `${label} to ${block}`)
      .join(", ");
  }

  function renderNftMetadataOptIns(response: {
    opt_ins?: NftMetadataCollectionOptIn[];
    ipfs_gateway_url?: string | null;
  }): void {
    const gatewayInput = document.getElementById("nftMetaGatewayUrl") as
      | HTMLInputElement
      | null;
    if (response.ipfs_gateway_url && gatewayInput && !gatewayInput.value) {
      gatewayInput.value = response.ipfs_gateway_url;
    }
    renderEntityList(
      "nftMetaOptInList",
      response.opt_ins || [],
      "No collections opted in. NFT metadata is never fetched without an explicit opt-in.",
      (optIn) => {
        const nextEnabled = !optIn.enabled;
        return (
          '<li><div class="entity-main">' +
          '<div class="entity-title">' +
          esc(optIn.contract_address) +
          " " +
          statusPill(optIn.enabled ? "enabled" : "disabled") +
          "</div>" +
          '<div class="entity-meta">' +
          "chain=" +
          esc(chainLabel(optIn.chain_id)) +
          " · updated=" +
          esc(formatTimestamp(optIn.updated_at_unix)) +
          "</div></div>" +
          '<div class="entity-actions">' +
          '<button class="btn-ghost" data-action="toggleNftMetadataOptIn" data-arg0="' +
          escAttr(optIn.contract_address) +
          '" data-arg1="' +
          escAttr(String(optIn.chain_id)) +
          '" data-arg2="' +
          escAttr(String(nextEnabled)) +
          '">' +
          esc(nextEnabled ? "Enable" : "Disable") +
          "</button>" +
          '<button class="btn-danger" data-action="deleteNftMetadataOptIn" data-arg0="' +
          escAttr(optIn.contract_address) +
          '" data-arg1="' +
          escAttr(String(optIn.chain_id)) +
          '">Delete</button>' +
          "</div></li>"
        );
      },
    );
  }

  function nftHoldingContext(inventory: any, entry: NftMetadataCacheEntry): string {
    const entryContract = entry.contract_address.toLowerCase();
    const entryTokenId = (entry.token_id_hex || "").toLowerCase();
    const matches = (inventory.holdings || []).filter((holding: any) => {
      const holdingContract = String(
        holding.asset_address || holding.contract_address || "",
      ).toLowerCase();
      const holdingTokenId = String(holding.token_id_hex || "").toLowerCase();
      return (
        Number(holding.chain_id) === Number(entry.chain_id) &&
        holdingContract === entryContract &&
        holdingTokenId === entryTokenId
      );
    });
    if (!matches.length) return "";
    const addresses = Array.from(
      new Set(matches.map((holding: any) => holding.address).filter(Boolean)),
    )
      .slice(0, 3)
      .join(", ");
    return (
      "<br>holdings=" +
      esc(String(matches.length)) +
      (addresses ? " · addresses=" + esc(addresses) : "")
    );
  }

  function nftMetadataProvenanceLine(entry: NftMetadataCacheEntry): string {
    const fetchedParts: string[] = [];
    if (entry.fetched_at_unix !== undefined && entry.fetched_at_unix !== null) {
      fetchedParts.push("fetched=" + formatTimestamp(entry.fetched_at_unix));
    }
    if (entry.fetched_uri) fetchedParts.push("uri=" + entry.fetched_uri);
    if (entry.content_sha256) {
      fetchedParts.push("sha256=" + entry.content_sha256.slice(0, 12));
    }
    if (fetchedParts.length) return "<br>" + esc(fetchedParts.join(" · "));
    if (entry.fetch_skipped_reason) {
      return "<br>skipped=" + esc(entry.fetch_skipped_reason);
    }
    return "";
  }

  function renderNftMetadata(inventory: any): void {
    const entries = (inventory.nft_metadata_cache || []) as NftMetadataCacheEntry[];
    renderEntityList(
      "nftMetadataList",
      entries,
      "No NFT metadata cache entries yet.",
      (entry) =>
        '<li><div class="entity-main">' +
        '<div class="entity-title">' +
        esc(entry.name || "(unnamed)") +
        " " +
        statusPill(entry.spam_label || "unlabeled") +
        "</div>" +
        '<div class="entity-meta">' +
        "contract=" +
        esc(entry.contract_address) +
        " · tokenId=" +
        esc(entry.token_id_hex || "-") +
        " · chain=" +
        esc(chainLabel(entry.chain_id)) +
        nftHoldingContext(inventory, entry) +
        nftMetadataProvenanceLine(entry) +
        "</div></div></li>",
    );
    renderEntityList(
      "nftSuspiciousList",
      entries.filter(
        (entry) => Boolean(entry.spam_label) && entry.spam_label !== "operator_trusted",
      ),
      "No suspicious NFTs flagged.",
      (entry) =>
        '<li><div class="entity-main">' +
        '<div class="entity-title">' +
        esc(entry.name || "(unnamed)") +
        " " +
        statusPill(entry.spam_label || "unlabeled") +
        "</div>" +
        '<div class="entity-meta">' +
        "contract=" +
        esc(entry.contract_address) +
        " · tokenId=" +
        esc(entry.token_id_hex || "-") +
        " · chain=" +
        esc(chainLabel(entry.chain_id)) +
        "<br>reasons=" +
        esc((entry.spam_reasons || []).join(", ") || entry.spam_label || "-") +
        "</div></div></li>",
    );
  }

  function renderInventoryState(inventory: any): void {
    renderEntityList("inventoryJobList", inventory.jobs || [], "No discovery jobs yet.", (job: any) => {
      const chainIds = job.chain_ids || [];
      const chainLabels = chainIds.map((chainId: number) => chainLabel(chainId)).join(", ");
      const cursorSummary = blockCursorSummary(job);
      return (
        '<li><div class="entity-main">' +
        '<div class="entity-title">' +
        esc(job.id) +
        " " +
        statusPill(job.status) +
        "</div>" +
        '<div class="entity-meta">' +
        "wallets=" +
        esc((job.wallet_profiles || []).join(", ") || "-") +
        " · providers=" +
        esc((job.provider_profiles || []).join(", ") || "-") +
        " · chains=" +
        esc(chainLabels || "-") +
        "<br>" +
        "scanned=" +
        esc(String(job.addresses_scanned || 0)) +
        " · active=" +
        esc(String(job.active_addresses || 0)) +
        " · holdings=" +
        esc(String(job.holdings_detected || 0)) +
        "<br>scanned to block=" +
        esc(cursorSummary) +
        "</div></div>" +
        '<div class="entity-actions">' +
        // Plan task 1.2: cancel/resume are real — cancel cooperatively
        // stops the running scan (progress so far is kept), resume starts a
        // new background operation continuing from the job's checkpoints.
        '<button class="btn-ghost" title="Stop this scan after the current address; progress so far is kept" data-action="cancelDiscoveryJob" data-arg0="' +
        escAttr(job.id) +
        '">Cancel</button>' +
        '<button class="btn-ghost" title="Continue from this job&#39;s checkpoints in a new background scan" data-action="resumeDiscoveryJob" data-arg0="' +
        escAttr(job.id) +
        '">Resume</button>' +
        "</div></li>"
      );
    });
    renderEntityList(
      "inventoryAddressList",
      inventory.addresses || [],
      {
        message:
          "No discovered addresses yet. Run a balance scan to discover holdings.",
        actionLabel: "Run balance scan",
        action: "journeyRunScan",
      },
      (address: any) =>
        '<li><div class="entity-main">' +
        '<div class="entity-title">' +
        esc(address.address) +
        " " +
        statusPill(address.activity_state) +
        "</div>" +
        '<div class="entity-meta">' +
        esc(address.wallet_family) +
        "/" +
        esc(address.wallet_profile) +
        " · chain=" +
        esc(chainLabel(address.chain_id)) +
        (address.derivation_pattern
          ? " · pattern=" + esc(address.derivation_pattern)
          : "") +
        (address.account_index !== undefined && address.account_index !== null
          ? " · account=" + esc(String(address.account_index))
          : "") +
        " · path=" +
        esc(address.derivation_path) +
        "<br>" +
        "native=" +
        amountWithRawHtml(
          address.native_balance_wei_hex || "0x0",
          nativeUnits(address.chain_id),
        ) +
        " · txCount=" +
        esc(String(address.transaction_count || 0)) +
        (address.last_activity_block !== undefined && address.last_activity_block !== null
          ? " · lastActivityBlock=" + esc(String(address.last_activity_block))
          : "") +
        ((address.classifications || []).length
          ? "<br>classifications=" + esc((address.classifications || []).join(", "))
          : "") +
        "</div></div></li>",
    );
    renderEntityList(
      "inventoryHoldingList",
      inventory.holdings || [],
      {
        message:
          "No positive asset holdings detected yet. Run a balance scan to discover holdings.",
        actionLabel: "Run balance scan",
        action: "journeyRunScan",
      },
      (holding: any) =>
        '<li><div class="entity-main">' +
        '<div class="entity-title">' +
        esc(holding.asset_kind) +
        " " +
        statusPill(holding.status) +
        "</div>" +
        '<div class="entity-meta">' +
        "address=" +
        esc(holding.address) +
        " · asset=" +
        esc(holding.asset_address || "native") +
        (holding.token_id_hex
          ? " · tokenId=" + esc(holding.token_id_hex)
          : "") +
        (holding.counterparty_address
          ? " · spender=" + esc(holding.counterparty_address)
          : "") +
        (holding.protocol_address ? " · protocol=" + esc(holding.protocol_address) : "") +
        (holding.claim_adapter ? " · claimAdapter=" + esc(holding.claim_adapter) : "") +
        (holding.claim_index_hex ? " · claimIndex=" + esc(holding.claim_index_hex) : "") +
        ((holding.claim_proof || []).length
          ? " · proofWords=" + esc(String((holding.claim_proof || []).length))
          : "") +
        " · amount=" +
        assetAmountHtml(
          holding.amount_hex,
          holding.asset_kind,
          holding.chain_id,
          holding.asset_address,
        ) +
        "<br>" +
        esc(holding.wallet_family) +
        "/" +
        esc(holding.wallet_profile) +
        " · provider=" +
        esc(holding.provider_profile) +
        " · chain=" +
        esc(chainLabel(holding.chain_id)) +
        " · source=" +
        esc(holding.source || "-") +
        "</div></div></li>",
    );
    renderNftMetadata(inventory);
  }

  function renderWatchAddressBook(entries: WatchAddressBookEntry[]): void {
    renderEntityList(
      "watchAddressBookList",
      entries,
      {
        message:
          "No saved watch addresses yet. Save an address you want to monitor.",
        actionLabel: "Save an address",
        action: "focusWatchBook",
      },
      (entry) => {
        const tagsCsv = (entry.tags || []).join(", ");
        const nextEnabled = !entry.enabled;
        return (
          '<li><div class="entity-main">' +
          '<div class="entity-title">' +
          esc(entry.label || entry.address) +
          " " +
          statusPill(entry.enabled ? "enabled" : "disabled") +
          "</div>" +
          '<div class="entity-meta">' +
          esc(entry.address) +
          " · tags=" +
          esc(tagsCsv || "-") +
          " · source=" +
          esc(entry.source || "-") +
          " · updated=" +
          esc(formatTimestamp(entry.updated_at_unix)) +
          "</div></div>" +
          '<div class="entity-actions">' +
          '<button class="btn-ghost" data-action="loadWatchAddressBookEntry" data-arg0="' +
          escAttr(entry.address) +
          '" data-arg1="' +
          escAttr(entry.label || "") +
          '" data-arg2="' +
          escAttr(tagsCsv) +
          '" data-arg3="' +
          escAttr(String(entry.enabled)) +
          '">Load</button>' +
          '<button class="btn-ghost" data-action="toggleWatchAddressBookEntry" data-arg0="' +
          escAttr(entry.address) +
          '" data-arg1="' +
          escAttr(entry.label || "") +
          '" data-arg2="' +
          escAttr(tagsCsv) +
          '" data-arg3="' +
          escAttr(String(nextEnabled)) +
          '">' +
          esc(nextEnabled ? "Enable" : "Disable") +
          "</button>" +
          '<button class="btn-danger" data-action="deleteWatchAddressBookEntry" data-arg0="' +
          escAttr(entry.address) +
          '">Delete</button>' +
          "</div></li>"
        );
      },
    );
  }

  function renderTokenRegistry(lists: TokenRegistryList[]): void {
    renderEntityList(
      "tokenRegistryList",
      lists,
      "No token registry lists imported yet.",
      (list) => {
        const entries = list.entries || [];
        const chainIds =
          Array.from(new Set(entries.map((entry) => entry.chain_id)))
            .sort((left, right) => left - right)
            .map((chainId) => String(chainId))
            .join(", ") || "-";
        return (
          '<li><div class="entity-main">' +
          '<div class="entity-title">' +
          esc(list.name) +
          " " +
          statusPill(list.source) +
          "</div>" +
          '<div class="entity-meta">' +
          esc(String(entries.length)) +
          " entries · chains=" +
          esc(chainIds) +
          " · updated=" +
          esc(formatTimestamp(list.updated_at_unix)) +
          "</div></div>" +
          '<div class="entity-actions">' +
          '<button class="btn-danger" data-action="deleteTokenRegistryList" data-arg0="' +
          escAttr(list.name) +
          '">Delete</button>' +
          "</div></li>"
        );
      },
    );
  }

  function renderRiskFindings(findings: any[]): void {
    renderEntityList(
      "riskFindingList",
      findings,
      "No risk findings from the current inventory.",
      (finding: any) =>
        '<li><div class="entity-main">' +
        '<div class="entity-title">' +
        esc(finding.category) +
        " " +
        statusPill(finding.risk_level) +
        "</div>" +
        '<div class="entity-meta">' +
        "subject=" +
        esc(finding.subject_type) +
        ":" +
        esc(finding.subject) +
        " · address=" +
        esc(finding.address) +
        "<br>" +
        esc(finding.recommendation || "") +
        "</div></div></li>",
    );
  }

  function renderRiskCatalog(entries: any[]): void {
    renderEntityList(
      "riskCatalogList",
      entries,
      "No local risk catalog entries yet.",
      (entry: any) =>
        '<li><div class="entity-main">' +
        '<div class="entity-title">' +
        esc(entry.label || entry.address) +
        " " +
        statusPill(entry.risk_level) +
        "</div>" +
        '<div class="entity-meta">' +
        esc(entry.address) +
        " · source=" +
        esc(entry.source || "-") +
        ((entry.notes || []).length
          ? "<br>" + esc((entry.notes || []).join(" | "))
          : "") +
        "</div></div>" +
        '<div class="entity-actions">' +
        '<button class="btn-danger" data-action="deleteRiskCatalogEntry" data-arg0="' +
        escAttr(entry.address) +
        '">Delete</button>' +
        "</div></li>",
    );
  }

  function renderConsolidationPlans(plans: ConsolidationPlan[]): void {
    renderEntityList(
      "consolidationPlanList",
      plans,
      "No consolidation plans generated yet.",
      (plan) => {
        const summary = plan.summary;
        const linkageFindings = plan.linkage_findings || [];
        const linkageBanner = linkageFindings.length
          ? '<div class="plan-linkage-banner"><strong>Privacy: this plan would link payers</strong><br>' +
            esc(linkageFindings.join(" | ")) +
            '<br><span class="linkage-warning">Scope: flags payers that would sweep to the same destination. Sigillum-generated fund_gas top-ups are checked: one sponsor funding different payers warns and is blocked when linkage protection is on. Manual gas funding, amount/timing correlation, downstream re-merging, and multi-hop flows remain operator discipline.</span>' +
            "</div>"
          : "";
        const safeAddressInput =
          '<input type="text" class="input-wide plan-safe-address" data-plan-safe-address placeholder="Safe address" autocomplete="off">';
        const nowSecs = Math.floor(Date.now() / 1000);
        const anyStepEligible = (plan.steps || []).some((step) =>
          stepExecutionEligible(step, latestTreasuryPolicy, nowSecs),
        );
        const stepLines = (plan.steps || [])
          .slice(0, 8)
          .map((step: ConsolidationPlanStep) => {
            const evidence = (step.simulation_evidence || []).join(" | ");
            const linkageWarnings = step.linkage_warnings || [];
            const blockers = (step.blockers || []).map(blockerLabel).join(", ");
            const stepAmountHtml = assetAmountHtml(
              step.amount_hex,
              step.asset_kind,
              step.chain_id,
              step.asset_address,
            );
            // Execute appears ONLY when every gate passes; the daemon
            // re-validates everything at enqueue time regardless.
            const executeButton = stepExecutionEligible(
              step,
              latestTreasuryPolicy,
              nowSecs,
            )
              ? ' <button class="btn-ghost" data-action="enqueuePlanStep" data-arg0="' +
                escAttr(plan.id) +
                '" data-arg1="' +
                escAttr(step.id) +
                '">Execute</button>'
              : "";
            return (
              '<div class="entity-meta">' +
              esc(step.action) +
              " " +
              statusPill(step.status) +
              " · " +
              esc(step.asset_kind) +
              " · seq=" +
              esc(String(step.sequence ?? 0)) +
              ((step.depends_on || []).length
                ? " · dependsOn=" + esc((step.depends_on || []).join(","))
                : "") +
              (step.action === "fund_gas"
                ? " · sponsor=" +
                  esc(step.address) +
                  " · funds=" +
                  esc(step.destination_address || "-") +
                  " · topup=" +
                  amountWithRawHtml(step.amount_hex, nativeUnits(step.chain_id))
                : "") +
              (step.token_id_hex ? " #" + esc(step.token_id_hex) : "") +
              (step.counterparty_address
                ? " · spender/operator=" + esc(step.counterparty_address)
                : "") +
              (step.protocol_address ? " · protocol=" + esc(step.protocol_address) : "") +
              (step.claim_adapter ? " · claimAdapter=" + esc(step.claim_adapter) : "") +
              (step.exit_token0_address ? " · token0=" + esc(step.exit_token0_address) : "") +
              (step.exit_token1_address ? " · token1=" + esc(step.exit_token1_address) : "") +
              (step.exit_amount0_min_hex
                ? " · amount0Min=" + esc(step.exit_amount0_min_hex)
                : "") +
              (step.exit_amount1_min_hex
                ? " · amount1Min=" + esc(step.exit_amount1_min_hex)
                : "") +
              (step.exit_deadline_unix
                ? " · deadline=" + esc(String(step.exit_deadline_unix))
                : "") +
              (step.claim_index_hex ? " · claimIndex=" + esc(step.claim_index_hex) : "") +
              ((step.claim_proof || []).length
                ? " · proofWords=" + esc(String((step.claim_proof || []).length))
                : "") +
              " · amount=" +
              stepAmountHtml +
              " · simulation=" +
              esc(step.simulation_status || "not_run") +
              " · blockers=" +
              esc(blockers || "-") +
              (step.queued_job_id ? " · queuedJob=" + esc(step.queued_job_id) : "") +
              (evidence ? "<br>evidence=" + esc(evidence) : "") +
              (linkageWarnings.length
                ? '<br><span class="linkage-warning">privacy: ' +
                  esc(linkageWarnings.join(", ")) +
                  "</span>"
                : "") +
              executeButton +
              "</div>"
            );
          })
          .join("");
        return (
          '<li><div class="entity-main">' +
          '<div class="entity-title">' +
          esc(plan.id) +
          " " +
          statusPill(plan.status) +
          "</div>" +
          '<div class="entity-meta">' +
          "chain=" +
          esc(chainLabel(plan.chain_id)) +
          " · " +
          "steps=" +
          esc(String(summary.total_steps || 0)) +
          " · blocked=" +
          esc(String(summary.blocked_steps || 0)) +
          " · review=" +
          esc(String(summary.review_required_steps || 0)) +
          " · approved=" +
          esc(String(summary.approved_steps || 0)) +
          " · executable=" +
          esc(String(summary.executable_steps || 0)) +
          "</div>" +
          linkageBanner +
          stepLines +
          "</div>" +
          '<div class="entity-actions">' +
          '<button class="btn-ghost" data-action="simulateConsolidationPlan" data-arg0="' +
          escAttr(plan.id) +
          '">Simulate</button>' +
          '<button class="btn-ghost" data-action="approveConsolidationPlan" data-arg0="' +
          escAttr(plan.id) +
          '">Approve Reviewable</button>' +
          '<button class="btn-ghost" data-action="exportConsolidationPlan" data-arg0="' +
          escAttr(plan.id) +
          '" data-arg1="call_manifest">Call JSON</button>' +
          (anyStepEligible
            ? '<button class="btn-primary" data-action="enqueuePlanBulk" data-arg0="' +
              escAttr(plan.id) +
              '">Execute All Eligible</button>'
            : "") +
          '<div class="plan-export-controls">' +
          safeAddressInput +
          '<button class="btn-ghost" data-action="exportConsolidationPlan" data-arg0="' +
          escAttr(plan.id) +
          '" data-arg1="safe_tx_builder" data-self="append">Safe JSON</button>' +
          "</div>" +
          "</div></li>"
        );
      },
    );
  }

  async function loadInventoryOperations(): Promise<void> {
    bindPlanRoutingSelect();
    void renderPlanPartyDestinations();
    try {
      const [
        chains,
        watchBook,
        inventory,
        tokenRegistry,
        catalog,
        risks,
        plans,
        nftOptIns,
        treasuryPolicy,
      ] = await Promise.all([
        deps.api("GET", ROUTE_PATHS.API_CHAINS),
        deps.api("GET", ROUTE_PATHS.API_INVENTORY_WATCH_ADDRESSES),
        deps.api("GET", ROUTE_PATHS.API_INVENTORY_WALLETS),
        deps.api("GET", ROUTE_PATHS.API_INVENTORY_TOKEN_REGISTRY),
        deps.api("GET", ROUTE_PATHS.API_RISK_CATALOG),
        deps.api("GET", ROUTE_PATHS.API_RISK_FINDINGS),
        deps.api("GET", ROUTE_PATHS.API_PLANS_CONSOLIDATION),
        deps.api("GET", ROUTE_PATHS.API_INVENTORY_NFT_METADATA_OPT_INS),
        deps.api("GET", ROUTE_PATHS.API_TREASURY_POLICY),
      ]);
      if (!chains.error) {
        latestChainProfiles = chains.profiles || [];
        renderChainProfiles(latestChainProfiles);
      }
      // Token units must be captured before the inventory render so holding
      // amounts humanize with registry decimals.
      if (!tokenRegistry.error) latestTokenLists = tokenRegistry.lists || [];
      if (!watchBook.error) renderWatchAddressBook(watchBook.entries || []);
      if (!inventory.error) renderInventoryState(inventory);
      if (!tokenRegistry.error) renderTokenRegistry(latestTokenLists);
      if (!catalog.error) renderRiskCatalog(catalog.entries || []);
      if (!risks.error) renderRiskFindings(risks.findings || []);
      // The policy gates decide whether Execute affordances render, so it
      // must be applied before the plan list renders.
      latestTreasuryPolicy = treasuryPolicy.error
        ? null
        : ((treasuryPolicy.policy as Record<string, unknown> | null) ?? null);
      if (!plans.error) renderConsolidationPlans(plans.plans || []);
      if (!nftOptIns.error) renderNftMetadataOptIns(nftOptIns);
    } catch (_) {}
  }

  async function upsertChainProfile(): Promise<void> {
    const name = textValue("chainProfileName");
    const family = textValue("chainProfileFamily");
    if (!name || !family) {
      deps.toast("Chain profile name and family are required", "error");
      return;
    }
    const r = await deps.api("POST", ROUTE_PATHS.API_CHAINS_UPSERT, {
      name,
      chain_family: family,
      chain_id: optionalNumberValue("chainProfileId"),
      provider_profile: optionalTextValue("chainProfileProvider"),
      native_symbol: optionalTextValue("chainProfileNativeSymbol"),
      native_decimals: optionalNumberValue("chainProfileNativeDecimals"),
      finality_blocks: optionalNumberValue("chainProfileFinalityBlocks"),
      permit2_address: optionalTextValue("chainProfilePermit2Address"),
      uniswap_v2_router_address: optionalTextValue("chainProfileUniswapV2Router"),
      capabilities: [],
      enabled: true,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    clearFields([
      "chainProfileName",
      "chainProfileFamily",
      "chainProfileId",
      "chainProfileProvider",
      "chainProfileNativeSymbol",
      "chainProfileNativeDecimals",
      "chainProfileFinalityBlocks",
      "chainProfilePermit2Address",
      "chainProfileUniswapV2Router",
    ]);
    deps.toast("Chain profile saved");
    void loadInventoryOperations();
  }

  async function deleteChainProfile(name: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Delete chain profile",
      body:
        'Delete chain profile "' +
        name +
        '"? Inventory scans and plans that reference this chain will no longer resolve it.',
      actionLabel: "Delete",
    });
    if (!confirmed) return;
    const r = await deps.api("POST", "/api/chains/delete", { name });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Chain profile deleted");
    void loadInventoryOperations();
  }

  async function scanInventoryEvm(): Promise<void> {
    const scanButton = document.querySelector('[data-action="scanInventoryEvm"]');
    if (scanButton) scanButton.classList.add("btn-busy");
    try {
      await scanInventoryEvmRequest();
    } finally {
      if (scanButton) scanButton.classList.remove("btn-busy");
    }
  }

  async function scanInventoryEvmRequest(): Promise<void> {
    const watchAddress = optionalTextValue("inventoryWatchAddress");
    const watchLabel = optionalTextValue("inventoryWatchLabel");
    const watchAddresses = parseWatchAddressProbes(
      optionalTextValue("inventoryWatchAddresses"),
      watchAddress,
      watchLabel,
    );
    const walletFamily =
      optionalTextValue("inventoryWalletFamily") || (watchAddresses.length ? "eth-watch" : null);
    const token = optionalTextValue("inventoryTokenAddress");
    const spender = optionalTextValue("inventoryAllowanceSpender");
    const permit2Contract = optionalTextValue("inventoryPermit2Contract");
    const permit2Spender = optionalTextValue("inventoryPermit2Spender");
    const nftOperator = optionalTextValue("inventoryNftOperator");
    const allConfiguredChains =
      (document.getElementById("inventoryAllConfiguredChains") as HTMLInputElement | null)
        ?.checked ?? false;
    const providerProfile = optionalTextValue("inventoryProviderProfile");
    if (allConfiguredChains && providerProfile) {
      deps.toast("Choose either one provider profile or all configured chains", "error");
      return;
    }
    const body: Record<string, unknown> = {
      wallet_family: walletFamily,
      wallet_profile: optionalTextValue("inventoryWalletProfile"),
      provider_profile: providerProfile,
      derivation_pattern: optionalTextValue("inventoryDerivationPattern"),
      account_limit: optionalNumberValue("inventoryAccountLimit"),
      watch_addresses: watchAddresses,
      include_watch_book: input("inventoryIncludeWatchBook").checked,
      gap_limit: optionalNumberValue("inventoryGapLimit"),
      max_index: optionalNumberValue("inventoryMaxIndex"),
      token_addresses: token ? [token] : [],
      block_tag: "latest",
      discover_erc20_transfers: input("inventoryDiscoverErc20Transfers").checked,
      probe_token_registry: input("inventoryProbeTokenRegistry").checked,
      token_discovery_from_block: optionalTextValue("inventoryTokenDiscoveryFromBlock"),
      token_discovery_to_block: optionalTextValue("inventoryTokenDiscoveryToBlock"),
      token_discovery_limit: optionalNumberValue("inventoryTokenDiscoveryLimit"),
      discover_erc20_allowances: input("inventoryDiscoverErc20Allowances").checked,
      allowance_spender_addresses: spender ? [spender] : [],
      allowance_discovery_limit: optionalNumberValue("inventoryAllowanceLimit"),
      discover_permit2_allowances: input("inventoryDiscoverPermit2Allowances").checked,
      permit2_contract_addresses: permit2Contract ? [permit2Contract] : [],
      permit2_spender_addresses: permit2Spender ? [permit2Spender] : [],
      permit2_allowance_limit: optionalNumberValue("inventoryPermit2AllowanceLimit"),
      discover_erc721_transfers: input("inventoryDiscoverErc721Transfers").checked,
      discover_erc1155_transfers: input("inventoryDiscoverErc1155Transfers").checked,
      discover_nft_operator_approvals: input("inventoryDiscoverNftOperatorApprovals").checked,
      nft_operator_addresses: nftOperator ? [nftOperator] : [],
      nft_operator_approval_limit: optionalNumberValue("inventoryNftOperatorApprovalLimit"),
      nft_discovery_from_block: optionalTextValue("inventoryNftDiscoveryFromBlock"),
      nft_discovery_to_block: optionalTextValue("inventoryNftDiscoveryToBlock"),
      nft_discovery_limit: optionalNumberValue("inventoryNftDiscoveryLimit"),
    };
    if (allConfiguredChains) body.all_configured_chains = true;
    const runAsync =
      (document.getElementById("inventoryRunAsync") as HTMLInputElement | null)?.checked ??
      false;
    if (runAsync) body.run_async = true;
    const r = await deps.api("POST", "/api/inventory/scan/evm", body);
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    if (runAsync && r.operation && r.operation.id) {
      // Background scan accepted: progress renders in the job list on the
      // next refresh; the operation id lets the operator cross-check via
      // GET /api/operations/{id} (and cancel from the job row).
      deps.toast(
        "Scan started in background — operation " +
          String(r.operation.id) +
          "; progress shows in the job list below",
      );
    } else {
      deps.toast("Inventory scan completed");
    }
    void loadInventoryOperations();
  }

  function loadWatchAddressBookEntry(
    address: string,
    label = "",
    tagsCsv = "",
    enabled = "true",
  ): void {
    input("watchBookAddress").value = address;
    input("watchBookLabel").value = label;
    input("watchBookTags").value = tagsCsv;
    input("watchBookEnabled").checked = enabled !== "false";
  }

  async function upsertWatchAddressBookEntry(): Promise<void> {
    const address = textValue("watchBookAddress");
    if (!address) {
      deps.toast("Watch address is required", "error");
      return;
    }
    const r = await deps.api("POST", ROUTE_PATHS.API_INVENTORY_WATCH_ADDRESSES_UPSERT, {
      address,
      label: optionalTextValue("watchBookLabel"),
      tags: parseTagList(optionalTextValue("watchBookTags")),
      enabled: input("watchBookEnabled").checked,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    clearFields(["watchBookAddress", "watchBookLabel", "watchBookTags"]);
    input("watchBookEnabled").checked = true;
    deps.toast("Watch address saved");
    void loadInventoryOperations();
  }

  async function upsertBulkWatchAddressBookEntries(): Promise<void> {
    const probes = parseWatchAddressProbes(
      optionalTextValue("inventoryWatchAddresses"),
      optionalTextValue("inventoryWatchAddress"),
      optionalTextValue("inventoryWatchLabel"),
    );
    if (!probes.length) {
      deps.toast("No watch addresses to save", "error");
      return;
    }
    const tags = parseTagList(optionalTextValue("watchBookTags"));
    for (const probe of probes) {
      const r = await deps.api("POST", ROUTE_PATHS.API_INVENTORY_WATCH_ADDRESSES_UPSERT, {
        address: probe.address,
        label: probe.label || null,
        tags,
        enabled: true,
      });
      if (r.error) {
        deps.toast(r.error, "error");
        return;
      }
    }
    deps.toast("Watch addresses saved");
    void loadInventoryOperations();
  }

  async function toggleWatchAddressBookEntry(
    address: string,
    label: string,
    tagsCsv: string,
    enabled: string,
  ): Promise<void> {
    const r = await deps.api("POST", ROUTE_PATHS.API_INVENTORY_WATCH_ADDRESSES_UPSERT, {
      address,
      label: label || null,
      tags: parseTagList(tagsCsv),
      enabled: enabled === "true",
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast(enabled === "true" ? "Watch address enabled" : "Watch address disabled");
    void loadInventoryOperations();
  }

  async function deleteWatchAddressBookEntry(address: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Delete watch address",
      body:
        'Delete saved watch address "' +
        address +
        '"? The daemon stops tracking its balances and approvals.',
      actionLabel: "Delete",
    });
    if (!confirmed) return;
    const r = await deps.api("POST", "/api/inventory/watch-addresses/delete", { address });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Watch address deleted");
    void loadInventoryOperations();
  }

  async function upsertNftMetadataOptIn(): Promise<void> {
    const chainId = optionalNumberValue("nftMetaOptInChainId");
    const contractAddress = textValue("nftMetaOptInContract");
    if (chainId === null || !contractAddress) {
      deps.toast("Chain id and collection contract address are required", "error");
      return;
    }
    const r = await deps.api("POST", ROUTE_PATHS.API_INVENTORY_NFT_METADATA_OPT_INS_UPSERT, {
      chain_id: chainId,
      contract_address: contractAddress,
      enabled: true,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    clearFields(["nftMetaOptInContract"]);
    deps.toast("NFT metadata collection opted in");
    void loadInventoryOperations();
  }

  async function toggleNftMetadataOptIn(
    contractAddress: string,
    chainId: string,
    enabled: string,
  ): Promise<void> {
    const numericChainId = Number(chainId);
    if (!contractAddress || !Number.isFinite(numericChainId)) {
      deps.toast("NFT metadata opt-in target is invalid", "error");
      return;
    }
    const nextEnabled = enabled === "true";
    const r = await deps.api("POST", ROUTE_PATHS.API_INVENTORY_NFT_METADATA_OPT_INS_UPSERT, {
      chain_id: numericChainId,
      contract_address: contractAddress,
      enabled: nextEnabled,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast(nextEnabled ? "NFT metadata collection enabled" : "NFT metadata collection disabled");
    void loadInventoryOperations();
  }

  async function deleteNftMetadataOptIn(
    contractAddress: string,
    chainId: string,
  ): Promise<void> {
    const numericChainId = Number(chainId);
    if (!contractAddress || !Number.isFinite(numericChainId)) {
      deps.toast("NFT metadata opt-in target is invalid", "error");
      return;
    }
    const confirmed = await confirmDangerDialog({
      title: "Delete NFT metadata opt-in",
      body:
        'Delete NFT metadata opt-in "' +
        contractAddress +
        '"? Cached metadata for this collection is dropped and no longer fetched.',
      actionLabel: "Delete",
    });
    if (!confirmed) return;
    const r = await deps.api("POST", "/api/inventory/nft-metadata/opt-ins/delete", {
      chain_id: numericChainId,
      contract_address: contractAddress,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("NFT metadata opt-in deleted");
    void loadInventoryOperations();
  }

  async function saveNftMetadataSettings(): Promise<void> {
    const r = await deps.api("POST", ROUTE_PATHS.API_INVENTORY_NFT_METADATA_SETTINGS, {
      ipfs_gateway_url: textValue("nftMetaGatewayUrl"),
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("NFT metadata settings saved");
    void loadInventoryOperations();
  }

  function skippedNftMetadataSummary(skip: any): string {
    const subject = [
      skip.contract_address || null,
      skip.token_id_hex ? "#" + skip.token_id_hex : null,
      skip.chain_id !== undefined && skip.chain_id !== null
        ? "chain=" + chainLabel(skip.chain_id)
        : null,
    ]
      .filter(Boolean)
      .join(" ");
    return (subject ? subject + ": " : "") + (skip.reason || "skipped");
  }

  async function fetchNftMetadata(): Promise<void> {
    const fetchButton = document.querySelector('[data-action="fetchNftMetadata"]');
    if (fetchButton) fetchButton.classList.add("btn-busy");
    try {
      const r = await deps.api("POST", ROUTE_PATHS.API_INVENTORY_NFT_METADATA_FETCH, {});
      if (r.error) {
        deps.toast(r.error, "error");
        return;
      }
      const skipped = (r.skipped || []) as any[];
      const skipSummary = skipped.slice(0, 3).map(skippedNftMetadataSummary).join("; ");
      deps.toast(
        "Fetched " +
          String(r.fetched || 0) +
          ", skipped " +
          String(skipped.length) +
          (skipSummary ? ": " + skipSummary : ""),
      );
      void loadInventoryOperations();
    } finally {
      if (fetchButton) fetchButton.classList.remove("btn-busy");
    }
  }

  async function cancelDiscoveryJob(id: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Cancel discovery scan",
      body:
        'Stop discovery job "' +
        id +
        '"? The scan stops after the address it is currently checking. Progress so far is kept and you can resume from it later.',
      actionLabel: "Cancel scan",
    });
    if (!confirmed) return;
    const r = await deps.api("POST", "/api/discovery/jobs/cancel", { id });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    if (r.status === "cancel_requested") {
      deps.toast("Cancel requested — the scan stops after the current address");
    } else {
      deps.toast("Discovery job canceled");
    }
    void loadInventoryOperations();
  }

  async function resumeDiscoveryJob(id: string): Promise<void> {
    const r = await deps.api("POST", ROUTE_PATHS.API_DISCOVERY_JOBS_RESUME, { id });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Discovery scan resumed in the background");
    void loadInventoryOperations();
  }

  async function importTokenRegistry(): Promise<void> {
    const name = textValue("tokenRegistryName");
    if (!name) {
      deps.toast("Token registry list name is required", "error");
      return;
    }
    const entriesJson = optionalTextValue("tokenRegistryEntriesJson");
    const filePath = optionalTextValue("tokenRegistryFilePath");
    if ((entriesJson ? 1 : 0) + (filePath ? 1 : 0) !== 1) {
      deps.toast("Provide pasted JSON entries or a local file path (not both)", "error");
      return;
    }
    const r = await deps.api("POST", ROUTE_PATHS.API_INVENTORY_TOKEN_REGISTRY_IMPORT, {
      name,
      entries_json: entriesJson || undefined,
      file_path: filePath || undefined,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    clearFields(["tokenRegistryName", "tokenRegistryEntriesJson", "tokenRegistryFilePath"]);
    deps.toast("Token registry list imported");
    void loadInventoryOperations();
  }

  async function deleteTokenRegistryList(name: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Delete token registry list",
      body:
        'Delete token registry list "' +
        name +
        '"? Its token metadata is removed from local inventory views.',
      actionLabel: "Delete",
    });
    if (!confirmed) return;
    const r = await deps.api("POST", "/api/inventory/token-registry/delete", { name });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Token registry list deleted");
    void loadInventoryOperations();
  }

  async function loadRiskFindings(): Promise<void> {
    const [catalog, risks] = await Promise.all([
      deps.api("GET", ROUTE_PATHS.API_RISK_CATALOG),
      deps.api("GET", ROUTE_PATHS.API_RISK_FINDINGS),
    ]);
    if (catalog.error || risks.error) {
      deps.toast(catalog.error || risks.error, "error");
      return;
    }
    renderRiskCatalog(catalog.entries || []);
    renderRiskFindings(risks.findings || []);
    deps.toast("Risk findings refreshed");
  }

  async function upsertRiskCatalogEntry(): Promise<void> {
    const address = textValue("riskCatalogAddress");
    const riskLevel = textValue("riskCatalogLevel");
    if (!address || !riskLevel) {
      deps.toast("Risk catalog address and level are required", "error");
      return;
    }
    const note = optionalTextValue("riskCatalogNote");
    const r = await deps.api("POST", ROUTE_PATHS.API_RISK_CATALOG_UPSERT, {
      address,
      label: optionalTextValue("riskCatalogLabel"),
      risk_level: riskLevel,
      notes: note ? [note] : [],
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    clearFields(["riskCatalogAddress", "riskCatalogLabel", "riskCatalogNote"]);
    deps.toast("Risk catalog entry saved");
    void loadInventoryOperations();
  }

  async function deleteRiskCatalogEntry(address: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Delete risk catalog entry",
      body:
        'Delete risk catalog entry "' +
        address +
        '"? Findings derived from it disappear from the risk view.',
      actionLabel: "Delete",
    });
    if (!confirmed) return;
    const r = await deps.api("POST", "/api/risk/catalog/delete", { address });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Risk catalog entry deleted");
    void loadInventoryOperations();
  }

  async function generateConsolidationPlan(): Promise<void> {
    const routingEl = document.getElementById(
      "planRoutingStrategy",
    ) as HTMLSelectElement | null;
    const routingStrategy = routingEl?.value === "per_party" ? "per_party" : "single";
    const partyDestinations =
      routingStrategy === "per_party" ? collectPlanPartyDestinations() : [];
    const chainId = optionalNumberValue("planChainId");
    const body: ConsolidationPlanGenerateRequest = {
      destination_address: optionalTextValue("planDestinationAddress"),
      include_watch_only: true,
      auto_queue_low_risk: false,
      routing_strategy: routingStrategy,
    };
    if (chainId !== null) body.chain_id = chainId;
    if (routingStrategy === "per_party") body.party_destinations = partyDestinations;

    const r = await deps.api("POST", ROUTE_PATHS.API_PLANS_CONSOLIDATION_GENERATE, body);
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Dry-run consolidation plan generated");
    void loadInventoryOperations();
  }

  async function approveConsolidationPlan(planId: string): Promise<void> {
    const r = await deps.api("POST", ROUTE_PATHS.API_PLANS_CONSOLIDATION_APPROVE, {
      plan_id: planId,
      step_ids: [],
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Reviewable plan steps approved");
    void loadInventoryOperations();
  }

  async function simulateConsolidationPlan(planId: string): Promise<void> {
    const r = await deps.api("POST", ROUTE_PATHS.API_PLANS_CONSOLIDATION_SIMULATE, {
      plan_id: planId,
      step_ids: [],
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Plan preflight simulation updated");
    void loadInventoryOperations();
  }

  async function exportConsolidationPlan(
    planId: string,
    format = "call_manifest",
    actionEl?: unknown,
  ): Promise<void> {
    const safeAddress =
      format === "safe_tx_builder" ? safeAddressForExportAction(actionEl) : null;
    if (format === "safe_tx_builder" && !safeAddress) {
      deps.toast("Safe address is required", "error");
      return;
    }

    const r = (await deps.api("POST", ROUTE_PATHS.API_PLANS_CONSOLIDATION_EXPORT, {
      plan_id: planId,
      step_ids: [],
      format,
      safe_address: safeAddress,
    })) as ConsolidationPlanExportResponse & { error?: string };
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }

    deps.downloadJson(exportFilename(r), r);
    deps.toast(
      "Exported " +
        String(r.exported_steps || 0) +
        " step(s); skipped " +
        String((r.skipped_steps || []).length),
    );
  }

  async function enqueuePlanStep(planId: string, stepId: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Enqueue plan step",
      body:
        "Enqueue plan step " +
        stepId +
        " as an execution queue job? The daemon re-validates every gate; " +
        "queued plan-step jobs stay blocked until execution is enabled (W7.3).",
      actionLabel: "Enqueue step",
    });
    if (!confirmed) {
      return;
    }
    const r = await deps.api("POST", ROUTE_PATHS.API_PLANS_ENQUEUE_STEP, {
      plan_id: planId,
      step_id: stepId,
      confirm: true,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Plan step queued as job " + String(r.job?.id || "?"));
    void loadInventoryOperations();
  }

  async function enqueuePlanBulk(planId: string): Promise<void> {
    // Probe with an empty confirmation: the daemon computes the exact
    // expected phrase from the CURRENTLY eligible steps and returns it in
    // the machine-readable `action` field (nothing is enqueued).
    const probe = await deps.api("POST", ROUTE_PATHS.API_PLANS_ENQUEUE_PLAN, {
      plan_id: planId,
      confirmation: "",
    });
    const expected = typeof probe.action === "string" ? probe.action : null;
    if (!expected) {
      deps.toast(probe.error || "No plan steps are eligible for enqueue", "error");
      return;
    }
    const confirmed = await confirmTypedDialog({
      title: "Enqueue all eligible plan steps",
      body:
        "This enqueues every eligible step of the plan as execution queue jobs. " +
        "The daemon re-validates every gate before executing; steps that pass " +
        "their checks are signed and broadcast on-chain.",
      phrase: expected,
      actionLabel: "Enqueue all",
    });
    if (!confirmed) return;
    const r = await deps.api("POST", "/api/plans/enqueue-plan", {
      plan_id: planId,
      confirmation: expected,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast(
      "Enqueued " +
        String((r.enqueued || []).length) +
        " step(s); skipped " +
        String((r.skipped || []).length),
    );
    void loadInventoryOperations();
  }

  async function exportInventoryReport(): Promise<void> {
    const [watchBook, inventory, risks, plans] = await Promise.all([
      deps.api("GET", ROUTE_PATHS.API_INVENTORY_WATCH_ADDRESSES),
      deps.api("GET", ROUTE_PATHS.API_INVENTORY_WALLETS),
      deps.api("GET", ROUTE_PATHS.API_RISK_FINDINGS),
      deps.api("GET", ROUTE_PATHS.API_PLANS_CONSOLIDATION),
    ]);
    if (watchBook.error || inventory.error || risks.error || plans.error) {
      deps.toast(watchBook.error || inventory.error || risks.error || plans.error, "error");
      return;
    }
    const report = buildInventoryReport(
      inventory,
      risks.findings || [],
      plans.plans || [],
      watchBook.entries || [],
    );
    deps.downloadJson(inventoryReportFilename(report.generated_at_unix), report);
    deps.toast("Inventory report exported");
  }

  return {
    renderChainProfiles,
    renderInventoryState,
    renderWatchAddressBook,
    renderTokenRegistry,
    renderRiskCatalog,
    renderRiskFindings,
    renderConsolidationPlans,
    renderPlanPartyDestinations,
    loadInventoryOperations,
    upsertChainProfile,
    deleteChainProfile,
    scanInventoryEvm,
    loadWatchAddressBookEntry,
    upsertWatchAddressBookEntry,
    upsertBulkWatchAddressBookEntries,
    toggleWatchAddressBookEntry,
    deleteWatchAddressBookEntry,
    upsertNftMetadataOptIn,
    toggleNftMetadataOptIn,
    deleteNftMetadataOptIn,
    saveNftMetadataSettings,
    fetchNftMetadata,
    cancelDiscoveryJob,
    resumeDiscoveryJob,
    importTokenRegistry,
    deleteTokenRegistryList,
    loadRiskFindings,
    upsertRiskCatalogEntry,
    deleteRiskCatalogEntry,
    generateConsolidationPlan,
    approveConsolidationPlan,
    simulateConsolidationPlan,
    exportConsolidationPlan,
    enqueuePlanStep,
    enqueuePlanBulk,
    exportInventoryReport,
  };
}

export function parseWatchAddressProbes(
  bulkInput: string | null | undefined,
  singleAddress?: string | null,
  singleLabel?: string | null,
): WatchAddressProbe[] {
  const probes: WatchAddressProbe[] = [];
  const addProbe = (probe: WatchAddressProbe | null) => {
    if (!probe) return;
    const key = probe.address.toLowerCase();
    const existing = probes.find((item) => item.address.toLowerCase() === key);
    if (existing) {
      if (!existing.label && probe.label) existing.label = probe.label;
      return;
    }
    probes.push(probe);
  };

  addProbe(buildWatchAddressProbe(singleAddress, singleLabel));
  (bulkInput || "")
    .split(/\r?\n/)
    .map(parseWatchAddressLine)
    .forEach(addProbe);
  return probes;
}

export function buildInventoryReport(
  inventory: any,
  riskFindings: any[],
  consolidationPlans: ConsolidationPlan[],
  watchAddressBook: WatchAddressBookEntry[] = [],
  generatedAtUnix = Math.floor(Date.now() / 1000),
) {
  const addresses = inventory.addresses || [];
  const holdings = inventory.holdings || [];
  const watchAddresses = addresses.filter((address: any) => address.wallet_family === "eth-watch");
  const activeAddresses = addresses.filter((address: any) => address.activity_state !== "empty");
  const blockedPlanSteps = consolidationPlans.reduce(
    (count, plan) => count + plan.steps.filter((step) => step.blockers.length > 0).length,
    0,
  );
  return {
    generated_at_unix: generatedAtUnix,
    summary: {
      discovery_job_count: (inventory.jobs || []).length,
      address_count: addresses.length,
      active_address_count: activeAddresses.length,
      watch_address_count: watchAddresses.length,
      saved_watch_address_count: watchAddressBook.length,
      holding_count: holdings.length,
      risk_finding_count: riskFindings.length,
      consolidation_plan_count: consolidationPlans.length,
      blocked_plan_step_count: blockedPlanSteps,
    },
    watch_address_book: watchAddressBook,
    watch_addresses: watchAddresses,
    addresses,
    holdings,
    risk_findings: riskFindings,
    consolidation_plans: consolidationPlans,
  };
}

function parseWatchAddressLine(line: string): WatchAddressProbe | null {
  const trimmed = line.trim();
  if (!trimmed || trimmed.startsWith("#")) return null;
  const commaIndex = trimmed.indexOf(",");
  if (commaIndex >= 0) {
    return buildWatchAddressProbe(trimmed.slice(0, commaIndex), trimmed.slice(commaIndex + 1));
  }
  const colonIndex = trimmed.indexOf(":");
  if (colonIndex >= 0) {
    return buildWatchAddressProbe(trimmed.slice(0, colonIndex), trimmed.slice(colonIndex + 1));
  }
  return buildWatchAddressProbe(trimmed, null);
}

function buildWatchAddressProbe(
  address: string | null | undefined,
  label: string | null | undefined,
): WatchAddressProbe | null {
  const trimmedAddress = (address || "").trim();
  if (!trimmedAddress) return null;
  const trimmedLabel = (label || "").trim();
  return {
    address: trimmedAddress,
    ...(trimmedLabel ? { label: trimmedLabel } : {}),
  };
}

function parseTagList(value: string | null | undefined): string[] {
  return (value || "")
    .split(",")
    .map((tag) => tag.trim())
    .filter(Boolean)
    .filter((tag, index, tags) => tags.findIndex((item) => item.toLowerCase() === tag.toLowerCase()) === index);
}

function inventoryReportFilename(generatedAtUnix: number): string {
  return "sigillum-inventory-report-" + generatedAtUnix + ".json";
}

function safeAddressForExportAction(actionEl: unknown): string | null {
  const maybeButton = actionEl as {
    closest?: (selector: string) => Element | null;
  };
  const row = maybeButton?.closest?.("li");
  const input = row?.querySelector("[data-plan-safe-address]") as
    | HTMLInputElement
    | null
    | undefined;
  const fromInput = input?.value?.trim();
  if (fromInput) return fromInput;
  // No modal prompt here: the row input is the only source, so the export
  // flow stays inside the styled UI and the fake-DOM test harness.
  return null;
}

function exportFilename(response: ConsolidationPlanExportResponse): string {
  const plan = response.plan_id.replace(/[^a-zA-Z0-9_.-]/g, "_");
  return "sigillum-" + plan + "-" + response.format + ".json";
}
