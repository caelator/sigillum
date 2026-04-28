import type {
  ChainProfile,
  ConsolidationPlanSummary,
  RiskFinding,
  WalletDiscoveryJob,
} from "../contracts";

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
