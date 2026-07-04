#!/usr/bin/env node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import { once } from "node:events";

const TARGET_URL = process.env.SIGILLUM_BROWSER_SMOKE_URL || "http://127.0.0.1:19843";
const PASSPHRASE = process.env.SIGILLUM_BROWSER_SMOKE_PASSPHRASE || "browser-smoke-passphrase-123";
const COMPARTMENT_LABEL = process.env.SIGILLUM_BROWSER_SMOKE_COMPARTMENT || "browser-smoke";
const API_KEY_NAME = process.env.SIGILLUM_BROWSER_SMOKE_API_KEY_NAME || "browser_rpc_canary";
const API_KEY_VALUE = process.env.SIGILLUM_BROWSER_SMOKE_API_KEY_VALUE || "browser-rpc-canary-value";
const SECRET_NAME = process.env.SIGILLUM_BROWSER_SMOKE_SECRET_NAME || "browser_secret_canary";
const SECRET_VALUE = process.env.SIGILLUM_BROWSER_SMOKE_SECRET_VALUE || "browser-secret-canary-value";
const ARTIFACT_DIR =
  process.env.SIGILLUM_BROWSER_SMOKE_ARTIFACT_DIR ||
  fs.mkdtempSync(path.join(os.tmpdir(), "sigillum-browser-smoke-artifacts."));
const TIMEOUT_MS = Number(process.env.SIGILLUM_BROWSER_SMOKE_TIMEOUT_MS || 60_000);
const REAUTH_TIMEOUT_MS = Number(process.env.SIGILLUM_BROWSER_SMOKE_REAUTH_TIMEOUT_MS || 120_000);

function fail(message) {
  throw new Error(`browser smoke failed: ${message}`);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function findChromeExecutable() {
  const candidates = [
    process.env.CHROME_BIN,
    process.env.GOOGLE_CHROME_BIN,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
  ].filter(Boolean);

  const found = candidates.find((candidate) => fs.existsSync(candidate));
  if (!found) {
    fail("Chrome or Chromium was not found. Set CHROME_BIN to a compatible browser executable.");
  }
  return found;
}

function launchChrome() {
  const executable = findChromeExecutable();
  const profileDir = fs.mkdtempSync(path.join(os.tmpdir(), "sigillum-browser-smoke-profile."));
  const args = [
    "--headless=new",
    "--remote-debugging-port=0",
    `--user-data-dir=${profileDir}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-extensions",
    "--disable-features=Translate,MediaRouter",
    "--disable-sync",
    "--metrics-recording-only",
    "about:blank",
  ];

  const child = spawn(executable, args, {
    stdio: ["ignore", "ignore", "pipe"],
  });

  let stderr = "";
  let resolved = false;
  const ready = new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error(`timed out waiting for Chrome DevTools endpoint\n${stderr}`));
    }, TIMEOUT_MS);

    child.once("exit", (code, signal) => {
      if (!resolved) {
        clearTimeout(timer);
        reject(new Error(`Chrome exited before DevTools was ready: code=${code} signal=${signal}\n${stderr}`));
      }
    });

    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
      const match = stderr.match(/DevTools listening on (ws:\/\/[^\s]+)/);
      if (!match || resolved) return;
      resolved = true;
      clearTimeout(timer);
      resolve(match[1]);
    });
  });

  return {
    child,
    profileDir,
    ready,
    async cleanup() {
      if (child.exitCode === null && child.signalCode === null) {
        child.kill("SIGTERM");
      }
      if (child.exitCode === null && child.signalCode === null) {
        await Promise.race([
          once(child, "exit").catch(() => {}),
          sleep(2_000),
        ]);
      }
      if (child.exitCode === null && child.signalCode === null) {
        child.kill("SIGKILL");
        await Promise.race([
          once(child, "exit").catch(() => {}),
          sleep(1_000),
        ]);
      }
      // Chromium helper processes can outlive the killed main process and
      // keep writing to the profile dir. A leftover temp dir must not fail
      // the smoke: retry with backoff, then warn and continue.
      const rmOptions = {
        recursive: true,
        force: true,
        maxRetries: 10,
        retryDelay: 200,
      };
      try {
        fs.rmSync(profileDir, rmOptions);
      } catch {
        await sleep(1_000);
        try {
          fs.rmSync(profileDir, rmOptions);
        } catch (retryError) {
          console.warn(
            `browser-smoke: leaving temp profile dir behind at ${profileDir}: ${retryError.message}`,
          );
        }
      }
    },
  };
}

class CdpClient {
  constructor(webSocketUrl) {
    this.webSocket = new WebSocket(webSocketUrl);
    this.nextId = 1;
    this.pending = new Map();
    this.handlers = new Map();
    this.opened = new Promise((resolve, reject) => {
      this.webSocket.addEventListener("open", resolve, { once: true });
      this.webSocket.addEventListener("error", reject, { once: true });
    });
    this.webSocket.addEventListener("message", (event) => this.handleMessage(event.data));
  }

  handleMessage(data) {
    const message = JSON.parse(data);
    if (message.id) {
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      if (message.error) {
        pending.reject(new Error(`${message.error.message || "CDP error"} ${JSON.stringify(message.error)}`));
      } else {
        pending.resolve(message.result || {});
      }
      return;
    }

    const handler = this.handlers.get(message.method);
    if (handler) handler(message.params || {});
  }

  on(method, handler) {
    this.handlers.set(method, handler);
  }

  async send(method, params = {}) {
    await this.opened;
    const id = this.nextId++;
    const payload = JSON.stringify({ id, method, params });
    const result = new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
    });
    this.webSocket.send(payload);
    return result;
  }

  close() {
    this.webSocket.close();
  }
}

async function openPage(browserWebSocketUrl, targetUrl) {
  const debugUrl = new URL(browserWebSocketUrl);
  const endpoint = `http://${debugUrl.hostname}:${debugUrl.port}/json/new?${encodeURIComponent(targetUrl)}`;
  let response = await fetch(endpoint, { method: "PUT" });
  if (!response.ok) {
    response = await fetch(endpoint);
  }
  if (!response.ok) {
    fail(`could not open browser target: HTTP ${response.status}`);
  }
  const target = await response.json();
  if (!target.webSocketDebuggerUrl) {
    fail("browser target did not include a DevTools websocket URL");
  }
  return new CdpClient(target.webSocketDebuggerUrl);
}

function quoted(value) {
  return JSON.stringify(value);
}

async function evaluate(cdp, expression, description = expression) {
  const result = await cdp.send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
    timeout: TIMEOUT_MS,
  });
  if (result.exceptionDetails) {
    const details = result.exceptionDetails;
    const reason =
      details.exception?.description ||
      details.exception?.value ||
      details.text ||
      "runtime exception";
    fail(`${description}: ${reason}`);
  }
  return result.result?.value;
}

async function waitFor(cdp, expression, description, timeoutMs = TIMEOUT_MS) {
  const deadline = Date.now() + timeoutMs;
  let lastError = "";
  while (Date.now() < deadline) {
    try {
      if (await evaluate(cdp, expression, description)) {
        return;
      }
    } catch (error) {
      lastError = error.message;
    }
    await sleep(150);
  }
  fail(`${description} did not become true${lastError ? ` (${lastError})` : ""}`);
}

async function click(cdp, selector, description = selector) {
  await evaluate(
    cdp,
    `(() => {
      const el = document.querySelector(${quoted(selector)});
      if (!el) throw new Error("missing element");
      el.scrollIntoView({ block: "center", inline: "center" });
      el.click();
      return true;
    })()`,
    `click ${description}`,
  );
}

async function clickByText(cdp, selector, text, description = `${selector} containing ${text}`) {
  await evaluate(
    cdp,
    `(() => {
      const expected = ${quoted(text)};
      const el = Array.from(document.querySelectorAll(${quoted(selector)}))
        .find((node) => (node.textContent || "").trim().includes(expected));
      if (!el) throw new Error("missing element with text " + expected);
      el.scrollIntoView({ block: "center", inline: "center" });
      el.click();
      return true;
    })()`,
    `click ${description}`,
  );
}

async function setValue(cdp, selector, value, description = selector) {
  await evaluate(
    cdp,
    `(() => {
      const el = document.querySelector(${quoted(selector)});
      if (!el) throw new Error("missing input");
      el.focus();
      el.value = ${quoted(value)};
      el.dispatchEvent(new Event("input", { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
      return true;
    })()`,
    `set ${description}`,
  );
}

async function selectWorkspace(cdp, section) {
  const selector = `[data-action="selectWorkspaceSection"][data-arg0="${section}"]`;
  await waitFor(cdp, `document.querySelector(${quoted(selector)})`, `workspace ${section} nav item`);
  await click(cdp, selector, `workspace ${section}`);
  await waitFor(
    cdp,
    `document.querySelector('[data-action="selectWorkspaceSection"][data-arg0="${section}"]')?.classList.contains("active") === true`,
    `workspace ${section} active`,
  );
}

function visibleAndEnabled(selector) {
  return `(() => {
    const el = document.querySelector(${quoted(selector)});
    if (!el || el.disabled) return false;
    if (el.closest(".hidden")) return false;
    const style = window.getComputedStyle(el);
    return style.display !== "none" && style.visibility !== "hidden";
  })()`;
}

async function submitPassphraseUnlock(cdp, description) {
  await waitFor(
    cdp,
    `document.body.dataset.mode === "locked" && ${visibleAndEnabled("#passphrase")} && ${visibleAndEnabled("#unlockButton")}`,
    `${description} controls ready`,
  );
  await setValue(cdp, "#passphrase", PASSPHRASE, `${description} passphrase`);
  await click(cdp, '[data-action="unlock"]', description);
}

function reauthStateExpression() {
  return `(() => {
    const input = document.getElementById("passphrase");
    const button = document.getElementById("unlockButton");
    const ready = (el) => {
      if (!el || el.disabled || el.closest(".hidden")) return false;
      const style = window.getComputedStyle(el);
      return style.display !== "none" && style.visibility !== "hidden";
    };
    return {
      mode: document.body.dataset.mode || "",
      unlockError: document.getElementById("unlockError")?.textContent?.trim() || "",
      passphraseValueLength: input?.value?.length || 0,
      unlockButtonDisabled: !!button?.disabled,
      controlsReady: ready(input) && ready(button),
      hasToken: !!sessionStorage.getItem("sigillumSessionToken")
    };
  })()`;
}

async function waitForReauthAttempt(cdp, description) {
  const deadline = Date.now() + REAUTH_TIMEOUT_MS;
  let lastState = null;
  while (Date.now() < deadline) {
    lastState = await evaluate(cdp, reauthStateExpression(), description);
    if (lastState.hasToken) {
      return { ok: true, state: lastState };
    }
    if (lastState.mode === "locked" && lastState.controlsReady) {
      return { ok: false, state: lastState };
    }
    await sleep(150);
  }
  return { ok: false, state: lastState, timedOut: true };
}

async function reauthAfterBrowserLogout(cdp) {
  let lastState = null;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    const label = attempt === 1 ? "unlock after browser logout" : `unlock after browser logout (retry ${attempt - 1})`;
    await submitPassphraseUnlock(cdp, label);
    const result = await waitForReauthAttempt(
      cdp,
      attempt === 1 ? "browser session token after reauth (first attempt)" : "browser session token after reauth",
    );
    lastState = result.state;
    if (result.ok) {
      return;
    }
    if (result.timedOut) {
      break;
    }
  }

  fail(`browser session token after reauth did not become true: ${JSON.stringify(lastState)}`);
}

async function captureFailure(cdp) {
  fs.mkdirSync(ARTIFACT_DIR, { recursive: true });
  const screenshotPath = path.join(ARTIFACT_DIR, "browser-smoke-failure.png");
  const htmlPath = path.join(ARTIFACT_DIR, "browser-smoke-failure.html");

  try {
    const screenshot = await cdp.send("Page.captureScreenshot", {
      format: "png",
      fromSurface: true,
    });
    if (screenshot.data) {
      fs.writeFileSync(screenshotPath, Buffer.from(screenshot.data, "base64"));
      console.error(`wrote browser smoke screenshot: ${screenshotPath}`);
    }
  } catch (error) {
    console.error(`could not capture screenshot: ${error.message}`);
  }

  try {
    const html = await evaluate(cdp, "document.documentElement.outerHTML", "capture html");
    fs.writeFileSync(htmlPath, html);
    console.error(`wrote browser smoke html: ${htmlPath}`);
  } catch (error) {
    console.error(`could not capture html: ${error.message}`);
  }
}

async function runBrowserSmoke(cdp) {
  const runtimeErrors = [];
  cdp.on("Runtime.exceptionThrown", (params) => {
    runtimeErrors.push(params.exceptionDetails?.text || "runtime exception");
  });
  cdp.on("Runtime.consoleAPICalled", (params) => {
    if (params.type === "error") {
      runtimeErrors.push(
        params.args?.map((arg) => arg.value || arg.description || "").join(" ") ||
          "console error",
      );
    }
  });

  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  await cdp.send("Page.setViewport", { width: 1440, height: 1100, deviceScaleFactor: 1, mobile: false }).catch(() => {});
  await cdp.send("Page.navigate", { url: TARGET_URL });

  await waitFor(cdp, "document.readyState === 'complete'", "page load");
  await waitFor(cdp, "document.title.includes('Sigillum Vault')", "Sigillum title");
  await waitFor(
    cdp,
    "document.body.dataset.mode === 'setup' && !document.getElementById('setupCard').classList.contains('hidden')",
    "first-run setup UI",
  );

  await click(cdp, '[data-action="wizPreset"][data-arg0="passphrase"]', "passphrase setup preset");
  await waitFor(cdp, "document.getElementById('wizStepPassphrase').classList.contains('active')", "passphrase wizard step");
  await setValue(cdp, "#wizPLabel", COMPARTMENT_LABEL, "compartment label");
  await setValue(cdp, "#wizPassphrase", PASSPHRASE, "setup passphrase");
  await setValue(cdp, "#wizPassphraseConfirm", PASSPHRASE, "setup passphrase confirmation");
  await click(cdp, '[data-action="wizInitPassphrase"]', "create vault");

  await waitFor(cdp, "sessionStorage.getItem('sigillumSessionToken')", "browser session token after setup");
  await waitFor(cdp, "document.body.dataset.mode === 'unlocked'", "unlocked workspace after setup");
  await waitFor(cdp, "document.getElementById('statusBadge').textContent.trim() === 'UNLOCKED'", "unlocked badge");
  await waitFor(cdp, "document.getElementById('compartmentCount').textContent.trim() === '1'", "compartment count after setup");

  await selectWorkspace(cdp, "secrets");
  await waitFor(
    cdp,
    "!document.getElementById('apiKeysCard').classList.contains('section-hidden') && !document.getElementById('secretsCard').classList.contains('section-hidden')",
    "secrets cards visible",
  );

  await setValue(cdp, "#apiKeyName", API_KEY_NAME, "API key name");
  await setValue(cdp, "#apiKeyValue", API_KEY_VALUE, "API key value");
  await click(cdp, '[data-action="setApiKey"]', "store API key");
  await waitFor(cdp, `document.getElementById('apiKeyList').textContent.includes(${quoted(API_KEY_NAME)})`, "API key listed");
  await waitFor(cdp, "document.getElementById('apiKeyCount').textContent.trim() === '1'", "API key count");

  await setValue(cdp, "#secretName", SECRET_NAME, "secret name");
  await setValue(cdp, "#secretValue", SECRET_VALUE, "secret value");
  await click(cdp, '[data-action="setSecret"]', "store secret");
  await waitFor(cdp, `document.getElementById('secretList').textContent.includes(${quoted(SECRET_NAME)})`, "secret listed");
  await waitFor(cdp, "document.getElementById('secretCount').textContent.trim() === '1'", "secret count");

  await clickByText(cdp, "#apiKeyList button", "Reveal", "API key reveal");
  await waitFor(cdp, `document.getElementById('apiKeyList').textContent.includes(${quoted(API_KEY_VALUE)})`, "API key revealed value");
  await clickByText(cdp, "#secretList button", "Reveal", "secret reveal");
  await waitFor(cdp, `document.getElementById('secretList').textContent.includes(${quoted(SECRET_VALUE)})`, "secret revealed value");

  await selectWorkspace(cdp, "security");
  await click(cdp, '[data-action="logoutSession"]', "logout browser session");
  await waitFor(cdp, "!sessionStorage.getItem('sigillumSessionToken')", "browser session token cleared");
  await waitFor(cdp, "document.body.dataset.mode === 'locked'", "locked UI after browser logout");

  await reauthAfterBrowserLogout(cdp);
  await waitFor(cdp, "document.body.dataset.mode === 'unlocked'", "unlocked workspace after reauth");
  await waitFor(cdp, "document.getElementById('apiKeyCount').textContent.trim() === '1'", "API key count after reauth");
  await waitFor(cdp, "document.getElementById('secretCount').textContent.trim() === '1'", "secret count after reauth");

  await selectWorkspace(cdp, "secrets");
  await waitFor(cdp, `document.getElementById('apiKeyList').textContent.includes(${quoted(API_KEY_NAME)})`, "API key listed after reauth");
  await waitFor(cdp, `document.getElementById('secretList').textContent.includes(${quoted(SECRET_NAME)})`, "secret listed after reauth");

  if (runtimeErrors.length > 0) {
    fail(`browser console/runtime errors: ${runtimeErrors.join("; ")}`);
  }
}

let chrome;
let cdp;
try {
  chrome = launchChrome();
  const browserWebSocketUrl = await chrome.ready;
  cdp = await openPage(browserWebSocketUrl, TARGET_URL);
  await runBrowserSmoke(cdp);
  console.log("browser smoke checks passed");
} catch (error) {
  if (cdp) {
    await captureFailure(cdp);
  }
  console.error(error.message);
  process.exitCode = 1;
} finally {
  if (cdp) cdp.close();
  if (chrome) await chrome.cleanup();
}
