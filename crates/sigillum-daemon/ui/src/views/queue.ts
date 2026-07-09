export interface QueueJobView {
  id: string;
  state: string;
  attempts: number;
  updated_at_unix: number;
  last_error?: string | null;
  // W7.4: post-broadcast receipt-confirmation truth (see
  // service/queue/plan_steps/receipts.rs) — present once a broadcast (and,
  // for confirmations/reverts, a mined receipt) has been observed.
  transaction_hash_hex?: string | null;
  broadcast_transaction_hash_hex?: string | null;
  confirmations?: number | null;
  receipt_block_number?: number | null;
  receipt_gas_used_hex?: string | null;
  receipt_status?: string | null;
}

export type QueueJobBadge =
  | "queued"
  | "blocked"
  | "retrying"
  | "sent"
  | "confirmed"
  | "failed"
  | "operator_action_required"
  | "unknown";

export function queueJobBadge(job: QueueJobView): QueueJobBadge {
  switch (job.state) {
    case "queued":
      return "queued";
    case "blocked":
    case "deferred":
      return "blocked";
    case "retrying":
      return "retrying";
    case "operator_action_required":
      return "operator_action_required";
    case "sent":
      return "sent";
    case "confirmed":
      return "confirmed";
    case "failed":
    case "failed_terminal":
      return "failed";
    default:
      return "unknown";
  }
}

export function queueJobNeedsAttention(job: QueueJobView): boolean {
  const badge = queueJobBadge(job);
  return (
    badge === "blocked" ||
    badge === "failed" ||
    badge === "operator_action_required"
  );
}
