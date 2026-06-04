#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

DAEMON_PORT="${SIGILLUM_SOAK_DAEMON_PORT:-19843}"
GATEWAY_PORT="${SIGILLUM_SOAK_GATEWAY_PORT:-19844}"
SOAK_SECONDS="${SIGILLUM_SOAK_SECONDS:-300}"
INTERVAL_SECONDS="${SIGILLUM_SOAK_INTERVAL_SECONDS:-5}"
DAEMON_URL="http://127.0.0.1:${DAEMON_PORT}"
GATEWAY_URL="http://127.0.0.1:${GATEWAY_PORT}"
BASE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sigillum-local-soak.XXXXXX")"
DAEMON_LOG="${BASE_DIR}/daemon.log"
GATEWAY_LOG="${BASE_DIR}/gateway.log"
DAEMON_PID=""
GATEWAY_PID=""
SESSION_TOKEN=""
PASSPHRASE="local-soak-passphrase-123"

cleanup() {
  for pid in "${GATEWAY_PID}" "${DAEMON_PID}"; do
    if [[ -n "${pid}" ]] && kill -0 "${pid}" >/dev/null 2>&1; then
      kill "${pid}" >/dev/null 2>&1 || true
      wait "${pid}" >/dev/null 2>&1 || true
    fi
  done
  rm -rf "${BASE_DIR}"
}
trap cleanup EXIT

fail() {
  echo "local soak failed: $*" >&2
  for log in "${DAEMON_LOG}" "${GATEWAY_LOG}"; do
    if [[ -f "${log}" ]]; then
      echo "--- ${log} ---" >&2
      tail -n 80 "${log}" >&2 || true
    fi
  done
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
  local url="$1"
  local token="${2:-}"
  if [[ -n "${token}" ]]; then
    curl -fsS -H "Authorization: Bearer ${token}" "${url}"
  else
    curl -fsS "${url}"
  fi
}

api_post() {
  local url="$1"
  local body="$2"
  local token="${3:-}"
  if [[ -n "${token}" ]]; then
    curl -fsS \
      -X POST \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer ${token}" \
      --data "${body}" \
      "${url}"
  else
    curl -fsS \
      -X POST \
      -H "Content-Type: application/json" \
      --data "${body}" \
      "${url}"
  fi
}

wait_for_url() {
  local url="$1"
  local pid="$2"
  local name="$3"
  for _ in {1..120}; do
    if curl -fsS "${url}" >/dev/null 2>&1; then
      return
    fi
    if ! kill -0 "${pid}" >/dev/null 2>&1; then
      fail "${name} exited before becoming ready"
    fi
    sleep 0.25
  done
  fail "${name} did not become ready at ${url}"
}

run_doctor() {
  SIGILLUM_BASE_DIR="${BASE_DIR}" \
    SIGILLUM_SESSION_TOKEN="${SESSION_TOKEN}" \
    cargo run -p sigillum-cli --quiet -- doctor --url "${DAEMON_URL}" >/dev/null
}

start_daemon() {
  echo "==> starting daemon on ${DAEMON_URL}"
  SIGILLUM_BASE_DIR="${BASE_DIR}" \
    cargo run -p sigillum-cli --quiet -- daemon --port "${DAEMON_PORT}" >"${DAEMON_LOG}" 2>&1 &
  DAEMON_PID="$!"
  wait_for_url "${DAEMON_URL}/api/status" "${DAEMON_PID}" "daemon"
}

initialize_daemon() {
  local init_body
  init_body='{"id":0,"passphrase":"local-soak-passphrase-123","label":"local-soak","threshold":1}'
  local init_response
  init_response="$(api_post "${DAEMON_URL}/api/compartment/init" "${init_body}")"
  json_assert "${init_response}" "soak compartment init" '
return data.status === "initialized" &&
  data.compartment_id === 0 &&
  typeof data.session_token === "string" &&
  data.session_token.length > 0;
'
  SESSION_TOKEN="$(json_session_token "${init_response}")"
}

start_gateway() {
  echo "==> starting gateway on ${GATEWAY_URL}"
  GATEWAY_ADMIN_KEY="local-soak-admin" \
    GATEWAY_BIND_ADDR="127.0.0.1:${GATEWAY_PORT}" \
    GATEWAY_DATABASE_URL="sqlite://${BASE_DIR}/gateway.db?mode=rwc" \
    SIGILLUM_DAEMON_URL="${DAEMON_URL}" \
    SIGILLUM_DAEMON_SESSION_TOKEN="${SESSION_TOKEN}" \
    GATEWAY_POLL_INTERVAL_SECS="${SIGILLUM_SOAK_GATEWAY_POLL_INTERVAL_SECONDS:-3600}" \
    GATEWAY_RATE_LIMIT_RPS="0" \
    GATEWAY_AUTH_CACHE_TTL_SECS="1" \
    cargo run -p sigillum-gateway --quiet >"${GATEWAY_LOG}" 2>&1 &
  GATEWAY_PID="$!"
  wait_for_url "${GATEWAY_URL}/api/v1/health" "${GATEWAY_PID}" "gateway"
}

soak_iteration() {
  local iteration="$1"
  local status
  status="$(api_get "${DAEMON_URL}/api/status" "${SESSION_TOKEN}")"
  json_assert "${status}" "daemon status iteration ${iteration}" '
return data.initialized === true &&
  data.locked === false &&
  Array.isArray(data.unlocked_compartments) &&
  data.unlocked_compartments.length === 1 &&
  data.active_compartment &&
  data.active_compartment.compartment_id === 0;
'

  local value="iteration-${iteration}"
  local set_response
  set_response="$(api_post "${DAEMON_URL}/api/api-keys/set" "{\"key\":\"soak_probe\",\"value\":\"${value}\"}" "${SESSION_TOKEN}")"
  json_assert "${set_response}" "api-key set iteration ${iteration}" '
return data.status === "ok" &&
  data.key === "soak_probe";
'

  local get_response
  get_response="$(api_post "${DAEMON_URL}/api/api-keys/get" '{"key":"soak_probe"}' "${SESSION_TOKEN}")"
  json_assert "${get_response}" "api-key get iteration ${iteration}" "
return data.key === 'soak_probe' &&
  data.value === '${value}';
"

  local health
  health="$(api_get "${GATEWAY_URL}/api/v1/health")"
  json_assert "${health}" "gateway health iteration ${iteration}" '
return data.gateway === "ok" &&
  data.daemon === "ok" &&
  data.daemon_healthy === true;
'

  run_doctor
}

require_command cargo
require_command curl
require_command node

if ! [[ "${SOAK_SECONDS}" =~ ^[0-9]+$ ]] || [[ "${SOAK_SECONDS}" -lt 1 ]]; then
  fail "SIGILLUM_SOAK_SECONDS must be a positive integer"
fi
if ! [[ "${INTERVAL_SECONDS}" =~ ^[0-9]+$ ]] || [[ "${INTERVAL_SECONDS}" -lt 1 ]]; then
  fail "SIGILLUM_SOAK_INTERVAL_SECONDS must be a positive integer"
fi

start_daemon
initialize_daemon
start_gateway

echo "==> soaking daemon and gateway for ${SOAK_SECONDS}s (interval ${INTERVAL_SECONDS}s)"
start_epoch="$(date +%s)"
deadline=$((start_epoch + SOAK_SECONDS))
iteration=0
while :; do
  iteration=$((iteration + 1))
  soak_iteration "${iteration}"

  now="$(date +%s)"
  if [[ "${now}" -ge "${deadline}" ]]; then
    break
  fi
  sleep "${INTERVAL_SECONDS}"
done

echo "local soak checks passed (${iteration} iteration(s), ${SOAK_SECONDS}s target)"
