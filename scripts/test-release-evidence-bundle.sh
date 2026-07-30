#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECKER="${ROOT}/scripts/check-release-evidence-bundle.sh"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/sigillum-release-evidence-test.XXXXXX")"
RC_TAG="v1.0.0-rc.5"
RC_SHA="1111111111111111111111111111111111111111"
RC_TAG_OBJECT="2222222222222222222222222222222222222222"
BUNDLE_NAME="sigillum-v1.0.0-release-evidence.tar.gz"
F6_FAILURE="F6 receipt does not prove four core families and both ordered gas-top-up chain legs across supported testnets"
DESKTOP_FAILURE="desktop receipt does not bind the qualified RC dmg, supported clean host, unlock screenshot, and operator review"
UI_FAILURE="UI receipt does not bind the qualified RC dmg, five-destination journey, screenshots, and operator review"
DOCTOR_FAILURE="doctor receipt does not bind the qualified RC CLI, supported host, all blocking checks, and operator review"

cleanup() {
  rm -rf "${TMP_ROOT}"
}
trap cleanup EXIT

fail() {
  echo "release evidence bundle test failed: $*" >&2
  exit 1
}

write_payload() {
  local payload="$1"
  local l2_chain_id="${2:-84532}"
  mkdir -p \
    "${payload}/f4" \
    "${payload}/f6/audit" \
    "${payload}/desktop/screenshots" \
    "${payload}/doctor" \
    "${payload}/ui/screenshots" \
    "${payload}/release"

  printf '\211PNG\r\n\032\nsigillum clean-install unlock fixture\n' \
    > "${payload}/desktop/screenshots/unlock.png"
  printf '\211PNG\r\n\032\nsigillum UI setup fixture\n' \
    > "${payload}/ui/screenshots/setup.png"
  printf '\211PNG\r\n\032\nsigillum UI locked fixture\n' \
    > "${payload}/ui/screenshots/locked.png"
  printf '\211PNG\r\n\032\nsigillum UI unlocked fixture\n' \
    > "${payload}/ui/screenshots/unlocked.png"
  local desktop_unlock_sha256
  local ui_setup_sha256
  local ui_locked_sha256
  local ui_unlocked_sha256
  desktop_unlock_sha256="$(
    shasum -a 256 "${payload}/desktop/screenshots/unlock.png" | awk '{ print $1 }'
  )"
  ui_setup_sha256="$(
    shasum -a 256 "${payload}/ui/screenshots/setup.png" | awk '{ print $1 }'
  )"
  ui_locked_sha256="$(
    shasum -a 256 "${payload}/ui/screenshots/locked.png" | awk '{ print $1 }'
  )"
  ui_unlocked_sha256="$(
    shasum -a 256 "${payload}/ui/screenshots/unlocked.png" | awk '{ print $1 }'
  )"

  cat > "${payload}/MANIFEST.json" <<JSON
{
  "schema_version": 1,
  "kind": "sigillum.release_evidence",
  "release_version": "1.0.0",
  "rc_tag": "${RC_TAG}",
  "rc_tag_object": "${RC_TAG_OBJECT}",
  "rc_peeled_sha": "${RC_SHA}",
  "release_workflow_run": 12345,
  "asset_checksums_verified": true,
  "gates": {
    "f4": "passed",
    "f6": "passed",
    "desktop": "passed",
    "doctor": "passed",
    "ui": "passed"
  }
}
JSON

  cat > "${payload}/f4/standard.json" <<JSON
{
  "schema_version": 1,
  "kind": "sigillum.local_soak",
  "status": "passed",
  "repo": {"commit": "${RC_SHA}", "dirty": false},
  "host": {"name": "mac-server", "os": "macOS test fixture"},
  "configured": {"soak_seconds": 3600},
  "timing": {"duration_seconds": 3605},
  "evidence": {"iterations": 120, "doctor_runs": 120},
  "chaos": {"enabled": false}
}
JSON

  cat > "${payload}/f4/chaos.json" <<JSON
{
  "schema_version": 1,
  "kind": "sigillum.local_soak",
  "status": "passed",
  "repo": {"commit": "${RC_SHA}", "dirty": false},
  "host": {"name": "mac-server", "os": "macOS test fixture"},
  "configured": {"soak_seconds": 600},
  "timing": {"duration_seconds": 605},
  "evidence": {"iterations": 60, "doctor_runs": 60},
  "chaos": {
    "enabled": true,
    "kill_cycles": 3,
    "in_flight_assertion": {"status": "passed"}
  }
}
JSON

  cat > "${payload}/f6/receipts.json" <<JSON
{
  "schema_version": 2,
  "kind": "sigillum.testnet_execution",
  "status": "passed",
  "rc_sha": "${RC_SHA}",
  "networks": [
    {"role": "sepolia", "chain_id": 11155111},
    {"role": "l2", "chain_id": ${l2_chain_id}}
  ],
  "executions": [
    {
      "family": "native_sweep",
      "network_role": "sepolia",
      "chain_id": 11155111,
      "status": "confirmed",
      "tx_hash": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "audit_export": "f6/audit/native-sweep.json"
    },
    {
      "family": "erc20_sweep",
      "network_role": "l2",
      "chain_id": ${l2_chain_id},
      "status": "confirmed",
      "tx_hash": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      "audit_export": "f6/audit/erc20-sweep.json"
    },
    {
      "family": "erc20_revoke",
      "network_role": "sepolia",
      "chain_id": 11155111,
      "status": "confirmed",
      "tx_hash": "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
      "audit_export": "f6/audit/erc20-revoke.json"
    },
    {
      "family": "gas_top_up_sweep",
      "network_role": "l2",
      "chain_id": ${l2_chain_id},
      "status": "confirmed",
      "plan_id": "plan-gas-top-up",
      "legs": [
        {
          "role": "fund_gas",
          "action": "fund_gas",
          "plan_id": "plan-gas-top-up",
          "step_id": "step-fund-gas",
          "job_id": "job-fund-gas",
          "network_role": "l2",
          "chain_id": ${l2_chain_id},
          "source_address": "0x1111111111111111111111111111111111111111",
          "destination_address": "0x2222222222222222222222222222222222222222",
          "prerequisite_job_ids": [],
          "queue_state": "confirmed",
          "receipt_status": "success",
          "confirmations": 2,
          "receipt_block_number": 100,
          "broadcast_at_unix": 1000,
          "tx_hash": "0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
          "audit_export": "f6/audit/gas-top-up.json"
        },
        {
          "role": "dependent_sweep",
          "action": "sweep_native",
          "plan_id": "plan-gas-top-up",
          "step_id": "step-dependent-sweep",
          "job_id": "job-dependent-sweep",
          "network_role": "l2",
          "chain_id": ${l2_chain_id},
          "source_address": "0x2222222222222222222222222222222222222222",
          "destination_address": "0x3333333333333333333333333333333333333333",
          "prerequisite_job_ids": ["job-fund-gas"],
          "queue_state": "confirmed",
          "receipt_status": "success",
          "confirmations": 2,
          "receipt_block_number": 101,
          "broadcast_at_unix": 1001,
          "tx_hash": "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
          "audit_export": "f6/audit/gas-dependent-sweep.json"
        }
      ]
    }
  ]
}
JSON

  while IFS= read -r claim; do
    audit_export="$(jq -r '.audit_export' <<<"${claim}")"
    jq -n \
      --arg rc_sha "${RC_SHA}" \
      --argjson claim "${claim}" '
        ({
          schema_version: 2,
          kind: "sigillum.execution_audit",
          status: "verified",
          rc_sha: $rc_sha,
          family: $claim.family,
          network_role: $claim.network_role,
          chain_id: $claim.chain_id,
          tx_hash: $claim.tx_hash,
          audit_chain_verified: true
        } +
        (if $claim.family == "gas_top_up_sweep" then {
          leg_role: $claim.role,
          plan_id: $claim.plan_id,
          step_id: $claim.step_id,
          job_id: $claim.job_id,
          action: $claim.action,
          source_address: $claim.source_address,
          destination_address: $claim.destination_address,
          prerequisite_job_ids: $claim.prerequisite_job_ids,
          queue_state: $claim.queue_state,
          receipt_status: $claim.receipt_status,
          confirmations: $claim.confirmations,
          receipt_block_number: $claim.receipt_block_number,
          broadcast_at_unix: $claim.broadcast_at_unix
        } else {} end))
      ' > "${payload}/${audit_export}"
  done < <(jq -c '
    .executions[] |
    if .family == "gas_top_up_sweep" then
      . as $parent |
      .legs[] |
      . + {
        family: $parent.family,
        network_role: $parent.network_role,
        chain_id: $parent.chain_id,
        plan_id: $parent.plan_id
      }
    else
      .
    end' "${payload}/f6/receipts.json")

  cat > "${payload}/desktop/clean-install.json" <<JSON
{
  "schema_version": 2,
  "kind": "sigillum.clean_install",
  "status": "passed",
  "rc": {
    "tag": "${RC_TAG}",
    "tag_object": "${RC_TAG_OBJECT}",
    "peeled_sha": "${RC_SHA}"
  },
  "artifact": {
    "filename": "Sigillum-${RC_TAG}-macos-aarch64.dmg",
    "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  },
  "host": {
    "role": "mac-server",
    "name": "mac-server",
    "platform": "macos",
    "os_version": "15.7.1",
    "arch": "aarch64"
  },
  "installation": {
    "path": "/Applications/Sigillum.app",
    "bundle_identifier": "com.sigillum.desktop",
    "app_version": "1.0.0",
    "checksum_verified": true,
    "dev_toolchain_absent": true,
    "unlock_reached": true
  },
  "reviewer": {
    "id": "release-operator",
    "role": "release_operator",
    "reviewed_at_utc": "2026-07-30T12:00:00Z"
  },
  "screenshots": [
    {
      "state": "unlock",
      "path": "desktop/screenshots/unlock.png",
      "sha256": "${desktop_unlock_sha256}"
    }
  ]
}
JSON

  cat > "${payload}/ui/signoff.json" <<JSON
{
  "schema_version": 2,
  "kind": "sigillum.ui_signoff",
  "status": "passed",
  "rc": {
    "tag": "${RC_TAG}",
    "tag_object": "${RC_TAG_OBJECT}",
    "peeled_sha": "${RC_SHA}"
  },
  "artifact": {
    "filename": "Sigillum-${RC_TAG}-macos-aarch64.dmg",
    "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  },
  "host": {
    "role": "mac-server",
    "name": "mac-server",
    "platform": "macos",
    "os_version": "15.7.1",
    "arch": "aarch64"
  },
  "reviewer": {
    "id": "release-operator",
    "role": "release_operator",
    "reviewed_at_utc": "2026-07-30T12:30:00Z"
  },
  "walkthrough": {
    "full_walkthrough_completed": true,
    "destinations": ["overview", "receive", "portfolio", "move", "vault"],
    "states": ["setup", "locked", "unlocked"],
    "journey": [
      "import_seed",
      "multi_chain_scan",
      "review_inventory_risk",
      "generate_plan",
      "approve_plan",
      "execute_mock_provider",
      "audit_trail_complete"
    ],
    "operator_surface_parity_reviewed": true,
    "accessibility_review_completed": true
  },
  "screenshots": [
    {
      "state": "setup",
      "path": "ui/screenshots/setup.png",
      "sha256": "${ui_setup_sha256}"
    },
    {
      "state": "locked",
      "path": "ui/screenshots/locked.png",
      "sha256": "${ui_locked_sha256}"
    },
    {
      "state": "unlocked",
      "path": "ui/screenshots/unlocked.png",
      "sha256": "${ui_unlocked_sha256}"
    }
  ]
}
JSON

  cat > "${payload}/doctor/mac-server.json" <<JSON
{
  "schema_version": 2,
  "kind": "sigillum.doctor",
  "status": "passed",
  "rc": {
    "tag": "${RC_TAG}",
    "tag_object": "${RC_TAG_OBJECT}",
    "peeled_sha": "${RC_SHA}"
  },
  "artifact": {
    "filename": "sigillum-cli-${RC_TAG}-macos-aarch64.tar.gz",
    "sha256": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
  },
  "host": {
    "role": "mac-server",
    "name": "mac-server",
    "platform": "macos",
    "os_version": "15.7.1",
    "arch": "aarch64"
  },
  "cli": {
    "version": "1.0.0",
    "executable_path": "/usr/local/bin/sigillum",
    "executable_sha256": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
  },
  "doctor": {
    "command": "sigillum doctor",
    "exit_code": 0,
    "checks_passed": true,
    "checks": [
      {"name": "data_dir", "status": "ok"},
      {"name": "data_dir_permissions", "status": "ok"},
      {"name": "audit_db", "status": "ok"},
      {"name": "daemon_url", "status": "ok"},
      {"name": "session_token", "status": "ok"},
      {"name": "daemon_reachability", "status": "ok"},
      {"name": "active_compartment", "status": "ok"}
    ]
  },
  "reviewer": {
    "id": "release-operator",
    "role": "release_operator",
    "reviewed_at_utc": "2026-07-30T12:45:00Z"
  }
}
JSON

  printf '%s\n' \
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  Sigillum-${RC_TAG}-macos-aarch64.app.zip" \
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  Sigillum-${RC_TAG}-macos-aarch64.dmg" \
    "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc  THIRD-PARTY-NOTICES.txt" \
    "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd  sigillum-cli-${RC_TAG}-linux-x86_64.tar.gz" \
    "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee  sigillum-cli-${RC_TAG}-macos-aarch64.tar.gz" \
    > "${payload}/release/asset-SHA256SUMS"
}

generate_sums() {
  local payload="$1"
  local sums_tmp
  sums_tmp="$(mktemp "${payload}/../SHA256SUMS.XXXXXX")"
  (
    cd "${payload}"
    find . -type f ! -name SHA256SUMS -print |
      awk '{ sub(/^\.\//, ""); print }' |
      sort |
      while IFS= read -r evidence_file; do
        shasum -a 256 "${evidence_file}"
      done > "${sums_tmp}"
    mv "${sums_tmp}" SHA256SUMS
  )
}

archive_payload() {
  local payload="$1"
  local bundle="$2"
  local archive_list="${payload}/../archive-list"
  (
    cd "${payload}"
    find . \( -type f -o -type l \) -print |
      awk '{ sub(/^\.\//, ""); print }' |
      sort > "${archive_list}"
  )
  tar -czf "${bundle}" -C "${payload}" -T "${archive_list}"
}

build_valid_case() {
  local case_name="$1"
  local l2_chain_id="${2:-84532}"
  local case_dir="${TMP_ROOT}/${case_name}"
  local payload="${case_dir}/payload"
  mkdir -p "${case_dir}"
  write_payload "${payload}" "${l2_chain_id}"
  generate_sums "${payload}"
  archive_payload "${payload}" "${case_dir}/${BUNDLE_NAME}"
  printf '%s\n' "${case_dir}/${BUNDLE_NAME}"
}

build_receipt_mutation_case() {
  local case_name="$1"
  local filter="$2"
  local case_dir="${TMP_ROOT}/${case_name}"
  local payload="${case_dir}/payload"
  mkdir -p "${case_dir}"
  write_payload "${payload}"
  jq "${filter}" "${payload}/f6/receipts.json" > "${case_dir}/receipts.tmp"
  mv "${case_dir}/receipts.tmp" "${payload}/f6/receipts.json"
  generate_sums "${payload}"
  archive_payload "${payload}" "${case_dir}/${BUNDLE_NAME}"
  printf '%s\n' "${case_dir}/${BUNDLE_NAME}"
}

build_evidence_mutation_case() {
  local case_name="$1"
  local evidence_path="$2"
  local filter="$3"
  local case_dir="${TMP_ROOT}/${case_name}"
  local payload="${case_dir}/payload"
  mkdir -p "${case_dir}"
  write_payload "${payload}"
  jq "${filter}" "${payload}/${evidence_path}" > "${case_dir}/evidence.tmp"
  mv "${case_dir}/evidence.tmp" "${payload}/${evidence_path}"
  generate_sums "${payload}"
  archive_payload "${payload}" "${case_dir}/${BUNDLE_NAME}"
  printf '%s\n' "${case_dir}/${BUNDLE_NAME}"
}

build_gas_broadcast_time_case() {
  local case_name="$1"
  local sweep_broadcast_at_unix="$2"
  local case_dir="${TMP_ROOT}/${case_name}"
  local payload="${case_dir}/payload"
  mkdir -p "${case_dir}"
  write_payload "${payload}"
  jq --argjson timestamp "${sweep_broadcast_at_unix}" '
    (.executions[] | select(.family == "gas_top_up_sweep") | .legs[1] |
      .broadcast_at_unix) = $timestamp
  ' "${payload}/f6/receipts.json" > "${case_dir}/receipts.tmp"
  mv "${case_dir}/receipts.tmp" "${payload}/f6/receipts.json"
  jq --argjson timestamp "${sweep_broadcast_at_unix}" \
    '.broadcast_at_unix = $timestamp' \
    "${payload}/f6/audit/gas-dependent-sweep.json" > \
    "${case_dir}/audit.tmp"
  mv "${case_dir}/audit.tmp" \
    "${payload}/f6/audit/gas-dependent-sweep.json"
  generate_sums "${payload}"
  archive_payload "${payload}" "${case_dir}/${BUNDLE_NAME}"
  printf '%s\n' "${case_dir}/${BUNDLE_NAME}"
}

expect_failure() {
  local bundle="$1"
  local expected_message="$2"
  local log_path="${bundle}.log"
  if bash "${CHECKER}" "${bundle}" "${RC_TAG}" "${RC_SHA}" "${RC_TAG_OBJECT}" \
    > "${log_path}" 2>&1; then
    fail "negative bundle unexpectedly passed: ${bundle}"
  fi
  grep -F "${expected_message}" "${log_path}" >/dev/null || {
    sed -n '1,120p' "${log_path}" >&2
    fail "negative bundle did not report: ${expected_message}"
  }
}

VALID_BUNDLE="$(build_valid_case valid)"
bash "${CHECKER}" "${VALID_BUNDLE}" "${RC_TAG}" "${RC_SHA}" "${RC_TAG_OBJECT}"

for supported_l2 in \
  "base-sepolia 84532" \
  "arbitrum-sepolia 421614" \
  "op-sepolia 11155420"; do
  read -r case_name chain_id <<< "${supported_l2}"
  supported_bundle="$(build_valid_case "${case_name}" "${chain_id}")"
  bash "${CHECKER}" "${supported_bundle}" "${RC_TAG}" "${RC_SHA}" \
    "${RC_TAG_OBJECT}"
done

MISSING_CASE="${TMP_ROOT}/missing"
mkdir -p "${MISSING_CASE}"
write_payload "${MISSING_CASE}/payload"
rm "${MISSING_CASE}/payload/ui/signoff.json"
generate_sums "${MISSING_CASE}/payload"
archive_payload "${MISSING_CASE}/payload" "${MISSING_CASE}/${BUNDLE_NAME}"
expect_failure "${MISSING_CASE}/${BUNDLE_NAME}" "required evidence file is missing or empty: ui/signoff.json"

WRONG_ID_CASE="${TMP_ROOT}/wrong-id"
mkdir -p "${WRONG_ID_CASE}"
write_payload "${WRONG_ID_CASE}/payload"
jq '.rc_peeled_sha = "3333333333333333333333333333333333333333"' \
  "${WRONG_ID_CASE}/payload/MANIFEST.json" > "${WRONG_ID_CASE}/manifest.tmp"
mv "${WRONG_ID_CASE}/manifest.tmp" "${WRONG_ID_CASE}/payload/MANIFEST.json"
generate_sums "${WRONG_ID_CASE}/payload"
archive_payload "${WRONG_ID_CASE}/payload" "${WRONG_ID_CASE}/${BUNDLE_NAME}"
expect_failure "${WRONG_ID_CASE}/${BUNDLE_NAME}" "MANIFEST.json does not bind every required gate to the exact RC identities"

UNCOVERED_CASE="${TMP_ROOT}/uncovered"
mkdir -p "${UNCOVERED_CASE}"
write_payload "${UNCOVERED_CASE}/payload"
generate_sums "${UNCOVERED_CASE}/payload"
printf '%s\n' "not checksummed" > "${UNCOVERED_CASE}/payload/unlisted.txt"
archive_payload "${UNCOVERED_CASE}/payload" "${UNCOVERED_CASE}/${BUNDLE_NAME}"
expect_failure "${UNCOVERED_CASE}/${BUNDLE_NAME}" "SHA256SUMS must cover every payload file exactly once"

SYMLINK_CASE="${TMP_ROOT}/symlink"
mkdir -p "${SYMLINK_CASE}"
write_payload "${SYMLINK_CASE}/payload"
generate_sums "${SYMLINK_CASE}/payload"
ln -s MANIFEST.json "${SYMLINK_CASE}/payload/manifest-link.json"
archive_payload "${SYMLINK_CASE}/payload" "${SYMLINK_CASE}/${BUNDLE_NAME}"
expect_failure "${SYMLINK_CASE}/${BUNDLE_NAME}" \
  "bundle contains a linked or special archive member"

HARDLINK_CASE="${TMP_ROOT}/hardlink"
mkdir -p "${HARDLINK_CASE}"
write_payload "${HARDLINK_CASE}/payload"
ln "${HARDLINK_CASE}/payload/MANIFEST.json" \
  "${HARDLINK_CASE}/payload/manifest-hardlink.json"
generate_sums "${HARDLINK_CASE}/payload"
archive_payload "${HARDLINK_CASE}/payload" "${HARDLINK_CASE}/${BUNDLE_NAME}"
expect_failure "${HARDLINK_CASE}/${BUNDLE_NAME}" \
  "bundle contains a linked or special archive member"

ABSOLUTE_CASE="${TMP_ROOT}/absolute-path"
mkdir -p "${ABSOLUTE_CASE}"
write_payload "${ABSOLUTE_CASE}/payload"
generate_sums "${ABSOLUTE_CASE}/payload"
tar -czPf "${ABSOLUTE_CASE}/${BUNDLE_NAME}" \
  "${ABSOLUTE_CASE}/payload/MANIFEST.json"
expect_failure "${ABSOLUTE_CASE}/${BUNDLE_NAME}" "bundle contains an unsafe path"

ALIAS_CASE="${TMP_ROOT}/path-alias"
mkdir -p "${ALIAS_CASE}"
write_payload "${ALIAS_CASE}/payload"
generate_sums "${ALIAS_CASE}/payload"
(
  cd "${ALIAS_CASE}/payload"
  find . -type f -print |
    awk '{ sub(/^\.\//, ""); print }' |
    sort > "${ALIAS_CASE}/archive-list"
)
printf '%s\n' "f4/./standard.json" >> "${ALIAS_CASE}/archive-list"
tar -czf "${ALIAS_CASE}/${BUNDLE_NAME}" -C "${ALIAS_CASE}/payload" \
  -T "${ALIAS_CASE}/archive-list"
expect_failure "${ALIAS_CASE}/${BUNDLE_NAME}" "bundle contains a non-canonical path"

CASEFOLD_CASE="${TMP_ROOT}/casefold-alias"
mkdir -p "${CASEFOLD_CASE}"
write_payload "${CASEFOLD_CASE}/payload"
generate_sums "${CASEFOLD_CASE}/payload"
printf '%s\n' "case-folded shadow" > "${CASEFOLD_CASE}/payload/shadow.json"
(
  cd "${CASEFOLD_CASE}/payload"
  find . -type f -print |
    awk '{ sub(/^\.\//, ""); print }' |
    sort > "${CASEFOLD_CASE}/archive-list"
)
if tar --version 2>/dev/null | grep -qi bsdtar; then
  tar -czf "${CASEFOLD_CASE}/${BUNDLE_NAME}" -C "${CASEFOLD_CASE}/payload" \
    -s ',^shadow\.json$,manifest.json,' -T "${CASEFOLD_CASE}/archive-list"
else
  tar -czf "${CASEFOLD_CASE}/${BUNDLE_NAME}" -C "${CASEFOLD_CASE}/payload" \
    --transform='s#^shadow\.json$#manifest.json#' -T "${CASEFOLD_CASE}/archive-list"
fi
expect_failure "${CASEFOLD_CASE}/${BUNDLE_NAME}" \
  "bundle contains case-folded path collisions"

SHORT_SOAK_CASE="${TMP_ROOT}/short-soak"
mkdir -p "${SHORT_SOAK_CASE}"
write_payload "${SHORT_SOAK_CASE}/payload"
jq '.timing.duration_seconds = 30' \
  "${SHORT_SOAK_CASE}/payload/f4/standard.json" > "${SHORT_SOAK_CASE}/standard.tmp"
mv "${SHORT_SOAK_CASE}/standard.tmp" \
  "${SHORT_SOAK_CASE}/payload/f4/standard.json"
generate_sums "${SHORT_SOAK_CASE}/payload"
archive_payload "${SHORT_SOAK_CASE}/payload" "${SHORT_SOAK_CASE}/${BUNDLE_NAME}"
expect_failure "${SHORT_SOAK_CASE}/${BUNDLE_NAME}" \
  "F4 standard receipt does not prove the required clean 3600-second RC soak"

STRING_SOAK_CASE="${TMP_ROOT}/string-soak"
mkdir -p "${STRING_SOAK_CASE}"
write_payload "${STRING_SOAK_CASE}/payload"
jq '.configured.soak_seconds = "3600" |
    .timing.duration_seconds = "3605" |
    .evidence.iterations = "120" |
    .evidence.doctor_runs = "120"' \
  "${STRING_SOAK_CASE}/payload/f4/standard.json" > "${STRING_SOAK_CASE}/standard.tmp"
mv "${STRING_SOAK_CASE}/standard.tmp" \
  "${STRING_SOAK_CASE}/payload/f4/standard.json"
generate_sums "${STRING_SOAK_CASE}/payload"
archive_payload "${STRING_SOAK_CASE}/payload" "${STRING_SOAK_CASE}/${BUNDLE_NAME}"
expect_failure "${STRING_SOAK_CASE}/${BUNDLE_NAME}" \
  "F4 standard receipt does not prove the required clean 3600-second RC soak"

WRONG_HOST_CASE="${TMP_ROOT}/wrong-host"
mkdir -p "${WRONG_HOST_CASE}"
write_payload "${WRONG_HOST_CASE}/payload"
jq '.host.name = "unqualified-laptop"' \
  "${WRONG_HOST_CASE}/payload/f4/standard.json" > "${WRONG_HOST_CASE}/standard.tmp"
mv "${WRONG_HOST_CASE}/standard.tmp" \
  "${WRONG_HOST_CASE}/payload/f4/standard.json"
generate_sums "${WRONG_HOST_CASE}/payload"
archive_payload "${WRONG_HOST_CASE}/payload" "${WRONG_HOST_CASE}/${BUNDLE_NAME}"
expect_failure "${WRONG_HOST_CASE}/${BUNDLE_NAME}" \
  "F4 standard receipt does not prove the required clean 3600-second RC soak"

legacy_clean_install_bundle="$(build_evidence_mutation_case \
  clean-install-legacy-schema desktop/clean-install.json \
  '.schema_version = 1')"
expect_failure "${legacy_clean_install_bundle}" "${DESKTOP_FAILURE}"

wrong_clean_install_artifact_bundle="$(build_evidence_mutation_case \
  clean-install-wrong-artifact desktop/clean-install.json \
  '.artifact.sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"')"
expect_failure "${wrong_clean_install_artifact_bundle}" "${DESKTOP_FAILURE}"

wrong_clean_install_host_bundle="$(build_evidence_mutation_case \
  clean-install-wrong-host desktop/clean-install.json \
  '.host.os_version = "26.5.2" | .host.arch = "x86_64"')"
expect_failure "${wrong_clean_install_host_bundle}" "${DESKTOP_FAILURE}"

TAMPERED_UNLOCK_CASE="${TMP_ROOT}/clean-install-tampered-screenshot"
mkdir -p "${TAMPERED_UNLOCK_CASE}"
write_payload "${TAMPERED_UNLOCK_CASE}/payload"
printf 'not the reviewed unlock screenshot\n' \
  > "${TAMPERED_UNLOCK_CASE}/payload/desktop/screenshots/unlock.png"
generate_sums "${TAMPERED_UNLOCK_CASE}/payload"
archive_payload "${TAMPERED_UNLOCK_CASE}/payload" \
  "${TAMPERED_UNLOCK_CASE}/${BUNDLE_NAME}"
expect_failure "${TAMPERED_UNLOCK_CASE}/${BUNDLE_NAME}" \
  "desktop unlock screenshot digest does not match desktop/clean-install.json"

missing_ui_destination_bundle="$(build_evidence_mutation_case \
  ui-missing-destination ui/signoff.json \
  '.walkthrough.destinations = ["overview", "receive", "portfolio", "move"]')"
expect_failure "${missing_ui_destination_bundle}" "${UI_FAILURE}"

wrong_ui_reviewer_bundle="$(build_evidence_mutation_case \
  ui-wrong-reviewer ui/signoff.json \
  '.reviewer.role = "automation"')"
expect_failure "${wrong_ui_reviewer_bundle}" "${UI_FAILURE}"

TAMPERED_UI_CASE="${TMP_ROOT}/ui-tampered-screenshot"
mkdir -p "${TAMPERED_UI_CASE}"
write_payload "${TAMPERED_UI_CASE}/payload"
printf 'not the reviewed unlocked screenshot\n' \
  > "${TAMPERED_UI_CASE}/payload/ui/screenshots/unlocked.png"
generate_sums "${TAMPERED_UI_CASE}/payload"
archive_payload "${TAMPERED_UI_CASE}/payload" \
  "${TAMPERED_UI_CASE}/${BUNDLE_NAME}"
expect_failure "${TAMPERED_UI_CASE}/${BUNDLE_NAME}" \
  "UI walkthrough screenshot digest does not match ui/signoff.json"

wrong_doctor_artifact_bundle="$(build_evidence_mutation_case \
  doctor-wrong-artifact doctor/mac-server.json \
  '.artifact.sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"')"
expect_failure "${wrong_doctor_artifact_bundle}" "${DOCTOR_FAILURE}"

warn_doctor_check_bundle="$(build_evidence_mutation_case \
  doctor-warning-check doctor/mac-server.json \
  '(.doctor.checks[] | select(.name == "active_compartment") | .status) = "warn"')"
expect_failure "${warn_doctor_check_bundle}" "${DOCTOR_FAILURE}"

wrong_doctor_rc_bundle="$(build_evidence_mutation_case \
  doctor-wrong-rc doctor/mac-server.json \
  '.rc.tag_object = "3333333333333333333333333333333333333333"')"
expect_failure "${wrong_doctor_rc_bundle}" "${DOCTOR_FAILURE}"

REPLAY_CASE="${TMP_ROOT}/f6-replay"
mkdir -p "${REPLAY_CASE}"
write_payload "${REPLAY_CASE}/payload"
jq '.executions[1].tx_hash = .executions[0].tx_hash' \
  "${REPLAY_CASE}/payload/f6/receipts.json" > "${REPLAY_CASE}/receipts.tmp"
mv "${REPLAY_CASE}/receipts.tmp" "${REPLAY_CASE}/payload/f6/receipts.json"
generate_sums "${REPLAY_CASE}/payload"
archive_payload "${REPLAY_CASE}/payload" "${REPLAY_CASE}/${BUNDLE_NAME}"
expect_failure "${REPLAY_CASE}/${BUNDLE_NAME}" \
  "${F6_FAILURE}"

MALFORMED_AUDIT_CASE="${TMP_ROOT}/f6-malformed-audit"
mkdir -p "${MALFORMED_AUDIT_CASE}"
write_payload "${MALFORMED_AUDIT_CASE}/payload"
jq '.audit_chain_verified = false' \
  "${MALFORMED_AUDIT_CASE}/payload/f6/audit/native-sweep.json" > \
  "${MALFORMED_AUDIT_CASE}/audit.tmp"
mv "${MALFORMED_AUDIT_CASE}/audit.tmp" \
  "${MALFORMED_AUDIT_CASE}/payload/f6/audit/native-sweep.json"
generate_sums "${MALFORMED_AUDIT_CASE}/payload"
archive_payload "${MALFORMED_AUDIT_CASE}/payload" \
  "${MALFORMED_AUDIT_CASE}/${BUNDLE_NAME}"
expect_failure "${MALFORMED_AUDIT_CASE}/${BUNDLE_NAME}" \
  "F6 audit export does not match its execution receipt"

WRONG_CHAIN_CASE="${TMP_ROOT}/f6-wrong-chain"
mkdir -p "${WRONG_CHAIN_CASE}"
write_payload "${WRONG_CHAIN_CASE}/payload"
jq '(.networks[] | select(.role == "sepolia") | .chain_id) = 1 |
    (.executions[] | select(.network_role == "sepolia") | .chain_id) = 1' \
  "${WRONG_CHAIN_CASE}/payload/f6/receipts.json" > "${WRONG_CHAIN_CASE}/receipts.tmp"
mv "${WRONG_CHAIN_CASE}/receipts.tmp" \
  "${WRONG_CHAIN_CASE}/payload/f6/receipts.json"
generate_sums "${WRONG_CHAIN_CASE}/payload"
archive_payload "${WRONG_CHAIN_CASE}/payload" "${WRONG_CHAIN_CASE}/${BUNDLE_NAME}"
expect_failure "${WRONG_CHAIN_CASE}/${BUNDLE_NAME}" \
  "${F6_FAILURE}"

for unsupported_l2 in \
  "base-mainnet 8453" \
  "arbitrum-mainnet 42161" \
  "op-mainnet 10" \
  "unknown-chain 999999" \
  "non-integer-chain 84532.5"; do
  read -r case_name chain_id <<< "${unsupported_l2}"
  unsupported_bundle="$(build_valid_case "${case_name}" "${chain_id}")"
  expect_failure "${unsupported_bundle}" "${F6_FAILURE}"
done

legacy_schema_bundle="$(build_receipt_mutation_case \
  f6-legacy-schema-v1 '.schema_version = 1')"
expect_failure "${legacy_schema_bundle}" "${F6_FAILURE}"

single_hash_gas_bundle="$(build_receipt_mutation_case f6-single-hash-gas-chain \
  '(.executions[] | select(.family == "gas_top_up_sweep")) |=
    (del(.legs, .plan_id) + {
      tx_hash: "0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
      audit_export: "f6/audit/gas-top-up.json"
    })')"
expect_failure "${single_hash_gas_bundle}" "${F6_FAILURE}"

reversed_legs_bundle="$(build_receipt_mutation_case \
  f6-reversed-gas-legs \
  '(.executions[] | select(.family == "gas_top_up_sweep") | .legs) |= reverse')"
expect_failure "${reversed_legs_bundle}" "${F6_FAILURE}"

mismatched_address_bundle="$(build_receipt_mutation_case \
  f6-mismatched-gas-address \
  '(.executions[] | select(.family == "gas_top_up_sweep") | .legs[1] |
    .source_address) = "0x4444444444444444444444444444444444444444"')"
expect_failure "${mismatched_address_bundle}" "${F6_FAILURE}"

missing_prerequisite_bundle="$(build_receipt_mutation_case \
  f6-missing-gas-prerequisite \
  '(.executions[] | select(.family == "gas_top_up_sweep") | .legs[1] |
    .prerequisite_job_ids) = []')"
expect_failure "${missing_prerequisite_bundle}" "${F6_FAILURE}"

EXTRA_PREREQUISITE_CASE="${TMP_ROOT}/f6-extra-gas-prerequisite"
mkdir -p "${EXTRA_PREREQUISITE_CASE}"
write_payload "${EXTRA_PREREQUISITE_CASE}/payload"
jq '(.executions[] | select(.family == "gas_top_up_sweep") | .legs[1] |
    .prerequisite_job_ids) += ["job-unevidenced"]' \
  "${EXTRA_PREREQUISITE_CASE}/payload/f6/receipts.json" > \
  "${EXTRA_PREREQUISITE_CASE}/receipts.tmp"
mv "${EXTRA_PREREQUISITE_CASE}/receipts.tmp" \
  "${EXTRA_PREREQUISITE_CASE}/payload/f6/receipts.json"
jq '.prerequisite_job_ids += ["job-unevidenced"]' \
  "${EXTRA_PREREQUISITE_CASE}/payload/f6/audit/gas-dependent-sweep.json" > \
  "${EXTRA_PREREQUISITE_CASE}/audit.tmp"
mv "${EXTRA_PREREQUISITE_CASE}/audit.tmp" \
  "${EXTRA_PREREQUISITE_CASE}/payload/f6/audit/gas-dependent-sweep.json"
generate_sums "${EXTRA_PREREQUISITE_CASE}/payload"
archive_payload "${EXTRA_PREREQUISITE_CASE}/payload" \
  "${EXTRA_PREREQUISITE_CASE}/${BUNDLE_NAME}"
expect_failure "${EXTRA_PREREQUISITE_CASE}/${BUNDLE_NAME}" "${F6_FAILURE}"

duplicate_job_bundle="$(build_receipt_mutation_case f6-duplicate-gas-job \
  '(.executions[] | select(.family == "gas_top_up_sweep") | .legs[1] |
    .job_id) = "job-fund-gas"')"
expect_failure "${duplicate_job_bundle}" "${F6_FAILURE}"

duplicate_leg_hash_bundle="$(build_receipt_mutation_case \
  f6-duplicate-gas-transaction \
  '(.executions[] | select(.family == "gas_top_up_sweep") | .legs[1] |
    .tx_hash) = "0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"')"
expect_failure "${duplicate_leg_hash_bundle}" "${F6_FAILURE}"

reversed_broadcast_bundle="$(build_gas_broadcast_time_case \
  f6-reversed-gas-broadcast-time 999)"
expect_failure "${reversed_broadcast_bundle}" "${F6_FAILURE}"

equal_broadcast_bundle="$(build_gas_broadcast_time_case \
  f6-equal-gas-broadcast-time 1000)"
expect_failure "${equal_broadcast_bundle}" "${F6_FAILURE}"

unordered_blocks_bundle="$(build_receipt_mutation_case \
  f6-unordered-gas-blocks \
  '(.executions[] | select(.family == "gas_top_up_sweep") | .legs[1] |
    .receipt_block_number) = 100')"
expect_failure "${unordered_blocks_bundle}" "${F6_FAILURE}"

zero_confirmations_bundle="$(build_receipt_mutation_case \
  f6-zero-gas-confirmations \
  '(.executions[] | select(.family == "gas_top_up_sweep") | .legs[0] |
    .confirmations) = 0')"
expect_failure "${zero_confirmations_bundle}" "${F6_FAILURE}"

wrong_dependent_action_bundle="$(build_receipt_mutation_case \
  f6-wrong-dependent-action \
  '(.executions[] | select(.family == "gas_top_up_sweep") | .legs[1] |
    .action) = "revoke_erc20_approval"')"
expect_failure "${wrong_dependent_action_bundle}" "${F6_FAILURE}"

leg_chain_mismatch_bundle="$(build_receipt_mutation_case \
  f6-gas-leg-chain-mismatch \
  '(.executions[] | select(.family == "gas_top_up_sweep") | .legs[1] |
    .chain_id) = 11155111')"
expect_failure "${leg_chain_mismatch_bundle}" "${F6_FAILURE}"

GAS_AUDIT_MISMATCH_CASE="${TMP_ROOT}/f6-gas-audit-mismatch"
mkdir -p "${GAS_AUDIT_MISMATCH_CASE}"
write_payload "${GAS_AUDIT_MISMATCH_CASE}/payload"
jq '.job_id = "unrelated-job"' \
  "${GAS_AUDIT_MISMATCH_CASE}/payload/f6/audit/gas-dependent-sweep.json" > \
  "${GAS_AUDIT_MISMATCH_CASE}/audit.tmp"
mv "${GAS_AUDIT_MISMATCH_CASE}/audit.tmp" \
  "${GAS_AUDIT_MISMATCH_CASE}/payload/f6/audit/gas-dependent-sweep.json"
generate_sums "${GAS_AUDIT_MISMATCH_CASE}/payload"
archive_payload "${GAS_AUDIT_MISMATCH_CASE}/payload" \
  "${GAS_AUDIT_MISMATCH_CASE}/${BUNDLE_NAME}"
expect_failure "${GAS_AUDIT_MISMATCH_CASE}/${BUNDLE_NAME}" \
  "F6 audit export does not match its execution receipt"

F6_CASE="${TMP_ROOT}/f6-missing-family"
mkdir -p "${F6_CASE}"
write_payload "${F6_CASE}/payload"
jq 'del(.executions[] | select(.family == "erc20_revoke"))' \
  "${F6_CASE}/payload/f6/receipts.json" > "${F6_CASE}/receipts.tmp"
mv "${F6_CASE}/receipts.tmp" "${F6_CASE}/payload/f6/receipts.json"
generate_sums "${F6_CASE}/payload"
archive_payload "${F6_CASE}/payload" "${F6_CASE}/${BUNDLE_NAME}"
expect_failure "${F6_CASE}/${BUNDLE_NAME}" \
  "${F6_FAILURE}"

echo "release evidence bundle tests passed"
