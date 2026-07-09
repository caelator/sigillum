import type {
  ActiveCompartment,
  StatusResponse,
  TreasuryPolicy,
  TreasuryPolicyUpdateRequest,
} from "../contracts";
import { clearFields } from "../render/forms";
import {
  setInlineInfoById,
  setTextById,
  setTrustedHtmlById,
} from "../render/dom";
import { esc } from "../render/html";
import { deriveUiMode } from "../state/status";

export interface SetupRequirement {
  id: "vault" | "unlock" | "compartment";
  complete: boolean;
}

export function setupRequirements(status: StatusResponse | null): SetupRequirement[] {
  return [
    {
      id: "vault",
      complete: deriveUiMode(status) !== "setup",
    },
    {
      id: "unlock",
      complete: deriveUiMode(status) === "unlocked",
    },
    {
      id: "compartment",
      complete: Boolean(status?.active_compartment),
    },
  ];
}

export function compartmentLabel(compartment: ActiveCompartment | null | undefined): string {
  return compartment?.label ?? "No active compartment";
}

interface WizardCompartment {
  label: string;
  threshold: number;
}

export interface SetupWizardDeps {
  api: (method: string, path: string, body?: unknown) => Promise<any>;
  toast: (message: string, type?: string) => void;
  refresh: () => unknown;
  submitNewFido2Pin: (
    pinId: string,
    confirmId: string,
    hintId: string,
    copyToId: string,
    focusId: string,
  ) => Promise<void>;
  friendlyFidoError: (message: unknown) => string;
}

const WIZARD_CHROME: Record<
  string,
  { pill: string; title: string; summary: string; checklist: string[] }
> = {
  wizStepWelcome: {
    pill: "Welcome",
    title: "Before you create a vault",
    summary:
      "A short orientation so you know exactly what Sigillum is and where your data lives before anything is created.",
    checklist: [
      "Confirm this is the machine you want to operate from.",
      "Decide whether you will use hardware keys, a passphrase, or both.",
      "Have any FIDO2 hardware key nearby if you plan to enroll one.",
    ],
  },
  wizStep0: {
    pill: "Step 1 of 3",
    title: "Choose a protection model",
    summary:
      "Pick the starting vault shape that best matches your risk level and how many hardware keys you want to manage.",
    checklist: [
      "Choose the plan that matches how many access layers you want on this machine.",
      "If you want hardware-key setup, attach a FIDO2 key and make sure you know its PIN.",
      "Decide whether you also want a fallback passphrase for local recovery.",
    ],
  },
  wizStepPassphrase: {
    pill: "Step 2 of 3",
    title: "Create your first local compartment",
    summary:
      "Choose the compartment name and passphrase that will protect the first vault on this machine.",
    checklist: [
      "Pick the compartment name you want to see later in the unlocked workspace.",
      "Create a passphrase with at least 8 characters.",
      "Confirm it once so Sigillum can initialize the vault cleanly.",
    ],
  },
  wizStepCompartments: {
    pill: "Step 2 of 3",
    title: "Review what Sigillum will create",
    summary:
      "Thresholds tell Sigillum how many distinct hardware keys are required to unlock each compartment.",
    checklist: [
      "Review each compartment label and the threshold attached to it.",
      "Use higher thresholds only for workflows that need stronger separation.",
      "Continue when the compartment plan matches your local operating model.",
    ],
  },
  wizStepCustomComps: {
    pill: "Step 2 of 3",
    title: "Design custom access layers",
    summary:
      "Define the compartments you want first, then continue to hardware-key registration.",
    checklist: [
      "Add each compartment you want Sigillum to create in this local vault.",
      "Choose a unique threshold for every custom access layer.",
      "Continue once the list matches the workflow split you actually want.",
    ],
  },
  wizStepFido2Pin: {
    pill: "Step 3 of 3",
    title: "Register your first hardware key",
    summary:
      "Leave the PIN blank for touch-only authenticators, or enter the current PIN for keys that require one, then give the key a label you will recognize later.",
    checklist: [
      "Leave the PIN field empty for touch-only keys, or enter the current FIDO2 PIN if the inserted key already requires one.",
      "Use a label you will recognize later, such as primary.",
      "Optionally set one fallback passphrase for local recovery if the keys are unavailable.",
    ],
  },
  wizStepAdditionalKeys: {
    pill: "Add backup keys",
    title: "Finish enrolling the keys this plan expects",
    summary:
      "Higher-threshold compartments only become usable once enough distinct hardware keys are enrolled.",
    checklist: [
      "Insert the next hardware key you want to trust for this vault.",
      "Leave the PIN field empty for touch-only keys, or set and enter the current FIDO2 PIN only if you want that backup key to require one.",
      "Finish for now only if you are comfortable leaving the higher-threshold compartments unavailable until you add more keys later.",
    ],
  },
  wizStepTouch: {
    pill: "Finishing setup",
    title: "Complete the hardware-key touch",
    summary:
      "Sigillum is waiting for a successful FIDO2 registration touch before it can finish the vault setup.",
    checklist: [
      "Keep the hardware key connected to this machine.",
      "Touch the device when it prompts for confirmation.",
      "Stay on this page until Sigillum confirms the vault is ready.",
    ],
  },
  wizStepDone: {
    pill: "Vault ready",
    title: "You can start using the daemon now",
    summary:
      "Unlock the vault, store your first secret, and add more keys or operator profiles when you are ready.",
    checklist: [
      "Unlock once to confirm the setup behaves the way you expect.",
      "Store a first secret or connection key so the workspace becomes useful immediately.",
      "Add more keys, profiles, or deposits whenever you are ready for the next workflow.",
    ],
  },
};

function input(id: string): HTMLInputElement {
  return document.getElementById(id) as HTMLInputElement;
}

function renderChecklist(items: string[]): string {
  return items
    .map(
      (item, index) =>
        '<div class="checklist-item"><div class="checklist-mark">' +
        String(index + 1).padStart(2, "0") +
        "</div><div>" +
        esc(item) +
        "</div></div>",
    )
    .join("");
}

export function createSetupWizard(deps: SetupWizardDeps) {
  let wizCompartments: WizardCompartment[] = [];
  let customCompartments: WizardCompartment[] = [];
  let wizRequiredKeyCount = 1;
  let wizRegisteredKeyCount = 0;
  let wizPrimaryKeyLabel = "";

  function reset(): void {
    wizCompartments = [];
    customCompartments = [];
    wizRequiredKeyCount = 1;
    wizRegisteredKeyCount = 0;
    wizPrimaryKeyLabel = "";
  }

  function wizShowStep(id: string): void {
    document.querySelectorAll(".wizard-step").forEach((step) => step.classList.remove("active"));
    document.getElementById(id)?.classList.add("active");
    updateWizardChrome(id);
  }

  function wizPreset(preset: string): void {
    switch (preset) {
      case "simple":
        wizCompartments = [{ label: "daily", threshold: 1 }];
        wizRenderCompList();
        wizShowStep("wizStepCompartments");
        break;
      case "secure":
        wizCompartments = [
          { label: "daily", threshold: 1 },
          { label: "secure", threshold: 2 },
        ];
        wizRenderCompList();
        wizShowStep("wizStepCompartments");
        break;
      case "legacy":
        wizCompartments = [
          { label: "hot", threshold: 1 },
          { label: "cold", threshold: 2 },
          { label: "legacy", threshold: 3 },
        ];
        wizRenderCompList();
        wizShowStep("wizStepCompartments");
        break;
      case "custom":
        customCompartments = [];
        setTrustedHtmlById("wizCustomCompList", "");
        wizShowStep("wizStepCustomComps");
        break;
      case "passphrase":
        wizShowStep("wizStepPassphrase");
        break;
    }
  }

  function wizCompRowHtml(comps: WizardCompartment[]): string {
    let html = "";
    comps.forEach((comp) => {
      html +=
        '<div class="wiz-comp-row">' +
        '<span class="wiz-comp-label">' +
        esc(comp.label) +
        "</span>" +
        '<span class="wiz-comp-threshold">Tap ' +
        comp.threshold +
        " key" +
        (comp.threshold > 1 ? "s" : "") +
        "</span></div>";
    });
    return html;
  }

  function wizRenderCompList(): void {
    setTrustedHtmlById("wizCompList", wizCompRowHtml(wizCompartments));
  }

  function updateWizardChrome(id: string): void {
    const meta = WIZARD_CHROME[id] || WIZARD_CHROME.wizStep0;
    setTextById("wizStagePill", meta.pill);
    setTextById("wizStageTitle", meta.title);
    setTextById("wizStageSummary", meta.summary);
    setTrustedHtmlById("wizChecklist", renderChecklist(meta.checklist || []));
  }

  function wizGetStarted(): void {
    wizShowStep("wizStep0");
  }

  function wizBackToPresets(): void {
    wizRequiredKeyCount = 1;
    wizRegisteredKeyCount = 0;
    wizPrimaryKeyLabel = "";
    wizShowStep("wizStep0");
  }

  async function wizDetectDevice(): Promise<void> {
    try {
      const r = await deps.api("GET", "/api/fido2/detect");
      const hint = document.getElementById("wizDeviceHint");
      if (hint) {
        if (r.device_present) {
          hint.textContent =
            r.device_count +
            " FIDO2 device(s) detected on this machine. You can continue with hardware-key setup.";
        } else {
          hint.textContent =
            "No FIDO2 device detected right now. You can insert a hardware key and retry, or choose passphrase-only.";
        }
      }
    } catch (_) {
      const hint = document.getElementById("wizDeviceHint");
      if (hint) {
        hint.textContent =
          "Sigillum could not verify hardware-key presence right now. You can still continue, then insert the device before registration if needed.";
      }
    }
  }

  async function wizInitPassphrase(): Promise<void> {
    const label = input("wizPLabel").value || "default";
    const p = input("wizPassphrase").value;
    const pc = input("wizPassphraseConfirm").value;
    if (p.length < 8) {
      deps.toast("Min 8 characters", "error");
      return;
    }
    if (p !== pc) {
      deps.toast("Passphrases do not match", "error");
      return;
    }

    const initR = await deps.api("POST", "/api/compartment/init", {
      id: 0,
      label,
      threshold: 1,
      passphrase: p,
    });
    if (initR.error) {
      deps.toast(initR.error, "error");
      return;
    }

    setTextById("wizDoneMsg", "Vault Created");
    setTextById("wizDoneDetail", 'Compartment "' + label + '" initialized. You are unlocked.');
    wizShowStep("wizStepDone");
    setTimeout(deps.refresh, 1500);
  }

  async function wizProceedFido2(): Promise<void> {
    let comps = wizCompartments;
    if (customCompartments.length > 0) comps = customCompartments;
    if (comps.length === 0) {
      deps.toast("Add at least one compartment", "error");
      return;
    }
    wizCompartments = comps;
    wizRequiredKeyCount = Math.max(1, ...wizCompartments.map((comp) => comp.threshold || 1));
    wizRegisteredKeyCount = 0;
    // Thresholds above the number of detected devices are reachable by
    // swapping keys in during registration, but the operator should know
    // before committing rather than discovering it mid-ceremony.
    if (wizRequiredKeyCount > 1) {
      try {
        const detect = await deps.api("GET", "/api/fido2/detect");
        const detected = Number(detect?.device_count) || 0;
        if (detected > 0 && detected < wizRequiredKeyCount) {
          deps.toast(
            "Highest threshold needs " +
              wizRequiredKeyCount +
              " distinct hardware keys; " +
              detected +
              " detected. You can register keys one at a time, swapping devices in between.",
          );
        }
      } catch (_) {
        // Detection is advisory only; never block the wizard on it.
      }
    }
    wizShowStep("wizStepFido2Pin");
  }

  function wizBackFromFido2Pin(): void {
    if (customCompartments.length > 0) {
      wizShowStep("wizStepCustomComps");
    } else if (wizCompartments.length > 0) {
      wizShowStep("wizStepCompartments");
    } else {
      wizShowStep("wizStep0");
    }
  }

  function wizAddCustomComp(): void {
    const label = input("wizCustomLabel").value;
    const threshold = parseInt(input("wizCustomThreshold").value);
    if (!label || !threshold) {
      deps.toast("Label and threshold required", "error");
      return;
    }
    if (customCompartments.some((comp) => comp.threshold === threshold)) {
      deps.toast("Threshold " + threshold + " already used", "error");
      return;
    }
    customCompartments.push({ label, threshold });
    clearFields(["wizCustomLabel", "wizCustomThreshold"]);
    setTrustedHtmlById("wizCustomCompList", wizCompRowHtml(customCompartments));
    input("wizCustomContinue").disabled = false;
  }

  function wizRenderAdditionalKeyState(): void {
    const remaining = Math.max(wizRequiredKeyCount - wizRegisteredKeyCount, 0);
    const status = document.getElementById("wizAdditionalKeyStatus");
    if (status) {
      status.textContent =
        wizRegisteredKeyCount +
        " of " +
        wizRequiredKeyCount +
        " required hardware key" +
        (wizRequiredKeyCount > 1 ? "s" : "") +
        " enrolled so far.";
    }

    const lead = document.getElementById("wizAdditionalKeysLead");
    if (lead) {
      if (remaining > 0) {
        lead.textContent =
          "Your chosen plan needs " +
          wizRequiredKeyCount +
          " distinct hardware keys. Register " +
          remaining +
          " more now so every compartment you just created can actually be unlocked later.";
      } else {
        lead.textContent =
          "You have enrolled enough hardware keys for the thresholds in this plan. You can finish setup now.";
      }
    }

    const note = document.getElementById("wizAdditionalKeysNote");
    if (note) {
      if (remaining > 0) {
        note.textContent =
          "If you finish with fewer keys than the highest threshold, the lower-threshold compartments will work now, but the stronger access layers will stay unavailable until you enroll more keys later.";
      } else {
        note.textContent =
          "Every configured compartment now has enough enrolled keys behind it to be usable when the corresponding threshold is met.";
      }
    }
  }

  function wizCompleteFido2Setup(): void {
    setTextById("wizDoneMsg", "Setup Complete");
    if (wizRegisteredKeyCount >= wizRequiredKeyCount) {
      setTextById(
        "wizDoneDetail",
        wizCompartments.length +
          " compartment(s) created. " +
          wizRegisteredKeyCount +
          ' hardware key(s) are enrolled for this plan, including "' +
          wizPrimaryKeyLabel +
          '".',
      );
    } else {
      setTextById(
        "wizDoneDetail",
        wizCompartments.length +
          " compartment(s) created. " +
          wizRegisteredKeyCount +
          " of " +
          wizRequiredKeyCount +
          " planned hardware key(s) are enrolled so far, so only the lower-threshold access layers are ready today.",
      );
    }
    wizShowStep("wizStepDone");
    setTimeout(deps.refresh, 1500);
  }

  async function wizRegisterKey(): Promise<void> {
    const pin = input("wizFido2Pin").value;
    const label = input("wizFido2Label").value;
    const passphrase = input("wizFallbackPass").value || null;
    if (!label) {
      deps.toast("Label required", "error");
      return;
    }

    wizShowStep("wizStepTouch");

    const body: any = {
      label,
      compartments: wizCompartments.map((comp) => ({
        label: comp.label,
        threshold: comp.threshold,
        passphrase_mode: null,
      })),
      passphrase: passphrase && passphrase.length >= 8 ? passphrase : null,
    };
    if (pin) body.pin = pin;

    const r = await deps.api("POST", "/api/fido2/setup", body);
    if (r.error) {
      const message = deps.friendlyFidoError(r.error);
      wizShowStep("wizStepFido2Pin");
      setTextById("wizDeviceHint", message);
      input("wizFido2Pin").focus();
      deps.toast(message, "error");
      return;
    }

    wizRegisteredKeyCount = r.total_keys || 1;
    wizPrimaryKeyLabel = label;
    if (wizRequiredKeyCount > wizRegisteredKeyCount) {
      clearFields([
        "wizAdditionalKeyPin",
        "wizAdditionalKeyLabel",
        "wizAdditionalNewPin",
        "wizAdditionalNewPinConfirm",
      ]);
      wizRenderAdditionalKeyState();
      wizShowStep("wizStepAdditionalKeys");
      deps.toast("Primary key registered. Insert the next trusted key to finish this plan.");
      return;
    }

    wizCompleteFido2Setup();
  }

  async function wizSetNewPin(): Promise<void> {
    await deps.submitNewFido2Pin(
      "wizNewFido2Pin",
      "wizNewFido2PinConfirm",
      "wizDeviceHint",
      "wizFido2Pin",
      "wizFido2Label",
    );
  }

  async function wizSetAdditionalKeyPin(): Promise<void> {
    await deps.submitNewFido2Pin(
      "wizAdditionalNewPin",
      "wizAdditionalNewPinConfirm",
      "wizAdditionalKeyStatus",
      "wizAdditionalKeyPin",
      "wizAdditionalKeyLabel",
    );
  }

  async function wizRegisterAdditionalKey(): Promise<void> {
    const pin = input("wizAdditionalKeyPin").value;
    const label = input("wizAdditionalKeyLabel").value;
    if (!label) {
      deps.toast("Label required", "error");
      return;
    }

    deps.toast("Touch your hardware key now...");
    const body: any = { label };
    if (pin) body.pin = pin;
    const r = await deps.api("POST", "/api/fido2/register", body);
    if (r.error) {
      const message = deps.friendlyFidoError(r.error);
      setInlineInfoById("wizAdditionalKeyStatus", message);
      deps.toast(message, "error");
      return;
    }

    wizRegisteredKeyCount = r.total_keys || wizRegisteredKeyCount + 1;
    clearFields([
      "wizAdditionalKeyPin",
      "wizAdditionalKeyLabel",
      "wizAdditionalNewPin",
      "wizAdditionalNewPinConfirm",
    ]);
    wizRenderAdditionalKeyState();

    if (wizRegisteredKeyCount < wizRequiredKeyCount) {
      const remaining = wizRequiredKeyCount - wizRegisteredKeyCount;
      deps.toast(
        'Key "' +
          label +
          '" registered. Insert ' +
          remaining +
          " more key" +
          (remaining > 1 ? "s" : "") +
          " to finish this plan.",
      );
      return;
    }

    deps.toast('Key "' + label + '" registered. Your plan now has enough enrolled hardware keys.');
    wizCompleteFido2Setup();
  }

  function wizFinishForNow(): void {
    wizCompleteFido2Setup();
  }

  function showLinkageChoiceStatus(message: string): void {
    const status = document.getElementById("wizLinkageChoiceStatus");
    if (!status) return;
    status.textContent = message;
    status.classList.remove("hidden");
  }

  function showClaimExecutionChoiceStatus(message: string): void {
    const status = document.getElementById("wizClaimExecutionChoiceStatus");
    if (!status) return;
    status.textContent = message;
    status.classList.remove("hidden");
  }

  function treasuryPolicyUpdateFromCurrent(
    policy: TreasuryPolicy,
  ): TreasuryPolicyUpdateRequest {
    return {
      enabled: policy.enabled,
      allowed_destinations: policy.allowed_destinations || [],
      max_step_native_wei_hex: policy.max_step_native_wei_hex ?? null,
      max_plan_native_wei_hex: policy.max_plan_native_wei_hex ?? null,
      require_simulation: policy.require_simulation,
      block_cross_party_linkage: Boolean(policy.block_cross_party_linkage),
      allow_claim_execution: Boolean(policy.allow_claim_execution),
      simulation_freshness_secs: policy.simulation_freshness_secs ?? null,
      hot_floor_wei_hex: policy.hot_floor_wei_hex ?? null,
      hot_target_wei_hex: policy.hot_target_wei_hex ?? null,
    };
  }

  async function fetchCurrentTreasuryPolicy(): Promise<TreasuryPolicy | null> {
    const r = await deps.api("GET", "/api/treasury/policy");
    if (r.error) {
      throw new Error(String(r.error));
    }
    return (r.policy || null) as TreasuryPolicy | null;
  }

  async function wizEnableLinkageProtection(): Promise<void> {
    try {
      const policy = await fetchCurrentTreasuryPolicy();
      const body: TreasuryPolicyUpdateRequest = policy
        ? {
            ...treasuryPolicyUpdateFromCurrent(policy),
            block_cross_party_linkage: true,
          }
        : { enabled: false, block_cross_party_linkage: true };
      const r = await deps.api("POST", "/api/treasury/policy/update", body);
      if (r.error) {
        deps.toast(r.error, "error");
        return;
      }
      showLinkageChoiceStatus(
        "Payer-linkage protection is on. Sweeps that would link different payers to the same destination are now blocked. Adjust anytime in Treasury policy.",
      );
      deps.toast("Payer-linkage protection enabled");
    } catch (e: any) {
      deps.toast(String(e?.message ?? e), "error");
    }
  }

  function wizDeclineLinkageProtection(): void {
    showLinkageChoiceStatus(
      "Left off for now. You can enable payer-linkage protection later in Treasury policy.",
    );
    deps.toast("You can enable payer-linkage protection later in Treasury policy.");
  }

  async function wizEnableClaimExecution(): Promise<void> {
    try {
      const policy = await fetchCurrentTreasuryPolicy();
      const body: TreasuryPolicyUpdateRequest = policy
        ? {
            ...treasuryPolicyUpdateFromCurrent(policy),
            allow_claim_execution: true,
          }
        : { enabled: false, allow_claim_execution: true };
      const r = await deps.api("POST", "/api/treasury/policy/update", body);
      if (r.error) {
        deps.toast(r.error, "error");
        return;
      }
      showClaimExecutionChoiceStatus(
        "Claim execution opt-in recorded. Claims still cannot run until the Treasury policy is enabled and each claim passes simulation, has a trusted or reviewed claim contract in the risk catalog, and is explicitly approved.",
      );
      deps.toast("Merkle claim execution opt-in recorded");
    } catch (e: any) {
      deps.toast(String(e?.message ?? e), "error");
    }
  }

  function wizDeclineClaimExecution(): void {
    showClaimExecutionChoiceStatus(
      "You can enable Merkle claim execution later in Treasury policy.",
    );
    deps.toast("You can enable Merkle claim execution later in Treasury policy.");
  }

  return {
    reset,
    updateWizardChrome,
    wizShowStep,
    wizPreset,
    wizBackToPresets,
    wizDetectDevice,
    wizGetStarted,
    wizInitPassphrase,
    wizProceedFido2,
    wizBackFromFido2Pin,
    wizAddCustomComp,
    wizDeclineClaimExecution,
    wizDeclineLinkageProtection,
    wizEnableClaimExecution,
    wizEnableLinkageProtection,
    wizRegisterKey,
    wizSetNewPin,
    wizSetAdditionalKeyPin,
    wizRegisterAdditionalKey,
    wizFinishForNow,
  };
}
