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
  if (value.includes("fail") || value.includes("error")) return "pill-danger";
  if (
    value.includes("ok") ||
    value.includes("success") ||
    value.includes("sent") ||
    value.includes("broadcast")
  ) {
    return "pill-good";
  }
  if (
    value.includes("queue") ||
    value.includes("funded") ||
    value.includes("detected") ||
    value.includes("processing") ||
    value.includes("block") ||
    value.includes("retry")
  ) {
    return "pill-warn";
  }
  return "pill-neutral";
}

export function statusPill(status: unknown): string {
  const label = String(status || "unknown").replace(/_/g, " ");
  return '<span class="pill ' + pillClass(status) + '">' + esc(label) + "</span>";
}

export function statBox(value: unknown, label: unknown): string {
  return (
    '<div class="stat"><div class="value" style="font-size:16px;">' +
    esc(String(value)) +
    '</div><div class="label">' +
    esc(label) +
    "</div></div>"
  );
}
