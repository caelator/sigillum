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
  .header-right { display: flex; align-items: center; gap: 12px; }
  .compartment-badge {
    padding: 6px 14px;
    border-radius: 20px;
    font-size: 12px;
    font-weight: 600;
    background: rgba(124,111,240,0.15);
    color: var(--accent);
    letter-spacing: 0.04em;
  }
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
  input[type="text"], input[type="password"], input[type="number"], select {
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
  input:focus, select:focus { border-color: var(--accent); }
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
  .wizard-step { display: none; }
  .wizard-step.active { display: block; }
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
  .method-btn.recommended { border-color: var(--accent); }
  .method-btn.recommended .method-title::after {
    content: ' (recommended)';
    color: var(--accent);
    font-weight: 400;
    font-size: 12px;
  }
  .pulse { animation: pulse 1.5s infinite; }
  @keyframes pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.5; } }
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
  .compartment-hint {
    padding: 8px 14px;
    margin-bottom: 6px;
    background: var(--bg);
    border-radius: var(--radius);
    font-size: 12px;
    color: var(--text-dim);
    display: flex;
    justify-content: space-between;
  }
  .compartment-hint .hint-label { color: var(--text); font-weight: 500; }
  .compartment-hint .hint-threshold { color: var(--accent); font-family: var(--mono); }
</style>
</head>
<body>

<header>
  <div class="logo"><span>SIGILLUM</span> VAULT</div>
  <div class="header-right">
    <div id="compartmentBadge" class="compartment-badge hidden"></div>
    <div id="statusBadge" class="status-badge status-locked">checking...</div>
  </div>
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
        <div class="value" id="compartmentCount">-</div>
        <div class="label">Compartments</div>
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
      <p style="color:var(--text-dim);font-size:12px;margin-top:4px;">
        Different passphrases unlock different compartments.
      </p>
    </div>
    <div id="unlockFido2" class="hidden">
      <div id="fido2Hints" style="margin-bottom:10px;"></div>
      <div class="form-row">
        <input type="password" id="fido2Pin" placeholder="FIDO2 PIN">
        <select id="fido2TapCount" style="flex:none;width:140px;"></select>
        <button class="btn-primary" onclick="fido2Unlock()">Unlock</button>
      </div>
      <p style="color:var(--text-dim);font-size:12px;margin-top:4px;">
        Select the tap count, enter PIN, click unlock, then touch your key(s).
      </p>
    </div>
    <div id="lockForm" class="hidden">
      <button class="btn-danger" onclick="lock()">Lock All Compartments</button>
    </div>
  </div>

  <!-- Setup Wizard -->
  <div class="card hidden" id="setupCard">
    <h2>Setup Wizard</h2>

    <!-- Step 1: Choose method -->
    <div class="wizard-step active" id="wizStep1">
      <p>No vault found. Choose how to protect your secrets:</p>
      <button class="method-btn recommended" onclick="wizChooseMethod('fido2')" id="wizFido2Btn">
        <div class="method-title">Hardware Key (FIDO2)</div>
        <div class="method-desc">Use YubiKeys with threshold compartments. Highest security.</div>
      </button>
      <button class="method-btn" onclick="wizChooseMethod('passphrase')">
        <div class="method-title">Passphrase Only</div>
        <div class="method-desc">Create compartments with different passphrases.</div>
      </button>
      <div id="wizNoDevice" class="info-box hidden">
        No FIDO2 device detected. Insert your hardware key and click "Detect" or choose passphrase.
        <div style="margin-top:8px">
          <button class="btn-ghost" onclick="wizDetectDevice()">Detect Device</button>
        </div>
      </div>
    </div>

    <!-- Step 2a: Passphrase compartment setup -->
    <div class="wizard-step" id="wizStepPassphrase">
      <p>Create a compartment with a passphrase. You can add more compartments later.</p>
      <div class="form-row">
        <input type="text" id="wizPLabel" placeholder="Compartment name (e.g. default)" value="default">
      </div>
      <div class="form-row">
        <input type="password" id="wizPassphrase" placeholder="Passphrase (min 8 chars)">
      </div>
      <div class="form-row">
        <input type="password" id="wizPassphraseConfirm" placeholder="Confirm passphrase">
        <button class="btn-primary" onclick="wizInitPassphrase()">Create Vault</button>
      </div>
    </div>

    <!-- Step 2b: FIDO2 compartment presets -->
    <div class="wizard-step" id="wizStepCompartments">
      <p>Define compartments. Each has a unique tap-count threshold.</p>
      <div id="wizCompList">
        <div class="compartment-hint">
          <span class="hint-label">Hot (daily)</span>
          <span class="hint-threshold">Tap 1 key</span>
        </div>
        <div class="compartment-hint">
          <span class="hint-label">Cold (long-term)</span>
          <span class="hint-threshold">Tap 2 keys</span>
        </div>
        <div class="compartment-hint">
          <span class="hint-label">Legacy (estate)</span>
          <span class="hint-threshold">Tap 3 keys</span>
        </div>
      </div>
      <div style="margin-top:12px;">
        <button class="btn-primary" onclick="wizProceedFido2()">Continue with these compartments</button>
        <button class="btn-ghost" style="margin-left:8px;" onclick="wizCustomCompartments()">Customize</button>
      </div>
    </div>

    <!-- Step 2c: Custom compartments -->
    <div class="wizard-step" id="wizStepCustomComps">
      <p>Add compartments with custom labels and thresholds.</p>
      <div id="wizCustomCompList"></div>
      <div class="form-row">
        <input type="text" id="wizCustomLabel" placeholder="Label">
        <input type="number" id="wizCustomThreshold" placeholder="Threshold" min="1" style="width:90px;flex:none;">
        <button class="btn-ghost" onclick="wizAddCustomComp()">Add</button>
      </div>
      <div style="margin-top:12px;">
        <button class="btn-primary" id="wizCustomContinue" onclick="wizProceedFido2()" disabled>Continue</button>
      </div>
    </div>

    <!-- Step 3: FIDO2 PIN + label -->
    <div class="wizard-step" id="wizStepFido2Pin">
      <p>Register your first FIDO2 hardware key.</p>
      <div class="form-row">
        <input type="password" id="wizFido2Pin" placeholder="FIDO2 PIN">
      </div>
      <div class="form-row">
        <input type="text" id="wizFido2Label" placeholder="Key label (e.g. yubikey-primary)">
        <button class="btn-primary" onclick="wizRegisterKey()">Register Key</button>
      </div>
      <p>Optional: set a backup passphrase for all compartments.</p>
      <div class="form-row">
        <input type="password" id="wizBackupPass" placeholder="Backup passphrase (optional)">
      </div>
    </div>

    <!-- Step 4: Touch -->
    <div class="wizard-step" id="wizStepTouch">
      <div class="info-box pulse" style="text-align:center;font-size:16px;padding:24px;">
        Touch your FIDO2 key now...
      </div>
    </div>

    <!-- Step 5: Done -->
    <div class="wizard-step" id="wizStepDone">
      <div style="text-align:center;padding:24px 0;">
        <div style="font-size:32px;margin-bottom:12px;color:var(--success);">&#10003;</div>
        <div style="font-size:16px;font-weight:600;" id="wizDoneMsg">Setup Complete</div>
        <p style="margin-top:8px;" id="wizDoneDetail">Your vault is ready.</p>
      </div>
    </div>
  </div>

  <!-- Compartment Management Card -->
  <div class="card hidden" id="compartmentCard">
    <h2>Compartments</h2>
    <div id="compartmentList"></div>
    <div style="margin-top:14px;border-top:1px solid var(--border);padding-top:14px;">
      <p style="color:var(--text-dim);font-size:12px;margin-bottom:8px;">Add a new compartment:</p>
      <div class="form-row">
        <input type="text" id="compAddLabel" placeholder="Label">
        <input type="number" id="compAddThreshold" placeholder="Threshold" min="1" style="width:90px;flex:none;">
        <button class="btn-ghost" onclick="addCompartment()">Add</button>
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
      Vault is locked. Unlock a compartment to manage secrets.
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

  <!-- FIDO2 Management Card -->
  <div class="card hidden" id="fido2Card">
    <h2>FIDO2 Hardware Keys</h2>
    <div id="fido2DeviceStatus" class="info-box">Checking for devices...</div>
    <div id="fido2RegisterSection">
      <p style="color:var(--text-dim);font-size:13px;margin-bottom:10px;">
        Register a FIDO2 hardware key. All compartments must be unlocked first.
      </p>
      <div class="form-row">
        <input type="password" id="fido2RegPin" placeholder="FIDO2 PIN">
        <input type="text" id="fido2RegLabel" placeholder="Key label">
        <button class="btn-primary" onclick="fido2Register()">Register Key</button>
      </div>
    </div>
    <div id="fido2KeyListSection" style="margin-top:14px;"></div>
  </div>
</main>

<footer>
  Sigillum &mdash; hardware-backed secret management &mdash;
  <a href="https://github.com/caelator/sigillum">GitHub</a>
</footer>

<script>
const API = '';
let currentStatus = null;
let wizCompartments = [
  { label: 'hot', threshold: 1 },
  { label: 'cold', threshold: 2 },
  { label: 'legacy', threshold: 3 },
];
let customCompartments = [];

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

async function refresh() {
  const s = await api('GET', '/api/status');
  currentStatus = s;
  const badge = document.getElementById('statusBadge');
  const compBadge = document.getElementById('compartmentBadge');
  const authCard = document.getElementById('authCard');
  const setupCard = document.getElementById('setupCard');
  const compartmentCard = document.getElementById('compartmentCard');
  const secretsLocked = document.getElementById('secretsLocked');
  const secretsUnlocked = document.getElementById('secretsUnlocked');

  const fido = s.fido2 || {};
  const active = s.active_compartment;
  const hasCompartments = fido.compartments && fido.compartments.length > 0;

  document.getElementById('fido2KeyCount').textContent = fido.key_count || 0;
  document.getElementById('compartmentCount').textContent =
    (fido.compartments || []).length;

  if (!s.any_vault_exists && !hasCompartments) {
    badge.className = 'status-badge status-no-vault';
    badge.textContent = 'NO VAULT';
    compBadge.classList.add('hidden');
    setupCard.classList.remove('hidden');
    authCard.classList.add('hidden');
    compartmentCard.classList.add('hidden');
    secretsLocked.classList.remove('hidden');
    secretsUnlocked.classList.add('hidden');
    document.getElementById('apiKeyCount').textContent = '-';
    document.getElementById('secretCount').textContent = '-';
    wizDetectDevice();
    return;
  }

  setupCard.classList.add('hidden');
  authCard.classList.remove('hidden');
  compartmentCard.classList.remove('hidden');

  // Show unlock tabs if FIDO2 keys exist
  const unlockTabs = document.getElementById('unlockTabs');
  const hasFido = fido.key_count > 0;
  const hasPassphrase = (fido.compartments || []).some(c => c.has_passphrase);
  if (hasFido && hasPassphrase) {
    unlockTabs.classList.remove('hidden');
  } else if (hasFido) {
    unlockTabs.classList.remove('hidden');
  } else {
    unlockTabs.classList.add('hidden');
  }

  // Build FIDO2 unlock hints and tap count selector
  buildFido2Hints(fido.compartments || []);

  if (active) {
    badge.className = 'status-badge status-unlocked';
    badge.textContent = 'UNLOCKED';
    compBadge.textContent = active.compartment_label || ('Compartment ' + active.compartment_id);
    compBadge.classList.remove('hidden');
    document.getElementById('apiKeyCount').textContent = active.api_key_count || 0;
    document.getElementById('secretCount').textContent =
      active.secret_count != null ? active.secret_count : '(locked)';
    document.getElementById('unlockPassphrase').classList.add('hidden');
    document.getElementById('unlockFido2').classList.add('hidden');
    document.getElementById('unlockTabs').classList.add('hidden');
    document.getElementById('lockForm').classList.remove('hidden');
    document.getElementById('authTitle').textContent = 'Vault Unlocked';
    secretsLocked.classList.add('hidden');
    secretsUnlocked.classList.remove('hidden');
    await loadSecrets();
  } else {
    badge.className = 'status-badge status-locked';
    badge.textContent = 'LOCKED';
    compBadge.classList.add('hidden');
    document.getElementById('apiKeyCount').textContent = '-';
    document.getElementById('secretCount').textContent = '(locked)';
    document.getElementById('lockForm').classList.add('hidden');
    document.getElementById('authTitle').textContent = 'Unlock Vault';
    secretsLocked.classList.remove('hidden');
    secretsUnlocked.classList.add('hidden');

    if (hasFido && !hasPassphrase) {
      switchUnlockTab('fido2');
    } else {
      switchUnlockTab('passphrase');
    }
  }

  await loadApiKeys();
  await loadFido2();
  await loadCompartments();
}

function buildFido2Hints(compartments) {
  const hints = document.getElementById('fido2Hints');
  const select = document.getElementById('fido2TapCount');
  hints.innerHTML = '';
  select.innerHTML = '';
  compartments.forEach(c => {
    hints.innerHTML += '<div class="compartment-hint">' +
      '<span class="hint-label">' + esc(c.label) + '</span>' +
      '<span class="hint-threshold">Tap ' + c.threshold + ' key' + (c.threshold > 1 ? 's' : '') + '</span></div>';
    const opt = document.createElement('option');
    opt.value = c.threshold;
    opt.textContent = c.threshold + ' tap' + (c.threshold > 1 ? 's' : '') + ' = ' + c.label;
    select.appendChild(opt);
  });
}

async function loadCompartments() {
  try {
    const r = await api('GET', '/api/compartment/list');
    const el = document.getElementById('compartmentList');
    const comps = r.compartments || [];
    if (comps.length === 0) {
      el.innerHTML = '<p style="color:var(--text-dim);font-size:13px;">No compartments defined.</p>';
      return;
    }
    let html = '<ul class="key-list">';
    comps.forEach(c => {
      const active = c.is_active ? ' <span style="color:var(--success);">(active)</span>' : '';
      const exists = c.vault_exists ? '' : ' <span style="color:var(--warning);">(not initialized)</span>';
      html += '<li><span>' + esc(c.label) + ' <span style="color:var(--text-dim);font-size:11px;">' +
        'threshold=' + c.threshold + exists + active + '</span></span>' +
        '<div class="key-actions"><button class="btn-danger" onclick="removeCompartment(' + c.id + ')">Remove</button></div></li>';
    });
    html += '</ul>';
    el.innerHTML = html;
  } catch(e) {}
}

async function addCompartment() {
  const label = document.getElementById('compAddLabel').value;
  const threshold = parseInt(document.getElementById('compAddThreshold').value);
  if (!label) { toast('Label required', 'error'); return; }
  if (!threshold || threshold < 1) { toast('Valid threshold required', 'error'); return; }
  const r = await api('POST', '/api/compartment/add', { label, threshold });
  if (r.error) { toast(r.error, 'error'); return; }
  document.getElementById('compAddLabel').value = '';
  document.getElementById('compAddThreshold').value = '';
  toast('Compartment "' + label + '" added');
  refresh();
}

async function removeCompartment(id) {
  if (!confirm('Remove compartment ' + id + '?')) return;
  const r = await api('POST', '/api/compartment/remove', { id });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Compartment removed');
  refresh();
}

async function loadFido2() {
  const fido2Card = document.getElementById('fido2Card');
  if (!currentStatus || !currentStatus.any_vault_exists) {
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
    if (keys.keys && keys.keys.length > 0) {
      let html = '<ul class="key-list">';
      keys.keys.forEach(k => {
        html += '<li><span>' + esc(k.label) + ' <span style="color:var(--text-dim);font-size:11px;">(' +
          esc(k.credential_id_short) + '...) ' + esc(k.registered_at) + '</span></span>' +
          '<div class="key-actions"><button class="btn-danger" onclick="fido2RemoveKey(\'' +
          esc(k.label) + '\')">Remove</button></div></li>';
      });
      html += '</ul>';
      listEl.innerHTML = html;
    } else {
      listEl.innerHTML = '<p style="color:var(--text-dim);font-size:13px;">No hardware keys registered.</p>';
    }
  } catch(e) {}
}

async function fido2Register() {
  const pin = document.getElementById('fido2RegPin').value;
  const label = document.getElementById('fido2RegLabel').value;
  if (!pin) { toast('PIN required', 'error'); return; }
  if (!label) { toast('Label required', 'error'); return; }
  toast('Touch your FIDO2 key now...');
  const r = await api('POST', '/api/fido2/register', { pin, label });
  if (r.error) { toast(r.error, 'error'); return; }
  document.getElementById('fido2RegPin').value = '';
  document.getElementById('fido2RegLabel').value = '';
  toast('Key "' + label + '" registered');
  refresh();
}

async function fido2RemoveKey(label) {
  if (!confirm('Remove FIDO2 key "' + label + '"?')) return;
  const pin = prompt('Enter FIDO2 PIN:');
  if (!pin) return;
  const r = await api('POST', '/api/fido2/remove', { label, pin });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Key removed');
  refresh();
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
  toast('Unlocked: ' + (r.compartment_label || 'vault'));
  refresh();
}

async function fido2Unlock() {
  const pin = document.getElementById('fido2Pin').value;
  const tapCount = parseInt(document.getElementById('fido2TapCount').value);
  if (!pin) { toast('PIN required', 'error'); return; }
  if (!tapCount) { toast('Select tap count', 'error'); return; }
  toast('Touch your hardware key now...');
  const r = await api('POST', '/api/fido2/unlock', { pins: [pin], tap_count: tapCount });
  if (r.error) { toast(r.error, 'error'); return; }
  document.getElementById('fido2Pin').value = '';
  toast('Unlocked: ' + (r.compartment_label || 'vault'));
  refresh();
}

async function lock() {
  await api('POST', '/api/lock');
  toast('All compartments locked');
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
    wizShowStep('wizStepCompartments');
  } else {
    wizShowStep('wizStepPassphrase');
  }
}

async function wizInitPassphrase() {
  const label = document.getElementById('wizPLabel').value || 'default';
  const p = document.getElementById('wizPassphrase').value;
  const pc = document.getElementById('wizPassphraseConfirm').value;
  if (p.length < 8) { toast('Min 8 characters', 'error'); return; }
  if (p !== pc) { toast('Passphrases do not match', 'error'); return; }

  // Add compartment, then init
  const addR = await api('POST', '/api/compartment/add', { label, threshold: 1, passphrase_mode: 'wrapped' });
  if (addR.error) { toast(addR.error, 'error'); return; }
  const initR = await api('POST', '/api/compartment/init', { id: addR.id, passphrase: p });
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
  wizShowStep('wizStepFido2Pin');
}

function wizCustomCompartments() {
  customCompartments = [];
  document.getElementById('wizCustomCompList').innerHTML = '';
  wizShowStep('wizStepCustomComps');
}

function wizAddCustomComp() {
  const label = document.getElementById('wizCustomLabel').value;
  const threshold = parseInt(document.getElementById('wizCustomThreshold').value);
  if (!label || !threshold) { toast('Label and threshold required', 'error'); return; }
  if (customCompartments.some(c => c.threshold === threshold)) {
    toast('Threshold ' + threshold + ' already used', 'error'); return;
  }
  customCompartments.push({ label, threshold });
  document.getElementById('wizCustomLabel').value = '';
  document.getElementById('wizCustomThreshold').value = '';
  let html = '';
  customCompartments.forEach(c => {
    html += '<div class="compartment-hint"><span class="hint-label">' +
      esc(c.label) + '</span><span class="hint-threshold">Tap ' +
      c.threshold + ' key' + (c.threshold > 1 ? 's' : '') + '</span></div>';
  });
  document.getElementById('wizCustomCompList').innerHTML = html;
  document.getElementById('wizCustomContinue').disabled = false;
}

async function wizRegisterKey() {
  const pin = document.getElementById('wizFido2Pin').value;
  const label = document.getElementById('wizFido2Label').value;
  const passphrase = document.getElementById('wizBackupPass').value || null;
  if (!pin) { toast('PIN required', 'error'); return; }
  if (!label) { toast('Label required', 'error'); return; }

  wizShowStep('wizStepTouch');

  const body = {
    pin,
    label,
    compartments: wizCompartments.map(c => ({
      label: c.label,
      threshold: c.threshold,
      passphrase_mode: null,
    })),
    passphrase: passphrase && passphrase.length >= 8 ? passphrase : null,
  };

  const r = await api('POST', '/api/fido2/setup', body);
  if (r.error) {
    toast(r.error, 'error');
    wizShowStep('wizStepFido2Pin');
    return;
  }

  document.getElementById('wizDoneMsg').textContent = 'Setup Complete';
  document.getElementById('wizDoneDetail').textContent =
    r.compartments + ' compartment(s) created, key "' + label + '" registered.';
  wizShowStep('wizStepDone');
  setTimeout(refresh, 1500);
}

// ── API Keys & Secrets ────────────────────────────────────────

async function loadApiKeys() {
  try {
    const r = await api('GET', '/api/api-keys');
    if (r.error) return;
    const list = document.getElementById('apiKeyList');
    list.innerHTML = '';
    (r.keys || []).forEach(k => {
      const li = document.createElement('li');
      li.innerHTML = '<span>' + esc(k) + '</span>' +
        '<div class="key-actions">' +
        '<button class="btn-ghost" onclick="revealApiKey(\'' + esc(k) + '\', this)">Reveal</button>' +
        '<button class="btn-danger" onclick="deleteApiKey(\'' + esc(k) + '\')">Delete</button></div>';
      list.appendChild(li);
    });
  } catch(e) {}
}

async function loadSecrets() {
  try {
    const r = await api('GET', '/api/secrets');
    if (r.error) return;
    const list = document.getElementById('secretList');
    list.innerHTML = '';
    (r.keys || []).forEach(k => {
      const li = document.createElement('li');
      li.innerHTML = '<span>' + esc(k) + '</span>' +
        '<div class="key-actions">' +
        '<button class="btn-ghost" onclick="revealSecret(\'' + esc(k) + '\', this)">Reveal</button>' +
        '<button class="btn-danger" onclick="deleteSecret(\'' + esc(k) + '\')">Delete</button></div>';
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

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}

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
