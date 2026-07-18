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
const ARTIFACT_DIR =
  process.env.SIGILLUM_BROWSER_SMOKE_ARTIFACT_DIR ||
  fs.mkdtempSync(path.join(os.tmpdir(), "sigillum-browser-smoke-artifacts."));
const TIMEOUT_MS = Number(process.env.SIGILLUM_BROWSER_SMOKE_TIMEOUT_MS || 60_000);
const REAUTH_TIMEOUT_MS = Number(process.env.SIGILLUM_BROWSER_SMOKE_REAUTH_TIMEOUT_MS || 300_000);
const REVEAL_TIMEOUT_MS = Number(process.env.SIGILLUM_BROWSER_SMOKE_REVEAL_TIMEOUT_MS || 120_000);
const REFRESH_CYCLE_TIMEOUT_MS = Number(
  process.env.SIGILLUM_BROWSER_SMOKE_REFRESH_CYCLE_TIMEOUT_MS || 20_000,
);

const DESTINATION_ROOTS = {
  overview: "#statusCard .dest-overview",
  receive: "#receivingCard .dest-recv",
  portfolio: '#inventoryCard [data-portfolio="root"]',
  move: '#plansCard [data-move-region="plans-banner"]',
  vault: '#secretsCard [data-vault="root"]',
};

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

async function openPage(browserWebSocketUrl) {
  const debugUrl = new URL(browserWebSocketUrl);
  const endpoint = `http://${debugUrl.hostname}:${debugUrl.port}/json/list`;
  const response = await fetch(endpoint);
  if (!response.ok) {
    fail(`could not list browser targets: HTTP ${response.status}`);
  }
  const targets = await response.json();
  const pageTargets = Array.isArray(targets)
    ? targets.filter((target) => target.type === "page")
    : [];
  if (pageTargets.length !== 1) {
    const summary = pageTargets.map((target) => ({
      id: target.id || "",
      url: target.url || "",
    }));
    fail(
      `expected exactly one initial browser page target; found ${pageTargets.length}: ` +
        JSON.stringify(summary),
    );
  }
  const [target] = pageTargets;
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
      if (!el.isConnected) throw new Error("element is disconnected");
      if (el.disabled || el.getAttribute("aria-disabled") === "true") {
        throw new Error("element is disabled");
      }
      if (el.closest("[hidden], .hidden, .section-hidden, .move-concealed")) {
        throw new Error("element is inside a hidden region");
      }
      const style = window.getComputedStyle(el);
      const rect = el.getBoundingClientRect();
      if (
        style.display === "none" ||
        style.visibility === "hidden" ||
        Number(style.opacity) === 0 ||
        (rect.width <= 0 && rect.height <= 0)
      ) {
        throw new Error("element is not visible");
      }
      el.scrollIntoView({ block: "center", inline: "center" });
      el.click();
      return true;
    })()`,
    `click ${description}`,
  );
}

async function submitForm(cdp, selector, description = selector) {
  await evaluate(
    cdp,
    `(() => {
      const form = document.querySelector(${quoted(selector)});
      if (!form) throw new Error("missing form");
      if (!form.isConnected) throw new Error("form is disconnected");
      if (form.closest("[hidden], .hidden, .section-hidden, .move-concealed")) {
        throw new Error("form is inside a hidden region");
      }
      const style = window.getComputedStyle(form);
      const rect = form.getBoundingClientRect();
      if (
        style.display === "none" ||
        style.visibility === "hidden" ||
        (rect.width <= 0 && rect.height <= 0)
      ) {
        throw new Error("form is not visible");
      }
      const submitter = form.querySelector('button[type="submit"], input[type="submit"]');
      if (submitter?.disabled || submitter?.getAttribute("aria-disabled") === "true") {
        throw new Error("form submitter is disabled");
      }
      if (typeof form.requestSubmit === "function") {
        if (submitter) form.requestSubmit(submitter);
        else form.requestSubmit();
      }
      else form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
      return true;
    })()`,
    `submit ${description}`,
  );
}

async function setValue(cdp, selector, value, description = selector) {
  await evaluate(
    cdp,
    `(() => {
      const el = document.querySelector(${quoted(selector)});
      if (!el) throw new Error("missing input");
      if (!el.isConnected) throw new Error("input is disconnected");
      if (el.disabled || el.getAttribute("aria-disabled") === "true") {
        throw new Error("input is disabled");
      }
      if (el.closest("[hidden], .hidden, .section-hidden, .move-concealed")) {
        throw new Error("input is inside a hidden region");
      }
      const style = window.getComputedStyle(el);
      const rect = el.getBoundingClientRect();
      if (
        style.display === "none" ||
        style.visibility === "hidden" ||
        (rect.width <= 0 && rect.height <= 0)
      ) {
        throw new Error("input is not visible");
      }
      el.focus();
      el.value = ${quoted(value)};
      el.dispatchEvent(new Event("input", { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
      return true;
    })()`,
    `set ${description}`,
  );
}

const KEY_DEFINITIONS = {
  Enter: { key: "Enter", code: "Enter", keyCode: 13 },
  Escape: { key: "Escape", code: "Escape", keyCode: 27 },
  Tab: { key: "Tab", code: "Tab", keyCode: 9 },
  ArrowDown: { key: "ArrowDown", code: "ArrowDown", keyCode: 40 },
  KeyK: { key: "k", code: "KeyK", keyCode: 75 },
};

async function pressKey(cdp, name, options = {}) {
  const definition = KEY_DEFINITIONS[name];
  if (!definition) fail(`unknown key definition: ${name}`);
  const modifiers = (options.modifiers || 0) | (options.shiftKey ? 8 : 0);
  const params = {
    key: definition.key,
    code: definition.code,
    windowsVirtualKeyCode: definition.keyCode,
    nativeVirtualKeyCode: definition.keyCode,
    modifiers,
  };
  await cdp.send("Input.dispatchKeyEvent", { type: "keyDown", ...params });
  await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", ...params });
}

async function pressPaletteShortcut(cdp) {
  // CDP modifier bitfield: Control=2, Meta=4. Match the platform shortcut the
  // operator sees rather than synthesizing a DOM event that skips browser
  // keyboard behavior.
  await pressKey(cdp, "KeyK", { modifiers: process.platform === "darwin" ? 4 : 2 });
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

function visibleElementExpression(selector) {
  return `(() => {
    const el = document.querySelector(${quoted(selector)});
    if (!el || !el.isConnected) return false;
    if (el.closest("[hidden], .hidden, .section-hidden, .move-concealed")) return false;
    const style = window.getComputedStyle(el);
    const rect = el.getBoundingClientRect();
    return style.display !== "none" &&
      style.visibility !== "hidden" &&
      Number(style.opacity) !== 0 &&
      (rect.width > 0 || rect.height > 0);
  })()`;
}

async function waitForDestination(cdp, destination) {
  const rootSelector = DESTINATION_ROOTS[destination];
  if (!rootSelector) fail(`missing root selector for destination ${destination}`);
  await waitFor(
    cdp,
    `window.location.hash === ${quoted(`#/${destination}`)} && ${visibleElementExpression(rootSelector)}`,
    `${destination} route and migrated root`,
  );
}

async function waitForRefreshCycle(cdp, description) {
  await evaluate(
    cdp,
    `new Promise((resolve, reject) => {
      const indicator = document.getElementById("refreshMeta");
      if (!indicator) {
        reject(new Error("missing refresh indicator"));
        return;
      }
      let sawBusy = indicator.dataset.state === "busy";
      let settled = false;
      let observer = null;
      let timer = null;
      const finish = (error) => {
        if (settled) return;
        settled = true;
        observer?.disconnect();
        if (timer !== null) clearTimeout(timer);
        if (error) reject(error);
        else resolve(true);
      };
      const inspect = () => {
        if (indicator.dataset.state === "busy") sawBusy = true;
        if (sawBusy && indicator.dataset.state === "live") finish();
      };
      observer = new MutationObserver(inspect);
      observer.observe(indicator, {
        attributes: true,
        attributeFilter: ["data-state"],
      });
      timer = setTimeout(() => {
        finish(new Error(
          "timed out waiting for refresh busy -> live; state=" +
          (indicator.dataset.state || "missing") +
          "; visibility=" +
          document.visibilityState +
          "; hidden=" +
          document.hidden +
          "; hasFocus=" +
          document.hasFocus() +
          "; visibilityHistory=" +
          JSON.stringify(globalThis.__sigillumSmokeVisibilityHistory || [])
        ));
      }, ${REFRESH_CYCLE_TIMEOUT_MS});
      inspect();
    })`,
    description,
  );
}

function visibleAndEnabled(selector) {
  return `(() => {
    const el = document.querySelector(${quoted(selector)});
    if (!el || !el.isConnected || el.disabled || el.getAttribute("aria-disabled") === "true") {
      return false;
    }
    if (el.closest("[hidden], .hidden, .section-hidden, .move-concealed")) return false;
    const style = window.getComputedStyle(el);
    const rect = el.getBoundingClientRect();
    return style.display !== "none" &&
      style.visibility !== "hidden" &&
      Number(style.opacity) !== 0 &&
      (rect.width > 0 || rect.height > 0);
  })()`;
}

async function submitPassphraseUnlock(cdp, description) {
  await waitFor(
    cdp,
    `document.body.dataset.mode === "locked" && ${visibleAndEnabled("#passphrase")} && ${visibleAndEnabled("#unlockButton")}`,
    `${description} controls ready`,
  );
  await setValue(cdp, "#passphrase", PASSPHRASE, `${description} passphrase`);
  await pressKey(cdp, "Enter");
}

function vaultRowExpression(sectionHook, name, body) {
  return `(() => {
    const section = document.querySelector(${quoted(`[data-vault="${sectionHook}"]`)});
    if (!section) return false;
    const row = Array.from(section.querySelectorAll('[data-vault="secret-row"]'))
      .find((candidate) =>
        (candidate.querySelector(".vault-item-title")?.textContent || "").trim() === ${quoted(name)}
      );
    if (!row) return false;
    return (${body});
  })()`;
}

async function waitForVaultRow(cdp, sectionHook, name, description) {
  await waitFor(
    cdp,
    vaultRowExpression(sectionHook, name, "row.isConnected"),
    description,
  );
}

async function clickVaultRowAction(cdp, sectionHook, name, ariaLabel, description, rememberAs = null) {
  await evaluate(
    cdp,
    `(() => {
      const section = document.querySelector(${quoted(`[data-vault="${sectionHook}"]`)});
      if (!section) throw new Error("missing vault section");
      const row = Array.from(section.querySelectorAll('[data-vault="secret-row"]'))
        .find((candidate) =>
          (candidate.querySelector(".vault-item-title")?.textContent || "").trim() === ${quoted(name)}
        );
      if (!row) throw new Error("missing exact vault row");
      const button = Array.from(row.querySelectorAll("button"))
        .find((candidate) => candidate.getAttribute("aria-label") === ${quoted(ariaLabel)});
      if (!button) throw new Error("missing row action");
      if (!button.isConnected) throw new Error("row action is disconnected");
      if (button.disabled || button.getAttribute("aria-disabled") === "true") {
        throw new Error("row action is disabled");
      }
      if (button.closest("[hidden], .hidden, .section-hidden, .move-concealed")) {
        throw new Error("row action is inside a hidden region");
      }
      const style = window.getComputedStyle(button);
      const rect = button.getBoundingClientRect();
      if (
        style.display === "none" ||
        style.visibility === "hidden" ||
        Number(style.opacity) === 0 ||
        (rect.width <= 0 && rect.height <= 0)
      ) {
        throw new Error("row action is not visible");
      }
      button.scrollIntoView({ block: "center", inline: "center" });
      if (${quoted(rememberAs)} !== null) window[${quoted(rememberAs)}] = button;
      button.focus();
      button.click();
      return true;
    })()`,
    description,
  );
}

async function revealVaultValue(cdp, sectionHook, name, noun, value) {
  await clickVaultRowAction(
    cdp,
    sectionHook,
    name,
    `Reveal ${noun} ${name}`,
    `reveal ${noun} ${name}`,
  );
  await waitFor(
    cdp,
    vaultRowExpression(
      sectionHook,
      name,
      `(row.querySelector('[data-vault="revealed"]')?.textContent || "") === ${quoted(value)}`,
    ),
    `${noun} revealed value`,
    REVEAL_TIMEOUT_MS,
  );
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
      unlockButtonText: button?.textContent || "",
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
  const networkRequests = [];
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
  cdp.on("Network.requestWillBeSent", (params) => {
    const request = params.request || {};
    let pathname = "";
    try {
      pathname = new URL(request.url).pathname;
    } catch {}
    networkRequests.push({
      method: request.method || "",
      pathname,
      url: request.url || "",
    });
  });

  await cdp.send("Page.enable");
  // Reuse Chrome's sole initial about:blank target instead of creating a
  // competing second tab through `/json/new`. With two page targets, hosted
  // macOS later reports the app target as hidden, which correctly pauses the
  // shipped visibility-aware refresh controller. Keep one page target and
  // activate it before navigation so refresh-liveness checks exercise a
  // genuinely visible operator console on every CI platform.
  await cdp.send("Page.bringToFront");
  await cdp.send("Runtime.enable");
  await cdp.send("Network.enable");
  await cdp.send("Page.addScriptToEvaluateOnNewDocument", {
    source: `(() => {
      const history = [];
      globalThis.__sigillumSmokeVisibilityHistory = history;
      const record = () => history.push({
        state: document.visibilityState,
        hidden: document.hidden,
        hasFocus: document.hasFocus(),
        at: Date.now(),
      });
      record();
      document.addEventListener("visibilitychange", record);
    })();`,
  });
  await cdp.send("Page.setViewport", { width: 1440, height: 1100, deviceScaleFactor: 1, mobile: false }).catch(() => {});
  await cdp.send("Page.navigate", { url: TARGET_URL });

  await waitFor(cdp, "document.readyState === 'complete'", "page load");
  const targetVisibility = await evaluate(
    cdp,
    "document.visibilityState",
    "browser smoke target visibility",
  );
  if (targetVisibility !== "visible") {
    fail(`browser smoke target is not foreground; visibility=${targetVisibility}`);
  }
  await waitFor(cdp, "document.title.includes('Sigillum Vault')", "Sigillum title");
  await waitFor(
    cdp,
    `document.body.dataset.mode === "setup" && ${visibleElementExpression("#setupCard")}`,
    "first-run setup UI",
  );

  await click(cdp, '[data-action="wizGetStarted"]', "start setup wizard");
  await waitFor(
    cdp,
    `document.getElementById("wizStep0")?.classList.contains("active") === true &&
      document.getElementById("wizStep0")?.contains(document.activeElement) === true`,
    "preset step active and focused",
  );
  await click(cdp, '[data-action="wizPreset"][data-arg0="passphrase"]', "passphrase setup preset");
  await waitFor(
    cdp,
    `document.getElementById("wizStepPassphrase")?.classList.contains("active") === true &&
      document.activeElement?.id === "wizPLabel"`,
    "passphrase wizard step focused",
  );
  await setValue(cdp, "#wizPLabel", COMPARTMENT_LABEL, "compartment label");
  await setValue(cdp, "#wizPassphrase", PASSPHRASE, "setup passphrase");
  await setValue(cdp, "#wizPassphraseConfirm", PASSPHRASE, "setup passphrase confirmation");
  await pressKey(cdp, "Enter");

  await waitFor(cdp, "sessionStorage.getItem('sigillumSessionToken')", "browser session token after setup");
  await waitFor(cdp, "document.body.dataset.mode === 'unlocked'", "unlocked workspace after setup");
  await waitFor(cdp, "document.getElementById('statusBadge').textContent.trim() === 'UNLOCKED'", "unlocked badge");
  await waitForDestination(cdp, "overview");

  // Exercise every migrated destination through the real sidebar/router seam.
  for (const destination of ["overview", "receive", "portfolio", "move", "vault"]) {
    await selectWorkspace(cdp, destination);
    await waitForDestination(cdp, destination);
  }

  await setValue(cdp, '[data-vault="apikey-name"]', API_KEY_NAME, "connection key name");
  await setValue(cdp, '[data-vault="apikey-value"]', API_KEY_VALUE, "connection key value");
  await submitForm(cdp, '[data-vault="apikey-form"]', "connection key form");
  await waitForVaultRow(cdp, "apikeys", API_KEY_NAME, "connection key listed");
  await revealVaultValue(cdp, "apikeys", API_KEY_NAME, "connection key", API_KEY_VALUE);

  // A full legacy refresh must patch the sidebar in place: the active nav
  // button remains the same node and retains keyboard focus.
  await waitFor(
    cdp,
    `document.getElementById("refreshMeta")?.dataset.state === "live"`,
    "live refresh state before focus retention check",
  );
  const navFocused = await evaluate(
    cdp,
    `(() => {
      const button = document.querySelector(
        '[data-action="selectWorkspaceSection"][data-arg0="vault"]'
      );
      if (!button) throw new Error("missing active Vault nav button");
      window.__sigillumSmokeNav = button;
      button.focus();
      return document.activeElement === button;
    })()`,
    "focus active Vault nav button",
  );
  if (!navFocused) fail("active Vault nav button did not receive focus");
  await waitForRefreshCycle(cdp, "focused nav refresh cycle");
  await waitFor(
    cdp,
    `window.__sigillumSmokeNav?.isConnected === true &&
      document.activeElement === window.__sigillumSmokeNav &&
      document.querySelector('[data-action="selectWorkspaceSection"][data-arg0="vault"]') ===
        window.__sigillumSmokeNav`,
    "active nav node identity and focus after refresh",
  );

  // Cancel a real migrated Vault delete modal. This checks safe initial focus,
  // both directions of the focus trap, modal/palette coordination, Escape
  // focus restoration, and the absence of a delete request.
  const deleteRequestStart = networkRequests.length;
  await clickVaultRowAction(
    cdp,
    "apikeys",
    API_KEY_NAME,
    `Delete connection key ${API_KEY_NAME}`,
    "open connection key delete confirmation",
    "__sigillumSmokeDeleteInvoker",
  );
  await waitFor(
    cdp,
    `${visibleElementExpression('[data-confirm-overlay="confirm"] [role="dialog"]')} &&
      document.activeElement?.matches('[data-confirm-cancel]') === true`,
    "delete confirmation with safe initial focus",
  );
  await pressKey(cdp, "Tab", { shiftKey: true });
  await waitFor(
    cdp,
    `document.activeElement?.matches('[data-confirm-action]') === true`,
    "Shift-Tab wraps to modal action",
  );
  await pressKey(cdp, "Tab");
  await waitFor(
    cdp,
    `document.activeElement?.matches('[data-confirm-cancel]') === true`,
    "Tab wraps to modal cancel",
  );
  await pressPaletteShortcut(cdp);
  await sleep(100);
  await waitFor(
    cdp,
    `document.querySelector('[data-confirm-overlay="confirm"]') !== null &&
      document.querySelector('[data-palette="dialog"]') === null`,
    "palette refused while confirmation modal is active",
  );
  await pressKey(cdp, "Escape");
  await waitFor(
    cdp,
    `document.querySelector('[data-confirm-overlay="confirm"]') === null &&
      window.__sigillumSmokeDeleteInvoker?.isConnected === true &&
      document.activeElement === window.__sigillumSmokeDeleteInvoker`,
    "delete confirmation Escape restores invoker focus",
  );
  await waitForVaultRow(cdp, "apikeys", API_KEY_NAME, "connection key remains after cancellation");
  await sleep(250);
  const deleteRequests = networkRequests
    .slice(deleteRequestStart)
    .filter(
      (request) =>
        request.method === "POST" && request.pathname === "/api/api-keys/delete",
    );
  if (deleteRequests.length > 0) {
    fail(
      `cancelled connection-key delete emitted: ${deleteRequests
        .map((request) => `${request.method} ${request.url}`)
        .join(", ")}`,
    );
  }

  // The palette is keyboard-only acceptance: open, filter, move selection,
  // activate, and prove it navigated through the real router.
  await pressPaletteShortcut(cdp);
  await waitFor(
    cdp,
    `${visibleElementExpression('[data-palette="dialog"]')} &&
      document.activeElement?.matches('[data-palette="input"]') === true`,
    "command palette dialog and focused input",
  );
  await setValue(cdp, '[data-palette="input"]', "Go to", "command palette query");
  await waitFor(
    cdp,
    `document.querySelectorAll('[data-palette="option"]').length === 5 &&
      document.querySelector('[data-command-id="navigate-overview"]')?.getAttribute("aria-selected") === "true"`,
    "navigation commands filtered with Overview initially selected",
  );
  await pressKey(cdp, "ArrowDown");
  await waitFor(
    cdp,
    `Array.from(document.querySelectorAll('[data-palette="option"]'))
      .some((option) =>
        option.getAttribute("data-command-id") === "navigate-receive" &&
        option.getAttribute("aria-selected") === "true"
      )`,
    "Receive navigation command selected",
  );
  await pressKey(cdp, "Enter");
  await waitForDestination(cdp, "receive");
  await waitFor(
    cdp,
    `document.querySelector('[data-palette="dialog"]') === null`,
    "command palette closed after navigation",
  );

  await selectWorkspace(cdp, "vault");
  await waitForDestination(cdp, "vault");
  await evaluate(
    cdp,
    `(() => {
      clearInterval(window.__sigillumSmokeLogoutWatch);
      window.__sigillumSmokeSawRevokedUnlockedShell = false;
      window.__sigillumSmokeLogoutWatch = setInterval(() => {
        if (
          !sessionStorage.getItem("sigillumSessionToken") &&
          document.body.dataset.mode !== "locked"
        ) {
          window.__sigillumSmokeSawRevokedUnlockedShell = true;
        }
      }, 0);
      return true;
    })()`,
    "watch logout authorization boundary",
  );
  await click(cdp, '[data-vault="logout"]', "logout browser session from Vault");
  await waitFor(cdp, "!sessionStorage.getItem('sigillumSessionToken')", "browser session token cleared");
  const lockedAtRevocation = await evaluate(
    cdp,
    `(() => {
      const locked =
        document.body.dataset.mode === "locked" &&
        window.__sigillumSmokeSawRevokedUnlockedShell !== true;
      clearInterval(window.__sigillumSmokeLogoutWatch);
      return locked;
    })()`,
    "locked shell applied in the token-revocation task",
  );
  if (!lockedAtRevocation) {
    fail("browser token cleared while unlocked shell or palette policy still lingered");
  }
  await waitFor(cdp, "document.body.dataset.mode === 'locked'", "locked UI after browser logout");

  await waitFor(
    cdp,
    `document.activeElement?.id === "passphrase"`,
    "passphrase receives focus on locked transition",
  );
  await waitFor(
    cdp,
    `document.getElementById("refreshMeta")?.dataset.state === "live"`,
    "live refresh state before locked focus retention check",
  );
  await setValue(cdp, "#authRestorePass", "focus-canary", "locked recovery focus canary");
  const lockedFocusRemembered = await evaluate(
    cdp,
    `(() => {
      window.__sigillumSmokeLockedFocus = document.getElementById("authRestorePass");
      return document.activeElement === window.__sigillumSmokeLockedFocus;
    })()`,
    "remember locked recovery focus",
  );
  if (!lockedFocusRemembered) fail("locked recovery input did not retain focus before refresh");
  await waitForRefreshCycle(cdp, "locked focus refresh cycle");
  await waitFor(
    cdp,
    `window.__sigillumSmokeLockedFocus?.isConnected === true &&
      document.activeElement === window.__sigillumSmokeLockedFocus &&
      window.__sigillumSmokeLockedFocus.value === "focus-canary"`,
    "locked refresh preserves focus and input value",
  );
  await setValue(cdp, "#authRestorePass", "", "clear locked recovery focus canary");

  await reauthAfterBrowserLogout(cdp);
  await waitFor(cdp, "document.body.dataset.mode === 'unlocked'", "unlocked workspace after reauth");
  await waitFor(cdp, "document.getElementById('statusBadge').textContent.trim() === 'UNLOCKED'", "unlocked badge after reauth");

  // Force a clean controller remount before checking encrypted persistence.
  await selectWorkspace(cdp, "overview");
  await waitForDestination(cdp, "overview");
  await selectWorkspace(cdp, "vault");
  await waitForDestination(cdp, "vault");
  await waitForVaultRow(cdp, "apikeys", API_KEY_NAME, "connection key listed after reauth");
  await revealVaultValue(cdp, "apikeys", API_KEY_NAME, "connection key", API_KEY_VALUE);

  const lateDeleteRequests = networkRequests
    .slice(deleteRequestStart)
    .filter(
      (request) =>
        request.method === "POST" && request.pathname === "/api/api-keys/delete",
    );
  if (lateDeleteRequests.length > 0) {
    fail(
      `cancelled connection-key delete eventually emitted: ${lateDeleteRequests
        .map((request) => `${request.method} ${request.url}`)
        .join(", ")}`,
    );
  }

  if (runtimeErrors.length > 0) {
    fail(`browser console/runtime errors: ${runtimeErrors.join("; ")}`);
  }
}

let chrome;
let cdp;
try {
  chrome = launchChrome();
  const browserWebSocketUrl = await chrome.ready;
  cdp = await openPage(browserWebSocketUrl);
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
