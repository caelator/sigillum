import type { ChainProfile } from "../contracts";
import { esc } from "./html";

// Shared human-facing formatters. Views must NOT grow local copies of
// these; extend this module instead. `formatTimestamp` re-exports the
// single timestamp formatter from ./html so every view renders unix
// seconds the same way.
export { formatTs as formatTimestamp } from "./html";

/// BigInt-safe conversion of a hex quantity ("0x…") into whole+fractional
/// units at `decimals` precision, trailing zeros trimmed. Returns null for
/// missing or malformed input so callers can keep their own fallback.
export function formatTokenAmount(
  amountHex: string | null | undefined,
  decimals = 18,
): string | null {
  if (typeof amountHex !== "string") return null;
  const trimmed = amountHex.trim();
  if (!/^0x[0-9a-fA-F]+$/.test(trimmed)) return null;
  if (!Number.isInteger(decimals) || decimals < 0) return null;
  let raw: bigint;
  try {
    raw = BigInt(trimmed);
  } catch (_) {
    return null;
  }
  const base = 10n ** BigInt(decimals);
  const whole = raw / base;
  const fraction = raw % base;
  if (fraction === 0n) return whole.toString();
  const fractionText = fraction
    .toString()
    .padStart(decimals, "0")
    .replace(/0+$/, "");
  return whole.toString() + "." + fractionText;
}

/// Native-currency convenience wrapper over formatTokenAmount (18 decimals)
/// with an optional unit symbol ("1.5 ETH").
export function formatEthAmount(
  weiHex: string | null | undefined,
  symbol?: string | null,
): string | null {
  const amount = formatTokenAmount(weiHex, 18);
  if (amount === null) return null;
  return symbol ? amount + " " + symbol : amount;
}

/// Hex quantity ("0x5208") rendered as a plain decimal string ("21000").
/// Used for unitless counters such as gas used.
export function formatHexQuantity(hex: string | null | undefined): string | null {
  if (typeof hex !== "string") return null;
  const trimmed = hex.trim();
  if (!/^0x[0-9a-fA-F]+$/.test(trimmed)) return null;
  try {
    return BigInt(trimmed).toString();
  } catch (_) {
    return null;
  }
}

/// Resolves a chain id to "1 (ethereum)" using the configured chain
/// registry, falling back to "Chain 1" when the id is not configured.
export function chainLabel(
  chainId: number | string | null | undefined,
  chains: ChainProfile[] | null | undefined,
): string {
  if (chainId === null || chainId === undefined || chainId === "") return "-";
  const numericChainId = Number(chainId);
  const profile = (chains || []).find(
    (chain) => chain.enabled && chain.chain_id === numericChainId,
  );
  if (profile) return String(numericChainId) + " (" + profile.name + ")";
  return Number.isFinite(numericChainId)
    ? "Chain " + numericChainId
    : "Chain " + String(chainId);
}

/// Human amount with the raw hex value kept one click away behind a
/// <details> "raw" affordance. Renders "-" for unknown amounts.
export function amountWithRawHtml(
  amountHex: string | null | undefined,
  options: { decimals?: number; symbol?: string | null } = {},
): string {
  const human = formatTokenAmount(amountHex, options.decimals ?? 18);
  if (human === null) return esc("-");
  const rendered = options.symbol ? human + " " + options.symbol : human;
  return esc(rendered) + rawDetailsHtml(amountHex);
}

/// Decimal quantity (gas used, …) with the raw hex behind "raw".
export function quantityWithRawHtml(hex: string | null | undefined): string {
  const human = formatHexQuantity(hex);
  if (human === null) return esc("-");
  return esc(human) + rawDetailsHtml(hex);
}

function rawDetailsHtml(raw: string | null | undefined): string {
  return (
    ' <details class="raw-details"><summary>raw</summary><code>' +
    esc(String(raw).trim()) +
    "</code></details>"
  );
}
