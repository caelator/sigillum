import type {
  EthSeedWalletProfile,
  EthXpubWalletProfile,
  TreasuryGroupSummary,
  TreasuryReceiveAllocation,
} from "../contracts";
import { confirmDangerDialog } from "../render/confirm";
import { setHiddenById as setHidden, setTextById as setText } from "../render/dom";
import {
  clearFields,
  optionalNumberValue,
  optionalTextValue,
  renderEntityList,
  setSelectOptions,
  textValue,
} from "../render/forms";
import { esc, escAttr } from "../render/html";
import { formatWeiHexAsEth } from "./treasury";

export type ManagedWalletKind = "seed" | "xpub";

const WALLET_NAME_PATTERN = /^[A-Za-z0-9_-]+$/;
const IMPORT_TABS = ["seed", "xpub", "watch"] as const;
const CREATE_FORM_CONTROL_IDS = [
  "walletCreateName",
  "walletCreateLabel",
  "walletCreateProvider",
  "walletCreateWords12",
  "walletCreateWords24",
  "walletCreateAccount",
  "walletCreateChainId",
  "walletCreateDestination",
  "walletCreatePassphrase",
  "walletCreateSubmit",
];

function isSeedProfile(
  profile: EthSeedWalletProfile | EthXpubWalletProfile,
): profile is EthSeedWalletProfile {
  return typeof (profile as EthSeedWalletProfile).word_count === "number";
}

/**
 * Sum native balances for one wallet profile across treasury overview groups
 * and format them per chain, e.g. "1.5 ETH on chain 1 · 0.2 on 8453".
 * Profiles with no groups have simply not been scanned yet.
 */
export function walletNativeBalanceFromGroups(
  profileName: string,
  groups?: TreasuryGroupSummary[] | null,
): string {
  const owned = (groups || []).filter(
    (group) => group.wallet_profile === profileName,
  );
  if (!owned.length) return "not scanned yet";
  const totalsByChain = new Map<number, bigint>();
  owned.forEach((group) => {
    const hex = (group.native_total_wei_hex || "").trim();
    let wei = 0n;
    if (/^0x[0-9a-fA-F]+$/.test(hex)) {
      try {
        wei = BigInt(hex);
      } catch (_) {}
    }
    totalsByChain.set(group.chain_id, (totalsByChain.get(group.chain_id) || 0n) + wei);
  });
  return Array.from(totalsByChain.keys())
    .sort((a, b) => a - b)
    .map((chainId, index) => {
      const amount = formatWeiHexAsEth(
        "0x" + (totalsByChain.get(chainId) || 0n).toString(16),
      );
      return index === 0
        ? amount + " ETH on chain " + chainId
        : amount + " on " + chainId;
    })
    .join(" · ");
}

/**
 * Identity line for a watch-only xpub profile. Imported xpub profiles expose
 * a public receive branch; legacy profiles derive that branch from the local
 * compartment when exported.
 */
export function xpubDisplay(profile: EthXpubWalletProfile): string {
  if (profile.external_account_path) {
    return "external account path " + profile.external_account_path + "/0";
  }
  if (profile.external_receive_path) {
    return "external receive path " + profile.external_receive_path;
  }
  const source =
    profile.external_account_xpub || profile.external_receive_xpub ? "external " : "";
  return source + "receive path m/44'/60'/" + profile.project_account + "'/0";
}

/**
 * Plain-text meta lines (joined with \n) for one unified wallet row:
 * address/path identity, provider facts, per-chain balance, and active
 * receive-allocation count when known.
 */
export function walletRowMeta(
  profile: EthSeedWalletProfile | EthXpubWalletProfile,
  groups?: TreasuryGroupSummary[] | null,
  activeReceiveCount?: number | null,
): string {
  const seed = isSeedProfile(profile) ? profile : null;
  const lines: string[] = [];
  lines.push(
    seed
      ? seed.first_receive_address || "first receive address unavailable"
      : xpubDisplay(profile as EthXpubWalletProfile),
  );
  const facts = [
    "provider=" + profile.provider_profile,
    "chain=" + (profile.chain_id != null ? String(profile.chain_id) : "-"),
    "account=" + profile.project_account,
  ];
  if (seed) facts.push("words=" + seed.word_count);
  if (!seed) {
    const xpub = profile as EthXpubWalletProfile;
    if (xpub.external_account_xpub) {
      facts.push(xpub.external_account_path ? "source=external custom account xpub" : "source=external account xpub");
    }
    else if (xpub.external_receive_xpub) {
      facts.push(xpub.external_receive_path ? "source=external custom xpub" : "source=external receive xpub");
    }
  }
  lines.push(facts.join(" · "));
  lines.push("balance=" + walletNativeBalanceFromGroups(profile.name, groups));
  if (activeReceiveCount) {
    lines.push("receive allocations=" + activeReceiveCount);
  }
  return lines.join("\n");
}

export interface WalletManagerDeps {
  api: (method: string, path: string, body?: unknown) => Promise<any>;
  toast: (message: string, type?: string) => void;
}

function input(id: string): HTMLInputElement | null {
  return document.getElementById(id) as HTMLInputElement | null;
}

export function createWalletManagerActions(deps: WalletManagerDeps) {
  let lastSeedProfiles: EthSeedWalletProfile[] = [];
  let lastXpubProfiles: EthXpubWalletProfile[] = [];
  let lastProviderProfiles: any[] = [];
  let lastOverviewGroups: TreasuryGroupSummary[] = [];
  let lastReceiveAllocations: TreasuryReceiveAllocation[] = [];
  // The one-time mnemonic lives here between create and confirm — never in
  // storage, never in a toast, never logged. confirmMnemonicSaved() nulls it.
  let pendingMnemonic: string | null = null;
  let receiveTargetProfile: string | null = null;

  function activeReceiveCountFor(profileName: string): number {
    return lastReceiveAllocations.filter(
      (allocation) =>
        allocation.wallet_profile === profileName && allocation.status === "active",
    ).length;
  }

  function kindPill(kind: ManagedWalletKind): string {
    return kind === "seed"
      ? '<span class="pill pill-good">signer</span>'
      : '<span class="pill pill-info">watch-only</span>';
  }

  function renderWalletManagerList(): void {
    type Row = {
      kind: ManagedWalletKind;
      profile: EthSeedWalletProfile | EthXpubWalletProfile;
    };
    const rows: Row[] = [
      ...lastSeedProfiles.map((profile) => ({ kind: "seed" as const, profile })),
      ...lastXpubProfiles.map((profile) => ({ kind: "xpub" as const, profile })),
    ];
    renderEntityList(
      "walletManagerList",
      rows,
      {
        message: "No wallets yet. Create one below or import an existing wallet.",
        actionLabel: "Create a wallet",
        action: "focusWalletCreate",
      },
      (row) => {
        const profile = row.profile;
        const seed = row.kind === "seed" ? (profile as EthSeedWalletProfile) : null;
        const title = (seed && seed.label) || profile.name;
        const metaHtml = walletRowMeta(
          profile,
          lastOverviewGroups,
          activeReceiveCountFor(profile.name),
        )
          .split("\n")
          .map((line) => esc(line))
          .join("<br>");
        let actions =
          '<button class="btn-ghost" data-action="promptWalletReceiveAddress" data-arg0="' +
          escAttr(profile.name) +
          '">Receive address</button>';
        if (seed && seed.first_receive_address) {
          actions +=
            '<button class="btn-ghost" data-action="copyWalletAddress" data-arg0="' +
            escAttr(seed.first_receive_address) +
            '" data-arg1="First receive address">Copy address</button>';
        }
        actions +=
          '<button class="btn-danger" data-action="deleteManagedWallet" data-arg0="' +
          escAttr(row.kind) +
          '" data-arg1="' +
          escAttr(profile.name) +
          '">Delete</button>';
        return (
          '<li><div class="entity-main">' +
          '<div class="entity-title">' +
          esc(title) +
          " " +
          kindPill(row.kind) +
          "</div>" +
          '<div class="entity-meta">' +
          metaHtml +
          "</div></div>" +
          '<div class="entity-actions">' +
          actions +
          "</div></li>"
        );
      },
    );
  }

  function syncCreateFormAvailability(): void {
    const submit = input("walletCreateSubmit");
    if (!submit) return;
    submit.disabled = pendingMnemonic != null || lastProviderProfiles.length === 0;
  }

  function renderProviderOptions(): void {
    const providerOptions = lastProviderProfiles.map((profile: any) => ({
      value: profile.name,
      label: profile.name + " · chain " + profile.chain_id,
    }));
    const placeholder = lastProviderProfiles.length
      ? "Select provider profile"
      : "No provider profiles available";
    setSelectOptions("walletCreateProvider", providerOptions, placeholder);
    setSelectOptions("walletImportSeedProvider", providerOptions, placeholder);
    setSelectOptions("walletImportXpubProvider", providerOptions, placeholder);
    setHidden("walletCreateProviderHint", lastProviderProfiles.length > 0);
    // No providers yet: offer the inline quick-add instead of sending the
    // operator hunting for another card. Creating a wallet is local key
    // derivation; the provider is only the RPC endpoint later used to read
    // balances — but the profile requires one, so collect it right here.
    setHidden("walletQuickProvider", lastProviderProfiles.length > 0);
    syncCreateFormAvailability();
  }

  async function quickAddWalletProvider(): Promise<void> {
    const name = textValue("walletQuickProviderName") || "mainnet";
    const rpcUrl = textValue("walletQuickProviderUrl");
    const chainIdRaw = textValue("walletQuickProviderChainId");
    const chainId = chainIdRaw ? Number(chainIdRaw) : 1;
    const urlInvalid = !rpcUrl || !/^https?:\/\//.test(rpcUrl);
    const urlField = input("walletQuickProviderUrl");
    if (urlField) urlField.classList.toggle("input-invalid", urlInvalid);
    if (urlInvalid) {
      deps.toast("Enter the RPC endpoint URL (http(s)://...)", "error");
      return;
    }
    if (!Number.isFinite(chainId) || chainId < 1) {
      deps.toast("Chain ID must be a positive number", "error");
      return;
    }
    const r = await deps.api("POST", "/api/profiles/evm/upsert", {
      name,
      rpc_url: rpcUrl,
      chain_id: chainId,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Provider '" + name + "' saved.");
    clearFields(["walletQuickProviderUrl"]);
    await loadWalletManager();
  }

  function setCreateFormDisabled(disabled: boolean): void {
    CREATE_FORM_CONTROL_IDS.forEach((id) => {
      const el = document.getElementById(id) as
        | HTMLInputElement
        | HTMLSelectElement
        | HTMLButtonElement
        | null;
      if (el) el.disabled = disabled;
    });
    const form = document.getElementById("walletCreateForm");
    if (form) form.classList.toggle("form-disabled", disabled);
    if (!disabled) syncCreateFormAvailability();
  }

  function selectedWordCount(): 12 | 24 {
    const twelve = input("walletCreateWords12");
    return twelve && twelve.checked ? 12 : 24;
  }

  function resetWordCountSelection(): void {
    const twelve = input("walletCreateWords12");
    const twentyFour = input("walletCreateWords24");
    if (twelve) twelve.checked = false;
    if (twentyFour) twentyFour.checked = true;
  }

  async function loadWalletManager(): Promise<void> {
    try {
      const [seedResp, xpubResp, providerResp, overviewResp, receiveResp] =
        await Promise.all([
          deps.api("GET", "/api/profiles/eth-seed"),
          deps.api("GET", "/api/profiles/eth-xpub"),
          deps.api("GET", "/api/profiles/evm"),
          deps.api("GET", "/api/treasury/overview"),
          deps.api("GET", "/api/treasury/receive-addresses"),
        ]);
      if (!seedResp.error) lastSeedProfiles = seedResp.profiles || [];
      if (!xpubResp.error) lastXpubProfiles = xpubResp.profiles || [];
      if (!providerResp.error) lastProviderProfiles = providerResp.profiles || [];
      if (!overviewResp.error) lastOverviewGroups = overviewResp.groups || [];
      if (!receiveResp.error) {
        lastReceiveAllocations = receiveResp.allocations || [];
      }
      renderProviderOptions();
      renderWalletManagerList();
    } catch (_) {}
  }

  async function refreshWalletManager(): Promise<void> {
    await loadWalletManager();
    deps.toast("Wallet list refreshed");
  }

  // ── Create (server-generated mnemonic, shown exactly once) ────────────

  function renderMnemonicReveal(mnemonic: string): void {
    const panel = document.getElementById("walletMnemonicReveal");
    if (!panel) return;
    const words = mnemonic.split(/\s+/).filter(Boolean);
    let html =
      '<div class="section-title">Back Up This Seed Phrase</div>' +
      '<p class="helper-text">This is the only copy. Write all ' +
      words.length +
      " words down in order and store them offline. The daemon keeps only an encrypted secret — never the phrase itself.</p>" +
      '<ol class="mnemonic-grid">';
    words.forEach((word, index) => {
      html +=
        '<li class="mnemonic-word"><span class="mnemonic-index">' +
        (index + 1) +
        '</span><span class="mono">' +
        esc(word) +
        "</span></li>";
    });
    html += "</ol>";
    html +=
      '<p class="mnemonic-warning">Written down? It will never be shown again.</p>';
    html +=
      '<div class="form-row">' +
      '<button class="btn-ghost" data-action="copyMnemonicPhrase">Copy phrase</button>' +
      '<button class="btn-primary" data-action="confirmMnemonicSaved">I saved it — hide the phrase</button>' +
      "</div>";
    panel.innerHTML = html;
    panel.classList.remove("hidden");
  }

  async function createWallet(): Promise<void> {
    // Busy affordance on the explicit submit control while the daemon
    // generates and stores the new seed material.
    const submit = input("walletCreateSubmit");
    if (submit) submit.classList.add("btn-busy");
    try {
      await createWalletRequest();
    } finally {
      if (submit) submit.classList.remove("btn-busy");
    }
  }

  async function createWalletRequest(): Promise<void> {
    if (pendingMnemonic != null) {
      deps.toast("Confirm the current seed phrase backup first", "error");
      return;
    }
    const name = textValue("walletCreateName");
    if (!name) {
      deps.toast("Wallet name is required", "error");
      return;
    }
    if (!WALLET_NAME_PATTERN.test(name)) {
      deps.toast(
        "Wallet name may only contain letters, digits, '-' and '_'",
        "error",
      );
      return;
    }
    const providerProfile = textValue("walletCreateProvider");
    if (!providerProfile) {
      deps.toast("Add an RPC provider first, then choose it here", "error");
      return;
    }
    const accountText = textValue("walletCreateAccount");
    const projectAccount = accountText ? parseInt(accountText, 10) : 0;
    if (!Number.isInteger(projectAccount) || projectAccount < 0) {
      deps.toast("Project account must be a non-negative number", "error");
      return;
    }

    const body: Record<string, unknown> = {
      name,
      word_count: selectedWordCount(),
      project_account: projectAccount,
      provider_profile: providerProfile,
    };
    const label = optionalTextValue("walletCreateLabel");
    if (label) body.label = label;
    const mnemonicPassphrase = optionalTextValue("walletCreatePassphrase");
    if (mnemonicPassphrase) body.mnemonic_passphrase = mnemonicPassphrase;
    const chainId = optionalNumberValue("walletCreateChainId");
    if (chainId != null) body.chain_id = chainId;
    const destination = optionalTextValue("walletCreateDestination");
    if (destination) body.default_destination_address = destination;

    const r = await deps.api("POST", "/api/profiles/eth-seed/create", body);
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    const mnemonic = typeof r.mnemonic === "string" ? r.mnemonic.trim() : "";
    if (!mnemonic) {
      deps.toast(
        "Daemon returned no seed phrase — do not use this wallet until you investigate",
        "error",
      );
      return;
    }
    pendingMnemonic = mnemonic;
    clearFields(["walletCreatePassphrase"]);
    renderMnemonicReveal(mnemonic);
    setCreateFormDisabled(true);
    deps.toast('Wallet "' + name + '" created — back up the seed phrase now');
  }

  async function copyMnemonicPhrase(): Promise<void> {
    if (pendingMnemonic == null) {
      deps.toast("No seed phrase is being shown", "error");
      return;
    }
    await copyValue(pendingMnemonic, "Seed phrase");
  }

  async function confirmMnemonicSaved(): Promise<void> {
    pendingMnemonic = null;
    const panel = document.getElementById("walletMnemonicReveal");
    if (panel) {
      panel.innerHTML = "";
      panel.classList.add("hidden");
    }
    clearFields([
      "walletCreateName",
      "walletCreateLabel",
      "walletCreateChainId",
      "walletCreateDestination",
      "walletCreatePassphrase",
    ]);
    const account = input("walletCreateAccount");
    if (account) account.value = "0";
    resetWordCountSelection();
    setCreateFormDisabled(false);
    deps.toast("Seed phrase cleared — it cannot be shown again");
    await loadWalletManager();
  }

  // ── Row actions ───────────────────────────────────────────────────────

  async function copyValue(value: string, labelText: string): Promise<void> {
    if (!value) {
      deps.toast("Nothing to copy", "error");
      return;
    }
    try {
      if (typeof navigator !== "undefined" && navigator.clipboard) {
        await navigator.clipboard.writeText(value);
        deps.toast(labelText + " copied");
        return;
      }
    } catch (_) {}
    // Prompt-less fallback: off-screen textarea + execCommand("copy").
    try {
      const area = document.createElement("textarea") as HTMLTextAreaElement;
      area.value = value;
      area.setAttribute("readonly", "readonly");
      area.style.position = "fixed";
      area.style.top = "-1000px";
      area.style.opacity = "0";
      document.body.appendChild(area);
      if (typeof area.select === "function") area.select();
      const copied =
        typeof (document as any).execCommand === "function" &&
        (document as any).execCommand("copy");
      area.remove();
      if (copied) {
        deps.toast(labelText + " copied");
        return;
      }
    } catch (_) {}
    deps.toast("Clipboard unavailable — copy manually from the screen", "error");
  }

  async function copyWalletAddress(value: string, labelText?: string): Promise<void> {
    await copyValue(value, labelText || "Address");
  }

  async function deleteManagedWallet(kind: string, name: string): Promise<void> {
    const targetKind: ManagedWalletKind = kind === "xpub" ? "xpub" : "seed";
    const confirmed = await confirmDangerDialog({
      title: targetKind === "seed" ? "Delete wallet" : "Delete xpub profile",
      body:
        (targetKind === "seed"
          ? 'Delete seed wallet "' + name + '"? The mnemonic is removed from this daemon\'s vault; on-chain funds are not moved, but this daemon can no longer sign with it.'
          : 'Delete xpub profile "' + name + '"? The watch-only profile is removed from this daemon.'),
      actionLabel: "Delete",
    });
    if (!confirmed) return;
    const path =
      targetKind === "seed"
        ? "/api/profiles/eth-seed/delete"
        : "/api/profiles/eth-xpub/delete";
    const r = await deps.api("POST", path, { name });
    if (r.error) {
      deps.toast(r.error, "error");
      renderWalletManagerList();
      return;
    }
    deps.toast(
      (targetKind === "seed" ? "Wallet" : "Xpub profile") + ' "' + name + '" deleted',
    );
    await loadWalletManager();
  }

  // ── Receive-address allocation (inline panel under the list) ──────────

  function promptWalletReceiveAddress(profileName: string): void {
    receiveTargetProfile = profileName;
    setText("walletReceiveTarget", profileName);
    setHidden("walletReceivePanel", false);
    const purpose = input("walletReceivePurpose");
    if (purpose && typeof purpose.focus === "function") purpose.focus();
  }

  function cancelWalletReceiveAddress(): void {
    receiveTargetProfile = null;
    clearFields(["walletReceivePurpose", "walletReceiveLabel"]);
    setHidden("walletReceivePanel", true);
  }

  async function allocateWalletReceiveAddress(): Promise<void> {
    const walletProfile = receiveTargetProfile;
    if (!walletProfile) {
      deps.toast('Pick a wallet via its "Receive address" action first', "error");
      return;
    }
    const purpose = textValue("walletReceivePurpose");
    if (!purpose) {
      deps.toast("Purpose is required (e.g. invoices)", "error");
      return;
    }
    const body: { wallet_profile: string; purpose: string; label?: string } = {
      wallet_profile: walletProfile,
      purpose,
    };
    const label = optionalTextValue("walletReceiveLabel");
    if (label) body.label = label;
    const r = await deps.api("POST", "/api/treasury/receive-addresses/allocate", body);
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    cancelWalletReceiveAddress();
    const allocation = r.allocation;
    deps.toast(
      "Receive address allocated" +
        (allocation && allocation.address ? ": " + allocation.address : ""),
    );
    await loadWalletManager();
  }

  // ── Import tabs ───────────────────────────────────────────────────────

  function setWalletImportTab(tab: string): void {
    const target = (IMPORT_TABS as readonly string[]).includes(tab) ? tab : "seed";
    IMPORT_TABS.forEach((name) => {
      const suffix = name.charAt(0).toUpperCase() + name.slice(1);
      setHidden("walletImport" + suffix + "Form", name !== target);
      const button = document.getElementById("walletImportTab" + suffix);
      if (button) button.classList.toggle("active", name === target);
    });
    // Security posture: switching tabs always drops mnemonic-bearing input.
    clearFields(["walletImportSeedMnemonic", "walletImportSeedPassphrase"]);
  }

  async function importSeedWallet(): Promise<void> {
    const name = textValue("walletImportSeedName");
    if (!name) {
      deps.toast("Wallet name is required", "error");
      return;
    }
    if (!WALLET_NAME_PATTERN.test(name)) {
      deps.toast(
        "Wallet name may only contain letters, digits, '-' and '_'",
        "error",
      );
      return;
    }
    const words = textValue("walletImportSeedMnemonic").split(/\s+/).filter(Boolean);
    if (words.length !== 12 && words.length !== 24) {
      deps.toast("Seed phrase must contain exactly 12 or 24 words", "error");
      return;
    }
    const providerProfile = textValue("walletImportSeedProvider");
    if (!providerProfile) {
      deps.toast("Choose a provider profile first", "error");
      return;
    }
    const accountText = textValue("walletImportSeedAccount");
    const projectAccount = accountText ? parseInt(accountText, 10) : 0;
    if (!Number.isInteger(projectAccount) || projectAccount < 0) {
      deps.toast("Project account must be a non-negative number", "error");
      return;
    }

    const body: Record<string, unknown> = {
      name,
      mnemonic: words.join(" "),
      project_account: projectAccount,
      provider_profile: providerProfile,
    };
    const label = optionalTextValue("walletImportSeedLabel");
    if (label) body.label = label;
    const passphrase = optionalTextValue("walletImportSeedPassphrase");
    if (passphrase) body.mnemonic_passphrase = passphrase;
    const chainId = optionalNumberValue("walletImportSeedChainId");
    if (chainId != null) body.chain_id = chainId;
    const destination = optionalTextValue("walletImportSeedDestination");
    if (destination) body.default_destination_address = destination;

    const r = await deps.api("POST", "/api/profiles/eth-seed/upsert", body);
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    clearFields([
      "walletImportSeedName",
      "walletImportSeedLabel",
      "walletImportSeedMnemonic",
      "walletImportSeedPassphrase",
      "walletImportSeedChainId",
      "walletImportSeedDestination",
    ]);
    const account = input("walletImportSeedAccount");
    if (account) account.value = "0";
    deps.toast('Seed wallet "' + name + '" imported');
    await loadWalletManager();
  }

  async function importXpubWallet(): Promise<void> {
    const name = textValue("walletImportXpubName");
    if (!name) {
      deps.toast("Profile name is required", "error");
      return;
    }
    const providerProfile = textValue("walletImportXpubProvider");
    if (!providerProfile) {
      deps.toast("Choose a provider profile first", "error");
      return;
    }
    const accountText = textValue("walletImportXpubAccount");
    const projectAccount = accountText ? parseInt(accountText, 10) : 0;
    if (!Number.isInteger(projectAccount) || projectAccount < 0) {
      deps.toast("Project account must be a non-negative number", "error");
      return;
    }

    const body: Record<string, unknown> = {
      name,
      project_account: projectAccount,
      provider_profile: providerProfile,
    };
    const compartmentId = optionalNumberValue("walletImportXpubCompartmentId");
    if (compartmentId != null) body.compartment_id = compartmentId;
    const chainId = optionalNumberValue("walletImportXpubChainId");
    if (chainId != null) body.chain_id = chainId;
    const destination = optionalTextValue("walletImportXpubDestination");
    if (destination) body.default_destination_address = destination;
    const externalReceiveXpub = optionalTextValue("walletImportExternalReceiveXpub");
    if (externalReceiveXpub) body.external_receive_xpub = externalReceiveXpub;
    const externalReceivePath = optionalTextValue("walletImportExternalReceivePath");
    if (externalReceivePath) body.external_receive_path = externalReceivePath;
    const externalAccountXpub = optionalTextValue("walletImportExternalAccountXpub");
    if (externalAccountXpub) body.external_account_xpub = externalAccountXpub;
    const externalAccountPath = optionalTextValue("walletImportExternalAccountPath");
    if (externalAccountPath) body.external_account_path = externalAccountPath;

    const r = await deps.api("POST", "/api/profiles/eth-xpub/upsert", body);
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    clearFields([
      "walletImportXpubName",
      "walletImportXpubCompartmentId",
      "walletImportXpubChainId",
      "walletImportExternalReceiveXpub",
      "walletImportExternalReceivePath",
      "walletImportExternalAccountXpub",
      "walletImportExternalAccountPath",
      "walletImportXpubDestination",
    ]);
    const account = input("walletImportXpubAccount");
    if (account) account.value = "0";
    deps.toast('Xpub wallet profile "' + name + '" saved');
    await loadWalletManager();
  }

  async function importWatchAddress(): Promise<void> {
    const address = textValue("walletImportWatchAddress");
    if (!address) {
      deps.toast("Watch address is required", "error");
      return;
    }
    const r = await deps.api("POST", "/api/inventory/watch-addresses/upsert", {
      address,
      label: optionalTextValue("walletImportWatchLabel"),
      tags: [],
      enabled: true,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    clearFields(["walletImportWatchAddress", "walletImportWatchLabel"]);
    deps.toast("Watch address saved to the inventory watch book");
    await loadWalletManager();
  }

  return {
    loadWalletManager,
    refreshWalletManager,
    renderWalletManagerList,
    quickAddWalletProvider,
    createWallet,
    confirmMnemonicSaved,
    copyMnemonicPhrase,
    copyWalletAddress,
    deleteManagedWallet,
    promptWalletReceiveAddress,
    allocateWalletReceiveAddress,
    cancelWalletReceiveAddress,
    setWalletImportTab,
    importSeedWallet,
    importXpubWallet,
    importWatchAddress,
    hasPendingMnemonic: (): boolean => pendingMnemonic != null,
  };
}
