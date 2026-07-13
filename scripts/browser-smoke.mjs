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
const SECOND_COMPARTMENT_LABEL = "browser-smoke-secure";
const SECOND_SECRET_NAME = "browser_second_secret_canary";
const SECOND_SECRET_VALUE = "browser-second-secret-canary-value";
const HELD_MUTATION_SECRET_NAME = "browser-held-mutation-canary";
const HELD_MUTATION_SECRET_VALUE = "browser-held-mutation-value";
const UNSAVED_API_KEY_VALUE = "browser-unsaved-api-key-value";
const UNSAVED_SECRET_VALUE = "browser-unsaved-secret-value";
const UNSAVED_MNEMONIC = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";
const ARTIFACT_DIR =
  process.env.SIGILLUM_BROWSER_SMOKE_ARTIFACT_DIR ||
  fs.mkdtempSync(path.join(os.tmpdir(), "sigillum-browser-smoke-artifacts."));
const TIMEOUT_MS = Number(process.env.SIGILLUM_BROWSER_SMOKE_TIMEOUT_MS || 60_000);
const REAUTH_TIMEOUT_MS = Number(process.env.SIGILLUM_BROWSER_SMOKE_REAUTH_TIMEOUT_MS || 300_000);
const REVEAL_TIMEOUT_MS = Number(process.env.SIGILLUM_BROWSER_SMOKE_REVEAL_TIMEOUT_MS || 120_000);
// The reveal button toggles state with no in-flight or disabled signal, so a
// ready-to-retry read right after clicking can still be the original request.
// Retrying too early can race that request and toggle the revealed value off.
const REVEAL_RETRY_SETTLE_MS = 1_200;
const DESKTOP_VIEWPORT = { width: 1440, height: 1100 };
const NARROW_VIEWPORT = { width: 390, height: 844 };
const WORKSPACE_DESTINATIONS = [
  {
    id: "overview",
    label: "Overview",
    anchors: [
      "#statusCard",
      "#journeyCard",
      "#nextStepCard",
      "#auditCard",
      "#selfCheckCard",
      "#guideCard",
    ],
  },
  {
    id: "receive",
    label: "Receive",
    anchors: ["#receivingCard", "#treasuryReceivingCard", "#depositsCard"],
  },
  {
    id: "portfolio",
    label: "Portfolio",
    anchors: [
      "#treasuryOverviewCard",
      "#walletManagerCard",
      "#profilesCard",
      "#xpubCard",
      "#inventoryCard",
    ],
  },
  {
    id: "move",
    label: "Move",
    anchors: ["#treasuryCard", "#consolidationCard", "#queueCard", "#maintenanceCard"],
  },
  {
    id: "vault",
    label: "Vault",
    anchors: [
      "#secretsCard",
      "#apiKeysCard",
      "#compartmentCard",
      "#fido2Card",
      "#backupCard",
      "#diagCard",
    ],
  },
];
const CONDITIONAL_WORKSPACE_CARDS = [
  {
    selector: "#pushCard",
    destination: "vault",
  },
];

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
  await waitFor(cdp, visibleAndEnabled(selector), `${description} visible and enabled`);
  await evaluate(
    cdp,
    `(() => {
      const isVisible = ${browserVisibilityFunction()};
      const el = document.querySelector(${quoted(selector)});
      if (!el) throw new Error("missing element");
      if (!isVisible(el, true)) throw new Error("element is not visible and enabled");
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
      const isVisible = ${browserVisibilityFunction()};
      const expected = ${quoted(text)};
      const el = Array.from(document.querySelectorAll(${quoted(selector)}))
        .find((node) => (node.textContent || "").trim().includes(expected) && isVisible(node, true));
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
  const destination = WORKSPACE_DESTINATIONS.find((candidate) => candidate.id === section);
  if (!destination) fail(`unknown workspace destination: ${section}`);
  const selector = `[data-action="selectWorkspaceSection"][data-arg0="${section}"]`;
  await waitFor(cdp, visibleAndEnabled(selector), `workspace ${section} nav item`);
  await click(cdp, selector, `workspace ${section}`);
  await waitFor(
    cdp,
    `(() => {
      const item = document.querySelector(${quoted(selector)});
      const activeItems = Array.from(
        document.querySelectorAll('[data-action="selectWorkspaceSection"].active'),
      );
      return item?.classList.contains("active") === true &&
        activeItems.length === 1 && activeItems[0] === item &&
        item.getAttribute("aria-current") === "page" &&
        (item.textContent || "").trim() === ${quoted(destination.label)};
    })()`,
    `workspace ${section} active`,
  );
}

function browserVisibilityFunction() {
  return `(el, requireEnabled = false) => {
    if (!el || (requireEnabled && el.disabled)) return false;
    if (el.closest('.hidden, .section-hidden, [hidden], [aria-hidden="true"]')) return false;
    for (let node = el; node; node = node.parentElement) {
      const style = window.getComputedStyle(node);
      if (style.display === "none" || style.visibility === "hidden" ||
          style.visibility === "collapse" || Number(style.opacity) === 0) {
        return false;
      }
    }
    const rect = el.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }`;
}

function actuallyVisible(selector, requireEnabled = false) {
  return `(() => {
    const isVisible = ${browserVisibilityFunction()};
    const el = document.querySelector(${quoted(selector)});
    return isVisible(el, ${String(requireEnabled)});
  })()`;
}

function visibleAndEnabled(selector) {
  return actuallyVisible(selector, true);
}

function workspaceNavigationExpression() {
  const expected = WORKSPACE_DESTINATIONS.map(({ id, label }) => ({ id, label }));
  return `(() => {
    const isVisible = ${browserVisibilityFunction()};
    const nav = document.getElementById("sectionNav");
    const expected = ${quoted(expected)};
    const items = Array.from(
      nav?.querySelectorAll('[data-action="selectWorkspaceSection"]') || [],
    );
    const actual = items.map((item) => ({
      id: item.dataset.arg0 || "",
      label: (item.textContent || "").trim(),
    }));
    return isVisible(nav) && items.every((item) => isVisible(item, true)) &&
      actual.length === expected.length &&
      actual.every((item, index) =>
        item.id === expected[index].id && item.label === expected[index].label
      );
  })()`;
}

function workspaceCardMappingExpression() {
  const expected = [
    ...WORKSPACE_DESTINATIONS.flatMap((destination) =>
      destination.anchors.map((selector) => ({
        id: selector.slice(1),
        destination: destination.id,
      })),
    ),
    ...CONDITIONAL_WORKSPACE_CARDS.map(({ selector, destination }) => ({
      id: selector.slice(1),
      destination,
    })),
  ];
  return `(() => {
    const expected = ${quoted(expected)};
    const actual = Array.from(
      document.querySelectorAll("main .card[data-workspace-section]"),
    ).map((card) => ({
      id: card.id || "",
      destination: card.dataset.workspaceSection || "",
    }));
    const expectedKeys = new Set(expected.map((card) => card.id + ":" + card.destination));
    const actualKeys = new Set(actual.map((card) => card.id + ":" + card.destination));
    return actual.length === expected.length &&
      expectedKeys.size === expected.length && actualKeys.size === actual.length &&
      expected.every((card) => actualKeys.has(card.id + ":" + card.destination));
  })()`;
}

function exclusivelyVisibleDestinationExpression(destination) {
  return `(() => {
    const isVisible = ${browserVisibilityFunction()};
    const visibleCards = Array.from(
      document.querySelectorAll("main .card[data-workspace-section]"),
    ).filter((card) => isVisible(card));
    return visibleCards.length > 0 &&
      visibleCards.every((card) => card.dataset.workspaceSection === ${quoted(destination)});
  })()`;
}

function persistentStatusStripExpression(expectedLockState, expectedCompartment = null) {
  return `(() => {
    const isVisible = ${browserVisibilityFunction()};
    const strip = document.getElementById("statusStrip");
    const lockState = document.getElementById("stripLockState");
    const compartment = document.getElementById("stripCompartment");
    const selfCheck = document.getElementById("stripSelfCheck");
    const lockNow = document.getElementById("stripLockNow");
    const allInside = [lockState, compartment, selfCheck, lockNow]
      .every((item) => item && strip?.contains(item));
    const expectedLockLabel = ${quoted(expectedLockState)};
    const expectedLockDataState = expectedLockLabel.toLowerCase();
    const selfCheckText = (selfCheck?.textContent || "").trim();
    const selfCheckState = selfCheck?.dataset.state || "";
    const compartmentMatches = ${expectedCompartment === null
      ? `(compartment?.textContent || "").trim().length > 0`
      : `(compartment?.textContent || "").trim() === ${quoted(expectedCompartment)}`};
    return allInside && isVisible(strip) &&
      isVisible(lockState) && isVisible(compartment) && isVisible(selfCheck) &&
      isVisible(selfCheck, true) && isVisible(lockNow, true) &&
      (lockState.textContent || "").trim().toLowerCase() === expectedLockDataState &&
      lockState.dataset.state === expectedLockDataState &&
      selfCheckText.startsWith("Self-check") && selfCheckText.length > "Self-check".length &&
      ["pending", "pass", "warn", "fail"].includes(selfCheckState) &&
      selfCheck.dataset.action === "journeyJump" && selfCheck.dataset.arg0 === "selfCheckCard" &&
      (lockNow.textContent || "").trim() === "Lock now" &&
      lockNow.dataset.action === "lock" && compartmentMatches;
  })()`;
}

function resetStatusStripExpression() {
  return `(() => {
    const isVisible = ${browserVisibilityFunction()};
    const strip = document.getElementById("statusStrip");
    const lockState = document.getElementById("stripLockState");
    const compartment = document.getElementById("stripCompartment");
    const selfCheck = document.getElementById("stripSelfCheck");
    const lockNow = document.getElementById("stripLockNow");
    const allInside = [lockState, compartment, selfCheck, lockNow]
      .every((item) => item && strip?.contains(item));
    return allInside && strip.classList.contains("hidden") && !isVisible(strip) &&
      !isVisible(lockState) && !isVisible(compartment) && !isVisible(selfCheck) &&
      !isVisible(lockNow) &&
      (lockState.textContent || "").trim() === "Locked" &&
      lockState.dataset.state === "locked" &&
      (compartment.textContent || "").trim() === "No active compartment" &&
      (selfCheck.textContent || "").trim() === "Self-check unavailable" &&
      selfCheck.dataset.state === "idle";
  })()`;
}

async function setViewport(cdp, { width, height }, description) {
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    width,
    height,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await waitFor(
    cdp,
    `window.innerWidth === ${width} && window.innerHeight === ${height}`,
    `${description} viewport`,
  );
}

function noHorizontalOverflowExpression() {
  return `document.documentElement.scrollWidth <= window.innerWidth &&
    document.body.scrollWidth <= window.innerWidth`;
}

async function assertNarrowWorkspaceDestinations(cdp) {
  await setViewport(cdp, NARROW_VIEWPORT, "narrow mobile-width");
  try {
    for (const destination of WORKSPACE_DESTINATIONS) {
      await selectWorkspace(cdp, destination.id);
      await waitFor(
        cdp,
        exclusivelyVisibleDestinationExpression(destination.id),
        `${destination.label} exclusive at 390px`,
      );
      await waitFor(
        cdp,
        noHorizontalOverflowExpression(),
        `${destination.label} has no horizontal overflow at 390px`,
      );
    }
  } finally {
    await setViewport(cdp, DESKTOP_VIEWPORT, "restored desktop");
  }
}

async function assertStatusStripSelfCheckNavigation(cdp) {
  await click(cdp, "#stripSelfCheck", "status-strip self-check");
  await waitFor(
    cdp,
    `(() => {
      const destination = document.querySelector(
        '[data-action="selectWorkspaceSection"][data-arg0="overview"]',
      );
      const card = document.getElementById("selfCheckCard");
      return destination?.classList.contains("active") === true &&
        destination.getAttribute("aria-current") === "page" &&
        ${actuallyVisible("#selfCheckCard")} && document.activeElement === card;
    })()`,
    "status-strip self-check opens and focuses the Overview self-check card",
  );
}

async function installPostStatusRequestHold(cdp) {
  await evaluate(
    cdp,
    `(() => {
      if (window.__sigillumBrowserSmokeOriginalFetch) {
        throw new Error("post-status request hold is already installed");
      }
      const originalFetch = window.fetch.bind(window);
      const held = [];
      const removeHeld = (entry) => {
        const index = held.indexOf(entry);
        if (index >= 0) held.splice(index, 1);
      };
      window.__sigillumBrowserSmokeOriginalFetch = originalFetch;
      window.__sigillumBrowserSmokeHeldRequestCount = 0;
      window.__sigillumBrowserSmokeHeldRequestPaths = [];
      window.__sigillumBrowserSmokeAbortedHeldRequestPaths = [];
      window.__sigillumBrowserSmokeReleaseRequests = () => {
        window.fetch = originalFetch;
        window.__sigillumBrowserSmokeOriginalFetch = null;
        const queued = held.splice(0);
        queued.forEach((entry) => {
          entry.cleanup();
          originalFetch(entry.input, entry.init).then(entry.resolve, entry.reject);
        });
        return queued.length;
      };
      window.fetch = (input, init) => {
        const rawUrl = typeof input === "string" ? input : input?.url || "";
        const url = new URL(rawUrl, window.location.href);
        if (url.pathname === "/api/unlock" || url.pathname === "/api/status") {
          return originalFetch(input, init);
        }
        return new Promise((resolve, reject) => {
          const signal = init?.signal || input?.signal || null;
          let entry;
          const cleanup = () => signal?.removeEventListener("abort", abort);
          const abort = () => {
            removeHeld(entry);
            cleanup();
            window.__sigillumBrowserSmokeAbortedHeldRequestPaths.push(entry.path);
            reject(signal?.reason || new DOMException("The operation was aborted.", "AbortError"));
          };
          entry = { input, init, resolve, reject, cleanup, path: url.pathname };
          if (signal?.aborted) {
            abort();
            return;
          }
          signal?.addEventListener("abort", abort, { once: true });
          window.__sigillumBrowserSmokeHeldRequestCount += 1;
          window.__sigillumBrowserSmokeHeldRequestPaths.push(url.pathname);
          held.push(entry);
        });
      };
      return true;
    })()`,
    "hold post-status refresh requests",
  );
}

async function releasePostStatusRequests(cdp) {
  const released = await evaluate(
    cdp,
    `(() => {
      const release = window.__sigillumBrowserSmokeReleaseRequests;
      if (typeof release !== "function") throw new Error("post-status request hold is not installed");
      return release();
    })()`,
    "release post-status refresh requests",
  );
  if (!Number.isInteger(released) || released < 1) {
    fail(`post-status refresh hold released an invalid request count: ${String(released)}`);
  }
}

async function rawSessionApi(cdp, method, requestPath, body) {
  const result = await evaluate(
    cdp,
    `(async () => {
      const token = sessionStorage.getItem("sigillumSessionToken");
      if (!token) throw new Error("browser session token is unavailable");
      const response = await fetch(${quoted(requestPath)}, {
        method: ${quoted(method)},
        headers: {
          Authorization: "Bearer " + token,
          "Content-Type": "application/json",
        },
        body: ${body === undefined ? "undefined" : quoted(JSON.stringify(body))},
      });
      const payload = await response.json();
      return { ok: response.ok, status: response.status, payload };
    })()`,
    `${method} ${requestPath}`,
  );
  if (!result?.ok || result?.payload?.error) {
    fail(
      `${method} ${requestPath} failed: HTTP ${String(result?.status)} ${JSON.stringify(result?.payload)}`,
    );
  }
  return result.payload;
}

async function installSessionTransitionProbe(cdp) {
  await evaluate(
    cdp,
    `(() => {
      if (window.__sigillumSessionTransitionProbe) {
        throw new Error("session transition probe is already installed");
      }
      const originalFetch = window.fetch.bind(window);
      const rules = [];
      const held = [];
      const records = [];
      let serial = 0;

      const requestParts = (input, init = {}) => {
        const rawUrl = typeof input === "string" ? input : input?.url || "";
        const url = new URL(rawUrl, window.location.href);
        const method = String(init?.method || input?.method || "GET").toUpperCase();
        const signal = init?.signal || input?.signal || null;
        return { method, path: url.pathname, signal };
      };
      const removeHeld = (entry) => {
        const index = held.indexOf(entry);
        if (index >= 0) held.splice(index, 1);
      };

      const probe = {
        holdNext(method, path, label, stage = "request") {
          if (rules.some((rule) => rule.label === label) ||
              records.some((record) => record.label === label)) {
            throw new Error("duplicate hold label: " + label);
          }
          if (stage !== "request" && stage !== "response") {
            throw new Error("invalid hold stage: " + stage);
          }
          rules.push({ method: String(method).toUpperCase(), path, label, stage });
        },
        cancel(label) {
          const index = rules.findIndex((rule) => rule.label === label);
          if (index < 0) throw new Error("no pending hold rule for " + label);
          rules.splice(index, 1);
        },
        release(label) {
          const entry = held.find((candidate) => candidate.record.label === label);
          if (!entry) throw new Error("no held request for " + label);
          removeHeld(entry);
          entry.cleanup();
          entry.record.state = "released";
          if (entry.record.stage === "response") {
            const response = new Proxy(entry.response, {
              get(target, property) {
                const value = Reflect.get(target, property, target);
                if (property === "json") {
                  return async (...args) => {
                    const payload = await value.apply(target, args);
                    entry.record.jsonConsumed = true;
                    entry.record.state = "json-consumed";
                    setTimeout(() => {
                      entry.record.actionSettled = true;
                      entry.record.state = "action-settled";
                    }, 0);
                    return payload;
                  };
                }
                return typeof value === "function" ? value.bind(target) : value;
              },
            });
            entry.resolve(response);
            return;
          }
          originalFetch(entry.input, entry.init).then(
            (response) => {
              entry.record.state = "resolved";
              entry.resolve(response);
            },
            (error) => {
              entry.record.state = "rejected";
              entry.reject(error);
            },
          );
        },
        snapshot() {
          return {
            pendingRules: rules.map((rule) => ({ ...rule })),
            heldLabels: held.map((entry) => entry.record.label),
            records: records.map((record) => ({ ...record })),
          };
        },
        teardown() {
          if (held.length > 0) {
            throw new Error("cannot remove transition probe while requests are held");
          }
          window.fetch = originalFetch;
          delete window.__sigillumSessionTransitionProbe;
        },
      };

      window.fetch = (input, init = {}) => {
        const parts = requestParts(input, init);
        const ruleIndex = rules.findIndex(
          (rule) => rule.method === parts.method && rule.path === parts.path,
        );
        if (ruleIndex < 0) return originalFetch(input, init);

        const [rule] = rules.splice(ruleIndex, 1);
        const record = {
          id: ++serial,
          label: rule.label,
          method: parts.method,
          path: parts.path,
          stage: rule.stage,
          state: rule.stage === "response" ? "awaiting-server" : "held",
          serverResolved: false,
          jsonConsumed: false,
          actionSettled: false,
        };
        records.push(record);
        return new Promise((resolve, reject) => {
          const entry = {
            input,
            init,
            record,
            resolve,
            reject,
            cleanup: () => {},
          };
          const abort = () => {
            if (!["held", "awaiting-server", "server-resolved"].includes(record.state)) return;
            removeHeld(entry);
            entry.cleanup();
            record.state = "aborted";
            reject(new DOMException("The operation was aborted.", "AbortError"));
          };
          entry.cleanup = () => parts.signal?.removeEventListener("abort", abort);
          if (parts.signal?.aborted) {
            abort();
          } else {
            parts.signal?.addEventListener("abort", abort, { once: true });
            if (rule.stage === "request") {
              held.push(entry);
            } else {
              originalFetch(input, init).then(
                (response) => {
                  if (record.state !== "awaiting-server") return;
                  entry.response = response;
                  record.serverResolved = true;
                  record.state = "server-resolved";
                  held.push(entry);
                },
                (error) => {
                  if (record.state === "aborted") return;
                  entry.cleanup();
                  record.state = "rejected";
                  reject(error);
                },
              );
            }
          }
        });
      };
      window.__sigillumSessionTransitionProbe = probe;
      return true;
    })()`,
    "install deterministic session transition request probe",
  );
}

async function holdNextRequest(cdp, method, requestPath, label) {
  await evaluate(
    cdp,
    `(() => {
      const probe = window.__sigillumSessionTransitionProbe;
      if (!probe) throw new Error("session transition probe is not installed");
      probe.holdNext(${quoted(method)}, ${quoted(requestPath)}, ${quoted(label)});
      return true;
    })()`,
    `hold ${label}`,
  );
}

async function holdNextResponse(cdp, method, requestPath, label) {
  await evaluate(
    cdp,
    `(() => {
      const probe = window.__sigillumSessionTransitionProbe;
      if (!probe) throw new Error("session transition probe is not installed");
      probe.holdNext(${quoted(method)}, ${quoted(requestPath)}, ${quoted(label)}, "response");
      return true;
    })()`,
    `hold server response ${label}`,
  );
}

function probeRecordStateExpression(label, state) {
  return `window.__sigillumSessionTransitionProbe?.snapshot().records
    .some((record) => record.label === ${quoted(label)} && record.state === ${quoted(state)})`;
}

function probeHasNoRecordExpression(label) {
  return `!window.__sigillumSessionTransitionProbe?.snapshot().records
    .some((record) => record.label === ${quoted(label)})`;
}

async function releaseHeldRequest(cdp, label) {
  await evaluate(
    cdp,
    `(() => {
      const probe = window.__sigillumSessionTransitionProbe;
      if (!probe) throw new Error("session transition probe is not installed");
      probe.release(${quoted(label)});
      return true;
    })()`,
    `release ${label}`,
  );
}

async function cancelHeldRequestRule(cdp, label) {
  await evaluate(
    cdp,
    `(() => {
      const probe = window.__sigillumSessionTransitionProbe;
      if (!probe) throw new Error("session transition probe is not installed");
      probe.cancel(${quoted(label)});
      return true;
    })()`,
    `cancel ${label}`,
  );
}

async function flushBrowserTasks(cdp, description) {
  await evaluate(
    cdp,
    `(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await new Promise((resolve) => setTimeout(resolve, 0));
      await Promise.resolve();
      return true;
    })()`,
    description,
  );
}

async function clickSyntheticAction(cdp, action, arg0, description) {
  await evaluate(
    cdp,
    `(() => {
      const button = document.createElement("button");
      button.type = "button";
      button.style.display = "none";
      button.dataset.action = ${quoted(action)};
      ${arg0 === undefined ? "" : `button.dataset.arg0 = ${quoted(String(arg0))};`}
      document.body.appendChild(button);
      button.click();
      button.remove();
      return true;
    })()`,
    description,
  );
}

function compartmentCanaryExpression(expectedName, unexpectedName) {
  return `(() => {
    const text = ["apiKeyList", "secretList"]
      .map((id) => document.getElementById(id)?.textContent || "")
      .join(" ");
    return text.includes(${quoted(expectedName)}) &&
      !text.includes(${quoted(unexpectedName)});
  })()`;
}

async function assertBlockedActionDoesNotFetch(cdp, description) {
  const label = `blocked-action-${Date.now()}`;
  await holdNextRequest(cdp, "GET", "/api/queue/jobs", label);
  await clickSyntheticAction(cdp, "loadQueueJobs", undefined, description);
  await flushBrowserTasks(cdp, `${description} task settlement`);
  const reachedFetch = await evaluate(
    cdp,
    `window.__sigillumSessionTransitionProbe.snapshot().records
      .some((record) => record.label === ${quoted(label)})`,
    `${description} fetch observation`,
  );
  if (reachedFetch) {
    fail(`${description} reached fetch during an exclusive session transition`);
  }
  await cancelHeldRequestRule(cdp, label);
}

async function assertWorkspaceDestinations(cdp) {
  await waitFor(cdp, workspaceCardMappingExpression(), "exact workspace card destination mapping");
  await waitFor(cdp, workspaceNavigationExpression(), "five ordered workspace destinations");
  await waitFor(
    cdp,
    `(() => {
      const current = Array.from(
        document.querySelectorAll('[data-action="selectWorkspaceSection"][aria-current="page"]'),
      );
      return current.length === 1 && current[0].dataset.arg0 === "overview" &&
        current[0].classList.contains("active");
    })()`,
    "Overview is the single default destination",
  );
  for (const destination of WORKSPACE_DESTINATIONS) {
    await selectWorkspace(cdp, destination.id);
    const selectedVisible = destination.anchors
      .map((selector) => `(${actuallyVisible(selector)})`)
      .join(" && ");
    const otherHidden = WORKSPACE_DESTINATIONS
      .filter((candidate) => candidate.id !== destination.id)
      .flatMap((candidate) => candidate.anchors)
      .map((selector) => `!(${actuallyVisible(selector)})`)
      .join(" && ");
    await waitFor(
      cdp,
      `(${selectedVisible}) && (${otherHidden})`,
      `${destination.label} unconditional cards visible exclusively`,
    );
    await waitFor(
      cdp,
      persistentStatusStripExpression("UNLOCKED", COMPARTMENT_LABEL),
      `persistent status strip in ${destination.label}`,
    );
  }
  await waitFor(
    cdp,
    `document.getElementById("pushCard")?.dataset.workspaceSection === "vault" &&
      !(${actuallyVisible("#pushCard")})`,
    "conditional Vault push card is mapped and hidden for the one-compartment fixture",
  );
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
      unlockButtonText: button?.textContent || "",
      controlsReady: ready(input) && ready(button),
      hasToken: !!sessionStorage.getItem("sigillumSessionToken")
    };
  })()`;
}

function privateWorkspaceScrubbedExpression() {
  return `(() => {
    const privateText = ["apiKeyList", "secretList", "walletMnemonicReveal"]
      .map((id) => document.getElementById(id)?.textContent || "")
      .join(" ");
    const sensitiveFieldsEmpty = Array.from(
      document.querySelectorAll('input[type="password"], input[type="file"]'),
    ).every((field) => !field.value);
    const mnemonicFieldsEmpty = ["walletImportSeedMnemonic", "seedMnemonic"]
      .every((id) => !(document.getElementById(id)?.value || ""));
    return document.querySelectorAll(".secret-value").length === 0 &&
      sensitiveFieldsEmpty && mnemonicFieldsEmpty &&
      !privateText.includes(${quoted(API_KEY_NAME)}) &&
      !privateText.includes(${quoted(API_KEY_VALUE)}) &&
      !privateText.includes(${quoted(SECRET_NAME)}) &&
      !privateText.includes(${quoted(SECRET_VALUE)}) &&
      !privateText.includes(${quoted(SECOND_SECRET_NAME)}) &&
      !privateText.includes(${quoted(SECOND_SECRET_VALUE)}) &&
      !privateText.includes(${quoted(HELD_MUTATION_SECRET_NAME)}) &&
      !privateText.includes(${quoted(HELD_MUTATION_SECRET_VALUE)}) &&
      !privateText.includes(${quoted(UNSAVED_API_KEY_VALUE)}) &&
      !privateText.includes(${quoted(UNSAVED_SECRET_VALUE)}) &&
      !privateText.includes(${quoted(UNSAVED_MNEMONIC)});
  })()`;
}

function revealStateExpression(listSelector, quotedValue) {
  return `(() => {
    const list = document.querySelector(${quoted(listSelector)});
    const buttons = Array.from(list?.querySelectorAll("button") || []);
    const revealButton = buttons.find((node) => (node.textContent || "").trim().includes("Reveal"));
    const displayButton = revealButton || buttons.find((node) => (node.textContent || "").trim().includes("Hide"));
    const ready = (el) => {
      if (!el || el.disabled || el.closest(".hidden")) return false;
      const style = window.getComputedStyle(el);
      return style.display !== "none" && style.visibility !== "hidden";
    };
    const listText = list?.textContent || "";
    return {
      revealed: listText.includes(${quotedValue}),
      buttonPresent: !!revealButton,
      buttonReady: ready(revealButton),
      buttonText: displayButton?.textContent?.trim() || "",
      listTextLength: listText.length
    };
  })()`;
}

async function waitForRevealAttempt(cdp, listSelector, quotedValue, description) {
  const attemptStart = Date.now();
  const deadline = Date.now() + REVEAL_TIMEOUT_MS;
  let lastState = null;
  while (Date.now() < deadline) {
    lastState = await evaluate(cdp, revealStateExpression(listSelector, quotedValue), description);
    if (lastState.revealed) {
      return { ok: true, state: lastState };
    }
    if (Date.now() - attemptStart >= REVEAL_RETRY_SETTLE_MS && lastState.buttonPresent && lastState.buttonReady) {
      return { ok: false, state: lastState };
    }
    await sleep(150);
  }
  return { ok: false, state: lastState, timedOut: true };
}

async function revealListValue(cdp, listSelector, buttonText, quotedValue, description) {
  let lastState = null;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    const label = attempt === 1 ? description : `${description} (retry ${attempt - 1})`;
    await clickByText(cdp, `${listSelector} button`, buttonText, label);
    const result = await waitForRevealAttempt(cdp, listSelector, quotedValue, description);
    lastState = result.state;
    if (result.ok) {
      return;
    }
    if (result.timedOut) {
      break;
    }
  }

  fail(`${description} did not become true: ${JSON.stringify(lastState)}`);
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

async function reauthWithPassphrase(cdp, reason) {
  let lastState = null;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    const label = attempt === 1 ? `unlock after ${reason}` : `unlock after ${reason} (retry ${attempt - 1})`;
    await submitPassphraseUnlock(cdp, label);
    const result = await waitForReauthAttempt(
      cdp,
      attempt === 1
        ? `browser session token after ${reason} reauth (first attempt)`
        : `browser session token after ${reason} reauth`,
    );
    lastState = result.state;
    if (result.ok) {
      return;
    }
    if (result.timedOut) {
      break;
    }
  }

  fail(`browser session token after ${reason} reauth did not become true: ${JSON.stringify(lastState)}`);
}

async function assertSessionTransitionBoundaries(cdp) {
  const added = await rawSessionApi(cdp, "POST", "/api/compartment/add", {
    label: SECOND_COMPARTMENT_LABEL,
    threshold: 2,
    passphrase_mode: null,
  });
  const secondCompartmentId = Number(added.id);
  if (!Number.isInteger(secondCompartmentId) || secondCompartmentId < 1) {
    fail(`second compartment returned an invalid id: ${JSON.stringify(added)}`);
  }
  const secondSelector =
    `[data-action="switchCompartment"][data-arg0="${secondCompartmentId}"]`;
  const firstSelector = '[data-action="switchCompartment"][data-arg0="0"]';

  await click(cdp, '[data-action="refreshWorkspace"]', "refresh after adding second compartment");
  await waitFor(
    cdp,
    "document.getElementById('compartmentCount').textContent.trim() === '2'",
    "two unlocked compartments in the C7 workspace",
  );
  await waitFor(cdp, visibleAndEnabled(secondSelector), "second compartment switch control");

  await click(cdp, secondSelector, "switch to second compartment for canary setup");
  await waitFor(
    cdp,
    persistentStatusStripExpression("UNLOCKED", SECOND_COMPARTMENT_LABEL),
    "second compartment active for canary setup",
  );
  await selectWorkspace(cdp, "vault");
  await waitFor(
    cdp,
    `!document.getElementById("secretList").textContent.includes(${quoted(SECRET_NAME)})`,
    "first-compartment canary absent from second compartment",
  );
  await setValue(cdp, "#secretName", SECOND_SECRET_NAME, "second-compartment secret name");
  await setValue(cdp, "#secretValue", SECOND_SECRET_VALUE, "second-compartment secret value");
  await click(cdp, '[data-action="setSecret"]', "store second-compartment secret canary");
  await waitFor(
    cdp,
    compartmentCanaryExpression(SECOND_SECRET_NAME, SECRET_NAME),
    "second-compartment canary isolated from first compartment",
  );

  await click(cdp, firstSelector, "return to first compartment after canary setup");
  await waitFor(
    cdp,
    persistentStatusStripExpression("UNLOCKED", COMPARTMENT_LABEL),
    "first compartment active after canary setup",
  );
  await selectWorkspace(cdp, "vault");
  await waitFor(
    cdp,
    compartmentCanaryExpression(SECRET_NAME, SECOND_SECRET_NAME),
    "first-compartment canary isolated from second compartment",
  );
  await waitFor(
    cdp,
    "document.getElementById('refreshMeta').dataset.state === 'live'",
    "stable workspace before transition probes",
  );

  await installSessionTransitionProbe(cdp);

  // A read already in flight must be aborted when a compartment transition
  // begins. The switch itself is held so the scrubbed transition DOM can be
  // inspected before either compartment is allowed to render.
  await holdNextRequest(cdp, "GET", "/api/status", "old-read");
  await click(cdp, '[data-action="refreshWorkspace"]', "start held old-compartment read");
  await waitFor(
    cdp,
    probeRecordStateExpression("old-read", "held"),
    "old-compartment read is held",
  );
  await holdNextRequest(cdp, "POST", "/api/compartment/switch", "scrubbed-switch");
  await holdNextRequest(cdp, "GET", "/api/status", "new-context-status");
  await click(cdp, secondSelector, "start scrubbed switch to second compartment");
  await waitFor(
    cdp,
    probeRecordStateExpression("old-read", "aborted"),
    "old-compartment read aborted by switch",
  );
  await waitFor(
    cdp,
    probeRecordStateExpression("scrubbed-switch", "held"),
    "compartment switch held after immediate scrub",
  );
  await waitFor(
    cdp,
    `document.body.dataset.sessionTransition === "true" &&
      !document.getElementById("stripLockNow").disabled &&
      ${privateWorkspaceScrubbedExpression()}`,
    "old private DOM scrubbed while switch is held and Lock remains operable",
  );

  // A concurrent same-path attempt must lose without clearing the original
  // transition owner. A normal action must also be rejected before fetch.
  await clickSyntheticAction(
    cdp,
    "switchCompartment",
    secondCompartmentId,
    "concurrent same-path compartment switch",
  );
  await flushBrowserTasks(cdp, "same-path loser settlement");
  await waitFor(
    cdp,
    `document.body.dataset.sessionTransition === "true" &&
      ${probeRecordStateExpression("scrubbed-switch", "held")}`,
    "same-path loser cannot end the winning switch",
  );
  await assertBlockedActionDoesNotFetch(
    cdp,
    "queue refresh blocked while compartment switch owns the session",
  );

  await releaseHeldRequest(cdp, "scrubbed-switch");
  await waitFor(
    cdp,
    probeRecordStateExpression("new-context-status", "held"),
    "new-compartment refresh held before private rendering",
  );
  await waitFor(
    cdp,
    privateWorkspaceScrubbedExpression(),
    "neither compartment rendered while new context status is held",
  );
  await releaseHeldRequest(cdp, "new-context-status");
  await waitFor(
    cdp,
    persistentStatusStripExpression("UNLOCKED", SECOND_COMPARTMENT_LABEL),
    "second compartment active after held switch",
  );
  await selectWorkspace(cdp, "vault");
  await waitFor(
    cdp,
    compartmentCanaryExpression(SECOND_SECRET_NAME, SECRET_NAME),
    "only second-compartment canary renders after switch",
  );

  // Mutations cannot be aborted safely. A switch must scrub/inert the old UI
  // immediately, then wait until the old mutation settles before its own
  // request reaches fetch.
  await setValue(cdp, "#secretName", HELD_MUTATION_SECRET_NAME, "held mutation secret name");
  await setValue(cdp, "#secretValue", HELD_MUTATION_SECRET_VALUE, "held mutation secret value");
  await holdNextRequest(cdp, "POST", "/api/secrets/set", "old-mutation");
  await click(cdp, '[data-action="setSecret"]', "start held old-compartment mutation");
  await waitFor(
    cdp,
    probeRecordStateExpression("old-mutation", "held"),
    "old-compartment mutation is held",
  );
  await holdNextRequest(cdp, "POST", "/api/compartment/switch", "post-drain-switch");
  await click(cdp, firstSelector, "request switch while old mutation is held");
  await waitFor(
    cdp,
    `document.body.dataset.sessionTransition === "true" &&
      ${probeHasNoRecordExpression("post-drain-switch")} &&
      ${privateWorkspaceScrubbedExpression()}`,
    "switch waits behind old mutation without retaining private DOM",
  );
  await assertBlockedActionDoesNotFetch(
    cdp,
    "queue refresh blocked while switch waits for old mutation",
  );
  await releaseHeldRequest(cdp, "old-mutation");
  await waitFor(
    cdp,
    probeRecordStateExpression("post-drain-switch", "held"),
    "switch reaches fetch only after old mutation settles",
  );
  await releaseHeldRequest(cdp, "post-drain-switch");
  await waitFor(
    cdp,
    persistentStatusStripExpression("UNLOCKED", COMPARTMENT_LABEL),
    "first compartment active after mutation-drained switch",
  );
  await selectWorkspace(cdp, "vault");
  await waitFor(
    cdp,
    compartmentCanaryExpression(SECRET_NAME, SECOND_SECRET_NAME),
    "first-compartment data restored without second-compartment mixing",
  );

  // Earlier cases prove pre-network holds. This final race lets the switch
  // reach the daemon and waits until its HTTP response exists, but withholds
  // that Response from the app. Lock must still work with predecessor token T.
  await holdNextResponse(
    cdp, "POST", "/api/compartment/switch", "committed-switch-response",
  );
  await click(cdp, secondSelector, "start server-committed switch that Lock will preempt");
  await waitFor(
    cdp,
    probeRecordStateExpression("committed-switch-response", "server-resolved"),
    "switch response held after the daemon committed",
  );
  await waitFor(
    cdp,
    `Boolean(sessionStorage.getItem("sigillumSessionToken")) &&
      document.body.dataset.sessionTransition === "true" &&
      ${privateWorkspaceScrubbedExpression()}`,
    "browser still holds predecessor token while committed response is withheld",
  );
  await evaluate(
    cdp,
    `(() => {
      window.__sigillumTransitionOriginalConfirm = window.confirm;
      window.confirm = () => true;
      return true;
    })()`,
    "install transition Lock confirmation",
  );
  await waitFor(cdp, visibleAndEnabled("#stripLockNow"), "Lock remains operable after server commit");
  await click(cdp, "#stripLockNow", "Lock with predecessor token supersedes committed switch");
  await waitFor(cdp, "!sessionStorage.getItem('sigillumSessionToken')", "Lock clears browser token");
  await waitFor(cdp, "document.body.dataset.mode === 'locked'", "Lock closes committed-switch session");
  await releaseHeldRequest(cdp, "committed-switch-response");
  await waitFor(
    cdp,
    probeRecordStateExpression("committed-switch-response", "action-settled"),
    "late committed switch response is consumed and its action settles",
  );
  await flushBrowserTasks(cdp, "late committed switch settlement");
  await waitFor(
    cdp,
    `document.body.dataset.mode === "locked" &&
      document.body.dataset.sessionTransition !== "true" &&
      !sessionStorage.getItem("sigillumSessionToken") &&
      ${resetStatusStripExpression()} &&
      ${privateWorkspaceScrubbedExpression()}`,
    "late committed switch cannot restore a token, status, or private DOM after Lock",
  );
  await evaluate(
    cdp,
    `(() => {
      window.confirm = window.__sigillumTransitionOriginalConfirm;
      delete window.__sigillumTransitionOriginalConfirm;
      window.__sigillumSessionTransitionProbe.teardown();
      return true;
    })()`,
    "remove session transition request probe",
  );

  // The rest of the smoke continues from the durable passphrase compartment;
  // the higher-threshold fixture compartment intentionally stays locked.
  await reauthWithPassphrase(cdp, "transition preemption proof");
  await waitFor(cdp, "document.body.dataset.mode === 'unlocked'", "unlocked after transition proofs");
  await waitFor(
    cdp,
    persistentStatusStripExpression("UNLOCKED", COMPARTMENT_LABEL),
    "first compartment active after transition proof reauth",
  );
  await waitFor(
    cdp,
    "document.getElementById('refreshMeta').dataset.state === 'live'",
    "workspace reconciled after transition proof reauth",
  );
  await selectWorkspace(cdp, "vault");
  await waitFor(
    cdp,
    compartmentCanaryExpression(SECRET_NAME, SECOND_SECRET_NAME),
    "durable first-compartment canary restored after transition proof reauth",
  );
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
  await setViewport(cdp, DESKTOP_VIEWPORT, "desktop");
  await cdp.send("Page.navigate", { url: TARGET_URL });

  await waitFor(cdp, "document.readyState === 'complete'", "page load");
  await waitFor(cdp, "document.title.includes('Sigillum Vault')", "Sigillum title");
  await waitFor(
    cdp,
    `document.body.dataset.mode === 'setup' && ${actuallyVisible("#setupCard")}`,
    "first-run setup UI",
  );
  await waitFor(
    cdp,
    "document.getElementById('statusBadge').textContent.trim() === 'NO VAULT'",
    "no-vault badge during setup",
  );
  await waitFor(
    cdp,
    resetStatusStripExpression(),
    "status strip hidden and reset during setup",
  );

  await waitFor(
    cdp,
    `${actuallyVisible("#wizStepWelcome")} && !(${actuallyVisible("#wizStep0")})`,
    "visible setup welcome step",
  );
  await click(cdp, '[data-action="wizGetStarted"]', "Get started");
  await waitFor(
    cdp,
    `${actuallyVisible("#wizStep0")} && !(${actuallyVisible("#wizStepWelcome")})`,
    "visible protection-model step",
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
  await waitFor(
    cdp,
    persistentStatusStripExpression("UNLOCKED", COMPARTMENT_LABEL),
    "unlocked persistent status strip",
  );

  await assertWorkspaceDestinations(cdp);
  await assertNarrowWorkspaceDestinations(cdp);
  await assertStatusStripSelfCheckNavigation(cdp);
  await selectWorkspace(cdp, "vault");
  await waitFor(
    cdp,
    `${actuallyVisible("#apiKeysCard")} && ${actuallyVisible("#secretsCard")}`,
    "vault secret cards actually visible",
  );
  await waitFor(
    cdp,
    [
      "#apiKeyName",
      "#apiKeyValue",
      '[data-action="setApiKey"]',
      "#secretName",
      "#secretValue",
      '[data-action="setSecret"]',
    ].map((selector) => `(${visibleAndEnabled(selector)})`).join(" && "),
    "vault secret controls actually visible",
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

  await revealListValue(cdp, "#apiKeyList", "Reveal", quoted(API_KEY_VALUE), "API key revealed value");
  await revealListValue(cdp, "#secretList", "Reveal", quoted(SECRET_VALUE), "secret revealed value");

  await assertSessionTransitionBoundaries(cdp);

  // Leave unsaved private values behind as canaries. The session boundary
  // must erase both rendered data and in-progress operator input.
  await setValue(cdp, "#apiKeyValue", UNSAVED_API_KEY_VALUE, "unsaved API key canary");
  await setValue(cdp, "#secretValue", UNSAVED_SECRET_VALUE, "unsaved secret canary");
  await setValue(cdp, "#walletImportSeedMnemonic", UNSAVED_MNEMONIC, "unsaved mnemonic canary");

  // Persist Overview so the locked-to-unlocked timing assertion has an exact,
  // deterministic destination to verify while the data fan-out is held.
  await selectWorkspace(cdp, "overview");

  await evaluate(
    cdp,
    `(() => {
      window.__sigillumBrowserSmokeOriginalConfirm = window.confirm;
      window.__sigillumBrowserSmokeLockConfirmation = "";
      window.confirm = (message) => {
        window.__sigillumBrowserSmokeLockConfirmation = String(message || "");
        return true;
      };
      return true;
    })()`,
    "install Lock now confirmation observer",
  );
  await waitFor(cdp, visibleAndEnabled("#stripLockNow"), "visible Lock now control");
  await click(cdp, "#stripLockNow", "Lock now");
  await waitFor(
    cdp,
    "window.__sigillumBrowserSmokeLockConfirmation.includes('Lock all compartments?')",
    "Lock now confirmation invoked",
  );
  await waitFor(cdp, "!sessionStorage.getItem('sigillumSessionToken')", "browser session token cleared by Lock now");
  await waitFor(cdp, "document.body.dataset.mode === 'locked'", "locked UI after Lock now");
  await waitFor(cdp, "document.getElementById('statusBadge').textContent.trim() === 'LOCKED'", "locked badge after Lock now");
  await waitFor(
    cdp,
    resetStatusStripExpression(),
    "status strip hidden and reset after Lock now",
  );
  await waitFor(
    cdp,
    privateWorkspaceScrubbedExpression(),
    "rendered and unsaved private workspace state scrubbed after Lock now",
  );
  await evaluate(
    cdp,
    `(() => {
      if (window.__sigillumBrowserSmokeOriginalConfirm) {
        window.confirm = window.__sigillumBrowserSmokeOriginalConfirm;
      }
      return true;
    })()`,
    "restore browser confirmation handler",
  );

  await waitFor(
    cdp,
    "document.body.dataset.sessionTransition !== 'true'",
    "Lock reconciliation settled before predecessor-read proof",
  );
  await installPostStatusRequestHold(cdp);
  await clickSyntheticAction(
    cdp, "loadQueueJobs", undefined, "start held predecessor read before reauth",
  );
  await waitFor(
    cdp,
    "window.__sigillumBrowserSmokeHeldRequestPaths.includes('/api/queue/jobs')",
    "predecessor session read is held before reauth",
  );
  await reauthWithPassphrase(cdp, "Lock now");
  await waitFor(
    cdp,
    "window.__sigillumBrowserSmokeAbortedHeldRequestPaths.includes('/api/queue/jobs')",
    "new session adoption aborts the held predecessor read",
  );
  await waitFor(cdp, "document.body.dataset.mode === 'unlocked'", "unlocked workspace after reauth");
  await waitFor(
    cdp,
    "Number(window.__sigillumBrowserSmokeHeldRequestCount || 0) > 0",
    "post-status refresh fan-out is held",
  );
  await waitFor(cdp, workspaceNavigationExpression(), "workspace navigation after reauth status");
  await waitFor(cdp, workspaceCardMappingExpression(), "workspace card mapping after reauth status");
  await waitFor(
    cdp,
    exclusivelyVisibleDestinationExpression("overview"),
    "only Overview cards visible while the post-unlock fan-out is pending",
  );
  await waitFor(
    cdp,
    persistentStatusStripExpression("UNLOCKED", COMPARTMENT_LABEL),
    "persistent status strip while the post-unlock fan-out is pending",
  );
  await selectWorkspace(cdp, "vault");
  await waitFor(
    cdp,
    privateWorkspaceScrubbedExpression(),
    "Vault stays scrubbed until current-session loaders finish",
  );
  await releasePostStatusRequests(cdp);
  await waitFor(
    cdp,
    "document.getElementById('refreshMeta').dataset.state === 'live'",
    "post-reauth refresh completion",
  );
  await waitFor(cdp, "document.getElementById('apiKeyCount').textContent.trim() === '1'", "API key count after reauth");
  await waitFor(cdp, "document.getElementById('secretCount').textContent.trim() === '1'", "secret count after reauth");

  await selectWorkspace(cdp, "vault");
  await waitFor(cdp, `document.getElementById('apiKeyList').textContent.includes(${quoted(API_KEY_NAME)})`, "API key listed after reauth");
  await waitFor(cdp, `document.getElementById('secretList').textContent.includes(${quoted(SECRET_NAME)})`, "secret listed after reauth");

  await waitFor(cdp, visibleAndEnabled('[data-action="logoutSession"]'), "visible browser logout control");
  await click(cdp, '[data-action="logoutSession"]', "logout browser session");
  await waitFor(cdp, "!sessionStorage.getItem('sigillumSessionToken')", "browser session token cleared by logout");
  await waitFor(cdp, "document.body.dataset.mode === 'locked'", "locked UI after browser logout");
  await waitFor(cdp, "document.getElementById('statusBadge').textContent.trim() === 'LOCKED'", "locked badge after browser logout");
  await waitFor(
    cdp,
    resetStatusStripExpression(),
    "status strip hidden and reset after browser logout",
  );
  await waitFor(
    cdp,
    privateWorkspaceScrubbedExpression(),
    "private workspace scrubbed after browser logout",
  );

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
