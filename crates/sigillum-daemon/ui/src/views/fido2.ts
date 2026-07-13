import { clearFields } from "../render/forms";
import { isSessionContextChangedError } from "../api/session";
import { setInlineInfoById, setTextById } from "../render/dom";
import { esc, escAttr } from "../render/html";

export interface Fido2State {
  detect: any | null;
  keys: any[];
}

export interface Fido2Deps {
  api: (method: string, path: string, body?: unknown) => Promise<any>;
  toast: (message: string, type?: string) => void;
  refresh: () => unknown;
  currentStatus: () => any;
}

export function applyUnlockTabKeydown(
  event: Pick<KeyboardEvent, "key" | "target" | "preventDefault">,
  tabs: HTMLElement[],
  activate: (tab: string) => void,
): boolean {
  const currentIndex = tabs.indexOf(event.target as HTMLElement);
  if (currentIndex < 0 || tabs.length === 0) return false;

  let nextIndex: number;
  switch (event.key) {
    case "ArrowRight":
      nextIndex = (currentIndex + 1) % tabs.length;
      break;
    case "ArrowLeft":
      nextIndex = (currentIndex - 1 + tabs.length) % tabs.length;
      break;
    case "Home":
      nextIndex = 0;
      break;
    case "End":
      nextIndex = tabs.length - 1;
      break;
    default:
      return false;
  }

  const target = tabs[nextIndex];
  const tab = target?.dataset.arg0;
  if (!target || !tab) return false;

  event.preventDefault();
  activate(tab);
  target.focus();
  return true;
}

function input(id: string): HTMLInputElement {
  return document.getElementById(id) as HTMLInputElement;
}

export function friendlyFidoError(message: unknown): string {
  const raw = String(message || "");
  const normalized = raw.toLowerCase();

  if (
    normalized.includes("pin_required") ||
    normalized.includes("pin is required for the selected operation")
  ) {
    return "This hardware key is configured to require its current FIDO2 PIN. Enter that PIN and retry, or use a touch-only key.";
  }
  if (
    normalized.includes("pin_not_set") ||
    normalized.includes("no pin has been set") ||
    normalized.includes("no fido2 pin is configured")
  ) {
    return "This hardware key does not have a FIDO2 PIN yet. Leave the PIN field empty to keep using touch-only access, or use Set key PIN first if you want this key to require one.";
  }
  if (
    normalized.includes("pin already set") ||
    normalized.includes("already has a fido2 pin") ||
    normalized.includes("already has a pin configured")
  ) {
    return "This hardware key already has a FIDO2 PIN. Enter the existing PIN when you use this key.";
  }
  if (
    normalized.includes("pin_policy") ||
    normalized.includes("pin policy") ||
    normalized.includes("at least 4 characters")
  ) {
    return "That new FIDO2 PIN does not meet the key policy. Use at least 4 characters and avoid unsupported patterns.";
  }
  if (
    normalized.includes("pin_auth_blocked") ||
    normalized.includes("pin authentication is temporarily blocked") ||
    normalized.includes("power recycle")
  ) {
    return "This hardware key has temporarily blocked PIN authentication. Unplug and reinsert the key, then retry with the correct PIN.";
  }
  if (
    normalized.includes("pin_blocked") ||
    normalized.includes("pin is fully blocked") ||
    normalized.includes("fully blocked on the hardware key")
  ) {
    return "This hardware key has fully blocked PIN attempts. Recover or reset the key with vendor tooling before trying again.";
  }
  if (normalized.includes("incorrect pin") || normalized.includes("pin_invalid")) {
    return "The hardware key rejected that PIN. Re-enter the current PIN carefully, or use a touch-only key if that is the policy you want.";
  }
  if (
    normalized.includes("cannot tell which one to use") ||
    normalized.includes("leave only the target key inserted")
  ) {
    return "More than one hardware key is attached and this step needs a specific target. Leave only the key you want Sigillum to act on, then retry.";
  }
  if (
    normalized.includes("already appears to be registered") ||
    normalized.includes("insert the new key you want to add")
  ) {
    return "The attached hardware keys all look like ones Sigillum already knows. Insert the new key you want to add, then retry.";
  }
  if (
    normalized.includes("matched the sigillum credential needed for this step") ||
    normalized.includes("matched the expected sigillum credential")
  ) {
    return "The attached hardware keys do not include the enrolled key Sigillum expected for this step. Keep the required registered key connected and retry.";
  }
  if (normalized.includes("no fido2 device") || normalized.includes("no device")) {
    return "No FIDO2 hardware key is currently available. Insert the key and try again.";
  }
  if (normalized.includes("timeout")) {
    return "Sigillum timed out while waiting for the hardware key. Keep it connected and touch it when prompted, then try again.";
  }
  if (normalized.includes("ctap1 device") || normalized.includes("hmac-secret")) {
    return "This key does not support the FIDO2 features Sigillum requires. Use a CTAP2 key with hmac-secret support.";
  }
  if (normalized.includes("clientpin support")) {
    return "This key does not expose PIN management in a way Sigillum can use. Set the PIN with the vendor tool, then return here.";
  }
  return raw;
}

function isAlreadyUnlockedConflict(message: unknown): boolean {
  return String(message || "").toLowerCase().includes("already unlocked");
}

interface PinPromptResult {
  submitted: boolean;
  pin: string | null;
}

function promptPin(
  msg: string,
  registerCancel?: (cancel: (() => void) | null) => void,
): Promise<PinPromptResult> {
  return new Promise((resolve) => {
    const previouslyFocused = document.activeElement as HTMLElement | null;
    const backgroundState = Array.from(document.body.children).map((element) => ({
      element,
      inert: element.getAttribute("inert"),
      ariaHidden: element.getAttribute("aria-hidden"),
    }));
    const previousBodyOverflow = document.body.style.overflow;
    const overlay = document.createElement("div");
    overlay.style.cssText =
      "position:fixed;inset:0;background:rgba(0,0,0,0.7);z-index:200;display:flex;align-items:center;justify-content:center;";
    overlay.innerHTML =
      '<div class="card pin-modal" role="dialog" aria-modal="true" aria-labelledby="pinModalTitle"><h2 id="pinModalTitle">' +
      esc(msg) +
      '</h2><div class="form-row"><input type="password" id="pinModalInput" aria-label="Current FIDO2 PIN" autocomplete="off" placeholder="Current PIN (leave blank if not required)">' +
      '<button type="button" class="btn-primary" id="pinModalOk">OK</button></div></div>';
    document.body.appendChild(overlay);
    const inp = input("pinModalInput");
    const ok = document.getElementById("pinModalOk") as HTMLButtonElement | null;
    let settled = false;

    backgroundState.forEach(({ element }) => {
      element.setAttribute("inert", "");
      element.setAttribute("aria-hidden", "true");
    });
    document.body.style.overflow = "hidden";
    inp.focus();

    const restoreBackground = () => {
      backgroundState.forEach(({ element, inert, ariaHidden }) => {
        if (inert == null) element.removeAttribute("inert");
        else element.setAttribute("inert", inert);
        if (ariaHidden == null) element.removeAttribute("aria-hidden");
        else element.setAttribute("aria-hidden", ariaHidden);
      });
      document.body.style.overflow = previousBodyOverflow;
    };

    const restoreTriggerFocus = () => {
      if (
        !previouslyFocused ||
        previouslyFocused.isConnected === false ||
        typeof previouslyFocused.focus !== "function"
      ) {
        return;
      }
      try {
        previouslyFocused.focus({ preventScroll: true });
      } catch (_) {
        previouslyFocused.focus();
      }
    };

    const finish = (result: PinPromptResult) => {
      if (settled) return;
      settled = true;
      registerCancel?.(null);
      document.removeEventListener("keydown", handleDocumentKeydown, true);
      overlay.remove();
      restoreBackground();
      restoreTriggerFocus();
      resolve(result);
    };

    registerCancel?.(() => finish({ submitted: false, pin: null }));

    const submit = () => finish({ submitted: true, pin: inp.value || null });
    const focusable = [inp, ok].filter(
      (element): element is HTMLInputElement | HTMLButtonElement => Boolean(element),
    );

    function handleDocumentKeydown(event: KeyboardEvent): void {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        finish({ submitted: false, pin: null });
        return;
      }
      if (event.key !== "Tab" || focusable.length === 0) return;

      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      if (!focusable.includes(active as HTMLInputElement | HTMLButtonElement)) {
        event.preventDefault();
        first.focus();
      } else if (event.shiftKey && active === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    }

    document.addEventListener("keydown", handleDocumentKeydown, true);
    ok?.addEventListener("click", submit);
    inp.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        submit();
      }
    });
    overlay.addEventListener("click", (event) => {
      if (event.target === overlay) {
        finish({ submitted: false, pin: null });
      }
    });
  });
}

export function createFido2Actions(deps: Fido2Deps) {
  let lastFidoDetect: any | null = null;
  let lastFidoKeys: any[] = [];
  let cancelPinPrompt: (() => void) | null = null;
  let sessionGeneration = 0;

  function resetSession(): void {
    sessionGeneration += 1;
    cancelPinPrompt?.();
    cancelPinPrompt = null;
    lastFidoDetect = null;
    lastFidoKeys = [];
  }

  function setUnlockGuidance(mode: string): void {
    const el = document.getElementById("authGuidance");
    if (!el) return;

    if (mode === "session") {
      el.textContent =
        "Lock All zeroizes unlocked vault material from daemon memory. Log Out Session only clears the browser session token and leaves daemon state unchanged.";
      return;
    }

    if (mode === "fido2") {
      const deviceLine =
        lastFidoDetect && lastFidoDetect.device_present
          ? lastFidoDetect.device_count + " hardware key(s) detected. "
          : "";
      el.textContent =
        deviceLine +
        "Set tap count to the number of distinct key touches Sigillum should wait for. Leave the PIN field blank for touch-only authenticators, or enter the current PIN only for keys that require one. For a two-key compartment, enter 2 and touch two enrolled keys in sequence.";
      return;
    }

    el.textContent =
      "Passphrase unlock works for passphrase-only vaults or fallback access. A successful unlock reveals the protected workspace in this browser tab only.";
  }

  function switchUnlockTab(tab: string): void {
    const tabs = Array.from(document.querySelectorAll<HTMLElement>(".unlock-tab"));
    tabs.forEach((target) => {
      target.classList.remove("active");
      target.setAttribute("aria-selected", "false");
      target.setAttribute("tabindex", "-1");
    });
    const activate = (target: HTMLElement | undefined) => {
      target?.classList.add("active");
      target?.setAttribute("aria-selected", "true");
      target?.setAttribute("tabindex", "0");
    };
    if (tab === "fido2") {
      document.getElementById("unlockPassphrase")?.classList.add("hidden");
      document.getElementById("unlockFido2")?.classList.remove("hidden");
      activate(tabs[1]);
      setUnlockGuidance("fido2");
    } else {
      document.getElementById("unlockPassphrase")?.classList.remove("hidden");
      document.getElementById("unlockFido2")?.classList.add("hidden");
      activate(tabs[0]);
      setUnlockGuidance("passphrase");
    }
  }

  const unlockTabs = document.getElementById("unlockTabs");
  unlockTabs?.addEventListener("keydown", (event) => {
    const tabs = Array.from(
      unlockTabs.querySelectorAll<HTMLElement>("[role=\"tab\"]"),
    );
    applyUnlockTabKeydown(event, tabs, switchUnlockTab);
  });

  async function showUnlockTabs(): Promise<void> {
    try {
      const detect = await deps.api("GET", "/api/fido2/detect");
      const hasFido = detect.device_present;
      lastFidoDetect = detect;
      const tabs = document.getElementById("unlockTabs");
      const activeTab = document.getElementById("unlockFido2")?.classList.contains("hidden")
        ? "passphrase"
        : "fido2";
      if (hasFido) {
        tabs?.classList.remove("hidden");
        setTextById(
          "authLead",
          detect.device_count +
            " hardware key(s) detected. Passphrase unlock stays available, or switch to Hardware Key when you want threshold-based unlock.",
        );
        switchUnlockTab(activeTab);
      } else {
        tabs?.classList.add("hidden");
        setTextById(
          "authLead",
          "No FIDO2 device is currently detected. Unlock with a passphrase, or attach a hardware key and refresh to use threshold-based unlock.",
        );
        switchUnlockTab("passphrase");
      }
    } catch (error) {
      if (isSessionContextChangedError(error)) throw error;
      setTextById(
        "authLead",
        "Unlock with the passphrase you configured during setup. Hardware-key unlock becomes available when a FIDO2 device is detected.",
      );
      switchUnlockTab("passphrase");
    }
  }

  async function loadFido2(): Promise<void> {
    const fido2Card = document.getElementById("fido2Card");
    const status = deps.currentStatus();
    if (!status || status.locked) {
      fido2Card?.classList.add("hidden");
      return;
    }
    fido2Card?.classList.remove("hidden");

    const optionalRead = async (path: string): Promise<any | null> => {
      try {
        return await deps.api("GET", path);
      } catch (error) {
        if (isSessionContextChangedError(error)) throw error;
        return null;
      }
    };
    // Detect and list are one panel snapshot. Start both under the same
    // session context and do not commit either half unless both completed.
    const [detect, keys] = await Promise.all([
      optionalRead("/api/fido2/detect"),
      optionalRead("/api/fido2/list"),
    ]);
    if (!detect || detect.error || !keys || keys.error) return;

    lastFidoDetect = detect;
    lastFidoKeys = keys.keys || [];

    const devEl = document.getElementById("fido2DeviceStatus");
    if (detect.device_present) {
      if (devEl) {
        devEl.innerHTML =
          '<span style="color:var(--success);">' +
          detect.device_count +
          " FIDO2 device(s) connected</span>";
      }
    } else if (devEl) {
      devEl.innerHTML =
        '<span style="color:var(--warning);">No FIDO2 device detected.</span>';
    }

    const listEl = document.getElementById("fido2KeyListSection");
    if (lastFidoKeys.length > 0) {
      let html = '<ul class="key-list">';
      lastFidoKeys.forEach((key: any) => {
        html +=
          "<li><span>" +
          esc(key.label) +
          ' <span style="color:var(--text-dim);font-size:11px;">(' +
          esc(key.credential_id_short) +
          "...) " +
          esc(key.registered_at) +
          "</span></span>" +
          '<div class="key-actions"><button class="btn-danger" data-action="fido2RemoveKey" data-arg0="' +
          escAttr(key.label) +
          '">Remove</button></div></li>';
      });
      html += "</ul>";
      if (listEl) listEl.innerHTML = html;
    } else if (listEl) {
      listEl.innerHTML =
        '<p class="text-meta">No additional hardware keys are registered yet. Add one above to improve recovery and higher-threshold unlock paths.</p>';
    }
  }

  function togglePoisonWarning(): void {
    const checked = input("fido2Poison").checked;
    document.getElementById("fido2PoisonWarning")?.classList.toggle("hidden", !checked);
  }

  async function submitNewFido2Pin(
    pinId: string,
    confirmId: string,
    hintId: string,
    copyToId: string,
    focusId: string,
  ): Promise<void> {
    const pin = input(pinId).value;
    const confirmPin = input(confirmId).value;
    if (!pin) {
      deps.toast("New PIN required", "error");
      return;
    }
    if (pin.length < 4) {
      deps.toast("New PIN must be at least 4 characters", "error");
      return;
    }
    if (pin !== confirmPin) {
      deps.toast("PIN entries do not match", "error");
      return;
    }

    const r = await deps.api("POST", "/api/fido2/pin/set", { new_pin: pin });
    if (r.error) {
      const message = friendlyFidoError(r.error);
      if (hintId) setInlineInfoById(hintId, message);
      deps.toast(message, "error");
      return;
    }

    if (copyToId) {
      const target = document.getElementById(copyToId) as HTMLInputElement | null;
      if (target) target.value = pin;
    }
    clearFields([pinId, confirmId]);
    if (hintId) {
      setInlineInfoById(
        hintId,
        "FIDO2 PIN set on the inserted hardware key. Use that PIN in the registration field and continue.",
        "success",
      );
    }
    document.getElementById(focusId)?.focus();
    deps.toast("Hardware-key PIN set");
  }

  async function fido2SetNewPin(): Promise<void> {
    await submitNewFido2Pin(
      "fido2NewPin",
      "fido2NewPinConfirm",
      "fido2DeviceStatus",
      "fido2RegPin",
      "fido2RegLabel",
    );
  }

  async function fido2Register(): Promise<void> {
    const pin = input("fido2RegPin").value;
    const label = input("fido2RegLabel").value;
    const poison = input("fido2Poison").checked;
    const skipRaw = input("fido2SkipKeys").value.trim();
    const skipKeys = skipRaw ? skipRaw.split(",").map((s) => s.trim()).filter(Boolean) : [];
    if (!label) {
      deps.toast("Label required", "error");
      return;
    }
    if (
      poison &&
      !confirm(
        'Register "' +
          label +
          '" as a POISON key? Including it during unlock will cause silent failure.',
      )
    ) {
      return;
    }
    deps.toast("Touch your FIDO2 key now...");
    const body: any = { label };
    if (pin) body.pin = pin;
    if (poison) body.poison = true;
    if (skipKeys.length > 0) body.skip_keys = skipKeys;
    const r = await deps.api("POST", "/api/fido2/register", body);
    if (r.error) {
      const message = friendlyFidoError(r.error);
      setInlineInfoById("fido2DeviceStatus", message);
      deps.toast(message, "error");
      return;
    }
    clearFields(["fido2RegPin", "fido2RegLabel", "fido2SkipKeys"]);
    input("fido2Poison").checked = false;
    togglePoisonWarning();
    deps.toast('Key "' + label + '" registered' + (poison ? " (poison)" : ""));
    deps.refresh();
  }

  async function fido2RemoveKey(label: string): Promise<void> {
    if (!confirm('Remove FIDO2 key "' + label + '"?')) return;
    const generation = sessionGeneration;
    const pinPrompt = await promptPin(
      "Enter the current FIDO2 PIN only if the remaining keys require one:",
      (cancel) => { cancelPinPrompt = cancel; },
    );
    if (generation !== sessionGeneration || !pinPrompt.submitted) return;
    const body: any = { label };
    if (pinPrompt.pin) body.pin = pinPrompt.pin;
    const r = await deps.api("POST", "/api/fido2/remove", body);
    if (r.error) {
      const message = friendlyFidoError(r.error);
      setInlineInfoById("fido2DeviceStatus", message);
      deps.toast(message, "error");
      return;
    }
    deps.toast("Key removed");
    deps.refresh();
  }

  async function fido2Unlock(): Promise<void> {
    const pin = input("fido2Pin").value;
    const tapCount = parseInt(input("fido2TapCount").value);
    if (!tapCount || tapCount < 1) {
      deps.toast("Enter number of keys", "error");
      return;
    }
    deps.toast("Touch your hardware key now...");
    const r = await deps.api("POST", "/api/fido2/unlock", {
      pins: pin ? [pin] : [],
      tap_count: tapCount,
    });
    if (r.error) {
      if (isAlreadyUnlockedConflict(r.error)) {
        deps.toast("Session already active. Refreshing workspace…");
        await deps.refresh();
        return;
      }
      const message = friendlyFidoError(r.error);
      setTextById("authLead", message);
      deps.toast(message, "error");
      return;
    }
    input("fido2Pin").value = "";
    if (r.unlocked_compartments && r.unlocked_compartments.length > 0) {
      const labels = r.unlocked_compartments.map((c: any) => c.label).join(", ");
      deps.toast("Unlocked: " + labels);
    } else {
      deps.toast("Unlocked");
    }
    deps.refresh();
  }

  return {
    resetSession,
    getState: (): Fido2State => ({ detect: lastFidoDetect, keys: lastFidoKeys }),
    friendlyFidoError,
    setUnlockGuidance,
    showUnlockTabs,
    loadFido2,
    togglePoisonWarning,
    submitNewFido2Pin,
    fido2SetNewPin,
    fido2Register,
    fido2RemoveKey,
    switchUnlockTab,
    fido2Unlock,
  };
}
