//! 0.1-era daemon data-dir and snapshot upgrade coverage.

mod common;

use common::{get, post_json, spawn_daemon};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const FIXTURE_PASSPHRASE: &str = "sigillum-0-1-fixture-pass";
const SNAPSHOT_PASSPHRASE: &str = "sigillum-0-1-snapshot-pass";
const DEPOSIT_ID: &str = "deposit-fixture-1";
const QUEUE_JOB_ID: &str = "job-fixture-1";
const CANARY_PROVIDER_TOKEN: &str = "canary-provider-token";
const CANARY_NOTE: &str = "canary-note";
const FIXTURE_COUNTERPARTY_ID: &str = "party-fixture-1";
const FIXTURE_ALLOCATION_ID: &str = "alloc-fixture-1";
const DATADIR_FIXTURE: &str = "tests/fixtures/upgrade_0_1_datadir.json";
const SNAPSHOT_FIXTURE: &str = "tests/fixtures/upgrade_0_1_snapshot.json";

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    entries: Vec<ManifestEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ManifestEntry {
    path: String,
    data_hex: String,
}

fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn manifest_fixture_path() -> PathBuf {
    fixture_path(DATADIR_FIXTURE)
}

fn snapshot_fixture_path() -> PathBuf {
    fixture_path(SNAPSHOT_FIXTURE)
}

fn backup_path(path: &Path) -> PathBuf {
    let file_name = path.file_name().unwrap().to_string_lossy();
    path.with_file_name(format!("{file_name}.bak"))
}

fn store_envelope(schema: &str, schema_version: u32, data: Value) -> Value {
    json!({
        "schema": schema,
        "schema_version": schema_version,
        "data": data,
    })
}

fn write_store_and_backup(base_dir: &Path, file_name: &str, body: Value) {
    let path = base_dir.join(file_name);
    let bytes = serde_json::to_vec_pretty(&body).unwrap();
    fs::write(&path, &bytes).unwrap();
    fs::write(backup_path(&path), &bytes).unwrap();
}

fn walk_manifest(base_dir: &Path) -> Manifest {
    let mut entries = Vec::new();
    collect_manifest_entries(base_dir, base_dir, &mut entries);
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Manifest { entries }
}

fn collect_manifest_entries(root: &Path, dir: &Path, entries: &mut Vec<ManifestEntry>) {
    let mut children = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    children.sort();
    for path in children {
        if path.is_dir() {
            collect_manifest_entries(root, &path, entries);
        } else if path.is_file() {
            let relative = path.strip_prefix(root).unwrap();
            entries.push(ManifestEntry {
                path: relative.to_string_lossy().replace('\\', "/"),
                data_hex: hex::encode(fs::read(&path).unwrap()),
            });
        }
    }
}

fn materialize_manifest(manifest_path: &Path, target_dir: &Path) {
    let manifest: Manifest = serde_json::from_slice(&fs::read(manifest_path).unwrap()).unwrap();
    for entry in manifest.entries {
        let path = target_dir.join(&entry.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, hex::decode(entry.data_hex).unwrap()).unwrap();
    }
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn store_schema_version(base_dir: &Path, file_name: &str) -> Option<u64> {
    read_json(&base_dir.join(file_name))["schema_version"].as_u64()
}

fn assert_preboot_versions(base_dir: &Path) {
    assert!(
        read_json(&base_dir.join("profiles.json"))
            .get("schema_version")
            .is_none()
    );
    assert_eq!(store_schema_version(base_dir, "deposits.json"), Some(1));
    assert_eq!(store_schema_version(base_dir, "queue.json"), Some(1));
    assert_eq!(
        store_schema_version(base_dir, "wallet_inventory.json"),
        Some(11)
    );
    assert_eq!(
        store_schema_version(base_dir, "token_registry.json"),
        Some(1)
    );
    assert!(base_dir.join("audit.log").exists());
    assert!(!base_dir.join("audit.db").exists());
}

async fn assert_success(response: reqwest::Response, context: &str) -> Value {
    let status = response.status();
    let text = response.text().await.unwrap();
    assert!(status.is_success(), "{context} failed: {status} {text}");
    if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap()
    }
}

async fn init_compartment(
    client: &reqwest::Client,
    addr: SocketAddr,
    label: &str,
    passphrase: &str,
) -> String {
    let body = assert_success(
        post_json(
            client,
            addr,
            "/api/compartment/init",
            json!({
                "id": 0,
                "label": label,
                "threshold": 1,
                "passphrase": passphrase,
            }),
            None,
        )
        .await,
        "compartment init",
    )
    .await;
    body["session_token"].as_str().unwrap().to_string()
}

async fn unlock(client: &reqwest::Client, addr: SocketAddr, passphrase: &str) -> String {
    let body = assert_success(
        post_json(
            client,
            addr,
            "/api/unlock",
            json!({ "passphrase": passphrase }),
            None,
        )
        .await,
        "unlock",
    )
    .await;
    body["session_token"].as_str().unwrap().to_string()
}

fn old_profiles_registry() -> Value {
    json!({
        "evm_providers": [{
            "name": "mainnet-fixture",
            "rpc_url": "https://mainnet-fixture.invalid",
            "compartment_id": 0,
            "chain_id": 1
        }],
        "eth_stealth_wallets": [{
            "name": "stealth-fixture",
            "wallet": "wallet-fixture",
            "short_name": "eth",
            "provider_profile": "mainnet-fixture",
            "compartment_id": 0,
            "chain_id": 1
        }],
        "eth_xpub_wallets": [{
            "name": "xpub-fixture",
            "project_account": 7,
            "provider_profile": "mainnet-fixture",
            "compartment_id": 0,
            "chain_id": 1
        }],
        "eth_seed_wallets": [{
            "name": "seed-main",
            "label": "Seed fixture",
            "project_account": 0,
            "provider_profile": "mainnet-fixture",
            "compartment_id": 0,
            "chain_id": 1,
            "word_count": 12,
            "mnemonic_secret_key": "seed/fixture/mnemonic",
            "account_path": "m/44'/60'/0'",
            "receive_path": "m/44'/60'/0'/0",
            "receive_xpub": "xpub6FAKE00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "first_receive_address": "0xdead000000000000000000000000000000000001"
        }]
    })
}

fn old_deposits_state() -> Value {
    store_envelope(
        "sigillum.deposits",
        1,
        json!({
            "eth_stealth": [{
                "id": DEPOSIT_ID,
                "status": "pending",
                "asset_kind": "native",
                "wallet_profile": "stealth-fixture",
                "wallet_compartment_id": 0,
                "provider_compartment_id": 0,
                "wallet": "wallet-fixture",
                "short_name": "eth",
                "stealth_meta_address": "st:eth:fixture",
                "stealth_address": "0xdead000000000000000000000000000000000101",
                "ephemeral_public_key_hex": "0x02",
                "view_tag_hex": "0xaa",
                "auto_queue_sweep": false,
                "created_at_unix": 1,
                "updated_at_unix": 1
            }]
        }),
    )
}

fn old_queue_state() -> Value {
    store_envelope(
        "sigillum.queue",
        1,
        json!({
            "jobs": [{
                "id": QUEUE_JOB_ID,
                "state": "queued",
                "attempts": 0,
                "created_at_unix": 1,
                "updated_at_unix": 1,
                "kind": "eth_stealth_native_sweep",
                "wallet_profile": "stealth-fixture",
                "stealth_address": "0xdead000000000000000000000000000000000001",
                "ephemeral_public_key_hex": "0x02",
                "destination_address": "0xdead000000000000000000000000000000000002"
            }]
        }),
    )
}

fn old_wallet_inventory_state() -> Value {
    store_envelope(
        "sigillum.wallet-inventory",
        11,
        json!({
            "chain_profiles": [{
                "name": "custom-rollup",
                "chain_family": "evm",
                "chain_id": 999,
                "native_symbol": "ETH",
                "enabled": true,
                "source": "operator",
                "created_at_unix": 1,
                "updated_at_unix": 2
            }],
            "watch_address_book": [{
                "id": "watch_fixture_1",
                "address": "0xdead000000000000000000000000000000000777",
                "label": "old-ledger",
                "tags": ["client", "hardware"],
                "source": "operator",
                "enabled": true,
                "created_at_unix": 1,
                "updated_at_unix": 2
            }],
            "addresses": [{
                "id": "addr_legacy",
                "wallet_family": "eth-seed",
                "wallet_profile": "seed-main",
                "provider_profile": "mainnet-fixture",
                "address": "0xdead000000000000000000000000000000000111",
                "derivation_path": "m/44'/60'/0'/0/0",
                "address_index": 0,
                "activity_state": "funded",
                "native_balance_wei_hex": "0x1",
                "transaction_count": 1,
                "source": "legacy",
                "first_seen_at_unix": 1,
                "last_checked_at_unix": 2
            }],
            "holdings": [{
                "id": "holding_legacy",
                "wallet_family": "eth-seed",
                "wallet_profile": "seed-main",
                "provider_profile": "mainnet-fixture",
                "address": "0xdead000000000000000000000000000000000111",
                "derivation_path": "m/44'/60'/0'/0/0",
                "asset_kind": "native",
                "amount_hex": "0x1",
                "source": "legacy",
                "status": "detected",
                "first_seen_at_unix": 1,
                "last_checked_at_unix": 2
            }],
            "risk_catalog": [{
                "address": "0xdead000000000000000000000000000000000444",
                "label": "Known router",
                "risk_level": "trusted",
                "source": "operator",
                "notes": ["Operator-approved spender"],
                "created_at_unix": 1,
                "updated_at_unix": 2
            }],
            "risk_findings": [{
                "id": "risk_fixture_1",
                "category": "approval",
                "risk_level": "medium",
                "status": "open",
                "wallet_family": "eth-seed",
                "wallet_profile": "seed-main",
                "provider_profile": "mainnet-fixture",
                "chain_id": 1,
                "address": "0xdead000000000000000000000000000000000111",
                "subject_type": "spender",
                "subject": "0xdead000000000000000000000000000000000444",
                "source": "legacy",
                "recommendation": "review",
                "evidence": ["fixture"],
                "first_seen_at_unix": 1,
                "last_checked_at_unix": 2
            }],
            "consolidation_plans": [{
                "id": "plan_legacy",
                "status": "review_required",
                "created_at_unix": 1,
                "updated_at_unix": 2,
                "summary": {
                    "total_steps": 1,
                    "blocked_steps": 0,
                    "review_required_steps": 1,
                    "approved_steps": 0,
                    "executable_steps": 0,
                    "value_items": 1
                },
                "steps": [{
                    "id": "step_legacy",
                    "action": "sweep_native",
                    "status": "review_required",
                    "wallet_family": "eth-seed",
                    "wallet_profile": "seed-main",
                    "provider_profile": "mainnet-fixture",
                    "address": "0xdead000000000000000000000000000000000111",
                    "derivation_path": "m/44'/60'/0'/0/0",
                    "asset_kind": "native",
                    "amount_hex": "0x1",
                    "signer_status": "unknown",
                    "simulation_status": "not_run",
                    "risk_level": "low",
                    "auto_eligible": false,
                    "approved": false
                }]
            }],
            "treasury_policy": {
                "enabled": true,
                "allowed_destinations": [{
                    "address": "0xdead000000000000000000000000000000000999",
                    "label": "cold-treasury"
                }],
                "max_step_native_wei_hex": "0xde0b6b3a7640000",
                "max_plan_native_wei_hex": null,
                "require_simulation": true,
                "allow_raw_digest_signing": false,
                "block_cross_party_linkage": false,
                "allow_claim_execution": false,
                "allow_gas_topups": false,
                "max_gas_topup_wei_hex": null,
                "allow_plan_execution": false,
                "allow_sweep_execution": false,
                "allow_revoke_execution": false,
                "allow_exit_execution": false,
                "execution_paused": false,
                "max_fee_per_gas_cap_hex": null,
                "simulation_freshness_secs": 900,
                "hot_floor_wei_hex": "0xde0b6b3a7640000",
                "hot_target_wei_hex": "0xde0b6b3a7640000",
                "created_at_unix": 1,
                "updated_at_unix": 2
            },
            "receive_allocations": [{
                "id": FIXTURE_ALLOCATION_ID,
                "wallet_family": "eth-seed",
                "wallet_profile": "seed-main",
                "address": "0xdead000000000000000000000000000000000222",
                "derivation_path": "m/44'/60'/0'/0/3",
                "address_index": 3,
                "purpose": "counterparty-fixture",
                "label": "Fixture invoices",
                "status": "active",
                "created_at_unix": 1,
                "counterparty_id": FIXTURE_COUNTERPARTY_ID
            }],
            "parties": [{
                "id": FIXTURE_COUNTERPARTY_ID,
                "name": "Fixture Counterparty",
                "note": "fake upgrade fixture",
                "sweep_destination_address": "0xdead000000000000000000000000000000000999",
                "created_at_unix": 1
            }]
        }),
    )
}

fn old_token_registry_state() -> Value {
    store_envelope(
        "sigillum.token-registry",
        1,
        json!({
            "lists": [{
                "id": "registry-fixture-1",
                "name": "fixture-list",
                "compartment_id": 0,
                "source": "pasted-json",
                "entries": [{
                    "chain_id": 1,
                    "address": "0xdead000000000000000000000000000000000aaa",
                    "symbol": "FAKE",
                    "decimals": 18
                }],
                "created_at_unix": 1,
                "updated_at_unix": 2
            }]
        }),
    )
}

fn audit_log_jsonl() -> String {
    let init = store_envelope(
        "sigillum.audit-event",
        1,
        json!({
            "created_at_unix": 1,
            "compartment_id": 0,
            "kind": "compartment.init",
            "details": {
                "label": "default",
                "threshold": 1
            }
        }),
    );
    let secret_set = store_envelope(
        "sigillum.audit-event",
        1,
        json!({
            "created_at_unix": 2,
            "compartment_id": 0,
            "kind": "secret.set",
            "details": {
                "key": CANARY_PROVIDER_TOKEN
            }
        }),
    );
    format!(
        "{}\n{}\n",
        serde_json::to_string(&init).unwrap(),
        serde_json::to_string(&secret_set).unwrap()
    )
}

fn write_old_store_files(base_dir: &Path) {
    write_store_and_backup(base_dir, "profiles.json", old_profiles_registry());
    write_store_and_backup(base_dir, "deposits.json", old_deposits_state());
    write_store_and_backup(base_dir, "queue.json", old_queue_state());
    write_store_and_backup(
        base_dir,
        "wallet_inventory.json",
        old_wallet_inventory_state(),
    );
    write_store_and_backup(base_dir, "token_registry.json", old_token_registry_state());
}

fn remove_if_exists(path: &Path) {
    if path.exists() {
        fs::remove_file(path).unwrap();
    }
}

fn assert_no_corrupt_files(dir: &Path) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            assert_no_corrupt_files(&path);
        } else {
            let file_name = path.file_name().unwrap().to_string_lossy();
            assert!(
                !file_name.contains(".corrupt-"),
                "unexpected corrupt quarantine file: {}",
                path.display()
            );
        }
    }
}

fn assert_schema_version(base_dir: &Path, file_name: &str, expected: u64) {
    assert_eq!(
        store_schema_version(base_dir, file_name),
        Some(expected),
        "{file_name} should have migrated schema_version {expected}"
    );
}

fn keys_include(body: &Value, key: &str) -> bool {
    body["keys"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value.as_str() == Some(key))
}

#[tokio::test]
#[ignore]
async fn generate_upgrade_0_1_fixture() {
    let base = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(base.path().to_path_buf()).await;
    let client = reqwest::Client::new();
    let token = init_compartment(&client, addr, "default", FIXTURE_PASSPHRASE).await;

    for (key, value) in [
        (CANARY_PROVIDER_TOKEN, "test-token-abcdef"),
        (CANARY_NOTE, "do-not-use"),
    ] {
        assert_success(
            post_json(
                &client,
                addr,
                "/api/api-keys/set",
                json!({ "key": key, "value": value }),
                Some(&token),
            )
            .await,
            "api key set",
        )
        .await;
    }

    write_old_store_files(base.path());

    let export = assert_success(
        post_json(
            &client,
            addr,
            "/api/backup/export",
            json!({ "passphrase": SNAPSHOT_PASSPHRASE }),
            Some(&token),
        )
        .await,
        "snapshot export",
    )
    .await;
    let snapshot_hex = export["snapshot_hex"].as_str().unwrap();
    let snapshot_bytes = hex::decode(snapshot_hex).unwrap();
    let snapshot_path = snapshot_fixture_path();
    fs::create_dir_all(snapshot_path.parent().unwrap()).unwrap();
    fs::write(&snapshot_path, snapshot_bytes).unwrap();

    handle.abort();
    remove_if_exists(&base.path().join("audit.db"));
    remove_if_exists(&base.path().join("audit.db-wal"));
    remove_if_exists(&base.path().join("audit.db-shm"));
    remove_if_exists(&base.path().join("audit.log.migrated"));
    fs::write(base.path().join("audit.log"), audit_log_jsonl()).unwrap();

    let manifest = walk_manifest(base.path());
    let manifest_path = manifest_fixture_path();
    fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let sanity = TempDir::new().unwrap();
    materialize_manifest(&manifest_path, sanity.path());
    assert_preboot_versions(sanity.path());
}

#[tokio::test]
async fn upgrade_0_1_datadir_migrates_all_stores() {
    let base = TempDir::new().unwrap();
    materialize_manifest(&manifest_fixture_path(), base.path());
    assert_preboot_versions(base.path());

    let (addr, handle) = spawn_daemon(base.path().to_path_buf()).await;
    let client = reqwest::Client::new();
    let health = assert_success(get(&client, addr, "/api/health", None).await, "health").await;
    assert_eq!(health["ready"], json!(true));
    assert_eq!(health["startup_error"], Value::Null);
    assert_no_corrupt_files(base.path());
    assert!(base.path().join("audit.db").exists());
    assert!(fs::metadata(base.path().join("audit.db")).unwrap().len() > 0);
    assert!(base.path().join("audit.log.migrated").exists());
    assert!(!base.path().join("audit.log").exists());

    let token = unlock(&client, addr, FIXTURE_PASSPHRASE).await;
    let keys = assert_success(
        get(&client, addr, "/api/api-keys", Some(&token)).await,
        "api key list",
    )
    .await;
    assert!(keys_include(&keys, CANARY_PROVIDER_TOKEN));
    assert!(keys_include(&keys, CANARY_NOTE));

    let jobs = assert_success(
        get(&client, addr, "/api/queue/jobs", Some(&token)).await,
        "queue list",
    )
    .await;
    let job = jobs["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["id"] == QUEUE_JOB_ID)
        .expect("fixture queue job present");
    let state = job["state"].as_str().unwrap();
    assert!(
        [
            "queued",
            "operator_action_required",
            "backoff",
            "confirmed",
            "failed",
            "blocked",
            "running",
            "succeeded",
            "cancelled",
        ]
        .contains(&state)
    );

    assert_success(
        post_json(
            &client,
            addr,
            "/api/profiles/evm/upsert",
            json!({
                "name": "upgrade-check",
                "rpc_url": "https://check.invalid",
                "chain_id": 1,
                "compartment_id": 0
            }),
            Some(&token),
        )
        .await,
        "profile upsert",
    )
    .await;
    assert_schema_version(base.path(), "profiles.json", 1);

    assert_success(
        post_json(
            &client,
            addr,
            "/api/receiving/deposits/tag",
            json!({ "deposit_id": DEPOSIT_ID, "counterparty_id": null }),
            Some(&token),
        )
        .await,
        "deposit tag",
    )
    .await;
    assert_schema_version(base.path(), "deposits.json", 2);

    assert_success(
        post_json(&client, addr, "/api/queue/process", json!({}), Some(&token)).await,
        "queue process",
    )
    .await;
    assert_schema_version(base.path(), "queue.json", 5);

    assert_success(
        post_json(
            &client,
            addr,
            "/api/inventory/watch-addresses/upsert",
            json!({
                "address": "0xdead00000000000000000000000000000000beef",
                "label": "upgrade-check"
            }),
            Some(&token),
        )
        .await,
        "watch address upsert",
    )
    .await;
    assert_schema_version(base.path(), "wallet_inventory.json", 20);
    assert_schema_version(base.path(), "token_registry.json", 1);

    let policy = assert_success(
        get(&client, addr, "/api/treasury/policy", Some(&token)).await,
        "treasury policy",
    )
    .await;
    assert!(policy["policy"].is_object());
    let parties = assert_success(
        get(&client, addr, "/api/treasury/parties", Some(&token)).await,
        "treasury parties",
    )
    .await;
    assert!(
        parties["parties"]
            .as_array()
            .unwrap()
            .iter()
            .any(|party| party["id"] == FIXTURE_COUNTERPARTY_ID)
    );
    let allocations = assert_success(
        get(
            &client,
            addr,
            "/api/treasury/receive-addresses",
            Some(&token),
        )
        .await,
        "treasury receive addresses",
    )
    .await;
    assert!(
        allocations["allocations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|allocation| allocation["id"] == FIXTURE_ALLOCATION_ID)
    );

    // Mirrors `sigillum doctor` blocking checks: data dir exists and daemon is reachable.
    assert!(base.path().is_dir());
    assert_success(
        get(&client, addr, "/api/status", Some(&token)).await,
        "status",
    )
    .await;
    handle.abort();
}

#[tokio::test]
async fn upgrade_0_1_snapshot_restores_under_1_0() {
    let restore_base = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(restore_base.path().to_path_buf()).await;
    let client = reqwest::Client::new();
    let token = init_compartment(&client, addr, "fresh", "fresh-pass").await;

    let snapshot_hex = hex::encode(fs::read(snapshot_fixture_path()).unwrap());
    assert_success(
        post_json(
            &client,
            addr,
            "/api/backup/restore",
            json!({
                "snapshot_hex": snapshot_hex,
                "passphrase": SNAPSHOT_PASSPHRASE
            }),
            Some(&token),
        )
        .await,
        "snapshot restore",
    )
    .await;

    let restored_token = unlock(&client, addr, FIXTURE_PASSPHRASE).await;
    let keys = assert_success(
        get(&client, addr, "/api/api-keys", Some(&restored_token)).await,
        "restored api keys",
    )
    .await;
    assert!(keys_include(&keys, CANARY_PROVIDER_TOKEN));
    assert!(keys_include(&keys, CANARY_NOTE));

    let jobs = assert_success(
        get(&client, addr, "/api/queue/jobs", Some(&restored_token)).await,
        "restored queue jobs",
    )
    .await;
    assert!(
        jobs["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|job| job["id"] == QUEUE_JOB_ID)
    );
    handle.abort();
}
