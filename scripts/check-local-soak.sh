#!/usr/bin/env bash
# Env toggles:
#   SIGILLUM_SOAK_SECONDS: total bounded soak duration in seconds.
#   SIGILLUM_SOAK_INTERVAL_SECONDS: delay between full soak iterations.
#   SIGILLUM_SOAK_RECEIPT: optional JSON receipt output path.
#   SIGILLUM_SOAK_KEEP_ARTIFACTS: keep the temp artifact directory when set to 1.
#   SIGILLUM_SOAK_DAEMON_PORT: daemon loopback port.
#   SIGILLUM_SOAK_GATEWAY_PORT: gateway loopback port.
#   SIGILLUM_SOAK_GATEWAY_POLL_INTERVAL_SECONDS: gateway daemon poll interval.
#   SIGILLUM_SOAK_CHAOS: enable chaos mode when set exactly to 1.
#   SIGILLUM_SOAK_CHAOS_EVERY: kill/restart the daemon every N iterations in chaos mode.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

DAEMON_PORT="${SIGILLUM_SOAK_DAEMON_PORT:-19843}"
GATEWAY_PORT="${SIGILLUM_SOAK_GATEWAY_PORT:-19844}"
SOAK_SECONDS="${SIGILLUM_SOAK_SECONDS:-300}"
INTERVAL_SECONDS="${SIGILLUM_SOAK_INTERVAL_SECONDS:-5}"
CHAOS="${SIGILLUM_SOAK_CHAOS:-0}"
CHAOS_EVERY="${SIGILLUM_SOAK_CHAOS_EVERY:-10}"
DAEMON_URL="http://127.0.0.1:${DAEMON_PORT}"
GATEWAY_URL="http://127.0.0.1:${GATEWAY_PORT}"
BASE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sigillum-local-soak.XXXXXX")"
DAEMON_LOG="${BASE_DIR}/daemon.log"
GATEWAY_LOG="${BASE_DIR}/gateway.log"
RECEIPT_PATH="${SIGILLUM_SOAK_RECEIPT:-}"
KEEP_ARTIFACTS="${SIGILLUM_SOAK_KEEP_ARTIFACTS:-0}"
HOST_NAME="$(hostname 2>/dev/null || echo unknown)"
HOST_OS="$(uname -s 2>/dev/null || echo unknown) $(uname -r 2>/dev/null || echo unknown)"
GIT_COMMIT="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
GIT_BRANCH="$(git branch --show-current 2>/dev/null || echo unknown)"
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  if [[ -n "$(git status --porcelain)" ]]; then
    GIT_DIRTY="true"
  else
    GIT_DIRTY="false"
  fi
else
  GIT_DIRTY="unknown"
fi
DAEMON_PID=""
GATEWAY_PID=""
SESSION_TOKEN=""
PASSPHRASE="local-soak-passphrase-123"
HARNESS_START_EPOCH=""
HARNESS_START_ISO=""
SOAK_START_EPOCH=""
SOAK_START_ISO=""
ITERATIONS=0
DOCTOR_RUNS=0
RECEIPT_WRITTEN=0
CHAOS_KILL_CYCLES=0
PENDING_CHAOS_VERIFY=0
CHAOS_IN_FLIGHT_TEST="chaos_kill_in_flight_plan_step_resumes_terminal_without_duplication"
CHAOS_IN_FLIGHT_STATUS=""
CHAOS_IN_FLIGHT_DURATION_SECONDS=""

write_receipt() {
  local status="$1"
  local failure_reason="${2:-}"
  if [[ -z "${RECEIPT_PATH}" ]]; then
    return 0
  fi

  mkdir -p "$(dirname "${RECEIPT_PATH}")"

  local finished_epoch
  local finished_iso
  local duration_seconds
  finished_epoch="$(date +%s)"
  finished_iso="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  if [[ -n "${HARNESS_START_EPOCH}" ]]; then
    duration_seconds=$((finished_epoch - HARNESS_START_EPOCH))
  else
    duration_seconds=0
  fi

  SIGILLUM_SOAK_RECEIPT_PATH="${RECEIPT_PATH}" \
  SIGILLUM_SOAK_RECEIPT_STATUS="${status}" \
  SIGILLUM_SOAK_RECEIPT_FAILURE="${failure_reason}" \
  SIGILLUM_SOAK_RECEIPT_HOST="${HOST_NAME}" \
  SIGILLUM_SOAK_RECEIPT_HOST_OS="${HOST_OS}" \
  SIGILLUM_SOAK_RECEIPT_COMMIT="${GIT_COMMIT}" \
  SIGILLUM_SOAK_RECEIPT_BRANCH="${GIT_BRANCH}" \
  SIGILLUM_SOAK_RECEIPT_DIRTY="${GIT_DIRTY}" \
  SIGILLUM_SOAK_RECEIPT_DAEMON_URL="${DAEMON_URL}" \
  SIGILLUM_SOAK_RECEIPT_GATEWAY_URL="${GATEWAY_URL}" \
  SIGILLUM_SOAK_RECEIPT_SOAK_SECONDS="${SOAK_SECONDS}" \
  SIGILLUM_SOAK_RECEIPT_INTERVAL_SECONDS="${INTERVAL_SECONDS}" \
  SIGILLUM_SOAK_RECEIPT_STARTED_AT="${HARNESS_START_ISO}" \
  SIGILLUM_SOAK_RECEIPT_SOAK_STARTED_AT="${SOAK_START_ISO}" \
  SIGILLUM_SOAK_RECEIPT_FINISHED_AT="${finished_iso}" \
  SIGILLUM_SOAK_RECEIPT_DURATION_SECONDS="${duration_seconds}" \
  SIGILLUM_SOAK_RECEIPT_ITERATIONS="${ITERATIONS}" \
  SIGILLUM_SOAK_RECEIPT_DOCTOR_RUNS="${DOCTOR_RUNS}" \
  SIGILLUM_SOAK_RECEIPT_CHAOS="${CHAOS}" \
  SIGILLUM_SOAK_RECEIPT_CHAOS_EVERY="${CHAOS_EVERY}" \
  SIGILLUM_SOAK_RECEIPT_CHAOS_KILL_CYCLES="${CHAOS_KILL_CYCLES}" \
  SIGILLUM_SOAK_RECEIPT_CHAOS_IN_FLIGHT_TEST="${CHAOS_IN_FLIGHT_TEST}" \
  SIGILLUM_SOAK_RECEIPT_CHAOS_IN_FLIGHT_STATUS="${CHAOS_IN_FLIGHT_STATUS}" \
  SIGILLUM_SOAK_RECEIPT_CHAOS_IN_FLIGHT_DURATION_SECONDS="${CHAOS_IN_FLIGHT_DURATION_SECONDS}" \
  SIGILLUM_SOAK_RECEIPT_KEEP_ARTIFACTS="${KEEP_ARTIFACTS}" \
  SIGILLUM_SOAK_RECEIPT_BASE_DIR="${BASE_DIR}" \
  SIGILLUM_SOAK_RECEIPT_DAEMON_LOG="${DAEMON_LOG}" \
  SIGILLUM_SOAK_RECEIPT_GATEWAY_LOG="${GATEWAY_LOG}" \
    node -e '
const fs = require("fs");
const path = process.env.SIGILLUM_SOAK_RECEIPT_PATH;
const number = (name) => {
  const value = Number(process.env[name] || "0");
  return Number.isFinite(value) ? value : 0;
};
const keepArtifacts = process.env.SIGILLUM_SOAK_RECEIPT_KEEP_ARTIFACTS === "1";
const chaosEnabled = process.env.SIGILLUM_SOAK_RECEIPT_CHAOS === "1";
const chaosAssertionStatus = process.env.SIGILLUM_SOAK_RECEIPT_CHAOS_IN_FLIGHT_STATUS || "";
const dirtyValue = process.env.SIGILLUM_SOAK_RECEIPT_DIRTY;
const data = {
  schema_version: 1,
  kind: "sigillum.local_soak",
  status: process.env.SIGILLUM_SOAK_RECEIPT_STATUS,
  failure_reason: process.env.SIGILLUM_SOAK_RECEIPT_FAILURE || null,
  repo: {
    commit: process.env.SIGILLUM_SOAK_RECEIPT_COMMIT,
    branch: process.env.SIGILLUM_SOAK_RECEIPT_BRANCH,
    dirty: dirtyValue === "true" ? true : dirtyValue === "false" ? false : null,
  },
  host: {
    name: process.env.SIGILLUM_SOAK_RECEIPT_HOST,
    os: process.env.SIGILLUM_SOAK_RECEIPT_HOST_OS,
  },
  urls: {
    daemon: process.env.SIGILLUM_SOAK_RECEIPT_DAEMON_URL,
    gateway: process.env.SIGILLUM_SOAK_RECEIPT_GATEWAY_URL,
  },
  configured: {
    soak_seconds: number("SIGILLUM_SOAK_RECEIPT_SOAK_SECONDS"),
    interval_seconds: number("SIGILLUM_SOAK_RECEIPT_INTERVAL_SECONDS"),
  },
  timing: {
    started_at_utc: process.env.SIGILLUM_SOAK_RECEIPT_STARTED_AT || null,
    soak_started_at_utc: process.env.SIGILLUM_SOAK_RECEIPT_SOAK_STARTED_AT || null,
    finished_at_utc: process.env.SIGILLUM_SOAK_RECEIPT_FINISHED_AT,
    duration_seconds: number("SIGILLUM_SOAK_RECEIPT_DURATION_SECONDS"),
  },
  evidence: {
    iterations: number("SIGILLUM_SOAK_RECEIPT_ITERATIONS"),
    doctor_runs: number("SIGILLUM_SOAK_RECEIPT_DOCTOR_RUNS"),
    checks: [
      "daemon_status",
      "vault_api_key_write_read",
      "gateway_health",
      "sigillum_doctor",
    ],
  },
  chaos: {
    enabled: chaosEnabled,
    every_iterations: chaosEnabled
      ? number("SIGILLUM_SOAK_RECEIPT_CHAOS_EVERY")
      : null,
    kill_cycles: number("SIGILLUM_SOAK_RECEIPT_CHAOS_KILL_CYCLES"),
    in_flight_assertion: chaosAssertionStatus
      ? {
          test: process.env.SIGILLUM_SOAK_RECEIPT_CHAOS_IN_FLIGHT_TEST,
          status: chaosAssertionStatus,
          duration_seconds: number(
            "SIGILLUM_SOAK_RECEIPT_CHAOS_IN_FLIGHT_DURATION_SECONDS",
          ),
        }
      : null,
  },
  artifacts: keepArtifacts
    ? {
        base_dir: process.env.SIGILLUM_SOAK_RECEIPT_BASE_DIR,
        daemon_log: process.env.SIGILLUM_SOAK_RECEIPT_DAEMON_LOG,
        gateway_log: process.env.SIGILLUM_SOAK_RECEIPT_GATEWAY_LOG,
      }
    : null,
};
fs.writeFileSync(path, `${JSON.stringify(data, null, 2)}\n`);
' || true
  RECEIPT_WRITTEN=1
}

cleanup() {
  for pid in "${GATEWAY_PID}" "${DAEMON_PID}"; do
    if [[ -n "${pid}" ]] && kill -0 "${pid}" >/dev/null 2>&1; then
      kill "${pid}" >/dev/null 2>&1 || true
      wait "${pid}" >/dev/null 2>&1 || true
    fi
  done
  if [[ "${KEEP_ARTIFACTS}" == "1" ]]; then
    echo "local soak artifacts kept at ${BASE_DIR}"
  else
    rm -rf "${BASE_DIR}"
  fi
}

on_exit() {
  local status="$?"
  if [[ "${status}" -ne 0 && "${RECEIPT_WRITTEN}" != "1" ]]; then
    write_receipt "failed" "script exited with status ${status}"
  fi
  cleanup
}
trap on_exit EXIT

fail() {
  echo "local soak failed: $*" >&2
  for log in "${DAEMON_LOG}" "${GATEWAY_LOG}"; do
    if [[ -f "${log}" ]]; then
      echo "--- ${log} ---" >&2
      tail -n 80 "${log}" >&2 || true
    fi
  done
  write_receipt "failed" "$*"
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
  if SIGILLUM_BASE_DIR="${BASE_DIR}" \
    SIGILLUM_SESSION_TOKEN="${SESSION_TOKEN}" \
    cargo run -p sigillum-cli --quiet -- doctor --url "${DAEMON_URL}" >/dev/null; then
    DOCTOR_RUNS=$((DOCTOR_RUNS + 1))
  else
    fail "sigillum doctor failed"
  fi
}

start_daemon() {
  echo "==> starting daemon on ${DAEMON_URL}"
  if [[ "${CHAOS}" == "1" ]]; then
    SIGILLUM_BASE_DIR="${BASE_DIR}" \
      "${ROOT}/target/debug/sigillum" daemon --port "${DAEMON_PORT}" >"${DAEMON_LOG}" 2>&1 &
  else
    SIGILLUM_BASE_DIR="${BASE_DIR}" \
      cargo run -p sigillum-cli --quiet -- daemon --port "${DAEMON_PORT}" >"${DAEMON_LOG}" 2>&1 &
  fi
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

unlock_daemon() {
  local unlock_response
  unlock_response="$(api_post "${DAEMON_URL}/api/unlock" "{\"passphrase\":\"${PASSPHRASE}\"}")"
  SESSION_TOKEN="$(json_session_token "${unlock_response}")"
}

run_chaos_in_flight_assertion() {
  local started_epoch
  local finished_epoch
  started_epoch="$(date +%s)"
  if cargo test -p sigillum-daemon --test execution_semantics chaos_kill_in_flight; then
    CHAOS_IN_FLIGHT_STATUS="passed"
  else
    finished_epoch="$(date +%s)"
    CHAOS_IN_FLIGHT_DURATION_SECONDS=$((finished_epoch - started_epoch))
    CHAOS_IN_FLIGHT_STATUS="failed"
    fail "chaos in-flight plan-step assertion failed"
  fi
  finished_epoch="$(date +%s)"
  CHAOS_IN_FLIGHT_DURATION_SECONDS=$((finished_epoch - started_epoch))
}

chaos_kill_cycle() {
  echo "==> chaos kill cycle after iteration ${ITERATIONS}: killing daemon pid ${DAEMON_PID}"
  if [[ -n "${DAEMON_PID}" ]] && kill -0 "${DAEMON_PID}" >/dev/null 2>&1; then
    kill -9 "${DAEMON_PID}" >/dev/null 2>&1 || true
    wait "${DAEMON_PID}" 2>/dev/null || true
    DAEMON_PID=""
  fi

  start_daemon
  unlock_daemon

  if [[ -z "${CHAOS_IN_FLIGHT_STATUS}" ]]; then
    run_chaos_in_flight_assertion
  fi

  PENDING_CHAOS_VERIFY=1
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
if [[ "${CHAOS}" == "1" ]]; then
  if ! [[ "${CHAOS_EVERY}" =~ ^[0-9]+$ ]] || [[ "${CHAOS_EVERY}" -lt 1 ]]; then
    fail "SIGILLUM_SOAK_CHAOS_EVERY must be a positive integer"
  fi
fi

HARNESS_START_EPOCH="$(date +%s)"
HARNESS_START_ISO="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

if [[ "${CHAOS}" == "1" ]]; then
  cargo build -p sigillum-cli --quiet
  cargo test -p sigillum-daemon --test execution_semantics --no-run --quiet
fi

start_daemon
initialize_daemon
start_gateway

echo "==> soaking daemon and gateway for ${SOAK_SECONDS}s (interval ${INTERVAL_SECONDS}s)"
SOAK_START_EPOCH="$(date +%s)"
SOAK_START_ISO="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
deadline=$((SOAK_START_EPOCH + SOAK_SECONDS))
while :; do
  ITERATIONS=$((ITERATIONS + 1))
  soak_iteration "${ITERATIONS}"

  if [[ "${PENDING_CHAOS_VERIFY}" == "1" ]]; then
    CHAOS_KILL_CYCLES=$((CHAOS_KILL_CYCLES + 1))
    PENDING_CHAOS_VERIFY=0
  fi
  if [[ "${CHAOS}" == "1" ]] && ((ITERATIONS % CHAOS_EVERY == 0)); then
    chaos_kill_cycle
  fi

  now="$(date +%s)"
  if [[ "${now}" -ge "${deadline}" && "${PENDING_CHAOS_VERIFY}" != "1" ]]; then
    break
  fi
  sleep "${INTERVAL_SECONDS}"
done

write_receipt "passed"

echo "local soak checks passed (${ITERATIONS} iteration(s), ${SOAK_SECONDS}s target)"
if [[ "${CHAOS}" == "1" ]]; then
  echo "local soak chaos cycles verified: ${CHAOS_KILL_CYCLES}"
fi
if [[ -n "${RECEIPT_PATH}" ]]; then
  echo "local soak receipt written to ${RECEIPT_PATH}"
fi
