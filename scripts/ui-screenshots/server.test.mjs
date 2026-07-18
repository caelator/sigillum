import assert from "node:assert/strict";
import test from "node:test";

import { startServer } from "./server.mjs";

async function withServer(run) {
  const daemon = startServer();
  const port = await daemon.ready;
  try {
    await run(daemon, `http://127.0.0.1:${port}`);
  } finally {
    await daemon.close();
  }
}

async function requestJson(base, contract) {
  const response = await fetch(base + contract.path, {
    method: contract.method,
    headers: contract.method === "POST" ? { "Content-Type": "application/json" } : undefined,
    body: contract.method === "POST" ? JSON.stringify(contract.body ?? {}) : undefined,
  });
  const responseText = await response.text();
  assert.equal(
    response.status,
    200,
    `${contract.method} ${contract.path} returned HTTP ${response.status}: ${responseText}`,
  );
  assert.match(response.headers.get("content-type") || "", /^application\/json\b/);
  const payload = JSON.parse(responseText);
  for (const key of contract.keys ?? []) {
    assert.ok(Object.hasOwn(payload, key), `${contract.method} ${contract.path} omitted ${key}`);
  }
  for (const key of contract.arrays ?? []) {
    assert.ok(Array.isArray(payload[key]), `${contract.method} ${contract.path} ${key} is not an array`);
  }
  for (const [collection, fields] of Object.entries(contract.itemKeys ?? {})) {
    assert.ok(payload[collection]?.length, `${contract.method} ${contract.path} ${collection} is empty`);
    for (const field of fields) {
      assert.ok(
        Object.hasOwn(payload[collection][0], field),
        `${contract.method} ${contract.path} ${collection}[0] omitted ${field}`,
      );
    }
  }
  return payload;
}

test("every destination read route returns its real top-level envelope", async () => {
  await withServer(async (_daemon, base) => {
    const contracts = [
      { method: "GET", path: "/api/status", keys: ["initialized", "locked"] },
      { method: "GET", path: "/api/profiles/evm", arrays: ["profiles"] },
      { method: "GET", path: "/api/profiles/eth-seed", arrays: ["profiles"] },
      { method: "GET", path: "/api/profiles/eth-xpub", arrays: ["profiles"] },
      { method: "GET", path: "/api/profiles/eth-stealth", arrays: ["profiles"] },
      {
        method: "GET",
        path: "/api/chains",
        arrays: ["profiles"],
        itemKeys: { profiles: ["provider_profile", "created_at_unix"] },
      },
      {
        method: "GET",
        path: "/api/inventory/wallets?limit=50&sort=activity&order=desc",
        keys: ["pagination"],
        arrays: ["jobs", "addresses", "holdings", "nft_metadata_cache"],
      },
      { method: "GET", path: "/api/discovery/jobs", arrays: ["jobs"] },
      { method: "GET", path: "/api/inventory/watch-addresses", arrays: ["entries"] },
      { method: "GET", path: "/api/inventory/token-registry", arrays: ["lists"] },
      { method: "GET", path: "/api/inventory/nft-metadata/opt-ins", arrays: ["opt_ins"] },
      { method: "GET", path: "/api/risk/catalog", arrays: ["entries"] },
      {
        method: "GET",
        path: "/api/risk/findings?severity=high&limit=20",
        keys: ["pagination"],
        arrays: ["findings"],
        itemKeys: {
          findings: [
            "risk_level",
            "status",
            "wallet_family",
            "wallet_profile",
            "provider_profile",
            "chain_id",
            "address",
            "subject_type",
            "subject",
            "recommendation",
            "first_seen_at_unix",
            "last_checked_at_unix",
          ],
        },
      },
      {
        method: "GET",
        path: "/api/plans/consolidation?limit=20",
        keys: ["pagination"],
        arrays: ["plans"],
      },
      { method: "GET", path: "/api/operations", arrays: ["operations"] },
      {
        method: "GET",
        path: "/api/operations/op-inventory-20260716-02",
        keys: ["operation"],
      },
      { method: "GET", path: "/api/treasury/overview", keys: ["groups", "risk", "plans"] },
      { method: "GET", path: "/api/treasury/policy", keys: ["policy"] },
      { method: "GET", path: "/api/treasury/receive-addresses", arrays: ["allocations"] },
      { method: "GET", path: "/api/treasury/parties", arrays: ["parties"] },
      { method: "GET", path: "/api/receiving/overview?include_retired=false", arrays: ["groups"] },
      {
        method: "GET",
        path: "/api/deposits/eth-stealth?limit=50",
        keys: ["pagination"],
        arrays: ["deposits"],
        itemKeys: {
          deposits: [
            "chain_id",
            "chain_id_assumed",
            "wallet_compartment_id",
            "provider_compartment_id",
            "wallet",
            "stealth_meta_address",
            "stealth_hash_convention",
          ],
        },
      },
      {
        method: "GET",
        path: "/api/queue/jobs?limit=50",
        keys: ["pagination"],
        arrays: ["jobs"],
      },
      { method: "GET", path: "/api/fido2/detect", keys: ["device_present", "device_count"] },
      { method: "GET", path: "/api/fido2/status", keys: ["enabled", "key_count"] },
      { method: "GET", path: "/api/fido2/list", arrays: ["keys"] },
      { method: "GET", path: "/api/diagnostics", keys: ["pending_operation_count"] },
      { method: "GET", path: "/api/audit?kind=snapshot.export&limit=1", arrays: ["events"] },
      { method: "GET", path: "/api/compartment/list", arrays: ["compartments"] },
      { method: "GET", path: "/api/secrets", arrays: ["keys"] },
      { method: "GET", path: "/api/api-keys", arrays: ["keys"] },
    ];

    for (const contract of contracts) await requestJson(base, contract);
  });
});

test("every destination mutation route returns a meaningful envelope", async () => {
  await withServer(async (_daemon, base) => {
    const contracts = [
      { method: "POST", path: "/api/unlock", keys: ["status", "session_token"] },
      { method: "POST", path: "/api/lock", keys: ["status"] },
      { method: "POST", path: "/api/session/revoke", keys: ["status"] },
      {
        method: "POST",
        path: "/api/operations/op-inventory-20260716-02/cancel",
        keys: ["status", "operation"],
      },
      { method: "POST", path: "/api/queue/pause", keys: ["status", "execution_paused"] },
      { method: "POST", path: "/api/queue/resume", keys: ["status", "execution_paused"] },
      {
        method: "POST",
        path: "/api/queue/process",
        keys: ["processed", "failures_by_cause", "jobs"],
      },
      {
        method: "POST",
        path: "/api/queue/process",
        body: { run_async: true },
        keys: ["processed", "failures_by_cause", "jobs", "operation"],
      },
      { method: "POST", path: "/api/treasury/policy/update", keys: ["status", "policy"] },
      { method: "POST", path: "/api/plans/consolidation/generate", keys: ["status", "plan"] },
      { method: "POST", path: "/api/plans/consolidation/approve", keys: ["status", "plan"] },
      { method: "POST", path: "/api/plans/consolidation/simulate", keys: ["status", "plan"] },
      { method: "POST", path: "/api/plans/consolidation/export", keys: ["status", "bundles"] },
      { method: "POST", path: "/api/plans/enqueue-step", keys: ["status", "job"] },
      {
        method: "POST",
        path: "/api/plans/enqueue-plan",
        body: { confirmation: "ENQUEUE plan-20260715-a1" },
        keys: ["status", "enqueued", "skipped"],
      },
      {
        method: "POST",
        path: "/api/maintenance/run",
        keys: ["status", "processed", "failures_by_cause", "deposits", "jobs"],
      },
      {
        method: "POST",
        path: "/api/receiving/refresh-balances",
        keys: ["provider_status", "addresses_refreshed", "errors"],
      },
      {
        method: "POST",
        path: "/api/receiving/deposits/tag",
        keys: ["status", "deposit", "warnings"],
      },
      { method: "POST", path: "/api/treasury/receive-addresses/allocate", keys: ["allocation"] },
      { method: "POST", path: "/api/treasury/receive-addresses/rotate", keys: ["allocation"] },
      { method: "POST", path: "/api/treasury/parties", body: { name: "Test" }, keys: ["party"] },
      {
        method: "POST",
        path: "/api/wallets/eth-stealth/export",
        keys: [
          "wallet",
          "short_name",
          "scheme_id",
          "stealth_meta_address",
          "spending_public_key_hex",
          "viewing_public_key_hex",
        ],
      },
      {
        method: "POST",
        path: "/api/deposits/eth-stealth/enqueue-sweep",
        keys: ["status", "deposit", "job", "risk_findings"],
      },
      {
        method: "POST",
        path: "/api/deposits/eth-stealth/delete",
        keys: ["status", "deposit", "warnings"],
      },
      {
        method: "POST",
        path: "/api/deposits/eth-stealth/refresh",
        keys: ["processed", "detected", "queued", "deposits"],
      },
      { method: "POST", path: "/api/deposits/eth-stealth/create-native", keys: ["deposit", "warnings"] },
      { method: "POST", path: "/api/deposits/eth-stealth/create-erc20", keys: ["deposit", "warnings"] },
      {
        method: "POST",
        path: "/api/deposits/eth-stealth/scan-announcements",
        keys: [
          "status",
          "wallet_profile",
          "provider_profile",
          "from_block",
          "to_block",
          "scanned",
          "matched",
          "created",
          "existing",
          "deposits",
        ],
      },
      { method: "POST", path: "/api/inventory/scan/evm", keys: ["job", "addresses", "holdings"] },
      { method: "POST", path: "/api/discovery/jobs/cancel", keys: ["status", "job"] },
      { method: "POST", path: "/api/discovery/jobs/resume", keys: ["status", "job"] },
      { method: "POST", path: "/api/risk/catalog/upsert", keys: ["status", "entry"] },
      { method: "POST", path: "/api/risk/catalog/delete", keys: ["status", "entry"] },
      { method: "POST", path: "/api/inventory/token-registry/import", keys: ["status", "list"] },
      { method: "POST", path: "/api/inventory/token-registry/delete", keys: ["status", "list"] },
      { method: "POST", path: "/api/inventory/nft-metadata/opt-ins/upsert", keys: ["opt_in"] },
      { method: "POST", path: "/api/inventory/nft-metadata/opt-ins/delete", keys: ["opt_in"] },
      { method: "POST", path: "/api/inventory/nft-metadata/settings", keys: ["status"] },
      { method: "POST", path: "/api/inventory/nft-metadata/fetch", keys: ["fetched", "entries"] },
      {
        method: "POST",
        path: "/api/compartment/switch",
        keys: ["status", "compartment_id", "compartment_label"],
      },
      {
        method: "POST",
        path: "/api/compartment/add",
        keys: ["status", "id", "label", "threshold"],
      },
      { method: "POST", path: "/api/secrets/set", keys: ["status", "key"] },
      { method: "POST", path: "/api/secrets/get", keys: ["key", "value"] },
      { method: "POST", path: "/api/secrets/delete", keys: ["status", "key"] },
      { method: "POST", path: "/api/secrets/push", keys: ["status", "from", "to", "key"] },
      { method: "POST", path: "/api/api-keys/set", keys: ["status", "key"] },
      { method: "POST", path: "/api/api-keys/get", keys: ["key", "value"] },
      { method: "POST", path: "/api/api-keys/delete", keys: ["status", "key"] },
      { method: "POST", path: "/api/fido2/pin/set", keys: ["status"] },
      {
        method: "POST",
        path: "/api/fido2/register",
        keys: ["status", "label", "total_keys", "poison"],
      },
      { method: "POST", path: "/api/fido2/remove", keys: ["status", "label"] },
      { method: "POST", path: "/api/backup/export", keys: ["status", "snapshot_hex", "summary"] },
      {
        method: "POST",
        path: "/api/backup/restore",
        keys: ["status", "summary", "requires_reauth"],
      },
      { method: "POST", path: "/api/setup/reset", keys: ["status", "archived_to"] },
      { method: "POST", path: "/api/selfcheck/run", keys: ["status", "checks"] },
    ];

    for (const contract of contracts) await requestJson(base, contract);
  });
});

test("events route serves a versioned snapshot and stays registered", async () => {
  await withServer(async (_daemon, base) => {
    const controller = new AbortController();
    const response = await fetch(base + "/api/events?session=ui-shots-session-token", {
      signal: controller.signal,
    });
    assert.equal(response.status, 200);
    assert.match(response.headers.get("content-type") || "", /^text\/event-stream\b/);
    const reader = response.body.getReader();
    const { value } = await reader.read();
    const frame = new TextDecoder().decode(value);
    assert.match(frame, /^event: snapshot\ndata: /);
    const data = JSON.parse(frame.split("\ndata: ")[1].trim());
    assert.equal(data.v, 1);
    assert.equal(data.locked, false);
    assert.ok(Array.isArray(data.operations));
    controller.abort();
    await reader.cancel().catch(() => {});
  });
});

test("unregistered API routes fail closed and are reported", async () => {
  await withServer(async (daemon, base) => {
    const response = await fetch(base + "/api/__contract_miss");
    assert.equal(response.status, 404);
    const payload = await response.json();
    assert.equal(payload.code, "not_found");
    assert.deepEqual(daemon.getUnknownRequests(), [
      { method: "GET", path: "/api/__contract_miss" },
    ]);
  });
});
