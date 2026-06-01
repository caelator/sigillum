import type {
  ChainProfile,
  ConsolidationPlanSummary,
  RiskFinding,
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
  discoveryJobs: WalletDiscoveryJob[];
  riskFindings: RiskFinding[];
  consolidationPlans: ConsolidationPlanSummary[];
}

export function summarizeInventory(view: InventoryViewModel): string {
  return [
    `${view.enabledChains.length} enabled chains`,
    `${view.discoveryJobs.length} discovery jobs`,
    `${view.riskFindings.length} risk findings`,
    `${view.consolidationPlans.length} plans`,
  ].join(" | ");
}

export function inventoryNeedsOperatorReview(view: InventoryViewModel): boolean {
  return (
    view.riskFindings.length > 0 ||
    view.consolidationPlans.some((plan) => plan.review_required_step_count > 0)
  );
}

export interface InventoryActionsDeps {
  api: (method: string, path: string, body?: unknown) => Promise<any>;
  toast: (message: string, type?: string) => void;
}

function input(id: string): HTMLInputElement {
  return document.getElementById(id) as HTMLInputElement;
}

export function createInventoryActions(deps: InventoryActionsDeps) {
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
        "<br>" +
        "capabilities=" +
        esc((profile.capabilities || []).join(", ") || "-") +
        " · source=" +
        esc(profile.source || "-") +
        "</div></div>" +
        '<div class="entity-actions">' +
        '<button class="btn-danger" data-action="deleteChainProfile" data-arg0="' +
        escAttr(profile.name) +
        '">Delete</button>' +
        "</div></li>",
    );
  }

  function renderInventoryState(inventory: any): void {
    renderEntityList("inventoryJobList", inventory.jobs || [], "No discovery jobs yet.", (job: any) => {
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
      "No discovered addresses yet.",
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
        esc(String(address.chain_id)) +
        " · path=" +
        esc(address.derivation_path) +
        "<br>" +
        "native=" +
        esc(address.native_balance_wei_hex || "0x0") +
        " · txCount=" +
        esc(String(address.transaction_count || 0)) +
        "</div></div></li>",
    );
    renderEntityList(
      "inventoryHoldingList",
      inventory.holdings || [],
      "No positive asset holdings detected yet.",
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
        " · amount=" +
        esc(holding.amount_hex) +
        "<br>" +
        esc(holding.wallet_family) +
        "/" +
        esc(holding.wallet_profile) +
        " · provider=" +
        esc(holding.provider_profile) +
        " · source=" +
        esc(holding.source || "-") +
        "</div></div></li>",
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

  function renderConsolidationPlans(plans: any[]): void {
    renderEntityList(
      "consolidationPlanList",
      plans,
      "No consolidation plans generated yet.",
      (plan) => {
        const summary = plan.summary || {};
        const stepLines = (plan.steps || [])
          .slice(0, 8)
          .map(
            (step: any) =>
              '<div class="entity-meta">' +
              esc(step.action) +
              " " +
              statusPill(step.status) +
              " · " +
              esc(step.asset_kind) +
              (step.token_id_hex ? " #" + esc(step.token_id_hex) : "") +
              " · amount=" +
              esc(step.amount_hex) +
              " · blockers=" +
              esc((step.blockers || []).join(", ") || "-") +
              "</div>",
          )
          .join("");
        return (
          '<li><div class="entity-main">' +
          '<div class="entity-title">' +
          esc(plan.id) +
          " " +
          statusPill(plan.status) +
          "</div>" +
          '<div class="entity-meta">' +
          "steps=" +
          esc(String(summary.total_steps || 0)) +
          " · blocked=" +
          esc(String(summary.blocked_steps || 0)) +
          " · review=" +
          esc(String(summary.review_required_steps || 0)) +
          " · approved=" +
          esc(String(summary.approved_steps || 0)) +
          "</div>" +
          stepLines +
          "</div>" +
          '<div class="entity-actions">' +
          '<button class="btn-ghost" data-action="approveConsolidationPlan" data-arg0="' +
          escAttr(plan.id) +
          '">Approve Reviewable</button>' +
          "</div></li>"
        );
      },
    );
  }

  async function loadInventoryOperations(): Promise<void> {
    try {
      const [chains, inventory, risks, plans] = await Promise.all([
        deps.api("GET", "/api/inventory/chains"),
        deps.api("GET", "/api/inventory/wallets"),
        deps.api("GET", "/api/risk/findings"),
        deps.api("GET", "/api/plans/consolidation"),
      ]);
      if (!chains.error) renderChainProfiles(chains.profiles || []);
      if (!inventory.error) renderInventoryState(inventory);
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
    const r = await deps.api("POST", "/api/inventory/chains/upsert", {
      name,
      chain_family: family,
      chain_id: optionalNumberValue("chainProfileId"),
      provider_profile: optionalTextValue("chainProfileProvider"),
      native_symbol: optionalTextValue("chainProfileNativeSymbol"),
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
    ]);
    deps.toast("Chain profile saved");
    void loadInventoryOperations();
  }

  async function deleteChainProfile(name: string): Promise<void> {
    if (!confirm('Delete chain profile "' + name + '"?')) return;
    const r = await deps.api("POST", "/api/inventory/chains/delete", { name });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Chain profile deleted");
    void loadInventoryOperations();
  }

  async function scanInventoryEvm(): Promise<void> {
    const token = optionalTextValue("inventoryTokenAddress");
    const spender = optionalTextValue("inventoryAllowanceSpender");
    const nftOperator = optionalTextValue("inventoryNftOperator");
    const r = await deps.api("POST", "/api/inventory/scan/evm", {
      wallet_family: optionalTextValue("inventoryWalletFamily"),
      wallet_profile: optionalTextValue("inventoryWalletProfile"),
      provider_profile: optionalTextValue("inventoryProviderProfile"),
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
      discover_erc721_transfers: input("inventoryDiscoverErc721Transfers").checked,
      discover_erc1155_transfers: input("inventoryDiscoverErc1155Transfers").checked,
      discover_nft_operator_approvals: input("inventoryDiscoverNftOperatorApprovals").checked,
      nft_operator_addresses: nftOperator ? [nftOperator] : [],
      nft_operator_approval_limit: optionalNumberValue("inventoryNftOperatorApprovalLimit"),
      nft_discovery_from_block: optionalTextValue("inventoryNftDiscoveryFromBlock"),
      nft_discovery_to_block: optionalTextValue("inventoryNftDiscoveryToBlock"),
      nft_discovery_limit: optionalNumberValue("inventoryNftDiscoveryLimit"),
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Inventory scan completed");
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
    const r = await deps.api("GET", "/api/risk/findings");
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    renderRiskFindings(r.findings || []);
    deps.toast("Risk findings refreshed");
  }

  async function generateConsolidationPlan(): Promise<void> {
    const r = await deps.api("POST", "/api/plans/consolidation/generate", {
      destination_address: optionalTextValue("planDestinationAddress"),
      include_watch_only: true,
      auto_queue_low_risk: false,
    });
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

  return {
    renderChainProfiles,
    renderInventoryState,
    renderRiskFindings,
    renderConsolidationPlans,
    loadInventoryOperations,
    upsertChainProfile,
    deleteChainProfile,
    scanInventoryEvm,
    cancelDiscoveryJob,
    resumeDiscoveryJob,
    loadRiskFindings,
    generateConsolidationPlan,
    approveConsolidationPlan,
  };
}
