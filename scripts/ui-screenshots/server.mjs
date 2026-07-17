#!/usr/bin/env node
// Zero-dependency mock daemon for the UI screenshot harness.
//
// Serves the operator console page assembled EXACTLY like
// crates/sigillum-daemon/src/ui.rs render_index_html — fragments from
// crates/sigillum-daemon/ui/src plus the checked-in vite bundles
// (src/app.js, src/styles.css) — and answers every /api/* route the UI
// calls with the canned, populated state in mock-data.mjs.
//
// The daemon injects a per-request CSP nonce into the script tag; for local
// screenshot purposes the nonce is dropped (no CSP header is sent either),
// which changes nothing about what the page renders.
//
// Run standalone:  node scripts/ui-screenshots/server.mjs [port]
// (port 0 = pick an ephemeral port; the URL is printed on stdout)
// In-process:      import { startServer } from "./server.mjs"  (drive.mjs)
// Control:         POST /__mode  {"mode":"setup"|"locked"|"unlocked"}

import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import * as D from "./mock-data.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, "../..");
const UI_SRC = path.join(REPO_ROOT, "crates/sigillum-daemon/ui/src");
const BUILD_HINT =
  "run `npm run build` in crates/sigillum-daemon/ui to regenerate the checked-in bundles";

function fail(message) {
  throw new Error(`ui-screenshots: ${message}`);
}

// The harness never builds the UI itself: missing or stale bundles mean the
// shots would not reflect the authored source, so refuse early with a clear
// remediation instead of rendering something misleading.
export function checkBundles() {
  const appJs = path.join(UI_SRC, "app.js");
  const stylesCss = path.join(UI_SRC, "styles.css");
  for (const bundle of [appJs, stylesCss]) {
    if (!fs.existsSync(bundle) || fs.statSync(bundle).size === 0) {
      fail(`${path.basename(bundle)} is missing or empty — ${BUILD_HINT}.`);
    }
  }
  const authored = [];
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (/\.(ts|css|html)$/.test(entry.name)) authored.push(full);
    }
  };
  walk(UI_SRC);
  const bundles = new Set([appJs, stylesCss]);
  const newestSource = Math.max(
    ...authored
      .filter((f) => !bundles.has(f))
      // Type-only modules emit no JavaScript, so editing them can never make
      // the bundles stale: contracts.ts (wire types) and *.d.ts (vite env).
      .filter((f) => !f.endsWith(".d.ts") && path.basename(f) !== "contracts.ts")
      .map((f) => fs.statSync(f).mtimeMs),
  );
  for (const bundle of [appJs, stylesCss]) {
    if (newestSource > fs.statSync(bundle).mtimeMs) {
      fail(
        `${path.relative(REPO_ROOT, bundle)} is older than the authored UI source — ${BUILD_HINT}.`,
      );
    }
  }
  return { appJs, stylesCss };
}

// Mirror of ui.rs: BEFORE_STYLE + <style>styles.css</style> + AFTER_STYLE
// + <script nonce>app.js</script> + AFTER_SCRIPT (nonce dropped, see header).
export function assembleIndex() {
  const { appJs, stylesCss } = checkBundles();
  const read = (name) => fs.readFileSync(path.join(UI_SRC, name), "utf8");
  return (
    read("index.before-style.html") +
    "<style>" +
    fs.readFileSync(stylesCss, "utf8") +
    "</style>" +
    read("index.after-style-before-script.html") +
    "<script>" +
    fs.readFileSync(appJs, "utf8") +
    "</script>" +
    read("index.after-script.html")
  );
}

export function startServer({ port = 0 } = {}) {
  const index = assembleIndex();
  let mode = "unlocked";

  const routes = new Map();
  const get = (p, fn) => routes.set("GET " + p, fn);
  const post = (p, fn) => routes.set("POST " + p, fn);

  get("/api/status", () => D.statusFor(mode));
  post("/api/unlock", () => D.unlockResponse);
  post("/api/lock", () => ({ status: "locked", message: "All compartments locked." }));
  post("/api/session/revoke", () => ({ status: "revoked", requires_reauth: true }));

  get("/api/profiles/evm", () => ({ profiles: D.evmProfiles }));
  get("/api/profiles/eth-seed", () => ({ profiles: D.seedProfiles }));
  get("/api/profiles/eth-xpub", () => ({ profiles: D.xpubProfiles }));
  get("/api/profiles/eth-stealth", () => ({ profiles: D.stealthProfiles }));

  get("/api/chains", () => ({ profiles: D.chainProfiles }));
  get("/api/inventory/wallets", () => ({ jobs: D.discoveryJobs, addresses: D.inventoryAddresses }));
  get("/api/inventory/watch-addresses", () => ({ entries: D.watchAddresses }));
  get("/api/inventory/token-registry", () => ({ lists: D.tokenRegistryLists }));
  get("/api/inventory/nft-metadata/opt-ins", () => D.nftOptIns);
  get("/api/risk/catalog", () => ({ entries: D.riskCatalog }));
  get("/api/risk/findings", () => ({ findings: D.riskFindings }));
  get("/api/plans/consolidation", () => ({ plans: D.consolidationPlans }));

  get("/api/treasury/overview", () => D.treasuryOverview);
  get("/api/treasury/policy", () => ({ policy: D.treasuryPolicy }));
  get("/api/treasury/receive-addresses", () => ({ allocations: D.receiveAllocations }));
  get("/api/treasury/parties", () => ({ parties: D.parties }));

  get("/api/receiving/overview", () => D.receivingOverview);
  get("/api/deposits/eth-stealth", () => ({ deposits: D.deposits }));
  get("/api/queue/jobs", () => ({ jobs: D.queueJobs }));

  get("/api/fido2/detect", () => D.fido2Detect);
  get("/api/fido2/status", () => D.fido2Status);
  get("/api/fido2/list", () => D.fido2Keys);

  get("/api/diagnostics", () => D.diagnostics);
  post("/api/selfcheck/run", () => D.selfCheckRun);
  get("/api/audit", () => ({ events: D.auditEvents }));
  get("/api/compartment/list", () => ({ compartments: D.COMPARTMENTS }));
  get("/api/secrets", () => D.secretKeys);
  get("/api/api-keys", () => D.apiKeys);
  post("/api/secrets/get", (body) => ({ value: "••••••-mock-revealed-value-for-" + (body?.key || "key") }));
  post("/api/api-keys/get", (body) => ({ value: "mock-key-value-" + (body?.key || "key") }));

  const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    const sendJson = (code, obj) => {
      res.writeHead(code, { "Content-Type": "application/json", "Cache-Control": "no-store" });
      res.end(JSON.stringify(obj));
    };

    if (url.pathname === "/__mode" && req.method === "POST") {
      let body = "";
      req.on("data", (chunk) => (body += chunk));
      req.on("end", () => {
        try {
          mode = JSON.parse(body).mode || mode;
        } catch (_) {}
        sendJson(200, { mode });
      });
      return;
    }

    if (url.pathname === "/" || url.pathname === "/index.html") {
      res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
      res.end(index);
      return;
    }

    if (url.pathname.startsWith("/api/")) {
      const handler = routes.get(req.method + " " + url.pathname);
      if (req.method === "POST") {
        let body = "";
        req.on("data", (chunk) => (body += chunk));
        req.on("end", () => {
          let parsed = null;
          try {
            parsed = JSON.parse(body || "null");
          } catch (_) {}
          // Unknown POST routes answer an empty object: form submissions in
          // the shots flow should render their success path, not an error.
          sendJson(200, handler ? handler(parsed) : {});
        });
      } else {
        sendJson(200, handler ? handler(null) : {});
      }
      return;
    }

    res.writeHead(404, { "Content-Type": "text/plain" });
    res.end("not found");
  });

  const ready = new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", () => resolve(server.address().port));
  });

  return {
    ready,
    setMode(next) {
      mode = next;
    },
    async close() {
      await new Promise((resolve) => server.close(resolve));
    },
  };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const daemon = startServer({ port: Number(process.argv[2] || 0) });
  daemon.ready.then((port) => {
    console.log(`mock daemon on http://127.0.0.1:${port}`);
  });
}
