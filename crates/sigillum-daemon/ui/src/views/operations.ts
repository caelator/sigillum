import { confirmDangerDialog } from "../render/confirm";
import { setHiddenById } from "../render/dom";
import {
  clearFields,
  optionalNumberValue,
  optionalTextValue,
  renderEntityList,
  textValue,
} from "../render/forms";
import { esc, escAttr, formatTs, statusPill } from "../render/html";

export interface OperationsState {
  deposits: any[];
  queueJobs: any[];
}

export interface OperationsDeps {
  api: (method: string, path: string, body?: unknown) => Promise<any>;
  toast: (message: string, type?: string) => void;
  refresh: () => unknown;
  showResultBox: (id: string, html: string) => void;
  updateNextStepCard: () => void;
}

function input(id: string): HTMLInputElement {
  return document.getElementById(id) as HTMLInputElement;
}

function describeQueueJob(job: any): string {
  const kind = job.kind || "unknown";
  switch (kind) {
    case "eth_stealth_transfer":
      return (
        "native transfer · " +
        (job.wallet_profile || "-") +
        " · to " +
        (job.destination_address || job.to_address || "-")
      );
    case "eth_stealth_erc20_transfer":
      return (
        "erc20 transfer · " +
        (job.wallet_profile || "-") +
        " · token " +
        (job.token_address || "-")
      );
    case "eth_stealth_native_sweep":
      return (
        "native sweep · " +
        (job.wallet_profile || "-") +
        " · " +
        (job.destination_address || "wallet default")
      );
    case "eth_stealth_erc20_sweep":
      return (
        "erc20 sweep · " +
        (job.wallet_profile || "-") +
        " · token " +
        (job.token_address || "-")
      );
    case "plan_step_execution":
      return (
        "plan step · " +
        (job.action || "-") +
        " · plan " +
        (job.plan_id || "-") +
        " · step " +
        (job.step_id || "-") +
        " · from " +
        (job.source_address || "-") +
        (job.destination_address ? " · to " + job.destination_address : "") +
        ((job.prerequisite_job_ids || []).length
          ? " · dependsOnJobs=" + (job.prerequisite_job_ids || []).join(",")
          : "")
      );
    default:
      return kind;
  }
}

// W7.4: truthful post-broadcast receipt info — `sent` means "broadcast,
// awaiting confirmation"; `confirmed` and a receipt-discovered revert
// (`operator_action_required` with `receipt_status`) both carry gas used
// and the mined block number. Empty until a receipt has actually been
// observed.
function queueReceiptLine(job: any): string {
  return (
    "tx=" +
    esc(job.transaction_hash_hex || "-") +
    " · broadcast=" +
    esc(job.broadcast_transaction_hash_hex || "-") +
    " · receipt=" +
    esc(job.receipt_status || (job.state === "sent" ? "pending" : "-")) +
    " · confirmations=" +
    esc(String(job.confirmations ?? "-")) +
    " · block=" +
    esc(String(job.receipt_block_number ?? "-")) +
    " · gasUsed=" +
    esc(job.receipt_gas_used_hex || "-")
  );
}

function queueScheduleLine(job: any): string {
  if (job.next_attempt_after_unix) {
    return "nextAttempt=" + formatTs(job.next_attempt_after_unix);
  }
  if ((job.state || "").toLowerCase().includes("retry")) {
    return "nextAttempt=manual-or-immediate";
  }
  return "nextAttempt=-";
}

function queueJobCanProcess(job: any): boolean {
  const state = String(job.state || "");
  // W7.4: `sent` for a `plan_step_execution` job means "broadcast, awaiting
  // confirmation" — the operator can still trigger a manual receipt-poll
  // via Process. Every other kind keeps `sent` as its pre-W7.4 terminal
  // meaning (broadcast = done, never re-driven).
  if (state === "sent" && job.kind === "plan_step_execution") {
    return true;
  }
  return !["operator_action_required", "sent", "confirmed", "failed", "failed_terminal"].includes(
    state,
  );
}

function failureBreakdownLine(summary: any): string {
  const failures = summary || {};
  return (
    " · failures_by_cause(provider_error=" +
    esc(String(failures.provider_error || 0)) +
    " · policy_block=" +
    esc(String(failures.policy_block || 0)) +
    " · insufficient_gas=" +
    esc(String(failures.insufficient_gas || 0)) +
    " · validation=" +
    esc(String(failures.validation || 0)) +
    " · on_chain_revert=" +
    esc(String(failures.on_chain_revert || 0)) +
    " · broadcast_rejected=" +
    esc(String(failures.broadcast_rejected || 0)) +
    " · receipt_timeout=" +
    esc(String(failures.receipt_timeout || 0)) +
    " · unknown=" +
    esc(String(failures.unknown || 0)) +
    ")"
  );
}

function depositObservedLine(deposit: any): string {
  const observedAmount = deposit.observed_amount_hex || "-";
  const nativeBalance = deposit.observed_native_balance_wei_hex || "-";
  return (
    "expected=" +
    esc(deposit.expected_amount_hex || "-") +
    " · observed=" +
    esc(observedAmount) +
    " · native=" +
    esc(nativeBalance)
  );
}

export function createOperationsActions(deps: OperationsDeps) {
  let lastDeposits: any[] = [];
  let lastQueueJobs: any[] = [];

  function renderDeposits(deposits: any[]): void {
    renderEntityList(
      "depositList",
      deposits,
      "No tracked deposits yet. Create a native or ERC-20 deposit above to start monitoring incoming funds and queue follow-up work.",
      (deposit) => {
        const queueInfo = deposit.queue_job_id
          ? "job=" + deposit.queue_job_id + " · state=" + (deposit.queue_job_state || "-")
          : "job=-";
        const announcement = deposit.announcement;
        const announcementMeta = announcement
          ? "<br>announcer=" +
            esc(announcement.announcer_address) +
            "<br>metadata=" +
            esc(announcement.metadata_hex) +
            " · calldata=" +
            esc(announcement.calldata_hex)
          : "";
        const announcementActions = announcement
          ? '<button class="btn-ghost" data-action="copyText" data-arg0="' +
            escAttr(announcement.announcer_address) +
            '" data-arg1="ERC-5564 announcer">Copy Announcer</button>' +
            '<button class="btn-ghost" data-action="copyText" data-arg0="' +
            escAttr(announcement.calldata_hex) +
            '" data-arg1="ERC-5564 calldata">Copy Announce Data</button>'
          : "";
        return (
          '<li><div class="entity-main">' +
          '<div class="entity-title">' +
          esc(deposit.id) +
          " " +
          statusPill(deposit.status) +
          "</div>" +
          '<div class="entity-meta">' +
          "walletProfile=" +
          esc(deposit.wallet_profile) +
          " · asset=" +
          esc(deposit.asset_kind) +
          " · short=" +
          esc(deposit.short_name) +
          "<br>" +
          "stealth=" +
          esc(deposit.stealth_address) +
          "<br>" +
          "ephemeral=" +
          esc(deposit.ephemeral_public_key_hex) +
          " · viewTag=" +
          esc(deposit.view_tag_hex) +
          announcementMeta +
          "<br>" +
          depositObservedLine(deposit) +
          "<br>" +
          "token=" +
          esc(deposit.token_address || "-") +
          " · autoQueue=" +
          esc(String(deposit.auto_queue_sweep)) +
          " · " +
          esc(queueInfo) +
          "<br>" +
          "created=" +
          esc(formatTs(deposit.created_at_unix)) +
          " · checked=" +
          esc(formatTs(deposit.last_checked_at_unix)) +
          " · updated=" +
          esc(formatTs(deposit.updated_at_unix)) +
          (deposit.note ? "<br>note=" + esc(deposit.note) : "") +
          "</div></div>" +
          '<div class="entity-actions">' +
          '<button class="btn-ghost" data-action="copyText" data-arg0="' +
          escAttr(deposit.stealth_address) +
          '" data-arg1="Deposit address">Copy Address</button>' +
          announcementActions +
          '<button class="btn-ghost" data-action="refreshSingleDeposit" data-arg0="' +
          escAttr(deposit.id) +
          '">Refresh</button>' +
          '<button class="btn-success" data-action="enqueueDepositSweep" data-arg0="' +
          escAttr(deposit.id) +
          '">Queue Sweep</button>' +
          '<button class="btn-danger" data-action="deleteDeposit" data-arg0="' +
          escAttr(deposit.id) +
          '">Delete</button>' +
          "</div></li>"
        );
      },
    );
  }

  async function loadDepositRegistry(): Promise<void> {
    try {
      const r = await deps.api("GET", "/api/deposits/eth-stealth");
      if (r.error) return;
      lastDeposits = r.deposits || [];
      renderDeposits(lastDeposits);
    } catch (_) {}
  }

  async function createNativeDeposit(): Promise<void> {
    const walletProfile = textValue("depositNativeWalletProfile");
    if (!walletProfile) {
      deps.toast("Select a wallet profile first", "error");
      return;
    }
    const r = await deps.api("POST", "/api/deposits/eth-stealth/create-native", {
      wallet_profile: walletProfile,
      expected_value_wei_hex: optionalTextValue("depositNativeExpected"),
      auto_queue_sweep: input("depositNativeAutoQueue").checked,
      sweep_destination_address: optionalTextValue("depositNativeDestination"),
      min_sweep_value_wei_hex: optionalTextValue("depositNativeMinSweep"),
      note: optionalTextValue("depositNativeNote"),
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    clearFields([
      "depositNativeExpected",
      "depositNativeMinSweep",
      "depositNativeDestination",
      "depositNativeNote",
    ]);
    deps.toast("Native deposit created");
    deps.refresh();
  }

  async function createErc20Deposit(): Promise<void> {
    const walletProfile = textValue("depositErc20WalletProfile");
    const tokenAddress = textValue("depositErc20TokenAddress");
    if (!walletProfile || !tokenAddress) {
      deps.toast("Wallet profile and token address are required", "error");
      return;
    }
    const r = await deps.api("POST", "/api/deposits/eth-stealth/create-erc20", {
      wallet_profile: walletProfile,
      token_address: tokenAddress,
      expected_amount_hex: optionalTextValue("depositErc20Expected"),
      auto_queue_sweep: input("depositErc20AutoQueue").checked,
      sweep_destination_address: optionalTextValue("depositErc20Destination"),
      min_sweep_amount_hex: optionalTextValue("depositErc20MinSweep"),
      note: optionalTextValue("depositErc20Note"),
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    clearFields([
      "depositErc20TokenAddress",
      "depositErc20Expected",
      "depositErc20MinSweep",
      "depositErc20Destination",
      "depositErc20Note",
    ]);
    deps.toast("ERC-20 deposit created");
    deps.refresh();
  }

  async function scanEthStealthAnnouncements(): Promise<void> {
    const walletProfile = textValue("depositScanWalletProfile");
    const fromBlock = textValue("depositScanFromBlock");
    if (!walletProfile || !fromBlock) {
      deps.toast("Wallet profile and from block are required", "error");
      return;
    }
    const r = await deps.api("POST", "/api/deposits/eth-stealth/scan-announcements", {
      wallet_profile: walletProfile,
      from_block: fromBlock,
      to_block: optionalTextValue("depositScanToBlock"),
      token_address: optionalTextValue("depositScanTokenAddress"),
      limit: optionalNumberValue("depositScanLimit"),
      auto_queue_sweep: input("depositScanAutoQueue").checked,
      sweep_destination_address: optionalTextValue("depositScanDestination"),
      min_sweep_amount_hex: optionalTextValue("depositScanMinSweep"),
      note: optionalTextValue("depositScanNote"),
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.showResultBox(
      "depositRefreshResult",
      "scanned=" +
        esc(String(r.scanned || 0)) +
        " · matched=" +
        esc(String(r.matched || 0)) +
        " · created=" +
        esc(String(r.created || 0)) +
        " · existing=" +
        esc(String(r.existing || 0)),
    );
    clearFields([
      "depositScanToBlock",
      "depositScanTokenAddress",
      "depositScanMinSweep",
      "depositScanDestination",
      "depositScanNote",
    ]);
    await loadDepositRegistry();
    deps.updateNextStepCard();
    deps.toast("Announcement scan completed");
  }

  async function refreshDepositRegistry(): Promise<void> {
    const r = await deps.api("POST", "/api/deposits/eth-stealth/refresh", {
      id: null,
      limit: optionalNumberValue("depositRefreshLimit"),
      auto_enqueue: input("depositRefreshAutoEnqueue").checked,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.showResultBox(
      "depositRefreshResult",
      "processed=" +
        esc(String(r.processed || 0)) +
        " · detected=" +
        esc(String(r.detected || 0)) +
        " · queued=" +
        esc(String(r.queued || 0)),
    );
    lastDeposits = r.deposits || [];
    renderDeposits(lastDeposits);
    deps.updateNextStepCard();
    deps.toast("Deposits refreshed");
    await loadQueueJobs();
  }

  async function refreshSingleDeposit(id: string): Promise<void> {
    const r = await deps.api("POST", "/api/deposits/eth-stealth/refresh", {
      id,
      limit: 1,
      auto_enqueue: input("depositRefreshAutoEnqueue").checked,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.showResultBox(
      "depositRefreshResult",
      "processed=" +
        esc(String(r.processed || 0)) +
        " · detected=" +
        esc(String(r.detected || 0)) +
        " · queued=" +
        esc(String(r.queued || 0)) +
        " · target=" +
        esc(id),
    );
    lastDeposits = r.deposits || [];
    renderDeposits(lastDeposits);
    deps.updateNextStepCard();
    void loadQueueJobs();
  }

  async function enqueueDepositSweep(id: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Queue deposit sweep",
      body:
        'Enqueue a sweep job for deposit "' +
        id +
        '"? When the queue processes it and the job passes its checks, the sweep is signed and broadcast on-chain.',
      actionLabel: "Queue sweep",
    });
    if (!confirmed) return;
    const r = await deps.api("POST", "/api/deposits/eth-stealth/enqueue-sweep", { id });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.showResultBox(
      "depositRefreshResult",
      "queued sweep for deposit " + esc(id) + " · job=" + esc(r.job?.id || "-"),
    );
    deps.toast("Deposit sweep queued");
    deps.refresh();
  }

  async function deleteDeposit(id: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Delete deposit",
      body:
        'Delete deposit "' +
        id +
        '"? The deposit record is removed from this daemon; funds already on-chain are not moved.',
      actionLabel: "Delete",
    });
    if (!confirmed) return;
    const r = await deps.api("POST", "/api/deposits/eth-stealth/delete", { id });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Deposit deleted");
    deps.refresh();
  }

  function renderQueueJobs(jobs: any[]): void {
    renderEntityList(
      "queueList",
      jobs,
      "Queue is empty. Once deposits enqueue sweeps or you create manual work, jobs will appear here for review and processing.",
      (job) => {
        const canProcess = queueJobCanProcess(job);
        const processButton =
          '<button class="btn-primary" data-action="processQueueJob" data-arg0="' +
          escAttr(job.id) +
          '"' +
          (canProcess ? "" : " disabled") +
          ">Process</button>";
        return (
          '<li><div class="entity-main">' +
          '<div class="entity-title">' +
          esc(job.id) +
          " " +
          statusPill(job.state) +
          "</div>" +
          '<div class="entity-meta">' +
          "kind=" +
          esc(job.kind || "-") +
          " · attempts=" +
          esc(String(job.attempts || 0)) +
          "<br>" +
          esc(describeQueueJob(job)) +
          "<br>" +
          "created=" +
          esc(formatTs(job.created_at_unix)) +
          " · updated=" +
          esc(formatTs(job.updated_at_unix)) +
          " · " +
          esc(queueScheduleLine(job)) +
          "<br>" +
          queueReceiptLine(job) +
          (job.last_error ? "<br>lastError=" + esc(job.last_error) : "") +
          "</div></div>" +
          '<div class="entity-actions">' +
          processButton +
          "</div></li>"
        );
      },
    );
  }

  async function loadQueueJobs(): Promise<void> {
    try {
      const r = await deps.api("GET", "/api/queue/jobs");
      if (r.error) return;
      lastQueueJobs = r.jobs || [];
      renderQueueJobs(lastQueueJobs);
    } catch (_) {}
    try {
      const policyResp = await deps.api("GET", "/api/treasury/policy");
      const paused = Boolean(policyResp?.policy?.execution_paused);
      setHiddenById("queuePausedBanner", !paused);
      setHiddenById("queuePauseBtn", paused);
      setHiddenById("queueResumeBtn", !paused);
    } catch (_) {}
  }

  async function pauseQueueExecution(): Promise<void> {
    const r = await deps.api("POST", "/api/queue/pause");
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Queue execution paused");
    void loadQueueJobs();
  }

  async function resumeQueueExecution(): Promise<void> {
    const r = await deps.api("POST", "/api/queue/resume");
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Queue execution resumed");
    void loadQueueJobs();
  }

  async function processQueueBatch(): Promise<void> {
    const limit = optionalNumberValue("queueProcessLimit");
    const confirmed = await confirmDangerDialog({
      title: "Process queue",
      body:
        (limit
          ? "Process up to " + String(limit) + " queued jobs now?"
          : "Process queued jobs now?") +
        " Jobs that pass their checks will be signed and broadcast on-chain.",
      actionLabel: "Process now",
    });
    if (!confirmed) return;
    const r = await deps.api("POST", "/api/queue/process", {
      id: null,
      limit,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.showResultBox(
      "queueProcessResult",
      "processed=" +
        esc(String(r.processed || 0)) +
        " · succeeded=" +
        esc(String(r.succeeded || 0)) +
        " · blocked=" +
        esc(String(r.blocked || 0)) +
        " · retrying=" +
        esc(String(r.retrying || 0)) +
        " · operator_action_required=" +
        esc(String(r.operator_action_required || 0)) +
        " · failed=" +
        esc(String(r.failed || 0)) +
        failureBreakdownLine(r.failures_by_cause) +
        (r.paused_reason ? " · paused: " + esc(String(r.paused_reason)) : ""),
    );
    lastQueueJobs = r.jobs || [];
    renderQueueJobs(lastQueueJobs);
    deps.updateNextStepCard();
    void loadDepositRegistry();
    deps.toast("Queue processed");
  }

  async function processQueueJob(id: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Process queued job",
      body:
        'Process queued job "' +
        id +
        '" now? If it passes its checks it will be signed and broadcast on-chain.',
      actionLabel: "Process now",
    });
    if (!confirmed) return;
    const r = await deps.api("POST", "/api/queue/process", { id, limit: 1 });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.showResultBox(
      "queueProcessResult",
      "processed=" +
        esc(String(r.processed || 0)) +
        " · succeeded=" +
        esc(String(r.succeeded || 0)) +
        " · blocked=" +
        esc(String(r.blocked || 0)) +
        " · retrying=" +
        esc(String(r.retrying || 0)) +
        " · operator_action_required=" +
        esc(String(r.operator_action_required || 0)) +
        " · failed=" +
        esc(String(r.failed || 0)) +
        failureBreakdownLine(r.failures_by_cause) +
        " · target=" +
        esc(id),
    );
    lastQueueJobs = r.jobs || [];
    renderQueueJobs(lastQueueJobs);
    deps.updateNextStepCard();
    void loadDepositRegistry();
  }

  async function runMaintenanceCycle(): Promise<void> {
    const r = await deps.api("POST", "/api/maintenance/run", {
      deposit_refresh_limit: optionalNumberValue("maintenanceDepositLimit"),
      queue_process_limit: optionalNumberValue("maintenanceQueueLimit"),
      auto_enqueue: input("maintenanceAutoEnqueue").checked,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.showResultBox(
      "maintenanceResult",
      "refreshed=" +
        esc(String(r.refreshed || 0)) +
        " · detected=" +
        esc(String(r.detected || 0)) +
        " · queued=" +
        esc(String(r.queued || 0)) +
        " · processed=" +
        esc(String(r.processed || 0)) +
        " · succeeded=" +
        esc(String(r.succeeded || 0)) +
        " · blocked=" +
        esc(String(r.blocked || 0)) +
        " · retrying=" +
        esc(String(r.retrying || 0)) +
        " · operator_action_required=" +
        esc(String(r.operator_action_required || 0)) +
        " · failed=" +
        esc(String(r.failed || 0)) +
        failureBreakdownLine(r.failures_by_cause) +
        (r.treasury_automation
          ? " · automationGenerated=" +
            esc(String(r.treasury_automation.generated_steps || 0)) +
            " · automationEnqueued=" +
            esc(String(r.treasury_automation.enqueued_steps || 0)) +
            " · automationSkipped=" +
            esc(String(r.treasury_automation.skipped_steps || 0))
          : ""),
    );
    lastDeposits = r.deposits || [];
    lastQueueJobs = r.jobs || [];
    renderDeposits(lastDeposits);
    renderQueueJobs(lastQueueJobs);
    deps.updateNextStepCard();
    deps.toast("Maintenance cycle complete");
  }

  return {
    getState: (): OperationsState => ({
      deposits: lastDeposits,
      queueJobs: lastQueueJobs,
    }),
    renderDeposits,
    renderQueueJobs,
    loadDepositRegistry,
    createNativeDeposit,
    createErc20Deposit,
    scanEthStealthAnnouncements,
    refreshDepositRegistry,
    refreshSingleDeposit,
    enqueueDepositSweep,
    deleteDeposit,
    loadQueueJobs,
    pauseQueueExecution,
    resumeQueueExecution,
    processQueueBatch,
    processQueueJob,
    runMaintenanceCycle,
  };
}
