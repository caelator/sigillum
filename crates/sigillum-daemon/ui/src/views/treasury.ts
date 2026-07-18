import type {
  Counterparty,
  TreasuryAutomationStatus,
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
import { confirmDangerDialog } from "../render/confirm";
import { setTextById as setText } from "../render/dom";
import {
  clearFields,
  optionalTextValue,
  renderEntityList,
  textValue,
} from "../render/forms";
import { formatTokenAmount } from "../render/format";
import { esc, escAttr, formatTs, statBox, statusPill } from "../render/html";

const WEI_PER_ETH = 10n ** 18n;
const DEFAULT_HOT_REFILL_WEI_HEX = "0xde0b6b3a7640000";

// The BigInt formatting core lives in render/format.ts (shared with the
// inventory/operations views); these keep the historical "0" fallback
// treasury callers rely on.
export function formatWeiHexAsEth(weiHex: string): string {
  return formatTokenAmount(weiHex, 18) ?? "0";
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

const WEI_PER_GWEI = 10n ** 9n;

export function formatWeiHexAsGwei(weiHex: string): string {
  return formatTokenAmount(weiHex, 9) ?? "0";
}

export function parseGweiToWeiHex(value: string): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  const match = /^(\d+)(?:\.(\d+))?$/.exec(trimmed);
  if (!match) return null;
  const fractionDigits = match[2] || "";
  if (fractionDigits.length > 9) return null;
  const wei = BigInt(match[1]) * WEI_PER_GWEI + BigInt(fractionDigits.padEnd(9, "0"));
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

function capAsGwei(weiHex: string | null | undefined): string {
  return weiHex ? formatWeiHexAsGwei(weiHex) + " Gwei" : "-";
}

function nativeAmount(
  weiHex: string | null | undefined,
  symbol?: string | null,
): string {
  const amount = formatWeiHexAsEth(weiHex || "");
  return symbol ? amount + " " + symbol : amount;
}

function countLabel(
  count: number | null | undefined,
  singular: string,
  plural = singular + "s",
): string {
  const value = count || 0;
  return String(value) + " " + (value === 1 ? singular : plural);
}

function humanizePolicyViolation(value: string): string {
  const trimmed = value.trim();
  const match = /^([^:=]+)(?:[:=](.*))?$/.exec(trimmed);
  const code = (match?.[1] || trimmed).trim();
  const detail = (match?.[2] || "").trim();
  switch (code) {
    case "exceeds_policy_plan_cap":
      return "The plan exceeds the policy's native-value cap";
    case "destination_not_allowed":
      return detail
        ? "Destination " + detail + " is not on the policy allow-list"
        : "A destination is not on the policy allow-list";
    case "cross_party_linkage":
      return "The plan would link different payers through a shared route";
    default: {
      const words = code.replace(/[_-]+/g, " ").trim();
      const label = words
        ? words.charAt(0).toUpperCase() + words.slice(1)
        : "Policy review required";
      return detail ? label + ": " + detail : label;
    }
  }
}

function shortAddress(value: string): string {
  const trimmed = value.trim();
  if (trimmed.startsWith("0x") && trimmed.length > 14) {
    return trimmed.slice(0, 6) + "..." + trimmed.slice(-4);
  }
  return trimmed.length > 24 ? trimmed.slice(0, 12) + "..." + trimmed.slice(-6) : trimmed;
}

function joinEnglishList(items: string[]): string {
  if (items.length <= 1) return items.join("");
  if (items.length === 2) return items[0] + " and " + items[1];
  return items.slice(0, -1).join(", ") + ", and " + items[items.length - 1];
}

/// Plain-English description of what the current policy permits, in 2-4
/// short sentences. The raw key=value state stays available behind the
/// "Technical state" details in renderTreasuryPolicy.
export function treasuryPolicySummary(policy: TreasuryPolicy): string[] {
  const sentences: string[] = [];
  if (!policy.enabled) {
    sentences.push(
      "The policy is disabled, so nothing may execute — no plan steps and no stealth deposit sweeps.",
    );
  } else if (!policy.allow_plan_execution) {
    sentences.push(
      "Plan execution is switched off, so no plan step or stealth deposit sweep may execute yet.",
    );
  } else {
    const allowed: string[] = [];
    if (policy.allow_sweep_execution) allowed.push("sweeps");
    if (policy.allow_revoke_execution) allowed.push("revokes");
    if (policy.allow_exit_execution) allowed.push("DeFi exits");
    if (policy.allow_claim_execution) allowed.push("claims");
    if (allowed.length) {
      sentences.push("Plans may execute " + joinEnglishList(allowed) + ".");
    }
    const blocked: string[] = [];
    if (!policy.allow_claim_execution) blocked.push("Claims");
    if (!policy.allow_exit_execution) blocked.push("DeFi exits");
    if (!policy.allow_sweep_execution) blocked.push("Sweeps");
    if (!policy.allow_revoke_execution) blocked.push("Revokes");
    if (blocked.length) {
      sentences.push(joinEnglishList(blocked) + " are blocked.");
    }
    // Plan task 2.5: the sweep gate covers the stealth families too — no
    // "stealth bypasses the gates" carve-out may be implied here.
    sentences.push(
      policy.allow_sweep_execution
        ? "The sweep gate also covers stealth deposit sweeps and transfers."
        : "Stealth deposit sweeps and transfers stay blocked until the sweep gate is on.",
    );
  }
  sentences.push(
    policy.block_cross_party_linkage
      ? "Cross-party linkage blocking is on; destinations are limited to the allow-list below."
      : "Cross-party linkage blocking is off — plans may route different payers to a shared destination.",
  );
  const operational: string[] = [];
  operational.push(
    policy.allow_gas_topups
      ? "Sponsor gas top-ups (plan and stealth) are allowed"
      : "Sponsor gas top-ups (plan and stealth) are off",
  );
  if (policy.execution_paused) {
    operational.push("queue execution is currently paused");
  }
  sentences.push(operational.join("; ") + ".");
  return sentences;
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
        "Chain " +
        esc(String(chain.chain_id)) +
        " · " +
        esc(chain.native_symbol || "-") +
        " " +
        statusPill((chain.funded_address_count || 0) > 0 ? "funded" : "empty") +
        "</div>" +
        '<div class="entity-meta">' +
        esc(String(chain.funded_address_count || 0)) +
        " of " +
        esc(String(chain.address_count || 0)) +
        " addresses funded · " +
        esc(nativeAmount(chain.native_total_wei_hex, chain.native_symbol)) +
        " native" +
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
        "Chain " +
        esc(String(group.chain_id)) +
        " · " +
        esc(String(group.funded_address_count || 0)) +
        " of " +
        esc(String(group.address_count || 0)) +
        " addresses funded · " +
        esc(nativeAmount(group.native_total_wei_hex, symbolByChain.get(group.chain_id))) +
        " native · " +
        esc(countLabel(group.signer_address_count, "signer address")) +
        " · " +
        esc(countLabel(group.watch_only_address_count, "watch-only address")) +
        "<br>" +
        esc(countLabel(group.erc20_holding_count, "ERC-20 holding")) +
        " · " +
        esc(countLabel(group.nft_holding_count, "NFT holding")) +
        " · " +
        esc(countLabel(group.defi_holding_count, "DeFi holding")) +
        " · " +
        esc(countLabel(group.claimable_holding_count, "claimable holding")) +
        " · " +
        esc(countLabel(group.approval_exposure_count, "approval exposure")) +
        " · " +
        esc(countLabel(group.dormant_candidate_count, "dormant candidate")) +
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
        "Hot wallet: " +
        esc(route.hot_address || "Not configured") +
        (route.hot_native_balance_wei_hex
          ? " · " +
            esc(formatWeiHexAsEth(route.hot_native_balance_wei_hex)) +
            " native units"
          : "") +
        "<br>" +
        "Treasury: " +
        esc(route.treasury_address || "Not configured") +
        (route.treasury_native_balance_wei_hex
          ? " · " +
            esc(formatWeiHexAsEth(route.treasury_native_balance_wei_hex)) +
            " native units"
          : "") +
        "<br>" +
        "Default destination: " +
        esc(route.default_destination_address || "Not configured") +
        "</div></div></li>",
    );
  }

  function renderTreasuryRiskAndPlans(
    risk: TreasuryRiskSummary,
    plans: TreasuryPlanSummary,
    automation?: TreasuryAutomationStatus,
  ): void {
    type RiskPlanRow =
      | { kind: "risk"; risk: TreasuryRiskSummary }
      | { kind: "plans"; plans: TreasuryPlanSummary }
      | { kind: "automation"; automation: TreasuryAutomationStatus };
    const rows: RiskPlanRow[] = [];
    if (risk) rows.push({ kind: "risk", risk });
    if (plans) rows.push({ kind: "plans", plans });
    if (automation) rows.push({ kind: "automation", automation });
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
            esc(countLabel(row.risk.critical_findings, "critical finding")) +
            " · " +
            esc(countLabel(row.risk.high_findings, "high finding")) +
            " · " +
            esc(countLabel(row.risk.medium_findings, "medium finding")) +
            " · " +
            esc(countLabel(row.risk.low_findings, "low finding")) +
            " · " +
            esc(countLabel(row.risk.total_findings, "total finding")) +
            "</div></div></li>"
          );
        }
        if (row.kind === "automation") {
          return (
            '<li><div class="entity-main">' +
            '<div class="entity-title">Treasury automation ' +
            statusPill(row.automation.enabled ? "enabled" : "off") +
            "</div>" +
            '<div class="entity-meta">' +
            "Overflow threshold: " +
            esc(
              row.automation.hot_overflow_wei_hex
                ? capAsEth(row.automation.hot_overflow_wei_hex)
                : "Not configured",
            ) +
            " · " +
            esc(countLabel(row.automation.generated_steps, "step generated")) +
            " · " +
            esc(countLabel(row.automation.enqueued_steps, "step enqueued")) +
            (row.automation.enabled
              ? "<br>auto-enqueue still requires passed simulation + execution gates"
              : "") +
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
          esc(countLabel(row.plans.total_plans, "plan")) +
          " · Latest plan: " +
          esc(row.plans.latest_plan_id || "-") +
          "<br>" +
          esc(countLabel(row.plans.latest_review_required_steps, "step needing review")) +
          " · " +
          esc(countLabel(row.plans.latest_approved_steps, "approved step")) +
          " · " +
          esc(countLabel(row.plans.latest_executable_steps, "executable step")) +
          " · " +
          esc(countLabel(row.plans.latest_blocked_steps, "blocked step")) +
          (policyViolations.length
            ? " · Policy review: " +
              esc(policyViolations.map(humanizePolicyViolation).join("; "))
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
    renderTreasuryRiskAndPlans(overview.risk, overview.plans, overview.automation);
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
          '<p class="policy-summary">' +
          esc(treasuryPolicySummary(current).join(" ")) +
          "</p>" +
          '<div class="entity-meta">' +
          "destinations=" +
          esc(destinations || "-") +
          "</div>" +
          '<details class="policy-details">' +
          "<summary>Technical state</summary>" +
          '<div class="entity-meta">' +
          "maxStep=" +
          esc(capAsEth(current.max_step_native_wei_hex)) +
          " · maxPlan=" +
          esc(capAsEth(current.max_plan_native_wei_hex)) +
          " · hotFloor=" +
          esc(capAsEth(current.hot_floor_wei_hex ?? DEFAULT_HOT_REFILL_WEI_HEX)) +
          " · hotTarget=" +
          esc(capAsEth(current.hot_target_wei_hex ?? DEFAULT_HOT_REFILL_WEI_HEX)) +
          " · hotOverflow=" +
          esc(capAsEth(current.hot_overflow_wei_hex)) +
          " · requireSimulation=" +
          esc(String(current.require_simulation)) +
          " · blockCrossPartyLinkage=" +
          esc(String(Boolean(current.block_cross_party_linkage))) +
          " · allowClaimExecution=" +
          esc(String(Boolean(current.allow_claim_execution))) +
          " · allowGasTopups=" +
          esc(String(Boolean(current.allow_gas_topups))) +
          " · allowTreasuryAutomation=" +
          esc(String(Boolean(current.allow_treasury_automation))) +
          " · maxGasTopup=" +
          esc(capAsEth(current.max_gas_topup_wei_hex)) +
          " · allowPlanExecution=" +
          esc(String(Boolean(current.allow_plan_execution))) +
          " · allowSweepExecution=" +
          esc(String(Boolean(current.allow_sweep_execution))) +
          " · allowRevokeExecution=" +
          esc(String(Boolean(current.allow_revoke_execution))) +
          " · allowExitExecution=" +
          esc(String(Boolean(current.allow_exit_execution))) +
          " · maxFeePerGasCap=" +
          esc(capAsGwei(current.max_fee_per_gas_cap_hex)) +
          " · paused=" +
          esc(String(Boolean(current.execution_paused))) +
          " · simulationFreshnessSecs=" +
          esc(String(current.simulation_freshness_secs ?? 900)) +
          " · updated=" +
          esc(formatTs(current.updated_at_unix)) +
          "</div></details></li>"
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
            policy.hot_overflow_wei_hex || null,
            policy.require_simulation,
            policy.block_cross_party_linkage,
            policy.allow_claim_execution,
            policy.allow_gas_topups,
            policy.allow_treasury_automation,
            policy.max_gas_topup_wei_hex || null,
            policy.allow_plan_execution,
            policy.allow_sweep_execution,
            policy.allow_revoke_execution,
            policy.allow_exit_execution,
            policy.max_fee_per_gas_cap_hex || null,
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
      // Default-on (plan task 3.5): with no saved policy the checkbox shows
      // the daemon's default posture — protection ON unless explicitly
      // turned off.
      blockLinkageEl.checked = policy ? Boolean(policy.block_cross_party_linkage) : true;
    }
    const allowClaimExecEl = input("treasuryPolicyAllowClaimExec");
    if (allowClaimExecEl) {
      allowClaimExecEl.checked = policy ? Boolean(policy.allow_claim_execution) : false;
    }
    const allowGasTopupsEl = input("treasuryPolicyAllowGasTopups");
    if (allowGasTopupsEl) {
      allowGasTopupsEl.checked = policy ? Boolean(policy.allow_gas_topups) : false;
    }
    const allowTreasuryAutomationEl = input("treasuryPolicyAllowTreasuryAutomation");
    if (allowTreasuryAutomationEl) {
      allowTreasuryAutomationEl.checked = policy
        ? Boolean(policy.allow_treasury_automation)
        : false;
    }
    const allowPlanExecEl = input("treasuryPolicyAllowPlanExec");
    if (allowPlanExecEl) {
      allowPlanExecEl.checked = policy ? Boolean(policy.allow_plan_execution) : false;
    }
    const allowSweepExecEl = input("treasuryPolicyAllowSweepExec");
    if (allowSweepExecEl) {
      allowSweepExecEl.checked = policy ? Boolean(policy.allow_sweep_execution) : false;
    }
    const allowRevokeExecEl = input("treasuryPolicyAllowRevokeExec");
    if (allowRevokeExecEl) {
      allowRevokeExecEl.checked = policy ? Boolean(policy.allow_revoke_execution) : false;
    }
    const allowExitExecEl = input("treasuryPolicyAllowExitExec");
    if (allowExitExecEl) {
      allowExitExecEl.checked = policy ? Boolean(policy.allow_exit_execution) : false;
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
    const maxGasTopupEl = input("treasuryPolicyMaxGasTopupEth");
    if (maxGasTopupEl) {
      maxGasTopupEl.value = policy?.max_gas_topup_wei_hex
        ? formatWeiHexAsEth(policy.max_gas_topup_wei_hex)
        : "";
    }
    const maxFeePerGasEl = input("treasuryPolicyMaxFeePerGasGwei");
    if (maxFeePerGasEl) {
      maxFeePerGasEl.value = policy?.max_fee_per_gas_cap_hex
        ? formatWeiHexAsGwei(policy.max_fee_per_gas_cap_hex)
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
    const hotOverflowEl = input("treasuryPolicyHotOverflowEth");
    if (hotOverflowEl) {
      hotOverflowEl.value = policy?.hot_overflow_wei_hex
        ? formatWeiHexAsEth(policy.hot_overflow_wei_hex)
        : "";
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
      (allocation) => {
        const oneTimeBadge = allocation.one_time
          ? ' <span class="pill">One-time</span>' +
            (allocation.lifecycle_state
              ? " " + statusPill(allocation.lifecycle_state)
              : "")
          : "";
        const oneTimeMeta = allocation.one_time
          ? "<br>" +
            "one-time sweep &rarr; " +
            esc(shortAddress(allocation.sweep_destination_address || "")) +
            " · threshold " +
            (allocation.min_sweep_amount_hex
              ? esc(formatWeiHexAsEth(allocation.min_sweep_amount_hex)) + " ETH"
              : "any funds") +
            (allocation.purge_after_sweep ? " · purges after sweep" : "") +
            (allocation.sweep_blocker
              ? " · " + esc(oneTimeBlockerCopy(allocation.sweep_blocker))
              : "")
          : "";
        return (
          '<li><div class="entity-main">' +
          '<div class="entity-title">' +
          esc(allocation.address) +
          " " +
          statusPill(allocation.status) +
          oneTimeBadge +
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
          oneTimeMeta +
          "</div></div>" +
          (allocation.status === "active"
            ? '<div class="entity-actions">' +
              '<button class="btn-ghost" data-action="rotateTreasuryReceiveAddress" data-arg0="' +
              escAttr(allocation.id) +
              '">Rotate</button>' +
              "</div>"
            : "") +
          "</li>"
        );
      },
    );
  }

  // Plain-English reason a watching one-time allocation has not swept yet.
  function oneTimeBlockerCopy(blocker: string): string {
    switch (blocker) {
      case "awaiting_balance":
        return "waiting for a balance check";
      case "below_threshold":
        return "below the sweep threshold";
      case "execution_gates":
        return "waiting on execution gates";
      case "destination_policy":
        return "destination blocked by treasury policy";
      case "step_cap":
        return "above the per-step cap";
      case "cross_party_linkage":
        return "blocked: shared destination would link parties";
      case "sweep_failed":
        return "last sweep failed";
      case "sweep_attention":
        return "sweep needs attention";
      default:
        return blocker.replace(/_/g, " ");
    }
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
    const maxGasTopupText = textValue("treasuryPolicyMaxGasTopupEth");
    const maxGasTopupWeiHex = parseEthToWeiHex(maxGasTopupText);
    const maxGasTopupInvalid =
      Boolean(maxGasTopupText) && maxGasTopupWeiHex === null;
    markInvalid("treasuryPolicyMaxGasTopupEth", maxGasTopupInvalid);
    if (maxGasTopupInvalid) {
      deps.toast(
        "Max gas top-up must be a decimal ETH amount with up to 18 decimals",
        "error",
      );
      return;
    }
    const maxFeePerGasText = textValue("treasuryPolicyMaxFeePerGasGwei");
    const maxFeePerGasWeiHex = parseGweiToWeiHex(maxFeePerGasText);
    const maxFeePerGasInvalid = Boolean(maxFeePerGasText) && maxFeePerGasWeiHex === null;
    markInvalid("treasuryPolicyMaxFeePerGasGwei", maxFeePerGasInvalid);
    if (maxFeePerGasInvalid) {
      deps.toast(
        "Max fee per gas must be a decimal gwei amount with up to 9 decimals",
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
    const hotOverflowText = textValue("treasuryPolicyHotOverflowEth");
    const hotOverflowWeiHex = hotOverflowText
      ? parseEthToWeiHex(hotOverflowText)
      : null;
    const hotOverflowInvalid =
      Boolean(hotOverflowText) && hotOverflowWeiHex === null;
    markInvalid("treasuryPolicyHotOverflowEth", hotOverflowInvalid);
    if (hotOverflowInvalid) {
      deps.toast(
        "Hot overflow threshold must be a decimal ETH amount with up to 18 decimals",
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
      allow_claim_execution: Boolean(input("treasuryPolicyAllowClaimExec")?.checked),
      allow_gas_topups: Boolean(input("treasuryPolicyAllowGasTopups")?.checked),
      allow_treasury_automation: Boolean(
        input("treasuryPolicyAllowTreasuryAutomation")?.checked,
      ),
      max_gas_topup_wei_hex: maxGasTopupWeiHex,
      allow_plan_execution: Boolean(input("treasuryPolicyAllowPlanExec")?.checked),
      allow_sweep_execution: Boolean(input("treasuryPolicyAllowSweepExec")?.checked),
      allow_revoke_execution: Boolean(input("treasuryPolicyAllowRevokeExec")?.checked),
      allow_exit_execution: Boolean(input("treasuryPolicyAllowExitExec")?.checked),
      max_fee_per_gas_cap_hex: maxFeePerGasWeiHex,
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
    if (hotOverflowText) {
      body.hot_overflow_wei_hex = hotOverflowWeiHex;
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
    const oneTime =
      (document.getElementById("treasuryReceiveOneTime") as HTMLInputElement | null)
        ?.checked === true;
    const body: {
      wallet_profile: string;
      purpose: string;
      label?: string;
      counterparty_id?: string;
      one_time?: boolean;
      sweep_destination_address?: string;
      min_sweep_amount_hex?: string;
      purge_after_sweep?: boolean;
    } = {
      wallet_profile: walletProfile,
      purpose,
    };
    if (label) body.label = label;
    if (partyId) body.counterparty_id = partyId;
    if (oneTime) {
      const destination = optionalTextValue("treasuryReceiveSweepDestination");
      if (!destination) {
        deps.toast("One-time addresses need a sweep destination", "error");
        return;
      }
      const thresholdEth = optionalTextValue("treasuryReceiveMinSweepEth");
      let thresholdWeiHex: string | null = null;
      if (thresholdEth) {
        thresholdWeiHex = parseEthToWeiHex(thresholdEth);
        if (!thresholdWeiHex) {
          deps.toast("Min sweep must be an ETH amount like 0.05", "error");
          return;
        }
      }
      body.one_time = true;
      body.sweep_destination_address = destination;
      if (thresholdWeiHex) body.min_sweep_amount_hex = thresholdWeiHex;
      body.purge_after_sweep =
        (document.getElementById("treasuryReceivePurgeAfterSweep") as HTMLInputElement | null)
          ?.checked === true;
    }
    const r = await deps.api("POST", "/api/treasury/receive-addresses/allocate", body);
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    clearFields([
      "treasuryReceiveProfile",
      "treasuryReceivePurpose",
      "treasuryReceiveLabel",
      "treasuryReceiveSweepDestination",
      "treasuryReceiveMinSweepEth",
    ]);
    const partySelect = document.getElementById(
      "treasuryReceiveParty",
    ) as HTMLSelectElement | null;
    if (partySelect) partySelect.value = "";
    const oneTimeBox = document.getElementById(
      "treasuryReceiveOneTime",
    ) as HTMLInputElement | null;
    if (oneTimeBox) oneTimeBox.checked = false;
    const purgeBox = document.getElementById(
      "treasuryReceivePurgeAfterSweep",
    ) as HTMLInputElement | null;
    if (purgeBox) purgeBox.checked = false;
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
    const confirmed = await confirmDangerDialog({
      title: "Rotate receive address",
      body:
        "Rotate this receive address? The current address is retired and a fresh address is derived for future payments. The old address stays valid on-chain, but it no longer shows as the active receive address here.",
      actionLabel: "Rotate address",
    });
    if (!confirmed) return;
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
    const party = treasuryParties.find((candidate) => candidate.id === partyId);
    const confirmed = await confirmDangerDialog({
      title: "Delete counterparty",
      body:
        'Delete counterparty "' +
        (party?.name || partyId) +
        '"? Existing receive allocations are kept, but their link to this party is removed.',
      actionLabel: "Delete",
    });
    if (!confirmed) return;
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
