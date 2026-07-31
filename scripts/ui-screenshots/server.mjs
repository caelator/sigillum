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
  let treasuryPolicy = { ...D.treasuryPolicy };
  let operations = D.operations.map((operation) => ({ ...operation }));
  let parties = D.parties.map((party) => ({ ...party }));
  let receiveAllocations = D.receiveAllocations.map((allocation) => ({ ...allocation }));
  let receivingOverview = JSON.parse(JSON.stringify(D.receivingOverview));
  let deposits = D.deposits.map((deposit) => ({ ...deposit }));
  const unknownRequests = [];
  const sseClients = new Set();

  const routes = new Map();
  const get = (p, fn) => routes.set("GET " + p, fn);
  const post = (p, fn) => routes.set("POST " + p, fn);
  const status = (value, extra = {}) => ({ status: value, ...extra });
  const firstPlan = () => D.consolidationPlans[0];
  const firstJob = () => D.queueJobs[0];
  const firstDeposit = () => deposits[0];
  const firstOperation = () => operations[0];
  const paginationFor = (url, total) => {
    if (!url?.searchParams.has("limit") && !url?.searchParams.has("offset")) return {};
    const limit = Math.max(0, Number(url.searchParams.get("limit") ?? total));
    const offset = Math.max(0, Number(url.searchParams.get("offset") ?? 0));
    return {
      pagination: {
        total,
        limit,
        offset,
        has_more: offset + limit < total,
      },
    };
  };

  get("/api/status", () => D.statusFor(mode));
  post("/api/unlock", () => D.unlockResponse);
  post("/api/lock", () => ({ status: "locked", message: "All compartments locked." }));
  post("/api/session/revoke", () => ({ status: "revoked", requires_reauth: true }));

  get("/api/profiles/evm", () => ({ profiles: D.evmProfiles }));
  get("/api/profiles/eth-seed", () => ({ profiles: D.seedProfiles }));
  get("/api/profiles/eth-xpub", () => ({ profiles: D.xpubProfiles }));
  get("/api/profiles/eth-stealth", () => ({ profiles: D.stealthProfiles }));

  get("/api/chains", () => ({ profiles: D.chainProfiles }));
  get("/api/inventory/wallets", (_body, url) => ({
    jobs: D.discoveryJobs,
    addresses: D.inventoryAddresses,
    holdings: D.inventoryHoldings,
    nft_metadata_cache: D.nftMetadataCache,
    ...paginationFor(url, D.inventoryAddresses.length),
  }));
  get("/api/discovery/jobs", (_body, url) => ({
    jobs: D.discoveryJobs,
    ...paginationFor(url, D.discoveryJobs.length),
  }));
  get("/api/inventory/watch-addresses", () => ({ entries: D.watchAddresses }));
  get("/api/inventory/token-registry", () => ({ lists: D.tokenRegistryLists }));
  get("/api/inventory/nft-metadata/opt-ins", () => D.nftOptIns);
  get("/api/risk/catalog", () => ({ entries: D.riskCatalog }));
  get("/api/risk/findings", (_body, url) => ({
    findings: D.riskFindings,
    ...paginationFor(url, D.riskFindings.length),
  }));
  get("/api/plans/consolidation", (_body, url) => ({
    plans: D.consolidationPlans,
    ...paginationFor(url, D.consolidationPlans.length),
  }));
  get("/api/operations", () => ({ operations }));

  get("/api/treasury/overview", () => D.treasuryOverview);
  get("/api/treasury/policy", () => ({ policy: treasuryPolicy }));
  get("/api/treasury/receive-addresses", () => ({ allocations: receiveAllocations }));
  get("/api/treasury/parties", () => ({ parties }));

  get("/api/receiving/overview", () => receivingOverview);
  get("/api/deposits/eth-stealth", (_body, url) => ({
    deposits,
    ...paginationFor(url, deposits.length),
  }));
  get("/api/queue/jobs", (_body, url) => ({
    jobs: D.queueJobs,
    ...paginationFor(url, D.queueJobs.length),
  }));

  get("/api/fido2/detect", () => D.fido2Detect);
  get("/api/fido2/status", () => D.fido2Status);
  get("/api/fido2/list", () => D.fido2Keys);

  get("/api/diagnostics", () => D.diagnostics);
  post("/api/selfcheck/run", () => D.selfCheckRun);
  get("/api/audit", () => ({ events: D.auditEvents }));
  get("/api/compartment/list", () => ({ compartments: D.COMPARTMENTS }));
  get("/api/secrets", () => D.secretKeys);
  get("/api/api-keys", () => D.apiKeys);
  post("/api/secrets/get", (body) => ({
    key: body?.key || "key",
    value: "••••••-mock-revealed-value-for-" + (body?.key || "key"),
  }));
  post("/api/api-keys/get", (body) => ({
    key: body?.key || "key",
    value: "mock-key-value-" + (body?.key || "key"),
  }));

  // Interaction endpoints are registered even when the current 12-shot set
  // does not click them. That keeps a future shot or manual CDP exploration
  // from silently succeeding with `{}` and rendering a false success state.
  post("/api/queue/pause", () => {
    treasuryPolicy = { ...treasuryPolicy, execution_paused: true };
    return status("paused", { execution_paused: true });
  });
  post("/api/queue/resume", () => {
    treasuryPolicy = { ...treasuryPolicy, execution_paused: false };
    return status("resumed", { execution_paused: false });
  });
  post("/api/queue/process", (body) => {
    if (body?.run_async) {
      return {
        processed: 0,
        succeeded: 0,
        blocked: 0,
        retrying: 0,
        operator_action_required: 0,
        failed: 0,
        confirmed: 0,
        failures_by_cause: {
          provider_error: 0,
          policy_block: 0,
          insufficient_gas: 0,
          validation: 0,
          unknown: 0,
          on_chain_revert: 0,
          broadcast_rejected: 0,
          receipt_timeout: 0,
        },
        jobs: [],
        operation: firstOperation(),
      };
    }
    return {
      processed: 1,
      succeeded: 1,
      blocked: 0,
      retrying: 0,
      operator_action_required: 0,
      failed: 0,
      confirmed: 0,
      failures_by_cause: {
        provider_error: 0,
        policy_block: 0,
        insufficient_gas: 0,
        validation: 0,
        unknown: 0,
        on_chain_revert: 0,
        broadcast_rejected: 0,
        receipt_timeout: 0,
      },
      jobs: [firstJob()],
    };
  });
  post("/api/treasury/policy/update", (body) => {
    treasuryPolicy = { ...treasuryPolicy, ...(body || {}), updated_at_unix: D.NOW };
    return status("updated", { policy: treasuryPolicy });
  });
  post("/api/plans/consolidation/generate", () =>
    status("generated", { plan: firstPlan(), plans: D.consolidationPlans }),
  );
  post("/api/plans/consolidation/approve", () =>
    status("approved", { plan: firstPlan(), plans: D.consolidationPlans }),
  );
  post("/api/plans/consolidation/simulate", () =>
    status("simulated", { plan: firstPlan(), plans: D.consolidationPlans }),
  );
  post("/api/plans/consolidation/export", (body) => ({
    status: "exported",
    plan_id: body?.plan_id || firstPlan().id,
    format: body?.format || "raw_transactions",
    exported_steps: 2,
    skipped_steps: [],
    bundles: [],
  }));
  post("/api/plans/enqueue-step", (body) =>
    status("queued", {
      plan_id: body?.plan_id || firstPlan().id,
      step_id: body?.step_id || D.planSteps[0].id,
      job: firstJob(),
    }),
  );
  post("/api/plans/enqueue-plan", (body) => {
    const confirmation = `ENQUEUE ${firstPlan().id}`;
    if (body?.confirmation !== confirmation) {
      return {
        code: "typed_confirmation_mismatch",
        error: "Type the confirmation phrase to enqueue this plan.",
        action: confirmation,
      };
    }
    return status("queued", {
      plan_id: firstPlan().id,
      enqueued: [{ step_id: D.planSteps[0].id, job_id: firstJob().id }],
      skipped: [],
    });
  });
  post("/api/maintenance/run", () => ({
    status: "completed",
    refreshed: 3,
    detected: 1,
    queued: 1,
    processed: 1,
    succeeded: 1,
    blocked: 0,
    retrying: 0,
    operator_action_required: 0,
    failed: 0,
    confirmed: 0,
    failures_by_cause: {
      provider_error: 0,
      policy_block: 0,
      insufficient_gas: 0,
      validation: 0,
      unknown: 0,
      on_chain_revert: 0,
      broadcast_rejected: 0,
      receipt_timeout: 0,
    },
    deposits,
    jobs: [firstJob()],
    treasury_automation: { generated_steps: 0, enqueued_steps: 0, skipped_steps: 0 },
  }));

  post("/api/receiving/refresh-balances", () => ({
    generated_at_unix: D.NOW,
    addresses_requested: 5,
    addresses_refreshed: 5,
    addresses_skipped: 0,
    stealth_refreshed: true,
    provider_status: "ok",
    errors: [],
  }));
  post("/api/receiving/deposits/tag", (body) => {
    const index = deposits.findIndex((deposit) => deposit.id === body?.deposit_id);
    if (index < 0) {
      return { code: "not_found", error: "Deposit not found." };
    }
    deposits[index] = {
      ...deposits[index],
      counterparty_id: body?.counterparty_id || null,
      updated_at_unix: D.NOW,
    };
    return status("updated", { deposit: deposits[index], warnings: [] });
  });
  post("/api/treasury/receive-addresses/allocate", () =>
    status("allocated", { allocation: receiveAllocations[0] }),
  );
  post("/api/treasury/receive-addresses/rotate", () =>
    status("rotated", { allocation: receiveAllocations[0] }),
  );
  post("/api/treasury/parties", (body) => {
    const party = {
      ...D.parties[0],
      id: "party-screenshot-new",
      name: body?.name || "New counterparty",
      note: body?.note || null,
      sweep_destination_address: body?.sweep_destination_address || null,
      created_at_unix: D.NOW,
    };
    parties = [...parties.filter((candidate) => candidate.id !== party.id), party];
    return status("created", { party });
  });
  post("/api/treasury/parties/update", (body) => {
    const index = parties.findIndex((party) => party.id === body?.id);
    if (index < 0) {
      return { code: "not_found", error: "Counterparty not found." };
    }
    parties[index] = {
      ...parties[index],
      name: body?.name ?? parties[index].name,
      note: body?.note ?? null,
      sweep_destination_address:
        body?.sweep_destination_address === ""
          ? null
          : body?.sweep_destination_address ?? parties[index].sweep_destination_address,
    };
    return status("updated", { party: parties[index] });
  });
  post("/api/treasury/parties/delete", (body) => {
    const deletedId = body?.id;
    if (!parties.some((party) => party.id === deletedId)) {
      return { code: "not_found", error: "Counterparty not found." };
    }
    parties = parties.filter((party) => party.id !== deletedId);
    receiveAllocations = receiveAllocations.map((allocation) =>
      allocation.counterparty_id === deletedId
        ? { ...allocation, counterparty_id: null }
        : allocation,
    );
    receivingOverview = {
      ...receivingOverview,
      groups: receivingOverview.groups.map((group) => ({
        ...group,
        counterparty: group.counterparty?.id === deletedId ? null : group.counterparty,
        items: group.items.map((item) =>
          item.source_type === "hd" && item.counterparty_id === deletedId
            ? { ...item, counterparty_id: null }
            : item,
        ),
      })),
    };
    return status("deleted", { party: null });
  });
  post("/api/wallets/eth-stealth/export", () => ({
    wallet: "ops-seed",
    short_name: "OPS-S",
    scheme_id: 1,
    stealth_meta_address: D.STEALTH_META_ADDRESS,
    spending_public_key_hex:
      "0x02f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9",
    viewing_public_key_hex:
      "0x03a34b99f22c790c4e36b2b3c2c35a36db06226e41c692fc82b8b56ac1c540c5bd",
  }));
  post("/api/deposits/eth-stealth/enqueue-sweep", () =>
    status("queued", {
      deposit: firstDeposit(),
      job: firstJob(),
      risk_findings: [],
    }),
  );
  post("/api/deposits/eth-stealth/delete", () =>
    status("deleted", { deposit: firstDeposit(), warnings: [] }),
  );
  post("/api/deposits/eth-stealth/refresh", () => ({
    processed: 4,
    detected: 1,
    queued: 1,
    deposits,
  }));
  post("/api/deposits/eth-stealth/create-native", () =>
    status("created", { deposit: firstDeposit(), warnings: [] }),
  );
  post("/api/deposits/eth-stealth/create-erc20", () =>
    status("created", { deposit: deposits[1], warnings: [] }),
  );
  post("/api/deposits/eth-stealth/scan-announcements", () => ({
    status: "completed",
    wallet_profile: "stealth-ops",
    provider_profile: "mainnet-llama",
    from_block: "22970000",
    to_block: "22970412",
    scanned: 24,
    matched: 2,
    created: 1,
    existing: 1,
    deposits,
  }));

  post("/api/inventory/scan/evm", () => ({
    job: D.discoveryJobs[1],
    addresses: [],
    holdings: [],
    operation: firstOperation(),
  }));
  post("/api/discovery/jobs/cancel", () =>
    status("cancel_requested", { job: D.discoveryJobs[1], operation: firstOperation() }),
  );
  post("/api/discovery/jobs/resume", () =>
    status("running", { job: D.discoveryJobs[1], operation: firstOperation() }),
  );
  post("/api/risk/catalog/upsert", () =>
    status("updated", { entry: D.riskCatalog[0] }),
  );
  post("/api/risk/catalog/delete", () =>
    status("deleted", { entry: D.riskCatalog[0] }),
  );
  post("/api/inventory/token-registry/import", () =>
    status("imported", { list: D.tokenRegistryLists[0] }),
  );
  post("/api/inventory/token-registry/delete", () =>
    status("deleted", { list: D.tokenRegistryLists[0] }),
  );
  post("/api/inventory/nft-metadata/opt-ins/upsert", () =>
    status("updated", { opt_in: D.nftOptIns.opt_ins[0] }),
  );
  post("/api/inventory/nft-metadata/opt-ins/delete", () =>
    status("deleted", { opt_in: D.nftOptIns.opt_ins[0] }),
  );
  post("/api/inventory/nft-metadata/settings", (body) =>
    status("updated", { ipfs_gateway_url: body?.ipfs_gateway_url || null }),
  );
  post("/api/inventory/nft-metadata/fetch", () => ({
    fetched: D.nftMetadataCache.length,
    skipped: [],
    entries: D.nftMetadataCache,
  }));

  post("/api/compartment/switch", (body) => {
    const compartment =
      D.COMPARTMENTS.find((candidate) => candidate.id === body?.id) || D.COMPARTMENTS[0];
    return status("switched", {
      compartment_id: compartment.id,
      compartment_label: compartment.label,
    });
  });
  post("/api/compartment/add", (body) =>
    status("created", {
      id: 3,
      label: body?.label || "new vault",
      threshold: body?.threshold || 2,
    }),
  );
  for (const prefix of ["/api/secrets", "/api/api-keys"]) {
    post(`${prefix}/set`, (body) => status("stored", { key: body?.key || "key" }));
    post(`${prefix}/delete`, (body) => status("deleted", { key: body?.key || "key" }));
  }
  post("/api/secrets/push", (body) =>
    status("pushed", {
      from: body?.from ?? 1,
      to: body?.to ?? 2,
      key: body?.key || "ops_seed_mnemonic",
    }),
  );
  post("/api/fido2/pin/set", () => status("pin_set"));
  post("/api/fido2/register", (body) =>
    status("registered", {
      label: body?.label || "Screenshot security key",
      total_keys: D.fido2Keys.keys.length + 1,
      poison: Boolean(body?.poison),
    }),
  );
  post("/api/fido2/remove", (body) =>
    status("removed", { label: body?.label || D.fido2Keys.keys[0].label }),
  );
  post("/api/backup/export", () => ({
    status: "exported",
    snapshot_hex: "0x7b226d6f636b223a747275657d",
    summary: { created_at_unix: D.NOW, file_count: 7, total_bytes: 4096 },
  }));
  post("/api/backup/restore", () =>
    status("restored", {
      summary: { created_at_unix: D.NOW - 86_400, file_count: 7, total_bytes: 4096 },
      requires_reauth: true,
    }),
  );
  post("/api/setup/reset", () =>
    status("reset", { archived_to: "/mock/sigillum-archive-20260717" }),
  );

  const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    const sendJson = (code, obj) => {
      res.writeHead(code, { "Content-Type": "application/json", "Cache-Control": "no-store" });
      res.end(JSON.stringify(obj));
    };
    const sendUnknownRoute = () => {
      const request = { method: req.method, path: url.pathname };
      unknownRequests.push(request);
      sendJson(404, {
        code: "not_found",
        error: `Screenshot mock has no registered route: ${request.method} ${request.path}`,
      });
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

    if (url.pathname === "/api/events" && req.method === "GET") {
      res.writeHead(200, {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-store",
        Connection: "keep-alive",
      });
      res.write(
        "event: snapshot\n" +
          "data: " +
          JSON.stringify({
            v: 1,
            locked: mode !== "unlocked",
            active_compartment_id: mode === "unlocked" ? 1 : undefined,
            operations:
              mode === "unlocked"
                ? operations.filter((operation) =>
                    ["running", "cancel_requested"].includes(operation.state),
                  )
                : [],
          }) +
          "\n\n",
      );
      sseClients.add(res);
      req.once("close", () => sseClients.delete(res));
      return;
    }

    if (url.pathname.startsWith("/api/")) {
      let handler = routes.get(req.method + " " + url.pathname);
      if (!handler) {
        const match = url.pathname.match(/^\/api\/operations\/([^/]+)(\/cancel)?$/);
        if (match) {
          const operationId = decodeURIComponent(match[1]);
          const operationIndex = operations.findIndex((operation) => operation.id === operationId);
          if (operationIndex < 0) {
            sendJson(404, {
              code: "not_found",
              error: `Operation not found: ${operationId}`,
            });
            return;
          }
          if (req.method === "GET" && !match[2]) {
            handler = () => ({ operation: operations[operationIndex] });
          } else if (req.method === "POST" && match[2]) {
            handler = () => {
              const current = operations[operationIndex];
              const operation = {
                ...current,
                state: "cancel_requested",
                updated_at_unix: D.NOW,
              };
              operations[operationIndex] = operation;
              return status("cancel_requested", { operation });
            };
          }
        }
      }
      if (!handler) {
        sendUnknownRoute();
        return;
      }
      if (req.method === "POST") {
        let body = "";
        req.on("data", (chunk) => (body += chunk));
        req.on("end", () => {
          let parsed = null;
          try {
            parsed = JSON.parse(body || "null");
          } catch (_) {
            sendJson(400, { code: "bad_request", error: "Request body is not valid JSON." });
            return;
          }
          sendJson(200, handler(parsed, url));
        });
      } else {
        sendJson(200, handler(null, url));
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
    getUnknownRequests() {
      return unknownRequests.map((request) => ({ ...request }));
    },
    async close() {
      for (const response of sseClients) response.end();
      sseClients.clear();
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
