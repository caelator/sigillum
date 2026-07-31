#!/usr/bin/env node
// Automated accessibility audit for the shipped Sigillum operator console.
//
// The audit starts the stateful screenshot mock, loads the exact checked-in UI
// bundles in headless Chrome, injects the pinned axe-core build, and checks the
// setup, locked, and representative unlocked routes. Harness failures are
// fatal: a missing browser/dependency, stale bundle, page exception, unknown
// mock route, missing scenario, violation, incomplete check, or malformed axe
// result can never look green.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { fileURLToPath } from "node:url";
import { startServer } from "./ui-screenshots/server.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, "..");
const UI_ROOT = path.join(REPO_ROOT, "crates/sigillum-daemon/ui");
const AXE_VERSION = "4.12.1";
const AXE_SOURCE_PATH = path.join(UI_ROOT, "node_modules/axe-core/axe.min.js");
const UI_PACKAGE_PATH = path.join(UI_ROOT, "package.json");
const TIMEOUT_MS = 30_000;

const AXE_TAGS = [
  "wcag2a",
  "wcag2aa",
  "wcag21a",
  "wcag21aa",
  "wcag22a",
  "wcag22aa",
  "best-practice",
];

// Scenarios are ordered so pages can be reused within each daemon mode. The
// five unlocked destinations are always covered; their routed subviews are
// included where the UI exposes materially different content.
const SCENARIOS = [
  {
    name: "setup welcome",
    mode: "setup",
    ready: "#setupCard:not(.hidden) #wizStepWelcome.active",
  },
  {
    name: "setup protection model",
    mode: "setup",
    click: '[data-action="wizGetStarted"]',
    ready: "#setupCard:not(.hidden) #wizStep0.active",
  },
  {
    name: "locked unlock",
    mode: "locked",
    ready: "#authCard:not(.hidden)",
  },
  {
    name: "unlocked overview",
    mode: "unlocked",
    route: "#/overview",
    ready: "#statusCard:not(.hidden)",
  },
  {
    name: "unlocked receive",
    mode: "unlocked",
    route: "#/receive",
    ready: "#receivingCard:not(.hidden)",
  },
  {
    name: "unlocked portfolio holdings",
    mode: "unlocked",
    route: "#/portfolio",
    ready: '[data-portfolio="addresses-wrap"]',
  },
  {
    name: "unlocked portfolio scan",
    mode: "unlocked",
    route: "#/portfolio/scan",
    ready: '[data-portfolio="steps"]',
  },
  {
    name: "unlocked portfolio risk",
    mode: "unlocked",
    route: "#/portfolio/risk",
    ready: '[data-portfolio="risk-filter-bar"]',
  },
  {
    name: "unlocked portfolio tokens",
    mode: "unlocked",
    route: "#/portfolio/tokens",
    ready: '[data-portfolio="registry-form"]',
  },
  {
    name: "unlocked move plans",
    mode: "unlocked",
    route: "#/move",
    ready: '[data-move-region="plans-list"]',
  },
  {
    name: "unlocked move plan detail",
    mode: "unlocked",
    route: "#/move/plan/plan-20260715-a1",
    ready: '[data-move-region="plan-detail"]',
  },
  {
    name: "unlocked move queue",
    mode: "unlocked",
    route: "#/move/queue",
    ready: '[data-move-region="queue-groups"]',
  },
  {
    name: "unlocked move policy",
    mode: "unlocked",
    route: "#/move/policy",
    ready: '[data-move-region="policy-current"]',
  },
  {
    name: "unlocked vault",
    mode: "unlocked",
    route: "#/vault",
    ready: '[data-vault="root"]',
  },
  {
    name: "unlocked command palette",
    mode: "unlocked",
    route: "#/overview",
    shortcut: "command-palette",
    ready: '[data-palette="dialog"]',
  },
];

function fail(message) {
  throw new Error(`ui-accessibility: ${message}`);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function quoted(value) {
  return JSON.stringify(value);
}

function loadAxeSource() {
  let packageJson;
  try {
    packageJson = JSON.parse(fs.readFileSync(UI_PACKAGE_PATH, "utf8"));
  } catch (error) {
    fail(`could not read UI package metadata: ${error.message}`);
  }
  const declared = packageJson.devDependencies?.["axe-core"];
  if (declared !== AXE_VERSION) {
    fail(
      `axe-core must be pinned exactly to ${AXE_VERSION}; package.json declares ${quoted(declared)}`,
    );
  }
  if (!fs.existsSync(AXE_SOURCE_PATH) || fs.statSync(AXE_SOURCE_PATH).size === 0) {
    fail(
      `pinned axe-core source is missing; run ` +
        "`npm --prefix crates/sigillum-daemon/ui ci --ignore-scripts`",
    );
  }
  return fs.readFileSync(AXE_SOURCE_PATH, "utf8");
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
    fail("Chrome or Chromium was not found; set CHROME_BIN to a compatible executable");
  }
  return found;
}

function launchChrome() {
  const profileDir = fs.mkdtempSync(path.join(os.tmpdir(), "sigillum-ui-a11y-profile."));
  const child = spawn(
    findChromeExecutable(),
    [
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
    ],
    { stdio: ["ignore", "ignore", "pipe"] },
  );

  let stderr = "";
  let resolved = false;
  const ready = new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error(`timed out waiting for Chrome DevTools endpoint\n${stderr}`));
    }, TIMEOUT_MS);
    child.once("exit", (code, signal) => {
      if (resolved) return;
      clearTimeout(timer);
      reject(new Error(`Chrome exited before DevTools was ready: code=${code} signal=${signal}\n${stderr}`));
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
    ready,
    async cleanup() {
      if (child.exitCode === null && child.signalCode === null) child.kill("SIGTERM");
      if (child.exitCode === null && child.signalCode === null) {
        await Promise.race([once(child, "exit").catch(() => {}), sleep(2_000)]);
      }
      if (child.exitCode === null && child.signalCode === null) {
        child.kill("SIGKILL");
        await Promise.race([once(child, "exit").catch(() => {}), sleep(1_000)]);
      }
      const options = { recursive: true, force: true, maxRetries: 10, retryDelay: 200 };
      try {
        fs.rmSync(profileDir, options);
      } catch {
        await sleep(1_000);
        try {
          fs.rmSync(profileDir, options);
        } catch (error) {
          console.warn(`ui-accessibility: leaving temporary profile ${profileDir}: ${error.message}`);
        }
      }
    },
  };
}

class CdpClient {
  constructor(webSocketUrl, targetId, debugOrigin) {
    this.webSocket = new WebSocket(webSocketUrl);
    this.targetId = targetId;
    this.debugOrigin = debugOrigin;
    this.nextId = 1;
    this.pending = new Map();
    this.pageErrors = [];
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
    if (message.method === "Runtime.consoleAPICalled" && message.params.type === "error") {
      this.pageErrors.push(
        (message.params.args || []).map((arg) => arg.value || arg.description || "").join(" "),
      );
    } else if (message.method === "Runtime.exceptionThrown") {
      const details = message.params.exceptionDetails || {};
      const reason =
        details.exception?.description ||
        details.exception?.value ||
        details.text ||
        "runtime exception";
      const frame = details.stackTrace?.callFrames?.[0];
      const location = frame
        ? ` at ${frame.functionName || "<anonymous>"} (${frame.url || "inline"}:${frame.lineNumber + 1}:${frame.columnNumber + 1})`
        : "";
      this.pageErrors.push("exception: " + reason + location);
    }
  }

  async send(method, params = {}) {
    await this.opened;
    const id = this.nextId++;
    const response = new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
    this.webSocket.send(JSON.stringify({ id, method, params }));
    return response;
  }

  takeErrors() {
    return this.pageErrors.splice(0);
  }

  async close() {
    this.webSocket.close();
    try {
      await fetch(`${this.debugOrigin}/json/close/${encodeURIComponent(this.targetId)}`);
    } catch (_) {
      // Chrome cleanup remains authoritative if the target is already gone.
    }
  }
}

async function openPage(browserWebSocketUrl) {
  const debugUrl = new URL(browserWebSocketUrl);
  const debugOrigin = `http://${debugUrl.hostname}:${debugUrl.port}`;
  const endpoint = `${debugOrigin}/json/new?about:blank`;
  let response = await fetch(endpoint, { method: "PUT" });
  if (!response.ok) response = await fetch(endpoint);
  if (!response.ok) fail(`could not open browser target: HTTP ${response.status}`);
  const target = await response.json();
  if (!target.webSocketDebuggerUrl) fail("browser target did not include a DevTools websocket URL");
  return new CdpClient(target.webSocketDebuggerUrl, target.id, debugOrigin);
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
      details.exception?.description || details.exception?.value || details.text || "runtime exception";
    fail(`${description}: ${reason}`);
  }
  return result.result?.value;
}

async function waitFor(cdp, expression, description, timeoutMs = TIMEOUT_MS) {
  const deadline = Date.now() + timeoutMs;
  let lastError = "";
  while (Date.now() < deadline) {
    try {
      if (await evaluate(cdp, expression, description)) return;
    } catch (error) {
      lastError = error.message;
    }
    await sleep(150);
  }
  fail(`${description} did not become true${lastError ? ` (${lastError})` : ""}`);
}

async function openModePage(browserWs, base, mode, axeSource) {
  const cdp = await openPage(browserWs);
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    width: 1440,
    height: 900,
    deviceScaleFactor: 1,
    mobile: false,
  });
  if (mode === "unlocked") {
    await cdp.send("Page.addScriptToEvaluateOnNewDocument", {
      source:
        "try{sessionStorage.setItem('sigillumSessionToken','ui-accessibility-session-token')}catch(e){}",
    });
  }
  await cdp.send("Page.navigate", { url: base + "/" });
  await waitFor(cdp, "document.readyState === 'complete'", `${mode} page load`);
  await waitFor(cdp, `document.body.dataset.mode === ${quoted(mode)}`, `${mode} body mode`, 20_000);
  await evaluate(cdp, axeSource + "\n//# sourceURL=axe-core-" + AXE_VERSION + ".js", "inject axe-core");
  const loadedVersion = await evaluate(cdp, "globalThis.axe?.version", "read axe-core version");
  if (loadedVersion !== AXE_VERSION) {
    fail(`expected axe-core ${AXE_VERSION}, browser loaded ${quoted(loadedVersion)}`);
  }
  await sleep(mode === "unlocked" ? 2_500 : 800);
  return cdp;
}

async function prepareScenario(cdp, scenario) {
  if (scenario.route) {
    await evaluate(
      cdp,
      `(() => {
        const route = ${quoted(scenario.route)};
        if (location.hash === route) {
          window.dispatchEvent(new HashChangeEvent("hashchange"));
        } else {
          location.hash = route;
        }
        window.scrollTo(0, 0);
        return true;
      })()`,
      `${scenario.name} route navigation`,
    );
    await waitFor(
      cdp,
      `location.hash === ${quoted(scenario.route)}`,
      `${scenario.name} route`,
    );
  }
  if (scenario.click) {
    await evaluate(
      cdp,
      `(() => {
        const element = document.querySelector(${quoted(scenario.click)});
        if (!element) throw new Error("missing click target");
        element.click();
        return true;
      })()`,
      `${scenario.name} interaction`,
    );
  }
  if (scenario.shortcut === "command-palette") {
    await evaluate(
      cdp,
      `(() => {
        const event = new KeyboardEvent("keydown", {
          key: "k",
          code: "KeyK",
          metaKey: true,
          bubbles: true,
          cancelable: true,
        });
        document.dispatchEvent(event);
        if (!event.defaultPrevented) {
          throw new Error("command palette shortcut was not consumed");
        }
        return true;
      })()`,
      `${scenario.name} shortcut`,
    );
  }
  await waitFor(
    cdp,
    `(() => {
      const element = document.querySelector(${quoted(scenario.ready)});
      if (!element || !element.isConnected) return false;
      const style = getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      return style.display !== "none" && style.visibility !== "hidden" && rect.width > 0 && rect.height > 0;
    })()`,
    `${scenario.name} content`,
    20_000,
  );
  await waitFor(
    cdp,
    `Array.from(document.querySelectorAll(".skeleton")).every((element) => element.offsetParent === null)`,
    `${scenario.name} data population`,
    20_000,
  );
  await sleep(300);
}

function assertNoPageErrors(cdp, context) {
  const errors = cdp.takeErrors();
  if (!errors.length) return;
  fail(`${context} produced browser errors:\n${errors.map((error) => "  " + error).join("\n")}`);
}

function assertNoUnknownRequests(server, context) {
  const unknown = server.getUnknownRequests();
  if (!unknown.length) return;
  fail(
    `${context} called unregistered mock routes:\n` +
      unknown.map((request) => `  ${request.method} ${request.path}`).join("\n"),
  );
}

async function runAxe(cdp, scenario) {
  const result = await evaluate(
    cdp,
    `(async () => globalThis.axe.run(document, {
      runOnly: { type: "tag", values: ${JSON.stringify(AXE_TAGS)} }
    }))()`,
    `${scenario.name} axe audit`,
  );
  const buckets = ["violations", "passes", "incomplete", "inapplicable"];
  if (
    result?.testEngine?.version !== AXE_VERSION ||
    buckets.some((bucket) => !Array.isArray(result?.[bucket])) ||
    buckets.reduce((total, bucket) => total + result[bucket].length, 0) === 0
  ) {
    fail(`${scenario.name} returned a malformed or empty axe result`);
  }
  return result;
}

function printFindings(findings) {
  const violationCount = findings.reduce((count, finding) => count + finding.result.violations.length, 0);
  const incompleteCount = findings.reduce((count, finding) => count + finding.result.incomplete.length, 0);
  const nodeCount = findings.reduce(
    (count, finding) =>
      count +
      [...finding.result.violations, ...finding.result.incomplete].reduce(
        (nodes, result) => nodes + result.nodes.length,
        0,
      ),
    0,
  );
  console.error(
    `ui-accessibility: ${violationCount} violation(s) and ${incompleteCount} incomplete check(s) ` +
      `affecting ${nodeCount} node(s) across ${findings.length} scenario(s)`,
  );
  for (const { scenario, result } of findings) {
    for (const [bucket, results] of [
      ["violation", result.violations],
      ["incomplete", result.incomplete],
    ]) {
      for (const axeResult of results) {
        console.error(`\n[${scenario.name}] ${bucket}: ${axeResult.id}`);
        console.error(`  impact: ${axeResult.impact || "unknown"}`);
        console.error(`  rule: ${axeResult.help}`);
        console.error(`  help URL: ${axeResult.helpUrl}`);
        console.error(`  nodes: ${axeResult.nodes.length}`);
        axeResult.nodes.forEach((node, index) => {
          console.error(`  node ${index + 1} selector: ${JSON.stringify(node.target)}`);
          console.error(`    html: ${node.html}`);
          console.error(`    failure: ${node.failureSummary || "No failure summary returned."}`);
        });
      }
    }
  }
}

let axeSource;
let server;
let chrome;
let cdp;
let pageMode = null;
const findings = [];

try {
  axeSource = loadAxeSource();
  server = startServer();
  const port = await server.ready;
  const base = `http://127.0.0.1:${port}`;
  chrome = launchChrome();
  const browserWs = await chrome.ready;

  for (const scenario of SCENARIOS) {
    if (scenario.mode !== pageMode) {
      if (cdp) {
        assertNoPageErrors(cdp, `${pageMode} page`);
        assertNoUnknownRequests(server, `${pageMode} page`);
        await cdp.close();
      }
      server.setMode(scenario.mode);
      cdp = await openModePage(browserWs, base, scenario.mode, axeSource);
      pageMode = scenario.mode;
    }
    await prepareScenario(cdp, scenario);
    assertNoPageErrors(cdp, scenario.name);
    assertNoUnknownRequests(server, scenario.name);
    const result = await runAxe(cdp, scenario);
    console.log(
      `ui-accessibility: audited ${scenario.name} (${result.violations.length} violation(s), ${result.incomplete.length} incomplete check(s))`,
    );
    if (result.violations.length || result.incomplete.length) {
      findings.push({ scenario, result });
    }
    assertNoPageErrors(cdp, scenario.name);
    assertNoUnknownRequests(server, scenario.name);
  }

  if (findings.length) {
    printFindings(findings);
    process.exitCode = 1;
  } else {
    console.log(`ui-accessibility: passed ${SCENARIOS.length} scenario(s) with axe-core ${AXE_VERSION}`);
  }
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
} finally {
  if (cdp) await cdp.close();
  if (chrome) await chrome.cleanup();
  if (server) await server.close();
}
