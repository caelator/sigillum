#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

PORT="${SIGILLUM_RUNTIME_SMOKE_PORT:-19743}"
URL="http://127.0.0.1:${PORT}"
BASE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sigillum-runtime-smoke.XXXXXX")"
LOG_FILE="${BASE_DIR}/daemon.log"
DAEMON_PID=""
PASSPHRASE="runtime-smoke-passphrase-123"
API_KEY_NAME="runtime_rpc_canary"
API_KEY_VALUE="runtime-rpc-canary-value"
SECRET_NAME="runtime_secret_canary"
SECRET_VALUE="runtime-secret-canary-value"

cleanup() {
  if [[ -n "${DAEMON_PID}" ]] && kill -0 "${DAEMON_PID}" >/dev/null 2>&1; then
    kill "${DAEMON_PID}" >/dev/null 2>&1 || true
    wait "${DAEMON_PID}" >/dev/null 2>&1 || true
  fi
  rm -rf "${BASE_DIR}"
}
trap cleanup EXIT

fail() {
  echo "runtime smoke failed: $*" >&2
  if [[ -f "${LOG_FILE}" ]]; then
    echo "--- daemon log ---" >&2
    tail -n 80 "${LOG_FILE}" >&2 || true
  fi
  exit 1
}

require_command() {
  local command_name="$1"
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    fail "required command is missing: ${command_name}"
  fi
}

json_assert() {
  local json="$1"
  local label="$2"
  local predicate="$3"
  node -e '
const fs = require("fs");
const label = process.argv[1];
const predicate = process.argv[2];
const input = fs.readFileSync(0, "utf8");
const data = JSON.parse(input);
const ok = Function("data", predicate)(data);
if (!ok) {
  console.error(`json assertion failed: ${label}`);
  console.error(JSON.stringify(data, null, 2));
  process.exit(1);
}
' "${label}" "${predicate}" <<< "${json}" || fail "${label}"
}

json_session_token() {
  node -e '
const fs = require("fs");
const data = JSON.parse(fs.readFileSync(0, "utf8"));
if (typeof data.session_token !== "string" || data.session_token.length === 0) {
  console.error("session_token missing");
  console.error(JSON.stringify(data, null, 2));
  process.exit(1);
}
console.log(data.session_token);
' <<< "$1"
}

api_get() {
  local path="$1"
  local token="${2:-}"
  if [[ -n "${token}" ]]; then
    curl -fsS -H "Authorization: Bearer ${token}" "${URL}${path}"
  else
    curl -fsS "${URL}${path}"
  fi
}

api_post() {
  local path="$1"
  local body="$2"
  local token="${3:-}"
  if [[ -n "${token}" ]]; then
    curl -fsS \
      -X POST \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer ${token}" \
      --data "${body}" \
      "${URL}${path}"
  else
    curl -fsS \
      -X POST \
      -H "Content-Type: application/json" \
      --data "${body}" \
      "${URL}${path}"
  fi
}

wait_for_daemon() {
  # 240 x 0.5s = 120s ceiling: cold CI runners need headroom; warm hosts
  # pass on the first few probes either way.
  for _ in {1..240}; do
    if curl -fsS "${URL}/api/status" >/dev/null 2>&1; then
      return
    fi
    if ! kill -0 "${DAEMON_PID}" >/dev/null 2>&1; then
      fail "daemon exited before becoming ready"
    fi
    sleep 0.5
  done
  fail "daemon did not become ready at ${URL}"
}

run_doctor() {
  local token="${1:-}"
  if [[ -n "${token}" ]]; then
    SIGILLUM_BASE_DIR="${BASE_DIR}" \
      SIGILLUM_SESSION_TOKEN="${token}" \
      cargo run -p sigillum-cli --quiet -- doctor --url "${URL}" >/dev/null
  else
    SIGILLUM_BASE_DIR="${BASE_DIR}" \
      cargo run -p sigillum-cli --quiet -- doctor --url "${URL}" >/dev/null
  fi
}

verify_vault_canaries() {
  local token="$1"
  local label="$2"

  local api_key_get
  api_key_get="$(api_post "/api/api-keys/get" "{\"key\":\"${API_KEY_NAME}\"}" "${token}")"
  json_assert "${api_key_get}" "api key canary get ${label}" "
return data.key === '${API_KEY_NAME}' &&
  data.value === '${API_KEY_VALUE}';
"

  local secret_get
  secret_get="$(api_post "/api/secrets/get" "{\"key\":\"${SECRET_NAME}\"}" "${token}")"
  json_assert "${secret_get}" "secret canary get ${label}" "
return data.key === '${SECRET_NAME}' &&
  data.value === '${SECRET_VALUE}';
"
}

require_command cargo
require_command curl
require_command node

echo "==> building sigillum-cli for runtime smoke"
# Build before launching: `cargo run --quiet` compiles silently into the
# daemon log, which can eat the entire readiness window on cold runners
# and leaves an empty log when it times out.
cargo build -p sigillum-cli

echo "==> starting sigillum daemon runtime smoke on ${URL}"
SIGILLUM_BASE_DIR="${BASE_DIR}" \
  cargo run -p sigillum-cli --quiet -- daemon --port "${PORT}" >"${LOG_FILE}" 2>&1 &
DAEMON_PID="$!"
wait_for_daemon

html="$(curl -fsS "${URL}/")"
[[ "${html}" == *"Sigillum Vault"* ]] || fail "daemon UI shell did not render expected title"
[[ "${html}" == *"id=\"statusCard\""* ]] || fail "daemon UI shell is missing status card"
[[ "${html}" == *"/api/status"* ]] || fail "daemon UI shell is missing API status wiring"

fresh_status="$(api_get "/api/status")"
json_assert "${fresh_status}" "fresh first-run status" '
return data.initialized === false &&
  data.locked === true &&
  Array.isArray(data.unlocked_compartments) &&
  data.unlocked_compartments.length === 0;
'
run_doctor

init_body='{"id":0,"passphrase":"runtime-smoke-passphrase-123","label":"runtime-smoke","threshold":1}'
init_response="$(api_post "/api/compartment/init" "${init_body}")"
json_assert "${init_response}" "passphrase compartment init" '
return data.status === "initialized" &&
  data.compartment_id === 0 &&
  data.compartment_label === "runtime-smoke" &&
  typeof data.session_token === "string" &&
  data.session_token.length > 0;
'
session_token="$(json_session_token "${init_response}")"

unlocked_status="$(api_get "/api/status" "${session_token}")"
json_assert "${unlocked_status}" "initialized unlocked status" '
return data.initialized === true &&
  data.locked === false &&
  Array.isArray(data.unlocked_compartments) &&
  data.unlocked_compartments.length === 1 &&
  data.active_compartment &&
  data.active_compartment.compartment_id === 0;
'

compartments="$(api_get "/api/compartment/list" "${session_token}")"
json_assert "${compartments}" "compartment list after init" '
return Array.isArray(data.compartments) &&
  data.compartments.length === 1 &&
  data.compartments[0].id === 0 &&
  data.compartments[0].label === "runtime-smoke" &&
  data.compartments[0].is_active === true;
'
run_doctor "${session_token}"

api_key_set="$(api_post "/api/api-keys/set" "{\"key\":\"${API_KEY_NAME}\",\"value\":\"${API_KEY_VALUE}\"}" "${session_token}")"
json_assert "${api_key_set}" "api key canary set" "
return data.status === 'ok' &&
  data.key === '${API_KEY_NAME}';
"

secret_set="$(api_post "/api/secrets/set" "{\"key\":\"${SECRET_NAME}\",\"value\":\"${SECRET_VALUE}\"}" "${session_token}")"
json_assert "${secret_set}" "secret canary set" "
return data.status === 'ok' &&
  data.key === '${SECRET_NAME}';
"
verify_vault_canaries "${session_token}" "after init"

api_post "/api/lock" "{}" "${session_token}" >/dev/null
locked_status="$(api_get "/api/status")"
json_assert "${locked_status}" "locked status after lock" '
return data.initialized === true &&
  data.locked === true &&
  Array.isArray(data.unlocked_compartments) &&
  data.unlocked_compartments.length === 0;
'

unlock_response="$(api_post "/api/unlock" "{\"passphrase\":\"${PASSPHRASE}\"}")"
json_assert "${unlock_response}" "unlock response after lock" '
return data.status === "unlocked" &&
  typeof data.session_token === "string" &&
  data.session_token.length > 0 &&
  Array.isArray(data.unlocked_compartments) &&
  data.unlocked_compartments.length === 1;
'
unlocked_again_token="$(json_session_token "${unlock_response}")"

unlocked_again_status="$(api_get "/api/status" "${unlocked_again_token}")"
json_assert "${unlocked_again_status}" "status after re-unlock" '
return data.initialized === true &&
  data.locked === false &&
  Array.isArray(data.unlocked_compartments) &&
  data.unlocked_compartments.length === 1 &&
  data.active_compartment &&
  data.active_compartment.compartment_id === 0;
'
run_doctor "${unlocked_again_token}"
verify_vault_canaries "${unlocked_again_token}" "after re-unlock"

echo "runtime smoke checks passed"
