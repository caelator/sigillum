export interface QueueJobView {
  id: string;
  state: string;
  attempts: number;
  updated_at_unix: number;
  last_error?: string | null;
}

export type QueueJobBadge = "queued" | "blocked" | "retrying" | "sent" | "failed" | "unknown";

export function queueJobBadge(job: QueueJobView): QueueJobBadge {
  switch (job.state) {
    case "queued":
      return "queued";
    case "blocked":
    case "deferred":
      return "blocked";
    case "retrying":
      return "retrying";
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
  return queueJobBadge(job) === "blocked" || queueJobBadge(job) === "failed";
}
