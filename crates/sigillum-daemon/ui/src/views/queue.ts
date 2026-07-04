export interface QueueJobView {
  id: string;
  state: string;
  attempts: number;
  updated_at_unix: number;
  last_error?: string | null;
}

export type QueueJobBadge =
  | "queued"
  | "blocked"
  | "retrying"
  | "sent"
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
