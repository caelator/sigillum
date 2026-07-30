import { ROUTE_PATHS } from "../routePaths";
import {
  clearFields,
  optionalNumberValue,
  optionalTextValue,
  renderEntityList,
  setSelectOptions,
  showResultBox,
  textValue,
} from "../render/forms";
import { confirmDangerDialog, informDialog } from "../render/confirm";
import { esc, escAttr } from "../render/html";
import { formatWeiHexAsGwei } from "./treasury";

export type WalletProfileKind = "stealth" | "xpub" | "seed";

// An xpub is watch-only material, but it exposes the wallet's ENTIRE past
// and future receive-address tree to anyone holding it (plan task 3.4).
// Export flows pin this warning inline and toast it; copy flows gate the
// FIRST xpub copy of each session behind an inform-tier acknowledgement
// instead of nagging on every click.
export const XPUB_EXPOSURE_WARNING =
  "An xpub exposes this wallet's entire receive tree — every past and future address — to anyone holding it. Share it only with systems that must watch those addresses; never publish it or send it to a payer.";

// The daemon restates the exposure on every xpub export; older daemons omit
// the field, so read it defensively and fall back to the local copy.
export function xpubExportWarnings(response: { warning?: unknown }): string[] {
  return typeof response.warning === "string" && response.warning.length > 0
    ? [response.warning]
    : [XPUB_EXPOSURE_WARNING];
}

export interface WalletProfileView {
  name: string;
  kind: WalletProfileKind;
  provider_profile?: string | null;
  signer_available?: boolean | null;
}

export interface WalletProfilesState {
  providerProfiles: any[];
  walletProfiles: any[];
  xpubWalletProfiles: any[];
  seedWalletProfiles: any[];
}

export interface WalletActionsDeps {
  api: (method: string, path: string, body?: unknown) => Promise<any>;
  toast: (message: string, type?: string) => void;
  refresh: () => unknown;
  copyText: (value: string, label: string) => Promise<void>;
}

function input(id: string): HTMLInputElement {
  return document.getElementById(id) as HTMLInputElement;
}

export function createWalletActions(deps: WalletActionsDeps) {
  let lastProviderProfiles: any[] = [];
  let lastWalletProfiles: any[] = [];
  let lastXpubWalletProfiles: any[] = [];
  let lastSeedWalletProfiles: any[] = [];
  // Session-scoped acknowledgement for the xpub exposure notice: the first
  // xpub copy of each session confirms the inform dialog once, later copies
  // proceed directly.
  let xpubCopyAcknowledged = false;

  function feeCap(value: unknown): string {
    if (value === null || value === undefined) return "Not configured";
    if (typeof value !== "string" || !/^0x[0-9a-fA-F]+$/.test(value)) {
      return "Invalid saved value";
    }
    return formatWeiHexAsGwei(value) + " gwei";
  }

  function renderProviderProfiles(profiles: any[]): void {
    renderEntityList(
      "providerProfileList",
      profiles,
      "No provider profiles yet. Save an RPC endpoint and fee policy above to let deposits and queue work talk to a chain.",
      (profile) => {
        const feeInfo =
          "Priority fee cap: " +
          feeCap(profile.max_priority_fee_per_gas_hex) +
          " · Max fee cap: " +
          feeCap(profile.max_fee_per_gas_hex) +
          " · Native gas limit: " +
          (profile.native_gas_limit || "Not configured") +
          " · ERC-20 gas limit: " +
          (profile.erc20_gas_limit || "Not configured") +
          " · Fee estimation " +
          (profile.fee_estimation_enabled ? "on" : "off");
        return (
          '<li><div class="entity-main">' +
          '<div class="entity-title">' +
          esc(profile.name) +
          "</div>" +
          '<div class="entity-meta">' +
          "RPC endpoint: " +
          esc(profile.rpc_url) +
          "<br>" +
          "Chain " +
          esc(String(profile.chain_id)) +
          " · " +
          (profile.compartment_id != null
            ? "Compartment " + esc(String(profile.compartment_id))
            : "Compartment not specified") +
          " · " +
          (profile.auth_token_key
            ? "Authentication key configured"
            : "No authentication key") +
          "<br>" +
          esc(feeInfo) +
          (profile.auth_token_key
            ? '<details class="technical-detail"><summary>Connection key reference</summary><code>' +
              esc(profile.auth_token_key) +
              "</code></details>"
            : "") +
          "</div></div>" +
          '<div class="entity-actions">' +
          '<button class="btn-ghost" data-action="copyText" data-arg0="' +
          escAttr(profile.rpc_url) +
          '" data-arg1="RPC URL">Copy RPC</button>' +
          '<button class="btn-danger" data-action="deleteProviderProfile" data-arg0="' +
          escAttr(profile.name) +
          '">Delete</button>' +
          "</div></li>"
        );
      },
    );
  }

  function renderWalletProfiles(profiles: any[]): void {
    renderEntityList(
      "walletProfileList",
      profiles,
      "No wallet profiles yet. Create one above to bind a Sigillum wallet label to a provider before you generate deposits.",
      (profile) =>
        '<li><div class="entity-main">' +
        '<div class="entity-title">' +
        esc(profile.name) +
        "</div>" +
        '<div class="entity-meta">' +
        "Wallet " +
        esc(profile.wallet) +
        " · Short name " +
        esc(profile.short_name) +
        " · Provider " +
        esc(profile.provider_profile) +
        "<br>" +
        "Compartment " +
        esc(String(profile.compartment_id)) +
        " · " +
        (profile.chain_id != null
          ? "Chain " + esc(String(profile.chain_id))
          : "Chain not specified") +
        " · Default destination: " +
        esc(profile.default_destination_address || "Not configured") +
        "</div></div>" +
        '<div class="entity-actions">' +
        '<button class="btn-ghost" data-action="exportWalletMeta" data-arg0="' +
        escAttr(profile.wallet) +
        '" data-arg1="' +
        escAttr(profile.short_name) +
        '">Export Meta</button>' +
        '<button class="btn-danger" data-action="deleteWalletProfile" data-arg0="' +
        escAttr(profile.name) +
        '">Delete</button>' +
        "</div></li>",
    );
  }

  function renderXpubWalletProfiles(profiles: any[]): void {
    renderEntityList(
      "xpubWalletProfileList",
      profiles,
      "No xpub wallet profiles yet. Save one above when you want a public receive tree without exposing private key material.",
      (profile) => {
        const accountPath = "m/44'/60'/" + profile.project_account + "'";
        const receivePath =
          profile.external_receive_path || (profile.external_account_path ? profile.external_account_path + "/0" : accountPath + "/0");
        const source = profile.external_account_xpub
          ? profile.external_account_path
            ? "external custom account xpub"
            : "external account xpub"
          : profile.external_receive_xpub
            ? profile.external_receive_path
              ? "external custom xpub"
              : "external receive xpub"
            : "Sigillum project xpub";
        return (
          '<li><div class="entity-main">' +
          '<div class="entity-title">' +
          esc(profile.name) +
          "</div>" +
          '<div class="entity-meta">' +
          "Project account " +
          esc(String(profile.project_account)) +
          " · Provider " +
          esc(profile.provider_profile) +
          " · Source: " +
          esc(source) +
          "<br>" +
          "Account path " +
          esc(accountPath) +
          " · Receive path " +
          esc(receivePath) +
          "<br>" +
          "Compartment " +
          esc(String(profile.compartment_id)) +
          " · " +
          (profile.chain_id != null
            ? "Chain " + esc(String(profile.chain_id))
            : "Chain not specified") +
          " · Default destination: " +
          esc(profile.default_destination_address || "Not configured") +
          "</div></div>" +
          '<div class="entity-actions">' +
          '<button class="btn-ghost" data-action="exportXpubWalletProfile" data-arg0="' +
          escAttr(profile.name) +
          '">Export Xpub</button>' +
          '<button class="btn-danger" data-action="deleteXpubWalletProfile" data-arg0="' +
          escAttr(profile.name) +
          '">Delete</button>' +
          "</div></li>"
        );
      },
    );
  }

  function renderSeedWalletProfiles(profiles: any[]): void {
    renderEntityList(
      "seedWalletProfileList",
      profiles,
      "No imported seed wallets yet. Import a 12-word or 24-word phrase to add another receive wallet profile.",
      (profile) => {
        const label = profile.label ? " · Label: " + profile.label : "";
        return (
          '<li><div class="entity-main">' +
          '<div class="entity-title">' +
          esc(profile.name) +
          "</div>" +
          '<div class="entity-meta">' +
          esc(String(profile.word_count)) +
          " words" +
          " · Account " +
          esc(String(profile.project_account)) +
          " · Provider " +
          esc(profile.provider_profile) +
          esc(label) +
          "<br>" +
          "Account path " +
          esc(profile.account_path || "Not available") +
          " · Receive path " +
          esc(profile.receive_path || "Not available") +
          "<br>" +
          "First address: " +
          esc(profile.first_receive_address || "Not available") +
          "<br>" +
          "Compartment " +
          esc(String(profile.compartment_id)) +
          " · " +
          (profile.chain_id != null
            ? "Chain " + esc(String(profile.chain_id))
            : "Chain not specified") +
          " · Default destination: " +
          esc(profile.default_destination_address || "Not configured") +
          "</div></div>" +
          '<div class="entity-actions">' +
          '<button class="btn-ghost" data-action="copyXpubWithWarning" data-arg0="' +
          escAttr(profile.receive_xpub || "") +
          '" data-arg1="Seed wallet receive xpub">Copy Xpub</button>' +
          '<button class="btn-ghost" data-action="copyText" data-arg0="' +
          escAttr(profile.first_receive_address || "") +
          '" data-arg1="First receive address">Copy Address</button>' +
          '<button class="btn-danger" data-action="deleteSeedWalletProfile" data-arg0="' +
          escAttr(profile.name) +
          '">Delete</button>' +
          "</div></li>"
        );
      },
    );
  }

  async function loadProfiles(): Promise<void> {
    try {
      const [providerResp, walletResp, xpubResp, seedResp] = await Promise.all([
        deps.api("GET", ROUTE_PATHS.API_PROFILES_EVM),
        deps.api("GET", ROUTE_PATHS.API_PROFILES_ETH_STEALTH),
        deps.api("GET", ROUTE_PATHS.API_PROFILES_ETH_XPUB),
        deps.api("GET", ROUTE_PATHS.API_PROFILES_ETH_SEED),
      ]);
      if (providerResp.error || walletResp.error || xpubResp.error || seedResp.error) return;

      const providers = providerResp.profiles || [];
      const wallets = walletResp.profiles || [];
      const xpubWallets = xpubResp.profiles || [];
      const seedWallets = seedResp.profiles || [];
      lastProviderProfiles = providers;
      lastWalletProfiles = wallets;
      lastXpubWalletProfiles = xpubWallets;
      lastSeedWalletProfiles = seedWallets;

      renderProviderProfiles(providers);
      renderWalletProfiles(wallets);
      renderXpubWalletProfiles(xpubWallets);
      renderSeedWalletProfiles(seedWallets);

      setSelectOptions(
        "walletProviderProfile",
        providers.map((profile: any) => ({
          value: profile.name,
          label: profile.name + " · chain " + profile.chain_id,
        })),
        providers.length ? "Select provider profile" : "No provider profiles available",
      );
      setSelectOptions(
        "xpubProviderProfile",
        providers.map((profile: any) => ({
          value: profile.name,
          label: profile.name + " · chain " + profile.chain_id,
        })),
        providers.length ? "Select provider profile" : "No provider profiles available",
      );
      setSelectOptions(
        "seedProviderProfile",
        providers.map((profile: any) => ({
          value: profile.name,
          label: profile.name + " · chain " + profile.chain_id,
        })),
        providers.length ? "Select provider profile" : "No provider profiles available",
      );

      const walletOptions = wallets.map((profile: any) => ({
        value: profile.name,
        label: profile.name + " · " + profile.wallet,
      }));
      setSelectOptions(
        "depositNativeWalletProfile",
        walletOptions,
        wallets.length ? "Select wallet profile" : "No wallet profiles available",
      );
      setSelectOptions(
        "depositErc20WalletProfile",
        walletOptions,
        wallets.length ? "Select wallet profile" : "No wallet profiles available",
      );
      setSelectOptions(
        "depositScanWalletProfile",
        walletOptions,
        wallets.length ? "Select wallet profile" : "No wallet profiles available",
      );
      setSelectOptions(
        "xpubPreviewProfile",
        xpubWallets.map((profile: any) => ({
          value: profile.name,
          label: profile.name + " · account " + profile.project_account,
        })),
        xpubWallets.length ? "Select xpub profile" : "No xpub profiles available",
      );
    } catch (_) {}
  }

  async function upsertProviderProfile(): Promise<void> {
    const name = textValue("providerName");
    const rpcUrl = textValue("providerRpcUrl");
    const chainId = parseInt(textValue("providerChainId"), 10);
    if (!name || !rpcUrl || !chainId) {
      deps.toast("Provider name, RPC URL, and chain ID are required", "error");
      return;
    }

    const r = await deps.api("POST", ROUTE_PATHS.API_PROFILES_EVM_UPSERT, {
      name,
      rpc_url: rpcUrl,
      auth_token_key: optionalTextValue("providerAuthTokenKey"),
      compartment_id: optionalNumberValue("providerCompartmentId"),
      chain_id: chainId,
      max_priority_fee_per_gas_hex: optionalTextValue("providerMaxPriorityFee"),
      max_fee_per_gas_hex: optionalTextValue("providerMaxFee"),
      native_gas_limit: optionalNumberValue("providerNativeGasLimit"),
      erc20_gas_limit: optionalNumberValue("providerErc20GasLimit"),
      fee_estimation_enabled: Boolean(input("providerFeeEstimation")?.checked),
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }

    clearFields([
      "providerName",
      "providerRpcUrl",
      "providerAuthTokenKey",
      "providerCompartmentId",
      "providerMaxPriorityFee",
      "providerMaxFee",
      "providerNativeGasLimit",
      "providerErc20GasLimit",
    ]);
    const feeEstimationEl = input("providerFeeEstimation");
    if (feeEstimationEl) feeEstimationEl.checked = false;
    deps.toast("Provider profile saved");
    deps.refresh();
  }

  async function deleteProviderProfile(name: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Delete provider profile",
      body:
        'Delete provider profile "' +
        name +
        '"? Wallets and deposits that reference it lose their chain connection.',
      actionLabel: "Delete",
    });
    if (!confirmed) return;
    const r = await deps.api("POST", "/api/profiles/evm/delete", { name });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Provider profile deleted");
    deps.refresh();
  }

  async function upsertWalletProfile(): Promise<void> {
    const name = textValue("walletProfileName");
    const wallet = textValue("walletLabel");
    const providerProfile = textValue("walletProviderProfile");
    if (!name || !wallet || !providerProfile) {
      deps.toast("Wallet profile name, wallet label, and provider profile are required", "error");
      return;
    }

    const r = await deps.api("POST", ROUTE_PATHS.API_PROFILES_ETH_STEALTH_UPSERT, {
      name,
      wallet,
      short_name: optionalTextValue("walletShortName"),
      provider_profile: providerProfile,
      compartment_id: optionalNumberValue("walletCompartmentId"),
      chain_id: optionalNumberValue("walletChainId"),
      default_destination_address: optionalTextValue("walletDefaultDestination"),
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }

    clearFields([
      "walletProfileName",
      "walletLabel",
      "walletShortName",
      "walletCompartmentId",
      "walletChainId",
      "walletDefaultDestination",
    ]);
    deps.toast("Wallet profile saved");
    deps.refresh();
  }

  async function deleteWalletProfile(name: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Delete wallet profile",
      body:
        'Delete wallet profile "' +
        name +
        '"? Its deposit and receive configuration is removed from this daemon.',
      actionLabel: "Delete",
    });
    if (!confirmed) return;
    const r = await deps.api("POST", "/api/profiles/eth-stealth/delete", { name });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Wallet profile deleted");
    deps.refresh();
  }

  async function upsertXpubWalletProfile(): Promise<void> {
    const name = textValue("xpubProfileName");
    const providerProfile = textValue("xpubProviderProfile");
    const projectAccount = parseInt(textValue("xpubProjectAccount"), 10);
    if (!name || !providerProfile || !Number.isInteger(projectAccount) || projectAccount < 0) {
      deps.toast("Profile name, provider profile, and a non-negative project account are required", "error");
      return;
    }

    const r = await deps.api("POST", ROUTE_PATHS.API_PROFILES_ETH_XPUB_UPSERT, {
      name,
      project_account: projectAccount,
      provider_profile: providerProfile,
      compartment_id: optionalNumberValue("xpubCompartmentId"),
      chain_id: optionalNumberValue("xpubChainId"),
      external_receive_xpub: optionalTextValue("xpubExternalReceiveXpub"),
      external_receive_path: optionalTextValue("xpubExternalReceivePath"),
      external_account_xpub: optionalTextValue("xpubExternalAccountXpub"),
      external_account_path: optionalTextValue("xpubExternalAccountPath"),
      default_destination_address: optionalTextValue("xpubDefaultDestination"),
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }

    clearFields([
      "xpubProfileName",
      "xpubCompartmentId",
      "xpubChainId",
      "xpubExternalReceiveXpub",
      "xpubExternalReceivePath",
      "xpubExternalAccountXpub",
      "xpubExternalAccountPath",
      "xpubDefaultDestination",
    ]);
    input("xpubProjectAccount").value = "0";
    deps.toast("Xpub wallet profile saved");
    deps.refresh();
  }

  async function deleteXpubWalletProfile(name: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Delete xpub wallet profile",
      body:
        'Delete xpub wallet profile "' +
        name +
        '"? The watch-only profile is removed from this daemon.',
      actionLabel: "Delete",
    });
    if (!confirmed) return;
    const r = await deps.api("POST", "/api/profiles/eth-xpub/delete", { name });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Xpub wallet profile deleted");
    deps.refresh();
  }

  async function upsertSeedWalletProfile(): Promise<void> {
    const name = textValue("seedProfileName");
    const mnemonic = textValue("seedMnemonic");
    const providerProfile = textValue("seedProviderProfile");
    const projectAccount = parseInt(textValue("seedProjectAccount"), 10);
    const wordCount = mnemonic ? mnemonic.split(/\s+/).filter(Boolean).length : 0;
    if (!name || !mnemonic || !providerProfile || !Number.isInteger(projectAccount) || projectAccount < 0) {
      deps.toast("Profile name, seed phrase, provider profile, and a non-negative account are required", "error");
      return;
    }
    if (wordCount !== 12 && wordCount !== 24) {
      deps.toast("Seed phrase must contain exactly 12 or 24 words", "error");
      return;
    }

    const r = await deps.api("POST", ROUTE_PATHS.API_PROFILES_ETH_SEED_UPSERT, {
      name,
      label: optionalTextValue("seedProfileLabel"),
      mnemonic,
      mnemonic_passphrase: optionalTextValue("seedMnemonicPassphrase"),
      project_account: projectAccount,
      provider_profile: providerProfile,
      compartment_id: optionalNumberValue("seedCompartmentId"),
      chain_id: optionalNumberValue("seedChainId"),
      default_destination_address: optionalTextValue("seedDefaultDestination"),
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }

    clearFields([
      "seedProfileName",
      "seedProfileLabel",
      "seedMnemonic",
      "seedMnemonicPassphrase",
      "seedCompartmentId",
      "seedChainId",
      "seedDefaultDestination",
    ]);
    input("seedProjectAccount").value = "0";
    deps.toast("Seed wallet profile imported");
    deps.refresh();
  }

  async function deleteSeedWalletProfile(name: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Delete seed wallet profile",
      body:
        'Delete seed wallet profile "' +
        name +
        '"? The stored mnemonic is removed from this daemon\'s vault; on-chain funds are not moved, but this daemon can no longer sign with it.',
      actionLabel: "Delete",
    });
    if (!confirmed) return;
    const r = await deps.api("POST", "/api/profiles/eth-seed/delete", { name });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Seed wallet profile deleted");
    deps.refresh();
  }

  async function exportSelectedXpubWallet(): Promise<void> {
    const walletProfile = textValue("xpubPreviewProfile");
    if (!walletProfile) {
      deps.toast("Choose an xpub wallet profile first", "error");
      return;
    }
    await exportXpubWalletProfile(walletProfile);
  }

  // Cautions only — never block the flow. The exposure warning gets a toast
  // and is pinned next to the exported xpub so it stays visible after the
  // toast fades; the next export refreshes the box.
  function surfaceXpubExportWarnings(response: { warning?: unknown }): void {
    const warnings = xpubExportWarnings(response);
    warnings.forEach((warning) => deps.toast(warning, "warning"));
    showResultBox(
      "xpubExportWarnings",
      "<strong>Xpub exposure — review before sharing.</strong><br>" +
        warnings.map((warning) => esc(warning)).join("<br>"),
    );
  }

  async function exportXpubWalletProfile(walletProfile: string): Promise<void> {
    const r = await deps.api("POST", ROUTE_PATHS.API_WALLETS_ETH_XPUB_EXPORT, {
      wallet_profile: walletProfile,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }

    const exportedXpub = r.receive_xpub || "";
    input("xpubPreviewProfile").value = walletProfile;
    input("xpubReceiveXpub").value = exportedXpub;
    surfaceXpubExportWarnings(r);

    showResultBox(
      "xpubExportResult",
      "<strong>" +
        esc(walletProfile) +
        "</strong><br>" +
        "accountPath=" +
        esc(r.account_path || "-") +
        "<br>" +
        "receivePath=" +
        esc(r.receive_path || "-") +
        "<br>" +
        "xpub=" +
        esc(exportedXpub) +
        "<br>" +
        '<div style="margin-top:10px;display:flex;gap:8px;flex-wrap:wrap;">' +
        '<button class="btn-ghost" data-action="copyXpubWithWarning" data-arg0="' +
        escAttr(exportedXpub) +
        '" data-arg1="Receive branch xpub">Copy Xpub</button>' +
        "</div>",
    );

    await previewXpubReceiveAddress();
  }

  async function copyXpubWithWarning(value: string, label: string): Promise<void> {
    if (!value) {
      deps.toast("No xpub to copy", "error");
      return;
    }
    if (!xpubCopyAcknowledged) {
      const acknowledged = await informDialog({
        title: "Xpub exposes the whole address tree",
        body: XPUB_EXPOSURE_WARNING,
        actionLabel: "Copy xpub",
      });
      if (!acknowledged) return;
      xpubCopyAcknowledged = true;
    }
    await deps.copyText(value, label);
  }

  async function previewXpubReceiveAddress(): Promise<void> {
    const xpub = textValue("xpubReceiveXpub");
    const index = parseInt(textValue("xpubPreviewIndex"), 10);
    if (!xpub) {
      deps.toast("Export or paste a receive-branch xpub first", "error");
      return;
    }
    if (!Number.isInteger(index) || index < 0) {
      deps.toast("Receive index must be a non-negative number", "error");
      return;
    }

    const r = await deps.api("POST", ROUTE_PATHS.API_WALLETS_ETH_XPUB_DERIVE, { xpub, index });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }

    showResultBox(
      "xpubPreviewResult",
      "<strong>Receive index " +
        esc(String(r.index)) +
        "</strong><br>" +
        "path=" +
        esc("receive/" + r.index) +
        "<br>" +
        "address=" +
        esc(r.address) +
        "<br>" +
        '<div style="margin-top:10px;display:flex;gap:8px;flex-wrap:wrap;">' +
        '<button class="btn-ghost" data-action="copyText" data-arg0="' +
        escAttr(r.address) +
        '" data-arg1="Receive address">Copy Address</button>' +
        "</div>",
    );
  }

  async function exportWalletMeta(wallet: string, shortName?: string): Promise<void> {
    const r = await deps.api("POST", ROUTE_PATHS.API_WALLETS_ETH_STEALTH_EXPORT, {
      wallet,
      short_name: shortName || null,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    await deps.copyText(r.stealth_meta_address, "Stealth meta-address");
  }

  return {
    getState: (): WalletProfilesState => ({
      providerProfiles: lastProviderProfiles,
      walletProfiles: lastWalletProfiles,
      xpubWalletProfiles: lastXpubWalletProfiles,
      seedWalletProfiles: lastSeedWalletProfiles,
    }),
    renderProviderProfiles,
    renderWalletProfiles,
    renderXpubWalletProfiles,
    renderSeedWalletProfiles,
    loadProfiles,
    upsertProviderProfile,
    deleteProviderProfile,
    upsertWalletProfile,
    deleteWalletProfile,
    upsertXpubWalletProfile,
    deleteXpubWalletProfile,
    upsertSeedWalletProfile,
    deleteSeedWalletProfile,
    exportSelectedXpubWallet,
    exportXpubWalletProfile,
    previewXpubReceiveAddress,
    copyXpubWithWarning,
    exportWalletMeta,
  };
}
