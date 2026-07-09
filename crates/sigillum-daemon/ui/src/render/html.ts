export function esc(value: unknown): string {
  const node = document.createElement("div");
  node.textContent = String(value);
  return node.innerHTML;
}

export function escAttr(value: unknown): string {
  return esc(value).replace(/'/g, "&#39;");
}

export function formatTs(unix: number | null | undefined): string {
  if (!unix) return "-";
  return new Date(unix * 1000).toLocaleString();
}

export function pillClass(status: unknown): string {
  const value = String(status || "").toLowerCase();
  if (
    value.includes("fail") ||
    value.includes("error") ||
    value.includes("critical") ||
    value.includes("block") ||
    value.includes("poison")
  ) {
    return "pill-danger";
  }
  if (
    value.includes("ok") ||
    value.includes("pass") ||
    value.includes("success") ||
    value.includes("sent") ||
    value.includes("confirm") ||
    value.includes("broadcast") ||
    value.includes("enabled") ||
    value.includes("active") ||
    value.includes("ready") ||
    value.includes("approved") ||
    value.includes("completed") ||
    value.includes("funded") ||
    value.includes("trusted") ||
    value.includes("unlocked") ||
    value.includes("executable")
  ) {
    return "pill-good";
  }
  if (
    value.includes("warn") ||
    value.includes("queue") ||
    value.includes("detected") ||
    value.includes("processing") ||
    value.includes("running") ||
    value.includes("retry") ||
    value.includes("review") ||
    value.includes("required") ||
    value.includes("pending") ||
    value.includes("unconfigured") ||
    value.includes("high") ||
    value.includes("medium") ||
    value.includes("dormant") ||
    value.includes("exposure")
  ) {
    return "pill-warn";
  }
  if (value.includes("retired") || value.includes("low")) {
    return "pill-info";
  }
  return "pill-neutral";
}

export function statusPill(status: unknown): string {
  const label = String(status || "unknown").replace(/_/g, " ");
  return '<span class="pill ' + pillClass(status) + '">' + esc(label) + "</span>";
}

export function statBox(value: unknown, label: unknown): string {
  return (
    '<div class="stat"><div class="value">' +
    esc(String(value)) +
    '</div><div class="label">' +
    esc(label) +
    "</div></div>"
  );
}
