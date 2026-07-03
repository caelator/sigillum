#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

fail() {
  echo "desktop check failed: $*" >&2
  exit 1
}

require_command() {
  local command_name="$1"
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    fail "required command is missing: ${command_name}"
  fi
}

run_step() {
  echo
  echo "==> $*"
  "$@"
}

find_debug_bundle_root() {
  local candidate
  for candidate in \
    "${ROOT}/target/debug/bundle" \
    "${ROOT}/crates/sigillum-desktop/target/debug/bundle"
  do
    if [[ -d "${candidate}/macos/Sigillum.app" ]]; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  done
  return 1
}

require_command cargo
run_step cargo build -p sigillum-desktop --locked

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo
  echo "==> cargo tauri build --debug (skipped: desktop bundle check is macOS-only)"
  exit 0
fi

if [[ "${SIGILLUM_SKIP_DESKTOP_BUNDLE:-0}" == "1" ]]; then
  echo
  echo "==> cargo tauri build --debug (skipped by SIGILLUM_SKIP_DESKTOP_BUNDLE=1)"
  exit 0
fi

if ! cargo tauri --version >/dev/null 2>&1; then
  fail "required cargo subcommand is missing: cargo tauri. Install with: cargo install tauri-cli --version 2.11.4 --locked"
fi
require_command codesign

pushd crates/sigillum-desktop >/dev/null
run_step cargo tauri build --debug
popd >/dev/null

BUNDLE_ROOT="$(find_debug_bundle_root)" || fail "debug Sigillum.app bundle was not produced"
APP_PATH="${BUNDLE_ROOT}/macos/Sigillum.app"
DMG_DIR="${BUNDLE_ROOT}/dmg"

if [[ ! -d "${APP_PATH}" ]]; then
  fail "debug app bundle is missing: ${APP_PATH}"
fi

if [[ ! -d "${DMG_DIR}" ]] || ! find "${DMG_DIR}" -maxdepth 1 -type f -name '*.dmg' -print -quit | grep -q .; then
  fail "debug dmg bundle is missing under: ${DMG_DIR}"
fi

SIGNATURE_OUTPUT="$(codesign -dv --verbose=4 "${APP_PATH}" 2>&1)" || {
  echo "${SIGNATURE_OUTPUT}" >&2
  fail "codesign could not inspect the debug app bundle"
}

if ! grep -q '^Signature=' <<<"${SIGNATURE_OUTPUT}"; then
  echo "${SIGNATURE_OUTPUT}" >&2
  fail "debug app bundle is not signed"
fi

grep '^Signature=' <<<"${SIGNATURE_OUTPUT}"
echo "desktop bundle checks passed: ${APP_PATH}"
