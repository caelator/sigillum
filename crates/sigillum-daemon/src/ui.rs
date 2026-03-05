/// Embedded single-page web UI for Sigillum vault management.
/// Served directly from the binary — no external files needed.
pub const INDEX_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Sigillum Vault</title>
<style>
  :root {
    --bg: #0a0a0f;
    --surface: #12121a;
    --border: #1e1e2e;
    --text: #e0e0e8;
    --text-dim: #6e6e8a;
    --accent: #7c6ff0;
    --accent-hover: #9488f5;
    --danger: #e5484d;
    --success: #46a758;
    --warning: #f5a623;
    --radius: 8px;
    --font: -apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif;
    --mono: 'SF Mono', 'Cascadia Code', 'JetBrains Mono', monospace;
  }
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body {
    font-family: var(--font);
    background: var(--bg);
    color: var(--text);
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    align-items: center;
  }
  header {
    width: 100%;
    padding: 24px 32px;
    border-bottom: 1px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .logo {
    font-size: 20px;
    font-weight: 700;
    letter-spacing: 0.05em;
  }
  .logo span { color: var(--accent); }
  .status-badge {
    padding: 6px 14px;
    border-radius: 20px;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
  .status-locked { background: rgba(229,72,77,0.15); color: var(--danger); }
  .status-unlocked { background: rgba(70,167,88,0.15); color: var(--success); }
  .status-no-vault { background: rgba(245,166,35,0.15); color: var(--warning); }
  main {
    width: 100%;
    max-width: 720px;
    padding: 32px 24px;
    flex: 1;
  }
  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 24px;
    margin-bottom: 20px;
  }
  .card h2 {
    font-size: 14px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    margin-bottom: 16px;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
    gap: 12px;
  }
  .stat {
    text-align: center;
    padding: 16px;
    background: var(--bg);
    border-radius: var(--radius);
  }
  .stat .value {
    font-size: 28px;
    font-weight: 700;
    font-family: var(--mono);
  }
  .stat .label {
    font-size: 11px;
    color: var(--text-dim);
    margin-top: 4px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .form-row {
    display: flex;
    gap: 10px;
    margin-bottom: 12px;
  }
  input[type="text"], input[type="password"], input[type="number"] {
    flex: 1;
    padding: 10px 14px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text);
    font-family: var(--mono);
    font-size: 14px;
    outline: none;
    transition: border-color 0.15s;
  }
  input:focus { border-color: var(--accent); }
  button {
    padding: 10px 20px;
    border: none;
    border-radius: var(--radius);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.15s;
    white-space: nowrap;
  }
  .btn-primary { background: var(--accent); color: #fff; }
  .btn-primary:hover { background: var(--accent-hover); }
  .btn-danger { background: rgba(229,72,77,0.15); color: var(--danger); }
  .btn-danger:hover { background: rgba(229,72,77,0.25); }
  .btn-ghost {
    background: transparent;
    color: var(--text-dim);
    border: 1px solid var(--border);
  }
  .btn-ghost:hover { border-color: var(--text-dim); color: var(--text); }
  .btn-success { background: rgba(70,167,88,0.15); color: var(--success); }
  .btn-success:hover { background: rgba(70,167,88,0.25); }
  .key-list { list-style: none; }
  .key-list li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
    font-family: var(--mono);
    font-size: 13px;
  }
  .key-list li:last-child { border-bottom: none; }
  .key-actions { display: flex; gap: 6px; }
  .key-actions button {
    padding: 4px 10px;
    font-size: 11px;
  }
  .toast {
    position: fixed;
    bottom: 24px;
    right: 24px;
    padding: 12px 20px;
    border-radius: var(--radius);
    font-size: 13px;
    font-weight: 500;
    z-index: 100;
    animation: fadeIn 0.2s ease;
  }
  .toast-success { background: rgba(70,167,88,0.9); color: #fff; }
  .toast-error { background: rgba(229,72,77,0.9); color: #fff; }
  @keyframes fadeIn { from { opacity: 0; transform: translateY(8px); } }
  .hidden { display: none; }
  .secret-value {
    font-family: var(--mono);
    font-size: 13px;
    padding: 8px 12px;
    background: var(--bg);
    border-radius: var(--radius);
    margin-top: 8px;
    word-break: break-all;
  }
  footer {
    padding: 16px;
    text-align: center;
    font-size: 11px;
    color: var(--text-dim);
    border-top: 1px solid var(--border);
    width: 100%;
  }
  footer a { color: var(--accent); text-decoration: none; }
  /* Setup wizard */
  .wizard-step {
    display: none;
  }
  .wizard-step.active {
    display: block;
  }
  .wizard-step p {
    color: var(--text-dim);
    font-size: 13px;
    margin-bottom: 14px;
    line-height: 1.6;
  }
  .method-btn {
    display: block;
    width: 100%;
    padding: 16px 20px;
    margin-bottom: 10px;
    text-align: left;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text);
    cursor: pointer;
    transition: border-color 0.15s;
  }
  .method-btn:hover { border-color: var(--accent); }
  .method-btn .method-title {
    font-weight: 600;
    font-size: 14px;
    margin-bottom: 4px;
  }
  .method-btn .method-desc {
    font-size: 12px;
    color: var(--text-dim);
  }
  .method-btn.recommended {
    border-color: var(--accent);
  }
  .method-btn.recommended .method-title::after {
    content: ' (recommended)';
    color: var(--accent);
    font-weight: 400;
    font-size: 12px;
  }
  .pulse {
    animation: pulse 1.5s infinite;
  }
  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
  }
  .info-box {
    padding: 12px 16px;
    background: rgba(124,111,240,0.08);
    border: 1px solid rgba(124,111,240,0.2);
    border-radius: var(--radius);
    font-size: 13px;
    color: var(--text-dim);
    margin-bottom: 14px;
  }
  .unlock-tabs {
    display: flex;
    gap: 8px;
    margin-bottom: 14px;
  }
  .unlock-tab {
    padding: 8px 16px;
    border-radius: var(--radius);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    background: var(--bg);
    border: 1px solid var(--border);
    color: var(--text-dim);
    transition: all 0.15s;
  }
  .unlock-tab.active {
    border-color: var(--accent);
    color: var(--accent);
    background: rgba(124,111,240,0.08);
  }
</style>
</head>
<body>

<header>
  <div class="logo"><span>SIGILLUM</span> VAULT</div>
  <div id="statusBadge" class="status-badge status-locked">checking...</div>
</header>

<main>
  <!-- Status Card -->
  <div class="card">
    <h2>Status</h2>
    <div class="stats">
      <div class="stat">
        <div class="value" id="apiKeyCount">-</div>
        <div class="label">API Keys</div>
      </div>
      <div class="stat">
        <div class="value" id="secretCount">-</div>
        <div class="label">Secrets</div>
      </div>
      <div class="stat">
        <div class="value" id="fido2KeyCount">-</div>
        <div class="label">FIDO2 Keys</div>
      </div>
      <div class="stat">
        <div class="value" id="unlockMethod">-</div>
        <div class="label">Unlock Method</div>
      </div>
    </div>
  </div>

  <!-- Unlock / Lock Card -->
  <div class="card" id="authCard">
    <h2 id="authTitle">Unlock Vault</h2>
    <div id="unlockTabs" class="unlock-tabs hidden">
      <div class="unlock-tab active" onclick="switchUnlockTab('passphrase')">Passphrase</div>
      <div class="unlock-tab" onclick="switchUnlockTab('fido2')">Hardware Key</div>
    </div>
    <div id="unlockPassphrase">
      <div class="form-row">
        <input type="password" id="passphrase" placeholder="Passphrase">
        <button class="btn-primary" onclick="unlock()">Unlock</button>
      </div>
    </div>
    <div id="unlockFido2" class="hidden">
      <div class="form-row">
        <input type="password" id="fido2Pin" placeholder="FIDO2 PIN">
        <button class="btn-primary" onclick="fido2Unlock()">Unlock with Key</button>
      </div>
      <p style="color:var(--text-dim);font-size:12px;margin-top:8px;">
        Enter your PIN, click unlock, then touch your hardware key.
      </p>
    </div>
    <div id="lockForm" class="hidden">
      <button class="btn-danger" onclick="lock()">Lock Vault</button>
    </div>
  </div>

  <!-- Setup Wizard (only if no vault) -->
  <div class="card hidden" id="setupCard">
    <h2>Setup Wizard</h2>

    <!-- Step 1: Choose method -->
    <div class="wizard-step active" id="wizStep1">
      <p>No vault found. Choose how to protect your secrets:</p>
      <button class="method-btn recommended" onclick="wizChooseMethod('fido2')" id="wizFido2Btn">
        <div class="method-title">Hardware Key (FIDO2)</div>
        <div class="method-desc">Use a YubiKey or similar CTAP2 device. Highest security.</div>
      </button>
      <button class="method-btn" onclick="wizChooseMethod('passphrase')">
        <div class="method-title">Passphrase Only</div>
        <div class="method-desc">Protect vault with a passphrase. Standard security.</div>
      </button>
      <div id="wizNoDevice" class="info-box hidden">
        No FIDO2 device detected. Insert your hardware key and click "Detect" or choose passphrase.
        <div style="margin-top:8px">
          <button class="btn-ghost" onclick="wizDetectDevice()">Detect Device</button>
        </div>
      </div>
    </div>

    <!-- Step 2a: Passphrase setup -->
    <div class="wizard-step" id="wizStepPassphrase">
      <p>Choose a strong passphrase (minimum 8 characters).</p>
      <div class="form-row">
        <input type="password" id="wizPassphrase" placeholder="Passphrase">
      </div>
      <div class="form-row">
        <input type="password" id="wizPassphraseConfirm" placeholder="Confirm passphrase">
        <button class="btn-primary" onclick="wizInitPassphrase()">Create Vault</button>
      </div>
    </div>

    <!-- Step 2b: FIDO2 PIN + label -->
    <div class="wizard-step" id="wizStepFido2Pin">
      <p>Enter your FIDO2 device PIN and a label for this key.</p>
      <div class="form-row">
        <input type="password" id="wizFido2Pin" placeholder="FIDO2 PIN">
      </div>
      <div class="form-row">
        <input type="text" id="wizFido2Label" placeholder="Key label (e.g. yubikey-primary)">
        <button class="btn-primary" onclick="wizRegisterKey()">Register Key</button>
      </div>
    </div>

    <!-- Step 3: Touch prompt -->
    <div class="wizard-step" id="wizStepTouch">
      <div class="info-box pulse" style="text-align:center;font-size:16px;padding:24px;">
        Touch your FIDO2 key now...
      </div>
    </div>

    <!-- Step 4: Success -->
    <div class="wizard-step" id="wizStepSuccess">
      <div style="text-align:center;padding:16px 0;">
        <div style="font-size:32px;margin-bottom:12px;color:var(--success);">&#10003;</div>
        <div style="font-size:16px;font-weight:600;" id="wizSuccessMsg">Vault created!</div>
        <p style="margin-top:8px;" id="wizSuccessDetail"></p>
      </div>
      <div style="margin-top:16px;">
        <p>Optional: Set a passphrase as backup unlock method?</p>
        <div class="form-row" style="margin-top:8px;">
          <input type="password" id="wizFallbackPassphrase" placeholder="Backup passphrase (optional)">
          <button class="btn-ghost" onclick="wizSetFallback()">Set</button>
          <button class="btn-ghost" onclick="wizFinish()">Skip</button>
        </div>
      </div>
    </div>

    <!-- Step 5: Done -->
    <div class="wizard-step" id="wizStepDone">
      <div style="text-align:center;padding:24px 0;">
        <div style="font-size:32px;margin-bottom:12px;color:var(--success);">&#10003;</div>
        <div style="font-size:16px;font-weight:600;">Setup Complete</div>
        <p style="margin-top:8px;">Your vault is ready. The page will refresh momentarily.</p>
      </div>
    </div>
  </div>

  <!-- API Keys Card -->
  <div class="card">
    <h2>Tier 1 &mdash; API Keys</h2>
    <div class="form-row">
      <input type="text" id="apiKeyName" placeholder="Key name">
      <input type="password" id="apiKeyValue" placeholder="Value">
      <button class="btn-primary" onclick="setApiKey()">Set</button>
    </div>
    <ul class="key-list" id="apiKeyList"></ul>
  </div>

  <!-- Secrets Card -->
  <div class="card" id="secretsCard">
    <h2>Tier 2 &mdash; Encrypted Secrets</h2>
    <div id="secretsLocked" class="hidden" style="color:var(--text-dim);font-size:13px;">
      Vault is locked. Unlock to manage secrets.
    </div>
    <div id="secretsUnlocked">
      <div class="form-row">
        <input type="text" id="secretName" placeholder="Secret name">
        <input type="password" id="secretValue" placeholder="Value">
        <button class="btn-primary" onclick="setSecret()">Set</button>
      </div>
      <ul class="key-list" id="secretList"></ul>
    </div>
  </div>
</main>

<footer>
  Sigillum &mdash; hardware-backed secret management &mdash;
  <a href="https://github.com/caelator/sigillum">GitHub</a>
</footer>

<script>
const API = '';

async function api(method, path, body) {
  const opts = { method, headers: { 'Content-Type': 'application/json' } };
  if (body) opts.body = JSON.stringify(body);
  const r = await fetch(API + path, opts);
  return r.json();
}

function toast(msg, type = 'success') {
  const el = document.createElement('div');
  el.className = 'toast toast-' + type;
  el.textContent = msg;
  document.body.appendChild(el);
  setTimeout(() => el.remove(), 3000);
}

let currentStatus = null;

async function refresh() {
  const s = await api('GET', '/api/status');
  currentStatus = s;
  const badge = document.getElementById('statusBadge');
  const authCard = document.getElementById('authCard');
  const setupCard = document.getElementById('setupCard');
  const secretsLocked = document.getElementById('secretsLocked');
  const secretsUnlocked = document.getElementById('secretsUnlocked');

  document.getElementById('apiKeyCount').textContent = s.api_key_count;

  // FIDO2 info
  const fido = s.fido2 || {};
  document.getElementById('fido2KeyCount').textContent = fido.key_count || 0;
  document.getElementById('unlockMethod').textContent = fido.unlock_method || 'passphrase';

  if (!s.vault_exists) {
    badge.className = 'status-badge status-no-vault';
    badge.textContent = 'NO VAULT';
    setupCard.classList.remove('hidden');
    authCard.classList.add('hidden');
    secretsLocked.classList.remove('hidden');
    secretsUnlocked.classList.add('hidden');
    document.getElementById('secretCount').textContent = '-';

    // Check for FIDO2 device presence
    wizDetectDevice();
    return;
  }

  setupCard.classList.add('hidden');
  authCard.classList.remove('hidden');

  // Show unlock tabs if FIDO2 is enabled
  const unlockTabs = document.getElementById('unlockTabs');
  const method = fido.unlock_method || 'passphrase';
  if (method === 'fido2' || method === 'both') {
    unlockTabs.classList.remove('hidden');
  } else {
    unlockTabs.classList.add('hidden');
  }

  if (s.unlocked) {
    badge.className = 'status-badge status-unlocked';
    badge.textContent = 'UNLOCKED';
    document.getElementById('unlockPassphrase').classList.add('hidden');
    document.getElementById('unlockFido2').classList.add('hidden');
    document.getElementById('unlockTabs').classList.add('hidden');
    document.getElementById('lockForm').classList.remove('hidden');
    document.getElementById('authTitle').textContent = 'Vault Unlocked';
    secretsLocked.classList.add('hidden');
    secretsUnlocked.classList.remove('hidden');
    document.getElementById('secretCount').textContent = s.secret_count;
    await loadSecrets();
  } else {
    badge.className = 'status-badge status-locked';
    badge.textContent = 'LOCKED';
    document.getElementById('lockForm').classList.add('hidden');
    document.getElementById('authTitle').textContent = 'Unlock Vault';
    secretsLocked.classList.remove('hidden');
    secretsUnlocked.classList.add('hidden');
    document.getElementById('secretCount').textContent = '(locked)';

    // Show appropriate unlock method
    if (method === 'fido2') {
      switchUnlockTab('fido2');
    } else {
      switchUnlockTab('passphrase');
    }
  }

  await loadApiKeys();
}

function switchUnlockTab(tab) {
  document.querySelectorAll('.unlock-tab').forEach(t => t.classList.remove('active'));
  if (tab === 'fido2') {
    document.getElementById('unlockPassphrase').classList.add('hidden');
    document.getElementById('unlockFido2').classList.remove('hidden');
    document.querySelectorAll('.unlock-tab')[1].classList.add('active');
  } else {
    document.getElementById('unlockPassphrase').classList.remove('hidden');
    document.getElementById('unlockFido2').classList.add('hidden');
    document.querySelectorAll('.unlock-tab')[0].classList.add('active');
  }
}

async function unlock() {
  const p = document.getElementById('passphrase').value;
  if (!p) return;
  const r = await api('POST', '/api/unlock', { passphrase: p });
  if (r.error) { toast(r.error, 'error'); return; }
  document.getElementById('passphrase').value = '';
  toast('Vault unlocked');
  refresh();
}

async function fido2Unlock() {
  const pin = document.getElementById('fido2Pin').value;
  if (!pin) { toast('PIN is required', 'error'); return; }
  toast('Touch your hardware key now...');
  const r = await api('POST', '/api/fido2/unlock', { pins: [pin] });
  if (r.error) { toast(r.error, 'error'); return; }
  document.getElementById('fido2Pin').value = '';
  toast('Vault unlocked (FIDO2)');
  refresh();
}

async function lock() {
  await api('POST', '/api/lock');
  toast('Vault locked');
  refresh();
}

// ── Setup Wizard ──────────────────────────────────────────────

function wizShowStep(id) {
  document.querySelectorAll('.wizard-step').forEach(s => s.classList.remove('active'));
  document.getElementById(id).classList.add('active');
}

async function wizDetectDevice() {
  try {
    const r = await api('GET', '/api/fido2/detect');
    const btn = document.getElementById('wizFido2Btn');
    const noDevice = document.getElementById('wizNoDevice');
    if (r.device_present) {
      btn.classList.add('recommended');
      btn.querySelector('.method-desc').textContent =
        r.device_count + ' device(s) detected. Highest security.';
      noDevice.classList.add('hidden');
    } else {
      btn.classList.remove('recommended');
      noDevice.classList.remove('hidden');
    }
  } catch(e) {}
}

function wizChooseMethod(method) {
  if (method === 'fido2') {
    wizShowStep('wizStepFido2Pin');
  } else {
    wizShowStep('wizStepPassphrase');
  }
}

async function wizInitPassphrase() {
  const p = document.getElementById('wizPassphrase').value;
  const pc = document.getElementById('wizPassphraseConfirm').value;
  if (p.length < 8) { toast('Passphrase must be at least 8 characters', 'error'); return; }
  if (p !== pc) { toast('Passphrases do not match', 'error'); return; }
  const r = await api('POST', '/api/init', { passphrase: p });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Vault initialized and unlocked');
  wizShowStep('wizStepDone');
  setTimeout(refresh, 1500);
}

async function wizRegisterKey() {
  const pin = document.getElementById('wizFido2Pin').value;
  const label = document.getElementById('wizFido2Label').value;
  if (!pin) { toast('PIN is required', 'error'); return; }
  if (!label) { toast('Label is required', 'error'); return; }

  wizShowStep('wizStepTouch');

  const r = await api('POST', '/api/fido2/setup', { pin, label });
  if (r.error) {
    toast(r.error, 'error');
    wizShowStep('wizStepFido2Pin');
    return;
  }

  document.getElementById('wizSuccessMsg').textContent = 'FIDO2 key registered!';
  document.getElementById('wizSuccessDetail').textContent =
    'Key "' + label + '" registered. Vault created and unlocked.';
  wizShowStep('wizStepSuccess');
}

async function wizSetFallback() {
  const p = document.getElementById('wizFallbackPassphrase').value;
  if (p.length < 8) { toast('Passphrase must be at least 8 characters', 'error'); return; }

  // We need to init the passphrase wrapped key via the API
  // For now, use the existing init endpoint concept — but the vault already exists.
  // The setup endpoint already handled passphrase if provided.
  // Re-call setup with passphrase isn't ideal. Let's just tell user to use CLI for now.
  // Actually, let's re-do the setup call but the vault already exists.
  // The proper way is to wrap the master key with the passphrase.
  // Since the vault is unlocked and we have the master key in daemon memory,
  // we can re-init the passphrase salt. But we need a dedicated endpoint.
  // For simplicity, let's save via a POST to /api/init with the passphrase
  // and handle the "already exists" case. Actually this won't work.

  // Simplest: tell the user this was set during setup.
  // The fido2/setup endpoint accepts a passphrase parameter.
  toast('Use "sigillum setup" CLI to add passphrase fallback to existing FIDO2 vault.', 'error');
  wizFinish();
}

function wizFinish() {
  wizShowStep('wizStepDone');
  setTimeout(refresh, 1500);
}

// ── API Keys & Secrets ────────────────────────────────────────

async function loadApiKeys() {
  const r = await api('GET', '/api/api-keys');
  const list = document.getElementById('apiKeyList');
  list.innerHTML = '';
  (r.keys || []).forEach(k => {
    const li = document.createElement('li');
    li.innerHTML = '<span>' + esc(k) + '</span>' +
      '<div class="key-actions">' +
      '<button class="btn-ghost" onclick="revealApiKey(\'' + esc(k) + '\', this)">Reveal</button>' +
      '<button class="btn-danger" onclick="deleteApiKey(\'' + esc(k) + '\')">Delete</button>' +
      '</div>';
    list.appendChild(li);
  });
}

async function loadSecrets() {
  const r = await api('GET', '/api/secrets');
  const list = document.getElementById('secretList');
  list.innerHTML = '';
  (r.keys || []).forEach(k => {
    const li = document.createElement('li');
    li.innerHTML = '<span>' + esc(k) + '</span>' +
      '<div class="key-actions">' +
      '<button class="btn-ghost" onclick="revealSecret(\'' + esc(k) + '\', this)">Reveal</button>' +
      '<button class="btn-danger" onclick="deleteSecret(\'' + esc(k) + '\')">Delete</button>' +
      '</div>';
    list.appendChild(li);
  });
}

async function setApiKey() {
  const key = document.getElementById('apiKeyName').value;
  const value = document.getElementById('apiKeyValue').value;
  if (!key || !value) { toast('Key and value required', 'error'); return; }
  const r = await api('POST', '/api/api-keys/set', { key, value });
  if (r.error) { toast(r.error, 'error'); return; }
  document.getElementById('apiKeyName').value = '';
  document.getElementById('apiKeyValue').value = '';
  toast('API key stored');
  refresh();
}

async function setSecret() {
  const key = document.getElementById('secretName').value;
  const value = document.getElementById('secretValue').value;
  if (!key || !value) { toast('Key and value required', 'error'); return; }
  const r = await api('POST', '/api/secrets/set', { key, value });
  if (r.error) { toast(r.error, 'error'); return; }
  document.getElementById('secretName').value = '';
  document.getElementById('secretValue').value = '';
  toast('Secret stored');
  refresh();
}

async function revealApiKey(key, btn) {
  const r = await api('POST', '/api/api-keys/get', { key });
  if (r.error) { toast(r.error, 'error'); return; }
  const li = btn.closest('li');
  let existing = li.querySelector('.secret-value');
  if (existing) { existing.remove(); btn.textContent = 'Reveal'; return; }
  const div = document.createElement('div');
  div.className = 'secret-value';
  div.textContent = r.value;
  li.appendChild(div);
  btn.textContent = 'Hide';
}

async function revealSecret(key, btn) {
  const r = await api('POST', '/api/secrets/get', { key });
  if (r.error) { toast(r.error, 'error'); return; }
  const li = btn.closest('li');
  let existing = li.querySelector('.secret-value');
  if (existing) { existing.remove(); btn.textContent = 'Reveal'; return; }
  const div = document.createElement('div');
  div.className = 'secret-value';
  div.textContent = r.value;
  li.appendChild(div);
  btn.textContent = 'Hide';
}

async function deleteApiKey(key) {
  if (!confirm('Delete API key "' + key + '"?')) return;
  const r = await api('POST', '/api/api-keys/delete', { key });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('API key deleted');
  refresh();
}

async function deleteSecret(key) {
  if (!confirm('Delete secret "' + key + '"?')) return;
  const r = await api('POST', '/api/secrets/delete', { key });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Secret deleted');
  refresh();
}

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}

// Enter key support
document.addEventListener('keydown', e => {
  if (e.key !== 'Enter') return;
  if (e.target.id === 'passphrase') unlock();
  if (e.target.id === 'fido2Pin') fido2Unlock();
  if (e.target.id === 'wizPassphraseConfirm') wizInitPassphrase();
  if (e.target.id === 'wizFido2Label') wizRegisterKey();
});

refresh();
setInterval(refresh, 5000);
</script>
</body>
</html>
"##;
