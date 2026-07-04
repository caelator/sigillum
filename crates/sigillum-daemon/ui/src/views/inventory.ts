import type {
  ChainProfile,
  ConsolidationPlan,
  ConsolidationPlanGenerateRequest,
  ConsolidationPlanExportResponse,
  ConsolidationPlanStep,
  Counterparty,
  PartyDestination,
  RiskCatalogEntry,
  RiskFinding,
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
import { esc, escAttr, statusPill } from "../render/html";

export interface InventoryViewModel {
  enabledChains: ChainProfile[];
  watchAddressBook: WatchAddressBookEntry[];
  discoveryJobs: WalletDiscoveryJob[];
  riskCatalog: RiskCatalogEntry[];
  riskFindings: RiskFinding[];
  consolidationPlans: ConsolidationPlan[];
}

export function summarizeInventory(view: InventoryViewModel): string {
  return [
    `${view.enabledChains.length} enabled chains`,
    `${view.watchAddressBook.length} saved watch addresses`,
    `${view.discoveryJobs.length} discovery jobs`,
    `${view.riskCatalog.length} risk catalog entries`,
    `${view.riskFindings.length} risk findings`,
    `${view.consolidationPlans.length} plans`,
  ].join(" | ");
}

export function inventoryNeedsOperatorReview(view: InventoryViewModel): boolean {
  return (
    view.riskFindings.length > 0 ||
    view.consolidationPlans.some((plan) => plan.summary?.review_required_steps > 0)
  );
}

export function blockerLabel(code: string): string {
  switch (code) {
    case "missing_party_destination":
      return "no destination set for this payer";
    case "missing_destination":
      return "no destination set";
    case "cross_party_linkage":
      return "Destination shared with another payer";
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
      const r = await deps.api("GET", "/api/treasury/parties");
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
    if (chainId === null || chainId === undefined) return "-";
    const numericChainId = Number(chainId);
    const profile = latestChainProfiles.find(
      (chain) => chain.enabled && chain.chain_id === numericChainId,
    );
    return profile ? `${numericChainId} (${profile.name})` : String(chainId);
  }

  function renderInventoryState(inventory: any): void {
    renderEntityList("inventoryJobList", inventory.jobs || [], "No discovery jobs yet.", (job: any) => {
      const chainIds = job.chain_ids || [];
      const chainLabels = chainIds.map((chainId: number) => chainLabel(chainId)).join(", ");
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
        "</div></div>" +
        '<div class="entity-actions">' +
        '<button class="btn-ghost" data-action="cancelDiscoveryJob" data-arg0="' +
        escAttr(job.id) +
        '">Cancel</button>' +
        '<button class="btn-ghost" data-action="resumeDiscoveryJob" data-arg0="' +
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
        esc(address.native_balance_wei_hex || "0x0") +
        " · txCount=" +
        esc(String(address.transaction_count || 0)) +
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
        esc(holding.amount_hex) +
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
          esc(String(entry.updated_at_unix || "-")) +
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
            '<br><span class="linkage-warning">Scope: flags payers that would sweep to the same destination. Does not cover gas-funding links, amount/timing correlation, downstream re-merging, or multi-hop flows — keep per-party destinations separate.</span>' +
            "</div>"
          : "";
        const safeAddressInput =
          '<input type="text" class="input-wide plan-safe-address" data-plan-safe-address placeholder="Safe address" autocomplete="off">';
        const stepLines = (plan.steps || [])
          .slice(0, 8)
          .map((step: ConsolidationPlanStep) => {
            const evidence = (step.simulation_evidence || []).join(" | ");
            const linkageWarnings = step.linkage_warnings || [];
            const blockers = (step.blockers || []).map(blockerLabel).join(", ");
            return (
              '<div class="entity-meta">' +
              esc(step.action) +
              " " +
              statusPill(step.status) +
              " · " +
              esc(step.asset_kind) +
              (step.token_id_hex ? " #" + esc(step.token_id_hex) : "") +
              (step.counterparty_address
                ? " · spender/operator=" + esc(step.counterparty_address)
                : "") +
              (step.protocol_address ? " · protocol=" + esc(step.protocol_address) : "") +
              (step.claim_adapter ? " · claimAdapter=" + esc(step.claim_adapter) : "") +
              (step.claim_index_hex ? " · claimIndex=" + esc(step.claim_index_hex) : "") +
              ((step.claim_proof || []).length
                ? " · proofWords=" + esc(String((step.claim_proof || []).length))
                : "") +
              " · amount=" +
              esc(step.amount_hex) +
              " · simulation=" +
              esc(step.simulation_status || "not_run") +
              " · blockers=" +
              esc(blockers || "-") +
              (evidence ? "<br>evidence=" + esc(evidence) : "") +
              (linkageWarnings.length
                ? '<br><span class="linkage-warning">privacy: ' +
                  esc(linkageWarnings.join(", ")) +
                  "</span>"
                : "") +
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
      const [chains, watchBook, inventory, catalog, risks, plans] = await Promise.all([
        deps.api("GET", "/api/chains"),
        deps.api("GET", "/api/inventory/watch-addresses"),
        deps.api("GET", "/api/inventory/wallets"),
        deps.api("GET", "/api/risk/catalog"),
        deps.api("GET", "/api/risk/findings"),
        deps.api("GET", "/api/plans/consolidation"),
      ]);
      if (!chains.error) {
        latestChainProfiles = chains.profiles || [];
        renderChainProfiles(latestChainProfiles);
      }
      if (!watchBook.error) renderWatchAddressBook(watchBook.entries || []);
      if (!inventory.error) renderInventoryState(inventory);
      if (!catalog.error) renderRiskCatalog(catalog.entries || []);
      if (!risks.error) renderRiskFindings(risks.findings || []);
      if (!plans.error) renderConsolidationPlans(plans.plans || []);
    } catch (_) {}
  }

  async function upsertChainProfile(): Promise<void> {
    const name = textValue("chainProfileName");
    const family = textValue("chainProfileFamily");
    if (!name || !family) {
      deps.toast("Chain profile name and family are required", "error");
      return;
    }
    const r = await deps.api("POST", "/api/chains/upsert", {
      name,
      chain_family: family,
      chain_id: optionalNumberValue("chainProfileId"),
      provider_profile: optionalTextValue("chainProfileProvider"),
      native_symbol: optionalTextValue("chainProfileNativeSymbol"),
      native_decimals: optionalNumberValue("chainProfileNativeDecimals"),
      finality_blocks: optionalNumberValue("chainProfileFinalityBlocks"),
      permit2_address: optionalTextValue("chainProfilePermit2Address"),
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
    ]);
    deps.toast("Chain profile saved");
    void loadInventoryOperations();
  }

  async function deleteChainProfile(name: string): Promise<void> {
    if (!confirm('Delete chain profile "' + name + '"?')) return;
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
    const r = await deps.api("POST", "/api/inventory/scan/evm", body);
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Inventory scan completed");
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
    const r = await deps.api("POST", "/api/inventory/watch-addresses/upsert", {
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
      const r = await deps.api("POST", "/api/inventory/watch-addresses/upsert", {
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
    const r = await deps.api("POST", "/api/inventory/watch-addresses/upsert", {
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
    if (!confirm('Delete saved watch address "' + address + '"?')) return;
    const r = await deps.api("POST", "/api/inventory/watch-addresses/delete", { address });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Watch address deleted");
    void loadInventoryOperations();
  }

  async function cancelDiscoveryJob(id: string): Promise<void> {
    const r = await deps.api("POST", "/api/discovery/jobs/cancel", { id });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Discovery job marked canceled");
    void loadInventoryOperations();
  }

  async function resumeDiscoveryJob(id: string): Promise<void> {
    const r = await deps.api("POST", "/api/discovery/jobs/resume", { id });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Discovery job marked for resume");
    void loadInventoryOperations();
  }

  async function loadRiskFindings(): Promise<void> {
    const [catalog, risks] = await Promise.all([
      deps.api("GET", "/api/risk/catalog"),
      deps.api("GET", "/api/risk/findings"),
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
    const r = await deps.api("POST", "/api/risk/catalog/upsert", {
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
    if (!confirm('Delete risk catalog entry "' + address + '"?')) return;
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

    const r = await deps.api("POST", "/api/plans/consolidation/generate", body);
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Dry-run consolidation plan generated");
    void loadInventoryOperations();
  }

  async function approveConsolidationPlan(planId: string): Promise<void> {
    const r = await deps.api("POST", "/api/plans/consolidation/approve", {
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
    const r = await deps.api("POST", "/api/plans/consolidation/simulate", {
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

    const r = (await deps.api("POST", "/api/plans/consolidation/export", {
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

  async function exportInventoryReport(): Promise<void> {
    const [watchBook, inventory, risks, plans] = await Promise.all([
      deps.api("GET", "/api/inventory/watch-addresses"),
      deps.api("GET", "/api/inventory/wallets"),
      deps.api("GET", "/api/risk/findings"),
      deps.api("GET", "/api/plans/consolidation"),
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
    cancelDiscoveryJob,
    resumeDiscoveryJob,
    loadRiskFindings,
    upsertRiskCatalogEntry,
    deleteRiskCatalogEntry,
    generateConsolidationPlan,
    approveConsolidationPlan,
    simulateConsolidationPlan,
    exportConsolidationPlan,
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
  return window.prompt("Safe address")?.trim() || null;
}

function exportFilename(response: ConsolidationPlanExportResponse): string {
  const plan = response.plan_id.replace(/[^a-zA-Z0-9_.-]/g, "_");
  return "sigillum-" + plan + "-" + response.format + ".json";
}
