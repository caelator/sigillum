#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

PORT="${SIGILLUM_BROWSER_SMOKE_PORT:-19843}"
URL="http://127.0.0.1:${PORT}"
STARTUP_TIMEOUT_SECONDS="${SIGILLUM_BROWSER_SMOKE_STARTUP_TIMEOUT_SECONDS:-120}"
BASE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sigillum-browser-smoke.XXXXXX")"
ARTIFACT_DIR="${SIGILLUM_BROWSER_SMOKE_ARTIFACT_DIR:-${BASE_DIR}/artifacts}"
LOG_FILE="${BASE_DIR}/daemon.log"
DAEMON_PID=""
FAILED=0

cleanup() {
  if [[ -n "${DAEMON_PID}" ]] && kill -0 "${DAEMON_PID}" >/dev/null 2>&1; then
    kill "${DAEMON_PID}" >/dev/null 2>&1 || true
    wait "${DAEMON_PID}" >/dev/null 2>&1 || true
  fi

  if [[ "${SIGILLUM_BROWSER_SMOKE_KEEP_ARTIFACTS:-0}" != "1" && "${FAILED}" != "1" ]]; then
    rm -rf "${BASE_DIR}"
  else
    echo "browser smoke artifacts kept at ${BASE_DIR}"
  fi
}
trap cleanup EXIT

fail() {
  FAILED=1
  echo "browser smoke failed: $*" >&2
  if [[ -f "${LOG_FILE}" ]]; then
    echo "--- daemon log ---" >&2
    tail -n 80 "${LOG_FILE}" >&2 || true
  fi
  if [[ -d "${ARTIFACT_DIR}" ]]; then
    echo "--- browser artifacts ---" >&2
    find "${ARTIFACT_DIR}" -maxdepth 1 -type f -print >&2 || true
  fi
  exit 1
}

require_command() {
  local command_name="$1"
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    fail "required command is missing: ${command_name}"
  fi
}

wait_for_daemon() {
  local deadline=$((SECONDS + STARTUP_TIMEOUT_SECONDS))
  while (( SECONDS < deadline )); do
    if curl -fsS "${URL}/api/status" >/dev/null 2>&1; then
      return
    fi
    if ! kill -0 "${DAEMON_PID}" >/dev/null 2>&1; then
      fail "daemon exited before becoming ready"
    fi
    sleep 0.25
  done
  fail "daemon did not become ready at ${URL} within ${STARTUP_TIMEOUT_SECONDS}s"
}

require_command cargo
require_command curl
require_command node

mkdir -p "${ARTIFACT_DIR}"

echo "==> starting sigillum daemon browser smoke on ${URL}"
SIGILLUM_BASE_DIR="${BASE_DIR}" \
  cargo run -p sigillum-cli --quiet --locked -- daemon --port "${PORT}" >"${LOG_FILE}" 2>&1 &
DAEMON_PID="$!"
wait_for_daemon

SIGILLUM_BROWSER_SMOKE_URL="${URL}" \
SIGILLUM_BROWSER_SMOKE_ARTIFACT_DIR="${ARTIFACT_DIR}" \
  node scripts/browser-smoke.mjs || fail "headless browser workflow failed"
