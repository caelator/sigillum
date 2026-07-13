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
run_step "${ROOT}/scripts/test-macos-signing-env.sh"
run_step cargo build -p sigillum-desktop --locked

if [[ "${SIGILLUM_SKIP_DESKTOP_BUNDLE:-0}" == "1" && "${CI:-}" == "true" ]]; then
  fail "SIGILLUM_SKIP_DESKTOP_BUNDLE=1 is forbidden in CI"
fi

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

NOTICE_DIR="${ROOT}/target/release-gate-notices"
NOTICE_PATH="${NOTICE_DIR}/THIRD-PARTY-NOTICES.txt"
notice_created=0
cleanup_notice() {
  if [[ "${notice_created}" == "1" ]]; then
    rm -f "${NOTICE_PATH}"
    rmdir "${NOTICE_DIR}" >/dev/null 2>&1 || true
    notice_created=0
  fi
}
trap cleanup_notice EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
if [[ -L "${NOTICE_PATH}" ]]; then
  fail "desktop notice-overlay fixture path must not be a symlink: ${NOTICE_PATH}"
fi
if [[ ! -e "${NOTICE_PATH}" ]]; then
  mkdir -p "${NOTICE_DIR}"
  printf '%s\n' 'Sigillum release-gate notice-overlay fixture' >"${NOTICE_PATH}"
  notice_created=1
fi

run_step "${ROOT}/scripts/build-macos-bundle.sh" --debug \
  --config '{"bundle":{"resources":{"../../target/release-gate-notices/THIRD-PARTY-NOTICES.txt":"THIRD-PARTY-NOTICES.txt"}}}' \
  -- --locked
cleanup_notice
trap - EXIT HUP INT TERM

BUNDLE_ROOT="$(find_debug_bundle_root)" || fail "debug Sigillum.app bundle was not produced"
APP_PATH="${BUNDLE_ROOT}/macos/Sigillum.app"
DMG_DIR="${BUNDLE_ROOT}/dmg"

if [[ ! -d "${APP_PATH}" ]]; then
  fail "debug app bundle is missing: ${APP_PATH}"
fi
if [[ ! -s "${APP_PATH}/Contents/Resources/THIRD-PARTY-NOTICES.txt" ]]; then
  fail "debug app bundle is missing the sealed notice-overlay fixture"
fi

if [[ ! -d "${DMG_DIR}" ]]; then
  fail "debug dmg bundle is missing under: ${DMG_DIR}"
fi

dmg_files=()
while IFS= read -r -d '' candidate; do
  dmg_files+=("${candidate}")
done < <(find "${DMG_DIR}" -mindepth 1 -maxdepth 1 -type f -name '*.dmg' -print0)
if [[ "${#dmg_files[@]}" != "1" ]]; then
  fail "expected exactly one debug dmg under ${DMG_DIR}; found ${#dmg_files[@]}"
fi
DMG_PATH="${dmg_files[0]}"
SIGNING_MODE="$("${ROOT}/scripts/check-macos-signing-env.sh")"

run_step "${ROOT}/scripts/check-macos-bundle-signature.sh" \
  --mode "${SIGNING_MODE}" "${APP_PATH}" "${DMG_PATH}"
run_step "${ROOT}/scripts/test-macos-bundle-signature.sh" \
  --mode "${SIGNING_MODE}" "${APP_PATH}" "${DMG_PATH}"
echo "desktop bundle checks passed: ${APP_PATH}"
