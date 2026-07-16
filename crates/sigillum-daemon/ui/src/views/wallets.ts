import {
  clearFields,
  optionalNumberValue,
  optionalTextValue,
  renderEntityList,
  setSelectOptions,
  showResultBox,
  textValue,
} from "../render/forms";
import { confirmDangerDialog } from "../render/confirm";
import { esc, escAttr } from "../render/html";

export type WalletProfileKind = "stealth" | "xpub" | "seed";

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

  function renderProviderProfiles(profiles: any[]): void {
    renderEntityList(
      "providerProfileList",
      profiles,
      "No provider profiles yet. Save an RPC endpoint and fee policy above to let deposits and queue work talk to a chain.",
      (profile) => {
        const feeInfo =
          "priority=" +
          (profile.max_priority_fee_per_gas_hex || "-") +
          " · max=" +
          (profile.max_fee_per_gas_hex || "-") +
          " · nativeGas=" +
          (profile.native_gas_limit || "-") +
          " · erc20Gas=" +
          (profile.erc20_gas_limit || "-") +
          " · feeEstimation=" +
          (profile.fee_estimation_enabled ? "on" : "off");
        return (
          '<li><div class="entity-main">' +
          '<div class="entity-title">' +
          esc(profile.name) +
          "</div>" +
          '<div class="entity-meta">' +
          "rpc=" +
          esc(profile.rpc_url) +
          "<br>" +
          "chain=" +
          esc(String(profile.chain_id)) +
          " · compartment=" +
          esc(String(profile.compartment_id)) +
          " · authKey=" +
          esc(profile.auth_token_key || "-") +
          "<br>" +
          esc(feeInfo) +
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
        "wallet=" +
        esc(profile.wallet) +
        " · short=" +
        esc(profile.short_name) +
        " · provider=" +
        esc(profile.provider_profile) +
        "<br>" +
        "compartment=" +
        esc(String(profile.compartment_id)) +
        " · chain=" +
        esc(profile.chain_id != null ? String(profile.chain_id) : "-") +
        " · defaultDestination=" +
        esc(profile.default_destination_address || "-") +
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
          "projectAccount=" +
          esc(String(profile.project_account)) +
          " · provider=" +
          esc(profile.provider_profile) +
          " · source=" +
          esc(source) +
          "<br>" +
          "accountPath=" +
          esc(accountPath) +
          " · receivePath=" +
          esc(receivePath) +
          "<br>" +
          "compartment=" +
          esc(String(profile.compartment_id)) +
          " · chain=" +
          esc(profile.chain_id != null ? String(profile.chain_id) : "-") +
          " · defaultDestination=" +
          esc(profile.default_destination_address || "-") +
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
        const label = profile.label ? " · label=" + profile.label : "";
        return (
          '<li><div class="entity-main">' +
          '<div class="entity-title">' +
          esc(profile.name) +
          "</div>" +
          '<div class="entity-meta">' +
          "words=" +
          esc(String(profile.word_count)) +
          " · account=" +
          esc(String(profile.project_account)) +
          " · provider=" +
          esc(profile.provider_profile) +
          esc(label) +
          "<br>" +
          "accountPath=" +
          esc(profile.account_path || "-") +
          " · receivePath=" +
          esc(profile.receive_path || "-") +
          "<br>" +
          "firstAddress=" +
          esc(profile.first_receive_address || "-") +
          "<br>" +
          "compartment=" +
          esc(String(profile.compartment_id)) +
          " · chain=" +
          esc(profile.chain_id != null ? String(profile.chain_id) : "-") +
          " · defaultDestination=" +
          esc(profile.default_destination_address || "-") +
          "</div></div>" +
          '<div class="entity-actions">' +
          '<button class="btn-ghost" data-action="copyText" data-arg0="' +
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
        deps.api("GET", "/api/profiles/evm"),
        deps.api("GET", "/api/profiles/eth-stealth"),
        deps.api("GET", "/api/profiles/eth-xpub"),
        deps.api("GET", "/api/profiles/eth-seed"),
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

    const r = await deps.api("POST", "/api/profiles/evm/upsert", {
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

    const r = await deps.api("POST", "/api/profiles/eth-stealth/upsert", {
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

    const r = await deps.api("POST", "/api/profiles/eth-xpub/upsert", {
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

    const r = await deps.api("POST", "/api/profiles/eth-seed/upsert", {
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

  async function exportXpubWalletProfile(walletProfile: string): Promise<void> {
    const r = await deps.api("POST", "/api/wallets/eth-xpub/export", {
      wallet_profile: walletProfile,
    });
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }

    const exportedXpub = r.receive_xpub || "";
    input("xpubPreviewProfile").value = walletProfile;
    input("xpubReceiveXpub").value = exportedXpub;

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
        '<button class="btn-ghost" data-action="copyText" data-arg0="' +
        escAttr(exportedXpub) +
        '" data-arg1="Receive branch xpub">Copy Xpub</button>' +
        "</div>",
    );

    await previewXpubReceiveAddress();
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

    const r = await deps.api("POST", "/api/wallets/eth-xpub/derive", { xpub, index });
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
    const r = await deps.api("POST", "/api/wallets/eth-stealth/export", {
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
    exportWalletMeta,
  };
}
