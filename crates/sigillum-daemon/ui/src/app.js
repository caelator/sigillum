
const API = '';
const SETUP_RESET_CONFIRMATION = 'RESET LOCAL SIGILLUM DATA';
const SESSION_TOKEN_KEY = 'sigillumSessionToken';
const REFRESH_INTERVAL_MS = 5000;
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
let wizCompartments = [];
let customCompartments = [];
let wizRequiredKeyCount = 1;
let wizRegisteredKeyCount = 0;
let wizPrimaryKeyLabel = '';
let lastRefreshAt = null;
let refreshPromise = null;
let refreshQueued = false;
let refreshTimer = null;
let lastFidoDetect = null;
let lastProviderProfiles = [];
let lastWalletProfiles = [];
let lastXpubWalletProfiles = [];
let lastSeedWalletProfiles = [];
let lastApiKeys = [];
let lastSecretKeys = [];
let lastDeposits = [];
let lastQueueJobs = [];
let lastFidoKeys = [];
let nextStepPrimaryTarget = null;
let nextStepSecondaryTarget = null;
let volatileSessionToken = null;

try {
  activeWorkspaceSection =
    window.sessionStorage.getItem(WORKSPACE_SECTION_KEY) || 'overview';
} catch (_) {}

const WIZARD_CHROME = {
  wizStep0: {
    pill: 'Step 1 of 3',
    title: 'Choose a protection model',
    summary: 'Pick the starting vault shape that best matches your risk level and how many hardware keys you want to manage.',
    checklist: [
      'Choose the plan that matches how many access layers you want on this machine.',
      'If you want hardware-key setup, attach a FIDO2 key and make sure you know its PIN.',
      'Decide whether you also want a fallback passphrase for local recovery.',
    ],
  },
  wizStepPassphrase: {
    pill: 'Step 2 of 3',
    title: 'Create your first local compartment',
    summary: 'Choose the compartment name and passphrase that will protect the first vault on this machine.',
    checklist: [
      'Pick the compartment name you want to see later in the unlocked workspace.',
      'Create a passphrase with at least 8 characters.',
      'Confirm it once so Sigillum can initialize the vault cleanly.',
    ],
  },
  wizStepCompartments: {
    pill: 'Step 2 of 3',
    title: 'Review what Sigillum will create',
    summary: 'Thresholds tell Sigillum how many distinct hardware keys are required to unlock each compartment.',
    checklist: [
      'Review each compartment label and the threshold attached to it.',
      'Use higher thresholds only for workflows that need stronger separation.',
      'Continue when the compartment plan matches your local operating model.',
    ],
  },
  wizStepCustomComps: {
    pill: 'Step 2 of 3',
    title: 'Design custom access layers',
    summary: 'Define the compartments you want first, then continue to hardware-key registration.',
    checklist: [
      'Add each compartment you want Sigillum to create in this local vault.',
      'Choose a unique threshold for every custom access layer.',
      'Continue once the list matches the workflow split you actually want.',
    ],
  },
  wizStepFido2Pin: {
    pill: 'Step 3 of 3',
    title: 'Register your first hardware key',
    summary: 'Leave the PIN blank for touch-only authenticators, or enter the current PIN for keys that require one, then give the key a label you will recognize later.',
    checklist: [
      'Leave the PIN field empty for touch-only keys, or enter the current FIDO2 PIN if the inserted key already requires one.',
      'Use a label you will recognize later, such as primary.',
      'Optionally set one fallback passphrase for local recovery if the keys are unavailable.',
    ],
  },
  wizStepAdditionalKeys: {
    pill: 'Add backup keys',
    title: 'Finish enrolling the keys this plan expects',
    summary: 'Higher-threshold compartments only become usable once enough distinct hardware keys are enrolled.',
    checklist: [
      'Insert the next hardware key you want to trust for this vault.',
      'Leave the PIN field empty for touch-only keys, or set and enter the current FIDO2 PIN only if you want that backup key to require one.',
      'Finish for now only if you are comfortable leaving the higher-threshold compartments unavailable until you add more keys later.',
    ],
  },
  wizStepTouch: {
    pill: 'Finishing setup',
    title: 'Complete the hardware-key touch',
    summary: 'Sigillum is waiting for a successful FIDO2 registration touch before it can finish the vault setup.',
    checklist: [
      'Keep the hardware key connected to this machine.',
      'Touch the device when it prompts for confirmation.',
      'Stay on this page until Sigillum confirms the vault is ready.',
    ],
  },
  wizStepDone: {
    pill: 'Vault ready',
    title: 'You can start using the daemon now',
    summary: 'Unlock the vault, store your first secret, and add more keys or operator profiles when you are ready.',
    checklist: [
      'Unlock once to confirm the setup behaves the way you expect.',
      'Store a first secret or connection key so the workspace becomes useful immediately.',
      'Add more keys, profiles, or deposits whenever you are ready for the next workflow.',
    ],
  },
};

function getSessionToken() {
  try {
    return window.sessionStorage.getItem(SESSION_TOKEN_KEY) || volatileSessionToken;
  } catch (_) {
    return volatileSessionToken;
  }
}

function setSessionToken(token) {
  if (!token) return;
  volatileSessionToken = token;
  try { window.sessionStorage.setItem(SESSION_TOKEN_KEY, token); }
  catch (_) {}
}

function clearSessionToken() {
  volatileSessionToken = null;
  try { window.sessionStorage.removeItem(SESSION_TOKEN_KEY); }
  catch (_) {}
}

function setHidden(id, hidden) {
  const el = document.getElementById(id);
  if (el) el.classList.toggle('hidden', hidden);
}

function setCardsHidden(ids, hidden) {
  ids.forEach(id => setHidden(id, hidden));
}

function setText(id, value) {
  const el = document.getElementById(id);
  if (el) el.textContent = value;
}

function setTrustedHtml(id, value) {
  const el = document.getElementById(id);
  if (el) el.innerHTML = value;
}

function setStatusBadge(className, label) {
  const badge = document.getElementById('statusBadge');
  badge.className = 'status-badge ' + className;
  badge.textContent = label;
}

function formatRefreshTime(date) {
  return date.toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

function setRefreshMeta(label, state) {
  const el = document.getElementById('refreshMeta');
  if (!el) return;
  el.textContent = label;
  el.dataset.state = state;
}

function shouldAutoRefresh() {
  return document.visibilityState === 'visible';
}

function clearRefreshTimer() {
  if (!refreshTimer) return;
  clearTimeout(refreshTimer);
  refreshTimer = null;
}

function updateRefreshMeta(stateOverride) {
  if (stateOverride === 'busy') {
    setRefreshMeta('Syncing', 'busy');
    return;
  }
  if (stateOverride === 'error') {
    setRefreshMeta('Connection issue', 'error');
    return;
  }
  const prefix = shouldAutoRefresh() ? 'Live' : 'Paused';
  const label = lastRefreshAt
    ? prefix + ' · ' + formatRefreshTime(lastRefreshAt)
    : prefix;
  setRefreshMeta(label, shouldAutoRefresh() ? 'live' : 'paused');
}

function scheduleRefresh() {
  clearRefreshTimer();
  if (!shouldAutoRefresh()) {
    updateRefreshMeta();
    return;
  }
  updateRefreshMeta();
  refreshTimer = setTimeout(() => {
    void refresh();
  }, REFRESH_INTERVAL_MS);
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
    switchUnlockTab('fido2');
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

function renderChecklist(items) {
  return items.map((item, index) =>
    '<div class="checklist-item"><div class="checklist-mark">' +
      String(index + 1).padStart(2, '0') +
      '</div><div>' + esc(item) + '</div></div>'
  ).join('');
}

function renderNextStepItems(items) {
  return items.map(item =>
    '<div class="next-step-item"><strong>' + esc(item.title) + '</strong><span>' + esc(item.body) + '</span></div>'
  ).join('');
}

function setUnlockGuidance(mode) {
  const el = document.getElementById('authGuidance');
  if (!el) return;

  if (mode === 'session') {
    el.textContent = 'Lock All zeroizes unlocked vault material from daemon memory. Log Out Session only clears the browser session token and leaves daemon state unchanged.';
    return;
  }

  if (mode === 'fido2') {
    const deviceLine = lastFidoDetect && lastFidoDetect.device_present
      ? lastFidoDetect.device_count + ' hardware key(s) detected. '
      : '';
    el.textContent = deviceLine + 'Set tap count to the number of distinct key touches Sigillum should wait for. Leave the PIN field blank for touch-only authenticators, or enter the current PIN only for keys that require one. For a two-key compartment, enter 2 and touch two enrolled keys in sequence.';
    return;
  }

  el.textContent = 'Passphrase unlock works for passphrase-only vaults or fallback access. A successful unlock reveals the protected workspace in this browser tab only.';
}

function friendlyFidoError(message) {
  const raw = String(message || '');
  const normalized = raw.toLowerCase();

  if (
    normalized.includes('pin_required') ||
    normalized.includes('pin is required for the selected operation')
  ) {
    return 'This hardware key is configured to require its current FIDO2 PIN. Enter that PIN and retry, or use a touch-only key.';
  }
  if (
    normalized.includes('pin_not_set') ||
    normalized.includes('no pin has been set') ||
    normalized.includes('no fido2 pin is configured')
  ) {
    return 'This hardware key does not have a FIDO2 PIN yet. Leave the PIN field empty to keep using touch-only access, or use Set key PIN first if you want this key to require one.';
  }
  if (
    normalized.includes('pin already set') ||
    normalized.includes('already has a fido2 pin') ||
    normalized.includes('already has a pin configured')
  ) {
    return 'This hardware key already has a FIDO2 PIN. Enter the existing PIN when you use this key.';
  }
  if (
    normalized.includes('pin_policy') ||
    normalized.includes('pin policy') ||
    normalized.includes('at least 4 characters')
  ) {
    return 'That new FIDO2 PIN does not meet the key policy. Use at least 4 characters and avoid unsupported patterns.';
  }
  if (
    normalized.includes('pin_auth_blocked') ||
    normalized.includes('pin authentication is temporarily blocked') ||
    normalized.includes('power recycle')
  ) {
    return 'This hardware key has temporarily blocked PIN authentication. Unplug and reinsert the key, then retry with the correct PIN.';
  }
  if (
    normalized.includes('pin_blocked') ||
    normalized.includes('pin is fully blocked') ||
    normalized.includes('fully blocked on the hardware key')
  ) {
    return 'This hardware key has fully blocked PIN attempts. Recover or reset the key with vendor tooling before trying again.';
  }
  if (normalized.includes('incorrect pin') || normalized.includes('pin_invalid')) {
    return 'The hardware key rejected that PIN. Re-enter the current PIN carefully, or use a touch-only key if that is the policy you want.';
  }
  if (
    normalized.includes('cannot tell which one to use') ||
    normalized.includes('leave only the target key inserted')
  ) {
    return 'More than one hardware key is attached and this step needs a specific target. Leave only the key you want Sigillum to act on, then retry.';
  }
  if (
    normalized.includes('already appears to be registered') ||
    normalized.includes('insert the new key you want to add')
  ) {
    return 'The attached hardware keys all look like ones Sigillum already knows. Insert the new key you want to add, then retry.';
  }
  if (
    normalized.includes('matched the sigillum credential needed for this step') ||
    normalized.includes('matched the expected sigillum credential')
  ) {
    return 'The attached hardware keys do not include the enrolled key Sigillum expected for this step. Keep the required registered key connected and retry.';
  }
  if (normalized.includes('no fido2 device') || normalized.includes('no device')) {
    return 'No FIDO2 hardware key is currently available. Insert the key and try again.';
  }
  if (normalized.includes('timeout')) {
    return 'Sigillum timed out while waiting for the hardware key. Keep it connected and touch it when prompted, then try again.';
  }
  if (normalized.includes('ctap1 device') || normalized.includes('hmac-secret')) {
    return 'This key does not support the FIDO2 features Sigillum requires. Use a CTAP2 key with hmac-secret support.';
  }
  if (normalized.includes('clientpin support')) {
    return 'This key does not expose PIN management in a way Sigillum can use. Set the PIN with the vendor tool, then return here.';
  }
  return raw;
}

function isAlreadyUnlockedConflict(message) {
  return String(message || '').toLowerCase().includes('already unlocked');
}

function setInlineInfo(id, message, tone = 'error') {
  const el = document.getElementById(id);
  if (!el) return;
  const color = tone === 'success'
    ? 'var(--success)'
    : tone === 'warning'
      ? 'var(--warning)'
      : 'var(--danger)';
  el.textContent = message;
  el.style.color = color;
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
  const hasStealthWalletProfiles = lastWalletProfiles.length > 0;
  const hasXpubWalletProfiles = lastXpubWalletProfiles.length > 0;
  const hasSeedWalletProfiles = lastSeedWalletProfiles.length > 0;

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
  } else if (lastProviderProfiles.length === 0) {
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
  } else if (lastFidoKeys.length === 1) {
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
  } else if (lastDeposits.length === 0) {
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
  } else if (lastQueueJobs.length > 0) {
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

function applySetupUi() {
  currentUiMode = 'setup';
  document.body.dataset.mode = 'setup';
  wizRequiredKeyCount = 1;
  wizRegisteredKeyCount = 0;
  wizPrimaryKeyLabel = '';
  clearSessionToken();
  setStatusBadge('status-no-vault', 'NO VAULT');
  setHidden('compartmentBadge', true);
  setHidden('setupCard', false);
  setHidden('authCard', true);
  setCardsHidden(OPERATOR_CARD_IDS, true);
  setSecretsAccess(false);
  resetVaultCounts();
  setUnlockGuidance('passphrase');
  updateHeroState('setup');
  updateWizardChrome(document.querySelector('.wizard-step.active')?.id || 'wizStep0');
}

function applyLockedUi() {
  currentUiMode = 'locked';
  document.body.dataset.mode = 'locked';
  clearSessionToken();
  setStatusBadge('status-locked', 'LOCKED');
  setHidden('compartmentBadge', true);
  setCardsHidden(OPERATOR_CARD_IDS, true);
  resetVaultCounts();
  setHidden('lockForm', true);
  setHidden('authRecovery', false);
  setHidden('compSwitcher', true);
  setText('authTitle', 'Unlock this local session');
  setText('authLead', 'Unlock with the passphrase or hardware-key threshold you configured during setup. The resulting session token stays only in this browser tab.');
  setSecretsAccess(false);
  setUnlockGuidance('passphrase');
  updateHeroState('locked');
}

function applyUnlockedUi(active, unlocked) {
  currentUiMode = 'unlocked';
  document.body.dataset.mode = 'unlocked';
  setStatusBadge('status-unlocked', 'UNLOCKED');
  setHidden('unlockPassphrase', true);
  setHidden('unlockFido2', true);
  setHidden('unlockTabs', true);
  setHidden('lockForm', false);
  setHidden('authRecovery', true);
  setText('authTitle', 'Session controls');
  setText('authLead', 'This browser currently holds a valid local session token. Locking clears unlocked keys from daemon memory; logging out only clears this browser session.');
  setUnlockGuidance('session');

  renderCompartmentSwitcher(unlocked, active);
  renderActiveCompartment(active, unlocked);
  setSecretsAccess(true);

  setHidden('compartmentCard', false);
  setHidden('pushCard', unlocked.length < 2);
  if (unlocked.length >= 2) buildPushSelectors(unlocked);

  setHidden('guideCard', false);
  setHidden('profilesCard', false);
  setHidden('xpubCard', false);
  setHidden('inventoryCard', false);
  setHidden('depositsCard', false);
  setHidden('queueCard', false);
  setHidden('maintenanceCard', false);
  setHidden('backupCard', false);
  setHidden('auditCard', false);
  setHidden('diagCard', false);
  updateHeroState('unlocked', active, unlocked);
}

async function api(method, path, body) {
  const headers = { 'Content-Type': 'application/json' };
  const token = getSessionToken();
  if (token) headers.Authorization = 'Bearer ' + token;

  const opts = { method, headers };
  if (body) opts.body = JSON.stringify(body);
  const r = await fetch(API + path, opts);
  let payload = {};
  try {
    payload = await r.json();
  } catch (_) {}
  if (payload && payload.session_token) setSessionToken(payload.session_token);
  if (r.status === 401) clearSessionToken();
  return payload;
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

async function runRefreshCycle() {
  const s = await api('GET', '/api/status');
  currentStatus = s;
  const active = s.active_compartment;
  const unlocked = s.unlocked_compartments || [];

  // Not initialized — show setup wizard
  if (!s.initialized) {
    applySetupUi();
    syncSectionNav();
    wizDetectDevice();
    return;
  }

  setHidden('setupCard', true);
  setHidden('authCard', false);

  if (s.locked) {
    applyLockedUi();
    syncSectionNav();
    showUnlockTabs();
    return;
  }

  applyUnlockedUi(active, unlocked);

  await Promise.all([
    loadSecrets(),
    loadApiKeys(),
    loadProfiles(),
    loadInventoryOperations(),
    loadDepositRegistry(),
    loadQueueJobs(),
    loadFido2(),
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
      lastRefreshAt = new Date();
      updateRefreshMeta();
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
        scheduleRefresh();
      }
    }
  })();

  return refreshPromise;
}

async function showUnlockTabs() {
  // Detect which unlock methods are available without revealing compartment info
  try {
    const detect = await api('GET', '/api/fido2/detect');
    const hasFido = detect.device_present;
    lastFidoDetect = detect;
    const tabs = document.getElementById('unlockTabs');
    const activeTab = document.getElementById('unlockFido2').classList.contains('hidden')
      ? 'passphrase'
      : 'fido2';
    if (hasFido) {
      tabs.classList.remove('hidden');
      setText('authLead', detect.device_count + ' hardware key(s) detected. Passphrase unlock stays available, or switch to Hardware Key when you want threshold-based unlock.');
      switchUnlockTab(activeTab);
    } else {
      tabs.classList.add('hidden');
      setText('authLead', 'No FIDO2 device is currently detected. Unlock with a passphrase, or attach a hardware key and refresh to use threshold-based unlock.');
      switchUnlockTab('passphrase');
    }
  } catch(e) {
    setText('authLead', 'Unlock with the passphrase you configured during setup. Hardware-key unlock becomes available when a FIDO2 device is detected.');
    switchUnlockTab('passphrase');
  }
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

// ── Form helpers ──────────────────────────────────────────────

function clearFields(ids) {
  ids.forEach(id => {
    const el = document.getElementById(id);
    if (el) el.value = '';
  });
}

function renderEntityList(containerId, items, emptyMsg, renderItem) {
  const el = document.getElementById(containerId);
  if (!items.length) {
    el.innerHTML = '<p class="helper-text">' + esc(emptyMsg) + '</p>';
    return;
  }
  let html = '<ul class="entity-list">';
  items.forEach(item => { html += renderItem(item); });
  html += '</ul>';
  el.innerHTML = html;
}

function textValue(id) {
  return document.getElementById(id).value.trim();
}

function optionalTextValue(id) {
  const value = textValue(id);
  return value ? value : null;
}

function optionalNumberValue(id) {
  const value = textValue(id);
  if (!value) return null;
  const parsed = parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : null;
}

function setSelectOptions(id, items, placeholder) {
  const el = document.getElementById(id);
  if (!el) return;
  const previous = el.value;
  let html = '';
  if (placeholder) {
    html += '<option value="">' + esc(placeholder) + '</option>';
  }
  items.forEach(item => {
    html += '<option value="' + escAttr(String(item.value)) + '">' + esc(item.label) + '</option>';
  });
  el.innerHTML = html;

  if (items.some(item => String(item.value) === previous)) {
    el.value = previous;
  } else if (!placeholder && items[0]) {
    el.value = String(items[0].value);
  } else {
    el.value = '';
  }
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

function formatTs(unix) {
  if (!unix) return '-';
  return new Date(unix * 1000).toLocaleString();
}

function pillClass(status) {
  const value = (status || '').toLowerCase();
  if (value.includes('fail') || value.includes('error')) return 'pill-danger';
  if (value.includes('ok') || value.includes('success') || value.includes('sent') || value.includes('broadcast')) return 'pill-good';
  if (value.includes('queue') || value.includes('funded') || value.includes('detected') || value.includes('processing') || value.includes('block') || value.includes('retry')) return 'pill-warn';
  return 'pill-neutral';
}

function statusPill(status) {
  const label = (status || 'unknown').replace(/_/g, ' ');
  return '<span class="pill ' + pillClass(status) + '">' + esc(label) + '</span>';
}

function showResultBox(id, html) {
  const el = document.getElementById(id);
  el.innerHTML = html;
  el.setAttribute('role', 'status');
  el.setAttribute('aria-live', 'polite');
  el.classList.remove('hidden');
}

function describeQueueJob(job) {
  const kind = job.kind || 'unknown';
  switch (kind) {
    case 'eth_stealth_transfer':
      return 'native transfer · ' + (job.wallet_profile || '-') + ' · to ' + (job.destination_address || job.to_address || '-');
    case 'eth_stealth_erc20_transfer':
      return 'erc20 transfer · ' + (job.wallet_profile || '-') + ' · token ' + (job.token_address || '-');
    case 'eth_stealth_native_sweep':
      return 'native sweep · ' + (job.wallet_profile || '-') + ' · ' + (job.destination_address || 'wallet default');
    case 'eth_stealth_erc20_sweep':
      return 'erc20 sweep · ' + (job.wallet_profile || '-') + ' · token ' + (job.token_address || '-');
    default:
      return kind;
  }
}

function queueScheduleLine(job) {
  if (job.next_attempt_after_unix) {
    return 'nextAttempt=' + formatTs(job.next_attempt_after_unix);
  }
  if ((job.state || '').toLowerCase().includes('retry')) {
    return 'nextAttempt=manual-or-immediate';
  }
  return 'nextAttempt=-';
}

function depositObservedLine(deposit) {
  const observedAmount = deposit.observed_amount_hex || '-';
  const nativeBalance = deposit.observed_native_balance_wei_hex || '-';
  return 'expected=' + esc(deposit.expected_amount_hex || '-') +
    ' · observed=' + esc(observedAmount) +
    ' · native=' + esc(nativeBalance);
}

function renderProviderProfiles(profiles) {
  renderEntityList('providerProfileList', profiles, 'No provider profiles yet. Save an RPC endpoint and fee policy above to let deposits and queue work talk to a chain.', profile => {
    const feeInfo = 'priority=' + (profile.max_priority_fee_per_gas_hex || '-') +
      ' · max=' + (profile.max_fee_per_gas_hex || '-') +
      ' · nativeGas=' + (profile.native_gas_limit || '-') +
      ' · erc20Gas=' + (profile.erc20_gas_limit || '-');
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(profile.name) + '</div>' +
      '<div class="entity-meta">' +
      'rpc=' + esc(profile.rpc_url) + '<br>' +
      'chain=' + esc(String(profile.chain_id)) +
      ' · compartment=' + esc(String(profile.compartment_id)) +
      ' · authKey=' + esc(profile.auth_token_key || '-') + '<br>' +
      esc(feeInfo) +
      '</div></div>' +
      '<div class="entity-actions">' +
      '<button class="btn-ghost" data-action="copyText" data-arg0="' + escAttr(profile.rpc_url) + '" data-arg1="RPC URL">Copy RPC</button>' +
      '<button class="btn-danger" data-action="deleteProviderProfile" data-arg0="' + escAttr(profile.name) + '">Delete</button>' +
      '</div></li>';
  });
}

function renderWalletProfiles(profiles) {
  renderEntityList('walletProfileList', profiles, 'No wallet profiles yet. Create one above to bind a Sigillum wallet label to a provider before you generate deposits.', profile => {
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(profile.name) + '</div>' +
      '<div class="entity-meta">' +
      'wallet=' + esc(profile.wallet) +
      ' · short=' + esc(profile.short_name) +
      ' · provider=' + esc(profile.provider_profile) + '<br>' +
      'compartment=' + esc(String(profile.compartment_id)) +
      ' · chain=' + esc(profile.chain_id != null ? String(profile.chain_id) : '-') +
      ' · defaultDestination=' + esc(profile.default_destination_address || '-') +
      '</div></div>' +
      '<div class="entity-actions">' +
      '<button class="btn-ghost" data-action="exportWalletMeta" data-arg0="' + escAttr(profile.wallet) + '" data-arg1="' + escAttr(profile.short_name) + '">Export Meta</button>' +
      '<button class="btn-danger" data-action="deleteWalletProfile" data-arg0="' + escAttr(profile.name) + '">Delete</button>' +
      '</div></li>';
  });
}

function renderXpubWalletProfiles(profiles) {
  renderEntityList('xpubWalletProfileList', profiles, 'No xpub wallet profiles yet. Save one above when you want a public receive tree without exposing private key material.', profile => {
    const accountPath = "m/44'/60'/" + profile.project_account + "'";
    const receivePath = accountPath + '/0';
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(profile.name) + '</div>' +
      '<div class="entity-meta">' +
      'projectAccount=' + esc(String(profile.project_account)) +
      ' · provider=' + esc(profile.provider_profile) + '<br>' +
      'accountPath=' + esc(accountPath) +
      ' · receivePath=' + esc(receivePath) + '<br>' +
      'compartment=' + esc(String(profile.compartment_id)) +
      ' · chain=' + esc(profile.chain_id != null ? String(profile.chain_id) : '-') +
      ' · defaultDestination=' + esc(profile.default_destination_address || '-') +
      '</div></div>' +
      '<div class="entity-actions">' +
      '<button class="btn-ghost" data-action="exportXpubWalletProfile" data-arg0="' + escAttr(profile.name) + '">Export Xpub</button>' +
      '<button class="btn-danger" data-action="deleteXpubWalletProfile" data-arg0="' + escAttr(profile.name) + '">Delete</button>' +
      '</div></li>';
  });
}

function renderSeedWalletProfiles(profiles) {
  renderEntityList('seedWalletProfileList', profiles, 'No imported seed wallets yet. Import a 12-word or 24-word phrase to add another receive wallet profile.', profile => {
    const label = profile.label ? ' · label=' + profile.label : '';
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(profile.name) + '</div>' +
      '<div class="entity-meta">' +
      'words=' + esc(String(profile.word_count)) +
      ' · account=' + esc(String(profile.project_account)) +
      ' · provider=' + esc(profile.provider_profile) + esc(label) + '<br>' +
      'accountPath=' + esc(profile.account_path || '-') +
      ' · receivePath=' + esc(profile.receive_path || '-') + '<br>' +
      'firstAddress=' + esc(profile.first_receive_address || '-') + '<br>' +
      'compartment=' + esc(String(profile.compartment_id)) +
      ' · chain=' + esc(profile.chain_id != null ? String(profile.chain_id) : '-') +
      ' · defaultDestination=' + esc(profile.default_destination_address || '-') +
      '</div></div>' +
      '<div class="entity-actions">' +
      '<button class="btn-ghost" data-action="copyText" data-arg0="' + escAttr(profile.receive_xpub || '') + '" data-arg1="Seed wallet receive xpub">Copy Xpub</button>' +
      '<button class="btn-ghost" data-action="copyText" data-arg0="' + escAttr(profile.first_receive_address || '') + '" data-arg1="First receive address">Copy Address</button>' +
      '<button class="btn-danger" data-action="deleteSeedWalletProfile" data-arg0="' + escAttr(profile.name) + '">Delete</button>' +
      '</div></li>';
  });
}

async function loadProfiles() {
  try {
    const [providerResp, walletResp, xpubResp, seedResp] = await Promise.all([
      api('GET', '/api/profiles/evm'),
      api('GET', '/api/profiles/eth-stealth'),
      api('GET', '/api/profiles/eth-xpub'),
      api('GET', '/api/profiles/eth-seed'),
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
      'walletProviderProfile',
      providers.map(profile => ({
        value: profile.name,
        label: profile.name + ' · chain ' + profile.chain_id,
      })),
      providers.length ? 'Select provider profile' : 'No provider profiles available'
    );
    setSelectOptions(
      'xpubProviderProfile',
      providers.map(profile => ({
        value: profile.name,
        label: profile.name + ' · chain ' + profile.chain_id,
      })),
      providers.length ? 'Select provider profile' : 'No provider profiles available'
    );
    setSelectOptions(
      'seedProviderProfile',
      providers.map(profile => ({
        value: profile.name,
        label: profile.name + ' · chain ' + profile.chain_id,
      })),
      providers.length ? 'Select provider profile' : 'No provider profiles available'
    );

    const walletOptions = wallets.map(profile => ({
      value: profile.name,
      label: profile.name + ' · ' + profile.wallet,
    }));
    setSelectOptions(
      'depositNativeWalletProfile',
      walletOptions,
      wallets.length ? 'Select wallet profile' : 'No wallet profiles available'
    );
    setSelectOptions(
      'depositErc20WalletProfile',
      walletOptions,
      wallets.length ? 'Select wallet profile' : 'No wallet profiles available'
    );
    setSelectOptions(
      'xpubPreviewProfile',
      xpubWallets.map(profile => ({
        value: profile.name,
        label: profile.name + ' · account ' + profile.project_account,
      })),
      xpubWallets.length ? 'Select xpub profile' : 'No xpub profiles available'
    );
  } catch (e) {}
}

async function upsertProviderProfile() {
  const name = textValue('providerName');
  const rpcUrl = textValue('providerRpcUrl');
  const chainId = parseInt(textValue('providerChainId'), 10);
  if (!name || !rpcUrl || !chainId) {
    toast('Provider name, RPC URL, and chain ID are required', 'error');
    return;
  }

  const r = await api('POST', '/api/profiles/evm/upsert', {
    name,
    rpc_url: rpcUrl,
    auth_token_key: optionalTextValue('providerAuthTokenKey'),
    compartment_id: optionalNumberValue('providerCompartmentId'),
    chain_id: chainId,
    max_priority_fee_per_gas_hex: optionalTextValue('providerMaxPriorityFee'),
    max_fee_per_gas_hex: optionalTextValue('providerMaxFee'),
    native_gas_limit: optionalNumberValue('providerNativeGasLimit'),
    erc20_gas_limit: optionalNumberValue('providerErc20GasLimit'),
  });
  if (r.error) { toast(r.error, 'error'); return; }

  clearFields(['providerName', 'providerRpcUrl', 'providerAuthTokenKey',
    'providerCompartmentId', 'providerMaxPriorityFee', 'providerMaxFee',
    'providerNativeGasLimit', 'providerErc20GasLimit']);
  toast('Provider profile saved');
  refresh();
}

async function deleteProviderProfile(name) {
  if (!confirm('Delete provider profile "' + name + '"?')) return;
  const r = await api('POST', '/api/profiles/evm/delete', { name });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Provider profile deleted');
  refresh();
}

async function upsertWalletProfile() {
  const name = textValue('walletProfileName');
  const wallet = textValue('walletLabel');
  const providerProfile = textValue('walletProviderProfile');
  if (!name || !wallet || !providerProfile) {
    toast('Wallet profile name, wallet label, and provider profile are required', 'error');
    return;
  }

  const r = await api('POST', '/api/profiles/eth-stealth/upsert', {
    name,
    wallet,
    short_name: optionalTextValue('walletShortName'),
    provider_profile: providerProfile,
    compartment_id: optionalNumberValue('walletCompartmentId'),
    chain_id: optionalNumberValue('walletChainId'),
    default_destination_address: optionalTextValue('walletDefaultDestination'),
  });
  if (r.error) { toast(r.error, 'error'); return; }

  clearFields(['walletProfileName', 'walletLabel', 'walletShortName',
    'walletCompartmentId', 'walletChainId', 'walletDefaultDestination']);
  toast('Wallet profile saved');
  refresh();
}

async function deleteWalletProfile(name) {
  if (!confirm('Delete wallet profile "' + name + '"?')) return;
  const r = await api('POST', '/api/profiles/eth-stealth/delete', { name });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Wallet profile deleted');
  refresh();
}

async function upsertXpubWalletProfile() {
  const name = textValue('xpubProfileName');
  const providerProfile = textValue('xpubProviderProfile');
  const projectAccount = parseInt(textValue('xpubProjectAccount'), 10);
  if (!name || !providerProfile || !Number.isInteger(projectAccount) || projectAccount < 0) {
    toast('Profile name, provider profile, and a non-negative project account are required', 'error');
    return;
  }

  const r = await api('POST', '/api/profiles/eth-xpub/upsert', {
    name,
    project_account: projectAccount,
    provider_profile: providerProfile,
    compartment_id: optionalNumberValue('xpubCompartmentId'),
    chain_id: optionalNumberValue('xpubChainId'),
    default_destination_address: optionalTextValue('xpubDefaultDestination'),
  });
  if (r.error) { toast(r.error, 'error'); return; }

  clearFields([
    'xpubProfileName',
    'xpubCompartmentId',
    'xpubChainId',
    'xpubDefaultDestination',
  ]);
  document.getElementById('xpubProjectAccount').value = '0';
  toast('Xpub wallet profile saved');
  refresh();
}

async function deleteXpubWalletProfile(name) {
  if (!confirm('Delete xpub wallet profile "' + name + '"?')) return;
  const r = await api('POST', '/api/profiles/eth-xpub/delete', { name });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Xpub wallet profile deleted');
  refresh();
}

async function upsertSeedWalletProfile() {
  const name = textValue('seedProfileName');
  const mnemonic = textValue('seedMnemonic');
  const providerProfile = textValue('seedProviderProfile');
  const projectAccount = parseInt(textValue('seedProjectAccount'), 10);
  const wordCount = mnemonic ? mnemonic.split(/\s+/).filter(Boolean).length : 0;
  if (!name || !mnemonic || !providerProfile || !Number.isInteger(projectAccount) || projectAccount < 0) {
    toast('Profile name, seed phrase, provider profile, and a non-negative account are required', 'error');
    return;
  }
  if (wordCount !== 12 && wordCount !== 24) {
    toast('Seed phrase must contain exactly 12 or 24 words', 'error');
    return;
  }

  const r = await api('POST', '/api/profiles/eth-seed/upsert', {
    name,
    label: optionalTextValue('seedProfileLabel'),
    mnemonic,
    mnemonic_passphrase: optionalTextValue('seedMnemonicPassphrase'),
    project_account: projectAccount,
    provider_profile: providerProfile,
    compartment_id: optionalNumberValue('seedCompartmentId'),
    chain_id: optionalNumberValue('seedChainId'),
    default_destination_address: optionalTextValue('seedDefaultDestination'),
  });
  if (r.error) { toast(r.error, 'error'); return; }

  clearFields([
    'seedProfileName',
    'seedProfileLabel',
    'seedMnemonic',
    'seedMnemonicPassphrase',
    'seedCompartmentId',
    'seedChainId',
    'seedDefaultDestination',
  ]);
  document.getElementById('seedProjectAccount').value = '0';
  toast('Seed wallet profile imported');
  refresh();
}

async function deleteSeedWalletProfile(name) {
  if (!confirm('Delete seed wallet profile "' + name + '"?')) return;
  const r = await api('POST', '/api/profiles/eth-seed/delete', { name });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Seed wallet profile deleted');
  refresh();
}

async function exportSelectedXpubWallet() {
  const walletProfile = textValue('xpubPreviewProfile');
  if (!walletProfile) {
    toast('Choose an xpub wallet profile first', 'error');
    return;
  }
  await exportXpubWalletProfile(walletProfile);
}

async function exportXpubWalletProfile(walletProfile) {
  const r = await api('POST', '/api/wallets/eth-xpub/export', {
    wallet_profile: walletProfile,
  });
  if (r.error) { toast(r.error, 'error'); return; }

  const exportedXpub = r.receive_xpub || '';
  document.getElementById('xpubPreviewProfile').value = walletProfile;
  document.getElementById('xpubReceiveXpub').value = exportedXpub;

  showResultBox(
    'xpubExportResult',
    '<strong>' + esc(walletProfile) + '</strong><br>' +
      'accountPath=' + esc(r.account_path || '-') + '<br>' +
      'receivePath=' + esc(r.receive_path || '-') + '<br>' +
      'xpub=' + esc(exportedXpub) + '<br>' +
      '<div style="margin-top:10px;display:flex;gap:8px;flex-wrap:wrap;">' +
        '<button class="btn-ghost" data-action="copyText" data-arg0="' + escAttr(exportedXpub) + '" data-arg1="Receive branch xpub">Copy Xpub</button>' +
      '</div>'
  );

  await previewXpubReceiveAddress();
}

async function previewXpubReceiveAddress() {
  const xpub = textValue('xpubReceiveXpub');
  const index = parseInt(textValue('xpubPreviewIndex'), 10);
  if (!xpub) {
    toast('Export or paste a receive-branch xpub first', 'error');
    return;
  }
  if (!Number.isInteger(index) || index < 0) {
    toast('Receive index must be a non-negative number', 'error');
    return;
  }

  const r = await api('POST', '/api/wallets/eth-xpub/derive', { xpub, index });
  if (r.error) { toast(r.error, 'error'); return; }

  showResultBox(
    'xpubPreviewResult',
    '<strong>Receive index ' + esc(String(r.index)) + '</strong><br>' +
      'path=' + esc('receive/' + r.index) + '<br>' +
      'address=' + esc(r.address) + '<br>' +
      '<div style="margin-top:10px;display:flex;gap:8px;flex-wrap:wrap;">' +
        '<button class="btn-ghost" data-action="copyText" data-arg0="' + escAttr(r.address) + '" data-arg1="Receive address">Copy Address</button>' +
      '</div>'
  );
}

async function exportWalletMeta(wallet, shortName) {
  const r = await api('POST', '/api/wallets/eth-stealth/export', {
    wallet,
    short_name: shortName || null,
  });
  if (r.error) { toast(r.error, 'error'); return; }
  await copyText(r.stealth_meta_address, 'Stealth meta-address');
}

function renderChainProfiles(profiles) {
  renderEntityList('chainProfileList', profiles, 'No chain profiles yet. Save one to describe discovery/indexing capabilities for a network.', profile => {
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(profile.name) + ' ' + statusPill(profile.enabled ? 'enabled' : 'disabled') + '</div>' +
      '<div class="entity-meta">' +
      'family=' + esc(profile.chain_family) +
      ' · chainId=' + esc(String(profile.chain_id || '-')) +
      ' · provider=' + esc(profile.provider_profile || '-') +
      ' · native=' + esc(profile.native_symbol || '-') + '<br>' +
      'capabilities=' + esc((profile.capabilities || []).join(', ') || '-') +
      ' · source=' + esc(profile.source || '-') +
      '</div></div>' +
      '<div class="entity-actions">' +
      '<button class="btn-danger" data-action="deleteChainProfile" data-arg0="' + escAttr(profile.name) + '">Delete</button>' +
      '</div></li>';
  });
}

function renderInventoryState(inventory) {
  renderEntityList('inventoryJobList', inventory.jobs || [], 'No discovery jobs yet.', job => {
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(job.id) + ' ' + statusPill(job.status) + '</div>' +
      '<div class="entity-meta">' +
      'wallets=' + esc((job.wallet_profiles || []).join(', ') || '-') +
      ' · providers=' + esc((job.provider_profiles || []).join(', ') || '-') + '<br>' +
      'scanned=' + esc(String(job.addresses_scanned || 0)) +
      ' · active=' + esc(String(job.active_addresses || 0)) +
      ' · holdings=' + esc(String(job.holdings_detected || 0)) +
      '</div></div>' +
      '<div class="entity-actions">' +
      '<button class="btn-ghost" data-action="cancelDiscoveryJob" data-arg0="' + escAttr(job.id) + '">Cancel</button>' +
      '<button class="btn-ghost" data-action="resumeDiscoveryJob" data-arg0="' + escAttr(job.id) + '">Resume</button>' +
      '</div></li>';
  });
  renderEntityList('inventoryAddressList', inventory.addresses || [], 'No discovered addresses yet.', address => {
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(address.address) + ' ' + statusPill(address.activity_state) + '</div>' +
      '<div class="entity-meta">' +
      esc(address.wallet_family) + '/' + esc(address.wallet_profile) +
      ' · chain=' + esc(String(address.chain_id)) +
      ' · path=' + esc(address.derivation_path) + '<br>' +
      'native=' + esc(address.native_balance_wei_hex || '0x0') +
      ' · txCount=' + esc(String(address.transaction_count || 0)) +
      '</div></div></li>';
  });
  renderEntityList('inventoryHoldingList', inventory.holdings || [], 'No positive asset holdings detected yet.', holding => {
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(holding.asset_kind) + ' ' + statusPill(holding.status) + '</div>' +
      '<div class="entity-meta">' +
      'address=' + esc(holding.address) +
      ' · asset=' + esc(holding.asset_address || 'native') +
      ' · amount=' + esc(holding.amount_hex) + '<br>' +
      esc(holding.wallet_family) + '/' + esc(holding.wallet_profile) +
      ' · provider=' + esc(holding.provider_profile) +
      '</div></div></li>';
  });
}

function renderRiskFindings(findings) {
  renderEntityList('riskFindingList', findings, 'No risk findings from the current inventory.', finding => {
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(finding.category) + ' ' + statusPill(finding.risk_level) + '</div>' +
      '<div class="entity-meta">' +
      'subject=' + esc(finding.subject_type) + ':' + esc(finding.subject) +
      ' · address=' + esc(finding.address) + '<br>' +
      esc(finding.recommendation || '') +
      '</div></div></li>';
  });
}

function renderConsolidationPlans(plans) {
  renderEntityList('consolidationPlanList', plans, 'No consolidation plans generated yet.', plan => {
    const summary = plan.summary || {};
    const stepLines = (plan.steps || []).slice(0, 8).map(step =>
      '<div class="entity-meta">' +
      esc(step.action) + ' ' + statusPill(step.status) +
      ' · ' + esc(step.asset_kind) +
      ' · amount=' + esc(step.amount_hex) +
      ' · blockers=' + esc((step.blockers || []).join(', ') || '-') +
      '</div>'
    ).join('');
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(plan.id) + ' ' + statusPill(plan.status) + '</div>' +
      '<div class="entity-meta">' +
      'steps=' + esc(String(summary.total_steps || 0)) +
      ' · blocked=' + esc(String(summary.blocked_steps || 0)) +
      ' · review=' + esc(String(summary.review_required_steps || 0)) +
      ' · approved=' + esc(String(summary.approved_steps || 0)) +
      '</div>' + stepLines + '</div>' +
      '<div class="entity-actions">' +
      '<button class="btn-ghost" data-action="approveConsolidationPlan" data-arg0="' + escAttr(plan.id) + '">Approve Reviewable</button>' +
      '</div></li>';
  });
}

async function loadInventoryOperations() {
  try {
    const [chains, inventory, risks, plans] = await Promise.all([
      api('GET', '/api/inventory/chains'),
      api('GET', '/api/inventory/wallets'),
      api('GET', '/api/risk/findings'),
      api('GET', '/api/plans/consolidation'),
    ]);
    if (!chains.error) renderChainProfiles(chains.profiles || []);
    if (!inventory.error) renderInventoryState(inventory);
    if (!risks.error) renderRiskFindings(risks.findings || []);
    if (!plans.error) renderConsolidationPlans(plans.plans || []);
  } catch (e) {}
}

async function upsertChainProfile() {
  const name = textValue('chainProfileName');
  const family = textValue('chainProfileFamily');
  if (!name || !family) {
    toast('Chain profile name and family are required', 'error');
    return;
  }
  const r = await api('POST', '/api/inventory/chains/upsert', {
    name,
    chain_family: family,
    chain_id: optionalNumberValue('chainProfileId'),
    provider_profile: optionalTextValue('chainProfileProvider'),
    native_symbol: optionalTextValue('chainProfileNativeSymbol'),
    capabilities: [],
    enabled: true,
  });
  if (r.error) { toast(r.error, 'error'); return; }
  clearFields(['chainProfileName', 'chainProfileFamily', 'chainProfileId',
    'chainProfileProvider', 'chainProfileNativeSymbol']);
  toast('Chain profile saved');
  loadInventoryOperations();
}

async function deleteChainProfile(name) {
  if (!confirm('Delete chain profile "' + name + '"?')) return;
  const r = await api('POST', '/api/inventory/chains/delete', { name });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Chain profile deleted');
  loadInventoryOperations();
}

async function scanInventoryEvm() {
  const token = optionalTextValue('inventoryTokenAddress');
  const r = await api('POST', '/api/inventory/scan/evm', {
    wallet_family: optionalTextValue('inventoryWalletFamily'),
    wallet_profile: optionalTextValue('inventoryWalletProfile'),
    provider_profile: optionalTextValue('inventoryProviderProfile'),
    gap_limit: optionalNumberValue('inventoryGapLimit'),
    max_index: optionalNumberValue('inventoryMaxIndex'),
    token_addresses: token ? [token] : [],
    block_tag: 'latest',
  });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Inventory scan completed');
  loadInventoryOperations();
}

async function cancelDiscoveryJob(id) {
  const r = await api('POST', '/api/discovery/jobs/cancel', { id });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Discovery job marked canceled');
  loadInventoryOperations();
}

async function resumeDiscoveryJob(id) {
  const r = await api('POST', '/api/discovery/jobs/resume', { id });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Discovery job marked for resume');
  loadInventoryOperations();
}

async function loadRiskFindings() {
  const r = await api('GET', '/api/risk/findings');
  if (r.error) { toast(r.error, 'error'); return; }
  renderRiskFindings(r.findings || []);
  toast('Risk findings refreshed');
}

async function generateConsolidationPlan() {
  const r = await api('POST', '/api/plans/consolidation/generate', {
    destination_address: optionalTextValue('planDestinationAddress'),
    include_watch_only: true,
    auto_queue_low_risk: false,
  });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Dry-run consolidation plan generated');
  loadInventoryOperations();
}

async function approveConsolidationPlan(planId) {
  const r = await api('POST', '/api/plans/consolidation/approve', {
    plan_id: planId,
    step_ids: [],
  });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Reviewable plan steps approved');
  loadInventoryOperations();
}

function renderDeposits(deposits) {
  renderEntityList('depositList', deposits, 'No tracked deposits yet. Create a native or ERC-20 deposit above to start monitoring incoming funds and queue follow-up work.', deposit => {
    const queueInfo = deposit.queue_job_id
      ? 'job=' + deposit.queue_job_id + ' · state=' + (deposit.queue_job_state || '-')
      : 'job=-';
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(deposit.id) + ' ' + statusPill(deposit.status) + '</div>' +
      '<div class="entity-meta">' +
      'walletProfile=' + esc(deposit.wallet_profile) +
      ' · asset=' + esc(deposit.asset_kind) +
      ' · short=' + esc(deposit.short_name) + '<br>' +
      'stealth=' + esc(deposit.stealth_address) + '<br>' +
      'ephemeral=' + esc(deposit.ephemeral_public_key_hex) +
      ' · viewTag=' + esc(deposit.view_tag_hex) + '<br>' +
      depositObservedLine(deposit) + '<br>' +
      'token=' + esc(deposit.token_address || '-') +
      ' · autoQueue=' + esc(String(deposit.auto_queue_sweep)) +
      ' · ' + esc(queueInfo) + '<br>' +
      'created=' + esc(formatTs(deposit.created_at_unix)) +
      ' · checked=' + esc(formatTs(deposit.last_checked_at_unix)) +
      ' · updated=' + esc(formatTs(deposit.updated_at_unix)) +
      (deposit.note ? '<br>note=' + esc(deposit.note) : '') +
      '</div></div>' +
      '<div class="entity-actions">' +
      '<button class="btn-ghost" data-action="copyText" data-arg0="' + escAttr(deposit.stealth_address) + '" data-arg1="Deposit address">Copy Address</button>' +
      '<button class="btn-ghost" data-action="refreshSingleDeposit" data-arg0="' + escAttr(deposit.id) + '">Refresh</button>' +
      '<button class="btn-success" data-action="enqueueDepositSweep" data-arg0="' + escAttr(deposit.id) + '">Queue Sweep</button>' +
      '<button class="btn-danger" data-action="deleteDeposit" data-arg0="' + escAttr(deposit.id) + '">Delete</button>' +
      '</div></li>';
  });
}

async function loadDepositRegistry() {
  try {
    const r = await api('GET', '/api/deposits/eth-stealth');
    if (r.error) return;
    lastDeposits = r.deposits || [];
    renderDeposits(lastDeposits);
  } catch (e) {}
}

async function createNativeDeposit() {
  const walletProfile = textValue('depositNativeWalletProfile');
  if (!walletProfile) {
    toast('Select a wallet profile first', 'error');
    return;
  }
  const r = await api('POST', '/api/deposits/eth-stealth/create-native', {
    wallet_profile: walletProfile,
    expected_value_wei_hex: optionalTextValue('depositNativeExpected'),
    auto_queue_sweep: document.getElementById('depositNativeAutoQueue').checked,
    sweep_destination_address: optionalTextValue('depositNativeDestination'),
    min_sweep_value_wei_hex: optionalTextValue('depositNativeMinSweep'),
    note: optionalTextValue('depositNativeNote'),
  });
  if (r.error) { toast(r.error, 'error'); return; }
  clearFields(['depositNativeExpected', 'depositNativeMinSweep',
    'depositNativeDestination', 'depositNativeNote']);
  toast('Native deposit created');
  refresh();
}

async function createErc20Deposit() {
  const walletProfile = textValue('depositErc20WalletProfile');
  const tokenAddress = textValue('depositErc20TokenAddress');
  if (!walletProfile || !tokenAddress) {
    toast('Wallet profile and token address are required', 'error');
    return;
  }
  const r = await api('POST', '/api/deposits/eth-stealth/create-erc20', {
    wallet_profile: walletProfile,
    token_address: tokenAddress,
    expected_amount_hex: optionalTextValue('depositErc20Expected'),
    auto_queue_sweep: document.getElementById('depositErc20AutoQueue').checked,
    sweep_destination_address: optionalTextValue('depositErc20Destination'),
    min_sweep_amount_hex: optionalTextValue('depositErc20MinSweep'),
    note: optionalTextValue('depositErc20Note'),
  });
  if (r.error) { toast(r.error, 'error'); return; }
  clearFields(['depositErc20TokenAddress', 'depositErc20Expected',
    'depositErc20MinSweep', 'depositErc20Destination', 'depositErc20Note']);
  toast('ERC-20 deposit created');
  refresh();
}

async function refreshDepositRegistry() {
  const r = await api('POST', '/api/deposits/eth-stealth/refresh', {
    id: null,
    limit: optionalNumberValue('depositRefreshLimit'),
    auto_enqueue: document.getElementById('depositRefreshAutoEnqueue').checked,
  });
  if (r.error) { toast(r.error, 'error'); return; }
  showResultBox(
    'depositRefreshResult',
    'processed=' + esc(String(r.processed || 0)) +
    ' · detected=' + esc(String(r.detected || 0)) +
    ' · queued=' + esc(String(r.queued || 0))
  );
  lastDeposits = r.deposits || [];
  renderDeposits(lastDeposits);
  updateNextStepCard();
  toast('Deposits refreshed');
  loadQueueJobs();
}

async function refreshSingleDeposit(id) {
  const r = await api('POST', '/api/deposits/eth-stealth/refresh', {
    id,
    limit: 1,
    auto_enqueue: document.getElementById('depositRefreshAutoEnqueue').checked,
  });
  if (r.error) { toast(r.error, 'error'); return; }
  showResultBox(
    'depositRefreshResult',
    'processed=' + esc(String(r.processed || 0)) +
    ' · detected=' + esc(String(r.detected || 0)) +
    ' · queued=' + esc(String(r.queued || 0)) +
    ' · target=' + esc(id)
  );
  lastDeposits = r.deposits || [];
  renderDeposits(lastDeposits);
  updateNextStepCard();
  loadQueueJobs();
}

async function enqueueDepositSweep(id) {
  const r = await api('POST', '/api/deposits/eth-stealth/enqueue-sweep', { id });
  if (r.error) { toast(r.error, 'error'); return; }
  showResultBox(
    'depositRefreshResult',
    'queued sweep for deposit ' + esc(id) + ' · job=' + esc(r.job?.id || '-')
  );
  toast('Deposit sweep queued');
  refresh();
}

async function deleteDeposit(id) {
  if (!confirm('Delete deposit "' + id + '"?')) return;
  const r = await api('POST', '/api/deposits/eth-stealth/delete', { id });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Deposit deleted');
  refresh();
}

function renderQueueJobs(jobs) {
  renderEntityList('queueList', jobs, 'Queue is empty. Once deposits enqueue sweeps or you create manual work, jobs will appear here for review and processing.', job => {
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(job.id) + ' ' + statusPill(job.state) + '</div>' +
      '<div class="entity-meta">' +
      'kind=' + esc(job.kind || '-') +
      ' · attempts=' + esc(String(job.attempts || 0)) + '<br>' +
      esc(describeQueueJob(job)) + '<br>' +
      'created=' + esc(formatTs(job.created_at_unix)) +
      ' · updated=' + esc(formatTs(job.updated_at_unix)) +
      ' · ' + esc(queueScheduleLine(job)) + '<br>' +
      'tx=' + esc(job.transaction_hash_hex || '-') +
      ' · broadcast=' + esc(job.broadcast_transaction_hash_hex || '-') +
      (job.last_error ? '<br>lastError=' + esc(job.last_error) : '') +
      '</div></div>' +
      '<div class="entity-actions">' +
      '<button class="btn-primary" data-action="processQueueJob" data-arg0="' + escAttr(job.id) + '">Process</button>' +
      '</div></li>';
  });
}

async function loadQueueJobs() {
  try {
    const r = await api('GET', '/api/queue/jobs');
    if (r.error) return;
    lastQueueJobs = r.jobs || [];
    renderQueueJobs(lastQueueJobs);
  } catch (e) {}
}

async function processQueueBatch() {
  const r = await api('POST', '/api/queue/process', {
    id: null,
    limit: optionalNumberValue('queueProcessLimit'),
  });
  if (r.error) { toast(r.error, 'error'); return; }
  showResultBox(
    'queueProcessResult',
    'processed=' + esc(String(r.processed || 0)) +
    ' · succeeded=' + esc(String(r.succeeded || 0)) +
    ' · blocked=' + esc(String(r.blocked || 0)) +
    ' · retrying=' + esc(String(r.retrying || 0)) +
    ' · failed=' + esc(String(r.failed || 0))
  );
  lastQueueJobs = r.jobs || [];
  renderQueueJobs(lastQueueJobs);
  updateNextStepCard();
  loadDepositRegistry();
  toast('Queue processed');
}

async function processQueueJob(id) {
  const r = await api('POST', '/api/queue/process', { id, limit: 1 });
  if (r.error) { toast(r.error, 'error'); return; }
  showResultBox(
    'queueProcessResult',
    'processed=' + esc(String(r.processed || 0)) +
    ' · succeeded=' + esc(String(r.succeeded || 0)) +
    ' · blocked=' + esc(String(r.blocked || 0)) +
    ' · retrying=' + esc(String(r.retrying || 0)) +
    ' · failed=' + esc(String(r.failed || 0)) +
    ' · target=' + esc(id)
  );
  lastQueueJobs = r.jobs || [];
  renderQueueJobs(lastQueueJobs);
  updateNextStepCard();
  loadDepositRegistry();
}

async function runMaintenanceCycle() {
  const r = await api('POST', '/api/maintenance/run', {
    deposit_refresh_limit: optionalNumberValue('maintenanceDepositLimit'),
    queue_process_limit: optionalNumberValue('maintenanceQueueLimit'),
    auto_enqueue: document.getElementById('maintenanceAutoEnqueue').checked,
  });
  if (r.error) { toast(r.error, 'error'); return; }
  showResultBox(
    'maintenanceResult',
    'refreshed=' + esc(String(r.refreshed || 0)) +
    ' · detected=' + esc(String(r.detected || 0)) +
    ' · queued=' + esc(String(r.queued || 0)) +
    ' · processed=' + esc(String(r.processed || 0)) +
    ' · succeeded=' + esc(String(r.succeeded || 0)) +
    ' · blocked=' + esc(String(r.blocked || 0)) +
    ' · retrying=' + esc(String(r.retrying || 0)) +
    ' · failed=' + esc(String(r.failed || 0))
  );
  lastDeposits = r.deposits || [];
  lastQueueJobs = r.jobs || [];
  renderDeposits(lastDeposits);
  renderQueueJobs(lastQueueJobs);
  updateNextStepCard();
  toast('Maintenance cycle complete');
}

async function loadFido2() {
  const fido2Card = document.getElementById('fido2Card');
  if (!currentStatus || currentStatus.locked) {
    fido2Card.classList.add('hidden');
    return;
  }
  fido2Card.classList.remove('hidden');

  try {
    const detect = await api('GET', '/api/fido2/detect');
    const devEl = document.getElementById('fido2DeviceStatus');
    if (detect.device_present) {
      devEl.innerHTML = '<span style="color:var(--success);">' + detect.device_count +
        ' FIDO2 device(s) connected</span>';
    } else {
      devEl.innerHTML = '<span style="color:var(--warning);">No FIDO2 device detected.</span>';
    }
  } catch(e) {}

  try {
    const keys = await api('GET', '/api/fido2/list');
    const listEl = document.getElementById('fido2KeyListSection');
    lastFidoKeys = keys.keys || [];
    if (keys.keys && keys.keys.length > 0) {
      let html = '<ul class="key-list">';
      keys.keys.forEach(k => {
        html += '<li><span>' + esc(k.label) + ' <span style="color:var(--text-dim);font-size:11px;">(' +
          esc(k.credential_id_short) + '...) ' + esc(k.registered_at) + '</span></span>' +
          '<div class="key-actions"><button class="btn-danger" data-action="fido2RemoveKey" data-arg0="' +
          escAttr(k.label) + '">Remove</button></div></li>';
      });
      html += '</ul>';
      listEl.innerHTML = html;
    } else {
      listEl.innerHTML = '<p class="text-meta">No additional hardware keys are registered yet. Add one above to improve recovery and higher-threshold unlock paths.</p>';
    }
  } catch(e) {}
}

function togglePoisonWarning() {
  const checked = document.getElementById('fido2Poison').checked;
  document.getElementById('fido2PoisonWarning').classList.toggle('hidden', !checked);
}

async function submitNewFido2Pin(pinId, confirmId, hintId, copyToId, focusId) {
  const pin = document.getElementById(pinId).value;
  const confirmPin = document.getElementById(confirmId).value;
  if (!pin) { toast('New PIN required', 'error'); return; }
  if (pin.length < 4) { toast('New PIN must be at least 4 characters', 'error'); return; }
  if (pin !== confirmPin) { toast('PIN entries do not match', 'error'); return; }

  const r = await api('POST', '/api/fido2/pin/set', { new_pin: pin });
  if (r.error) {
    const message = friendlyFidoError(r.error);
    if (hintId) setInlineInfo(hintId, message);
    toast(message, 'error');
    return;
  }

  if (copyToId) {
    const target = document.getElementById(copyToId);
    if (target) target.value = pin;
  }
  clearFields([pinId, confirmId]);
  if (hintId) {
    setInlineInfo(
      hintId,
      'FIDO2 PIN set on the inserted hardware key. Use that PIN in the registration field and continue.',
      'success'
    );
  }
  const focusTarget = document.getElementById(focusId);
  if (focusTarget) focusTarget.focus();
  toast('Hardware-key PIN set');
}

async function fido2SetNewPin() {
  await submitNewFido2Pin(
    'fido2NewPin',
    'fido2NewPinConfirm',
    'fido2DeviceStatus',
    'fido2RegPin',
    'fido2RegLabel'
  );
}

async function fido2Register() {
  const pin = document.getElementById('fido2RegPin').value;
  const label = document.getElementById('fido2RegLabel').value;
  const poison = document.getElementById('fido2Poison').checked;
  const skipRaw = document.getElementById('fido2SkipKeys').value.trim();
  const skipKeys = skipRaw ? skipRaw.split(',').map(s => s.trim()).filter(Boolean) : [];
  if (!label) { toast('Label required', 'error'); return; }
  if (poison && !confirm('Register "' + label + '" as a POISON key? Including it during unlock will cause silent failure.')) return;
  toast('Touch your FIDO2 key now...');
  const body = { label };
  if (pin) body.pin = pin;
  if (poison) body.poison = true;
  if (skipKeys.length > 0) body.skip_keys = skipKeys;
  const r = await api('POST', '/api/fido2/register', body);
  if (r.error) {
    const message = friendlyFidoError(r.error);
    setInlineInfo('fido2DeviceStatus', message);
    toast(message, 'error');
    return;
  }
  clearFields(['fido2RegPin', 'fido2RegLabel', 'fido2SkipKeys']);
  document.getElementById('fido2Poison').checked = false;
  togglePoisonWarning();
  toast('Key "' + label + '" registered' + (poison ? ' (poison)' : ''));
  refresh();
}

async function fido2RemoveKey(label) {
  if (!confirm('Remove FIDO2 key "' + label + '"?')) return;
  const pin = await promptPin('Enter the current FIDO2 PIN only if the remaining keys require one:');
  const body = { label };
  if (pin) body.pin = pin;
  const r = await api('POST', '/api/fido2/remove', body);
  if (r.error) {
    const message = friendlyFidoError(r.error);
    setInlineInfo('fido2DeviceStatus', message);
    toast(message, 'error');
    return;
  }
  toast('Key removed');
  refresh();
}

function promptPin(msg) {
  return new Promise(resolve => {
    const overlay = document.createElement('div');
    overlay.style.cssText = 'position:fixed;inset:0;background:rgba(0,0,0,0.7);z-index:200;display:flex;align-items:center;justify-content:center;';
    overlay.innerHTML = '<div class="card pin-modal"><h2>' + esc(msg) + '</h2>' +
      '<div class="form-row"><input type="password" id="pinModalInput" placeholder="Current PIN (leave blank if not required)">' +
      '<button class="btn-primary" id="pinModalOk">OK</button></div></div>';
    document.body.appendChild(overlay);
    const inp = document.getElementById('pinModalInput');
    inp.focus();
    const done = () => { const v = inp.value; overlay.remove(); resolve(v || null); };
    document.getElementById('pinModalOk').addEventListener('click', done);
    inp.addEventListener('keydown', e => {
      if (e.key === 'Enter') done();
      if (e.key === 'Escape') {
        overlay.remove();
        resolve(null);
      }
    });
    overlay.addEventListener('click', e => {
      if (e.target === overlay) {
        overlay.remove();
        resolve(null);
      }
    });
  });
}

function switchUnlockTab(tab) {
  document.querySelectorAll('.unlock-tab').forEach(t => t.classList.remove('active'));
  if (tab === 'fido2') {
    document.getElementById('unlockPassphrase').classList.add('hidden');
    document.getElementById('unlockFido2').classList.remove('hidden');
    document.querySelectorAll('.unlock-tab')[1].classList.add('active');
    setUnlockGuidance('fido2');
  } else {
    document.getElementById('unlockPassphrase').classList.remove('hidden');
    document.getElementById('unlockFido2').classList.add('hidden');
    document.querySelectorAll('.unlock-tab')[0].classList.add('active');
    setUnlockGuidance('passphrase');
  }
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

async function fido2Unlock() {
  const pin = document.getElementById('fido2Pin').value;
  const tapCount = parseInt(document.getElementById('fido2TapCount').value);
  if (!tapCount || tapCount < 1) { toast('Enter number of keys', 'error'); return; }
  toast('Touch your hardware key now...');
  const r = await api('POST', '/api/fido2/unlock', { pins: pin ? [pin] : [], tap_count: tapCount });
  if (r.error) {
    if (isAlreadyUnlockedConflict(r.error)) {
      toast('Session already active. Refreshing workspace…');
      await refresh();
      return;
    }
    const message = friendlyFidoError(r.error);
    setText('authLead', message);
    toast(message, 'error');
    return;
  }
  document.getElementById('fido2Pin').value = '';
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

// ── Setup Wizard ──────────────────────────────────────────────

function wizShowStep(id) {
  document.querySelectorAll('.wizard-step').forEach(s => s.classList.remove('active'));
  document.getElementById(id).classList.add('active');
  updateWizardChrome(id);
}

function wizPreset(preset) {
  switch (preset) {
    case 'simple':
      wizCompartments = [{ label: 'daily', threshold: 1 }];
      wizRenderCompList();
      wizShowStep('wizStepCompartments');
      break;
    case 'secure':
      wizCompartments = [
        { label: 'daily', threshold: 1 },
        { label: 'secure', threshold: 2 },
      ];
      wizRenderCompList();
      wizShowStep('wizStepCompartments');
      break;
    case 'legacy':
      wizCompartments = [
        { label: 'hot', threshold: 1 },
        { label: 'cold', threshold: 2 },
        { label: 'legacy', threshold: 3 },
      ];
      wizRenderCompList();
      wizShowStep('wizStepCompartments');
      break;
    case 'custom':
      customCompartments = [];
      document.getElementById('wizCustomCompList').innerHTML = '';
      wizShowStep('wizStepCustomComps');
      break;
    case 'passphrase':
      wizShowStep('wizStepPassphrase');
      break;
  }
}

function wizCompRowHtml(comps) {
  let html = '';
  comps.forEach(c => {
    html += '<div class="wiz-comp-row">' +
      '<span class="wiz-comp-label">' + esc(c.label) + '</span>' +
      '<span class="wiz-comp-threshold">Tap ' + c.threshold + ' key' + (c.threshold > 1 ? 's' : '') + '</span></div>';
  });
  return html;
}

function wizRenderCompList() {
  document.getElementById('wizCompList').innerHTML = wizCompRowHtml(wizCompartments);
}

function updateWizardChrome(id) {
  const meta = WIZARD_CHROME[id] || WIZARD_CHROME.wizStep0;
  setText('wizStagePill', meta.pill);
  setText('wizStageTitle', meta.title);
  setText('wizStageSummary', meta.summary);
  setTrustedHtml('wizChecklist', renderChecklist(meta.checklist || []));
}

function wizBackToPresets() {
  wizRequiredKeyCount = 1;
  wizRegisteredKeyCount = 0;
  wizPrimaryKeyLabel = '';
  wizShowStep('wizStep0');
}

async function wizDetectDevice() {
  try {
    const r = await api('GET', '/api/fido2/detect');
    lastFidoDetect = r;
    const hint = document.getElementById('wizDeviceHint');
    if (hint) {
      if (r.device_present) {
        hint.textContent = r.device_count + ' FIDO2 device(s) detected on this machine. You can continue with hardware-key setup.';
      } else {
        hint.textContent = 'No FIDO2 device detected right now. You can insert a hardware key and retry, or choose passphrase-only.';
      }
    }
  } catch(e) {
    const hint = document.getElementById('wizDeviceHint');
    if (hint) {
      hint.textContent = 'Sigillum could not verify hardware-key presence right now. You can still continue, then insert the device before registration if needed.';
    }
  }
}

async function wizInitPassphrase() {
  const label = document.getElementById('wizPLabel').value || 'default';
  const p = document.getElementById('wizPassphrase').value;
  const pc = document.getElementById('wizPassphraseConfirm').value;
  if (p.length < 8) { toast('Min 8 characters', 'error'); return; }
  if (p !== pc) { toast('Passphrases do not match', 'error'); return; }

  const initR = await api('POST', '/api/compartment/init', {
    id: 0,
    label,
    threshold: 1,
    passphrase: p,
  });
  if (initR.error) { toast(initR.error, 'error'); return; }

  document.getElementById('wizDoneMsg').textContent = 'Vault Created';
  document.getElementById('wizDoneDetail').textContent =
    'Compartment "' + label + '" initialized. You are unlocked.';
  wizShowStep('wizStepDone');
  setTimeout(refresh, 1500);
}

function wizProceedFido2() {
  let comps = wizCompartments;
  if (customCompartments.length > 0) comps = customCompartments;
  if (comps.length === 0) { toast('Add at least one compartment', 'error'); return; }
  wizCompartments = comps;
  wizRequiredKeyCount = Math.max(1, ...wizCompartments.map(c => c.threshold || 1));
  wizRegisteredKeyCount = 0;
  wizShowStep('wizStepFido2Pin');
}

function wizBackFromFido2Pin() {
  if (customCompartments.length > 0) {
    wizShowStep('wizStepCustomComps');
  } else if (wizCompartments.length > 0) {
    wizShowStep('wizStepCompartments');
  } else {
    wizShowStep('wizStep0');
  }
}

function wizAddCustomComp() {
  const label = document.getElementById('wizCustomLabel').value;
  const threshold = parseInt(document.getElementById('wizCustomThreshold').value);
  if (!label || !threshold) { toast('Label and threshold required', 'error'); return; }
  if (customCompartments.some(c => c.threshold === threshold)) {
    toast('Threshold ' + threshold + ' already used', 'error'); return;
  }
  customCompartments.push({ label, threshold });
  clearFields(['wizCustomLabel', 'wizCustomThreshold']);
  document.getElementById('wizCustomCompList').innerHTML = wizCompRowHtml(customCompartments);
  document.getElementById('wizCustomContinue').disabled = false;
}

function wizRenderAdditionalKeyState() {
  const remaining = Math.max(wizRequiredKeyCount - wizRegisteredKeyCount, 0);
  const status = document.getElementById('wizAdditionalKeyStatus');
  if (status) {
    status.textContent = wizRegisteredKeyCount + ' of ' + wizRequiredKeyCount +
      ' required hardware key' + (wizRequiredKeyCount > 1 ? 's' : '') + ' enrolled so far.';
  }

  const lead = document.getElementById('wizAdditionalKeysLead');
  if (lead) {
    if (remaining > 0) {
      lead.textContent = 'Your chosen plan needs ' + wizRequiredKeyCount + ' distinct hardware keys. Register ' +
        remaining + ' more now so every compartment you just created can actually be unlocked later.';
    } else {
      lead.textContent = 'You have enrolled enough hardware keys for the thresholds in this plan. You can finish setup now.';
    }
  }

  const note = document.getElementById('wizAdditionalKeysNote');
  if (note) {
    if (remaining > 0) {
      note.textContent = 'If you finish with fewer keys than the highest threshold, the lower-threshold compartments will work now, but the stronger access layers will stay unavailable until you enroll more keys later.';
    } else {
      note.textContent = 'Every configured compartment now has enough enrolled keys behind it to be usable when the corresponding threshold is met.';
    }
  }
}

function wizCompleteFido2Setup() {
  document.getElementById('wizDoneMsg').textContent = 'Setup Complete';
  if (wizRegisteredKeyCount >= wizRequiredKeyCount) {
    document.getElementById('wizDoneDetail').textContent =
      wizCompartments.length + ' compartment(s) created. ' + wizRegisteredKeyCount +
      ' hardware key(s) are enrolled for this plan, including "' + wizPrimaryKeyLabel + '".';
  } else {
    document.getElementById('wizDoneDetail').textContent =
      wizCompartments.length + ' compartment(s) created. ' + wizRegisteredKeyCount + ' of ' +
      wizRequiredKeyCount + ' planned hardware key(s) are enrolled so far, so only the lower-threshold access layers are ready today.';
  }
  wizShowStep('wizStepDone');
  setTimeout(refresh, 1500);
}

async function wizRegisterKey() {
  const pin = document.getElementById('wizFido2Pin').value;
  const label = document.getElementById('wizFido2Label').value;
  const passphrase = document.getElementById('wizFallbackPass').value || null;
  if (!label) { toast('Label required', 'error'); return; }

  wizShowStep('wizStepTouch');

  const body = {
    label,
    compartments: wizCompartments.map(c => ({
      label: c.label,
      threshold: c.threshold,
      passphrase_mode: null,
    })),
    passphrase: passphrase && passphrase.length >= 8 ? passphrase : null,
  };
  if (pin) body.pin = pin;

  const r = await api('POST', '/api/fido2/setup', body);
  if (r.error) {
    const message = friendlyFidoError(r.error);
    wizShowStep('wizStepFido2Pin');
    setText('wizDeviceHint', message);
    const input = document.getElementById('wizFido2Pin');
    if (input) input.focus();
    toast(message, 'error');
    return;
  }

  wizRegisteredKeyCount = r.total_keys || 1;
  wizPrimaryKeyLabel = label;
  if (wizRequiredKeyCount > wizRegisteredKeyCount) {
    clearFields(['wizAdditionalKeyPin', 'wizAdditionalKeyLabel', 'wizAdditionalNewPin', 'wizAdditionalNewPinConfirm']);
    wizRenderAdditionalKeyState();
    wizShowStep('wizStepAdditionalKeys');
    toast('Primary key registered. Insert the next trusted key to finish this plan.');
    return;
  }

  wizCompleteFido2Setup();
}

async function wizSetNewPin() {
  await submitNewFido2Pin(
    'wizNewFido2Pin',
    'wizNewFido2PinConfirm',
    'wizDeviceHint',
    'wizFido2Pin',
    'wizFido2Label'
  );
}

async function wizSetAdditionalKeyPin() {
  await submitNewFido2Pin(
    'wizAdditionalNewPin',
    'wizAdditionalNewPinConfirm',
    'wizAdditionalKeyStatus',
    'wizAdditionalKeyPin',
    'wizAdditionalKeyLabel'
  );
}

async function wizRegisterAdditionalKey() {
  const pin = document.getElementById('wizAdditionalKeyPin').value;
  const label = document.getElementById('wizAdditionalKeyLabel').value;
  if (!label) { toast('Label required', 'error'); return; }

  toast('Touch your hardware key now...');
  const body = { label };
  if (pin) body.pin = pin;
  const r = await api('POST', '/api/fido2/register', body);
  if (r.error) {
    const message = friendlyFidoError(r.error);
    setInlineInfo('wizAdditionalKeyStatus', message);
    toast(message, 'error');
    return;
  }

  wizRegisteredKeyCount = r.total_keys || (wizRegisteredKeyCount + 1);
  clearFields(['wizAdditionalKeyPin', 'wizAdditionalKeyLabel', 'wizAdditionalNewPin', 'wizAdditionalNewPinConfirm']);
  wizRenderAdditionalKeyState();

  if (wizRegisteredKeyCount < wizRequiredKeyCount) {
    const remaining = wizRequiredKeyCount - wizRegisteredKeyCount;
    toast('Key "' + label + '" registered. Insert ' + remaining + ' more key' + (remaining > 1 ? 's' : '') + ' to finish this plan.');
    return;
  }

  toast('Key "' + label + '" registered. Your plan now has enough enrolled hardware keys.');
  wizCompleteFido2Setup();
}

function wizFinishForNow() {
  wizCompleteFido2Setup();
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

function statBox(value, label) {
  return '<div class="stat"><div class="value" style="font-size:16px;">' + esc(String(value)) +
    '</div><div class="label">' + esc(label) + '</div></div>';
}

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}
function escAttr(s) {
  return esc(s).replace(/'/g, '&#39;');
}

function coerceActionArg(value, type) {
  if (type === 'number') {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : value;
  }
  return value;
}

function collectActionArgs(actionEl) {
  const args = [];
  for (let index = 0; ; index += 1) {
    const key = 'arg' + index;
    if (!(key in actionEl.dataset)) break;
    args.push(coerceActionArg(
      actionEl.dataset[key],
      actionEl.dataset[key + 'Type']
    ));
  }
  if (actionEl.dataset.self === 'append') args.push(actionEl);
  return args;
}

function dispatchDataAction(actionEl) {
  const actionName = actionEl.dataset.action;
  const action = globalThis[actionName];
  if (typeof action !== 'function') {
    console.warn('Unknown UI action:', actionName);
    return;
  }
  const shouldShowBusy =
    actionEl.tagName === 'BUTTON' &&
    !actionEl.disabled &&
    !['selectWorkspaceSection', 'switchUnlockTab', 'togglePoisonWarning'].includes(actionName);
  if (shouldShowBusy) {
    actionEl.disabled = true;
    actionEl.classList.add('is-busy');
    actionEl.setAttribute('aria-busy', 'true');
  }
  Promise.resolve(action(...collectActionArgs(actionEl)))
    .catch(error => {
      console.error('UI action failed:', actionName, error);
      toast('Action failed: ' + actionName, 'error');
    })
    .finally(() => {
      if (!shouldShowBusy || !actionEl.isConnected) return;
      actionEl.disabled = false;
      actionEl.classList.remove('is-busy');
      actionEl.removeAttribute('aria-busy');
    });
}

function handleActionEvent(event) {
  const actionEl = event.target instanceof Element
    ? event.target.closest('[data-action]')
    : null;
  if (!actionEl) return;
  if (actionEl.tagName === 'BUTTON') event.preventDefault();
  dispatchDataAction(actionEl);
}

document.addEventListener('keydown', e => {
  if (e.key !== 'Enter') return;
  if (e.target.id === 'passphrase') unlock();
  if (e.target.id === 'fido2Pin') fido2Unlock();
  if (e.target.id === 'wizPassphraseConfirm') wizInitPassphrase();
  if (e.target.id === 'wizFido2Label') wizRegisterKey();
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

