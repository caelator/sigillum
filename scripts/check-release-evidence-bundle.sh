#!/usr/bin/env bash
set -euo pipefail

fail() {
  echo "release evidence bundle failed: $*" >&2
  exit 1
}

BUNDLE="${1:-}"
RC_TAG="${2:-}"
RC_SHA="${3:-}"
RC_TAG_OBJECT="${4:-}"

[[ -n "${BUNDLE}" ]] || fail "bundle path is required"
[[ -f "${BUNDLE}" ]] || fail "bundle does not exist: ${BUNDLE}"
[[ "${RC_SHA}" =~ ^[0-9a-f]{40}$ ]] || fail "RC SHA must be 40 lowercase hexadecimal characters"
[[ "${RC_TAG_OBJECT}" =~ ^[0-9a-f]{40}$ ]] ||
  fail "RC tag object must be 40 lowercase hexadecimal characters"

ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || fail "not inside a git repository"
VERSION="$(awk -F '"' '/^version = "/ { print $2; exit }' "${ROOT}/Cargo.toml")"
[[ -n "${VERSION}" ]] || fail "workspace version is missing from Cargo.toml"
[[ "${RC_TAG}" =~ ^v${VERSION//./\.}-rc\.[1-9][0-9]*$ ]] ||
  fail "RC tag does not match workspace version ${VERSION}: ${RC_TAG}"

EXPECTED_BASENAME="sigillum-v${VERSION}-release-evidence.tar.gz"
[[ "$(basename -- "${BUNDLE}")" == "${EXPECTED_BASENAME}" ]] ||
  fail "bundle filename must be ${EXPECTED_BASENAME}"

for command in awk cmp find git jq shasum sort stat tar uniq; do
  command -v "${command}" >/dev/null 2>&1 || fail "required command is missing: ${command}"
done

TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/sigillum-release-evidence.XXXXXX")"
cleanup() {
  rm -rf "${TMP_ROOT}"
}
trap cleanup EXIT

ARCHIVE_LIST="${TMP_ROOT}/archive-list"
NORMALIZED_LIST="${TMP_ROOT}/normalized-list"
ARCHIVE_VERBOSE="${TMP_ROOT}/archive-verbose"
EXTRACTED="${TMP_ROOT}/extracted"
mkdir -p "${EXTRACTED}"

tar -tzf "${BUNDLE}" > "${ARCHIVE_LIST}" || fail "bundle is not a readable gzip tar archive"
[[ -s "${ARCHIVE_LIST}" ]] || fail "bundle archive is empty"

: > "${NORMALIZED_LIST}"
while IFS= read -r archive_path || [[ -n "${archive_path}" ]]; do
  normalized_path="${archive_path#./}"
  [[ -n "${normalized_path}" && "${normalized_path}" != "." ]] || continue
  [[ "${normalized_path}" =~ ^[A-Za-z0-9._/-]+/?$ ]] ||
    fail "bundle contains a non-portable path: ${archive_path}"
  [[ "${normalized_path}" != /* && "${normalized_path}" != *"//"* ]] ||
    fail "bundle contains an unsafe path: ${archive_path}"
  [[ "/${normalized_path}/" != *"/./"* ]] ||
    fail "bundle contains a non-canonical path: ${archive_path}"
  [[ "/${normalized_path}/" != *"/../"* ]] ||
    fail "bundle contains a parent-directory path: ${archive_path}"
  printf '%s\n' "${normalized_path%/}" >> "${NORMALIZED_LIST}"
done < "${ARCHIVE_LIST}"

DUPLICATE_PATHS="$(sort "${NORMALIZED_LIST}" | uniq -d)"
[[ -z "${DUPLICATE_PATHS}" ]] || fail "bundle contains duplicate paths: ${DUPLICATE_PATHS}"
CASEFOLDED_DUPLICATE_PATHS="$(
  LC_ALL=C tr '[:upper:]' '[:lower:]' < "${NORMALIZED_LIST}" |
    LC_ALL=C sort | uniq -d
)"
[[ -z "${CASEFOLDED_DUPLICATE_PATHS}" ]] ||
  fail "bundle contains case-folded path collisions: ${CASEFOLDED_DUPLICATE_PATHS}"

tar -tvzf "${BUNDLE}" > "${ARCHIVE_VERBOSE}" || fail "could not inspect bundle entry types"
while IFS= read -r verbose_line || [[ -n "${verbose_line}" ]]; do
  entry_type="${verbose_line:0:1}"
  [[ "${entry_type}" == "-" || "${entry_type}" == "d" ]] ||
    fail "bundle contains a linked or special archive member"
done < "${ARCHIVE_VERBOSE}"

tar -xzf "${BUNDLE}" -C "${EXTRACTED}" || fail "could not extract validated bundle"

if stat -f '%l' "${EXTRACTED}" >/dev/null 2>&1; then
  STAT_STYLE="bsd"
else
  STAT_STYLE="gnu"
fi
while IFS= read -r -d '' extracted_file; do
  if [[ "${STAT_STYLE}" == "bsd" ]]; then
    LINK_COUNT="$(stat -f '%l' "${extracted_file}")"
  else
    LINK_COUNT="$(stat -c '%h' "${extracted_file}")"
  fi
  [[ "${LINK_COUNT}" -eq 1 ]] || fail "bundle contains a linked or special archive member"
done < <(find "${EXTRACTED}" -type f -print0)

REQUIRED_FILES=(
  "MANIFEST.json"
  "SHA256SUMS"
  "f4/standard.json"
  "f4/chaos.json"
  "f6/receipts.json"
  "desktop/clean-install.json"
  "doctor/mac-server.json"
  "ui/signoff.json"
  "release/asset-SHA256SUMS"
)
for required_file in "${REQUIRED_FILES[@]}"; do
  [[ -s "${EXTRACTED}/${required_file}" ]] ||
    fail "required evidence file is missing or empty: ${required_file}"
done

SUM_FILES="${TMP_ROOT}/sum-files"
ACTUAL_FILES="${TMP_ROOT}/actual-files"
: > "${SUM_FILES}"
while IFS= read -r checksum_line || [[ -n "${checksum_line}" ]]; do
  [[ "${checksum_line}" =~ ^([0-9a-f]{64})\ \ ([A-Za-z0-9._/-]+)$ ]] ||
    fail "SHA256SUMS contains a malformed line"
  checksum_path="${BASH_REMATCH[2]}"
  [[ "${checksum_path}" != "SHA256SUMS" ]] || fail "SHA256SUMS must not checksum itself"
  [[ "${checksum_path}" != /* && "/${checksum_path}/" != *"/../"* ]] ||
    fail "SHA256SUMS contains an unsafe path: ${checksum_path}"
  printf '%s\n' "${checksum_path}" >> "${SUM_FILES}"
done < "${EXTRACTED}/SHA256SUMS"

[[ -s "${SUM_FILES}" ]] || fail "SHA256SUMS has no payload entries"
[[ -z "$(sort "${SUM_FILES}" | uniq -d)" ]] || fail "SHA256SUMS contains duplicate payload paths"

(
  cd "${EXTRACTED}"
  find . -type f -print |
    awk '{ sub(/^\.\//, ""); if ($0 != "SHA256SUMS") print }' |
    sort > "${ACTUAL_FILES}"
)
sort -o "${SUM_FILES}" "${SUM_FILES}"
cmp -s "${SUM_FILES}" "${ACTUAL_FILES}" ||
  fail "SHA256SUMS must cover every payload file exactly once"
(
  cd "${EXTRACTED}"
  shasum -a 256 --check SHA256SUMS >/dev/null
) || fail "bundle payload checksum verification failed"

jq -e \
  --arg version "${VERSION}" \
  --arg rc_tag "${RC_TAG}" \
  --arg rc_sha "${RC_SHA}" \
  --arg rc_tag_object "${RC_TAG_OBJECT}" '
    .schema_version == 1 and
    .kind == "sigillum.release_evidence" and
    .release_version == $version and
    .rc_tag == $rc_tag and
    .rc_peeled_sha == $rc_sha and
    .rc_tag_object == $rc_tag_object and
    (.release_workflow_run | type == "number" and . > 0) and
    .asset_checksums_verified == true and
    .gates.f4 == "passed" and
    .gates.f6 == "passed" and
    .gates.desktop == "passed" and
    .gates.doctor == "passed" and
    .gates.ui == "passed"
  ' "${EXTRACTED}/MANIFEST.json" >/dev/null ||
  fail "MANIFEST.json does not bind every required gate to the exact RC identities"

F4_HOST_FILTER='
  .host.name == "mac-server" and
  .host.platform == "macos" and
  (.host.product_version |
    type == "string" and
    test("^15\\.[0-9]+(\\.[0-9]+)?$")) and
  .host.arch == "aarch64" and
  (.host.identity_sha256 |
    type == "string" and test("^[0-9a-f]{64}$"))
'

jq -e --arg rc_sha "${RC_SHA}" "${F4_HOST_FILTER}"'
    and
    .schema_version == 2 and
    .kind == "sigillum.local_soak" and
    .status == "passed" and
    .repo.commit == $rc_sha and
    .repo.dirty == false and
    (.configured.soak_seconds |
      type == "number" and . == floor and . >= 3600) and
    (.timing.duration_seconds |
      type == "number" and . == floor and . >= 3600) and
    (.evidence.iterations | type == "number" and . == floor and . > 0) and
    (.evidence.doctor_runs | type == "number" and . == floor and . > 0) and
    .chaos.enabled == false
  ' "${EXTRACTED}/f4/standard.json" >/dev/null ||
  fail "F4 standard receipt does not prove the required clean 3600-second RC soak"

jq -e --arg rc_sha "${RC_SHA}" "${F4_HOST_FILTER}"'
    and
    .schema_version == 2 and
    .kind == "sigillum.local_soak" and
    .status == "passed" and
    .repo.commit == $rc_sha and
    .repo.dirty == false and
    (.configured.soak_seconds |
      type == "number" and . == floor and . >= 600) and
    (.timing.duration_seconds |
      type == "number" and . == floor and . >= 600) and
    (.evidence.iterations | type == "number" and . == floor and . > 0) and
    (.evidence.doctor_runs | type == "number" and . == floor and . > 0) and
    .chaos.enabled == true and
    (.chaos.kill_cycles | type == "number" and . == floor and . >= 2) and
    .chaos.in_flight_assertion.status == "passed"
  ' "${EXTRACTED}/f4/chaos.json" >/dev/null ||
  fail "F4 chaos receipt does not prove the required clean 600-second RC soak"

STANDARD_HOST_IDENTITY="$(
  jq -r '.host.identity_sha256' "${EXTRACTED}/f4/standard.json"
)"
CHAOS_HOST_IDENTITY="$(
  jq -r '.host.identity_sha256' "${EXTRACTED}/f4/chaos.json"
)"
[[ "${STANDARD_HOST_IDENTITY}" == "${CHAOS_HOST_IDENTITY}" ]] ||
  fail "F4 standard and chaos receipts do not bind the same host identity"

jq -e --arg rc_sha "${RC_SHA}" '
    def flattened_claims:
      [.executions[] |
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
        end];
    .networks as $networks |
    .schema_version == 2 and
    .kind == "sigillum.testnet_execution" and
    .status == "passed" and
    .rc_sha == $rc_sha and
    (.networks | type == "array" and length == 2) and
    ([.networks[].role] | sort) == ["l2", "sepolia"] and
    any(.networks[];
      .role == "sepolia" and
      (.chain_id |
        type == "number" and
        . == floor and
        . == 11155111)) and
    any(.networks[];
      .role == "l2" and
      (.chain_id |
        type == "number" and
        . == floor and
        (. as $id |
          [84532, 421614, 11155420] | index($id) != null))) and
    (.executions | type == "array" and length == 4) and
    ([.executions[].family] | sort) ==
      ["erc20_revoke", "erc20_sweep", "gas_top_up_sweep", "native_sweep"] and
    (["sepolia", "l2"] - [.executions[].network_role] | length == 0) and
    all(.executions[];
      . as $execution |
      any($networks[];
        .role == $execution.network_role and
        .chain_id == $execution.chain_id)) and
    all(.executions[] | select(.family != "gas_top_up_sweep");
      .status == "confirmed" and
      (has("legs") | not) and
      (.tx_hash | test("^0x[0-9a-f]{64}$")) and
      (.audit_export | test("^f6/audit/[A-Za-z0-9._-]+$"))) and
    (
      (.executions[] | select(.family == "gas_top_up_sweep")) as $gas_chain |
      ($gas_chain | has("tx_hash") | not) and
      ($gas_chain | has("audit_export") | not) and
      ($gas_chain.plan_id | type == "string" and length > 0) and
      $gas_chain.status == "confirmed" and
      ($gas_chain.legs | type == "array" and length == 2) and
      [$gas_chain.legs[].role] == ["fund_gas", "dependent_sweep"] and
      all($gas_chain.legs[];
        .plan_id == $gas_chain.plan_id and
        .network_role == $gas_chain.network_role and
        .chain_id == $gas_chain.chain_id and
        (.step_id | type == "string" and length > 0) and
        (.job_id | type == "string" and length > 0) and
        (.source_address | test("^0x[0-9a-f]{40}$"; "i")) and
        (.destination_address | test("^0x[0-9a-f]{40}$"; "i")) and
        (.prerequisite_job_ids | type == "array") and
        all(.prerequisite_job_ids[]; type == "string" and length > 0) and
        ([.prerequisite_job_ids[]] | length == (unique | length)) and
        .queue_state == "confirmed" and
        .receipt_status == "success" and
        (.confirmations |
          type == "number" and . == floor and . > 0) and
        (.receipt_block_number |
          type == "number" and . == floor and . >= 0) and
        (.broadcast_at_unix |
          type == "number" and . == floor and . > 0) and
        (.tx_hash | test("^0x[0-9a-f]{64}$")) and
        (.audit_export | test("^f6/audit/[A-Za-z0-9._-]+$"))) and
      ($gas_chain.legs[0] as $fund |
        $gas_chain.legs[1] as $sweep |
        $fund.action == "fund_gas" and
        ($sweep.action == "sweep_native" or $sweep.action == "sweep_erc20") and
        $fund.prerequisite_job_ids == [] and
        $sweep.prerequisite_job_ids == [$fund.job_id] and
        $fund.job_id != $sweep.job_id and
        $fund.step_id != $sweep.step_id and
        ($fund.destination_address | ascii_downcase) ==
          ($sweep.source_address | ascii_downcase) and
        $fund.broadcast_at_unix < $sweep.broadcast_at_unix and
        $fund.receipt_block_number < $sweep.receipt_block_number)
    ) and
    (flattened_claims as $claims |
      ($claims | length == 5) and
      ([$claims[].tx_hash] | length == (unique | length)) and
      ([$claims[].audit_export] | length == (unique | length)))
  ' "${EXTRACTED}/f6/receipts.json" >/dev/null ||
  fail "F6 receipt does not prove four core families and both ordered gas-top-up chain legs across supported testnets"

while IFS= read -r claim; do
  audit_export="$(jq -r '.audit_export' <<<"${claim}")"
  [[ -s "${EXTRACTED}/${audit_export}" ]] ||
    fail "F6 audit export is missing or empty: ${audit_export}"
  jq -e \
    --arg rc_sha "${RC_SHA}" \
    --argjson claim "${claim}" '
      .schema_version == 2 and
      .kind == "sigillum.execution_audit" and
      .status == "verified" and
      .rc_sha == $rc_sha and
      .family == $claim.family and
      .network_role == $claim.network_role and
      .chain_id == $claim.chain_id and
      .tx_hash == $claim.tx_hash and
      .audit_chain_verified == true and
      (if $claim.family == "gas_top_up_sweep" then
        .leg_role == $claim.role and
        .plan_id == $claim.plan_id and
        .step_id == $claim.step_id and
        .job_id == $claim.job_id and
        .action == $claim.action and
        (.source_address | ascii_downcase) ==
          ($claim.source_address | ascii_downcase) and
        (.destination_address | ascii_downcase) ==
          ($claim.destination_address | ascii_downcase) and
        .prerequisite_job_ids == $claim.prerequisite_job_ids and
        .queue_state == $claim.queue_state and
        .receipt_status == $claim.receipt_status and
        .confirmations == $claim.confirmations and
        .receipt_block_number == $claim.receipt_block_number and
        .broadcast_at_unix == $claim.broadcast_at_unix
      else
        true
      end)
    ' "${EXTRACTED}/${audit_export}" >/dev/null ||
    fail "F6 audit export does not match its execution receipt: ${audit_export}"
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
  end' "${EXTRACTED}/f6/receipts.json")

EXPECTED_RC_ASSETS="${TMP_ROOT}/expected-rc-assets"
RECORDED_RC_ASSETS="${TMP_ROOT}/recorded-rc-assets"
printf '%s\n' \
  "Sigillum-${RC_TAG}-macos-aarch64.app.zip" \
  "Sigillum-${RC_TAG}-macos-aarch64.dmg" \
  "THIRD-PARTY-NOTICES.txt" \
  "sigillum-cli-${RC_TAG}-linux-x86_64.tar.gz" \
  "sigillum-cli-${RC_TAG}-macos-aarch64.tar.gz" |
  LC_ALL=C sort > "${EXPECTED_RC_ASSETS}"
: > "${RECORDED_RC_ASSETS}"
while IFS= read -r rc_checksum_line || [[ -n "${rc_checksum_line}" ]]; do
  [[ "${rc_checksum_line}" =~ ^[0-9a-f]{64}\ \ ([A-Za-z0-9._-]+)$ ]] ||
    fail "release/asset-SHA256SUMS contains a malformed line"
  printf '%s\n' "${BASH_REMATCH[1]}" >> "${RECORDED_RC_ASSETS}"
done < "${EXTRACTED}/release/asset-SHA256SUMS"
LC_ALL=C sort -o "${RECORDED_RC_ASSETS}" "${RECORDED_RC_ASSETS}"
cmp -s "${EXPECTED_RC_ASSETS}" "${RECORDED_RC_ASSETS}" ||
  fail "release/asset-SHA256SUMS must contain the exact five RC payload assets"

RC_DMG_NAME="Sigillum-${RC_TAG}-macos-aarch64.dmg"
RC_CLI_NAME="sigillum-cli-${RC_TAG}-macos-aarch64.tar.gz"
RC_DMG_SHA256="$(
  awk -v name="${RC_DMG_NAME}" '$2 == name { print $1 }' \
    "${EXTRACTED}/release/asset-SHA256SUMS"
)"
RC_CLI_SHA256="$(
  awk -v name="${RC_CLI_NAME}" '$2 == name { print $1 }' \
    "${EXTRACTED}/release/asset-SHA256SUMS"
)"
[[ "${RC_DMG_SHA256}" =~ ^[0-9a-f]{64}$ ]] ||
  fail "release/asset-SHA256SUMS is missing the RC dmg digest"
[[ "${RC_CLI_SHA256}" =~ ^[0-9a-f]{64}$ ]] ||
  fail "release/asset-SHA256SUMS is missing the RC macOS CLI digest"

REVIEWER_AND_HOST_FILTER='
  (.reviewer.id |
    type == "string" and test("^[A-Za-z0-9._@-]{1,128}$")) and
  .reviewer.role == "release_operator" and
  (.reviewer.reviewed_at_utc |
    type == "string" and
    test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$")) and
  .host.role == "mac-server" and
  .host.name == "mac-server" and
  .host.platform == "macos" and
  (.host.os_version |
    type == "string" and test("^15(\\.|$)")) and
  .host.arch == "aarch64"
'

jq -e \
  --arg version "${VERSION}" \
  --arg rc_tag "${RC_TAG}" \
  --arg rc_sha "${RC_SHA}" \
  --arg rc_tag_object "${RC_TAG_OBJECT}" \
  --arg artifact_name "${RC_DMG_NAME}" \
  --arg artifact_sha256 "${RC_DMG_SHA256}" \
  "${REVIEWER_AND_HOST_FILTER} and
    .schema_version == 2 and
    .kind == \"sigillum.clean_install\" and
    .status == \"passed\" and
    .rc.tag == \$rc_tag and
    .rc.tag_object == \$rc_tag_object and
    .rc.peeled_sha == \$rc_sha and
    .artifact.filename == \$artifact_name and
    .artifact.sha256 == \$artifact_sha256 and
    .installation.path == \"/Applications/Sigillum.app\" and
    .installation.bundle_identifier == \"com.sigillum.desktop\" and
    .installation.app_version == \$version and
    .installation.checksum_verified == true and
    .installation.dev_toolchain_absent == true and
    .installation.unlock_reached == true and
    (.screenshots | type == \"array\" and length == 1) and
    .screenshots[0].state == \"unlock\" and
    .screenshots[0].path == \"desktop/screenshots/unlock.png\" and
    (.screenshots[0].sha256 | test(\"^[0-9a-f]{64}$\"))
  " "${EXTRACTED}/desktop/clean-install.json" >/dev/null ||
  fail "desktop receipt does not bind the qualified RC dmg, supported clean host, unlock screenshot, and operator review"

jq -e \
  --arg rc_tag "${RC_TAG}" \
  --arg rc_sha "${RC_SHA}" \
  --arg rc_tag_object "${RC_TAG_OBJECT}" \
  --arg artifact_name "${RC_DMG_NAME}" \
  --arg artifact_sha256 "${RC_DMG_SHA256}" \
  "${REVIEWER_AND_HOST_FILTER} and
    .schema_version == 2 and
    .kind == \"sigillum.ui_signoff\" and
    .status == \"passed\" and
    .rc.tag == \$rc_tag and
    .rc.tag_object == \$rc_tag_object and
    .rc.peeled_sha == \$rc_sha and
    .artifact.filename == \$artifact_name and
    .artifact.sha256 == \$artifact_sha256 and
    .walkthrough.full_walkthrough_completed == true and
    .walkthrough.destinations ==
      [\"overview\", \"receive\", \"portfolio\", \"move\", \"vault\"] and
    .walkthrough.states == [\"setup\", \"locked\", \"unlocked\"] and
    .walkthrough.journey == [
      \"import_seed\",
      \"multi_chain_scan\",
      \"review_inventory_risk\",
      \"generate_plan\",
      \"approve_plan\",
      \"execute_mock_provider\",
      \"audit_trail_complete\"
    ] and
    .walkthrough.operator_surface_parity_reviewed == true and
    .walkthrough.accessibility_review_completed == true and
    (.screenshots | type == \"array\" and length == 3) and
    [.screenshots[].state] == [\"setup\", \"locked\", \"unlocked\"] and
    [.screenshots[].path] == [
      \"ui/screenshots/setup.png\",
      \"ui/screenshots/locked.png\",
      \"ui/screenshots/unlocked.png\"
    ] and
    all(.screenshots[]; (.sha256 | test(\"^[0-9a-f]{64}$\")))
  " "${EXTRACTED}/ui/signoff.json" >/dev/null ||
  fail "UI receipt does not bind the qualified RC dmg, five-destination journey, screenshots, and operator review"

jq -e \
  --arg version "${VERSION}" \
  --arg rc_tag "${RC_TAG}" \
  --arg rc_sha "${RC_SHA}" \
  --arg rc_tag_object "${RC_TAG_OBJECT}" \
  --arg artifact_name "${RC_CLI_NAME}" \
  --arg artifact_sha256 "${RC_CLI_SHA256}" \
  "${REVIEWER_AND_HOST_FILTER} and
    .schema_version == 2 and
    .kind == \"sigillum.doctor\" and
    .status == \"passed\" and
    .rc.tag == \$rc_tag and
    .rc.tag_object == \$rc_tag_object and
    .rc.peeled_sha == \$rc_sha and
    .artifact.filename == \$artifact_name and
    .artifact.sha256 == \$artifact_sha256 and
    .cli.version == \$version and
    (.cli.executable_path |
      type == \"string\" and startswith(\"/\") and length > 1) and
    (.cli.executable_sha256 | test(\"^[0-9a-f]{64}$\")) and
    .doctor.command == \"sigillum doctor\" and
    .doctor.exit_code == 0 and
    .doctor.checks_passed == true and
    (.doctor.checks | type == \"array\" and length == 7) and
    ([.doctor.checks[].name] | sort) == [
      \"active_compartment\",
      \"audit_db\",
      \"daemon_reachability\",
      \"daemon_url\",
      \"data_dir\",
      \"data_dir_permissions\",
      \"session_token\"
    ] and
    all(.doctor.checks[]; .status == \"ok\")
  " "${EXTRACTED}/doctor/mac-server.json" >/dev/null ||
  fail "doctor receipt does not bind the qualified RC CLI, supported host, all blocking checks, and operator review"

verify_bound_evidence_file() {
  local receipt_path="$1"
  local evidence_path="$2"
  local expected_sha256="$3"
  local label="$4"
  local actual_sha256
  [[ -s "${EXTRACTED}/${evidence_path}" ]] ||
    fail "${label} is missing or empty: ${evidence_path}"
  actual_sha256="$(shasum -a 256 "${EXTRACTED}/${evidence_path}" | awk '{ print $1 }')"
  [[ "${actual_sha256}" == "${expected_sha256}" ]] ||
    fail "${label} digest does not match ${receipt_path}: ${evidence_path}"
}

while IFS=$'\t' read -r screenshot_path screenshot_sha256; do
  verify_bound_evidence_file \
    "desktop/clean-install.json" "${screenshot_path}" "${screenshot_sha256}" \
    "desktop unlock screenshot"
done < <(jq -r '.screenshots[] | [.path, .sha256] | @tsv' \
  "${EXTRACTED}/desktop/clean-install.json")

while IFS=$'\t' read -r screenshot_path screenshot_sha256; do
  verify_bound_evidence_file \
    "ui/signoff.json" "${screenshot_path}" "${screenshot_sha256}" \
    "UI walkthrough screenshot"
done < <(jq -r '.screenshots[] | [.path, .sha256] | @tsv' \
  "${EXTRACTED}/ui/signoff.json")

BUNDLE_SHA256="$(shasum -a 256 "${BUNDLE}" | awk '{print $1}')"
[[ "${BUNDLE_SHA256}" =~ ^[0-9a-f]{64}$ ]] || fail "could not compute bundle SHA-256"
echo "release evidence bundle passed: ${EXPECTED_BASENAME} sha256=${BUNDLE_SHA256}"
