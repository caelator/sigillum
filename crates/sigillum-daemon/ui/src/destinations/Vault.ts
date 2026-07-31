/**
 * destinations/Vault.ts — the migrated Vault destination (plan task 4.3.5).
 *
 * One coherent security story: lock state always visible at the top with the
 * session/idle countdown, compartments, encrypted secrets + connection keys
 * with reveal-on-demand, FIDO2 hardware keys (poison flows preserved),
 * encrypted snapshots with the backup-age nudge, a paginated audit viewer,
 * and grouped diagnostics + self-check.
 *
 * Migration seam (see core/router.ts header): this controller renders INTO
 * the legacy `#secretsCard` container (the first vault-section card) and
 * hides the remaining legacy vault cards while mounted; both are restored on
 * unmount. The legacy refresh loop keeps re-showing those cards every cycle,
 * so ownership is re-asserted on every store `sync` notification.
 *
 * Endpoints not covered by core/api.ts are thin local wrappers around
 * `requestWithSession` (same error-envelope contract: `{code,error,fields}`
 * failures are thrown and branched on via `apiFailure`).
 */

import { clearSessionToken, requestWithSession } from "../api/session";
import { apiFailure } from "../core/api";
import { el, renderList, clearList, type ElChild } from "../core/dom";
import type { CoreRuntime } from "../core/live";
import type { DestinationController, Route } from "../core/router";
import type { Unsubscribe } from "../core/store";
import type {
  AuditEvent,
  DiagnosticsResponse,
  FieldError,
  SelfCheckResult,
  SelfCheckRunResponse,
  StatusResponse,
} from "../contracts";
import {
  confirmDangerDialog,
  confirmTypedDialog,
} from "../render/confirm";
import { formatTimestamp } from "../render/format";
import { promptSecret } from "../render/secret-prompt";
import { friendlyFidoError } from "../views/fido2";

// ── Constants (server contracts kept verbatim) ───────────────────────

/** Typed-confirm phrase the daemon requires for /api/setup/reset. */
const SETUP_RESET_CONFIRMATION = "RESET LOCAL SIGILLUM DATA";
/** Client-side typed phrase gating snapshot restore (plan 4.3.5e). */
const RESTORE_CONFIRMATION = "RESTORE SNAPSHOT";
/** Reveal-on-demand auto-hide, preserved from the legacy console. */
const REVEAL_AUTOHIDE_MS = 30_000;

/**
 * Reveal auto-hide delay, read lazily so the fake-DOM smoke tests can shrink
 * it via a global (mocking setTimeout itself deadlocks node --test).
 */
function revealAutohideMs(): number {
  return (
    (globalThis as { __SIGILLUM_VAULT_REVEAL_MS__?: number })
      .__SIGILLUM_VAULT_REVEAL_MS__ ?? REVEAL_AUTOHIDE_MS
  );
}
/** Audit page size for the "show more" pager. */
const AUDIT_PAGE = 20;

const HOST_CARD_ID = "secretsCard";
const LEGACY_VAULT_SIBLING_IDS = [
  "apiKeysCard",
  "pushCard",
  "compartmentCard",
  "fido2Card",
  "backupCard",
  "diagCard",
  "guideCard",
];

// ── Local DTOs for the thin wrappers (mirror sigillum-api shapes) ────

interface CompartmentInfo {
  id: number;
  label: string;
  threshold: number;
  passphrase_mode?: string | null;
  is_active: boolean;
}

interface Fido2Detect {
  device_present: boolean;
  device_count: number;
}

interface Fido2KeyInfo {
  label: string;
  credential_id_short: string;
  registered_at: string;
}

interface Envelope {
  code?: string;
  error?: string;
  action?: string;
  fields?: FieldError[];
  [key: string]: unknown;
}

/**
 * Thin wrapper around requestWithSession with the same error-envelope
 * contract as core/api.ts: an `error` payload is thrown as a failure-shaped
 * value so callers branch with `apiFailure`.
 */
async function call<T>(
  method: "GET" | "POST" | "DELETE",
  path: string,
  body?: unknown,
): Promise<T> {
  const payload = (await requestWithSession(method, path, body)) as Envelope;
  if (payload && payload.error != null) {
    throw {
      code: payload.code ?? "unknown",
      error: payload.error,
      action: payload.action,
      fields: payload.fields,
    };
  }
  return payload as T;
}

// ── Humanizers ───────────────────────────────────────────────────────

/** snake_case / camelCase enum → lowercase words ("funded_needs_gas" → "funded needs gas"). */
function humanizeEnum(value: unknown): string {
  return String(value ?? "")
    .replace(/_/g, " ")
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .toLowerCase();
}

function pad2(value: number): string {
  return String(value).padStart(2, "0");
}

/** "just now" / "5m ago" / "3h ago" / "12d ago" for unix-second timestamps. */
function relativeAge(unix: number | null | undefined): string {
  if (!unix) return "never";
  const delta = Math.max(0, Math.floor(Date.now() / 1000) - unix);
  if (delta < 60) return "just now";
  if (delta < 3600) return Math.floor(delta / 60) + "m ago";
  if (delta < 86400) return Math.floor(delta / 3600) + "h ago";
  return Math.floor(delta / 86400) + "d ago";
}

/** Idle-policy seconds → "30 minutes" / "1 hour". */
function durationLabel(secs: number | null | undefined): string {
  if (!secs || secs <= 0) return "disabled";
  if (secs % 3600 === 0) {
    const hours = secs / 3600;
    return hours + (hours === 1 ? " hour" : " hours");
  }
  if (secs % 60 === 0) {
    const minutes = secs / 60;
    return minutes + (minutes === 1 ? " minute" : " minutes");
  }
  return secs + " seconds";
}

function yesNo(value: boolean | null | undefined): string {
  return value ? "yes" : "no";
}

// ── Audit humanization (ported from the legacy formatAuditEvent) ─────

const AUDIT_KIND_LABELS: Record<string, string> = {
  "unlock.passphrase": "Unlocked with passphrase",
  "unlock.fido2": "Unlocked with FIDO2",
  "lock.all": "Locked all compartments",
  "session.revoke": "Revoked session",
  "compartment.add": "Added compartment",
  "compartment.init": "Initialized compartment",
  "compartment.remove": "Removed compartment",
  "compartment.switch": "Switched compartment",
  "api_key.set": "Stored connection key",
  "api_key.delete": "Deleted connection key",
  "secret.set": "Stored encrypted secret",
  "secret.delete": "Deleted encrypted secret",
  "secret.push": "Pushed secret between compartments",
  "profiles.eth_xpub_wallet.upsert": "Saved xpub wallet profile",
  "profiles.eth_xpub_wallet.delete": "Deleted xpub wallet profile",
  "profiles.eth_seed_wallet.upsert": "Imported seed wallet profile",
  "profiles.eth_seed_wallet.delete": "Deleted seed wallet profile",
  "wallet_inventory.risk_catalog.upsert": "Saved risk catalog entry",
  "wallet_inventory.risk_catalog.delete": "Deleted risk catalog entry",
  "wallet.eth_xpub.export": "Exported xpub receive branch",
  "fido2.setup": "Completed FIDO2 setup",
  "fido2.register": "Registered FIDO2 key",
  "fido2.register_poison": "Registered poison FIDO2 key",
  "fido2.remove": "Removed FIDO2 key",
  "snapshot.export": "Exported encrypted snapshot",
  "snapshot.restore": "Restored encrypted snapshot",
};

function formatAuditEvent(event: AuditEvent): string {
  const details = event.details || {};
  const text = (name: string): string => {
    const value = details[name];
    return typeof value === "string" || typeof value === "number"
      ? String(value)
      : "";
  };
  let suffix = "";
  if (text("label")) suffix = " — " + text("label");
  else if (text("key")) suffix = " — " + text("key");
  else if (text("name")) suffix = " — " + text("name");
  else if (text("address")) suffix = " — " + text("address");
  else if (text("wallet_profile")) suffix = " — " + text("wallet_profile");
  else if (text("compartment_count"))
    suffix = " — " + text("compartment_count") + " compartments";
  else if (text("count")) suffix = " — " + text("count") + " compartments";
  else if (text("file_count")) suffix = " — " + text("file_count") + " files";
  const label = AUDIT_KIND_LABELS[event.kind] ?? humanizeEnum(event.kind);
  return label + suffix;
}

/** Newest audit event whose kind starts with `prefix`, or null. */
function newestAuditOfKind(
  events: AuditEvent[] | null,
  prefix: string,
): AuditEvent | null {
  if (!events) return null;
  for (const event of events) {
    if (event.kind.startsWith(prefix)) return event;
  }
  return null;
}

// ── Hex helpers (snapshot export/restore, ported from legacy app.ts) ─

function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

function hexToBytes(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) throw new Error("Invalid hex length");
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < hex.length; i += 2) {
    out[i / 2] = parseInt(hex.slice(i, i + 2), 16);
  }
  return out;
}

// ── Small DOM helpers ────────────────────────────────────────────────

/**
 * Remove all children (keyed-list bookkeeping included). Works in the real
 * DOM and the fake-DOM test harness, where `textContent = ""` does not
 * detach children.
 */
function wipe(node: HTMLElement): void {
  clearList(node);
  while (node.childNodes.length) {
    node.childNodes[0].remove();
  }
}

function skeletonBlock(): HTMLElement {
  return el(
    "div",
    { attrs: { "data-vault": "skeleton", "aria-hidden": "true" } },
    el("div", { class: "skeleton skeleton-text" }),
    el("div", { class: "skeleton skeleton-text short" }),
    el("div", { class: "skeleton skeleton-block" }),
  );
}

function sectionEmpty(
  title: string,
  body: string,
  actionLabel?: string,
  onAction?: () => void,
): HTMLElement {
  return el(
    "div",
    { class: "section-empty" },
    el("p", { class: "section-empty-title", text: title }),
    el("p", { class: "section-empty-body", text: body }),
    actionLabel && onAction
      ? el("button", {
          class: "btn-ghost",
          text: actionLabel,
          attrs: { type: "button" },
          on: { click: () => onAction() },
        })
      : null,
  );
}

function lockedEmpty(what: string): HTMLElement {
  return sectionEmpty(
    "Vault is locked",
    "Unlock the vault from the session panel at the top of the console to " +
      what +
      ".",
  );
}

/** Native <details> raw-value disclosure (raw data allowed behind it). */
function rawDetails(label: string, value: string): HTMLElement {
  return el(
    "details",
    { class: "raw-details" },
    el("summary", { text: label }),
    el("code", { text: value }),
  );
}

// ── The destination ──────────────────────────────────────────────────

export function createVaultDestination(runtime: CoreRuntime): DestinationController {
  // Mount state.
  let host: HTMLElement | null = null;
  let root: HTMLElement | null = null;
  let stashedChildren: ChildNode[] = [];
  let hostWasHidden = false;
  let hostHadHiddenAttribute = false;
  const siblingWasHidden = new Map<HTMLElement, boolean>();
  const siblingHadHiddenAttribute = new Map<HTMLElement, boolean>();
  const unsubs: Unsubscribe[] = [];
  const timers = new Set<ReturnType<typeof setTimeout>>();
  let countdownInterval: ReturnType<typeof setInterval> | null = null;
  let lastActivityAt = Date.now();
  let mounted = false;

  // Resource state (null = first load → skeletons).
  let compartments: CompartmentInfo[] | null = null;
  let secretKeys: string[] | null = null;
  let apiKeys: string[] | null = null;
  let fido2Detect: Fido2Detect | null = null;
  let fido2Keys: Fido2KeyInfo[] | null = null;
  let diagnostics: DiagnosticsResponse | null = null;
  let auditEvents: AuditEvent[] | null = null;
  let auditLimit = AUDIT_PAGE;
  let auditKind = "";
  let selfCheck: SelfCheckRunResponse | null = null;
  let lastSnapshotAtUnix: number | null = null;
  let backupChecked = false;
  const failures = new Set<string>();

  // Reveal-on-demand state per secret/key name.
  const revealed = new Map<string, string>();

  // Persistent keyed-list containers (DESIGN rule 5: patch, don't rebuild).
  let compartmentListInner: HTMLElement | null = null;
  const secretListInner: Partial<Record<string, HTMLElement | null>> = {};
  let fido2ListInner: HTMLElement | null = null;
  let auditTableBody: HTMLElement | null = null;

  // Element refs (built at mount).
  let bannerEl: HTMLElement;
  let flashEl: HTMLElement;
  let sessionEl: HTMLElement;
  let countdownEl: HTMLElement;
  let compartmentListEl: HTMLElement;
  let secretListEl: HTMLElement;
  let apiKeyListEl: HTMLElement;
  let pushWrapEl: HTMLElement;
  let fido2DetectEl: HTMLElement;
  let fido2KeyListEl: HTMLElement;
  let backupNoteEl: HTMLElement;
  let auditListEl: HTMLElement;
  let auditMoreEl: HTMLElement;
  let diagEl: HTMLElement;
  let selfCheckEl: HTMLElement;

  // ── Generic helpers ──────────────────────────────────────────────

  function status(): StatusResponse | null {
    return runtime.store.get("status");
  }

  function isLocked(): boolean {
    return status()?.locked !== false;
  }

  function later(fn: () => void, ms: number): ReturnType<typeof setTimeout> {
    const handle = setTimeout(() => {
      timers.delete(handle);
      fn();
    }, ms);
    timers.add(handle);
    return handle;
  }

  /** Persistent stale-data banner (NOT a toast): lists failed resources. */
  function renderBanner(): void {
    if (!bannerEl) return;
    if (!failures.size) {
      bannerEl.classList.add("hidden");
      wipe(bannerEl);
      return;
    }
    bannerEl.classList.remove("hidden");
    wipe(bannerEl);
    bannerEl.appendChild(
      el(
        "div",
        {},
        el("p", {
          class: "vault-banner-title",
          text: "Some vault data could not be refreshed — what you see may be stale.",
        }),
        el("p", {
          class: "vault-banner-body",
          text: "Failed: " + Array.from(failures).join(" · "),
        }),
      ),
    );
    bannerEl.appendChild(
      el("button", {
        class: "btn-ghost btn-small",
        text: "Retry now",
        attrs: { type: "button" },
        on: { click: () => void refreshAll() },
      }),
    );
  }

  /** One-line action feedback (role=status); errors stay until the next action. */
  function flash(message: string, tier?: "danger" | "review"): void {
    if (!flashEl) return;
    flashEl.textContent = message;
    if (tier) flashEl.dataset.tier = tier;
    else delete flashEl.dataset.tier;
  }

  function fail(resource: string, error: unknown): void {
    const failure = apiFailure(error);
    if (failure?.code === "vault_locked" || failure?.code === "unauthorized") {
      // Lock state drives the section placeholders; not a banner failure.
      return;
    }
    failures.add(resource + (failure ? ": " + failure.error : ""));
    renderBanner();
  }

  function clearFailure(resource: string): void {
    for (const entry of Array.from(failures)) {
      if (entry === resource || entry.startsWith(resource + ":")) {
        failures.delete(entry);
      }
    }
    renderBanner();
  }

  function markInvalid(input: HTMLElement | null, on: boolean): void {
    input?.classList.toggle("input-invalid", on);
  }

  function fieldErrors(error: unknown, inputs: Record<string, HTMLElement | null>): string {
    const failure = apiFailure(error);
    if (!failure) return String(error);
    if (failure.code === "validation_failed" && failure.fields?.length) {
      for (const field of failure.fields) {
        const target = inputs[field.field] ?? inputs[field.field.split(".")[0]];
        markInvalid(target ?? null, true);
      }
      return failure.fields.map((field) => field.message).join(" ");
    }
    return failure.error;
  }

  function setBusy(button: HTMLButtonElement, busy: boolean, busyLabel?: string): void {
    if (busy) {
      button.dataset.idleLabel = button.textContent || "";
      button.disabled = true;
      button.classList.add("btn-busy");
      if (busyLabel) button.textContent = busyLabel;
    } else {
      button.disabled = false;
      button.classList.remove("btn-busy");
      if (button.dataset.idleLabel) button.textContent = button.dataset.idleLabel;
    }
  }

  // ── Host ownership (migration seam) ──────────────────────────────

  function assertHostOwnership(): void {
    if (!host) return;
    const unlocked = document.body.dataset.mode === "unlocked";
    host.classList.toggle("hidden", !unlocked);
    if (unlocked) host.removeAttribute("hidden");
    else host.setAttribute("hidden", "");
    for (const id of LEGACY_VAULT_SIBLING_IDS) {
      const card = document.getElementById(id);
      if (!card) continue;
      card.classList.add("hidden");
      // The legacy refresh loop toggles the class directly. Keep a native
      // visibility barrier as well so a refresh cannot briefly expose stale
      // controls while this destination owns the vault surface.
      card.setAttribute("hidden", "");
    }
  }

  function releaseHostOwnership(): void {
    for (const id of LEGACY_VAULT_SIBLING_IDS) {
      const card = document.getElementById(id);
      if (!card) continue;
      const was = siblingWasHidden.get(card);
      if (was !== undefined) card.classList.toggle("hidden", was);
      const hadHiddenAttribute = siblingHadHiddenAttribute.get(card);
      if (hadHiddenAttribute === true) card.setAttribute("hidden", "");
      else if (hadHiddenAttribute === false) card.removeAttribute("hidden");
    }
    siblingWasHidden.clear();
    siblingHadHiddenAttribute.clear();
  }

  // ── Session & lock state (spec item a) ───────────────────────────

  function renderSession(): void {
    if (!sessionEl) return;
    wipe(sessionEl);
    const current = status();

    if (!current) {
      sessionEl.appendChild(skeletonBlock());
      return;
    }

    const strip = el("div", {
      class: "vault-lock-strip",
      attrs: { "data-vault": "lock-strip" },
    });
    strip.dataset.tier = current.locked ? "review" : "quiet";

    const dot = el("span", {
      class: "status-dot",
      attrs: { "aria-hidden": "true" },
    });
    dot.dataset.state = current.locked ? "paused" : "live";

    const main = el("div", { class: "vault-lock-strip-main" });
    if (current.locked) {
      main.appendChild(
        el("p", {
          class: "vault-lock-title",
          text: "Vault is locked",
        }),
      );
      main.appendChild(
        el("p", {
          class: "vault-lock-body",
          text:
            "Master keys are zeroized. Unlock from the session panel at the top of the console to manage compartments, secrets, keys, and snapshots.",
        }),
      );
    } else {
      const active = current.active_compartment;
      const count = current.unlocked_compartments.length;
      main.appendChild(
        el("p", {
          class: "vault-lock-title",
          text:
            "Unlocked — " +
            count +
            (count === 1 ? " compartment" : " compartments") +
            " in this session",
        }),
      );
      main.appendChild(
        el("p", {
          class: "vault-lock-body",
          text: active
            ? "Active compartment: " + active.compartment_label + "."
            : "No active compartment selected.",
        }),
      );
    }
    strip.appendChild(dot);
    strip.appendChild(main);

    const actions = el("div", { class: "vault-lock-strip-actions" });
    if (!current.locked) {
      actions.appendChild(
        el("button", {
          class: "btn-ghost btn-small",
          text: "Lock now",
          attrs: { type: "button", "data-vault": "lock-now" },
          on: { click: () => void lockNow() },
        }),
      );
      actions.appendChild(
        el("button", {
          class: "btn-ghost btn-small",
          text: "Log out",
          attrs: { type: "button", "data-vault": "logout" },
          on: { click: () => void logout() },
        }),
      );
    }
    strip.appendChild(actions);
    sessionEl.appendChild(strip);

    // Compartment switcher (spec item a/b): the unlocked set from the store.
    if (!current.locked && current.unlocked_compartments.length > 1) {
      const select = el(
        "select",
        {
          class: "input-wide",
          attrs: { "aria-label": "Active compartment", "data-vault": "switcher" },
        },
      ) as HTMLSelectElement;
      for (const compartment of current.unlocked_compartments) {
        const option = el("option", {
          text: compartment.label + " (#" + compartment.id + ")",
          attrs: { value: String(compartment.id) },
        }) as HTMLOptionElement;
        if (current.active_compartment?.compartment_id === compartment.id) {
          option.selected = true;
        }
        select.appendChild(option);
      }
      if (current.active_compartment) {
        select.value = String(current.active_compartment.compartment_id);
      }
      const switchButton = el("button", {
        class: "btn-primary btn-small",
        text: "Switch",
        attrs: { type: "submit" },
      }) as HTMLButtonElement;
      const form = el(
        "form",
        {
          class: "vault-inline-form",
          attrs: { "data-vault": "switcher-form" },
          on: {
            submit: (event) => {
              event.preventDefault?.();
              void switchCompartment(Number(select.value), switchButton);
            },
          },
        },
        el("label", { class: "vault-form-label", text: "Active compartment" }),
        select,
        switchButton,
      );
      sessionEl.appendChild(form);
    }

    // Idle countdown (spec item a): policy from diagnostics + this tab's
    // activity estimate. Honest labelling: it is an estimate, since the
    // daemon counts its own request activity.
    countdownEl = el("p", {
      class: "vault-countdown nums",
      attrs: { "data-vault": "countdown" },
    });
    sessionEl.appendChild(countdownEl);
    renderCountdown();
  }

  function renderCountdown(): void {
    if (!countdownEl || !countdownEl.isConnected) return;
    if (isLocked()) {
      countdownEl.textContent = "";
      return;
    }
    const policy = diagnostics?.runtime_policy;
    if (!policy) {
      countdownEl.textContent = "Idle auto-lock: policy loading…";
      return;
    }
    if (!policy.idle_lock_secs || policy.idle_lock_secs <= 0) {
      countdownEl.textContent = "Idle auto-lock is disabled on this daemon.";
      return;
    }
    const elapsedSecs = Math.floor((Date.now() - lastActivityAt) / 1000);
    const remaining = Math.max(0, policy.idle_lock_secs - elapsedSecs);
    const minutes = Math.floor(remaining / 60);
    const seconds = remaining % 60;
    countdownEl.textContent =
      "Auto-lock after " +
      durationLabel(policy.idle_lock_secs) +
      " idle — about " +
      minutes +
      ":" +
      pad2(seconds) +
      " left (this tab's estimate).";
  }

  async function lockNow(): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Lock all compartments",
      body: "Master keys will be zeroized from daemon memory. You will need to unlock the vault again to continue working.",
      actionLabel: "Lock all",
    });
    if (!confirmed) return;
    try {
      await call("POST", "/api/lock");
      clearSessionToken();
      flash("All compartments locked.");
    } catch (error) {
      flash(fieldErrors(error, {}), "danger");
    }
  }

  async function logout(): Promise<void> {
    try {
      await call("POST", "/api/session/revoke");
      clearSessionToken();
      flash("Session logged out. The daemon state is unchanged.");
    } catch (error) {
      flash(fieldErrors(error, {}), "danger");
    }
  }

  async function switchCompartment(id: number, button: HTMLButtonElement): Promise<void> {
    if (!Number.isFinite(id)) return;
    setBusy(button, true);
    try {
      await call("POST", "/api/compartment/switch", { id });
      flash("Switched to compartment #" + id + ".");
      // The SSE status event refreshes the slice; fetch once for immediacy.
      runtime.store.set("status", await runtime.api.getStatus());
    } catch (error) {
      flash(fieldErrors(error, {}), "danger");
    } finally {
      setBusy(button, false);
    }
  }

  // ── Compartments (spec item b) ───────────────────────────────────

  function renderCompartments(): void {
    if (!compartmentListEl) return;
    const current = status();
    if (!current || current.locked || !compartments || !compartments.length) {
      wipe(compartmentListEl);
      compartmentListInner = null;
      if (!current) compartmentListEl.appendChild(skeletonBlock());
      else if (current.locked)
        compartmentListEl.appendChild(lockedEmpty("see compartments"));
      else if (!compartments) compartmentListEl.appendChild(skeletonBlock());
      else
        compartmentListEl.appendChild(
          sectionEmpty(
            "No compartments yet",
            "Compartments are isolated vault spaces with their own unlock requirements. Add the first one below.",
          ),
        );
      return;
    }
    if (!compartmentListInner || compartmentListInner.parentNode !== compartmentListEl) {
      wipe(compartmentListEl);
      compartmentListInner = el("div", { class: "vault-item-list" });
      compartmentListEl.appendChild(compartmentListInner);
    }
    renderList(
      compartmentListInner,
      compartments,
      (compartment) => String(compartment.id),
      (compartment, existing) => renderCompartmentRow(compartment, existing),
    );
  }

  function renderCompartmentRow(
    compartment: CompartmentInfo,
    existing: HTMLElement | null,
  ): HTMLElement {
    const signature =
      compartment.label +
      "|" +
      compartment.threshold +
      "|" +
      (compartment.is_active ? "a" : "i") +
      "|" +
      (compartment.passphrase_mode ?? "");
    if (existing && existing.dataset.signature === signature) return existing;

    const children: ElChild[] = [
      el(
        "div",
        { class: "vault-item-main" },
        el(
          "p",
          { class: "vault-item-title" },
          el("span", { text: compartment.label }),
          " ",
          compartment.is_active
            ? el("span", { class: "pill pill-good", text: "active" })
            : null,
        ),
        el("p", {
          class: "vault-item-body",
          text:
            "Requires " +
            compartment.threshold +
            (compartment.threshold === 1 ? " key" : " keys") +
            " to unlock" +
            (compartment.passphrase_mode
              ? " · passphrase " + humanizeEnum(compartment.passphrase_mode)
              : "") +
            ".",
        }),
      ),
    ];
    if (!compartment.is_active) {
      const button = el("button", {
        class: "btn-ghost btn-small",
        text: "Make active",
        attrs: { type: "button" },
      }) as HTMLButtonElement;
      button.addEventListener("click", () =>
        void switchCompartment(compartment.id, button),
      );
      children.push(el("div", { class: "vault-item-actions" }, button));
    }
    const row = el(
      "div",
      {
        class: "vault-item",
        attrs: { "data-vault": "compartment-row" },
      },
      ...children,
    );
    row.dataset.signature = signature;
    // renderList only removes vanished keys: a shape change must retire
    // the old node itself or it lingers as a zombie next to the fresh one.
    existing?.remove();
    return row;
  }

  async function loadCompartments(): Promise<void> {
    try {
      const response = await call<{ compartments: CompartmentInfo[] }>(
        "GET",
        "/api/compartment/list",
      );
      compartments = response.compartments ?? [];
      clearFailure("compartments");
    } catch (error) {
      fail("compartments", error);
    }
    renderCompartments();
    renderPush();
  }

  async function addCompartment(
    inputs: {
      label: HTMLInputElement;
      threshold: HTMLInputElement;
      mode: HTMLSelectElement;
    },
    button: HTMLButtonElement,
  ): Promise<void> {
    const label = inputs.label.value.trim();
    const threshold = parseInt(inputs.threshold.value, 10);
    markInvalid(inputs.label, false);
    markInvalid(inputs.threshold, false);
    if (!label) {
      markInvalid(inputs.label, true);
      flash("Name the compartment first.", "review");
      inputs.label.focus();
      return;
    }
    if (!Number.isFinite(threshold) || threshold < 1) {
      markInvalid(inputs.threshold, true);
      flash("Threshold is how many keys unlock this compartment — 1 or more.", "review");
      inputs.threshold.focus();
      return;
    }
    setBusy(button, true, "Adding…");
    try {
      await call("POST", "/api/compartment/add", {
        label,
        threshold,
        passphrase_mode: inputs.mode.value || null,
      });
      inputs.label.value = "";
      inputs.threshold.value = "";
      flash('Compartment "' + label + '" added.');
      await loadCompartments();
    } catch (error) {
      flash(
        fieldErrors(error, { label: inputs.label, threshold: inputs.threshold }),
        "danger",
      );
    } finally {
      setBusy(button, false);
    }
  }

  // ── Secrets + connection keys (spec item c) ──────────────────────

  interface SecretKindConfig {
    resource: string;
    noun: string;
    listPath: string;
    setPath: string;
    getPath: string;
    deletePath: string;
    emptyTitle: string;
    emptyBody: string;
    deleteTitle: (name: string) => string;
    deleteBody: (name: string) => string;
  }

  const SECRET_KIND: SecretKindConfig = {
    resource: "secrets",
    noun: "secret",
    listPath: "/api/secrets",
    setPath: "/api/secrets/set",
    getPath: "/api/secrets/get",
    deletePath: "/api/secrets/delete",
    emptyTitle: "No encrypted secrets yet",
    emptyBody:
      "Values are encrypted at rest in the active compartment. Store the first protected value above.",
    deleteTitle: (name) => 'Delete secret "' + name + '"?',
    deleteBody: (name) =>
      'Delete secret "' +
      name +
      '"? The encrypted value is removed from this compartment and cannot be recovered from the vault.',
  };

  const API_KEY_KIND: SecretKindConfig = {
    resource: "connection keys",
    noun: "connection key",
    listPath: "/api/api-keys",
    setPath: "/api/api-keys/set",
    getPath: "/api/api-keys/get",
    deletePath: "/api/api-keys/delete",
    emptyTitle: "No connection keys yet",
    emptyBody:
      "RPC tokens and similar operational keys the daemon uses directly. Store the first one above.",
    deleteTitle: (name) => 'Delete connection key "' + name + '"?',
    deleteBody: (name) =>
      'Delete connection key "' +
      name +
      '"? Providers using this stored credential lose their authenticated access.',
  };

  function keysFor(config: SecretKindConfig): string[] | null {
    return config === SECRET_KIND ? secretKeys : apiKeys;
  }

  function listFor(config: SecretKindConfig): HTMLElement {
    return config === SECRET_KIND ? secretListEl : apiKeyListEl;
  }

  function renderSecretKind(config: SecretKindConfig): void {
    const container = listFor(config);
    if (!container) return;
    const current = status();
    const keys = keysFor(config);
    if (!current || current.locked || !keys || !keys.length) {
      wipe(container);
      secretListInner[config.noun] = null;
      if (!current) container.appendChild(skeletonBlock());
      else if (current.locked)
        container.appendChild(lockedEmpty("manage " + config.noun + "s"));
      else if (!keys) container.appendChild(skeletonBlock());
      else container.appendChild(sectionEmpty(config.emptyTitle, config.emptyBody));
      return;
    }
    let list = secretListInner[config.noun] ?? null;
    if (!list || list.parentNode !== container) {
      wipe(container);
      list = el("div", {
        class: "vault-item-list",
        attrs: { "data-vault": config.resource + "-list" },
      });
      container.appendChild(list);
      secretListInner[config.noun] = list;
    }
    renderList(list, keys, (key) => key, (key, existing) =>
      renderSecretRow(config, key, existing),
    );
  }

  function renderSecretRow(
    config: SecretKindConfig,
    key: string,
    existing: HTMLElement | null,
  ): HTMLElement {
    const value = revealed.get(config.noun + ":" + key) ?? null;
    const signature = key + "|" + (value !== null ? "r" : "h");
    if (existing && existing.dataset.signature === signature) return existing;

    const revealButton = el("button", {
      class: "btn-ghost btn-small",
      text: value !== null ? "Hide" : "Reveal",
      attrs: {
        type: "button",
        "aria-label": (value !== null ? "Hide " : "Reveal ") + config.noun + " " + key,
      },
    }) as HTMLButtonElement;
    revealButton.addEventListener("click", () =>
      value !== null ? hideSecret(config, key) : void revealSecret(config, key),
    );
    const deleteButton = el("button", {
      class: "btn-danger btn-small",
      text: "Delete",
      attrs: { type: "button", "aria-label": "Delete " + config.noun + " " + key },
    }) as HTMLButtonElement;
    deleteButton.addEventListener("click", () => void deleteSecret(config, key));

    const row = el(
      "div",
      { class: "vault-item", attrs: { "data-vault": "secret-row" } },
      el(
        "div",
        { class: "vault-item-main" },
        el("p", { class: "vault-item-title", text: key }),
        value !== null
          ? el("p", {
              class: "vault-reveal mono",
              text: value,
              attrs: { "data-vault": "revealed" },
            })
          : null,
      ),
      el("div", { class: "vault-item-actions" }, revealButton, deleteButton),
    );
    row.dataset.signature = signature;
    // renderList only removes vanished keys: a shape change must retire
    // the old node itself or it lingers as a zombie next to the fresh one.
    existing?.remove();
    return row;
  }

  async function loadSecretKind(config: SecretKindConfig): Promise<void> {
    try {
      const response = await call<{ keys: string[] }>("GET", config.listPath);
      if (config === SECRET_KIND) secretKeys = response.keys ?? [];
      else apiKeys = response.keys ?? [];
      clearFailure(config.resource);
    } catch (error) {
      fail(config.resource, error);
    }
    renderSecretKind(config);
  }

  async function setSecret(
    config: SecretKindConfig,
    nameInput: HTMLInputElement,
    valueInput: HTMLInputElement,
    button: HTMLButtonElement,
  ): Promise<void> {
    const key = nameInput.value.trim();
    const value = valueInput.value;
    markInvalid(nameInput, false);
    markInvalid(valueInput, false);
    if (!key || !value) {
      markInvalid(nameInput, !key);
      markInvalid(valueInput, !value);
      flash("Both a name and a value are required.", "review");
      (key ? valueInput : nameInput).focus();
      return;
    }
    setBusy(button, true, "Storing…");
    try {
      await call("POST", config.setPath, { key, value });
      nameInput.value = "";
      valueInput.value = "";
      flash(config.noun + ' "' + key + '" stored.');
      await loadSecretKind(config);
    } catch (error) {
      flash(
        fieldErrors(error, { key: nameInput, value: valueInput }),
        "danger",
      );
    } finally {
      setBusy(button, false);
    }
  }

  async function revealSecret(config: SecretKindConfig, key: string): Promise<void> {
    try {
      const response = await call<{ value: string }>("POST", config.getPath, { key });
      revealed.set(config.noun + ":" + key, response.value);
      renderSecretKind(config);
      // Auto-hide after 30s, preserved from the legacy console.
      later(() => {
        revealed.delete(config.noun + ":" + key);
        renderSecretKind(config);
      }, revealAutohideMs());
    } catch (error) {
      flash(fieldErrors(error, {}), "danger");
    }
  }

  function hideSecret(config: SecretKindConfig, key: string): void {
    revealed.delete(config.noun + ":" + key);
    renderSecretKind(config);
  }

  async function deleteSecret(config: SecretKindConfig, key: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: config.deleteTitle(key),
      body: config.deleteBody(key),
      actionLabel: "Delete",
    });
    if (!confirmed) return;
    try {
      await call("POST", config.deletePath, { key });
      revealed.delete(config.noun + ":" + key);
      flash(config.noun + ' "' + key + '" deleted.');
      await loadSecretKind(config);
    } catch (error) {
      flash(fieldErrors(error, {}), "danger");
    }
  }

  // Inter-compartment push (preserved flow; shown with ≥2 unlocked).

  function renderPush(): void {
    if (!pushWrapEl) return;
    wipe(pushWrapEl);
    const current = status();
    if (!current || current.locked || current.unlocked_compartments.length < 2) {
      pushWrapEl.appendChild(
        el("p", {
          class: "helper-text",
          text: "Push is available once at least two compartments are unlocked in this session.",
        }),
      );
      return;
    }
    const from = el("select", {
      class: "input-wide",
      attrs: { "aria-label": "From compartment" },
    }) as HTMLSelectElement;
    const to = el("select", {
      class: "input-wide",
      attrs: { "aria-label": "To compartment" },
    }) as HTMLSelectElement;
    for (const compartment of current.unlocked_compartments) {
      const label = compartment.label + " (#" + compartment.id + ")";
      from.appendChild(
        el("option", { text: label, attrs: { value: String(compartment.id) } }),
      );
      to.appendChild(
        el("option", { text: label, attrs: { value: String(compartment.id) } }),
      );
    }
    if (current.unlocked_compartments.length > 1) to.selectedIndex = 1;
    const keyInput = el("input", {
      attrs: {
        type: "text",
        placeholder: "Secret key name",
        "aria-label": "Secret key name",
      },
    }) as HTMLInputElement;
    const newKeyInput = el("input", {
      attrs: {
        type: "text",
        placeholder: "New name (optional)",
        "aria-label": "New name (optional)",
      },
    }) as HTMLInputElement;
    const tier = el(
      "select",
      { class: "input-wide", attrs: { "aria-label": "Storage tier" } },
      el("option", { text: "Encrypted secret (tier 2)", attrs: { value: "2" } }),
      el("option", { text: "Connection key (tier 1)", attrs: { value: "1" } }),
    ) as HTMLSelectElement;
    const pushButton = el("button", {
      class: "btn-primary",
      text: "Push",
      attrs: { type: "submit" },
    }) as HTMLButtonElement;
    pushWrapEl.appendChild(
      el(
        "form",
        {
          class: "vault-form",
          attrs: { "data-vault": "push-form" },
          on: {
            submit: (event) => {
              event.preventDefault?.();
              void pushSecret(from, to, keyInput, newKeyInput, tier, pushButton);
            },
          },
        },
        el(
          "div",
          { class: "form-row" },
          from,
          el("span", { class: "helper-text", text: "→" }),
          to,
        ),
        el("div", { class: "form-row" }, keyInput, newKeyInput),
        el("div", { class: "form-row" }, tier, pushButton),
      ),
    );
  }

  async function pushSecret(
    from: HTMLSelectElement,
    to: HTMLSelectElement,
    keyInput: HTMLInputElement,
    newKeyInput: HTMLInputElement,
    tier: HTMLSelectElement,
    button: HTMLButtonElement,
  ): Promise<void> {
    const fromId = parseInt(from.value, 10);
    const toId = parseInt(to.value, 10);
    const key = keyInput.value.trim();
    const newKey = newKeyInput.value.trim() || null;
    markInvalid(keyInput, false);
    if (!key) {
      markInvalid(keyInput, true);
      flash("Name the secret to push.", "review");
      keyInput.focus();
      return;
    }
    if (fromId === toId) {
      flash("Source and target compartments must differ.", "review");
      return;
    }
    setBusy(button, true, "Pushing…");
    try {
      await call("POST", "/api/secrets/push", {
        from_compartment: fromId,
        to_compartment: toId,
        key,
        new_key: newKey,
        tier: parseInt(tier.value, 10),
      });
      keyInput.value = "";
      newKeyInput.value = "";
      flash('Pushed "' + key + '" to compartment #' + toId + ".");
    } catch (error) {
      flash(fieldErrors(error, { key: keyInput }), "danger");
    } finally {
      setBusy(button, false);
    }
  }

  // ── Hardware keys (spec item d) ──────────────────────────────────

  function renderFido2(): void {
    if (!fido2DetectEl || !fido2KeyListEl) return;
    wipe(fido2DetectEl);
    if (!status()) {
      wipe(fido2KeyListEl);
      fido2ListInner = null;
      fido2DetectEl.appendChild(skeletonBlock());
      fido2KeyListEl.appendChild(skeletonBlock());
      return;
    }
    if (isLocked()) {
      wipe(fido2KeyListEl);
      fido2ListInner = null;
      fido2DetectEl.appendChild(lockedEmpty("manage hardware keys"));
      return;
    }
    if (!fido2Detect) {
      fido2DetectEl.appendChild(skeletonBlock());
    } else {
      const dot = el("span", {
        class: "status-dot",
        attrs: { "aria-hidden": "true" },
      });
      dot.dataset.state = fido2Detect.device_present ? "live" : "paused";
      fido2DetectEl.appendChild(
        el(
          "p",
          { class: "vault-detect", attrs: { "data-vault": "fido2-detect-line" } },
          dot,
          fido2Detect.device_present
            ? " " +
                fido2Detect.device_count +
                " hardware key(s) connected."
            : " No hardware key detected. Insert a FIDO2 key to register or remove one.",
        ),
      );
    }

    if (!fido2Keys || !fido2Keys.length) {
      wipe(fido2KeyListEl);
      fido2ListInner = null;
      if (!fido2Keys) fido2KeyListEl.appendChild(skeletonBlock());
      else
        fido2KeyListEl.appendChild(
          sectionEmpty(
            "No additional hardware keys registered",
            "Register a key above to improve recovery and enable higher-threshold unlock paths.",
          ),
        );
      return;
    }
    if (!fido2ListInner || fido2ListInner.parentNode !== fido2KeyListEl) {
      wipe(fido2KeyListEl);
      fido2ListInner = el("div", { class: "vault-item-list" });
      fido2KeyListEl.appendChild(fido2ListInner);
    }
    renderList(
      fido2ListInner,
      fido2Keys,
      (key) => key.label,
      (key, existing) => renderFido2Row(key, existing),
    );
  }

  function renderFido2Row(key: Fido2KeyInfo, existing: HTMLElement | null): HTMLElement {
    const signature = key.label + "|" + key.credential_id_short + "|" + key.registered_at;
    if (existing && existing.dataset.signature === signature) return existing;
    const removeButton = el("button", {
      class: "btn-danger btn-small",
      text: "Remove",
      attrs: { type: "button", "aria-label": "Remove hardware key " + key.label },
    }) as HTMLButtonElement;
    removeButton.addEventListener("click", () => void removeFido2Key(key.label));
    const row = el(
      "div",
      { class: "vault-item", attrs: { "data-vault": "fido2-row" } },
      el(
        "div",
        { class: "vault-item-main" },
        el("p", { class: "vault-item-title", text: key.label }),
        el(
          "p",
          { class: "vault-item-body" },
          el("span", { text: "Registered " + key.registered_at + " " }),
          rawDetails("credential", key.credential_id_short + "…"),
        ),
      ),
      el("div", { class: "vault-item-actions" }, removeButton),
    );
    row.dataset.signature = signature;
    // renderList only removes vanished keys: a shape change must retire
    // the old node itself or it lingers as a zombie next to the fresh one.
    existing?.remove();
    return row;
  }

  async function loadFido2(): Promise<void> {
    try {
      fido2Detect = await call<Fido2Detect>("GET", "/api/fido2/detect");
      clearFailure("hardware keys");
    } catch (error) {
      fail("hardware keys", error);
    }
    try {
      const response = await call<{ keys: Fido2KeyInfo[] }>("GET", "/api/fido2/list");
      fido2Keys = response.keys ?? [];
      clearFailure("hardware key list");
    } catch (error) {
      fail("hardware key list", error);
    }
    renderFido2();
  }

  async function setFido2Pin(
    pinInput: HTMLInputElement,
    confirmInput: HTMLInputElement,
    button: HTMLButtonElement,
  ): Promise<void> {
    const pin = pinInput.value;
    markInvalid(pinInput, false);
    markInvalid(confirmInput, false);
    if (pin.length < 4) {
      markInvalid(pinInput, true);
      flash("New PIN must be at least 4 characters.", "review");
      pinInput.focus();
      return;
    }
    if (pin !== confirmInput.value) {
      markInvalid(confirmInput, true);
      flash("PIN entries do not match.", "review");
      confirmInput.focus();
      return;
    }
    setBusy(button, true, "Setting PIN…");
    try {
      await call("POST", "/api/fido2/pin/set", { new_pin: pin });
      pinInput.value = "";
      confirmInput.value = "";
      flash("Hardware-key PIN set. Use that PIN in the registration field below.");
    } catch (error) {
      flash(friendlyFidoError(apiFailure(error)?.error ?? error), "danger");
    } finally {
      setBusy(button, false);
    }
  }

  async function registerFido2Key(inputs: {
    pin: HTMLInputElement;
    label: HTMLInputElement;
    poison: HTMLInputElement;
    skip: HTMLInputElement;
    button: HTMLButtonElement;
  }): Promise<void> {
    const label = inputs.label.value.trim();
    const pin = inputs.pin.value;
    const poison = inputs.poison.checked;
    const skipKeys = inputs.skip.value
      .split(",")
      .map((part) => part.trim())
      .filter(Boolean);
    markInvalid(inputs.label, false);
    if (!label) {
      markInvalid(inputs.label, true);
      flash("Label the key first — something you will recognize later.", "review");
      inputs.label.focus();
      return;
    }
    // Poison-key flow preserved verbatim from the legacy console.
    if (poison) {
      const confirmed = await confirmDangerDialog({
        title: "Register poison key",
        body:
          'Register "' +
          label +
          '" as a POISON key? Including it during unlock will cause silent failure.',
        actionLabel: "Register poison key",
      });
      if (!confirmed) return;
    }
    setBusy(inputs.button, true, "Touch your key…");
    try {
      const body: Record<string, unknown> = { label };
      if (pin) body.pin = pin;
      if (poison) body.poison = true;
      if (skipKeys.length) body.skip_keys = skipKeys;
      await call("POST", "/api/fido2/register", body);
      inputs.pin.value = "";
      inputs.label.value = "";
      inputs.skip.value = "";
      inputs.poison.checked = false;
      flash('Key "' + label + '" registered' + (poison ? " (poison)" : "") + ".");
      await loadFido2();
    } catch (error) {
      flash(friendlyFidoError(apiFailure(error)?.error ?? error), "danger");
    } finally {
      setBusy(inputs.button, false);
    }
  }

  async function removeFido2Key(label: string): Promise<void> {
    // Confirm copy preserved verbatim from the legacy console.
    const confirmed = await confirmDangerDialog({
      title: "Remove FIDO2 key",
      body:
        'Remove FIDO2 key "' +
        label +
        '"? Unlock thresholds that count on this key will no longer be able to use it.',
      actionLabel: "Remove key",
    });
    if (!confirmed) return;
    const pinDecision = await promptSecret({
      title: "Enter the current FIDO2 PIN only if the remaining keys require one:",
      inputLabel: "Current FIDO2 PIN",
      placeholder: "Current PIN (leave blank if not required)",
    });
    if (!pinDecision.submitted) return;
    try {
      const body: Record<string, unknown> = { label };
      if (pinDecision.value) body.pin = pinDecision.value;
      await call("POST", "/api/fido2/remove", body);
      flash('Key "' + label + '" removed.');
      await loadFido2();
    } catch (error) {
      flash(friendlyFidoError(apiFailure(error)?.error ?? error), "danger");
    }
  }

  // ── Snapshots (spec item e) ──────────────────────────────────────

  function renderBackupNote(): void {
    if (!backupNoteEl) return;
    wipe(backupNoteEl);
    if (!status() || isLocked()) return;
    if (!backupChecked) {
      backupNoteEl.appendChild(
        el("div", { class: "skeleton skeleton-text short" }),
      );
      return;
    }
    if (lastSnapshotAtUnix) {
      const note = el(
        "p",
        { class: "helper-text", attrs: { "data-vault": "backup-age" } },
        el("span", {
          text:
            "Last snapshot exported " +
            relativeAge(lastSnapshotAtUnix) +
            " (" +
            formatTimestamp(lastSnapshotAtUnix) +
            "). ",
        }),
      );
      backupNoteEl.appendChild(note);
      return;
    }
    const nudge = el(
      "div",
      { class: "vault-nudge", attrs: { "data-vault": "backup-nudge" } },
    );
    nudge.dataset.tier = "review";
    nudge.appendChild(
      el("p", {
        class: "vault-nudge-title",
        text: "No snapshot export is recorded in this daemon's audit trail.",
      }),
    );
    nudge.appendChild(
      el("p", {
        class: "vault-nudge-body",
        text:
          "An encrypted snapshot is the only way to restore this vault if this machine fails. Export one now and store the file somewhere safe.",
      }),
    );
    backupNoteEl.appendChild(nudge);
  }

  async function loadBackupState(): Promise<void> {
    try {
      const response = await runtime.api.listAudit({ kind: "snapshot.export", limit: 1 });
      lastSnapshotAtUnix = response.events?.[0]?.created_at_unix ?? null;
      backupChecked = true;
      clearFailure("backup history");
    } catch (error) {
      fail("backup history", error);
    }
    renderBackupNote();
  }

  async function exportSnapshot(
    passInput: HTMLInputElement,
    button: HTMLButtonElement,
  ): Promise<void> {
    const passphrase = passInput.value;
    markInvalid(passInput, false);
    if (passphrase.length < 8) {
      markInvalid(passInput, true);
      flash("Export passphrase must be at least 8 characters.", "review");
      passInput.focus();
      return;
    }
    setBusy(button, true, "Exporting…");
    try {
      const response = await call<{
        snapshot_hex: string;
        summary?: { created_at_unix?: number };
      }>("POST", "/api/backup/export", { passphrase });
      passInput.value = "";
      const bytes = hexToBytes(response.snapshot_hex);
      if (typeof URL !== "undefined" && typeof URL.createObjectURL === "function") {
        const blob = new Blob([bytes as unknown as BlobPart], {
          type: "application/json",
        });
        const url = URL.createObjectURL(blob);
        const anchor = document.createElement("a");
        anchor.href = url;
        anchor.download =
          "sigillum-snapshot-" +
          (response.summary?.created_at_unix || Date.now()) +
          ".json";
        document.body.appendChild(anchor);
        anchor.click();
        anchor.remove();
        URL.revokeObjectURL(url);
        flash("Snapshot downloaded. Store it somewhere safe.");
      } else {
        flash("Snapshot exported, but this browser cannot download files.", "review");
      }
      backupChecked = false;
      await loadBackupState();
    } catch (error) {
      flash(fieldErrors(error, { passphrase: passInput }), "danger");
    } finally {
      setBusy(button, false);
    }
  }

  async function restoreSnapshot(
    fileInput: HTMLInputElement,
    passInput: HTMLInputElement,
    button: HTMLButtonElement,
  ): Promise<void> {
    const file = fileInput.files?.[0];
    const passphrase = passInput.value;
    markInvalid(passInput, false);
    if (!file) {
      flash("Choose a snapshot file first.", "review");
      return;
    }
    if (passphrase.length < 8) {
      markInvalid(passInput, true);
      flash("Restore passphrase must be at least 8 characters.", "review");
      passInput.focus();
      return;
    }
    const confirmed = await confirmTypedDialog({
      title: "Restore snapshot",
      body: "Restoring replaces the current on-disk Sigillum data with this snapshot and logs you out. You will need to unlock the vault again afterwards.",
      phrase: RESTORE_CONFIRMATION,
      actionLabel: "Restore",
    });
    if (!confirmed) return;
    let snapshotHex: string;
    try {
      snapshotHex = bytesToHex(new Uint8Array(await file.arrayBuffer()));
    } catch (_) {
      flash("Failed to read the snapshot file.", "danger");
      return;
    }
    setBusy(button, true, "Restoring…");
    try {
      await call("POST", "/api/backup/restore", {
        passphrase,
        snapshot_hex: snapshotHex,
      });
      clearSessionToken();
      passInput.value = "";
      fileInput.value = "";
      flash("Snapshot restored. Unlock the vault again to continue.");
    } catch (error) {
      flash(fieldErrors(error, { passphrase: passInput }), "danger");
    } finally {
      setBusy(button, false);
    }
  }

  async function resetLocalData(button: HTMLButtonElement): Promise<void> {
    const confirmed = await confirmTypedDialog({
      title: "Reset local Sigillum data",
      body: "Archive this machine's Sigillum data and return to first-run setup? The current data directory is moved aside (not deleted), but you will need a new vault to continue.",
      phrase: SETUP_RESET_CONFIRMATION,
      actionLabel: "Archive & reset",
    });
    if (!confirmed) return;
    setBusy(button, true, "Archiving…");
    try {
      const response = await call<{ archived_to?: string }>(
        "POST",
        "/api/setup/reset",
        { confirmation: SETUP_RESET_CONFIRMATION },
      );
      clearSessionToken();
      flash(
        response.archived_to
          ? "Previous data archived to " + response.archived_to + ". Starting first-run setup."
          : "Local Sigillum data cleared. Starting first-run setup.",
      );
    } catch (error) {
      flash(fieldErrors(error, {}), "danger");
    } finally {
      setBusy(button, false);
    }
  }

  // ── Audit viewer (spec item f) ───────────────────────────────────

  function renderAudit(): void {
    if (!auditListEl || !auditMoreEl) return;
    const current = status();
    if (!current || current.locked || !auditEvents || !auditEvents.length) {
      wipe(auditListEl);
      auditTableBody = null;
      auditMoreEl.classList.add("hidden");
      if (!current) auditListEl.appendChild(skeletonBlock());
      else if (current.locked)
        auditListEl.appendChild(lockedEmpty("read the audit trail"));
      else if (!auditEvents) auditListEl.appendChild(skeletonBlock());
      else
        auditListEl.appendChild(
          sectionEmpty(
            "No audit events" + (auditKind ? ' for "' + auditKind + '"' : " yet"),
            "The daemon records every sensitive action locally — unlocks, secret changes, compartment switches, snapshots.",
          ),
        );
      return;
    }
    if (!auditTableBody || auditTableBody.parentNode?.parentNode?.parentNode !== auditListEl) {
      wipe(auditListEl);
      const table = el("table", { class: "table compact" });
      table.appendChild(
        el(
          "thead",
          {},
          el(
            "tr",
            {},
            el("th", { text: "When" }),
            el("th", { text: "Event" }),
            el("th", { text: "Scope" }),
          ),
        ),
      );
      auditTableBody = el("tbody", {});
      table.appendChild(auditTableBody);
      auditListEl.appendChild(table);
    }
    renderList(
      auditTableBody,
      auditEvents,
      (event) =>
        event.created_at_unix +
        ":" +
        event.kind +
        ":" +
        auditEvents.indexOf(event),
      (event, existing) => renderAuditRow(event, existing),
    );
    auditMoreEl.classList.toggle("hidden", auditEvents.length < auditLimit);
  }

  function renderAuditRow(event: AuditEvent, existing: HTMLElement | null): HTMLElement {
    const signature =
      event.created_at_unix + "|" + event.kind + "|" + (event.compartment_id ?? "g");
    if (existing && existing.dataset.signature === signature) return existing;
    const row = el(
      "tr",
      { attrs: { "data-vault": "audit-row" } },
      el(
        "td",
        { class: "nums" },
        el("span", { text: formatTimestamp(event.created_at_unix) + " " }),
        rawDetails("raw", String(event.created_at_unix)),
      ),
      el("td", { text: formatAuditEvent(event) }),
      el("td", {
        text:
          event.compartment_id != null
            ? "compartment #" + event.compartment_id
            : "global",
      }),
    );
    row.dataset.signature = signature;
    // renderList only removes vanished keys: a shape change must retire
    // the old node itself or it lingers as a zombie next to the fresh one.
    existing?.remove();
    return row;
  }

  async function loadAudit(): Promise<void> {
    try {
      const response = await runtime.api.listAudit({
        limit: auditLimit,
        ...(auditKind ? { kind: auditKind } : {}),
      });
      auditEvents = response.events ?? [];
      clearFailure("audit trail");
    } catch (error) {
      fail("audit trail", error);
    }
    renderAudit();
  }

  // ── Diagnostics + self-check (spec item g) ───────────────────────

  function diagRow(label: string, value: string, raw?: string): HTMLElement {
    return el(
      "tr",
      {},
      el("td", { text: label }),
      el(
        "td",
        { class: "nums" },
        el("span", { text: value }),
        raw !== undefined && raw !== value
          ? el("span", { text: " " }, rawDetails("raw", raw))
          : null,
      ),
    );
  }

  function diagGroup(title: string, rows: HTMLElement[]): HTMLElement {
    return el(
      "div",
      { class: "vault-diag-group" },
      el("h4", { class: "vault-diag-title", text: title }),
      el(
        "table",
        { class: "table compact" },
        el("tbody", {}, ...rows),
      ),
    );
  }

  function renderDiagnostics(): void {
    if (!diagEl) return;
    wipe(diagEl);
    if (!status()) {
      diagEl.appendChild(skeletonBlock());
      return;
    }
    if (isLocked()) {
      diagEl.appendChild(lockedEmpty("read diagnostics"));
      return;
    }
    if (!diagnostics) {
      diagEl.appendChild(skeletonBlock());
      return;
    }
    const d = diagnostics;
    const policy = d.runtime_policy;
    const scheduler = d.scheduler;

    diagEl.appendChild(
      diagGroup("Daemon", [
        diagRow("Version", d.version || "-"),
        diagRow("Started", formatTimestamp(d.started_at_unix), String(d.started_at_unix)),
        diagRow("Initialized", yesNo(d.initialized)),
        diagRow("Unlock scope", humanizeEnum(d.unlock_scope)),
        diagRow("Session scope", humanizeEnum(d.session_scope)),
        diagRow("Active sessions", String(d.active_session_count ?? 0)),
        diagRow("Unlocked compartments", String(d.unlocked_compartment_count ?? 0)),
        diagRow(
          "Max unlock threshold",
          d.max_unlocked_threshold != null ? String(d.max_unlocked_threshold) : "-",
        ),
        diagRow("Audit log present", yesNo(d.audit_log_present)),
      ]),
    );

    diagEl.appendChild(
      diagGroup("Queue", [
        diagRow("Jobs", String(d.queue_job_count ?? 0)),
        diagRow("Blocked", String(d.blocked_queue_job_count ?? 0)),
        diagRow("Retrying", String(d.retrying_queue_job_count ?? 0)),
        diagRow("Failed", String(d.failed_queue_job_count ?? 0)),
        diagRow(
          "Needs operator action",
          String(d.operator_action_required_queue_job_count ?? 0),
        ),
        diagRow("Deferred", String(d.deferred_queue_job_count ?? 0)),
        diagRow("Recovered at startup", String(d.startup_recovered_queue_job_count ?? 0)),
      ]),
    );

    diagEl.appendChild(
      diagGroup("Operations & deposits", [
        diagRow("Pending operations", String(d.pending_operation_count ?? 0)),
        diagRow("Interrupted at startup", String(d.startup_interrupted_operation_count ?? 0)),
        diagRow("Recovered at startup", String(d.startup_recovered_operation_count ?? 0)),
        diagRow("Unresolved at startup", String(d.startup_unresolved_operation_count ?? 0)),
        diagRow("Stealth deposits", String(d.eth_stealth_deposit_count ?? 0)),
        diagRow("Funded deposits", String(d.funded_eth_stealth_deposit_count ?? 0)),
        diagRow(
          "Reconciled at startup",
          String(d.startup_reconciled_deposit_count ?? 0),
        ),
      ]),
    );

    diagEl.appendChild(
      diagGroup("Runtime policy", [
        diagRow(
          "Queue process limit",
          (policy?.queue_default_process_limit ?? "-") +
            " of " +
            (policy?.queue_max_process_limit ?? "-") +
            " max",
        ),
        diagRow(
          "Deposit refresh limit",
          (policy?.deposit_default_refresh_limit ?? "-") +
            " of " +
            (policy?.deposit_max_refresh_limit ?? "-") +
            " max",
        ),
        diagRow(
          "Queue retry backoff",
          durationLabel(policy?.queue_retry_base_delay_secs) +
            " up to " +
            durationLabel(policy?.queue_retry_max_delay_secs),
        ),
        diagRow(
          "RPC concurrency",
          String(policy?.provider_balance_observation_concurrency ?? "-"),
        ),
        diagRow(
          "Audit page limit",
          (policy?.audit_default_limit ?? "-") +
            " of " +
            (policy?.audit_max_limit ?? "-") +
            " max",
        ),
        diagRow("Idle auto-lock", durationLabel(policy?.idle_lock_secs)),
        diagRow("Idle-lock drain", durationLabel(policy?.idle_lock_drain_secs)),
        diagRow(
          "Idle-lock force after",
          durationLabel(policy?.idle_lock_force_after_secs),
        ),
      ]),
    );

    diagEl.appendChild(
      diagGroup("Scheduler", [
        diagRow("Enabled", yesNo(scheduler?.enabled)),
        diagRow("Tick", durationLabel(scheduler?.queue_tick_secs)),
        diagRow(
          "Last cycle",
          scheduler?.last_cycle_outcome
            ? humanizeEnum(scheduler.last_cycle_outcome)
            : "not run yet",
        ),
        diagRow("Consecutive failures", String(scheduler?.consecutive_failures ?? 0)),
        diagRow("Due queue jobs", String(scheduler?.due_queue_job_count ?? 0)),
        diagRow(
          "Next retry",
          scheduler?.next_retry_at_unix
            ? formatTimestamp(scheduler.next_retry_at_unix)
            : "-",
          scheduler?.next_retry_at_unix ? String(scheduler.next_retry_at_unix) : undefined,
        ),
      ]),
    );
  }

  async function loadDiagnostics(): Promise<void> {
    try {
      diagnostics = await runtime.api.getDiagnostics();
      clearFailure("diagnostics");
    } catch (error) {
      fail("diagnostics", error);
    }
    renderDiagnostics();
    renderCountdown();
  }

  function renderSelfCheck(): void {
    if (!selfCheckEl) return;
    wipe(selfCheckEl);
    if (!status()) {
      selfCheckEl.appendChild(skeletonBlock());
      return;
    }
    if (isLocked()) {
      selfCheckEl.appendChild(lockedEmpty("run the self-check"));
      return;
    }
    if (!selfCheck) {
      selfCheckEl.appendChild(
        sectionEmpty(
          "Not run yet in this session",
          "Self-check verifies every configured input live: providers answer RPC on the right chain, wallets re-derive, policy and allocations stay consistent.",
          "Run Self-Check",
          () => void runSelfCheckNow(),
        ),
      );
      return;
    }
    const counts = { pass: 0, warn: 0, fail: 0 };
    for (const check of selfCheck.checks ?? []) {
      if (check.status === "pass") counts.pass += 1;
      else if (check.status === "warn") counts.warn += 1;
      else counts.fail += 1;
    }
    const summary = el(
      "p",
      { class: "vault-selfcheck-summary", attrs: { "data-vault": "selfcheck-summary" } },
      el("span", { class: "pill pill-good", text: counts.pass + " pass" }),
      " ",
      el("span", { class: "pill pill-warn", text: counts.warn + " warn" }),
      " ",
      el("span", { class: "pill pill-danger", text: counts.fail + " fail" }),
      " · ran " + relativeAge(selfCheck.generated_at_unix),
    );
    selfCheckEl.appendChild(summary);

    // pass/warn/fail groupings with expandable raw rows (not 30 raw tiles).
    const groups: Array<{ status: "fail" | "warn" | "pass"; title: string; open: boolean }> = [
      { status: "fail", title: "Failing", open: true },
      { status: "warn", title: "Warnings", open: true },
      { status: "pass", title: "Passing", open: false },
    ];
    for (const group of groups) {
      const checks = (selfCheck.checks ?? []).filter(
        (check) => check.status === group.status,
      );
      if (!checks.length) continue;
      const details = el("details", {
        class: "vault-check-group",
        attrs: { "data-vault": "selfcheck-" + group.status },
      }) as HTMLDetailsElement;
      if (group.open) details.setAttribute("open", "");
      details.appendChild(
        el("summary", {
          text: group.title + " (" + checks.length + ")",
        }),
      );
      const list = el("div", { class: "vault-item-list" });
      details.appendChild(list);
      renderList(
        list,
        checks,
        (check) => check.id,
        (check, existing) => renderSelfCheckRow(check, existing),
      );
      selfCheckEl.appendChild(details);
    }
  }

  function renderSelfCheckRow(
    check: SelfCheckResult,
    existing: HTMLElement | null,
  ): HTMLElement {
    const signature =
      check.id + "|" + check.status + "|" + check.detail + "|" + (check.latency_ms ?? "-");
    if (existing && existing.dataset.signature === signature) return existing;
    const row = el(
      "div",
      { class: "vault-item", attrs: { "data-vault": "selfcheck-row" } },
      el(
        "div",
        { class: "vault-item-main" },
        el(
          "p",
          { class: "vault-item-title" },
          el("span", { text: check.subject + " " }),
          el("span", {
            class:
              "pill " +
              (check.status === "pass"
                ? "pill-good"
                : check.status === "warn"
                  ? "pill-warn"
                  : "pill-danger"),
            text: check.status,
          }),
        ),
        el("p", {
          class: "vault-item-body",
          text:
            humanizeEnum(check.domain) +
            " · " +
            check.detail +
            (check.latency_ms != null ? " · " + check.latency_ms + "ms" : ""),
        }),
      ),
    );
    row.dataset.signature = signature;
    // renderList only removes vanished keys: a shape change must retire
    // the old node itself or it lingers as a zombie next to the fresh one.
    existing?.remove();
    return row;
  }

  async function runSelfCheckNow(button?: HTMLButtonElement): Promise<void> {
    if (button) setBusy(button, true, "Running…");
    try {
      selfCheck = await runtime.api.runSelfCheck();
      clearFailure("self-check");
      renderSelfCheck();
      const failing = (selfCheck.checks ?? []).filter(
        (check) => check.status !== "pass",
      ).length;
      flash(
        failing
          ? "Self-check found " + failing + " issue(s) — see the groups below."
          : "Self-check passed: every configured input verified.",
        failing ? "review" : undefined,
      );
    } catch (error) {
      fail("self-check", error);
      flash(fieldErrors(error, {}), "danger");
    } finally {
      if (button) setBusy(button, false);
    }
  }

  // ── Static shell (built once per mount) ──────────────────────────

  function sectionShell(
    vaultId: string,
    title: string,
    help: string,
    actions?: HTMLElement[],
  ): { section: HTMLElement; body: HTMLElement } {
    const body = el("div", { attrs: { "data-vault": vaultId } });
    const section = el(
      "section",
      { class: "vault-section" },
      el(
        "div",
        { class: "vault-section-head" },
        el(
          "div",
          {},
          el("h3", { class: "vault-section-title", text: title }),
          el("p", { class: "helper-text", text: help }),
        ),
        actions?.length
          ? el("div", { class: "vault-section-actions" }, ...actions)
          : null,
      ),
      body,
    );
    return { section, body };
  }

  function secretKindShell(
    config: SecretKindConfig,
    title: string,
    help: string,
  ): HTMLElement {
    const prefix = config === SECRET_KIND ? "secret" : "apikey";
    const nameInput = el("input", {
      attrs: {
        type: "text",
        placeholder: config.noun === "secret" ? "Secret name" : "Key name",
        "aria-label": config.noun === "secret" ? "Secret name" : "Key name",
        autocomplete: "off",
        "data-vault": prefix + "-name",
      },
    }) as HTMLInputElement;
    const valueInput = el("input", {
      attrs: {
        type: "password",
        placeholder: "Value",
        "aria-label": "Value",
        autocomplete: "off",
        "data-vault": prefix + "-value",
      },
    }) as HTMLInputElement;
    const storeButton = el("button", {
      class: "btn-primary",
      text: config.noun === "secret" ? "Store secret" : "Store key",
      attrs: { type: "submit" },
    }) as HTMLButtonElement;
    const form = el(
      "form",
      {
        class: "form-row",
        attrs: { "data-vault": prefix + "-form" },
        on: {
          submit: (event) => {
            event.preventDefault?.();
            void setSecret(config, nameInput, valueInput, storeButton);
          },
        },
      },
      nameInput,
      valueInput,
      storeButton,
    );
    const { section, body } = sectionShell(
      config === SECRET_KIND ? "secrets" : "apikeys",
      title,
      help,
    );
    section.insertBefore(form, body);
    if (config === SECRET_KIND) secretListEl = body;
    else apiKeyListEl = body;
    return section;
  }

  function buildRoot(): HTMLElement {
    const fragment = el("div", { class: "vault-dest", attrs: { "data-vault": "root" } });

    // Page header: the question this destination answers.
    fragment.appendChild(
      el(
        "div",
        { class: "page-header" },
        el(
          "div",
          {},
          el("h2", { class: "page-header-title", text: "Vault" }),
          el("p", {
            class: "page-header-summary",
            text:
              "What protects this machine? Lock state, compartments, secrets, hardware keys, snapshots, and the audit trail — one security story.",
          }),
        ),
      ),
    );

    bannerEl = el("div", {
      class: "vault-banner",
      attrs: { "data-vault": "banner", role: "alert" },
    });
    bannerEl.classList.add("hidden");
    bannerEl.dataset.tier = "review";
    fragment.appendChild(bannerEl);

    flashEl = el("p", {
      class: "vault-flash",
      attrs: { "data-vault": "flash", role: "status", "aria-live": "polite" },
    });
    fragment.appendChild(flashEl);

    // (a) Session & lock state.
    {
      const { section, body } = sectionShell(
        "session",
        "Session & lock state",
        "Lock state is always visible here. Locking zeroizes master keys from daemon memory; logging out only clears this browser's session token.",
      );
      sessionEl = body;
      fragment.appendChild(section);
    }

    // (b) Compartments.
    {
      const addLabel = el("input", {
        attrs: {
          type: "text",
          placeholder: "Compartment name",
          "aria-label": "Compartment name",
          autocomplete: "off",
        },
      }) as HTMLInputElement;
      const addThreshold = el("input", {
        attrs: {
          type: "number",
          placeholder: "Keys required (threshold)",
          "aria-label": "Keys required (threshold)",
          min: "1",
        },
      }) as HTMLInputElement;
      const addMode = el(
        "select",
        { class: "input-wide", attrs: { "aria-label": "Passphrase mode" } },
        el("option", { text: "Passphrase allowed", attrs: { value: "" } }),
        el("option", { text: "Passphrase required", attrs: { value: "required" } }),
        el("option", { text: "Hardware keys only", attrs: { value: "disabled" } }),
      ) as HTMLSelectElement;
      const addButton = el("button", {
        class: "btn-primary",
        text: "Add compartment",
        attrs: { type: "submit" },
      }) as HTMLButtonElement;
      const addForm = el(
        "form",
        {
          class: "vault-form",
          attrs: { "data-vault": "compartment-add-form" },
          on: {
            submit: (event) => {
              event.preventDefault?.();
              void addCompartment(
                { label: addLabel, threshold: addThreshold, mode: addMode },
                addButton,
              );
            },
          },
        },
        el(
          "p",
          {
            class: "helper-text",
            text:
              "A compartment is an isolated vault space with its own unlock requirements. The threshold is how many distinct hardware keys unlock it.",
          },
        ),
        el("div", { class: "form-row" }, addLabel, addThreshold),
        el("div", { class: "form-row" }, addMode, addButton),
      );
      const wizard = el(
        "details",
        { class: "vault-disclosure" },
        el("summary", { text: "Add a compartment" }),
        addForm,
      );
      const { section, body } = sectionShell(
        "compartments",
        "Compartments",
        "Isolated vault spaces available in this session. Switching changes which compartment new operations target.",
      );
      compartmentListEl = body;
      section.appendChild(wizard);
      fragment.appendChild(section);
    }

    // (c) Secrets + connection keys + push.
    fragment.appendChild(
      secretKindShell(
        SECRET_KIND,
        "Encrypted secrets",
        "Values encrypted at rest; viewing or changing them requires an unlocked compartment. Revealed values hide again after 30 seconds.",
      ),
    );
    {
      const { section, body } = sectionShell(
        "push",
        "Move a secret between compartments",
        "Copy a secret from one compartment to another; the copy is indistinguishable from manual entry.",
      );
      pushWrapEl = body;
      fragment.appendChild(section);
    }
    fragment.appendChild(
      secretKindShell(
        API_KEY_KIND,
        "Connection keys",
        "RPC tokens and similar operational keys the daemon uses directly during workflows.",
      ),
    );

    // (d) Hardware keys.
    {
      const newPin = el("input", {
        attrs: {
          type: "password",
          placeholder: "New PIN for inserted key (min 4 chars)",
          "aria-label": "New PIN for inserted key",
          autocomplete: "off",
        },
      }) as HTMLInputElement;
      const newPinConfirm = el("input", {
        attrs: {
          type: "password",
          placeholder: "Confirm new PIN",
          "aria-label": "Confirm new PIN",
          autocomplete: "off",
        },
      }) as HTMLInputElement;
      const setPinButton = el("button", {
        class: "btn-ghost",
        text: "Set key PIN",
        attrs: { type: "submit" },
      }) as HTMLButtonElement;
      const setPinForm = el(
        "form",
        {
          class: "form-row",
          attrs: { "data-vault": "fido2-setpin-form" },
          on: {
            submit: (event) => {
              event.preventDefault?.();
              void setFido2Pin(newPin, newPinConfirm, setPinButton);
            },
          },
        },
        newPin,
        newPinConfirm,
        setPinButton,
      );

      const regPin = el("input", {
        attrs: {
          type: "password",
          placeholder: "Current FIDO2 PIN if this key requires one",
          "aria-label": "Current FIDO2 PIN if this key requires one",
          autocomplete: "off",
        },
      }) as HTMLInputElement;
      const regLabel = el("input", {
        attrs: {
          type: "text",
          placeholder: "Key label",
          "aria-label": "Key label",
          autocomplete: "off",
        },
      }) as HTMLInputElement;
      const regButton = el("button", {
        class: "btn-primary",
        text: "Register key",
        attrs: { type: "submit" },
      }) as HTMLButtonElement;
      const poison = el("input", {
        attrs: { type: "checkbox", "data-vault": "poison" },
      }) as HTMLInputElement;
      const poisonWarning = el("div", {
        class: "poison-warning",
        text: "This key will contain RANDOM shard data. Including it during unlock causes silent failure. No data is destroyed. Exclude it and retry with real keys to unlock normally.",
      });
      poisonWarning.classList.add("hidden");
      poison.addEventListener("change", () => {
        poisonWarning.classList.toggle("hidden", !poison.checked);
      });
      const skip = el("input", {
        attrs: {
          type: "text",
          placeholder: "Skip keys (comma-separated labels)",
          "aria-label": "Skip keys (comma-separated labels)",
          autocomplete: "off",
        },
      }) as HTMLInputElement;
      const registerForm = el(
        "form",
        {
          class: "vault-form",
          attrs: { "data-vault": "fido2-register-form" },
          on: {
            submit: (event) => {
              event.preventDefault?.();
              void registerFido2Key({
                pin: regPin,
                label: regLabel,
                poison,
                skip,
                button: regButton,
              });
            },
          },
        },
        el("div", { class: "form-row" }, regPin, regLabel, regButton),
        el(
          "div",
          { class: "form-row-center" },
          el(
            "label",
            { class: "checkbox-label" },
            poison,
            el("span", { text: " Poison key (duress)" }),
          ),
          skip,
        ),
        poisonWarning,
      );

      const registerDetails = el(
        "details",
        { class: "vault-disclosure" },
        el("summary", { text: "Register or re-PIN a hardware key" }),
        el(
          "p",
          {
            class: "helper-text",
            text:
              "Register another FIDO2 hardware key. All compartments must be unlocked first so Sigillum can safely reshare the vault material. Fresh backup keys may not have a FIDO2 PIN yet — touch-only enrollment works when the authenticator allows it.",
          },
        ),
        setPinForm,
        registerForm,
      );

      const { section, body } = sectionShell(
        "fido2",
        "Hardware keys",
        "FIDO2 keys enrolled for threshold unlock. Poison keys hold random shard data and cause silent unlock failure when included.",
      );
      fido2DetectEl = el("div", {});
      fido2KeyListEl = body;
      section.insertBefore(fido2DetectEl, body);
      section.appendChild(registerDetails);
      fragment.appendChild(section);
    }

    // (e) Snapshots.
    {
      const exportPass = el("input", {
        attrs: {
          type: "password",
          placeholder: "Export passphrase (min 8 chars)",
          "aria-label": "Export passphrase (min 8 chars)",
          autocomplete: "off",
          "data-vault": "export-pass",
        },
      }) as HTMLInputElement;
      const exportButton = el("button", {
        class: "btn-primary",
        text: "Export snapshot",
        attrs: { type: "submit", "data-vault": "export" },
      }) as HTMLButtonElement;
      const exportForm = el(
        "form",
        {
          class: "form-row",
          attrs: { "data-vault": "export-form" },
          on: {
            submit: (event) => {
              event.preventDefault?.();
              void exportSnapshot(exportPass, exportButton);
            },
          },
        },
        exportPass,
        exportButton,
      );
      const restoreFile = el("input", {
        class: "flex-1",
        attrs: { type: "file", "aria-label": "Snapshot file" },
      }) as HTMLInputElement;
      const restorePass = el("input", {
        class: "flex-1",
        attrs: {
          type: "password",
          placeholder: "Restore passphrase",
          "aria-label": "Restore passphrase",
          autocomplete: "off",
        },
      }) as HTMLInputElement;
      const restoreButton = el("button", {
        class: "btn-danger",
        text: "Restore snapshot",
        attrs: { type: "submit", "data-vault": "restore" },
      }) as HTMLButtonElement;
      const restoreForm = el(
        "form",
        {
          class: "form-row",
          attrs: { "data-vault": "restore-form" },
          on: {
            submit: (event) => {
              event.preventDefault?.();
              void restoreSnapshot(restoreFile, restorePass, restoreButton);
            },
          },
        },
        restoreFile,
        restorePass,
        restoreButton,
      );
      const resetButton = el("button", {
        class: "btn-danger",
        text: "Reset local data",
        attrs: { type: "button", "data-vault": "reset" },
      }) as HTMLButtonElement;
      resetButton.addEventListener("click", () => void resetLocalData(resetButton));

      const { section } = sectionShell(
        "snapshots",
        "Encrypted snapshots",
        "Export the local data directory as a passphrase-encrypted snapshot, or restore one. Restoring replaces on-disk state and logs you out.",
      );
      backupNoteEl = el("div", {});
      section.appendChild(backupNoteEl);
      section.appendChild(exportForm);
      section.appendChild(restoreForm);
      section.appendChild(
        el("p", {
          class: "card-note",
          text: "Exports create an encrypted file you can store elsewhere. Restores require a fresh unlock afterward.",
        }),
      );
      section.appendChild(
        el(
          "div",
          { class: "vault-danger-zone" },
          el("h4", { class: "vault-diag-title", text: "Start over on this machine" }),
          el("p", {
            class: "helper-text",
            text: "Wipe the local Sigillum data directory and return this daemon to first-run setup. The data directory is archived, not deleted.",
          }),
          resetButton,
        ),
      );
      fragment.appendChild(section);
    }

    // (f) Audit trail.
    {
      const kindSelect = el(
        "select",
        {
          class: "input-wide",
          attrs: { "aria-label": "Filter by event kind", "data-vault": "audit-kind" },
        },
        el("option", { text: "All event kinds", attrs: { value: "" } }),
        ...Object.keys(AUDIT_KIND_LABELS).map((kind) =>
          el("option", { text: AUDIT_KIND_LABELS[kind], attrs: { value: kind } }),
        ),
      ) as HTMLSelectElement;
      const filterForm = el(
        "form",
        {
          class: "form-row",
          attrs: { "data-vault": "audit-filter-form" },
          on: {
            submit: (event) => {
              event.preventDefault?.();
              auditKind = kindSelect.value;
              auditLimit = AUDIT_PAGE;
              auditEvents = null;
              renderAudit();
              void loadAudit();
            },
          },
        },
        kindSelect,
        el("button", {
          class: "btn-ghost",
          text: "Apply filter",
          attrs: { type: "submit" },
        }),
      );
      const { section, body } = sectionShell(
        "audit",
        "Audit trail",
        "Recent local audit events from this daemon process and its persisted audit log.",
      );
      auditListEl = body;
      auditMoreEl = el("div", {});
      auditMoreEl.classList.add("hidden");
      const moreButton = el("button", {
        class: "btn-ghost btn-small",
        text: "Show more",
        attrs: { type: "button", "data-vault": "audit-more" },
      }) as HTMLButtonElement;
      moreButton.addEventListener("click", () => {
        auditLimit += AUDIT_PAGE;
        void loadAudit();
      });
      auditMoreEl.appendChild(moreButton);
      section.insertBefore(filterForm, body);
      section.appendChild(auditMoreEl);
      fragment.appendChild(section);
    }

    // (g) Diagnostics + self-check.
    {
      const runButton = el("button", {
        class: "btn-ghost btn-small",
        text: "Run Self-Check",
        attrs: { type: "button", "data-vault": "run-selfcheck" },
      }) as HTMLButtonElement;
      runButton.addEventListener("click", () => void runSelfCheckNow(runButton));
      const { section, body } = sectionShell(
        "diagnostics",
        "Diagnostics",
        "Daemon health grouped by area: daemon, queue, operations, runtime policy, and scheduler.",
        [runButton],
      );
      diagEl = body;
      selfCheckEl = el("div", { attrs: { "data-vault": "selfcheck" } });
      section.appendChild(
        el(
          "div",
          { class: "vault-selfcheck" },
          el("h4", { class: "vault-diag-title", text: "Self-Check" }),
          selfCheckEl,
        ),
      );
      fragment.appendChild(section);
    }

    return fragment;
  }

  // ── Refresh orchestration ────────────────────────────────────────

  /** Re-render every section from current state (keyed lists patch in place). */
  function renderAll(): void {
    renderSession();
    renderCompartments();
    renderSecretKind(SECRET_KIND);
    renderSecretKind(API_KEY_KIND);
    renderPush();
    renderFido2();
    renderBackupNote();
    renderAudit();
    renderDiagnostics();
    renderSelfCheck();
  }

  async function ensureStatus(): Promise<void> {
    if (runtime.store.get("status")) return;
    try {
      runtime.store.set("status", await runtime.api.getStatus());
    } catch (error) {
      fail("status", error);
    }
  }

  async function refreshAll(): Promise<void> {
    await ensureStatus();
    renderAll();
    if (isLocked()) return;
    await Promise.all([
      loadCompartments(),
      loadSecretKind(SECRET_KIND),
      loadSecretKind(API_KEY_KIND),
      loadFido2(),
      loadBackupState(),
      loadAudit(),
      loadDiagnostics(),
    ]);
  }

  // ── Mount / unmount ──────────────────────────────────────────────

  function onActivity(): void {
    lastActivityAt = Date.now();
  }

  function mount(_route: Route): void {
    const target = document.getElementById(HOST_CARD_ID);
    if (!target) return;
    host = target;
    mounted = true;
    hostWasHidden = host.classList.contains("hidden");
    hostHadHiddenAttribute = host.hasAttribute("hidden");

    // Take over the host card: stash legacy children (restored on unmount).
    stashedChildren = Array.from(host.childNodes) as unknown as ChildNode[];
    for (const child of stashedChildren) child.remove();
    host.classList.add("vault-host");

    // Hide the remaining legacy vault cards; record prior state for restore.
    for (const id of LEGACY_VAULT_SIBLING_IDS) {
      const card = document.getElementById(id);
      if (!card) continue;
      siblingWasHidden.set(card, card.classList.contains("hidden"));
      siblingHadHiddenAttribute.set(card, card.hasAttribute("hidden"));
      card.classList.add("hidden");
      card.setAttribute("hidden", "");
    }

    root = buildRoot();
    host.appendChild(root);
    assertHostOwnership();

    // Live updates from the store: lock/compartment changes re-render every
    // section (locked placeholders swap in/out); resync (SSE snapshot)
    // refetches everything; the sync slice fires after every legacy refresh
    // cycle, which re-shows the sibling cards — re-assert ownership.
    unsubs.push(
      runtime.store.subscribe("status", (next) => {
        // A lock must not leave revealed values on screen (or in the map).
        if (next?.locked) revealed.clear();
        renderAll();
      }),
      runtime.store.subscribe("resync", () => {
        void refreshAll();
      }),
      runtime.store.subscribe("sync", () => {
        assertHostOwnership();
      }),
    );

    document.addEventListener("click", onActivity);
    document.addEventListener("keydown", onActivity);
    countdownInterval = setInterval(renderCountdown, 1000);

    renderSession();
    renderBanner();
    void refreshAll();
  }

  function unmount(): void {
    if (!mounted) return;
    mounted = false;
    for (const unsubscribe of unsubs.splice(0)) unsubscribe();
    for (const handle of Array.from(timers)) clearTimeout(handle);
    timers.clear();
    if (countdownInterval !== null) {
      clearInterval(countdownInterval);
      countdownInterval = null;
    }
    document.removeEventListener("click", onActivity);
    document.removeEventListener("keydown", onActivity);

    if (host) {
      root?.remove();
      root = null;
      for (const child of stashedChildren) {
        host.appendChild(child);
      }
      stashedChildren = [];
      host.classList.remove("vault-host");
      host.classList.toggle("hidden", hostWasHidden);
      if (hostHadHiddenAttribute) host.setAttribute("hidden", "");
      else host.removeAttribute("hidden");
    }
    releaseHostOwnership();
    host = null;

    // Reset resource state so a remount starts from skeletons.
    compartments = null;
    secretKeys = null;
    apiKeys = null;
    fido2Detect = null;
    fido2Keys = null;
    diagnostics = null;
    auditEvents = null;
    auditLimit = AUDIT_PAGE;
    auditKind = "";
    selfCheck = null;
    lastSnapshotAtUnix = null;
    backupChecked = false;
    failures.clear();
    revealed.clear();
    compartmentListInner = null;
    secretListInner.secret = null;
    secretListInner["connection key"] = null;
    fido2ListInner = null;
    auditTableBody = null;
  }

  return {
    id: "vault",
    migrated: true,
    mount,
    unmount,
  };
}
