import type {
  Counterparty,
  TreasuryAllowedDestination,
  TreasuryChainSummary,
  TreasuryGroupSummary,
  TreasuryOverviewResponse,
  TreasuryPlanSummary,
  TreasuryPolicy,
  TreasuryPolicyUpdateRequest,
  TreasuryReceiveAllocation,
  TreasuryRiskSummary,
  TreasuryRoutingStatus,
} from "../contracts";
import { setTextById as setText } from "../render/dom";
import {
  clearFields,
  optionalTextValue,
  renderEntityList,
  textValue,
} from "../render/forms";
import { esc, escAttr, formatTs, statBox, statusPill } from "../render/html";

const WEI_PER_ETH = 10n ** 18n;
const DEFAULT_HOT_REFILL_WEI_HEX = "0xde0b6b3a7640000";

export function formatWeiHexAsEth(weiHex: string): string {
  if (typeof weiHex !== "string") return "0";
  const trimmed = weiHex.trim();
  if (!/^0x[0-9a-fA-F]+$/.test(trimmed)) return "0";
  let wei: bigint;
  try {
    wei = BigInt(trimmed);
  } catch (_) {
    return "0";
  }
  const whole = wei / WEI_PER_ETH;
  const fraction = wei % WEI_PER_ETH;
  if (fraction === 0n) return whole.toString();
  const fractionText = fraction.toString().padStart(18, "0").replace(/0+$/, "");
  return whole.toString() + "." + fractionText;
}

export function parseEthToWeiHex(value: string): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  const match = /^(\d+)(?:\.(\d+))?$/.exec(trimmed);
  if (!match) return null;
  const fractionDigits = match[2] || "";
  if (fractionDigits.length > 18) return null;
  const wei = BigInt(match[1]) * WEI_PER_ETH + BigInt(fractionDigits.padEnd(18, "0"));
  return "0x" + wei.toString(16);
}

export function parseTreasuryDestinationLines(
  value: string | null | undefined,
): TreasuryAllowedDestination[] {
  const destinations: TreasuryAllowedDestination[] = [];
  (value || "").split(/\r?\n/).forEach((line) => {
    const trimmed = line.trim();
    if (!trimmed) return;
    const colonIndex = trimmed.indexOf(":");
    if (colonIndex < 0) {
      destinations.push({ address: trimmed });
      return;
    }
    const address = trimmed.slice(0, colonIndex).trim();
    if (!address) return;
    const label = trimmed.slice(colonIndex + 1).trim();
    destinations.push(label ? { address, label } : { address });
  });
  return destinations;
}

function formatDestinationLines(
  destinations: TreasuryAllowedDestination[] | undefined,
): string {
  return (destinations || [])
    .map((destination) =>
      destination.label
        ? destination.address + ":" + destination.label
        : destination.address,
    )
    .join("\n");
}

function capAsEth(weiHex: string | null | undefined): string {
  return weiHex ? formatWeiHexAsEth(weiHex) + " ETH" : "-";
}

function nativeAmount(
  weiHex: string | null | undefined,
  symbol?: string | null,
): string {
  const amount = formatWeiHexAsEth(weiHex || "");
  return symbol ? amount + " " + symbol : amount;
}

function shortAddress(value: string): string {
  const trimmed = value.trim();
  if (trimmed.startsWith("0x") && trimmed.length > 14) {
    return trimmed.slice(0, 6) + "..." + trimmed.slice(-4);
  }
  return trimmed.length > 24 ? trimmed.slice(0, 12) + "..." + trimmed.slice(-6) : trimmed;
}

export interface TreasuryActionsDeps {
  api: (method: string, path: string, body?: unknown) => Promise<any>;
  toast: (message: string, type?: string) => void;
}

export function createTreasuryActions(deps: TreasuryActionsDeps) {
  let treasuryParties: Counterparty[] = [];

  function partyNameById(id: string | null | undefined): string | undefined {
    if (!id) return undefined;
    const p = treasuryParties.find((x) => x.id === id);
    return p ? p.name : undefined;
  }

  function renderTreasuryChains(
    chains: TreasuryChainSummary[],
  ): void {
    renderEntityList(
      "treasuryChainList",
      chains,
      "No per-chain balances yet. Run a wallet inventory scan first.",
      (chain) =>
        '<li><div class="entity-main">' +
        '<div class="entity-title">' +
        "chain " +
        esc(String(chain.chain_id)) +
        " · " +
        esc(chain.native_symbol || "-") +
        " " +
        statusPill((chain.funded_address_count || 0) > 0 ? "funded" : "empty") +
        "</div>" +
        '<div class="entity-meta">' +
        "addresses=" +
        esc(String(chain.funded_address_count || 0)) +
        "/" +
        esc(String(chain.address_count || 0)) +
        " funded · native=" +
        esc(nativeAmount(chain.native_total_wei_hex, chain.native_symbol)) +
        "</div></div></li>",
    );
  }

  function renderTreasuryGroups(
    groups: TreasuryGroupSummary[],
    symbolByChain: Map<number, string>,
  ): void {
    renderEntityList(
      "treasuryGroupList",
      groups,
      "No wallet groups yet. Run a wallet inventory scan first.",
      (group) =>
        '<li><div class="entity-main">' +
        '<div class="entity-title">' +
        esc(group.wallet_family) +
        "/" +
        esc(group.wallet_profile) +
        " " +
        statusPill((group.funded_address_count || 0) > 0 ? "funded" : "empty") +
        ((group.approval_exposure_count || 0) > 0
          ? " " + statusPill("approval exposure")
          : "") +
        ((group.dormant_candidate_count || 0) > 0
          ? " " + statusPill("dormant")
          : "") +
        "</div>" +
        '<div class="entity-meta">' +
        "chain=" +
        esc(String(group.chain_id)) +
        " · addresses=" +
        esc(String(group.funded_address_count || 0)) +
        "/" +
        esc(String(group.address_count || 0)) +
        " funded · native=" +
        esc(nativeAmount(group.native_total_wei_hex, symbolByChain.get(group.chain_id))) +
        " · signers=" +
        esc(String(group.signer_address_count || 0)) +
        " · watchOnly=" +
        esc(String(group.watch_only_address_count || 0)) +
        "<br>" +
        "erc20=" +
        esc(String(group.erc20_holding_count || 0)) +
        " · nft=" +
        esc(String(group.nft_holding_count || 0)) +
        " · defi=" +
        esc(String(group.defi_holding_count || 0)) +
        " · claimable=" +
        esc(String(group.claimable_holding_count || 0)) +
        " · approvals=" +
        esc(String(group.approval_exposure_count || 0)) +
        " · dormant=" +
        esc(String(group.dormant_candidate_count || 0)) +
        "</div></div></li>",
    );
  }

  function renderTreasuryRouting(routing: TreasuryRoutingStatus[]): void {
    renderEntityList(
      "treasuryRoutingList",
      routing,
      "No treasury routing configured yet.",
      (route) =>
        '<li><div class="entity-main">' +
        '<div class="entity-title">' +
        esc(route.wallet_profile) +
        " " +
        statusPill(route.routing_ready ? "ready" : "unconfigured") +
        "</div>" +
        '<div class="entity-meta">' +
        "hot=" +
        esc(route.hot_address || "-") +
        (route.hot_native_balance_wei_hex
          ? " (" + esc(formatWeiHexAsEth(route.hot_native_balance_wei_hex)) + ")"
          : "") +
        "<br>" +
        "treasury=" +
        esc(route.treasury_address || "-") +
        (route.treasury_native_balance_wei_hex
          ? " (" + esc(formatWeiHexAsEth(route.treasury_native_balance_wei_hex)) + ")"
          : "") +
        "<br>" +
        "defaultDestination=" +
        esc(route.default_destination_address || "-") +
        "</div></div></li>",
    );
  }

  function renderTreasuryRiskAndPlans(
    risk: TreasuryRiskSummary,
    plans: TreasuryPlanSummary,
  ): void {
    type RiskPlanRow =
      | { kind: "risk"; risk: TreasuryRiskSummary }
      | { kind: "plans"; plans: TreasuryPlanSummary };
    const rows: RiskPlanRow[] = [];
    if (risk) rows.push({ kind: "risk", risk });
    if (plans) rows.push({ kind: "plans", plans });
    renderEntityList(
      "treasuryRiskPlanList",
      rows,
      "No risk or plan summary yet.",
      (row) => {
        if (row.kind === "risk") {
          return (
            '<li><div class="entity-main">' +
            '<div class="entity-title">Risk findings ' +
            statusPill((row.risk.total_findings || 0) > 0 ? "detected" : "ok") +
            "</div>" +
            '<div class="entity-meta">' +
            "critical=" +
            esc(String(row.risk.critical_findings || 0)) +
            " · high=" +
            esc(String(row.risk.high_findings || 0)) +
            " · medium=" +
            esc(String(row.risk.medium_findings || 0)) +
            " · low=" +
            esc(String(row.risk.low_findings || 0)) +
            " · total=" +
            esc(String(row.risk.total_findings || 0)) +
            "</div></div></li>"
          );
        }
        const policyViolations =
          row.plans.policy_violations || row.plans.latest_policy_violations || [];
        return (
          '<li><div class="entity-main">' +
          '<div class="entity-title">Consolidation plans ' +
          statusPill(row.plans.latest_plan_status || "none") +
          "</div>" +
          '<div class="entity-meta">' +
          "plans=" +
          esc(String(row.plans.total_plans || 0)) +
          " · latest=" +
          esc(row.plans.latest_plan_id || "-") +
          "<br>" +
          "review=" +
          esc(String(row.plans.latest_review_required_steps || 0)) +
          " · approved=" +
          esc(String(row.plans.latest_approved_steps || 0)) +
          " · executable=" +
          esc(String(row.plans.latest_executable_steps || 0)) +
          " · blocked=" +
          esc(String(row.plans.latest_blocked_steps || 0)) +
          (policyViolations.length
            ? " · policyViolations=" + esc(policyViolations.join(", "))
            : "") +
          "</div></div></li>"
        );
      },
    );
  }

  function renderTreasuryOverview(overview: TreasuryOverviewResponse): void {
    const summaryEl = document.getElementById("treasuryOverviewStats");
    if (summaryEl) {
      const tiles = [
        statBox(String(overview.tracked_address_count || 0), "Tracked Addresses"),
        statBox(String(overview.funded_address_count || 0), "Funded"),
        statBox(String(overview.signer_address_count || 0), "Signer"),
        statBox(String(overview.watch_only_address_count || 0), "Watch-Only"),
      ];
      if (overview.receive) {
        tiles.push(
          statBox(String(overview.receive.active_allocations || 0), "Receive Active"),
        );
      }
      summaryEl.innerHTML = tiles.join("");
    }
    setText("treasuryGeneratedAt", "Updated " + formatTs(overview.generated_at_unix));

    const symbolByChain = new Map<number, string>();
    (overview.chains || []).forEach((chain) => {
      if (chain.native_symbol) symbolByChain.set(chain.chain_id, chain.native_symbol);
    });

    renderTreasuryChains(overview.chains || []);
    renderTreasuryGroups(overview.groups || [], symbolByChain);
    renderTreasuryRouting(overview.routing || []);
    renderTreasuryRiskAndPlans(overview.risk, overview.plans);
  }

  function renderTreasuryPolicy(policy: TreasuryPolicy | null): void {
    renderEntityList(
      "treasuryPolicyList",
      policy ? [policy] : [],
      "No treasury policy configured yet.",
      (current) => {
        const destinations = (current.allowed_destinations || [])
          .map(
            (destination) =>
              destination.address +
              (destination.label ? " (" + destination.label + ")" : ""),
          )
          .join(", ");
        return (
          '<li><div class="entity-main">' +
          '<div class="entity-title">Treasury policy ' +
          statusPill(current.enabled ? "enabled" : "disabled") +
          "</div>" +
          '<div class="entity-meta">' +
          "destinations=" +
          esc(destinations || "-") +
          "<br>" +
          "maxStep=" +
          esc(capAsEth(current.max_step_native_wei_hex)) +
          " · maxPlan=" +
          esc(capAsEth(current.max_plan_native_wei_hex)) +
          " · hotFloor=" +
          esc(capAsEth(current.hot_floor_wei_hex ?? DEFAULT_HOT_REFILL_WEI_HEX)) +
          " · hotTarget=" +
          esc(capAsEth(current.hot_target_wei_hex ?? DEFAULT_HOT_REFILL_WEI_HEX)) +
          " · requireSimulation=" +
          esc(String(current.require_simulation)) +
          " · blockCrossPartyLinkage=" +
          esc(String(Boolean(current.block_cross_party_linkage))) +
          " · simulationFreshnessSecs=" +
          esc(String(current.simulation_freshness_secs ?? 900)) +
          " · updated=" +
          esc(formatTs(current.updated_at_unix)) +
          "</div></div></li>"
        );
      },
    );
  }

  function input(id: string): HTMLInputElement | null {
    return document.getElementById(id) as HTMLInputElement | null;
  }

  let policyFormFingerprint: string | null = null;

  function prefillTreasuryPolicyForm(policy: TreasuryPolicy | null): void {
    const fingerprint = JSON.stringify(
      policy
        ? [
            policy.enabled,
            policy.allowed_destinations || [],
            policy.max_step_native_wei_hex || null,
            policy.max_plan_native_wei_hex || null,
            policy.hot_floor_wei_hex ?? DEFAULT_HOT_REFILL_WEI_HEX,
            policy.hot_target_wei_hex ?? DEFAULT_HOT_REFILL_WEI_HEX,
            policy.require_simulation,
            policy.block_cross_party_linkage,
            policy.simulation_freshness_secs,
          ]
        : null,
    );
    if (fingerprint === policyFormFingerprint) return;
    policyFormFingerprint = fingerprint;

    const enabledEl = input("treasuryPolicyEnabled");
    if (enabledEl) enabledEl.checked = policy ? policy.enabled : false;
    const requireSimEl = input("treasuryPolicyRequireSim");
    if (requireSimEl) requireSimEl.checked = policy ? policy.require_simulation : true;
    const blockLinkageEl = input("treasuryPolicyBlockLinkage");
    if (blockLinkageEl) {
      blockLinkageEl.checked = policy ? Boolean(policy.block_cross_party_linkage) : false;
    }
    const destinationsEl = input("treasuryPolicyDestinations");
    if (destinationsEl) {
      destinationsEl.value = policy
        ? formatDestinationLines(policy.allowed_destinations)
        : "";
    }
    const maxStepEl = input("treasuryPolicyMaxStepEth");
    if (maxStepEl) {
      maxStepEl.value = policy?.max_step_native_wei_hex
        ? formatWeiHexAsEth(policy.max_step_native_wei_hex)
        : "";
    }
    const maxPlanEl = input("treasuryPolicyMaxPlanEth");
    if (maxPlanEl) {
      maxPlanEl.value = policy?.max_plan_native_wei_hex
        ? formatWeiHexAsEth(policy.max_plan_native_wei_hex)
        : "";
    }
    const hotFloorEl = input("treasuryPolicyHotFloorEth");
    if (hotFloorEl) {
      hotFloorEl.value = formatWeiHexAsEth(
        policy?.hot_floor_wei_hex ?? DEFAULT_HOT_REFILL_WEI_HEX,
      );
    }
    const hotTargetEl = input("treasuryPolicyHotTargetEth");
    if (hotTargetEl) {
      hotTargetEl.value = formatWeiHexAsEth(
        policy?.hot_target_wei_hex ?? DEFAULT_HOT_REFILL_WEI_HEX,
      );
    }
    const freshnessEl = input("treasuryPolicyFreshnessSecs");
    if (freshnessEl) {
      freshnessEl.value = policy ? String(policy.simulation_freshness_secs ?? 900) : "900";
    }
  }

  function renderTreasuryReceiveAllocations(
    allocations: TreasuryReceiveAllocation[],
  ): void {
    renderEntityList(
      "treasuryReceiveList",
      allocations,
      {
        message:
          "No receive allocations yet. Allocate a labeled receive address for a counterparty.",
        actionLabel: "Allocate an address",
        action: "focusTreasuryReceive",
      },
      (allocation) =>
        '<li><div class="entity-main">' +
        '<div class="entity-title">' +
        esc(allocation.address) +
        " " +
        statusPill(allocation.status) +
        "</div>" +
        '<div class="entity-meta">' +
        esc(allocation.wallet_family) +
        "/" +
        esc(allocation.wallet_profile) +
        " · chain=" +
        esc(String(allocation.chain_id)) +
        (allocation.chain_id_assumed ? " (assumed mainnet)" : "") +
        " · purpose=" +
        esc(allocation.purpose) +
        (allocation.label ? " · label=" + esc(allocation.label) : "") +
        (allocation.counterparty_id
          ? " · party=" +
            esc(partyNameById(allocation.counterparty_id) || allocation.counterparty_id)
          : "") +
        "<br>" +
        "path=" +
        esc(allocation.derivation_path) +
        " · index=" +
        esc(String(allocation.address_index)) +
        "</div></div>" +
        (allocation.status === "active"
          ? '<div class="entity-actions">' +
            '<button class="btn-ghost" data-action="rotateTreasuryReceiveAddress" data-arg0="' +
            escAttr(allocation.id) +
            '">Rotate</button>' +
            "</div>"
          : "") +
        "</li>",
    );
  }

  function renderTreasuryParties(parties: Counterparty[]): void {
    renderEntityList(
      "treasuryPartyList",
      parties,
      {
        message:
          "No counterparties yet. Add a payer to hand out dedicated receive addresses.",
        actionLabel: "Add a party",
        action: "focusTreasuryParty",
      },
      (party) =>
        '<li><div class="entity-main">' +
        '<div class="entity-title">' +
        esc(party.name) +
        "</div>" +
        '<div class="entity-meta">' +
        (party.note ? esc(party.note) + "<br>" : "") +
        (party.sweep_destination_address
          ? "sweep &rarr; " + esc(shortAddress(party.sweep_destination_address)) + "<br>"
          : "") +
        "created=" +
        esc(formatTs(party.created_at_unix)) +
        "</div></div>" +
        '<div class="entity-actions">' +
        '<input type="text" class="mono input-wider" placeholder="Sweep destination" value="' +
        escAttr(party.sweep_destination_address || "") +
        '" data-party-sweep-dest-input="' +
        escAttr(party.id) +
        '">' +
        '<button class="btn-ghost" data-action="updateTreasuryPartySweepDest" data-arg0="' +
        escAttr(party.id) +
        '" data-self="append">Save dest</button>' +
        '<button class="btn-ghost" data-action="clearTreasuryPartySweepDest" data-arg0="' +
        escAttr(party.id) +
        '">Clear</button>' +
        '<button class="btn-ghost" data-action="deleteTreasuryParty" data-arg0="' +
        escAttr(party.id) +
        '">Delete</button>' +
        "</div>" +
        "</li>",
    );
  }

  async function loadTreasuryParties(): Promise<void> {
    try {
      const r = await deps.api("GET", "/api/treasury/parties");
      if (r.error) return;
      treasuryParties = (r.parties || []) as Counterparty[];
      renderTreasuryParties(treasuryParties);
      const select = document.getElementById(
        "treasuryReceiveParty",
      ) as HTMLSelectElement | null;
      if (select) {
        const previous = select.value;
        let html = '<option value="">No party (optional)</option>';
        treasuryParties.forEach((party) => {
          html +=
            '<option value="' +
            escAttr(party.id) +
            '">' +
            esc(party.name) +
            "</option>";
        });
        select.innerHTML = html;
        select.value =
          previous && treasuryParties.some((party) => party.id === previous)
            ? previous
            : "";
      }
    } catch (_) {}
  }

  async function loadTreasuryOverviewOnly(): Promise<void> {
    try {
      const r = await deps.api("GET", "/api/treasury/overview");
      if (r.error) return;
      renderTreasuryOverview(r as TreasuryOverviewResponse);
    } catch (_) {}
  }

  async function loadTreasuryPolicy(): Promise<void> {
    try {
      const r = await deps.api("GET", "/api/treasury/policy");
      if (r.error) return;
      const policy = (r.policy || null) as TreasuryPolicy | null;
      renderTreasuryPolicy(policy);
      prefillTreasuryPolicyForm(policy);
    } catch (_) {}
  }

  async function loadTreasuryReceiveAddresses(): Promise<void> {
    try {
      const r = await deps.api("GET", "/api/treasury/receive-addresses");
      if (r.error) return;
      renderTreasuryReceiveAllocations(
        (r.allocations || []) as TreasuryReceiveAllocation[],
      );
    } catch (_) {}
  }

  async function loadTreasuryOverview(): Promise<void> {
    await Promise.all([
      loadTreasuryOverviewOnly(),
      loadTreasuryPolicy(),
      loadTreasuryParties(),
      loadTreasuryReceiveAddresses(),
    ]);
  }

  async function refreshTreasuryOverview(): Promise<void> {
    const r = await deps.api("GET", "/api/treasury/overview");
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    renderTreasuryOverview(r as TreasuryOverviewResponse);
    deps.toast("Treasury overview refreshed");
    void Promise.all([
      loadTreasuryPolicy(),
      loadTreasuryParties(),
      loadTreasuryReceiveAddresses(),
    ]);
  }

  function markInvalid(id: string, invalid: boolean): void {
    const el = document.getElementById(id);
    if (el) el.classList.toggle("input-invalid", invalid);
  }

  async function updateTreasuryPolicy(): Promise<void> {
    const maxStepText = textValue("treasuryPolicyMaxStepEth");
    const maxStepWeiHex = parseEthToWeiHex(maxStepText);
    const maxStepInvalid = Boolean(maxStepText) && maxStepWeiHex === null;
    markInvalid("treasuryPolicyMaxStepEth", maxStepInvalid);
    if (maxStepInvalid) {
      deps.toast(
        "Max per-step cap must be a decimal ETH amount with up to 18 decimals",
        "error",
      );
      return;
    }
    const maxPlanText = textValue("treasuryPolicyMaxPlanEth");
    const maxPlanWeiHex = parseEthToWeiHex(maxPlanText);
    const maxPlanInvalid = Boolean(maxPlanText) && maxPlanWeiHex === null;
    markInvalid("treasuryPolicyMaxPlanEth", maxPlanInvalid);
    if (maxPlanInvalid) {
      deps.toast(
        "Max per-plan cap must be a decimal ETH amount with up to 18 decimals",
        "error",
      );
      return;
    }
    const hotFloorText = textValue("treasuryPolicyHotFloorEth");
    const hotFloorWeiHex = hotFloorText ? parseEthToWeiHex(hotFloorText) : null;
    const hotFloorInvalid = Boolean(hotFloorText) && hotFloorWeiHex === null;
    markInvalid("treasuryPolicyHotFloorEth", hotFloorInvalid);
    if (hotFloorInvalid) {
      deps.toast(
        "Hot refill floor must be a decimal ETH amount with up to 18 decimals",
        "error",
      );
      return;
    }
    const hotTargetText = textValue("treasuryPolicyHotTargetEth");
    const hotTargetWeiHex = hotTargetText ? parseEthToWeiHex(hotTargetText) : null;
    const hotTargetInvalid = Boolean(hotTargetText) && hotTargetWeiHex === null;
    markInvalid("treasuryPolicyHotTargetEth", hotTargetInvalid);
    if (hotTargetInvalid) {
      deps.toast(
        "Hot refill target must be a decimal ETH amount with up to 18 decimals",
        "error",
      );
      return;
    }
    const freshnessText = textValue("treasuryPolicyFreshnessSecs");
    const freshnessSecs = freshnessText ? parseInt(freshnessText, 10) : null;
    const freshnessInvalid =
      Boolean(freshnessText) &&
      (freshnessSecs === null || Number.isNaN(freshnessSecs) || freshnessSecs <= 0);
    markInvalid("treasuryPolicyFreshnessSecs", freshnessInvalid);
    if (freshnessInvalid) {
      deps.toast("Simulation freshness must be a positive number of seconds", "error");
      return;
    }
    const body: TreasuryPolicyUpdateRequest = {
      enabled: Boolean(input("treasuryPolicyEnabled")?.checked),
      allowed_destinations: parseTreasuryDestinationLines(
        optionalTextValue("treasuryPolicyDestinations"),
      ),
      max_step_native_wei_hex: maxStepWeiHex,
      max_plan_native_wei_hex: maxPlanWeiHex,
      require_simulation: Boolean(input("treasuryPolicyRequireSim")?.checked),
      block_cross_party_linkage: Boolean(input("treasuryPolicyBlockLinkage")?.checked),
    };
    if (freshnessText) {
      body.simulation_freshness_secs = freshnessSecs;
    }
    if (hotFloorText) {
      body.hot_floor_wei_hex = hotFloorWeiHex;
    }
    if (hotTargetText) {
      body.hot_target_wei_hex = hotTargetWeiHex;
    }
    const saveButton = document.querySelector(
      '[data-action="updateTreasuryPolicy"]',
    );
    if (saveButton) saveButton.classList.add("btn-busy");
    try {
      const r = await deps.api("POST", "/api/treasury/policy/update", body);
      if (r.error) {
        deps.toast(r.error, "error");
        return;
      }
      const policy = (r.policy || null) as TreasuryPolicy | null;
      renderTreasuryPolicy(policy);
      prefillTreasuryPolicyForm(policy);
      deps.toast("Treasury policy saved");
      void loadTreasuryOverviewOnly();
    } finally {
      if (saveButton) saveButton.classList.remove("btn-busy");
    }
  }

  async function allocateTreasuryReceiveAddress(): Promise<void> {
    const walletProfile = textValue("treasuryReceiveProfile");
    const purpose = textValue("treasuryReceivePurpose");
    if (!walletProfile || !purpose) {
      deps.toast("Wallet profile and purpose are required", "error");
      return;
    }
    const label = optionalTextValue("treasuryReceiveLabel");
    const partyId =
      (document.getElementById("treasuryReceiveParty") as HTMLSelectElement | null)
        ?.value || "";
    const body: {
      wallet_profile: string;
      purpose: string;
      label?: string;
      counterparty_id?: string;
    } = {
      wallet_profile: walletProfile,
      purpose,
    };
    if (label) body.label = label;
    if (partyId) body.counterparty_id = partyId;
    const r = await deps.api("POST", "/api/treasury/receive-addresses/allocate", body);
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    clearFields(["treasuryReceiveProfile", "treasuryReceivePurpose", "treasuryReceiveLabel"]);
    const partySelect = document.getElementById(
      "treasuryReceiveParty",
    ) as HTMLSelectElement | null;
    if (partySelect) partySelect.value = "";
    const allocation = r.allocation as TreasuryReceiveAllocation | undefined;
    deps.toast(
      "Receive address allocated" + (allocation?.address ? ": " + allocation.address : ""),
    );
    void Promise.all([
      loadTreasuryParties(),
      loadTreasuryReceiveAddresses(),
      loadTreasuryOverviewOnly(),
    ]);
  }

  async function rotateTreasuryReceiveAddress(allocationId: string): Promise<void> {
    const r = await deps.api("POST", "/api/treasury/receive-addresses/rotate", {
      allocation_id: allocationId,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Receive address rotated");
    void Promise.all([
      loadTreasuryParties(),
      loadTreasuryReceiveAddresses(),
      loadTreasuryOverviewOnly(),
    ]);
  }

  async function createTreasuryParty(): Promise<void> {
    const name = textValue("treasuryPartyName");
    if (!name) {
      deps.toast("Party name is required", "error");
      return;
    }
    const note = optionalTextValue("treasuryPartyNote");
    const sweepDestination = optionalTextValue("treasuryPartySweepDestination");
    const body: { name: string; note?: string; sweep_destination_address?: string } = { name };
    if (note) body.note = note;
    if (sweepDestination) body.sweep_destination_address = sweepDestination;
    const r = await deps.api("POST", "/api/treasury/parties", body);
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    clearFields(["treasuryPartyName", "treasuryPartyNote", "treasuryPartySweepDestination"]);
    deps.toast("Counterparty added");
    await loadTreasuryParties();
    await loadTreasuryReceiveAddresses();
  }

  function partyUpdateBody(
    party: Counterparty,
    sweepDestination: string,
  ): { id: string; name: string; note?: string; sweep_destination_address: string } {
    const body: {
      id: string;
      name: string;
      note?: string;
      sweep_destination_address: string;
    } = {
      id: party.id,
      name: party.name,
      sweep_destination_address: sweepDestination,
    };
    if (party.note) body.note = party.note;
    return body;
  }

  function partySweepDestinationInput(controlEl: unknown): HTMLInputElement | null {
    if (controlEl instanceof HTMLInputElement) return controlEl;
    if (!(controlEl instanceof Element)) return null;
    return controlEl
      .closest("li")
      ?.querySelector<HTMLInputElement>("input[data-party-sweep-dest-input]") || null;
  }

  async function updateTreasuryPartySweepDest(
    partyId: string,
    controlEl?: unknown,
  ): Promise<void> {
    const party = treasuryParties.find((candidate) => candidate.id === partyId);
    if (!party) {
      deps.toast("Counterparty not found", "error");
      return;
    }
    const inputEl = partySweepDestinationInput(controlEl);
    if (!inputEl) {
      deps.toast("Sweep destination input unavailable", "error");
      return;
    }
    const sweepDestination = inputEl.value.trim();
    if (!sweepDestination) {
      deps.toast("Enter a sweep destination or use Clear", "error");
      return;
    }
    const r = await deps.api(
      "POST",
      "/api/treasury/parties/update",
      partyUpdateBody(party, sweepDestination),
    );
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Sweep destination saved");
    await Promise.all([loadTreasuryParties(), loadTreasuryReceiveAddresses()]);
  }

  async function clearTreasuryPartySweepDest(partyId: string): Promise<void> {
    const party = treasuryParties.find((candidate) => candidate.id === partyId);
    if (!party) {
      deps.toast("Counterparty not found", "error");
      return;
    }
    const r = await deps.api(
      "POST",
      "/api/treasury/parties/update",
      partyUpdateBody(party, ""),
    );
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Sweep destination cleared");
    await Promise.all([loadTreasuryParties(), loadTreasuryReceiveAddresses()]);
  }

  async function deleteTreasuryParty(partyId: string): Promise<void> {
    const r = await deps.api("POST", "/api/treasury/parties/delete", {
      id: partyId,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Counterparty deleted");
    await Promise.all([loadTreasuryParties(), loadTreasuryReceiveAddresses()]);
  }

  return {
    renderTreasuryOverview,
    renderTreasuryPolicy,
    renderTreasuryReceiveAllocations,
    renderTreasuryParties,
    loadTreasuryOverview,
    loadTreasuryParties,
    refreshTreasuryOverview,
    updateTreasuryPolicy,
    allocateTreasuryReceiveAddress,
    rotateTreasuryReceiveAddress,
    createTreasuryParty,
    updateTreasuryPartySweepDest,
    clearTreasuryPartySweepDest,
    deleteTreasuryParty,
  };
}
