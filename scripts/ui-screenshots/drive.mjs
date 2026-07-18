#!/usr/bin/env node
// Screenshot driver for the Sigillum operator console.
//
// Starts the in-process mock daemon (server.mjs) serving the REAL assembled
// UI (fragments + checked-in vite bundles), drives headless Chrome over the
// raw DevTools protocol — same zero-dependency approach as
// scripts/browser-smoke.mjs — and captures the shot set below.
//
// Usage:
//   node scripts/ui-screenshots/drive.mjs [--out=<dir>] [--width=1440] [--height=900] [--scale=2]
//
// Configuration (argv wins over env):
//   --out     / SIGILLUM_UI_SHOTS_DIR    output directory
//                                        (default: target/ui-screenshots/ in the repo)
//   --width   / SIGILLUM_UI_SHOTS_WIDTH  viewport width  (default: 1440)
//   --height  / SIGILLUM_UI_SHOTS_HEIGHT viewport height (default: 900)
//   --scale   / SIGILLUM_UI_SHOTS_SCALE  deviceScaleFactor (default: 2)
//   CHROME_BIN / GOOGLE_CHROME_BIN       browser executable override
//
// Prerequisites: built UI bundles (`npm run build` in
// crates/sigillum-daemon/ui) and a Chrome/Chromium executable. The harness
// never runs builds itself — server.mjs refuses on missing/stale bundles.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { fileURLToPath } from "node:url";
import { startServer } from "./server.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, "../..");
const TIMEOUT_MS = 30_000;

function argValue(name) {
  const prefix = `--${name}=`;
  const hit = process.argv.slice(2).find((a) => a.startsWith(prefix));
  return hit ? hit.slice(prefix.length) : undefined;
}

const OUT_DIR = path.resolve(
  argValue("out") ||
    process.env.SIGILLUM_UI_SHOTS_DIR ||
    path.join(REPO_ROOT, "target/ui-screenshots"),
);
const VIEWPORT_WIDTH = Number(argValue("width") || process.env.SIGILLUM_UI_SHOTS_WIDTH || 1440);
const VIEWPORT_HEIGHT = Number(argValue("height") || process.env.SIGILLUM_UI_SHOTS_HEIGHT || 900);
const DEVICE_SCALE = Number(argValue("scale") || process.env.SIGILLUM_UI_SHOTS_SCALE || 2);

// ── The shot set ────────────────────────────────────────────────────────────
// This list IS the harness output contract: add an entry here to add a shot.
//   name     output file "<name>.png" in the output directory
//   mode     daemon mode for the page: "setup" | "locked" | "unlocked"
//   section  workspace destination to open (nav click) before shooting
//   click    optional selector to click before shooting (wizard flows)
//   waitFor  optional extra expression that must become true before shooting
//   scrollTo optional CSS selector scrolled below the sticky topbar
//   fullPage capture the whole scroll height instead of the viewport
const SHOTS = [
  { name: "setup-welcome", mode: "setup" },
  {
    name: "setup-protection-model",
    mode: "setup",
    click: '[data-action="wizGetStarted"]',
    waitFor: "document.getElementById('wizStep0')?.classList.contains('active')",
  },
  { name: "unlock", mode: "locked" },
  { name: "section-overview", mode: "unlocked", section: "overview" },
  { name: "section-receive", mode: "unlocked", section: "receive" },
  // Card-level shots keep populated surfaces visible that a top-of-section
  // viewport shot would leave below the fold.
  {
    name: "section-receive-deposits",
    mode: "unlocked",
    section: "receive",
    scrollTo: '[data-section="deposits"]',
  },
  { name: "section-portfolio", mode: "unlocked", section: "portfolio" },
  { name: "section-move", mode: "unlocked", section: "move" },
  { name: "section-move-plans", mode: "unlocked", section: "move", scrollTo: "#plansCard" },
  { name: "section-move-queue", mode: "unlocked", section: "move", scrollTo: "#queueCard" },
  { name: "section-vault", mode: "unlocked", section: "vault" },
  {
    name: "section-vault-diagnostics",
    mode: "unlocked",
    section: "vault",
    scrollTo: '[data-vault="diagnostics"]',
  },
];

function fail(message) {
  throw new Error(`ui-screenshots: ${message}`);
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
  const profileDir = fs.mkdtempSync(path.join(os.tmpdir(), "sigillum-ui-shots-profile."));
  const child = spawn(
    executable,
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
    ready,
    async cleanup() {
      if (child.exitCode === null && child.signalCode === null) {
        child.kill("SIGTERM");
      }
      if (child.exitCode === null && child.signalCode === null) {
        await Promise.race([once(child, "exit").catch(() => {}), sleep(2_000)]);
      }
      if (child.exitCode === null && child.signalCode === null) {
        child.kill("SIGKILL");
        await Promise.race([once(child, "exit").catch(() => {}), sleep(1_000)]);
      }
      // Chromium helper processes can outlive the killed main process and
      // keep writing to the profile dir: retry with backoff, then warn.
      const rmOptions = { recursive: true, force: true, maxRetries: 10, retryDelay: 200 };
      try {
        fs.rmSync(profileDir, rmOptions);
      } catch {
        await sleep(1_000);
        try {
          fs.rmSync(profileDir, rmOptions);
        } catch (retryError) {
          console.warn(`ui-screenshots: leaving temp profile dir behind at ${profileDir}: ${retryError.message}`);
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
    this.consoleErrors = [];
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
      this.consoleErrors.push(
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
      this.consoleErrors.push("exception: " + reason + location);
    }
  }

  async send(method, params = {}) {
    await this.opened;
    const id = this.nextId++;
    const result = new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
    });
    this.webSocket.send(JSON.stringify({ id, method, params }));
    return result;
  }

  takeErrors() {
    return this.consoleErrors.splice(0);
  }

  async close() {
    this.webSocket.close();
    if (!this.targetId) return;
    try {
      await fetch(`${this.debugOrigin}/json/close/${encodeURIComponent(this.targetId)}`);
    } catch (_) {
      // Chrome may already be exiting; cleanup() remains authoritative.
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

async function shot(cdp, name, { fullPage = false } = {}) {
  const params = { format: "png", fromSurface: true };
  if (fullPage) {
    const metrics = await cdp.send("Page.getLayoutMetrics");
    const size = metrics.cssContentSize || metrics.contentSize;
    params.captureBeyondViewport = true;
    params.clip = { x: 0, y: 0, width: Math.ceil(size.width), height: Math.ceil(size.height), scale: 1 };
  }
  const result = await cdp.send("Page.captureScreenshot", params);
  const file = path.join(OUT_DIR, name + ".png");
  fs.writeFileSync(file, Buffer.from(result.data, "base64"));
  console.log("wrote", file);
}

// One fresh page per daemon mode. Unlocked pages get the session token the
// real unlock flow would have stored (the mock daemon accepts any bearer).
async function openModePage(browserWs, base, mode) {
  const cdp = await openPage(browserWs);
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    width: VIEWPORT_WIDTH,
    height: VIEWPORT_HEIGHT,
    deviceScaleFactor: DEVICE_SCALE,
    mobile: false,
  });
  if (mode === "unlocked") {
    await cdp.send("Page.addScriptToEvaluateOnNewDocument", {
      source: "try{sessionStorage.setItem('sigillumSessionToken','ui-shots-session-token')}catch(e){}",
    });
  }
  await cdp.send("Page.navigate", { url: base + "/" });
  await waitFor(cdp, "document.readyState === 'complete'", `${mode} page load`);
  await waitFor(cdp, `document.body.dataset.mode === '${mode}'`, `${mode} mode`, 20_000);
  return cdp;
}

async function selectWorkspaceSection(cdp, section) {
  const selector = `[data-action="selectWorkspaceSection"][data-arg0="${section}"]`;
  await waitFor(cdp, `document.querySelector(${quoted(selector)})`, `workspace ${section} nav item`);
  await evaluate(
    cdp,
    `(() => {
      const el = document.querySelector(${quoted(selector)});
      if (!el) throw new Error("missing element");
      el.click();
      window.scrollTo(0, 0);
      return true;
    })()`,
    `click workspace ${section}`,
  );
  await waitFor(
    cdp,
    `document.querySelector(${quoted(selector)})?.classList.contains("active") === true`,
    `workspace ${section} active`,
  );
  await waitFor(
    cdp,
    `Array.from(document.querySelectorAll(".skeleton")).every((el) => el.offsetParent === null)`,
    `workspace ${section} populated`,
    20_000,
  );
}

function assertNoPageErrors(cdp, context) {
  const errors = cdp?.takeErrors() || [];
  if (!errors.length) return;
  fail(`${context} produced browser errors:\n${errors.map((error) => "  " + error).join("\n")}`);
}

function assertNoUnknownRequests(server, context) {
  const unknown = server?.getUnknownRequests?.() || [];
  if (!unknown.length) return;
  fail(
    `${context} called unregistered mock routes:\n` +
      unknown.map((request) => `  ${request.method} ${request.path}`).join("\n"),
  );
}

async function assertNoHorizontalDocumentOverflow(cdp, context) {
  const layout = await evaluate(
    cdp,
    `(() => {
      const root = document.documentElement;
      const clientWidth = root.clientWidth;
      const scrollWidth = root.scrollWidth;
      const measured = Array.from(document.body.querySelectorAll("*"))
        .map((element) => {
          const rect = element.getBoundingClientRect();
          return {
            selector: element.id
              ? "#" + element.id
              : element.tagName.toLowerCase() +
                (element.classList.length
                  ? "." + Array.from(element.classList).slice(0, 3).join(".")
                  : ""),
            left: Math.floor(rect.left),
            right: Math.ceil(rect.right),
            width: Math.ceil(rect.width),
            height: Math.ceil(rect.height),
            scrollWidth: element.scrollWidth,
            clientWidth: element.clientWidth,
          };
        })
        .filter((item) => item.width > 0 && item.height > 0);
      const leftClipped = measured
        .filter((item) => item.left < -1)
        .sort((a, b) => a.left - b.left)
        .slice(0, 5);
      const offenders = measured
        .filter(
          (item) =>
            item.left < -1 ||
            item.right > clientWidth + 1 ||
            item.scrollWidth > item.clientWidth + 1,
        )
        .sort((a, b) => Math.max(b.right, b.scrollWidth) - Math.max(a.right, a.scrollWidth))
        .slice(0, 5);
      return { clientWidth, scrollWidth, leftClipped, offenders };
    })()`,
    `${context} horizontal-overflow check`,
  );
  if (layout.scrollWidth <= layout.clientWidth && !layout.leftClipped.length) return;
  const detail = layout.offenders
    .map(
      (item) =>
        `${item.selector} (left=${item.left}, right=${item.right}, ` +
        `scroll=${item.scrollWidth}, client=${item.clientWidth})`,
    )
    .join("; ");
  fail(
    `${context} has horizontal clipping or document overflow: scrollWidth=${layout.scrollWidth}, ` +
      `clientWidth=${layout.clientWidth}${detail ? `; likely offenders: ${detail}` : ""}`,
  );
}

let server;
let chrome;
let cdp;
try {
  fs.mkdirSync(OUT_DIR, { recursive: true });
  // Remove only this harness's contract files. A failed rerun must not leave
  // old PNGs behind that can be mistaken for current evidence.
  for (const entry of SHOTS) {
    fs.rmSync(path.join(OUT_DIR, entry.name + ".png"), { force: true });
  }
  server = startServer();
  const port = await server.ready;
  const base = `http://127.0.0.1:${port}`;
  chrome = launchChrome();
  const browserWs = await chrome.ready;

  let pageMode = null;
  for (const entry of SHOTS) {
    if (entry.mode !== pageMode) {
      if (cdp) {
        assertNoPageErrors(cdp, `${pageMode} page`);
        assertNoUnknownRequests(server, `${pageMode} page`);
        await cdp.close();
      }
      server.setMode(entry.mode);
      cdp = await openModePage(browserWs, base, entry.mode);
      pageMode = entry.mode;
      if (entry.mode === "unlocked") {
        // Let the parallel load*() fan-out (and the ambient self-check that
        // fills the topbar status strip) settle before the first shot.
        await sleep(2_500);
      } else {
        await sleep(800);
      }
    }
    if (entry.section) {
      await selectWorkspaceSection(cdp, entry.section);
      await sleep(600);
    }
    if (entry.click) {
      await evaluate(
        cdp,
        `(() => {
          const el = document.querySelector(${quoted(entry.click)});
          if (!el) throw new Error("missing element");
          el.click();
          return true;
        })()`,
        `click ${entry.click}`,
      );
      await sleep(500);
    }
    if (entry.waitFor) {
      await waitFor(cdp, entry.waitFor, `${entry.name} precondition`);
    }
    if (entry.scrollTo) {
      const scrolled = await evaluate(
        cdp,
        `(() => {
          const found = document.querySelector(${quoted(entry.scrollTo)});
          const el = found?.closest("section") || found;
          if (!el) return false;
          el.scrollIntoView({ block: "start" });
          const topbar = document.querySelector(".topbar");
          window.scrollBy(0, -((topbar?.getBoundingClientRect().height || 0) + 16));
          const rect = el.getBoundingClientRect();
          return el.isConnected && rect.height > 0 && rect.bottom > 0 && rect.top < window.innerHeight;
        })()`,
        `scroll to ${entry.scrollTo}`,
      );
      if (!scrolled) {
        fail(`${entry.name} scroll target ${entry.scrollTo} was not connected and visible`);
      }
      await sleep(400);
    }
    await assertNoHorizontalDocumentOverflow(cdp, entry.name);
    await shot(cdp, entry.name, { fullPage: !!entry.fullPage });
    await sleep(100);
    assertNoPageErrors(cdp, entry.name);
    assertNoUnknownRequests(server, entry.name);
  }

  for (const entry of SHOTS) {
    const file = path.join(OUT_DIR, entry.name + ".png");
    if (!fs.existsSync(file) || fs.statSync(file).size === 0) {
      fail(`missing or empty screenshot: ${file}`);
    }
  }
  console.log(`ui-screenshots: ${SHOTS.length} shot(s) in ${OUT_DIR}`);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
} finally {
  if (cdp) await cdp.close();
  if (chrome) await chrome.cleanup();
  if (server) await server.close();
}
