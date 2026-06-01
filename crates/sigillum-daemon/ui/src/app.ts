// @ts-nocheck

import "./styles/app.css";
import { clearSessionToken, requestWithSession } from "./api/session";
import { handleActionEvent as handleDispatchedActionEvent } from "./actions/dispatcher";
import {
  setHiddenById as setHidden,
  setTextById as setText,
  setTrustedHtmlById as setTrustedHtml,
} from "./render/dom";
import { clearFields, showResultBox } from "./render/forms";
import { esc, escAttr, formatTs, statBox } from "./render/html";
import {
  clearRefreshTimer,
  markRefreshCompleted,
  scheduleRefresh,
  shouldAutoRefresh,
  updateRefreshMeta,
} from "./state/refresh";
import { deriveUiMode } from "./state/status";
import { createFido2Actions } from "./views/fido2";
import { createInventoryActions } from "./views/inventory";
import { createOperationsActions } from "./views/operations";
import { createShellRenderer } from "./views/shell";
import { createSetupWizard } from "./views/setup";
import { createWalletActions } from "./views/wallets";

const SETUP_RESET_CONFIRMATION = 'RESET LOCAL SIGILLUM DATA';
const OPERATOR_CARD_IDS = [
  'nextStepCard',
  'guideCard',
  'compartmentCard',
  'pushCard',
  'apiKeysCard',
  'secretsCard',
  'profilesCard',
  'xpubCard',
  'inventoryCard',
  'depositsCard',
  'queueCard',
  'maintenanceCard',
  'fido2Card',
  'backupCard',
  'auditCard',
  'diagCard',
];
const WORKSPACE_SECTION_KEY = 'sigillumWorkspaceSection';
const WORKSPACE_SECTIONS = [
  {
    id: 'overview',
    label: 'Overview',
    summary: 'Product framing, current status, and the highest-leverage next move.',
  },
  {
    id: 'access',
    label: 'Access',
    summary: 'Setup, unlock, session controls, and machine-level recovery.',
  },
  {
    id: 'vault',
    label: 'Vault',
    summary: 'Compartments, connection keys, encrypted secrets, and secret movement.',
  },
  {
    id: 'wallets',
    label: 'Wallets',
    summary: 'Providers, stealth wallets, and xpub receive-wallet setup.',
  },
  {
    id: 'operations',
    label: 'Operations',
    summary: 'Deposits, queue execution, and local maintenance cycles.',
  },
  {
    id: 'recovery',
    label: 'Recovery',
    summary: 'Hardware keys, snapshots, audit trail, and diagnostics.',
  },
];
let currentStatus = null;
let currentUiMode = 'loading';
let activeWorkspaceSection = 'overview';
let refreshPromise = null;
let refreshQueued = false;
let lastApiKeys = [];
let lastSecretKeys = [];
let nextStepPrimaryTarget = null;
let nextStepSecondaryTarget = null;

try {
  activeWorkspaceSection =
    window.sessionStorage.getItem(WORKSPACE_SECTION_KEY) || 'overview';
} catch (_) {}

function setCardsHidden(ids, hidden) {
  ids.forEach(id => setHidden(id, hidden));
}

function setStatusBadge(className, label) {
  const badge = document.getElementById('statusBadge');
  badge.className = 'status-badge ' + className;
  badge.textContent = label;
}

function setSecretsAccess(unlocked) {
  setHidden('secretsLocked', unlocked);
  setHidden('secretsUnlocked', !unlocked);
}

function resetVaultCounts() {
  setText('apiKeyCount', '-');
  setText('secretCount', '-');
  setText('compartmentCount', '-');
}

function visibleWorkspaceCards() {
  return Array.from(document.querySelectorAll('main .card[data-workspace-section]'))
    .filter(card => !card.classList.contains('hidden'));
}

function availableWorkspaceSections() {
  const visibleSections = new Set(
    visibleWorkspaceCards()
      .map(card => card.dataset.workspaceSection)
      .filter(Boolean)
  );
  return WORKSPACE_SECTIONS.filter(section => visibleSections.has(section.id));
}

function firstVisibleCardInSection(sectionId) {
  return visibleWorkspaceCards().find(
    card => card.dataset.workspaceSection === sectionId
  );
}

function focusCard(card) {
  if (!card) return;
  if (!card.hasAttribute('tabindex')) card.setAttribute('tabindex', '-1');
  card.focus({ preventScroll: true });
}

function storeWorkspaceSection(sectionId) {
  try {
    window.sessionStorage.setItem(WORKSPACE_SECTION_KEY, sectionId);
  } catch (_) {}
}

function ensureActiveWorkspaceSection() {
  const sections = availableWorkspaceSections();
  if (!sections.length) {
    activeWorkspaceSection = 'overview';
    return;
  }
  if (!sections.some(section => section.id === activeWorkspaceSection)) {
    activeWorkspaceSection = sections[0].id;
    storeWorkspaceSection(activeWorkspaceSection);
  }
}

function syncWorkspaceSections() {
  ensureActiveWorkspaceSection();
  const targetSection = activeWorkspaceSection;
  document.querySelectorAll('main .card[data-workspace-section]').forEach(card => {
    card.classList.toggle(
      'section-hidden',
      Boolean(targetSection) && card.dataset.workspaceSection !== targetSection
    );
  });
}

function syncSectionNav() {
  const nav = document.getElementById('sectionNav');
  const main = document.querySelector('main');
  if (!nav) return;

  const sections = availableWorkspaceSections();
  if (sections.length <= 1) {
    nav.classList.add('hidden');
    nav.innerHTML = '';
    if (main) main.classList.remove('has-nav');
    syncWorkspaceSections();
    return;
  }

  ensureActiveWorkspaceSection();
  nav.innerHTML = sections.map(section =>
    '<button type="button" class="workspace-tab' +
      (section.id === activeWorkspaceSection ? ' active' : '') +
      '" data-action="selectWorkspaceSection" data-arg0="' + escAttr(section.id) + '">' +
      '<strong>' + esc(section.label) + '</strong>' +
      '<span>' + esc(section.summary) + '</span>' +
    '</button>'
  ).join('');
  nav.classList.remove('hidden');
  if (main) main.classList.add('has-nav');
  syncWorkspaceSections();
}

function selectWorkspaceSection(sectionId) {
  activeWorkspaceSection = sectionId;
  storeWorkspaceSection(sectionId);
  syncSectionNav();
  const firstCard = firstVisibleCardInSection(sectionId);
  if (firstCard) {
    firstCard.scrollIntoView({ behavior: 'smooth', block: 'start' });
    focusCard(firstCard);
  }
}

function jumpToCard(id) {
  const el = document.getElementById(id);
  if (!el || el.classList.contains('hidden')) return;
  const targetSection = el.dataset.workspaceSection;
  if (targetSection && targetSection !== activeWorkspaceSection) {
    activeWorkspaceSection = targetSection;
    storeWorkspaceSection(targetSection);
    syncSectionNav();
  }
  requestAnimationFrame(() => {
    if (!el.classList.contains('hidden') && !el.classList.contains('section-hidden')) {
      el.scrollIntoView({ behavior: 'smooth', block: 'start' });
      focusCard(el);
    }
  });
}

function heroPrimaryAction() {
  if (currentUiMode === 'setup') {
    jumpToCard('setupCard');
    return;
  }
  if (currentUiMode === 'locked') {
    jumpToCard('authCard');
    const input = document.getElementById('passphrase');
    if (input) input.focus();
    return;
  }
  jumpToCard('secretsCard');
}

function heroSecondaryAction() {
  if (currentUiMode === 'setup') {
    jumpToCard('setupCard');
    return;
  }
  if (currentUiMode === 'locked') {
    jumpToCard('authCard');
    fido2Actions.switchUnlockTab('fido2');
    const input = document.getElementById('fido2Pin');
    if (input) input.focus();
    return;
  }
  jumpToCard('profilesCard');
}

function renderHeroContext(items) {
  return items.map(item =>
    '<div class="context-row"><strong>' + esc(item.title) + '</strong><span>' + esc(item.body) + '</span></div>'
  ).join('');
}

function renderNextStepItems(items) {
  return items.map(item =>
    '<div class="next-step-item"><strong>' + esc(item.title) + '</strong><span>' + esc(item.body) + '</span></div>'
  ).join('');
}

function isAlreadyUnlockedConflict(message) {
  return String(message || '').toLowerCase().includes('already unlocked');
}

function nextStepPrimaryAction() {
  if (nextStepPrimaryTarget) jumpToCard(nextStepPrimaryTarget);
}

function nextStepSecondaryAction() {
  if (nextStepSecondaryTarget) jumpToCard(nextStepSecondaryTarget);
}

function updateNextStepCard() {
  const card = document.getElementById('nextStepCard');
  if (!card || currentUiMode !== 'unlocked') {
    setHidden('nextStepCard', true);
    return;
  }
  const walletState = walletActions.getState();
  const operationsState = operationsActions.getState();
  const fido2State = fido2Actions.getState();
  const hasStealthWalletProfiles = walletState.walletProfiles.length > 0;
  const hasXpubWalletProfiles = walletState.xpubWalletProfiles.length > 0;
  const hasSeedWalletProfiles = walletState.seedWalletProfiles.length > 0;

  let nextStep = {
    title: 'Choose the next concrete operation',
    summary: 'The vault is live. Use the cards below to run maintenance, inspect queue work, review audit history, and verify local daemon health.',
    items: [
      { title: 'Operations', body: 'Maintenance refreshes deposits and drains queue work with the current local policy settings.' },
      { title: 'Recovery', body: 'Snapshots, audit trail, and diagnostics help you validate and recover the local daemon state.' },
    ],
    primaryLabel: 'Open maintenance',
    primaryTarget: 'maintenanceCard',
    secondaryLabel: 'Review diagnostics',
    secondaryTarget: 'diagCard',
    note: 'This card stays focused on the next useful step instead of the whole workspace at once.',
  };

  if (lastSecretKeys.length === 0 && lastApiKeys.length === 0) {
    nextStep = {
      title: 'Store the first protected value',
      summary: 'The vault is unlocked, but it does not hold any working data yet. Start by storing a secret or a connection key so the daemon can do useful work locally.',
      items: [
        { title: 'Encrypted Secrets', body: 'Use this for values you want encrypted at rest and revealed only after unlock.' },
        { title: 'Connection Keys', body: 'Use this for RPC or auth tokens the daemon needs directly during profile or queue workflows.' },
      ],
      primaryLabel: 'Store a secret',
      primaryTarget: 'secretsCard',
      secondaryLabel: 'Store a connection key',
      secondaryTarget: 'apiKeysCard',
      note: 'Start with one real value. That is enough to prove the local vault flow end to end.',
    };
  } else if (walletState.providerProfiles.length === 0) {
    nextStep = {
      title: 'Connect an EVM provider',
      summary: 'Profiles turn stored credentials into reusable operator configuration. Save an RPC endpoint and fee policy before setting up wallets or deposits.',
      items: [
        { title: 'Provider profile', body: 'This defines RPC URL, chain ID, optional auth token key, and fee defaults.' },
        { title: 'Why now', body: 'Wallet profiles, deposits, and queue work all depend on provider configuration being in place first.' },
      ],
      primaryLabel: 'Open profiles',
      primaryTarget: 'profilesCard',
      secondaryLabel: 'Review connection keys',
      secondaryTarget: 'apiKeysCard',
      note: 'If your provider needs an auth token, store that connection key first and then reference it from the provider profile.',
    };
  } else if (!hasStealthWalletProfiles && !hasXpubWalletProfiles && !hasSeedWalletProfiles) {
    nextStep = {
      title: 'Choose a wallet family',
      summary: 'Provider settings are ready. Next choose whether you want a stealth operator wallet, an xpub receive wallet, or an imported seed wallet for public receive-tree visibility.',
      items: [
        { title: 'Stealth wallet', body: 'Use this when you want tracked deposits, sweep queues, and maintenance workflows today.' },
        { title: 'Seed or xpub receive wallet', body: 'Use this when you want deterministic receive-address previews and multiple wallet profiles visible in one place.' },
      ],
      primaryLabel: 'Open wallets',
      primaryTarget: 'profilesCard',
      secondaryLabel: 'Read operator guide',
      secondaryTarget: 'guideCard',
      note: 'Stealth is the current end-to-end operator path. Xpub is now available for receive-branch export and preview, but not yet for discovery or sweeping.',
    };
  } else if (!hasStealthWalletProfiles && (hasXpubWalletProfiles || hasSeedWalletProfiles)) {
    nextStep = {
      title: 'Add a stealth wallet for live operator flows',
      summary: 'Your receive wallet profile is ready for address visibility, but tracked deposits, sweep queues, and maintenance still run on stealth wallets today.',
      items: [
        { title: 'Keep receive wallets for visibility', body: 'Use the xpub card to export receive branches and preview deposit addresses by index.' },
        { title: 'Add stealth for operations', body: 'Use the stealth wallet card when you want deposits, queue jobs, and maintenance cycles to run locally.' },
      ],
      primaryLabel: 'Open wallets',
      primaryTarget: 'profilesCard',
      secondaryLabel: 'Open xpub tools',
      secondaryTarget: 'xpubCard',
      note: 'This keeps the current product honest: receive wallets are live for visibility, while stealth remains the operational wallet family.',
    };
  } else if (fido2State.keys.length === 1) {
    nextStep = {
      title: 'Add a backup hardware key',
      summary: 'The workspace is usable, but a single enrolled hardware key is still a recovery risk. Register one more trusted key while the vault is already unlocked.',
      items: [
        { title: 'Primary plus backup', body: 'A second enrolled key gives you a much safer path if the primary device is lost or unavailable.' },
        { title: 'Unlock behavior', body: 'Higher-threshold compartments only become practical when you have enough enrolled keys to satisfy them.' },
      ],
      primaryLabel: 'Register another key',
      primaryTarget: 'fido2Card',
      secondaryLabel: 'Review snapshots',
      secondaryTarget: 'backupCard',
      note: 'If you intentionally rely on passphrase recovery, keep the snapshot flow healthy too.',
    };
  } else if (operationsState.deposits.length === 0) {
    nextStep = {
      title: 'Create the first tracked deposit',
      summary: 'Profiles are ready. The next meaningful test is generating a deposit address, monitoring it, and letting the daemon enqueue the follow-up work.',
      items: [
        { title: 'Native or ERC-20', body: 'Use the deposit card to create an address, set expected values, and decide whether sweep jobs should auto-queue.' },
        { title: 'Why this matters', body: 'This is the clearest end-to-end validation of wallet, provider, queue, and maintenance behavior.' },
      ],
      primaryLabel: 'Open deposits',
      primaryTarget: 'depositsCard',
      secondaryLabel: 'Open maintenance',
      secondaryTarget: 'maintenanceCard',
      note: 'After the first deposit exists, you can refresh balances and verify queue work without leaving the daemon.',
    };
  } else if (operationsState.queueJobs.length > 0) {
    nextStep = {
      title: 'Work the queue that is already waiting',
      summary: 'Sigillum has queued operator jobs. Review them first, then process a batch or run a full maintenance cycle if you want the daemon to move forward now.',
      items: [
        { title: 'Queue review', body: 'Inspect job kind, attempts, state, and recent errors before processing.' },
        { title: 'Maintenance option', body: 'Use maintenance when you want refresh, enqueue, and queue execution to happen as one cycle.' },
      ],
      primaryLabel: 'Open queue',
      primaryTarget: 'queueCard',
      secondaryLabel: 'Run maintenance',
      secondaryTarget: 'maintenanceCard',
      note: 'Queue processing changes live operator state, so review the job list before pushing a batch through.',
    };
  }

  nextStepPrimaryTarget = nextStep.primaryTarget;
  nextStepSecondaryTarget = nextStep.secondaryTarget;
  setText('nextStepTitle', nextStep.title);
  setText('nextStepSummary', nextStep.summary);
  setTrustedHtml('nextStepList', renderNextStepItems(nextStep.items));
  setText('nextStepNote', nextStep.note);
  const primaryBtn = document.getElementById('nextStepPrimaryBtn');
  const secondaryBtn = document.getElementById('nextStepSecondaryBtn');
  primaryBtn.textContent = nextStep.primaryLabel;
  secondaryBtn.textContent = nextStep.secondaryLabel;
  setHidden('nextStepCard', false);
}

function updateHeroState(mode, active, unlocked) {
  const primary = document.getElementById('heroPrimaryBtn');
  const secondary = document.getElementById('heroSecondaryBtn');

  if (mode === 'setup') {
    setText('statusEyebrow', 'First run');
    setText('statusTitle', 'Set up the vault');
    setText('statusSummary', 'This daemon is ready, but the machine does not have a vault yet. Finish setup first, then Sigillum will reveal the working operator surface below.');
    setText('heroModeValue', 'Setup required');
    setText('heroModeDetail', 'You only do this once per local data directory. Everything else stays quiet until setup is complete.');
    primary.textContent = 'Start setup';
    secondary.textContent = 'View options';
    setTrustedHtml('statusContext', renderHeroContext([
      { title: 'Pick a model', body: 'Start with Daily + Secure if you want the best default for one person managing everyday and higher-trust work.' },
      { title: 'Register access', body: 'Use a hardware key for stronger local protection, or choose passphrase-only if this machine will not use hardware keys.' },
      { title: 'Verify the path', body: 'When setup finishes, unlock once and store a first real value so the workflow is proven end to end.' },
    ]));
    return;
  }

  if (mode === 'locked') {
    setText('statusEyebrow', 'Locked state');
    setText('statusTitle', 'Unlock to continue');
    setText('statusSummary', 'Your local data is still on disk, but this browser session is not authenticated. Unlock to reach secrets, profiles, deposits, queue actions, backups, and diagnostics.');
    setText('heroModeValue', 'Locked');
    setText('heroModeDetail', 'Use the same passphrase or hardware-key threshold you configured during setup. The session token stays only in this tab.');
    primary.textContent = 'Unlock now';
    secondary.textContent = 'Use hardware key';
    setTrustedHtml('statusContext', renderHeroContext([
      { title: 'Passphrase path', body: 'Use this if you configured passphrase fallback or built a passphrase-only vault.' },
      { title: 'Hardware-key path', body: 'Use the hardware-key tab when a FIDO2 device is attached and you want threshold-based unlock.' },
      { title: 'Session scope', body: 'Unlock state lives in the daemon process and the session token lives only in this browser tab.' },
    ]));
    return;
  }

  const activeLabel = active ? (active.compartment_label || ('Compartment ' + active.compartment_id)) : 'No active compartment';
  setText('statusEyebrow', 'Unlocked workspace');
  setText('statusTitle', 'Local vault workspace');
  setText('statusSummary', 'The vault is unlocked. Use the workspace modes above to move between overview, access, vault, wallets, operations, and recovery without losing your place in one long page.');
  setText('heroModeValue', activeLabel);
  setText('heroModeDetail', unlocked.length > 1
    ? 'Multiple compartments are unlocked. Use the switcher in Access to choose which compartment new operations should target.'
    : 'One compartment is unlocked in this session. Additional compartments appear when their thresholds are met.');
  primary.textContent = 'Open secrets';
  secondary.textContent = 'Open profiles';
  setTrustedHtml('statusContext', renderHeroContext([
    { title: 'Protected values', body: 'Use Encrypted Secrets for sensitive data and Connection Keys for values the daemon needs during operator workflows.' },
    { title: 'Wallet families', body: 'Stealth wallets drive deposits and queue workflows today, while xpub receive wallets export public receive branches and preview deterministic addresses.' },
    { title: 'Operator loop', body: 'Deposits, queue, maintenance, snapshots, audit, and diagnostics now live in dedicated workspace modes instead of one scrolling operator page.' },
  ]));
}

function renderCompartmentSwitcher(unlocked, active) {
  const switcher = document.getElementById('compSwitcher');
  if (unlocked.length <= 1) {
    switcher.innerHTML = '';
    setHidden('compSwitcher', true);
    return;
  }

  let html = '';
  unlocked.forEach(c => {
    const isActive = active && active.compartment_id === c.id;
    html += '<button class="' + (isActive ? 'active' : '') + '" data-action="switchCompartment" data-arg0="' +
      escAttr(String(c.id)) + '" data-arg0-type="number">' + esc(c.label) + '</button>';
  });
  switcher.innerHTML = html;
  setHidden('compSwitcher', false);
}

function renderActiveCompartment(active, unlocked) {
  const compBadge = document.getElementById('compartmentBadge');
  if (active) {
    compBadge.textContent = active.compartment_label || ('Compartment ' + active.compartment_id);
    setHidden('compartmentBadge', false);
    setText('apiKeyCount', active.api_key_count || 0);
    setText('secretCount', active.secret_count != null ? active.secret_count : '(locked)');
  } else {
    setHidden('compartmentBadge', true);
    setText('apiKeyCount', '-');
    setText('secretCount', '-');
  }

  setText('compartmentCount', unlocked.length);
}

async function api(method, path, body) {
  return requestWithSession(method, path, body);
}

function toast(msg, type = 'success') {
  const el = document.createElement('div');
  el.className = 'toast toast-' + type;
  el.textContent = msg;
  el.setAttribute('role', 'status');
  el.setAttribute('aria-live', 'polite');
  document.body.appendChild(el);
  setTimeout(() => el.remove(), 3000);
}

const fido2Actions = createFido2Actions({
  api,
  toast,
  refresh: () => refresh(),
  currentStatus: () => currentStatus,
});

const setupWizard = createSetupWizard({
  api,
  toast,
  refresh: () => refresh(),
  submitNewFido2Pin: fido2Actions.submitNewFido2Pin,
  friendlyFidoError: fido2Actions.friendlyFidoError,
});

const walletActions = createWalletActions({
  api,
  toast,
  refresh: () => refresh(),
  copyText,
});

const inventoryActions = createInventoryActions({
  api,
  toast,
});

const operationsActions = createOperationsActions({
  api,
  toast,
  refresh: () => refresh(),
  showResultBox,
  updateNextStepCard: () => updateNextStepCard(),
});

const shellRenderer = createShellRenderer({
  operatorCardIds: OPERATOR_CARD_IDS,
  setUiMode: mode => { currentUiMode = mode; },
  setCardsHidden,
  setStatusBadge,
  setSecretsAccess,
  resetVaultCounts,
  setUnlockGuidance: fido2Actions.setUnlockGuidance,
  updateHeroState,
  updateWizardChrome: setupWizard.updateWizardChrome,
  resetSetupWizard: setupWizard.reset,
  renderCompartmentSwitcher,
  renderActiveCompartment,
  buildPushSelectors,
});

async function runRefreshCycle() {
  const s = await api('GET', '/api/status');
  currentStatus = s;
  const active = s.active_compartment;
  const unlocked = s.unlocked_compartments || [];
  const mode = deriveUiMode(s);

  // Not initialized — show setup wizard
  if (mode === 'setup') {
    shellRenderer.applySetupUi();
    syncSectionNav();
    setupWizard.wizDetectDevice();
    return;
  }

  setHidden('setupCard', true);
  setHidden('authCard', false);

  if (mode === 'locked') {
    shellRenderer.applyLockedUi();
    syncSectionNav();
    fido2Actions.showUnlockTabs();
    return;
  }

  shellRenderer.applyUnlockedUi(active, unlocked);

  await Promise.all([
    loadSecrets(),
    loadApiKeys(),
    walletActions.loadProfiles(),
    inventoryActions.loadInventoryOperations(),
    operationsActions.loadDepositRegistry(),
    operationsActions.loadQueueJobs(),
    fido2Actions.loadFido2(),
    loadCompartments(),
    loadAudit(),
    loadDiagnostics(),
  ]);
  updateNextStepCard();
  syncSectionNav();
}

async function refresh() {
  if (refreshPromise) {
    refreshQueued = true;
    return refreshPromise;
  }

  updateRefreshMeta('busy');
  refreshPromise = (async () => {
    try {
      await runRefreshCycle();
      markRefreshCompleted();
    } catch (e) {
      console.error('refresh failed', e);
      updateRefreshMeta('error');
    } finally {
      const rerun = refreshQueued;
      refreshQueued = false;
      refreshPromise = null;

      if (rerun) {
        void refresh();
      } else {
        scheduleRefresh(refresh);
      }
    }
  })();

  return refreshPromise;
}

function buildPushSelectors(unlocked) {
  const from = document.getElementById('pushFrom');
  const to = document.getElementById('pushTo');
  from.innerHTML = '';
  to.innerHTML = '';
  unlocked.forEach(c => {
    const opt1 = document.createElement('option');
    opt1.value = c.id;
    opt1.textContent = c.label + ' (#' + c.id + ')';
    from.appendChild(opt1);
    const opt2 = document.createElement('option');
    opt2.value = c.id;
    opt2.textContent = c.label + ' (#' + c.id + ')';
    to.appendChild(opt2);
  });
  if (unlocked.length > 1) to.selectedIndex = 1;
}

async function switchCompartment(id) {
  const r = await api('POST', '/api/compartment/switch', { id });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Switched to compartment #' + id);
  refresh();
}

async function pushSecret() {
  const from = parseInt(document.getElementById('pushFrom').value);
  const to = parseInt(document.getElementById('pushTo').value);
  const key = document.getElementById('pushKey').value;
  const newKey = document.getElementById('pushNewKey').value || null;
  const tier = parseInt(document.getElementById('pushTier').value);
  if (!key) { toast('Key name required', 'error'); return; }
  if (from === to) { toast('Source and target must differ', 'error'); return; }
  const r = await api('POST', '/api/secrets/push', {
    from_compartment: from, to_compartment: to, key, new_key: newKey, tier,
  });
  if (r.error) { toast(r.error, 'error'); return; }
  document.getElementById('pushKey').value = '';
  document.getElementById('pushNewKey').value = '';
  toast('Secret pushed');
  refresh();
}

async function loadCompartments() {
  try {
    const r = await api('GET', '/api/compartment/list');
    const el = document.getElementById('compartmentList');
    const comps = r.compartments || [];
    if (comps.length === 0) {
      el.innerHTML = '<p class="text-meta">No compartments available.</p>';
      return;
    }
    let html = '<ul class="key-list">';
    comps.forEach(c => {
      const active = c.is_active ? ' <span style="color:var(--success);">(active)</span>' : '';
      html += '<li><span>' + esc(c.label) + ' <span style="color:var(--text-dim);font-size:11px;">' +
        'threshold=' + c.threshold + active + '</span></span></li>';
    });
    html += '</ul>';
    el.innerHTML = html;
  } catch(e) {}
}

async function copyText(value, label) {
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(value);
      toast(label + ' copied');
      return;
    }
  } catch (_) {}
  window.prompt('Copy ' + label + ':', value);
}

async function unlock() {
  const p = document.getElementById('passphrase').value;
  if (!p) return;
  const r = await api('POST', '/api/unlock', { passphrase: p });
  if (r.error) {
    if (isAlreadyUnlockedConflict(r.error)) {
      toast('Session already active. Refreshing workspace…');
      await refresh();
      return;
    }
    toast(r.error, 'error');
    return;
  }
  document.getElementById('passphrase').value = '';
  if (r.unlocked_compartments && r.unlocked_compartments.length > 0) {
    const labels = r.unlocked_compartments.map(c => c.label).join(', ');
    toast('Unlocked: ' + labels);
  } else {
    toast('Unlocked');
  }
  refresh();
}

async function lock() {
  if (!confirm('Lock all compartments? Master keys will be zeroized from memory.')) return;
  const r = await api('POST', '/api/lock');
  if (r.error) { toast(r.error, 'error'); return; }
  clearSessionToken();
  toast('All compartments locked');
  refresh();
}

async function logoutSession() {
  const r = await api('POST', '/api/session/revoke');
  if (r.error) { toast(r.error, 'error'); return; }
  clearSessionToken();
  toast('Session logged out');
  refresh();
}

// ── API Keys & Secrets ────────────────────────────────────────

async function loadApiKeys() {
  try {
    const r = await api('GET', '/api/api-keys');
    if (r.error) return;
    const list = document.getElementById('apiKeyList');
    lastApiKeys = r.keys || [];
    list.innerHTML = '';
    if (lastApiKeys.length === 0) {
      list.innerHTML = '<li><span class="helper-text">No connection keys yet. Store the first RPC or auth token above so provider-backed workflows can run locally.</span></li>';
      return;
    }
    lastApiKeys.forEach(k => {
      const li = document.createElement('li');
      li.innerHTML = '<span>' + esc(k) + '</span>' +
        '<div class="key-actions">' +
        '<button class="btn-ghost" data-action="revealApiKeyButton" data-arg0="' + escAttr(k) + '" data-self="append">Reveal</button>' +
        '<button class="btn-danger" data-action="deleteApiKey" data-arg0="' + escAttr(k) + '">Delete</button></div>';
      list.appendChild(li);
    });
  } catch(e) {}
}

async function loadSecrets() {
  try {
    const r = await api('GET', '/api/secrets');
    if (r.error) return;
    const list = document.getElementById('secretList');
    lastSecretKeys = r.keys || [];
    list.innerHTML = '';
    if (lastSecretKeys.length === 0) {
      list.innerHTML = '<li><span class="helper-text">No encrypted secrets yet. Store the first protected value above to confirm the vault flow end to end.</span></li>';
      return;
    }
    lastSecretKeys.forEach(k => {
      const li = document.createElement('li');
      li.innerHTML = '<span>' + esc(k) + '</span>' +
        '<div class="key-actions">' +
        '<button class="btn-ghost" data-action="revealSecretButton" data-arg0="' + escAttr(k) + '" data-self="append">Reveal</button>' +
        '<button class="btn-danger" data-action="deleteSecret" data-arg0="' + escAttr(k) + '">Delete</button></div>';
      list.appendChild(li);
    });
  } catch(e) {}
}

async function setApiKey() {
  const key = document.getElementById('apiKeyName').value;
  const value = document.getElementById('apiKeyValue').value;
  if (!key || !value) { toast('Key and value required', 'error'); return; }
  const r = await api('POST', '/api/api-keys/set', { key, value });
  if (r.error) { toast(r.error, 'error'); return; }
  clearFields(['apiKeyName', 'apiKeyValue']);
  toast('API key stored');
  refresh();
}

async function setSecret() {
  const key = document.getElementById('secretName').value;
  const value = document.getElementById('secretValue').value;
  if (!key || !value) { toast('Key and value required', 'error'); return; }
  const r = await api('POST', '/api/secrets/set', { key, value });
  if (r.error) { toast(r.error, 'error'); return; }
  clearFields(['secretName', 'secretValue']);
  toast('Secret stored');
  refresh();
}

function showRevealedValue(li, btn, value) {
  let existing = li.querySelector('.secret-value');
  if (existing) { existing.remove(); btn.textContent = 'Reveal'; return; }
  const div = document.createElement('div');
  div.className = 'secret-value';
  div.textContent = value;
  li.appendChild(div);
  btn.textContent = 'Hide';
  setTimeout(() => {
    const el = li.querySelector('.secret-value');
    if (el) { el.remove(); btn.textContent = 'Reveal'; }
  }, 30000);
}

async function revealApiKey(key, btn) {
  const r = await api('POST', '/api/api-keys/get', { key });
  if (r.error) { toast(r.error, 'error'); return; }
  showRevealedValue(btn.closest('li'), btn, r.value);
}

async function revealSecret(key, btn) {
  const r = await api('POST', '/api/secrets/get', { key });
  if (r.error) { toast(r.error, 'error'); return; }
  showRevealedValue(btn.closest('li'), btn, r.value);
}

function revealApiKeyButton(key, btn) {
  return revealApiKey(key, btn);
}

function revealSecretButton(key, btn) {
  return revealSecret(key, btn);
}

async function deleteApiKey(key) {
  if (!confirm('Delete API key "' + key + '"?')) return;
  const r = await api('POST', '/api/api-keys/delete', { key });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Deleted');
  refresh();
}

async function deleteSecret(key) {
  if (!confirm('Delete secret "' + key + '"?')) return;
  const r = await api('POST', '/api/secrets/delete', { key });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Deleted');
  refresh();
}

function bytesToHex(bytes) {
  return Array.from(bytes, b => b.toString(16).padStart(2, '0')).join('');
}

function hexToBytes(hex) {
  if (hex.length % 2 !== 0) throw new Error('Invalid hex length');
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < hex.length; i += 2) {
    out[i / 2] = parseInt(hex.slice(i, i + 2), 16);
  }
  return out;
}

async function exportSnapshot() {
  const passphrase = document.getElementById('backupExportPass').value;
  if (!passphrase || passphrase.length < 8) {
    toast('Export passphrase must be at least 8 characters', 'error');
    return;
  }

  const r = await api('POST', '/api/backup/export', { passphrase });
  if (r.error) { toast(r.error, 'error'); return; }

  try {
    const bytes = hexToBytes(r.snapshot_hex);
    const blob = new Blob([bytes], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'sigillum-snapshot-' + (r.summary?.created_at_unix || Date.now()) + '.json';
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
    document.getElementById('backupExportPass').value = '';
    toast('Snapshot downloaded');
  } catch (e) {
    toast('Failed to prepare snapshot download', 'error');
  }
}

async function restoreSnapshot(
  fileInputId = 'backupRestoreFile',
  passphraseId = 'backupRestorePass',
  successMessage = 'Snapshot restored. Unlock the vault again.'
) {
  const fileInput = document.getElementById(fileInputId);
  const passphraseInput = document.getElementById(passphraseId);
  if (!fileInput || !passphraseInput) {
    toast('Snapshot restore controls are unavailable.', 'error');
    return;
  }
  const passphrase = passphraseInput.value;
  const file = fileInput.files && fileInput.files[0];
  if (!file) { toast('Choose a snapshot file', 'error'); return; }
  if (!passphrase || passphrase.length < 8) {
    toast('Restore passphrase must be at least 8 characters', 'error');
    return;
  }
  if (!confirm('Restore this snapshot? Current on-disk Sigillum data will be replaced and you will need to unlock again.')) return;

  let snapshotHex;
  try {
    const bytes = new Uint8Array(await file.arrayBuffer());
    snapshotHex = bytesToHex(bytes);
  } catch (e) {
    toast('Failed to read snapshot file', 'error');
    return;
  }

  const r = await api('POST', '/api/backup/restore', {
    passphrase,
    snapshot_hex: snapshotHex,
  });
  if (r.error) { toast(r.error, 'error'); return; }

  clearSessionToken();
  passphraseInput.value = '';
  fileInput.value = '';
  toast(successMessage);
  refresh();
}

function restoreSetupSnapshot() {
  return restoreSnapshot(
    'setupRestoreFile',
    'setupRestorePass',
    'Snapshot restored. Continue setup or unlock the restored vault.'
  );
}

function restoreAuthSnapshot() {
  return restoreSnapshot(
    'authRestoreFile',
    'authRestorePass',
    'Snapshot restored. Unlock the restored vault to continue.'
  );
}

async function resetLocalData(confirmId = 'setupResetConfirm') {
  const confirmInput = document.getElementById(confirmId);
  if (!confirmInput) {
    toast('Reset controls are unavailable.', 'error');
    return;
  }
  const confirmation = confirmInput.value.trim();
  if (confirmation !== SETUP_RESET_CONFIRMATION) {
    toast("Type '" + SETUP_RESET_CONFIRMATION + "' exactly to continue.", 'error');
    return;
  }
  if (!confirm('Erase all local Sigillum data on this machine and return to first-run setup?')) {
    return;
  }

  const r = await api('POST', '/api/setup/reset', { confirmation });
  if (r.error) { toast(r.error, 'error'); return; }

  clearSessionToken();
  clearFields([
    'setupResetConfirm',
    'authResetConfirm',
    'backupResetConfirm',
    'setupRestorePass',
    'authRestorePass',
    'backupRestorePass',
  ]);
  ['setupRestoreFile', 'authRestoreFile', 'backupRestoreFile'].forEach(id => {
    const el = document.getElementById(id);
    if (el) el.value = '';
  });
  toast('Local Sigillum data cleared. You can start setup again or restore a snapshot.');
  refresh();
}

function formatAuditEvent(event) {
  const details = event.details || {};
  const labels = {
    'unlock.passphrase': 'Unlocked with passphrase',
    'unlock.fido2': 'Unlocked with FIDO2',
    'lock.all': 'Locked all compartments',
    'session.revoke': 'Revoked session',
    'compartment.add': 'Added compartment',
    'compartment.init': 'Initialized compartment',
    'compartment.remove': 'Removed compartment',
    'compartment.switch': 'Switched compartment',
    'api_key.set': 'Stored API key',
    'api_key.delete': 'Deleted API key',
    'secret.set': 'Stored encrypted secret',
    'secret.delete': 'Deleted encrypted secret',
    'secret.push': 'Pushed secret between compartments',
    'profiles.eth_xpub_wallet.upsert': 'Saved xpub wallet profile',
    'profiles.eth_xpub_wallet.delete': 'Deleted xpub wallet profile',
    'profiles.eth_seed_wallet.upsert': 'Imported seed wallet profile',
    'profiles.eth_seed_wallet.delete': 'Deleted seed wallet profile',
    'wallet_inventory.risk_catalog.upsert': 'Saved risk catalog entry',
    'wallet_inventory.risk_catalog.delete': 'Deleted risk catalog entry',
    'wallet.eth_xpub.export': 'Exported xpub receive branch',
    'fido2.setup': 'Completed FIDO2 setup',
    'fido2.register': 'Registered FIDO2 key',
    'fido2.register_poison': 'Registered poison FIDO2 key',
    'fido2.remove': 'Removed FIDO2 key',
    'snapshot.export': 'Exported encrypted snapshot',
    'snapshot.restore': 'Restored encrypted snapshot',
  };
  let suffix = '';
  if (details.label) suffix = ' - ' + details.label;
  else if (details.key) suffix = ' - ' + details.key;
  else if (details.name) suffix = ' - ' + details.name;
  else if (details.address) suffix = ' - ' + details.address;
  else if (details.wallet_profile) suffix = ' - ' + details.wallet_profile;
  else if (details.compartment_count) suffix = ' - ' + details.compartment_count + ' compartments';
  else if (details.count) suffix = ' - ' + details.count + ' compartments';
  else if (details.file_count) suffix = ' - ' + details.file_count + ' files';
  return (labels[event.kind] || event.kind) + suffix;
}

async function loadAudit() {
  try {
    const r = await api('GET', '/api/audit?limit=20');
    if (r.error) return;
    const list = document.getElementById('auditList');
    const events = r.events || [];
    if (events.length === 0) {
      list.innerHTML = '<p class="text-meta">No audit events yet.</p>';
      return;
    }
    let html = '<ul class="key-list">';
    events.forEach(event => {
      const when = new Date((event.created_at_unix || 0) * 1000).toLocaleString();
      const comp = event.compartment_id != null
        ? '<span style="color:var(--text-dim);font-size:11px;">compartment #' + event.compartment_id + '</span>'
        : '<span style="color:var(--text-dim);font-size:11px;">global</span>';
      html += '<li><span>' + esc(formatAuditEvent(event)) +
        '<div style="color:var(--text-dim);font-size:11px;margin-top:4px;">' +
        esc(when) + ' · ' + comp + '</div></span></li>';
    });
    html += '</ul>';
    list.innerHTML = html;
  } catch (e) {}
}

async function loadDiagnostics() {
  try {
    const r = await api('GET', '/api/diagnostics');
    const el = document.getElementById('diagGrid');
    if (r.error) {
      el.innerHTML = '<div style="color:var(--danger);font-size:13px;">' + esc(r.error) + '</div>';
      return;
    }
    const started = r.started_at_unix ? new Date(r.started_at_unix * 1000).toLocaleString() : '-';
    el.innerHTML = [
      statBox(r.version || '-', 'Version'),
      statBox(r.unlock_scope || '-', 'Unlock Scope'),
      statBox(r.session_scope || '-', 'Session Scope'),
      statBox(String(r.active_session_count ?? 0), 'Sessions'),
      statBox(String(r.unlocked_compartment_count ?? 0), 'Unlocked'),
      statBox(r.max_unlocked_threshold != null ? String(r.max_unlocked_threshold) : '-', 'Max Threshold'),
      statBox(r.default_active_compartment_id != null ? String(r.default_active_compartment_id) : '-', 'Default Active'),
      statBox(r.audit_log_present ? 'yes' : 'no', 'Audit Log'),
      statBox(String(r.pending_operation_count ?? 0), 'Pending Ops'),
      statBox(String(r.startup_interrupted_operation_count ?? 0), 'Interrupted Ops'),
      statBox(String(r.startup_recovered_operation_count ?? 0), 'Recovered Ops'),
      statBox(String(r.startup_unresolved_operation_count ?? 0), 'Unresolved Ops'),
      statBox(String(r.queue_job_count ?? 0), 'Queue Jobs'),
      statBox(String(r.blocked_queue_job_count ?? 0), 'Blocked Jobs'),
      statBox(String(r.retrying_queue_job_count ?? 0), 'Retrying Jobs'),
      statBox(String(r.failed_queue_job_count ?? 0), 'Failed Jobs'),
      statBox(String(r.startup_recovered_queue_job_count ?? 0), 'Recovered Jobs'),
      statBox(String(r.startup_reconciled_deposit_count ?? 0), 'Recovered Deposits'),
      statBox(
        (r.runtime_policy?.queue_default_process_limit ?? '-') + '/' + (r.runtime_policy?.queue_max_process_limit ?? '-'),
        'Queue Limit'
      ),
      statBox(
        (r.runtime_policy?.deposit_default_refresh_limit ?? '-') + '/' + (r.runtime_policy?.deposit_max_refresh_limit ?? '-'),
        'Refresh Limit'
      ),
      statBox(
        (r.runtime_policy?.queue_retry_base_delay_secs ?? '-') + '/' + (r.runtime_policy?.queue_retry_max_delay_secs ?? '-'),
        'Retry Backoff'
      ),
      statBox(String(r.runtime_policy?.provider_balance_observation_concurrency ?? '-'), 'RPC Concurrency'),
      statBox(
        (r.runtime_policy?.audit_default_limit ?? '-') + '/' + (r.runtime_policy?.audit_max_limit ?? '-'),
        'Audit Limit'
      ),
      statBox(String(r.eth_stealth_deposit_count ?? 0), 'Deposits'),
      statBox(r.initialized ? 'yes' : 'no', 'Initialized'),
      statBox(started, 'Started'),
    ].join('');
  } catch (e) {}
}

const UI_ACTIONS = {
  approveConsolidationPlan: inventoryActions.approveConsolidationPlan,
  cancelDiscoveryJob: inventoryActions.cancelDiscoveryJob,
  copyText,
  createErc20Deposit: operationsActions.createErc20Deposit,
  createNativeDeposit: operationsActions.createNativeDeposit,
  deleteApiKey,
  deleteChainProfile: inventoryActions.deleteChainProfile,
  deleteRiskCatalogEntry: inventoryActions.deleteRiskCatalogEntry,
  deleteDeposit: operationsActions.deleteDeposit,
  deleteProviderProfile: walletActions.deleteProviderProfile,
  deleteSecret,
  deleteSeedWalletProfile: walletActions.deleteSeedWalletProfile,
  deleteWalletProfile: walletActions.deleteWalletProfile,
  deleteXpubWalletProfile: walletActions.deleteXpubWalletProfile,
  enqueueDepositSweep: operationsActions.enqueueDepositSweep,
  exportSelectedXpubWallet: walletActions.exportSelectedXpubWallet,
  exportSnapshot,
  exportWalletMeta: walletActions.exportWalletMeta,
  exportXpubWalletProfile: walletActions.exportXpubWalletProfile,
  fido2Register: fido2Actions.fido2Register,
  fido2RemoveKey: fido2Actions.fido2RemoveKey,
  fido2SetNewPin: fido2Actions.fido2SetNewPin,
  fido2Unlock: fido2Actions.fido2Unlock,
  generateConsolidationPlan: inventoryActions.generateConsolidationPlan,
  heroPrimaryAction,
  heroSecondaryAction,
  loadQueueJobs: operationsActions.loadQueueJobs,
  loadRiskFindings: inventoryActions.loadRiskFindings,
  lock,
  logoutSession,
  nextStepPrimaryAction,
  nextStepSecondaryAction,
  previewXpubReceiveAddress: walletActions.previewXpubReceiveAddress,
  processQueueBatch: operationsActions.processQueueBatch,
  processQueueJob: operationsActions.processQueueJob,
  pushSecret,
  refreshDepositRegistry: operationsActions.refreshDepositRegistry,
  refreshSingleDeposit: operationsActions.refreshSingleDeposit,
  resetLocalData,
  restoreAuthSnapshot,
  restoreSetupSnapshot,
  restoreSnapshot,
  resumeDiscoveryJob: inventoryActions.resumeDiscoveryJob,
  revealApiKeyButton,
  revealSecretButton,
  runMaintenanceCycle: operationsActions.runMaintenanceCycle,
  scanEthStealthAnnouncements: operationsActions.scanEthStealthAnnouncements,
  scanInventoryEvm: inventoryActions.scanInventoryEvm,
  simulateConsolidationPlan: inventoryActions.simulateConsolidationPlan,
  selectWorkspaceSection,
  setApiKey,
  setSecret,
  switchCompartment,
  switchUnlockTab: fido2Actions.switchUnlockTab,
  togglePoisonWarning: fido2Actions.togglePoisonWarning,
  unlock,
  upsertChainProfile: inventoryActions.upsertChainProfile,
  upsertRiskCatalogEntry: inventoryActions.upsertRiskCatalogEntry,
  upsertProviderProfile: walletActions.upsertProviderProfile,
  upsertSeedWalletProfile: walletActions.upsertSeedWalletProfile,
  upsertWalletProfile: walletActions.upsertWalletProfile,
  upsertXpubWalletProfile: walletActions.upsertXpubWalletProfile,
  wizAddCustomComp: setupWizard.wizAddCustomComp,
  wizBackFromFido2Pin: setupWizard.wizBackFromFido2Pin,
  wizBackToPresets: setupWizard.wizBackToPresets,
  wizDetectDevice: setupWizard.wizDetectDevice,
  wizFinishForNow: setupWizard.wizFinishForNow,
  wizInitPassphrase: setupWizard.wizInitPassphrase,
  wizPreset: setupWizard.wizPreset,
  wizProceedFido2: setupWizard.wizProceedFido2,
  wizRegisterAdditionalKey: setupWizard.wizRegisterAdditionalKey,
  wizRegisterKey: setupWizard.wizRegisterKey,
  wizSetAdditionalKeyPin: setupWizard.wizSetAdditionalKeyPin,
  wizSetNewPin: setupWizard.wizSetNewPin,
};

function handleActionEvent(event) {
  handleDispatchedActionEvent(event, {
    actions: UI_ACTIONS,
    toast,
    quietActions: ['selectWorkspaceSection', 'switchUnlockTab', 'togglePoisonWarning'],
  });
}

document.addEventListener('keydown', e => {
  if (e.key !== 'Enter') return;
  if (e.target.id === 'passphrase') unlock();
  if (e.target.id === 'fido2Pin') fido2Actions.fido2Unlock();
  if (e.target.id === 'wizPassphraseConfirm') setupWizard.wizInitPassphrase();
  if (e.target.id === 'wizFido2Label') setupWizard.wizRegisterKey();
});

document.addEventListener('visibilitychange', () => {
  if (shouldAutoRefresh()) {
    void refresh();
  } else {
    clearRefreshTimer();
    updateRefreshMeta();
  }
});

document.addEventListener('click', handleActionEvent);
window.addEventListener('beforeunload', clearRefreshTimer);

function enhanceUiChrome() {
  document.querySelectorAll('button:not([type])').forEach(button => {
    button.type = 'button';
  });
  document.querySelectorAll('input[placeholder]:not([aria-label])').forEach(input => {
    input.setAttribute('aria-label', input.getAttribute('placeholder'));
  });
  document.querySelectorAll('select:not([aria-label])').forEach(select => {
    const label = select.closest('.card')?.querySelector('h2')?.textContent || select.id || 'Select option';
    select.setAttribute('aria-label', label);
  });
  document.querySelectorAll('main .card').forEach(card => {
    if (!card.hasAttribute('tabindex')) card.setAttribute('tabindex', '-1');
  });
  document
    .querySelectorAll('input[type="text"], input[type="password"], input[type="number"], textarea')
    .forEach(input => {
      input.autocomplete = 'off';
      input.spellcheck = false;
      input.setAttribute('autocapitalize', 'off');
    });
}

enhanceUiChrome();
void refresh();
